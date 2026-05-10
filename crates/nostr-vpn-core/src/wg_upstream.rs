//! Userspace WireGuard upstream runtime — single-peer (Mullvad/Proton-style).
//!
//! Wraps `boringtun::noise::Tunn` for the case where this device is a WG
//! *client* of one upstream provider, not a multi-peer mesh participant.
//! Lives here in `nostr-vpn-core` so both the desktop daemon
//! (`nostr-vpn-cli`) and the mobile runtime (`nostr-vpn-app-core`) can use
//! the same boringtun pump — only the platform glue (tun ownership,
//! routing-table changes) lives in the platform-specific crate.
//!
//! Three concurrent jobs drive it, all dispatched through a single
//! coordinator task so the `Tunn` doesn't need a mutex:
//!   * UDP-rx: receive ciphertext from upstream → `Tunn::decapsulate` →
//!     forward plaintext to the platform writer.
//!   * tun-rx: receive plaintext from the platform reader →
//!     `Tunn::encapsulate` → send ciphertext to upstream.
//!   * timer: every 250ms call `Tunn::update_timers` so the handshake +
//!     keepalive state machine can re-key / re-init on schedule.
//!
//! Platforms wire the tun side through one of three constructors:
//!   * `start_handshake_only` — no tun, just a connectivity probe (safe
//!     to run on a host with live internet).
//!   * `start_with_channels` — caller pumps plaintext packets via mpsc
//!     channels. Used by mobile (iOS NEPacketTunnelProvider, Android
//!     VpnService) where the OS owns the tun.
//!   * `start_with_tun` (POSIX) / `start_with_wintun` (Windows) — the
//!     daemon path; the runtime owns reader+writer tasks that talk to
//!     the OS tun directly.

use std::net::{IpAddr, SocketAddr};
use std::os::raw::c_int;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use tokio::net::UdpSocket;
use tokio::sync::{Notify, RwLock, mpsc};
use tokio::task::JoinHandle;
use tokio::time::interval;

use crate::config::WireGuardExitConfig;

pub const MAX_WG_PACKET: usize = 65_535;
const TIMER_TICK: Duration = Duration::from_millis(250);

/// Default time the daemon / mobile runtime waits for the WG handshake
/// to complete before giving up. Acts as the implicit watchdog: by
/// only swapping the default route after a real handshake, a
/// misconfigured config or unreachable upstream cannot take the host
/// offline.
pub const DAEMON_WG_UPSTREAM_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Handle to a running userspace WG upstream tunnel.
///
/// Drop or call [`Self::shutdown`] to stop the pump tasks. If the
/// runtime owns the tun (POSIX `start_with_tun` or Windows
/// `start_with_wintun`), the platform reader+writer tasks are also
/// aborted here.
pub struct WgUpstreamRuntime {
    pump: Option<JoinHandle<()>>,
    tun_reader: Option<JoinHandle<()>>,
    tun_writer: Option<JoinHandle<()>>,
    handshake: Arc<HandshakeState>,
    upstream: SocketAddr,
    udp_socket_fd: c_int,
}

#[derive(Default)]
struct HandshakeState {
    completed: Notify,
    last_age: RwLock<Option<Duration>>,
}

impl WgUpstreamRuntime {
    /// Probe the WG handshake without creating a tun device.
    /// Safe-by-construction: cannot blackhole the host's internet.
    pub async fn start_handshake_only(config: &WireGuardExitConfig) -> Result<Self> {
        Self::start_with_io(config, None, None).await
    }

    /// Build the runtime with raw mpsc channels for tun I/O. Used by
    /// platforms where the OS owns the tun (iOS NEPacketTunnelProvider,
    /// Android VpnService): the host code feeds plaintext packets into
    /// `tun_in_rx` and reads plaintext packets out of `tun_out_tx`.
    pub async fn start_with_channels(
        config: &WireGuardExitConfig,
        tun_in_rx: mpsc::Receiver<Vec<u8>>,
        tun_out_tx: mpsc::Sender<Vec<u8>>,
    ) -> Result<Self> {
        Self::start_with_io(config, Some((tun_in_rx, tun_out_tx)), None).await
    }

    /// Lower-level constructor used by the desktop daemon: callers
    /// build their own platform-specific tun reader/writer tasks (e.g.
    /// using `boringtun::device::tun::TunSocket` on POSIX or
    /// `wintun::Session` on Windows) and hand them in along with the
    /// matching channel pair. This keeps the platform-specific tun
    /// imports out of `nostr-vpn-core` so the crate continues to build
    /// on mobile without the boringtun `device` feature.
    pub async fn start_with_io(
        config: &WireGuardExitConfig,
        tun_io: Option<(mpsc::Receiver<Vec<u8>>, mpsc::Sender<Vec<u8>>)>,
        tun_handles: Option<(JoinHandle<()>, JoinHandle<()>)>,
    ) -> Result<Self> {
        log_android_info("wg-upstream: start_with_io entered");
        let private = decode_private_key(&config.private_key)?;
        let public = decode_public_key(&config.peer_public_key)?;
        let preshared = decode_optional_preshared_key(&config.peer_preshared_key)?;
        let upstream = resolve_endpoint(&config.endpoint).await?;
        log_android_info(&format!(
            "wg-upstream: keys decoded, upstream resolved to {upstream}"
        ));

        let udp = UdpSocket::bind("0.0.0.0:0")
            .await
            .context("bind upstream WG udp socket")?;
        let udp_socket_fd = raw_udp_socket_fd(&udp);
        log_android_info(&format!(
            "wg-upstream: udp socket bound, fd={udp_socket_fd}"
        ));
        let udp = Arc::new(udp);

        let keepalive = if config.persistent_keepalive_secs == 0 {
            None
        } else {
            Some(config.persistent_keepalive_secs)
        };
        let tunn = Tunn::new(private, public, preshared, keepalive, 1, None);

        let handshake = Arc::new(HandshakeState::default());
        let (tun_in_rx, tun_out_tx) = match tun_io {
            Some((rx, tx)) => (Some(rx), Some(tx)),
            None => (None, None),
        };
        let (tun_reader, tun_writer) = match tun_handles {
            Some((r, w)) => (Some(r), Some(w)),
            None => (None, None),
        };

        let pump = tokio::spawn(run_pump(
            tunn,
            udp,
            upstream,
            tun_in_rx,
            tun_out_tx,
            handshake.clone(),
        ));

        Ok(Self {
            pump: Some(pump),
            tun_reader,
            tun_writer,
            handshake,
            upstream,
            udp_socket_fd,
        })
    }

    /// Wait for at most `timeout` for the WG handshake to complete.
    /// Returns `true` if a handshake was observed; `false` on timeout.
    pub async fn wait_for_handshake(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.handshake.last_age.read().await.is_some() {
                return true;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }
            tokio::select! {
                _ = self.handshake.completed.notified() => continue,
                _ = tokio::time::sleep(remaining) => return false,
            }
        }
    }

    pub fn upstream(&self) -> SocketAddr {
        self.upstream
    }

    /// Raw fd of the UDP socket talking to the WG upstream. On Android
    /// the host should pass this to `VpnService.protect(fd)` so the
    /// encrypted UDP escapes the VPN tun. Returns -1 on platforms
    /// where the underlying socket type doesn't expose a raw fd.
    pub fn udp_socket_fd(&self) -> c_int {
        self.udp_socket_fd
    }

    /// Stop the pump and drop the tunnel state. Idempotent.
    pub async fn shutdown(mut self) {
        if let Some(reader) = self.tun_reader.take() {
            reader.abort();
            let _ = reader.await;
        }
        if let Some(writer) = self.tun_writer.take() {
            writer.abort();
            let _ = writer.await;
        }
        if let Some(pump) = self.pump.take() {
            pump.abort();
            let _ = pump.await;
        }
    }
}

impl Drop for WgUpstreamRuntime {
    fn drop(&mut self) {
        if let Some(reader) = self.tun_reader.take() {
            reader.abort();
        }
        if let Some(writer) = self.tun_writer.take() {
            writer.abort();
        }
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
    }
}

/// Subset of `WireGuardExitConfig` that meaningfully affects the
/// userspace tunnel — used to short-circuit reconcile if nothing
/// changed. We deliberately exclude DNS / MTU since they don't
/// require tearing the WG tunnel down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireGuardExitFingerprint {
    pub enabled: bool,
    pub address: String,
    pub private_key: String,
    pub peer_public_key: String,
    pub peer_preshared_key: String,
    pub endpoint: String,
    pub allowed_ips: Vec<String>,
    pub persistent_keepalive_secs: u16,
}

impl WireGuardExitFingerprint {
    pub fn from_config(config: &WireGuardExitConfig) -> Self {
        Self {
            enabled: config.enabled,
            address: config.address.clone(),
            private_key: config.private_key.clone(),
            peer_public_key: config.peer_public_key.clone(),
            peer_preshared_key: config.peer_preshared_key.clone(),
            endpoint: config.endpoint.clone(),
            allowed_ips: config.allowed_ips.clone(),
            persistent_keepalive_secs: config.persistent_keepalive_secs,
        }
    }
}

enum PumpEvent {
    Timer,
    UdpDatagram { source: IpAddr, payload: Vec<u8> },
    TunPacket(Vec<u8>),
    TunReaderClosed,
}

async fn run_pump(
    mut tunn: Tunn,
    udp: Arc<UdpSocket>,
    upstream: SocketAddr,
    tun_in_rx: Option<mpsc::Receiver<Vec<u8>>>,
    tun_out_tx: Option<mpsc::Sender<Vec<u8>>>,
    handshake: Arc<HandshakeState>,
) {
    log_android_info(&format!(
        "wg-upstream: run_pump starting, upstream={upstream}"
    ));
    {
        let mut buf = vec![0u8; MAX_WG_PACKET];
        match tunn.format_handshake_initiation(&mut buf, false) {
            TunnResult::WriteToNetwork(packet) => {
                let len = packet.len();
                match udp.send_to(packet, upstream).await {
                    Ok(n) => log_android_info(&format!(
                        "wg-upstream: initial handshake init sent ({n}/{len} bytes to {upstream})"
                    )),
                    Err(error) => {
                        log_android_warn(&format!(
                            "wg-upstream: initial handshake send failed: {error}"
                        ));
                        tracing::warn!(?error, "wg-upstream: initial handshake send failed");
                    }
                }
            }
            _ => log_android_info(
                "wg-upstream: format_handshake_initiation returned non-WriteToNetwork",
            ),
        }
    }

    let (event_tx, mut event_rx) = mpsc::channel::<PumpEvent>(256);

    let timer_tx = event_tx.clone();
    let timer_task = tokio::spawn(async move {
        let mut ticker = interval(TIMER_TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if timer_tx.send(PumpEvent::Timer).await.is_err() {
                return;
            }
        }
    });

    let udp_rx_socket = udp.clone();
    let udp_tx = event_tx.clone();
    let udp_task = tokio::spawn(async move {
        let mut buf = vec![0u8; MAX_WG_PACKET];
        let mut count: u32 = 0;
        loop {
            match udp_rx_socket.recv_from(&mut buf).await {
                Ok((n, src)) => {
                    count = count.saturating_add(1);
                    if count <= 3 || count % 50 == 0 {
                        log_android_info(&format!(
                            "wg-upstream: udp recv #{count} from {src} ({n} bytes)"
                        ));
                    }
                    let event = PumpEvent::UdpDatagram {
                        source: src.ip(),
                        payload: buf[..n].to_vec(),
                    };
                    if udp_tx.send(event).await.is_err() {
                        return;
                    }
                }
                Err(error) => {
                    log_android_warn(&format!("wg-upstream: udp recv failed: {error}"));
                    tracing::warn!(?error, "wg-upstream: udp recv failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    });

    let tun_forward_task = tun_in_rx.map(|mut tun_rx| {
        let tun_forward_tx = event_tx.clone();
        tokio::spawn(async move {
            while let Some(packet) = tun_rx.recv().await {
                if tun_forward_tx
                    .send(PumpEvent::TunPacket(packet))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let _ = tun_forward_tx.send(PumpEvent::TunReaderClosed).await;
        })
    });

    drop(event_tx);

    while let Some(event) = event_rx.recv().await {
        match event {
            PumpEvent::Timer => {
                let mut out = vec![0u8; MAX_WG_PACKET];
                let result = tunn.update_timers(&mut out);
                handle_tunn_result(&result, &udp, upstream, tun_out_tx.as_ref()).await;
                drain_decapsulate(&mut tunn, &udp, upstream, tun_out_tx.as_ref()).await;
                let (age, _, _, _, _) = tunn.stats();
                let mut current = handshake.last_age.write().await;
                let prev = current.is_some();
                *current = age;
                if !prev && age.is_some() {
                    handshake.completed.notify_waiters();
                }
            }
            PumpEvent::UdpDatagram { source, payload } => {
                let mut out = vec![0u8; MAX_WG_PACKET];
                let result = tunn.decapsulate(Some(source), &payload, &mut out);
                handle_tunn_result(&result, &udp, upstream, tun_out_tx.as_ref()).await;
                drain_decapsulate(&mut tunn, &udp, upstream, tun_out_tx.as_ref()).await;
                let (age, _, _, _, _) = tunn.stats();
                let mut current = handshake.last_age.write().await;
                let prev = current.is_some();
                *current = age;
                if !prev && age.is_some() {
                    handshake.completed.notify_waiters();
                }
            }
            PumpEvent::TunPacket(packet) => {
                let mut out = vec![0u8; MAX_WG_PACKET];
                let result = tunn.encapsulate(&packet, &mut out);
                handle_tunn_result(&result, &udp, upstream, tun_out_tx.as_ref()).await;
            }
            PumpEvent::TunReaderClosed => break,
        }
    }

    timer_task.abort();
    udp_task.abort();
    if let Some(handle) = tun_forward_task {
        handle.abort();
    }
}

async fn handle_tunn_result(
    result: &TunnResult<'_>,
    udp: &Arc<UdpSocket>,
    upstream: SocketAddr,
    tun_out_tx: Option<&mpsc::Sender<Vec<u8>>>,
) {
    match result {
        TunnResult::Done => {}
        TunnResult::Err(error) => {
            log_android_warn(&format!("wg-upstream: tunn error {error:?}"));
        }
        TunnResult::WriteToNetwork(packet) => {
            if let Err(error) = udp.send_to(packet, upstream).await {
                log_android_warn(&format!(
                    "wg-upstream: udp send failed ({} bytes): {error}",
                    packet.len()
                ));
                tracing::warn!(?error, "wg-upstream: udp send failed");
            }
        }
        TunnResult::WriteToTunnelV4(packet, _) | TunnResult::WriteToTunnelV6(packet, _) => {
            let len = packet.len();
            if let Some(tx) = tun_out_tx {
                if let Err(error) = tx.try_send(packet.to_vec()) {
                    log_android_warn(&format!(
                        "wg-upstream: tun_out send failed ({len} bytes): {error}"
                    ));
                }
            } else {
                log_android_warn(&format!(
                    "wg-upstream: dropped {len}-byte plaintext (no tun_out_tx)"
                ));
            }
        }
    }
}

async fn drain_decapsulate(
    tunn: &mut Tunn,
    udp: &Arc<UdpSocket>,
    upstream: SocketAddr,
    tun_out_tx: Option<&mpsc::Sender<Vec<u8>>>,
) {
    loop {
        let mut out = vec![0u8; MAX_WG_PACKET];
        let result = tunn.decapsulate(None, &[], &mut out);
        match &result {
            TunnResult::Done | TunnResult::Err(_) => return,
            _ => handle_tunn_result(&result, udp, upstream, tun_out_tx).await,
        }
    }
}

// Platform-specific tun reader/writer tasks (POSIX TunSocket, Windows
// WinTun) live in nostr-vpn-cli, where the boringtun `device` feature
// is enabled. Mobile callers don't need them — they use
// `start_with_channels` and feed packets directly from the OS-managed
// tun (NEPacketTunnelProvider on iOS, VpnService on Android).

fn decode_private_key(encoded: &str) -> Result<StaticSecret> {
    let raw = decode_key_bytes(encoded.trim()).context("invalid WG private key")?;
    Ok(StaticSecret::from(raw))
}

fn decode_public_key(encoded: &str) -> Result<PublicKey> {
    let raw = decode_key_bytes(encoded.trim()).context("invalid WG public key")?;
    Ok(PublicKey::from(raw))
}

fn decode_optional_preshared_key(encoded: &str) -> Result<Option<[u8; 32]>> {
    let trimmed = encoded.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        decode_key_bytes(trimmed).context("invalid WG preshared key")?,
    ))
}

fn decode_key_bytes(encoded: &str) -> Result<[u8; 32]> {
    let raw = STANDARD
        .decode(encoded)
        .map_err(|_| anyhow!("base64 decode failed"))?;
    raw.try_into()
        .map_err(|_| anyhow!("WG key must be exactly 32 bytes"))
}

async fn resolve_endpoint(endpoint: &str) -> Result<SocketAddr> {
    let endpoint = endpoint.trim();
    if let Ok(addr) = endpoint.parse::<SocketAddr>() {
        return Ok(addr);
    }
    let resolved = tokio::net::lookup_host(endpoint)
        .await
        .with_context(|| format!("resolve WG upstream endpoint '{endpoint}'"))?
        .next()
        .ok_or_else(|| anyhow!("no DNS results for '{endpoint}'"))?;
    Ok(resolved)
}

#[cfg(unix)]
fn raw_udp_socket_fd(socket: &UdpSocket) -> c_int {
    use std::os::unix::io::AsRawFd;
    socket.as_raw_fd() as c_int
}

#[cfg(not(unix))]
fn raw_udp_socket_fd(_socket: &UdpSocket) -> c_int {
    -1
}

// Direct Android logcat bridge so the WG pump's diagnostic messages
// actually surface during device testing — Rust stderr/stdout on
// Android is redirected to /dev/null by default, and the existing
// `tracing` macros silently no-op without a registered subscriber.
// Cheap, self-contained, no extra dep.

#[cfg(target_os = "android")]
fn log_android(prio: i32, message: &str) {
    use std::ffi::CString;
    let tag = CString::new("nvpn-wg").unwrap_or_default();
    if let Ok(msg) = CString::new(message) {
        unsafe {
            __android_log_write(prio, tag.as_ptr(), msg.as_ptr());
        }
    }
}

#[cfg(target_os = "android")]
fn log_android_info(message: &str) {
    log_android(4 /* ANDROID_LOG_INFO */, message);
}

#[cfg(target_os = "android")]
fn log_android_warn(message: &str) {
    log_android(5 /* ANDROID_LOG_WARN */, message);
}

#[cfg(not(target_os = "android"))]
fn log_android_info(_message: &str) {}

#[cfg(not(target_os = "android"))]
fn log_android_warn(_message: &str) {}

#[cfg(target_os = "android")]
unsafe extern "C" {
    fn __android_log_write(
        prio: i32,
        tag: *const std::os::raw::c_char,
        text: *const std::os::raw::c_char,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_wireguard_exit_config;

    fn random_keypair() -> (StaticSecret, PublicKey, String, String) {
        // Deterministic but unique per call. boringtun + x25519-dalek
        // accept any 32-byte little-endian secret; ChaCha20-style
        // clamping is applied internally on use.
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let mut bytes = [0u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = (nanos as u8).wrapping_add(i as u8 * 7);
        }
        let private = StaticSecret::from(bytes);
        let public = PublicKey::from(&private);
        let priv_b64 = STANDARD.encode(private.to_bytes());
        let pub_b64 = STANDARD.encode(public.as_bytes());
        (private, public, priv_b64, pub_b64)
    }

    /// Stand up a paired Tunn on a real UDP port acting as the upstream
    /// "server"; verifies the boringtun pump's handshake state machine
    /// drives `wait_for_handshake` to true.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn handshake_completes_against_paired_responder() {
        let (_, _client_pub, client_priv_b64, _) = random_keypair();
        let (server_priv_obj, _, _, server_pub_b64) = random_keypair();

        let server_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server_addr = server_socket.local_addr().unwrap();
        let server_socket = Arc::new(server_socket);

        let mut server_tunn = Tunn::new(
            server_priv_obj,
            PublicKey::from(&decode_private_key(&client_priv_b64).unwrap()),
            None,
            Some(25),
            2,
            None,
        );

        let server_socket_pump = server_socket.clone();
        let server_pump = tokio::spawn(async move {
            let mut udp_buf = vec![0u8; MAX_WG_PACKET];
            for _ in 0..32 {
                let (n, src) = match tokio::time::timeout(
                    Duration::from_millis(500),
                    server_socket_pump.recv_from(&mut udp_buf),
                )
                .await
                {
                    Ok(Ok(value)) => value,
                    _ => continue,
                };
                let mut out = vec![0u8; MAX_WG_PACKET];
                let to_send = match server_tunn.decapsulate(Some(src.ip()), &udp_buf[..n], &mut out)
                {
                    TunnResult::WriteToNetwork(packet) => Some(packet.to_vec()),
                    _ => None,
                };
                if let Some(bytes) = to_send {
                    let _ = server_socket_pump.send_to(&bytes, src).await;
                }
                loop {
                    let mut drain_buf = vec![0u8; MAX_WG_PACKET];
                    let drained = match server_tunn.decapsulate(None, &[], &mut drain_buf) {
                        TunnResult::WriteToNetwork(packet) => Some(packet.to_vec()),
                        _ => None,
                    };
                    let Some(bytes) = drained else { break };
                    let _ = server_socket_pump.send_to(&bytes, src).await;
                }
            }
        });

        let cfg_text = format!(
            "[Interface]\nPrivateKey = {client_priv_b64}\nAddress = 10.99.99.2/32\n\n[Peer]\nPublicKey = {server_pub_b64}\nEndpoint = {server_addr}\nAllowedIPs = 0.0.0.0/0\nPersistentKeepalive = 1\n"
        );
        let cfg = parse_wireguard_exit_config(&cfg_text).expect("parse WG config");

        let runtime = WgUpstreamRuntime::start_handshake_only(&cfg)
            .await
            .expect("start runtime");
        let ok = runtime.wait_for_handshake(Duration::from_secs(10)).await;
        runtime.shutdown().await;
        server_pump.abort();
        let _ = server_pump.await;
        assert!(
            ok,
            "expected handshake to complete against the paired responder"
        );
    }
}
