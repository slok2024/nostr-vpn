use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nostr_vpn_core::config::{
    AppConfig, PendingInboundJoinRequest, PendingOutboundJoinRequest, derive_mesh_tunnel_ip,
    normalize_runtime_network_id,
};

use crate::ui_models::to_npub;
use crate::ui_types::{
    DaemonRuntimeState, InboundJoinRequestView, NetworkView, OutboundJoinRequestView,
    ParticipantView,
};

const PEER_PRESENCE_GRACE_SECS: u64 = 45;

#[derive(Debug, Clone, Default)]
struct PeerSnapshot {
    reachable: Option<bool>,
    last_handshake_at: Option<SystemTime>,
    endpoint: Option<String>,
    tx_bytes: u64,
    rx_bytes: u64,
    error: Option<String>,
    last_signal_seen_at: Option<SystemTime>,
    advertised_routes: Vec<String>,
    offers_exit_node: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransportStatus {
    Local,
    Online,
    Present,
    Offline,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresenceStatus {
    Local,
    Present,
    Absent,
    Unknown,
}

pub(crate) struct NetworkRuntimeViews {
    pub(crate) networks: Vec<NetworkView>,
    pub(crate) expected_peer_count: usize,
    pub(crate) connected_peer_count: usize,
}

pub(crate) fn build_network_views(
    config: &AppConfig,
    daemon_state: Option<&DaemonRuntimeState>,
    session_active: bool,
) -> NetworkRuntimeViews {
    let peer_snapshots = peer_snapshots(config, daemon_state, session_active);
    let networks = network_rows(config, &peer_snapshots, session_active);
    let expected_peer_count = expected_peer_count(config);
    let connected_peer_count = connected_configured_peer_count(config, &peer_snapshots);

    NetworkRuntimeViews {
        networks,
        expected_peer_count,
        connected_peer_count,
    }
}

pub(crate) fn is_mesh_complete(connected: usize, expected: usize) -> bool {
    expected > 0 && connected >= expected
}

fn peer_snapshots(
    config: &AppConfig,
    daemon_state: Option<&DaemonRuntimeState>,
    session_active: bool,
) -> HashMap<String, PeerSnapshot> {
    let daemon_peers = daemon_state
        .map(|state| {
            state
                .peers
                .iter()
                .map(|peer| (peer.participant_pubkey.as_str(), peer))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    config
        .all_participant_pubkeys_hex()
        .into_iter()
        .map(|participant| {
            let snapshot = if !session_active {
                PeerSnapshot {
                    error: Some("vpn off".to_string()),
                    ..PeerSnapshot::default()
                }
            } else if let Some(peer) = daemon_peers.get(participant.as_str()) {
                let last_signal_seen_at = peer
                    .last_signal_seen_at
                    .and_then(epoch_secs_to_system_time)
                    .or_else(|| epoch_secs_to_system_time(peer.presence_timestamp));
                PeerSnapshot {
                    reachable: Some(peer.reachable),
                    last_handshake_at: peer.last_handshake_at.and_then(epoch_secs_to_system_time),
                    endpoint: (!peer.endpoint.trim().is_empty()).then(|| peer.endpoint.clone()),
                    tx_bytes: peer.tx_bytes,
                    rx_bytes: peer.rx_bytes,
                    error: if peer.reachable {
                        None
                    } else {
                        Some(
                            peer.error
                                .clone()
                                .unwrap_or_else(|| "fips presence pending".to_string()),
                        )
                    },
                    last_signal_seen_at,
                    advertised_routes: peer.advertised_routes.clone(),
                    offers_exit_node: peer_offers_exit_node(&peer.advertised_routes),
                }
            } else {
                PeerSnapshot {
                    reachable: Some(false),
                    error: Some("no signal yet".to_string()),
                    ..PeerSnapshot::default()
                }
            };
            (participant, snapshot)
        })
        .collect()
}

fn network_rows(
    config: &AppConfig,
    snapshots: &HashMap<String, PeerSnapshot>,
    session_active: bool,
) -> Vec<NetworkView> {
    let own_pubkey_hex = config.own_nostr_pubkey_hex().ok();
    let mut rows = Vec::with_capacity(config.networks.len());

    for network in &config.networks {
        let mut participants = network.participants.clone();
        participants.sort();
        participants.dedup();

        let mut admin_npubs = network
            .admins
            .iter()
            .map(|admin| to_npub(admin))
            .collect::<Vec<_>>();
        admin_npubs.sort();
        admin_npubs.dedup();

        let participant_rows = participants
            .iter()
            .map(|participant| {
                participant_view(
                    config,
                    snapshots,
                    participant,
                    &network.network_id,
                    own_pubkey_hex.as_deref(),
                    network.admins.iter().any(|admin| admin == participant),
                )
            })
            .collect::<Vec<_>>();

        let remote_expected_count = if network.enabled {
            participants
                .iter()
                .filter(|participant| Some(participant.as_str()) != own_pubkey_hex.as_deref())
                .count()
        } else {
            0
        };
        let remote_online_count = if network.enabled {
            participants
                .iter()
                .filter(|participant| Some(participant.as_str()) != own_pubkey_hex.as_deref())
                .filter(|participant| {
                    matches!(
                        peer_transport_status(
                            snapshots.get(participant.as_str()),
                            participant,
                            own_pubkey_hex.as_deref()
                        ),
                        TransportStatus::Online
                    )
                })
                .count()
        } else {
            0
        };

        rows.push(NetworkView {
            id: network.id.clone(),
            name: network.name.clone(),
            enabled: network.enabled,
            network_id: normalize_runtime_network_id(&network.network_id),
            local_is_admin: own_pubkey_hex
                .as_deref()
                .is_some_and(|pubkey| network.admins.iter().any(|admin| admin == pubkey)),
            admin_npubs,
            listen_for_join_requests: network.listen_for_join_requests,
            invite_inviter_npub: if network.invite_inviter.is_empty() {
                String::new()
            } else {
                to_npub(&network.invite_inviter)
            },
            outbound_join_request: network
                .outbound_join_request
                .as_ref()
                .map(outbound_join_request_view),
            inbound_join_requests: inbound_join_request_views(&network.inbound_join_requests),
            online_count: network_online_device_count(
                remote_online_count,
                network.enabled,
                session_active,
            ),
            expected_count: network_device_count(remote_expected_count, network.enabled),
            participants: participant_rows,
        });
    }

    rows
}

fn participant_view(
    config: &AppConfig,
    snapshots: &HashMap<String, PeerSnapshot>,
    participant: &str,
    network_id: &str,
    own_pubkey_hex: Option<&str>,
    is_admin: bool,
) -> ParticipantView {
    let snapshot = snapshots.get(participant);
    let transport_status = peer_transport_status(snapshot, participant, own_pubkey_hex);
    let presence_status = peer_presence_status(snapshot, participant, own_pubkey_hex);
    let is_local = Some(participant) == own_pubkey_hex;
    let (magic_dns_alias, magic_dns_name) = if is_local {
        (
            config.self_magic_dns_label().unwrap_or_default(),
            config.self_magic_dns_name().unwrap_or_default(),
        )
    } else {
        (
            config.peer_alias(participant).unwrap_or_default(),
            config
                .magic_dns_name_for_participant(participant)
                .unwrap_or_default(),
        )
    };
    let advertised_routes = if is_local {
        config.effective_advertised_routes()
    } else {
        snapshot
            .map(|value| value.advertised_routes.clone())
            .unwrap_or_default()
    };
    let offers_exit_node = if is_local {
        config.node.advertise_exit_node
    } else {
        snapshot
            .map(|value| value.offers_exit_node)
            .unwrap_or(false)
    };
    ParticipantView {
        npub: to_npub(participant),
        pubkey_hex: participant.to_string(),
        is_admin,
        tunnel_ip: derive_mesh_tunnel_ip(network_id, participant)
            .unwrap_or_else(|| "-".to_string()),
        magic_dns_alias,
        magic_dns_name,
        tx_bytes: snapshot.map(|value| value.tx_bytes).unwrap_or(0),
        rx_bytes: snapshot.map(|value| value.rx_bytes).unwrap_or(0),
        advertised_routes,
        offers_exit_node,
        state: transport_state_label(transport_status).to_string(),
        presence_state: presence_state_label(presence_status).to_string(),
        status_text: peer_status_line(snapshot, transport_status),
        last_signal_text: peer_presence_line(snapshot, participant, own_pubkey_hex),
    }
}

fn outbound_join_request_view(request: &PendingOutboundJoinRequest) -> OutboundJoinRequestView {
    OutboundJoinRequestView {
        recipient_npub: to_npub(&request.recipient),
        recipient_pubkey_hex: request.recipient.clone(),
        requested_at_text: join_request_age_text(request.requested_at),
    }
}

fn inbound_join_request_views(
    requests: &[PendingInboundJoinRequest],
) -> Vec<InboundJoinRequestView> {
    requests
        .iter()
        .map(|request| InboundJoinRequestView {
            requester_npub: to_npub(&request.requester),
            requester_pubkey_hex: request.requester.clone(),
            requester_node_name: request.requester_node_name.clone(),
            requested_at_text: join_request_age_text(request.requested_at),
        })
        .collect()
}

fn peer_transport_status(
    snapshot: Option<&PeerSnapshot>,
    participant: &str,
    own_pubkey_hex: Option<&str>,
) -> TransportStatus {
    if Some(participant) == own_pubkey_hex {
        return TransportStatus::Local;
    }

    match snapshot {
        Some(status) if status.reachable == Some(true) => TransportStatus::Online,
        Some(status) if within_peer_presence_grace(status.last_signal_seen_at) => {
            TransportStatus::Present
        }
        Some(status) if status.reachable == Some(false) => TransportStatus::Offline,
        _ => TransportStatus::Unknown,
    }
}

fn peer_presence_status(
    snapshot: Option<&PeerSnapshot>,
    participant: &str,
    own_pubkey_hex: Option<&str>,
) -> PresenceStatus {
    if Some(participant) == own_pubkey_hex {
        return PresenceStatus::Local;
    }

    match snapshot {
        Some(status) if status.reachable == Some(true) => PresenceStatus::Present,
        Some(status) if within_peer_presence_grace(status.last_signal_seen_at) => {
            PresenceStatus::Present
        }
        Some(status) if status.reachable == Some(false) => PresenceStatus::Absent,
        _ => PresenceStatus::Unknown,
    }
}

fn peer_status_line(snapshot: Option<&PeerSnapshot>, status: TransportStatus) -> String {
    match status {
        TransportStatus::Local => "local".to_string(),
        TransportStatus::Online => match snapshot
            .and_then(|value| value.last_handshake_at)
            .and_then(|handshake_at| handshake_at.elapsed().ok())
            .map(|elapsed| elapsed.as_secs())
        {
            Some(age_secs) => format!("online (seen {})", compact_age_text(age_secs)),
            None => "online".to_string(),
        },
        TransportStatus::Present => match snapshot.and_then(|value| value.endpoint.as_deref()) {
            Some(endpoint) if !endpoint.trim().is_empty() => {
                format!("fips pending via {}", shorten_middle(endpoint, 18, 10))
            }
            _ => "fips presence pending".to_string(),
        },
        TransportStatus::Offline => match snapshot {
            Some(value) => {
                let checked = value
                    .last_signal_seen_at
                    .and_then(|seen_at| seen_at.elapsed().ok())
                    .map(|elapsed| elapsed.as_secs());
                match (value.error.as_deref(), checked) {
                    (Some(error), Some(age_secs)) => format!(
                        "offline ({}, {})",
                        shorten_middle(error, 18, 8),
                        compact_age_text(age_secs)
                    ),
                    (Some(error), None) => {
                        format!("offline ({})", shorten_middle(error, 18, 8))
                    }
                    (None, Some(age_secs)) => format!("offline ({})", compact_age_text(age_secs)),
                    (None, None) => "offline".to_string(),
                }
            }
            None => "offline".to_string(),
        },
        TransportStatus::Unknown => "unknown".to_string(),
    }
}

fn peer_presence_line(
    snapshot: Option<&PeerSnapshot>,
    participant: &str,
    own_pubkey_hex: Option<&str>,
) -> String {
    if Some(participant) == own_pubkey_hex {
        return "self".to_string();
    }

    let Some(seen_at) = snapshot.and_then(|value| value.last_signal_seen_at) else {
        return "nostr unseen".to_string();
    };

    let age_secs = seen_at
        .elapsed()
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    format!("nostr seen {}", compact_age_text(age_secs))
}

fn transport_state_label(status: TransportStatus) -> &'static str {
    match status {
        TransportStatus::Local => "local",
        TransportStatus::Online => "online",
        TransportStatus::Present => "pending",
        TransportStatus::Offline => "offline",
        TransportStatus::Unknown => "unknown",
    }
}

fn presence_state_label(status: PresenceStatus) -> &'static str {
    match status {
        PresenceStatus::Local => "local",
        PresenceStatus::Present => "present",
        PresenceStatus::Absent => "absent",
        PresenceStatus::Unknown => "unknown",
    }
}

fn within_peer_presence_grace(last_seen_at: Option<SystemTime>) -> bool {
    last_seen_at
        .and_then(|seen_at| seen_at.elapsed().ok())
        .map(|elapsed| elapsed.as_secs() <= PEER_PRESENCE_GRACE_SECS)
        .unwrap_or(false)
}

fn peer_offers_exit_node(routes: &[String]) -> bool {
    routes
        .iter()
        .any(|route| route == "0.0.0.0/0" || route == "::/0")
}

fn epoch_secs_to_system_time(value: u64) -> Option<SystemTime> {
    if value == 0 {
        return None;
    }
    UNIX_EPOCH.checked_add(Duration::from_secs(value))
}

fn compact_age_text(age_secs: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    const MONTH: u64 = 30 * DAY;
    const YEAR: u64 = 365 * DAY;

    match age_secs {
        0..MINUTE => format!("{age_secs}s ago"),
        MINUTE..HOUR => format!("{}m ago", age_secs / MINUTE),
        HOUR..DAY => format!("{}h ago", age_secs / HOUR),
        DAY..WEEK => format!("{}d ago", age_secs / DAY),
        WEEK..MONTH => format!("{}w ago", age_secs / WEEK),
        MONTH..YEAR => format!("{}mo ago", age_secs / MONTH),
        _ => format!("{}y ago", age_secs / YEAR),
    }
}

fn join_request_age_text(requested_at: u64) -> String {
    let age_secs = epoch_secs_to_system_time(requested_at)
        .and_then(|requested_at| requested_at.elapsed().ok())
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    compact_age_text(age_secs)
}

fn shorten_middle(value: &str, head: usize, tail: usize) -> String {
    if value.len() <= head + tail + 3 {
        return value.to_string();
    }
    format!(
        "{}...{}",
        value.chars().take(head).collect::<String>(),
        value
            .chars()
            .rev()
            .take(tail)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
    )
}

fn expected_peer_count(config: &AppConfig) -> usize {
    let participants = config.participant_pubkeys_hex();
    if participants.is_empty() {
        return 0;
    }
    let mut expected = participants.len();
    if let Ok(own_pubkey) = config.own_nostr_pubkey_hex()
        && participants
            .iter()
            .any(|participant| participant == &own_pubkey)
    {
        expected = expected.saturating_sub(1);
    }
    expected
}

fn connected_configured_peer_count(
    config: &AppConfig,
    snapshots: &HashMap<String, PeerSnapshot>,
) -> usize {
    let own_pubkey = config.own_nostr_pubkey_hex().ok();
    config
        .participant_pubkeys_hex()
        .iter()
        .filter(|participant| Some(participant.as_str()) != own_pubkey.as_deref())
        .filter(|participant| {
            snapshots
                .get(participant.as_str())
                .and_then(|snapshot| snapshot.reachable)
                .unwrap_or(false)
        })
        .count()
}

fn network_device_count(remote_device_count: usize, enabled: bool) -> usize {
    if enabled {
        remote_device_count.saturating_add(1)
    } else {
        0
    }
}

fn network_online_device_count(
    remote_online_count: usize,
    enabled: bool,
    session_active: bool,
) -> usize {
    if enabled {
        remote_online_count.saturating_add(usize::from(session_active))
    } else {
        0
    }
}
