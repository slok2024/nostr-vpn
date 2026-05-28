use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use hashtree_blossom::{BlossomClient, BlossomStore};
use hashtree_core::{HashTree, HashTreeConfig};
use hashtree_resolver::nostr::{NostrResolverConfig, NostrRootResolver};
use hashtree_updater::{
    DownloadOptions, HashtreeUpdater, UpdateAsset, UpdateCheckOptions, UpdateManifest, UpdateRef,
    UpdateTarget,
};
use serde::Deserialize;

const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/mmalmi/nostr-vpn/releases/latest";
const HTREE_MANIFEST_URL: &str = "https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Fnostr-vpn/latest/release.json";
const HTREE_UPDATE_REF: &str = "htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Fnostr-vpn/latest";
const UPDATE_CONNECT_TIMEOUT_SECS: &str = "4";
const UPDATE_TOTAL_TIMEOUT_SECS: &str = "8";
const UPDATE_DOWNLOAD_TIMEOUT_SECS: &str = "180";
const UPDATE_USER_AGENT: &str = "nvpn-updater";
const SECURE_SOURCE_URL: &str = "hashtree://signed-nostr-release";
const DEFAULT_UPDATE_RELAYS: &[&str] = &[
    "wss://temp.iris.to",
    "wss://relay.damus.io",
    "wss://relay.snort.social",
    "wss://relay.primal.net",
    "wss://upload.iris.to/nostr",
];
const DEFAULT_BLOSSOM_READ_SERVERS: &[&str] = &[
    "https://cdn.iris.to",
    "https://hashtree.iris.to",
    "https://upload.iris.to",
    "https://blossom.primal.net",
];

#[derive(Clone, Debug, Default)]
pub struct UpdateState {
    pub checking: bool,
    pub downloading: bool,
    pub available: bool,
    pub auto_install: bool,
    pub version: String,
    pub status: String,
    pub asset: Option<ReleaseAsset>,
}

#[derive(Clone, Debug)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub verified: bool,
}

#[derive(Debug)]
pub enum UpdateEvent {
    Checked {
        manual: bool,
        result: Result<UpdateCheck, String>,
    },
    Downloaded(Result<PathBuf, String>),
}

#[derive(Debug)]
pub struct UpdateCheck {
    pub tag: String,
    pub asset: Option<ReleaseAsset>,
    pub newer: bool,
}

#[derive(Debug, Deserialize)]
struct ReleaseManifest {
    #[serde(alias = "tag_name")]
    tag: String,
    assets: Vec<ManifestAsset>,
}

#[derive(Debug, Deserialize)]
struct ManifestAsset {
    name: String,
    #[serde(alias = "browser_download_url")]
    path: String,
}

pub fn check(current_version: String, manual: bool, sender: Sender<UpdateEvent>) {
    thread::spawn(move || {
        let result = check_blocking(&current_version).map_err(|error| error.to_string());
        let _ = sender.send(UpdateEvent::Checked { manual, result });
    });
}

pub fn download(asset: ReleaseAsset, sender: Sender<UpdateEvent>) {
    thread::spawn(move || {
        let result = download_blocking(&asset).map_err(|error| error.to_string());
        let _ = sender.send(UpdateEvent::Downloaded(result));
    });
}

pub fn check_blocking(current_version: &str) -> Result<UpdateCheck, String> {
    if should_use_secure_hashtree() {
        return check_secure_blocking(current_version);
    }

    let manifest_urls = manifest_urls();
    let mut last_error = None;
    for manifest_url in manifest_urls {
        match fetch_manifest(&manifest_url) {
            Ok(manifest) => {
                let tag = manifest.tag.clone();
                return Ok(UpdateCheck {
                    asset: preferred_linux_asset(&manifest, &manifest_url),
                    newer: version_is_newer(&tag, current_version),
                    tag,
                });
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "No update manifest URL configured".to_string()))
}

pub fn download_blocking(asset: &ReleaseAsset) -> Result<PathBuf, String> {
    if asset.verified {
        return download_secure_blocking(asset);
    }
    download_asset(asset)
}

fn should_use_secure_hashtree() -> bool {
    std::env::var("NVPN_UPDATE_MANIFEST_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_none()
}

fn check_secure_blocking(current_version: &str) -> Result<UpdateCheck, String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("Could not start update runtime: {error}"))?;
    runtime.block_on(check_secure(current_version))
}

fn download_secure_blocking(asset: &ReleaseAsset) -> Result<PathBuf, String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("Could not start update runtime: {error}"))?;
    runtime.block_on(download_secure(asset))
}

async fn build_secure_updater() -> Result<HashtreeUpdater<NostrRootResolver, BlossomStore>, String>
{
    let resolver = NostrRootResolver::new(NostrResolverConfig {
        relays: update_relays(),
        resolve_timeout: Duration::from_secs(UPDATE_TOTAL_TIMEOUT_SECS.parse::<u64>().unwrap_or(8)),
        secret_key: None,
    })
    .await
    .map_err(|error| format!("Could not connect to Nostr release relays: {error}"))?;
    let blossom = BlossomClient::new_empty(nostr::Keys::generate())
        .with_read_servers(blossom_read_servers())
        .with_timeout(Duration::from_secs(
            UPDATE_DOWNLOAD_TIMEOUT_SECS.parse::<u64>().unwrap_or(180),
        ));
    let store = Arc::new(BlossomStore::new(blossom));
    let tree = HashTree::new(HashTreeConfig::new(store).public());
    Ok(HashtreeUpdater::new(resolver, tree))
}

async fn check_secure(current_version: &str) -> Result<UpdateCheck, String> {
    let updater = build_secure_updater().await?;
    let mut check = updater
        .check(UpdateCheckOptions {
            reference: secure_update_ref()?,
            current_version: current_version.to_string(),
            target: UpdateTarget::new(current_target()),
            ..UpdateCheckOptions::default()
        })
        .await
        .map_err(|error| format!("Could not resolve signed hashtree release: {error}"))?;
    let asset = preferred_secure_linux_asset(&check.manifest)
        .ok_or_else(|| "Signed release has no Linux desktop asset".to_string())?;
    check.asset = Some(asset.clone());
    let tag = display_manifest_tag(&check.manifest);
    Ok(UpdateCheck {
        tag,
        asset: Some(ReleaseAsset {
            name: asset.name,
            url: SECURE_SOURCE_URL.to_string(),
            verified: true,
        }),
        newer: check.update_available,
    })
}

async fn download_secure(asset: &ReleaseAsset) -> Result<PathBuf, String> {
    let updater = build_secure_updater().await?;
    let mut check = updater
        .check(UpdateCheckOptions {
            reference: secure_update_ref()?,
            current_version: "0.0.0".to_string(),
            target: UpdateTarget::new(current_target()),
            ..UpdateCheckOptions::default()
        })
        .await
        .map_err(|error| format!("Could not resolve signed hashtree release: {error}"))?;
    let selected = preferred_secure_linux_asset(&check.manifest)
        .ok_or_else(|| "Signed release has no Linux desktop asset".to_string())?;
    if selected.name != asset.name {
        return Err(format!(
            "Signed latest release changed from {} to {}; please check again",
            asset.name, selected.name
        ));
    }
    check.asset = Some(selected.clone());
    let downloaded = updater
        .download(&check, DownloadOptions::default(), None)
        .await
        .map_err(|error| format!("Could not download verified update: {error}"))?;
    let destination = update_download_dir().join(&selected.name);
    write_downloaded_asset(&destination, &downloaded.bytes)?;
    maybe_make_executable_and_open(&destination, &selected.name)?;
    Ok(destination)
}

fn manifest_urls() -> Vec<String> {
    manifest_urls_for(
        std::env::var("NVPN_UPDATE_MANIFEST_URL")
            .ok()
            .filter(|value| !value.trim().is_empty()),
    )
}

fn manifest_urls_for(override_url: Option<String>) -> Vec<String> {
    if let Some(override_url) = override_url.filter(|value| !value.trim().is_empty()) {
        return vec![override_url];
    }
    vec![
        HTREE_MANIFEST_URL.to_string(),
        GITHUB_LATEST_RELEASE_URL.to_string(),
    ]
}

fn secure_update_ref() -> Result<UpdateRef, String> {
    let raw = std::env::var("NVPN_UPDATE_HTREE_REF")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| HTREE_UPDATE_REF.to_string());
    UpdateRef::parse(&raw).map_err(|error| format!("Invalid update hashtree ref: {error}"))
}

fn update_relays() -> Vec<String> {
    split_env_csv("NVPN_UPDATE_RELAYS").unwrap_or_else(|| {
        DEFAULT_UPDATE_RELAYS
            .iter()
            .map(|value| (*value).to_string())
            .collect()
    })
}

fn blossom_read_servers() -> Vec<String> {
    split_env_csv("NVPN_UPDATE_BLOSSOM_SERVERS").unwrap_or_else(|| {
        DEFAULT_BLOSSOM_READ_SERVERS
            .iter()
            .map(|value| (*value).to_string())
            .collect()
    })
}

fn split_env_csv(name: &str) -> Option<Vec<String>> {
    let values = std::env::var(name)
        .ok()?
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn fetch_manifest(manifest_url: &str) -> Result<ReleaseManifest, String> {
    let mut command = Command::new("curl");
    command.args([
        "-fsSL",
        "--connect-timeout",
        UPDATE_CONNECT_TIMEOUT_SECS,
        "--max-time",
        UPDATE_TOTAL_TIMEOUT_SECS,
    ]);
    if manifest_url.contains("api.github.com") {
        command
            .arg("-H")
            .arg("Accept: application/vnd.github+json")
            .arg("-H")
            .arg(format!("User-Agent: {UPDATE_USER_AGENT}"));
    }
    let output = command
        .arg(manifest_url)
        .output()
        .map_err(|error| format!("Could not run curl: {error}"))?;
    if !output.status.success() {
        return Err(command_error("Update check failed", &output));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Could not read release manifest: {error}"))
}

fn preferred_linux_asset(manifest: &ReleaseManifest, manifest_url: &str) -> Option<ReleaseAsset> {
    preferred_asset_patterns()
        .iter()
        .find_map(|pattern| {
            manifest
                .assets
                .iter()
                .find(|asset| asset.name.ends_with(pattern))
        })
        .or_else(|| {
            manifest.assets.iter().find(|asset| {
                asset.name.contains("-linux-")
                    && (asset.name.ends_with(".AppImage") || asset.name.ends_with(".deb"))
            })
        })
        .map(|asset| ReleaseAsset {
            name: asset.name.clone(),
            url: manifest_asset_url(manifest_url, &asset.path),
            verified: false,
        })
}

fn preferred_secure_linux_asset(manifest: &UpdateManifest) -> Option<UpdateAsset> {
    preferred_asset_patterns()
        .iter()
        .find_map(|pattern| {
            manifest
                .assets
                .iter()
                .find(|asset| asset.name.ends_with(pattern))
        })
        .or_else(|| {
            manifest.assets.iter().find(|asset| {
                asset.name.contains("-linux-")
                    && (asset.name.ends_with(".AppImage") || asset.name.ends_with(".deb"))
            })
        })
        .cloned()
}

fn current_target() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        "unknown-linux"
    }
}

fn display_manifest_tag(manifest: &UpdateManifest) -> String {
    manifest
        .tag
        .clone()
        .filter(|tag| !tag.trim().is_empty())
        .unwrap_or_else(|| format!("v{}", manifest.effective_version()))
}

fn preferred_asset_patterns() -> &'static [&'static str] {
    #[cfg(target_arch = "x86_64")]
    {
        &["-linux-x64.AppImage", "-linux-x64.deb"]
    }
    #[cfg(target_arch = "aarch64")]
    {
        &["-linux-arm64.AppImage", "-linux-arm64.deb"]
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        &[".AppImage", ".deb"]
    }
}

fn manifest_asset_url(manifest_url: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    if path.starts_with("file://") {
        return path.to_string();
    }
    let base = manifest_url
        .rsplit_once('/')
        .map(|(base, _)| base)
        .unwrap_or(manifest_url);
    format!("{}/{}", base, path.trim_start_matches('/'))
}

fn download_asset(asset: &ReleaseAsset) -> Result<PathBuf, String> {
    let destination = update_download_dir().join(&asset.name);
    let parent = destination
        .parent()
        .ok_or_else(|| "Download folder unavailable".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create download folder: {error}"))?;
    if destination.exists() {
        fs::remove_file(&destination)
            .map_err(|error| format!("Could not replace old download: {error}"))?;
    }

    let output = Command::new("curl")
        .arg("-fL")
        .arg("--connect-timeout")
        .arg(UPDATE_CONNECT_TIMEOUT_SECS)
        .arg("--max-time")
        .arg(UPDATE_DOWNLOAD_TIMEOUT_SECS)
        .arg("-o")
        .arg(&destination)
        .arg(&asset.url)
        .output()
        .map_err(|error| format!("Could not run curl: {error}"))?;
    if !output.status.success() {
        return Err(command_error("Update download failed", &output));
    }

    maybe_make_executable_and_open(&destination, &asset.name)?;
    Ok(destination)
}

fn write_downloaded_asset(destination: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "Download folder unavailable".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create download folder: {error}"))?;
    if destination.exists() {
        fs::remove_file(destination)
            .map_err(|error| format!("Could not replace old download: {error}"))?;
    }
    fs::write(destination, bytes).map_err(|error| format!("Could not write update: {error}"))
}

fn maybe_make_executable_and_open(destination: &PathBuf, asset_name: &str) -> Result<(), String> {
    if asset_name.ends_with(".AppImage") {
        let mut permissions = fs::metadata(destination)
            .map_err(|error| format!("Downloaded update unavailable: {error}"))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(destination, permissions)
            .map_err(|error| format!("Could not make AppImage executable: {error}"))?;
    }

    if std::env::var("NVPN_UPDATE_SKIP_OPEN").ok().as_deref() != Some("1") {
        let _ = Command::new("xdg-open").arg(destination).spawn();
    }
    Ok(())
}

fn update_download_dir() -> PathBuf {
    std::env::var("NVPN_UPDATE_DOWNLOAD_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("NostrVpnDownloads"))
}

fn command_error(prefix: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stderr.is_empty() {
        format!("{prefix}: {stderr}")
    } else if !stdout.is_empty() {
        format!("{prefix}: {stdout}")
    } else {
        format!("{prefix}: exit {}", output.status)
    }
}

fn version_is_newer(candidate: &str, current: &str) -> bool {
    let left = version_parts(candidate);
    let right = version_parts(current);
    for index in 0..left.len().max(right.len()) {
        let left_value = left.get(index).copied().unwrap_or_default();
        let right_value = right.get(index).copied().unwrap_or_default();
        if left_value != right_value {
            return left_value > right_value;
        }
    }
    false
}

fn version_parts(value: &str) -> Vec<u32> {
    value
        .trim_matches(|ch: char| ch == 'v' || ch == 'V' || ch.is_whitespace())
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u32>().unwrap_or_default())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semver_like_tags() {
        assert!(version_is_newer("v0.3.24", "0.3.23"));
        assert!(version_is_newer("1.0.0", "0.9.9"));
        assert!(!version_is_newer("0.3.23", "0.3.23"));
        assert!(!version_is_newer("0.3.22", "0.3.23"));
    }

    #[test]
    fn prefers_linux_desktop_asset_for_arch() {
        let manifest = ReleaseManifest {
            tag: "v1.2.3".to_string(),
            assets: vec![
                ManifestAsset {
                    name: "nvpn-v1.2.3-x86_64-unknown-linux-musl.tar.gz".to_string(),
                    path: "assets/cli.tar.gz".to_string(),
                },
                ManifestAsset {
                    name: preferred_test_asset_name().to_string(),
                    path: "assets/app".to_string(),
                },
            ],
        };
        let asset = preferred_linux_asset(&manifest, HTREE_MANIFEST_URL).expect("asset");
        assert_eq!(asset.name, preferred_test_asset_name());
        assert!(asset.url.ends_with("/assets/app"));
        assert!(!asset.verified);
    }

    #[test]
    fn checks_htree_before_github_by_default() {
        assert_eq!(
            manifest_urls_for(None),
            vec![
                HTREE_MANIFEST_URL.to_string(),
                GITHUB_LATEST_RELEASE_URL.to_string(),
            ]
        );
    }

    #[test]
    fn parses_github_release_manifest() {
        let manifest: ReleaseManifest = serde_json::from_str(
            r#"{
                "tag_name": "v4.0.12",
                "assets": [
                    {
                        "name": "nostr-vpn-v4.0.12-linux-x64.deb",
                        "browser_download_url": "https://example.invalid/app.deb"
                    }
                ]
            }"#,
        )
        .expect("manifest");

        assert_eq!(manifest.tag, "v4.0.12");
        assert_eq!(manifest.assets[0].path, "https://example.invalid/app.deb");
    }

    #[test]
    fn secure_linux_asset_selection_ignores_cli_archives() {
        let manifest: UpdateManifest = serde_json::from_str(&format!(
            r#"{{
                "tag": "v4.0.48",
                "assets": [
                    {{ "name": "nvpn-v4.0.48-{target}.tar.gz", "path": "assets/cli.tgz" }},
                    {{ "name": "{app}", "path": "assets/app" }}
                ]
            }}"#,
            target = current_target(),
            app = preferred_test_asset_name(),
        ))
        .expect("manifest");

        let asset = preferred_secure_linux_asset(&manifest).expect("linux app asset");
        assert_eq!(asset.path, "assets/app");
    }

    #[cfg(target_arch = "x86_64")]
    fn preferred_test_asset_name() -> &'static str {
        "nostr-vpn-v1.2.3-linux-x64.AppImage"
    }

    #[cfg(target_arch = "aarch64")]
    fn preferred_test_asset_name() -> &'static str {
        "nostr-vpn-v1.2.3-linux-arm64.AppImage"
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    fn preferred_test_asset_name() -> &'static str {
        "nostr-vpn-v1.2.3-linux.AppImage"
    }
}
