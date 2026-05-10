use std::net::Ipv4Addr;
use std::os::raw::c_int;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fips_endpoint::{
    Config as FipsConfig, ConnectPolicy, FipsEndpoint, NostrDiscoveryPolicy,
    PeerConfig as FipsPeerConfig, TransportInstances, UdpConfig,
};
use nostr_vpn_core::config::{
    AppConfig, WireGuardExitConfig, derive_mesh_tunnel_ip, maybe_autoconfigure_node,
};
use nostr_vpn_core::fips_mesh::{FipsMeshPeerConfig, FipsMeshRuntime};
use nostr_vpn_core::wg_upstream::{DAEMON_WG_UPSTREAM_HANDSHAKE_TIMEOUT, WgUpstreamRuntime};
use serde::{Deserialize, Serialize};

use crate::wg_upstream_nat::{rewrite_ipv4_destination, rewrite_ipv4_source};
use tokio::runtime::{Builder as RuntimeBuilder, Runtime};
use tokio::sync::mpsc as tokio_mpsc;
use tokio::task::JoinHandle;

const DEFAULT_MOBILE_MTU: u16 = 1280;
const TUNNEL_CHANNEL_CAPACITY: usize = 1024;
const FIPS_NOSTR_DISCOVERY_APP: &str = "fips-overlay-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MobileTunnelConfig {
    pub(crate) identity_nsec: String,
    pub(crate) network_id: String,
    pub(crate) local_address: String,
    pub(crate) mtu: u16,
    pub(crate) peers: Vec<FipsMeshPeerConfig>,
    pub(crate) route_targets: Vec<String>,
    #[serde(default)]
    pub(crate) nostr_relays: Vec<String>,
    #[serde(default)]
    pub(crate) stun_servers: Vec<String>,
    #[serde(default)]
    pub(crate) share_local_candidates: bool,
    /// When the user has WG upstream enabled + configured, the OS-side
    /// (NEPacketTunnelProvider on iOS, VpnService on Android) is
    /// expected to:
    ///   * include `0.0.0.0/0` in the tunnel's includedRoutes (so all
    ///     non-mesh outbound traffic enters the tun and we can forward
    ///     it to boringtun)
    ///   * route every IP in `excluded_routes` outside the tunnel so
    ///     the encrypted UDP can actually reach the WG upstream
    ///     endpoint (iOS does this via `NEIPv4Settings.excludedRoutes`;
    ///     on Android the host calls `VpnService.protect(socket_fd)`
    ///     instead, see `MobileTunnel::wg_upstream_socket_fd`).
    #[serde(default)]
    pub(crate) excluded_routes: Vec<String>,
    /// The WG upstream config to drive boringtun against. None when
    /// the user hasn't enabled WG upstream — in which case the mobile
    /// tunnel runs in pure FIPS-mesh mode.
    #[serde(default)]
    pub(crate) wireguard_exit: Option<WireGuardExitConfig>,
    #[serde(default)]
    pub(crate) error: String,
}

impl MobileTunnelConfig {
    pub(crate) fn from_data_dir(data_dir: &str) -> Result<Self> {
        let config_path = native_config_path(data_dir);
        let mut app = if config_path.exists() {
            AppConfig::load(&config_path)?
        } else {
            let generated = AppConfig::generated();
            generated.save(&config_path)?;
            generated
        };
        app.ensure_defaults();
        maybe_autoconfigure_node(&mut app);
        app.save(&config_path)?;
        Self::from_app(&app)
    }

    fn from_app(app: &AppConfig) -> Result<Self> {
        let own_pubkey = app.own_nostr_pubkey_hex()?;
        let network_id = app.effective_network_id();
        let mut peers = Vec::new();
        let mut route_targets = Vec::new();

        for participant in app
            .active_network_signal_pubkeys_hex()
            .into_iter()
            .filter(|participant| participant != &own_pubkey)
        {
            let Some(tunnel_ip) = derive_mesh_tunnel_ip(&network_id, &participant) else {
                continue;
            };
            let route = format!("{}/32", strip_cidr(&tunnel_ip));
            route_targets.push(route.clone());
            peers.push(FipsMeshPeerConfig::from_participant_pubkey(
                participant,
                vec![route],
            )?);
        }

        peers.sort_by(|left, right| left.participant_pubkey.cmp(&right.participant_pubkey));
        peers.dedup_by(|left, right| left.participant_pubkey == right.participant_pubkey);
        route_targets.sort();
        route_targets.dedup();

        let local_address = derive_mesh_tunnel_ip(&network_id, &own_pubkey).map_or_else(
            || local_interface_address_for_tunnel(&app.node.tunnel_ip),
            |tunnel_ip| local_interface_address_for_tunnel(&tunnel_ip),
        );

        // WireGuard upstream: when the user has enabled it AND the
        // config is fully populated, expand the tunnel's route set to
        // 0.0.0.0/0 (all outbound traffic should enter the tun) and
        // ask the host platform to keep the WG endpoint outside the
        // tunnel via `excluded_routes`.
        let (wireguard_exit, excluded_routes) =
            if app.wireguard_exit.enabled && app.wireguard_exit.configured() {
                let mut excluded = Vec::new();
                if let Some(ip) = wireguard_endpoint_host_ip(&app.wireguard_exit.endpoint) {
                    excluded.push(format!("{ip}/32"));
                }
                if !route_targets.iter().any(|route| route == "0.0.0.0/0") {
                    route_targets.push("0.0.0.0/0".to_string());
                }
                route_targets.sort();
                route_targets.dedup();
                (Some(app.wireguard_exit.clone()), excluded)
            } else {
                (None, Vec::new())
            };

        Ok(Self {
            identity_nsec: app.nostr.secret_key.clone(),
            network_id,
            local_address,
            mtu: DEFAULT_MOBILE_MTU,
            peers,
            route_targets,
            nostr_relays: app.nostr.relays.clone(),
            stun_servers: app.nat.stun_servers.clone(),
            share_local_candidates: app.lan_discovery_enabled,
            excluded_routes,
            wireguard_exit,
            error: String::new(),
        })
    }
}

/// Pull just the IP literal out of an `Endpoint = host:port` string.
/// Returns None if the host is a DNS name (we can't pre-resolve here
/// — the OS-side glue would need to do that). Mullvad / Proton ship
/// configs with literal IPs, so this is fine for the common case.
fn wireguard_endpoint_host_ip(endpoint: &str) -> Option<std::net::IpAddr> {
    let trimmed = endpoint.trim();
    let host = trimmed.rsplit_once(':').map(|(h, _)| h).unwrap_or(trimmed);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.parse().ok()
}

pub(crate) fn tunnel_config_json(data_dir: &str) -> String {
    let config =
        MobileTunnelConfig::from_data_dir(data_dir).unwrap_or_else(|error| MobileTunnelConfig {
            error: error.to_string(),
            ..empty_config()
        });
    serde_json::to_string(&config).unwrap_or_else(|error| {
        format!(
            r#"{{"error":"{}"}}"#,
            error.to_string().replace(['\\', '"'], "")
        )
    })
}

pub(crate) struct MobileTunnel {
    runtime: Runtime,
    endpoint: Option<Arc<FipsEndpoint>>,
    outbound_tx: tokio_mpsc::Sender<Vec<u8>>,
    inbound_rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    tasks: Vec<JoinHandle<()>>,
    wg_upstream: Option<WgUpstreamRuntime>,
    /// Raw fd of the boringtun UDP socket. On Android the host
    /// reads this and calls `VpnService.protect(fd)` so the encrypted
    /// UDP escapes the VPN tun. -1 when WG upstream isn't running.
    wg_upstream_socket_fd: c_int,
}

impl MobileTunnel {
    pub(crate) fn start(config_json: &str) -> Result<Self> {
        let config: MobileTunnelConfig =
            serde_json::from_str(config_json).context("invalid mobile tunnel config JSON")?;
        if !config.error.trim().is_empty() {
            return Err(anyhow!(config.error));
        }
        let runtime = RuntimeBuilder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("nvpn-mobile-fips")
            .build()
            .context("failed to start mobile FIPS runtime")?;
        let started = runtime.block_on(Self::start_async(config))?;
        Ok(Self {
            runtime,
            endpoint: Some(started.endpoint),
            outbound_tx: started.outbound_tx,
            inbound_rx: Mutex::new(started.inbound_rx),
            tasks: started.tasks,
            wg_upstream: started.wg_upstream,
            wg_upstream_socket_fd: started.wg_upstream_socket_fd,
        })
    }

    async fn start_async(config: MobileTunnelConfig) -> Result<MobileTunnelStarted> {
        let scope = format!("nostr-vpn:{}", config.network_id.trim());
        let endpoint = FipsEndpoint::builder()
            .config(fips_endpoint_config(&scope, &config))
            .identity_nsec(config.identity_nsec)
            .discovery_scope(scope)
            .without_system_tun()
            .bind()
            .await
            .context("failed to bind mobile FIPS endpoint")?;
        let endpoint = Arc::new(endpoint);
        let local_routes = vec![config.local_address.clone()];
        let mesh = Arc::new(RwLock::new(FipsMeshRuntime::with_local_routes(
            config.peers,
            local_routes,
        )));
        let (outbound_tx, mut outbound_rx) =
            tokio_mpsc::channel::<Vec<u8>>(TUNNEL_CHANNEL_CAPACITY);
        let (inbound_tx, inbound_rx) = mpsc::sync_channel::<Vec<u8>>(TUNNEL_CHANNEL_CAPACITY);

        // If the user has WG upstream enabled, stand up the boringtun
        // pump alongside the FIPS endpoint. The WG runtime is fed via
        // an mpsc::channel pair: `wg_send_tx` carries plaintext that
        // should be encapsulated and sent to the upstream;
        // `wg_recv_rx` carries plaintext we got back after
        // decapsulating the upstream's reply, ready to write back to
        // the OS tun.
        let mesh_ipv4 = parse_ipv4(&config.local_address);
        let mut tasks: Vec<JoinHandle<()>> = Vec::new();
        let mut wg_runtime: Option<WgUpstreamRuntime> = None;
        let mut wg_send_tx: Option<tokio_mpsc::Sender<Vec<u8>>> = None;
        let mut wg_socket_fd: c_int = -1;
        let mut wg_address_ipv4: Option<Ipv4Addr> = None;
        if let Some(wg_config) = config.wireguard_exit.as_ref() {
            wg_address_ipv4 = parse_ipv4(&wg_config.address);
            let (send_tx, send_rx) = tokio_mpsc::channel::<Vec<u8>>(TUNNEL_CHANNEL_CAPACITY);
            let (recv_tx, mut recv_rx) = tokio_mpsc::channel::<Vec<u8>>(TUNNEL_CHANNEL_CAPACITY);
            match WgUpstreamRuntime::start_with_channels(wg_config, send_rx, recv_tx).await {
                Ok(runtime) => {
                    wg_socket_fd = runtime.udp_socket_fd();
                    let upstream = runtime.upstream();
                    wg_runtime = Some(runtime);
                    wg_send_tx = Some(send_tx);
                    // Forward decrypted WG packets back to the OS as
                    // inbound traffic. DNAT: rewrite the WG-side
                    // destination IP back to the mesh tun address so
                    // the OS routes the reply to the local app stack.
                    let inbound_tx_for_wg = inbound_tx.clone();
                    let wg_addr = wg_address_ipv4;
                    let mesh_addr = mesh_ipv4;
                    tasks.push(tokio::spawn(async move {
                        while let Some(mut packet) = recv_rx.recv().await {
                            if let (Some(wg), Some(mesh)) = (wg_addr, mesh_addr) {
                                rewrite_ipv4_destination(&mut packet, wg, mesh);
                            }
                            if inbound_tx_for_wg.send(packet).is_err() {
                                break;
                            }
                        }
                    }));
                    // Watchdog: log if the handshake doesn't complete
                    // promptly. We don't tear down the tun on mobile
                    // (the OS owns it) but the host can surface the
                    // status to the UI.
                    if let Some(runtime_ref) = wg_runtime.as_ref() {
                        let timeout = DAEMON_WG_UPSTREAM_HANDSHAKE_TIMEOUT;
                        if !runtime_ref.wait_for_handshake(timeout).await {
                            tracing::warn!(
                                ?upstream,
                                "wg-upstream: no handshake within {timeout:?} on mobile tunnel; \
                                 traffic will queue until upstream becomes reachable"
                            );
                        } else {
                            tracing::info!(
                                ?upstream,
                                "wg-upstream: mobile tunnel handshake completed"
                            );
                        }
                    }
                }
                Err(error) => {
                    // Don't fail the whole tunnel — FIPS mesh still
                    // works. Just log and continue without WG.
                    tracing::warn!(
                        ?error,
                        "wg-upstream: failed to start mobile WG runtime; continuing without WG upstream"
                    );
                }
            }
        }

        let send_task = {
            let endpoint = Arc::clone(&endpoint);
            let mesh = Arc::clone(&mesh);
            let wg_send_tx_for_dispatch = wg_send_tx.clone();
            let wg_addr = wg_address_ipv4;
            let mesh_addr = mesh_ipv4;
            tokio::spawn(async move {
                while let Some(packet) = outbound_rx.recv().await {
                    let outgoing = mesh
                        .read()
                        .ok()
                        .and_then(|mesh| mesh.route_outbound_packet(&packet));
                    if let Some(outgoing) = outgoing {
                        let _ = endpoint.send(outgoing.endpoint_npub, outgoing.bytes).await;
                    } else if let Some(wg_tx) = wg_send_tx_for_dispatch.as_ref() {
                        // No matching mesh peer route: hand the
                        // plaintext off to the WG runtime, which will
                        // boringtun-encapsulate and send out via the
                        // upstream UDP socket. SNAT first so the inner
                        // source IP matches the WG peer's configured
                        // address — Mullvad / Proton silently drop
                        // packets whose inner source isn't an allowed
                        // peer IP.
                        let mut packet = packet;
                        if let (Some(wg), Some(mesh)) = (wg_addr, mesh_addr) {
                            rewrite_ipv4_source(&mut packet, mesh, wg);
                        }
                        let _ = wg_tx.try_send(packet);
                    }
                }
            })
        };
        tasks.push(send_task);

        let recv_task = {
            let endpoint = Arc::clone(&endpoint);
            let mesh = Arc::clone(&mesh);
            tokio::spawn(async move {
                loop {
                    let Some(message) = endpoint.recv().await else {
                        break;
                    };
                    let packet = mesh.read().ok().and_then(|mesh| {
                        mesh.receive_endpoint_data(message.source_npub.as_deref(), &message.data)
                    });
                    if let Some(packet) = packet
                        && inbound_tx.send(packet.bytes).is_err()
                    {
                        break;
                    }
                }
            })
        };
        tasks.push(recv_task);

        Ok(MobileTunnelStarted {
            endpoint,
            outbound_tx,
            inbound_rx,
            tasks,
            wg_upstream: wg_runtime,
            wg_upstream_socket_fd: wg_socket_fd,
        })
    }

    /// Raw fd of the WG upstream UDP socket, or -1 if WG upstream
    /// isn't running. On Android, the host's `VpnService` calls
    /// `protect(fd)` on this so the encrypted UDP escapes the VPN
    /// tun. iOS doesn't need this — it relies on `excludedRoutes`
    /// declared at tunnel-establish time instead.
    pub(crate) fn wg_upstream_socket_fd(&self) -> c_int {
        self.wg_upstream_socket_fd
    }

    pub(crate) fn send_packet(&self, packet: &[u8]) -> bool {
        if packet.is_empty() {
            return false;
        }
        self.outbound_tx.try_send(packet.to_vec()).is_ok()
    }

    pub(crate) fn next_packet(&self, out: &mut [u8], timeout: Duration) -> Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let rx = self
            .inbound_rx
            .lock()
            .map_err(|_| anyhow!("mobile tunnel inbound packet lock poisoned"))?;
        match rx.recv_timeout(timeout) {
            Ok(packet) => {
                let len = packet.len().min(out.len());
                out[..len].copy_from_slice(&packet[..len]);
                Ok(len)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(0),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(anyhow!("mobile tunnel stopped")),
        }
    }
}

struct MobileTunnelStarted {
    endpoint: Arc<FipsEndpoint>,
    outbound_tx: tokio_mpsc::Sender<Vec<u8>>,
    inbound_rx: mpsc::Receiver<Vec<u8>>,
    tasks: Vec<JoinHandle<()>>,
    wg_upstream: Option<WgUpstreamRuntime>,
    wg_upstream_socket_fd: c_int,
}

impl Drop for MobileTunnel {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        let tasks = std::mem::take(&mut self.tasks);
        let endpoint = self.endpoint.take();
        let wg_upstream = self.wg_upstream.take();
        self.runtime.block_on(async move {
            for task in tasks {
                let _ = task.await;
            }
            if let Some(wg) = wg_upstream {
                wg.shutdown().await;
            }
            if let Some(endpoint) = endpoint
                && let Ok(endpoint) = Arc::try_unwrap(endpoint)
            {
                let _ = endpoint.shutdown().await;
            }
        });
    }
}

fn fips_endpoint_config(scope: &str, mobile: &MobileTunnelConfig) -> FipsConfig {
    let mut config = FipsConfig::new();
    let nostr_enabled = !mobile.peers.is_empty();
    config.node.discovery.nostr.enabled = nostr_enabled;
    config.node.discovery.nostr.advertise = false;
    config.node.discovery.nostr.policy = NostrDiscoveryPolicy::ConfiguredOnly;
    config.node.discovery.nostr.share_local_candidates = mobile.share_local_candidates;
    config.node.discovery.nostr.app = format!("{FIPS_NOSTR_DISCOVERY_APP}:{scope}");
    if !mobile.nostr_relays.is_empty() {
        config
            .node
            .discovery
            .nostr
            .advert_relays
            .clone_from(&mobile.nostr_relays);
        config
            .node
            .discovery
            .nostr
            .dm_relays
            .clone_from(&mobile.nostr_relays);
    }
    if !mobile.stun_servers.is_empty() {
        config
            .node
            .discovery
            .nostr
            .stun_servers
            .clone_from(&mobile.stun_servers);
    }
    config.transports.udp = TransportInstances::Single(UdpConfig {
        bind_addr: Some("0.0.0.0:0".to_string()),
        outbound_only: Some(false),
        accept_connections: Some(false),
        advertise_on_nostr: Some(false),
        public: Some(false),
        ..UdpConfig::default()
    });
    config.peers = mobile
        .peers
        .iter()
        .map(|peer| FipsPeerConfig {
            npub: peer.endpoint_npub.clone(),
            alias: None,
            addresses: Vec::new(),
            connect_policy: ConnectPolicy::AutoConnect,
            auto_reconnect: true,
        })
        .collect();
    config
}

fn native_config_path(data_dir: &str) -> PathBuf {
    let trimmed = data_dir.trim();
    if trimmed.is_empty() {
        default_config_path()
    } else {
        PathBuf::from(trimmed).join("config.toml")
    }
}

fn default_config_path() -> PathBuf {
    dirs::config_dir().map_or_else(
        || PathBuf::from("nvpn.toml"),
        |dir| dir.join("nvpn").join("config.toml"),
    )
}

fn local_interface_address_for_tunnel(tunnel_ip: &str) -> String {
    let tunnel_ip = tunnel_ip.trim();
    if tunnel_ip.is_empty() {
        return "10.44.0.1/32".to_string();
    }
    if tunnel_ip.contains('/') {
        return tunnel_ip.to_string();
    }
    format!("{}/32", strip_cidr(tunnel_ip))
}

fn strip_cidr(value: &str) -> &str {
    value.split('/').next().unwrap_or(value)
}

fn parse_ipv4(value: &str) -> Option<Ipv4Addr> {
    strip_cidr(value.trim()).parse().ok()
}

fn empty_config() -> MobileTunnelConfig {
    MobileTunnelConfig {
        identity_nsec: String::new(),
        network_id: String::new(),
        local_address: String::new(),
        mtu: DEFAULT_MOBILE_MTU,
        peers: Vec::new(),
        route_targets: Vec::new(),
        nostr_relays: Vec::new(),
        stun_servers: Vec::new(),
        share_local_candidates: false,
        excluded_routes: Vec::new(),
        wireguard_exit: None,
        error: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_vpn_core::config::NetworkConfig;

    #[test]
    fn mobile_config_routes_only_private_peer_addresses() {
        let mut app = AppConfig::generated();
        app.ensure_defaults();
        let own = app.own_nostr_pubkey_hex().expect("own pubkey");
        let peer = "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc";
        app.networks = vec![NetworkConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            enabled: true,
            network_id: "test".to_string(),
            participants: vec![peer.to_string()],
            admins: vec![own],
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        }];
        app.exit_node = peer.to_string();

        let config = MobileTunnelConfig::from_app(&app).expect("mobile config");

        assert_eq!(config.peers.len(), 1);
        assert_eq!(config.route_targets.len(), 1);
        assert!(config.route_targets[0].starts_with("10."));
        assert!(
            !config
                .route_targets
                .iter()
                .any(|route| route == "0.0.0.0/0")
        );
    }

    #[test]
    fn mobile_config_json_reports_errors_as_json() {
        let json = tunnel_config_json("\0/not-a-path");
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert!(value["error"].as_str().is_some());
    }

    #[test]
    fn mobile_fips_config_uses_discovery_for_roster_peers() {
        let peer = FipsMeshPeerConfig::from_participant_pubkey(
            "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc",
            vec!["10.44.22.44/32".to_string()],
        )
        .expect("peer");
        let mobile = MobileTunnelConfig {
            peers: vec![peer],
            nostr_relays: vec!["wss://relay.example".to_string()],
            stun_servers: vec!["stun:stun.example:3478".to_string()],
            share_local_candidates: true,
            ..empty_config()
        };
        let config = fips_endpoint_config("nostr-vpn:test", &mobile);

        config
            .validate()
            .expect("mobile FIPS config should validate");
        assert!(config.node.discovery.nostr.enabled);
        assert!(!config.node.discovery.nostr.advertise);
        assert!(config.node.discovery.nostr.share_local_candidates);
        assert_eq!(
            config.node.discovery.nostr.policy,
            NostrDiscoveryPolicy::ConfiguredOnly
        );
        assert_eq!(
            config.node.discovery.nostr.app,
            format!("{FIPS_NOSTR_DISCOVERY_APP}:nostr-vpn:test")
        );
        assert_eq!(
            config.node.discovery.nostr.advert_relays,
            vec!["wss://relay.example".to_string()]
        );
        assert_eq!(
            config.node.discovery.nostr.dm_relays,
            vec!["wss://relay.example".to_string()]
        );
        assert_eq!(
            config.node.discovery.nostr.stun_servers,
            vec!["stun:stun.example:3478".to_string()]
        );
        let TransportInstances::Single(udp) = &config.transports.udp else {
            panic!("expected single udp transport");
        };
        assert_eq!(udp.bind_addr(), "0.0.0.0:0");
        assert!(!udp.outbound_only());
        assert!(!udp.accept_connections());
        assert!(!udp.advertise_on_nostr());
        assert!(!udp.is_public());
        assert_eq!(config.peers.len(), 1);
    }

    #[test]
    fn mobile_fips_config_does_not_advertise_without_peers() {
        let config = fips_endpoint_config("nostr-vpn:test", &empty_config());

        config
            .validate()
            .expect("empty mobile FIPS config should validate");
        assert!(!config.node.discovery.nostr.enabled);
        assert!(!config.node.discovery.nostr.advertise);
        let TransportInstances::Single(udp) = &config.transports.udp else {
            panic!("expected single udp transport");
        };
        assert!(!udp.advertise_on_nostr());
        assert!(config.peers.is_empty());
    }
}
