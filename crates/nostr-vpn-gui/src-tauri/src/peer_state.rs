use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use nostr_sdk::prelude::ToBech32;
use nostr_vpn_core::config::{AppConfig, normalize_nostr_pubkey, normalize_runtime_network_id};
use serde::{Deserialize, Serialize};

use super::{LAN_PAIRING_ANNOUNCEMENT_VERSION, NETWORK_INVITE_PREFIX};

const NETWORK_INVITE_VERSION: u8 = 3;

#[derive(Debug, Clone, Default)]
pub(crate) struct PeerLinkStatus {
    pub(crate) reachable: Option<bool>,
    pub(crate) last_handshake_at: Option<SystemTime>,
    pub(crate) endpoint: Option<String>,
    pub(crate) runtime_endpoint: Option<String>,
    pub(crate) tx_bytes: u64,
    pub(crate) rx_bytes: u64,
    pub(crate) error: Option<String>,
    pub(crate) checked_at: Option<SystemTime>,
    pub(crate) last_signal_seen_at: Option<SystemTime>,
    pub(crate) advertised_routes: Vec<String>,
    pub(crate) offers_exit_node: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfiguredPeerStatus {
    Local,
    Online,
    Present,
    Offline,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PeerPresenceStatus {
    Local,
    Present,
    Absent,
    Unknown,
}

#[derive(Debug, Clone)]
pub(crate) struct LanPeerRecord {
    pub(crate) npub: String,
    pub(crate) node_name: String,
    pub(crate) endpoint: String,
    pub(crate) network_name: String,
    pub(crate) network_id: String,
    pub(crate) invite: String,
    pub(crate) last_seen: SystemTime,
}

#[derive(Debug, Clone)]
pub(crate) struct LanPairingSignal {
    pub(crate) npub: String,
    pub(crate) node_name: String,
    pub(crate) endpoint: String,
    pub(crate) network_name: String,
    pub(crate) network_id: String,
    pub(crate) invite: String,
    pub(crate) seen_at: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LanAnnouncement {
    pub(crate) v: u8,
    pub(crate) npub: String,
    pub(crate) node_name: String,
    pub(crate) endpoint: String,
    pub(crate) invite: String,
    pub(crate) timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NetworkInvite {
    pub(crate) v: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) network_name: String,
    pub(crate) network_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) inviter_npub: String,
    #[serde(default)]
    pub(crate) inviter_node_name: String,
    #[serde(default)]
    pub(crate) admins: Vec<String>,
    #[serde(default)]
    pub(crate) participants: Vec<String>,
    #[serde(default)]
    pub(crate) relays: Vec<String>,
}

pub(crate) fn active_network_invite_code(config: &AppConfig) -> Result<String> {
    let active_network = config.active_network();
    let roster = config.shared_network_roster(&active_network.id)?;
    if roster.admins.is_empty() {
        return Err(anyhow!("active network has no admin configured"));
    }
    let invite = NetworkInvite {
        v: NETWORK_INVITE_VERSION,
        network_name: String::new(),
        network_id: roster.network_id,
        inviter_npub: String::new(),
        inviter_node_name: String::new(),
        admins: roster.admins.iter().map(|admin| to_npub(admin)).collect(),
        participants: Vec::new(),
        relays: normalized_invite_relays(&config.nostr.relays)?,
    };
    let encoded = URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&invite).context("failed to encode network invite JSON")?);
    Ok(format!("{NETWORK_INVITE_PREFIX}{encoded}"))
}

pub(crate) fn parse_network_invite(value: &str) -> Result<NetworkInvite> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("invite code is empty"));
    }

    let mut invite = if trimmed.starts_with('{') {
        serde_json::from_str::<NetworkInvite>(trimmed)
            .context("failed to parse network invite JSON")?
    } else {
        let payload = trimmed
            .strip_prefix(NETWORK_INVITE_PREFIX)
            .unwrap_or(trimmed);
        let decoded = URL_SAFE_NO_PAD
            .decode(payload)
            .context("failed to decode network invite payload")?;
        serde_json::from_slice::<NetworkInvite>(&decoded)
            .context("failed to parse network invite payload")?
    };

    if invite.v != 1 && invite.v != 2 && invite.v != NETWORK_INVITE_VERSION {
        return Err(anyhow!(
            "unsupported invite version {}; expected 1, 2, or {}",
            invite.v,
            NETWORK_INVITE_VERSION
        ));
    }

    invite.network_name = invite.network_name.trim().to_string();
    invite.network_id = invite.network_id.trim().to_string();
    if invite.network_id.is_empty() {
        return Err(anyhow!("invite network id is empty"));
    }

    invite.admins = normalized_invite_pubkeys(&invite.admins)?;
    if !invite.inviter_npub.trim().is_empty() {
        invite.inviter_npub = to_npub(&normalize_nostr_pubkey(&invite.inviter_npub)?);
        if !invite
            .admins
            .iter()
            .any(|admin| admin == &invite.inviter_npub)
        {
            invite.admins.push(invite.inviter_npub.clone());
        }
    } else {
        invite.inviter_npub = invite
            .admins
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("invite must include at least one admin"))?;
    }
    if invite.admins.is_empty() {
        invite.admins.push(invite.inviter_npub.clone());
        invite.admins.sort();
        invite.admins.dedup();
    }
    invite.inviter_node_name = invite.inviter_node_name.trim().to_string();
    invite.participants = normalized_invite_pubkeys(&invite.participants)?;
    if invite.participants.is_empty() && invite.v < NETWORK_INVITE_VERSION {
        invite.participants.push(invite.inviter_npub.clone());
    }
    invite.relays = normalized_invite_relays(&invite.relays)?;

    Ok(invite)
}

pub(crate) fn apply_network_invite_to_active_network(
    config: &mut AppConfig,
    invite: &NetworkInvite,
) -> Result<()> {
    let normalized_invite_network_id = normalize_runtime_network_id(&invite.network_id);
    let inviter_npub = if invite.inviter_npub.trim().is_empty() {
        invite
            .admins
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("invite must include at least one admin"))?
    } else {
        invite.inviter_npub.clone()
    };
    let normalized_inviter_pubkey = normalize_nostr_pubkey(&inviter_npub)?;
    let own_pubkey = config.own_nostr_pubkey_hex().ok();
    let invite_admins = invite
        .admins
        .iter()
        .map(|admin| normalize_nostr_pubkey(admin))
        .collect::<Result<Vec<_>>>()?;
    let invite_participants = invite
        .participants
        .iter()
        .map(|participant| normalize_nostr_pubkey(participant))
        .collect::<Result<Vec<_>>>()?;
    let (target_network_entry_id, reset_membership_from_invite) = if let Some(existing) =
        config.networks.iter().find(|network| {
            normalize_runtime_network_id(&network.network_id) == normalized_invite_network_id
        }) {
        (existing.id.clone(), false)
    } else if network_should_adopt_invite(config.active_network()) {
        (config.active_network().id.clone(), true)
    } else {
        let network_id = config.add_network(&invite.network_name);
        config.set_network_enabled(&network_id, true)?;
        (network_id, true)
    };
    let should_adopt_name = config
        .network_by_id(&target_network_entry_id)
        .map(network_should_adopt_invite)
        .unwrap_or(false);
    let inviter_already_configured = config
        .network_by_id(&target_network_entry_id)
        .map(|network| {
            network
                .participants
                .iter()
                .any(|participant| participant == &normalized_inviter_pubkey)
                || network
                    .admins
                    .iter()
                    .any(|admin| admin == &normalized_inviter_pubkey)
        })
        .unwrap_or(false);

    config.set_network_enabled(&target_network_entry_id, true)?;
    config.set_network_mesh_id(&target_network_entry_id, &invite.network_id)?;
    if let Some(network) = config.network_by_id_mut(&target_network_entry_id) {
        if reset_membership_from_invite {
            network.participants.clear();
            network.admins.clear();
            network.shared_roster_updated_at = 0;
            network.shared_roster_signed_by.clear();
        }

        for participant in &invite_participants {
            if own_pubkey.as_deref() == Some(participant.as_str()) {
                continue;
            }
            network.participants.push(participant.clone());
        }
        network.participants.sort();
        network.participants.dedup();

        for admin in &invite_admins {
            network.admins.push(admin.clone());
        }
        if !network
            .admins
            .iter()
            .any(|admin| admin == &normalized_inviter_pubkey)
        {
            network.admins.push(normalized_inviter_pubkey.clone());
        }
        network.admins.sort();
        network.admins.dedup();

        network.invite_inviter = if network
            .admins
            .iter()
            .any(|admin| admin == &normalized_inviter_pubkey)
        {
            normalized_inviter_pubkey.clone()
        } else {
            network.admins.first().cloned().unwrap_or_default()
        };
        if network
            .outbound_join_request
            .as_ref()
            .map(|request| {
                !network
                    .admins
                    .iter()
                    .any(|admin| admin == &request.recipient)
            })
            .unwrap_or(false)
        {
            network.outbound_join_request = None;
        }
    }

    if !inviter_already_configured && !invite.inviter_node_name.trim().is_empty() {
        let _ = config.set_peer_alias(&normalized_inviter_pubkey, &invite.inviter_node_name);
    }

    if should_adopt_name
        && !invite.network_name.trim().is_empty()
        && let Some(network) = config.network_by_id_mut(&target_network_entry_id)
    {
        network.name = invite.network_name.trim().to_string();
    }

    for relay in &invite.relays {
        if !config.nostr.relays.iter().any(|existing| existing == relay) {
            config.nostr.relays.push(relay.clone());
        }
    }

    Ok(())
}

pub(crate) fn connected_configured_peer_count(
    config: &AppConfig,
    peer_status: &HashMap<String, PeerLinkStatus>,
) -> usize {
    let own_pubkey = config.own_nostr_pubkey_hex().ok();
    let participants = config.participant_pubkeys_hex();

    participants
        .iter()
        .filter(|participant| Some(participant.as_str()) != own_pubkey.as_deref())
        .filter(|participant| {
            peer_status
                .get(*participant)
                .and_then(|status| status.reachable)
                .unwrap_or(false)
        })
        .count()
}

pub(crate) fn peer_state_label(state: ConfiguredPeerStatus) -> &'static str {
    match state {
        ConfiguredPeerStatus::Local => "local",
        ConfiguredPeerStatus::Online => "online",
        ConfiguredPeerStatus::Present => "pending",
        ConfiguredPeerStatus::Offline => "offline",
        ConfiguredPeerStatus::Unknown => "unknown",
    }
}

pub(crate) fn peer_presence_state_label(state: PeerPresenceStatus) -> &'static str {
    match state {
        PeerPresenceStatus::Local => "local",
        PeerPresenceStatus::Present => "present",
        PeerPresenceStatus::Absent => "absent",
        PeerPresenceStatus::Unknown => "unknown",
    }
}

pub(crate) fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(crate) fn to_npub(pubkey_hex: &str) -> String {
    nostr_sdk::PublicKey::from_hex(pubkey_hex)
        .ok()
        .and_then(|pubkey| pubkey.to_bech32().ok())
        .unwrap_or_else(|| pubkey_hex.to_string())
}

pub(crate) fn decode_lan_pairing_announcement(
    payload: &[u8],
    own_npub: &str,
) -> Option<LanPairingSignal> {
    let parsed = serde_json::from_slice::<LanAnnouncement>(payload).ok()?;
    if parsed.v != LAN_PAIRING_ANNOUNCEMENT_VERSION || parsed.npub == own_npub {
        return None;
    }

    let invite = parse_network_invite(&parsed.invite).ok()?;
    if !invite.admins.iter().any(|admin| admin == &parsed.npub) {
        return None;
    }

    Some(LanPairingSignal {
        npub: parsed.npub,
        node_name: parsed.node_name,
        endpoint: parsed.endpoint,
        network_name: invite.display_name().to_string(),
        network_id: invite.network_id,
        invite: parsed.invite,
        seen_at: SystemTime::now(),
    })
}

fn network_should_adopt_invite(network: &nostr_vpn_core::config::NetworkConfig) -> bool {
    let trimmed = network.name.trim();
    network.participants.is_empty() && (trimmed.is_empty() || trimmed.starts_with("Network "))
}

pub(crate) fn preferred_join_request_recipient(
    network: &nostr_vpn_core::config::NetworkConfig,
) -> Option<String> {
    if !network.invite_inviter.is_empty()
        && network
            .admins
            .iter()
            .any(|admin| admin == &network.invite_inviter)
    {
        return Some(network.invite_inviter.clone());
    }

    network.admins.first().cloned()
}

impl NetworkInvite {
    pub(crate) fn display_name(&self) -> &str {
        if self.network_name.trim().is_empty() {
            &self.network_id
        } else {
            &self.network_name
        }
    }
}

fn normalized_invite_pubkeys(pubkeys: &[String]) -> Result<Vec<String>> {
    let mut normalized = pubkeys
        .iter()
        .map(|pubkey| normalize_nostr_pubkey(pubkey).map(|value| to_npub(&value)))
        .collect::<Result<Vec<_>>>()?;
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn normalized_invite_relays(relays: &[String]) -> Result<Vec<String>> {
    let mut normalized = Vec::new();
    for relay in relays {
        let relay = relay.trim();
        if relay.is_empty() {
            continue;
        }
        if !super::is_valid_relay_url(relay) {
            return Err(anyhow!("invalid invite relay '{relay}'"));
        }
        if !normalized.iter().any(|existing| existing == relay) {
            normalized.push(relay.to_string());
        }
    }
    Ok(normalized)
}
