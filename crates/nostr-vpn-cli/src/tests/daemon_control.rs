use std::fs;
use std::net::Ipv4Addr;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nostr_sdk::prelude::Keys;

use super::control_daemon_request_for_test;
use crate::*;

fn activate_first_network(config: &mut AppConfig) {
    let network_id = config.networks[0].id.clone();
    config
        .set_network_enabled(&network_id, true)
        .expect("activate first network");
}

#[test]
fn daemon_runtime_state_requires_advertised_routes() {
    let raw = r#"{
  "updated_at": 1773650797,
  "vpn_enabled": true,
  "vpn_active": true,
  "vpn_status": "Connected",
  "expected_peer_count": 1,
  "connected_peer_count": 1,
  "mesh_ready": true,
  "peers": [
{
  "participant_pubkey": "ed91c2fcdf6857157e72497d67be9dad91d865db6407bb0440ca53129e10cb1f",
  "node_id": "6bea57f5-e06b-49d1-83b5-484ab0a3df12",
  "tunnel_ip": "10.44.0.239/32",
  "endpoint": "192.168.178.80:51820",
  "public_key": "+fi3YvMFH0JQFNuQPiPy5xBXNKvpaCKIbbbgrlXT5yw=",
  "last_mesh_seen_at": 1773650779,
  "last_fips_seen_at": 1773650779,
  "reachable": true,
  "last_handshake_at": null,
  "error": null
}
  ]
}"#;

    assert!(serde_json::from_str::<DaemonRuntimeState>(raw).is_err());
}

#[test]
fn read_daemon_state_trims_nul_padding() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-daemon-state-trim-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let state_path = dir.join("daemon.state.json");
    let mut raw = br#"{
  "updated_at": 1,
  "binary_version": "test",
  "vpn_enabled": true,
  "vpn_active": true,
  "vpn_status": "Ready",
  "expected_peer_count": 0,
  "connected_peer_count": 0,
  "mesh_ready": false
}"#
    .to_vec();
    raw.extend_from_slice(&[0, 0, b'\n']);
    fs::write(&state_path, raw).expect("write padded daemon state");

    let state = read_daemon_state(&state_path)
        .expect("read daemon state")
        .expect("daemon state should exist");
    assert_eq!(state.vpn_status, "Ready");

    let rewritten = fs::read(&state_path).expect("read rewritten daemon state");
    assert!(!rewritten.contains(&0));

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn atomic_runtime_write_creates_private_file() {
    use std::os::unix::fs::PermissionsExt;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-runtime-mode-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("staged-config.toml");

    write_runtime_file_atomically(&path, b"secret = true\n").expect("write runtime file");
    let mode = fs::metadata(&path)
        .expect("runtime metadata")
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(mode, 0o600);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_control_request_projects_desired_vpn_state_immediately() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-control-project-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");
    let state_path = daemon_state_file_path(&config_path);
    let state = DaemonRuntimeState {
        updated_at: 1,
        vpn_enabled: true,
        vpn_active: true,
        vpn_status: "VPN on".to_string(),
        expected_peer_count: 1,
        connected_peer_count: 1,
        mesh_ready: true,
        ..DaemonRuntimeState::default()
    };
    write_daemon_state(&state_path, &state).expect("write daemon state");

    write_daemon_control_request(&config_path, DaemonControlRequest::Pause)
        .expect("write pause control request");
    let paused = read_daemon_state(&state_path)
        .expect("read projected daemon state")
        .expect("daemon state should exist");
    assert!(!paused.vpn_enabled);
    assert!(paused.vpn_active);
    assert_eq!(paused.vpn_status, "Turning VPN off");

    write_daemon_control_request(&config_path, DaemonControlRequest::Resume)
        .expect("write resume control request");
    let resumed = read_daemon_state(&state_path)
        .expect("read projected daemon state")
        .expect("daemon state should exist");
    assert!(resumed.vpn_enabled);
    assert_eq!(resumed.vpn_status, "VPN on");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_control_request_persists_desired_vpn_state() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-control-persist-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");
    let mut config = AppConfig::generated();
    config.autoconnect = true;
    config.save(&config_path).expect("save config");

    write_daemon_control_request(&config_path, DaemonControlRequest::Pause)
        .expect("write pause control request");
    let paused = AppConfig::load(&config_path).expect("load paused config");
    assert!(!paused.autoconnect);

    write_daemon_control_request(&config_path, DaemonControlRequest::Resume)
        .expect("write resume control request");
    let resumed = AppConfig::load(&config_path).expect("load resumed config");
    assert!(resumed.autoconnect);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_start_honors_persisted_desired_vpn_state() {
    let mut config = AppConfig::generated();

    config.autoconnect = true;
    assert!(daemon_start_vpn_enabled(&config, false));
    assert!(!daemon_start_vpn_enabled(&config, true));

    config.autoconnect = false;
    assert!(!daemon_start_vpn_enabled(&config, false));
    assert!(!daemon_start_vpn_enabled(&config, true));
}

#[test]
fn daemon_state_freshness_allows_pid_namespace_status() {
    let state = DaemonRuntimeState {
        updated_at: 100,
        ..DaemonRuntimeState::default()
    };

    assert!(daemon_state_is_fresh(&state, 105, 10));
    assert!(!daemon_state_is_fresh(&state, 111, 10));
    assert!(!daemon_state_is_fresh(
        &DaemonRuntimeState::default(),
        105,
        10
    ));
}

#[test]
fn daemon_log_compaction_leaves_small_log_untouched() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-daemon-log-small-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let log_path = dir.join("daemon.log");
    fs::write(&log_path, "short\n").expect("write daemon log");

    assert!(
        !compact_log_file_if_needed(&log_path, 64, 16).expect("compact daemon log"),
        "small logs should not be compacted"
    );
    assert_eq!(
        fs::read_to_string(&log_path).expect("read daemon log"),
        "short\n"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_log_compaction_keeps_line_aligned_tail() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-daemon-log-compact-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let log_path = dir.join("daemon.log");
    fs::write(&log_path, "old-0\nold-1\nold-2\nkeep-1\nkeep-2\n").expect("write daemon log");

    assert!(
        compact_log_file_if_needed(&log_path, 20, 14).expect("compact daemon log"),
        "oversized logs should be compacted"
    );
    let compacted = fs::read_to_string(&log_path).expect("read compacted daemon log");
    assert!(compacted.starts_with("[nvpn] daemon log compacted at "));
    assert!(!compacted.contains("old-0"));
    assert!(!compacted.contains("keep-1"));
    assert!(compacted.ends_with("keep-2\n"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_status_ignores_and_quarantines_corrupt_daemon_state() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-daemon-status-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");
    fs::write(&config_path, "").expect("write config placeholder");
    let state_path = dir.join("daemon.state.json");
    fs::write(&state_path, vec![0; 64]).expect("write corrupt daemon state");

    let status = crate::daemon_status(&config_path).expect("daemon status should succeed");
    assert!(status.state.is_none());
    assert!(!state_path.exists());

    let quarantined: Vec<_> = fs::read_dir(&dir)
        .expect("list temp dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.starts_with("daemon.state.json.corrupt-"))
        })
        .collect();
    assert_eq!(quarantined.len(), 1);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_status_does_not_repair_network_state_when_daemon_is_stopped() {
    let _guard = crate::repair_saved_network_state_call_lock_for_test()
        .lock()
        .expect("repair call test lock");
    crate::reset_repair_saved_network_state_call_count_for_test();

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-daemon-status-pure-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");
    fs::write(&config_path, "").expect("write config placeholder");

    let status = crate::daemon_status(&config_path).expect("daemon status should succeed");
    assert!(!status.running);
    assert_eq!(crate::repair_saved_network_state_call_count_for_test(), 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn persist_daemon_runtime_state_marks_vpn_on_as_active() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-daemon-state-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let state_path = dir.join("daemon.state.json");

    let mut config = AppConfig::generated();
    config.networks[0].participants = vec!["11".repeat(32)];
    let tunnel_runtime = crate::CliTunnelRuntime::new("utun100");

    crate::persist_daemon_runtime_state(
        &state_path,
        &config,
        true,
        1,
        &tunnel_runtime,
        &[],
        &[crate::DaemonRelayState {
            url: "wss://relay.example".to_string(),
            status: "connected".to_string(),
        }],
        &std::collections::HashMap::new(),
        "VPN on",
        &nostr_vpn_core::diagnostics::NetworkSummary::default(),
        &nostr_vpn_core::diagnostics::PortMappingStatus::default(),
    )
    .expect("persist daemon runtime state");

    let state = read_daemon_state(&state_path)
        .expect("read daemon state")
        .expect("daemon state should exist");
    assert!(state.vpn_active);
    assert_eq!(state.vpn_status, "VPN on");
    assert_eq!(state.relays.len(), 1);
    assert_eq!(state.relays[0].status, "connected");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn fips_runtime_state_is_ready_without_waiting_for_every_peer() {
    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    config.networks[0].participants = vec!["11".repeat(32), "22".repeat(32)];
    let tunnel_runtime = crate::CliTunnelRuntime::new("utun100");

    let state = crate::build_daemon_runtime_state(
        &config,
        true,
        true,
        2,
        &tunnel_runtime,
        &[],
        &[],
        &std::collections::HashMap::new(),
        "VPN on",
        &nostr_vpn_core::diagnostics::NetworkSummary::default(),
        &nostr_vpn_core::diagnostics::PortMappingStatus::default(),
    );

    assert_eq!(state.connected_peer_count, 0);
    assert!(state.vpn_enabled);
    assert!(state.mesh_ready);
    assert_eq!(state.vpn_status, "VPN on");
}

#[test]
fn fips_runtime_state_counts_direct_roster_and_other_peers() {
    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    let roster_peer = Keys::generate().public_key().to_hex();
    let routed_roster_peer = Keys::generate().public_key().to_hex();
    let other_peer = Keys::generate().public_key().to_hex();
    config.networks[0].participants = vec![roster_peer.clone(), routed_roster_peer.clone()];
    let tunnel_runtime = crate::CliTunnelRuntime::new("utun100");

    let state = crate::build_daemon_runtime_state(
        &config,
        true,
        true,
        2,
        &tunnel_runtime,
        &[
            MeshPeerStatus {
                pubkey: roster_peer,
                connected: true,
                endpoint_npub: "npub1roster".to_string(),
                transport_addr: Some("203.0.113.8:9000".to_string()),
                transport_type: Some("udp".to_string()),
                srtt_ms: Some(5),
                link_packets_sent: 0,
                link_packets_recv: 0,
                link_bytes_sent: 0,
                link_bytes_recv: 0,
                direct_probe_pending: false,
                direct_probe_after_ms: None,
                last_seen_at: Some(100),
                tx_bytes: 0,
                rx_bytes: 0,
                error: None,
            },
            MeshPeerStatus {
                pubkey: routed_roster_peer,
                connected: true,
                endpoint_npub: "npub1routed".to_string(),
                transport_addr: None,
                transport_type: None,
                srtt_ms: Some(8),
                link_packets_sent: 0,
                link_packets_recv: 0,
                link_bytes_sent: 0,
                link_bytes_recv: 0,
                direct_probe_pending: false,
                direct_probe_after_ms: None,
                last_seen_at: Some(100),
                tx_bytes: 0,
                rx_bytes: 0,
                error: None,
            },
            MeshPeerStatus {
                pubkey: other_peer,
                connected: true,
                endpoint_npub: "npub1other".to_string(),
                transport_addr: Some("203.0.113.9:9000".to_string()),
                transport_type: Some("udp".to_string()),
                srtt_ms: Some(13),
                link_packets_sent: 0,
                link_packets_recv: 0,
                link_bytes_sent: 0,
                link_bytes_recv: 0,
                direct_probe_pending: false,
                direct_probe_after_ms: None,
                last_seen_at: Some(100),
                tx_bytes: 0,
                rx_bytes: 0,
                error: None,
            },
        ],
        &[],
        &std::collections::HashMap::new(),
        "VPN on",
        &nostr_vpn_core::diagnostics::NetworkSummary::default(),
        &nostr_vpn_core::diagnostics::PortMappingStatus::default(),
    );

    assert_eq!(state.connected_peer_count, 2);
    assert_eq!(state.fips_direct_roster_peer_count, 1);
    assert_eq!(state.fips_other_peer_count, 1);
}

#[test]
fn daemon_runtime_state_marks_peers_unreachable_when_vpn_is_off() {
    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    let peer_pubkey = Keys::generate().public_key().to_hex();
    config.networks[0].participants = vec![peer_pubkey.clone()];
    let tunnel_runtime = crate::CliTunnelRuntime::new("utun100");
    let peer_status = MeshPeerStatus {
        pubkey: peer_pubkey,
        connected: true,
        endpoint_npub: "npub1endpoint".to_string(),
        transport_addr: Some("127.0.0.1:9000".to_string()),
        transport_type: Some("loopback".to_string()),
        srtt_ms: Some(3),
        link_packets_sent: 10,
        link_packets_recv: 11,
        link_bytes_sent: 120,
        link_bytes_recv: 130,
        direct_probe_pending: false,
        direct_probe_after_ms: None,
        last_seen_at: Some(100),
        tx_bytes: 200,
        rx_bytes: 300,
        error: None,
    };

    let state = crate::build_daemon_runtime_state(
        &config,
        false,
        false,
        1,
        &tunnel_runtime,
        &[peer_status],
        &[],
        &std::collections::HashMap::new(),
        "Paused",
        &nostr_vpn_core::diagnostics::NetworkSummary::default(),
        &nostr_vpn_core::diagnostics::PortMappingStatus::default(),
    );

    assert!(!state.vpn_active);
    assert!(!state.vpn_enabled);
    assert_eq!(state.connected_peer_count, 0);
    assert!(!state.mesh_ready);
    assert_eq!(state.peers.len(), 1);
    assert!(!state.peers[0].reachable);
    assert!(state.peers[0].runtime_endpoint.is_none());
}

#[test]
fn macos_route_delete_error_is_absent_matches_missing_route_output() {
    assert!(macos_route_delete_error_is_absent(
        "command failed: route -n delete -ifscope utun100 default\nstdout: not in table\nstderr:"
    ));
    assert!(macos_route_delete_error_is_absent(
        "command failed: route -n delete -ifscope utun100 -net 0.0.0.0/1\nstdout: route: bad interface name\nstderr:"
    ));
    assert!(macos_route_delete_error_is_absent(
        "command failed: route -n delete -host 203.0.113.8\nstdout:\nstderr: route: writing to routing socket: No such process"
    ));
    assert!(!macos_route_delete_error_is_absent(
        "command failed: route -n delete -host 203.0.113.8\nstdout:\nstderr: permission denied"
    ));
}

#[test]
fn macos_default_routes_from_netstat_finds_underlay_and_utun_routes() {
    let routes = macos_default_routes_from_netstat(
        "Routing tables\n\
Internet:\n\
Destination        Gateway            Flags               Netif Expire\n\
default            192.168.64.1       UGScg                 en0\n\
default            link#13            UCSIg               utun5\n\
default            link#26            UCSIg           bridge100      !\n",
    );

    assert_eq!(
        routes,
        vec![
            crate::MacosRouteSpec {
                gateway: Some("192.168.64.1".to_string()),
                interface: "en0".to_string(),
            },
            crate::MacosRouteSpec {
                gateway: None,
                interface: "utun5".to_string(),
            },
            crate::MacosRouteSpec {
                gateway: None,
                interface: "bridge100".to_string(),
            },
        ]
    );

    assert_eq!(
        macos_underlay_default_route_from_routes(&routes),
        Some(crate::MacosRouteSpec {
            gateway: Some("192.168.64.1".to_string()),
            interface: "en0".to_string(),
        })
    );
}

#[test]
fn macos_split_defaults_are_detected_from_netstat() {
    assert!(crate::macos_network::macos_has_tunnel_split_default_routes(
        "Routing tables\n\
Internet:\n\
Destination        Gateway            Flags               Netif Expire\n\
0/1                link#13            UCS                 utun5\n\
128/1              link#13            UCS                 utun5\n\
"
    ));
    assert!(
        !crate::macos_network::macos_has_tunnel_split_default_routes(
            "Routing tables\n\
Internet:\n\
Destination        Gateway            Flags               Netif Expire\n\
default            192.168.64.1       UGScg                 en0\n\
"
        )
    );
}

#[test]
fn macos_underlay_default_route_detection_requires_real_underlay_route() {
    assert!(crate::macos_network::macos_has_underlay_default_route(
        "Routing tables\n\
Internet:\n\
Destination        Gateway            Flags               Netif Expire\n\
default            192.168.64.1       UGScg                 en0\n\
0/1                link#13            UCS                 utun5\n\
128/1              link#13            UCS                 utun5\n\
"
    ));
    assert!(!crate::macos_network::macos_has_underlay_default_route(
        "Routing tables\n\
Internet:\n\
Destination        Gateway            Flags               Netif Expire\n\
0/1                link#13            UCS                 utun5\n\
128/1              link#13            UCS                 utun5\n\
"
    ));
}

#[test]
fn macos_tunnel_default_route_targets_use_split_defaults() {
    assert_eq!(
        crate::macos_network::macos_tunnel_default_route_targets(),
        &["0.0.0.0/1", "128.0.0.0/1"]
    );
}

#[test]
fn macos_gateway_route_args_install_global_host_routes() {
    assert_eq!(
        crate::macos_network::macos_gateway_route_args_for_test(
            "add",
            "65.109.48.91/32",
            "192.168.64.1",
        ),
        vec![
            "-n".to_string(),
            "add".to_string(),
            "-host".to_string(),
            "65.109.48.91".to_string(),
            "192.168.64.1".to_string(),
        ]
    );
    assert_eq!(
        crate::macos_network::macos_gateway_route_args_for_test(
            "change",
            "0.0.0.0/0",
            "192.168.64.1",
        ),
        vec![
            "-n".to_string(),
            "change".to_string(),
            "default".to_string(),
            "192.168.64.1".to_string(),
        ]
    );
}

#[test]
fn macos_interface_names_from_ifconfig_list_parses_interfaces() {
    assert_eq!(
        crate::macos_network::macos_interface_names_from_ifconfig_list(
            "lo0 gif0 stf0 anpi0 en0 en1 utun0 utun100\n"
        ),
        vec![
            "lo0", "gif0", "stf0", "anpi0", "en0", "en1", "utun0", "utun100"
        ]
    );
}

#[test]
fn macos_ipconfig_router_from_output_parses_ip_and_ip_mult_formats() {
    assert_eq!(
        crate::macos_network::macos_ipconfig_router_from_output("router (ip): 192.168.64.1\n"),
        Some("192.168.64.1".parse().unwrap())
    );
    assert_eq!(
        crate::macos_network::macos_ipconfig_router_from_output(
            "router (ip_mult): {192.168.64.1}\n"
        ),
        Some("192.168.64.1".parse().unwrap())
    );
}

#[test]
fn macos_ifconfig_has_ipv4_matches_exact_interface_address() {
    let output = "utun5: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1380\n\
\tinet 10.44.10.23 --> 10.44.10.23 netmask 0xffffffff\n\
\tinet6 fe80::1%utun5 prefixlen 64 scopeid 0x8\n";

    assert!(macos_ifconfig_has_ipv4(
        output,
        Ipv4Addr::new(10, 44, 10, 23)
    ));
    assert!(!macos_ifconfig_has_ipv4(
        output,
        Ipv4Addr::new(10, 44, 10, 24)
    ));
}

#[test]
fn daemon_runtime_state_tracks_live_endpoint_and_listen_port() {
    let mut config = AppConfig::generated();
    config.node.endpoint = "198.51.100.10:51820".to_string();
    let mut tunnel_runtime = crate::CliTunnelRuntime::new("utun100");
    tunnel_runtime.active_listen_port = Some(53083);
    let state = crate::build_daemon_runtime_state(
        &config,
        true,
        true,
        0,
        &tunnel_runtime,
        &[],
        &[],
        &std::collections::HashMap::new(),
        "Connected",
        &nostr_vpn_core::diagnostics::NetworkSummary::default(),
        &nostr_vpn_core::diagnostics::PortMappingStatus::default(),
    );
    let daemon = crate::DaemonStatus {
        running: true,
        pid: Some(1),
        pid_file: std::path::PathBuf::from("/tmp/nvpn.pid"),
        log_file: std::path::PathBuf::from("/tmp/nvpn.log"),
        state_file: std::path::PathBuf::from("/tmp/nvpn.state.json"),
        state: Some(state.clone()),
    };

    assert_eq!(state.listen_port, 53083);
    let expected_endpoint = crate::local_signal_endpoint(&config, 53083);
    assert_eq!(state.local_endpoint, expected_endpoint);
    // advertised_endpoint now mirrors local_endpoint — fips-core owns
    // public-endpoint discovery and advertising.
    assert_eq!(state.advertised_endpoint, expected_endpoint);
    assert_eq!(crate::status_endpoint(&config, &daemon), expected_endpoint);
    assert_eq!(crate::status_listen_port(&config, &daemon), 53083);
}

#[test]
fn daemon_pid_scan_matches_processes_for_config() {
    let config_path = Path::new("/Users/example/Library/Application Support/nvpn/config.toml");
    let ps = "  42063 /Applications/Nostr VPN.app/Contents/MacOS/nvpn daemon --config /Users/example/Library/Application Support/nvpn/config.toml --iface utun100\n\
              97597 /Applications/Nostr VPN.app/Contents/MacOS/nvpn daemon --config /Users/example/Library/Application Support/nvpn/config.toml --iface utun100\n\
              55555 /Applications/Nostr VPN.app/Contents/MacOS/nvpn daemon --config /tmp/other.toml --iface utun100\n";
    let pids = daemon_pids_from_ps_output(ps, config_path);
    assert_eq!(pids, vec![42063, 97597]);
}

#[test]
fn daemon_pid_scan_ignores_exiting_processes_for_config() {
    let config_path = Path::new("/Users/example/Library/Application Support/nvpn/config.toml");
    let ps = "  42063 UNE /Applications/Nostr VPN.app/Contents/MacOS/nvpn daemon --config /Users/example/Library/Application Support/nvpn/config.toml --iface utun100\n\
              97597 Ss /Applications/Nostr VPN.app/Contents/MacOS/nvpn daemon --config /Users/example/Library/Application Support/nvpn/config.toml --iface utun100\n";

    let pids = daemon_pids_from_ps_output(ps, config_path);

    assert_eq!(pids, vec![97597]);
}

#[test]
fn daemon_pid_scan_matches_macos_service_helper_path() {
    // /Library/PrivilegedHelperTools/to.nostrvpn.nvpn is the stable
    // service-owned path the launchd plist points at. Its basename ends with
    // `.nvpn`, not `/nvpn`, so the original heuristic missed it and `nvpn
    // status` reported `daemon.running: false` even when the launchd daemon
    // was healthy.
    let config_path = Path::new("/Users/example/Library/Application Support/nvpn/config.toml");
    let ps = "  2853 Ss /Library/PrivilegedHelperTools/to.nostrvpn.nvpn daemon --service --config /Users/example/Library/Application Support/nvpn/config.toml --iface utun --mesh-refresh-interval-secs 20\n";
    let pids = daemon_pids_from_ps_output(ps, config_path);
    assert_eq!(pids, vec![2853]);
}

#[test]
fn daemon_pid_scan_matches_macos_service_helper_with_config_suffix() {
    let config_path = Path::new("/tmp/custom/config.toml");
    let ps = "  3001 Ss /Library/PrivilegedHelperTools/to.nostrvpn.nvpn.tmp_custom daemon --service --config /tmp/custom/config.toml\n";
    let pids = daemon_pids_from_ps_output(ps, config_path);
    assert_eq!(pids, vec![3001]);
}

#[test]
fn daemon_pid_scan_ignores_shell_wrappers_that_mention_nvpn_daemon() {
    let config_path = Path::new("/root/.config/nvpn/config.toml");
    let ps = "2433278 bash -c set -e; nohup /root/nostr-vpn-current/target/debug/nvpn daemon --config /root/.config/nvpn/config.toml --iface utun100 >/root/.config/nvpn/launch.out 2>&1 </dev/null & sleep 5\n\
2433301 /root/nostr-vpn-current/target/debug/nvpn daemon --config /root/.config/nvpn/config.toml --iface utun100 --mesh-refresh-interval-secs 20\n";

    let pids = daemon_pids_from_ps_output(ps, config_path);

    assert_eq!(pids, vec![2433301]);
}

#[test]
fn windows_daemon_pid_scan_matches_processes_for_config() {
    let config_path = Path::new("C:\\Users\\Example\\AppData\\Roaming\\nvpn\\config.toml");
    let cim_json = r#"[{"ProcessId":42063,"CommandLine":"\"C:\\Program Files\\Nostr VPN\\nvpn.exe\" daemon --config \"C:\\Users\\Example\\AppData\\Roaming\\nvpn\\config.toml\" --iface NostrVPN"},{"ProcessId":97597,"CommandLine":"\"C:\\Program Files\\Nostr VPN\\nvpn.exe\" daemon --config \"C:\\Users\\Example\\AppData\\Roaming\\nvpn\\config.toml\" --iface NostrVPN"},{"ProcessId":55555,"CommandLine":"\"C:\\Program Files\\Nostr VPN\\nvpn.exe\" daemon --config \"C:\\temp\\other.toml\" --iface NostrVPN"}]"#;
    let pids = crate::daemon_pids_from_windows_cim_json(cim_json, config_path);
    assert_eq!(pids, vec![42063, 97597]);
}

#[test]
fn windows_daemon_pid_scan_accepts_single_process_object() {
    let config_path = Path::new("C:\\Users\\Example\\AppData\\Roaming\\nvpn\\config.toml");
    let cim_json = r#"{"ProcessId":42063,"CommandLine":"\"C:\\Program Files\\Nostr VPN\\nvpn.exe\" daemon --config \"C:\\Users\\Example\\AppData\\Roaming\\nvpn\\config.toml\" --iface NostrVPN"}"#;
    let pids = crate::daemon_pids_from_windows_cim_json(cim_json, config_path);
    assert_eq!(pids, vec![42063]);
}

#[test]
fn windows_service_bin_path_runs_hidden_service_daemon_with_same_config() {
    let executable = Path::new("C:\\Program Files\\Nostr VPN\\nvpn.exe");
    let config_path = Path::new("C:\\Users\\Example\\AppData\\Roaming\\nvpn\\config.toml");

    let command = windows_service_bin_path(executable, config_path, "nvpn", 20);

    assert!(command.starts_with("\"C:\\Program Files\\Nostr VPN\\nvpn.exe\""));
    assert!(command.contains(" daemon "));
    assert!(command.contains(" --service "));
    assert!(command.contains(" --config "));
    assert!(command.contains("\"C:\\Users\\Example\\AppData\\Roaming\\nvpn\\config.toml\""));
    assert!(command.contains(" --iface \"nvpn\""));
    assert!(command.contains(" --mesh-refresh-interval-secs 20"));
}

#[test]
fn windows_tasklist_pid_parser_extracts_pids() {
    let output = "\"nvpn.exe\",\"9564\",\"Services\",\"0\",\"172,248 K\"\n\"nvpn.exe\",\"3496\",\"Console\",\"1\",\"19,472 K\"\n";
    assert_eq!(crate::tasklist_pids_from_output(output), vec![3496, 9564]);
}

#[test]
fn windows_tasklist_pid_parser_ignores_no_tasks_message() {
    let output = "INFO: No tasks are running which match the specified criteria.\r\n";
    assert!(crate::tasklist_pids_from_output(output).is_empty());
}

#[test]
fn recent_windows_daemon_pid_candidate_uses_fresh_state_and_single_other_process() {
    let state = DaemonRuntimeState {
        updated_at: 100,
        ..Default::default()
    };
    assert_eq!(
        crate::recent_windows_daemon_pid_candidate(Some(&state), 42, &[42, 9564], 103),
        Some(9564)
    );
    assert_eq!(
        crate::recent_windows_daemon_pid_candidate(Some(&state), 42, &[42, 9564], 106),
        None
    );
    assert_eq!(
        crate::recent_windows_daemon_pid_candidate(Some(&state), 42, &[42, 9564, 97597], 103),
        None
    );
    assert_eq!(
        crate::recent_windows_daemon_pid_candidate(None, 42, &[42, 9564], 103),
        None
    );
}

#[test]
fn visible_daemon_state_for_status_keeps_state_while_running() {
    let state = DaemonRuntimeState {
        vpn_active: true,
        ..Default::default()
    };

    let visible = crate::visible_daemon_state_for_status(true, Some(&state));
    assert!(visible.is_some());
    assert!(visible.expect("visible state").vpn_active);
}

#[test]
fn visible_daemon_state_for_status_hides_state_when_stopped() {
    let state = DaemonRuntimeState {
        vpn_active: true,
        ..Default::default()
    };

    assert!(crate::visible_daemon_state_for_status(false, Some(&state)).is_none());
}

#[test]
fn daemon_reload_config_uses_reloaded_network_id() {
    let mut app = AppConfig::generated();
    activate_first_network(&mut app);
    app.set_active_network_id("mesh-home")
        .expect("set initial network id");
    app.networks[0].participants = vec!["11".repeat(32)];
    let initial_network_id = app.effective_network_id();

    app.set_active_network_id("mesh-work")
        .expect("set reloaded network id");
    app.networks[0].participants = vec!["22".repeat(32)];
    let reloaded_network_id = app.effective_network_id();
    assert_ne!(initial_network_id, reloaded_network_id);

    let reload = build_daemon_reload_config(app, reloaded_network_id.clone());

    assert_eq!(reload.network_id, reloaded_network_id);
}

#[test]
fn daemon_pid_scan_excludes_current_pid_when_filtering_duplicates() {
    let config_path = Path::new("/Users/example/Library/Application Support/nvpn/config.toml");
    let ps = "  42063 /Applications/Nostr VPN.app/Contents/MacOS/nvpn daemon --config /Users/example/Library/Application Support/nvpn/config.toml --iface utun100\n\
              97597 /Applications/Nostr VPN.app/Contents/MacOS/nvpn daemon --config /Users/example/Library/Application Support/nvpn/config.toml --iface utun100\n";
    let mut pids = daemon_pids_from_ps_output(ps, config_path);
    pids.retain(|pid| *pid != 97597);
    assert_eq!(pids, vec![42063]);
}

#[test]
fn unix_process_stat_treats_exiting_and_dead_states_as_not_running() {
    assert!(crate::unix_process_stat_counts_as_running("Ss"));
    assert!(!crate::unix_process_stat_counts_as_running("UNE"));
    assert!(!crate::unix_process_stat_counts_as_running("Z"));
}

#[test]
fn default_cli_install_path_uses_nvpn_filename() {
    let path = default_cli_install_path();
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(if cfg!(target_os = "windows") {
            "nvpn.exe"
        } else {
            "nvpn"
        })
    );
}

#[test]
fn default_tunnel_iface_matches_platform() {
    let iface = crate::default_tunnel_iface();
    assert_eq!(
        iface,
        if cfg!(target_os = "windows") {
            "nvpn"
        } else if cfg!(target_os = "macos") {
            "utun"
        } else {
            "utun100"
        }
    );
}

#[test]
fn install_cli_and_uninstall_cli_roundtrip_for_custom_path() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-install-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let target = dir.join("nvpn");

    install_cli(InstallCliArgs {
        path: Some(target.clone()),
        force: false,
    })
    .expect("install custom cli target");
    assert!(target.exists(), "installed target should exist");

    let duplicate = install_cli(InstallCliArgs {
        path: Some(target.clone()),
        force: false,
    });
    assert!(duplicate.is_err(), "install without --force should fail");

    install_cli(InstallCliArgs {
        path: Some(target.clone()),
        force: true,
    })
    .expect("force reinstall custom cli target");

    uninstall_cli(UninstallCliArgs {
        path: Some(target.clone()),
    })
    .expect("uninstall custom cli target");
    assert!(!target.exists(), "uninstall should remove target");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn apply_config_file_writes_target_config() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-apply-config-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");

    let source = dir.join("source.toml");
    let target = dir.join("target.toml");
    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    config.node_name = "windows-box".to_string();
    config.networks[0].participants = vec!["ab".repeat(32)];
    config.save(&source).expect("save source config");

    apply_config_file(&source, &target).expect("apply config should succeed");

    let loaded = AppConfig::load(&target).expect("load target config");
    assert_eq!(loaded.node_name, "windows-box");
    assert_eq!(loaded.participant_pubkeys_hex(), vec!["ab".repeat(32)]);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn load_or_default_config_migrates_plaintext_config_secrets() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-load-config-secrets-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("config.toml");
    let mut config = AppConfig::generated();
    config.wireguard_exit.private_key = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=".to_string();
    config.wireguard_exit.peer_public_key =
        "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=".to_string();
    config.wireguard_exit.peer_preshared_key =
        "AwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwM=".to_string();
    let nostr_secret = config.nostr.secret_key.clone();
    let wireguard_private_key = config.wireguard_exit.private_key.clone();
    let wireguard_peer_preshared_key = config.wireguard_exit.peer_preshared_key.clone();
    fs::write(
        &path,
        config.plaintext_toml().expect("encode plaintext config"),
    )
    .expect("write plaintext config");

    let loaded = load_or_default_config(&path).expect("load config");
    let raw = fs::read_to_string(&path).expect("read migrated config");
    AppConfig::delete_persisted_secrets_for_path(&path).expect("delete migrated secrets");

    assert_eq!(loaded.nostr.secret_key, nostr_secret);
    assert_eq!(loaded.wireguard_exit.private_key, wireguard_private_key);
    assert_eq!(
        loaded.wireguard_exit.peer_preshared_key,
        wireguard_peer_preshared_key
    );
    assert!(!raw.contains(&nostr_secret));
    assert!(!raw.contains(&wireguard_private_key));
    assert!(!raw.contains(&wireguard_peer_preshared_key));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn stage_daemon_config_apply_writes_staged_file() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-stage-config-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");

    let source = dir.join("source.toml");
    let target = dir.join("config.toml");
    let mut config = AppConfig::generated();
    config.node_name = "staged-node".to_string();
    config.save(&source).expect("save source config");

    stage_daemon_config_apply(&target, &source).expect("stage config should succeed");

    let staged = daemon_staged_config_file_path(&target);
    let loaded = AppConfig::load(&staged).expect("load staged config");
    assert_eq!(loaded.node_name, "staged-node");

    AppConfig::delete_persisted_secrets_for_path(&source).expect("delete source secrets");
    AppConfig::delete_persisted_secrets_for_path(&staged).expect("delete staged secrets");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn update_daemon_config_from_staged_request_replaces_target_and_cleans_up() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-stage-apply-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");

    let source = dir.join("source.toml");
    let target = dir.join("config.toml");
    let mut source_config = AppConfig::generated();
    source_config.node_name = "service-owned".to_string();
    source_config.save(&source).expect("save source config");

    let mut target_config = AppConfig::generated();
    target_config.node_name = "old-name".to_string();
    target_config.save(&target).expect("save target config");

    stage_daemon_config_apply(&target, &source).expect("stage config should succeed");
    update_daemon_config_from_staged_request(&target).expect("apply staged config");

    let loaded = AppConfig::load(&target).expect("load target config");
    assert_eq!(loaded.node_name, "service-owned");
    assert!(
        !daemon_staged_config_file_path(&target).exists(),
        "staged config should be cleaned up"
    );

    AppConfig::delete_persisted_secrets_for_path(&source).expect("delete source secrets");
    AppConfig::delete_persisted_secrets_for_path(&target).expect("delete target secrets");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn kill_error_fallback_matcher_detects_permission_denied() {
    assert!(kill_error_requires_control_fallback(
        "kill -TERM 123 failed\nstderr: Operation not permitted"
    ));
    assert!(kill_error_requires_control_fallback(
        "kill -TERM 123 failed\nstderr: permission denied"
    ));
    assert!(!kill_error_requires_control_fallback(
        "kill -TERM 123 failed\nstderr: no such process"
    ));
}

#[test]
fn daemon_control_stop_request_roundtrip() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-control-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config = dir.join("config.toml");
    fs::write(&config, "node_name = \"test\"").expect("write config");

    request_daemon_stop(&config).expect("write stop request");
    assert!(
        take_daemon_control_request(&config) == Some(crate::DaemonControlRequest::Stop),
        "daemon should read stop request"
    );
    request_daemon_reload(&config).expect("write reload request");
    assert!(
        take_daemon_control_request(&config) == Some(crate::DaemonControlRequest::Reload),
        "daemon should read reload request"
    );
    control_daemon_request_for_test(&config, crate::DaemonControlRequest::Pause);
    assert!(
        take_daemon_control_request(&config) == Some(crate::DaemonControlRequest::Pause),
        "daemon should read pause request"
    );
    control_daemon_request_for_test(&config, crate::DaemonControlRequest::Resume);
    assert!(
        take_daemon_control_request(&config) == Some(crate::DaemonControlRequest::Resume),
        "daemon should read resume request"
    );
    let _ = fs::remove_file(daemon_control_file_path(&config));
    assert!(
        take_daemon_control_request(&config).is_none(),
        "without control file there should be no stop request"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_control_timeout_errors_use_generic_service_wording() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-control-timeout-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config = dir.join("config.toml");
    fs::write(&config, "node_name = \"test\"").expect("write config");

    let ack_error = crate::wait_for_daemon_control_ack(&config, Duration::from_millis(0))
        .expect_err("ack wait should time out");
    assert!(
        ack_error
            .to_string()
            .contains("background service may be busy or stuck")
    );
    assert!(!ack_error.to_string().contains("newer nvpn binary"));

    let result_error = crate::wait_for_daemon_control_result(
        &config,
        crate::DaemonControlRequest::Reload,
        Duration::from_millis(0),
    )
    .expect_err("result wait should time out");
    assert!(
        result_error
            .to_string()
            .contains("background service may be busy or stuck")
    );
    assert!(!result_error.to_string().contains("newer nvpn binary"));

    let vpn_error = crate::wait_for_daemon_vpn_enabled(&config, true, Duration::from_millis(0))
        .expect_err("vpn wait should time out");
    assert!(
        vpn_error
            .to_string()
            .contains("background service may be busy or stuck")
    );
    assert!(
        !vpn_error
            .to_string()
            .contains("older nvpn daemon binary is still running")
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_control_wait_timeouts_allow_longer_mac_recovery_windows() {
    assert_eq!(
        crate::daemon_control_ack_timeout(crate::DaemonControlRequest::Reload),
        Duration::from_secs(10)
    );
    assert_eq!(
        crate::daemon_control_result_timeout(crate::DaemonControlRequest::Reload),
        Duration::from_secs(15)
    );
    assert_eq!(
        crate::daemon_control_vpn_transition_timeout(crate::DaemonControlRequest::Reload),
        Duration::ZERO
    );

    if cfg!(target_os = "macos") {
        assert_eq!(
            crate::daemon_control_ack_timeout(crate::DaemonControlRequest::Resume),
            Duration::from_secs(15)
        );
        assert_eq!(
            crate::daemon_control_result_timeout(crate::DaemonControlRequest::Resume),
            Duration::from_secs(30)
        );
        assert_eq!(
            crate::daemon_control_vpn_transition_timeout(crate::DaemonControlRequest::Resume),
            Duration::from_secs(30)
        );
    } else {
        assert_eq!(
            crate::daemon_control_ack_timeout(crate::DaemonControlRequest::Resume),
            Duration::from_secs(10)
        );
        assert_eq!(
            crate::daemon_control_result_timeout(crate::DaemonControlRequest::Resume),
            Duration::from_secs(15)
        );
        assert_eq!(
            crate::daemon_control_vpn_transition_timeout(crate::DaemonControlRequest::Resume),
            Duration::from_secs(2)
        );
    }
}
