//! Signed GitHub releases, installed by Velopack. The app finds and checks a release with
//! its own signed manifest, Velopack downloads that package and applies it after the app
//! exits. Checking and downloading never change the installed app. Call `install` only after
//! the editor has saved recovery data and the user has requested a restart.

use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use velopack::{
    UpdateInfo, UpdateManager, VelopackApp, VelopackAsset,
    locator::{LocationContext, auto_locate_app_manifest},
    sources::{HttpSource, NoneSource},
};

const REPOSITORY: &str = "m4tta/sniplet";
const PUBLIC_KEY: &str = include_str!("../../../assets/update-public-key.hex");
const MAX_PACKAGE_SIZE: u64 = 512 * 1024 * 1024;
/// The manifest format this app can install. A newer release with another schema
/// installs a different way, so the app asks the user to download it by hand.
const SCHEMA: u32 = 4;
/// The Velopack package id, set by `vpk pack --packId` in scripts/release.py.
const PACK_ID: &str = "Sniplet";

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

/// Run Velopack's install, update, and uninstall hooks. Call it first in `main`, because a
/// hook can end the process. The app applies a downloaded update only when the user asks.
pub fn startup() {
    VelopackApp::build().set_auto_apply_on_startup(false).run();
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

/// True when Velopack installed this copy, so it can update itself. The Linux tar.gz,
/// `scripts/package.sh` builds, and `cargo run` cannot.
pub fn can_update() -> bool {
    UpdateManager::new(NoneSource {}, None, None).is_ok()
}

/// A package in Velopack's packages folder, ready to apply after the app exits.
pub struct DownloadedUpdate {
    manager: UpdateManager,
    asset: VelopackAsset,
    package: PathBuf,
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

fn package_target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok("windows-x64"),
        ("macos", "aarch64") => Ok("macos-arm64"),
        ("macos", "x86_64") => Ok("macos-x64"),
        ("linux", "x86_64") => Ok("linux-x64"),
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
        asset.name == format!("{PACK_ID}-{}-{target}-full.nupkg", manifest.version),
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
    let target = package_target()?;
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
    )
}

/// Velopack skips its own size and checksum checks for a package that is already in its
/// folder, so check every package against the signed manifest here.
fn verify_package(path: &Path, size: u64, sha256: &str) -> Result<()> {
    let mut file = std::fs::File::open(path)?;
    ensure!(
        file.metadata()?.len() == size,
        "The update download is incomplete"
    );
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash)?;
    ensure!(
        hex::encode(hash.finalize()) == sha256,
        "The update checksum is invalid. No files were installed"
    );
    Ok(())
}

/// Download and verify only. The installed app is unchanged until `install`.
pub fn download(release: &UpdateRelease) -> Result<DownloadedUpdate> {
    ensure!(!release.reinstall, "Download this version from the website");
    let source = HttpSource::new(format!(
        "https://github.com/{REPOSITORY}/releases/download/{}/",
        release.tag
    ));
    #[cfg(target_os = "linux")]
    let locator = Some(updater_outside_appimage()?);
    #[cfg(not(target_os = "linux"))]
    let locator: Option<velopack::locator::VelopackLocatorConfig> = None;
    let packages = match &locator {
        Some(config) => config.PackagesDir.clone(),
        None => auto_locate_app_manifest(LocationContext::FromCurrentExe)?.get_packages_dir(),
    };
    let manager = UpdateManager::new(source, None, locator)?;
    let package = packages.join(&release.asset_name);
    // Velopack would use a damaged or replaced copy as it is. Remove it, so it downloads again.
    if verify_package(&package, release.size, &release.sha256).is_err() {
        let _ = std::fs::remove_file(&package);
    }
    let asset = VelopackAsset {
        PackageId: PACK_ID.into(),
        Version: release.version.clone(),
        Type: "Full".into(),
        FileName: release.asset_name.clone(),
        SHA256: release.sha256.clone(),
        Size: release.size,
        ..Default::default()
    };
    let update = UpdateInfo {
        TargetFullRelease: asset.clone(),
        ..Default::default()
    };
    manager
        .download_updates(&update, None)
        .context("Could not download the update")?;
    verify_package(&package, release.size, &release.sha256)?;
    Ok(DownloadedUpdate {
        manager,
        asset,
        package,
    })
}

/// Start Velopack's updater. It waits for this process to exit, replaces the app, and
/// opens the new version with `restart_args`. The caller must exit next.
pub fn install(downloaded: DownloadedUpdate, restart_args: &[&str]) -> Result<()> {
    let asset = &downloaded.asset;
    verify_package(&downloaded.package, asset.Size, &asset.SHA256)?;
    downloaded
        .manager
        .wait_exit_then_apply_updates(&downloaded.asset, false, true, restart_args)
        .context("Could not start the updater")
}

/// The AppImage unmounts as soon as the app exits, so an updater that runs from inside it
/// dies partway through. Run a copy from a temporary folder instead. The copy finds its
/// manifest beside it, in a folder that ends in `usr/bin` like the AppImage's own. The
/// package goes in the same folder, because Velopack's default packages folder is in the
/// shared `/var/tmp`, where another user can create it first.
#[cfg(target_os = "linux")]
fn updater_outside_appimage() -> Result<velopack::locator::VelopackLocatorConfig> {
    use velopack::locator::VelopackLocatorConfig;
    let locator = auto_locate_app_manifest(LocationContext::FromCurrentExe)?;
    use std::os::unix::fs::PermissionsExt;
    // A new folder that only this user can open. It stays after the app exits, because
    // the updater still runs from it.
    let root = tempfile::Builder::new()
        .prefix("sniplet-updater-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()?
        .keep();
    let bin = root.join("usr/bin");
    std::fs::create_dir_all(&bin)?;
    let updater = bin.join("UpdateNix");
    let manifest = bin.join("sq.version");
    std::fs::copy(locator.get_update_path(), &updater)?;
    std::fs::copy(locator.get_current_bin_dir().join("sq.version"), &manifest)?;
    Ok(VelopackLocatorConfig {
        RootAppDir: locator.get_appimage_path(),
        UpdateExePath: updater,
        PackagesDir: root.join("packages"),
        ManifestPath: manifest,
        CurrentBinaryDir: locator.get_current_bin_dir(),
        IsPortable: true,
    })
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
        serde_json::to_vec(&serde_json::json!({"schema":SCHEMA,"version":version,
            "sha":"abcdef0123456789abcdef0123456789abcdef01","channel":channel,
            "tag":format!("v{version}"),"assets":[{"target":"windows-x64",
            "name":format!("Sniplet-{version}-windows-x64-full.nupkg"),
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
        select_manifest(manifest, &tag, channel, &current, "windows-x64")
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
        old.schema = SCHEMA - 1;
        let current = BuildIdentity {
            version: "0.2.1-nightly.20261104.17".into(),
            sha: "local".into(),
        };
        let selected = select_manifest(old, "v0.2.0", Stable, &current, "windows-x64");
        assert!(selected.unwrap().is_none());
        // A newer release in a newer format asks for a new install, and never downloads.
        let mut newer: Manifest = serde_json::from_slice(&payload("0.3.0")).unwrap();
        newer.schema = SCHEMA + 1;
        newer.assets.clear();
        let selected = select_manifest(newer, "v0.3.0", Stable, &current, "windows-x64")
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
        assert!(select_manifest(manifest, "v0.2.0", Stable, &current, "windows-x64").is_err());
    }

    #[test]
    fn package_verification_rejects_incomplete_or_changed_download() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("package.nupkg");
        let sha256 = hex::encode(Sha256::digest(b"good"));
        std::fs::write(&path, b"good").unwrap();
        assert!(verify_package(&path, 4, &sha256).is_ok());
        std::fs::write(&path, b"evil").unwrap();
        assert!(verify_package(&path, 4, &sha256).is_err());
        std::fs::write(&path, b"x").unwrap();
        assert!(verify_package(&path, 4, &sha256).is_err());
    }
}
