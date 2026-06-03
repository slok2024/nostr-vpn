use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use anyhow::{Context, Result};
use nostr_sdk::prelude::{PublicKey, ToBech32};
use serde::{Deserialize, Serialize};

use crate::config::normalize_nostr_pubkey;
use crate::data_plane::{MeshPeerStatus, PrivatePacket};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FipsMeshPeerConfig {
    pub participant_pubkey: String,
    pub endpoint_npub: String,
    pub allowed_ips: Vec<String>,
}

impl FipsMeshPeerConfig {
    pub fn from_participant_pubkey(
        participant_pubkey: impl AsRef<str>,
        allowed_ips: Vec<String>,
    ) -> Result<Self> {
        let participant_pubkey = normalize_nostr_pubkey(participant_pubkey.as_ref())?;
        let endpoint_npub = npub_for_pubkey_hex(&participant_pubkey)?;

        Ok(Self {
            participant_pubkey,
            endpoint_npub,
            allowed_ips,
        })
    }

    pub fn advertises_default_route(&self) -> bool {
        self.allowed_ips
            .iter()
            .any(|route| matches!(route.trim(), "0.0.0.0/0" | "::/0"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingFipsPacket {
    pub participant_pubkey: String,
    pub endpoint_npub: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct FipsMeshRuntime {
    peers: Vec<FipsMeshPeerRuntime>,
    local_routes: Vec<IpRoute>,
}

#[derive(Debug, Clone)]
struct FipsMeshPeerRuntime {
    participant_pubkey: String,
    endpoint_npub: String,
    endpoint_pubkey: Option<String>,
    routes: Vec<IpRoute>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IpRoute {
    network: IpAddr,
    prefix_len: u8,
}

impl FipsMeshRuntime {
    pub fn new(peers: Vec<FipsMeshPeerConfig>) -> Self {
        Self::with_local_routes(peers, Vec::new())
    }

    pub fn with_local_routes(
        peers: Vec<FipsMeshPeerConfig>,
        local_allowed_ips: Vec<String>,
    ) -> Self {
        let peers = peers
            .into_iter()
            .map(|peer| {
                let routes = peer
                    .allowed_ips
                    .iter()
                    .filter_map(|route| IpRoute::parse(route))
                    .collect();

                FipsMeshPeerRuntime {
                    participant_pubkey: normalize_participant_pubkey(&peer.participant_pubkey),
                    endpoint_npub: normalize_endpoint_npub(&peer.endpoint_npub),
                    endpoint_pubkey: normalize_nostr_pubkey(&peer.endpoint_npub).ok(),
                    routes,
                }
            })
            .collect();
        let local_routes = local_allowed_ips
            .iter()
            .filter_map(|route| IpRoute::parse(route))
            .collect();

        Self {
            peers,
            local_routes,
        }
    }

    pub fn route_outbound_packet(&self, packet: &[u8]) -> Option<OutgoingFipsPacket> {
        let destination = packet_destination(packet)?;
        let peer = self.select_peer_for_ip(destination)?;

        Some(OutgoingFipsPacket {
            participant_pubkey: peer.participant_pubkey.clone(),
            endpoint_npub: peer.endpoint_npub.clone(),
            bytes: packet.to_vec(),
        })
    }

    pub fn route_outbound_packet_owned(&self, packet: Vec<u8>) -> Option<OutgoingFipsPacket> {
        let destination = packet_destination(&packet)?;
        let peer = self.select_peer_for_ip(destination)?;

        Some(OutgoingFipsPacket {
            participant_pubkey: peer.participant_pubkey.clone(),
            endpoint_npub: peer.endpoint_npub.clone(),
            bytes: packet,
        })
    }

    pub fn receive_endpoint_data(
        &self,
        source_npub: Option<&str>,
        data: &[u8],
    ) -> Option<PrivatePacket> {
        let source_npub = source_npub?.trim();
        if source_npub.is_empty() {
            return None;
        }

        // Hot path. peer.endpoint_npub is already in canonical bech32 form
        // (`normalize_endpoint_npub` runs at construction time). Compare
        // bech32-to-bech32 instead of EC-parsing the source npub on every
        // received tunnel packet.
        let packet_source = packet_source(data)?;
        let peer = self.select_peer_for_ip(packet_source)?;
        if peer.endpoint_npub.as_str() != source_npub {
            return None;
        }
        if !self.local_routes.is_empty() {
            let packet_destination = packet_destination(data)?;
            if !self
                .local_routes
                .iter()
                .any(|route| route.matches(packet_destination))
            {
                return None;
            }
        }

        Some(PrivatePacket {
            source_pubkey: peer.participant_pubkey.clone(),
            bytes: data.to_vec(),
        })
    }

    pub fn receive_endpoint_data_owned(
        &self,
        source_npub: Option<&str>,
        data: Vec<u8>,
    ) -> Option<PrivatePacket> {
        let source_npub = source_npub?.trim();
        if source_npub.is_empty() {
            return None;
        }

        let packet_source = packet_source(&data)?;
        let peer = self.select_peer_for_ip(packet_source)?;
        if peer.endpoint_npub.as_str() != source_npub {
            return None;
        }
        if !self.local_routes.is_empty() {
            let packet_destination = packet_destination(&data)?;
            if !self
                .local_routes
                .iter()
                .any(|route| route.matches(packet_destination))
            {
                return None;
            }
        }

        Some(PrivatePacket {
            source_pubkey: peer.participant_pubkey.clone(),
            bytes: data,
        })
    }

    pub fn peer_statuses(&self) -> Vec<MeshPeerStatus> {
        self.peers
            .iter()
            .map(|peer| MeshPeerStatus {
                pubkey: peer.participant_pubkey.clone(),
                connected: false,
                endpoint_npub: peer.endpoint_npub.clone(),
                transport_addr: None,
                transport_type: None,
                srtt_ms: None,
                link_packets_sent: 0,
                link_packets_recv: 0,
                link_bytes_sent: 0,
                link_bytes_recv: 0,
                direct_probe_pending: false,
                direct_probe_after_ms: None,
                last_seen_at: None,
                tx_bytes: 0,
                rx_bytes: 0,
                error: Some("fips link pending".to_string()),
            })
            .collect()
    }

    pub fn participant_for_endpoint_npub(&self, endpoint_npub: &str) -> Option<String> {
        let source_pubkey = normalize_nostr_pubkey(endpoint_npub).ok()?;
        self.peers
            .iter()
            .find(|peer| peer.endpoint_pubkey.as_deref() == Some(source_pubkey.as_str()))
            .map(|peer| peer.participant_pubkey.clone())
    }

    pub fn peer_endpoint_npub(&self, participant_pubkey: &str) -> Option<String> {
        let participant_pubkey = normalize_participant_pubkey(participant_pubkey);
        self.peers
            .iter()
            .find(|peer| peer.participant_pubkey == participant_pubkey)
            .map(|peer| peer.endpoint_npub.clone())
    }

    pub fn peer_pubkeys(&self) -> Vec<String> {
        self.peers
            .iter()
            .map(|peer| peer.participant_pubkey.clone())
            .collect()
    }

    fn select_peer_for_ip(&self, destination: IpAddr) -> Option<&FipsMeshPeerRuntime> {
        let mut best = None;
        let mut ambiguous = false;

        for peer in &self.peers {
            for route in &peer.routes {
                if !route.matches(destination) {
                    continue;
                }
                match best {
                    None => {
                        best = Some((peer, route.prefix_len));
                        ambiguous = false;
                    }
                    Some((_, best_prefix)) if route.prefix_len > best_prefix => {
                        best = Some((peer, route.prefix_len));
                        ambiguous = false;
                    }
                    Some((best_peer, best_prefix))
                        if route.prefix_len == best_prefix
                            && best_peer.participant_pubkey != peer.participant_pubkey =>
                    {
                        ambiguous = true;
                    }
                    Some(_) => {}
                }
            }
        }

        if ambiguous {
            None
        } else {
            best.map(|(peer, _)| peer)
        }
    }
}

fn normalize_endpoint_npub(value: &str) -> String {
    let trimmed = value.trim();
    normalize_nostr_pubkey(trimmed)
        .ok()
        .and_then(|pubkey| npub_for_pubkey_hex(&pubkey).ok())
        .unwrap_or_else(|| trimmed.to_string())
}

fn normalize_participant_pubkey(value: &str) -> String {
    normalize_nostr_pubkey(value).unwrap_or_else(|_| value.trim().to_string())
}

fn npub_for_pubkey_hex(pubkey_hex: &str) -> Result<String> {
    PublicKey::from_hex(pubkey_hex)
        .context("invalid endpoint public key")?
        .to_bech32()
        .context("failed to encode endpoint npub")
}

impl IpRoute {
    fn parse(value: &str) -> Option<Self> {
        let (addr, prefix_len) = value.trim().split_once('/')?;
        let network = addr.trim().parse::<IpAddr>().ok()?;
        let prefix_len = prefix_len.trim().parse::<u8>().ok()?;

        match network {
            IpAddr::V4(ip) if prefix_len <= 32 => Some(Self {
                network: IpAddr::V4(mask_ipv4(ip, prefix_len)),
                prefix_len,
            }),
            IpAddr::V6(ip) if prefix_len <= 128 => Some(Self {
                network: IpAddr::V6(mask_ipv6(ip, prefix_len)),
                prefix_len,
            }),
            _ => None,
        }
    }

    fn matches(self, ip: IpAddr) -> bool {
        match (self.network, ip) {
            (IpAddr::V4(network), IpAddr::V4(ip)) => mask_ipv4(ip, self.prefix_len) == network,
            (IpAddr::V6(network), IpAddr::V6(ip)) => mask_ipv6(ip, self.prefix_len) == network,
            _ => false,
        }
    }
}

fn packet_destination(packet: &[u8]) -> Option<IpAddr> {
    match packet.first()? >> 4 {
        4 => ipv4_packet_addr(packet, 16),
        6 => ipv6_packet_addr(packet, 24),
        _ => None,
    }
}

fn packet_source(packet: &[u8]) -> Option<IpAddr> {
    match packet.first()? >> 4 {
        4 => ipv4_packet_addr(packet, 12),
        6 => ipv6_packet_addr(packet, 8),
        _ => None,
    }
}

fn ipv4_packet_addr(packet: &[u8], offset: usize) -> Option<IpAddr> {
    if packet.len() < 20 || offset + 4 > packet.len() {
        return None;
    }
    let ihl = packet[0] & 0x0f;
    if ihl < 5 || packet.len() < usize::from(ihl) * 4 {
        return None;
    }

    Some(IpAddr::V4(Ipv4Addr::new(
        packet[offset],
        packet[offset + 1],
        packet[offset + 2],
        packet[offset + 3],
    )))
}

fn ipv6_packet_addr(packet: &[u8], offset: usize) -> Option<IpAddr> {
    if packet.len() < 40 || offset + 16 > packet.len() {
        return None;
    }

    let mut octets = [0_u8; 16];
    octets.copy_from_slice(&packet[offset..offset + 16]);
    Some(IpAddr::V6(Ipv6Addr::from(octets)))
}

fn mask_ipv4(ip: Ipv4Addr, bits: u8) -> Ipv4Addr {
    let mask = if bits == 0 {
        0
    } else {
        u32::MAX << (32 - bits)
    };
    Ipv4Addr::from(u32::from(ip) & mask)
}

fn mask_ipv6(ip: Ipv6Addr, bits: u8) -> Ipv6Addr {
    let mask = if bits == 0 {
        0
    } else {
        u128::MAX << (128 - bits)
    };
    Ipv6Addr::from(u128::from_be_bytes(ip.octets()) & mask)
}

#[cfg(test)]
mod tests {
    use super::{FipsMeshPeerConfig, FipsMeshRuntime};
    use nostr_sdk::prelude::{Keys, ToBech32};
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[derive(Debug, Clone)]
    struct TestPeer {
        participant_pubkey: String,
        endpoint_npub: String,
    }

    impl TestPeer {
        fn generate() -> Self {
            let keys = Keys::generate();
            Self {
                participant_pubkey: keys.public_key().to_hex(),
                endpoint_npub: keys.public_key().to_bech32().expect("npub"),
            }
        }
    }

    fn runtime() -> FipsMeshRuntime {
        let general = TestPeer::generate();
        let specific = TestPeer::generate();
        FipsMeshRuntime::new(vec![
            FipsMeshPeerConfig {
                participant_pubkey: general.participant_pubkey,
                endpoint_npub: general.endpoint_npub,
                allowed_ips: vec!["10.44.0.0/16".to_string()],
            },
            FipsMeshPeerConfig {
                participant_pubkey: specific.participant_pubkey,
                endpoint_npub: specific.endpoint_npub,
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            },
        ])
    }

    fn ipv4_packet(source: Ipv4Addr, destination: Ipv4Addr) -> Vec<u8> {
        let payload = [0xde, 0xad, 0xbe, 0xef];
        let total_len = 20 + payload.len();
        let mut packet = vec![0_u8; total_len];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        packet[8] = 64;
        packet[9] = 17;
        packet[12..16].copy_from_slice(&source.octets());
        packet[16..20].copy_from_slice(&destination.octets());
        packet[20..].copy_from_slice(&payload);
        packet
    }

    fn ipv6_packet(source: Ipv6Addr, destination: Ipv6Addr) -> Vec<u8> {
        let payload = [0xde, 0xad, 0xbe, 0xef];
        let mut packet = vec![0_u8; 40 + payload.len()];
        packet[0] = 0x60;
        packet[4..6].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        packet[6] = 17;
        packet[7] = 64;
        packet[8..24].copy_from_slice(&source.octets());
        packet[24..40].copy_from_slice(&destination.octets());
        packet[40..].copy_from_slice(&payload);
        packet
    }

    #[test]
    fn peer_config_from_participant_pubkey_derives_endpoint_npub() {
        let peer = TestPeer::generate();

        let config = FipsMeshPeerConfig::from_participant_pubkey(
            &peer.participant_pubkey,
            vec!["10.44.22.44/32".to_string()],
        )
        .expect("peer config");

        assert_eq!(config.participant_pubkey, peer.participant_pubkey);
        assert_eq!(config.endpoint_npub, peer.endpoint_npub);
        assert_eq!(config.allowed_ips, vec!["10.44.22.44/32"]);
    }

    #[test]
    fn peer_config_detects_default_route_advertisement() {
        let peer = TestPeer::generate();
        let config = FipsMeshPeerConfig {
            participant_pubkey: peer.participant_pubkey,
            endpoint_npub: peer.endpoint_npub,
            allowed_ips: vec![
                "10.44.22.44/32".to_string(),
                " 0.0.0.0/0 ".to_string(),
                "::/0".to_string(),
            ],
        };

        assert!(config.advertises_default_route());
    }

    #[test]
    fn peer_config_ignores_non_default_routes() {
        let peer = TestPeer::generate();
        let config = FipsMeshPeerConfig {
            participant_pubkey: peer.participant_pubkey,
            endpoint_npub: peer.endpoint_npub,
            allowed_ips: vec!["10.44.0.0/16".to_string(), "fd00::/8".to_string()],
        };

        assert!(!config.advertises_default_route());
    }

    #[test]
    fn outbound_packet_uses_longest_prefix_route() {
        let general = TestPeer::generate();
        let specific = TestPeer::generate();
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 10, 1), Ipv4Addr::new(10, 44, 22, 44));
        let runtime = FipsMeshRuntime::new(vec![
            FipsMeshPeerConfig {
                participant_pubkey: general.participant_pubkey,
                endpoint_npub: general.endpoint_npub,
                allowed_ips: vec!["10.44.0.0/16".to_string()],
            },
            FipsMeshPeerConfig {
                participant_pubkey: specific.participant_pubkey.clone(),
                endpoint_npub: specific.endpoint_npub.clone(),
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            },
        ]);

        let outgoing = runtime
            .route_outbound_packet(&packet)
            .expect("packet should route");

        assert_eq!(outgoing.participant_pubkey, specific.participant_pubkey);
        assert_eq!(outgoing.endpoint_npub, specific.endpoint_npub);
        assert_eq!(outgoing.bytes, packet);

        let outgoing = runtime
            .route_outbound_packet_owned(packet.clone())
            .expect("owned packet should route");

        assert_eq!(outgoing.participant_pubkey, specific.participant_pubkey);
        assert_eq!(outgoing.endpoint_npub, specific.endpoint_npub);
        assert_eq!(outgoing.bytes, packet);
    }

    #[test]
    fn outbound_packet_without_route_is_dropped() {
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 10, 1), Ipv4Addr::new(192, 0, 2, 10));

        assert!(runtime().route_outbound_packet(&packet).is_none());
    }

    #[test]
    fn inbound_endpoint_data_accepts_roster_source_with_owned_packet_source() {
        let peer = TestPeer::generate();
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 1));
        let runtime = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: peer.participant_pubkey.clone(),
            endpoint_npub: peer.endpoint_npub.clone(),
            allowed_ips: vec!["10.44.22.44/32".to_string()],
        }]);

        let received = runtime
            .receive_endpoint_data(Some(&peer.endpoint_npub), &packet)
            .expect("source npub and packet source should be admitted");

        assert_eq!(received.source_pubkey, peer.participant_pubkey);
        assert_eq!(received.bytes, packet);

        let received = runtime
            .receive_endpoint_data_owned(Some(&peer.endpoint_npub), packet.clone())
            .expect("owned source npub and packet source should be admitted");

        assert_eq!(received.source_pubkey, peer.participant_pubkey);
        assert_eq!(received.bytes, packet);
    }

    #[test]
    fn inbound_endpoint_data_drops_unknown_source_npub() {
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 1));

        assert!(
            runtime()
                .receive_endpoint_data(Some("npub1unknown"), &packet)
                .is_none()
        );
    }

    #[test]
    fn inbound_endpoint_data_drops_known_npub_with_unowned_packet_source() {
        let peer = TestPeer::generate();
        let packet = ipv4_packet(Ipv4Addr::new(192, 0, 2, 10), Ipv4Addr::new(10, 44, 10, 1));
        let runtime = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: peer.participant_pubkey,
            endpoint_npub: peer.endpoint_npub.clone(),
            allowed_ips: vec!["10.44.22.44/32".to_string()],
        }]);

        assert!(
            runtime
                .receive_endpoint_data(Some(&peer.endpoint_npub), &packet)
                .is_none()
        );
    }

    #[test]
    fn inbound_endpoint_data_rejects_broad_route_spoofing_specific_peer_source() {
        let general = TestPeer::generate();
        let specific = TestPeer::generate();
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 1));
        let runtime = FipsMeshRuntime::new(vec![
            FipsMeshPeerConfig {
                participant_pubkey: general.participant_pubkey,
                endpoint_npub: general.endpoint_npub.clone(),
                allowed_ips: vec!["10.44.0.0/16".to_string()],
            },
            FipsMeshPeerConfig {
                participant_pubkey: specific.participant_pubkey,
                endpoint_npub: specific.endpoint_npub,
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            },
        ]);

        assert!(
            runtime
                .receive_endpoint_data(Some(&general.endpoint_npub), &packet)
                .is_none()
        );
    }

    #[test]
    fn equal_prefix_route_ambiguity_is_dropped() {
        let first = TestPeer::generate();
        let second = TestPeer::generate();
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 1));
        let runtime = FipsMeshRuntime::new(vec![
            FipsMeshPeerConfig {
                participant_pubkey: first.participant_pubkey,
                endpoint_npub: first.endpoint_npub.clone(),
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            },
            FipsMeshPeerConfig {
                participant_pubkey: second.participant_pubkey,
                endpoint_npub: second.endpoint_npub,
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            },
        ]);

        assert!(runtime.route_outbound_packet(&packet).is_none());
        assert!(
            runtime
                .receive_endpoint_data(Some(&first.endpoint_npub), &packet)
                .is_none()
        );
    }

    #[test]
    fn local_routes_limit_inbound_packet_destinations() {
        let peer = TestPeer::generate();
        let runtime = FipsMeshRuntime::with_local_routes(
            vec![FipsMeshPeerConfig {
                participant_pubkey: peer.participant_pubkey,
                endpoint_npub: peer.endpoint_npub.clone(),
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            }],
            vec!["10.44.10.1/32".to_string()],
        );
        let admitted = ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 1));
        let rejected = ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 2));

        assert!(
            runtime
                .receive_endpoint_data(Some(&peer.endpoint_npub), &admitted)
                .is_some()
        );
        assert!(
            runtime
                .receive_endpoint_data(Some(&peer.endpoint_npub), &rejected)
                .is_none()
        );
    }

    #[test]
    fn local_default_route_allows_exit_node_destinations() {
        let peer = TestPeer::generate();
        let runtime = FipsMeshRuntime::with_local_routes(
            vec![FipsMeshPeerConfig {
                participant_pubkey: peer.participant_pubkey,
                endpoint_npub: peer.endpoint_npub.clone(),
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            }],
            vec!["0.0.0.0/0".to_string()],
        );
        let packet = ipv4_packet(
            Ipv4Addr::new(10, 44, 22, 44),
            Ipv4Addr::new(203, 0, 113, 10),
        );

        assert!(
            runtime
                .receive_endpoint_data(Some(&peer.endpoint_npub), &packet)
                .is_some()
        );
    }

    #[test]
    fn two_device_private_mesh_routes_and_admits_bidirectional_packets() {
        let alice = TestPeer::generate();
        let bob = TestPeer::generate();
        let alice_ip = Ipv4Addr::new(10, 44, 1, 10);
        let bob_ip = Ipv4Addr::new(10, 44, 1, 20);
        let alice_runtime = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: bob.participant_pubkey.clone(),
            endpoint_npub: bob.endpoint_npub.clone(),
            allowed_ips: vec![format!("{bob_ip}/32")],
        }]);
        let bob_runtime = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: alice.participant_pubkey.clone(),
            endpoint_npub: alice.endpoint_npub.clone(),
            allowed_ips: vec![format!("{alice_ip}/32")],
        }]);

        let alice_to_bob = ipv4_packet(alice_ip, bob_ip);
        let outgoing = alice_runtime
            .route_outbound_packet(&alice_to_bob)
            .expect("Alice should route packet to Bob");
        assert_eq!(outgoing.participant_pubkey, bob.participant_pubkey);
        assert_eq!(outgoing.endpoint_npub, bob.endpoint_npub);
        let received = bob_runtime
            .receive_endpoint_data(Some(&alice.endpoint_npub), &outgoing.bytes)
            .expect("Bob should admit Alice's owned source IP");
        assert_eq!(received.source_pubkey, alice.participant_pubkey);
        assert_eq!(received.bytes, alice_to_bob);

        let bob_to_alice = ipv4_packet(bob_ip, alice_ip);
        let outgoing = bob_runtime
            .route_outbound_packet(&bob_to_alice)
            .expect("Bob should route packet to Alice");
        assert_eq!(outgoing.participant_pubkey, alice.participant_pubkey);
        assert_eq!(outgoing.endpoint_npub, alice.endpoint_npub);
        let received = alice_runtime
            .receive_endpoint_data(Some(&bob.endpoint_npub), &outgoing.bytes)
            .expect("Alice should admit Bob's owned source IP");
        assert_eq!(received.source_pubkey, bob.participant_pubkey);
        assert_eq!(received.bytes, bob_to_alice);
    }

    #[test]
    fn ipv6_routes_are_supported_for_raw_packets() {
        let peer = TestPeer::generate();
        let runtime = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: peer.participant_pubkey.clone(),
            endpoint_npub: peer.endpoint_npub.clone(),
            allowed_ips: vec!["fd00:44::/64".to_string()],
        }]);
        let packet = ipv6_packet(
            "fd00:44::20".parse().expect("source"),
            "fd00:44::10".parse().expect("destination"),
        );

        let outgoing = runtime
            .route_outbound_packet(&packet)
            .expect("IPv6 packet should route");
        let received = runtime
            .receive_endpoint_data(Some(&peer.endpoint_npub), &packet)
            .expect("IPv6 source should be admitted");

        assert_eq!(outgoing.endpoint_npub, peer.endpoint_npub);
        assert_eq!(outgoing.bytes, packet);
        assert_eq!(received.source_pubkey, peer.participant_pubkey);
    }
}
