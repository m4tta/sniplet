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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuildIdentity {
    pub version: String,
    pub sha: String,
    pub build_number: u64,
}

pub fn current_build() -> BuildIdentity {
    BuildIdentity {
        version: env!("CARGO_PKG_VERSION").into(),
        sha: env!("SNIPLET_BUILD_SHA").into(),
        build_number: env!("SNIPLET_BUILD_NUMBER").parse().unwrap_or(0),
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
    pub build_number: u64,
    pub tag: String,
    pub channel: ReleaseChannel,
    pub size: u64,
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
    build_number: u64,
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
    id: u64,
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

fn select_manifest(
    manifest: Manifest,
    tag: &str,
    channel: ReleaseChannel,
    current: &BuildIdentity,
    target: &str,
    extension: &str,
) -> Result<Option<UpdateRelease>> {
    let version = semver::Version::parse(&manifest.version)?;
    ensure!(
        version.pre.is_empty() && version.build.is_empty(),
        "Invalid release version"
    );
    ensure!(
        manifest.schema == 2 && manifest.build_number > 0,
        "Unsupported update manifest"
    );
    ensure!(
        manifest.sha.len() == 40
            && manifest
                .sha
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "Invalid release commit"
    );
    let expected_tag = match channel {
        ReleaseChannel::Stable => format!("v{}", manifest.version),
        ReleaseChannel::Nightly => format!("v{}-{}", manifest.version, &manifest.sha[..7]),
    };
    ensure!(
        manifest.channel == channel && manifest.tag == tag && tag == expected_tag,
        "The update channel does not match its signature"
    );
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
        asset.name == format!("Sniplet-{}-{target}{extension}", &tag[1..]),
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
    let current_version = semver::Version::parse(&current.version)?;
    // A channel change must never silently install an older version. A nightly
    // may become a stable release without replacing identical package bytes.
    let newer = version > current_version
        || (version == current_version
            && manifest.sha != current.sha
            && manifest.build_number > current.build_number);
    if !newer {
        return Ok(None);
    }
    Ok(Some(UpdateRelease {
        version: manifest.version,
        sha: manifest.sha,
        build_number: manifest.build_number,
        tag: manifest.tag,
        channel,
        size: asset.size,
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
            releases
                .into_iter()
                .filter(|r| {
                    !r.draft
                        && r.prerelease
                        && r.assets.iter().any(|a| a.name == "release.json.sig")
                })
                .max_by_key(|r| r.id)
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

    fn payload(channel: ReleaseChannel, number: u64) -> Vec<u8> {
        let sha = "abcdef0123456789abcdef0123456789abcdef01";
        let tag = if channel == ReleaseChannel::Nightly {
            "v0.2.0-abcdef0"
        } else {
            "v0.2.0"
        };
        serde_json::to_vec(&serde_json::json!({"schema":2,"version":"0.2.0","sha":sha,
            "build_number":number,"channel":channel,"tag":tag,"assets":[{
            "target":"windows-x64","name":format!("Sniplet-{}-windows-x64.zip", &tag[1..]),
            "sha256":"00".repeat(32),"size":100}]}))
        .unwrap()
    }

    #[test]
    fn signature_rejects_changed_manifest_and_wrong_key() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let bytes = payload(ReleaseChannel::Stable, 10);
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
    fn nightly_order_uses_build_number_and_stable_never_downgrades() {
        let mut current = BuildIdentity {
            version: "0.2.0".into(),
            sha: "other".into(),
            build_number: 9,
        };
        let select = |channel, number, current: &BuildIdentity| {
            let manifest: Manifest = serde_json::from_slice(&payload(channel, number)).unwrap();
            let tag = manifest.tag.clone();
            select_manifest(manifest, &tag, channel, current, "windows-x64", ".zip").unwrap()
        };
        assert!(select(ReleaseChannel::Nightly, 10, &current).is_some());
        assert!(select(ReleaseChannel::Nightly, 8, &current).is_none());
        assert!(select(ReleaseChannel::Stable, 10, &current).is_some());
        current.sha = "abcdef0123456789abcdef0123456789abcdef01".into();
        assert!(select(ReleaseChannel::Stable, 10, &current).is_none());
        current.version = "0.3.0".into();
        assert!(select(ReleaseChannel::Stable, 10, &current).is_none());
    }

    #[test]
    fn signed_manifest_cannot_change_channel_or_escape_package_path() {
        let current = BuildIdentity {
            version: "0.1.0".into(),
            sha: "".into(),
            build_number: 0,
        };
        let manifest: Manifest =
            serde_json::from_slice(&payload(ReleaseChannel::Stable, 10)).unwrap();
        assert!(
            select_manifest(
                manifest,
                "v0.2.0",
                ReleaseChannel::Nightly,
                &current,
                "windows-x64",
                ".zip"
            )
            .is_err()
        );
        let mut manifest: Manifest =
            serde_json::from_slice(&payload(ReleaseChannel::Stable, 10)).unwrap();
        manifest.assets[0].name = "../Sniplet.exe".into();
        assert!(
            select_manifest(
                manifest,
                "v0.2.0",
                ReleaseChannel::Stable,
                &current,
                "windows-x64",
                ".zip"
            )
            .is_err()
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
            build_number: 1,
            tag: String::new(),
            channel: ReleaseChannel::Stable,
            size: 4,
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
                build_number: 1,
                tag: String::new(),
                channel: ReleaseChannel::Stable,
                size: bytes.len() as u64,
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
                build_number: 1,
                tag: String::new(),
                channel: ReleaseChannel::Stable,
                size: bytes.len() as u64,
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
