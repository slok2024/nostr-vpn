use super::*;
use nostr_vpn_core::config::PendingOutboundJoinRequest;

#[test]
fn default_relays_match_hashtree_defaults() {
    assert!(DEFAULT_RELAYS.is_empty());
}

#[test]
fn generated_config_auto_populates_keys() {
    let config = AppConfig::generated();

    assert!(!config.nostr.secret_key.is_empty());
    assert!(!config.nostr.public_key.is_empty());
    assert!(!config.node_name.trim().is_empty());
    assert_ne!(config.node_name, "nostr-vpn-node");
    assert!(config.nostr.relays.is_empty());
    assert!(config.autoconnect);
    assert!(config.lan_discovery_enabled);
    assert!(config.launch_on_startup);
    assert!(config.close_to_tray_on_close);
    assert!(config.nat.enabled);
    assert!(!config.nat.stun_servers.is_empty());
    assert!(config.exit_node.is_empty());
    assert!(config.exit_node_leak_protection);
    assert!(!config.node.advertise_exit_node);
    assert!(config.node.advertised_routes.is_empty());
    assert!(config.effective_advertised_routes().is_empty());
}

#[test]
fn exit_node_leak_protection_defaults_on_when_missing() {
    let config: AppConfig = toml::from_str("").expect("parse empty config");

    assert!(config.exit_node_leak_protection);
}

#[test]
fn exit_node_leak_protection_off_is_preserved() {
    let config = AppConfig {
        exit_node_leak_protection: false,
        ..AppConfig::default()
    };

    let encoded = toml::to_string(&config).expect("serialize config");
    assert!(encoded.contains("exit_node_leak_protection = false"));

    let decoded: AppConfig = toml::from_str(&encoded).expect("parse config");
    assert!(!decoded.exit_node_leak_protection);
}

#[test]
fn generated_config_can_start_without_networks() {
    let mut config = AppConfig::generated_without_networks();

    assert!(config.networks.is_empty());
    config.ensure_defaults();

    assert!(config.networks.is_empty());
    assert!(config.effective_network_id().is_empty());
    assert!(config.enabled_network_meshes().is_empty());
    assert!(config.participant_pubkeys_hex().is_empty());
}

#[test]
fn default_routes_promote_to_exit_node_toggle() {
    let mut config = AppConfig::generated();
    config.node.advertised_routes = vec![
        "10.0.0.0/24".to_string(),
        "0.0.0.0/0".to_string(),
        "::/0".to_string(),
        "10.0.0.0/24".to_string(),
    ];

    config.ensure_defaults();

    assert!(config.node.advertise_exit_node);
    assert_eq!(
        config.node.advertised_routes,
        vec!["10.0.0.0/24".to_string()]
    );
    assert_eq!(
        config.effective_advertised_routes(),
        vec![
            "10.0.0.0/24".to_string(),
            "0.0.0.0/0".to_string(),
            "::/0".to_string(),
        ]
    );
}

#[test]
fn exit_node_normalizes_from_npub() {
    let peer = Keys::generate();
    let peer_hex = peer.public_key().to_hex();
    let peer_npub = peer.public_key().to_bech32().expect("peer npub");

    let mut config = AppConfig::generated();
    set_default_network_participants(&mut config, vec![peer_hex.clone()]);
    config.exit_node = peer_npub;

    config.ensure_defaults();

    assert_eq!(config.exit_node, peer_hex);
}

#[test]
fn stale_exit_node_is_cleared_when_not_in_active_network_roster() {
    let peer = Keys::generate();
    let peer_npub = peer.public_key().to_bech32().expect("peer npub");

    let mut config = AppConfig::generated();
    config.exit_node = peer_npub;

    config.ensure_defaults();

    assert!(config.exit_node.is_empty());
}

#[test]
fn participants_are_normalized_to_hex_pubkeys() {
    let keys = Keys::generate();
    let npub = keys.public_key().to_bech32().expect("npub");
    let hex = keys.public_key().to_hex();

    let mut config = AppConfig::generated();
    set_default_network_participants(&mut config, vec![npub, hex.clone()]);
    config.ensure_defaults();

    assert_eq!(config.participant_pubkeys_hex(), vec![hex]);
}

#[test]
fn normalize_accepts_npub() {
    let keys = Keys::generate();
    let npub = keys.public_key().to_bech32().expect("npub");

    let normalized = normalize_nostr_pubkey(&npub).expect("normalize npub");

    assert_eq!(normalized, keys.public_key().to_hex());
}

#[test]
fn derive_mesh_tunnel_ip_is_deterministic_for_participant_member() {
    let tunnel_ip = derive_mesh_tunnel_ip("mesh-a", "bb").expect("tunnel ip");
    assert_eq!(
        tunnel_ip,
        derive_mesh_tunnel_ip("mesh-a", "bb").expect("tunnel ip")
    );
    assert_ne!(
        tunnel_ip,
        derive_mesh_tunnel_ip("mesh-b", "bb").expect("different mesh id changes ip")
    );
}

#[test]
fn maybe_autoconfigure_node_assigns_tunnel_ip_from_participants() {
    let keys = Keys::generate();
    let own_hex = keys.public_key().to_hex();

    let mut config = AppConfig::generated();
    config.nostr.secret_key = keys.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex.clone();
    set_default_network_participants(&mut config, vec!["0".repeat(64), own_hex.clone()]);
    config.node.tunnel_ip = "10.44.0.1/32".to_string();
    config.node.endpoint = "198.51.100.10:51820".to_string();

    maybe_autoconfigure_node(&mut config);

    assert_eq!(
        config.node.tunnel_ip,
        derive_mesh_tunnel_ip(&config.effective_network_id(), &own_hex).expect("derived ip")
    );
}

#[test]
fn explicit_active_network_id_is_preserved() {
    let keys = Keys::generate();
    let peer = Keys::generate();
    let own_hex = keys.public_key().to_hex();

    let mut config = AppConfig::generated();
    config.networks[0].network_id = "mesh-fixed".to_string();
    config.nostr.secret_key = keys.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex;
    set_default_network_participants(&mut config, vec![peer.public_key().to_hex()]);

    config.ensure_defaults();

    assert_eq!(config.effective_network_id(), "mesh-fixed");
}

#[test]
fn join_requests_enabled_is_true_when_any_network_listens() {
    let mut config = AppConfig::generated();
    config.networks[0].listen_for_join_requests = false;
    let network_id = config.add_network("Other");
    config
        .set_network_join_requests_enabled(&network_id, true)
        .expect("enable join requests");

    assert!(config.join_requests_enabled());
}

#[test]
fn generated_network_defaults_local_identity_to_admin() {
    let config = AppConfig::generated();
    let own_pubkey = config.own_nostr_pubkey_hex().expect("own pubkey");

    assert_eq!(config.active_network_admin_pubkeys_hex(), vec![own_pubkey]);
}

#[test]
fn apply_admin_signed_shared_roster_replaces_members_from_known_admin() {
    let own = Keys::generate();
    let current_admin = Keys::generate();
    let new_admin = Keys::generate();
    let member = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let current_admin_hex = current_admin.public_key().to_hex();
    let new_admin_hex = new_admin.public_key().to_hex();
    let member_hex = member.public_key().to_hex();

    let mut config = AppConfig::generated();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex.clone();
    config.networks[0].network_id = "mesh-home".to_string();
    config.networks[0].admins = vec![current_admin_hex.clone()];
    config.networks[0].participants = vec![current_admin_hex.clone()];
    config.ensure_defaults();

    let changed = config
        .apply_admin_signed_shared_roster(
            "mesh-home",
            "Home",
            vec![
                current_admin_hex.clone(),
                member_hex.clone(),
                own_hex.clone(),
            ],
            vec![current_admin_hex.clone(), new_admin_hex.clone()],
            std::collections::HashMap::new(),
            1_726_000_000,
            &current_admin_hex,
        )
        .expect("apply shared roster");

    assert!(changed);
    assert_eq!(config.networks[0].name, "Home");
    let mut expected_participants = vec![current_admin_hex.clone(), member_hex.clone()];
    expected_participants.sort();
    assert_eq!(config.networks[0].participants, expected_participants);
    let mut expected_admins = vec![new_admin_hex, current_admin.public_key().to_hex()];
    expected_admins.sort();
    assert_eq!(config.networks[0].admins, expected_admins);
    assert_eq!(config.networks[0].shared_roster_updated_at, 1_726_000_000);
}

#[test]
fn apply_admin_signed_shared_roster_clears_join_request_when_own_key_is_added() {
    let own = Keys::generate();
    let current_admin = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let current_admin_hex = current_admin.public_key().to_hex();

    let mut config = AppConfig::generated();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex.clone();
    config.networks[0].network_id = "mesh-home".to_string();
    config.networks[0].admins = vec![current_admin_hex.clone()];
    config.networks[0].participants = Vec::new();
    config.networks[0].outbound_join_request = Some(PendingOutboundJoinRequest {
        recipient: current_admin_hex.clone(),
        requested_at: 1_725_999_999,
    });
    config.ensure_defaults();

    let changed = config
        .apply_admin_signed_shared_roster(
            "mesh-home",
            "Home",
            vec![current_admin_hex.clone(), own_hex],
            vec![current_admin_hex.clone()],
            std::collections::HashMap::new(),
            1_726_000_000,
            &current_admin_hex,
        )
        .expect("apply accepted roster");

    assert!(changed);
    assert!(config.networks[0].outbound_join_request.is_none());
}

#[test]
fn apply_admin_signed_shared_roster_ignores_unknown_signer() {
    let known_admin = Keys::generate();
    let unknown_admin = Keys::generate();
    let mut config = AppConfig::generated();
    config.networks[0].network_id = "mesh-home".to_string();
    config.networks[0].admins = vec![known_admin.public_key().to_hex()];
    config.ensure_defaults();

    let changed = config
        .apply_admin_signed_shared_roster(
            "mesh-home",
            "Home",
            vec![known_admin.public_key().to_hex()],
            vec![known_admin.public_key().to_hex()],
            std::collections::HashMap::new(),
            1_726_000_000,
            &unknown_admin.public_key().to_hex(),
        )
        .expect("ignore unknown signer");

    assert!(!changed);
}

#[test]
fn apply_admin_signed_shared_roster_clears_data_peers_when_own_key_is_removed() {
    let own = Keys::generate();
    let current_admin = Keys::generate();
    let other_member = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let current_admin_hex = current_admin.public_key().to_hex();
    let other_member_hex = other_member.public_key().to_hex();

    let mut config = AppConfig::generated();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex.clone();
    config.networks[0].network_id = "mesh-home".to_string();
    config.networks[0].admins = vec![current_admin_hex.clone()];
    config.networks[0].participants = vec![current_admin_hex.clone(), other_member_hex.clone()];
    config.ensure_defaults();

    let changed = config
        .apply_admin_signed_shared_roster(
            "mesh-home",
            "Home",
            vec![current_admin_hex.clone(), other_member_hex],
            vec![current_admin_hex.clone()],
            std::collections::HashMap::new(),
            1_726_000_000,
            &current_admin_hex,
        )
        .expect("apply removal roster");

    assert!(changed);
    assert!(config.networks[0].participants.is_empty());
    assert_eq!(config.networks[0].admins, vec![current_admin_hex]);
}

#[test]
fn shared_network_roster_includes_network_aliases() {
    let own = Keys::generate();
    let peer = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let peer_hex = peer.public_key().to_hex();
    let mut config = AppConfig::generated();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex.clone();
    config.node_name = "helios-admin".to_string();
    config.networks[0].network_id = "mesh-home".to_string();
    config.networks[0].admins = vec![own_hex.clone()];
    config.networks[0].participants = vec![peer_hex.clone()];
    config.ensure_defaults();
    config
        .set_peer_alias(&own_hex, "helios-admin")
        .expect("own alias");
    config
        .set_peer_alias(&peer_hex, "garden-node")
        .expect("peer alias");

    let roster = config
        .shared_network_roster(&config.networks[0].id)
        .expect("shared roster");

    assert_eq!(
        roster.aliases.get(&own_hex).map(String::as_str),
        Some("helios-admin")
    );
    assert_eq!(
        roster.aliases.get(&peer_hex).map(String::as_str),
        Some("garden-node")
    );
}

#[test]
fn apply_admin_signed_shared_roster_applies_aliases_for_members() {
    let own = Keys::generate();
    let current_admin = Keys::generate();
    let member = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let own_npub = own.public_key().to_bech32().expect("own npub");
    let current_admin_hex = current_admin.public_key().to_hex();
    let member_hex = member.public_key().to_hex();

    let mut config = AppConfig::generated();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex.clone();
    config.networks[0].network_id = "mesh-home".to_string();
    config.networks[0].admins = vec![current_admin_hex.clone()];
    config.networks[0].participants = vec![current_admin_hex.clone()];
    config
        .peer_aliases
        .insert(own_npub, "old-local".to_string());
    config.ensure_defaults();
    assert_eq!(config.self_magic_dns_label().as_deref(), Some("old-local"));

    let changed = config
        .apply_admin_signed_shared_roster(
            "mesh-home",
            "Home",
            vec![
                current_admin_hex.clone(),
                member_hex.clone(),
                own_hex.clone(),
            ],
            vec![current_admin_hex.clone()],
            std::collections::HashMap::from([
                (own_hex.clone(), "iphone".to_string()),
                (current_admin_hex.clone(), "home-server".to_string()),
                (member_hex.clone(), "alice-phone".to_string()),
            ]),
            1_726_000_000,
            &current_admin_hex,
        )
        .expect("apply shared roster");

    assert!(changed);
    assert_eq!(config.self_magic_dns_label().as_deref(), Some("iphone"));
    assert_eq!(
        config.peer_alias(&current_admin_hex).as_deref(),
        Some("home-server")
    );
    assert_eq!(
        config.peer_alias(&member_hex).as_deref(),
        Some("alice-phone")
    );
}

#[test]
fn apply_admin_signed_shared_roster_clears_removed_exit_node() {
    let own = Keys::generate();
    let current_admin = Keys::generate();
    let removed_exit = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let current_admin_hex = current_admin.public_key().to_hex();
    let removed_exit_hex = removed_exit.public_key().to_hex();

    let mut config = AppConfig::generated();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex.clone();
    config.networks[0].network_id = "mesh-home".to_string();
    config.networks[0].admins = vec![current_admin_hex.clone()];
    config.networks[0].participants = vec![current_admin_hex.clone(), removed_exit_hex.clone()];
    config.exit_node = removed_exit_hex.clone();
    config.ensure_defaults();

    let changed = config
        .apply_admin_signed_shared_roster(
            "mesh-home",
            "Home",
            vec![current_admin_hex.clone(), own_hex],
            vec![current_admin_hex.clone()],
            std::collections::HashMap::new(),
            1_726_000_000,
            &current_admin_hex,
        )
        .expect("apply shared roster");

    assert!(changed);
    assert!(config.exit_node.is_empty());
}

#[test]
fn set_peer_alias_marks_member_network_roster_changed() {
    let peer_hex = Keys::generate().public_key().to_hex();
    let mut config = AppConfig::generated();
    config.networks[0].participants = vec![peer_hex.clone()];
    config.ensure_defaults();
    assert_eq!(config.networks[0].shared_roster_updated_at, 0);

    config
        .set_peer_alias(&peer_hex, "home-server")
        .expect("set peer alias");

    assert!(config.networks[0].shared_roster_updated_at > 0);
}

#[test]
fn set_peer_alias_bumps_shared_roster_timestamp_past_previous_value() {
    let own = Keys::generate();
    let peer_hex = Keys::generate().public_key().to_hex();
    let mut config = AppConfig::generated();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own.public_key().to_hex();
    config.networks[0].participants = vec![peer_hex.clone()];
    config.networks[0].admins = vec![config.nostr.public_key.clone()];
    config.ensure_defaults();
    config.networks[0].shared_roster_updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_secs();

    let previous = config.networks[0].shared_roster_updated_at;
    config
        .set_peer_alias(&peer_hex, "home-server")
        .expect("set peer alias");

    assert!(config.networks[0].shared_roster_updated_at > previous);
}

#[test]
fn record_inbound_join_request_ignores_mismatched_mesh_id() {
    let requester = Keys::generate().public_key().to_hex();
    let mut config = AppConfig::generated();
    config.networks[0].network_id = "mesh-home".to_string();

    let recorded = config
        .record_inbound_join_request("mesh-other", &requester, "alice-phone", 1_726_000_000)
        .expect("record join request");

    assert!(recorded.is_none());
    assert!(config.networks[0].inbound_join_requests.is_empty());
}

#[test]
fn record_inbound_join_request_updates_matching_listening_network() {
    let requester = Keys::generate().public_key().to_hex();
    let mut config = AppConfig::generated();
    config.networks[0].name = "Home".to_string();
    config.networks[0].network_id = "mesh-home".to_string();

    let recorded = config
        .record_inbound_join_request("mesh-home", &requester, "alice-phone", 1_726_000_000)
        .expect("record join request");

    assert_eq!(recorded.as_deref(), Some("Home"));
    assert_eq!(config.networks[0].inbound_join_requests.len(), 1);
    assert_eq!(
        config.networks[0].inbound_join_requests[0].requester,
        requester
    );
    assert_eq!(
        config.networks[0].inbound_join_requests[0].requester_node_name,
        "alice-phone"
    );
}

#[test]
fn reject_inbound_join_request_removes_matching_request() {
    let requester = Keys::generate().public_key().to_hex();
    let other = Keys::generate().public_key().to_hex();
    let mut config = AppConfig::generated();
    let network_id = config.networks[0].id.clone();
    config.networks[0].network_id = "mesh-home".to_string();

    config
        .record_inbound_join_request("mesh-home", &requester, "alice-phone", 1_726_000_000)
        .expect("record requester");
    config
        .record_inbound_join_request("mesh-home", &other, "bob-phone", 1_726_000_001)
        .expect("record other");

    config
        .reject_inbound_join_request(&network_id, &requester)
        .expect("reject join request");

    assert_eq!(config.networks[0].inbound_join_requests.len(), 1);
    assert_eq!(config.networks[0].inbound_join_requests[0].requester, other);
}
