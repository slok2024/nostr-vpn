use std::collections::HashMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use nostr_vpn_core::config::{
    AppConfig, NetworkConfig, PendingInboundJoinRequest, PendingOutboundJoinRequest,
    derive_mesh_tunnel_ip, maybe_autoconfigure_node, normalize_advertised_route,
    normalize_nostr_pubkey, normalize_runtime_network_id,
};
use nostr_vpn_core::diagnostics::ProbeStatus;
use serde::Deserialize;

use crate::actions::NativeAppAction;
use crate::invite::{
    active_network_invite_code, apply_network_invite_to_active_network, parse_network_invite,
    preferred_join_request_recipient, to_npub,
};
use crate::lan_pairing::{
    LAN_PAIRING_DURATION, LAN_PAIRING_STALE_AFTER, LanPairingAnnouncement, LanPairingSignal,
    LanPairingWorker, spawn_lan_pairing_worker,
};
use crate::native_state::{
    NativeAppState, NativeHealthIssue, NativeInboundJoinRequestState, NativeLanPeerState,
    NativeNetworkState, NativeNetworkSummary, NativeOutboundJoinRequestState,
    NativeParticipantState, NativePortMappingStatus, NativeProbeStatus,
};
use crate::platform::current_runtime_capabilities;
use crate::state::{
    DaemonPeerState, DaemonRuntimeState, HealthIssue, NetworkSummary, PortMappingStatus,
    SettingsPatch,
};

const NVPN_BIN_ENV: &str = "NVPN_CLI_PATH";
const SERVICE_STATUS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

#[derive(uniffi::Object, Debug)]
pub struct FfiApp {
    runtime: Mutex<NativeAppRuntime>,
}

#[uniffi::export]
impl FfiApp {
    #[uniffi::constructor]
    #[allow(clippy::needless_pass_by_value)]
    #[must_use]
    pub fn new(data_dir: String, app_version: String) -> Arc<Self> {
        let runtime = NativeAppRuntime::new(&data_dir, app_version)
            .unwrap_or_else(|error| NativeAppRuntime::from_startup_error(&error));
        Arc::new(Self {
            runtime: Mutex::new(runtime),
        })
    }

    #[must_use]
    pub fn state(&self) -> NativeAppState {
        self.with_runtime(|runtime| runtime.state())
    }

    #[must_use]
    pub fn refresh(&self) -> NativeAppState {
        self.dispatch(NativeAppAction::Tick)
    }

    #[must_use]
    pub fn dispatch(&self, action: NativeAppAction) -> NativeAppState {
        self.with_runtime(|runtime| {
            runtime.dispatch(action);
            runtime.state()
        })
    }
}

impl FfiApp {
    fn with_runtime(
        &self,
        f: impl FnOnce(&mut NativeAppRuntime) -> NativeAppState,
    ) -> NativeAppState {
        match self.runtime.lock() {
            Ok(mut runtime) => f(&mut runtime),
            Err(poisoned) => {
                let mut runtime = poisoned.into_inner();
                runtime.set_error("native app state lock was poisoned");
                f(&mut runtime)
            }
        }
    }
}

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
struct NativeAppRuntime {
    rev: u64,
    app_version: String,
    config_path: PathBuf,
    config: AppConfig,
    nvpn_bin: Option<PathBuf>,
    mobile_runtime: bool,
    startup_error: Option<String>,
    last_error: String,
    daemon_running: bool,
    vpn_enabled: bool,
    vpn_active: bool,
    vpn_status: String,
    daemon_state: Option<DaemonRuntimeState>,
    service_supported: bool,
    service_enablement_supported: bool,
    service_installed: bool,
    service_disabled: bool,
    service_running: bool,
    service_status_detail: String,
    service_binary_version: String,
    last_service_status_refresh_at: Option<Instant>,
    lan_pairing_worker: Option<LanPairingWorker>,
    lan_pairing_expires_at: Option<SystemTime>,
    lan_peers: HashMap<String, LanPeerRecord>,
}

#[derive(Debug, Clone)]
struct LanPeerRecord {
    signal: LanPairingSignal,
    last_seen: SystemTime,
}

#[derive(Debug, Deserialize)]
struct CliStatusResponse {
    daemon: CliDaemonStatus,
}

#[derive(Debug, Deserialize)]
struct CliDaemonStatus {
    running: bool,
    state: Option<DaemonRuntimeState>,
}

#[derive(Debug, Deserialize, Default)]
#[allow(clippy::struct_excessive_bools)]
struct CliServiceStatusResponse {
    supported: bool,
    installed: bool,
    #[serde(default)]
    disabled: bool,
    loaded: bool,
    running: bool,
    pid: Option<u32>,
    #[serde(default)]
    label: String,
    #[serde(default)]
    plist_path: String,
    #[serde(default)]
    binary_version: String,
}

impl NativeAppRuntime {
    fn new(data_dir: &str, app_version: String) -> Result<Self> {
        let config_path = native_config_path(data_dir);
        let mut config = if config_path.exists() {
            AppConfig::load(&config_path)?
        } else {
            let generated = AppConfig::generated();
            generated.save(&config_path)?;
            generated
        };
        config.ensure_defaults();
        maybe_autoconfigure_node(&mut config);
        config.save(&config_path)?;

        let capabilities = current_runtime_capabilities();
        let mut runtime = Self {
            rev: 0,
            app_version,
            config_path,
            config,
            nvpn_bin: resolve_nvpn_cli_path().ok(),
            mobile_runtime: capabilities.mobile,
            startup_error: None,
            last_error: String::new(),
            daemon_running: false,
            vpn_enabled: false,
            vpn_active: false,
            vpn_status: "Disconnected".to_string(),
            daemon_state: None,
            service_supported: !capabilities.mobile && desktop_service_supported(),
            service_enablement_supported: !capabilities.mobile && desktop_service_supported(),
            service_installed: false,
            service_disabled: false,
            service_running: false,
            service_status_detail: String::new(),
            service_binary_version: String::new(),
            last_service_status_refresh_at: None,
            lan_pairing_worker: None,
            lan_pairing_expires_at: None,
            lan_peers: HashMap::new(),
        };
        if runtime.mobile_runtime {
            let _ = runtime.refresh_mobile_status();
        } else {
            let _ = runtime.refresh_status();
        }
        Ok(runtime)
    }

    fn from_startup_error(error: &anyhow::Error) -> Self {
        let error = error.to_string();
        Self {
            rev: 0,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            config_path: default_config_path(),
            config: AppConfig::generated(),
            nvpn_bin: resolve_nvpn_cli_path().ok(),
            mobile_runtime: current_runtime_capabilities().mobile,
            startup_error: Some(error.clone()),
            last_error: error,
            daemon_running: false,
            vpn_enabled: false,
            vpn_active: false,
            vpn_status: "Startup failed".to_string(),
            daemon_state: None,
            service_supported: desktop_service_supported(),
            service_enablement_supported: desktop_service_supported(),
            service_installed: false,
            service_disabled: false,
            service_running: false,
            service_status_detail: "Service status unavailable during startup failure".to_string(),
            service_binary_version: String::new(),
            last_service_status_refresh_at: None,
            lan_pairing_worker: None,
            lan_pairing_expires_at: None,
            lan_peers: HashMap::new(),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn state(&self) -> NativeAppState {
        let capabilities = current_runtime_capabilities();
        let own_pubkey_hex = self.config.own_nostr_pubkey_hex().unwrap_or_default();
        let active_network = self.config.active_network();
        let daemon_state = self.daemon_state.as_ref();
        let vpn_enabled = self
            .daemon_state
            .as_ref()
            .map_or(self.vpn_enabled, |state| state.vpn_enabled);
        let vpn_active = self
            .daemon_state
            .as_ref()
            .map_or(self.vpn_active, |state| state.vpn_active);
        let expected_peer_count = daemon_state.map_or_else(
            || remote_network_participant_count(active_network, &own_pubkey_hex),
            |state| state.expected_peer_count,
        );
        let connected_peer_count = if vpn_active {
            daemon_state.map_or(0, |state| state.connected_peer_count)
        } else {
            0
        };
        let endpoint = daemon_state
            .and_then(|state| non_empty(&state.advertised_endpoint))
            .unwrap_or_else(|| self.config.node.endpoint.clone());
        let listen_port = daemon_state
            .and_then(|state| (state.listen_port > 0).then_some(state.listen_port))
            .unwrap_or(self.config.node.listen_port);
        let health = daemon_state
            .map(|state| native_health_issues(&state.health))
            .unwrap_or_default();
        let network = daemon_state
            .map(|state| native_network_summary(&state.network))
            .unwrap_or_default();
        let port_mapping = daemon_state
            .map(|state| native_port_mapping_status(&state.port_mapping))
            .unwrap_or_default();

        NativeAppState {
            rev: self.rev,
            platform: capabilities.platform,
            mobile: capabilities.mobile,
            vpn_control_supported: capabilities.vpn_control_supported,
            cli_install_supported: capabilities.cli_install_supported,
            startup_settings_supported: capabilities.startup_settings_supported,
            tray_behavior_supported: capabilities.tray_behavior_supported,
            runtime_status_detail: capabilities.runtime_status_detail,
            app_version: if self.app_version.is_empty() {
                env!("CARGO_PKG_VERSION").to_string()
            } else {
                self.app_version.clone()
            },
            config_path: self.config_path.display().to_string(),
            error: self
                .startup_error
                .clone()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| self.last_error.clone()),
            cli_installed: capabilities.cli_install_supported && cli_binary_installed(),
            service_supported: self.service_supported,
            service_enablement_supported: self.service_enablement_supported,
            service_installed: self.service_installed,
            service_disabled: self.service_disabled,
            service_running: self.service_running,
            service_status_detail: self.service_status_detail.clone(),
            daemon_running: self.daemon_running,
            vpn_enabled,
            vpn_active,
            vpn_status: self.vpn_status.clone(),
            daemon_binary_version: daemon_state
                .map(|state| state.binary_version.clone())
                .unwrap_or_default(),
            service_binary_version: self.service_binary_version.clone(),
            own_npub: to_npub(&own_pubkey_hex),
            own_pubkey_hex: own_pubkey_hex.clone(),
            node_id: self.config.node.id.clone(),
            node_name: self.config.node_name.clone(),
            self_magic_dns_name: self.config.self_magic_dns_name().unwrap_or_default(),
            endpoint,
            tunnel_ip: self.config.node.tunnel_ip.clone(),
            listen_port: u32::from(listen_port),
            network_id: self.config.effective_network_id(),
            active_network_invite: active_network_invite_code(&self.config).unwrap_or_default(),
            exit_node: if self.config.exit_node.trim().is_empty() {
                String::new()
            } else {
                to_npub(&self.config.exit_node)
            },
            advertise_exit_node: self.config.node.advertise_exit_node,
            advertised_routes: self.config.node.advertised_routes.clone(),
            effective_advertised_routes: self.config.effective_advertised_routes(),
            magic_dns_suffix: self.config.magic_dns_suffix.clone(),
            magic_dns_status: self.magic_dns_status(),
            autoconnect: self.config.autoconnect,
            lan_pairing_active: self.lan_pairing_active(),
            lan_pairing_remaining_secs: self.lan_pairing_remaining_secs(),
            launch_on_startup: self.config.launch_on_startup,
            close_to_tray_on_close: self.config.close_to_tray_on_close,
            connected_peer_count: connected_peer_count as u64,
            expected_peer_count: expected_peer_count as u64,
            mesh_ready: vpn_active && daemon_state.is_some_and(|state| state.mesh_ready),
            health,
            network,
            port_mapping,
            networks: self.network_states(&own_pubkey_hex, vpn_active),
            lan_peers: self.lan_peer_states(),
        }
    }

    fn dispatch(&mut self, action: NativeAppAction) {
        let result = self.apply_action(action);
        match result {
            Ok(()) => self.last_error.clear(),
            Err(error) => self.set_error(error.to_string()),
        }
        self.rev = self.rev.saturating_add(1);
    }

    #[allow(clippy::too_many_lines)]
    fn apply_action(&mut self, action: NativeAppAction) -> Result<()> {
        match action {
            NativeAppAction::GetState | NativeAppAction::Tick => {
                if self.mobile_runtime {
                    self.refresh_mobile_status()
                } else {
                    self.refresh_status()
                }
            }
            NativeAppAction::ConnectVpn => self.connect_vpn(),
            NativeAppAction::DisconnectVpn => self.disconnect_vpn(),
            NativeAppAction::InstallCli => {
                let output = self.run_nvpn(["install-cli", "--force"])?;
                ensure_success("nvpn install-cli", &output)
            }
            NativeAppAction::UninstallCli => {
                let output = self.run_nvpn(["uninstall-cli"])?;
                ensure_success("nvpn uninstall-cli", &output)
            }
            NativeAppAction::InstallSystemService => {
                let output = self.run_nvpn_service_action([
                    "service",
                    "install",
                    "--force",
                    "--config",
                    self.config_path_str()?,
                ])?;
                ensure_success("nvpn service install", &output)?;
                self.invalidate_service_status();
                self.refresh_service_status()
            }
            NativeAppAction::UninstallSystemService => {
                let output = self.run_nvpn_service_action([
                    "service",
                    "uninstall",
                    "--config",
                    self.config_path_str()?,
                ])?;
                ensure_success("nvpn service uninstall", &output)?;
                self.invalidate_service_status();
                self.refresh_service_status()
            }
            NativeAppAction::EnableSystemService => {
                let output = self.run_nvpn_service_action([
                    "service",
                    "enable",
                    "--config",
                    self.config_path_str()?,
                ])?;
                ensure_success("nvpn service enable", &output)?;
                self.invalidate_service_status();
                self.refresh_service_status()
            }
            NativeAppAction::DisableSystemService => {
                let output = self.run_nvpn_service_action([
                    "service",
                    "disable",
                    "--config",
                    self.config_path_str()?,
                ])?;
                ensure_success("nvpn service disable", &output)?;
                self.invalidate_service_status();
                self.refresh_service_status()
            }
            NativeAppAction::AddNetwork { name } => {
                self.config.add_network(&name);
                self.save_reload_and_refresh()
            }
            NativeAppAction::RenameNetwork { network_id, name } => {
                self.config.rename_network(&network_id, &name)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::RemoveNetwork { network_id } => {
                self.config.remove_network(&network_id)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::SetNetworkMeshId {
                network_id,
                mesh_id,
            } => {
                self.config.set_network_mesh_id(&network_id, &mesh_id)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::SetNetworkEnabled {
                network_id,
                enabled,
            } => {
                self.config.set_network_enabled(&network_id, enabled)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::SetNetworkJoinRequestsEnabled {
                network_id,
                enabled,
            } => {
                self.config
                    .set_network_join_requests_enabled(&network_id, enabled)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::RequestNetworkJoin { network_id } => {
                self.request_network_join(&network_id)
            }
            NativeAppAction::StartLanPairing => self.start_lan_pairing(),
            NativeAppAction::StopLanPairing => {
                self.stop_lan_pairing();
                Ok(())
            }
            NativeAppAction::AddParticipant {
                network_id,
                npub,
                alias,
            } => {
                let normalized = self.config.add_participant_to_network(&network_id, &npub)?;
                if let Some(alias) = alias
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    self.config.set_peer_alias(&normalized, alias)?;
                }
                self.save_reload_and_refresh()
            }
            NativeAppAction::AddAdmin { network_id, npub } => {
                self.config.add_admin_to_network(&network_id, &npub)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::ImportNetworkInvite { invite } => {
                self.import_network_invite(&invite)?;
                if !self.vpn_enabled {
                    self.connect_vpn()?;
                }
                Ok(())
            }
            NativeAppAction::RemoveParticipant { network_id, npub } => {
                self.config
                    .remove_participant_from_network(&network_id, &npub)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::RemoveAdmin { network_id, npub } => {
                self.config.remove_admin_from_network(&network_id, &npub)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::AcceptJoinRequest {
                network_id,
                requester_npub,
            } => self.accept_join_request(&network_id, &requester_npub),
            NativeAppAction::SetParticipantAlias { npub, alias } => {
                self.config.set_peer_alias(&npub, &alias)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::UpdateSettings { patch } => {
                self.apply_settings_patch(patch)?;
                self.save_reload_and_refresh()
            }
        }
    }

    fn import_network_invite(&mut self, invite: &str) -> Result<()> {
        let parsed = parse_network_invite(invite)?;
        apply_network_invite_to_active_network(&mut self.config, &parsed)?;
        let network_id = self.config.active_network().id.clone();
        self.queue_network_join_request(&network_id)?;
        self.save_reload_and_refresh()
    }

    fn request_network_join(&mut self, network_id: &str) -> Result<()> {
        self.queue_network_join_request(network_id)?;
        self.save_reload_and_refresh()?;
        if !self.vpn_enabled {
            self.connect_vpn()?;
        }
        Ok(())
    }

    fn queue_network_join_request(&mut self, network_id: &str) -> Result<bool> {
        let network = self
            .config
            .network_by_id(network_id)
            .ok_or_else(|| anyhow!("network not found"))?
            .clone();
        if self.network_contains_own_identity(&network) {
            return Ok(false);
        }
        let recipient = preferred_join_request_recipient(&network)
            .ok_or_else(|| anyhow!("this network was not imported from an invite"))?;
        if network
            .outbound_join_request
            .as_ref()
            .is_some_and(|existing| existing.recipient == recipient)
        {
            return Ok(false);
        }

        let network = self
            .config
            .network_by_id_mut(network_id)
            .ok_or_else(|| anyhow!("network not found"))?;
        network.outbound_join_request = Some(PendingOutboundJoinRequest {
            recipient,
            requested_at: unix_timestamp(),
        });
        Ok(true)
    }

    fn network_contains_own_identity(&self, network: &NetworkConfig) -> bool {
        let Some(own_pubkey) = self.config.own_nostr_pubkey_hex().ok() else {
            return false;
        };
        network
            .participants
            .iter()
            .chain(network.admins.iter())
            .any(|member| member == &own_pubkey)
    }

    fn accept_join_request(&mut self, network_id: &str, requester_npub: &str) -> Result<()> {
        let requester = normalize_nostr_pubkey(requester_npub)?;
        let requester_node_name = self
            .config
            .network_by_id(network_id)
            .and_then(|network| {
                network
                    .inbound_join_requests
                    .iter()
                    .find(|pending| pending.requester == requester)
                    .map(|pending| pending.requester_node_name.clone())
            })
            .unwrap_or_default();

        self.config
            .add_participant_to_network(network_id, &requester)?;
        if !requester_node_name.trim().is_empty() {
            let _ = self.config.set_peer_alias(&requester, &requester_node_name);
        }
        if let Some(network) = self.config.network_by_id_mut(network_id) {
            network
                .inbound_join_requests
                .retain(|pending| pending.requester != requester);
        }
        self.save_reload_and_refresh()?;
        if !self.vpn_enabled {
            self.connect_vpn()?;
        }
        Ok(())
    }

    fn start_lan_pairing(&mut self) -> Result<()> {
        self.refresh_lan_pairing();
        if self.lan_pairing_active() {
            return Ok(());
        }

        let own_npub = to_npub(&self.config.own_nostr_pubkey_hex()?);
        let invite = active_network_invite_code(&self.config)?;
        let endpoint = self
            .daemon_state
            .as_ref()
            .and_then(|state| non_empty(&state.advertised_endpoint))
            .unwrap_or_else(|| self.config.node.endpoint.clone());
        let expires_at = SystemTime::now()
            .checked_add(LAN_PAIRING_DURATION)
            .unwrap_or(SystemTime::now());
        let announcement = LanPairingAnnouncement {
            npub: own_npub,
            node_name: self.config.node_name.clone(),
            endpoint,
            invite,
        };
        let worker = spawn_lan_pairing_worker(announcement, expires_at)?;
        self.lan_pairing_worker = Some(worker);
        self.lan_pairing_expires_at = Some(expires_at);
        self.lan_peers.clear();
        Ok(())
    }

    fn stop_lan_pairing(&mut self) {
        if let Some(mut worker) = self.lan_pairing_worker.take() {
            worker.stop();
        }
        self.lan_pairing_expires_at = None;
        self.lan_peers.clear();
    }

    fn refresh_lan_pairing(&mut self) {
        let now = SystemTime::now();
        if self
            .lan_pairing_expires_at
            .is_some_and(|expires_at| expires_at <= now)
        {
            self.stop_lan_pairing();
            return;
        }

        let Some(worker) = &mut self.lan_pairing_worker else {
            return;
        };
        let signals = worker.drain();
        for signal in signals {
            if self.lan_signal_is_existing_peer(&signal) {
                continue;
            }
            let key = format!("{}:{}", signal.network_id, signal.npub);
            self.lan_peers.insert(
                key,
                LanPeerRecord {
                    signal,
                    last_seen: now,
                },
            );
        }
    }

    fn lan_pairing_active(&self) -> bool {
        self.lan_pairing_worker.is_some() && self.lan_pairing_remaining_secs() > 0
    }

    fn lan_pairing_remaining_secs(&self) -> u64 {
        self.lan_pairing_expires_at
            .and_then(|expires_at| expires_at.duration_since(SystemTime::now()).ok())
            .map_or(0, |remaining| remaining.as_secs())
    }

    fn lan_peer_states(&self) -> Vec<NativeLanPeerState> {
        let mut peers = self
            .lan_peers
            .values()
            .filter(|record| {
                record
                    .last_seen
                    .elapsed()
                    .is_ok_and(|age| age <= LAN_PAIRING_STALE_AFTER)
            })
            .map(|record| NativeLanPeerState {
                npub: record.signal.npub.clone(),
                node_name: record.signal.node_name.clone(),
                endpoint: record.signal.endpoint.clone(),
                network_name: record.signal.network_name.clone(),
                network_id: record.signal.network_id.clone(),
                invite: record.signal.invite.clone(),
                last_seen_text: record.last_seen.elapsed().map_or_else(
                    |_| "just now".to_string(),
                    |age| compact_age_text(age.as_secs()),
                ),
            })
            .collect::<Vec<_>>();
        peers.sort_by(|left, right| {
            left.network_name
                .cmp(&right.network_name)
                .then_with(|| left.node_name.cmp(&right.node_name))
                .then_with(|| left.npub.cmp(&right.npub))
        });
        peers
    }

    fn lan_signal_is_existing_peer(&self, signal: &LanPairingSignal) -> bool {
        let Ok(sender_hex) = normalize_nostr_pubkey(&signal.npub) else {
            return false;
        };
        let signal_network_id = normalize_runtime_network_id(&signal.network_id);
        self.config.networks.iter().any(|network| {
            normalize_runtime_network_id(&network.network_id) == signal_network_id
                && (network.admins.iter().any(|admin| admin == &sender_hex)
                    || network
                        .participants
                        .iter()
                        .any(|participant| participant == &sender_hex))
        })
    }

    fn apply_settings_patch(&mut self, patch: SettingsPatch) -> Result<()> {
        if let Some(value) = patch.node_name {
            self.config.node_name = value.trim().to_string();
        }
        if let Some(value) = patch.endpoint {
            self.config.node.endpoint = value.trim().to_string();
        }
        if let Some(value) = patch.tunnel_ip {
            self.config.node.tunnel_ip = value.trim().to_string();
        }
        if let Some(value) = patch.listen_port {
            self.config.node.listen_port = value;
        }
        if let Some(value) = patch.exit_node {
            self.config.exit_node = if value.trim().is_empty() {
                String::new()
            } else {
                normalize_nostr_pubkey(&value)?
            };
        }
        if let Some(value) = patch.advertise_exit_node {
            self.config.node.advertise_exit_node = value;
        }
        if let Some(value) = patch.advertised_routes {
            self.config.node.advertised_routes = parse_advertised_routes(&value);
        }
        if let Some(value) = patch.magic_dns_suffix {
            self.config.magic_dns_suffix = value.trim().trim_matches('.').to_ascii_lowercase();
        }
        if let Some(value) = patch.autoconnect {
            self.config.autoconnect = value;
        }
        if let Some(value) = patch.launch_on_startup {
            self.config.launch_on_startup = value;
        }
        if let Some(value) = patch.close_to_tray_on_close {
            self.config.close_to_tray_on_close = value;
        }
        Ok(())
    }

    fn connect_vpn(&mut self) -> Result<()> {
        self.vpn_enabled = true;
        if self.mobile_runtime {
            self.vpn_enabled = true;
            self.vpn_active = true;
            self.daemon_running = true;
            self.vpn_status = "VPN on".to_string();
            return self.refresh_mobile_status();
        }
        let _ = self.refresh_status();
        if self.daemon_running {
            let output = self.run_nvpn(["resume", "--config", self.config_path_str()?])?;
            ensure_success("nvpn resume", &output)?;
            return self.refresh_status();
        }
        #[cfg(target_os = "macos")]
        {
            self.refresh_service_status_if_due();
            if !self.service_running {
                self.vpn_enabled = false;
                self.vpn_active = false;
                self.vpn_status = if self.service_installed {
                    "Start background service first".to_string()
                } else {
                    "Install background service first".to_string()
                };
                return Err(anyhow!(self.vpn_status.clone()));
            }
        }
        let output = self.run_nvpn([
            "start",
            "--daemon",
            "--connect",
            "--config",
            self.config_path_str()?,
        ])?;
        ensure_success("nvpn start", &output)?;
        self.refresh_status()
    }

    fn disconnect_vpn(&mut self) -> Result<()> {
        self.vpn_enabled = false;
        if self.mobile_runtime {
            self.vpn_enabled = false;
            self.vpn_active = false;
            self.daemon_running = false;
            self.vpn_status = "Disconnected".to_string();
            return self.refresh_mobile_status();
        }
        let output = self.run_nvpn(["pause", "--config", self.config_path_str()?])?;
        ensure_success("nvpn pause", &output)?;
        self.refresh_status()
    }

    fn refresh_mobile_status(&mut self) -> Result<()> {
        self.reload_config_from_disk()?;
        self.refresh_lan_pairing();
        self.daemon_state = None;
        self.service_supported = false;
        self.service_enablement_supported = false;
        self.service_installed = false;
        self.service_disabled = false;
        self.service_running = false;
        self.service_binary_version.clear();
        self.service_status_detail = "Background service unsupported on mobile".to_string();
        if self.vpn_enabled {
            self.daemon_running = true;
            self.vpn_active = true;
            if self.vpn_status.trim().is_empty()
                || self.vpn_status == "CLI unavailable"
                || self.vpn_status.starts_with("nvpn CLI binary not found")
            {
                self.vpn_status = "VPN on".to_string();
            }
        } else {
            self.daemon_running = false;
            self.vpn_active = false;
            self.vpn_status = "Disconnected".to_string();
        }
        Ok(())
    }

    fn refresh_status(&mut self) -> Result<()> {
        self.reload_config_from_disk()?;
        self.refresh_service_status_if_due();
        self.refresh_lan_pairing();
        let output = self.run_nvpn([
            "status",
            "--json",
            "--discover-secs",
            "0",
            "--config",
            self.config_path_str()?,
        ]);

        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let json_text = extract_json_document(&stdout)?;
                let parsed = serde_json::from_str::<CliStatusResponse>(json_text)
                    .context("failed to parse `nvpn status --json` output")?;
                self.daemon_state = parsed.daemon.state;
                self.daemon_running = parsed.daemon.running;
                self.vpn_enabled = self
                    .daemon_state
                    .as_ref()
                    .map_or(parsed.daemon.running, |state| state.vpn_enabled);
                self.vpn_active = self
                    .daemon_state
                    .as_ref()
                    .map_or(parsed.daemon.running, |state| state.vpn_active);
                self.vpn_status = self.daemon_state.as_ref().map_or_else(
                    || {
                        if parsed.daemon.running {
                            "Daemon running".to_string()
                        } else {
                            "Disconnected".to_string()
                        }
                    },
                    |state| state.vpn_status.clone(),
                );
                Ok(())
            }
            Ok(output) => {
                self.daemon_state = None;
                self.daemon_running = false;
                self.vpn_enabled = false;
                self.vpn_active = false;
                self.vpn_status = "Daemon status unavailable".to_string();
                Err(command_failure("nvpn status", &output))
            }
            Err(error) => {
                self.daemon_state = None;
                self.daemon_running = false;
                self.vpn_enabled = false;
                self.vpn_active = false;
                self.vpn_status = "CLI unavailable".to_string();
                Err(error)
            }
        }
    }

    fn save_reload_and_refresh(&mut self) -> Result<()> {
        self.save_config()?;
        if self.mobile_runtime {
            return self.refresh_mobile_status();
        }
        if self.daemon_running {
            let output = self.run_nvpn(["reload", "--config", self.config_path_str()?])?;
            ensure_success("nvpn reload", &output)?;
        }
        self.refresh_status()
    }

    fn save_config(&mut self) -> Result<()> {
        self.config.ensure_defaults();
        maybe_autoconfigure_node(&mut self.config);
        self.config.save(&self.config_path)
    }

    fn reload_config_from_disk(&mut self) -> Result<()> {
        if self.config_path.exists() {
            self.config = AppConfig::load(&self.config_path)?;
            self.config.ensure_defaults();
            maybe_autoconfigure_node(&mut self.config);
        }
        Ok(())
    }

    fn network_states(&self, own_pubkey_hex: &str, vpn_active: bool) -> Vec<NativeNetworkState> {
        self.config
            .networks
            .iter()
            .map(|network| self.network_state(network, own_pubkey_hex, vpn_active))
            .collect()
    }

    fn network_state(
        &self,
        network: &NetworkConfig,
        own_pubkey_hex: &str,
        vpn_active: bool,
    ) -> NativeNetworkState {
        let mut admins = network
            .admins
            .iter()
            .map(|admin| to_npub(admin))
            .collect::<Vec<_>>();
        admins.sort();
        admins.dedup();
        let mut participant_keys = network.participants.clone();
        participant_keys.extend(network.admins.iter().cloned());
        participant_keys.sort();
        participant_keys.dedup();
        if !own_pubkey_hex.is_empty()
            && !participant_keys.iter().any(|value| value == own_pubkey_hex)
        {
            participant_keys.push(own_pubkey_hex.to_string());
        }
        let participants = participant_keys
            .iter()
            .map(|participant| {
                self.participant_state(participant, network, own_pubkey_hex, vpn_active)
            })
            .collect::<Vec<_>>();
        let online_count = participants
            .iter()
            .filter(|participant| participant.pubkey_hex != own_pubkey_hex && participant.reachable)
            .count() as u64;
        let expected_count = participants
            .iter()
            .filter(|participant| participant.pubkey_hex != own_pubkey_hex)
            .count() as u64;

        NativeNetworkState {
            id: network.id.clone(),
            name: network.name.clone(),
            enabled: network.enabled,
            network_id: normalize_runtime_network_id(&network.network_id),
            local_is_admin: self.config.is_network_admin(&network.id, own_pubkey_hex),
            join_requests_enabled: network.listen_for_join_requests,
            invite_inviter_npub: if network.invite_inviter.is_empty() {
                String::new()
            } else {
                to_npub(&network.invite_inviter)
            },
            admin_npubs: admins.clone(),
            outbound_join_request: network
                .outbound_join_request
                .as_ref()
                .map(native_outbound_join_request),
            inbound_join_requests: network
                .inbound_join_requests
                .iter()
                .map(native_inbound_join_request)
                .collect(),
            online_count,
            expected_count,
            admins,
            participants,
        }
    }

    fn participant_state(
        &self,
        participant: &str,
        network: &NetworkConfig,
        own_pubkey_hex: &str,
        vpn_active: bool,
    ) -> NativeParticipantState {
        let daemon_peer = vpn_active.then_some(()).and_then(|()| {
            self.daemon_state.as_ref().and_then(|state| {
                state
                    .peers
                    .iter()
                    .find(|peer| peer.participant_pubkey == participant)
            })
        });
        let is_local = participant == own_pubkey_hex;
        let reachable = vpn_active && (is_local || daemon_peer.is_some_and(|peer| peer.reachable));
        let magic_dns_alias = if is_local {
            self.config.self_magic_dns_label().unwrap_or_default()
        } else {
            self.config.peer_alias(participant).unwrap_or_default()
        };
        let magic_dns_name = if is_local {
            self.config.self_magic_dns_name().unwrap_or_default()
        } else {
            self.config
                .magic_dns_name_for_participant(participant)
                .unwrap_or_default()
        };
        let alias = non_empty(&magic_dns_alias).unwrap_or_else(|| short_pubkey(participant));
        let tunnel_ip = daemon_peer
            .map(|peer| peer.tunnel_ip.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                derive_mesh_tunnel_ip(&network.network_id, participant)
                    .unwrap_or_else(|| "-".to_string())
            });
        let advertised_routes = if is_local {
            self.config.effective_advertised_routes()
        } else {
            daemon_peer
                .map(|peer| peer.advertised_routes.clone())
                .unwrap_or_default()
        };
        let offers_exit_node = if is_local {
            self.config.node.advertise_exit_node
        } else {
            peer_offers_exit_node(&advertised_routes)
        };
        let peer_state = self.peer_state_label(participant, daemon_peer, is_local, vpn_active);
        let mesh_state = Self::peer_mesh_label(daemon_peer, is_local, vpn_active);

        NativeParticipantState {
            npub: to_npub(participant),
            pubkey_hex: participant.to_string(),
            alias,
            magic_dns_alias,
            magic_dns_name,
            tunnel_ip,
            is_admin: network.admins.iter().any(|admin| admin == participant),
            reachable,
            tx_bytes: daemon_peer.map_or(0, |peer| peer.tx_bytes),
            rx_bytes: daemon_peer.map_or(0, |peer| peer.rx_bytes),
            advertised_routes,
            offers_exit_node,
            fips_endpoint_npub: daemon_peer
                .map(|peer| peer.fips_endpoint_npub.clone())
                .unwrap_or_default(),
            fips_transport_addr: daemon_peer
                .map(|peer| peer.fips_transport_addr.clone())
                .unwrap_or_default(),
            fips_transport_type: daemon_peer
                .map(|peer| peer.fips_transport_type.clone())
                .unwrap_or_default(),
            fips_srtt_ms: daemon_peer.and_then(|peer| peer.fips_srtt_ms).unwrap_or(0),
            fips_packets_sent: daemon_peer.map_or(0, |peer| peer.fips_packets_sent),
            fips_packets_recv: daemon_peer.map_or(0, |peer| peer.fips_packets_recv),
            fips_bytes_sent: daemon_peer.map_or(0, |peer| peer.fips_bytes_sent),
            fips_bytes_recv: daemon_peer.map_or(0, |peer| peer.fips_bytes_recv),
            state: peer_state.clone(),
            mesh_state,
            status_text: Self::peer_status_text(daemon_peer, is_local, &peer_state),
            last_seen_text: Self::peer_last_fips_seen_text(daemon_peer, is_local),
        }
    }

    fn refresh_service_status_if_due(&mut self) {
        if !desktop_service_supported() {
            self.service_supported = false;
            self.service_enablement_supported = false;
            self.service_installed = false;
            self.service_disabled = false;
            self.service_running = false;
            self.service_binary_version.clear();
            self.service_status_detail =
                "Background service unsupported on this platform".to_string();
            return;
        }

        let now = Instant::now();
        if self
            .last_service_status_refresh_at
            .is_some_and(|last| now.duration_since(last) < SERVICE_STATUS_REFRESH_INTERVAL)
        {
            return;
        }

        if let Err(error) = self.refresh_service_status() {
            self.service_supported = true;
            self.service_enablement_supported = true;
            self.service_installed = false;
            self.service_disabled = false;
            self.service_running = false;
            self.service_binary_version.clear();
            self.service_status_detail = format!("Service status unavailable: {error}");
        }
    }

    fn refresh_service_status(&mut self) -> Result<()> {
        self.last_service_status_refresh_at = Some(Instant::now());
        let output = self.run_nvpn([
            "service",
            "status",
            "--json",
            "--config",
            self.config_path_str()?,
        ])?;
        if !output.status.success() {
            return Err(command_failure("nvpn service status", &output));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json_text = extract_json_document(&stdout)?;
        let status = serde_json::from_str::<CliServiceStatusResponse>(json_text)
            .context("failed to parse `nvpn service status --json` output")?;
        self.service_supported = status.supported;
        self.service_enablement_supported = status.supported;
        self.service_installed = status.installed;
        self.service_disabled = status.disabled;
        self.service_running = status.running;
        self.service_binary_version
            .clone_from(&status.binary_version);
        self.service_status_detail = service_status_detail(&status);
        Ok(())
    }

    fn invalidate_service_status(&mut self) {
        self.last_service_status_refresh_at = None;
    }

    fn magic_dns_status(&self) -> String {
        if self.config.magic_dns_suffix.trim().is_empty() {
            return "DNS disabled".to_string();
        }
        if self.vpn_active {
            format!("Serving .{} names", self.config.magic_dns_suffix)
        } else {
            "DNS disabled (VPN off)".to_string()
        }
    }

    fn peer_state_label(
        &self,
        participant: &str,
        peer: Option<&DaemonPeerState>,
        is_local: bool,
        vpn_active: bool,
    ) -> String {
        if !vpn_active {
            return "off".to_string();
        }
        if is_local {
            return "local".to_string();
        }
        if peer.is_some_and(|peer| peer.reachable) {
            return "online".to_string();
        }
        if peer
            .and_then(peer_last_fips_seen_secs)
            .is_some_and(within_presence_grace)
        {
            return "pending".to_string();
        }
        if peer.is_some() {
            return "offline".to_string();
        }
        if self
            .config
            .all_participant_pubkeys_hex()
            .iter()
            .any(|configured| configured == participant)
        {
            return "unknown".to_string();
        }
        "unknown".to_string()
    }

    fn peer_mesh_label(peer: Option<&DaemonPeerState>, is_local: bool, vpn_active: bool) -> String {
        if !vpn_active {
            return "off".to_string();
        }
        if is_local {
            return "local".to_string();
        }
        if peer.is_some_and(|peer| peer.reachable)
            || peer
                .and_then(peer_last_fips_seen_secs)
                .is_some_and(within_presence_grace)
        {
            return "present".to_string();
        }
        if peer.is_some() {
            return "absent".to_string();
        }
        "unknown".to_string()
    }

    fn peer_status_text(peer: Option<&DaemonPeerState>, is_local: bool, state: &str) -> String {
        if is_local {
            return "local".to_string();
        }
        match state {
            "online" => peer.map_or_else(
                || "online".to_string(),
                |peer| {
                    if let Some(link) = peer_link_text(peer) {
                        format!("online via {link}")
                    } else if let Some(seen) = peer.last_handshake_at {
                        format!("online (seen {})", compact_age_text(age_secs_since(seen)))
                    } else {
                        "online".to_string()
                    }
                },
            ),
            "pending" => peer
                .and_then(|peer| {
                    peer_link_text(peer).or_else(|| {
                        non_empty(peer.runtime_endpoint.as_deref().unwrap_or(&peer.endpoint))
                    })
                })
                .map_or_else(
                    || "fips link pending".to_string(),
                    |endpoint| format!("fips pending via {}", shorten_middle(&endpoint, 18, 10)),
                ),
            "offline" => peer.and_then(peer_last_fips_seen_secs).map_or_else(
                || "offline".to_string(),
                |seen| format!("offline ({})", compact_age_text(age_secs_since(seen))),
            ),
            _ => "unknown".to_string(),
        }
    }

    fn peer_last_fips_seen_text(peer: Option<&DaemonPeerState>, is_local: bool) -> String {
        if is_local {
            return "self".to_string();
        }
        peer.and_then(peer_last_fips_seen_secs)
            .map_or_else(String::new, |seen| {
                format!("seen {}", compact_age_text(age_secs_since(seen)))
            })
    }

    fn config_path_str(&self) -> Result<&str> {
        self.config_path
            .to_str()
            .ok_or_else(|| anyhow!("config path is not valid UTF-8"))
    }

    fn run_nvpn<const N: usize>(&self, args: [&str; N]) -> Result<Output> {
        let Some(nvpn_bin) = &self.nvpn_bin else {
            return Err(anyhow!(
                "nvpn CLI binary not found; set {NVPN_BIN_ENV} or install nvpn"
            ));
        };
        Command::new(nvpn_bin)
            .args(args)
            .output()
            .with_context(|| format!("failed to execute {}", nvpn_bin.display()))
    }

    fn run_nvpn_service_action<const N: usize>(&self, args: [&str; N]) -> Result<Output> {
        #[cfg(target_os = "macos")]
        {
            self.run_nvpn_service_action_with_macos_admin(args)
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.run_nvpn(args)
        }
    }

    #[cfg(target_os = "macos")]
    fn run_nvpn_service_action_with_macos_admin<const N: usize>(
        &self,
        args: [&str; N],
    ) -> Result<Output> {
        let Some(nvpn_bin) = &self.nvpn_bin else {
            return Err(anyhow!(
                "nvpn CLI binary not found; set {NVPN_BIN_ENV} or install nvpn"
            ));
        };
        let shell_command = macos_service_action_shell_command(nvpn_bin, &args);
        let script = format!(
            "do shell script {} with administrator privileges",
            applescript_quote(&shell_command)
        );
        Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .context("failed to request administrator privileges")
    }

    fn set_error(&mut self, error: impl Into<String>) {
        let error = error.into();
        self.last_error.clone_from(&error);
        if !error.trim().is_empty() {
            self.vpn_status = error;
        }
    }
}

impl Drop for NativeAppRuntime {
    fn drop(&mut self) {
        self.stop_lan_pairing();
    }
}

fn native_outbound_join_request(
    request: &PendingOutboundJoinRequest,
) -> NativeOutboundJoinRequestState {
    NativeOutboundJoinRequestState {
        recipient_npub: to_npub(&request.recipient),
        recipient_pubkey_hex: request.recipient.clone(),
        requested_at_text: join_request_age_text(request.requested_at),
    }
}

fn native_inbound_join_request(
    request: &PendingInboundJoinRequest,
) -> NativeInboundJoinRequestState {
    NativeInboundJoinRequestState {
        requester_npub: to_npub(&request.requester),
        requester_pubkey_hex: request.requester.clone(),
        requester_node_name: request.requester_node_name.clone(),
        requested_at_text: join_request_age_text(request.requested_at),
    }
}

fn remote_network_participant_count(network: &NetworkConfig, own_pubkey_hex: &str) -> usize {
    let mut participants = network.participants.clone();
    participants.extend(network.admins.iter().cloned());
    participants.sort();
    participants.dedup();
    participants
        .iter()
        .filter(|participant| participant.as_str() != own_pubkey_hex)
        .count()
}

fn native_health_issues(issues: &[HealthIssue]) -> Vec<NativeHealthIssue> {
    issues
        .iter()
        .map(|issue| NativeHealthIssue {
            code: issue.code.clone(),
            severity: format!("{:?}", issue.severity).to_ascii_lowercase(),
            summary: issue.summary.clone(),
            detail: issue.detail.clone(),
        })
        .collect()
}

fn native_network_summary(summary: &NetworkSummary) -> NativeNetworkSummary {
    NativeNetworkSummary {
        default_interface: summary.default_interface.clone().unwrap_or_default(),
        primary_ipv4: summary.primary_ipv4.clone().unwrap_or_default(),
        primary_ipv6: summary.primary_ipv6.clone().unwrap_or_default(),
        gateway_ipv4: summary.gateway_ipv4.clone().unwrap_or_default(),
        gateway_ipv6: summary.gateway_ipv6.clone().unwrap_or_default(),
        changed_at: summary.changed_at.unwrap_or_default(),
        captive_portal: summary
            .captive_portal
            .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
    }
}

fn native_probe_status(status: &ProbeStatus) -> NativeProbeStatus {
    NativeProbeStatus {
        state: format!("{:?}", status.state).to_ascii_lowercase(),
        detail: status.detail.clone(),
    }
}

fn native_port_mapping_status(status: &PortMappingStatus) -> NativePortMappingStatus {
    NativePortMappingStatus {
        upnp: native_probe_status(&status.upnp),
        nat_pmp: native_probe_status(&status.nat_pmp),
        pcp: native_probe_status(&status.pcp),
        active_protocol: status.active_protocol.clone().unwrap_or_default(),
        external_endpoint: status.external_endpoint.clone().unwrap_or_default(),
        gateway: status.gateway.clone().unwrap_or_default(),
        good_until: status.good_until.unwrap_or_default(),
    }
}

fn service_status_detail(status: &CliServiceStatusResponse) -> String {
    if !status.supported {
        return "Background service unsupported on this platform".to_string();
    }
    if !status.installed {
        return "Background service is not installed".to_string();
    }
    if status.disabled {
        return "Background service is installed but disabled in launchd".to_string();
    }
    if status.running {
        let label = status
            .label
            .trim()
            .strip_prefix("to.iris.")
            .unwrap_or_else(|| status.label.trim());
        let label_suffix = if label.is_empty() {
            String::new()
        } else {
            format!(" ({label})")
        };
        return status.pid.map_or_else(
            || format!("Background service running{label_suffix}"),
            |pid| format!("Background service running{label_suffix}, pid {pid}"),
        );
    }
    if status.loaded {
        return "Background service installed but not running".to_string();
    }
    if !status.plist_path.trim().is_empty() {
        return format!(
            "Background service installed but launch status is unavailable: {}",
            status.plist_path
        );
    }
    "Background service installed but launch status is unavailable".to_string()
}

fn desktop_service_supported() -> bool {
    cfg!(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))
}

fn cli_binary_installed() -> bool {
    resolve_nvpn_cli_path().is_ok()
}

fn peer_offers_exit_node(routes: &[String]) -> bool {
    routes
        .iter()
        .any(|route| route == "0.0.0.0/0" || route == "::/0")
}

fn peer_last_fips_seen_secs(peer: &DaemonPeerState) -> Option<u64> {
    peer.last_fips_seen_at
        .or_else(|| (peer.last_mesh_seen_at > 0).then_some(peer.last_mesh_seen_at))
}

fn within_presence_grace(seen_at: u64) -> bool {
    age_secs_since(seen_at) <= 90
}

fn age_secs_since(epoch_secs: u64) -> u64 {
    unix_timestamp().saturating_sub(epoch_secs)
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn join_request_age_text(requested_at: u64) -> String {
    if requested_at == 0 {
        return "just now".to_string();
    }
    compact_age_text(age_secs_since(requested_at))
}

fn compact_age_text(age_secs: u64) -> String {
    match age_secs {
        0..=59 => format!("{age_secs}s ago"),
        60..=3_599 => format!("{}m ago", age_secs / 60),
        3_600..=86_399 => format!("{}h ago", age_secs / 3_600),
        86_400..=604_799 => format!("{}d ago", age_secs / 86_400),
        604_800..=2_591_999 => format!("{}w ago", age_secs / 604_800),
        2_592_000..=31_535_999 => format!("{}mo ago", age_secs / 2_592_000),
        _ => format!("{}y ago", age_secs / 31_536_000),
    }
}

fn shorten_middle(value: &str, prefix: usize, suffix: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= prefix + suffix + 1 {
        return value.to_string();
    }
    let start = chars.iter().take(prefix).collect::<String>();
    let end = chars
        .iter()
        .skip(chars.len().saturating_sub(suffix))
        .collect::<String>();
    format!("{start}...{end}")
}

fn peer_link_text(peer: &DaemonPeerState) -> Option<String> {
    let addr = non_empty(&peer.fips_transport_addr)?;
    let transport = non_empty(&peer.fips_transport_type).unwrap_or_else(|| "fips".to_string());
    let mut text = format!("{transport} {}", shorten_middle(&addr, 22, 10));
    if let Some(srtt_ms) = peer.fips_srtt_ms {
        let _ = write!(text, " ({srtt_ms} ms)");
    }
    Some(text)
}

fn native_config_path(data_dir: &str) -> PathBuf {
    let trimmed = data_dir.trim();
    if trimmed.is_empty() {
        default_config_path()
    } else {
        PathBuf::from(trimmed).join("config.toml")
    }
}

fn default_config_path() -> PathBuf {
    dirs::config_dir().map_or_else(
        || PathBuf::from("nvpn.toml"),
        |dir| dir.join("nvpn").join("config.toml"),
    )
}

fn resolve_nvpn_cli_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os(NVPN_BIN_ENV) {
        return validate_nvpn_binary(&PathBuf::from(path));
    }
    if let Ok(exe) = env::current_exe()
        && let Some(dir) = exe.parent()
    {
        for candidate in bundled_nvpn_candidate_paths(dir) {
            if let Ok(validated) = validate_nvpn_binary(&candidate) {
                return Ok(validated);
            }
        }
    }
    if let Some(path_var) = env::var_os("PATH") {
        for dir in env::split_paths(&path_var) {
            if let Ok(validated) = validate_nvpn_binary(&dir.join(nvpn_binary_name())) {
                return Ok(validated);
            }
        }
    }
    Err(anyhow!("nvpn CLI binary not found"))
}

fn bundled_nvpn_candidate_paths(exe_dir: &Path) -> Vec<PathBuf> {
    let name = nvpn_binary_name();
    let mut paths = vec![exe_dir.join(name)];
    paths.push(exe_dir.join("binaries").join(name));
    if let Some(contents_dir) = exe_dir.parent() {
        paths.push(contents_dir.join("Resources").join("binaries").join(name));
        paths.push(contents_dir.join("Resources").join(name));
    }
    paths
}

fn nvpn_binary_name() -> &'static str {
    if cfg!(windows) { "nvpn.exe" } else { "nvpn" }
}

fn validate_nvpn_binary(path: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("failed to canonicalize {}", path.display()))?;
    let metadata = fs::metadata(&canonical)
        .with_context(|| format!("failed to inspect {}", canonical.display()))?;
    if !metadata.is_file() {
        return Err(anyhow!("{} is not a file", canonical.display()));
    }
    Ok(canonical)
}

fn ensure_success(command_name: &str, output: &Output) -> Result<()> {
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failure(command_name, output))
    }
}

fn command_failure(command_name: &str, output: &Output) -> anyhow::Error {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow!(
        "{command_name} failed\nstdout: {}\nstderr: {}",
        stdout.trim(),
        stderr.trim()
    )
}

fn extract_json_document(output: &str) -> Result<&str> {
    let start = output
        .find('{')
        .ok_or_else(|| anyhow!("command output did not contain JSON"))?;
    let end = output
        .rfind('}')
        .ok_or_else(|| anyhow!("command output did not contain complete JSON"))?;
    Ok(&output[start..=end])
}

fn parse_advertised_routes(input: &str) -> Vec<String> {
    let mut routes = input
        .split([',', '\n', ' ', '\t'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter_map(normalize_advertised_route)
        .collect::<Vec<_>>();
    routes.sort();
    routes.dedup();
    routes
}

fn short_pubkey(pubkey_hex: &str) -> String {
    if pubkey_hex.len() <= 12 {
        pubkey_hex.to_string()
    } else {
        format!(
            "{}...{}",
            &pubkey_hex[..8],
            &pubkey_hex[pubkey_hex.len() - 4..]
        )
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(target_os = "macos")]
fn macos_service_action_shell_command(nvpn_bin: &Path, args: &[&str]) -> String {
    std::iter::once(shell_quote(&nvpn_bin.display().to_string()))
        .chain(args.iter().map(|arg| shell_quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(target_os = "macos")]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn applescript_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertised_routes_are_normalized_and_deduplicated() {
        assert_eq!(
            parse_advertised_routes(" 10.0.0.0/8,10.0.0.0/8\n::/0 "),
            vec!["10.0.0.0/8".to_string(), "::/0".to_string()]
        );
    }

    #[test]
    fn default_config_path_matches_desktop_config_location() {
        let path = default_config_path();

        assert!(path.ends_with(Path::new("nvpn").join("config.toml")));
    }

    #[test]
    fn native_state_initializes_from_generated_config() {
        let error = anyhow!("boom");
        let runtime = NativeAppRuntime::from_startup_error(&error);
        let state = runtime.state();

        assert_eq!(state.error, "boom");
        assert!(!state.own_pubkey_hex.is_empty());
        assert_eq!(state.expected_peer_count, 0);
        assert_eq!(state.connected_peer_count, 0);
        assert_eq!(state.networks[0].expected_count, 0);
        assert_eq!(state.networks[0].online_count, 0);
        assert_eq!(state.networks[0].participants.len(), 1);
    }

    #[test]
    fn native_peer_counts_exclude_local_participant() {
        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        let own_pubkey = runtime
            .config
            .own_nostr_pubkey_hex()
            .expect("generated config should have own pubkey");
        let peer_pubkey = "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc";
        runtime.config.networks[0].admins = vec![own_pubkey.clone()];
        runtime.config.networks[0].participants = vec![peer_pubkey.to_string()];

        let state = runtime.state();
        let network = &state.networks[0];

        assert_eq!(state.expected_peer_count, 1);
        assert_eq!(state.connected_peer_count, 0);
        assert_eq!(network.expected_count, 1);
        assert_eq!(network.online_count, 0);
        assert_eq!(network.participants.len(), 2);
        assert!(network.participants.iter().any(|participant| {
            participant.pubkey_hex == own_pubkey
                && !participant.reachable
                && participant.state == "off"
        }));
    }

    #[test]
    fn invite_import_queues_join_request_to_invite_admin() {
        use nostr_sdk::prelude::{Keys, ToBech32};

        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-invite-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");

        let admin_npub = Keys::generate()
            .public_key()
            .to_bech32()
            .expect("admin npub");
        let admin_hex = normalize_nostr_pubkey(&admin_npub).expect("normalize admin");
        let invite = serde_json::json!({
            "v": 3,
            "networkId": "8d4f34f5425bc50e",
            "admins": [admin_npub],
            "relays": ["wss://temp.iris.to"]
        })
        .to_string();

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.mobile_runtime = true;
        runtime.config_path = dir.join("config.toml");

        runtime
            .import_network_invite(&invite)
            .expect("import invite");

        let network = runtime.config.active_network();
        let pending = network
            .outbound_join_request
            .as_ref()
            .expect("join request should be queued");
        assert_eq!(pending.recipient, admin_hex);
        assert!(network.participants.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn native_state_hides_reachable_peers_when_vpn_is_paused() {
        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        let own_pubkey = runtime
            .config
            .own_nostr_pubkey_hex()
            .expect("generated config should have own pubkey");
        let peer_pubkey = "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc";
        runtime.config.networks[0].admins = vec![own_pubkey.clone()];
        runtime.config.networks[0].participants = vec![peer_pubkey.to_string()];
        runtime.daemon_running = true;
        runtime.vpn_enabled = false;
        runtime.vpn_active = false;
        runtime.daemon_state = Some(DaemonRuntimeState {
            vpn_enabled: false,
            vpn_active: false,
            expected_peer_count: 1,
            connected_peer_count: 1,
            peers: vec![DaemonPeerState {
                participant_pubkey: peer_pubkey.to_string(),
                tunnel_ip: "10.44.10.23".to_string(),
                reachable: true,
                ..DaemonPeerState::default()
            }],
            ..DaemonRuntimeState::default()
        });

        let state = runtime.state();
        let network = &state.networks[0];

        assert!(!state.vpn_active);
        assert_eq!(state.connected_peer_count, 0);
        assert_eq!(network.online_count, 0);
        assert!(
            network
                .participants
                .iter()
                .all(|participant| { !participant.reachable && participant.state == "off" })
        );
    }

    #[test]
    fn mobile_connect_reports_vpn_on_without_pending_placeholder() {
        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.mobile_runtime = true;

        runtime.dispatch(NativeAppAction::ConnectVpn);
        let state = runtime.state();

        assert!(state.vpn_enabled);
        assert!(state.vpn_active);
        assert_eq!(state.vpn_status, "VPN on");
    }

    #[cfg(unix)]
    #[test]
    fn connect_vpn_resumes_running_daemon_without_elevated_start() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-resume-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");
        let calls_path = dir.join("calls.txt");
        let script_path = dir.join("nvpn");
        let calls_literal = calls_path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let script = format!(
            r#"#!/bin/sh
CALLS="{calls_literal}"
printf '%s\n' "$*" >> "$CALLS"
if [ "$1" = "service" ] && [ "$2" = "status" ]; then
  cat <<'JSON'
{{"supported":true,"installed":true,"disabled":false,"loaded":true,"running":true,"pid":123,"label":"to.iris.nvpn.test","binary_version":"test"}}
JSON
  exit 0
fi
if [ "$1" = "status" ]; then
  cat <<'JSON'
{{"daemon":{{"running":true,"state":{{"updated_at":1,"binary_version":"test","local_endpoint":"","advertised_endpoint":"","listen_port":0,"vpn_enabled":false,"vpn_active":false,"vpn_status":"Paused","expected_peer_count":0,"connected_peer_count":0,"mesh_ready":false,"peers":[]}}}}}}
JSON
  exit 0
fi
if [ "$1" = "resume" ]; then
  exit 0
fi
if [ "$1" = "start" ]; then
  echo "unexpected elevated start" >&2
  exit 42
fi
exit 0
"#
        );
        fs::write(&script_path, script).expect("write fake nvpn");
        let mut permissions = fs::metadata(&script_path)
            .expect("fake nvpn metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("make fake nvpn executable");

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.last_error.clear();
        runtime.config_path = dir.join("config.toml");
        runtime.nvpn_bin = Some(script_path);

        runtime.dispatch(NativeAppAction::ConnectVpn);

        let calls = fs::read_to_string(&calls_path).expect("read fake nvpn calls");
        assert!(calls.contains("resume --config"));
        assert!(!calls.contains("start --daemon --connect"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_connect_without_service_does_not_start_or_prompt() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-no-service-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");
        let calls_path = dir.join("calls.txt");
        let script_path = dir.join("nvpn");
        let calls_literal = calls_path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let script = format!(
            r#"#!/bin/sh
CALLS="{calls_literal}"
printf '%s\n' "$*" >> "$CALLS"
if [ "$1" = "service" ] && [ "$2" = "status" ]; then
  cat <<'JSON'
{{"supported":true,"installed":false,"disabled":false,"loaded":false,"running":false,"pid":null,"label":"to.iris.nvpn.test","binary_version":""}}
JSON
  exit 0
fi
if [ "$1" = "status" ]; then
  cat <<'JSON'
{{"daemon":{{"running":false,"state":null}}}}
JSON
  exit 0
fi
if [ "$1" = "start" ]; then
  echo "unexpected start" >&2
  exit 42
fi
exit 0
"#
        );
        fs::write(&script_path, script).expect("write fake nvpn");
        let mut permissions = fs::metadata(&script_path)
            .expect("fake nvpn metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("make fake nvpn executable");

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.last_error.clear();
        runtime.config_path = dir.join("config.toml");
        runtime.nvpn_bin = Some(script_path);

        runtime.dispatch(NativeAppAction::ConnectVpn);

        let calls = fs::read_to_string(&calls_path).expect("read fake nvpn calls");
        assert!(calls.contains("service status --json --config"));
        assert!(calls.contains("status --json --discover-secs 0 --config"));
        assert!(!calls.contains("start --daemon --connect"));
        assert_eq!(runtime.last_error, "Install background service first");
        assert!(!runtime.vpn_enabled);

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_service_action_shell_command_quotes_bundled_cli_path() {
        let command = macos_service_action_shell_command(
            Path::new("/Applications/Nostr VPN.app/Contents/Resources/nvpn"),
            &["service", "install", "--force"],
        );

        assert!(command.starts_with("'/Applications/Nostr VPN.app/Contents/Resources/nvpn' "));
        assert!(command.contains(" 'service' 'install' '--force'"));
    }
}
