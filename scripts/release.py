#!/usr/bin/env python3
"""Build release archives and promote their verified bytes. Requires Python 3.11+."""

import argparse
from dataclasses import asdict, dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parent.parent
VERSION = r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)'
TARGETS = {
    'macos-arm64': ('aarch64-apple-darwin', '.zip'),
    'macos-x64': ('x86_64-apple-darwin', '.zip'),
    'windows-x64': ('x86_64-pc-windows-msvc', '.zip'),
    'linux-x64': ('x86_64-unknown-linux-gnu', '.tar.gz'),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def version(root=ROOT):
    value = tomllib.loads((root / 'Cargo.toml').read_text())['workspace']['package']['version']
    require(re.fullmatch(VERSION, value), 'Use a numeric major.minor.patch version')
    return value


def bump(kind, exact='', root=ROOT):
    old = version(root)
    numbers = list(map(int, old.split('.')))
    if kind == 'set':
        require(re.fullmatch(VERSION, exact), 'Enter an exact major.minor.patch version')
        new = exact
    else:
        index = {'major': 0, 'minor': 1, 'patch': 2}[kind]
        numbers[index] += 1
        numbers[index + 1:] = [0] * (2 - index)
        new = '.'.join(map(str, numbers))
    require(tuple(map(int, new.split('.'))) > tuple(map(int, old.split('.'))),
            'The new version must be greater than the current version')
    cargo = root / 'Cargo.toml'
    text, count = re.subn(r'(?m)^version = "' + re.escape(old) + r'"$',
                         f'version = "{new}"', cargo.read_text())
    require(count == 1, 'Expected one shared workspace version')
    lock = root / 'Cargo.lock'
    pattern = r'(\[\[package\]\]\nname = "sniplet-(?:app|core|platform)"\nversion = ")' + re.escape(old) + '"'
    lock_text, count = re.subn(pattern, lambda match: match[1] + new + '"', lock.read_text())
    require(count == 3, 'Expected all three workspace packages in Cargo.lock')
    cargo.write_text(text)
    lock.write_text(lock_text)
    output(version=new)
    return new


@dataclass(frozen=True)
class Build:
    version: str
    sha: str
    channel: str = 'nightly'

    def __post_init__(self):
        require(re.fullmatch(VERSION, self.version), 'Invalid release version')
        require(re.fullmatch(r'[0-9a-f]{40}', self.sha), 'Expected a full commit SHA')
        require(self.channel in ('nightly', 'stable'), 'Invalid release channel')

    @property
    def tag(self):
        suffix = '-' + self.sha[:7] if self.channel == 'nightly' else ''
        return 'v' + self.version + suffix

    def filename(self, target):
        return f'Sniplet-{self.tag[1:]}-{target}{TARGETS[target][1]}'


@dataclass(frozen=True)
class Asset:
    target: str
    name: str
    sha256: str
    size: int

    def validate(self, build):
        require(self.target in TARGETS, 'Unknown package target')
        require(self.name == build.filename(self.target), 'Unexpected package filename')
        require(re.fullmatch(r'[0-9a-f]{64}', self.sha256), 'Invalid package checksum')
        require(type(self.size) is int and self.size > 0, 'Invalid package size')


def checksum(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def manifest(build, assets, source_nightly=None):
    require(len(assets) == len(TARGETS) and {asset.target for asset in assets} == set(TARGETS),
            'All four platform packages are required')
    for asset in assets:
        asset.validate(build)
    result = {'schema': 1, **asdict(build), 'tag': build.tag,
              'assets': [asdict(asset) for asset in sorted(assets, key=lambda item: item.target)]}
    if source_nightly:
        result['source_nightly'] = source_nightly
    return result


def read_manifest(path):
    data = json.loads(path.read_text())
    require(data['schema'] == 1, 'Unsupported release manifest')
    build = Build(data['version'], data['sha'], data['channel'])
    assets = [Asset(**item) for item in data['assets']]
    require(data['tag'] == build.tag, 'Release tag does not match its source commit')
    manifest(build, assets)
    return build, assets


def verify_files(directory, build, assets):
    manifest(build, assets)
    for asset in assets:
        path = directory / asset.name
        require(path.is_file() and not path.is_symlink(), f'Missing package: {asset.name}')
        require(path.stat().st_size == asset.size and checksum(path) == asset.sha256,
                f'Package checksum failed: {asset.name}')


def write_metadata(directory, build, assets, source_nightly=None):
    verify_files(directory, build, assets)
    write_json(directory / 'release.json', manifest(build, assets, source_nightly))
    (directory / 'SHA256SUMS').write_text(''.join(
        f'{asset.sha256}  {asset.name}\n' for asset in sorted(assets, key=lambda item: item.name)))


def archive_bundle(bundle, destination, target):
    if target.startswith('macos-'):
        subprocess.run(['ditto', '-c', '-k', '--sequesterRsrc', '--keepParent',
                        str(bundle), str(destination)], check=True)
    elif target == 'linux-x64':
        with tarfile.open(destination, 'w:gz') as archive:
            archive.add(bundle, arcname='sniplet')
    else:
        shutil.make_archive(str(destination.with_suffix('')), 'zip', bundle.parent, bundle.name)


def package(target, sha):
    build = Build(version(), sha)
    current = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    require(current == sha, 'Checkout does not match the requested source commit')
    host = subprocess.check_output(['rustc', '-vV'], text=True)
    require(f'host: {TARGETS[target][0]}\n' in host, 'Runner architecture does not match the package')
    if target == 'windows-x64':
        subprocess.run(['pwsh', '-NoProfile', '-File', 'scripts/package.ps1',
                        '-OutputDirectory', 'dist-release', '-SkipBuild'], check=True, cwd=ROOT)
        bundle = ROOT / 'dist-release' / ('sniplet-' + TARGETS[target][0])
    else:
        subprocess.run(['sh', 'scripts/package.sh', 'dist-release'], check=True, cwd=ROOT,
                       env={**os.environ, 'SNIPLET_PACKAGE_SKIP_BUILD': '1',
                            'SNIPLET_PACKAGE_PROFILE': 'release'})
        name = 'Sniplet.app' if target.startswith('macos-') else 'sniplet-' + TARGETS[target][0]
        bundle = ROOT / 'dist-release' / name
    directory = ROOT / 'dist-release' / 'assets'
    directory.mkdir(parents=True, exist_ok=True)
    destination = directory / build.filename(target)
    archive_bundle(bundle, destination, target)
    asset = Asset(target, destination.name, checksum(destination), destination.stat().st_size)
    write_json(directory / (target + '.json'), {**asdict(build), 'asset': asdict(asset)})


class GitHub:
    """Use gh's existing token handling; release jobs supply only GITHUB_TOKEN."""

    def __init__(self):
        self.repo = os.environ['GITHUB_REPOSITORY']

    def api(self, endpoint, method='GET', data=None):
        command = ['gh', 'api', f'repos/{self.repo}/{endpoint}', '--method', method]
        if data is not None:
            command += ['--input', '-']
        result = subprocess.run(command, input=json.dumps(data) if data is not None else None,
                                capture_output=True, text=True)
        if result.returncode and method == 'GET' and 'HTTP 404' in result.stderr:
            return None
        require(result.returncode == 0, result.stderr.strip() or 'GitHub request failed')
        return json.loads(result.stdout) if result.stdout.strip() else None

    def release(self, tag):
        return self.api('releases/tags/' + tag)

    def tag_sha(self, tag):
        ref = self.api('git/ref/tags/' + tag)
        if ref is None:
            return None
        obj = ref['object']
        while obj['type'] == 'tag':
            obj = self.api('git/tags/' + obj['sha'])['object']
        require(obj['type'] == 'commit', 'Release tag must point to a commit')
        return obj['sha']

    def download(self, tag, directory, patterns):
        command = ['gh', 'release', 'download', tag, '--repo', self.repo, '--dir', str(directory)]
        for pattern in patterns:
            command += ['--pattern', pattern]
        subprocess.run(command, check=True)

    def upload(self, tag, files):
        subprocess.run(['gh', 'release', 'upload', tag, '--repo', self.repo, '--clobber',
                        *map(str, files)], check=True)


def check_tag(github, build):
    sha = github.tag_sha(build.tag)
    require(sha is None or sha == build.sha, f'Tag {build.tag} already points to another commit')
    return sha


def published_manifest(github, release):
    require(not release['draft'], 'Select a published nightly release')
    with tempfile.TemporaryDirectory() as temporary:
        directory = Path(temporary)
        github.download(release['tag_name'], directory, ['release.json'])
        build, assets = read_manifest(directory / 'release.json')
    require(release['tag_name'] == build.tag and release['prerelease'] == (build.channel == 'nightly'),
            'Release channel does not match its manifest')
    require(github.tag_sha(build.tag) == build.sha, 'Release tag does not match its source commit')
    expected = {asset.name for asset in assets} | {'release.json', 'SHA256SUMS'}
    require({item['name'] for item in release['assets']} == expected, 'Release package set is incomplete')
    for asset in assets:
        remote = next(item for item in release['assets'] if item['name'] == asset.name)
        require(remote['size'] == asset.size, 'Release package size does not match its manifest')
        digest = remote.get('digest')
        require(not digest or digest == 'sha256:' + asset.sha256, 'GitHub package checksum mismatch')
    return build, assets


def output(**values):
    text = ''.join(f'{key}={str(value).lower() if isinstance(value, bool) else value}\n'
                   for key, value in values.items())
    print(text, end='')
    if path := os.environ.get('GITHUB_OUTPUT'):
        with open(path, 'a') as stream:
            stream.write(text)


def prepare(github):
    sha = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    build = Build(version(), sha)
    check_tag(github, build)
    release = github.release(build.tag)
    completed = release is not None and not release['draft']
    if completed:
        existing, _ = published_manifest(github, release)
        require(existing == build, 'Existing nightly has a different source commit')
    output(version=build.version, sha=build.sha, tag=build.tag, build=not completed)


def publish(github, build, assets, directory):
    """Retry draft uploads only. Published packages and tags are never overwritten."""
    verify_files(directory, build, assets)
    tag_sha = check_tag(github, build)
    release = github.release(build.tag)
    if release and not release['draft']:
        existing, published_assets = published_manifest(github, release)
        require(existing == build, 'Existing release has a different source commit')
        if build.channel == 'stable':
            require(sorted(published_assets, key=lambda item: item.target) ==
                    sorted(assets, key=lambda item: item.target),
                    'Existing stable release has different packages')
        print(f'Reusing published release {build.tag}')
        return
    files = [directory / asset.name for asset in assets]
    files += [directory / 'release.json', directory / 'SHA256SUMS']
    if release:
        require({item['name'] for item in release['assets']} <= {path.name for path in files},
                'Draft contains unexpected assets; inspect it before retrying')
    if tag_sha is None:
        github.api('git/refs', 'POST', {'ref': 'refs/tags/' + build.tag, 'sha': build.sha})
    nightly = build.channel == 'nightly'
    title = f'Sniplet {"Nightly " if nightly else ""}{build.tag[1:]}'
    notes = (f'Source commit: `{build.sha}`.\n\n'
             'Packages: macOS Apple Silicon and Intel, Windows x64, and Linux x64.\n'
             'Use SHA256SUMS to check the downloads. Mac packages use ad hoc signing; '
             'they are not notarized. Linux packages need the native libraries listed in the README.\n')
    if not nightly:
        notes += f'\nPromoted from `{Build(build.version, build.sha).tag}` without rebuilding.\n'
    if release is None:
        release = github.api('releases', 'POST', {'tag_name': build.tag, 'target_commitish': build.sha,
                             'name': title, 'body': notes, 'draft': True, 'prerelease': nightly,
                             'make_latest': 'false'})
    github.upload(build.tag, files)
    # Publication happens only after every upload is complete and verified.
    uploaded = github.release(build.tag)
    require(uploaded['draft'], 'Release changed during upload; publication stopped')
    require({item['name'] for item in uploaded['assets']} == {path.name for path in files},
            'Draft upload is incomplete')
    for path in files:
        remote = next(item for item in uploaded['assets'] if item['name'] == path.name)
        require(remote['size'] == path.stat().st_size and
                remote.get('digest') == 'sha256:' + checksum(path), 'Uploaded asset verification failed')
    require(check_tag(github, build) == build.sha, 'Release tag changed during upload')
    github.api('releases/' + str(release['id']), 'PATCH', {
        'draft': False, 'prerelease': nightly, 'name': title, 'body': notes,
        'make_latest': 'false' if nightly else 'legacy',
    })
    print(f'Published {build.tag}')


def publish_nightly(github, sha, directory):
    build = Build(version(), sha)
    current = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    require(current == sha, 'Checkout does not match the requested source commit')
    assets = []
    for target in TARGETS:
        data = json.loads((directory / (target + '.json')).read_text())
        require(Build(data['version'], data['sha'], data['channel']) == build,
                'Platform packages came from different builds')
        assets.append(Asset(**data['asset']))
    write_metadata(directory, build, assets)
    publish(github, build, assets, directory)


def promote(github, tag):
    require(re.fullmatch('v' + VERSION + r'-[0-9a-f]{7}', tag), 'Enter an exact nightly tag')
    release = github.release(tag)
    require(release is not None, 'Nightly release not found')
    build, assets = published_manifest(github, release)
    require(build.channel == 'nightly', 'Select a nightly release')
    stable = Build(build.version, build.sha, 'stable')
    check_tag(github, stable)
    with tempfile.TemporaryDirectory() as temporary:
        directory = Path(temporary)
        github.download(tag, directory, [asset.name for asset in assets] + ['SHA256SUMS'])
        verify_files(directory, build, assets)
        expected = ''.join(f'{asset.sha256}  {asset.name}\n' for asset in sorted(assets, key=lambda item: item.name))
        require((directory / 'SHA256SUMS').read_text() == expected, 'Nightly checksums do not match')
        promoted = []
        for asset in assets:
            name = stable.filename(asset.target)
            (directory / asset.name).rename(directory / name)
            promoted.append(Asset(asset.target, name, asset.sha256, asset.size))
        write_metadata(directory, stable, promoted, source_nightly=tag)
        publish(github, stable, promoted, directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    commands.add_parser('version')
    bump_parser = commands.add_parser('bump')
    bump_parser.add_argument('kind', choices=['patch', 'minor', 'major', 'set'])
    bump_parser.add_argument('--version', default='')
    commands.add_parser('prepare')
    package_parser = commands.add_parser('package')
    package_parser.add_argument('target', choices=TARGETS)
    package_parser.add_argument('--sha', required=True)
    publish_parser = commands.add_parser('publish-nightly')
    publish_parser.add_argument('--sha', required=True)
    publish_parser.add_argument('--directory', type=Path, default=ROOT / 'dist-release/assets')
    promote_parser = commands.add_parser('promote')
    promote_parser.add_argument('tag')
    args = parser.parse_args()
    if args.command == 'version':
        print(version())
    elif args.command == 'bump':
        bump(args.kind, args.version)
    elif args.command == 'package':
        package(args.target, args.sha)
    elif args.command == 'prepare':
        prepare(GitHub())
    elif args.command == 'publish-nightly':
        publish_nightly(GitHub(), args.sha, args.directory)
    elif args.command == 'promote':
        promote(GitHub(), args.tag)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, TypeError, OSError, subprocess.CalledProcessError) as error:
        raise SystemExit(f'Release failed: {error}') from error
