use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use nostr_vpn_core::config::normalize_nostr_pubkey;
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, SockRef, Socket, Type};

use crate::invite::{parse_network_invite, to_npub};

pub(crate) const LAN_PAIRING_ANNOUNCEMENT_VERSION: u8 = 2;
pub const LAN_PAIRING_DURATION: Duration = Duration::from_mins(15);
pub(crate) const LAN_PAIRING_STALE_AFTER: Duration = Duration::from_secs(16);

const LAN_PAIRING_ADDR: Ipv4Addr = Ipv4Addr::new(239, 255, 73, 73);
const LAN_PAIRING_PORT: u16 = 38_911;
const LAN_PAIRING_ANNOUNCE_EVERY: Duration = Duration::from_secs(3);
const LAN_PAIRING_READ_TIMEOUT: Duration = Duration::from_millis(250);
const LAN_PAIRING_BUFFER_BYTES: usize = 8_192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanPairingSignal {
    pub npub: String,
    pub node_name: String,
    pub endpoint: String,
    pub network_name: String,
    pub network_id: String,
    pub invite: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanPairingAnnouncement {
    pub npub: String,
    pub node_name: String,
    pub endpoint: String,
    pub invite: String,
}

/// Shared control surface for the worker thread.
///
/// Both `broadcast_until` and `listen_until` are unix-second deadlines; the
/// worker stays alive as long as either is in the future, and inside each tick
/// it checks them independently to decide whether to send / accept.
#[derive(Debug)]
pub(crate) struct LanPairingControl {
    stop: AtomicBool,
    announcement: RwLock<LanPairingAnnouncement>,
    broadcast_until: AtomicU64,
    listen_until: AtomicU64,
}

impl LanPairingControl {
    fn new(announcement: LanPairingAnnouncement) -> Self {
        Self {
            stop: AtomicBool::new(false),
            announcement: RwLock::new(announcement),
            broadcast_until: AtomicU64::new(0),
            listen_until: AtomicU64::new(0),
        }
    }

    fn broadcast_active(&self, now_secs: u64) -> bool {
        self.broadcast_until.load(Ordering::Relaxed) > now_secs
    }

    fn listen_active(&self, now_secs: u64) -> bool {
        self.listen_until.load(Ordering::Relaxed) > now_secs
    }

    fn any_active(&self, now_secs: u64) -> bool {
        self.broadcast_active(now_secs) || self.listen_active(now_secs)
    }
}

#[derive(Debug)]
pub struct LanPairingWorker {
    receiver: Receiver<LanPairingSignal>,
    control: Arc<LanPairingControl>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct LanPairingAnnouncementPayload {
    v: u8,
    npub: String,
    #[serde(default)]
    node_name: String,
    #[serde(default)]
    endpoint: String,
    invite: String,
    #[serde(default)]
    timestamp: u64,
}

impl LanPairingWorker {
    pub fn drain(&mut self) -> Vec<LanPairingSignal> {
        let mut signals = Vec::new();
        while let Ok(signal) = self.receiver.try_recv() {
            signals.push(signal);
        }
        signals
    }

    /// Mark the worker as broadcasting our invite until `expires_at`.
    pub fn set_broadcast_until(&self, expires_at: SystemTime) {
        self.control
            .broadcast_until
            .store(unix_seconds(expires_at), Ordering::Relaxed);
    }

    /// Mark the worker as listening for nearby invites until `expires_at`.
    pub fn set_listen_until(&self, expires_at: SystemTime) {
        self.control
            .listen_until
            .store(unix_seconds(expires_at), Ordering::Relaxed);
    }

    pub(crate) fn clear_broadcast(&self) {
        self.control.broadcast_until.store(0, Ordering::Relaxed);
    }

    pub(crate) fn clear_listen(&self) {
        self.control.listen_until.store(0, Ordering::Relaxed);
    }

    pub(crate) fn update_announcement(&self, announcement: LanPairingAnnouncement) {
        if let Ok(mut current) = self.control.announcement.write() {
            *current = announcement;
        }
    }

    pub fn stop(&mut self) {
        self.control.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LanPairingWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn spawn_lan_pairing_worker(announcement: LanPairingAnnouncement) -> Result<LanPairingWorker> {
    let socket = bind_lan_pairing_socket()?;
    let interfaces = lan_pairing_interfaces();
    join_multicast_on_interfaces(&socket, &interfaces);

    let (sender, receiver) = mpsc::channel();
    let control = Arc::new(LanPairingControl::new(announcement));
    let own_npub = to_npub_for_filter(&control);
    let thread_control = Arc::clone(&control);
    let handle = thread::spawn(move || {
        run_lan_pairing_loop(&socket, &thread_control, &interfaces, &own_npub, &sender);
    });

    Ok(LanPairingWorker {
        receiver,
        control,
        handle: Some(handle),
    })
}

fn to_npub_for_filter(control: &LanPairingControl) -> String {
    control
        .announcement
        .read()
        .map(|guard| guard.npub.clone())
        .unwrap_or_default()
}

fn bind_lan_pairing_socket() -> Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("failed to create LAN pairing UDP socket")?;
    socket
        .set_reuse_address(true)
        .context("failed to configure LAN pairing socket reuse")?;
    #[cfg(all(
        unix,
        not(target_os = "solaris"),
        not(target_os = "illumos"),
        not(target_os = "cygwin")
    ))]
    socket
        .set_reuse_port(true)
        .context("failed to configure LAN pairing port reuse")?;
    socket
        .set_broadcast(true)
        .context("failed to enable broadcast on LAN pairing socket")?;
    socket
        .bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, LAN_PAIRING_PORT).into())
        .context("failed to bind LAN pairing UDP socket")?;

    let socket: UdpSocket = socket.into();
    // Default-interface join — preserved as a baseline so single-interface hosts
    // and the loopback test still work without depending on netdev.
    let _ = socket.join_multicast_v4(&LAN_PAIRING_ADDR, &Ipv4Addr::UNSPECIFIED);
    socket
        .set_read_timeout(Some(LAN_PAIRING_READ_TIMEOUT))
        .context("failed to configure LAN pairing socket timeout")?;
    socket
        .set_multicast_loop_v4(true)
        .context("failed to configure LAN pairing multicast loopback")?;
    socket
        .set_multicast_ttl_v4(1)
        .context("failed to configure LAN pairing multicast TTL")?;
    Ok(socket)
}

#[derive(Debug, Clone)]
struct LanInterface {
    addr: Ipv4Addr,
    broadcast: Option<Ipv4Addr>,
}

/// Enumerate non-loopback IPv4 interfaces with link addresses.
///
/// On Windows, the routing-table-derived `INADDR_ANY` multicast join often
/// picks a virtual adapter (Hyper-V vEthernet, WSL, Tailscale, etc.) instead
/// of the physical Wi-Fi/Ethernet, which is why a Mac on the same LAN can see
/// the Windows announcements (the Mac's join is correct) but not vice versa.
/// Joining on every real interface fixes that asymmetry.
fn lan_pairing_interfaces() -> Vec<LanInterface> {
    let mut out = Vec::new();
    for iface in netdev::get_interfaces() {
        if iface.is_loopback() {
            continue;
        }
        for net in &iface.ipv4 {
            let addr = net.addr();
            if addr.is_loopback() || addr.is_unspecified() || addr.is_link_local() {
                continue;
            }
            let broadcast = directed_broadcast(addr, net.prefix_len());
            out.push(LanInterface { addr, broadcast });
        }
    }
    out
}

fn directed_broadcast(addr: Ipv4Addr, prefix_len: u8) -> Option<Ipv4Addr> {
    if prefix_len == 0 || prefix_len >= 32 {
        return None;
    }
    let host_bits = 32 - u32::from(prefix_len);
    let mask = u32::MAX.checked_shl(host_bits).unwrap_or(0);
    let bcast = u32::from(addr) | !mask;
    Some(Ipv4Addr::from(bcast))
}

fn join_multicast_on_interfaces(socket: &UdpSocket, interfaces: &[LanInterface]) {
    for iface in interfaces {
        // Duplicate joins (already covered by the INADDR_ANY join) return
        // EADDRINUSE on some platforms — harmless, swallow.
        let _ = socket.join_multicast_v4(&LAN_PAIRING_ADDR, &iface.addr);
    }
}

pub(crate) fn decode_lan_pairing_payload(
    payload: &[u8],
    own_npub: &str,
) -> Result<Option<LanPairingSignal>> {
    let announcement = serde_json::from_slice::<LanPairingAnnouncementPayload>(payload)
        .context("failed to parse LAN pairing announcement")?;
    if announcement.v != LAN_PAIRING_ANNOUNCEMENT_VERSION {
        return Ok(None);
    }

    let sender_npub = normalize_nostr_pubkey(&announcement.npub).map(|value| to_npub(&value))?;
    if sender_npub == own_npub.trim() {
        return Ok(None);
    }

    let invite =
        parse_network_invite(&announcement.invite).context("failed to parse LAN pairing invite")?;
    if !invite.admins.iter().any(|admin| admin == &sender_npub) {
        return Ok(None);
    }

    Ok(Some(LanPairingSignal {
        npub: sender_npub,
        node_name: announcement.node_name.trim().to_string(),
        endpoint: announcement.endpoint.trim().to_string(),
        network_name: if invite.network_name.trim().is_empty() {
            invite.network_id.clone()
        } else {
            invite.network_name
        },
        network_id: invite.network_id,
        invite: announcement.invite.trim().to_string(),
    }))
}

fn run_lan_pairing_loop(
    socket: &UdpSocket,
    control: &Arc<LanPairingControl>,
    interfaces: &[LanInterface],
    own_npub: &str,
    sender: &mpsc::Sender<LanPairingSignal>,
) {
    let multicast_target = SocketAddrV4::new(LAN_PAIRING_ADDR, LAN_PAIRING_PORT);
    let mut next_announcement = SystemTime::UNIX_EPOCH;
    let mut buffer = [0_u8; LAN_PAIRING_BUFFER_BYTES];

    while !control.stop.load(Ordering::Relaxed) {
        let now = SystemTime::now();
        let now_secs = unix_seconds(now);
        if !control.any_active(now_secs) {
            break;
        }

        if control.broadcast_active(now_secs) && now >= next_announcement {
            let snapshot = control.announcement.read().ok().map(|guard| guard.clone());
            if let Some(announcement) = snapshot {
                let _ = send_lan_pairing_announcement(
                    socket,
                    multicast_target,
                    interfaces,
                    &announcement,
                );
            }
            next_announcement = now
                .checked_add(LAN_PAIRING_ANNOUNCE_EVERY)
                .unwrap_or_else(SystemTime::now);
        }

        match socket.recv_from(&mut buffer) {
            Ok((len, _)) => {
                if !control.listen_active(unix_seconds(SystemTime::now())) {
                    continue;
                }
                if let Ok(Some(signal)) = decode_lan_pairing_payload(&buffer[..len], own_npub) {
                    let _ = sender.send(signal);
                }
            }
            Err(error)
                if error.kind() == ErrorKind::WouldBlock || error.kind() == ErrorKind::TimedOut => {
            }
            Err(_) => break,
        }
    }
}

fn send_lan_pairing_announcement(
    socket: &UdpSocket,
    multicast_target: SocketAddrV4,
    interfaces: &[LanInterface],
    announcement: &LanPairingAnnouncement,
) -> Result<()> {
    let payload = LanPairingAnnouncementPayload {
        v: LAN_PAIRING_ANNOUNCEMENT_VERSION,
        npub: announcement.npub.clone(),
        node_name: announcement.node_name.clone(),
        endpoint: announcement.endpoint.clone(),
        invite: announcement.invite.clone(),
        timestamp: unix_timestamp(),
    };
    let encoded = serde_json::to_vec(&payload).context("failed to encode LAN announcement")?;

    let sock_ref = SockRef::from(socket);

    // Always send via the OS-default interface — covers loopback test, single-NIC
    // hosts, and platforms where the per-interface fan-out below adds nothing.
    let _ = sock_ref.set_multicast_if_v4(&Ipv4Addr::UNSPECIFIED);
    let _ = socket.send_to(&encoded, multicast_target);

    // Fan out to every real interface so multi-homed hosts (Windows with
    // Hyper-V/WSL/Tailscale, Macs on Wi-Fi + Ethernet, etc.) actually reach
    // peers on every L2 segment they're connected to. Multicast is tried first;
    // a directed broadcast is the belt-and-suspenders fallback when the LAN
    // suppresses multicast (some consumer routers do this for IPv4).
    for iface in interfaces {
        if sock_ref.set_multicast_if_v4(&iface.addr).is_ok() {
            let _ = socket.send_to(&encoded, multicast_target);
        }
        if let Some(broadcast) = iface.broadcast {
            let target = SocketAddrV4::new(broadcast, LAN_PAIRING_PORT);
            let _ = socket.send_to(&encoded, target);
        }
    }

    // Global limited broadcast — last-resort fallback for hosts whose interface
    // enumeration picked up nothing useful (locked-down VMs, captive portals).
    let _ = socket.send_to(
        &encoded,
        SocketAddrV4::new(Ipv4Addr::BROADCAST, LAN_PAIRING_PORT),
    );

    Ok(())
}

fn unix_timestamp() -> u64 {
    unix_seconds(SystemTime::now())
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use nostr_sdk::prelude::ToBech32;
    use serde_json::json;
    use std::time::Instant;

    use super::*;

    #[test]
    fn decodes_lan_announcement_with_invite_metadata() {
        let admin_npub = nostr_sdk::Keys::generate()
            .public_key()
            .to_bech32()
            .expect("npub");
        let own_npub = nostr_sdk::Keys::generate()
            .public_key()
            .to_bech32()
            .expect("npub");
        let invite = invite_for(&admin_npub, "Office mesh", "office-mesh");
        let payload = json!({
            "v": LAN_PAIRING_ANNOUNCEMENT_VERSION,
            "npub": admin_npub,
            "nodeName": "Alice Mac",
            "endpoint": "192.0.2.10:51820",
            "invite": invite,
            "timestamp": 42
        })
        .to_string();

        let signal = decode_lan_pairing_payload(payload.as_bytes(), &own_npub)
            .expect("decode")
            .expect("peer");

        assert_eq!(signal.node_name, "Alice Mac");
        assert_eq!(signal.endpoint, "192.0.2.10:51820");
        assert_eq!(signal.network_name, "Office mesh");
        assert_eq!(signal.network_id, "office-mesh");
    }

    #[test]
    fn lan_pairing_workers_exchange_invites_over_looped_multicast() {
        let alice_npub = nostr_sdk::Keys::generate()
            .public_key()
            .to_bech32()
            .expect("alice npub");
        let bob_npub = nostr_sdk::Keys::generate()
            .public_key()
            .to_bech32()
            .expect("bob npub");
        let alice_invite = invite_for(&alice_npub, "Alice mesh", "alice-mesh");
        let bob_invite = invite_for(&bob_npub, "Bob mesh", "bob-mesh");
        let expires_at = SystemTime::now()
            .checked_add(Duration::from_secs(7))
            .expect("expiry");

        let mut alice = spawn_lan_pairing_worker(LanPairingAnnouncement {
            npub: alice_npub.clone(),
            node_name: "Alice".to_string(),
            endpoint: "192.0.2.10:51820".to_string(),
            invite: alice_invite,
        })
        .expect("start alice LAN pairing");
        alice.set_broadcast_until(expires_at);
        alice.set_listen_until(expires_at);
        let mut bob = spawn_lan_pairing_worker(LanPairingAnnouncement {
            npub: bob_npub.clone(),
            node_name: "Bob".to_string(),
            endpoint: "192.0.2.11:51820".to_string(),
            invite: bob_invite,
        })
        .expect("start bob LAN pairing");
        bob.set_broadcast_until(expires_at);
        bob.set_listen_until(expires_at);

        let deadline = Instant::now() + Duration::from_secs(8);
        let mut alice_saw_bob = false;
        let mut bob_saw_alice = false;
        while Instant::now() < deadline && !(alice_saw_bob && bob_saw_alice) {
            alice_saw_bob |= alice.drain().into_iter().any(|signal| {
                signal.npub == bob_npub
                    && signal.node_name == "Bob"
                    && signal.network_id == "bob-mesh"
            });
            bob_saw_alice |= bob.drain().into_iter().any(|signal| {
                signal.npub == alice_npub
                    && signal.node_name == "Alice"
                    && signal.network_id == "alice-mesh"
            });
            thread::sleep(Duration::from_millis(100));
        }

        alice.stop();
        bob.stop();

        assert!(alice_saw_bob, "alice did not receive bob's LAN invite");
        assert!(bob_saw_alice, "bob did not receive alice's LAN invite");
    }

    #[test]
    fn listen_only_worker_receives_without_advertising() {
        let alice_npub = nostr_sdk::Keys::generate()
            .public_key()
            .to_bech32()
            .expect("alice npub");
        let bob_npub = nostr_sdk::Keys::generate()
            .public_key()
            .to_bech32()
            .expect("bob npub");
        let alice_invite = invite_for(&alice_npub, "Alice mesh", "alice-mesh");
        let bob_invite = invite_for(&bob_npub, "Bob mesh", "bob-mesh");
        let expires_at = SystemTime::now()
            .checked_add(Duration::from_secs(7))
            .expect("expiry");

        // Alice broadcasts only — she should never surface bob's invite.
        let mut alice = spawn_lan_pairing_worker(LanPairingAnnouncement {
            npub: alice_npub.clone(),
            node_name: "Alice".to_string(),
            endpoint: "192.0.2.10:51820".to_string(),
            invite: alice_invite,
        })
        .expect("start alice");
        alice.set_broadcast_until(expires_at);

        // Bob listens only — he should still see alice.
        let mut bob = spawn_lan_pairing_worker(LanPairingAnnouncement {
            npub: bob_npub.clone(),
            node_name: "Bob".to_string(),
            endpoint: "192.0.2.11:51820".to_string(),
            invite: bob_invite,
        })
        .expect("start bob");
        bob.set_listen_until(expires_at);

        let deadline = Instant::now() + Duration::from_secs(8);
        let mut bob_saw_alice = false;
        let mut alice_saw_bob = false;
        while Instant::now() < deadline && !bob_saw_alice {
            bob_saw_alice |= bob
                .drain()
                .into_iter()
                .any(|signal| signal.npub == alice_npub && signal.network_id == "alice-mesh");
            alice_saw_bob |= alice
                .drain()
                .into_iter()
                .any(|signal| signal.npub == bob_npub && signal.network_id == "bob-mesh");
            thread::sleep(Duration::from_millis(100));
        }

        alice.stop();
        bob.stop();

        assert!(
            bob_saw_alice,
            "listen-only worker did not receive broadcast"
        );
        assert!(
            !alice_saw_bob,
            "broadcast-only worker should not surface peers (listen disabled)"
        );
    }

    #[test]
    fn directed_broadcast_for_common_prefixes() {
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(192, 168, 1, 17), 24),
            Some(Ipv4Addr::new(192, 168, 1, 255))
        );
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(10, 0, 0, 5), 8),
            Some(Ipv4Addr::new(10, 255, 255, 255))
        );
        assert_eq!(directed_broadcast(Ipv4Addr::new(10, 0, 0, 5), 32), None);
        assert_eq!(directed_broadcast(Ipv4Addr::new(10, 0, 0, 5), 0), None);
    }

    fn invite_for(admin_npub: &str, network_name: &str, network_id: &str) -> String {
        let payload = json!({
            "v": 3,
            "networkName": network_name,
            "networkId": network_id,
            "admins": [admin_npub],
            "relays": ["wss://relay.example"]
        })
        .to_string();
        format!("nvpn://invite/{}", URL_SAFE_NO_PAD.encode(payload))
    }
}
