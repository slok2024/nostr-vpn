//! Canonical `nvpn://invite/...` payload.
//!
//! The CLI and the native-app cores both decode/encode invites; both used to
//! ship near-identical copies. This module is the single source of truth for
//! the wire shape (camelCase JSON, version 3) and the `parse_network_invite`
//! / `to_npub` helpers. Higher-level "apply this invite to my config" logic
//! still lives crate-locally because each consumer has its own config model.

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use nostr_sdk::prelude::{PublicKey, ToBech32};
use serde::{Deserialize, Serialize};

use crate::config::normalize_nostr_pubkey;

pub const NETWORK_INVITE_PREFIX: &str = "nvpn://invite/";
pub const NETWORK_INVITE_VERSION: u8 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInvite {
    pub v: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub network_name: String,
    pub network_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub invite_secret: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub inviter_npub: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub inviter_node_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inviter_endpoints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub admins: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relays: Vec<String>,
}

/// Decode `nvpn://invite/<base64>` (or a bare JSON document) into a normalized
/// `NetworkInvite`. Normalization: trims whitespace, npub-encodes all pubkeys,
/// derives the inviter from the first admin when omitted, drops legacy relay
/// hints.
pub fn parse_network_invite(value: &str) -> Result<NetworkInvite> {
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
        if payload.trim().is_empty() {
            return Err(anyhow!("invite payload is empty"));
        }
        if looks_like_invite_placeholder(payload) {
            return Err(anyhow!(
                "invite code is a placeholder; paste the full nvpn://invite/... value printed by `nvpn create-invite`"
            ));
        }
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
    invite.invite_secret = invite.invite_secret.trim().to_string();

    invite.admins = normalized_invite_pubkeys(&invite.admins)?;
    if invite.inviter_npub.trim().is_empty() {
        invite.inviter_npub = invite
            .admins
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("invite must include at least one admin"))?;
    } else {
        invite.inviter_npub = to_npub(&normalize_nostr_pubkey(&invite.inviter_npub)?);
        if !invite
            .admins
            .iter()
            .any(|admin| admin == &invite.inviter_npub)
        {
            invite.admins.push(invite.inviter_npub.clone());
        }
    }
    if invite.admins.is_empty() {
        invite.admins.push(invite.inviter_npub.clone());
        invite.admins.sort();
        invite.admins.dedup();
    }

    invite.inviter_node_name = invite.inviter_node_name.trim().to_string();
    invite.inviter_endpoints = normalized_invite_strings(&invite.inviter_endpoints);
    invite.participants = normalized_invite_pubkeys(&invite.participants)?;
    if invite.participants.is_empty() && invite.v < NETWORK_INVITE_VERSION {
        invite.participants.push(invite.inviter_npub.clone());
    }
    invite.relays.clear();

    Ok(invite)
}

/// Encode a `NetworkInvite` into `nvpn://invite/<base64>`.
pub fn encode_network_invite(invite: &NetworkInvite) -> Result<String> {
    let bytes = serde_json::to_vec(invite).context("failed to encode network invite JSON")?;
    Ok(format!(
        "{NETWORK_INVITE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(bytes)
    ))
}

/// Convert a 32-byte hex pubkey to its `npub1...` bech32 form. Returns the
/// original string if it's not valid hex — callers that need a hard error
/// should `normalize_nostr_pubkey` first.
pub fn to_npub(pubkey_hex: &str) -> String {
    PublicKey::parse(pubkey_hex)
        .ok()
        .and_then(|pubkey| pubkey.to_bech32().ok())
        .unwrap_or_else(|| pubkey_hex.to_string())
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

fn normalized_invite_strings(values: &[String]) -> Vec<String> {
    let mut normalized = values
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn looks_like_invite_placeholder(payload: &str) -> bool {
    let trimmed = payload.trim();
    trimmed.contains("...")
        || trimmed.contains('…')
        || matches!(
            trimmed,
            "<code>" | "<payload>" | "<invite>" | "<full-invite-code>"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_invite_has_actionable_error() {
        let error = parse_network_invite("nvpn://invite/...").expect_err("placeholder fails");

        assert!(
            error.to_string().contains("placeholder"),
            "unexpected error: {error:#}"
        );
        assert!(
            error.to_string().contains("nvpn create-invite"),
            "unexpected error: {error:#}"
        );
    }
}
