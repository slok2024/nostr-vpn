use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use hashtree_blossom::{BlossomClient, BlossomStore};
use hashtree_core::{HashTree, HashTreeConfig};
use hashtree_resolver::nostr::{NostrResolverConfig, NostrRootResolver};
use hashtree_updater::{
    DownloadOptions, HashtreeUpdater, UpdateAsset, UpdateCheckOptions, UpdateManifest, UpdateRef,
    UpdateTarget,
};
use serde::{Deserialize, Serialize};

use super::{PRODUCT_VERSION, UpdateArgs, UpdateSource};

const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/mmalmi/nostr-vpn/releases/latest";
const HTREE_MANIFEST_URL: &str = "https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Fnostr-vpn/latest/release.json";
const HTREE_UPDATE_REF: &str = "htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Fnostr-vpn/latest";
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
const SECURE_SOURCE_NAME: &str = "hashtree-nostr-blossom";
const LEGACY_HTREE_SOURCE_NAME: &str = "legacy-htree-url";
const GITHUB_SOURCE_NAME: &str = "github";

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateMode {
    Cli,
    App,
}

impl UpdateMode {
    fn from_args(args: &UpdateArgs) -> Self {
        if args.app { Self::App } else { Self::Cli }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Cli => "nvpn CLI",
            Self::App => "Nostr VPN app",
        }
    }
}

#[derive(Debug, Serialize)]
struct UpdateJson<'a> {
    available: bool,
    current_version: &'a str,
    latest_version: String,
    tag: String,
    asset: String,
    source: &'a str,
    verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

struct LegacySelection {
    manifest: ReleaseManifest,
    asset: ReleaseAsset,
    asset_url: String,
    source_name: &'static str,
    update_available: bool,
}

pub(crate) async fn run_update(args: UpdateArgs) -> Result<()> {
    if should_use_secure_hashtree(args.source) {
        return run_secure_update(args).await;
    }

    run_legacy_update(args)
}

fn should_use_secure_hashtree(source: UpdateSource) -> bool {
    std::env::var("NVPN_UPDATE_MANIFEST_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_none()
        && !matches!(source, UpdateSource::Github)
}

fn run_legacy_update(args: UpdateArgs) -> Result<()> {
    let mode = UpdateMode::from_args(&args);
    let selection = legacy_selection(args.source, mode)?;

    if args.check {
        print_update_check(
            mode,
            selection.update_available,
            &selection.manifest.tag,
            &selection.asset.name,
            selection.source_name,
            false,
            Some(&selection.asset_url),
            args.json,
            None,
        )?;
        return Ok(());
    }

    if !selection.update_available && !args.force {
        print_up_to_date(
            mode,
            &selection.manifest.tag,
            selection.source_name,
            false,
            args.json,
        )?;
        return Ok(());
    }

    let temp_dir = create_temp_dir("nvpn-update")?;
    let archive_path = selected_download_path(
        args.download_dir.as_deref(),
        &selection.asset.name,
        &temp_dir,
    )?;
    download_asset(&selection.asset_url, &archive_path)?;

    if args.download_only || mode == UpdateMode::App {
        print_downloaded(
            DownloadedUpdate {
                available: selection.update_available,
                tag: &selection.manifest.tag,
                asset: &selection.asset.name,
                source: selection.source_name,
                verified: false,
                url: Some(&selection.asset_url),
                path: &archive_path,
            },
            args.json,
        )?;
        return Ok(());
    }

    install_cli_archive(&archive_path, &temp_dir, args.path.as_deref())?;
    let _ = fs::remove_dir_all(&temp_dir);

    println!(
        "updated nvpn at {} from {PRODUCT_VERSION} to {}",
        args.path
            .as_deref()
            .map_or_else(current_exe_display, |path| path.display().to_string()),
        selection.manifest.tag
    );
    Ok(())
}

fn legacy_selection(source: UpdateSource, mode: UpdateMode) -> Result<LegacySelection> {
    let (manifest_url, manifest) = fetch_first_manifest(source)?;
    let newer = version_is_newer(&manifest.tag, PRODUCT_VERSION);
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

async fn run_secure_update(args: UpdateArgs) -> Result<()> {
    let mode = UpdateMode::from_args(&args);
    let reference = secure_update_ref()?;
    let relays = update_relays();
    let blossom_servers = blossom_read_servers();
    let resolver = NostrRootResolver::new(NostrResolverConfig {
        relays,
        resolve_timeout: Duration::from_secs(
            UPDATE_MANIFEST_TIMEOUT_SECS.parse::<u64>().unwrap_or(8),
        ),
        secret_key: None,
    })
    .await
    .context("failed to connect to Nostr release relays")?;
    let blossom = BlossomClient::new_empty(nostr::Keys::generate())
        .with_read_servers(blossom_servers)
        .with_timeout(Duration::from_secs(
            UPDATE_DOWNLOAD_TIMEOUT_SECS.parse::<u64>().unwrap_or(180),
        ));
    let store = Arc::new(BlossomStore::new(blossom));
    let tree = HashTree::new(HashTreeConfig::new(store).public());
    let updater = HashtreeUpdater::new(resolver, tree);
    let mut check = updater
        .check(UpdateCheckOptions {
            reference,
            current_version: PRODUCT_VERSION.to_string(),
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
    let available = check.update_available;

    if args.check {
        print_update_check(
            mode,
            available,
            &tag,
            &asset.name,
            SECURE_SOURCE_NAME,
            true,
            None,
            args.json,
            None,
        )?;
        return Ok(());
    }

    if !available && !args.force {
        print_up_to_date(mode, &tag, SECURE_SOURCE_NAME, true, args.json)?;
        return Ok(());
    }

    let temp_dir = create_temp_dir("nvpn-update")?;
    let destination = selected_download_path(args.download_dir.as_deref(), &asset.name, &temp_dir)?;
    let downloaded = updater
        .download(
            &check,
            DownloadOptions {
                max_size: None,
                ..DownloadOptions::default()
            },
            None,
        )
        .await
        .with_context(|| format!("failed to download verified hashtree asset {}", asset.name))?;
    write_downloaded_asset(&destination, &downloaded.bytes)?;

    if args.download_only || mode == UpdateMode::App {
        print_downloaded(
            DownloadedUpdate {
                available,
                tag: &tag,
                asset: &asset.name,
                source: SECURE_SOURCE_NAME,
                verified: true,
                url: None,
                path: &destination,
            },
            args.json,
        )?;
        return Ok(());
    }

    install_cli_archive(&destination, &temp_dir, args.path.as_deref())?;
    let _ = fs::remove_dir_all(&temp_dir);

    println!(
        "updated nvpn at {} from {PRODUCT_VERSION} to {tag}",
        args.path
            .as_deref()
            .map_or_else(current_exe_display, |path| path.display().to_string())
    );
    Ok(())
}

fn fetch_first_manifest(source: UpdateSource) -> Result<(String, ReleaseManifest)> {
    let mut last_error = None;
    for url in manifest_urls(source) {
        match fetch_manifest(&url) {
            Ok(manifest) => return Ok((url, manifest)),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("no update manifest URL configured")))
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

fn manifest_urls(source: UpdateSource) -> Vec<String> {
    manifest_urls_for(
        source,
        std::env::var("NVPN_UPDATE_MANIFEST_URL")
            .ok()
            .filter(|value| !value.trim().is_empty()),
    )
}

fn manifest_urls_for(source: UpdateSource, override_url: Option<String>) -> Vec<String> {
    if let Some(override_url) = override_url.filter(|value| !value.trim().is_empty()) {
        return vec![override_url];
    }

    match source {
        UpdateSource::Auto => vec![
            HTREE_MANIFEST_URL.to_string(),
            GITHUB_LATEST_RELEASE_URL.to_string(),
        ],
        UpdateSource::Github => vec![GITHUB_LATEST_RELEASE_URL.to_string()],
        UpdateSource::Hashtree => vec![HTREE_MANIFEST_URL.to_string()],
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

fn preferred_asset(manifest: &ReleaseManifest, mode: UpdateMode) -> Option<ReleaseAsset> {
    match mode {
        UpdateMode::Cli => preferred_cli_asset(manifest),
        UpdateMode::App => preferred_legacy_app_asset(manifest),
    }
}

fn preferred_legacy_app_asset(manifest: &ReleaseManifest) -> Option<ReleaseAsset> {
    manifest
        .assets
        .iter()
        .find(|asset| app_asset_name_matches_current_target(&asset.name))
        .cloned()
}

fn preferred_secure_asset(manifest: &UpdateManifest, mode: UpdateMode) -> Option<UpdateAsset> {
    match mode {
        UpdateMode::Cli => preferred_secure_cli_asset(manifest),
        UpdateMode::App => preferred_secure_app_asset(manifest),
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

fn current_target() -> &'static str {
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

fn selected_download_path(
    download_dir: Option<&Path>,
    asset_name: &str,
    temp_dir: &Path,
) -> Result<PathBuf> {
    let file_name = safe_file_name(asset_name);
    let parent = download_dir.unwrap_or(temp_dir);
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    Ok(parent.join(file_name))
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

fn install_cli_archive(
    archive_path: &Path,
    temp_dir: &Path,
    destination: Option<&Path>,
) -> Result<()> {
    extract_archive(archive_path, temp_dir)?;
    let binary = find_nvpn_binary(temp_dir)?;
    let destination = destination
        .map(Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(|| {
            std::env::current_exe().context("failed to resolve current executable")
        })?;
    install_parent(&destination)?;
    install_bundled_helpers(&binary, &destination)?;
    install_binary(&binary, &destination)
}

fn current_exe_display() -> String {
    std::env::current_exe().map_or_else(
        |_| "<current executable>".to_string(),
        |path| path.display().to_string(),
    )
}

#[allow(clippy::too_many_arguments)]
fn print_update_check(
    mode: UpdateMode,
    available: bool,
    tag: &str,
    asset: &str,
    source: &'static str,
    verified: bool,
    url: Option<&str>,
    json: bool,
    path: Option<&Path>,
) -> Result<()> {
    if json {
        print_update_json(UpdateJson {
            available,
            current_version: PRODUCT_VERSION,
            latest_version: tag.trim_start_matches('v').to_string(),
            tag: tag.to_string(),
            asset: asset.to_string(),
            source,
            verified,
            url: url.map(ToOwned::to_owned),
            path: path.map(|value| value.display().to_string()),
        })?;
        return Ok(());
    }

    if available {
        println!("update available: {PRODUCT_VERSION} -> {tag}");
    } else {
        println!("{} {PRODUCT_VERSION} is up to date", mode.noun());
    }
    println!("asset={asset}");
    println!("source={source}");
    println!("verified={verified}");
    if let Some(url) = url {
        println!("url={url}");
    }
    if let Some(path) = path {
        println!("path={}", path.display());
    }
    Ok(())
}

struct DownloadedUpdate<'a> {
    available: bool,
    tag: &'a str,
    asset: &'a str,
    source: &'static str,
    verified: bool,
    url: Option<&'a str>,
    path: &'a Path,
}

fn print_downloaded(download: DownloadedUpdate<'_>, json: bool) -> Result<()> {
    if json {
        print_update_json(UpdateJson {
            available: download.available,
            current_version: PRODUCT_VERSION,
            latest_version: download.tag.trim_start_matches('v').to_string(),
            tag: download.tag.to_string(),
            asset: download.asset.to_string(),
            source: download.source,
            verified: download.verified,
            url: download.url.map(ToOwned::to_owned),
            path: Some(download.path.display().to_string()),
        })?;
        return Ok(());
    }
    println!("downloaded {}", download.asset);
    println!("path={}", download.path.display());
    println!("source={}", download.source);
    println!("verified={}", download.verified);
    if let Some(url) = download.url {
        println!("url={url}");
    }
    Ok(())
}

fn print_up_to_date(
    mode: UpdateMode,
    tag: &str,
    source: &'static str,
    verified: bool,
    json: bool,
) -> Result<()> {
    if json {
        print_update_json(UpdateJson {
            available: false,
            current_version: PRODUCT_VERSION,
            latest_version: tag.trim_start_matches('v').to_string(),
            tag: tag.to_string(),
            asset: String::new(),
            source,
            verified,
            url: None,
            path: None,
        })?;
        return Ok(());
    }
    println!("{} {PRODUCT_VERSION} is up to date", mode.noun());
    Ok(())
}

fn print_update_json(output: UpdateJson<'_>) -> Result<()> {
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

fn extract_archive(archive_path: &Path, destination: &Path) -> Result<()> {
    let mut command = Command::new("tar");
    if archive_path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.ends_with(".tar.gz") || name.ends_with(".tgz"))
    {
        command.arg("-xzf");
    } else {
        command.arg("-xf");
    }
    let output = command
        .arg(archive_path)
        .arg("-C")
        .arg(destination)
        .output()
        .with_context(|| format!("failed to extract {}", archive_path.display()))?;
    if !output.status.success() {
        return Err(anyhow!(
            "{}",
            command_error("archive extraction failed", &output)
        ));
    }
    Ok(())
}

fn find_nvpn_binary(root: &Path) -> Result<PathBuf> {
    let binary_name = if cfg!(target_os = "windows") {
        "nvpn.exe"
    } else {
        "nvpn"
    };
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in
            fs::read_dir(&path).with_context(|| format!("failed to read {}", path.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file()
                && path.file_name().and_then(|value| value.to_str()) == Some(binary_name)
            {
                return Ok(path);
            }
        }
    }
    Err(anyhow!("downloaded archive did not contain {binary_name}"))
}

fn install_binary(source: &Path, destination: &Path) -> Result<()> {
    let parent = install_parent(destination)?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create directory {}", parent.display()))?;

    let temp_path = parent.join(format!(
        ".nvpn-update-{}-{}{}",
        std::process::id(),
        unix_timestamp(),
        std::env::consts::EXE_SUFFIX
    ));
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    fs::copy(source, &temp_path).with_context(|| {
        format!(
            "failed to copy {} to {}",
            source.display(),
            temp_path.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o755)).with_context(|| {
            format!(
                "failed to set executable permissions on {}",
                temp_path.display()
            )
        })?;
    }
    #[cfg(target_os = "windows")]
    if destination.exists() {
        fs::remove_file(destination)
            .with_context(|| format!("failed to replace {}", destination.display()))?;
    }
    fs::rename(&temp_path, destination).with_context(|| {
        format!(
            "failed to move {} into {}",
            temp_path.display(),
            destination.display()
        )
    })?;
    Ok(())
}

fn install_parent(destination: &Path) -> Result<&Path> {
    if destination.as_os_str().is_empty() {
        return Err(anyhow!("install path must not be empty"));
    }
    if destination.is_dir() {
        return Err(anyhow!(
            "install path points to a directory: {}",
            destination.display()
        ));
    }
    destination.parent().ok_or_else(|| {
        anyhow!(
            "install path must include parent directory: {}",
            destination.display()
        )
    })
}

#[cfg(target_os = "windows")]
fn install_bundled_helpers(source_binary: &Path, destination_binary: &Path) -> Result<()> {
    let Some(source_dir) = bundled_helper_source_dir(source_binary) else {
        return Ok(());
    };
    let destination_dir = install_parent(destination_binary)?.join("binaries");
    install_bundled_helper_dir(&source_dir, &destination_dir)?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn install_bundled_helpers(_source_binary: &Path, _destination_binary: &Path) -> Result<()> {
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
fn bundled_helper_source_dir(source_binary: &Path) -> Option<PathBuf> {
    source_binary
        .parent()
        .map(|parent| parent.join("binaries"))
        .filter(|path| path.is_dir())
}

#[cfg(any(target_os = "windows", test))]
fn install_bundled_helper_dir(source_dir: &Path, destination_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut installed = Vec::new();
    let mut stack = vec![source_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in
            fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            let relative = path
                .strip_prefix(source_dir)
                .with_context(|| format!("failed to relativize {}", path.display()))?;
            let destination = destination_dir.join(relative);
            install_helper_file(&path, &destination)?;
            installed.push(destination);
        }
    }
    Ok(installed)
}

#[cfg(any(target_os = "windows", test))]
fn install_helper_file(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination.parent().ok_or_else(|| {
        anyhow!(
            "helper install path must include parent directory: {}",
            destination.display()
        )
    })?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create directory {}", parent.display()))?;

    let file_name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("helper");
    let temp_path = destination.with_file_name(format!(
        ".nvpn-update-{}-{}-{file_name}",
        std::process::id(),
        unix_timestamp(),
    ));
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    fs::copy(source, &temp_path).with_context(|| {
        format!(
            "failed to copy helper {} to {}",
            source.display(),
            temp_path.display()
        )
    })?;
    if destination.exists() {
        fs::remove_file(destination)
            .with_context(|| format!("failed to replace {}", destination.display()))?;
    }
    fs::rename(&temp_path, destination).with_context(|| {
        format!(
            "failed to move helper {} into {}",
            temp_path.display(),
            destination.display()
        )
    })?;
    Ok(())
}

fn create_temp_dir(prefix: &str) -> Result<PathBuf> {
    let base = std::env::temp_dir();
    for attempt in 0..128u32 {
        let path = base.join(format!(
            "{prefix}-{}-{}-{attempt}",
            std::process::id(),
            unix_timestamp()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("failed to create {}", path.display()));
            }
        }
    }
    Err(anyhow!("failed to allocate temporary update directory"))
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

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("nvpn-updater-{label}-{nonce}"))
    }

    #[test]
    fn parses_github_release_manifest() {
        let manifest: ReleaseManifest = serde_json::from_str(
            r#"{
                "tag_name": "v4.0.12",
                "assets": [
                    {
                        "name": "nvpn-v4.0.12-aarch64-apple-darwin.tar.gz",
                        "browser_download_url": "https://example.invalid/nvpn.tgz"
                    }
                ]
            }"#,
        )
        .expect("manifest");

        assert_eq!(manifest.tag, "v4.0.12");
        assert_eq!(manifest.assets[0].path, "https://example.invalid/nvpn.tgz");
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

    #[test]
    fn auto_source_checks_htree_before_github() {
        assert_eq!(
            manifest_urls_for(UpdateSource::Auto, None),
            vec![
                HTREE_MANIFEST_URL.to_string(),
                GITHUB_LATEST_RELEASE_URL.to_string(),
            ]
        );
    }

    #[test]
    fn secure_cli_selection_prefers_cli_archive_over_desktop_app() {
        let archive_ext = if cfg!(target_os = "windows") {
            ".zip"
        } else {
            ".tar.gz"
        };
        let manifest: UpdateManifest = serde_json::from_str(&format!(
            r#"{{
                "tag": "v4.0.48",
                "assets": [
                    {{ "name": "nostr-vpn-v4.0.48-macos-arm64.app.tar.gz", "path": "assets/app.tgz" }},
                    {{ "name": "nvpn-v4.0.48-{target}{archive_ext}", "path": "assets/cli.tgz" }}
                ]
            }}"#,
            target = current_target(),
            archive_ext = archive_ext,
        ))
        .expect("manifest");

        let asset = preferred_secure_asset(&manifest, UpdateMode::Cli).expect("cli asset");
        assert_eq!(asset.path, "assets/cli.tgz");
    }

    #[test]
    fn secure_app_selection_ignores_cli_archives() {
        let manifest: UpdateManifest = serde_json::from_str(
            r#"{
                "tag": "v4.0.48",
                "assets": [
                    { "name": "nvpn-v4.0.48-aarch64-apple-darwin.tar.gz", "path": "assets/cli.tgz" },
                    { "name": "nostr-vpn-v4.0.48-macos-arm64.app.tar.gz", "path": "assets/app.tgz" }
                ]
            }"#,
        )
        .expect("manifest");

        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let asset = preferred_secure_asset(&manifest, UpdateMode::App).expect("app asset");
            assert_eq!(asset.path, "assets/app.tgz");
        }
    }

    #[test]
    fn app_asset_name_matching_rejects_cli_archive() {
        assert!(!app_asset_name_matches_current_target(&format!(
            "nvpn-v4.0.48-{}.tar.gz",
            current_target()
        )));
    }

    #[test]
    fn compares_semver_like_tags() {
        assert!(version_is_newer("v4.0.13", "4.0.12"));
        assert!(!version_is_newer("v4.0.12", "4.0.12"));
        assert!(!version_is_newer("v4.0.11", "4.0.12"));
    }

    #[test]
    fn helper_source_dir_is_next_to_downloaded_binary() {
        let root = unique_temp_dir("helper-source");
        let binary = root.join("archive").join("nvpn.exe");
        let helper_dir = binary.parent().expect("binary parent").join("binaries");
        fs::create_dir_all(&helper_dir).expect("create helper dir");
        fs::write(&binary, b"exe").expect("write binary placeholder");
        fs::write(helper_dir.join("wintun.dll"), b"dll").expect("write helper placeholder");

        assert_eq!(bundled_helper_source_dir(&binary), Some(helper_dir));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn installs_bundled_helper_files_recursively() {
        let root = unique_temp_dir("helper-install");
        let source_dir = root.join("source").join("binaries");
        let destination_dir = root.join("install").join("binaries");
        fs::create_dir_all(source_dir.join("drivers")).expect("create source dirs");
        fs::create_dir_all(&destination_dir).expect("create destination dir");
        fs::write(source_dir.join("wintun.dll"), b"new").expect("write wintun helper");
        fs::write(source_dir.join("drivers").join("extra.dll"), b"extra")
            .expect("write nested helper");
        fs::write(destination_dir.join("wintun.dll"), b"old").expect("write old helper");

        let installed = install_bundled_helper_dir(&source_dir, &destination_dir)
            .expect("install bundled helpers");

        assert!(installed.contains(&destination_dir.join("wintun.dll")));
        assert!(installed.contains(&destination_dir.join("drivers").join("extra.dll")));
        assert_eq!(
            fs::read(destination_dir.join("wintun.dll")).expect("read installed wintun"),
            b"new"
        );
        assert_eq!(
            fs::read(destination_dir.join("drivers").join("extra.dll"))
                .expect("read installed nested helper"),
            b"extra"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
