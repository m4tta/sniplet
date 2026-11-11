# Sniplet releases

`.github/workflows/release.yml` builds, signs, and publishes each release to
[GitHub Releases](https://github.com/m4tta/sniplet/releases). The app updates
itself from there.

## Channels

| Channel | Trigger | Version |
| --- | --- | --- |
| nightly | A check every hour. It builds `main` if `main` has commits after the newest nightly, and that nightly is at least 2 hours old. Or run **Actions → Release → Run workflow** by hand. | `0.2.1-nightly.20261104.17` |
| stable | Run **Release** by hand with channel **stable**. | `0.2.1` |

`python scripts/release.py plan` makes these decisions. Git tags hold the
version, not `Cargo.toml`:

- **Nightly:** the newest stable tag plus one patch, then the UTC date and the
  workflow run number. After `v0.2.0`, nightlies are `0.2.1-nightly.*`. Each
  nightly is newer than the newest stable and older than the next stable. Each
  nightly run pushes its tag before it builds, so a failed build does not run
  again until there are new commits and 2 hours have passed.
- **Stable:** finds the newest nightly whose run passed, and rebuilds that
  nightly's commit. Stable ships only code that nightly users already ran. The
  version is the nightly's base (`0.2.1-nightly.*` becomes `0.2.1`). For a minor
  or major release, type a version such as `0.3.0` in the **version** field.
  The `v0.2.1` tag goes up with the release, after all four builds pass.

The version in `Cargo.toml` is used only for local builds.

Releases run one at a time, in one queue. A nightly check never replaces a
stable run that waits.

A pull request that changes packaging (the paths at the top of
`.github/workflows/release.yml`) runs the four build jobs with a throwaway
version such as `0.0.1-nightly.20261110.42` (vpk needs 0.0.1 or higher). It
pushes no tag and publishes nothing. Its packages stay on the run as artifacts
for 7 days.

If one build fails, use **Re-run failed jobs**. It keeps the same version. Do
not use **Re-run all jobs** on a nightly: the nightly tag already exists, so the
plan fails. Start a new run instead.

## Downloads

Each release has these files (a nightly uses its full version in the names):

| Download | Contents |
| --- | --- |
| `Sniplet-0.2.1-macos-arm64.dmg` | Mac Apple Silicon drag-to-Applications disk image |
| `Sniplet-0.2.1-macos-x64.dmg` | Mac Intel drag-to-Applications disk image |
| `Sniplet-0.2.1-windows-x64-Setup.exe` | Windows installer. It installs for the current user, with no admin prompt. |
| `Sniplet-0.2.1-windows-x64-Portable.zip` | Windows app in a folder that you can put anywhere. It updates itself too. |
| `Sniplet-0.2.1-linux-x64.AppImage` | Linux app in one executable file |
| `Sniplet-0.2.1-linux-x64.tar.gz` | Linux executable, desktop entry, icons, and support files. It does not update itself. |
| `Sniplet-0.2.1-<target>-full.nupkg` | One Velopack package for each target, for in-app updates |
| `SHA256SUMS` | SHA-256 checksums for all ten packages |
| `release.json` | Version, channel, source commit, package sizes, and checksums |
| `release.json.sig` | Ed25519 signature over the exact `release.json` bytes |

On Mac, open the DMG for your processor and drag Sniplet to the Applications
shortcut. Eject the disk image, then open Sniplet from Applications.

[Velopack](https://velopack.io) makes the packages. Each build job runs
`vpk pack` with the target as its channel, for example `--channel windows-x64`,
so the four targets have different file names in one GitHub release. The Mac
DMG holds the `.app` from Velopack's portable package. Velopack's `.pkg` is not
published. The `vpk` version in the Release workflow must be the same as the
`velopack` crate version in `crates/sniplet-platform/Cargo.toml`.

The publish job waits for all four builds. It uploads every file into a draft,
checks each size and checksum on GitHub, and only then publishes. A nightly is a
GitHub prerelease. A stable release becomes **Latest**. A rerun after an
interrupted upload resumes the draft. A published release is never changed.

## How the app updates

General settings has the channel choice, automatic checks, and a manual check
button. The app starts on Stable.

- **Stable** reads GitHub's Latest release.
- **Nightly** reads the newest `vX.Y.Z-nightly.*` prerelease.

The app checks the Ed25519 signature on `release.json` with the public key in
`assets/update-public-key.hex`. The signed manifest gives the name, size, and
SHA-256 checksum of the Velopack package for each target. Velopack downloads
that package and checks its size and checksum. The app does not read
Velopack's own `releases.<channel>.json` feed. Automatic checks download a
verified update in the background. Installation waits for **Restart & update**.

The app never installs an older version:

- Stable to Nightly: the app gets the newest nightly, because it is newer.
- Nightly to Stable: the app keeps its nightly until a newer stable release is
  available. Because of the version rule above, that is the next stable release.

Before an update, the app saves the current screenshot and edits to a recovery
project beside its settings. Then it starts Velopack's updater and quits. The
updater waits for the app to exit, replaces the app, and opens the new version,
which restores the project. Recovery keeps the screenshot, crop, backdrop, and
annotations. It does not keep undo history. If the app is in a folder that the
user cannot change, Velopack asks for an administrator password. Copies that
Velopack did not install (the Linux tar.gz, `scripts/package.sh` builds, and
`cargo run`) cannot update themselves. Their Settings show a newer release with
an **Open website** button, and they never download it.

`release.json` has a `schema` number. When a newer release has a schema that the
app cannot install, the app tells the user to download it from the website. The
change to Velopack (schema 4) is such a release. Older schema 3 builds that do
not have this message show "Unsupported update manifest" instead. Builds before
schema 3 (`0.2.0`, and nightlies tagged `v0.2.1-<commit>`) cannot read the new
releases. Install a new build by hand once.

## Keys and secrets

| GitHub secret | Use |
| --- | --- |
| `UPDATER_SIGNING_KEY` | Ed25519 private key (PEM) that signs `release.json`. Only the publish job gets it. |
| `MACOS_SIGNING_P12`, `MACOS_SIGNING_P12_PASSWORD` | Optional. A self-signed "Sniplet Self-Signed" code signing certificate (base64 `.p12`). |
| `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID` | IDs for Windows signing through Azure Artifact Signing. They are not secret values. Infisical "The Vault" (`dev`) has copies as `SNIPLET_RELEASE_AZURE_*`. |

### Update key

Install the release tool dependencies:

```sh
python3 -m pip install -r scripts/requirements-release.txt
```

For the first setup only, run:

```sh
python3 scripts/update_signing.py init --install-github-secret
```

This creates a private key at `~/.sniplet-release/update-signing.pem`, writes
`assets/update-public-key.hex`, and sets the `UPDATER_SIGNING_KEY` secret
through `gh`. Do not make a new key for each release. Installed apps trust the
public key that was compiled into them. If the key is lost, users must install
a new build by hand.

To make a verified backup outside the checkout:

```sh
python3 scripts/update_signing.py backup --destination /secure-backup/sniplet-update.pem
```

The command limits file access to the current user (and SYSTEM on Windows). It
refuses to overwrite a file or to put the backup in this repository. Keep an
offline copy too.

### Windows signing

On Windows, `vpk pack --azureTrustedSignFile packaging/windows/signing.json`
signs `Sniplet.exe`, Velopack's `Update.exe` and execution stub, and
`Setup.exe` before it packs them. So `Setup.exe`, the update package
(`-full.nupkg`), and `Portable.zip` contain only signed executables. The
checksums in `SHA256SUMS` and `release.json` are for those signed packages.
Each signature has a timestamp and shows "Matthew Anderson" as publisher. `signing.json` names the account and profile
only. It holds no secret.

The pull request check of the Release workflow does not sign, because pull
requests have no Azure access. Only release runs from `main` sign.

| Item | Value |
| --- | --- |
| Artifact Signing account | `m4tta` (resource group `code-signing`, East US, `https://eus.codesigning.azure.net/`) |
| Certificate profile | `PublicTrust`, subject `CN=Matthew Anderson, O=Matthew Anderson, L=Chino, S=ca, C=US` |
| CI identity | Entra app `sniplet-release-signing` (app ID `1dba9d40-44dc-4c4a-835d-8fbb1a64461b`). Only for this repository. |
| Role | **Artifact Signing Certificate Profile Signer**, scoped to the `PublicTrust` profile only |
| Sign-in | GitHub OIDC through `azure/login`. There is no client secret. |
| OIDC subject | `repo:m4tta@427218/sniplet@1404794552:ref:refs/heads/main` |

GitHub sends the subject with the owner and repository IDs. A release that
runs from a branch other than `main` cannot sign. If `azure/login` fails with
`AADSTS700213`, the error shows the subject GitHub sent. Add a federated
credential for that exact subject.

To stop signing for this repository only, delete the `sniplet-release-signing`
app. Other apps that use the same account are not affected.

**Renew the identity validation before 2027-11-10.** After it expires, the
account cannot sign. Renew it in the Azure portal: **Artifact Signing account
`m4tta` → Identity validations**.

### Mac signing identity

Without the Mac secrets, packages use ad hoc signing. Each update then has a new
code identity, and macOS can ask for the Screen Recording permission again. A
fixed self-signed identity keeps the permission across updates. It does not need
an Apple developer account. Make it once:

```sh
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -subj "/CN=Sniplet Self-Signed" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=critical,codeSigning" \
  -keyout sniplet-signing.key -out sniplet-signing.crt
openssl pkcs12 -export -legacy -name "Sniplet Self-Signed" \
  -inkey sniplet-signing.key -in sniplet-signing.crt -out sniplet-signing.p12
base64 < sniplet-signing.p12 | gh secret set MACOS_SIGNING_P12
gh secret set MACOS_SIGNING_P12_PASSWORD
```

Keep the `.p12` and its password outside the repository, with an offline copy.
If the identity changes, macOS asks for the permission again one time.

## Platform limits

Mac releases are not signed by Apple. Windows releases have an Authenticode
signature from Azure Artifact Signing.

- macOS: packages are not notarized. Gatekeeper blocks the first launch. Open
  **System Settings → Privacy & Security**, then select **Open Anyway**.
- Windows: SmartScreen shows "Matthew Anderson" as publisher for the installer
  and the app. It can still warn on first launch until downloads build
  reputation for the certificate.
- Linux: packages are built on Ubuntu 24.04 and need compatible system
  libraries, including the graphics and capture libraries listed in the README.
  They are not distribution packages.

## Local checks

```sh
python3 -m pip install -r scripts/requirements-release.txt
python3 -m unittest discover -s scripts -p test_release.py
```

These checks do not publish a release or start a workflow.
