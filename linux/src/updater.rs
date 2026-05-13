use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::Sender;
use std::thread;

use serde::Deserialize;

const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/mmalmi/nostr-vpn/releases/latest";
const HTREE_MANIFEST_URL: &str = "https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Fnostr-vpn/latest/release.json";
const UPDATE_CONNECT_TIMEOUT_SECS: &str = "4";
const UPDATE_TOTAL_TIMEOUT_SECS: &str = "8";
const UPDATE_DOWNLOAD_TIMEOUT_SECS: &str = "180";
const UPDATE_USER_AGENT: &str = "nvpn-updater";

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
    download_asset(asset)
}

fn manifest_urls() -> Vec<String> {
    if let Some(override_url) = std::env::var("NVPN_UPDATE_MANIFEST_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return vec![override_url];
    }
    vec![
        GITHUB_LATEST_RELEASE_URL.to_string(),
        HTREE_MANIFEST_URL.to_string(),
    ]
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
        })
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

    if asset.name.ends_with(".AppImage") {
        let mut permissions = fs::metadata(&destination)
            .map_err(|error| format!("Downloaded update unavailable: {error}"))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&destination, permissions)
            .map_err(|error| format!("Could not make AppImage executable: {error}"))?;
    }

    if std::env::var("NVPN_UPDATE_SKIP_OPEN").ok().as_deref() != Some("1") {
        let _ = Command::new("xdg-open").arg(&destination).spawn();
    }
    Ok(destination)
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
