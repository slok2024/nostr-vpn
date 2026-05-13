//! 1:1 SNAT/DNAT between the mesh tun address and the WG upstream
//! peer address.
//!
//! On mobile we have a single OS-managed tun (Android `VpnService`,
//! iOS `NEPacketTunnelProvider`). That tun is configured with the
//! FIPS mesh IP (e.g. `10.44.0.1`) so other mesh peers can reach us.
//! When the user enables WG upstream, the same tun also carries
//! "rest of internet" traffic — and Mullvad / Proton WG endpoints
//! enforce that the inner source IP of decapsulated packets matches
//! the peer's configured address (e.g. `10.66.66.7`). The kernel,
//! seeing only the FIPS mesh address on the tun, picks `10.44.0.1`
//! as the source for outbound packets to the internet, and the WG
//! server silently drops them.
//!
//! On Linux this is hidden by an iptables `MASQUERADE` rule and a
//! separate kernel WG interface holding the WG address. We don't have
//! that on mobile, so we do the equivalent rewrite in userspace just
//! before handing plaintext to the boringtun pump (and the reverse
//! after decapsulation).
//!
//! Single-tunnel-client semantics keeps this stateless: every
//! WG-bound packet has source = mesh IP, every WG-decapped packet has
//! destination = WG address. No NAT table is needed.
//!
//! IPv6 is not yet rewritten — the desktop config only supports IPv4
//! WG endpoints today.

// 16-bit ones-complement folding (RFC 1071/1624) is the canonical
// idiom for IP/UDP/TCP checksum updates. The `as u16` truncations are
// intentional after folding; this pattern appears in every TCP/IP stack.
#![allow(clippy::cast_possible_truncation)]

use std::net::Ipv4Addr;

/// Rewrite the IPv4 source address in `packet` from `old` to `new`,
/// updating the IP header checksum and the TCP/UDP transport
/// checksum (which covers the pseudo-header). No-op for non-IPv4
/// packets, packets too short to be IPv4, or packets whose source
/// already matches `new`.
pub(crate) fn rewrite_ipv4_source(packet: &mut [u8], old: Ipv4Addr, new: Ipv4Addr) {
    rewrite_ipv4_address(packet, old, new, AddressField::Source);
}

/// Reverse of [`rewrite_ipv4_source`]. Used on inbound traffic from
/// the WG upstream to flip the destination back to the mesh IP so the
/// OS routes the packet to the local stack.
pub(crate) fn rewrite_ipv4_destination(packet: &mut [u8], old: Ipv4Addr, new: Ipv4Addr) {
    rewrite_ipv4_address(packet, old, new, AddressField::Destination);
}

#[derive(Copy, Clone)]
enum AddressField {
    Source,
    Destination,
}

fn rewrite_ipv4_address(packet: &mut [u8], old: Ipv4Addr, new: Ipv4Addr, field: AddressField) {
    if old == new {
        return;
    }
    if packet.len() < 20 {
        return;
    }
    if packet[0] >> 4 != 4 {
        return;
    }
    let ihl = (packet[0] & 0x0f) as usize * 4;
    if ihl < 20 || packet.len() < ihl {
        return;
    }
    let offset = match field {
        AddressField::Source => 12,
        AddressField::Destination => 16,
    };
    let current = [
        packet[offset],
        packet[offset + 1],
        packet[offset + 2],
        packet[offset + 3],
    ];
    if Ipv4Addr::from(current) != old {
        return;
    }
    let new_octets = new.octets();

    let ip_check = u16::from_be_bytes([packet[10], packet[11]]);
    let new_ip_check = update_checksum(ip_check, current, new_octets);
    packet[10..12].copy_from_slice(&new_ip_check.to_be_bytes());

    packet[offset..offset + 4].copy_from_slice(&new_octets);

    let frag_off = u16::from_be_bytes([packet[6], packet[7]]) & 0x1fff;
    let mf = packet[6] & 0x20 != 0;
    let is_first_fragment = frag_off == 0;
    if !is_first_fragment {
        return;
    }

    let protocol = packet[9];
    let payload = &mut packet[ihl..];
    match protocol {
        6 if payload.len() >= 18 => {
            let tcp_check = u16::from_be_bytes([payload[16], payload[17]]);
            let new_tcp_check = update_checksum(tcp_check, current, new_octets);
            payload[16..18].copy_from_slice(&new_tcp_check.to_be_bytes());
        }
        17 if payload.len() >= 8 => {
            let udp_check = u16::from_be_bytes([payload[6], payload[7]]);
            if udp_check != 0 {
                let new_udp_check = update_checksum(udp_check, current, new_octets);
                let final_check = if new_udp_check == 0 {
                    0xffff
                } else {
                    new_udp_check
                };
                payload[6..8].copy_from_slice(&final_check.to_be_bytes());
            }
        }
        _ => {}
    }

    let _ = mf;
}

/// RFC 1624 incremental checksum update for replacing one 4-byte
/// field with another. Works for both the IPv4 header checksum and
/// TCP/UDP checksums, since the only contribution of an IP address
/// to the latter is the same 4 bytes in the pseudo-header.
fn update_checksum(old_check: u16, old: [u8; 4], new: [u8; 4]) -> u16 {
    let mut sum: u32 = u32::from(!old_check);
    sum += u32::from(!u16::from_be_bytes([old[0], old[1]]));
    sum += u32::from(!u16::from_be_bytes([old[2], old[3]]));
    sum += u32::from(u16::from_be_bytes([new[0], new[1]]));
    sum += u32::from(u16::from_be_bytes([new[2], new[3]]));
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_udp_packet(src: Ipv4Addr, dst: Ipv4Addr, payload: &[u8]) -> Vec<u8> {
        let total_len = 20 + 8 + payload.len();
        let mut buf = vec![0u8; total_len];
        buf[0] = 0x45;
        buf[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        buf[4..6].copy_from_slice(&[0x12, 0x34]);
        buf[8] = 64;
        buf[9] = 17;
        buf[12..16].copy_from_slice(&src.octets());
        buf[16..20].copy_from_slice(&dst.octets());

        let ip_check = compute_ipv4_checksum(&buf[..20]);
        buf[10..12].copy_from_slice(&ip_check.to_be_bytes());

        buf[20..22].copy_from_slice(&5000u16.to_be_bytes());
        buf[22..24].copy_from_slice(&5001u16.to_be_bytes());
        let udp_len = 8 + payload.len();
        buf[24..26].copy_from_slice(&(udp_len as u16).to_be_bytes());
        buf[28..28 + payload.len()].copy_from_slice(payload);

        let udp_check = compute_udp_checksum(&buf, src, dst);
        buf[26..28].copy_from_slice(&udp_check.to_be_bytes());
        buf
    }

    fn compute_ipv4_checksum(header: &[u8]) -> u16 {
        let mut sum: u32 = 0;
        let mut i = 0;
        while i < header.len() {
            if i == 10 {
                i += 2;
                continue;
            }
            sum += u32::from(u16::from_be_bytes([header[i], header[i + 1]]));
            i += 2;
        }
        while sum > 0xffff {
            sum = (sum & 0xffff) + (sum >> 16);
        }
        !(sum as u16)
    }

    fn compute_udp_checksum(packet: &[u8], src: Ipv4Addr, dst: Ipv4Addr) -> u16 {
        let udp = &packet[20..];
        let udp_len = u32::from(u16::from_be_bytes([udp[4], udp[5]]));
        let mut sum: u32 = 0;
        let s = src.octets();
        let d = dst.octets();
        sum += u32::from(u16::from_be_bytes([s[0], s[1]]));
        sum += u32::from(u16::from_be_bytes([s[2], s[3]]));
        sum += u32::from(u16::from_be_bytes([d[0], d[1]]));
        sum += u32::from(u16::from_be_bytes([d[2], d[3]]));
        sum += 17;
        sum += udp_len;
        let mut i = 0;
        while i + 1 < udp.len() {
            if i == 6 {
                i += 2;
                continue;
            }
            sum += u32::from(u16::from_be_bytes([udp[i], udp[i + 1]]));
            i += 2;
        }
        if i < udp.len() {
            sum += u32::from(udp[i]) << 8;
        }
        while sum > 0xffff {
            sum = (sum & 0xffff) + (sum >> 16);
        }
        let folded = !(sum as u16);
        if folded == 0 { 0xffff } else { folded }
    }

    #[test]
    fn rewrite_source_keeps_packet_well_formed() {
        let mesh = Ipv4Addr::new(10, 44, 0, 1);
        let wg = Ipv4Addr::new(10, 66, 66, 7);
        let dest = Ipv4Addr::new(1, 1, 1, 1);

        let mut packet = build_udp_packet(mesh, dest, b"hello world");
        rewrite_ipv4_source(&mut packet, mesh, wg);

        assert_eq!(&packet[12..16], &wg.octets());
        let ip_recomputed = compute_ipv4_checksum(&packet[..20]);
        assert_eq!(
            ip_recomputed,
            u16::from_be_bytes([packet[10], packet[11]]),
            "ip header checksum must verify after rewrite",
        );
        let udp_recomputed = compute_udp_checksum(&packet, wg, dest);
        let udp_field = u16::from_be_bytes([packet[26], packet[27]]);
        assert_eq!(
            udp_recomputed, udp_field,
            "udp checksum must verify against new pseudo-header",
        );
    }

    #[test]
    fn rewrite_destination_keeps_packet_well_formed() {
        let mesh = Ipv4Addr::new(10, 44, 0, 1);
        let wg = Ipv4Addr::new(10, 66, 66, 7);
        let upstream = Ipv4Addr::new(8, 8, 8, 8);

        let mut packet = build_udp_packet(upstream, wg, b"reply");
        rewrite_ipv4_destination(&mut packet, wg, mesh);

        assert_eq!(&packet[16..20], &mesh.octets());
        let ip_recomputed = compute_ipv4_checksum(&packet[..20]);
        assert_eq!(ip_recomputed, u16::from_be_bytes([packet[10], packet[11]]),);
        let udp_recomputed = compute_udp_checksum(&packet, upstream, mesh);
        let udp_field = u16::from_be_bytes([packet[26], packet[27]]);
        assert_eq!(udp_recomputed, udp_field);
    }

    #[test]
    fn no_op_when_source_does_not_match() {
        let mut packet =
            build_udp_packet(Ipv4Addr::new(10, 44, 0, 1), Ipv4Addr::new(1, 1, 1, 1), b"x");
        let original = packet.clone();
        rewrite_ipv4_source(
            &mut packet,
            Ipv4Addr::new(10, 99, 99, 99),
            Ipv4Addr::new(10, 66, 66, 7),
        );
        assert_eq!(packet, original);
    }
}
