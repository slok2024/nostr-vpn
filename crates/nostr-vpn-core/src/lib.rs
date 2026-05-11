pub mod config;
mod config_defaults;
mod config_magic_dns;
pub mod control;
pub mod data_plane;
pub mod diagnostics;
pub mod fips_control;
pub mod fips_mesh;
pub mod invite;
pub mod join_requests;
pub mod lan_pairing;
pub mod magic_dns;
mod network_roster;
mod network_routes;
pub mod paths;
pub mod platform_paths;
pub mod process_ext;
pub mod wg_upstream;

pub use config::DEFAULT_RELAYS;

/// Underlay UDP MTU the daemon targets for the encrypted FIPS frame.
///
/// 1280 is the IPv6 minimum and the value fips-core's NAT-traversal
/// adoption path uses by default. Keeping the primary transport at the
/// same value means any session that gets promoted onto a NAT-traversed
/// link silently keeps working — the encrypted wire image is sized to
/// fit, regardless of which transport the next-hop happens to be.
///
/// Bumping this requires the matching change to NAT-traversal transport
/// inheritance (see fips-core `Node::adopt_established_traversal`) so
/// adopted sockets pick up the same framing budget instead of falling
/// back to the 1280 default and dropping every full-sized datagram.
pub const MESH_UNDERLAY_UDP_MTU: u16 = 1280;

/// Tunnel-side MTU: maximum IPv4/IPv6 packet a TUN device hands to the daemon
/// for encryption + transit. Equals `MESH_UNDERLAY_UDP_MTU` minus the 106-byte
/// FIPS overhead (handshake nonce + AEAD framing + inner header; see fips-core
/// `upper::icmp::FIPS_OVERHEAD`) minus a 24-byte cushion for the optional
/// COORDS warmup tag and any per-link variance. Single source of truth —
/// every TUN config, every UdpConfig, every Wintun adapter, every linux
/// `ip link set mtu` should derive from this.
pub const MESH_TUNNEL_MTU: u16 = 1150;
