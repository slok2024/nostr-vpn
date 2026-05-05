use std::collections::HashMap;

use super::sample_peer_announcement;
use crate::*;
use nostr_vpn_core::crypto::generate_keypair;
use nostr_vpn_core::presence::PeerPresenceBook;
use nostr_vpn_core::signaling::SignalPayload;

#[test]
fn cached_peerbook_keeps_connected_peer_count_after_presence_expires() {
    let mut config = AppConfig::generated();
    let participant = "11".repeat(32);
    config.networks[0].participants = vec![participant.clone()];

    let peer_keys = generate_keypair();
    let announcement = PeerAnnouncement {
        node_id: "peer-a".to_string(),
        public_key: peer_keys.public_key.clone(),
        endpoint: "203.0.113.20:51820".to_string(),
        local_endpoint: None,
        public_endpoint: Some("203.0.113.20:51820".to_string()),
        tunnel_ip: "10.44.0.2/32".to_string(),
        advertised_routes: Vec::new(),
        timestamp: 1,
    };

    let mut presence = PeerPresenceBook::default();
    assert!(presence.apply_signal(
        participant.clone(),
        SignalPayload::Announce(announcement.clone()),
        100,
    ));
    assert_eq!(presence.prune_stale(200, 20), vec![participant.clone()]);
    assert!(presence.active().is_empty());
    assert!(presence.announcement_for(&participant).is_some());

    let now = unix_timestamp();
    let runtime_peers = HashMap::from([(
        key_b64_to_hex(&peer_keys.public_key).expect("peer pubkey hex"),
        WireGuardPeerStatus {
            endpoint: Some("203.0.113.20:51820".to_string()),
            last_handshake_sec: Some(now - 5),
            last_handshake_nsec: Some(0),
            ..WireGuardPeerStatus::default()
        },
    )]);

    assert_eq!(
        connected_peer_count_for_runtime(&config, None, &presence, Some(&runtime_peers), now),
        1
    );

    let runtime_peer = peer_runtime_lookup(&announcement, Some(&runtime_peers))
        .expect("runtime peer should resolve from cached announcement");
    assert!(peer_has_recent_handshake(runtime_peer));
}

#[test]
fn known_private_announce_participants_include_cached_inactive_peers() {
    let mut config = AppConfig::generated();
    let participant = "11".repeat(32);
    config.networks[0].participants = vec![participant.clone()];

    let announcement = PeerAnnouncement {
        node_id: "peer-a".to_string(),
        public_key: generate_keypair().public_key,
        endpoint: "203.0.113.20:51820".to_string(),
        local_endpoint: None,
        public_endpoint: Some("203.0.113.20:51820".to_string()),
        tunnel_ip: "10.44.0.2/32".to_string(),
        advertised_routes: vec!["0.0.0.0/0".to_string()],
        timestamp: 1,
    };

    let mut presence = PeerPresenceBook::default();
    assert!(presence.apply_signal(
        participant.clone(),
        SignalPayload::Announce(announcement),
        100,
    ));
    assert_eq!(presence.prune_stale(200, 20), vec![participant.clone()]);
    assert!(presence.active().is_empty());
    assert!(presence.announcement_for(&participant).is_some());

    assert!(
        active_private_announce_participants(&config, None, &presence).is_empty(),
        "inactive cached peers should not be part of active announce refreshes"
    );
    assert_eq!(
        known_private_announce_participants(&config, None, &presence),
        vec![participant]
    );
}

#[test]
fn known_private_announce_repair_participants_only_include_peers_without_recent_handshake() {
    let mut config = AppConfig::generated();
    let healthy_participant = "11".repeat(32);
    let stale_participant = "22".repeat(32);
    config.networks[0].participants = vec![healthy_participant.clone(), stale_participant.clone()];

    let healthy_keys = generate_keypair();
    let stale_keys = generate_keypair();
    let healthy_announcement = PeerAnnouncement {
        node_id: "peer-a".to_string(),
        public_key: healthy_keys.public_key.clone(),
        endpoint: "203.0.113.20:51820".to_string(),
        local_endpoint: None,
        public_endpoint: Some("203.0.113.20:51820".to_string()),
        tunnel_ip: "10.44.0.2/32".to_string(),
        advertised_routes: Vec::new(),
        timestamp: 1,
    };
    let stale_announcement = PeerAnnouncement {
        node_id: "peer-b".to_string(),
        public_key: stale_keys.public_key.clone(),
        endpoint: "203.0.113.21:51820".to_string(),
        local_endpoint: None,
        public_endpoint: Some("203.0.113.21:51820".to_string()),
        tunnel_ip: "10.44.0.3/32".to_string(),
        advertised_routes: Vec::new(),
        timestamp: 1,
    };

    let mut presence = PeerPresenceBook::default();
    assert!(presence.apply_signal(
        healthy_participant.clone(),
        SignalPayload::Announce(healthy_announcement),
        100,
    ));
    assert!(presence.apply_signal(
        stale_participant.clone(),
        SignalPayload::Announce(stale_announcement),
        100,
    ));
    assert_eq!(
        presence.prune_stale(200, 20),
        vec![healthy_participant.clone(), stale_participant.clone()]
    );

    let now = unix_timestamp();
    let runtime_peers = HashMap::from([(
        key_b64_to_hex(&healthy_keys.public_key).expect("healthy peer pubkey hex"),
        WireGuardPeerStatus {
            endpoint: Some("203.0.113.20:51820".to_string()),
            last_handshake_sec: Some(now - 5),
            last_handshake_nsec: Some(0),
            ..WireGuardPeerStatus::default()
        },
    )]);

    assert_eq!(
        known_private_announce_repair_participants(&config, None, &presence, Some(&runtime_peers),),
        vec![stale_participant]
    );
}

#[test]
fn idle_handshake_within_wireguard_session_window_counts_mesh_as_ready() {
    let now = unix_timestamp();
    let runtime_peer = WireGuardPeerStatus {
        endpoint: Some("203.0.113.20:51820".to_string()),
        last_handshake_sec: Some(now - 120),
        last_handshake_nsec: Some(0),
        ..WireGuardPeerStatus::default()
    };

    assert!(peer_has_recent_handshake(&runtime_peer));
}

#[test]
fn stale_handshake_does_not_count_mesh_as_ready() {
    let now = unix_timestamp();
    let runtime_peer = WireGuardPeerStatus {
        endpoint: Some("203.0.113.20:51820".to_string()),
        last_handshake_sec: Some(now - 181),
        last_handshake_nsec: Some(0),
        ..WireGuardPeerStatus::default()
    };

    assert!(!peer_has_recent_handshake(&runtime_peer));
}

#[test]
fn daemon_peer_transport_state_reports_missing_signal() {
    let state = daemon_peer_transport_state(None, false, None, 1_700_000_000);

    assert!(!state.reachable);
    assert_eq!(state.last_handshake_at, None);
    assert_eq!(state.error.as_deref(), Some("no signal yet"));
}

#[test]
fn daemon_peer_transport_state_reports_invalid_peer_key() {
    let announcement = sample_peer_announcement("not-a-wireguard-key".to_string());

    let state = daemon_peer_transport_state(Some(&announcement), true, None, 1_700_000_000);

    assert!(!state.reachable);
    assert_eq!(state.error.as_deref(), Some("invalid peer key"));
}

#[test]
fn daemon_peer_transport_state_reports_signal_stale_before_runtime_error() {
    let keys = generate_keypair();
    let announcement = sample_peer_announcement(keys.public_key);

    let state = daemon_peer_transport_state(Some(&announcement), false, None, 1_700_000_000);

    assert!(!state.reachable);
    assert_eq!(state.error.as_deref(), Some("signal stale"));
}

#[test]
fn daemon_peer_transport_state_reports_missing_runtime_peer_for_fresh_signal() {
    let keys = generate_keypair();
    let announcement = sample_peer_announcement(keys.public_key);

    let state = daemon_peer_transport_state(Some(&announcement), true, None, 1_700_000_000);

    assert!(!state.reachable);
    assert_eq!(state.error.as_deref(), Some("peer not in tunnel runtime"));
}

#[test]
fn daemon_peer_transport_state_reports_awaiting_handshake_when_runtime_peer_is_idle() {
    let keys = generate_keypair();
    let announcement = sample_peer_announcement(keys.public_key);
    let runtime_peer = WireGuardPeerStatus {
        endpoint: Some("203.0.113.20:51820".to_string()),
        ..WireGuardPeerStatus::default()
    };

    let state = daemon_peer_transport_state(
        Some(&announcement),
        true,
        Some(&runtime_peer),
        1_700_000_000,
    );

    assert!(!state.reachable);
    assert_eq!(state.last_handshake_at, None);
    assert_eq!(state.error.as_deref(), Some("awaiting handshake"));
}

#[test]
fn handshake_age_converts_to_observed_epoch() {
    let now = 1_700_000_000;
    let runtime_peer = WireGuardPeerStatus {
        endpoint: Some("203.0.113.20:51820".to_string()),
        last_handshake_sec: Some(5),
        last_handshake_nsec: Some(0),
        ..WireGuardPeerStatus::default()
    };

    assert_eq!(runtime_peer.last_handshake_at(now), Some(now - 5));
}

#[test]
fn parse_wg_peer_status_extracts_transfer_counters() {
    let response = "\
public_key=peer-a\n\
endpoint=203.0.113.20:51820\n\
last_handshake_time_sec=1700000005\n\
last_handshake_time_nsec=0\n\
tx_bytes=1234\n\
rx_bytes=5678\n\
errno=0\n";

    let peers = crate::parse_wg_peer_status(response);
    let peer = peers.get("peer-a").expect("peer should parse");

    assert_eq!(peer.endpoint.as_deref(), Some("203.0.113.20:51820"));
    assert_eq!(peer.last_handshake_at(1_700_000_010), Some(1_700_000_005));
    assert_eq!(peer.tx_bytes, 1234);
    assert_eq!(peer.rx_bytes, 5678);
}

#[test]
fn parse_wg_peer_status_interprets_small_handshake_seconds_as_age() {
    let response = "\
public_key=peer-a\n\
endpoint=203.0.113.20:51820\n\
last_handshake_time_sec=11\n\
last_handshake_time_nsec=0\n\
tx_bytes=628\n\
rx_bytes=396\n\
errno=0\n";

    let peers = crate::parse_wg_peer_status(response);
    let peer = peers.get("peer-a").expect("peer should parse");
    let now = 1_775_232_141;

    assert_eq!(peer.last_handshake_at(now), Some(now - 11));
    assert!(peer_has_recent_handshake(peer));
    assert_eq!(peer.tx_bytes, 628);
    assert_eq!(peer.rx_bytes, 396);
}
