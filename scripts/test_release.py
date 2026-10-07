"""Release checks use temporary packages and a local GitHub stand-in."""

from contextlib import redirect_stdout
import copy
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import tomllib
import unittest
from unittest.mock import patch
from urllib.parse import parse_qs

import release

SHA = 'a1b2c3d' + '0' * 33


class FakeGitHub(release.GitHub):
    def __init__(self):
        self.tags = {}
        self.releases = {}
        self.files = {}
        self.writes = []
        self.uploads = 0

    def release_snapshot(self, tag):
        if tag not in self.releases:
            return None
        value = copy.deepcopy(self.releases[tag])
        value['assets'] = [{'name': name, 'size': len(data),
                            'digest': 'sha256:' + hashlib.sha256(data).hexdigest()}
                           for name, data in self.files[tag].items()]
        return value

    def tag_sha(self, tag):
        return self.tags.get(tag)

    def api(self, endpoint, method='GET', data=None):
        if method == 'GET':
            if endpoint.startswith('releases/tags/'):
                value = self.release_snapshot(endpoint.removeprefix('releases/tags/'))
                return None if value and value['draft'] else value
            path, _, query = endpoint.partition('?')
            if path == 'releases':
                parameters = parse_qs(query)
                page = int(parameters.get('page', ['1'])[0])
                per_page = int(parameters.get('per_page', ['30'])[0])
                start = (page - 1) * per_page
                tags = list(reversed(self.releases))[start:start + per_page]
                return [self.release_snapshot(tag) for tag in tags]
            if path.startswith('releases/'):
                release_id = int(path.removeprefix('releases/'))
                tag = next((tag for tag, value in self.releases.items()
                            if value['id'] == release_id), None)
                return self.release_snapshot(tag)
            raise AssertionError(endpoint)
        self.writes.append((endpoint, method, data))
        if endpoint == 'git/refs':
            tag = data['ref'].removeprefix('refs/tags/')
            assert tag not in self.tags
            self.tags[tag] = data['sha']
        elif endpoint == 'releases':
            tag = data['tag_name']
            assert tag not in self.releases, 'Release already exists'
            self.releases[tag] = {'id': len(self.releases) + 1, **data}
            self.files[tag] = {}
            return self.release_snapshot(tag)
        elif endpoint.startswith('releases/'):
            value = next(item for item in self.releases.values()
                         if item['id'] == int(endpoint.split('/')[-1]))
            value.update(data)
            return copy.deepcopy(value)
        else:
            raise AssertionError(endpoint)

    def upload(self, tag, files):
        assert self.releases[tag]['draft'], 'Published files must not be replaced'
        self.uploads += 1
        self.files[tag].update({path.name: path.read_bytes() for path in files})

    def download(self, tag, directory, patterns):
        for name in patterns:
            (directory / name).write_bytes(self.files[tag][name])


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        temporary = self.enterContext(tempfile.TemporaryDirectory())
        self.root = Path(temporary)
        self.github = FakeGitHub()
        self.enterContext(redirect_stdout(io.StringIO()))
        self.enterContext(patch.dict('os.environ', {'GITHUB_OUTPUT': ''}))

    def packages(self, build, name=None):
        directory = self.root / (name or build.tag)
        directory.mkdir()
        assets = []
        for target in release.TARGETS:
            path = directory / build.filename(target)
            path.write_bytes((target + '\n' + build.sha).encode())
            assets.append(release.Asset(target, path.name, release.checksum(path), path.stat().st_size))
        release.write_metadata(directory, build, assets)
        return directory, assets

    def nightly(self, sha=SHA):
        build = release.Build('0.2.0', sha)
        directory, assets = self.packages(build)
        release.publish(self.github, build, assets, directory)
        return build, directory, assets

    def test_version_bump_updates_workspace_and_own_lock_entries_only(self):
        (self.root / 'Cargo.toml').write_text('[workspace.package]\nversion = "1.2.3"\n')
        (self.root / 'Cargo.lock').write_text(''.join(
            f'[[package]]\nname = "{name}"\nversion = "1.2.3"\n\n'
            for name in ['sniplet-app', 'sniplet-core', 'sniplet-platform', 'some-dependency']))
        self.assertEqual(release.bump('minor', root=self.root), '1.3.0')
        packages = tomllib.loads((self.root / 'Cargo.lock').read_text())['package']
        self.assertEqual([item['version'] for item in packages], ['1.3.0'] * 3 + ['1.2.3'])
        self.assertEqual(release.bump('major', root=self.root), '2.0.0')
        self.assertEqual(release.bump('patch', root=self.root), '2.0.1')
        self.assertEqual(release.bump('set', '3.4.5', root=self.root), '3.4.5')
        for invalid in ['3.4.5', '1.0.0', '4.0.0-beta', '04.0.0', '$(false)']:
            with self.assertRaises(ValueError):
                release.bump('set', invalid, root=self.root)

    def test_bump_does_not_write_when_lockfile_is_inconsistent(self):
        cargo = self.root / 'Cargo.toml'
        cargo.write_text('[workspace.package]\nversion = "0.1.0"\n')
        (self.root / 'Cargo.lock').write_text('')
        with self.assertRaises(ValueError):
            release.bump('patch', root=self.root)
        self.assertEqual(release.version(self.root), '0.1.0')

    def test_promotion_keeps_every_package_byte_and_removes_sha_from_names(self):
        build, _, assets = self.nightly()
        release.promote(self.github, build.tag)
        stable = release.Build(build.version, build.sha, 'stable')
        self.assertEqual(self.github.tags[stable.tag], build.sha)
        self.assertFalse(self.github.release(stable.tag)['prerelease'])
        for asset in assets:
            self.assertEqual(self.github.files[stable.tag][stable.filename(asset.target)],
                             self.github.files[build.tag][asset.name])
        metadata = json.loads(self.github.files[stable.tag]['release.json'])
        self.assertEqual(metadata['source_nightly'], build.tag)
        self.assertEqual(metadata['version'], '0.2.0')
        self.assertEqual(self.github.releases[stable.tag]['make_latest'], 'legacy')
        uploads = self.github.uploads
        release.promote(self.github, build.tag)
        self.assertEqual(self.github.uploads, uploads)

    def test_completed_nightly_is_reused_and_prepare_skips_builds(self):
        build, directory, assets = self.nightly()
        before = copy.deepcopy(self.github.files)
        release.publish(self.github, build, assets, directory)
        self.assertEqual(self.github.uploads, 1)
        self.assertEqual(self.github.files, before)
        with patch.object(release, 'version', return_value=build.version), \
                patch.object(release.subprocess, 'check_output', return_value=build.sha), \
                patch.object(release, 'output') as output:
            release.prepare(self.github)
        self.assertFalse(output.call_args.kwargs['build'])

    def test_partial_draft_upload_can_resume(self):
        build = release.Build('0.2.0', SHA)
        directory, assets = self.packages(build)
        upload = self.github.upload

        def interrupted(tag, files):
            upload(tag, files[:1])
            raise OSError('Interrupted upload')

        with patch.object(self.github, 'upload', side_effect=interrupted):
            with self.assertRaises(OSError):
                release.publish(self.github, build, assets, directory)
        self.assertTrue(self.github.release(build.tag)['draft'])
        release.publish(self.github, build, assets, directory)
        self.assertFalse(self.github.release(build.tag)['draft'])
        self.assertEqual(len(self.github.files[build.tag]), 6)

    def test_new_draft_is_verified_and_published(self):
        build, _, _ = self.nightly()
        self.assertFalse(self.github.release(build.tag)['draft'])
        self.assertEqual(len(self.github.files[build.tag]), 6)

    def test_draft_after_first_release_page_can_resume(self):
        build = release.Build('0.2.0', SHA)
        directory, assets = self.packages(build)
        self.github.tags[build.tag] = build.sha
        draft = self.github.api('releases', 'POST', {
            'tag_name': build.tag, 'draft': True, 'prerelease': True,
        })
        for index in range(100):
            self.github.api('releases', 'POST', {
                'tag_name': f'v0.0.{index}', 'draft': False,
            })
        release.publish(self.github, build, assets, directory)
        published = self.github.release(build.tag)
        self.assertEqual(published['id'], draft['id'])
        self.assertFalse(published['draft'])
        self.assertEqual(len(published['assets']), 6)
        self.assertEqual(len(self.github.releases), 101)

    def test_another_commit_gets_another_nightly_but_cannot_replace_stable(self):
        build, _, _ = self.nightly()
        release.promote(self.github, build.tag)
        other, _, _ = self.nightly('b1b2c3d' + '1' * 33)
        before = copy.deepcopy(self.github.files['v0.2.0'])
        with self.assertRaisesRegex(ValueError, 'another commit'):
            release.promote(self.github, other.tag)
        self.assertEqual(self.github.files['v0.2.0'], before)

    def test_short_sha_collision_is_rejected(self):
        build, _, _ = self.nightly()
        collision = release.Build(build.version, build.sha[:7] + 'f' * 33)
        directory, assets = self.packages(collision, 'collision')
        with self.assertRaisesRegex(ValueError, 'another commit'):
            release.publish(self.github, collision, assets, directory)

    def test_missing_or_changed_nightly_package_stops_promotion(self):
        build, _, assets = self.nightly()
        self.github.files[build.tag][assets[0].name] = b'changed'
        with self.assertRaises(ValueError):
            release.promote(self.github, build.tag)
        del self.github.files[build.tag][assets[0].name]
        with self.assertRaises(ValueError):
            release.promote(self.github, build.tag)
        self.assertNotIn('v0.2.0', self.github.tags)

    def test_inconsistent_manifest_or_checksums_stops_promotion(self):
        build, _, _ = self.nightly()
        self.github.files[build.tag]['SHA256SUMS'] = b'wrong checksums\n'
        with self.assertRaisesRegex(ValueError, 'checksums'):
            release.promote(self.github, build.tag)
        metadata = json.loads(self.github.files[build.tag]['release.json'])
        metadata['sha'] = 'b' * 40
        self.github.files[build.tag]['release.json'] = json.dumps(metadata).encode()
        with self.assertRaises(ValueError):
            release.promote(self.github, build.tag)
        self.assertNotIn('v0.2.0', self.github.tags)

    def test_missing_platform_and_unsafe_filenames_are_rejected(self):
        build, directory, assets = self.nightly()
        with self.assertRaises(ValueError):
            release.write_metadata(directory, build, assets[:-1])
        bad = release.Asset(assets[0].target, '../outside.zip', assets[0].sha256, assets[0].size)
        with self.assertRaises(ValueError):
            release.write_metadata(directory, build, [bad, *assets[1:]])

    def test_linux_archive_keeps_execute_permission(self):
        bundle = self.root / 'portable'
        (bundle / 'bin').mkdir(parents=True)
        binary = bundle / 'bin/sniplet'
        binary.write_bytes(b'fixture')
        binary.chmod(0o755)
        destination = self.root / 'package.tar.gz'
        release.archive_bundle(bundle, destination, 'linux-x64')
        with tarfile.open(destination) as archive:
            self.assertEqual(archive.getmember('sniplet/bin/sniplet').mode, 0o755)

    def test_github_permission_and_server_failures_are_not_missing_releases(self):
        with patch.dict('os.environ', {'GITHUB_REPOSITORY': 'owner/repo'}):
            github = release.GitHub()
        for code in [403, 500]:
            response = subprocess.CompletedProcess([], 1, '', f'gh: error (HTTP {code})')
            with patch.object(release.subprocess, 'run', return_value=response):
                with self.assertRaises(ValueError):
                    github.release('v0.2.0')
        response = subprocess.CompletedProcess([], 1, '', 'gh: Not Found (HTTP 404)')
        empty = subprocess.CompletedProcess([], 0, '[]', '')
        with patch.object(release.subprocess, 'run', side_effect=[response, empty]):
            self.assertIsNone(github.release('v0.2.0'))
        with patch.object(release.subprocess, 'run', return_value=response):
            with self.assertRaises(ValueError):
                github.api('releases/1', 'PATCH', {'draft': False})


if __name__ == '__main__':
    unittest.main()
