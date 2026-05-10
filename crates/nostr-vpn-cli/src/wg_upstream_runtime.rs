//! Userspace WireGuard upstream runtime — single-peer (Mullvad/Proton-style).
//!
//! Wraps `boringtun::noise::Tunn` for the case where this device is a WG
//! *client* of one upstream provider, not a multi-peer mesh participant.
//! The same shape used to live in tree as `userspace_wg.rs` until commit
//! `2ca833c`; this is the slimmed single-peer reincarnation we use to bring
//! WireGuard upstream support to macOS (and any platform without a kernel
//! WG implementation handy).
//!
//! The runtime owns:
//!   * a `Tunn` (encrypts/decrypts packets, runs the handshake state machine)
//!   * a UDP socket bound to ephemeral 0.0.0.0:0 that talks to the upstream
//!   * optionally a `TunSocket` (utun on macOS / tun on Linux). Without one,
//!     this is just a "did the handshake succeed" probe — useful for the
//!     first integration test before any platform routing is attempted.
//!
//! Three concurrent jobs drive it, all dispatched in a single tokio
//! `select!` loop so the `Tunn` doesn't need a mutex:
//!   * UDP-rx: receive ciphertext from upstream → `Tunn::decapsulate` → write
//!     plaintext to tun (and drain queued outputs).
//!   * tun-rx: receive plaintext from the kernel → `Tunn::encapsulate` →
//!     send ciphertext to upstream.
//!   * timer: every 250ms call `Tunn::update_timers` so the handshake +
//!     keepalive state machine can re-key / re-init on schedule.

use std::net::{IpAddr, SocketAddr};
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use nostr_vpn_core::config::WireGuardExitConfig;
use tokio::net::UdpSocket;
use tokio::sync::{Notify, RwLock, mpsc};
use tokio::task::JoinHandle;
use tokio::time::interval;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use boringtun::device::tun::TunSocket;

const MAX_WG_PACKET: usize = 65_535;
const TIMER_TICK: Duration = Duration::from_millis(250);

/// Handle to a running userspace WG upstream tunnel.
///
/// Drop or call [`Self::shutdown`] to stop the pump tasks. The owned
/// `TunSocket` (if any) is dropped here too, which on macOS deletes the
/// utun device.
pub struct WgUpstreamRuntime {
    pump: Option<JoinHandle<()>>,
    tun_reader: Option<JoinHandle<()>>,
    handshake: Arc<HandshakeState>,
    upstream: SocketAddr,
}

#[derive(Default)]
struct HandshakeState {
    completed: Notify,
    last_age: RwLock<Option<Duration>>,
}

impl WgUpstreamRuntime {
    /// Build the runtime *without* a tun device — i.e. just enough to
    /// drive the WG handshake against the upstream and prove the keys +
    /// connectivity work. Useful as a smoke test before any platform
    /// routing is attempted.
    pub async fn start_handshake_only(config: &WireGuardExitConfig) -> Result<Self> {
        Self::start_inner(config, None).await
    }

    /// Build the runtime with a tun device attached. Plaintext packets
    /// read from the tun get encapsulated and sent to the upstream;
    /// decrypted packets from the upstream get written back to the tun.
    /// The caller is responsible for assigning the tun's IP / MTU /
    /// routes — this runtime only does the data plane.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub async fn start_with_tun(config: &WireGuardExitConfig, tun: Arc<TunSocket>) -> Result<Self> {
        Self::start_inner(config, Some(tun)).await
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    async fn start_inner(
        config: &WireGuardExitConfig,
        tun: Option<Arc<TunSocket>>,
    ) -> Result<Self> {
        let private = decode_private_key(&config.private_key)?;
        let public = decode_public_key(&config.peer_public_key)?;
        let preshared = decode_optional_preshared_key(&config.peer_preshared_key)?;
        let upstream = resolve_endpoint(&config.endpoint).await?;

        let udp = UdpSocket::bind("0.0.0.0:0")
            .await
            .context("bind upstream WG udp socket")?;
        let udp = Arc::new(udp);

        let keepalive = if config.persistent_keepalive_secs == 0 {
            None
        } else {
            Some(config.persistent_keepalive_secs)
        };
        let tunn = Tunn::new(private, public, preshared, keepalive, 1, None);

        // tun → loop fan-in. Spawned producer reads the tun and pushes
        // plaintext packets into this channel; the main loop pulls and
        // encapsulates them. Channel decouples blocking-ish tun reads
        // from the tunn-bearing loop. Closing this sender (which
        // happens when we drop the runtime + abort the reader task) is
        // also how the pump observes shutdown.
        let (tun_tx, tun_rx) = mpsc::channel::<Vec<u8>>(256);
        let handshake = Arc::new(HandshakeState::default());

        let tun_reader = tun.clone().map(|tun| spawn_tun_reader(tun, tun_tx));

        let pump = tokio::spawn(run_pump(
            tunn,
            udp,
            upstream,
            tun,
            tun_rx,
            handshake.clone(),
        ));

        Ok(Self {
            pump: Some(pump),
            tun_reader,
            handshake,
            upstream,
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    async fn start_inner(_config: &WireGuardExitConfig, _tun: Option<()>) -> Result<Self> {
        Err(anyhow!(
            "userspace WG upstream runtime is only supported on Linux and macOS for now"
        ))
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

    /// Stop the pump and drop the tunnel state. Idempotent.
    pub async fn shutdown(mut self) {
        if let Some(reader) = self.tun_reader.take() {
            reader.abort();
            let _ = reader.await;
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
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
    }
}

/// Events fed into the coordinator task. We funnel UDP-rx, tun-rx, and the
/// timer through a single mpsc instead of using `tokio::select!` directly
/// so the `Tunn` (which is `!Send`-friendly but still requires `&mut self`
/// per call) is owned by exactly one task and we don't have to wrestle
/// with the `select!` borrow surface across `&mut udp_buf` and other arms.
#[cfg(any(target_os = "linux", target_os = "macos"))]
enum PumpEvent {
    Timer,
    UdpDatagram {
        source: std::net::IpAddr,
        payload: Vec<u8>,
    },
    TunPacket(Vec<u8>),
    TunReaderClosed,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn run_pump(
    mut tunn: Tunn,
    udp: Arc<UdpSocket>,
    upstream: SocketAddr,
    tun: Option<Arc<TunSocket>>,
    mut tun_rx: mpsc::Receiver<Vec<u8>>,
    handshake: Arc<HandshakeState>,
) {
    // Kick off the handshake immediately so the upstream sees us and we
    // don't have to wait for the first plaintext packet (which might
    // never arrive in handshake-only mode).
    {
        let mut buf = vec![0u8; MAX_WG_PACKET];
        if let TunnResult::WriteToNetwork(packet) =
            tunn.format_handshake_initiation(&mut buf, false)
        {
            if let Err(error) = udp.send_to(packet, upstream).await {
                tracing::warn!(?error, "wg-upstream: initial handshake send failed");
            }
        }
    }

    let (event_tx, mut event_rx) = mpsc::channel::<PumpEvent>(256);

    // Feeder: timer. Drives `Tunn::update_timers` so handshake
    // re-init / keepalives / cookies happen on schedule.
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

    // Feeder: UDP receive from the upstream WG endpoint.
    let udp_rx_socket = udp.clone();
    let udp_tx = event_tx.clone();
    let udp_task = tokio::spawn(async move {
        let mut buf = vec![0u8; MAX_WG_PACKET];
        loop {
            match udp_rx_socket.recv_from(&mut buf).await {
                Ok((n, src)) => {
                    let event = PumpEvent::UdpDatagram {
                        source: src.ip(),
                        payload: buf[..n].to_vec(),
                    };
                    if udp_tx.send(event).await.is_err() {
                        return;
                    }
                }
                Err(error) => {
                    tracing::warn!(?error, "wg-upstream: udp recv failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    });

    // Feeder: tun receive (forwarded from the per-tun async-fd reader
    // task that the runtime started before us). In handshake-only mode
    // we still get the receiver but the matching sender was dropped
    // immediately, so don't bother forwarding — and definitely don't
    // emit a TunReaderClosed event the moment we start, which would
    // immediately tear the coordinator down.
    let tun_forward_task = if tun.is_some() {
        let tun_forward_tx = event_tx.clone();
        Some(tokio::spawn(async move {
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
        }))
    } else {
        None
    };

    // Drop the cloned sender so the channel closes when all feeders exit.
    drop(event_tx);

    // Coordinator: own the Tunn, dispatch events sequentially. This is
    // where every &mut Tunn call lives, so the Tunn never needs a mutex
    // and we don't fight the borrow checker on `select!` arms.
    while let Some(event) = event_rx.recv().await {
        match event {
            PumpEvent::Timer => {
                let mut out = vec![0u8; MAX_WG_PACKET];
                let result = tunn.update_timers(&mut out);
                handle_tunn_result(&result, &udp, upstream, tun.as_deref()).await;
                drain_decapsulate(&mut tunn, &udp, upstream, tun.as_deref()).await;
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
                handle_tunn_result(&result, &udp, upstream, tun.as_deref()).await;
                drain_decapsulate(&mut tunn, &udp, upstream, tun.as_deref()).await;
                // Update handshake-completion eagerly on UDP rx as well —
                // boringtun establishes the session as a side-effect of
                // decapsulate. Don't wait for the next 250ms timer tick.
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
                handle_tunn_result(&result, &udp, upstream, tun.as_deref()).await;
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn handle_tunn_result(
    result: &TunnResult<'_>,
    udp: &Arc<UdpSocket>,
    upstream: SocketAddr,
    tun: Option<&TunSocket>,
) {
    match result {
        TunnResult::Done | TunnResult::Err(_) => {}
        TunnResult::WriteToNetwork(packet) => {
            if let Err(error) = udp.send_to(packet, upstream).await {
                tracing::warn!(?error, "wg-upstream: udp send failed");
            }
        }
        TunnResult::WriteToTunnelV4(packet, _) => {
            if let Some(tun) = tun {
                let _ = tun.write4(packet);
            }
        }
        TunnResult::WriteToTunnelV6(packet, _) => {
            if let Some(tun) = tun {
                let _ = tun.write6(packet);
            }
        }
    }
}

/// After `decapsulate(Some(src), ciphertext, ...)` produces a result,
/// boringtun may still hold queued outbound packets that need to be
/// flushed via `decapsulate(None, &[], ...)` calls. Loop until Done.
#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn drain_decapsulate(
    tunn: &mut Tunn,
    udp: &Arc<UdpSocket>,
    upstream: SocketAddr,
    tun: Option<&TunSocket>,
) {
    loop {
        let mut out = vec![0u8; MAX_WG_PACKET];
        let result = tunn.decapsulate(None, &[], &mut out);
        match &result {
            TunnResult::Done | TunnResult::Err(_) => return,
            _ => handle_tunn_result(&result, udp, upstream, tun).await,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_tun_reader(tun: Arc<TunSocket>, tun_tx: mpsc::Sender<Vec<u8>>) -> JoinHandle<()> {
    use std::os::unix::io::{AsRawFd, RawFd};
    use tokio::io::Interest;
    use tokio::io::unix::AsyncFd;

    struct BorrowedFd(RawFd);
    impl AsRawFd for BorrowedFd {
        fn as_raw_fd(&self) -> RawFd {
            self.0
        }
    }

    tokio::spawn(async move {
        let async_fd = match AsyncFd::with_interest(BorrowedFd(tun.as_raw_fd()), Interest::READABLE)
        {
            Ok(fd) => fd,
            Err(error) => {
                tracing::warn!(?error, "wg-upstream: failed to register tun fd");
                return;
            }
        };
        let mut buf = vec![0u8; MAX_WG_PACKET];
        loop {
            let mut guard = match async_fd.readable().await {
                Ok(g) => g,
                Err(error) => {
                    tracing::warn!(?error, "wg-upstream: tun reactor error");
                    return;
                }
            };
            match tun.read(&mut buf) {
                Ok(packet) if packet.is_empty() => guard.clear_ready(),
                Ok(packet) => {
                    let bytes = packet.to_vec();
                    if tun_tx.send(bytes).await.is_err() {
                        return;
                    }
                }
                Err(_) => guard.clear_ready(),
            }
        }
    })
}

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

/// Bring up a userspace WG tun interface and install **only** a single
/// host route via it. Default route is not touched, so this is safe to
/// run on a host with live internet — even if the WG handshake fails,
/// the worst case is that the one scoped target becomes unreachable.
///
/// Returns a `ScopedHostRoute` guard that, when dropped, removes the
/// route. The caller should also drop the `TunSocket` to delete the
/// tun device (utun on macOS auto-vanishes when the fd closes).
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn apply_scoped_host_route(
    iface: &str,
    address: &str,
    target: IpAddr,
    mtu: u16,
) -> Result<ScopedHostRoute> {
    let target_str = target.to_string();
    let address_ip = address
        .split('/')
        .next()
        .ok_or_else(|| anyhow!("empty WG tunnel address"))?
        .to_string();
    let mtu_str = mtu.to_string();

    #[cfg(target_os = "linux")]
    {
        run_checked(
            ProcessCommand::new("ip")
                .arg("address")
                .arg("replace")
                .arg(format!("{address_ip}/32"))
                .arg("dev")
                .arg(iface),
        )?;
        run_checked(
            ProcessCommand::new("ip")
                .arg("link")
                .arg("set")
                .arg("mtu")
                .arg(&mtu_str)
                .arg("up")
                .arg("dev")
                .arg(iface),
        )?;
        run_checked(
            ProcessCommand::new("ip")
                .arg("route")
                .arg("replace")
                .arg(format!("{target_str}/32"))
                .arg("dev")
                .arg(iface),
        )?;
        return Ok(ScopedHostRoute {
            iface: iface.to_string(),
            target,
        });
    }

    #[cfg(target_os = "macos")]
    {
        // ifconfig <iface> inet <addr> <addr> netmask 255.255.255.255 mtu N up
        run_checked(
            ProcessCommand::new("ifconfig")
                .arg(iface)
                .arg("inet")
                .arg(&address_ip)
                .arg(&address_ip)
                .arg("netmask")
                .arg("255.255.255.255")
                .arg("mtu")
                .arg(&mtu_str)
                .arg("up"),
        )?;
        // route add -host <target> -interface <iface>
        run_checked(
            ProcessCommand::new("route")
                .arg("-n")
                .arg("add")
                .arg("-host")
                .arg(&target_str)
                .arg("-interface")
                .arg(iface),
        )?;
        return Ok(ScopedHostRoute {
            iface: iface.to_string(),
            target,
        });
    }

    #[allow(unreachable_code)]
    Err(anyhow!(
        "scoped host route is only implemented on Linux and macOS"
    ))
}

/// Drop guard that removes the host route installed by
/// [`apply_scoped_host_route`]. Idempotent and best-effort: if the
/// route was already gone (or the tun device disappeared first, taking
/// its routes with it), this just logs.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub struct ScopedHostRoute {
    iface: String,
    target: IpAddr,
}

/// Full default-route replacement: bring up the userspace WG tun and
/// route **all** outbound traffic through it (Mullvad/Proton-style),
/// while installing a bypass /32 route for the WG endpoint itself so
/// the encrypted UDP keeps escaping via the original default route.
///
/// **This is the dangerous mode** — if the WG handshake fails after
/// this call returns, the host has lost its way to the internet
/// except through a tunnel that doesn't work. The caller is expected
/// to either:
///   1. Wait for handshake completion (with a timeout) BEFORE calling
///      this, so we only swap the default once we know the tunnel is
///      live, OR
///   2. Spawn a watchdog that drops the returned guard if the
///      handshake doesn't complete within a few seconds.
///
/// The returned guard restores the original default route + deletes
/// the bypass on Drop, even on panic.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn apply_full_default_route(
    iface: &str,
    address: &str,
    upstream_endpoint: SocketAddr,
    mtu: u16,
) -> Result<FullDefaultRoute> {
    let upstream_ip = upstream_endpoint.ip();
    let address_ip = address
        .split('/')
        .next()
        .ok_or_else(|| anyhow!("empty WG tunnel address"))?
        .to_string();
    let mtu_str = mtu.to_string();

    // 1. Bring up the tun with the WG tunnel IP.
    #[cfg(target_os = "linux")]
    {
        run_checked(
            ProcessCommand::new("ip")
                .arg("address")
                .arg("replace")
                .arg(format!("{address_ip}/32"))
                .arg("dev")
                .arg(iface),
        )?;
        run_checked(
            ProcessCommand::new("ip")
                .arg("link")
                .arg("set")
                .arg("mtu")
                .arg(&mtu_str)
                .arg("up")
                .arg("dev")
                .arg(iface),
        )?;
    }
    #[cfg(target_os = "macos")]
    {
        run_checked(
            ProcessCommand::new("ifconfig")
                .arg(iface)
                .arg("inet")
                .arg(&address_ip)
                .arg(&address_ip)
                .arg("netmask")
                .arg("255.255.255.255")
                .arg("mtu")
                .arg(&mtu_str)
                .arg("up"),
        )?;
    }

    // 2. Capture the original default route so we can restore it on
    // Drop. Do this before touching anything routing-related.
    let original_default = capture_default_route()?;

    // 3. Install the bypass /32 for the WG endpoint via the original
    // default gateway. This MUST exist before we swap the default,
    // otherwise the encrypted UDP would loop back into the tun.
    install_endpoint_bypass(&upstream_ip, &original_default)?;

    // 4. Swap the default route to the WG tun.
    install_default_via_iface(iface, &address_ip)?;

    Ok(FullDefaultRoute {
        iface: iface.to_string(),
        bypass_target: upstream_ip,
        original_default,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub struct FullDefaultRoute {
    iface: String,
    bypass_target: IpAddr,
    original_default: CapturedDefaultRoute,
}

/// Captured underlay default route, used to restore on Drop. The
/// shape differs by platform — Linux carries the raw `ip route show
/// default` line so `ip route replace` puts it back verbatim; macOS
/// carries gateway + interface so we can call `route add default`.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug, Clone)]
struct CapturedDefaultRoute {
    #[cfg(target_os = "linux")]
    line: String,
    #[cfg(target_os = "macos")]
    gateway: String,
    #[cfg(target_os = "macos")]
    interface: String,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn capture_default_route() -> Result<CapturedDefaultRoute> {
    #[cfg(target_os = "linux")]
    {
        let output = ProcessCommand::new("ip")
            .arg("-4")
            .arg("route")
            .arg("show")
            .arg("default")
            .output()
            .context("ip route show default")?;
        if !output.status.success() {
            return Err(anyhow!("ip route show default exited {}", output.status));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Take the first non-empty line, prefer one that doesn't go
        // through a utun (in case a previous run left a stale default
        // there; this is the kind of state the watchdog protects
        // against, but capturing the wrong one would be terminal).
        let line = stdout
            .lines()
            .find(|line| {
                let line = line.trim();
                !line.is_empty()
                    && !line.contains(" dev utun")
                    && !line.contains(" dev wg-")
                    && !line.contains(" dev nvpn-wg")
            })
            .or_else(|| stdout.lines().find(|line| !line.trim().is_empty()))
            .ok_or_else(|| anyhow!("no IPv4 default route found"))?
            .trim()
            .to_string();
        Ok(CapturedDefaultRoute { line })
    }
    #[cfg(target_os = "macos")]
    {
        let output = ProcessCommand::new("netstat")
            .arg("-rn")
            .arg("-f")
            .arg("inet")
            .output()
            .context("netstat -rn -f inet")?;
        if !output.status.success() {
            return Err(anyhow!("netstat exited {}", output.status));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Find a "default <gateway> ... <interface>" row whose
        // interface is not a utun (filter out our own / stale tunnels).
        for line in stdout.lines() {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens.len() < 4 || tokens[0] != "default" {
                continue;
            }
            let gateway = tokens[1];
            // last token is the interface name on macOS netstat -rn output.
            let interface = tokens.last().copied().unwrap_or("");
            if interface.starts_with("utun")
                || interface.starts_with("bridge")
                || interface == "lo0"
            {
                continue;
            }
            if gateway.starts_with("link#") {
                continue;
            }
            return Ok(CapturedDefaultRoute {
                gateway: gateway.to_string(),
                interface: interface.to_string(),
            });
        }
        Err(anyhow!(
            "no underlay IPv4 default route found in netstat output"
        ))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn install_endpoint_bypass(target: &IpAddr, original: &CapturedDefaultRoute) -> Result<()> {
    let target_str = target.to_string();
    #[cfg(target_os = "linux")]
    {
        // Reuse the captured `ip route show default` line, just swap
        // the destination from "default" to the host IP. e.g.
        //   "default via 192.168.1.1 dev en0 ..." →
        //   "192.168.1.1/32 ..." with the rest preserved.
        let after_default = original
            .line
            .strip_prefix("default ")
            .unwrap_or(&original.line)
            .trim();
        let mut command = ProcessCommand::new("ip");
        command
            .arg("route")
            .arg("replace")
            .arg(format!("{target_str}/32"));
        for arg in after_default.split_whitespace() {
            command.arg(arg);
        }
        run_checked(&mut command)?;
    }
    #[cfg(target_os = "macos")]
    {
        // route add -host <ip> <gateway> -ifscope <iface>
        run_checked(
            ProcessCommand::new("route")
                .arg("-n")
                .arg("add")
                .arg("-host")
                .arg(&target_str)
                .arg(&original.gateway)
                .arg("-ifscope")
                .arg(&original.interface),
        )?;
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn install_default_via_iface(iface: &str, _src: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        // Replace the default route to go via the WG iface.
        run_checked(
            ProcessCommand::new("ip")
                .arg("-4")
                .arg("route")
                .arg("replace")
                .arg("default")
                .arg("dev")
                .arg(iface)
                .arg("src")
                .arg(_src),
        )?;
    }
    #[cfg(target_os = "macos")]
    {
        // macOS default-route replacement: delete the existing default
        // first (route add default would otherwise fail with "File
        // exists"), then add via the tun interface.
        let _ = ProcessCommand::new("route")
            .arg("-n")
            .arg("delete")
            .arg("default")
            .status();
        run_checked(
            ProcessCommand::new("route")
                .arg("-n")
                .arg("add")
                .arg("default")
                .arg("-interface")
                .arg(iface),
        )?;
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FullDefaultRoute {
    /// Cleanup explicitly. Returning a `Result` lets the caller see
    /// what failed; on Drop the result is ignored.
    pub fn revert(&mut self) -> Result<()> {
        let target_str = self.bypass_target.to_string();
        // Restore the original default route FIRST so the host has a
        // working route to the internet again before we delete the
        // bypass for the WG endpoint.
        #[cfg(target_os = "linux")]
        {
            // `ip route replace` is idempotent: it'll overwrite
            // whatever the default currently is (likely "dev <wg
            // iface>").
            let mut command = ProcessCommand::new("ip");
            command.arg("route").arg("replace");
            for arg in self.original_default.line.split_whitespace() {
                command.arg(arg);
            }
            run_checked(&mut command)?;
            let _ = ProcessCommand::new("ip")
                .arg("route")
                .arg("del")
                .arg(format!("{target_str}/32"))
                .status();
        }
        #[cfg(target_os = "macos")]
        {
            let _ = ProcessCommand::new("route")
                .arg("-n")
                .arg("delete")
                .arg("default")
                .arg("-interface")
                .arg(&self.iface)
                .status();
            run_checked(
                ProcessCommand::new("route")
                    .arg("-n")
                    .arg("add")
                    .arg("default")
                    .arg(&self.original_default.gateway)
                    .arg("-ifscope")
                    .arg(&self.original_default.interface),
            )?;
            let _ = ProcessCommand::new("route")
                .arg("-n")
                .arg("delete")
                .arg("-host")
                .arg(&target_str)
                .status();
        }
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for FullDefaultRoute {
    fn drop(&mut self) {
        if let Err(error) = self.revert() {
            eprintln!(
                "wg-upstream: WARNING — failed to restore default route on cleanup: {error}. \
                 You may need to run `route delete default && route add default <gateway>` \
                 (macOS) or `ip route replace {}` (Linux) manually.",
                self.original_default_repr()
            );
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FullDefaultRoute {
    fn original_default_repr(&self) -> String {
        #[cfg(target_os = "linux")]
        {
            self.original_default.line.clone()
        }
        #[cfg(target_os = "macos")]
        {
            format!(
                "default via {} -ifscope {}",
                self.original_default.gateway, self.original_default.interface
            )
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for ScopedHostRoute {
    fn drop(&mut self) {
        let target = self.target.to_string();
        #[cfg(target_os = "linux")]
        {
            let _ = ProcessCommand::new("ip")
                .arg("route")
                .arg("del")
                .arg(format!("{target}/32"))
                .arg("dev")
                .arg(&self.iface)
                .status();
        }
        #[cfg(target_os = "macos")]
        {
            let _ = ProcessCommand::new("route")
                .arg("-n")
                .arg("delete")
                .arg("-host")
                .arg(&target)
                .arg("-interface")
                .arg(&self.iface)
                .status();
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_checked(command: &mut ProcessCommand) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("spawn {:?}", command.get_program()))?;
    if !status.success() {
        return Err(anyhow!(
            "{:?} {:?} failed: {status}",
            command.get_program(),
            command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        ));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use boringtun::x25519::PublicKey;
    use nostr_vpn_core::config::parse_wireguard_exit_config;

    fn random_keypair() -> (StaticSecret, PublicKey, String, String) {
        let bytes: [u8; 32] = rand::random();
        let private = StaticSecret::from(bytes);
        let public = PublicKey::from(&private);
        let priv_b64 = STANDARD.encode(private.to_bytes());
        let pub_b64 = STANDARD.encode(public.as_bytes());
        (private, public, priv_b64, pub_b64)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn handshake_completes_against_paired_responder() {
        let (_, _client_pub, client_priv_b64, _client_pub_b64) = random_keypair();
        let (server_priv_obj, _, _, server_pub_b64) = random_keypair();

        // Stand up a paired Tunn on a real UDP port acting as the
        // "server"; this is enough to drive the handshake without
        // wiring up any actual tun.
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
                // Decapsulate the inbound datagram, then drain queued
                // outputs via decapsulate(None, &[], ...) until Done.
                let to_send = match server_tunn.decapsulate(Some(src.ip()), &udp_buf[..n], &mut out)
                {
                    TunnResult::WriteToNetwork(packet) => Some(packet.to_vec()),
                    _ => None,
                };
                if let Some(bytes) = to_send {
                    let _ = server_socket_pump.send_to(&bytes, src).await;
                }
                // Drain any queued outputs (handshake protocol can stack
                // packets in the boringtun internal queue).
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

        // Compose a WG config that points at the local "server"
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
