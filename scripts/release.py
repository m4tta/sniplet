#!/usr/bin/env python3
"""Plan, package, and publish Sniplet releases on GitHub. Requires Python 3.11+.
Packaging also needs Velopack's `vpk` tool, and on Mac the `dmgbuild` package.

Git tags hold the version, not Cargo.toml:
- Nightly: the newest stable tag plus one patch, then `-nightly.<YYYYMMDD>.<run number>`,
  such as `0.2.1-nightly.20261104.17`. Each nightly is newer than the newest stable and
  older than the next one, so the in-app channel switch never needs a downgrade.
- Stable: the newest nightly whose run passed, rebuilt as its base version (`0.2.1`),
  or as an exact version for a minor or major release.
"""

import argparse
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
VERSION = r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)'
NIGHTLY = VERSION + r'-nightly\.([0-9]{8})\.([1-9][0-9]*)'
MIN_NIGHTLY_GAP = 2 * 60 * 60
TARGETS = {
    'macos-arm64': 'aarch64-apple-darwin',
    'macos-x64': 'x86_64-apple-darwin',
    'windows-x64': 'x86_64-pc-windows-msvc',
    'linux-x64': 'x86_64-unknown-linux-gnu',
}
# Velopack's package id. It must match PACK_ID in crates/sniplet-platform/src/updates.rs.
PACK_ID = 'Sniplet'
# The in-app update package for each target, made by `vpk pack`.
UPDATE = '-full.nupkg'
# Velopack's runtime id for each target. Without it, vpk picks x86 on Windows.
RUNTIMES = {
    'macos-arm64': 'osx-arm64',
    'macos-x64': 'osx-x64',
    'windows-x64': 'win-x64',
    'linux-x64': 'linux-x64',
}
# The files that people download and install, by the end of their names.
DOWNLOADS = {
    'macos-arm64': ('.dmg',),
    'macos-x64': ('.dmg',),
    'windows-x64': ('-Setup.exe', '-Portable.zip'),
    'linux-x64': ('.AppImage', '.tar.gz'),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def version_key(version):
    """Sort key for a stable or nightly version; None for any other text."""
    if match := re.fullmatch(VERSION, version):
        # A stable release sorts after every nightly of the same base version.
        return (*map(int, match.groups()), 1, 0, 0)
    if match := re.fullmatch(NIGHTLY, version):
        return (*map(int, match.groups()[:3]), 0, *map(int, match.groups()[3:]))
    return None


@dataclass(frozen=True)
class Build:
    version: str
    sha: str

    def __post_init__(self):
        require(version_key(self.version), 'Invalid release version')
        require(re.fullmatch(r'[0-9a-f]{40}', self.sha), 'Expected a full commit SHA')

    @property
    def channel(self):
        return 'nightly' if '-' in self.version else 'stable'

    @property
    def tag(self):
        return 'v' + self.version

    def filename(self, target, ending=UPDATE):
        return f'{PACK_ID}-{self.version}-{target}{ending}'


@dataclass(frozen=True)
class Asset:
    target: str
    name: str
    sha256: str
    size: int
    kind: str = 'update'

    def validate(self, build):
        require(self.target in TARGETS, 'Unknown package target')
        require(self.kind in ('update', 'download'), 'Unknown package kind')
        endings = (UPDATE,) if self.kind == 'update' else DOWNLOADS[self.target]
        require(self.name in [build.filename(self.target, ending) for ending in endings],
                'Unexpected package filename')
        require(re.fullmatch(r'[0-9a-f]{64}', self.sha256), 'Invalid package checksum')
        require(type(self.size) is int and self.size > 0, 'Invalid package size')


def checksum(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def output(**values):
    text = ''.join(f'{key}={str(value).lower() if isinstance(value, bool) else value}\n'
                   for key, value in values.items())
    print(text, end='')
    if path := os.environ.get('GITHUB_OUTPUT'):
        with open(path, 'a') as stream:
            stream.write(text)


def git(*args, root=ROOT):
    return subprocess.check_output(['git', *args], cwd=root, text=True).strip()


def run_passed(run_id):
    result = subprocess.check_output(['gh', 'run', 'view', run_id, '--repo', os.environ['GITHUB_REPOSITORY'],
                                      '--json', 'conclusion', '--jq', '.conclusion'], text=True)
    return result.strip() == 'success'


def release_tags(root=ROOT):
    """Return {version: creation time} for every stable and nightly tag."""
    lines = git('for-each-ref', '--format=%(refname:short) %(creatordate:unix)', 'refs/tags/v*',
                root=root).splitlines()
    tags = {}
    for line in lines:
        tag, created = line.split()
        if version_key(tag[1:]):
            tags[tag[1:]] = int(created)
    return tags


def plan(env=os.environ, root=ROOT, now=None):
    """Decide what the Release workflow builds. A nightly pushes its tag before it builds,
    so a failed build also waits for new commits and the 2-hour gap.
    A pull request gets a throwaway nightly version, no tag, and no release."""
    if env.get('GITHUB_EVENT_NAME') == 'pull_request':
        date = datetime.fromtimestamp(now or time.time(), timezone.utc).strftime('%Y%m%d')
        build = Build(f'0.0.1-nightly.{date}.{env["GITHUB_RUN_NUMBER"]}', git('rev-parse', 'HEAD', root=root))
        return output(channel=build.channel, version=build.version, sha=build.sha, skip=False)
    require(env.get('GITHUB_REF') == 'refs/heads/main', 'Run releases from main')
    channel = env.get('CHANNEL') or 'nightly'
    tags = release_tags(root)
    stables = [v for v in tags if '-' not in v]
    nightlies = sorted((v for v in tags if '-' in v), key=version_key, reverse=True)
    latest_stable = max(stables, key=version_key, default='0.0.0')

    if channel == 'nightly':
        # A nightly started by hand always builds.
        if env.get('GITHUB_EVENT_NAME') == 'schedule' and nightlies:
            last = nightlies[0]
            age = (now or time.time()) - tags[last]
            if age < MIN_NIGHTLY_GAP:
                print(f'v{last} is {int(age // 60)} minutes old. Nightly skipped.')
                return output(skip=True)
            # The website in site/ deploys on its own, so site-only commits do not need a nightly.
            if not git('rev-list', '-n', '1', f'v{last}..HEAD', '--', '.', ':(exclude)site', root=root):
                print(f'main has no app commits after v{last}. Nightly skipped.')
                return output(skip=True)
        major, minor, patch = map(int, latest_stable.split('.'))
        date = datetime.fromtimestamp(now or time.time(), timezone.utc).strftime('%Y%m%d')
        build = Build(f'{major}.{minor}.{patch + 1}-nightly.{date}.{env["GITHUB_RUN_NUMBER"]}',
                      git('rev-parse', 'HEAD', root=root))
        # The tag body holds the run URL. Stable promotion reads it to check that this run passed.
        run_url = f'{env["GITHUB_SERVER_URL"]}/{env["GITHUB_REPOSITORY"]}/actions/runs/{env["GITHUB_RUN_ID"]}'
        git('-c', 'user.name=github-actions[bot]',
            '-c', 'user.email=41898282+github-actions[bot]@users.noreply.github.com',
            'tag', '-a', build.tag, build.sha, '-m', f'Nightly {build.version}', '-m', run_url, root=root)
        git('push', 'origin', build.tag, root=root)

    elif channel == 'stable':
        nightly = None
        for candidate in nightlies:
            body = git('tag', '-l', '--format=%(contents:body)', 'v' + candidate, root=root)
            run_id = body.strip().rsplit('/', 1)[-1]
            if run_id.isdigit() and run_passed(run_id):
                nightly = candidate
                break
        require(nightly, 'No nightly has passed yet. Run a nightly first')
        version = (env.get('RELEASE_VERSION') or nightly.split('-')[0]).removeprefix('v')
        require(re.fullmatch(VERSION, version), f'Version {version} is not X.Y.Z')
        require(version not in tags, f'v{version} already exists. Wait for a newer nightly, or set a version')
        require(version_key(version) > version_key(latest_stable), f'{version} is not newer than {latest_stable}')
        build = Build(version, git('rev-list', '-n', '1', 'v' + nightly, root=root))
        print(f'Stable {version} builds {build.sha}, the commit of v{nightly}.')

    else:
        raise ValueError(f'Unknown channel: {channel}')

    output(channel=build.channel, version=build.version, sha=build.sha, skip=False)


def manifest(build, assets):
    """The signed release.json. Apps read `assets`, one Velopack package for each target.
    Schema 4 keeps the field names of schema 3, so an app that reads schema 3 can tell
    its user to download this release by hand."""
    for asset in assets:
        asset.validate(build)
    expected = sorted(build.filename(target, ending)
                      for target in TARGETS for ending in (UPDATE, *DOWNLOADS[target]))
    require(sorted(asset.name for asset in assets) == expected,
            'Every platform needs its update package and all of its downloads')

    def entries(kind):
        return [{key: value for key, value in asdict(item).items() if key != 'kind'}
                for item in sorted(assets, key=lambda item: item.name) if item.kind == kind]

    return {'schema': 4, **asdict(build), 'channel': build.channel, 'tag': build.tag,
            'assets': entries('update'), 'downloads': entries('download')}


def verify_files(directory, build, assets):
    manifest(build, assets)
    for asset in assets:
        path = directory / asset.name
        require(path.is_file() and not path.is_symlink(), f'Missing package: {asset.name}')
        require(path.stat().st_size == asset.size and checksum(path) == asset.sha256,
                f'Package checksum failed: {asset.name}')


def write_metadata(directory, build, assets):
    from update_signing import sign
    verify_files(directory, build, assets)
    write_json(directory / 'release.json', manifest(build, assets))
    sign(directory / 'release.json')
    (directory / 'SHA256SUMS').write_text(''.join(
        f'{asset.sha256}  {asset.name}\n' for asset in sorted(assets, key=lambda item: item.name)))


def archive_linux(bundle, destination):
    """The tar.gz for people who install by hand. It does not update itself."""
    with tarfile.open(destination, 'w:gz') as archive:
        archive.add(bundle, arcname='sniplet')


def vpk_pack(target, build, pack_dir, main_exe, output, *options):
    """Make the Velopack update package and installers for one target. The channel is the
    target, so the file names of the four targets never collide in one GitHub release."""
    subprocess.run(['vpk', 'pack', '--yes', '--packId', PACK_ID, '--packTitle', 'Sniplet',
                    '--packVersion', build.version, '--channel', target, '--runtime', RUNTIMES[target],
                    '--packDir', str(pack_dir), '--mainExe', main_exe,
                    '--outputDir', str(output), *options], check=True, cwd=ROOT)
    made = output / f'{PACK_ID}-{build.version}-{target}{UPDATE}'
    require(made.is_file(), f'vpk did not make {made.name}')
    return lambda name: output / f'{PACK_ID}-{target}{name}'


def package(target, build):
    """Package an app that `cargo build -p sniplet-app --release` already built with this identity."""
    require(git('rev-parse', 'HEAD') == build.sha, 'Checkout does not match the requested source commit')
    host = subprocess.check_output(['rustc', '-vV'], text=True)
    require(f'host: {TARGETS[target]}\n' in host, 'Runner architecture does not match the package')
    mac = target.startswith('macos-')
    if target == 'windows-x64':
        subprocess.run(['pwsh', '-NoProfile', '-File', 'scripts/package.ps1',
                        '-OutputDirectory', 'dist-release', '-SkipBuild'], check=True, cwd=ROOT)
        bundle = ROOT / 'dist-release' / ('sniplet-' + TARGETS[target])
        executable = bundle / 'Sniplet.exe'
    else:
        subprocess.run(['sh', 'scripts/package.sh', 'dist-release'], check=True, cwd=ROOT,
                       env={**os.environ, 'SNIPLET_PACKAGE_SKIP_BUILD': '1',
                            'SNIPLET_PACKAGE_PROFILE': 'release'})
        bundle = ROOT / 'dist-release' / ('Sniplet.app' if mac else 'sniplet-' + TARGETS[target])
        executable = bundle / ('Contents/MacOS/Sniplet' if mac else 'bin/sniplet')
    identity = json.loads(subprocess.check_output([str(executable), '--build-info'], text=True, timeout=15))
    require(identity == {'version': build.version, 'sha': build.sha},
            'The executable was not built with this release identity')
    directory = ROOT / 'dist-release' / 'assets'
    directory.mkdir(parents=True, exist_ok=True)
    output = ROOT / 'dist-release' / 'velopack'
    shutil.rmtree(output, ignore_errors=True)
    downloads = {}
    if target == 'windows-x64':
        # Release builds sign with Azure Artifact Signing (docs/releases.md).
        signing = os.environ.get('SNIPLET_AZURE_SIGNING')
        made = vpk_pack(target, build, bundle, 'Sniplet.exe', output,
                        '--icon', str(ROOT / 'assets/icons/sniplet.ico'),
                        *(['--azureTrustedSignFile', str(ROOT / signing)] if signing else []))
        downloads = {ending: made(ending) for ending in DOWNLOADS[target]}
    elif mac:
        # vpk adds its updater to the bundle and signs it again, with hardened runtime.
        # With a notary profile (release.yml), vpk also notarizes and staples the app.
        identity = os.environ.get('SNIPLET_SIGNING_IDENTITY', '-')
        notary = os.environ.get('SNIPLET_NOTARY_PROFILE')
        keychain = os.environ.get('SNIPLET_MACOS_KEYCHAIN')
        made = vpk_pack(target, build, bundle, 'Sniplet', output, '--noInst',
                        '--signAppIdentity', identity,
                        '--signEntitlements', str(ROOT / 'packaging/macos/Sniplet.entitlements'),
                        *(['--notaryProfile', notary] if notary else []),
                        *(['--keychain', keychain] if keychain else []))
        import macos_dmg
        unpacked = Path(tempfile.mkdtemp(prefix='sniplet-portable-', dir=output))
        subprocess.run(['ditto', '-x', '-k', str(made('-Portable.zip')), str(unpacked)], check=True)
        app = unpacked / 'Sniplet.app'
        if notary:
            # Gatekeeper must accept the app that the update package and the DMG carry.
            subprocess.run(['spctl', '--assess', '--type', 'execute', '-vv', str(app)], check=True)
        image = macos_dmg.create(app, directory / build.filename(target, '.dmg'))
        if notary:
            macos_dmg.notarize(image, identity, notary, keychain)
    else:
        # vpk wants the executable at the top of its folder.
        app_dir = ROOT / 'dist-release' / 'appimage'
        shutil.rmtree(app_dir, ignore_errors=True)
        shutil.copytree(bundle, app_dir, ignore=shutil.ignore_patterns('bin'))
        shutil.copy2(executable, app_dir / 'sniplet')
        made = vpk_pack(target, build, app_dir, 'sniplet', output,
                        '--icon', str(ROOT / 'assets/icons/sniplet-512.png'))
        downloads = {'.AppImage': made('.AppImage')}
        archive_linux(bundle, directory / build.filename(target, '.tar.gz'))
    (output / f'{PACK_ID}-{build.version}-{target}{UPDATE}').replace(directory / build.filename(target))
    for ending, source in downloads.items():
        source.replace(directory / build.filename(target, ending))
    names = [(build.filename(target), 'update')]
    names += [(build.filename(target, ending), 'download') for ending in DOWNLOADS[target]]
    assets = [Asset(target, name, checksum(directory / name), (directory / name).stat().st_size, kind)
              for name, kind in names]
    write_json(directory / (target + '.json'), {**asdict(build), 'assets': [asdict(a) for a in assets]})


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
        """Find a release by tag, including drafts, which the tag endpoint does not return."""
        release = self.api('releases/tags/' + tag)
        if release is not None:
            return release
        page = 1
        while True:
            releases = self.api(f'releases?per_page=100&page={page}')
            require(releases is not None, 'Could not list releases')
            match = next((item for item in releases if item['tag_name'] == tag), None)
            if match is not None or len(releases) < 100:
                return match
            page += 1

    def tag_sha(self, tag):
        ref = self.api('git/ref/tags/' + tag)
        if ref is None:
            return None
        obj = ref['object']
        while obj['type'] == 'tag':
            obj = self.api('git/tags/' + obj['sha'])['object']
        require(obj['type'] == 'commit', 'Release tag must point to a commit')
        return obj['sha']

    def upload(self, tag, files):
        subprocess.run(['gh', 'release', 'upload', tag, '--repo', self.repo, '--clobber',
                        *map(str, files)], check=True)


def collect(directory, build):
    """Read the packages that each platform job made, and check they came from one build."""
    assets = []
    for target in TARGETS:
        data = json.loads((directory / (target + '.json')).read_text())
        require(Build(data['version'], data['sha']) == build, 'Platform packages came from different builds')
        target_assets = [Asset(**item) for item in data['assets']]
        require(all(asset.target == target for asset in target_assets), 'Package metadata has the wrong platform')
        assets.extend(target_assets)
    return assets


def publish(github, build, directory):
    """Upload into a draft, verify it, then publish. Re-running resumes an unfinished draft.
    A published release is never changed. A stable tag goes up only after every OS built."""
    assets = collect(directory, build)
    write_metadata(directory, build, assets)
    files = [directory / asset.name for asset in assets]
    files += [directory / name for name in ('release.json', 'release.json.sig', 'SHA256SUMS')]
    tag_sha = github.tag_sha(build.tag)
    require(tag_sha in (None, build.sha), f'Tag {build.tag} already points to another commit')
    release = github.release(build.tag)
    require(release is None or release['draft'], f'{build.tag} is already published')
    if release:
        require({item['name'] for item in release['assets']} <= {path.name for path in files},
                'Draft contains unexpected assets; inspect it before retrying')
    if tag_sha is None:
        github.api('git/refs', 'POST', {'ref': 'refs/tags/' + build.tag, 'sha': build.sha})
    nightly = build.channel == 'nightly'
    title = f'Sniplet {"Nightly " if nightly else ""}{build.version}'
    notes = (f'Source commit: `{build.sha}`.\n\n'
             'Packages: macOS Apple Silicon and Intel, Windows x64, and Linux x64.\n'
             'Use SHA256SUMS to check the downloads. '
             'Linux packages need the native libraries listed in the README.\n\n'
             'Mac: open the DMG, drag Sniplet to Applications, then eject the disk image.\n'
             'Windows: run the Setup file, or unzip the Portable file into a folder.\n'
             'Linux: make the AppImage executable and run it. The tar.gz does not update itself.\n'
             'The .nupkg files are for in-app updates.\n')
    if release is None:
        release = github.api('releases', 'POST', {'tag_name': build.tag, 'target_commitish': build.sha,
                             'name': title, 'body': notes, 'draft': True, 'prerelease': nightly,
                             'make_latest': 'false'})
    github.upload(build.tag, files)
    uploaded = github.api('releases/' + str(release['id']))
    require(uploaded is not None and uploaded['draft'], 'Release changed during upload; publication stopped')
    require({item['name'] for item in uploaded['assets']} == {path.name for path in files},
            'Draft upload is incomplete')
    for path in files:
        remote = next(item for item in uploaded['assets'] if item['name'] == path.name)
        require(remote['size'] == path.stat().st_size and remote.get('digest') == 'sha256:' + checksum(path),
                'Uploaded asset verification failed')
    require(github.tag_sha(build.tag) == build.sha, 'Release tag changed during upload')
    github.api('releases/' + str(release['id']), 'PATCH', {
        'draft': False, 'prerelease': nightly, 'name': title, 'body': notes,
        'make_latest': 'false' if nightly else 'true',
    })
    print(f'Published {build.tag}')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest='command', required=True)
    commands.add_parser('plan')
    for name in ('package', 'publish'):
        command = commands.add_parser(name)
        if name == 'package':
            command.add_argument('target', choices=TARGETS)
        command.add_argument('--version', required=True)
        command.add_argument('--sha', required=True)
    args = parser.parse_args()
    if args.command == 'plan':
        plan()
    elif args.command == 'package':
        package(args.target, Build(args.version, args.sha))
    else:
        publish(GitHub(), Build(args.version, args.sha), ROOT / 'dist-release/assets')


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, TypeError, OSError, subprocess.SubprocessError) as error:
        raise SystemExit(f'Release failed: {error}') from error
