use nostr_vpn_core::diagnostics::{HealthIssue, NetworkSummary, PortMappingStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CliStatusResponse {
    pub daemon: CliDaemonStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CliDaemonStatus {
    pub running: bool,
    pub state: Option<DaemonRuntimeState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DaemonRuntimeState {
    pub updated_at: u64,
    #[serde(default)]
    pub binary_version: String,
    pub vpn_enabled: bool,
    pub vpn_active: bool,
    pub vpn_status: String,
    pub expected_peer_count: usize,
    pub connected_peer_count: usize,
    pub mesh_ready: bool,
    #[serde(default)]
    pub health: Vec<HealthIssue>,
    #[serde(default)]
    pub network: NetworkSummary,
    #[serde(default)]
    pub port_mapping: PortMappingStatus,
    #[serde(default)]
    pub peers: Vec<DaemonPeerState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DaemonPeerState {
    pub participant_pubkey: String,
    pub node_id: String,
    pub tunnel_ip: String,
    pub endpoint: String,
    #[serde(default)]
    pub runtime_endpoint: Option<String>,
    #[serde(default)]
    pub fips_endpoint_npub: String,
    #[serde(default)]
    pub fips_transport_addr: String,
    #[serde(default)]
    pub fips_transport_type: String,
    #[serde(default)]
    pub fips_srtt_ms: Option<u64>,
    #[serde(default)]
    pub fips_packets_sent: u64,
    #[serde(default)]
    pub fips_packets_recv: u64,
    #[serde(default)]
    pub fips_bytes_sent: u64,
    #[serde(default)]
    pub fips_bytes_recv: u64,
    #[serde(default)]
    pub tx_bytes: u64,
    #[serde(default)]
    pub rx_bytes: u64,
    pub public_key: String,
    #[serde(default)]
    pub advertised_routes: Vec<String>,
    #[serde(default, alias = "presence_timestamp", alias = "presenceTimestamp")]
    pub last_mesh_seen_at: u64,
    #[serde(default, alias = "last_signal_seen_at", alias = "lastSignalSeenAt")]
    pub last_fips_seen_at: Option<u64>,
    pub reachable: bool,
    #[serde(default)]
    pub last_handshake_at: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParticipantView {
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OutboundJoinRequestView {
    pub recipient_npub: String,
    pub recipient_pubkey_hex: String,
    pub requested_at_text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InboundJoinRequestView {
    pub requester_npub: String,
    pub requester_pubkey_hex: String,
    pub requester_node_name: String,
    pub requested_at_text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NetworkView {
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UiState {
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
    pub lan_peers: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsPatch {
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NameRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NetworkNameRequest {
    pub network_id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NetworkMeshRequest {
    pub network_id: String,
    pub mesh_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NetworkEnabledRequest {
    pub network_id: String,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NetworkIdRequest {
    pub network_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParticipantRequest {
    pub network_id: String,
    pub npub: String,
    pub alias: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NetworkPeerRequest {
    pub network_id: String,
    pub npub: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InviteRequest {
    pub invite: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JoinRequestAction {
    pub network_id: String,
    pub requester_npub: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AliasRequest {
    pub npub: String,
    pub alias: String,
}
