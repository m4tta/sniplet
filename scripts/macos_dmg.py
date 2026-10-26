#!/usr/bin/env python3
"""Create and check a drag-to-Applications disk image on macOS."""

import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile


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
    with tempfile.TemporaryDirectory(prefix='.sniplet-dmg-', dir=destination.parent) as temporary:
        root = Path(temporary)
        contents = root / 'contents'
        contents.mkdir()
        subprocess.run(['ditto', '--rsrc', '--extattr', str(bundle),
                        str(contents / bundle.name)], check=True)
        (contents / 'Applications').symlink_to('/Applications', target_is_directory=True)
        image = root / 'Sniplet.dmg'
        subprocess.run(['hdiutil', 'create', '-volname', 'Sniplet', '-fs', 'HFS+',
                        '-format', 'UDZO', '-srcfolder', str(contents), str(image)], check=True)
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
            installed = mount / bundle.name
            subprocess.run(['codesign', '--verify', '--deep', '--strict', str(installed)], check=True)
            for path in [binary, Path('Contents/Info.plist')]:
                if (bundle / path).read_bytes() != (installed / path).read_bytes():
                    raise ValueError('The disk image changed the app bundle')
        finally:
            subprocess.run(['hdiutil', 'detach', str(mount)], check=True)
        # Replace the output only after the new image passes all checks.
        image.replace(destination)
    print(f'Created and verified Mac disk image: {destination}')
    return destination


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('bundle', type=Path)
    parser.add_argument('destination', type=Path)
    args = parser.parse_args()
    try:
        create(args.bundle, args.destination)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        raise SystemExit(f'Disk image failed: {error}') from error
