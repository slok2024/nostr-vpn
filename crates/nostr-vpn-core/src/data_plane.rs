use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivateDataPlane {
    #[default]
    #[serde(alias = "wireguard")]
    Fips,
}

impl PrivateDataPlane {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fips => "fips",
        }
    }
}

impl fmt::Display for PrivateDataPlane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitDataPlane {
    None,
    #[default]
    #[serde(rename = "wireguard")]
    WireGuard,
}

impl ExitDataPlane {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::WireGuard => "wireguard",
        }
    }
}

impl fmt::Display for ExitDataPlane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FipsDataPlaneCapability {
    pub endpoint_npub: String,
    pub network_scope: String,
    #[serde(default)]
    pub bridge_ok: bool,
}

impl FipsDataPlaneCapability {
    pub fn new(endpoint_npub: impl Into<String>, network_scope: impl Into<String>) -> Self {
        Self {
            endpoint_npub: endpoint_npub.into(),
            network_scope: network_scope.into(),
            bridge_ok: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "data_plane", rename_all = "snake_case")]
pub enum DataPlaneCapability {
    Fips { fips: FipsDataPlaneCapability },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshRoster {
    pub network_id: String,
    pub member_pubkeys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePolicy {
    pub private_routes: Vec<String>,
    pub exit_routes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivatePacket {
    pub source_pubkey: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshPeerStatus {
    pub pubkey: String,
    pub connected: bool,
    pub data_plane: PrivateDataPlane,
    pub endpoint_npub: String,
    pub transport_addr: Option<String>,
    pub transport_type: Option<String>,
    pub srtt_ms: Option<u64>,
    pub link_packets_sent: u64,
    pub link_packets_recv: u64,
    pub link_bytes_sent: u64,
    pub link_bytes_recv: u64,
    pub last_seen_at: Option<u64>,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub error: Option<String>,
}

#[async_trait]
pub trait PrivateMeshBackend: Send {
    async fn start(&mut self, roster: MeshRoster, routes: RoutePolicy) -> Result<()>;

    async fn send_private_packet(&self, packet: &[u8]) -> Result<()>;

    async fn recv_private_packet(&mut self) -> Result<Option<PrivatePacket>>;

    async fn peer_status(&self) -> Result<Vec<MeshPeerStatus>>;
}

pub fn private_data_plane_routes_to_fips(private_data_plane: PrivateDataPlane) -> bool {
    matches!(private_data_plane, PrivateDataPlane::Fips)
}

pub fn exit_data_plane_routes_to_wireguard(exit_data_plane: ExitDataPlane) -> bool {
    exit_data_plane == ExitDataPlane::WireGuard
}

#[cfg(test)]
mod tests {
    use super::{
        DataPlaneCapability, ExitDataPlane, FipsDataPlaneCapability, PrivateDataPlane,
        exit_data_plane_routes_to_wireguard, private_data_plane_routes_to_fips,
    };

    #[test]
    fn defaults_use_fips_private_mesh_with_wireguard_exit() {
        assert_eq!(PrivateDataPlane::default(), PrivateDataPlane::Fips);
        assert_eq!(ExitDataPlane::default(), ExitDataPlane::WireGuard);
        assert!(private_data_plane_routes_to_fips(
            PrivateDataPlane::default()
        ));
        assert!(exit_data_plane_routes_to_wireguard(ExitDataPlane::default()));
    }

    #[test]
    fn legacy_wireguard_private_data_plane_deserializes_as_fips() {
        let data_plane: PrivateDataPlane =
            serde_json::from_str("\"wireguard\"").expect("legacy value should load");

        assert_eq!(data_plane, PrivateDataPlane::Fips);
        assert_eq!(
            serde_json::to_string(&data_plane).expect("serialize data plane"),
            "\"fips\""
        );
    }

    #[test]
    fn fips_capability_advertises_endpoint_without_app_protocol() {
        let capability = FipsDataPlaneCapability::new("npub1example", "network-a");
        assert!(!capability.bridge_ok);

        let encoded = serde_json::to_value(DataPlaneCapability::Fips { fips: capability })
            .expect("capability should serialize");
        assert_eq!(encoded["data_plane"], "fips");
        assert_eq!(encoded["fips"]["endpoint_npub"], "npub1example");
        assert_eq!(encoded["fips"]["network_scope"], "network-a");
        assert!(encoded["fips"].get("protocol").is_none());
    }
}
