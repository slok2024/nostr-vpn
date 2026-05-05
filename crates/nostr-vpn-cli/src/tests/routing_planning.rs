use std::collections::HashMap;
use std::net::Ipv4Addr;

use crate::*;

use nostr_sdk::prelude::Keys;
use nostr_vpn_core::crypto::generate_keypair;
use nostr_vpn_core::paths::PeerPathBook;

#[test]
fn planned_tunnel_peers_assign_selected_exit_node_default_route() {
    let mut config = AppConfig::generated();
    let exit_participant = Keys::generate().public_key().to_hex();
    let routed_participant = Keys::generate().public_key().to_hex();
    config.networks[0].participants = vec![exit_participant.clone(), routed_participant.clone()];
    config.exit_node = exit_participant.clone();
    config.ensure_defaults();

    let announcements = HashMap::from([
        (
            exit_participant.clone(),
            PeerAnnouncement {
                node_id: "exit-node".to_string(),
                public_key: generate_keypair().public_key,
                endpoint: "203.0.113.20:51820".to_string(),
                local_endpoint: None,
                public_endpoint: Some("203.0.113.20:51820".to_string()),
                tunnel_ip: "10.44.0.2/32".to_string(),
                advertised_routes: vec![
                    "10.60.0.0/24".to_string(),
                    "0.0.0.0/0".to_string(),
                    "::/0".to_string(),
                ],
                timestamp: 1,
            },
        ),
        (
            routed_participant.clone(),
            PeerAnnouncement {
                node_id: "routed-node".to_string(),
                public_key: generate_keypair().public_key,
                endpoint: "203.0.113.21:51820".to_string(),
                local_endpoint: None,
                public_endpoint: Some("203.0.113.21:51820".to_string()),
                tunnel_ip: "10.44.0.3/32".to_string(),
                advertised_routes: vec!["10.70.0.0/24".to_string()],
                timestamp: 1,
            },
        ),
    ]);

    let planned = planned_tunnel_peers(
        &config,
        None,
        &announcements,
        &mut PeerPathBook::default(),
        Some("192.0.2.10:51820"),
        10,
    )
    .expect("planned tunnel peers");

    let exit_peer = planned
        .iter()
        .find(|planned| planned.participant == exit_participant)
        .expect("exit peer");
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    assert_eq!(
        exit_peer.peer.allowed_ips,
        vec![
            "10.44.0.2/32".to_string(),
            "0.0.0.0/0".to_string(),
            "10.60.0.0/24".to_string(),
        ]
    );
    #[cfg(target_os = "windows")]
    assert_eq!(
        exit_peer.peer.allowed_ips,
        vec![
            "10.44.0.2/32".to_string(),
            "0.0.0.0/0".to_string(),
            "10.60.0.0/24".to_string(),
        ]
    );
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    assert_eq!(
        exit_peer.peer.allowed_ips,
        vec!["10.44.0.2/32".to_string(), "10.60.0.0/24".to_string()]
    );

    let routed_peer = planned
        .iter()
        .find(|planned| planned.participant == routed_participant)
        .expect("routed peer");
    assert_eq!(
        routed_peer.peer.allowed_ips,
        vec!["10.44.0.3/32".to_string(), "10.70.0.0/24".to_string()]
    );
}

#[test]
fn planned_tunnel_peers_ignore_default_route_without_selected_exit_node() {
    let mut config = AppConfig::generated();
    let exit_participant = Keys::generate().public_key().to_hex();
    config.networks[0].participants = vec![exit_participant.clone()];
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

    assert_eq!(
        planned[0].peer.allowed_ips,
        vec!["10.44.0.2/32".to_string(), "10.60.0.0/24".to_string()]
    );
}

#[test]
fn reuses_running_listen_port_without_rebind() {
    assert!(can_reuse_active_listen_port(true, true, Some(51820), 51820));
    assert!(!can_reuse_active_listen_port(
        true,
        true,
        Some(51820),
        51821
    ));
    assert!(!can_reuse_active_listen_port(
        false,
        true,
        Some(51820),
        51820
    ));
    assert!(!can_reuse_active_listen_port(
        true,
        false,
        Some(51820),
        51820
    ));
    assert!(!can_reuse_active_listen_port(true, true, None, 51820));
}

#[test]
fn tunnel_heartbeat_targets_only_include_peers_without_handshake() {
    let now = unix_timestamp();
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
    let announcements = HashMap::from([(participant.clone(), announcement)]);

    let pending = pending_tunnel_heartbeat_ips(&config, None, &announcements, None);
    assert_eq!(pending, vec![Ipv4Addr::new(10, 44, 0, 2)]);

    let runtime_peers = HashMap::from([(
        key_b64_to_hex(&peer_keys.public_key).expect("peer pubkey hex"),
        WireGuardPeerStatus {
            endpoint: Some("203.0.113.20:51820".to_string()),
            last_handshake_sec: Some(now - 1),
            last_handshake_nsec: Some(0),
            ..WireGuardPeerStatus::default()
        },
    )]);
    let pending = pending_tunnel_heartbeat_ips(&config, None, &announcements, Some(&runtime_peers));
    assert!(pending.is_empty(), "handshaken peer should not be probed");
}

#[test]
fn tunnel_heartbeat_targets_include_peers_with_stale_handshakes() {
    let now = unix_timestamp();
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
    let announcements = HashMap::from([(participant.clone(), announcement)]);
    let runtime_peers = HashMap::from([(
        key_b64_to_hex(&peer_keys.public_key).expect("peer pubkey hex"),
        WireGuardPeerStatus {
            endpoint: Some("203.0.113.20:51820".to_string()),
            last_handshake_sec: Some(now - PEER_ONLINE_GRACE_SECS - 1),
            last_handshake_nsec: Some(0),
            ..WireGuardPeerStatus::default()
        },
    )]);

    let pending = pending_tunnel_heartbeat_ips(&config, None, &announcements, Some(&runtime_peers));
    assert_eq!(
        pending,
        vec![Ipv4Addr::new(10, 44, 0, 2)],
        "stale peers should still get tunnel heartbeats"
    );
}

#[test]
fn relay_connection_action_reconnects_only_when_disconnected() {
    assert_eq!(
        relay_connection_action(true),
        crate::RelayConnectionAction::KeepConnected
    );
    assert_eq!(
        relay_connection_action(false),
        crate::RelayConnectionAction::ReconnectWhenDue
    );
}

#[test]
fn runtime_magic_dns_records_prefer_live_announcement_tunnel_ip() {
    let mut config = AppConfig::generated();
    config.magic_dns_suffix = "nvpn".to_string();
    config.networks[0].participants =
        vec!["3d332ed94c79863e73ff8af62882de2853c77d6a5c1fe61d7598a90db1fab645".to_string()];
    config.ensure_defaults();
    config
        .set_peer_alias(
            "3d332ed94c79863e73ff8af62882de2853c77d6a5c1fe61d7598a90db1fab645",
            "pig",
        )
        .expect("set alias");

    let mut announcements = HashMap::new();
    announcements.insert(
        "3d332ed94c79863e73ff8af62882de2853c77d6a5c1fe61d7598a90db1fab645".to_string(),
        PeerAnnouncement {
            node_id: "peer-node".to_string(),
            public_key: "pubkey".to_string(),
            endpoint: "192.168.1.55:51820".to_string(),
            local_endpoint: None,
            public_endpoint: None,
            tunnel_ip: "10.44.0.113/32".to_string(),
            advertised_routes: Vec::new(),
            timestamp: 1,
        },
    );

    let records = build_runtime_magic_dns_records(&config, &announcements);
    assert_eq!(
        records.get("pig.nvpn").map(|ip| ip.to_string()),
        Some("10.44.0.113".to_string())
    );
    assert_eq!(
        records.get("pig").map(|ip| ip.to_string()),
        Some("10.44.0.113".to_string())
    );
}

#[test]
fn runtime_magic_dns_records_follow_latest_announcement_ip() {
    let mut config = AppConfig::generated();
    config.magic_dns_suffix = "nvpn".to_string();
    config.networks[0].participants =
        vec!["3d332ed94c79863e73ff8af62882de2853c77d6a5c1fe61d7598a90db1fab645".to_string()];
    config.ensure_defaults();
    config
        .set_peer_alias(
            "3d332ed94c79863e73ff8af62882de2853c77d6a5c1fe61d7598a90db1fab645",
            "pig",
        )
        .expect("set alias");

    let mut announcements = HashMap::new();
    announcements.insert(
        "3d332ed94c79863e73ff8af62882de2853c77d6a5c1fe61d7598a90db1fab645".to_string(),
        PeerAnnouncement {
            node_id: "peer-node".to_string(),
            public_key: "pubkey".to_string(),
            endpoint: "192.168.1.55:51820".to_string(),
            local_endpoint: None,
            public_endpoint: None,
            tunnel_ip: "10.44.0.113/32".to_string(),
            advertised_routes: Vec::new(),
            timestamp: 1,
        },
    );
    let first = build_runtime_magic_dns_records(&config, &announcements);
    assert_eq!(
        first.get("pig.nvpn").map(|ip| ip.to_string()),
        Some("10.44.0.113".to_string())
    );

    announcements.insert(
        "3d332ed94c79863e73ff8af62882de2853c77d6a5c1fe61d7598a90db1fab645".to_string(),
        PeerAnnouncement {
            node_id: "peer-node".to_string(),
            public_key: "pubkey".to_string(),
            endpoint: "192.168.1.55:51820".to_string(),
            local_endpoint: None,
            public_endpoint: None,
            tunnel_ip: "10.44.0.114/32".to_string(),
            advertised_routes: Vec::new(),
            timestamp: 2,
        },
    );
    let second = build_runtime_magic_dns_records(&config, &announcements);
    assert_eq!(
        second.get("pig.nvpn").map(|ip| ip.to_string()),
        Some("10.44.0.114".to_string())
    );
}
