use std::io::{Read, Write};
use std::net::{
    IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, SocketAddrV4, SocketAddrV6, TcpStream,
    ToSocketAddrs, UdpSocket,
};
use std::time::Duration;

use nostr_vpn_core::diagnostics::{ProbeState, ProbeStatus};

pub(super) const PCP_DEFAULT_PORT: u16 = 5351;
pub(super) const NAT_PMP_DEFAULT_PORT: u16 = 5351;
pub(super) const SSDP_DISCOVERY_ADDR: &str = "239.255.255.250:1900";
const PCP_ANNOUNCE_PACKET_BYTES: usize = 24;

#[derive(Debug, Clone, Copy)]
pub(super) struct CaptivePortalEndpoint {
    pub(super) url: &'static str,
    pub(super) expected_status: u16,
    pub(super) expected_prefix: &'static str,
}

pub(super) const CAPTIVE_PORTAL_ENDPOINTS: &[CaptivePortalEndpoint] = &[
    CaptivePortalEndpoint {
        url: "http://connectivitycheck.gstatic.com/generate_204",
        expected_status: 204,
        expected_prefix: "",
    },
    CaptivePortalEndpoint {
        url: "http://www.msftconnecttest.com/connecttest.txt",
        expected_status: 200,
        expected_prefix: "Microsoft Connect Test",
    },
    CaptivePortalEndpoint {
        url: "http://captive.apple.com/hotspot-detect.html",
        expected_status: 200,
        expected_prefix: "<HTML><HEAD><TITLE>Success</TITLE></HEAD><BODY>Success</BODY></HTML>",
    },
];

pub(super) fn check_captive_portal_endpoint(
    endpoint: CaptivePortalEndpoint,
    timeout: Duration,
) -> Option<bool> {
    let (host, port, path) = parse_http_url(endpoint.url)?;
    let address = (host.as_str(), port).to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&address, timeout).ok()?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nCache-Control: no-cache\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).ok()?;
    let _ = stream.shutdown(Shutdown::Write);
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    let (status, body) = parse_http_response(&response)?;
    if status != endpoint.expected_status {
        return Some(true);
    }
    if !endpoint.expected_prefix.is_empty() && !body.starts_with(endpoint.expected_prefix) {
        return Some(true);
    }
    Some(false)
}

fn parse_http_url(url: &str) -> Option<(String, u16, String)> {
    let raw = url.strip_prefix("http://")?;
    let (authority, path) = raw
        .split_once('/')
        .map_or((raw, "/".to_string()), |(host, path)| {
            (host, format!("/{path}"))
        });
    let (host, port) = authority
        .rsplit_once(':')
        .and_then(|(host, port)| {
            port.parse::<u16>()
                .ok()
                .map(|port| (host.to_string(), port))
        })
        .unwrap_or_else(|| (authority.to_string(), 80));
    Some((host, port, path))
}

pub(super) fn parse_http_response(response: &str) -> Option<(u16, String)> {
    let (headers, body) = response.split_once("\r\n\r\n")?;
    let status = headers
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse::<u16>()
        .ok()?;
    Some((status, body.to_string()))
}

fn udp_client_bind_addr_for_server(server: SocketAddr) -> SocketAddr {
    match server {
        SocketAddr::V4(addr) if addr.ip().is_loopback() => {
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        }
        SocketAddr::V4(_) => SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
        SocketAddr::V6(addr) if addr.ip().is_loopback() => {
            SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0))
        }
        SocketAddr::V6(_) => SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0)),
    }
}

pub(super) fn probe_nat_pmp_server(server: SocketAddr, timeout: Duration) -> ProbeStatus {
    let bind_addr = udp_client_bind_addr_for_server(server);
    let socket = match UdpSocket::bind(bind_addr) {
        Ok(socket) => socket,
        Err(error) => return ProbeStatus::new(ProbeState::Error, error.to_string()),
    };
    let _ = socket.set_read_timeout(Some(timeout));
    let _ = socket.set_write_timeout(Some(timeout));

    if let Err(error) = socket.send_to(&[0, 0], server) {
        return ProbeStatus::new(ProbeState::Error, error.to_string());
    }
    let mut buf = [0_u8; 64];
    match socket.recv_from(&mut buf) {
        Ok((read, _)) if read >= 12 && buf[0] == 0 && buf[1] == 128 => ProbeStatus::new(
            ProbeState::Available,
            "gateway responded to external address request",
        ),
        Ok((read, _)) => ProbeStatus::new(
            ProbeState::Unavailable,
            format!("unexpected NAT-PMP response length {read}"),
        ),
        Err(error) => ProbeStatus::new(ProbeState::Unavailable, error.to_string()),
    }
}

pub(super) fn probe_pcp_server(
    server: SocketAddr,
    client_ip: Option<IpAddr>,
    timeout: Duration,
) -> ProbeStatus {
    let bind_addr = udp_client_bind_addr_for_server(server);
    let socket = match UdpSocket::bind(bind_addr) {
        Ok(socket) => socket,
        Err(error) => return ProbeStatus::new(ProbeState::Error, error.to_string()),
    };
    let _ = socket.set_read_timeout(Some(timeout));
    let _ = socket.set_write_timeout(Some(timeout));

    let client_ip = client_ip.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let mut packet = [0_u8; PCP_ANNOUNCE_PACKET_BYTES];
    packet[0] = 2;
    packet[1] = 0;
    match client_ip {
        IpAddr::V4(ip) => {
            packet[20..24].copy_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            packet[8..24].copy_from_slice(&ip.octets());
        }
    }

    if let Err(error) = socket.send_to(&packet, server) {
        return ProbeStatus::new(ProbeState::Error, error.to_string());
    }
    let mut buf = [0_u8; 128];
    match socket.recv_from(&mut buf) {
        Ok((read, _)) if read >= 24 && buf[0] == 2 && buf[1] == 0x80 => ProbeStatus::new(
            ProbeState::Available,
            "gateway responded to PCP announce request",
        ),
        Ok((read, _)) => ProbeStatus::new(
            ProbeState::Unavailable,
            format!("unexpected PCP response length {read}"),
        ),
        Err(error) => ProbeStatus::new(ProbeState::Unavailable, error.to_string()),
    }
}

pub(super) fn probe_upnp_ssdp_server(server: SocketAddr, timeout: Duration) -> ProbeStatus {
    let socket = match UdpSocket::bind(udp_client_bind_addr_for_server(server)) {
        Ok(socket) => socket,
        Err(error) => return ProbeStatus::new(ProbeState::Error, error.to_string()),
    };
    let _ = socket.set_read_timeout(Some(timeout));
    let _ = socket.set_write_timeout(Some(timeout));

    let request = concat!(
        "M-SEARCH * HTTP/1.1\r\n",
        "HOST: 239.255.255.250:1900\r\n",
        "MAN: \"ssdp:discover\"\r\n",
        "MX: 1\r\n",
        "ST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n",
        "\r\n"
    );
    if let Err(error) = socket.send_to(request.as_bytes(), server) {
        return ProbeStatus::new(ProbeState::Error, error.to_string());
    }
    let mut buf = [0_u8; 1536];
    match socket.recv_from(&mut buf) {
        Ok((read, _)) => {
            let response = String::from_utf8_lossy(&buf[..read]).to_ascii_lowercase();
            if response.contains("location:") || response.contains("internetgatewaydevice") {
                ProbeStatus::new(ProbeState::Available, "gateway responded to SSDP discovery")
            } else {
                ProbeStatus::new(ProbeState::Unavailable, "unexpected SSDP response")
            }
        }
        Err(error) => ProbeStatus::new(ProbeState::Unavailable, error.to_string()),
    }
}

pub(super) fn mapping_varies_by_dest_ip(endpoints: &[String]) -> Option<bool> {
    if endpoints.len() < 2 {
        return None;
    }
    let distinct = endpoints
        .iter()
        .filter_map(|value| value.parse::<SocketAddr>().ok())
        .map(|value| value.ip())
        .collect::<std::collections::HashSet<_>>();
    Some(distinct.len() > 1)
}
