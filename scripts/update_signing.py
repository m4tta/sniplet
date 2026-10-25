#!/usr/bin/env python3
"""Create the update key once, and sign or verify release manifests with Ed25519.

The private PEM key stays outside the checkout. Never print it or commit it.
Requires the version of cryptography listed in requirements-release.txt.
"""
import argparse
import os
from pathlib import Path
import subprocess

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey, Ed25519PublicKey
from cryptography.exceptions import InvalidSignature

ROOT = Path(__file__).resolve().parent.parent
PUBLIC_KEY = ROOT / 'assets/update-public-key.hex'


def sign(path, key_text=None):
    key_text = key_text or os.environ.get('UPDATER_SIGNING_KEY')
    if not key_text:
        raise ValueError('UPDATER_SIGNING_KEY is required for an update release')
    key = serialization.load_pem_private_key(key_text.encode(), password=None)
    if not isinstance(key, Ed25519PrivateKey):
        raise ValueError('The update key must be Ed25519')
    public = key.public_key().public_bytes_raw().hex()
    if public != PUBLIC_KEY.read_text().strip():
        raise ValueError('The private update key does not match the public key in this checkout')
    signature = key.sign(path.read_bytes()).hex()
    path.with_suffix(path.suffix + '.sig').write_text(signature + '\n', encoding='ascii')


def verify(path):
    signature = bytes.fromhex(path.with_suffix(path.suffix + '.sig').read_text().strip())
    key = Ed25519PublicKey.from_public_bytes(bytes.fromhex(PUBLIC_KEY.read_text().strip()))
    try:
        key.verify(signature, path.read_bytes())
    except InvalidSignature as error:
        raise ValueError('Release manifest signature is invalid') from error


def initialize(path, install_secret):
    if path.exists() or PUBLIC_KEY.exists():
        raise ValueError('An update key already exists. Do not rotate it during normal releases')
    key = Ed25519PrivateKey.generate()
    private = key.private_bytes(serialization.Encoding.PEM,
                               serialization.PrivateFormat.PKCS8,
                               serialization.NoEncryption())
    write_private(path, private)
    PUBLIC_KEY.write_text(key.public_key().public_bytes_raw().hex() + '\n', encoding='ascii')
    if install_secret:
        subprocess.run(['gh', 'secret', 'set', 'UPDATER_SIGNING_KEY', '--repo', 'm4tta/sniplet'],
                       input=private, check=True)
    print('Update key created. Back up the private key in a secure location: ' + str(path))
    print('Public verification key: ' + str(PUBLIC_KEY))


def write_private(path, private):
    """Create a new private file without changing permissions on existing directories."""
    new_directory = not path.parent.exists()
    path.parent.mkdir(parents=True, exist_ok=True)
    if os.name == 'nt' and new_directory:
        # Limit the private directory to the current user and Windows system services.
        identity = subprocess.check_output(['whoami'], text=True).strip()
        subprocess.run(['icacls', str(path.parent), '/inheritance:r', '/grant:r',
                        identity + ':(OI)(CI)F', '*S-1-5-18:(OI)(CI)F'], check=True,
                       stdout=subprocess.DEVNULL)
    elif os.name != 'nt' and new_directory:
        path.parent.chmod(0o700)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, 'wb') as stream:
        if os.name == 'nt':
            identity = subprocess.check_output(['whoami'], text=True).strip()
            subprocess.run(['icacls', str(path), '/inheritance:r', '/grant:r',
                            identity + ':F', '*S-1-5-18:F'], check=True,
                           stdout=subprocess.DEVNULL)
        stream.write(private)


def backup(source, destination):
    destination = destination.resolve()
    if destination.is_relative_to(ROOT):
        raise ValueError('Keep private key backups outside the repository')
    private = source.read_bytes()
    key = serialization.load_pem_private_key(private, password=None)
    if key.public_key().public_bytes_raw().hex() != PUBLIC_KEY.read_text().strip():
        raise ValueError('The private key does not match the app public key')
    write_private(destination, private)
    if destination.read_bytes() != private:
        raise ValueError('Private key backup verification failed')
    print('Verified private key backup: ' + str(destination))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    init = commands.add_parser('init')
    init.add_argument('--private-key', type=Path,
                      default=Path.home() / '.sniplet-release' / 'update-signing.pem')
    init.add_argument('--install-github-secret', action='store_true')
    copy = commands.add_parser('backup')
    copy.add_argument('--private-key', type=Path,
                      default=Path.home() / '.sniplet-release' / 'update-signing.pem')
    copy.add_argument('--destination', type=Path, required=True)
    args = parser.parse_args()
    if args.command == 'init':
        initialize(args.private_key, args.install_github_secret)
    elif args.command == 'backup':
        backup(args.private_key, args.destination)
