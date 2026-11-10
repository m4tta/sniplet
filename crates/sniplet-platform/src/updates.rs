//! Signed GitHub releases, staged downloads, and replacement of portable app packages.
//! Checking and downloading never change the installed app. Call `install` only after
//! the editor has saved recovery data and the user has requested a restart.

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const REPOSITORY: &str = "m4tta/sniplet";
const PUBLIC_KEY: &str = include_str!("../../../assets/update-public-key.hex");
const MAX_PACKAGE_SIZE: u64 = 512 * 1024 * 1024;
/// The manifest format this app can install. A newer release with another schema
/// installs a different way, so the app asks the user to download it by hand.
const SCHEMA: u32 = 3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseChannel {
    #[default]
    Stable,
    Nightly,
}

impl ReleaseChannel {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::Nightly => "Nightly",
        }
    }
}

/// The app version: `0.2.1` for stable, `0.2.1-nightly.20261104.17` for nightly.
pub const VERSION: &str = env!("SNIPLET_VERSION");

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuildIdentity {
    pub version: String,
    pub sha: String,
}

pub fn current_build() -> BuildIdentity {
    BuildIdentity {
        version: VERSION.into(),
        sha: env!("SNIPLET_BUILD_SHA").into(),
    }
}

/// A replacement process waits for the old process before it registers hotkeys
/// or opens recovery data. This avoids two app instances during an update restart.
pub fn wait_for_parent(process_id: u32) -> Result<()> {
    ensure!(
        process_id != 0 && process_id != std::process::id(),
        "Invalid restart process"
    );
    #[cfg(windows)]
    unsafe {
        use windows::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        match OpenProcess(PROCESS_SYNCHRONIZE, false, process_id) {
            Ok(handle) => {
                let result = WaitForSingleObject(handle, 30_000);
                let _ = CloseHandle(handle);
                ensure!(
                    result == WAIT_OBJECT_0,
                    "The previous Sniplet process did not close"
                );
            }
            Err(error) if error.code().0 as u32 == 0x80070057 => {} // Process already exited.
            Err(error) => return Err(error.into()),
        }
    }
    #[cfg(unix)]
    {
        let process_id = i32::try_from(process_id)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            // Signal 0 checks process existence without sending a signal.
            if unsafe { libc::kill(process_id, 0) } == -1 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ESRCH) {
                    break;
                }
                return Err(error.into());
            }
            ensure!(
                std::time::Instant::now() < deadline,
                "The previous Sniplet process did not close"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct UpdateRelease {
    pub version: String,
    pub sha: String,
    pub tag: String,
    pub size: u64,
    /// The release cannot install over this copy. The user downloads it from the website.
    pub reinstall: bool,
    asset_name: String,
    sha256: String,
}

#[derive(Debug)]
pub struct DownloadedUpdate {
    directory: tempfile::TempDir,
    release: UpdateRelease,
}

#[derive(Deserialize)]
struct Manifest {
    schema: u32,
    version: String,
    sha: String,
    channel: ReleaseChannel,
    tag: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    target: String,
    name: String,
    sha256: String,
    size: u64,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
}

fn package_target() -> Result<(&'static str, &'static str)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok(("windows-x64", ".zip")),
        ("macos", "aarch64") => Ok(("macos-arm64", ".zip")),
        ("macos", "x86_64") => Ok(("macos-x64", ".zip")),
        ("linux", "x86_64") => Ok(("linux-x64", ".tar.gz")),
        _ => bail!("Updates are not available for this system"),
    }
}

fn fetch(url: &str, limit: u64) -> Result<Vec<u8>> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .header("User-Agent", "Sniplet-Updater")
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("Could not connect to the update server")?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "The update server response is too large"
    );
    Ok(bytes)
}

fn download_url(tag: &str, name: &str) -> String {
    format!("https://github.com/{REPOSITORY}/releases/download/{tag}/{name}")
}

fn verify_manifest(bytes: &[u8], signature: &[u8], key: &str) -> Result<Manifest> {
    let key: [u8; 32] = hex::decode(key.trim())?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid update public key"))?;
    let signature = hex::decode(std::str::from_utf8(signature)?.trim())?;
    let signature = Signature::from_slice(&signature)?;
    VerifyingKey::from_bytes(&key)?
        .verify_strict(bytes, &signature)
        .context("The update signature is invalid. No files were installed")?;
    serde_json::from_slice(bytes).context("Invalid update manifest")
}

/// The channel a version belongs to: `X.Y.Z` is stable, `X.Y.Z-nightly.<date>.<run>` is nightly.
fn version_channel(version: &semver::Version) -> Option<ReleaseChannel> {
    let pre: Vec<_> = version.pre.as_str().split('.').collect();
    match pre.as_slice() {
        _ if !version.build.is_empty() => None,
        [""] => Some(ReleaseChannel::Stable),
        ["nightly", date, run]
            if date.len() == 8 && [date, run].iter().all(|p| p.parse::<u64>().is_ok()) =>
        {
            Some(ReleaseChannel::Nightly)
        }
        _ => None,
    }
}

fn select_manifest(
    manifest: Manifest,
    tag: &str,
    channel: ReleaseChannel,
    current: &BuildIdentity,
    target: &str,
    extension: &str,
) -> Result<Option<UpdateRelease>> {
    let version = semver::Version::parse(&manifest.version)?;
    // A channel change must never install an older version. Each nightly sorts after
    // the newest stable and before the next one, so Nightly to Stable waits for that.
    // This check comes first, so older releases with other manifest formats are skipped.
    if version <= semver::Version::parse(&current.version)? {
        return Ok(None);
    }
    ensure!(
        version_channel(&version) == Some(channel),
        "Invalid release version"
    );
    ensure!(
        manifest.sha.len() == 40
            && manifest
                .sha
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "Invalid release commit"
    );
    ensure!(
        manifest.channel == channel
            && manifest.tag == tag
            && tag == format!("v{}", manifest.version),
        "The update channel does not match its signature"
    );
    if manifest.schema != SCHEMA {
        return Ok(Some(UpdateRelease {
            version: manifest.version,
            sha: manifest.sha,
            tag: manifest.tag,
            size: 0,
            reinstall: true,
            asset_name: String::new(),
            sha256: String::new(),
        }));
    }
    let asset = manifest
        .assets
        .iter()
        .find(|a| a.target == target)
        .context("This release has no package for this system")?;
    ensure!(
        manifest
            .assets
            .iter()
            .filter(|a| a.target == target)
            .count()
            == 1,
        "Duplicate update package"
    );
    ensure!(
        asset.name == format!("Sniplet-{}-{target}{extension}", manifest.version),
        "Unexpected update package name"
    );
    ensure!(
        asset.size > 0 && asset.size <= MAX_PACKAGE_SIZE,
        "Invalid update package size"
    );
    ensure!(
        asset.sha256.len() == 64 && hex::decode(&asset.sha256).is_ok(),
        "Invalid update checksum"
    );
    Ok(Some(UpdateRelease {
        version: manifest.version,
        sha: manifest.sha,
        tag: manifest.tag,
        size: asset.size,
        reinstall: false,
        asset_name: asset.name.clone(),
        sha256: asset.sha256.clone(),
    }))
}

/// Check only published, signed releases. Older unsigned releases remain available
/// for manual download, but are never eligible for installation through the app.
pub fn check(channel: ReleaseChannel, current: &BuildIdentity) -> Result<Option<UpdateRelease>> {
    let (target, extension) = package_target()?;
    let api = format!("https://api.github.com/repos/{REPOSITORY}/releases");
    let release = match channel {
        ReleaseChannel::Stable => {
            let releases: GitHubRelease =
                serde_json::from_slice(&fetch(&format!("{api}/latest"), 2 * 1024 * 1024)?)?;
            Some(releases)
        }
        ReleaseChannel::Nightly => {
            let releases: Vec<GitHubRelease> =
                serde_json::from_slice(&fetch(&format!("{api}?per_page=100"), 2 * 1024 * 1024)?)?;
            // Older nightlies used other tag formats; only `vX.Y.Z-nightly.*` can update.
            releases
                .into_iter()
                .filter(|r| !r.draft && r.prerelease)
                .filter_map(|r| {
                    let version = semver::Version::parse(r.tag_name.strip_prefix('v')?).ok()?;
                    (version_channel(&version) == Some(ReleaseChannel::Nightly))
                        .then_some((version, r))
                })
                .max_by(|a, b| a.0.cmp(&b.0))
                .map(|(_, r)| r)
        }
    };
    let Some(release) = release else {
        return Ok(None);
    };
    ensure!(
        !release.draft && release.prerelease == (channel == ReleaseChannel::Nightly),
        "Unexpected release channel"
    );
    if !release.assets.iter().any(|a| a.name == "release.json.sig") {
        return Ok(None);
    }
    ensure!(
        release
            .tag_name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
        "Invalid release tag"
    );
    let bytes = fetch(&download_url(&release.tag_name, "release.json"), 256 * 1024)?;
    let signature = fetch(&download_url(&release.tag_name, "release.json.sig"), 1024)?;
    select_manifest(
        verify_manifest(&bytes, &signature, PUBLIC_KEY)?,
        &release.tag_name,
        channel,
        current,
        target,
        extension,
    )
}

fn verify_package(path: &Path, release: &UpdateRelease) -> Result<()> {
    let mut file = fs::File::open(path)?;
    ensure!(
        file.metadata()?.len() == release.size,
        "The update download is incomplete"
    );
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash)?;
    ensure!(
        hex::encode(hash.finalize()) == release.sha256,
        "The update checksum is invalid. No files were installed"
    );
    Ok(())
}

/// Download and verify only. The installed app is unchanged until `install`.
pub fn download(release: &UpdateRelease) -> Result<DownloadedUpdate> {
    ensure!(!release.reinstall, "Download this version from the website");
    let directory = tempfile::Builder::new()
        .prefix("sniplet-update-")
        .tempdir()?;
    let path = directory.path().join(&release.asset_name);
    let mut file = fs::File::create(&path)?;
    self_update::Download::from_url(download_url(&release.tag, &release.asset_name))
        .timeout(Duration::from_secs(600))
        .max_download_size(release.size)
        .download_to(&mut file)
        .context("Could not download the update")?;
    file.flush()?;
    verify_package(&path, release)?;
    Ok(DownloadedUpdate {
        directory,
        release: release.clone(),
    })
}

fn install_location(executable: &Path) -> Result<(PathBuf, &'static str, &'static str)> {
    #[cfg(target_os = "macos")]
    {
        let bundle = executable
            .ancestors()
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .context("Move Sniplet to Applications before updating")?;
        ensure!(
            !bundle.to_string_lossy().contains("/AppTranslocation/"),
            "Move Sniplet to Applications and open it again before updating"
        );
        Ok((
            bundle.to_path_buf(),
            "Sniplet.app",
            "Contents/MacOS/Sniplet",
        ))
    }
    #[cfg(target_os = "linux")]
    {
        ensure!(
            executable.file_name().is_some_and(|n| n == "sniplet")
                && executable
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|n| n == "bin"),
            "Install the portable Sniplet package before updating"
        );
        let root = executable
            .parent()
            .and_then(Path::parent)
            .context("Invalid app folder")?;
        // Only replace a complete portable bundle, never /usr or another system prefix.
        ensure!(
            root.file_name()
                .is_some_and(|n| n == "sniplet" || n.to_string_lossy().starts_with("sniplet-")),
            "Update this system installation with its package manager"
        );
        Ok((root.to_path_buf(), "sniplet", "bin/sniplet"))
    }
    #[cfg(target_os = "windows")]
    {
        Ok((
            executable.to_path_buf(),
            "sniplet-x86_64-pc-windows-msvc",
            "Sniplet.exe",
        ))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = executable;
        bail!("Updates are not supported on this system")
    }
}

/// Replace the executable on Windows, or the complete portable bundle on Unix.
/// Extract beside the destination so the bundle swap stays on one filesystem.
pub fn install(downloaded: DownloadedUpdate) -> Result<()> {
    let executable = std::env::current_exe()?.canonicalize()?;
    let (destination, archive_root, binary) = install_location(&executable)?;
    let parent = destination.parent().context("Invalid app folder")?;
    let staging = tempfile::Builder::new()
        .prefix(".sniplet-update-")
        .tempdir_in(parent)
        .context("The app folder is read-only. Move Sniplet to a folder you can change")?;
    let archive = downloaded
        .directory
        .path()
        .join(&downloaded.release.asset_name);
    verify_package(&archive, &downloaded.release)?;
    self_update::Extract::from_source(&archive).extract_into(staging.path())?;
    let bundle = staging.path().join(archive_root);
    ensure!(
        bundle.join(binary).is_file(),
        "The update package is missing the app"
    );
    #[cfg(target_os = "windows")]
    self_replace::self_replace(bundle.join(binary)).context("Could not replace the app")?;
    #[cfg(not(target_os = "windows"))]
    {
        let backup = tempfile::Builder::new()
            .prefix(".sniplet-backup-")
            .tempdir_in(parent)?;
        self_update::MoveAll::from_temp(backup.path())
            .add(&bundle, &destination)
            .commit()
            .context("Could not replace the app; the previous version was restored")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn payload(version: &str) -> Vec<u8> {
        let channel = if version.contains('-') {
            ReleaseChannel::Nightly
        } else {
            ReleaseChannel::Stable
        };
        serde_json::to_vec(&serde_json::json!({"schema":3,"version":version,
            "sha":"abcdef0123456789abcdef0123456789abcdef01","channel":channel,
            "tag":format!("v{version}"),"assets":[{"target":"windows-x64",
            "name":format!("Sniplet-{version}-windows-x64.zip"),
            "sha256":"00".repeat(32),"size":100}]}))
        .unwrap()
    }

    fn select(
        version: &str,
        channel: ReleaseChannel,
        current: &str,
    ) -> Result<Option<UpdateRelease>> {
        let manifest: Manifest = serde_json::from_slice(&payload(version)).unwrap();
        let current = BuildIdentity {
            version: current.into(),
            sha: "local".into(),
        };
        let tag = manifest.tag.clone();
        select_manifest(manifest, &tag, channel, &current, "windows-x64", ".zip")
    }

    #[test]
    fn signature_rejects_changed_manifest_and_wrong_key() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let bytes = payload("0.2.0");
        let signature = hex::encode(key.sign(&bytes).to_bytes());
        let public = hex::encode(key.verifying_key().to_bytes());
        assert!(verify_manifest(&bytes, signature.as_bytes(), &public).is_ok());
        let mut changed = bytes.clone();
        changed.push(b' ');
        assert!(verify_manifest(&changed, signature.as_bytes(), &public).is_err());
        assert!(
            verify_manifest(
                &bytes,
                signature.as_bytes(),
                &hex::encode(SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes())
            )
            .is_err()
        );
    }

    #[test]
    fn versions_order_nightlies_between_stables_and_never_downgrade() {
        use ReleaseChannel::{Nightly, Stable};
        let newer =
            |version, channel, current| select(version, channel, current).unwrap().is_some();
        assert!(newer("0.2.1-nightly.20261104.17", Nightly, "0.2.0"));
        assert!(newer(
            "0.2.1-nightly.20261104.18",
            Nightly,
            "0.2.1-nightly.20261104.17"
        ));
        assert!(newer(
            "0.2.1-nightly.20261105.1",
            Nightly,
            "0.2.1-nightly.20261104.17"
        ));
        assert!(!newer(
            "0.2.1-nightly.20261104.17",
            Nightly,
            "0.2.1-nightly.20261104.17"
        ));
        // Nightly to Stable waits for the stable release after the installed nightly.
        assert!(newer("0.2.1", Stable, "0.2.1-nightly.20261104.17"));
        assert!(!newer("0.2.0", Stable, "0.2.1-nightly.20261104.17"));
        // Stable to Nightly gets the newest nightly, because it is newer.
        assert!(newer("0.2.2-nightly.20261106.20", Nightly, "0.2.1"));
        assert!(!newer("0.2.1", Stable, "0.2.1"));
        // An older release in the previous manifest format is skipped, not an error.
        let mut old: Manifest = serde_json::from_slice(&payload("0.2.0")).unwrap();
        old.schema = 2;
        let current = BuildIdentity {
            version: "0.2.1-nightly.20261104.17".into(),
            sha: "local".into(),
        };
        let selected = select_manifest(old, "v0.2.0", Stable, &current, "windows-x64", ".zip");
        assert!(selected.unwrap().is_none());
        // A newer release in a newer format asks for a new install, and never downloads.
        let mut newer: Manifest = serde_json::from_slice(&payload("0.3.0")).unwrap();
        newer.schema = SCHEMA + 1;
        newer.assets.clear();
        let selected = select_manifest(newer, "v0.3.0", Stable, &current, "windows-x64", ".zip")
            .unwrap()
            .unwrap();
        assert!(selected.reinstall);
        assert_eq!(selected.version, "0.3.0");
        assert!(download(&selected).is_err());
    }

    #[test]
    fn signed_manifest_cannot_change_channel_or_escape_package_path() {
        use ReleaseChannel::{Nightly, Stable};
        assert!(select("0.2.0", Nightly, "0.1.0").is_err());
        assert!(select("0.2.1-nightly.20261104.17", Stable, "0.1.0").is_err());
        for version in ["0.2.1-a1b2c3d", "0.2.1-nightly.1.2", "0.2.1+build"] {
            assert!(select(version, Nightly, "0.1.0").is_err(), "{version}");
        }
        let mut manifest: Manifest = serde_json::from_slice(&payload("0.2.0")).unwrap();
        manifest.assets[0].name = "../Sniplet.exe".into();
        let current = BuildIdentity {
            version: "0.1.0".into(),
            sha: "local".into(),
        };
        assert!(
            select_manifest(manifest, "v0.2.0", Stable, &current, "windows-x64", ".zip").is_err()
        );
    }

    #[test]
    fn package_verification_rejects_incomplete_or_changed_download() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("package.zip");
        fs::write(&path, b"good").unwrap();
        let release = UpdateRelease {
            version: "0.2.0".into(),
            sha: String::new(),
            tag: String::new(),
            size: 4,
            reinstall: false,
            asset_name: String::new(),
            sha256: hex::encode(Sha256::digest(b"good")),
        };
        assert!(verify_package(&path, &release).is_ok());
        fs::write(&path, b"evil").unwrap();
        assert!(verify_package(&path, &release).is_err());
        fs::write(&path, b"x").unwrap();
        assert!(verify_package(&path, &release).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_install_replaces_a_running_executable() {
        const CHILD: &str = "SNIPLET_UPDATE_INSTALL_TEST";
        if std::env::var_os(CHILD).is_some() {
            let directory = tempfile::tempdir().unwrap();
            let archive = directory.path().join("fixture.zip");
            let replacement = fs::read(
                Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32/whoami.exe"),
            )
            .unwrap();
            let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
            zip.start_file(
                "sniplet-x86_64-pc-windows-msvc/Sniplet.exe",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(&replacement).unwrap();
            zip.finish().unwrap();
            let bytes = fs::read(&archive).unwrap();
            let release = UpdateRelease {
                version: "0.2.0".into(),
                sha: String::new(),
                tag: String::new(),
                size: bytes.len() as u64,
                reinstall: false,
                asset_name: "fixture.zip".into(),
                sha256: hex::encode(Sha256::digest(&bytes)),
            };
            install(DownloadedUpdate { directory, release }).unwrap();
            assert_eq!(
                fs::read(std::env::current_exe().unwrap()).unwrap(),
                replacement
            );
            return;
        }
        // Replace a disposable copy of this test runner, never the real test runner
        // or the user's installed app. Windows keeps the copied image locked.
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("Sniplet.exe");
        fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        let status = std::process::Command::new(&executable)
            .args([
                "--exact",
                "updates::tests::windows_install_replaces_a_running_executable",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            fs::read(executable).unwrap(),
            fs::read(
                Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32/whoami.exe")
            )
            .unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    #[expect(
        clippy::zombie_processes,
        reason = "The replacement waits for this process to exit; Unix then adopts and reaps it"
    )]
    fn unix_install_replaces_the_complete_running_bundle() {
        const CHILD: &str = "SNIPLET_UPDATE_BUNDLE_TEST";
        if std::env::var(CHILD).as_deref() == Ok("restarted") {
            let parent: u32 = std::env::var("SNIPLET_UPDATE_PARENT")
                .unwrap()
                .parse()
                .unwrap();
            wait_for_parent(parent).unwrap();
            let (destination, _, _) = install_location(&std::env::current_exe().unwrap()).unwrap();
            assert_eq!(
                fs::read(destination.join("version.txt")).unwrap(),
                b"new resources"
            );
            fs::write(
                std::env::var_os("SNIPLET_UPDATE_ACK").unwrap(),
                b"restarted",
            )
            .unwrap();
            return;
        }
        if std::env::var(CHILD).as_deref() == Ok("install") {
            let executable = std::env::current_exe().unwrap();
            let (destination, root, binary) = install_location(&executable).unwrap();
            let directory = tempfile::tempdir().unwrap();
            let archive = directory.path().join("fixture.zip");
            let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
            zip.start_file(
                format!("{root}/{binary}"),
                zip::write::SimpleFileOptions::default().unix_permissions(0o755),
            )
            .unwrap();
            zip.write_all(&fs::read(&executable).unwrap()).unwrap();
            zip.start_file(
                format!("{root}/version.txt"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"new resources").unwrap();
            zip.finish().unwrap();
            let bytes = fs::read(&archive).unwrap();
            let release = UpdateRelease {
                version: "0.2.0".into(),
                sha: String::new(),
                tag: String::new(),
                size: bytes.len() as u64,
                reinstall: false,
                asset_name: "fixture.zip".into(),
                sha256: hex::encode(Sha256::digest(&bytes)),
            };
            install(DownloadedUpdate { directory, release }).unwrap();
            assert_eq!(
                fs::read(destination.join("version.txt")).unwrap(),
                b"new resources"
            );
            assert!(!destination.join("old-resource.txt").exists());
            std::process::Command::new(destination.join(binary))
                .args([
                    "--exact",
                    "updates::tests::unix_install_replaces_the_complete_running_bundle",
                    "--nocapture",
                ])
                .env(CHILD, "restarted")
                .env("SNIPLET_UPDATE_PARENT", std::process::id().to_string())
                .spawn()
                .unwrap();
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        #[cfg(target_os = "macos")]
        let executable = directory.path().join("Sniplet.app/Contents/MacOS/Sniplet");
        #[cfg(target_os = "linux")]
        let executable = directory.path().join("sniplet-test/bin/sniplet");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        let (bundle, _, _) = install_location(&executable).unwrap();
        fs::write(bundle.join("old-resource.txt"), b"old").unwrap();
        let status = std::process::Command::new(executable)
            .args([
                "--exact",
                "updates::tests::unix_install_replaces_the_complete_running_bundle",
                "--nocapture",
            ])
            .env(CHILD, "install")
            .env("SNIPLET_UPDATE_ACK", directory.path().join("restarted.txt"))
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            fs::read(bundle.join("version.txt")).unwrap(),
            b"new resources"
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(35);
        let acknowledgement = directory.path().join("restarted.txt");
        while !acknowledgement.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "The replacement app did not restart"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(fs::read(acknowledgement).unwrap(), b"restarted");
    }
}
