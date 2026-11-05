"""Release checks use temporary git repositories, packages, and a local GitHub stand-in."""

from contextlib import redirect_stdout
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
from urllib.parse import parse_qs

import release

SHA = 'a1b2c3d' + '0' * 33
NIGHTLY = '0.2.1-nightly.20261104.17'


class FakeGitHub(release.GitHub):
    def __init__(self):
        self.tags = {}
        self.releases = {}
        self.files = {}
        self.writes = []

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
            release_id = int(path.removeprefix('releases/'))
            return self.release_snapshot(next(
                (tag for tag, value in self.releases.items() if value['id'] == release_id), None))
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
        else:
            value = next(item for item in self.releases.values()
                         if item['id'] == int(endpoint.split('/')[-1]))
            value.update(data)
            return copy.deepcopy(value)

    def upload(self, tag, files):
        assert self.releases[tag]['draft'], 'Published files must not be replaced'
        self.files[tag].update({path.name: path.read_bytes() for path in files})


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        temporary = self.enterContext(tempfile.TemporaryDirectory())
        self.root = Path(temporary)
        self.github = FakeGitHub()
        self.enterContext(redirect_stdout(io.StringIO()))
        self.enterContext(patch.dict('os.environ', {'GITHUB_OUTPUT': ''}))

    # --- Plan -------------------------------------------------------------

    def repository(self):
        """A main checkout with a stable tag, and a bare origin that receives pushed tags."""
        origin = self.root / 'origin.git'
        checkout = self.root / 'checkout'
        subprocess.run(['git', 'init', '-q', '--bare', str(origin)], check=True)
        subprocess.run(['git', 'init', '-q', '-b', 'main', str(checkout)], check=True)
        self.git = lambda *args: release.git('-c', 'user.name=t', '-c', 'user.email=t@t', *args,
                                             root=checkout)
        self.git('remote', 'add', 'origin', str(origin))
        self.commit()
        self.git('tag', 'v0.2.0')
        self.origin = origin
        return checkout

    def commit(self):
        self.git('commit', '-q', '--allow-empty', '-m', 'change')
        return self.git('rev-parse', 'HEAD')

    def plan(self, checkout, now=None, **values):
        env = {'GITHUB_REF': 'refs/heads/main', 'GITHUB_EVENT_NAME': 'workflow_dispatch',
               'GITHUB_RUN_NUMBER': '17', 'GITHUB_RUN_ID': '900', 'GITHUB_REPOSITORY': 'o/r',
               'GITHUB_SERVER_URL': 'https://github.com', **values}
        outputs = {}
        with patch.object(release, 'output', side_effect=outputs.update):
            release.plan(env, checkout, now)
        return outputs

    def test_nightly_is_the_next_patch_and_pushes_a_tag_with_its_run(self):
        checkout = self.repository()
        sha = self.commit()
        now = 1793793600  # 2026-11-04 12:00 UTC
        planned = self.plan(checkout, now)
        self.assertEqual(planned, {'channel': 'nightly', 'version': NIGHTLY, 'sha': sha, 'skip': False})
        pushed = release.git('rev-list', '-n', '1', 'v' + NIGHTLY, root=self.origin)
        self.assertEqual(pushed, sha)
        body = self.git('tag', '-l', '--format=%(contents:body)', 'v' + NIGHTLY)
        self.assertEqual(body.strip(), 'https://github.com/o/r/actions/runs/900')

    def test_scheduled_nightly_waits_for_six_hours_and_new_commits(self):
        checkout = self.repository()
        self.plan(checkout, 1793793600)  # 2026-11-04 12:00 UTC, the date in NIGHTLY
        created = release.release_tags(checkout)[NIGHTLY]
        schedule = {'GITHUB_EVENT_NAME': 'schedule', 'GITHUB_RUN_NUMBER': '18'}
        self.assertEqual(self.plan(checkout, created + 5 * 3600, **schedule), {'skip': True})
        self.assertEqual(self.plan(checkout, created + 7 * 3600, **schedule), {'skip': True})
        self.commit()
        self.assertEqual(self.plan(checkout, created + 5 * 3600, **schedule), {'skip': True})
        planned = self.plan(checkout, created + 7 * 3600, **schedule)
        self.assertRegex(planned['version'], r'^0\.2\.1-nightly\.[0-9]{8}\.18$')
        # A nightly started by hand always builds.
        self.assertFalse(self.plan(checkout, created + 7 * 3600, GITHUB_RUN_NUMBER='19')['skip'])

    def test_stable_rebuilds_the_newest_passing_nightly(self):
        checkout = self.repository()
        first = self.commit()
        self.plan(checkout, GITHUB_RUN_NUMBER='17', GITHUB_RUN_ID='1')
        self.commit()
        self.plan(checkout, GITHUB_RUN_NUMBER='18', GITHUB_RUN_ID='2')
        with patch.object(release, 'run_passed', side_effect=lambda run: run == '1'):
            planned = self.plan(checkout, CHANNEL='stable')
            self.assertEqual(planned, {'channel': 'stable', 'version': '0.2.1', 'sha': first, 'skip': False})
            self.assertEqual(self.plan(checkout, CHANNEL='stable', RELEASE_VERSION='0.3.0')['version'], '0.3.0')
            for version, error in [('0.2.0', 'already exists'), ('0.1.9', 'not newer'), ('0.3', 'X.Y.Z')]:
                with self.assertRaisesRegex(ValueError, error):
                    self.plan(checkout, CHANNEL='stable', RELEASE_VERSION=version)
        with patch.object(release, 'run_passed', return_value=False):
            with self.assertRaisesRegex(ValueError, 'No nightly has passed'):
                self.plan(checkout, CHANNEL='stable')
        with self.assertRaisesRegex(ValueError, 'from main'):
            self.plan(checkout, GITHUB_REF='refs/heads/feature')

    def test_versions_sort_nightlies_between_stables(self):
        order = ['0.2.0', '0.2.1-nightly.20261104.9', '0.2.1-nightly.20261104.10',
                 '0.2.1-nightly.20261105.1', '0.2.1', '0.10.0']
        self.assertEqual(sorted(reversed(order), key=release.version_key), order)
        for invalid in ['0.2', '01.2.3', '0.2.1-a1b2c3d', '0.2.1-nightly.2026.1', '$(false)']:
            self.assertIsNone(release.version_key(invalid))

    # --- Publish ----------------------------------------------------------

    def signing_key(self):
        import update_signing
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
        key = Ed25519PrivateKey.generate()
        public = self.root / 'public.hex'
        public.write_text(key.public_key().public_bytes_raw().hex())
        self.enterContext(patch.object(update_signing, 'PUBLIC_KEY', public))
        private = key.private_bytes(serialization.Encoding.PEM,
                                    serialization.PrivateFormat.PKCS8,
                                    serialization.NoEncryption()).decode()
        self.enterContext(patch.dict('os.environ', {'UPDATER_SIGNING_KEY': private}))

    def packages(self, build):
        """Write what the four platform jobs upload: packages plus one metadata file each."""
        self.signing_key()
        directory = self.root / build.tag
        directory.mkdir()
        for target in release.TARGETS:
            kinds = ['update', 'installer'] if target.startswith('macos-') else ['update']
            assets = []
            for kind in kinds:
                path = directory / build.filename(target, kind)
                path.write_bytes(f'{target} {kind} {build.sha}'.encode())
                assets.append(release.Asset(target, path.name, release.checksum(path),
                                            path.stat().st_size, kind))
            release.write_json(directory / (target + '.json'),
                               {**release.asdict(build), 'assets': [release.asdict(a) for a in assets]})
        return directory

    def test_nightly_publishes_a_signed_prerelease_on_its_existing_tag(self):
        import update_signing
        build = release.Build(NIGHTLY, SHA)
        self.github.tags[build.tag] = SHA
        directory = self.packages(build)
        release.publish(self.github, build, directory)
        published = self.github.release(build.tag)
        self.assertFalse(published['draft'])
        self.assertTrue(published['prerelease'])
        self.assertEqual(published['make_latest'], 'false')
        self.assertEqual(len(self.github.files[build.tag]), 9)
        self.assertNotIn('git/refs', [write[0] for write in self.github.writes])
        metadata = json.loads(self.github.files[build.tag]['release.json'])
        self.assertEqual((metadata['schema'], metadata['channel'], metadata['tag']), (3, 'nightly', build.tag))
        self.assertEqual(len(metadata['assets']), 4)
        self.assertEqual(len(metadata['installers']), 2)
        self.assertEqual(metadata['assets'][0]['name'], f'Sniplet-{NIGHTLY}-linux-x64.tar.gz')
        update_signing.verify(directory / 'release.json')
        with self.assertRaisesRegex(ValueError, 'already published'):
            release.publish(self.github, build, directory)

    def test_stable_tag_goes_up_with_the_release(self):
        build = release.Build('0.2.1', SHA)
        release.publish(self.github, build, self.packages(build))
        self.assertEqual(self.github.tags['v0.2.1'], SHA)
        published = self.github.release('v0.2.1')
        self.assertFalse(published['prerelease'])
        self.assertEqual(published['make_latest'], 'true')

    def test_partial_draft_upload_can_resume(self):
        build = release.Build('0.2.1', SHA)
        directory = self.packages(build)
        upload = self.github.upload

        def interrupted(tag, files):
            upload(tag, files[:1])
            raise OSError('Interrupted upload')

        with patch.object(self.github, 'upload', side_effect=interrupted):
            with self.assertRaises(OSError):
                release.publish(self.github, build, directory)
        self.assertTrue(self.github.release(build.tag)['draft'])
        release.publish(self.github, build, directory)
        self.assertFalse(self.github.release(build.tag)['draft'])
        self.assertEqual(len(self.github.files[build.tag]), 9)

    def test_draft_after_first_release_page_can_resume(self):
        build = release.Build('0.2.1', SHA)
        directory = self.packages(build)
        draft = self.github.api('releases', 'POST', {'tag_name': build.tag, 'draft': True})
        for index in range(100):
            self.github.api('releases', 'POST', {'tag_name': f'v0.0.{index}', 'draft': False})
        release.publish(self.github, build, directory)
        published = self.github.release(build.tag)
        self.assertEqual(published['id'], draft['id'])
        self.assertFalse(published['draft'])

    def test_tag_on_another_commit_stops_before_any_write(self):
        build = release.Build('0.2.1', SHA)
        self.github.tags[build.tag] = 'b' * 40
        with self.assertRaisesRegex(ValueError, 'another commit'):
            release.publish(self.github, build, self.packages(build))
        self.assertEqual(self.github.writes, [])

    def test_missing_platforms_disk_images_and_unsafe_names_are_rejected(self):
        build = release.Build('0.2.1', SHA)
        directory = self.packages(build)
        assets = release.collect(directory, build)
        updates = [a for a in assets if a.kind == 'update']
        installers = [a for a in assets if a.kind == 'installer']
        with self.assertRaisesRegex(ValueError, 'four platform'):
            release.manifest(build, updates[:-1] + installers)
        with self.assertRaisesRegex(ValueError, 'Both Mac'):
            release.manifest(build, updates + installers[:-1])
        bad = release.Asset(updates[0].target, '../outside.zip', updates[0].sha256, updates[0].size)
        with self.assertRaisesRegex(ValueError, 'filename'):
            release.manifest(build, [bad, *updates[1:], *installers])
        other = release.Build('0.2.1', 'b' * 40)
        with self.assertRaisesRegex(ValueError, 'different builds'):
            release.collect(directory, other)

    def test_linux_archive_keeps_execute_permission(self):
        if os.name == 'nt':
            self.skipTest('Windows does not store Unix execute bits')
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

    def test_key_setup_creates_a_matching_pair_and_refuses_replacement(self):
        import update_signing
        from cryptography.hazmat.primitives import serialization
        private = self.root / 'private' / 'update.pem'
        public = self.root / 'update-public.hex'
        with patch.object(update_signing, 'PUBLIC_KEY', public):
            update_signing.initialize(private, False)
            key = serialization.load_pem_private_key(private.read_bytes(), password=None)
            self.assertEqual(public.read_text().strip(), key.public_key().public_bytes_raw().hex())
            with self.assertRaisesRegex(ValueError, 'already exists'):
                update_signing.initialize(private, False)
            backup = self.root / 'backup' / 'update.pem'
            update_signing.backup(private, backup)
            copied = serialization.load_pem_private_key(backup.read_bytes(), password=None)
            self.assertEqual(copied.public_key().public_bytes_raw(), key.public_key().public_bytes_raw())
            with self.assertRaises(FileExistsError):
                update_signing.backup(private, backup)
            with self.assertRaisesRegex(ValueError, 'outside the repository'):
                update_signing.backup(private, release.ROOT / 'private-key.pem')

    @unittest.skipUnless(sys.platform == 'darwin', 'Disk images require macOS')
    def test_mac_disk_image_can_install_the_signed_app_into_applications(self):
        import macos_dmg
        bundle = self.root / 'source/Sniplet.app'
        binary = bundle / 'Contents/MacOS/Sniplet'
        binary.parent.mkdir(parents=True)
        shutil.copyfile('/usr/bin/true', binary)
        binary.chmod(0o755)
        resources = bundle / 'Contents/Resources'
        resources.mkdir()
        (resources / 'manual.txt').write_text('Bundle resource fixture')
        (resources / 'manual-link').symlink_to('manual.txt')
        (bundle / 'Contents/Info.plist').write_bytes(plistlib.dumps({
            'CFBundleExecutable': 'Sniplet', 'CFBundleIdentifier': 'io.github.m4tta.sniplet.dmg-test',
            'CFBundlePackageType': 'APPL', 'CFBundleVersion': '1',
        }))
        subprocess.run(['codesign', '--force', '--timestamp=none', '--sign', '-', str(bundle)], check=True)
        image = macos_dmg.create(bundle, self.root / 'Sniplet.dmg')
        mount = self.root / 'mount'
        mount.mkdir()
        subprocess.run(['hdiutil', 'attach', '-readonly', '-nobrowse', '-noautoopen',
                        '-mountpoint', str(mount), str(image)], check=True)
        try:
            self.assertEqual(os.readlink(mount / 'Applications'), '/Applications')
            installed = self.root / 'Applications/Sniplet.app'
            subprocess.run(['ditto', str(mount / 'Sniplet.app'), str(installed)], check=True)
            subprocess.run(['codesign', '--verify', '--deep', '--strict', str(installed)], check=True)
            self.assertEqual((installed / 'Contents/Resources/manual.txt').read_text(),
                             'Bundle resource fixture')
            self.assertEqual(os.readlink(installed / 'Contents/Resources/manual-link'), 'manual.txt')
            subprocess.run([str(installed / 'Contents/MacOS/Sniplet')], check=True)
        finally:
            subprocess.run(['hdiutil', 'detach', str(mount)], check=True)


if __name__ == '__main__':
    unittest.main()
