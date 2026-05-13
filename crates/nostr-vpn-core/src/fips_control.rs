use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

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
    pub signed_at: u64,
}

const FIPS_CONTROL_MAGIC: &[u8] = b"NVPN-FIPS-CTRL\0";
const FIPS_CONTROL_VERSION: u8 = 1;
pub const FIPS_CONTROL_DIRECT_FRAME_LIMIT: usize = 1100;
pub const FIPS_CONTROL_FRAGMENT_CHUNK_LEN: usize = 700;

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
