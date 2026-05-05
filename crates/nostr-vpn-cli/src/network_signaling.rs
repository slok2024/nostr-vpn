use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use nostr_sdk::prelude::ToBech32;
use nostr_vpn_core::config::{
    AppConfig, maybe_autoconfigure_node, normalize_nostr_pubkey, normalize_runtime_network_id,
};
use nostr_vpn_core::control::PeerAnnouncement;
use nostr_vpn_core::signaling::{NetworkRoster, NostrSignalingClient, SignalPayload};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{
    DaemonControlRequest, UpdateRosterArgs, build_explicit_peer_announcement,
    clear_daemon_control_result, daemon_status, default_config_path, endpoint_is_local_only,
    load_config_with_overrides, load_or_default_config, local_signal_endpoint,
    request_daemon_reload, resolve_relays, runtime_effective_advertised_routes,
    signaling_networks_for_app, unix_timestamp, wait_for_daemon_control_ack,
    wait_for_daemon_control_result,
};

pub(crate) const NETWORK_INVITE_PREFIX: &str = "nvpn://invite/";
const NETWORK_INVITE_VERSION: u8 = 3;

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
        invite.inviter_npub = normalize_nostr_pubkey(&invite.inviter_npub)?;
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
    let inviter_pubkey = if invite.inviter_npub.trim().is_empty() {
        invite
            .admins
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("invite must include at least one admin"))?
    } else {
        invite.inviter_npub.clone()
    };
    let normalized_inviter_pubkey = normalize_nostr_pubkey(&inviter_pubkey)?;
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

    let (target_network_id, reset_membership) = if let Some(existing) =
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
        .network_by_id(&target_network_id)
        .map(network_should_adopt_invite)
        .unwrap_or(false);
    let inviter_already_configured = config
        .network_by_id(&target_network_id)
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

    config.set_network_enabled(&target_network_id, true)?;
    config.set_network_mesh_id(&target_network_id, &invite.network_id)?;
    if let Some(network) = config.network_by_id_mut(&target_network_id) {
        if reset_membership {
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
            .is_some_and(|request| {
                !network
                    .admins
                    .iter()
                    .any(|admin| admin == &request.recipient)
            })
        {
            network.outbound_join_request = None;
        }
    }

    if !inviter_already_configured && !invite.inviter_node_name.trim().is_empty() {
        let _ = config.set_peer_alias(&normalized_inviter_pubkey, &invite.inviter_node_name);
    }

    if should_adopt_name
        && !invite.network_name.trim().is_empty()
        && let Some(network) = config.network_by_id_mut(&target_network_id)
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

fn normalized_invite_pubkeys(pubkeys: &[String]) -> Result<Vec<String>> {
    let mut normalized = pubkeys
        .iter()
        .map(|pubkey| normalize_nostr_pubkey(pubkey))
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
        if !is_valid_relay_url(relay) {
            return Err(anyhow!("invalid invite relay '{relay}'"));
        }
        if !normalized.iter().any(|existing| existing == relay) {
            normalized.push(relay.to_string());
        }
    }
    Ok(normalized)
}

fn is_valid_relay_url(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("wss://") || trimmed.starts_with("ws://")
}

pub(crate) fn maybe_reload_running_daemon(config_path: &Path) {
    let status = match daemon_status(config_path) {
        Ok(status) => status,
        Err(error) => {
            eprintln!("config: failed to inspect daemon status after save: {error}");
            return;
        }
    };
    if !status.running {
        return;
    }
    clear_daemon_control_result(config_path);
    if let Err(error) = request_daemon_reload(config_path) {
        eprintln!("config: failed to request daemon reload after save: {error}");
        return;
    }
    if let Err(error) = wait_for_daemon_control_ack(config_path, Duration::from_secs(2)) {
        eprintln!("config: daemon did not acknowledge reload after save: {error}");
        return;
    }
    if let Err(error) = wait_for_daemon_control_result(
        config_path,
        DaemonControlRequest::Reload,
        Duration::from_secs(2),
    ) {
        eprintln!("config: daemon reload after save failed: {error}");
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum RosterEditAction {
    AddParticipant,
    RemoveParticipant,
    AddAdmin,
    RemoveAdmin,
}

fn to_npub(pubkey_hex: &str) -> String {
    nostr_sdk::PublicKey::from_hex(pubkey_hex)
        .ok()
        .and_then(|pubkey| pubkey.to_bech32().ok())
        .unwrap_or_else(|| pubkey_hex.to_string())
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

pub(crate) async fn update_active_network_roster(
    args: UpdateRosterArgs,
    action: RosterEditAction,
) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let mut app = load_or_default_config(&config_path)?;
    if let Some(network_id) = args.network_id {
        app.set_active_network_id(&network_id)?;
    }
    let active_network_id = app.active_network().id.clone();

    let mut changed = Vec::new();
    for participant in &args.participants {
        let normalized = match action {
            RosterEditAction::AddParticipant => {
                app.add_participant_to_network(&active_network_id, participant)?
            }
            RosterEditAction::RemoveParticipant => {
                let normalized = normalize_nostr_pubkey(participant)?;
                app.remove_participant_from_network(&active_network_id, participant)?;
                normalized
            }
            RosterEditAction::AddAdmin => {
                app.add_admin_to_network(&active_network_id, participant)?
            }
            RosterEditAction::RemoveAdmin => {
                let normalized = normalize_nostr_pubkey(participant)?;
                app.remove_admin_from_network(&active_network_id, participant)?;
                normalized
            }
        };
        changed.push(normalized);
    }

    app.ensure_defaults();
    maybe_autoconfigure_node(&mut app);
    app.save(&config_path)?;
    maybe_reload_running_daemon(&config_path);

    let mut published = 0usize;
    let relays = resolve_relays(&args.relays, &app);
    if args.publish {
        let client = NostrSignalingClient::from_secret_key_with_networks(
            &app.nostr.secret_key,
            signaling_networks_for_app(&app),
        )?;
        client
            .connect(&relays)
            .await
            .context("failed to connect signaling client")?;
        published = publish_active_network_roster(&client, &app, None).await?;
        client.disconnect().await;
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "network_id": app.effective_network_id(),
                "participants": app.active_network().participants,
                "admins": app.active_network().admins,
                "changed": changed,
                "published_recipients": published,
                "published": args.publish,
                "relays": if args.publish { relays } else { Vec::<String>::new() },
            }))?
        );
    } else {
        println!("saved {}", config_path.display());
        println!("network_id={}", app.effective_network_id());
        println!("changed={}", changed.join(","));
        if args.publish {
            println!("published_recipients={published}");
        }
    }

    Ok(())
}

#[derive(Debug)]
pub(crate) struct AnnounceRequest {
    pub(crate) config: Option<PathBuf>,
    pub(crate) network_id: Option<String>,
    pub(crate) participants: Vec<String>,
    pub(crate) node_id: Option<String>,
    pub(crate) endpoint: Option<String>,
    pub(crate) tunnel_ip: Option<String>,
    pub(crate) public_key: Option<String>,
    pub(crate) relay: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct PublishedAnnouncement {
    pub(crate) app: AppConfig,
    pub(crate) network_id: String,
    pub(crate) relays: Vec<String>,
    pub(crate) announcement: PeerAnnouncement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NetworkInvite {
    pub(crate) v: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) network_name: String,
    pub(crate) network_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) inviter_npub: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) inviter_node_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) admins: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) participants: Vec<String>,
    #[serde(default)]
    pub(crate) relays: Vec<String>,
}

pub(crate) async fn publish_active_network_roster(
    client: &NostrSignalingClient,
    app: &AppConfig,
    recipients: Option<&[String]>,
) -> Result<usize> {
    let network = app.active_network();
    let own_pubkey = match app.own_nostr_pubkey_hex() {
        Ok(pubkey) => pubkey,
        Err(_) => return Ok(0),
    };
    let roster = app.shared_network_roster(&network.id)?;
    if !crate::shared_roster_publish_allowed(app, &network.id, &own_pubkey, &roster.signed_by) {
        return Ok(0);
    }
    let allowed = app.active_network_signal_pubkeys_hex();
    let allowed_set = allowed.iter().cloned().collect::<HashSet<_>>();
    let mut recipients = recipients
        .map(|recipients| {
            recipients
                .iter()
                .filter(|recipient| allowed_set.contains(recipient.as_str()))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or(allowed);
    recipients.retain(|recipient| recipient != &own_pubkey);
    recipients.sort();
    recipients.dedup();
    if recipients.is_empty() {
        return Ok(0);
    }

    let payload = SignalPayload::Roster(NetworkRoster {
        network_name: roster.name,
        participants: roster.participants,
        admins: roster.admins,
        aliases: roster.aliases,
        signed_at: if roster.updated_at > 0 {
            roster.updated_at
        } else {
            unix_timestamp()
        },
    });
    client.publish_to(payload, &recipients).await?;
    Ok(recipients.len())
}

fn network_should_adopt_invite(network: &nostr_vpn_core::config::NetworkConfig) -> bool {
    let trimmed = network.name.trim();
    network.participants.is_empty() && (trimmed.is_empty() || trimmed.starts_with("Network "))
}

pub(crate) async fn publish_announcement(
    request: AnnounceRequest,
) -> Result<PublishedAnnouncement> {
    let config_path = request.config.unwrap_or_else(default_config_path);
    let (app, network_id) =
        load_config_with_overrides(&config_path, request.network_id, request.participants)?;
    let node_id = request.node_id.unwrap_or_else(|| app.node.id.clone());
    let endpoint = request
        .endpoint
        .unwrap_or_else(|| app.node.endpoint.clone());
    let tunnel_ip = request
        .tunnel_ip
        .unwrap_or_else(|| app.node.tunnel_ip.clone());
    let public_key = request
        .public_key
        .unwrap_or_else(|| app.node.public_key.clone());
    let relays = resolve_relays(&request.relay, &app);

    let client = NostrSignalingClient::from_secret_key_with_networks(
        &app.nostr.secret_key,
        signaling_networks_for_app(&app),
    )?;
    client.connect(&relays).await?;

    let listen_port = endpoint
        .parse::<SocketAddr>()
        .map(|addr| addr.port())
        .unwrap_or(app.node.listen_port);
    let local_endpoint = if endpoint_is_local_only(&endpoint) {
        endpoint.clone()
    } else {
        local_signal_endpoint(&app, listen_port)
    };
    let announcement = build_explicit_peer_announcement(
        node_id,
        public_key,
        endpoint,
        local_endpoint,
        tunnel_ip,
        runtime_effective_advertised_routes(&app),
    );

    client
        .publish(SignalPayload::Announce(announcement.clone()))
        .await
        .context("failed to publish presence signal")?;

    client.disconnect().await;

    Ok(PublishedAnnouncement {
        app,
        network_id,
        relays,
        announcement,
    })
}

pub(crate) async fn discover_peers(
    app: &AppConfig,
    _network_id: &str,
    relays: &[String],
    discover_secs: u64,
) -> Result<Vec<PeerAnnouncement>> {
    if discover_secs == 0 {
        return Ok(Vec::new());
    }

    let client = NostrSignalingClient::from_secret_key_with_networks(
        &app.nostr.secret_key,
        signaling_networks_for_app(app),
    )?;
    client.connect(relays).await?;
    let _ = client.publish(SignalPayload::Hello).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(discover_secs);
    let mut peers = std::collections::HashMap::<String, PeerAnnouncement>::new();

    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            break;
        }

        let wait_for = std::cmp::min(
            deadline.saturating_duration_since(now),
            Duration::from_millis(250),
        );

        match tokio::time::timeout(wait_for, client.recv()).await {
            Ok(Some(message)) => match message.payload {
                SignalPayload::Hello => {}
                SignalPayload::Announce(announcement) => {
                    let should_insert = peers
                        .get(&announcement.node_id)
                        .is_none_or(|existing| existing.timestamp <= announcement.timestamp);
                    if should_insert {
                        peers.insert(announcement.node_id.clone(), announcement);
                    }
                }
                SignalPayload::Disconnect { node_id } => {
                    peers.remove(&node_id);
                }
                SignalPayload::Roster(_) => {}
                SignalPayload::JoinRequest { .. } => {}
            },
            Ok(None) => break,
            Err(_) => continue,
        }
    }

    client.disconnect().await;

    let mut values = peers.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    Ok(values)
}
