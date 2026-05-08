use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use nostr_sdk::prelude::{PublicKey, ToBech32};

use nostr_vpn_core::config::{
    AppConfig, maybe_autoconfigure_node, normalize_advertised_route, normalize_nostr_pubkey,
};

use crate::ServerState;
use crate::invite::active_network_invite_code;
use crate::network_views::{build_network_views, is_mesh_complete};
use crate::nvpn_cli::{fetch_cli_status, load_config, reload_daemon_if_running, save_config};
use crate::ui_types::{CliStatusResponse, UiState};

pub(crate) fn update_config_and_reload(
    state: &ServerState,
    update: impl FnOnce(&mut AppConfig) -> Result<String>,
) -> Result<UiState> {
    let mut config = load_config(&state.config_path)?;
    let message = update(&mut config)?;
    finalize_config_change(state, &mut config)?;
    set_action_status(state, message);
    build_ui_state(state)
}

pub(crate) fn finalize_config_change(state: &ServerState, config: &mut AppConfig) -> Result<()> {
    config.ensure_defaults();
    maybe_autoconfigure_node(config);
    save_config(&state.config_path, config)?;
    reload_daemon_if_running(state)?;
    Ok(())
}

pub(crate) fn build_ui_state(state: &ServerState) -> Result<UiState> {
    let mut config = load_config(&state.config_path)?;
    let daemon = fetch_cli_status(state).ok();
    clear_connected_join_requests(&state.config_path, &mut config, daemon.as_ref())?;

    let daemon_running = daemon.as_ref().is_some_and(|status| status.daemon.running);
    let daemon_state = daemon
        .as_ref()
        .and_then(|status| status.daemon.state.as_ref());
    let vpn_active = daemon_state.is_some_and(|value| value.vpn_active);
    let vpn_enabled = daemon_state.is_some_and(|value| value.vpn_enabled);
    let own_pubkey_hex = config.own_nostr_pubkey_hex().unwrap_or_default();
    let own_npub = to_npub(&own_pubkey_hex);
    let network_runtime_views = build_network_views(&config, daemon_state, vpn_active);
    let networks = network_runtime_views.networks;
    let fallback_expected_peer_count = network_runtime_views.expected_peer_count;
    let fallback_connected_peer_count = network_runtime_views.connected_peer_count;
    let expected_peer_count = daemon_state
        .map(|value| value.expected_peer_count)
        .unwrap_or(fallback_expected_peer_count);
    let connected_peer_count = daemon_state
        .map(|value| value.connected_peer_count)
        .unwrap_or(fallback_connected_peer_count);
    let mesh_ready = daemon_state
        .map(|value| value.mesh_ready)
        .unwrap_or_else(|| is_mesh_complete(connected_peer_count, expected_peer_count));
    let health = daemon_state
        .map(|value| value.health.clone())
        .unwrap_or_default();
    let network = daemon_state
        .map(|value| value.network.clone())
        .unwrap_or_default();
    let port_mapping = daemon_state
        .map(|value| value.port_mapping.clone())
        .unwrap_or_default();
    let daemon_binary_version = daemon_state
        .map(|value| value.binary_version.clone())
        .unwrap_or_default();
    let vpn_status = if let Some(runtime) = daemon_state {
        runtime.vpn_status.clone()
    } else {
        let fallback = current_action_status(state);
        if fallback.trim().is_empty() {
            "Daemon not running".to_string()
        } else {
            fallback
        }
    };
    let magic_dns_status = {
        let suffix = config
            .magic_dns_suffix
            .trim()
            .trim_matches('.')
            .to_ascii_lowercase();
        if !vpn_active {
            "DNS disabled (VPN off)".to_string()
        } else if suffix.is_empty() {
            "MagicDNS suffix disabled".to_string()
        } else {
            format!(
                "MagicDNS local server is running for .{suffix}, but Umbrel host split-DNS is not installed yet"
            )
        }
    };

    Ok(UiState {
        platform: "umbrel".to_string(),
        mobile: false,
        vpn_control_supported: true,
        cli_install_supported: false,
        startup_settings_supported: false,
        tray_behavior_supported: false,
        runtime_status_detail: String::new(),
        daemon_running,
        vpn_enabled,
        vpn_active,
        cli_installed: false,
        service_supported: false,
        service_enablement_supported: false,
        service_installed: false,
        service_disabled: false,
        service_running: false,
        service_status_detail: "Managed directly by the Umbrel app".to_string(),
        vpn_status,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        daemon_binary_version,
        config_path: state.config_path.display().to_string(),
        own_npub,
        own_pubkey_hex: own_pubkey_hex.clone(),
        network_id: config.effective_network_id(),
        active_network_invite: active_network_invite_code(&config).unwrap_or_default(),
        node_id: config.node.id.clone(),
        node_name: config.node_name.clone(),
        self_magic_dns_name: config.self_magic_dns_name().unwrap_or_default(),
        endpoint: config.node.endpoint.clone(),
        tunnel_ip: config.node.tunnel_ip.clone(),
        listen_port: config.node.listen_port,
        exit_node: npub_or_none(&config.exit_node).unwrap_or_default(),
        advertise_exit_node: config.node.advertise_exit_node,
        advertised_routes: config.node.advertised_routes.clone(),
        effective_advertised_routes: config.effective_advertised_routes(),
        magic_dns_suffix: config.magic_dns_suffix.clone(),
        magic_dns_status,
        autoconnect: config.autoconnect,
        lan_pairing_active: false,
        lan_pairing_remaining_secs: 0,
        launch_on_startup: config.launch_on_startup,
        close_to_tray_on_close: config.close_to_tray_on_close,
        connected_peer_count,
        expected_peer_count,
        mesh_ready,
        health,
        network,
        port_mapping,
        networks,
        lan_peers: Vec::new(),
    })
}

fn clear_connected_join_requests(
    config_path: &Path,
    config: &mut AppConfig,
    daemon_status: Option<&CliStatusResponse>,
) -> Result<()> {
    let Some(daemon_state) = daemon_status.and_then(|status| status.daemon.state.as_ref()) else {
        return Ok(());
    };
    if !daemon_state.vpn_active {
        return Ok(());
    }

    let own_pubkey_hex = config.own_nostr_pubkey_hex().ok();
    let peer_map = daemon_state
        .peers
        .iter()
        .map(|peer| (peer.participant_pubkey.as_str(), peer))
        .collect::<HashMap<_, _>>();

    let mut changed = false;
    for network in &mut config.networks {
        let Some(request) = network.outbound_join_request.as_ref() else {
            continue;
        };
        if Some(request.recipient.as_str()) == own_pubkey_hex.as_deref() {
            continue;
        }
        let Some(peer) = peer_map.get(request.recipient.as_str()) else {
            continue;
        };
        let Some(last_handshake_at) = peer.last_handshake_at.and_then(epoch_secs_to_system_time)
        else {
            continue;
        };
        let Some(requested_at) = epoch_secs_to_system_time(request.requested_at) else {
            continue;
        };
        if peer.reachable && last_handshake_at > requested_at {
            network.outbound_join_request = None;
            changed = true;
        }
    }

    if changed {
        save_config(config_path, config)?;
    }
    Ok(())
}

pub(crate) fn set_action_status(state: &ServerState, status: impl Into<String>) {
    if let Ok(mut guard) = state.action_status.lock() {
        *guard = status.into();
    }
}

pub(crate) fn current_action_status(state: &ServerState) -> String {
    state
        .action_status
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

pub(crate) fn bad_request(error: anyhow::Error) -> crate::ApiError {
    crate::ApiError::bad_request(error.to_string())
}

pub(crate) fn internal_error(error: anyhow::Error) -> crate::ApiError {
    crate::ApiError::internal(error.to_string())
}

pub(crate) fn to_npub(pubkey_hex: &str) -> String {
    PublicKey::from_hex(pubkey_hex)
        .ok()
        .and_then(|pubkey| pubkey.to_bech32().ok())
        .unwrap_or_else(|| pubkey_hex.to_string())
}

pub(crate) fn npub_or_none(value: &str) -> Option<String> {
    PublicKey::from_hex(value)
        .ok()
        .and_then(|pubkey| pubkey.to_bech32().ok())
}

pub(crate) fn parse_exit_node_input(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("off")
        || trimmed.eq_ignore_ascii_case("none")
    {
        return Ok(String::new());
    }
    normalize_nostr_pubkey(trimmed)
}

pub(crate) fn parse_advertised_routes_input(value: &str) -> Result<Vec<String>> {
    let mut routes = Vec::new();
    for raw in value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let normalized = normalize_advertised_route(raw)
            .ok_or_else(|| anyhow!("invalid advertised route '{raw}'"))?;
        if !routes.iter().any(|existing| existing == &normalized) {
            routes.push(normalized);
        }
    }
    Ok(routes)
}

fn epoch_secs_to_system_time(value: u64) -> Option<SystemTime> {
    if value == 0 {
        return None;
    }
    UNIX_EPOCH.checked_add(Duration::from_secs(value))
}

pub(crate) fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

pub(crate) fn local_join_request_listener_enabled(config: &AppConfig) -> bool {
    let Ok(own_pubkey) = config.own_nostr_pubkey_hex() else {
        return false;
    };
    config.networks.iter().any(|network| {
        network.listen_for_join_requests && network.admins.iter().any(|admin| admin == &own_pubkey)
    })
}

pub(crate) fn nvpn_gui_iface_override() -> Option<String> {
    env::var("NVPN_GUI_IFACE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn is_already_running_message(message: &str) -> bool {
    message.to_ascii_lowercase().contains("already running")
}

pub(crate) fn is_not_running_message(message: &str) -> bool {
    message.to_ascii_lowercase().contains("not running")
}
