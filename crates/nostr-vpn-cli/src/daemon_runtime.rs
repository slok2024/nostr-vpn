use super::*;

const DAEMON_LOG_MAX_BYTES: u64 = 8 * 1024 * 1024;
const DAEMON_LOG_RETAIN_BYTES: u64 = 2 * 1024 * 1024;
const DAEMON_LOG_COMPACT_CHECK_SECS: u64 = 60;

pub(crate) fn daemon_pid_file_path(config_path: &Path) -> PathBuf {
    let parent = config_path
        .parent()
        .map_or_else(|| Path::new(".").to_path_buf(), PathBuf::from);
    parent.join("daemon.pid")
}

pub(crate) fn visible_daemon_state_for_status(
    running: bool,
    state: Option<&DaemonRuntimeState>,
) -> Option<DaemonRuntimeState> {
    if running { state.cloned() } else { None }
}

pub(crate) fn daemon_log_file_path(config_path: &Path) -> PathBuf {
    let parent = config_path
        .parent()
        .map_or_else(|| Path::new(".").to_path_buf(), PathBuf::from);
    parent.join("daemon.log")
}

pub(crate) fn redirect_stdio_to_daemon_log(config_path: &Path) -> Result<()> {
    let log_path = daemon_log_file_path(config_path);
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open {}", log_path.display()))?;
    let _ = set_daemon_runtime_file_permissions(&log_path);

    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let fd = log_file.as_raw_fd();
        unsafe {
            if libc::dup2(fd, libc::STDOUT_FILENO) < 0 {
                return Err(std::io::Error::last_os_error())
                    .context("failed to redirect stdout to daemon log");
            }
            if libc::dup2(fd, libc::STDERR_FILENO) < 0 {
                return Err(std::io::Error::last_os_error())
                    .context("failed to redirect stderr to daemon log");
            }
        }
    }

    #[cfg(windows)]
    {
        // Windows Service Manager doesn't allocate a console for services, so
        // stdout/stderr go nowhere by default. Point both at the daemon log
        // via SetStdHandle. Anything tracing-subscriber and eprintln! emit
        // ends up in the same file as macOS/Linux daemons.
        use std::os::windows::io::AsRawHandle;
        let handle = log_file.as_raw_handle();
        unsafe {
            // STD_OUTPUT_HANDLE = -11, STD_ERROR_HANDLE = -12
            // SetStdHandle from kernel32; signature: BOOL SetStdHandle(DWORD nStdHandle, HANDLE hHandle)
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn SetStdHandle(nStdHandle: u32, hHandle: *mut std::ffi::c_void) -> i32;
            }
            const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;
            const STD_ERROR_HANDLE: u32 = (-12i32) as u32;
            if SetStdHandle(STD_OUTPUT_HANDLE, handle as *mut _) == 0 {
                return Err(std::io::Error::last_os_error())
                    .context("failed to redirect stdout to daemon log");
            }
            if SetStdHandle(STD_ERROR_HANDLE, handle as *mut _) == 0 {
                return Err(std::io::Error::last_os_error())
                    .context("failed to redirect stderr to daemon log");
            }
        }
        // Keep the file alive for the lifetime of the daemon process — the
        // OS holds the handle, but dropping our File would close it.
        std::mem::forget(log_file);
    }

    Ok(())
}

pub(crate) fn compact_daemon_log_if_needed(config_path: &Path) -> Result<bool> {
    compact_log_file_if_needed(
        &daemon_log_file_path(config_path),
        DAEMON_LOG_MAX_BYTES,
        DAEMON_LOG_RETAIN_BYTES,
    )
}

pub(crate) fn daemon_log_compact_check_due(last_checked: &mut Instant) -> bool {
    if last_checked.elapsed() < Duration::from_secs(DAEMON_LOG_COMPACT_CHECK_SECS) {
        return false;
    }
    *last_checked = Instant::now();
    true
}

pub(crate) fn compact_log_file_if_needed(
    path: &Path,
    max_bytes: u64,
    retain_bytes: u64,
) -> Result<bool> {
    if max_bytes == 0 || retain_bytes == 0 || retain_bytes >= max_bytes {
        return Ok(false);
    }

    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
        }
    };
    let original_len = metadata.len();
    if original_len <= max_bytes {
        return Ok(false);
    }

    let retain_start = original_len.saturating_sub(retain_bytes);
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(retain_start))
        .with_context(|| format!("failed to seek {}", path.display()))?;
    let mut retained = Vec::with_capacity(retain_bytes as usize);
    std::io::Read::read_to_end(&mut file, &mut retained)
        .with_context(|| format!("failed to read {}", path.display()))?;
    if let Some(newline) = retained.iter().position(|byte| *byte == b'\n') {
        retained.drain(..=newline);
    }

    let header = format!(
        "[nvpn] daemon log compacted at {}; retained {} bytes from {} bytes\n",
        unix_timestamp(),
        retained.len(),
        original_len
    );
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("failed to compact {}", path.display()))?;
    std::io::Write::write_all(&mut file, header.as_bytes())
        .with_context(|| format!("failed to write compaction header to {}", path.display()))?;
    std::io::Write::write_all(&mut file, &retained)
        .with_context(|| format!("failed to write retained log tail to {}", path.display()))?;
    std::io::Write::flush(&mut file)
        .with_context(|| format!("failed to flush {}", path.display()))?;
    Ok(true)
}

pub(crate) fn daemon_state_file_path(config_path: &Path) -> PathBuf {
    let parent = config_path
        .parent()
        .map_or_else(|| Path::new(".").to_path_buf(), PathBuf::from);
    parent.join("daemon.state.json")
}

#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn daemon_network_cleanup_file_path(config_path: &Path) -> PathBuf {
    let parent = config_path
        .parent()
        .map_or_else(|| Path::new(".").to_path_buf(), PathBuf::from);
    parent.join("daemon.cleanup.json")
}

pub(crate) fn daemon_control_file_path(config_path: &Path) -> PathBuf {
    let parent = config_path
        .parent()
        .map_or_else(|| Path::new(".").to_path_buf(), PathBuf::from);
    parent.join("daemon.control")
}

pub(crate) fn daemon_control_result_file_path(config_path: &Path) -> PathBuf {
    let parent = config_path
        .parent()
        .map_or_else(|| Path::new(".").to_path_buf(), PathBuf::from);
    parent.join("daemon.control.result.json")
}

pub(crate) fn daemon_staged_config_file_path(config_path: &Path) -> PathBuf {
    let parent = config_path
        .parent()
        .map_or_else(|| Path::new(".").to_path_buf(), PathBuf::from);
    parent.join("config.pending.toml")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DaemonControlResult {
    request: String,
    ok: bool,
    error: Option<String>,
}

pub(crate) fn ensure_no_other_daemon_processes_for_config(
    config_path: &Path,
    current_pid: u32,
) -> Result<()> {
    let daemon_pids = daemon_candidate_pids(config_path, current_pid)?;

    if let Some(existing_pid) = daemon_pids.first().copied() {
        return Err(anyhow!("daemon already running with pid {}", existing_pid));
    }

    Ok(())
}

pub(crate) fn write_daemon_control_request(
    config_path: &Path,
    request: DaemonControlRequest,
) -> Result<()> {
    let control_file = daemon_control_file_path(config_path);
    if let Some(parent) = control_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&control_file, format!("{}\n", request.as_str())).with_context(|| {
        format!(
            "failed to write daemon control request {}",
            control_file.display()
        )
    })?;
    set_daemon_runtime_file_permissions(&control_file)?;
    if let Err(error) = persist_desired_daemon_vpn_enabled_for_request(config_path, request) {
        eprintln!(
            "daemon: failed to persist desired VPN state in {}: {}",
            config_path.display(),
            error
        );
    }
    project_daemon_vpn_enabled_request(config_path, request);
    Ok(())
}

pub(crate) fn persist_desired_daemon_vpn_enabled_for_request(
    config_path: &Path,
    request: DaemonControlRequest,
) -> Result<Option<bool>> {
    let vpn_enabled = match request {
        DaemonControlRequest::Pause => false,
        DaemonControlRequest::Resume => true,
        DaemonControlRequest::Reload | DaemonControlRequest::Stop => return Ok(None),
    };

    persist_desired_daemon_vpn_enabled(config_path, vpn_enabled)?;
    Ok(Some(vpn_enabled))
}

pub(crate) fn persist_desired_daemon_vpn_enabled(
    config_path: &Path,
    vpn_enabled: bool,
) -> Result<bool> {
    let mut app = load_or_default_config(config_path)?;
    persist_desired_daemon_vpn_enabled_in_config(&mut app, config_path, vpn_enabled)
}

pub(crate) fn persist_desired_daemon_vpn_enabled_in_config(
    app: &mut AppConfig,
    config_path: &Path,
    vpn_enabled: bool,
) -> Result<bool> {
    if app.autoconnect == vpn_enabled {
        return Ok(false);
    }

    app.autoconnect = vpn_enabled;
    app.ensure_defaults();
    maybe_autoconfigure_node(app);
    app.save(config_path)?;
    Ok(true)
}

fn project_daemon_vpn_enabled_request(config_path: &Path, request: DaemonControlRequest) {
    let Some(vpn_enabled) = (match request {
        DaemonControlRequest::Pause => Some(false),
        DaemonControlRequest::Resume => Some(true),
        DaemonControlRequest::Reload | DaemonControlRequest::Stop => None,
    }) else {
        return;
    };

    let state_file = daemon_state_file_path(config_path);
    let Ok(Some(mut state)) = read_daemon_state(&state_file) else {
        return;
    };

    state.updated_at = unix_timestamp();
    state.vpn_enabled = vpn_enabled;
    state.vpn_status = match (vpn_enabled, state.vpn_active) {
        (true, true) if state.vpn_status == "Turning VPN off" || state.vpn_status == "Paused" => {
            "VPN on".to_string()
        }
        (true, true) => state.vpn_status,
        (true, false) => "Turning VPN on".to_string(),
        (false, true) => "Turning VPN off".to_string(),
        (false, false) => "Paused".to_string(),
    };

    if let Err(error) = write_daemon_state(&state_file, &state) {
        eprintln!(
            "daemon: failed to project VPN control state {}: {}",
            state_file.display(),
            error
        );
    }
}

pub(crate) fn clear_daemon_control_result(config_path: &Path) {
    let _ = fs::remove_file(daemon_control_result_file_path(config_path));
}

pub(crate) fn write_daemon_control_result(
    config_path: &Path,
    request: DaemonControlRequest,
    result: Result<()>,
) -> Result<()> {
    let result_file = daemon_control_result_file_path(config_path);
    if let Some(parent) = result_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let payload = match result {
        Ok(()) => DaemonControlResult {
            request: request.as_str().to_string(),
            ok: true,
            error: None,
        },
        Err(error) => DaemonControlResult {
            request: request.as_str().to_string(),
            ok: false,
            error: Some(error.to_string()),
        },
    };
    let raw = serde_json::to_vec_pretty(&payload)?;
    write_runtime_file_atomically(&result_file, &raw)
        .with_context(|| format!("failed to write {}", result_file.display()))?;
    set_daemon_runtime_file_permissions(&result_file)?;
    Ok(())
}

pub(crate) fn read_daemon_control_result(
    config_path: &Path,
) -> Result<Option<DaemonControlResult>> {
    let result_file = daemon_control_result_file_path(config_path);
    if !result_file.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&result_file)
        .with_context(|| format!("failed to read {}", result_file.display()))?;
    let parsed = serde_json::from_str::<DaemonControlResult>(&raw)
        .with_context(|| format!("failed to parse {}", result_file.display()))?;
    Ok(Some(parsed))
}

pub(crate) fn wait_for_daemon_control_result(
    config_path: &Path,
    request: DaemonControlRequest,
    timeout: Duration,
) -> Result<()> {
    let result_file = daemon_control_result_file_path(config_path);
    let started = Instant::now();
    while started.elapsed() < timeout {
        if let Some(result) = read_daemon_control_result(config_path)?
            && result.request == request.as_str()
        {
            let _ = fs::remove_file(&result_file);
            return if result.ok {
                Ok(())
            } else {
                Err(anyhow!(
                    "{}",
                    result
                        .error
                        .unwrap_or_else(|| "daemon control request failed".to_string())
                ))
            };
        }
        thread::sleep(Duration::from_millis(100));
    }

    Err(anyhow!(
        "daemon did not report result for {} within {}s; background service may be busy or stuck. try again, or restart/reinstall the app/service if it keeps happening",
        request.as_str(),
        timeout.as_secs()
    ))
}

pub(crate) fn stage_daemon_config_apply(config_path: &Path, source_path: &Path) -> Result<()> {
    let staged_path = daemon_staged_config_file_path(config_path);
    if let Some(parent) = staged_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut config = AppConfig::load(source_path)
        .with_context(|| format!("failed to load source config {}", source_path.display()))?;
    config.ensure_defaults();
    maybe_autoconfigure_node(&mut config);
    config
        .save(&staged_path)
        .with_context(|| format!("failed to stage config {}", staged_path.display()))?;
    set_private_cache_file_permissions(&staged_path)?;
    Ok(())
}

pub(crate) fn update_daemon_config_from_staged_request(config_path: &Path) -> Result<bool> {
    let staged_path = daemon_staged_config_file_path(config_path);
    if !staged_path.exists() {
        return Ok(false);
    }

    let result = apply_config_file(&staged_path, config_path);
    let _ = fs::remove_file(&staged_path);
    let _ = AppConfig::delete_persisted_secrets_for_path(&staged_path);
    result?;
    Ok(true)
}

pub(crate) fn request_daemon_stop(config_path: &Path) -> Result<()> {
    write_daemon_control_request(config_path, DaemonControlRequest::Stop)
}

pub(crate) fn request_daemon_reload(config_path: &Path) -> Result<()> {
    write_daemon_control_request(config_path, DaemonControlRequest::Reload)
}

pub(crate) fn apply_config_via_running_daemon(
    source_path: &Path,
    config_path: &Path,
) -> Result<()> {
    let status = daemon_status(config_path)?;
    if !status.running {
        #[cfg(target_os = "windows")]
        {
            let service_status = service_management::windows_query_service_status(false)?;
            if windows_should_apply_config_via_service(&service_status) {
                apply_config_file(source_path, config_path)?;
                service_management::windows_start_service_and_wait(true, Duration::from_secs(10))?;
                return Ok(());
            }
        }

        return Err(anyhow!("daemon: not running"));
    }

    clear_daemon_control_result(config_path);
    stage_daemon_config_apply(config_path, source_path)?;
    request_daemon_reload(config_path)?;
    wait_for_daemon_control_ack(
        config_path,
        crate::daemon_control_ack_timeout(DaemonControlRequest::Reload),
    )?;
    wait_for_daemon_control_result(
        config_path,
        DaemonControlRequest::Reload,
        crate::daemon_control_result_timeout(DaemonControlRequest::Reload),
    )
}

pub(crate) fn wait_for_daemon_control_ack(config_path: &Path, timeout: Duration) -> Result<()> {
    let control_file = daemon_control_file_path(config_path);
    let started = Instant::now();
    while started.elapsed() < timeout {
        if !control_file.exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    Err(anyhow!(
        "daemon did not acknowledge control request within {}s; background service may be busy or stuck. try again, or restart/reinstall the app/service if it keeps happening",
        timeout.as_secs()
    ))
}

#[cfg(test)]
pub(crate) fn wait_for_daemon_vpn_enabled(
    config_path: &Path,
    expected_enabled: bool,
    timeout: Duration,
) -> Result<()> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if let Ok(status) = daemon_status(config_path) {
            let current_state = status.state.as_ref();
            let current_enabled = current_state
                .map(|state| state.vpn_enabled)
                .unwrap_or(status.running);
            let resumed_waiting_for_participants = expected_enabled
                && current_state
                    .is_some_and(|state| state.vpn_status == WAITING_FOR_PARTICIPANTS_STATUS);
            if current_enabled == expected_enabled || resumed_waiting_for_participants {
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(100));
    }

    let verb = if expected_enabled { "resume" } else { "pause" };
    Err(anyhow!(
        "daemon acknowledged control request but did not {verb} within {}s; background service may be busy or stuck. try again, or restart/reinstall the app/service if it keeps happening",
        timeout.as_secs()
    ))
}

pub(crate) fn take_daemon_control_request(config_path: &Path) -> Option<DaemonControlRequest> {
    let control_file = daemon_control_file_path(config_path);
    let raw = match fs::read_to_string(&control_file) {
        Ok(raw) => raw,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "daemon: failed to read control request {}: {}",
                    control_file.display(),
                    error
                );
            }
            return None;
        }
    };

    let _ = fs::remove_file(&control_file);
    DaemonControlRequest::parse(&raw)
}

pub(crate) fn read_daemon_pid_record(path: &Path) -> Result<Option<DaemonPidRecord>> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read daemon pid file {}", path.display()))?;
    let parsed = serde_json::from_str::<DaemonPidRecord>(&raw)
        .with_context(|| format!("failed to parse daemon pid file {}", path.display()))?;
    Ok(Some(parsed))
}

pub(crate) fn write_daemon_pid_record(path: &Path, record: &DaemonPidRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(record)?;
    write_runtime_file_atomically(path, raw.as_bytes())
        .with_context(|| format!("failed to write daemon pid file {}", path.display()))?;
    set_daemon_runtime_file_permissions(path)?;
    Ok(())
}

pub(crate) fn read_daemon_state(path: &Path) -> Result<Option<DaemonRuntimeState>> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read(path)
        .with_context(|| format!("failed to read daemon state file {}", path.display()))?;
    match serde_json::from_slice::<DaemonRuntimeState>(&raw) {
        Ok(parsed) => Ok(Some(parsed)),
        Err(parse_error) => {
            let trimmed = trim_runtime_json_padding(&raw);
            if trimmed.len() != raw.len()
                && !trimmed.is_empty()
                && let Ok(parsed) = serde_json::from_slice::<DaemonRuntimeState>(trimmed)
            {
                if let Err(error) = write_runtime_file_atomically(path, trimmed) {
                    eprintln!(
                        "daemon: parsed padded state file {} but failed to rewrite clean copy: {}",
                        path.display(),
                        error
                    );
                } else {
                    let _ = set_daemon_runtime_file_permissions(path);
                }
                return Ok(Some(parsed));
            }

            quarantine_corrupt_runtime_file(path, "daemon state", &parse_error);
            Ok(None)
        }
    }
}

pub(crate) fn write_daemon_state(path: &Path, state: &DaemonRuntimeState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(state)?;
    write_runtime_file_atomically(path, raw.as_bytes())
        .with_context(|| format!("failed to write daemon state file {}", path.display()))?;
    set_daemon_runtime_file_permissions(path)?;
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn read_daemon_network_cleanup_state(
    path: &Path,
) -> Result<Option<MacosNetworkCleanupState>> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read(path)
        .with_context(|| format!("failed to read daemon cleanup file {}", path.display()))?;
    match serde_json::from_slice::<MacosNetworkCleanupState>(&raw) {
        Ok(parsed) => Ok(Some(parsed)),
        Err(parse_error) => {
            let trimmed = trim_runtime_json_padding(&raw);
            if trimmed.len() != raw.len()
                && !trimmed.is_empty()
                && let Ok(parsed) = serde_json::from_slice::<MacosNetworkCleanupState>(trimmed)
            {
                if let Err(error) = write_runtime_file_atomically(path, trimmed) {
                    eprintln!(
                        "daemon: parsed padded cleanup file {} but failed to rewrite clean copy: {}",
                        path.display(),
                        error
                    );
                } else {
                    let _ = set_daemon_runtime_file_permissions(path);
                }
                return Ok(Some(parsed));
            }

            quarantine_corrupt_runtime_file(path, "daemon cleanup", &parse_error);
            Ok(None)
        }
    }
}

#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn write_daemon_network_cleanup_state(
    path: &Path,
    state: &MacosNetworkCleanupState,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(state)?;
    write_runtime_file_atomically(path, raw.as_bytes())
        .with_context(|| format!("failed to write daemon cleanup file {}", path.display()))?;
    set_daemon_runtime_file_permissions(path)?;
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn remove_runtime_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

pub(crate) fn persist_daemon_network_cleanup_state(
    config_path: &Path,
    tunnel_runtime: &CliTunnelRuntime,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let path = daemon_network_cleanup_file_path(config_path);
        if let Some(state) = tunnel_runtime.macos_network_cleanup_state() {
            write_daemon_network_cleanup_state(&path, &state)?;
        } else {
            remove_runtime_file_if_exists(&path)?;
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (config_path, tunnel_runtime);
    }

    Ok(())
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn macos_route_delete_error_is_absent(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("not in table")
        || lower.contains("no such process")
        || lower.contains("no such route")
        || lower.contains("bad interface name")
        || lower.contains("not a network interface")
}

#[cfg(target_os = "macos")]
fn macos_cleanup_managed_routes(state: &MacosNetworkCleanupState) -> Vec<MacosManagedRoute> {
    let mut routes = state.managed_routes.clone();
    if routes.is_empty() {
        routes.extend(state.endpoint_bypass_routes.iter().cloned().map(|target| {
            MacosManagedRoute {
                target,
                gateway: None,
                interface: None,
            }
        }));
        if state.original_default_route.is_some() && !state.iface.trim().is_empty() {
            routes.extend(
                crate::macos_network::macos_tunnel_default_route_targets()
                    .iter()
                    .map(|target| MacosManagedRoute {
                        target: (*target).to_string(),
                        gateway: None,
                        interface: Some(state.iface.clone()),
                    }),
            );
        }
    }

    routes.sort_by(|left, right| {
        (
            left.target.as_str(),
            left.gateway.as_deref().unwrap_or(""),
            left.interface.as_deref().unwrap_or(""),
        )
            .cmp(&(
                right.target.as_str(),
                right.gateway.as_deref().unwrap_or(""),
                right.interface.as_deref().unwrap_or(""),
            ))
    });
    routes.dedup();
    routes
}

#[cfg(target_os = "macos")]
pub(crate) fn repair_legacy_macos_network_state(config_path: &Path) -> Result<bool> {
    let app = load_or_default_config(config_path)?;
    let mut repaired = false;

    if let Ok(tunnel_ip) = strip_cidr(&app.node.tunnel_ip).parse::<Ipv4Addr>() {
        let default_routes = macos_default_routes()?;
        let underlay_default =
            macos_underlay_default_route_from_routes(&default_routes).or_else(|| {
                crate::macos_network::macos_underlay_default_route_from_system()
                    .ok()
                    .flatten()
            });
        let mut tunnel_default_ifaces = Vec::new();

        for route in &default_routes {
            if !route.interface.starts_with("utun") {
                continue;
            }

            match macos_iface_has_ipv4_address(&route.interface, tunnel_ip) {
                Ok(true) => tunnel_default_ifaces.push(route.interface.clone()),
                Ok(false) => {}
                Err(error) => {
                    eprintln!(
                        "repair-network: failed to inspect macOS interface {}: {}",
                        route.interface, error
                    );
                }
            }
        }

        if tunnel_default_ifaces.is_empty() {
            tunnel_default_ifaces =
                crate::macos_network::macos_tunnel_interfaces_with_ipv4(tunnel_ip)?;
        }
        tunnel_default_ifaces.sort();
        tunnel_default_ifaces.dedup();

        let should_restore_underlay_default = default_routes.is_empty()
            || default_routes
                .iter()
                .all(|route| route.interface.starts_with("utun"));
        if let Some(underlay_default) = underlay_default {
            for iface in tunnel_default_ifaces {
                match delete_macos_default_route_for_interface(&iface) {
                    Ok(()) => repaired = true,
                    Err(error) if macos_route_delete_error_is_absent(&error.to_string()) => {}
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("failed to remove legacy macOS default route on {iface}")
                        });
                    }
                }
            }

            if repaired || should_restore_underlay_default {
                restore_macos_default_route(&underlay_default)
                    .context("failed to restore legacy macOS default route")?;
                repaired = true;
            }
        }
    }

    let cleanup_plan = legacy_macos_exit_cleanup_plan(&runtime_effective_advertised_routes(&app));
    if cleanup_plan.cleanup_pf_nat {
        if let Err(error) = cleanup_macos_pf_nat() {
            eprintln!("repair-network: failed to clear legacy macOS PF NAT rules: {error}");
        } else {
            repaired = true;
        }
    }

    Ok(repaired)
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LegacyMacosExitCleanupPlan {
    pub(crate) cleanup_pf_nat: bool,
    pub(crate) restore_ipv4_forwarding: bool,
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn legacy_macos_exit_cleanup_plan(routes: &[String]) -> LegacyMacosExitCleanupPlan {
    let route_families = linux_exit_node_default_route_families(routes);
    LegacyMacosExitCleanupPlan {
        cleanup_pf_nat: route_families.ipv4,
        // Legacy repair has no reliable saved owner for this global knob.
        // Internet Sharing and VM hosts also use it, so do not force it off.
        restore_ipv4_forwarding: false,
    }
}

pub(crate) fn repair_saved_network_state(config_path: &Path) -> Result<bool> {
    #[cfg(test)]
    crate::TEST_REPAIR_SAVED_NETWORK_STATE_CALLS.fetch_add(1, Ordering::Relaxed);

    #[cfg(target_os = "macos")]
    {
        let path = daemon_network_cleanup_file_path(config_path);
        let Some(state) = read_daemon_network_cleanup_state(&path)? else {
            return repair_legacy_macos_network_state(config_path);
        };
        let managed_routes = macos_cleanup_managed_routes(&state);
        let using_legacy_route_cleanup = state.managed_routes.is_empty();

        let mut failures = Vec::new();
        for route in &managed_routes {
            if let Err(error) = delete_macos_managed_route(
                &route.target,
                route.gateway.as_deref(),
                route.interface.as_deref(),
            ) && !macos_route_delete_error_is_absent(&error.to_string())
            {
                failures.push(format!("remove managed route {}: {error}", route.target));
            }
        }

        if let Some(route) = state.original_default_route.as_ref()
            && let Err(error) = restore_macos_default_route(route)
        {
            failures.push(format!("restore default route: {error}"));
        }

        if state.pf_was_enabled.is_some() {
            if let Err(error) = cleanup_macos_pf_nat() {
                failures.push(format!("remove PF NAT rules: {error}"));
            }
            if state.pf_was_enabled == Some(false)
                && let Err(error) = run_checked(ProcessCommand::new("pfctl").arg("-d"))
            {
                failures.push(format!("restore PF enabled state: {error}"));
            }
        }

        if using_legacy_route_cleanup
            && let Err(error) = repair_legacy_macos_network_state(config_path)
        {
            failures.push(format!("repair legacy macOS routes: {error}"));
        }

        if !failures.is_empty() {
            return Err(anyhow!(failures.join("; ")))
                .with_context(|| format!("failed to repair {}", path.display()));
        }

        remove_runtime_file_if_exists(&path)?;
        Ok(true)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = config_path;
        Ok(false)
    }
}

pub(crate) fn write_runtime_file_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("runtime file has no parent: {}", path.display()))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("runtime");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let mut temp_file = None;
    let mut temp_path = None;
    for attempt in 0..128u32 {
        let candidate = parent.join(format!(
            ".{file_name}.tmp-{}-{nonce}-{attempt}",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        match options.open(&candidate) {
            Ok(file) => {
                temp_file = Some(file);
                temp_path = Some(candidate);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to create temp runtime file {}", candidate.display())
                });
            }
        }
    }
    let temp_path = temp_path.ok_or_else(|| {
        anyhow!(
            "failed to allocate temp runtime file for {}",
            path.display()
        )
    })?;
    let mut file = temp_file.expect("temp file set with temp path");
    if let Err(error) = file.write_all(contents) {
        let _ = fs::remove_file(&temp_path);
        return Err(error)
            .with_context(|| format!("failed to write temp runtime file {}", temp_path.display()));
    }
    // These files are runtime status/control files. Keeping the replace atomic
    // matters for readers, but forcing every status update to durable storage
    // adds visible latency on macOS and is not needed for crash recovery.
    drop(file);
    fs::rename(&temp_path, path).with_context(|| {
        format!(
            "failed to replace {} with {}",
            path.display(),
            temp_path.display()
        )
    })?;
    Ok(())
}

pub(crate) fn trim_runtime_json_padding(raw: &[u8]) -> &[u8] {
    let start = raw
        .iter()
        .position(|byte| *byte != 0 && !byte.is_ascii_whitespace())
        .unwrap_or(raw.len());
    let end = raw
        .iter()
        .rposition(|byte| *byte != 0 && !byte.is_ascii_whitespace())
        .map(|index| index + 1)
        .unwrap_or(start);
    &raw[start..end]
}

pub(crate) fn quarantine_corrupt_runtime_file(
    path: &Path,
    label: &str,
    parse_error: &serde_json::Error,
) {
    let Some(parent) = path.parent() else {
        eprintln!(
            "daemon: ignoring corrupt {label} file {}: {}",
            path.display(),
            parse_error
        );
        return;
    };
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("runtime");
    let quarantined = parent.join(format!(
        "{file_name}.corrupt-{}-{}",
        std::process::id(),
        unix_timestamp()
    ));

    match fs::rename(path, &quarantined) {
        Ok(()) => eprintln!(
            "daemon: ignoring corrupt {label} file {}: {}; moved aside to {}",
            path.display(),
            parse_error,
            quarantined.display()
        ),
        Err(rename_error) => eprintln!(
            "daemon: ignoring corrupt {label} file {}: {}; failed to move aside: {}",
            path.display(),
            parse_error,
            rename_error
        ),
    }
}

pub(crate) fn spawn_daemon_process(args: &ConnectArgs, config_path: &Path) -> Result<u32> {
    if let Some(existing_pid) = daemon_candidate_pids(config_path, std::process::id())?
        .into_iter()
        .next()
    {
        return Err(anyhow!("daemon already running with pid {}", existing_pid));
    }

    let log_file_path = daemon_log_file_path(config_path);
    if let Some(parent) = log_file_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_file_path)
        .with_context(|| format!("failed to truncate {}", log_file_path.display()))?;
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)
        .with_context(|| format!("failed to open {}", log_file_path.display()))?;
    let _ = set_daemon_runtime_file_permissions(&log_file_path);
    let stderr_log = log_file
        .try_clone()
        .context("failed to clone daemon log file handle")?;

    let mut command = ProcessCommand::new(
        std::env::current_exe().context("failed to resolve current executable")?,
    );
    command
        .arg("daemon")
        .arg("--config")
        .arg(config_path)
        .arg("--iface")
        .arg(&args.iface)
        .arg("--mesh-refresh-interval-secs")
        .arg(args.mesh_refresh_interval_secs.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(stderr_log));

    if let Some(network_id) = &args.network_id {
        command.arg("--network-id").arg(network_id);
    }
    for participant in &args.participants {
        command.arg("--participant").arg(participant);
    }

    let mut child = command
        .spawn()
        .context("failed to spawn daemonized connect process")?;
    let pid = child.id();

    // Wait briefly to catch startup failures that occur after initial bootstrapping
    // (for example: missing tunnel permissions or resolver install errors).
    for _ in 0..25 {
        if let Some(status) = child
            .try_wait()
            .context("failed to verify daemon process state")?
        {
            let log_tail = read_daemon_log_tail(&log_file_path, 20);
            return if log_tail.is_empty() {
                Err(anyhow!(
                    "daemon process exited during startup with status {status}"
                ))
            } else {
                Err(anyhow!(
                    "daemon process exited during startup with status {status}\nlog tail:\n{log_tail}"
                ))
            };
        }
        thread::sleep(Duration::from_millis(100));
    }

    let record = DaemonPidRecord {
        pid,
        config_path: config_path.display().to_string(),
        started_at: unix_timestamp(),
    };
    let pid_file = daemon_pid_file_path(config_path);
    write_daemon_pid_record(&pid_file, &record)?;
    Ok(pid)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn stop_existing_daemons_before_service_install(config_path: &Path) -> Result<()> {
    stop_daemon(StopArgs {
        config: Some(config_path.to_path_buf()),
        timeout_secs: 5,
        force: true,
    })
}

pub(crate) fn read_daemon_log_tail(path: &Path, max_lines: usize) -> String {
    let Ok(raw) = fs::read_to_string(path) else {
        return String::new();
    };

    let mut lines = raw
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() > max_lines {
        lines.drain(0..(lines.len() - max_lines));
    }
    lines.join("\n")
}

#[cfg(unix)]
pub(crate) fn is_process_running(pid: u32) -> bool {
    ProcessCommand::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("pid=")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| !String::from_utf8_lossy(&output.stdout).trim().is_empty())
        .unwrap_or(false)
}

#[cfg(windows)]
pub(crate) fn is_process_running(pid: u32) -> bool {
    let output = ProcessCommand::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    tasklist_pids_from_output(&String::from_utf8_lossy(&output.stdout)).contains(&pid)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn is_process_running(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
pub(crate) fn daemon_pid_record_counts_as_running(pid: u32, config_path: &Path) -> bool {
    if !is_process_running(pid) {
        return false;
    }

    let output = ProcessCommand::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("stat=,command=")
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    daemon_pids_from_ps_output(&String::from_utf8_lossy(&output.stdout), config_path).contains(&pid)
}

#[cfg(windows)]
pub(crate) fn daemon_pid_record_counts_as_running(pid: u32, _config_path: &Path) -> bool {
    is_process_running(pid)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn daemon_pid_record_counts_as_running(_pid: u32, _config_path: &Path) -> bool {
    false
}

#[cfg(unix)]
pub(crate) fn find_daemon_pids_by_config(config_path: &Path) -> Vec<u32> {
    let output = ProcessCommand::new("ps")
        .arg("ax")
        .arg("-o")
        .arg("pid=,stat=,command=")
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    daemon_pids_from_ps_output(&String::from_utf8_lossy(&output.stdout), config_path)
}

#[cfg(windows)]
pub(crate) fn find_daemon_pids_by_config(config_path: &Path) -> Vec<u32> {
    let output = ProcessCommand::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_Process -Filter \"Name LIKE 'nvpn%.exe'\" | Select-Object ProcessId,CommandLine | ConvertTo-Json -Compress",
        ])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    daemon_pids_from_windows_cim_json(&String::from_utf8_lossy(&output.stdout), config_path)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn find_daemon_pids_by_config(_config_path: &Path) -> Vec<u32> {
    Vec::new()
}

#[cfg(any(unix, test))]
pub(crate) fn daemon_pids_from_ps_output(ps_output: &str, config_path: &Path) -> Vec<u32> {
    let mut pids = Vec::new();

    for line in ps_output.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        let Some(pid_text) = parts.next() else {
            continue;
        };
        let Some(second) = parts.next() else {
            continue;
        };
        let Ok(pid) = pid_text.parse::<u32>() else {
            continue;
        };

        let (stat, command) = if unix_ps_field_looks_like_stat(second) {
            let Some((_, command)) = trimmed
                .split_once(second)
                .map(|(prefix, suffix)| (prefix, suffix.trim_start()))
            else {
                continue;
            };
            (second, command)
        } else {
            ("", trimmed[pid_text.len()..].trim_start())
        };

        if !unix_process_stat_counts_as_running(stat) {
            continue;
        }

        if daemon_command_matches_config(command, config_path) {
            pids.push(pid);
        }
    }

    pids.sort_unstable();
    pids.dedup();
    pids
}

#[cfg(any(unix, test))]
pub(crate) fn unix_process_stat_counts_as_running(stat: &str) -> bool {
    let trimmed = stat.trim();
    if trimmed.is_empty() {
        return true;
    }

    let state = trimmed.chars().next().unwrap_or_default();
    if matches!(state, 'Z' | 'X') {
        return false;
    }

    !trimmed.contains('E')
}

#[cfg(any(unix, test))]
fn unix_ps_field_looks_like_stat(field: &str) -> bool {
    let trimmed = field.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 8
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphabetic() || matches!(ch, '<' | '>' | '+' | '-' | '|' | ':'))
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn tasklist_pids_from_output(tasklist_output: &str) -> Vec<u32> {
    let trimmed = tasklist_output.trim();
    if trimmed.is_empty()
        || trimmed
            .to_ascii_lowercase()
            .contains("no tasks are running which match")
    {
        return Vec::new();
    }

    let mut pids = Vec::new();
    for line in trimmed.lines() {
        let line = line.trim();
        if !(line.starts_with('"') && line.ends_with('"')) {
            continue;
        }
        let inner = &line[1..line.len().saturating_sub(1)];
        let mut fields = inner.split("\",\"");
        let _image_name = fields.next();
        let Some(pid_text) = fields.next() else {
            continue;
        };
        let Ok(pid) = pid_text.parse::<u32>() else {
            continue;
        };
        pids.push(pid);
    }

    pids.sort_unstable();
    pids.dedup();
    pids
}

#[cfg(windows)]
pub(crate) fn windows_nvpn_pids() -> Vec<u32> {
    let output = ProcessCommand::new("tasklist")
        .args(["/FI", "IMAGENAME eq nvpn.exe", "/FO", "CSV", "/NH"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    tasklist_pids_from_output(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn recent_windows_daemon_pid_candidate(
    state: Option<&DaemonRuntimeState>,
    current_pid: u32,
    nvpn_pids: &[u32],
    now: u64,
) -> Option<u32> {
    let state = state?;
    if now.saturating_sub(state.updated_at) > WINDOWS_DAEMON_STATE_FRESHNESS_SECS {
        return None;
    }

    let mut other_pids = nvpn_pids
        .iter()
        .copied()
        .filter(|pid| *pid != current_pid)
        .collect::<Vec<_>>();
    other_pids.sort_unstable();
    other_pids.dedup();
    if other_pids.len() == 1 {
        Some(other_pids[0])
    } else {
        None
    }
}

pub(crate) fn daemon_candidate_pids(config_path: &Path, current_pid: u32) -> Result<Vec<u32>> {
    let mut daemon_pids = find_daemon_pids_by_config(config_path);

    let pid_file = daemon_pid_file_path(config_path);
    if let Some(record) = read_daemon_pid_record(&pid_file)?
        && record.pid != current_pid
        && daemon_pid_record_counts_as_running(record.pid, config_path)
        && !daemon_pids.contains(&record.pid)
    {
        daemon_pids.push(record.pid);
    }

    #[cfg(windows)]
    {
        let state = read_daemon_state(&daemon_state_file_path(config_path))?;
        if let Some(pid) = recent_windows_daemon_pid_candidate(
            state.as_ref(),
            current_pid,
            &windows_nvpn_pids(),
            unix_timestamp(),
        ) && !daemon_pids.contains(&pid)
        {
            daemon_pids.push(pid);
        }
    }

    daemon_pids.retain(|pid| *pid != current_pid);
    daemon_pids.sort_unstable();
    daemon_pids.dedup();
    Ok(daemon_pids)
}

pub(crate) fn daemon_command_matches_config(command: &str, config_path: &Path) -> bool {
    let config_text = config_path.display().to_string();
    let Some((prefix, _)) = command.split_once(" daemon ") else {
        return false;
    };

    daemon_command_has_nvpn_executable_prefix(prefix)
        && !daemon_command_prefix_looks_like_shell_wrapper(prefix)
        && command.contains(" daemon ")
        && command.contains("--config")
        && command.contains(config_text.as_str())
}

fn daemon_command_has_nvpn_executable_prefix(prefix: &str) -> bool {
    let trimmed = prefix.trim().trim_matches(|ch| ch == '"' || ch == '\'');
    if trimmed.is_empty() {
        return false;
    }

    let normalized = trimmed.replace('\\', "/");
    if normalized == "nvpn"
        || normalized.ends_with("/nvpn")
        || normalized.eq_ignore_ascii_case("nvpn.exe")
        || normalized.to_ascii_lowercase().ends_with("/nvpn.exe")
    {
        return true;
    }

    #[cfg(any(target_os = "macos", test))]
    {
        // macOS service-managed daemons live at
        // /Library/PrivilegedHelperTools/to.nostrvpn.nvpn(.<config-suffix>)
        // — the basename starts with the service label, not "nvpn".
        // Without this match, the user-mode CLI can't tell the launchd
        // daemon is running, the GUI falls through to `nvpn start
        // --daemon` (which requires root), and VPN toggle silently fails
        // on a freshly-installed service.
        if let Some(name) = normalized.rsplit('/').next()
            && (name == MACOS_SERVICE_LABEL
                || name
                    .strip_prefix(MACOS_SERVICE_LABEL)
                    .is_some_and(|rest| rest.starts_with('.')))
        {
            return true;
        }
    }

    false
}

fn daemon_command_prefix_looks_like_shell_wrapper(prefix: &str) -> bool {
    let trimmed = prefix.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("bash ")
        || lower.starts_with("sh ")
        || lower.starts_with("zsh ")
        || lower.starts_with("dash ")
        || lower.starts_with("fish ")
        || lower.starts_with("cmd ")
        || lower.starts_with("powershell ")
        || lower.starts_with("pwsh ")
        || trimmed.contains(" -c ")
        || trimmed.contains(';')
        || trimmed.contains("&&")
        || trimmed.contains("||")
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn daemon_pids_from_windows_cim_json(cim_json: &str, config_path: &Path) -> Vec<u32> {
    let trimmed = cim_json.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return Vec::new();
    }

    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return Vec::new();
    };

    let entries = match parsed {
        serde_json::Value::Array(entries) => entries,
        serde_json::Value::Object(entry) => vec![serde_json::Value::Object(entry)],
        _ => return Vec::new(),
    };

    let mut pids = Vec::new();
    for entry in entries {
        let Some(command) = entry.get("CommandLine").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(pid) = entry
            .get("ProcessId")
            .and_then(serde_json::Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
        else {
            continue;
        };

        if daemon_command_matches_config(command, config_path) {
            pids.push(pid);
        }
    }

    pids.sort_unstable();
    pids.dedup();
    pids
}

pub(crate) fn set_daemon_runtime_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        // Daemon runtime files must stay readable by the desktop app even when
        // the daemon was started with elevated privileges.
        let permissions = fs::Permissions::from_mode(0o644);
        fs::set_permissions(path, permissions).with_context(|| {
            format!(
                "failed to set daemon runtime file permissions on {}",
                path.display()
            )
        })?;
    }

    #[cfg(not(unix))]
    let _ = path;

    Ok(())
}

pub(crate) fn set_private_cache_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).with_context(|| {
            format!(
                "failed to set daemon peer cache file permissions on {}",
                path.display()
            )
        })?;
    }

    #[cfg(not(unix))]
    let _ = path;

    Ok(())
}

pub(crate) fn executable_fingerprint(path: &Path) -> Result<ExecutableFingerprint> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to stat executable {}", path.display()))?;
    let modified_unix_nanos = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos());
    Ok(ExecutableFingerprint {
        len: metadata.len(),
        modified_unix_nanos,
    })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn current_executable_fingerprint() -> Result<(PathBuf, ExecutableFingerprint)> {
    let executable = std::env::current_exe().context("failed to resolve current executable")?;
    let executable = fs::canonicalize(&executable)
        .with_context(|| format!("failed to canonicalize {}", executable.display()))?;
    let fingerprint = executable_fingerprint(&executable)?;
    Ok((executable, fingerprint))
}

pub(crate) fn service_supervisor_restart_due(
    executable: &Path,
    launched_fingerprint: &ExecutableFingerprint,
) -> Result<bool> {
    Ok(executable_fingerprint(executable)? != *launched_fingerprint)
}

#[cfg(unix)]
pub(crate) fn send_signal(pid: u32, signal: &str) -> Result<()> {
    if cfg!(not(unix)) {
        return Err(anyhow!("daemon signal control is only supported on unix"));
    }

    let output = ProcessCommand::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .output()
        .with_context(|| format!("failed to execute kill {signal} {pid}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(anyhow!(
        "kill {signal} {pid} failed\nstdout: {}\nstderr: {}",
        stdout.trim(),
        stderr.trim()
    ))
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_taskkill_pid(pid: u32) -> Result<()> {
    let output = ProcessCommand::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output()
        .with_context(|| format!("failed to execute taskkill /PID {pid} /F"))?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let details = format!("{}\n{}", stdout.trim(), stderr.trim())
        .trim()
        .to_string();
    let lower = details.to_ascii_lowercase();
    if lower.contains("not found") || lower.contains("no running instance") {
        return Ok(());
    }

    Err(anyhow!(
        "taskkill /PID {pid} /F failed\nstdout: {}\nstderr: {}",
        stdout.trim(),
        stderr.trim()
    ))
}

#[cfg(any(unix, test))]
pub(crate) fn kill_error_requires_control_fallback(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("operation not permitted") || lower.contains("permission denied")
}

pub(crate) fn run_ping(target: &str, count: u32, timeout_secs: u64) -> Result<()> {
    let mut command = ProcessCommand::new("ping");
    if cfg!(target_os = "windows") {
        command
            .arg("-n")
            .arg(count.to_string())
            .arg("-w")
            .arg((timeout_secs.saturating_mul(1000)).to_string())
            .arg(target);
    } else {
        command
            .arg("-c")
            .arg(count.to_string())
            .arg("-W")
            .arg(timeout_secs.to_string())
            .arg(target);
    }

    let output = command
        .output()
        .with_context(|| format!("failed to execute ping for {target}"))?;

    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));

    if !output.status.success() {
        return Err(anyhow!("ping failed for {target}"));
    }

    Ok(())
}

pub(crate) fn resolve_ping_target(target: &str, peers: &[PeerAnnouncement]) -> Option<String> {
    if target.parse::<IpAddr>().is_ok() {
        return Some(target.to_string());
    }

    peers.iter().find_map(|peer| {
        let tunnel_ip = strip_cidr(&peer.tunnel_ip);
        if peer.node_id == target || peer.tunnel_ip == target || tunnel_ip == target {
            Some(tunnel_ip.to_string())
        } else {
            None
        }
    })
}
