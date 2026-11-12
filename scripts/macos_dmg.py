#!/usr/bin/env python3
"""Create and check a drag-to-Applications disk image on macOS."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ICON = Path(__file__).resolve().parent.parent / 'assets/icons/sniplet.icns'


def layout(app_name):
    """Finder view for the mounted image: a small window with the app beside its Applications target."""
    return {
        'format': 'UDZO',
        'filesystem': 'HFS+',
        'symlinks': {'Applications': '/Applications'},
        'icon': str(ICON),
        'window_rect': ((200, 200), (540, 340)),
        'icon_size': 128,
        'text_size': 13,
        'icon_locations': {app_name: (140, 150), 'Applications': (400, 150)},
    }


def detach(mount):
    """Unmount a checked image. macOS can scan a signed app on a new mount and
    keep the disk busy for a few seconds, so wait for that before forcing."""
    for _ in range(5):
        if subprocess.run(['hdiutil', 'detach', str(mount)]).returncode == 0:
            return
        time.sleep(3)
    subprocess.run(['hdiutil', 'detach', '-force', str(mount)], check=True)


def create(bundle, destination):
    if sys.platform != 'darwin':
        raise ValueError('Mac disk images must be created on macOS')
    bundle = Path(bundle).resolve()
    destination = Path(destination).absolute()
    binary = Path('Contents/MacOS/Sniplet')
    if bundle.suffix != '.app' or not (bundle / binary).is_file():
        raise ValueError('Select a complete Sniplet.app bundle')
    if destination.suffix != '.dmg':
        raise ValueError('The output filename must end in .dmg')
    if destination.resolve().is_relative_to(bundle):
        raise ValueError('The disk image must be outside the signed app bundle')
    destination.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(bundle)], check=True)
    try:
        from dmgbuild.core import DMGError, build_dmg
    except ImportError as error:
        raise ValueError('Install the Mac packaging tools: '
                         'python3 -m pip install -r scripts/requirements-release.txt') from error
    with tempfile.TemporaryDirectory(prefix='.sniplet-dmg-', dir=destination.parent) as temporary:
        root = Path(temporary)
        image = root / 'Sniplet.dmg'
        try:
            build_dmg(str(image), 'Sniplet', settings={'files': [str(bundle)], **layout(bundle.name)})
        except DMGError as error:
            raise ValueError(f'dmgbuild failed: {error}') from error
        subprocess.run(['hdiutil', 'verify', str(image)], check=True)
        mount = root / 'mounted'
        mount.mkdir()
        subprocess.run(['hdiutil', 'attach', '-readonly', '-nobrowse', '-noautoopen',
                        '-mountpoint', str(mount), str(image)], check=True)
        try:
            if not (os.statvfs(mount).f_flag & os.ST_RDONLY):
                raise ValueError('The disk image is not read-only')
            if not (mount / 'Applications').is_symlink() or os.readlink(mount / 'Applications') != '/Applications':
                raise ValueError('The disk image has no Applications shortcut')
            if not (mount / '.DS_Store').is_file():
                raise ValueError('The disk image has no Finder window layout')
            installed = mount / bundle.name
            subprocess.run(['codesign', '--verify', '--deep', '--strict', str(installed)], check=True)
            for path in [binary, Path('Contents/Info.plist')]:
                if (bundle / path).read_bytes() != (installed / path).read_bytes():
                    raise ValueError('The disk image changed the app bundle')
        finally:
            detach(mount)
        # Replace the output only after the new image passes all checks.
        image.replace(destination)
    print(f'Created and verified Mac disk image: {destination}')
    return destination


def notarize(image, identity, profile, keychain=None):
    """Sign the disk image with Developer ID, notarize it with Apple, and staple the ticket.

    `profile` is a `notarytool store-credentials` profile. The stapled ticket lets
    Gatekeeper accept the first open without a network check.
    """
    keychain = ['--keychain', str(keychain)] if keychain else []
    subprocess.run(['codesign', '--force', '--timestamp', '--sign', identity, *keychain, str(image)], check=True)
    result = subprocess.run(['xcrun', 'notarytool', 'submit', str(image), '--keychain-profile', profile,
                             *keychain, '--wait', '--output-format', 'json'],
                            capture_output=True, text=True)
    try:
        status = json.loads(result.stdout)
    except json.JSONDecodeError:
        raise ValueError(f'notarytool failed: {result.stdout}{result.stderr}') from None
    if status.get('status') != 'Accepted':
        log = subprocess.run(['xcrun', 'notarytool', 'log', status.get('id', ''), '--keychain-profile', profile,
                              *keychain], capture_output=True, text=True)
        raise ValueError(f'Notarization failed: {result.stdout}\n{log.stdout}{log.stderr}')
    subprocess.run(['xcrun', 'stapler', 'staple', str(image)], check=True)
    subprocess.run(['spctl', '--assess', '--type', 'open', '--context', 'context:primary-signature',
                    '-vv', str(image)], check=True)
    print(f'Notarized and stapled Mac disk image: {image}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('bundle', type=Path)
    parser.add_argument('destination', type=Path)
    args = parser.parse_args()
    try:
        create(args.bundle, args.destination)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        raise SystemExit(f'Disk image failed: {error}') from error
