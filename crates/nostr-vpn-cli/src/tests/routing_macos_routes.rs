use std::collections::HashMap;

use crate::*;

use nostr_sdk::prelude::Keys;
use nostr_vpn_core::crypto::generate_keypair;
use nostr_vpn_core::paths::PeerPathBook;

#[test]
fn macos_route_targets_add_default_route_for_selected_exit_peer() {
    let mut config = AppConfig::generated();
    let exit_participant = Keys::generate().public_key().to_hex();
    config.networks[0].participants = vec![exit_participant.clone()];
    config.exit_node = exit_participant.clone();
    config.ensure_defaults();

    let announcements = HashMap::from([(
        exit_participant.clone(),
        PeerAnnouncement {
            node_id: "exit-node".to_string(),
            public_key: generate_keypair().public_key,
            endpoint: "203.0.113.20:51820".to_string(),
            local_endpoint: None,
            public_endpoint: Some("203.0.113.20:51820".to_string()),
            tunnel_ip: "10.44.0.2/32".to_string(),
            advertised_routes: vec!["0.0.0.0/0".to_string(), "10.60.0.0/24".to_string()],
            timestamp: 1,
        },
    )]);

    let planned = planned_tunnel_peers(
        &config,
        None,
        &announcements,
        &mut PeerPathBook::default(),
        Some("192.0.2.10:51820"),
        10,
    )
    .expect("planned tunnel peers");

    let routes = route_targets_for_planned_tunnel_peers(
        &config,
        None,
        &announcements,
        &planned,
        &PeerPathBook::default(),
        None,
        10,
    );

    assert_eq!(
        routes,
        vec!["10.44.0.2/32".to_string(), "10.60.0.0/24".to_string()]
    );
}

#[test]
fn macos_route_targets_add_default_route_for_selected_exit_peer_after_handshake() {
    let now = unix_timestamp();
    let mut config = AppConfig::generated();
    let exit_participant = Keys::generate().public_key().to_hex();
    config.networks[0].participants = vec![exit_participant.clone()];
    config.exit_node = exit_participant.clone();
    config.ensure_defaults();

    let public_key = generate_keypair().public_key;
    let public_key_hex = crate::key_b64_to_hex(&public_key).expect("peer public key hex");
    let announcements = HashMap::from([(
        exit_participant.clone(),
        PeerAnnouncement {
            node_id: "exit-node".to_string(),
            public_key,
            endpoint: "203.0.113.20:51820".to_string(),
            local_endpoint: None,
            public_endpoint: Some("203.0.113.20:51820".to_string()),
            tunnel_ip: "10.44.0.2/32".to_string(),
            advertised_routes: vec!["0.0.0.0/0".to_string(), "10.60.0.0/24".to_string()],
            timestamp: 1,
        },
    )]);

    let planned = planned_tunnel_peers(
        &config,
        None,
        &announcements,
        &mut PeerPathBook::default(),
        Some("192.0.2.10:51820"),
        10,
    )
    .expect("planned tunnel peers");

    let runtime_peers = HashMap::from([(
        public_key_hex,
        WireGuardPeerStatus {
            endpoint: Some("203.0.113.20:51820".to_string()),
            last_handshake_sec: Some(now - 1),
            ..Default::default()
        },
    )]);

    let routes = route_targets_for_planned_tunnel_peers(
        &config,
        None,
        &announcements,
        &planned,
        &PeerPathBook::default(),
        Some(&runtime_peers),
        now,
    );

    assert_eq!(
        routes,
        vec![
            "0.0.0.0/0".to_string(),
            "10.44.0.2/32".to_string(),
            "10.60.0.0/24".to_string(),
        ]
    );
}

#[test]
fn macos_route_targets_add_default_route_for_selected_exit_peer_on_runtime_public_endpoint() {
    let now = unix_timestamp();
    let mut config = AppConfig::generated();
    let exit_participant = Keys::generate().public_key().to_hex();
    config.networks[0].participants = vec![exit_participant.clone()];
    config.exit_node = exit_participant.clone();
    config.ensure_defaults();

    let public_key = generate_keypair().public_key;
    let public_key_hex = crate::key_b64_to_hex(&public_key).expect("peer public key hex");
    let announcements = HashMap::from([(
        exit_participant.clone(),
        PeerAnnouncement {
            node_id: "exit-node".to_string(),
            public_key,
            endpoint: "203.0.113.20:51820".to_string(),
            local_endpoint: None,
            public_endpoint: Some("203.0.113.20:51820".to_string()),
            tunnel_ip: "10.44.0.2/32".to_string(),
            advertised_routes: vec!["0.0.0.0/0".to_string(), "10.60.0.0/24".to_string()],
            timestamp: 1,
        },
    )]);

    let planned = planned_tunnel_peers(
        &config,
        None,
        &announcements,
        &mut PeerPathBook::default(),
        Some("192.0.2.10:51820"),
        10,
    )
    .expect("planned tunnel peers");

    let runtime_peers = HashMap::from([(
        public_key_hex,
        WireGuardPeerStatus {
            endpoint: Some("198.51.100.40:51820".to_string()),
            last_handshake_sec: Some(now - 1),
            ..Default::default()
        },
    )]);

    let routes = route_targets_for_planned_tunnel_peers(
        &config,
        None,
        &announcements,
        &planned,
        &PeerPathBook::default(),
        Some(&runtime_peers),
        now,
    );

    assert_eq!(
        routes,
        vec![
            "0.0.0.0/0".to_string(),
            "10.44.0.2/32".to_string(),
            "10.60.0.0/24".to_string(),
        ]
    );
}

#[test]
fn macos_route_targets_skip_default_route_when_exit_handshake_is_on_unrelated_local_endpoint() {
    let now = unix_timestamp();
    let mut config = AppConfig::generated();
    let exit_participant = Keys::generate().public_key().to_hex();
    config.networks[0].participants = vec![exit_participant.clone()];
    config.exit_node = exit_participant.clone();
    config.ensure_defaults();

    let public_key = generate_keypair().public_key;
    let public_key_hex = crate::key_b64_to_hex(&public_key).expect("peer public key hex");
    let announcements = HashMap::from([(
        exit_participant.clone(),
        PeerAnnouncement {
            node_id: "exit-node".to_string(),
            public_key,
            endpoint: "203.0.113.20:51820".to_string(),
            local_endpoint: None,
            public_endpoint: Some("203.0.113.20:51820".to_string()),
            tunnel_ip: "10.44.0.2/32".to_string(),
            advertised_routes: vec!["0.0.0.0/0".to_string(), "10.60.0.0/24".to_string()],
            timestamp: 1,
        },
    )]);

    let planned = planned_tunnel_peers(
        &config,
        None,
        &announcements,
        &mut PeerPathBook::default(),
        Some("192.0.2.10:51820"),
        10,
    )
    .expect("planned tunnel peers");

    let runtime_peers = HashMap::from([(
        public_key_hex,
        WireGuardPeerStatus {
            endpoint: Some("192.168.50.40:51820".to_string()),
            last_handshake_sec: Some(now - 1),
            ..Default::default()
        },
    )]);

    let routes = route_targets_for_planned_tunnel_peers(
        &config,
        None,
        &announcements,
        &planned,
        &PeerPathBook::default(),
        Some(&runtime_peers),
        now,
    );

    assert_eq!(
        routes,
        vec!["10.44.0.2/32".to_string(), "10.60.0.0/24".to_string()]
    );
}

#[test]
fn macos_route_targets_keep_default_route_for_recent_successful_exit_path() {
    let now = unix_timestamp();
    let mut config = AppConfig::generated();
    let exit_participant = Keys::generate().public_key().to_hex();
    config.networks[0].participants = vec![exit_participant.clone()];
    config.exit_node = exit_participant.clone();
    config.ensure_defaults();

    let announcements = HashMap::from([(
        exit_participant.clone(),
        PeerAnnouncement {
            node_id: "exit-node".to_string(),
            public_key: generate_keypair().public_key,
            endpoint: "203.0.113.20:51820".to_string(),
            local_endpoint: None,
            public_endpoint: Some("203.0.113.20:51820".to_string()),
            tunnel_ip: "10.44.0.2/32".to_string(),
            advertised_routes: vec!["0.0.0.0/0".to_string(), "10.60.0.0/24".to_string()],
            timestamp: 1,
        },
    )]);

    let mut path_book = PeerPathBook::default();
    path_book.refresh_from_announcement(
        exit_participant.clone(),
        &announcements[&exit_participant],
        now,
    );
    path_book.note_selected(exit_participant.clone(), "203.0.113.20:51820", now);
    path_book.note_success(exit_participant.clone(), "203.0.113.20:51820", now - 1);

    let planned = planned_tunnel_peers(
        &config,
        None,
        &announcements,
        &mut path_book,
        Some("192.0.2.10:51820"),
        now,
    )
    .expect("planned tunnel peers");

    let routes = route_targets_for_planned_tunnel_peers(
        &config,
        None,
        &announcements,
        &planned,
        &path_book,
        None,
        now,
    );

    assert_eq!(
        routes,
        vec![
            "0.0.0.0/0".to_string(),
            "10.44.0.2/32".to_string(),
            "10.60.0.0/24".to_string(),
        ]
    );
}

#[test]
fn macos_route_targets_drop_default_route_for_stale_successful_exit_path() {
    let now = unix_timestamp();
    let mut config = AppConfig::generated();
    let exit_participant = Keys::generate().public_key().to_hex();
    config.networks[0].participants = vec![exit_participant.clone()];
    config.exit_node = exit_participant.clone();
    config.ensure_defaults();

    let announcements = HashMap::from([(
        exit_participant.clone(),
        PeerAnnouncement {
            node_id: "exit-node".to_string(),
            public_key: generate_keypair().public_key,
            endpoint: "203.0.113.20:51820".to_string(),
            local_endpoint: None,
            public_endpoint: Some("203.0.113.20:51820".to_string()),
            tunnel_ip: "10.44.0.2/32".to_string(),
            advertised_routes: vec!["0.0.0.0/0".to_string(), "10.60.0.0/24".to_string()],
            timestamp: 1,
        },
    )]);

    let mut path_book = PeerPathBook::default();
    path_book.refresh_from_announcement(
        exit_participant.clone(),
        &announcements[&exit_participant],
        now,
    );
    path_book.note_selected(exit_participant.clone(), "203.0.113.20:51820", now);
    path_book.note_success(
        exit_participant.clone(),
        "203.0.113.20:51820",
        now.saturating_sub(PEER_ONLINE_GRACE_SECS + 1),
    );

    let planned = planned_tunnel_peers(
        &config,
        None,
        &announcements,
        &mut path_book,
        Some("192.0.2.10:51820"),
        now,
    )
    .expect("planned tunnel peers");

    let routes = route_targets_for_planned_tunnel_peers(
        &config,
        None,
        &announcements,
        &planned,
        &path_book,
        None,
        now,
    );

    assert_eq!(
        routes,
        vec!["10.44.0.2/32".to_string(), "10.60.0.0/24".to_string()]
    );
}
