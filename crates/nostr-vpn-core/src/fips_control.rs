use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use crate::join_requests::MeshJoinRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkRoster {
    #[serde(default)]
    pub network_name: String,
    pub participants: Vec<String>,
    #[serde(default)]
    pub admins: Vec<String>,
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    #[serde(default)]
    pub signed_at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerCapabilities {
    #[serde(default)]
    pub advertised_routes: Vec<String>,
    #[serde(default)]
    pub endpoint_hints: Vec<PeerEndpointHint>,
    #[serde(default)]
    pub signed_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerEndpointHint {
    #[serde(default = "default_peer_endpoint_hint_transport")]
    pub transport: String,
    pub addr: String,
}

impl PeerEndpointHint {
    pub fn udp(addr: impl Into<String>) -> Self {
        Self {
            transport: default_peer_endpoint_hint_transport(),
            addr: addr.into(),
        }
    }
}

fn default_peer_endpoint_hint_transport() -> String {
    "udp".to_string()
}

const FIPS_CONTROL_MAGIC: &[u8] = b"NVPN-FIPS-CTRL\0";
const FIPS_CONTROL_VERSION: u8 = 1;
pub const FIPS_CONTROL_DIRECT_FRAME_LIMIT: usize = 1100;
pub const FIPS_CONTROL_FRAGMENT_CHUNK_LEN: usize = 700;
pub const FIPS_CONTROL_FRAGMENT_TTL_SECS: u64 = 120;
pub const FIPS_CONTROL_MAX_FRAGMENTS: u16 = 128;
pub const FIPS_CONTROL_MAX_REASSEMBLED_LEN: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FipsControlFrame {
    Ping {
        network_id: String,
        sent_at: u64,
    },
    Pong {
        network_id: String,
        sent_at: u64,
        replied_at: u64,
    },
    JoinRequest {
        requested_at: u64,
        request: MeshJoinRequest,
    },
    Roster {
        network_id: String,
        roster: NetworkRoster,
    },
    Capabilities {
        network_id: String,
        capabilities: PeerCapabilities,
    },
    Fragment {
        id: String,
        index: u16,
        total: u16,
        data: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FipsControlEnvelope {
    v: u8,
    frame: FipsControlFrame,
}

pub fn encode_fips_control_frame(frame: &FipsControlFrame) -> Result<Vec<u8>> {
    let envelope = FipsControlEnvelope {
        v: FIPS_CONTROL_VERSION,
        frame: frame.clone(),
    };
    let mut out = Vec::from(FIPS_CONTROL_MAGIC);
    out.extend_from_slice(
        &serde_json::to_vec(&envelope).context("failed to encode FIPS control frame")?,
    );
    Ok(out)
}

pub fn encode_fips_control_messages(frame: &FipsControlFrame) -> Result<Vec<Vec<u8>>> {
    let encoded = encode_fips_control_frame(frame)?;
    if encoded.len() <= FIPS_CONTROL_DIRECT_FRAME_LIMIT {
        return Ok(vec![encoded]);
    }

    let total = encoded.len().div_ceil(FIPS_CONTROL_FRAGMENT_CHUNK_LEN);
    let total = u16::try_from(total).context("FIPS control frame has too many fragments")?;
    let id = fips_control_fragment_id(&encoded);
    encoded
        .chunks(FIPS_CONTROL_FRAGMENT_CHUNK_LEN)
        .enumerate()
        .map(|(index, chunk)| {
            let fragment = FipsControlFrame::Fragment {
                id: id.clone(),
                index: u16::try_from(index).context("FIPS control fragment index overflow")?,
                total,
                data: URL_SAFE_NO_PAD.encode(chunk),
            };
            encode_fips_control_frame(&fragment)
        })
        .collect()
}

pub fn decode_fips_control_frame(data: &[u8]) -> Result<Option<FipsControlFrame>> {
    let Some(payload) = data.strip_prefix(FIPS_CONTROL_MAGIC) else {
        return Ok(None);
    };
    let envelope: FipsControlEnvelope = match serde_json::from_slice(payload) {
        Ok(envelope) => envelope,
        Err(_) => return Ok(None),
    };
    if envelope.v != FIPS_CONTROL_VERSION {
        return Ok(None);
    }
    Ok(Some(envelope.frame))
}

#[derive(Debug, Clone, Default)]
pub struct FipsControlFragmentBuffer {
    entries: HashMap<ControlFragmentKey, PendingControlFragment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ControlFragmentKey {
    source_npub: String,
    id: String,
}

#[derive(Debug, Clone)]
struct PendingControlFragment {
    total: u16,
    received_at: u64,
    chunks: Vec<Option<Vec<u8>>>,
    received_len: usize,
}

impl FipsControlFragmentBuffer {
    pub fn decode(
        &mut self,
        source_npub: &str,
        data: &[u8],
        now: u64,
    ) -> Result<Option<FipsControlFrame>> {
        let Some(frame) = decode_fips_control_frame(data)? else {
            return Ok(None);
        };
        let FipsControlFrame::Fragment {
            id,
            index,
            total,
            data,
        } = frame
        else {
            return Ok(Some(frame));
        };

        let Some(reassembled) = self.push(source_npub, id, index, total, data, now)? else {
            return Ok(None);
        };
        decode_fips_control_frame(&reassembled)
    }

    pub fn push(
        &mut self,
        source_npub: &str,
        id: String,
        index: u16,
        total: u16,
        data: String,
        now: u64,
    ) -> Result<Option<Vec<u8>>> {
        if total == 0 || total > FIPS_CONTROL_MAX_FRAGMENTS || index >= total {
            return Ok(None);
        }

        self.entries.retain(|_, entry| {
            now.saturating_sub(entry.received_at) <= FIPS_CONTROL_FRAGMENT_TTL_SECS
        });

        let Ok(decoded) = URL_SAFE_NO_PAD.decode(data.as_bytes()) else {
            return Ok(None);
        };
        if decoded.len() > FIPS_CONTROL_FRAGMENT_CHUNK_LEN {
            return Ok(None);
        }

        let key = ControlFragmentKey {
            source_npub: source_npub.to_string(),
            id,
        };
        let entry = self
            .entries
            .entry(key.clone())
            .or_insert_with(|| PendingControlFragment {
                total,
                received_at: now,
                chunks: vec![None; usize::from(total)],
                received_len: 0,
            });
        if entry.total != total {
            *entry = PendingControlFragment {
                total,
                received_at: now,
                chunks: vec![None; usize::from(total)],
                received_len: 0,
            };
        }
        entry.received_at = now;

        let slot = &mut entry.chunks[usize::from(index)];
        if let Some(existing) = slot.as_ref() {
            entry.received_len = entry.received_len.saturating_sub(existing.len());
        }
        entry.received_len += decoded.len();
        if entry.received_len > FIPS_CONTROL_MAX_REASSEMBLED_LEN {
            self.entries.remove(&key);
            return Ok(None);
        }
        *slot = Some(decoded);

        if !entry.chunks.iter().all(|chunk| chunk.is_some()) {
            return Ok(None);
        }

        let entry = self
            .entries
            .remove(&key)
            .expect("complete fragment entry should exist");
        let mut reassembled = Vec::with_capacity(entry.received_len);
        for chunk in entry.chunks.into_iter().flatten() {
            reassembled.extend_from_slice(&chunk);
        }
        Ok(Some(reassembled))
    }
}

pub fn peer_endpoint_hint_addr(hint: &PeerEndpointHint) -> Option<String> {
    if !hint.transport.trim().eq_ignore_ascii_case("udp") {
        return None;
    }
    let trimmed = hint.addr.trim();
    if let Ok(parsed) = trimmed.parse::<SocketAddr>() {
        if parsed.port() == 0 || endpoint_hint_ip_is_unusable(parsed.ip()) {
            return None;
        }
        return Some(parsed.to_string());
    }

    let (host, port) = trimmed.rsplit_once(':')?;
    let host = host.trim();
    let port = port.trim().parse::<u16>().ok()?;
    if host.is_empty() || port == 0 || host.eq_ignore_ascii_case("localhost") {
        return None;
    }
    if host_looks_like_nostr_pubkey(host) {
        return None;
    }
    if host.contains(':') {
        return None;
    }
    if let Ok(ip) = host.parse::<IpAddr>()
        && endpoint_hint_ip_is_unusable(ip)
    {
        return None;
    }
    Some(format!("{host}:{port}"))
}

fn endpoint_hint_ip_is_unusable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ipv4_is_cgnat(ip)
                || ipv4_is_documentation(ip)
        }
        IpAddr::V6(ip) => {
            ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
        }
    }
}

fn ipv4_is_cgnat(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (64..=127).contains(&octets[1])
}

fn ipv4_is_documentation(ip: Ipv4Addr) -> bool {
    matches!(
        ip.octets(),
        [192, 0, 2, _] | [198, 51, 100, _] | [203, 0, 113, _]
    )
}

fn host_looks_like_nostr_pubkey(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    host.len() >= 60 && host.starts_with("npub1")
}

fn fips_control_fragment_id(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

pub fn roster_control_frame(
    network_id: impl Into<String>,
    roster: NetworkRoster,
) -> FipsControlFrame {
    FipsControlFrame::Roster {
        network_id: network_id.into(),
        roster,
    }
}

pub fn peer_capabilities_control_frame(
    network_id: impl Into<String>,
    capabilities: PeerCapabilities,
) -> FipsControlFrame {
    FipsControlFrame::Capabilities {
        network_id: network_id.into(),
        capabilities,
    }
}

pub fn network_roster_from_shared(
    network_name: String,
    participants: Vec<String>,
    admins: Vec<String>,
    aliases: HashMap<String, String>,
    signed_at: u64,
) -> NetworkRoster {
    NetworkRoster {
        network_name,
        participants,
        admins,
        aliases,
        signed_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_frame_roundtrips_with_magic_prefix() {
        let frame = FipsControlFrame::Ping {
            network_id: "mesh".to_string(),
            sent_at: 42,
        };

        let encoded = encode_fips_control_frame(&frame).expect("encode");
        assert!(encoded.starts_with(FIPS_CONTROL_MAGIC));

        let decoded = decode_fips_control_frame(&encoded)
            .expect("decode")
            .expect("control frame");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn raw_packet_is_not_control() {
        let packet = [0x45, 0, 0, 20];

        assert!(
            decode_fips_control_frame(&packet)
                .expect("decode")
                .is_none()
        );
    }

    #[test]
    fn capabilities_frame_roundtrips() {
        let frame = FipsControlFrame::Capabilities {
            network_id: "mesh".to_string(),
            capabilities: PeerCapabilities {
                advertised_routes: vec!["0.0.0.0/0".to_string(), "::/0".to_string()],
                endpoint_hints: vec![PeerEndpointHint::udp("192.168.50.22:51820")],
                signed_at: 99,
            },
        };

        let encoded = encode_fips_control_frame(&frame).expect("encode");
        let decoded = decode_fips_control_frame(&encoded)
            .expect("decode")
            .expect("control frame");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn old_capabilities_decode_with_empty_endpoint_hints() {
        let caps: PeerCapabilities =
            serde_json::from_str(r#"{"advertised_routes":["0.0.0.0/0"],"signed_at":99}"#)
                .expect("decode old capabilities");

        assert_eq!(caps.advertised_routes, vec!["0.0.0.0/0".to_string()]);
        assert!(caps.endpoint_hints.is_empty());
        assert_eq!(caps.signed_at, 99);
    }

    #[test]
    fn endpoint_hints_default_to_udp_transport() {
        let hint: PeerEndpointHint =
            serde_json::from_str(r#"{"addr":"192.168.50.22:51820"}"#).expect("decode hint");

        assert_eq!(hint.transport, "udp");
        assert_eq!(hint.addr, "192.168.50.22:51820");
    }

    #[test]
    fn peer_endpoint_hint_addr_accepts_lan_and_dns_udp_hints() {
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("192.168.50.22:51820")),
            Some("192.168.50.22:51820".to_string())
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("peer.example.com:51820")),
            Some("peer.example.com:51820".to_string())
        );
    }

    #[test]
    fn peer_endpoint_hint_addr_rejects_unusable_hints() {
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint {
                transport: "tcp".to_string(),
                addr: "192.168.50.22:51820".to_string(),
            }),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("192.168.50.22")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("127.0.0.1:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("100.120.94.10:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("198.51.100.10:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("0.0.0.0:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("localhost:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp(format!(
                "{}:51820",
                "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq"
            ))),
            None
        );
    }

    #[test]
    fn large_control_frame_fragments_under_direct_limit() {
        let roster = NetworkRoster {
            network_name: "Network 1".to_string(),
            participants: (0..12).map(|value| format!("{value:064x}")).collect(),
            admins: vec!["f".repeat(64)],
            aliases: (0..12)
                .map(|value| (format!("{value:064x}"), format!("node-{value}")))
                .collect(),
            signed_at: 123,
        };
        let frame = FipsControlFrame::Roster {
            network_id: "mesh".to_string(),
            roster,
        };

        let messages = encode_fips_control_messages(&frame).expect("fragment");

        assert!(messages.len() > 1);
        for message in messages {
            assert!(message.len() <= FIPS_CONTROL_DIRECT_FRAME_LIMIT);
            assert!(matches!(
                decode_fips_control_frame(&message).expect("decode"),
                Some(FipsControlFrame::Fragment { .. })
            ));
        }
    }

    #[test]
    fn fragment_buffer_decodes_fragmented_frame() {
        let roster = NetworkRoster {
            network_name: "Network 1".to_string(),
            participants: (0..12).map(|value| format!("{value:064x}")).collect(),
            admins: vec!["f".repeat(64)],
            aliases: (0..12)
                .map(|value| (format!("{value:064x}"), format!("node-{value}")))
                .collect(),
            signed_at: 123,
        };
        let frame = FipsControlFrame::Roster {
            network_id: "mesh".to_string(),
            roster,
        };
        let messages = encode_fips_control_messages(&frame).expect("fragment messages");
        let mut buffer = FipsControlFragmentBuffer::default();
        let mut decoded = None;

        for message in messages {
            decoded = buffer
                .decode("npub1source", &message, 1)
                .expect("decode with fragments")
                .or(decoded);
        }

        assert_eq!(decoded, Some(frame));
    }

    #[test]
    fn unknown_kind_is_dropped_silently() {
        let mut bytes = Vec::from(FIPS_CONTROL_MAGIC);
        bytes.extend_from_slice(br#"{"v":1,"frame":{"kind":"future_kind","x":1}}"#);

        assert!(decode_fips_control_frame(&bytes).expect("decode").is_none());
    }

    #[test]
    fn future_version_is_dropped_silently() {
        let mut bytes = Vec::from(FIPS_CONTROL_MAGIC);
        bytes
            .extend_from_slice(br#"{"v":99,"frame":{"kind":"ping","network_id":"x","sent_at":1}}"#);

        assert!(decode_fips_control_frame(&bytes).expect("decode").is_none());
    }
}
