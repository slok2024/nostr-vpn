use super::*;

#[test]
fn legacy_prefixed_network_ids_are_normalized_at_runtime() {
    let mut config = AppConfig::generated();
    config.networks[0].network_id = "nostr-vpn:1234abcd5678ef90".to_string();

    config.ensure_defaults();

    assert_eq!(config.networks[0].network_id, "nostr-vpn:1234abcd5678ef90");
    assert_eq!(config.effective_network_id(), "1234abcd5678ef90");
}

#[test]
fn default_node_name_from_hostname_uses_first_label() {
    assert_eq!(
        default_node_name_from_hostname("Siriuss-Mini.fritz.box").as_deref(),
        Some("siriuss-mini")
    );
}

#[test]
fn default_node_name_from_hostname_normalizes_human_device_names() {
    assert_eq!(
        default_node_name_from_hostname("Sirius's Mac mini").as_deref(),
        Some("sirius-s-mac-mini")
    );
}

#[test]
fn default_node_name_from_hostname_ignores_localhost_placeholders() {
    assert_eq!(default_node_name_from_hostname("localhost"), None);
    assert_eq!(
        default_node_name_from_hostname("localhost.localdomain"),
        None
    );
}

#[test]
fn default_node_name_resolution_prefers_hostname_over_petname() {
    let keys = Keys::generate();
    let own_hex = keys.public_key().to_hex();

    assert_eq!(
        default_node_name_for_hostname_or_pubkey(Some("Siriuss-Mini.fritz.box"), &own_hex),
        "siriuss-mini"
    );
}

#[test]
fn default_node_name_resolution_falls_back_to_petname_for_localhost() {
    let keys = Keys::generate();
    let own_hex = keys.public_key().to_hex();

    assert_eq!(
        default_node_name_for_hostname_or_pubkey(Some("localhost.localdomain"), &own_hex),
        default_node_name_for_pubkey(&own_hex)
    );
}

#[test]
fn legacy_default_node_name_migrates_to_non_generic_default() {
    let keys = Keys::generate();
    let mut config = AppConfig::generated();
    config.nostr.secret_key = keys.secret_key().to_secret_hex();
    config.nostr.public_key = keys.public_key().to_hex();
    config.node_name = "nostr-vpn-node".to_string();

    config.ensure_defaults();

    assert!(!config.node_name.trim().is_empty());
    assert_ne!(config.node_name, "nostr-vpn-node");
}

#[test]
fn custom_node_name_is_preserved() {
    let keys = Keys::generate();
    let own_hex = keys.public_key().to_hex();
    let mut config = AppConfig::generated();
    config.nostr.secret_key = keys.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex;
    config.node_name = "my-pocket-router".to_string();

    config.ensure_defaults();

    assert_eq!(config.node_name, "my-pocket-router");
}

#[test]
fn default_network_id_stays_placeholder_without_participants() {
    let mut config = AppConfig::generated();
    keep_endpoint_autoconfig_off(&mut config);

    maybe_autoconfigure_node(&mut config);

    assert_eq!(config.effective_network_id(), "nostr-vpn");
}

#[test]
fn legacy_top_level_network_id_is_ignored_when_loading_current_config_schema() {
    let own = Keys::generate();
    let peer = Keys::generate();
    let own_hex = own.public_key().to_hex();
    let peer_hex = peer.public_key().to_hex();
    let expected_network_id =
        derive_network_id_from_participants(&[own_hex.clone(), peer_hex.clone()]);
    let raw = format!(
        r#"
network_id = "mesh-legacy"
node_name = "node"
auto_disconnect_relays_when_mesh_ready = true
lan_discovery_enabled = true
launch_on_startup = true
autoconnect = true
close_to_tray_on_close = true

[[networks]]
id = "network-1"
name = "Network 1"
enabled = true
network_id = "nostr-vpn"
participants = ["{peer_hex}"]

[nostr]
relays = ["wss://temp.iris.to"]
secret_key = "{secret_key}"
public_key = "{own_hex}"

[node]
id = "node-id"
private_key = ""
public_key = ""
endpoint = "127.0.0.1:51820"
tunnel_ip = "10.44.0.1/32"
listen_port = 51820
"#,
        secret_key = own.secret_key().to_secret_hex(),
    );

    let mut config: AppConfig = toml::from_str(&raw).expect("parse config");
    config.ensure_defaults();

    assert_eq!(config.effective_network_id(), expected_network_id);
}

#[test]
fn tunnel_ip_stays_stable_when_roster_changes_if_network_id_is_fixed() {
    let mut keys = vec![Keys::generate(), Keys::generate(), Keys::generate()];
    keys.sort_by_key(|entry| entry.public_key().to_hex());

    let own = keys.remove(1);
    let low = keys.remove(0).public_key().to_hex();
    let high = keys.remove(0).public_key().to_hex();
    let own_hex = own.public_key().to_hex();

    let mut config = AppConfig::generated();
    config.networks[0].network_id = "mesh-fixed".to_string();
    config.nostr.secret_key = own.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex.clone();
    set_default_network_participants(&mut config, vec![high.clone()]);
    config.node.tunnel_ip = "10.44.0.1/32".to_string();
    keep_endpoint_autoconfig_off(&mut config);

    maybe_autoconfigure_node(&mut config);
    let first_ip = config.node.tunnel_ip.clone();

    set_default_network_participants(&mut config, vec![high, low]);
    config.node.tunnel_ip = "10.44.0.1/32".to_string();
    maybe_autoconfigure_node(&mut config);

    assert_eq!(config.node.tunnel_ip, first_ip);
    assert_ne!(config.node.tunnel_ip, "10.44.0.1/32");
}

#[test]
fn endpoint_and_tunnel_autoconfig_detection_works() {
    assert!(needs_endpoint_autoconfig("127.0.0.1:51820"));
    assert!(needs_endpoint_autoconfig("0.0.0.0:51820"));
    assert!(!needs_endpoint_autoconfig("198.51.100.10:51820"));

    assert!(needs_tunnel_ip_autoconfig("10.44.0.1/32"));
    assert!(!needs_tunnel_ip_autoconfig("10.44.0.15/32"));
}

#[test]
fn lan_discovery_defaults_true_when_missing_from_toml() {
    let raw = r#"
network_id = "nostr-vpn"
node_name = "node"
auto_disconnect_relays_when_mesh_ready = true
[[networks]]
id = "network-1"
name = "Network 1"
enabled = true
participants = []

[nostr]
relays = ["wss://temp.iris.to"]
secret_key = ""
public_key = ""

[node]
id = "node-id"
private_key = ""
public_key = ""
endpoint = "127.0.0.1:51820"
tunnel_ip = "10.44.0.1/32"
listen_port = 51820
"#;

    let config: AppConfig = toml::from_str(raw).expect("parse config");
    assert!(config.lan_discovery_enabled);
}

#[test]
fn save_omits_legacy_lan_discovery_flag() {
    let path = unique_temp_config_path("omit-legacy-lan-discovery");
    let config = AppConfig::generated();

    config.save(&path).expect("save config");
    let raw = fs::read_to_string(&path).expect("read saved config");
    let _ = fs::remove_file(&path);

    assert!(!raw.contains("lan_discovery_enabled"));
    assert!(!raw.contains("auto_disconnect_relays_when_mesh_ready"));
}

#[cfg(unix)]
#[test]
fn save_creates_private_config_file_on_unix() {
    use std::os::unix::fs::PermissionsExt;

    let path = unique_temp_config_path("private-config-mode");
    let config = AppConfig::generated();

    config.save(&path).expect("save config");
    let mode = fs::metadata(&path)
        .expect("config metadata")
        .permissions()
        .mode()
        & 0o777;
    let _ = fs::remove_file(&path);

    assert_eq!(mode, 0o600);
}

#[cfg(unix)]
#[test]
fn save_replaces_config_symlink_instead_of_following_it() {
    use std::os::unix::fs::symlink;

    let dir = std::env::temp_dir().join(format!(
        "nostr-vpn-config-symlink-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    let target = dir.join("target");
    let link = dir.join("config.toml");
    fs::write(&target, "do-not-overwrite").expect("write target");
    symlink(&target, &link).expect("create symlink");

    AppConfig::generated().save(&link).expect("save config");

    assert_eq!(
        fs::read_to_string(&target).expect("read target"),
        "do-not-overwrite"
    );
    assert!(
        !fs::symlink_metadata(&link)
            .expect("link metadata")
            .file_type()
            .is_symlink()
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn legacy_auto_disconnect_flag_is_ignored_when_loading() {
    let path = unique_temp_config_path("ignore-legacy-auto-disconnect");
    let raw = r#"
node_name = "node"
auto_disconnect_relays_when_mesh_ready = true
lan_discovery_enabled = true
launch_on_startup = true
autoconnect = true
close_to_tray_on_close = true

[[networks]]
id = "network-1"
name = "Network 1"
enabled = true
participants = []

[nostr]
relays = ["wss://temp.iris.to"]
secret_key = ""
public_key = ""

[node]
id = "node-id"
private_key = ""
public_key = ""
endpoint = "127.0.0.1:51820"
tunnel_ip = "10.44.0.1/32"
listen_port = 51820
"#;

    fs::write(&path, raw).expect("write config");
    let config = AppConfig::load(&path).expect("load config");
    let _ = fs::remove_file(&path);

    assert_eq!(config.node_name, "node");
}

#[test]
fn close_to_tray_defaults_true_when_missing_from_toml() {
    let raw = r#"
network_id = "nostr-vpn"
node_name = "node"
auto_disconnect_relays_when_mesh_ready = true
lan_discovery_enabled = true
[[networks]]
id = "network-1"
name = "Network 1"
enabled = true
participants = []

[nostr]
relays = ["wss://temp.iris.to"]
secret_key = ""
public_key = ""

[node]
id = "node-id"
private_key = ""
public_key = ""
endpoint = "127.0.0.1:51820"
tunnel_ip = "10.44.0.1/32"
listen_port = 51820
"#;

    let config: AppConfig = toml::from_str(raw).expect("parse config");
    assert!(config.close_to_tray_on_close);
}

#[test]
fn launch_on_startup_defaults_true_when_missing_from_toml() {
    let raw = r#"
network_id = "nostr-vpn"
node_name = "node"
auto_disconnect_relays_when_mesh_ready = true
lan_discovery_enabled = true
close_to_tray_on_close = true
[[networks]]
id = "network-1"
name = "Network 1"
enabled = true
participants = []

[nostr]
relays = ["wss://temp.iris.to"]
secret_key = ""
public_key = ""

[node]
id = "node-id"
private_key = ""
public_key = ""
endpoint = "127.0.0.1:51820"
tunnel_ip = "10.44.0.1/32"
listen_port = 51820
"#;

    let config: AppConfig = toml::from_str(raw).expect("parse config");
    assert!(config.launch_on_startup);
}

#[test]
fn autoconnect_defaults_true_when_missing_from_toml() {
    let raw = r#"
network_id = "nostr-vpn"
node_name = "node"
auto_disconnect_relays_when_mesh_ready = true
lan_discovery_enabled = true
launch_on_startup = true
close_to_tray_on_close = true
[[networks]]
id = "network-1"
name = "Network 1"
enabled = true
participants = []

[nostr]
relays = ["wss://temp.iris.to"]
secret_key = ""
public_key = ""

[node]
id = "node-id"
private_key = ""
public_key = ""
endpoint = "127.0.0.1:51820"
tunnel_ip = "10.44.0.1/32"
listen_port = 51820
"#;

    let config: AppConfig = toml::from_str(raw).expect("parse config");
    assert!(config.autoconnect);
}

#[test]
fn reciprocal_participant_configs_share_effective_network_id() {
    let alice = Keys::generate();
    let bob = Keys::generate();
    let alice_hex = alice.public_key().to_hex();
    let bob_hex = bob.public_key().to_hex();

    let mut alice_config = AppConfig::generated();
    alice_config.nostr.secret_key = alice.secret_key().to_secret_hex();
    alice_config.nostr.public_key = alice_hex.clone();
    set_default_network_participants(&mut alice_config, vec![bob_hex.clone()]);
    keep_endpoint_autoconfig_off(&mut alice_config);
    maybe_autoconfigure_node(&mut alice_config);

    let mut bob_config = AppConfig::generated();
    bob_config.nostr.secret_key = bob.secret_key().to_secret_hex();
    bob_config.nostr.public_key = bob_hex.clone();
    set_default_network_participants(&mut bob_config, vec![alice_hex.clone()]);
    keep_endpoint_autoconfig_off(&mut bob_config);
    maybe_autoconfigure_node(&mut bob_config);

    assert_ne!(alice_config.effective_network_id(), "nostr-vpn");
    assert_ne!(bob_config.effective_network_id(), "nostr-vpn");
    assert!(!alice_config.effective_network_id().contains(':'));
    assert!(!bob_config.effective_network_id().contains(':'));
    assert_eq!(
        alice_config.effective_network_id(),
        bob_config.effective_network_id()
    );

    assert_ne!(alice_config.node.tunnel_ip, bob_config.node.tunnel_ip);
    assert_eq!(
        derive_mesh_tunnel_ip(&alice_config.effective_network_id(), &alice_hex)
            .expect("alice tunnel ip"),
        alice_config.node.tunnel_ip
    );
    assert_eq!(
        derive_mesh_tunnel_ip(&bob_config.effective_network_id(), &bob_hex).expect("bob tunnel ip"),
        bob_config.node.tunnel_ip
    );
}

#[test]
fn active_network_helpers_ignore_inactive_networks() {
    let own_keys = Keys::generate();
    let own_hex = own_keys.public_key().to_hex();
    let peer_a = Keys::generate().public_key().to_hex();
    let peer_b = Keys::generate().public_key().to_hex();

    let mut config = AppConfig::generated();
    config.nostr.secret_key = own_keys.secret_key().to_secret_hex();
    config.nostr.public_key = own_hex.clone();
    config.networks = vec![
        NetworkConfig {
            id: "network-1".to_string(),
            name: "oma".to_string(),
            enabled: true,
            network_id: "mesh-home".to_string(),
            participants: vec![peer_a.clone()],
            admins: Vec::new(),
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        },
        NetworkConfig {
            id: "network-2".to_string(),
            name: "lauri".to_string(),
            enabled: false,
            network_id: "mesh-work".to_string(),
            participants: vec![peer_b.clone()],
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

    assert_eq!(config.effective_network_id(), "mesh-home");
    assert_eq!(config.participant_pubkeys_hex(), vec![peer_a.clone()]);

    let mut expected_all = vec![peer_a.clone(), peer_b];
    expected_all.sort();
    assert_eq!(config.all_participant_pubkeys_hex(), expected_all);

    let mut expected_members = vec![peer_a, own_hex];
    expected_members.sort();
    assert_eq!(config.mesh_members_pubkeys(), expected_members);

    let meshes = config.enabled_network_meshes();
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].network_id, "mesh-home");
}

#[test]
fn activating_one_network_disables_the_others() {
    let mut config = AppConfig::generated();
    let first_id = config.networks[0].id.clone();
    config.networks[0].network_id = "mesh-home".to_string();
    let second_id = config.add_network("Work");
    config
        .network_by_id_mut(&second_id)
        .expect("second network")
        .network_id = "mesh-work".to_string();

    config
        .set_network_enabled(&second_id, true)
        .expect("activate second network");

    assert_eq!(config.enabled_network_count(), 1);
    assert!(
        !config
            .network_by_id(&first_id)
            .expect("first network")
            .enabled
    );
    assert!(
        config
            .network_by_id(&second_id)
            .expect("second network")
            .enabled
    );
    assert_eq!(config.effective_network_id(), "mesh-work");
}

#[test]
fn cannot_disable_the_last_active_network() {
    let mut config = AppConfig::generated();
    let active_id = config.networks[0].id.clone();

    let error = config
        .set_network_enabled(&active_id, false)
        .expect_err("last active network should stay active");

    assert!(error.to_string().contains("active network"));
    assert_eq!(config.enabled_network_count(), 1);
    assert!(
        config
            .network_by_id(&active_id)
            .expect("active network")
            .enabled
    );
}

#[test]
fn added_networks_start_inactive_with_their_own_mesh_slot() {
    let mut config = AppConfig::generated();
    let original_active_id = config.networks[0].id.clone();

    let added_id = config.add_network("Work");

    assert_eq!(config.enabled_network_count(), 1);
    assert!(
        config
            .network_by_id(&original_active_id)
            .expect("original active network")
            .enabled
    );

    let added = config.network_by_id(&added_id).expect("added network");
    assert!(!added.enabled);
    assert_eq!(added.network_id, "nostr-vpn");
}

#[test]
fn explicit_network_id_takes_precedence_over_participant_hash() {
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
fn set_network_mesh_id_updates_the_selected_network() {
    let mut config = AppConfig::generated();
    let original_active_id = config.networks[0].id.clone();
    let added_id = config.add_network("Work");

    config
        .set_network_mesh_id(&added_id, "mesh-work")
        .expect("mesh id should update");

    assert_eq!(
        config
            .network_by_id(&added_id)
            .expect("saved network")
            .network_id,
        "mesh-work"
    );
    assert_eq!(
        config
            .network_by_id(&original_active_id)
            .expect("active network")
            .network_id,
        "nostr-vpn"
    );
    assert_eq!(config.effective_network_id(), "nostr-vpn");
}

#[test]
fn set_network_mesh_id_rejects_empty_values() {
    let mut config = AppConfig::generated();
    let active_id = config.networks[0].id.clone();

    let error = config
        .set_network_mesh_id(&active_id, "   ")
        .expect_err("empty mesh id should fail");

    assert_eq!(error.to_string(), "network id cannot be empty");
}
