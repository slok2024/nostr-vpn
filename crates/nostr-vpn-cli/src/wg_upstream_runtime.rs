//! Platform-routing helpers around the shared userspace WG runtime.
//!
//! The boringtun pump itself lives in `nostr_vpn_core::wg_upstream`
//! (so mobile + desktop both use the same tunnel state machine). This
//! module is the desktop-only glue: routing-table swaps, default-route
//! capture/restore, scoped host routes for the test command, and the
//! `DaemonWgUpstream` lifecycle holder that the daemon's reconcile
//! loop owns.

use std::net::{IpAddr, SocketAddr};
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use boringtun::device::tun::TunSocket;
#[cfg(target_os = "windows")]
use wintun::Session as WintunSession;

use nostr_vpn_core::config::WireGuardExitConfig;
pub use nostr_vpn_core::wg_upstream::{
    DAEMON_WG_UPSTREAM_HANDSHAKE_TIMEOUT, MAX_WG_PACKET, WgUpstreamRuntime,
    WireGuardExitFingerprint,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Spin up a userspace WG runtime over a POSIX `TunSocket` (Linux tun
/// or macOS utun). Builds the platform-specific reader+writer tasks
/// here so `nostr-vpn-core` doesn't need the boringtun `device`
/// feature (which doesn't compile on iOS/Android).
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub async fn start_wg_runtime_with_posix_tun(
    config: &WireGuardExitConfig,
    tun: Arc<TunSocket>,
) -> Result<WgUpstreamRuntime> {
    let (in_tx, in_rx) = mpsc::channel::<Vec<u8>>(256);
    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(256);
    let reader = spawn_posix_tun_reader(tun.clone(), in_tx);
    let writer = spawn_posix_tun_writer(tun, out_rx);
    WgUpstreamRuntime::start_with_io(config, Some((in_rx, out_tx)), Some((reader, writer))).await
}

/// Same idea for Windows WinTun.
#[cfg(target_os = "windows")]
pub async fn start_wg_runtime_with_wintun(
    config: &WireGuardExitConfig,
    session: Arc<WintunSession>,
) -> Result<WgUpstreamRuntime> {
    let (in_tx, in_rx) = mpsc::channel::<Vec<u8>>(256);
    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(256);
    let reader = spawn_wintun_reader(session.clone(), in_tx);
    let writer = spawn_wintun_writer(session, out_rx);
    WgUpstreamRuntime::start_with_io(config, Some((in_rx, out_tx)), Some((reader, writer))).await
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_posix_tun_reader(tun: Arc<TunSocket>, tun_tx: mpsc::Sender<Vec<u8>>) -> JoinHandle<()> {
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
                Ok([]) => guard.clear_ready(),
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_posix_tun_writer(tun: Arc<TunSocket>, mut rx: mpsc::Receiver<Vec<u8>>) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(packet) = rx.recv().await {
            match packet.first().map(|byte| byte >> 4) {
                Some(4) => {
                    let _ = tun.write4(&packet);
                }
                Some(6) => {
                    let _ = tun.write6(&packet);
                }
                _ => {}
            }
        }
    })
}

#[cfg(target_os = "windows")]
fn spawn_wintun_reader(
    session: Arc<WintunSession>,
    tun_tx: mpsc::Sender<Vec<u8>>,
) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        loop {
            match session.receive_blocking() {
                Ok(packet) => {
                    let bytes = packet.bytes().to_vec();
                    drop(packet);
                    if tun_tx.blocking_send(bytes).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    tracing::warn!(?error, "wg-upstream: wintun receive failed");
                    return;
                }
            }
        }
    })
}

#[cfg(target_os = "windows")]
fn spawn_wintun_writer(
    session: Arc<WintunSession>,
    mut rx: mpsc::Receiver<Vec<u8>>,
) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        while let Some(packet) = rx.blocking_recv() {
            let Ok(size) = u16::try_from(packet.len()) else {
                tracing::warn!(
                    "wg-upstream: wintun packet too large to send ({} bytes)",
                    packet.len()
                );
                continue;
            };
            match session.allocate_send_packet(size) {
                Ok(mut outbound) => {
                    outbound.bytes_mut().copy_from_slice(&packet);
                    session.send_packet(outbound);
                }
                Err(error) => {
                    tracing::warn!(?error, "wg-upstream: wintun allocate_send_packet failed");
                }
            }
        }
    })
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
    // Read only on macOS (route delete -interface ...); Linux uses the raw
    // `ip route show default` line for restore and never touches iface.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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

// ---------------------------------------------------------------------------
// Long-lived holder used by the daemon (macOS-only for now). Owns the tun,
// the userspace WG runtime, and the FullDefaultRoute guard for the lifetime
// of "WireGuard upstream is enabled". Reconciled by FipsPrivateTunnelRuntime
// whenever the config changes.
// ---------------------------------------------------------------------------

// `DAEMON_WG_UPSTREAM_HANDSHAKE_TIMEOUT` and `WireGuardExitFingerprint`
// are re-exported from `nostr_vpn_core::wg_upstream` at the top of
// this module so the daemon-side code below can keep referring to
// them by short names without duplicating definitions.

#[cfg(target_os = "macos")]
pub struct DaemonWgUpstream {
    pub iface: String,
    pub upstream: SocketAddr,
    runtime: Option<WgUpstreamRuntime>,
    full_route: Option<FullDefaultRoute>,
    // Tun is held to keep the utun fd open for the lifetime of the
    // tunnel; dropping it auto-removes the utun device on macOS.
    _tun: Arc<TunSocket>,
    config_fingerprint: WireGuardExitFingerprint,
}

#[cfg(target_os = "windows")]
pub struct DaemonWgUpstream {
    pub iface: String,
    pub upstream: SocketAddr,
    runtime: Option<WgUpstreamRuntime>,
    full_route: Option<WindowsFullDefaultRoute>,
    // Adapter + session held to keep the WinTun device open for the
    // lifetime of the tunnel; dropping releases the WinTun adapter
    // (which removes its routes too).
    _session: Arc<WintunSession>,
    _adapter: Arc<wintun::Adapter>,
    config_fingerprint: WireGuardExitFingerprint,
}

/// Bring up the daemon-owned WG upstream tunnel: create utun, run the
/// userspace WG state machine, wait for handshake, and only then swap
/// the default route. If the handshake doesn't complete within
/// `handshake_timeout`, the tunnel is torn down and the routing table
/// is **not** modified.
///
/// This is the "happy path" entry point used by the macOS daemon
/// reconcile loop. The caller stores the returned `DaemonWgUpstream`
/// inside the long-lived runtime; dropping it (or calling `cleanup`)
/// restores the original default route.
#[cfg(target_os = "macos")]
pub async fn apply_daemon_wg_upstream(
    config: &WireGuardExitConfig,
    handshake_timeout: Duration,
) -> Result<DaemonWgUpstream> {
    let fingerprint = WireGuardExitFingerprint::from_config(config);
    let interface_hint =
        if config.interface.trim().is_empty() || !config.interface.starts_with("utun") {
            // Daemon-side: always let the kernel pick the next utunN.
            // The user-facing config's `interface` is just a label.
            "utun".to_string()
        } else {
            config.interface.clone()
        };

    let tun = TunSocket::new(&interface_hint)
        .with_context(|| format!("create utun for WG upstream (hint='{interface_hint}')"))?
        .set_non_blocking()
        .context("set utun non-blocking")?;
    let actual_iface = tun.name().context("read utun name (probably needs root)")?;
    let tun = Arc::new(tun);

    let runtime = start_wg_runtime_with_posix_tun(config, tun.clone())
        .await
        .context("start userspace WG runtime")?;
    let upstream = runtime.upstream();

    // Watchdog: wait up to `handshake_timeout` for the WG handshake to
    // complete. If it doesn't, we never modify the routing table —
    // tear down the tun + runtime and surface the error.
    if !runtime.wait_for_handshake(handshake_timeout).await {
        runtime.shutdown().await;
        return Err(anyhow!(
            "WG upstream handshake to {upstream} did not complete within {}s; \
             routing table NOT modified",
            handshake_timeout.as_secs()
        ));
    }

    let mtu = if config.mtu > 0 { config.mtu } else { 1420 };
    let full_route = match apply_full_default_route(&actual_iface, &config.address, upstream, mtu) {
        Ok(route) => route,
        Err(error) => {
            runtime.shutdown().await;
            return Err(error.context("swap default route via WG upstream"));
        }
    };

    Ok(DaemonWgUpstream {
        iface: actual_iface,
        upstream,
        runtime: Some(runtime),
        full_route: Some(full_route),
        _tun: tun,
        config_fingerprint: fingerprint,
    })
}

#[cfg(target_os = "macos")]
impl DaemonWgUpstream {
    /// Whether the daemon should consider this WG upstream tunnel
    /// equivalent to a fresh apply for `new_config`. Used by the
    /// reconcile loop to short-circuit a teardown+rebuild on every
    /// tick.
    pub fn matches(&self, new_config: &WireGuardExitConfig) -> bool {
        self.config_fingerprint == WireGuardExitFingerprint::from_config(new_config)
    }

    /// Tear down the WG upstream cleanly: restore the default route
    /// FIRST (so the host has a working route to the internet again
    /// while the WG runtime is winding down), then stop the boringtun
    /// pump, then drop the tun (which removes the utun device).
    pub async fn cleanup(mut self) {
        if let Some(mut full_route) = self.full_route.take() {
            if let Err(error) = full_route.revert() {
                eprintln!(
                    "fips: WG upstream route revert failed: {error}. \
                     Routing table may need manual cleanup."
                );
            }
            // Drop FullDefaultRoute *after* explicit revert; the Drop
            // impl is idempotent, so doing it twice is harmless.
            drop(full_route);
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown().await;
        }
        // self._tun drops here, removing the utun device.
    }
}

#[cfg(target_os = "windows")]
impl DaemonWgUpstream {
    pub fn matches(&self, new_config: &WireGuardExitConfig) -> bool {
        self.config_fingerprint == WireGuardExitFingerprint::from_config(new_config)
    }

    pub async fn cleanup(mut self) {
        if let Some(mut full_route) = self.full_route.take() {
            if let Err(error) = full_route.revert() {
                eprintln!(
                    "fips: WG upstream route revert failed: {error}. \
                     Routing table may need manual cleanup."
                );
            }
            drop(full_route);
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown().await;
        }
        // self._session and self._adapter drop here. WinTun removes
        // its adapter (and any routes pointing at it) when the last
        // reference goes; the kernel falls back to whatever default
        // route still exists with a higher metric, which the
        // WindowsFullDefaultRoute revert above just restored.
    }
}

// ---------------------------------------------------------------------------
// Windows full default-route swap. Same shape as FullDefaultRoute on
// POSIX, but the routing table is driven by `netsh interface ipv4`
// instead of `ip` / `route`, and the WG iface is identified by its
// kernel interface index rather than a name. The captured original
// default route is held verbatim from `route print 0.0.0.0` so we can
// re-add it on cleanup.
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
pub fn apply_windows_full_default_route(
    wg_iface_index: u32,
    upstream: SocketAddr,
) -> Result<WindowsFullDefaultRoute> {
    let upstream_ip = match upstream.ip() {
        IpAddr::V4(ip) => ip,
        IpAddr::V6(_) => {
            return Err(anyhow!(
                "WG upstream IPv6 endpoint not yet supported on Windows"
            ));
        }
    };

    // Capture the underlay default route (gateway + interface index)
    // before we touch anything. We need the gateway to install a /32
    // bypass for the WG endpoint.
    let original = capture_windows_default_route()?;
    if original.interface_index == wg_iface_index {
        return Err(anyhow!(
            "captured default route already points at the WG WinTun adapter (interface={}); \
             refusing to swap to avoid creating a routing loop",
            wg_iface_index
        ));
    }

    // 1. /32 bypass for the WG endpoint via the original gateway.
    //    Must exist BEFORE we add the WG default; otherwise the
    //    encrypted UDP would loop into the WG iface.
    run_windows_netsh(&[
        "interface".to_string(),
        "ipv4".to_string(),
        "add".to_string(),
        "route".to_string(),
        format!("{upstream_ip}/32"),
        format!("interface={}", original.interface_index),
        format!("nexthop={}", original.gateway),
        "metric=1".to_string(),
        "store=active".to_string(),
    ])?;

    // 2. Default via the WG WinTun adapter at metric=1 so it wins
    //    against the underlay's default (typically metric ~10).
    if let Err(error) = run_windows_netsh(&[
        "interface".to_string(),
        "ipv4".to_string(),
        "add".to_string(),
        "route".to_string(),
        "0.0.0.0/0".to_string(),
        format!("interface={}", wg_iface_index),
        "metric=1".to_string(),
        "store=active".to_string(),
    ]) {
        // Roll back the bypass we just added before bubbling the
        // error up — leaving a /32 to a now-broken gateway around
        // would be a real footgun.
        let _ = run_windows_netsh(&[
            "interface".to_string(),
            "ipv4".to_string(),
            "delete".to_string(),
            "route".to_string(),
            format!("{upstream_ip}/32"),
            format!("interface={}", original.interface_index),
            "store=active".to_string(),
        ]);
        return Err(error);
    }

    Ok(WindowsFullDefaultRoute {
        wg_iface_index,
        bypass_target: upstream_ip,
        original,
        reverted: false,
    })
}

#[cfg(target_os = "windows")]
pub struct WindowsFullDefaultRoute {
    wg_iface_index: u32,
    bypass_target: std::net::Ipv4Addr,
    original: WindowsDefaultRoute,
    reverted: bool,
}

#[cfg(target_os = "windows")]
#[derive(Debug, Clone)]
struct WindowsDefaultRoute {
    gateway: String,
    interface_index: u32,
}

#[cfg(target_os = "windows")]
impl WindowsFullDefaultRoute {
    pub fn revert(&mut self) -> Result<()> {
        if self.reverted {
            return Ok(());
        }
        // Delete our 0.0.0.0/0 → WG-iface entry. The original default
        // (which we never touched) is still in the table at its
        // higher metric and now becomes the active default again.
        let _ = run_windows_netsh(&[
            "interface".to_string(),
            "ipv4".to_string(),
            "delete".to_string(),
            "route".to_string(),
            "0.0.0.0/0".to_string(),
            format!("interface={}", self.wg_iface_index),
            "store=active".to_string(),
        ]);
        // Delete the /32 bypass for the WG endpoint.
        let _ = run_windows_netsh(&[
            "interface".to_string(),
            "ipv4".to_string(),
            "delete".to_string(),
            "route".to_string(),
            format!("{}/32", self.bypass_target),
            format!("interface={}", self.original.interface_index),
            "store=active".to_string(),
        ]);
        self.reverted = true;
        Ok(())
    }
}

#[cfg(target_os = "windows")]
impl Drop for WindowsFullDefaultRoute {
    fn drop(&mut self) {
        if let Err(error) = self.revert() {
            eprintln!(
                "wg-upstream: WARNING — Windows route revert failed: {error}. \
                 You may need to run `netsh interface ipv4 delete route 0.0.0.0/0 \
                 interface={}` manually.",
                self.wg_iface_index
            );
        }
    }
}

#[cfg(target_os = "windows")]
fn capture_windows_default_route() -> Result<WindowsDefaultRoute> {
    // `route print -4 0.0.0.0` lists IPv4 default routes. Output
    // includes columns like:
    //   Network Destination | Netmask | Gateway | Interface | Metric
    //   0.0.0.0             | 0.0.0.0 | 192.168.1.1 | 192.168.1.42 | 25
    let output = ProcessCommand::new("route")
        .arg("print")
        .arg("-4")
        .arg("0.0.0.0")
        .output()
        .context("spawn `route print -4 0.0.0.0`")?;
    if !output.status.success() {
        return Err(anyhow!("route print failed: {}", output.status));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let interface_ip = parse_windows_default_route_columns(&stdout)
        .ok_or_else(|| anyhow!("no IPv4 default route found in `route print` output"))?;
    let interface_index = resolve_windows_interface_index_for_address(&interface_ip.interface_ip)?;
    Ok(WindowsDefaultRoute {
        gateway: interface_ip.gateway,
        interface_index,
    })
}

#[cfg(any(test, target_os = "windows"))]
struct ParsedWindowsDefaultRoute {
    gateway: String,
    interface_ip: String,
}

#[cfg(any(test, target_os = "windows"))]
fn parse_windows_default_route_columns(output: &str) -> Option<ParsedWindowsDefaultRoute> {
    for line in output.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.len() < 5 {
            continue;
        }
        if tokens[0] == "0.0.0.0" && tokens[1] == "0.0.0.0" {
            // Some columns may be "On-link" for the gateway when the
            // default goes via a /32 host route; skip those — they
            // can't be used as the bypass nexthop.
            if tokens[2].eq_ignore_ascii_case("on-link") {
                continue;
            }
            return Some(ParsedWindowsDefaultRoute {
                gateway: tokens[2].to_string(),
                interface_ip: tokens[3].to_string(),
            });
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn resolve_windows_interface_index_for_address(interface_ip: &str) -> Result<u32> {
    use std::net::Ipv4Addr;
    let target: Ipv4Addr = interface_ip
        .parse()
        .with_context(|| format!("invalid IPv4 interface address {interface_ip}"))?;

    // `netsh interface ipv4 show ipaddresses level=verbose` enumerates
    // every IPv4 address with its interface index. Cheap parse; we
    // could use the IpHelper API but that's a bigger crate dep.
    let output = ProcessCommand::new("netsh")
        .args(["interface", "ipv4", "show", "ipaddresses", "level=verbose"])
        .output()
        .context("spawn `netsh interface ipv4 show ipaddresses`")?;
    if !output.status.success() {
        return Err(anyhow!("netsh show ipaddresses failed: {}", output.status));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut current_index: Option<u32> = None;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Interface ") {
            // "Interface 7: ..."
            if let Some((idx_str, _)) = rest.split_once(':')
                && let Ok(idx) = idx_str.trim().parse::<u32>()
            {
                current_index = Some(idx);
            }
        } else if let Some(rest) = trimmed.strip_prefix("Address ") {
            if let Some((addr_str, _)) = rest.split_once(' ')
                && let Ok(addr) = addr_str.parse::<Ipv4Addr>()
                && addr == target
                && let Some(idx) = current_index
            {
                return Ok(idx);
            }
        }
    }
    Err(anyhow!(
        "no Windows interface found with IPv4 address {target}"
    ))
}

#[cfg(target_os = "windows")]
fn run_windows_netsh(args: &[String]) -> Result<()> {
    let output = ProcessCommand::new("netsh")
        .args(args)
        .output()
        .with_context(|| format!("spawn `netsh {}`", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(anyhow!(
            "netsh {} failed:\n  stdout: {}\n  stderr: {}",
            args.join(" "),
            stdout.trim(),
            stderr.trim()
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Windows daemon entry point. Mirrors the macOS variant: creates a
// dedicated WinTun adapter for the WG upstream, runs the userspace
// state machine, watchdogs the handshake, and only swaps the default
// route after a successful handshake.
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
pub async fn apply_daemon_wg_upstream(
    config: &WireGuardExitConfig,
    handshake_timeout: Duration,
) -> Result<DaemonWgUpstream> {
    let fingerprint = WireGuardExitFingerprint::from_config(config);
    let adapter_name = if config.interface.trim().is_empty() {
        "nvpn-wg-upstream".to_string()
    } else {
        config.interface.clone()
    };

    let wintun = nostr_vpn_wintun::load_wintun().context("load wintun.dll for WG upstream")?;
    let adapter = wintun::Adapter::open(&wintun, &adapter_name)
        .or_else(|_| wintun::Adapter::create(&wintun, &adapter_name, "NostrVPN", None))
        .with_context(|| format!("open or create wintun adapter {adapter_name}"))?;

    let mtu = if config.mtu > 0 { config.mtu } else { 1420 };
    adapter
        .set_mtu(mtu as usize)
        .with_context(|| format!("set MTU on wintun adapter {adapter_name}"))?;
    let parsed_address = crate::windows_tunnel::windows_interface_address(&config.address)?;
    adapter
        .set_network_addresses_tuple(
            parsed_address.address.into(),
            parsed_address.mask.into(),
            None,
        )
        .with_context(|| format!("set address on wintun adapter {adapter_name}"))?;
    let interface_index = adapter
        .get_adapter_index()
        .with_context(|| format!("read interface index for {adapter_name}"))?;
    let session = Arc::new(
        adapter
            .start_session(wintun::MAX_RING_CAPACITY)
            .with_context(|| format!("start wintun session for {adapter_name}"))?,
    );
    let adapter = Arc::new(adapter);

    let runtime = start_wg_runtime_with_wintun(config, session.clone())
        .await
        .context("start userspace WG runtime on wintun")?;
    let upstream = runtime.upstream();

    if !runtime.wait_for_handshake(handshake_timeout).await {
        runtime.shutdown().await;
        // Adapter and session drop here, removing the WinTun device.
        return Err(anyhow!(
            "WG upstream handshake to {upstream} did not complete within {}s; \
             routing table NOT modified",
            handshake_timeout.as_secs()
        ));
    }

    let full_route = match apply_windows_full_default_route(interface_index, upstream) {
        Ok(route) => route,
        Err(error) => {
            runtime.shutdown().await;
            return Err(error.context("swap Windows default route via WG upstream"));
        }
    };

    Ok(DaemonWgUpstream {
        iface: adapter_name,
        upstream,
        runtime: Some(runtime),
        full_route: Some(full_route),
        _session: session,
        _adapter: adapter,
        config_fingerprint: fingerprint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_windows_default_route_from_route_print() {
        // Synthetic `route print -4 0.0.0.0` output. Only the
        // 0.0.0.0/0.0.0.0 row matters; all other content is meant to
        // be skipped by the parser.
        let sample = "\
===========================================================================
Interface List
 23...00 ff a1 b2 c3 d4 ......WireGuard Tunnel
 12...c0 d4 fe ff aa bb ......Realtek PCIe GbE
===========================================================================

IPv4 Route Table
===========================================================================
Active Routes:
Network Destination        Netmask          Gateway       Interface  Metric
          0.0.0.0          0.0.0.0      192.168.1.1     192.168.1.42     25
        127.0.0.0        255.0.0.0         On-link         127.0.0.1    331
===========================================================================
";
        let parsed = parse_windows_default_route_columns(sample).expect("default route parsed");
        assert_eq!(parsed.gateway, "192.168.1.1");
        assert_eq!(parsed.interface_ip, "192.168.1.42");
    }

    #[test]
    fn skips_on_link_default_routes() {
        let sample = "\
Active Routes:
Network Destination        Netmask          Gateway       Interface  Metric
          0.0.0.0          0.0.0.0         On-link        10.0.0.1     50
          0.0.0.0          0.0.0.0      192.168.1.1   192.168.1.42     25
";
        let parsed =
            parse_windows_default_route_columns(sample).expect("non-On-link default parsed");
        assert_eq!(parsed.gateway, "192.168.1.1");
        assert_eq!(parsed.interface_ip, "192.168.1.42");
    }

    #[test]
    fn returns_none_when_no_default_route_present() {
        let sample = "Active Routes:\n      127.0.0.0  255.0.0.0  On-link  127.0.0.1  331\n";
        assert!(parse_windows_default_route_columns(sample).is_none());
    }
}
