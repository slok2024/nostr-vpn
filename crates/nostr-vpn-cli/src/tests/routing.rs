use crate::*;

#[path = "routing_linux_helpers.rs"]
mod routing_linux_helpers;
#[path = "routing_macos_routes.rs"]
mod routing_macos_routes;
#[path = "routing_peer_paths.rs"]
mod routing_peer_paths;
#[path = "routing_planning.rs"]
mod routing_planning;
#[path = "routing_platform_helpers.rs"]
mod routing_platform_helpers;
#[path = "routing_runtime_endpoints.rs"]
mod routing_runtime_endpoints;

#[test]
fn utun_candidates_expand_for_default_style_names() {
    let candidates = utun_interface_candidates("utun100");
    if cfg!(target_os = "macos") {
        assert_eq!(candidates[0], "utun");
        assert_eq!(candidates[1], "utun100");
        assert_eq!(candidates[2], "utun101");
        assert_eq!(candidates[16], "utun115");
    } else {
        assert_eq!(candidates.len(), 16);
        assert_eq!(candidates[0], "utun100");
        assert_eq!(candidates[1], "utun101");
        assert_eq!(candidates[15], "utun115");
    }
}

#[test]
fn utun_candidates_keep_custom_iface_as_is() {
    let candidates = utun_interface_candidates("wg0");
    assert_eq!(candidates, vec!["wg0".to_string()]);
}

#[test]
fn uapi_addr_in_use_matcher_detects_common_errnos() {
    assert!(is_uapi_addr_in_use_error("uapi set failed: errno=48"));
    assert!(is_uapi_addr_in_use_error("uapi set failed: errno=98"));
    assert!(!is_uapi_addr_in_use_error("uapi set failed: errno=1"));
}

#[test]
fn endpoint_listen_port_rewrite_updates_socket_port() {
    assert_eq!(
        endpoint_with_listen_port("192.168.1.10:51820", 52000),
        "192.168.1.10:52000"
    );
    assert_eq!(
        endpoint_with_listen_port("[2001:db8::1]:51820", 52000),
        "[2001:db8::1]:52000"
    );
    assert_eq!(
        endpoint_with_listen_port("not-a-socket", 52000),
        "not-a-socket"
    );
}

#[test]
fn public_endpoint_discovery_bind_conflict_matches_discovery_bind_errors() {
    assert!(public_endpoint_discovery_bind_conflict(
        "failed to bind udp stun socket on 0.0.0.0:51820"
    ));
    assert!(public_endpoint_discovery_bind_conflict(
        "failed to bind udp discovery socket on 0.0.0.0:51820"
    ));
    assert!(public_endpoint_discovery_bind_conflict(
        "Address already in use"
    ));
    assert!(!public_endpoint_discovery_bind_conflict(
        "timed out waiting for stun response"
    ));
}

#[test]
fn public_endpoint_discovery_falls_back_to_host_inference_when_port_bind_is_busy() {
    let mut calls = Vec::new();
    let endpoint = discover_public_endpoint_with_bind_fallback(58686, |port| {
        calls.push(port);
        match port {
            58686 => Err(anyhow!("failed to bind udp stun socket on 0.0.0.0:58686")),
            0 => Ok("89.27.103.157:41829".to_string()),
            other => Err(anyhow!("unexpected port {other}")),
        }
    })
    .expect("fallback endpoint");

    assert_eq!(calls, vec![58686, 0]);
    assert_eq!(endpoint, "89.27.103.157:58686");
}

#[test]
fn local_interface_address_for_tunnel_preserves_host_prefix() {
    assert_eq!(
        local_interface_address_for_tunnel("10.44.0.1/32"),
        "10.44.0.1/32"
    );
    assert_eq!(
        local_interface_address_for_tunnel("10.44.0.1"),
        "10.44.0.1/32"
    );
}

#[test]
fn route_targets_for_tunnel_peers_use_peer_allowed_ips() {
    let routes = route_targets_for_tunnel_peers(&[
        TunnelPeer {
            pubkey_hex: "a".repeat(64),
            endpoint: "203.0.113.10:51820".to_string(),
            allowed_ips: vec!["10.44.0.3/32".to_string()],
        },
        TunnelPeer {
            pubkey_hex: "b".repeat(64),
            endpoint: "203.0.113.11:51820".to_string(),
            allowed_ips: vec!["10.44.0.2/32".to_string(), "10.55.0.0/24".to_string()],
        },
        TunnelPeer {
            pubkey_hex: "c".repeat(64),
            endpoint: "203.0.113.12:51820".to_string(),
            allowed_ips: vec!["10.44.0.2/32".to_string()],
        },
    ]);

    assert_eq!(
        routes,
        vec![
            "10.44.0.2/32".to_string(),
            "10.44.0.3/32".to_string(),
            "10.55.0.0/24".to_string(),
        ]
    );
}
