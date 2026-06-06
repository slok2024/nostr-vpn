use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use hashtree_blossom::{BlossomClient, BlossomStore};
use hashtree_core::{HashTree, HashTreeConfig};
use hashtree_resolver::{
    Keys as HashtreeResolverKeys,
    nostr::{NostrResolverConfig, NostrRootResolver},
};
use hashtree_updater::{
    DownloadOptions, HashtreeUpdater, UpdateAsset, UpdateCheck, UpdateCheckOptions, UpdateManifest,
    UpdateRef, UpdateTarget,
};
use serde::{Deserialize, Serialize};

pub const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/mmalmi/nostr-vpn/releases/latest";
pub const HTREE_MANIFEST_URL: &str = "https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Fnostr-vpn/latest/release.json";
pub const HTREE_UPDATE_REF: &str = "htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Fnostr-vpn/latest";
pub const SECURE_SOURCE_NAME: &str = "hashtree-nostr-blossom";
pub const LEGACY_HTREE_SOURCE_NAME: &str = "legacy-htree-url";
pub const GITHUB_SOURCE_NAME: &str = "github";

const UPDATE_CONNECT_TIMEOUT_SECS: &str = "4";
const UPDATE_MANIFEST_TIMEOUT_SECS: &str = "8";
const UPDATE_DOWNLOAD_TIMEOUT_SECS: &str = "180";
const UPDATE_USER_AGENT: &str = "nvpn-updater";
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductUpdateMode {
    Cli,
    App,
}

impl ProductUpdateMode {
    #[must_use]
    pub fn noun(self) -> &'static str {
        match self {
            Self::Cli => "nvpn CLI",
            Self::App => "Nostr VPN app",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductUpdateSource {
    Auto,
    Github,
    Hashtree,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProductUpdateResult {
    pub available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub tag: String,
    pub asset: String,
    pub source: String,
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReleaseManifest {
    #[serde(alias = "tag_name")]
    tag: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Clone, Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    #[serde(alias = "browser_download_url")]
    path: String,
}

struct LegacySelection {
    manifest: ReleaseManifest,
    asset: ReleaseAsset,
    asset_url: String,
    source_name: &'static str,
    update_available: bool,
}

type SecureUpdater = HashtreeUpdater<NostrRootResolver, BlossomStore>;

struct SecureSelection {
    updater: SecureUpdater,
    check: UpdateCheck,
    asset: UpdateAsset,
    tag: String,
    update_available: bool,
}

enum UpdateSelection {
    Secure(Box<SecureSelection>),
    Legacy(LegacySelection),
}

pub fn check_product_update_blocking(
    current_version: &str,
    mode: ProductUpdateMode,
    source: ProductUpdateSource,
) -> Result<ProductUpdateResult> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start update runtime")?;
    runtime.block_on(check_product_update(current_version, mode, source))
}

pub async fn check_product_update(
    current_version: &str,
    mode: ProductUpdateMode,
    source: ProductUpdateSource,
) -> Result<ProductUpdateResult> {
    let selection = select_update(current_version, mode, source).await?;
    Ok(result_from_selection(current_version, &selection, None))
}

pub fn download_product_update_blocking(
    current_version: &str,
    mode: ProductUpdateMode,
    source: ProductUpdateSource,
    download_dir: Option<&Path>,
) -> Result<ProductUpdateResult> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start update runtime")?;
    runtime.block_on(download_product_update(
        current_version,
        mode,
        source,
        download_dir,
    ))
}

pub async fn download_product_update(
    current_version: &str,
    mode: ProductUpdateMode,
    source: ProductUpdateSource,
    download_dir: Option<&Path>,
) -> Result<ProductUpdateResult> {
    let selection = select_update(current_version, mode, source).await?;
    let destination = download_selection(&selection, download_dir).await?;
    Ok(result_from_selection(
        current_version,
        &selection,
        Some(&destination),
    ))
}

async fn select_update(
    current_version: &str,
    mode: ProductUpdateMode,
    source: ProductUpdateSource,
) -> Result<UpdateSelection> {
    if !should_use_secure_hashtree(source) {
        return legacy_selection(current_version, source, mode).map(UpdateSelection::Legacy);
    }

    let secure = secure_selection(current_version, mode).await;
    let selection = match secure {
        Ok(selection) => selection,
        Err(error) if should_try_github_fallback(source, false) => {
            return legacy_selection(current_version, ProductUpdateSource::Github, mode)
                .map(UpdateSelection::Legacy)
                .with_context(|| format!("secure hashtree update check failed: {error}"));
        }
        Err(error) => return Err(error),
    };

    if should_try_github_fallback(source, selection.update_available)
        && let Ok(legacy) = legacy_selection(current_version, ProductUpdateSource::Github, mode)
        && legacy.update_available
    {
        return Ok(UpdateSelection::Legacy(legacy));
    }

    Ok(UpdateSelection::Secure(Box::new(selection)))
}

fn result_from_selection(
    current_version: &str,
    selection: &UpdateSelection,
    path: Option<&Path>,
) -> ProductUpdateResult {
    match selection {
        UpdateSelection::Secure(selection) => ProductUpdateResult {
            available: selection.update_available,
            current_version: current_version.to_string(),
            latest_version: selection.tag.trim_start_matches('v').to_string(),
            tag: selection.tag.clone(),
            asset: selection.asset.name.clone(),
            source: SECURE_SOURCE_NAME.to_string(),
            verified: true,
            url: None,
            path: path.map(|value| value.display().to_string()),
        },
        UpdateSelection::Legacy(selection) => ProductUpdateResult {
            available: selection.update_available,
            current_version: current_version.to_string(),
            latest_version: selection.manifest.tag.trim_start_matches('v').to_string(),
            tag: selection.manifest.tag.clone(),
            asset: selection.asset.name.clone(),
            source: selection.source_name.to_string(),
            verified: false,
            url: Some(selection.asset_url.clone()),
            path: path.map(|value| value.display().to_string()),
        },
    }
}

async fn secure_selection(
    current_version: &str,
    mode: ProductUpdateMode,
) -> Result<SecureSelection> {
    let updater = build_secure_updater().await?;
    let mut check = updater
        .check(UpdateCheckOptions {
            reference: secure_update_ref()?,
            current_version: current_version.to_string(),
            target: UpdateTarget::new(current_target()),
            ..UpdateCheckOptions::default()
        })
        .await
        .context("failed to resolve signed hashtree release")?;
    let asset = preferred_secure_asset(&check.manifest, mode).ok_or_else(|| {
        anyhow!(
            "release {} has no {} asset for {}",
            check.manifest.effective_version(),
            mode.noun(),
            current_target()
        )
    })?;
    check.asset = Some(asset.clone());
    let tag = display_manifest_tag(&check.manifest);
    let update_available = check.update_available;
    Ok(SecureSelection {
        updater,
        check,
        asset,
        tag,
        update_available,
    })
}

async fn build_secure_updater() -> Result<SecureUpdater> {
    let resolver = NostrRootResolver::new(NostrResolverConfig {
        relays: update_relays(),
        resolve_timeout: Duration::from_secs(
            UPDATE_MANIFEST_TIMEOUT_SECS.parse::<u64>().unwrap_or(8),
        ),
        secret_key: None,
    })
    .await
    .context("failed to connect to Nostr release relays")?;
    let blossom = BlossomClient::new_empty(HashtreeResolverKeys::generate())
        .with_read_servers(blossom_read_servers())
        .with_timeout(Duration::from_secs(
            UPDATE_DOWNLOAD_TIMEOUT_SECS.parse::<u64>().unwrap_or(180),
        ));
    let store = Arc::new(BlossomStore::new(blossom));
    let tree = HashTree::new(HashTreeConfig::new(store).public());
    Ok(HashtreeUpdater::new(resolver, tree))
}

fn legacy_selection(
    current_version: &str,
    source: ProductUpdateSource,
    mode: ProductUpdateMode,
) -> Result<LegacySelection> {
    let (manifest_url, manifest) = fetch_first_manifest(source)?;
    let newer = version_is_newer(&manifest.tag, current_version);
    let asset = preferred_asset(&manifest, mode).ok_or_else(|| {
        anyhow!(
            "release {} has no {} asset for {}",
            manifest.tag,
            mode.noun(),
            current_target()
        )
    })?;
    let asset_url = manifest_asset_url(&manifest_url, &asset.path);
    let source_name = if manifest_url.contains("api.github.com") {
        GITHUB_SOURCE_NAME
    } else {
        LEGACY_HTREE_SOURCE_NAME
    };

    Ok(LegacySelection {
        manifest,
        asset,
        asset_url,
        source_name,
        update_available: newer,
    })
}

async fn download_selection(
    selection: &UpdateSelection,
    download_dir: Option<&Path>,
) -> Result<PathBuf> {
    match selection {
        UpdateSelection::Secure(selection) => {
            let destination = selected_download_path(download_dir, &selection.asset.name)?;
            let downloaded = selection
                .updater
                .download(
                    &selection.check,
                    DownloadOptions {
                        max_size: None,
                        ..DownloadOptions::default()
                    },
                    None,
                )
                .await
                .with_context(|| {
                    format!(
                        "failed to download verified hashtree asset {}",
                        selection.asset.name
                    )
                })?;
            write_downloaded_asset(&destination, &downloaded.bytes)?;
            Ok(destination)
        }
        UpdateSelection::Legacy(selection) => {
            let destination = selected_download_path(download_dir, &selection.asset.name)?;
            download_asset(&selection.asset_url, &destination)?;
            Ok(destination)
        }
    }
}

fn should_use_secure_hashtree(source: ProductUpdateSource) -> bool {
    std::env::var("NVPN_UPDATE_MANIFEST_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_none()
        && !matches!(source, ProductUpdateSource::Github)
}

#[must_use]
pub fn should_try_github_fallback(source: ProductUpdateSource, secure_available: bool) -> bool {
    matches!(source, ProductUpdateSource::Auto) && !secure_available
}

fn secure_update_ref() -> Result<UpdateRef> {
    let raw = std::env::var("NVPN_UPDATE_HTREE_REF")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| HTREE_UPDATE_REF.to_string());
    UpdateRef::parse(&raw).with_context(|| format!("invalid update hashtree ref: {raw}"))
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

fn fetch_first_manifest(source: ProductUpdateSource) -> Result<(String, ReleaseManifest)> {
    let mut last_error = None;
    for url in manifest_urls(source) {
        match fetch_manifest(&url) {
            Ok(manifest) => return Ok((url, manifest)),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("no update manifest URL configured")))
}

fn manifest_urls(source: ProductUpdateSource) -> Vec<String> {
    manifest_urls_for(
        source,
        std::env::var("NVPN_UPDATE_MANIFEST_URL")
            .ok()
            .filter(|value| !value.trim().is_empty()),
    )
}

fn manifest_urls_for(source: ProductUpdateSource, override_url: Option<String>) -> Vec<String> {
    if let Some(override_url) = override_url.filter(|value| !value.trim().is_empty()) {
        return vec![override_url];
    }

    match source {
        ProductUpdateSource::Auto => vec![
            HTREE_MANIFEST_URL.to_string(),
            GITHUB_LATEST_RELEASE_URL.to_string(),
        ],
        ProductUpdateSource::Github => vec![GITHUB_LATEST_RELEASE_URL.to_string()],
        ProductUpdateSource::Hashtree => vec![HTREE_MANIFEST_URL.to_string()],
    }
}

fn fetch_manifest(url: &str) -> Result<ReleaseManifest> {
    let mut command = Command::new("curl");
    command.args([
        "-fsSL",
        "--connect-timeout",
        UPDATE_CONNECT_TIMEOUT_SECS,
        "--max-time",
        UPDATE_MANIFEST_TIMEOUT_SECS,
    ]);
    if url.contains("api.github.com") {
        command
            .arg("-H")
            .arg("Accept: application/vnd.github+json")
            .arg("-H")
            .arg(format!("User-Agent: {UPDATE_USER_AGENT}"));
    }
    let output = command
        .arg(url)
        .output()
        .with_context(|| format!("failed to run curl for {url}"))?;
    if !output.status.success() {
        return Err(anyhow!("{}", command_error("update check failed", &output)));
    }
    serde_json::from_slice(&output.stdout).context("failed to parse release manifest")
}

fn preferred_asset(manifest: &ReleaseManifest, mode: ProductUpdateMode) -> Option<ReleaseAsset> {
    match mode {
        ProductUpdateMode::Cli => preferred_cli_asset(manifest),
        ProductUpdateMode::App => preferred_legacy_app_asset(manifest),
    }
}

fn preferred_cli_asset(manifest: &ReleaseManifest) -> Option<ReleaseAsset> {
    let target = current_target();
    let archive_ext = if cfg!(target_os = "windows") {
        ".zip"
    } else {
        ".tar.gz"
    };
    let exact = format!("nvpn-{}-{target}{archive_ext}", manifest.tag);
    let unversioned = format!("nvpn-{target}{archive_ext}");

    manifest
        .assets
        .iter()
        .find(|asset| asset.name == exact)
        .or_else(|| {
            manifest
                .assets
                .iter()
                .find(|asset| asset.name == unversioned)
        })
        .or_else(|| {
            manifest.assets.iter().find(|asset| {
                asset.name.starts_with("nvpn-")
                    && asset.name.contains(target)
                    && asset.name.ends_with(archive_ext)
            })
        })
        .cloned()
}

fn preferred_legacy_app_asset(manifest: &ReleaseManifest) -> Option<ReleaseAsset> {
    manifest
        .assets
        .iter()
        .find(|asset| app_asset_name_matches_current_target(&asset.name))
        .cloned()
}

fn preferred_secure_asset(
    manifest: &UpdateManifest,
    mode: ProductUpdateMode,
) -> Option<UpdateAsset> {
    match mode {
        ProductUpdateMode::Cli => preferred_secure_cli_asset(manifest),
        ProductUpdateMode::App => preferred_secure_app_asset(manifest),
    }
}

fn preferred_secure_cli_asset(manifest: &UpdateManifest) -> Option<UpdateAsset> {
    let tag = display_manifest_tag(manifest);
    let target = current_target();
    let archive_ext = if cfg!(target_os = "windows") {
        ".zip"
    } else {
        ".tar.gz"
    };
    let exact = format!("nvpn-{tag}-{target}{archive_ext}");
    let unversioned = format!("nvpn-{target}{archive_ext}");

    manifest
        .assets
        .iter()
        .find(|asset| asset.name == exact)
        .or_else(|| {
            manifest
                .assets
                .iter()
                .find(|asset| asset.name == unversioned)
        })
        .or_else(|| {
            manifest.assets.iter().find(|asset| {
                asset.name.starts_with("nvpn-")
                    && asset.name.contains(target)
                    && asset.name.ends_with(archive_ext)
            })
        })
        .cloned()
}

fn preferred_secure_app_asset(manifest: &UpdateManifest) -> Option<UpdateAsset> {
    manifest
        .assets
        .iter()
        .find(|asset| app_asset_name_matches_current_target(&asset.name))
        .cloned()
}

fn app_asset_name_matches_current_target(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        lower.ends_with("-macos-arm64.app.tar.gz")
            || lower.ends_with("-macos-arm64.dmg")
            || lower.ends_with("-macos-arm64.zip")
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        lower.ends_with("-linux-x64.deb") || lower.ends_with("-linux-x64.appimage")
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        lower.ends_with("-linux-arm64.deb") || lower.ends_with("-linux-arm64.appimage")
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        lower.ends_with("-windows-x64-setup.exe")
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        let _ = lower;
        false
    }
}

fn display_manifest_tag(manifest: &UpdateManifest) -> String {
    manifest
        .tag
        .clone()
        .filter(|tag| !tag.trim().is_empty())
        .unwrap_or_else(|| format!("v{}", manifest.effective_version()))
}

#[must_use]
pub fn current_target() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-musl"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-musl"
    }
    #[cfg(all(target_os = "linux", target_arch = "arm"))]
    {
        "arm-unknown-linux-musleabihf"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "arm"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        "unsupported"
    }
}

fn manifest_asset_url(manifest_url: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") || path.starts_with("file://") {
        return path.to_string();
    }
    let base = manifest_url
        .rsplit_once('/')
        .map(|(base, _)| base)
        .unwrap_or(manifest_url);
    format!("{}/{}", base, path.trim_start_matches('/'))
}

fn selected_download_path(download_dir: Option<&Path>, asset_name: &str) -> Result<PathBuf> {
    let file_name = safe_file_name(asset_name);
    let parent = download_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    fs::create_dir_all(&parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;
    Ok(parent.join(file_name))
}

fn download_asset(url: &str, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let output = Command::new("curl")
        .arg("-fL")
        .arg("--connect-timeout")
        .arg(UPDATE_CONNECT_TIMEOUT_SECS)
        .arg("--max-time")
        .arg(UPDATE_DOWNLOAD_TIMEOUT_SECS)
        .arg("-o")
        .arg(destination)
        .arg(url)
        .output()
        .with_context(|| format!("failed to run curl for {url}"))?;
    if !output.status.success() {
        return Err(anyhow!(
            "{}",
            command_error("update download failed", &output)
        ));
    }
    Ok(())
}

fn write_downloaded_asset(destination: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(destination, bytes).with_context(|| {
        format!(
            "failed to write verified update to {}",
            destination.display()
        )
    })
}

fn safe_file_name(name: &str) -> String {
    let value = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() {
        "nvpn-update-archive".to_string()
    } else {
        value
    }
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

#[must_use]
pub fn version_is_newer(candidate: &str, current: &str) -> bool {
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
    fn auto_source_checks_htree_before_github() {
        assert_eq!(
            manifest_urls_for(ProductUpdateSource::Auto, None),
            vec![
                HTREE_MANIFEST_URL.to_string(),
                GITHUB_LATEST_RELEASE_URL.to_string(),
            ]
        );
    }

    #[test]
    fn auto_source_can_cross_check_github_when_secure_hashtree_is_not_newer() {
        assert!(should_try_github_fallback(ProductUpdateSource::Auto, false));
        assert!(!should_try_github_fallback(ProductUpdateSource::Auto, true));
        assert!(!should_try_github_fallback(
            ProductUpdateSource::Hashtree,
            false
        ));
        assert!(!should_try_github_fallback(
            ProductUpdateSource::Github,
            false
        ));
    }

    #[test]
    fn compares_semver_like_tags() {
        assert!(version_is_newer("v4.0.55", "4.0.52"));
        assert!(version_is_newer("v4.0.13", "4.0.12"));
        assert!(!version_is_newer("v4.0.12", "4.0.12"));
        assert!(!version_is_newer("v4.0.11", "4.0.12"));
    }

    #[test]
    fn resolves_relative_manifest_asset_urls() {
        assert_eq!(
            manifest_asset_url(
                "https://example.invalid/latest/release.json",
                "assets/nvpn.tgz"
            ),
            "https://example.invalid/latest/assets/nvpn.tgz"
        );
    }
}
