use super::*;
use nostr_vpn_core::magic_dns::build_magic_dns_records;

#[test]
fn unnamed_participant_has_no_magic_dns_record_until_admin_sets_alias() {
    let own = Keys::generate();
    let peer = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let peer_hex = peer.public_key().to_hex();

    let mut config = AppConfig::generated();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex;
    set_default_network_participants(&mut config, vec![peer_hex.clone()]);
    config.ensure_defaults();

    assert_eq!(config.peer_alias(&peer_hex), None);
    assert_eq!(config.magic_dns_name_for_participant(&peer_hex), None);
    assert_eq!(config.resolve_magic_dns_query("peer.nvpn"), None);

    let alias = config
        .set_peer_alias(&peer_hex, "Home Server")
        .expect("set alias");
    let fqdn = config
        .magic_dns_name_for_participant(&peer_hex)
        .expect("magic dns fqdn");
    assert_eq!(alias, "home-server");
    assert_eq!(
        config.resolve_magic_dns_query(&alias),
        Some(peer_hex.clone())
    );
    assert_eq!(
        config.resolve_magic_dns_query(&fqdn),
        Some(peer_hex.clone())
    );
}

#[test]
fn set_peer_alias_normalizes_and_blank_clears_magic_dns() {
    let own = Keys::generate();
    let peer = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let peer_hex = peer.public_key().to_hex();

    let mut config = AppConfig::generated();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex;
    set_default_network_participants(&mut config, vec![peer_hex.clone()]);
    config.ensure_defaults();

    let custom_alias = config
        .set_peer_alias(&peer_hex, "Home Server !!")
        .expect("set custom alias");
    assert_eq!(custom_alias, "home-server");
    assert_eq!(
        config
            .magic_dns_name_for_participant(&peer_hex)
            .expect("dns name"),
        "home-server.nvpn"
    );

    let reset_alias = config
        .set_peer_alias(&peer_hex, "   ")
        .expect("reset alias");
    assert_eq!(reset_alias, "");
    assert_eq!(config.peer_alias(&peer_hex), None);
    assert_eq!(config.magic_dns_name_for_participant(&peer_hex), None);
}

#[test]
fn self_magic_dns_name_uses_assigned_own_alias_and_resolves_to_own_pubkey() {
    let own = Keys::generate();
    let own_hex = own.public_key().to_hex();

    let mut config = AppConfig::generated_without_networks();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex.clone();
    config.node_name = "My Pocket Router".to_string();
    config.add_network("Joined");
    config.ensure_defaults();
    config
        .set_peer_alias(&own_hex, "My Pocket Router")
        .expect("set own alias");

    assert_eq!(
        config.self_magic_dns_label().as_deref(),
        Some("my-pocket-router")
    );
    assert_eq!(
        config.self_magic_dns_name().as_deref(),
        Some("my-pocket-router.nvpn")
    );
    assert_eq!(
        config.resolve_magic_dns_query("my-pocket-router"),
        Some(own_hex.clone())
    );
    assert_eq!(
        config.resolve_magic_dns_query("my-pocket-router.nvpn"),
        Some(own_hex)
    );
}

#[test]
fn generic_add_network_does_not_guess_self_magic_dns_from_node_name() {
    let mut config = AppConfig::generated_without_networks();
    config.node_name = "Example iPhone".to_string();

    config.add_network("Joined");
    config.ensure_defaults();

    assert_ne!(
        config.self_magic_dns_name().as_deref(),
        Some("example-iphone.nvpn")
    );
}

#[test]
fn add_owned_first_network_seeds_local_device_name_as_self_magic_dns_alias() {
    let mut config = AppConfig::generated_without_networks();
    config.node_name = "Example Mac mini".to_string();

    config.add_owned_network("Home");
    config.ensure_defaults();

    assert_eq!(config.node_name, "Example Mac mini");
    assert_eq!(
        config.self_magic_dns_name().as_deref(),
        Some("example-mac-mini.nvpn")
    );
}

#[test]
fn adding_later_network_preserves_configured_device_name() {
    let mut config = AppConfig::generated_without_networks();
    config.node_name = "Example Mac mini".to_string();

    config.add_owned_network("Home");
    config.node_name = "My Pocket Router".to_string();
    config.add_owned_network("Work");
    config.ensure_defaults();

    assert_eq!(config.node_name, "My Pocket Router");
    assert_eq!(
        config.self_magic_dns_name().as_deref(),
        Some("example-mac-mini.nvpn")
    );
}

#[test]
fn self_magic_dns_label_keeps_assigned_own_alias_and_suffixes_colliding_peer_aliases() {
    let own = Keys::generate();
    let peer = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let peer_hex = peer.public_key().to_hex();

    let mut config = AppConfig::generated_without_networks();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex;
    config.node_name = "Home Server".to_string();
    config.add_network("Joined");
    set_default_network_participants(&mut config, vec![peer_hex.clone()]);
    config.ensure_defaults();
    let own_pubkey = config.own_nostr_pubkey_hex().expect("own pubkey");
    config
        .set_peer_alias(&own_pubkey, "home-server")
        .expect("set own alias");

    let assigned_peer_alias = config
        .set_peer_alias(&peer_hex, "home-server")
        .expect("set colliding alias");

    assert_eq!(
        config.self_magic_dns_name().as_deref(),
        Some("home-server.nvpn")
    );
    assert_eq!(assigned_peer_alias, "home-server-2");
    assert_eq!(
        config.peer_alias(&peer_hex).as_deref(),
        Some("home-server-2")
    );
}

#[test]
fn self_magic_dns_label_uses_assigned_own_peer_alias() {
    let own = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let own_npub = own.public_key().to_bech32().expect("own npub");

    let mut config = AppConfig::generated_without_networks();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex;
    config.node_name = "Example iPhone".to_string();
    config.peer_aliases.insert(own_npub, "iphone1".to_string());
    config.add_network("Joined");
    config.ensure_defaults();

    assert_eq!(config.self_magic_dns_label().as_deref(), Some("iphone1"));
}

#[test]
fn existing_generated_self_alias_adopts_custom_node_name_on_update() {
    let own = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let own_npub = own.public_key().to_bech32().expect("own npub");

    let mut config = AppConfig::generated_without_networks();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex.clone();
    config.node_name = "iPhone".to_string();
    config.add_network("Home");
    config.networks[0].admins = vec![own_hex.clone()];
    config
        .peer_aliases
        .insert(own_npub, default_node_name_for_pubkey(&own_hex));

    config.ensure_defaults();

    assert_eq!(config.self_magic_dns_label().as_deref(), Some("iphone"));
}

#[test]
fn pending_join_self_alias_uses_self_until_roster_names_device() {
    let own = Keys::generate();
    let own_hex = own.public_key().to_hex();

    let mut config = AppConfig::generated_without_networks();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex.clone();
    config.node_name = "Example iPhone".to_string();
    config.add_network("Joined");

    let alias = config
        .ensure_temporary_self_magic_dns_alias()
        .expect("temporary self alias");

    assert_eq!(alias, "self");
    assert_eq!(config.self_magic_dns_name().as_deref(), Some("self.nvpn"));

    config.networks[0].participants = vec![own_hex];
    config.ensure_defaults();

    assert_eq!(
        config.self_magic_dns_name().as_deref(),
        Some("example-iphone.nvpn")
    );
}

#[test]
fn active_admin_alias_resolves_even_when_not_a_participant() {
    let own = Keys::generate();
    let admin = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let admin_hex = admin.public_key().to_hex();

    let mut config = AppConfig::generated_without_networks();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex;
    let network_id = config.add_network("Joined");
    config
        .set_network_mesh_id(&network_id, "mesh-home")
        .expect("set mesh id");
    config.networks[0].admins = vec![admin_hex.clone()];
    config
        .set_peer_alias(&admin_hex, "admin")
        .expect("set admin alias");

    let records = build_magic_dns_records(&config);
    let expected_ip = derive_mesh_tunnel_ip("mesh-home", &admin_hex)
        .expect("admin tunnel ip")
        .trim_end_matches("/32")
        .parse()
        .expect("ipv4");

    assert_eq!(records.get("admin.nvpn").copied(), Some(expected_ip));
}

#[test]
fn peer_aliases_use_npub_keys_in_serialized_config() {
    let own = Keys::generate();
    let peer = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let peer_hex = peer.public_key().to_hex();
    let peer_npub = peer.public_key().to_bech32().expect("peer npub");

    let mut config = AppConfig::generated();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex;
    set_default_network_participants(&mut config, vec![peer_hex.clone()]);
    config.ensure_defaults();
    config
        .set_peer_alias(&peer_hex, "server-a")
        .expect("set alias");

    assert!(config.peer_aliases.contains_key(&peer_npub));
    assert!(!config.peer_aliases.contains_key(&peer_hex));
}

#[test]
fn save_serializes_user_facing_pubkeys_as_npubs() {
    let own = Keys::generate();
    let peer = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let own_npub = own.public_key().to_bech32().expect("own npub");
    let peer_hex = peer.public_key().to_hex();
    let peer_npub = peer.public_key().to_bech32().expect("peer npub");

    let mut config = AppConfig::generated();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex;
    set_default_network_participants(&mut config, vec![peer_hex.clone()]);
    config.exit_node = peer_hex.clone();
    config.ensure_defaults();

    let path = unique_temp_config_path("save-serializes-user-facing-pubkeys");
    config.save(&path).expect("save config");
    let raw = fs::read_to_string(&path).expect("read saved config");
    let _ = fs::remove_file(&path);

    assert!(raw.contains(&format!("public_key = \"{own_npub}\"")));
    assert!(raw.contains(&format!("exit_node = \"{peer_npub}\"")));
    assert!(raw.contains(&format!("participants = [\"{peer_npub}\"]")));
    assert!(!raw.contains(&peer_hex));
}

#[test]
fn save_and_load_round_trip_keeps_runtime_pubkeys_normalized() {
    let own = Keys::generate();
    let peer = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let peer_hex = peer.public_key().to_hex();

    let mut config = AppConfig::generated();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex.clone();
    set_default_network_participants(&mut config, vec![peer_hex.clone()]);
    config.exit_node = peer_hex.clone();
    config.ensure_defaults();

    let path = unique_temp_config_path("save-load-roundtrip");
    config.save(&path).expect("save config");
    let loaded = AppConfig::load(&path).expect("load config");
    let _ = fs::remove_file(&path);

    assert_eq!(loaded.participant_pubkeys_hex(), vec![peer_hex.clone()]);
    assert_eq!(loaded.exit_node, peer_hex);
    assert_eq!(
        loaded.own_nostr_pubkey_hex().expect("own pubkey hex"),
        own_hex
    );
}

#[test]
fn default_aliases_are_not_generated_for_unnamed_participants() {
    let own = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let peer_a = Keys::generate().public_key().to_hex();
    let peer_b = Keys::generate().public_key().to_hex();
    let peer_c = Keys::generate().public_key().to_hex();

    let mut config = AppConfig::generated();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex;
    set_default_network_participants(
        &mut config,
        vec![peer_a.clone(), peer_b.clone(), peer_c.clone()],
    );
    config.ensure_defaults();

    assert_eq!(config.peer_alias(&peer_a), None);
    assert_eq!(config.peer_alias(&peer_b), None);
    assert_eq!(config.peer_alias(&peer_c), None);
    assert!(config.peer_aliases.is_empty());
}
