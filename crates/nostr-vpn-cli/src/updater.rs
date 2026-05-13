use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use super::{PRODUCT_VERSION, UpdateArgs, UpdateSource};

const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/mmalmi/nostr-vpn/releases/latest";
const HTREE_MANIFEST_URL: &str = "https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Fnostr-vpn/latest/release.json";
const UPDATE_CONNECT_TIMEOUT_SECS: &str = "4";
const UPDATE_MANIFEST_TIMEOUT_SECS: &str = "8";
const UPDATE_DOWNLOAD_TIMEOUT_SECS: &str = "180";
const UPDATE_USER_AGENT: &str = "nvpn-updater";

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

pub(crate) fn run_update(args: UpdateArgs) -> Result<()> {
    let (manifest_url, manifest) = fetch_first_manifest(args.source)?;
    let newer = version_is_newer(&manifest.tag, PRODUCT_VERSION);
    let asset = preferred_cli_asset(&manifest).ok_or_else(|| {
        anyhow!(
            "release {} has no nvpn CLI asset for {}",
            manifest.tag,
            current_target()
        )
    })?;
    let asset_url = manifest_asset_url(&manifest_url, &asset.path);

    if args.check {
        if newer {
            println!("update available: {PRODUCT_VERSION} -> {}", manifest.tag);
        } else {
            println!("nvpn {PRODUCT_VERSION} is up to date");
        }
        println!("asset={}", asset.name);
        println!("url={asset_url}");
        return Ok(());
    }

    if !newer && !args.force {
        println!("nvpn {PRODUCT_VERSION} is up to date");
        return Ok(());
    }

    let destination = args.path.map(Ok).unwrap_or_else(|| {
        std::env::current_exe().context("failed to resolve current executable")
    })?;
    let temp_dir = create_temp_dir("nvpn-update")?;
    let archive_path = temp_dir.join(safe_file_name(&asset.name));
    download_asset(&asset_url, &archive_path)?;
    extract_archive(&archive_path, &temp_dir)?;
    let binary = find_nvpn_binary(&temp_dir)?;
    install_binary(&binary, &destination)?;
    let _ = fs::remove_dir_all(&temp_dir);

    println!(
        "updated nvpn at {} from {PRODUCT_VERSION} to {}",
        destination.display(),
        manifest.tag
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

fn manifest_urls(source: UpdateSource) -> Vec<String> {
    if let Some(override_url) = std::env::var("NVPN_UPDATE_MANIFEST_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return vec![override_url];
    }

    match source {
        UpdateSource::Auto => vec![
            GITHUB_LATEST_RELEASE_URL.to_string(),
            HTREE_MANIFEST_URL.to_string(),
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
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
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
    if destination.as_os_str().is_empty() {
        return Err(anyhow!("install path must not be empty"));
    }
    if destination.is_dir() {
        return Err(anyhow!(
            "install path points to a directory: {}",
            destination.display()
        ));
    }
    let parent = destination.parent().ok_or_else(|| {
        anyhow!(
            "install path must include parent directory: {}",
            destination.display()
        )
    })?;
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
    fn compares_semver_like_tags() {
        assert!(version_is_newer("v4.0.13", "4.0.12"));
        assert!(!version_is_newer("v4.0.12", "4.0.12"));
        assert!(!version_is_newer("v4.0.11", "4.0.12"));
    }
}
