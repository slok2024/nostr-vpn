use std::collections::HashMap;
use std::net::Ipv4Addr;

use crate::*;

use nostr_vpn_core::crypto::generate_keypair;

#[test]
fn route_targets_detect_when_endpoint_bypass_is_required() {
    assert!(!route_targets_require_endpoint_bypass(&[
        "10.44.0.2/32".to_string()
    ]));
    assert!(route_targets_require_endpoint_bypass(&[
        "10.55.0.0/24".to_string()
    ]));
    assert!(route_targets_require_endpoint_bypass(&[
        "0.0.0.0/0".to_string()
    ]));
}

#[test]
fn macos_default_route_can_be_withheld_from_route_targets() {
    let mut routes = vec!["0.0.0.0/0".to_string(), "10.55.0.0/24".to_string()];

    assert!(withhold_macos_default_route(&mut routes));
    assert_eq!(routes, vec!["10.55.0.0/24".to_string()]);
    assert!(!withhold_macos_default_route(&mut routes));
}

#[test]
fn tunnel_runtime_fingerprint_changes_when_route_targets_change() {
    let base = "iface|key|51820|10.44.0.1/32|peer";
    let direct_only = vec!["10.44.0.2/32".to_string()];
    let with_exit = vec!["0.0.0.0/0".to_string(), "10.44.0.2/32".to_string()];

    assert_ne!(
        tunnel_runtime_fingerprint(base, &direct_only),
        tunnel_runtime_fingerprint(base, &with_exit)
    );
}

#[test]
fn stun_host_port_supports_default_and_explicit_ports() {
    assert_eq!(
        stun_host_port("stun:stun.iris.to"),
        Some(("stun.iris.to".to_string(), 3478))
    );
    assert_eq!(
        stun_host_port("stun://198.51.100.30:5349"),
        Some(("198.51.100.30".to_string(), 5349))
    );
    assert_eq!(stun_host_port(""), None);
}

#[test]
fn control_plane_bypass_hosts_include_nat_helpers_and_management_hosts() {
    use netdev::interface::flags::{IFF_POINTOPOINT, IFF_UP};
    use netdev::net::device::NetworkDevice;
    use std::net::IpAddr;

    let mut config = AppConfig::generated();
    config.nostr.relays = vec![
        "wss://203.0.113.10".to_string(),
        "wss://198.51.100.20:444".to_string(),
    ];
    config.nat.stun_servers = vec![
        "stun:198.51.100.30:3478".to_string(),
        "stun://203.0.113.10".to_string(),
        "not-a-stun-url".to_string(),
    ];
    config.nat.reflectors = vec!["192.0.2.40:5000".to_string(), "invalid".to_string()];

    let mut physical = NetworkInterface::dummy();
    physical.name = "en0".to_string();
    physical.flags = IFF_UP as u32;
    let mut gateway = NetworkDevice::new();
    gateway.ipv4.push(Ipv4Addr::new(192, 168, 64, 1));
    physical.gateway = Some(gateway);
    physical.dns_servers = vec![
        IpAddr::V4(Ipv4Addr::new(192, 168, 64, 1)),
        IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
    ];

    let mut tunnel = NetworkInterface::dummy();
    tunnel.name = "utun100".to_string();
    tunnel.flags = (IFF_UP | IFF_POINTOPOINT) as u32;
    let mut tunnel_gateway = NetworkDevice::new();
    tunnel_gateway.ipv4.push(Ipv4Addr::new(100, 64, 0, 1));
    tunnel.gateway = Some(tunnel_gateway);
    tunnel.dns_servers = vec![IpAddr::V4(Ipv4Addr::new(100, 64, 0, 2))];

    let hosts = control_plane_bypass_ipv4_hosts_from_interfaces(&config, &[physical, tunnel]);

    assert_eq!(
        hosts,
        vec![
            Ipv4Addr::new(1, 1, 1, 1),
            Ipv4Addr::new(192, 0, 2, 40),
            Ipv4Addr::new(192, 168, 64, 1),
            Ipv4Addr::new(198, 51, 100, 20),
            Ipv4Addr::new(198, 51, 100, 30),
            Ipv4Addr::new(203, 0, 113, 10),
        ]
    );
}

#[test]
fn runtime_effective_advertised_routes_filter_default_exit_routes_by_platform() {
    let mut config = AppConfig::default();
    config.node.advertise_exit_node = true;
    config.node.advertised_routes = vec!["10.55.0.0/24".to_string()];

    let effective = runtime_effective_advertised_routes(&config);

    #[cfg(target_os = "linux")]
    assert_eq!(
        effective,
        vec![
            "10.55.0.0/24".to_string(),
            "0.0.0.0/0".to_string(),
            "::/0".to_string(),
        ]
    );

    #[cfg(target_os = "macos")]
    assert_eq!(
        effective,
        vec!["10.55.0.0/24".to_string(), "0.0.0.0/0".to_string()]
    );

    #[cfg(target_os = "windows")]
    assert_eq!(
        effective,
        vec![
            "10.55.0.0/24".to_string(),
            "0.0.0.0/0".to_string(),
            "::/0".to_string(),
        ]
    );

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    assert_eq!(effective, vec!["10.55.0.0/24".to_string()]);
}

#[test]
fn selected_exit_node_participant_tracks_supported_platforms() {
    let mut config = AppConfig::generated();
    let participant = "11".repeat(32);
    config.networks[0].participants = vec![participant.clone()];
    config.exit_node = participant.clone();

    let announcements = HashMap::from([(
        participant.clone(),
        PeerAnnouncement {
            node_id: "peer-a".to_string(),
            public_key: generate_keypair().public_key,
            endpoint: "203.0.113.20:51820".to_string(),
            local_endpoint: None,
            public_endpoint: Some("203.0.113.20:51820".to_string()),
            tunnel_ip: "10.44.0.2/32".to_string(),
            advertised_routes: vec!["0.0.0.0/0".to_string()],
            timestamp: 10,
        },
    )]);

    let selected = selected_exit_node_participant(&config, None, &announcements);

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    assert_eq!(selected.as_deref(), Some(participant.as_str()));

    #[cfg(target_os = "windows")]
    assert_eq!(selected.as_deref(), Some(participant.as_str()));

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    assert!(selected.is_none());
}

#[test]
fn macos_route_get_spec_parses_gateway_and_interface() {
    let output = "\
   route to: default\n\
destination: default\n\
   mask: default\n\
gateway: 10.10.243.254\n\
  interface: en0\n";
    let spec = macos_route_get_spec_from_output(output).expect("macOS route spec");
    assert_eq!(spec.gateway.as_deref(), Some("10.10.243.254"));
    assert_eq!(spec.interface, "en0");
}

#[test]
fn macos_tunnel_mtu_matches_other_desktop_tunnels() {
    assert_eq!(MACOS_TUNNEL_MTU, "1380");
}

#[test]
fn split_host_port_keeps_literal_host_without_port() {
    assert_eq!(
        split_host_port("relay.example.com", 443),
        Some(("relay.example.com".to_string(), 443))
    );
    assert_eq!(
        split_host_port("203.0.113.10:51820", 443),
        Some(("203.0.113.10".to_string(), 51820))
    );
}
