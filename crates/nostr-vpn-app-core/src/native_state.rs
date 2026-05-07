use serde::{Deserialize, Serialize};

#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRelayState {
    pub url: String,
    pub state: String,
    pub status_text: String,
}

#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRelaySummary {
    pub up: u64,
    pub down: u64,
    pub checking: u64,
    pub unknown: u64,
}

#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeParticipantState {
    pub npub: String,
    pub pubkey_hex: String,
    pub alias: String,
    pub magic_dns_alias: String,
    pub magic_dns_name: String,
    pub tunnel_ip: String,
    pub is_admin: bool,
    pub reachable: bool,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub advertised_routes: Vec<String>,
    pub offers_exit_node: bool,
    pub fips_endpoint_npub: String,
    pub fips_transport_addr: String,
    pub fips_transport_type: String,
    pub fips_srtt_ms: u64,
    pub fips_packets_sent: u64,
    pub fips_packets_recv: u64,
    pub fips_bytes_sent: u64,
    pub fips_bytes_recv: u64,
    pub state: String,
    pub presence_state: String,
    pub status_text: String,
    pub last_signal_text: String,
}

#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeOutboundJoinRequestState {
    pub recipient_npub: String,
    pub recipient_pubkey_hex: String,
    pub requested_at_text: String,
}

#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeInboundJoinRequestState {
    pub requester_npub: String,
    pub requester_pubkey_hex: String,
    pub requester_node_name: String,
    pub requested_at_text: String,
}

#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeNetworkState {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub network_id: String,
    pub local_is_admin: bool,
    pub join_requests_enabled: bool,
    pub invite_inviter_npub: String,
    pub admin_npubs: Vec<String>,
    pub outbound_join_request: Option<NativeOutboundJoinRequestState>,
    pub inbound_join_requests: Vec<NativeInboundJoinRequestState>,
    pub online_count: u64,
    pub expected_count: u64,
    pub admins: Vec<String>,
    pub participants: Vec<NativeParticipantState>,
}

#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeLanPeerState {
    pub npub: String,
    pub node_name: String,
    pub endpoint: String,
    pub network_name: String,
    pub network_id: String,
    pub invite: String,
    pub last_seen_text: String,
}

#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeHealthIssue {
    pub code: String,
    pub severity: String,
    pub summary: String,
    pub detail: String,
}

#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeNetworkSummary {
    pub default_interface: String,
    pub primary_ipv4: String,
    pub primary_ipv6: String,
    pub gateway_ipv4: String,
    pub gateway_ipv6: String,
    pub changed_at: u64,
    pub captive_portal: String,
}

#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeProbeStatus {
    pub state: String,
    pub detail: String,
}

#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePortMappingStatus {
    pub upnp: NativeProbeStatus,
    pub nat_pmp: NativeProbeStatus,
    pub pcp: NativeProbeStatus,
    pub active_protocol: String,
    pub external_endpoint: String,
    pub gateway: String,
    pub good_until: u64,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeAppState {
    pub rev: u64,
    pub platform: String,
    pub mobile: bool,
    pub vpn_control_supported: bool,
    pub cli_install_supported: bool,
    pub startup_settings_supported: bool,
    pub tray_behavior_supported: bool,
    pub runtime_status_detail: String,
    pub app_version: String,
    pub config_path: String,
    pub error: String,
    pub cli_installed: bool,
    pub service_supported: bool,
    pub service_enablement_supported: bool,
    pub service_installed: bool,
    pub service_disabled: bool,
    pub service_running: bool,
    pub service_status_detail: String,
    pub daemon_running: bool,
    pub vpn_enabled: bool,
    pub vpn_active: bool,
    pub vpn_status: String,
    pub daemon_binary_version: String,
    pub service_binary_version: String,
    pub own_npub: String,
    pub own_pubkey_hex: String,
    pub node_id: String,
    pub node_name: String,
    pub self_magic_dns_name: String,
    pub endpoint: String,
    pub tunnel_ip: String,
    pub listen_port: u32,
    pub network_id: String,
    pub active_network_invite: String,
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
    pub connected_peer_count: u64,
    pub expected_peer_count: u64,
    pub mesh_ready: bool,
    pub health: Vec<NativeHealthIssue>,
    pub network: NativeNetworkSummary,
    pub port_mapping: NativePortMappingStatus,
    pub networks: Vec<NativeNetworkState>,
    pub relays: Vec<NativeRelayState>,
    pub relay_summary: NativeRelaySummary,
    pub lan_peers: Vec<NativeLanPeerState>,
}
