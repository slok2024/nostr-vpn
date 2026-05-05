use serde::{Deserialize, Serialize};

pub use nostr_vpn_core::diagnostics::{HealthIssue, NetworkSummary, PortMappingStatus};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RelaySummary {
    pub up: usize,
    pub down: usize,
    pub checking: usize,
    pub unknown: usize,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DaemonRuntimeState {
    pub updated_at: u64,
    #[serde(default)]
    pub binary_version: String,
    #[serde(default)]
    pub local_endpoint: String,
    #[serde(default)]
    pub advertised_endpoint: String,
    #[serde(default)]
    pub listen_port: u16,
    pub session_active: bool,
    pub relay_connected: bool,
    pub session_status: String,
    pub expected_peer_count: usize,
    pub connected_peer_count: usize,
    pub mesh_ready: bool,
    #[serde(default)]
    pub health: Vec<HealthIssue>,
    #[serde(default)]
    pub network: NetworkSummary,
    #[serde(default)]
    pub port_mapping: PortMappingStatus,
    pub peers: Vec<DaemonPeerState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DaemonPeerState {
    pub participant_pubkey: String,
    pub node_id: String,
    pub tunnel_ip: String,
    pub endpoint: String,
    #[serde(default)]
    pub runtime_endpoint: Option<String>,
    #[serde(default)]
    pub tx_bytes: u64,
    #[serde(default)]
    pub rx_bytes: u64,
    pub public_key: String,
    pub advertised_routes: Vec<String>,
    pub presence_timestamp: u64,
    pub last_signal_seen_at: Option<u64>,
    pub reachable: bool,
    pub last_handshake_at: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RelayView {
    pub url: String,
    pub state: String,
    pub status_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantView {
    pub npub: String,
    pub pubkey_hex: String,
    pub is_admin: bool,
    pub tunnel_ip: String,
    pub magic_dns_alias: String,
    pub magic_dns_name: String,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub advertised_routes: Vec<String>,
    pub offers_exit_node: bool,
    pub state: String,
    pub presence_state: String,
    pub status_text: String,
    pub last_signal_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OutboundJoinRequestView {
    pub recipient_npub: String,
    pub recipient_pubkey_hex: String,
    pub requested_at_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InboundJoinRequestView {
    pub requester_npub: String,
    pub requester_pubkey_hex: String,
    pub requester_node_name: String,
    pub requested_at_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NetworkView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub network_id: String,
    pub local_is_admin: bool,
    pub admin_npubs: Vec<String>,
    #[serde(rename = "joinRequestsEnabled")]
    pub listen_for_join_requests: bool,
    pub invite_inviter_npub: String,
    pub outbound_join_request: Option<OutboundJoinRequestView>,
    pub inbound_join_requests: Vec<InboundJoinRequestView>,
    pub online_count: usize,
    pub expected_count: usize,
    pub participants: Vec<ParticipantView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LanPeerView {
    pub npub: String,
    pub node_name: String,
    pub endpoint: String,
    pub network_name: String,
    pub network_id: String,
    pub invite: String,
    pub last_seen_text: String,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UiState {
    pub platform: String,
    pub mobile: bool,
    pub vpn_session_control_supported: bool,
    pub cli_install_supported: bool,
    pub startup_settings_supported: bool,
    pub tray_behavior_supported: bool,
    pub runtime_status_detail: String,
    pub daemon_running: bool,
    pub session_active: bool,
    pub relay_connected: bool,
    pub cli_installed: bool,
    pub service_supported: bool,
    pub service_enablement_supported: bool,
    pub service_installed: bool,
    pub service_disabled: bool,
    pub service_running: bool,
    pub service_status_detail: String,
    pub session_status: String,
    pub app_version: String,
    pub daemon_binary_version: String,
    pub service_binary_version: String,
    pub config_path: String,
    pub own_npub: String,
    pub own_pubkey_hex: String,
    pub network_id: String,
    pub active_network_invite: String,
    pub node_id: String,
    pub node_name: String,
    pub self_magic_dns_name: String,
    pub endpoint: String,
    pub tunnel_ip: String,
    pub listen_port: u16,
    pub exit_node: String,
    pub advertise_exit_node: bool,
    pub advertised_routes: Vec<String>,
    pub effective_advertised_routes: Vec<String>,
    pub magic_dns_suffix: String,
    pub magic_dns_status: String,
    pub autoconnect: bool,
    pub lan_pairing_active: bool,
    pub lan_pairing_remaining_secs: u64,
    pub launch_on_startup: bool,
    pub close_to_tray_on_close: bool,
    pub connected_peer_count: usize,
    pub expected_peer_count: usize,
    pub mesh_ready: bool,
    pub health: Vec<HealthIssue>,
    pub network: NetworkSummary,
    pub port_mapping: PortMappingStatus,
    pub networks: Vec<NetworkView>,
    pub relays: Vec<RelayView>,
    pub relay_summary: RelaySummary,
    pub lan_peers: Vec<LanPeerView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub node_name: Option<String>,
    pub endpoint: Option<String>,
    pub tunnel_ip: Option<String>,
    pub listen_port: Option<u16>,
    pub exit_node: Option<String>,
    pub advertise_exit_node: Option<bool>,
    pub advertised_routes: Option<String>,
    pub magic_dns_suffix: Option<String>,
    pub autoconnect: Option<bool>,
    pub launch_on_startup: Option<bool>,
    pub close_to_tray_on_close: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrayNetworkGroup {
    pub title: String,
    pub devices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrayExitNodeEntry {
    pub pubkey_hex: String,
    pub title: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TrayMenuItemSpec {
    Check {
        id: String,
        text: String,
        enabled: bool,
        checked: bool,
    },
    Text {
        id: Option<String>,
        text: String,
        enabled: bool,
    },
    Submenu {
        text: String,
        enabled: bool,
        items: Vec<TrayMenuItemSpec>,
    },
    #[default]
    Separator,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayRuntimeState {
    pub session_active: bool,
    pub service_setup_required: bool,
    pub service_enable_required: bool,
    pub status_text: String,
    pub this_device_text: String,
    pub this_device_copy_value: String,
    pub advertise_exit_node: bool,
    pub network_groups: Vec<TrayNetworkGroup>,
    pub exit_nodes: Vec<TrayExitNodeEntry>,
}

impl Default for TrayRuntimeState {
    fn default() -> Self {
        Self {
            session_active: false,
            service_setup_required: false,
            service_enable_required: false,
            status_text: "Disconnected".to_string(),
            this_device_text: "This Device: unavailable".to_string(),
            this_device_copy_value: String::new(),
            advertise_exit_node: false,
            network_groups: Vec::new(),
            exit_nodes: Vec::new(),
        }
    }
}

#[must_use]
pub fn empty_state_json() -> String {
    serde_json::to_string(&UiState::default()).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_state_serializes_current_frontend_field_names() {
        let state = UiState {
            vpn_session_control_supported: true,
            own_npub: "npub1example".to_string(),
            relay_summary: RelaySummary {
                unknown: 3,
                ..RelaySummary::default()
            },
            ..UiState::default()
        };

        let value = serde_json::to_value(state).expect("serialize state");
        assert_eq!(value["vpnSessionControlSupported"], true);
        assert_eq!(value["ownNpub"], "npub1example");
        assert_eq!(value["relaySummary"]["unknown"], 3);
    }

    #[test]
    fn network_join_request_field_keeps_frontend_name() {
        let network = NetworkView {
            listen_for_join_requests: true,
            ..NetworkView::default()
        };

        let value = serde_json::to_value(network).expect("serialize network");
        assert_eq!(value["joinRequestsEnabled"], true);
        assert!(value.get("listenForJoinRequests").is_none());
    }

    #[test]
    fn empty_state_json_round_trips() {
        let encoded = empty_state_json();
        let decoded = serde_json::from_str::<UiState>(&encoded).expect("empty state");

        assert_eq!(decoded, UiState::default());
    }
}
