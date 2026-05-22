use serde::{Deserialize, Serialize};

pub use nostr_vpn_core::diagnostics::{HealthIssue, NetworkSummary, PortMappingStatus};

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DaemonRuntimeState {
    #[serde(alias = "updated_at")]
    pub updated_at: u64,
    #[serde(default, alias = "binary_version")]
    pub binary_version: String,
    #[serde(default, alias = "local_endpoint")]
    pub local_endpoint: String,
    #[serde(default, alias = "advertised_endpoint")]
    pub advertised_endpoint: String,
    #[serde(default, alias = "listen_port")]
    pub listen_port: u16,
    #[serde(alias = "vpn_enabled")]
    pub vpn_enabled: bool,
    #[serde(alias = "vpn_active")]
    pub vpn_active: bool,
    #[serde(alias = "vpn_status")]
    pub vpn_status: String,
    #[serde(alias = "expected_peer_count")]
    pub expected_peer_count: usize,
    #[serde(alias = "connected_peer_count")]
    pub connected_peer_count: usize,
    #[serde(alias = "mesh_ready")]
    pub mesh_ready: bool,
    #[serde(default)]
    pub health: Vec<HealthIssue>,
    #[serde(default)]
    pub network: NetworkSummary,
    #[serde(default, alias = "port_mapping")]
    pub port_mapping: PortMappingStatus,
    #[serde(default)]
    pub relays: Vec<RelayView>,
    pub peers: Vec<DaemonPeerState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DaemonPeerState {
    #[serde(alias = "participant_pubkey")]
    pub participant_pubkey: String,
    #[serde(alias = "node_id")]
    pub node_id: String,
    #[serde(alias = "tunnel_ip")]
    pub tunnel_ip: String,
    pub endpoint: String,
    #[serde(default, alias = "runtime_endpoint")]
    pub runtime_endpoint: Option<String>,
    #[serde(default, alias = "fips_endpoint_npub")]
    pub fips_endpoint_npub: String,
    #[serde(default, alias = "fips_transport_addr")]
    pub fips_transport_addr: String,
    #[serde(default, alias = "fips_transport_type")]
    pub fips_transport_type: String,
    #[serde(default, alias = "fips_srtt_ms")]
    pub fips_srtt_ms: Option<u64>,
    #[serde(default, alias = "fips_packets_sent")]
    pub fips_packets_sent: u64,
    #[serde(default, alias = "fips_packets_recv")]
    pub fips_packets_recv: u64,
    #[serde(default, alias = "fips_bytes_sent")]
    pub fips_bytes_sent: u64,
    #[serde(default, alias = "fips_bytes_recv")]
    pub fips_bytes_recv: u64,
    #[serde(default, alias = "tx_bytes")]
    pub tx_bytes: u64,
    #[serde(default, alias = "rx_bytes")]
    pub rx_bytes: u64,
    #[serde(alias = "public_key")]
    pub public_key: String,
    #[serde(alias = "advertised_routes")]
    pub advertised_routes: Vec<String>,
    #[serde(
        default,
        alias = "last_mesh_seen_at",
        alias = "presence_timestamp",
        alias = "presenceTimestamp"
    )]
    pub last_mesh_seen_at: u64,
    #[serde(
        default,
        alias = "last_signal_seen_at",
        alias = "lastSignalSeenAt",
        alias = "last_fips_seen_at"
    )]
    pub last_fips_seen_at: Option<u64>,
    pub reachable: bool,
    #[serde(alias = "last_handshake_at")]
    pub last_handshake_at: Option<u64>,
    pub error: Option<String>,
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
    pub fips_endpoint_npub: String,
    pub fips_endpoint_hints: Vec<String>,
    pub fips_transport_addr: String,
    pub fips_transport_type: String,
    pub fips_srtt_ms: Option<u64>,
    pub fips_packets_sent: u64,
    pub fips_packets_recv: u64,
    pub fips_bytes_sent: u64,
    pub fips_bytes_recv: u64,
    pub state: String,
    pub mesh_state: String,
    pub status_text: String,
    pub last_seen_text: String,
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
    pub vpn_control_supported: bool,
    pub cli_install_supported: bool,
    pub startup_settings_supported: bool,
    pub tray_behavior_supported: bool,
    pub runtime_status_detail: String,
    pub daemon_running: bool,
    pub vpn_enabled: bool,
    pub vpn_active: bool,
    pub cli_installed: bool,
    pub service_supported: bool,
    pub service_enablement_supported: bool,
    pub service_installed: bool,
    pub service_disabled: bool,
    pub service_running: bool,
    pub service_status_detail: String,
    pub vpn_status: String,
    pub app_version: String,
    pub daemon_binary_version: String,
    pub service_binary_version: String,
    pub expected_service_binary_version: String,
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
    pub relays: Vec<RelayView>,
    pub exit_node: String,
    pub exit_node_leak_protection: bool,
    pub exit_node_active: bool,
    pub exit_node_blocked: bool,
    pub exit_node_status_text: String,
    pub advertise_exit_node: bool,
    pub advertised_routes: Vec<String>,
    pub effective_advertised_routes: Vec<String>,
    pub wireguard_exit_enabled: bool,
    pub wireguard_exit_configured: bool,
    pub wireguard_exit_interface: String,
    pub wireguard_exit_address: String,
    pub wireguard_exit_private_key: String,
    pub wireguard_exit_peer_public_key: String,
    pub wireguard_exit_peer_preshared_key: String,
    pub wireguard_exit_endpoint: String,
    pub wireguard_exit_allowed_ips: String,
    pub wireguard_exit_dns: String,
    pub wireguard_exit_mtu: u16,
    pub wireguard_exit_persistent_keepalive_secs: u16,
    pub wireguard_exit_config: String,
    pub fips_host_tunnel_enabled: bool,
    pub fips_host_inbound_tcp_ports: String,
    pub magic_dns_suffix: String,
    pub magic_dns_status: String,
    pub autoconnect: bool,
    pub invite_broadcast_active: bool,
    pub invite_broadcast_remaining_secs: u64,
    pub nearby_discovery_active: bool,
    pub nearby_discovery_remaining_secs: u64,
    pub launch_on_startup: bool,
    pub close_to_tray_on_close: bool,
    pub connected_peer_count: usize,
    pub expected_peer_count: usize,
    pub fips_connected_peer_count: usize,
    pub fips_roster_peer_count: usize,
    pub non_fips_roster_peer_count: usize,
    pub mesh_ready: bool,
    pub health: Vec<HealthIssue>,
    pub network: NetworkSummary,
    pub port_mapping: PortMappingStatus,
    pub networks: Vec<NetworkView>,
    pub lan_peers: Vec<LanPeerView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RelayView {
    pub url: String,
    pub status: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub node_name: Option<String>,
    pub endpoint: Option<String>,
    pub tunnel_ip: Option<String>,
    pub listen_port: Option<u16>,
    pub relays: Option<Vec<String>>,
    pub disabled_relays: Option<Vec<String>>,
    pub exit_node: Option<String>,
    pub exit_node_leak_protection: Option<bool>,
    pub advertise_exit_node: Option<bool>,
    pub advertised_routes: Option<String>,
    pub wireguard_exit_enabled: Option<bool>,
    pub wireguard_exit_interface: Option<String>,
    pub wireguard_exit_address: Option<String>,
    pub wireguard_exit_private_key: Option<String>,
    pub wireguard_exit_peer_public_key: Option<String>,
    pub wireguard_exit_peer_preshared_key: Option<String>,
    pub wireguard_exit_endpoint: Option<String>,
    pub wireguard_exit_allowed_ips: Option<String>,
    pub wireguard_exit_dns: Option<String>,
    pub wireguard_exit_mtu: Option<u16>,
    pub wireguard_exit_persistent_keepalive_secs: Option<u16>,
    pub wireguard_exit_config: Option<String>,
    pub fips_host_tunnel_enabled: Option<bool>,
    pub fips_host_inbound_tcp_ports: Option<String>,
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
    pub vpn_enabled: bool,
    pub vpn_active: bool,
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
            vpn_active: false,
            vpn_enabled: false,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_state_serializes_current_frontend_field_names() {
        let state = UiState {
            vpn_control_supported: true,
            own_npub: "npub1example".to_string(),
            ..UiState::default()
        };

        let value = serde_json::to_value(state).expect("serialize state");
        assert_eq!(value["vpnControlSupported"], true);
        assert_eq!(value["ownNpub"], "npub1example");
        assert!(value.get("relaySummary").is_none());
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
    fn daemon_runtime_state_accepts_cli_snake_case_json() {
        let json = r#"{
            "updated_at": 1778104080,
            "binary_version": "0.3.23",
            "local_endpoint": "89.27.103.157:51820",
            "advertised_endpoint": "89.27.103.157:51820",
            "listen_port": 51820,
            "vpn_enabled": true,
            "vpn_active": true,
            "vpn_status": "Running",
            "expected_peer_count": 1,
            "connected_peer_count": 1,
            "mesh_ready": true,
            "port_mapping": {
                "upnp": { "state": "unknown" },
                "natPmp": { "state": "unknown" },
                "pcp": { "state": "unknown" }
            },
            "relays": [{
                "url": "wss://temp.iris.to",
                "status": "connected"
            }],
            "peers": [{
                "participant_pubkey": "67c745be74407dd6d3427c0c2815fcf924313aed9416fd0d10806571c674cd08",
                "node_id": "",
                "tunnel_ip": "10.44.219.172/32",
                "endpoint": "fips",
                "runtime_endpoint": "fips",
                "tx_bytes": 8340,
                "rx_bytes": 19269,
                "public_key": "",
                "advertised_routes": [],
                "last_mesh_seen_at": 1778104080,
                "last_fips_seen_at": 1778104080,
                "reachable": true,
                "last_handshake_at": 1778104080,
                "error": null
            }]
        }"#;

        let state = serde_json::from_str::<DaemonRuntimeState>(json).expect("parse daemon state");

        assert!(state.vpn_enabled);
        assert_eq!(state.connected_peer_count, 1);
        assert_eq!(state.port_mapping.active_protocol, None);
        assert_eq!(state.relays[0].url, "wss://temp.iris.to");
        assert_eq!(state.relays[0].status, "connected");
        assert!(state.relays[0].enabled);
        assert_eq!(state.peers[0].runtime_endpoint.as_deref(), Some("fips"));
        assert_eq!(state.peers[0].last_fips_seen_at, Some(1_778_104_080));
        assert_eq!(state.peers[0].last_mesh_seen_at, 1_778_104_080);
        assert!(state.peers[0].reachable);
    }
}
