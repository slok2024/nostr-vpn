use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, anyhow};
use nostr_vpn_core::config::{AppConfig, maybe_autoconfigure_node};

use crate::{
    CliStatusResponse, DEFAULT_STATIC_DIR, NVPN_BIN_ENV, ServerState, is_already_running_message,
    is_not_running_message, nvpn_gui_iface_override,
};

pub(crate) fn load_config(path: &Path) -> Result<AppConfig> {
    let mut config = if path.exists() {
        AppConfig::load(path).with_context(|| format!("failed to load {}", path.display()))?
    } else {
        AppConfig::generated()
    };
    config.ensure_defaults();
    maybe_autoconfigure_node(&mut config);
    Ok(config)
}

pub(crate) fn ensure_config_exists(path: &Path) -> Result<()> {
    let mut config = if path.exists() {
        AppConfig::load(path).with_context(|| format!("failed to load {}", path.display()))?
    } else {
        AppConfig::generated()
    };
    config.ensure_defaults();
    maybe_autoconfigure_node(&mut config);
    save_config(path, &config)
}

pub(crate) fn save_config(path: &Path, config: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    config
        .save(path)
        .with_context(|| format!("failed to save {}", path.display()))
}

pub(crate) fn fetch_cli_status(state: &ServerState) -> Result<CliStatusResponse> {
    let config_path = config_path_arg(&state.config_path)?;
    let output = run_nvpn_command(
        state,
        &[
            "status",
            "--json",
            "--discover-secs",
            "0",
            "--config",
            config_path,
        ],
    )?;
    if !output.status.success() {
        return Err(command_failure("nvpn status", &output));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_text = extract_json_document(&stdout)?;
    serde_json::from_str::<CliStatusResponse>(json_text)
        .context("failed to parse `nvpn status --json` output")
}

pub(crate) fn connect_vpn_inner(state: &ServerState) -> Result<()> {
    let config_path = config_path_arg(&state.config_path)?;
    let status = fetch_cli_status(state).ok();
    if status.as_ref().is_some_and(|value| value.daemon.running) {
        let output = run_nvpn_command(state, &["resume", "--config", config_path])?;
        if !output.status.success() {
            let failure = command_failure("nvpn resume", &output);
            if !is_not_running_message(&failure.to_string()) {
                return Err(failure);
            }
        }
        return Ok(());
    }

    if let Some(iface) = nvpn_gui_iface_override() {
        let output = run_nvpn_command(
            state,
            &[
                "start",
                "--daemon",
                "--connect",
                "--iface",
                &iface,
                "--config",
                config_path,
            ],
        )?;
        if !output.status.success() {
            let failure = command_failure("nvpn start", &output);
            if !is_already_running_message(&failure.to_string()) {
                return Err(failure);
            }
        }
    } else {
        let output = run_nvpn_command(
            state,
            &["start", "--daemon", "--connect", "--config", config_path],
        )?;
        if !output.status.success() {
            let failure = command_failure("nvpn start", &output);
            if !is_already_running_message(&failure.to_string()) {
                return Err(failure);
            }
        }
    }
    Ok(())
}

pub(crate) fn disconnect_vpn_inner(state: &ServerState) -> Result<()> {
    let config_path = config_path_arg(&state.config_path)?;
    let status = fetch_cli_status(state).ok();
    if !status.as_ref().is_some_and(|value| value.daemon.running) {
        return Ok(());
    }
    let output = run_nvpn_command(state, &["pause", "--config", config_path])?;
    if output.status.success() {
        return Ok(());
    }
    let failure = command_failure("nvpn pause", &output);
    if is_not_running_message(&failure.to_string()) {
        return Ok(());
    }
    Err(failure)
}

pub(crate) fn reload_daemon_if_running(state: &ServerState) -> Result<()> {
    let status = fetch_cli_status(state).ok();
    if !status.as_ref().is_some_and(|value| value.daemon.running) {
        return Ok(());
    }
    let config_path = config_path_arg(&state.config_path)?;
    let output = run_nvpn_command(state, &["reload", "--config", config_path])?;
    if output.status.success() {
        return Ok(());
    }
    let failure = command_failure("nvpn reload", &output);
    if is_not_running_message(&failure.to_string()) {
        return Ok(());
    }
    Err(failure)
}

fn run_nvpn_command(state: &ServerState, args: &[&str]) -> Result<Output> {
    Command::new(&state.nvpn_bin)
        .args(args)
        .output()
        .with_context(|| {
            format!(
                "failed to execute {} {}",
                state.nvpn_bin.display(),
                args.join(" ")
            )
        })
}

pub(crate) fn resolve_nvpn_cli_path(override_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return validate_executable(path);
    }
    if let Some(path) = env::var_os(NVPN_BIN_ENV) {
        return validate_executable(PathBuf::from(path));
    }
    if let Some(path_var) = env::var_os("PATH") {
        for dir in env::split_paths(&path_var) {
            let candidate = dir.join(nvpn_binary_name());
            if candidate.exists()
                && let Ok(validated) = validate_executable(candidate)
            {
                return Ok(validated);
            }
        }
    }
    Err(anyhow!(
        "nvpn CLI binary not found; set {} or add nvpn to PATH",
        NVPN_BIN_ENV
    ))
}

#[cfg(target_os = "windows")]
fn nvpn_binary_name() -> &'static str {
    "nvpn.exe"
}

#[cfg(not(target_os = "windows"))]
fn nvpn_binary_name() -> &'static str {
    "nvpn"
}

fn validate_executable(path: PathBuf) -> Result<PathBuf> {
    let canonical = fs::canonicalize(&path)
        .with_context(|| format!("failed to canonicalize {}", path.display()))?;
    let metadata = fs::metadata(&canonical)
        .with_context(|| format!("failed to inspect {}", canonical.display()))?;
    if !metadata.is_file() {
        return Err(anyhow!("{} is not a file", canonical.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(anyhow!("{} is not executable", canonical.display()));
        }
    }
    Ok(canonical)
}

pub(crate) fn default_config_path() -> PathBuf {
    if let Some(config_dir) = dirs::config_dir() {
        return config_dir.join("nvpn").join("config.toml");
    }
    PathBuf::from("nvpn.toml")
}

pub(crate) fn discover_static_dir() -> Option<PathBuf> {
    let path = PathBuf::from(DEFAULT_STATIC_DIR);
    path.join("index.html").exists().then_some(path)
}

fn config_path_arg(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| anyhow!("config path is not valid UTF-8"))
}

fn command_failure(command: &str, output: &Output) -> anyhow::Error {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    anyhow!(
        "{command} failed\nstdout: {}\nstderr: {}",
        stdout.trim(),
        stderr.trim()
    )
}

fn extract_json_document(raw: &str) -> Result<&str> {
    let start = raw
        .find('{')
        .ok_or_else(|| anyhow!("command output did not contain JSON start"))?;
    let end = raw
        .rfind('}')
        .ok_or_else(|| anyhow!("command output did not contain JSON end"))?;
    Ok(&raw[start..=end])
}
