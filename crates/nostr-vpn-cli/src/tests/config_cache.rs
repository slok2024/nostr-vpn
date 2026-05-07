use std::fs;

use crate::*;
use nostr_sdk::prelude::{Keys, ToBech32};
use nostr_vpn_core::config::{NetworkConfig, PendingOutboundJoinRequest};
use nostr_vpn_core::crypto::generate_keypair;
use nostr_vpn_core::paths::PeerPathBook;
use nostr_vpn_core::presence::PeerPresenceBook;

#[test]
fn participants_override_targets_the_active_network() {
    let alice = Keys::generate().public_key().to_hex();
    let bob = Keys::generate().public_key().to_hex();
    let carol = Keys::generate().public_key().to_hex();

    let mut config = AppConfig::generated();
    config.networks = vec![
        NetworkConfig {
            id: "home".to_string(),
            name: "Home".to_string(),
            enabled: false,
            network_id: "mesh-home".to_string(),
            participants: vec![alice.clone()],
            admins: Vec::new(),
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        },
        NetworkConfig {
            id: "work".to_string(),
            name: "Work".to_string(),
            enabled: true,
            network_id: "mesh-work".to_string(),
            participants: vec![bob],
            admins: Vec::new(),
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        },
    ];
    config.ensure_defaults();

    apply_participants_override(&mut config, vec![carol.clone()]).expect("apply override");

    assert_eq!(config.participant_pubkeys_hex(), vec![carol.clone()]);
    assert_eq!(
        config
            .network_by_id("home")
            .expect("home network")
            .participants,
        vec![alice]
    );
    assert_eq!(
        config
            .network_by_id("work")
            .expect("work network")
            .participants,
        vec![carol]
    );
}

#[test]
fn participants_override_preserves_selected_exit_node_when_it_remains_a_member() {
    let exit_peer = Keys::generate().public_key().to_hex();

    let mut config = AppConfig::generated();
    config.exit_node = exit_peer.clone();

    apply_participants_override(&mut config, vec![exit_peer.clone()]).expect("apply override");

    assert_eq!(config.participant_pubkeys_hex(), vec![exit_peer.clone()]);
    assert_eq!(config.exit_node, exit_peer);
}

#[test]
fn pending_join_request_recipients_use_selected_admin_and_skip_self() {
    let mut config = AppConfig::generated();
    let own_pubkey = Keys::parse(&config.nostr.secret_key)
        .expect("own keys")
        .public_key()
        .to_hex();
    let admin = Keys::generate().public_key().to_hex();
    let backup_admin = Keys::generate().public_key().to_hex();

    config.networks[0].enabled = true;
    config.networks[0].participants.clear();
    config.networks[0].admins = vec![own_pubkey, admin.clone(), backup_admin];
    config.networks[0].outbound_join_request = Some(PendingOutboundJoinRequest {
        recipient: admin.clone(),
        requested_at: 123,
    });

    assert_eq!(pending_fips_join_request_recipients(&config), vec![admin]);
}

#[test]
fn pending_join_request_recipients_fall_back_to_admins_without_self() {
    let mut config = AppConfig::generated();
    let own_pubkey = Keys::parse(&config.nostr.secret_key)
        .expect("own keys")
        .public_key()
        .to_hex();
    let admin = Keys::generate().public_key().to_hex();
    let stale_recipient = Keys::generate().public_key().to_hex();

    config.networks[0].enabled = true;
    config.networks[0].participants.clear();
    config.networks[0].admins = vec![own_pubkey, admin.clone()];
    config.networks[0].outbound_join_request = Some(PendingOutboundJoinRequest {
        recipient: stale_recipient,
        requested_at: 123,
    });

    assert_eq!(pending_fips_join_request_recipients(&config), vec![admin]);
}

#[test]
fn participants_override_marks_shared_roster_updated_for_admin_owned_network() {
    let member = Keys::generate().public_key().to_hex();

    let mut config = AppConfig::generated();
    let own_pubkey = config.own_nostr_pubkey_hex().expect("own nostr pubkey");
    config.networks[0].admins = vec![own_pubkey.clone()];
    config.networks[0].shared_roster_updated_at = 0;
    config.networks[0].shared_roster_signed_by.clear();

    apply_participants_override(&mut config, vec![member.clone()]).expect("apply override");

    let active_network = config.active_network();
    assert_eq!(active_network.participants, vec![member]);
    assert!(active_network.shared_roster_updated_at > 0);
    assert_eq!(active_network.shared_roster_signed_by, own_pubkey);
}

#[test]
fn shared_roster_publish_allowed_only_for_current_signer() {
    let other_admin = Keys::generate().public_key().to_hex();
    let outsider = Keys::generate().public_key().to_hex();

    let mut config = AppConfig::generated();
    let own_pubkey = config.own_nostr_pubkey_hex().expect("own nostr pubkey");
    let network_id = config.active_network().id.clone();
    config.networks[0].admins = vec![own_pubkey.clone(), other_admin.clone()];

    assert!(shared_roster_publish_allowed(
        &config,
        &network_id,
        &own_pubkey,
        ""
    ));
    assert!(shared_roster_publish_allowed(
        &config,
        &network_id,
        &own_pubkey,
        &own_pubkey
    ));
    assert!(!shared_roster_publish_allowed(
        &config,
        &network_id,
        &own_pubkey,
        &other_admin
    ));
    assert!(!shared_roster_publish_allowed(
        &config,
        &network_id,
        &outsider,
        &outsider
    ));
}

#[test]
fn active_network_invite_code_roundtrips_current_roster() {
    let inviter_hex = Keys::generate().public_key().to_hex();
    let participant_hex = Keys::generate().public_key().to_hex();
    let admin_hex = Keys::generate().public_key().to_hex();

    let mut config = AppConfig::generated();
    config.networks[0].name = "Work".to_string();
    config.networks[0].network_id = "8d4f34f5425bc50e".to_string();
    config.networks[0].participants = vec![participant_hex];
    config.networks[0].admins = vec![inviter_hex.clone(), admin_hex];
    config.networks[0].invite_inviter = inviter_hex;
    config.nostr.relays = vec!["wss://temp.iris.to".to_string()];

    let invite = active_network_invite_code(&config).expect("invite should encode");
    let parsed = parse_network_invite(&invite).expect("invite should decode");

    assert!(invite.starts_with(NETWORK_INVITE_PREFIX));
    assert!(parsed.network_name.is_empty());
    assert_eq!(parsed.network_id, "8d4f34f5425bc50e");
    assert_eq!(parsed.admins.len(), 2);
    assert!(parsed.participants.is_empty());
    assert!(parsed.relays.is_empty());
}

#[test]
fn importing_current_invite_queues_join_request_to_admin() {
    let admin_npub = Keys::generate()
        .public_key()
        .to_bech32()
        .expect("admin npub");
    let admin_hex = normalize_nostr_pubkey(&admin_npub).expect("normalize admin");
    let invite = serde_json::json!({
        "v": 3,
        "network_id": "8d4f34f5425bc50e",
        "admins": [admin_npub],
        "relays": ["wss://temp.iris.to"]
    })
    .to_string();

    let mut config = AppConfig::generated();
    let parsed = parse_network_invite(&invite).expect("invite should parse");
    apply_network_invite_to_active_network(&mut config, &parsed).expect("invite should apply");
    let queued = queue_active_network_join_request(&mut config).expect("join request should queue");

    let network = config.active_network();
    assert!(queued);
    assert_eq!(
        network
            .outbound_join_request
            .as_ref()
            .expect("pending join request")
            .recipient,
        admin_hex
    );
    assert!(network.participants.is_empty());
}

#[test]
fn config_overrides_set_the_active_network_mesh_id() {
    let nonce = unix_timestamp();
    let dir = std::env::temp_dir().join(format!("nvpn-load-config-override-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");

    let mut config = AppConfig::generated();
    config.networks = vec![
        NetworkConfig {
            id: "home".to_string(),
            name: "Home".to_string(),
            enabled: false,
            network_id: "mesh-home".to_string(),
            participants: vec!["11".repeat(32)],
            admins: Vec::new(),
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        },
        NetworkConfig {
            id: "work".to_string(),
            name: "Work".to_string(),
            enabled: true,
            network_id: "mesh-work".to_string(),
            participants: vec!["22".repeat(32)],
            admins: Vec::new(),
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        },
    ];
    config.ensure_defaults();
    config.save(&config_path).expect("save temp config");

    let (loaded, network_id) =
        load_config_with_overrides(&config_path, Some("mesh-override".to_string()), Vec::new())
            .expect("load config with override");

    assert_eq!(network_id, "mesh-override");
    assert_eq!(loaded.effective_network_id(), "mesh-override");
    assert_eq!(
        loaded
            .network_by_id("home")
            .expect("home network")
            .network_id,
        "mesh-home"
    );
    assert_eq!(
        loaded
            .network_by_id("work")
            .expect("work network")
            .network_id,
        "mesh-override"
    );

    let _ = fs::remove_file(&config_path);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn persisted_peer_cache_roundtrips_known_peers_and_path_hints() {
    let nonce = unix_timestamp();
    let dir = std::env::temp_dir().join(format!("nvpn-peer-cache-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp cache dir");
    let config_path = dir.join("config.toml");
    let cache_path = daemon_peer_cache_file_path(&config_path);

    let mut config = AppConfig::generated();
    let participant = "11".repeat(32);
    config.networks[0].participants = vec![participant.clone()];
    let network_id = config.effective_network_id();
    let own_pubkey = config.own_nostr_pubkey_hex().ok();

    let peer_keys = generate_keypair();
    let announcement = PeerAnnouncement {
        node_id: "peer-a".to_string(),
        public_key: peer_keys.public_key.clone(),
        endpoint: "203.0.113.20:51820".to_string(),
        local_endpoint: Some("192.168.1.20:51820".to_string()),
        public_endpoint: Some("203.0.113.20:51820".to_string()),
        tunnel_ip: "10.44.0.2/32".to_string(),
        advertised_routes: Vec::new(),
        timestamp: 100,
    };

    let mut path_book = PeerPathBook::default();
    assert!(path_book.note_success(participant.clone(), "203.0.113.20:51820", 120,));

    let cache = DaemonPeerCacheState {
        version: 1,
        network_id: network_id.clone(),
        own_pubkey: own_pubkey.clone(),
        updated_at: 130,
        peers: vec![DaemonPeerCacheEntry {
            participant_pubkey: participant.clone(),
            announcement: announcement.clone(),
            last_signal_seen_at: Some(110),
            cached_at: 125,
        }],
        path_book,
    };
    write_daemon_peer_cache(&cache_path, &cache).expect("write peer cache");
    let loaded = read_daemon_peer_cache(&cache_path)
        .expect("read peer cache")
        .expect("cache should exist");
    assert_eq!(loaded.network_id, network_id);
    assert_eq!(loaded.peers.len(), 1);

    let mut restored_presence = PeerPresenceBook::default();
    let mut restored_paths = PeerPathBook::default();
    assert!(
        restore_daemon_peer_cache(
            DaemonPeerCacheRestore {
                path: &cache_path,
                app: &config,
                network_id: &network_id,
                own_pubkey: own_pubkey.as_deref(),
                now: 130,
                announce_interval_secs: 20,
            },
            &mut restored_presence,
            &mut restored_paths,
        )
        .expect("restore peer cache")
    );
    assert!(restored_presence.active().is_empty());
    assert!(restored_presence.known().contains_key(&participant));

    let planned = planned_tunnel_peers(
        &config,
        own_pubkey.as_deref(),
        restored_presence.known(),
        &mut restored_paths,
        Some("10.0.0.33:51820"),
        130,
    )
    .expect("planned peers from restored cache");
    assert_eq!(planned.len(), 1);
    assert_eq!(planned[0].endpoint, "203.0.113.20:51820");

    let _ = fs::remove_file(&cache_path);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn stale_persisted_peer_cache_is_ignored() {
    let nonce = unix_timestamp();
    let dir = std::env::temp_dir().join(format!("nvpn-stale-peer-cache-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp cache dir");
    let config_path = dir.join("config.toml");
    let cache_path = daemon_peer_cache_file_path(&config_path);

    let mut config = AppConfig::generated();
    let participant = "11".repeat(32);
    config.networks[0].participants = vec![participant.clone()];
    let network_id = config.effective_network_id();
    let own_pubkey = config.own_nostr_pubkey_hex().ok();

    let cache = DaemonPeerCacheState {
        version: 1,
        network_id: network_id.clone(),
        own_pubkey: own_pubkey.clone(),
        updated_at: 10,
        peers: vec![DaemonPeerCacheEntry {
            participant_pubkey: participant,
            announcement: PeerAnnouncement {
                node_id: "peer-a".to_string(),
                public_key: generate_keypair().public_key,
                endpoint: "203.0.113.20:51820".to_string(),
                local_endpoint: None,
                public_endpoint: Some("203.0.113.20:51820".to_string()),
                tunnel_ip: "10.44.0.2/32".to_string(),
                advertised_routes: Vec::new(),
                timestamp: 1,
            },
            last_signal_seen_at: Some(10),
            cached_at: 10,
        }],
        path_book: PeerPathBook::default(),
    };
    write_daemon_peer_cache(&cache_path, &cache).expect("write stale peer cache");

    let mut restored_presence = PeerPresenceBook::default();
    let mut restored_paths = PeerPathBook::default();
    assert!(
        !restore_daemon_peer_cache(
            DaemonPeerCacheRestore {
                path: &cache_path,
                app: &config,
                network_id: &network_id,
                own_pubkey: own_pubkey.as_deref(),
                now: 10 + persisted_peer_cache_timeout_secs(20) + 1,
                announce_interval_secs: 20,
            },
            &mut restored_presence,
            &mut restored_paths,
        )
        .expect("restore stale cache")
    );
    assert!(restored_presence.known().is_empty());
    assert!(!restored_paths.prune_stale(
        10 + persisted_path_cache_timeout_secs(20) + 1,
        persisted_path_cache_timeout_secs(20),
    ));

    let _ = fs::remove_file(&cache_path);
    let _ = fs::remove_dir_all(&dir);
}
