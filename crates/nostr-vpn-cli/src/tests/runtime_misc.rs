use crate::*;
#[cfg(feature = "embedded-fips")]
use nostr_sdk::prelude::{Keys, ToBech32};
use std::collections::HashSet;
#[cfg(feature = "embedded-fips")]
use std::net::Ipv4Addr;
use std::path::Path;
use std::time::{Duration, Instant};

#[test]
fn daemon_vpn_requires_remote_participants_to_be_active() {
    assert!(!daemon_vpn_active(true, 0));
    assert!(daemon_vpn_active(true, 1));
    assert!(!daemon_vpn_active(false, 1));
}

#[test]
fn daemon_vpn_idle_status_distinguishes_waiting_from_paused() {
    assert_eq!(
        daemon_vpn_idle_status(true, 0, false),
        crate::WAITING_FOR_PARTICIPANTS_STATUS
    );
    assert_eq!(
        daemon_vpn_idle_status(false, 0, true),
        "Listening for join requests"
    );
    assert_eq!(daemon_vpn_idle_status(false, 0, false), "Paused");
    assert_eq!(daemon_vpn_idle_status(true, 2, false), "Paused");
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_roster_publish_keeps_disconnected_recipients_pending() {
    let connected = HashSet::from(["alice".to_string()]);
    let recipients = vec!["alice".to_string(), "bob".to_string()];

    let (ready, pending) = split_ready_fips_roster_recipients(recipients, &connected);

    assert_eq!(ready, vec!["alice".to_string()]);
    assert_eq!(pending, HashSet::from(["bob".to_string()]));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_include_configured_and_lan_candidates() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "89.27.103.157:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = true;

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(192, 168, 50, 10)]);
    let addrs = hints.into_iter().map(|hint| hint.addr).collect::<Vec<_>>();

    assert_eq!(
        addrs,
        vec![
            "192.168.50.10:51820".to_string(),
            "89.27.103.157:51820".to_string(),
        ]
    );
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_do_not_share_lan_when_disabled() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "127.0.0.1:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = false;

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(192, 168, 50, 10)]);

    assert!(hints.is_empty());
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_do_not_share_loopback_when_lan_enabled() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "127.0.0.1:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = true;

    let hints = local_fips_endpoint_hints(&app, Vec::new());

    assert!(hints.is_empty());
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_keep_dns_endpoint_and_listen_port() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "peer.example.com:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = false;

    let hints = local_fips_endpoint_hints(&app, Vec::new());

    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].addr, "peer.example.com:51820");
}

#[cfg(feature = "embedded-fips")]
#[test]
fn runtime_signal_ipv4_candidates_keep_local_non_tunnel_addresses() {
    let candidates =
        runtime_signal_ipv4_candidates(Some(Ipv4Addr::new(192, 168, 50, 10)), "10.44.1.1/32");

    assert!(candidates.contains(&Ipv4Addr::new(192, 168, 50, 10)));
    assert!(!candidates.contains(&Ipv4Addr::new(10, 44, 1, 1)));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn endpoint_hint_recipients_are_active_participants_only() {
    let own = Keys::generate();
    let peer = Keys::generate();
    let admin = Keys::generate();
    let own_pubkey = own.public_key().to_hex();
    let peer_pubkey = peer.public_key().to_hex();
    let admin_pubkey = admin.public_key().to_hex();
    let mut app = AppConfig::generated();
    app.nostr.secret_key = own.secret_key().to_bech32().expect("own nsec");
    app.nostr.public_key = own_pubkey.clone();
    app.networks[0].participants = vec![own_pubkey.clone(), peer_pubkey.clone()];
    app.networks[0].admins = vec![admin_pubkey.clone()];

    let recipients = desired_fips_endpoint_hint_recipients(&app);

    assert_eq!(recipients, HashSet::from([peer_pubkey]));
    assert!(!recipients.contains(&own_pubkey));
    assert!(!recipients.contains(&admin_pubkey));
}

#[test]
fn parse_nonzero_pid_rejects_zero_and_invalid_values() {
    assert_eq!(parse_nonzero_pid("4242"), Some(4242));
    assert_eq!(parse_nonzero_pid("0"), None);
    assert_eq!(parse_nonzero_pid("not-a-number"), None);
}

#[test]
fn wall_time_jump_detection_flags_sleep_resume_after_threshold() {
    let observed_at = Instant::now();
    assert!(!wall_time_jump_detected(
        0,
        1_000,
        observed_at,
        observed_at,
        MAJOR_LINK_CHANGE_TIME_JUMP_SECS
    ));
    assert!(!wall_time_jump_detected(
        1_000,
        1_000 + MAJOR_LINK_CHANGE_TIME_JUMP_SECS - 1,
        observed_at,
        observed_at + Duration::from_secs(MAJOR_LINK_CHANGE_TIME_JUMP_SECS - 1),
        MAJOR_LINK_CHANGE_TIME_JUMP_SECS,
    ));
    assert!(wall_time_jump_detected(
        1_000,
        1_000 + MAJOR_LINK_CHANGE_TIME_JUMP_SECS,
        observed_at,
        observed_at,
        MAJOR_LINK_CHANGE_TIME_JUMP_SECS,
    ));
}

#[test]
fn wall_time_jump_detection_ignores_busy_loop_delays() {
    let observed_at = Instant::now();
    assert!(!wall_time_jump_detected(
        1_000,
        1_000 + MAJOR_LINK_CHANGE_TIME_JUMP_SECS + 5,
        observed_at,
        observed_at + Duration::from_secs(MAJOR_LINK_CHANGE_TIME_JUMP_SECS + 5),
        MAJOR_LINK_CHANGE_TIME_JUMP_SECS,
    ));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn runtime_exit_node_routes_do_not_advertise_ipv6_default() {
    let mut app = AppConfig::generated();
    app.node.advertise_exit_node = true;

    assert_eq!(runtime_exit_node_default_routes(), vec!["0.0.0.0/0"]);
    assert_eq!(runtime_effective_advertised_routes(&app), vec!["0.0.0.0/0"]);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_underlay_repair_resets_tunnel_runtime() {
    let mut runtime = CliTunnelRuntime::new("utun4");
    runtime.active_listen_port = Some(51820);

    crate::session_runtime::reset_tunnel_runtime_after_macos_underlay_repair(&mut runtime);

    assert!(runtime.active_listen_port.is_none());
}

#[test]
fn macos_connect_privilege_preflight_requires_admin_when_euid_is_not_root() {
    let _guard = crate::macos_euid_override_lock_for_test()
        .lock()
        .expect("macos euid test lock");
    crate::set_macos_euid_override_for_test(Some(501));

    let error = crate::ensure_macos_connect_privileges(Path::new("/tmp/nvpn.toml"))
        .expect_err("non-root macOS preflight should fail");
    let message = error.to_string();
    assert!(message.contains("admin privileges"));
    assert!(message.contains("did you run with sudo?"));
    assert!(message.contains("sudo nvpn start --connect"));
    assert!(message.contains("sudo nvpn service install"));

    crate::set_macos_euid_override_for_test(None);
}

#[test]
fn macos_connect_privilege_preflight_allows_root() {
    let _guard = crate::macos_euid_override_lock_for_test()
        .lock()
        .expect("macos euid test lock");
    crate::set_macos_euid_override_for_test(Some(0));

    crate::ensure_macos_connect_privileges(Path::new("/tmp/nvpn.toml"))
        .expect("root macOS preflight should pass");

    crate::set_macos_euid_override_for_test(None);
}
