use crate::*;
#[cfg(feature = "embedded-fips")]
use nostr_sdk::prelude::{Keys, ToBech32};
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

#[test]
fn fips_private_runtime_active_tolerates_no_active_network() {
    let mut app = AppConfig::generated();
    app.fips_host_tunnel_enabled = false;
    for network in &mut app.networks {
        network.listen_for_join_requests = false;
    }

    assert!(app.active_network_opt().is_none());
    assert!(!fips_private_runtime_active(&app, true, 0));

    app.networks[0].listen_for_join_requests = true;
    assert!(fips_private_runtime_active(&app, false, 0));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_roster_publish_attempts_disconnected_recipients() {
    let recipients = vec!["alice".to_string(), "bob".to_string()];

    let (ready, pending) = split_ready_fips_roster_recipients(recipients.clone());

    assert_eq!(ready, recipients);
    assert!(pending.is_empty());
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
fn local_fips_endpoint_hints_keep_configured_lan_when_lan_discovery_disabled() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "192.168.50.22:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = false;

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(192, 168, 50, 10)]);

    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].addr, "192.168.50.22:51820");
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_do_not_share_cgnat_candidates() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "127.0.0.1:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = true;

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(100, 120, 94, 10)]);

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
fn local_fips_endpoint_hints_do_not_share_tunnel_endpoint() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "10.44.1.1:1111".to_string();
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
    assert!(!candidates.contains(&Ipv4Addr::new(100, 120, 94, 10)));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn runtime_signal_ipv4_candidates_drop_detected_cgnat_address() {
    let candidates =
        runtime_signal_ipv4_candidates(Some(Ipv4Addr::new(100, 120, 94, 10)), "10.44.1.1/32");

    assert!(!candidates.contains(&Ipv4Addr::new(100, 120, 94, 10)));
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
    let network_id = app.networks[0].id.clone();
    app.set_network_enabled(&network_id, true)
        .expect("activate first network");
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

#[test]
fn daemon_network_refresh_cadence_keeps_link_changes_low_latency() {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        assert_eq!(DAEMON_NETWORK_REFRESH_INTERVAL_SECS, 15);
        const {
            assert!(DAEMON_NETWORK_EVENT_DEBOUNCE_MILLIS <= 1_000);
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    assert_eq!(DAEMON_NETWORK_REFRESH_INTERVAL_SECS, 1);
}

#[test]
fn macos_underlay_route_check_throttles_route_event_storms() {
    assert_eq!(MACOS_UNDERLAY_ROUTE_CHECK_INTERVAL_SECS, 5);

    let start = Instant::now();
    let mut last_check_at = start;

    assert!(!macos_underlay_route_check_due(
        &mut last_check_at,
        false,
        false,
        start + Duration::from_secs(1),
    ));
    assert_eq!(last_check_at, start);

    assert!(macos_underlay_route_check_due(
        &mut last_check_at,
        false,
        false,
        start + Duration::from_secs(5),
    ));
    assert_eq!(last_check_at, start + Duration::from_secs(5));

    assert!(macos_underlay_route_check_due(
        &mut last_check_at,
        true,
        false,
        start + Duration::from_secs(6),
    ));
    assert_eq!(last_check_at, start + Duration::from_secs(6));

    assert!(macos_underlay_route_check_due(
        &mut last_check_at,
        false,
        true,
        start + Duration::from_secs(7),
    ));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_link_events_restart_endpoint_for_major_link_changes() {
    assert_eq!(
        fips_link_event_refresh(true, false, false, false),
        FipsLinkEventRefresh::RestartEndpoint
    );
    assert_eq!(
        fips_link_event_refresh(false, false, true, false),
        FipsLinkEventRefresh::RestartEndpoint
    );
    assert_eq!(
        fips_link_event_refresh(false, false, false, true),
        FipsLinkEventRefresh::RestartEndpoint
    );
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_link_events_restart_endpoint_for_endpoint_only_changes() {
    assert_eq!(
        fips_link_event_refresh(false, true, false, false),
        FipsLinkEventRefresh::RestartEndpoint
    );
    assert_eq!(
        fips_link_event_refresh(false, false, false, false),
        FipsLinkEventRefresh::None
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn runtime_exit_node_routes_do_not_advertise_ipv6_default() {
    let mut app = AppConfig::generated();
    app.node.advertise_exit_node = true;

    assert_eq!(runtime_exit_node_default_routes(), vec!["0.0.0.0/0"]);
    assert_eq!(runtime_effective_advertised_routes(&app), vec!["0.0.0.0/0"]);
}

#[test]
fn legacy_macos_exit_cleanup_leaves_global_ipv4_forwarding_alone() {
    let mut app = AppConfig::generated();
    app.node.advertise_exit_node = true;

    let plan = legacy_macos_exit_cleanup_plan(&runtime_effective_advertised_routes(&app));

    assert!(plan.cleanup_pf_nat);
    assert!(!plan.restore_ipv4_forwarding);
}

#[test]
fn macos_exit_node_pf_rules_are_scoped_to_tunnel_source_and_outbound_iface() {
    let rules = crate::macos_network::macos_exit_node_pf_rules("utun42", "en0", "10.44.0.0/16");

    assert_eq!(
        rules,
        concat!(
            "nat on en0 inet from 10.44.0.0/16 to any -> (en0)\n",
            "pass in quick on utun42 inet from 10.44.0.0/16 to any keep state\n",
            "pass out quick on en0 inet from 10.44.0.0/16 to any keep state\n",
        )
    );
    assert!(!rules.contains("net.inet.ip.forwarding"));
    assert!(!rules.contains("pass in quick on en0"));
}

#[test]
fn macos_exit_node_cleanup_flushes_only_nvpn_anchor() {
    assert_eq!(
        crate::macos_network::macos_pf_anchor_flush_args(),
        vec!["-a", "com.apple/to.nostrvpn/exit", "-F", "all"]
    );
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
