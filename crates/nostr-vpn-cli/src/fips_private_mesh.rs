#![allow(dead_code)]

use anyhow::{Context, Result, anyhow};
use fips_endpoint::{
    Config, ConnectPolicy, FipsEndpoint, FipsEndpointError, NostrDiscoveryPolicy,
    PeerConfig as FipsPeerConfig, TransportInstances, UdpConfig,
};
use nostr_vpn_core::config::{
    AppConfig, derive_mesh_tunnel_ip, exit_node_default_routes, normalize_nostr_pubkey,
};
use nostr_vpn_core::data_plane::{MeshPeerStatus, PrivatePacket};
use nostr_vpn_core::fips_control::{
    FipsControlFrame, decode_fips_control_frame, encode_fips_control_frame,
};
use nostr_vpn_core::fips_mesh::{FipsMeshPeerConfig, FipsMeshRuntime};
use nostr_vpn_core::join_requests::MeshJoinRequest;
use nostr_vpn_core::signaling::NetworkRoster;
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::net::Ipv4Addr;
use std::net::{SocketAddr, SocketAddrV4};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use tokio::sync::mpsc;

const FIPS_PEER_ONLINE_GRACE_SECS: u64 = 45;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use boringtun::device::{Error as TunError, tun::TunSocket};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use tokio::task::JoinHandle;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use tokio::time::{Duration, sleep};

pub(crate) struct FipsPrivateMeshRuntime {
    endpoint: FipsEndpoint,
    mesh: RwLock<FipsMeshRuntime>,
    presence: RwLock<HashMap<String, FipsPeerPresence>>,
}

#[derive(Debug, Clone, Default)]
struct FipsPeerPresence {
    last_seen_at: Option<u64>,
    tx_bytes: u64,
    rx_bytes: u64,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum FipsPrivateMeshEvent {
    Packet(PrivatePacket),
    Presence {
        participant_pubkey: String,
        last_seen_at: u64,
    },
    JoinRequest {
        sender_pubkey: String,
        requested_at: u64,
        request: MeshJoinRequest,
    },
    Roster {
        sender_pubkey: String,
        network_id: String,
        roster: NetworkRoster,
    },
}

impl FipsPrivateMeshRuntime {
    pub(crate) async fn bind(
        identity_nsec: impl Into<String>,
        network_id: impl AsRef<str>,
        peers: Vec<FipsMeshPeerConfig>,
    ) -> Result<Self> {
        Self::bind_with_relays(identity_nsec, network_id, peers, &[]).await
    }

    pub(crate) async fn bind_with_relays(
        identity_nsec: impl Into<String>,
        network_id: impl AsRef<str>,
        peers: Vec<FipsMeshPeerConfig>,
        relays: &[String],
    ) -> Result<Self> {
        let scope = format!("nostr-vpn:{}", network_id.as_ref().trim());
        let config = fips_endpoint_config(&scope, relays, &peers, None);
        Self::bind_with_config(identity_nsec, scope, peers, config).await
    }

    async fn bind_with_config(
        identity_nsec: impl Into<String>,
        scope: impl Into<String>,
        peers: Vec<FipsMeshPeerConfig>,
        config: Config,
    ) -> Result<Self> {
        let scope = scope.into();
        let endpoint = FipsEndpoint::builder()
            .config(config)
            .identity_nsec(identity_nsec)
            .discovery_scope(scope)
            .without_system_tun()
            .bind()
            .await
            .context("failed to bind embedded FIPS endpoint")?;

        Ok(Self {
            endpoint,
            mesh: RwLock::new(FipsMeshRuntime::new(peers)),
            presence: RwLock::new(HashMap::new()),
        })
    }

    pub(crate) fn npub(&self) -> &str {
        self.endpoint.npub()
    }

    pub(crate) async fn send_tunnel_packet(&self, packet: &[u8]) -> Result<bool> {
        let outgoing = {
            self.mesh
                .read()
                .map_err(|_| anyhow!("FIPS mesh route table lock poisoned"))?
                .route_outbound_packet(packet)
        };
        let Some(outgoing) = outgoing else {
            return Ok(false);
        };

        self.endpoint
            .send(outgoing.endpoint_npub, outgoing.bytes.clone())
            .await
            .context("failed to send private packet over FIPS endpoint data")?;
        self.note_tx(&outgoing.participant_pubkey, outgoing.bytes.len())?;
        Ok(true)
    }

    pub(crate) async fn recv_mesh_event(&self) -> Result<Option<FipsPrivateMeshEvent>> {
        loop {
            let Some(message) = self.endpoint.recv().await else {
                return Ok(None);
            };
            let Some(source_pubkey) = self.source_pubkey(message.source_npub.as_deref()) else {
                continue;
            };

            if let Some(frame) = decode_fips_control_frame(&message.data)? {
                let now = unix_timestamp();
                self.note_rx(&source_pubkey, message.data.len(), now)?;
                match frame {
                    FipsControlFrame::Ping {
                        network_id,
                        sent_at,
                    } => {
                        let reply = FipsControlFrame::Pong {
                            network_id,
                            sent_at,
                            replied_at: now,
                        };
                        if let Some(source_npub) = message.source_npub {
                            let encoded = encode_fips_control_frame(&reply)?;
                            if let Err(error) = self.endpoint.send(source_npub, encoded).await {
                                eprintln!("fips: failed to reply to peer ping: {error}");
                            }
                        }
                        return Ok(Some(FipsPrivateMeshEvent::Presence {
                            participant_pubkey: source_pubkey,
                            last_seen_at: now,
                        }));
                    }
                    FipsControlFrame::Pong { .. } => {
                        return Ok(Some(FipsPrivateMeshEvent::Presence {
                            participant_pubkey: source_pubkey,
                            last_seen_at: now,
                        }));
                    }
                    FipsControlFrame::JoinRequest {
                        requested_at,
                        request,
                    } => {
                        return Ok(Some(FipsPrivateMeshEvent::JoinRequest {
                            sender_pubkey: source_pubkey,
                            requested_at,
                            request,
                        }));
                    }
                    FipsControlFrame::Roster { network_id, roster } => {
                        return Ok(Some(FipsPrivateMeshEvent::Roster {
                            sender_pubkey: source_pubkey,
                            network_id,
                            roster,
                        }));
                    }
                }
            }

            if let Some(packet) = self
                .mesh
                .read()
                .map_err(|_| anyhow!("FIPS mesh route table lock poisoned"))?
                .receive_endpoint_data(message.source_npub.as_deref(), &message.data)
            {
                let now = unix_timestamp();
                self.note_rx(&packet.source_pubkey, message.data.len(), now)?;
                return Ok(Some(FipsPrivateMeshEvent::Packet(packet)));
            }
        }
    }

    #[cfg(test)]
    pub(crate) async fn recv_tunnel_packet(&self) -> Result<Option<PrivatePacket>> {
        loop {
            match self.recv_mesh_event().await? {
                Some(FipsPrivateMeshEvent::Packet(packet)) => return Ok(Some(packet)),
                Some(_) => {}
                None => return Ok(None),
            }
        }
    }

    pub(crate) fn peer_statuses(&self) -> Vec<MeshPeerStatus> {
        let now = unix_timestamp();
        let presence = self.presence.read().ok();
        let mut statuses = self
            .mesh
            .read()
            .map(|mesh| mesh.peer_statuses())
            .unwrap_or_default();
        for status in &mut statuses {
            let peer_presence = presence
                .as_ref()
                .and_then(|presence| presence.get(&status.pubkey));
            status.last_seen_at = peer_presence.and_then(|value| value.last_seen_at);
            status.tx_bytes = peer_presence.map(|value| value.tx_bytes).unwrap_or(0);
            status.rx_bytes = peer_presence.map(|value| value.rx_bytes).unwrap_or(0);
            status.connected = status.last_seen_at.is_some_and(|last_seen_at| {
                now.saturating_sub(last_seen_at) <= FIPS_PEER_ONLINE_GRACE_SECS
            });
            status.error = if status.connected {
                None
            } else {
                peer_presence
                    .and_then(|value| value.error.clone())
                    .or_else(|| Some("fips presence pending".to_string()))
            };
        }
        statuses
    }

    pub(crate) fn peer_pubkeys(&self) -> Vec<String> {
        self.mesh
            .read()
            .map(|mesh| mesh.peer_pubkeys())
            .unwrap_or_default()
    }

    #[cfg(target_os = "linux")]
    pub(crate) async fn peer_transport_ipv4_hosts(&self) -> Result<Vec<Ipv4Addr>> {
        let mut hosts = self
            .endpoint
            .peers()
            .await
            .context("failed to snapshot FIPS endpoint peers")?
            .into_iter()
            .filter_map(|peer| peer.transport_addr)
            .filter_map(|addr| endpoint_transport_ipv4_host(&addr))
            .collect::<Vec<_>>();
        hosts.sort_unstable();
        hosts.dedup();
        Ok(hosts)
    }

    pub(crate) fn replace_peers(&self, peers: Vec<FipsMeshPeerConfig>) -> Result<()> {
        *self
            .mesh
            .write()
            .map_err(|_| anyhow!("FIPS mesh route table lock poisoned"))? =
            FipsMeshRuntime::new(peers);
        let configured = self
            .mesh
            .read()
            .map_err(|_| anyhow!("FIPS mesh route table lock poisoned"))?
            .peer_pubkeys();
        self.presence
            .write()
            .map_err(|_| anyhow!("FIPS mesh presence lock poisoned"))?
            .retain(|participant, _| configured.iter().any(|value| value == participant));
        Ok(())
    }

    pub(crate) async fn ping_peers(&self, network_id: &str, now: u64) -> Result<usize> {
        let frame = FipsControlFrame::Ping {
            network_id: network_id.to_string(),
            sent_at: now,
        };
        self.broadcast_control_frame(&frame).await
    }

    pub(crate) async fn send_join_request(
        &self,
        participant: &str,
        requested_at: u64,
        request: MeshJoinRequest,
    ) -> Result<()> {
        self.send_control_frame(
            participant,
            &FipsControlFrame::JoinRequest {
                requested_at,
                request,
            },
        )
        .await
    }

    pub(crate) async fn send_roster(
        &self,
        participant: &str,
        network_id: &str,
        roster: NetworkRoster,
    ) -> Result<()> {
        self.send_control_frame(
            participant,
            &FipsControlFrame::Roster {
                network_id: network_id.to_string(),
                roster,
            },
        )
        .await
    }

    async fn broadcast_control_frame(&self, frame: &FipsControlFrame) -> Result<usize> {
        let participants = self
            .mesh
            .read()
            .map_err(|_| anyhow!("FIPS mesh route table lock poisoned"))?
            .peer_pubkeys();
        let mut sent = 0usize;
        for participant in participants {
            if self.send_control_frame(&participant, frame).await.is_ok() {
                sent += 1;
            }
        }
        Ok(sent)
    }

    async fn send_control_frame(&self, participant: &str, frame: &FipsControlFrame) -> Result<()> {
        let endpoint_npub = self
            .mesh
            .read()
            .map_err(|_| anyhow!("FIPS mesh route table lock poisoned"))?
            .peer_endpoint_npub(participant)
            .ok_or_else(|| anyhow!("no FIPS endpoint peer for {participant}"))?;
        let encoded = encode_fips_control_frame(frame)?;
        self.endpoint
            .send(endpoint_npub, encoded.clone())
            .await
            .with_context(|| format!("failed to send FIPS control frame to {participant}"))?;
        self.note_tx(participant, encoded.len())?;
        Ok(())
    }

    fn source_pubkey(&self, source_npub: Option<&str>) -> Option<String> {
        let source_npub = source_npub?;
        self.mesh
            .read()
            .ok()
            .and_then(|mesh| mesh.participant_for_endpoint_npub(source_npub))
            .or_else(|| normalize_nostr_pubkey(source_npub).ok())
    }

    fn note_tx(&self, participant: &str, len: usize) -> Result<()> {
        let participant = normalize_nostr_pubkey(participant)?;
        let mut presence = self
            .presence
            .write()
            .map_err(|_| anyhow!("FIPS mesh presence lock poisoned"))?;
        let entry = presence.entry(participant).or_default();
        entry.tx_bytes = entry.tx_bytes.saturating_add(len as u64);
        Ok(())
    }

    fn note_rx(&self, participant: &str, len: usize, now: u64) -> Result<()> {
        let participant = normalize_nostr_pubkey(participant)?;
        let mut presence = self
            .presence
            .write()
            .map_err(|_| anyhow!("FIPS mesh presence lock poisoned"))?;
        let entry = presence.entry(participant).or_default();
        entry.last_seen_at = Some(now);
        entry.rx_bytes = entry.rx_bytes.saturating_add(len as u64);
        entry.error = None;
        Ok(())
    }

    pub(crate) async fn shutdown(self) -> Result<(), FipsEndpointError> {
        self.endpoint.shutdown().await
    }
}

#[derive(Debug, Clone)]
struct FipsEndpointTransportConfig {
    listen_port: u16,
    advertised_endpoint: String,
    stun_servers: Vec<String>,
}

fn fips_endpoint_config(
    scope: &str,
    relays: &[String],
    peers: &[FipsMeshPeerConfig],
    transport: Option<&FipsEndpointTransportConfig>,
) -> Config {
    let mut config = Config::new();
    config.node.discovery.nostr.enabled = !relays.is_empty();
    config.node.discovery.nostr.advertise = !relays.is_empty() && transport.is_some();
    config.node.discovery.nostr.policy = NostrDiscoveryPolicy::ConfiguredOnly;
    config.node.discovery.nostr.app = scope.to_string();
    if !relays.is_empty() {
        config.node.discovery.nostr.advert_relays = relays.to_vec();
        config.node.discovery.nostr.dm_relays = relays.to_vec();
    }
    let bind_addr = transport.map(fips_udp_bind_addr);
    let external_addr = transport.and_then(fips_udp_external_addr);
    let advertise_udp = !relays.is_empty() && transport.is_some();
    if let Some(transport) = transport {
        config.node.discovery.nostr.stun_servers = transport.stun_servers.clone();
    }
    config.transports.udp = TransportInstances::Single(UdpConfig {
        bind_addr,
        advertise_on_nostr: Some(advertise_udp),
        public: Some(external_addr.is_some()),
        external_addr,
        outbound_only: Some(transport.is_none()),
        accept_connections: Some(transport.is_some()),
        ..UdpConfig::default()
    });
    if !relays.is_empty() {
        config.peers = peers
            .iter()
            .map(|peer| FipsPeerConfig {
                npub: peer.endpoint_npub.clone(),
                alias: None,
                addresses: Vec::new(),
                connect_policy: ConnectPolicy::AutoConnect,
                auto_reconnect: true,
                via_nostr: true,
            })
            .collect();
    }
    config
}

fn fips_udp_bind_addr(transport: &FipsEndpointTransportConfig) -> String {
    SocketAddr::V4(SocketAddrV4::new(
        std::net::Ipv4Addr::UNSPECIFIED,
        transport.listen_port,
    ))
    .to_string()
}

fn fips_udp_external_addr(transport: &FipsEndpointTransportConfig) -> Option<String> {
    let endpoint = transport.advertised_endpoint.trim();
    if endpoint.is_empty() {
        return None;
    }
    endpoint.parse::<SocketAddr>().ok()?;
    Some(endpoint.to_string())
}

#[derive(Debug, Clone)]
pub(crate) struct FipsPrivateTunnelConfig {
    pub(crate) identity_nsec: String,
    pub(crate) network_id: String,
    pub(crate) relays: Vec<String>,
    pub(crate) iface: String,
    pub(crate) local_address: String,
    pub(crate) listen_port: u16,
    pub(crate) advertised_endpoint: String,
    pub(crate) stun_servers: Vec<String>,
    pub(crate) peers: Vec<FipsMeshPeerConfig>,
    pub(crate) route_targets: Vec<String>,
    pub(crate) local_advertised_routes: Vec<String>,
    #[cfg(target_os = "linux")]
    pub(crate) control_plane_bypass_hosts: Vec<Ipv4Addr>,
}

impl FipsPrivateTunnelConfig {
    pub(crate) fn from_app(
        app: &AppConfig,
        network_id: &str,
        iface: impl Into<String>,
        relays: &[String],
        own_pubkey: Option<&str>,
    ) -> Result<Self> {
        let mut peers = Vec::new();
        let mut route_targets = Vec::new();
        let participants = app.participant_pubkeys_hex();
        let mut route_by_participant = HashMap::<String, Vec<String>>::new();
        for participant in participants {
            if Some(participant.as_str()) == own_pubkey {
                continue;
            }
            let Some(tunnel_ip) = derive_mesh_tunnel_ip(network_id, &participant) else {
                continue;
            };
            let allowed_ip = format!("{}/32", strip_cidr(&tunnel_ip));
            route_targets.push(allowed_ip.clone());
            route_by_participant
                .entry(participant.clone())
                .or_default()
                .push(allowed_ip);
            if app.exit_node == participant {
                let exit_routes = exit_node_default_routes();
                route_targets.extend(exit_routes.iter().cloned());
                route_by_participant
                    .entry(participant)
                    .or_default()
                    .extend(exit_routes);
            }
        }

        for participant in app
            .active_network_signal_pubkeys_hex()
            .into_iter()
            .filter(|participant| Some(participant.as_str()) != own_pubkey)
        {
            let mut allowed_ips = route_by_participant
                .remove(&participant)
                .unwrap_or_default();
            allowed_ips.sort();
            allowed_ips.dedup();
            peers.push(FipsMeshPeerConfig::from_participant_pubkey(
                participant,
                allowed_ips,
            )?);
        }
        peers.sort_by(|left, right| left.participant_pubkey.cmp(&right.participant_pubkey));
        peers.dedup_by(|left, right| left.participant_pubkey == right.participant_pubkey);
        route_targets.sort();
        route_targets.dedup();

        Ok(Self {
            identity_nsec: app.nostr.secret_key.clone(),
            network_id: network_id.to_string(),
            relays: relays.to_vec(),
            iface: iface.into(),
            local_address: own_pubkey
                .and_then(|pubkey| derive_mesh_tunnel_ip(network_id, pubkey))
                .map(|tunnel_ip| local_interface_address_for_tunnel(&tunnel_ip))
                .unwrap_or_else(|| local_interface_address_for_tunnel(&app.node.tunnel_ip)),
            listen_port: app.node.listen_port,
            advertised_endpoint: app.node.endpoint.clone(),
            stun_servers: app.nat.stun_servers.clone(),
            peers,
            route_targets,
            local_advertised_routes: app.effective_advertised_routes(),
            #[cfg(target_os = "linux")]
            control_plane_bypass_hosts: crate::control_plane_bypass_ipv4_hosts(app),
        })
    }
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

#[cfg(target_os = "linux")]
fn endpoint_transport_ipv4_host(addr: &str) -> Option<Ipv4Addr> {
    if let Ok(socket_addr) = addr.parse::<SocketAddr>() {
        return match socket_addr.ip() {
            std::net::IpAddr::V4(ip) => Some(ip),
            std::net::IpAddr::V6(_) => None,
        };
    }

    let (host, _) = crate::split_host_port(addr, 0)?;
    host.parse::<Ipv4Addr>().ok()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) struct FipsPrivateTunnelRuntime {
    iface: String,
    mesh: Arc<FipsPrivateMeshRuntime>,
    config: FipsPrivateTunnelConfig,
    tun_read_task: JoinHandle<()>,
    mesh_send_task: JoinHandle<()>,
    mesh_recv_task: JoinHandle<()>,
    event_rx: mpsc::Receiver<FipsPrivateMeshEvent>,
    #[cfg(target_os = "linux")]
    endpoint_bypass_routes: Vec<String>,
    #[cfg(target_os = "linux")]
    original_default_route: Option<String>,
    #[cfg(target_os = "linux")]
    exit_node_runtime: crate::LinuxExitNodeRuntime,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FipsPrivateTunnelRuntime {
    pub(crate) async fn start(config: FipsPrivateTunnelConfig) -> Result<Self> {
        let scope = format!("nostr-vpn:{}", config.network_id.trim());
        let transport = FipsEndpointTransportConfig {
            listen_port: config.listen_port,
            advertised_endpoint: config.advertised_endpoint.clone(),
            stun_servers: config.stun_servers.clone(),
        };
        let endpoint_config =
            fips_endpoint_config(&scope, &config.relays, &config.peers, Some(&transport));
        let mesh = Arc::new(
            FipsPrivateMeshRuntime::bind_with_config(
                config.identity_nsec.clone(),
                scope,
                config.peers.clone(),
                endpoint_config,
            )
            .await?,
        );
        let tun = Arc::new(
            TunSocket::new(&config.iface)
                .with_context(|| format!("failed to create FIPS tunnel {}", config.iface))?
                .set_non_blocking()
                .context("failed to set FIPS tunnel nonblocking")?,
        );
        let iface = tun.name().context("failed to read FIPS tunnel name")?;

        let (packet_tx, mut packet_rx) = mpsc::channel::<Vec<u8>>(1024);
        let (event_tx, event_rx) = mpsc::channel::<FipsPrivateMeshEvent>(1024);
        let tun_read_task = spawn_tun_read_task(Arc::clone(&tun), packet_tx);
        let mesh_send_task = {
            let mesh = Arc::clone(&mesh);
            tokio::spawn(async move {
                while let Some(packet) = packet_rx.recv().await {
                    if let Err(error) = mesh.send_tunnel_packet(&packet).await {
                        eprintln!("fips: failed to send tunnel packet: {error}");
                    }
                }
            })
        };
        let mesh_recv_task = spawn_mesh_recv_task(Arc::clone(&mesh), tun, event_tx);

        let mut runtime = Self {
            iface,
            mesh,
            config: config.clone(),
            tun_read_task,
            mesh_send_task,
            mesh_recv_task,
            event_rx,
            #[cfg(target_os = "linux")]
            endpoint_bypass_routes: Vec::new(),
            #[cfg(target_os = "linux")]
            original_default_route: None,
            #[cfg(target_os = "linux")]
            exit_node_runtime: crate::LinuxExitNodeRuntime::default(),
        };
        runtime.apply_interface_config(&config).await?;
        Ok(runtime)
    }

    pub(crate) fn iface(&self) -> &str {
        &self.iface
    }

    pub(crate) fn peer_statuses(&self) -> Vec<MeshPeerStatus> {
        self.mesh.peer_statuses()
    }

    pub(crate) fn peer_pubkeys(&self) -> Vec<String> {
        self.mesh.peer_pubkeys()
    }

    pub(crate) async fn apply_config(&mut self, config: FipsPrivateTunnelConfig) -> Result<()> {
        self.mesh.replace_peers(config.peers.clone())?;
        self.apply_interface_config(&config).await?;
        self.config = config;
        Ok(())
    }

    pub(crate) async fn refresh_peer_dependent_routes(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        if !crate::route_targets_require_endpoint_bypass(&self.config.route_targets) {
            return Ok(());
        }

        let config = self.config.clone();
        self.apply_interface_config(&config).await
    }

    pub(crate) async fn ping_peers(&self, network_id: &str, now: u64) -> Result<usize> {
        self.mesh.ping_peers(network_id, now).await
    }

    pub(crate) async fn send_join_request(
        &self,
        participant: &str,
        requested_at: u64,
        request: MeshJoinRequest,
    ) -> Result<()> {
        self.mesh
            .send_join_request(participant, requested_at, request)
            .await
    }

    pub(crate) async fn send_roster(
        &self,
        participant: &str,
        network_id: &str,
        roster: NetworkRoster,
    ) -> Result<()> {
        self.mesh.send_roster(participant, network_id, roster).await
    }

    pub(crate) fn drain_events(&mut self) -> Vec<FipsPrivateMeshEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    pub(crate) async fn stop(self) -> Result<()> {
        #[cfg(target_os = "linux")]
        let mut runtime = self;
        #[cfg(not(target_os = "linux"))]
        let runtime = self;
        #[cfg(target_os = "linux")]
        runtime.cleanup_linux_network_state();
        runtime.tun_read_task.abort();
        runtime.mesh_send_task.abort();
        runtime.mesh_recv_task.abort();
        let _ = runtime.tun_read_task.await;
        let _ = runtime.mesh_send_task.await;
        let _ = runtime.mesh_recv_task.await;
        if let Ok(mesh) = Arc::try_unwrap(runtime.mesh) {
            mesh.shutdown()
                .await
                .context("failed to stop FIPS endpoint")?;
        }
        Ok(())
    }

    async fn apply_interface_config(&mut self, config: &FipsPrivateTunnelConfig) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.apply_linux_network_state(config).await?;
        }
        #[cfg(target_os = "macos")]
        {
            crate::apply_local_interface_network(
                &self.iface,
                &config.local_address,
                &config.route_targets,
            )
            .with_context(|| format!("failed to configure FIPS tunnel interface {}", self.iface))?;
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    async fn apply_linux_network_state(&mut self, config: &FipsPrivateTunnelConfig) -> Result<()> {
        let mut route_targets = config.route_targets.clone();
        let mut peer_endpoint_hosts = Vec::new();
        if crate::route_targets_require_endpoint_bypass(&route_targets) {
            peer_endpoint_hosts = self.mesh.peer_transport_ipv4_hosts().await?;
            if route_targets.iter().any(|route| route == "0.0.0.0/0")
                && peer_endpoint_hosts.is_empty()
            {
                eprintln!(
                    "fips: withholding default route until the selected exit peer underlay endpoint is known"
                );
                route_targets.retain(|route| !crate::is_exit_node_route(route));
            }
        }

        if route_targets.iter().any(|route| route == "0.0.0.0/0") {
            self.capture_linux_original_default_route();
        } else {
            self.restore_linux_original_default_route();
        }

        let endpoint_bypass_specs = if crate::route_targets_require_endpoint_bypass(&route_targets)
        {
            let mut bypass_hosts = config.control_plane_bypass_hosts.clone();
            bypass_hosts.extend(peer_endpoint_hosts);
            bypass_hosts.sort_unstable();
            bypass_hosts.dedup();
            crate::linux_bypass_route_specs_for_hosts(
                bypass_hosts,
                &self.iface,
                self.original_default_route.as_deref(),
            )?
        } else {
            Vec::new()
        };
        self.reconcile_linux_endpoint_bypass_routes(&endpoint_bypass_specs);

        crate::apply_local_interface_network(&self.iface, &config.local_address, &route_targets)
            .with_context(|| format!("failed to configure FIPS tunnel interface {}", self.iface))?;
        if let Err(error) = crate::flush_linux_route_cache() {
            eprintln!("fips: failed to flush linux route cache: {error}");
        }
        self.reconcile_linux_exit_node_forwarding(
            &config.local_address,
            &config.local_advertised_routes,
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn capture_linux_original_default_route(&mut self) {
        if self.original_default_route.is_some() {
            return;
        }
        match crate::linux_default_route() {
            Ok(route) => self.original_default_route = Some(route.line),
            Err(error) => eprintln!("fips: failed to capture original default route: {error}"),
        }
    }

    #[cfg(target_os = "linux")]
    fn restore_linux_original_default_route(&mut self) {
        let Some(route) = self.original_default_route.take() else {
            return;
        };
        if let Err(error) = crate::restore_linux_default_route(&route) {
            eprintln!("fips: failed to restore original default route: {error}");
            self.original_default_route = Some(route);
        }
    }

    #[cfg(target_os = "linux")]
    fn reconcile_linux_endpoint_bypass_routes(
        &mut self,
        routes: &[crate::LinuxEndpointBypassRoute],
    ) {
        let desired = routes
            .iter()
            .map(|route| route.target.clone())
            .collect::<std::collections::HashSet<_>>();

        let stale = self
            .endpoint_bypass_routes
            .iter()
            .filter(|route| !desired.contains(*route))
            .cloned()
            .collect::<Vec<_>>();
        for route in stale {
            if let Err(error) = crate::delete_linux_endpoint_bypass_route(&route) {
                eprintln!("fips: failed to remove endpoint bypass route {route}: {error}");
            }
        }

        for route in routes {
            if let Err(error) = crate::apply_linux_endpoint_bypass_route(route) {
                eprintln!(
                    "fips: failed to install endpoint bypass route {}: {}",
                    route.target, error
                );
            }
        }

        self.endpoint_bypass_routes = desired.into_iter().collect();
        self.endpoint_bypass_routes.sort();
    }

    #[cfg(target_os = "linux")]
    fn reconcile_linux_exit_node_forwarding(&mut self, local_address: &str, routes: &[String]) {
        let mut route_families = crate::linux_exit_node_default_route_families(routes);
        if !route_families.ipv4 && !route_families.ipv6 {
            self.reconcile_linux_exit_node_forwarding_cleanup();
            return;
        }

        let ipv4_tunnel_source_cidr = if route_families.ipv4 {
            let Some(tunnel_source_cidr) = crate::linux_exit_node_source_cidr(local_address) else {
                eprintln!("fips: invalid IPv4 tunnel address '{local_address}'");
                self.reconcile_linux_exit_node_forwarding_cleanup();
                return;
            };
            Some(tunnel_source_cidr)
        } else {
            None
        };

        let ipv4_outbound_iface = if route_families.ipv4 {
            match crate::linux_default_route() {
                Ok(route) => Some(route.dev),
                Err(error) => {
                    eprintln!("fips: failed to resolve default IPv4 route device: {error}");
                    self.reconcile_linux_exit_node_forwarding_cleanup();
                    return;
                }
            }
        } else {
            None
        };

        let ipv6_outbound_iface = if route_families.ipv6 {
            match crate::linux_default_ipv6_route() {
                Ok(route) => Some(route.dev),
                Err(error) => {
                    eprintln!(
                        "fips: skipping IPv6 forwarding (default route unavailable): {error}"
                    );
                    route_families.ipv6 = false;
                    None
                }
            }
        } else {
            None
        };

        if !route_families.ipv4 && !route_families.ipv6 {
            self.reconcile_linux_exit_node_forwarding_cleanup();
            return;
        }

        let already_configured = self.exit_node_runtime.ipv4_outbound_iface == ipv4_outbound_iface
            && self.exit_node_runtime.ipv6_outbound_iface == ipv6_outbound_iface
            && self.exit_node_runtime.ipv4_tunnel_source_cidr == ipv4_tunnel_source_cidr;
        if already_configured {
            return;
        }

        self.reconcile_linux_exit_node_forwarding_cleanup();

        self.exit_node_runtime.ipv4_outbound_iface = ipv4_outbound_iface.clone();
        self.exit_node_runtime.ipv6_outbound_iface = ipv6_outbound_iface.clone();
        self.exit_node_runtime.ipv4_tunnel_source_cidr = ipv4_tunnel_source_cidr.clone();

        if route_families.ipv4 {
            match crate::read_linux_ip_forward(crate::LinuxExitNodeIpFamily::V4) {
                Ok(previous) => {
                    self.exit_node_runtime.ipv4_forward_was_enabled = Some(previous);
                    if !previous
                        && let Err(error) =
                            crate::write_linux_ip_forward(crate::LinuxExitNodeIpFamily::V4, true)
                    {
                        eprintln!("fips: failed to enable IPv4 forwarding: {error}");
                        self.reconcile_linux_exit_node_forwarding_cleanup();
                        return;
                    }
                }
                Err(error) => {
                    eprintln!("fips: failed to read IPv4 forwarding state: {error}");
                    self.reconcile_linux_exit_node_forwarding_cleanup();
                    return;
                }
            }
        }

        if route_families.ipv6 {
            match crate::read_linux_ip_forward(crate::LinuxExitNodeIpFamily::V6) {
                Ok(previous) => {
                    self.exit_node_runtime.ipv6_forward_was_enabled = Some(previous);
                    if !previous
                        && let Err(error) =
                            crate::write_linux_ip_forward(crate::LinuxExitNodeIpFamily::V6, true)
                    {
                        eprintln!("fips: skipping IPv6 forwarding setup: {error}");
                        self.exit_node_runtime.ipv6_forward_was_enabled = None;
                        self.exit_node_runtime.ipv6_outbound_iface = None;
                        route_families.ipv6 = false;
                    }
                }
                Err(error) => {
                    eprintln!("fips: skipping IPv6 forwarding state check: {error}");
                    self.exit_node_runtime.ipv6_forward_was_enabled = None;
                    self.exit_node_runtime.ipv6_outbound_iface = None;
                    route_families.ipv6 = false;
                }
            }
        }

        if let (Some(outbound_iface), Some(tunnel_source_cidr)) = (
            ipv4_outbound_iface.as_deref(),
            ipv4_tunnel_source_cidr.as_deref(),
        ) {
            let forward_in = crate::linux_exit_node_forward_in_rule(
                &self.iface,
                crate::LinuxExitNodeIpFamily::V4,
            );
            let forward_out = crate::linux_exit_node_forward_out_rule(
                &self.iface,
                crate::LinuxExitNodeIpFamily::V4,
            );
            let masquerade =
                crate::linux_exit_node_ipv4_masquerade_rule(outbound_iface, tunnel_source_cidr);

            if let Err(error) = crate::linux_iptables_ensure_rule(
                crate::LinuxExitNodeIpFamily::V4,
                None,
                &forward_in,
            )
            .and_then(|()| {
                crate::linux_iptables_ensure_rule(
                    crate::LinuxExitNodeIpFamily::V4,
                    None,
                    &forward_out,
                )
            })
            .and_then(|()| {
                crate::linux_iptables_ensure_rule(
                    crate::LinuxExitNodeIpFamily::V4,
                    Some("nat"),
                    &masquerade,
                )
            }) {
                eprintln!("fips: failed to install IPv4 exit firewall rules: {error}");
                self.reconcile_linux_exit_node_forwarding_cleanup();
                return;
            }
        }

        if route_families.ipv6 {
            let forward_in = crate::linux_exit_node_forward_in_rule(
                &self.iface,
                crate::LinuxExitNodeIpFamily::V6,
            );
            let forward_out = crate::linux_exit_node_forward_out_rule(
                &self.iface,
                crate::LinuxExitNodeIpFamily::V6,
            );

            if let Err(error) = crate::linux_iptables_ensure_rule(
                crate::LinuxExitNodeIpFamily::V6,
                None,
                &forward_in,
            )
            .and_then(|()| {
                crate::linux_iptables_ensure_rule(
                    crate::LinuxExitNodeIpFamily::V6,
                    None,
                    &forward_out,
                )
            }) {
                eprintln!("fips: skipping IPv6 exit firewall rules: {error}");
                self.exit_node_runtime.ipv6_outbound_iface = None;
                self.exit_node_runtime.ipv6_forward_was_enabled = None;
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn reconcile_linux_exit_node_forwarding_cleanup(&mut self) {
        if let (Some(outbound_iface), Some(tunnel_source_cidr)) = (
            self.exit_node_runtime.ipv4_outbound_iface.as_deref(),
            self.exit_node_runtime.ipv4_tunnel_source_cidr.as_deref(),
        ) {
            let forward_in = crate::linux_exit_node_forward_in_rule(
                &self.iface,
                crate::LinuxExitNodeIpFamily::V4,
            );
            let forward_out = crate::linux_exit_node_forward_out_rule(
                &self.iface,
                crate::LinuxExitNodeIpFamily::V4,
            );
            let masquerade =
                crate::linux_exit_node_ipv4_masquerade_rule(outbound_iface, tunnel_source_cidr);

            if let Err(error) = crate::linux_iptables_delete_rule(
                crate::LinuxExitNodeIpFamily::V4,
                Some("nat"),
                &masquerade,
            ) {
                eprintln!("fips: failed to remove masquerade rule: {error}");
            }
            if let Err(error) = crate::linux_iptables_delete_rule(
                crate::LinuxExitNodeIpFamily::V4,
                None,
                &forward_out,
            ) {
                eprintln!("fips: failed to remove forward-out rule: {error}");
            }
            if let Err(error) = crate::linux_iptables_delete_rule(
                crate::LinuxExitNodeIpFamily::V4,
                None,
                &forward_in,
            ) {
                eprintln!("fips: failed to remove forward-in rule: {error}");
            }
        }

        if self.exit_node_runtime.ipv6_outbound_iface.is_some() {
            let forward_in = crate::linux_exit_node_forward_in_rule(
                &self.iface,
                crate::LinuxExitNodeIpFamily::V6,
            );
            let forward_out = crate::linux_exit_node_forward_out_rule(
                &self.iface,
                crate::LinuxExitNodeIpFamily::V6,
            );

            if let Err(error) = crate::linux_iptables_delete_rule(
                crate::LinuxExitNodeIpFamily::V6,
                None,
                &forward_out,
            ) {
                eprintln!("fips: failed to remove IPv6 forward-out rule: {error}");
            }
            if let Err(error) = crate::linux_iptables_delete_rule(
                crate::LinuxExitNodeIpFamily::V6,
                None,
                &forward_in,
            ) {
                eprintln!("fips: failed to remove IPv6 forward-in rule: {error}");
            }
        }

        if self.exit_node_runtime.ipv4_forward_was_enabled == Some(false)
            && let Err(error) =
                crate::write_linux_ip_forward(crate::LinuxExitNodeIpFamily::V4, false)
        {
            eprintln!("fips: failed to restore IPv4 forwarding state: {error}");
        }
        if self.exit_node_runtime.ipv6_forward_was_enabled == Some(false)
            && let Err(error) =
                crate::write_linux_ip_forward(crate::LinuxExitNodeIpFamily::V6, false)
        {
            eprintln!("fips: failed to restore IPv6 forwarding state: {error}");
        }

        self.exit_node_runtime = crate::LinuxExitNodeRuntime::default();
    }

    #[cfg(target_os = "linux")]
    fn cleanup_linux_network_state(&mut self) {
        self.reconcile_linux_endpoint_bypass_routes(&[]);
        self.reconcile_linux_exit_node_forwarding_cleanup();
        self.restore_linux_original_default_route();
        if let Err(error) = crate::flush_linux_route_cache() {
            eprintln!("fips: failed to flush linux route cache: {error}");
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_tun_read_task(tun: Arc<TunSocket>, packet_tx: mpsc::Sender<Vec<u8>>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = vec![0_u8; 65_535];
        loop {
            match tun.read(&mut buf) {
                Ok([]) => {
                    sleep(Duration::from_millis(10)).await;
                }
                Ok(packet) => {
                    if packet_tx.send(packet.to_vec()).await.is_err() {
                        break;
                    }
                }
                Err(error) if temporary_tun_read_error(&error) => {
                    sleep(Duration::from_millis(10)).await;
                }
                Err(error) => {
                    eprintln!("fips: tunnel read failed: {error}");
                    sleep(Duration::from_millis(100)).await;
                }
            }
        }
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_mesh_recv_task(
    mesh: Arc<FipsPrivateMeshRuntime>,
    tun: Arc<TunSocket>,
    event_tx: mpsc::Sender<FipsPrivateMeshEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match mesh.recv_mesh_event().await {
                Ok(Some(FipsPrivateMeshEvent::Packet(packet))) => {
                    write_packet_to_tun(&tun, &packet.bytes);
                    let _ = event_tx.send(FipsPrivateMeshEvent::Packet(packet)).await;
                }
                Ok(Some(event)) => {
                    if event_tx.send(event).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    eprintln!("fips: failed to receive tunnel packet: {error}");
                    sleep(Duration::from_millis(100)).await;
                }
            }
        }
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_packet_to_tun(tun: &TunSocket, packet: &[u8]) {
    match packet.first().map(|byte| byte >> 4) {
        Some(4) => {
            let _ = tun.write4(packet);
        }
        Some(6) => {
            let _ = tun.write6(packet);
        }
        _ => {}
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn temporary_tun_read_error(error: &TunError) -> bool {
    match error {
        TunError::IfaceRead(source) => matches!(
            source.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
        ),
        _ => false,
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) struct FipsPrivateTunnelRuntime;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl FipsPrivateTunnelRuntime {
    pub(crate) async fn start(_config: FipsPrivateTunnelConfig) -> Result<Self> {
        Err(anyhow!(
            "FIPS private tunnel runtime is not implemented for this platform"
        ))
    }

    pub(crate) fn iface(&self) -> &str {
        ""
    }

    pub(crate) fn peer_statuses(&self) -> Vec<MeshPeerStatus> {
        Vec::new()
    }

    pub(crate) fn peer_pubkeys(&self) -> Vec<String> {
        Vec::new()
    }

    pub(crate) async fn apply_config(&self, _config: FipsPrivateTunnelConfig) -> Result<()> {
        Ok(())
    }

    pub(crate) async fn refresh_peer_dependent_routes(&self) -> Result<()> {
        Ok(())
    }

    pub(crate) async fn ping_peers(&self, _network_id: &str, _now: u64) -> Result<usize> {
        Ok(0)
    }

    pub(crate) async fn send_join_request(
        &self,
        _participant: &str,
        _requested_at: u64,
        _request: MeshJoinRequest,
    ) -> Result<()> {
        Ok(())
    }

    pub(crate) async fn send_roster(
        &self,
        _participant: &str,
        _network_id: &str,
        _roster: NetworkRoster,
    ) -> Result<()> {
        Ok(())
    }

    pub(crate) fn drain_events(&mut self) -> Vec<FipsPrivateMeshEvent> {
        Vec::new()
    }

    pub(crate) async fn stop(self) -> Result<()> {
        Ok(())
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        FipsEndpointTransportConfig, FipsPrivateMeshRuntime, FipsPrivateTunnelConfig,
        fips_endpoint_config,
    };
    use fips_endpoint::{Config, PeerConfig as FipsPeerConfig, TransportInstances, UdpConfig};
    use nostr_sdk::prelude::{Keys, ToBech32};
    use nostr_vpn_core::config::{AppConfig, derive_mesh_tunnel_ip};
    use nostr_vpn_core::fips_mesh::FipsMeshPeerConfig;
    use std::net::{Ipv4Addr, UdpSocket};
    use std::time::Duration;

    fn ipv4_packet(source: Ipv4Addr, destination: Ipv4Addr) -> Vec<u8> {
        let payload = [0xde, 0xad, 0xbe, 0xef];
        let total_len = 20 + payload.len();
        let mut packet = vec![0_u8; total_len];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        packet[8] = 64;
        packet[9] = 17;
        packet[12..16].copy_from_slice(&source.octets());
        packet[16..20].copy_from_slice(&destination.octets());
        packet[20..].copy_from_slice(&payload);
        packet
    }

    #[tokio::test]
    async fn endpoint_data_runtime_sends_and_receives_raw_packets() {
        let keys = Keys::generate();
        let nsec = keys.secret_key().to_bech32().expect("nsec");
        let participant_pubkey = keys.public_key().to_hex();
        let source = Ipv4Addr::new(10, 44, 10, 1);
        let destination = Ipv4Addr::new(10, 44, 22, 44);

        // The FIPS endpoint self-loop is used only to exercise send/recv
        // without external discovery. Real peers should not own both routes.
        let peer = FipsMeshPeerConfig::from_participant_pubkey(
            &participant_pubkey,
            vec![format!("{source}/32"), format!("{destination}/32")],
        )
        .expect("peer config");
        let runtime = FipsPrivateMeshRuntime::bind(nsec, "test-network", vec![peer])
            .await
            .expect("runtime should bind");
        let packet = ipv4_packet(source, destination);

        let sent = runtime
            .send_tunnel_packet(&packet)
            .await
            .expect("send packet");
        assert!(sent);

        let received = tokio::time::timeout(Duration::from_secs(2), runtime.recv_tunnel_packet())
            .await
            .expect("packet should arrive")
            .expect("receive packet")
            .expect("packet should pass admission");

        assert_eq!(received.source_pubkey, participant_pubkey);
        assert_eq!(received.bytes, packet);
        runtime.shutdown().await.expect("shutdown");
    }

    fn available_udp_port() -> u16 {
        UdpSocket::bind("127.0.0.1:0")
            .expect("bind test port")
            .local_addr()
            .expect("local addr")
            .port()
    }

    #[test]
    fn tunnel_config_routes_default_through_selected_exit_peer() {
        let alice_keys = Keys::generate();
        let bob_keys = Keys::generate();
        let carol_keys = Keys::generate();
        let alice_nsec = alice_keys.secret_key().to_bech32().expect("alice nsec");
        let alice_pubkey = alice_keys.public_key().to_hex();
        let bob_pubkey = bob_keys.public_key().to_hex();
        let carol_pubkey = carol_keys.public_key().to_hex();
        let network_id = "fips-exit-route-test";
        let bob_tunnel_ip = derive_mesh_tunnel_ip(network_id, &bob_pubkey).expect("bob tunnel ip");

        let mut app = AppConfig::default();
        app.nostr.secret_key = alice_nsec;
        app.networks[0].network_id = network_id.to_string();
        app.networks[0].participants = vec![
            alice_pubkey.clone(),
            bob_pubkey.clone(),
            carol_pubkey.clone(),
        ];
        app.exit_node = bob_pubkey.clone();

        let config = FipsPrivateTunnelConfig::from_app(
            &app,
            network_id,
            "utun-test",
            &[],
            Some(&alice_pubkey),
        )
        .expect("fips tunnel config");
        let bob_peer = config
            .peers
            .iter()
            .find(|peer| peer.participant_pubkey == bob_pubkey)
            .expect("bob peer");
        let carol_peer = config
            .peers
            .iter()
            .find(|peer| peer.participant_pubkey == carol_pubkey)
            .expect("carol peer");

        assert!(bob_peer.allowed_ips.contains(&bob_tunnel_ip));
        assert!(bob_peer.allowed_ips.contains(&"0.0.0.0/0".to_string()));
        assert!(bob_peer.allowed_ips.contains(&"::/0".to_string()));
        assert!(!carol_peer.allowed_ips.contains(&"0.0.0.0/0".to_string()));
        assert!(config.route_targets.contains(&"0.0.0.0/0".to_string()));
        assert!(config.route_targets.contains(&"::/0".to_string()));
    }

    fn direct_udp_endpoint_config(local_port: u16, peer_npub: &str, peer_port: u16) -> Config {
        let mut config = Config::new();
        config.transports.udp = TransportInstances::Single(UdpConfig {
            bind_addr: Some(format!("127.0.0.1:{local_port}")),
            accept_connections: Some(true),
            ..UdpConfig::default()
        });
        config.peers.push(FipsPeerConfig::new(
            peer_npub,
            "udp",
            format!("127.0.0.1:{peer_port}"),
        ));
        config
    }

    async fn send_with_retry(runtime: &FipsPrivateMeshRuntime, packet: &[u8]) {
        let mut last_error = None;
        for _ in 0..50 {
            match runtime.send_tunnel_packet(packet).await {
                Ok(true) => return,
                Ok(false) => panic!("packet had no FIPS route"),
                Err(error) => {
                    last_error = Some(error);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
        panic!(
            "packet did not send after retry: {}",
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "unknown error".to_string())
        );
    }

    #[tokio::test]
    async fn two_local_endpoints_exchange_raw_packets_over_fips() {
        let alice_keys = Keys::generate();
        let bob_keys = Keys::generate();
        let alice_nsec = alice_keys.secret_key().to_bech32().expect("alice nsec");
        let bob_nsec = bob_keys.secret_key().to_bech32().expect("bob nsec");
        let alice_pubkey = alice_keys.public_key().to_hex();
        let bob_pubkey = bob_keys.public_key().to_hex();
        let alice_npub = alice_keys.public_key().to_bech32().expect("alice npub");
        let bob_npub = bob_keys.public_key().to_bech32().expect("bob npub");
        let alice_port = available_udp_port();
        let bob_port = available_udp_port();
        let alice_ip = Ipv4Addr::new(10, 44, 11, 1);
        let bob_ip = Ipv4Addr::new(10, 44, 11, 2);
        let scope = "nostr-vpn:two-local-endpoints";

        let alice_runtime = FipsPrivateMeshRuntime::bind_with_config(
            alice_nsec,
            scope,
            vec![FipsMeshPeerConfig {
                participant_pubkey: bob_pubkey.clone(),
                endpoint_npub: bob_npub.clone(),
                allowed_ips: vec![format!("{bob_ip}/32")],
            }],
            direct_udp_endpoint_config(alice_port, &bob_npub, bob_port),
        )
        .await
        .expect("alice endpoint should bind");
        let bob_runtime = FipsPrivateMeshRuntime::bind_with_config(
            bob_nsec,
            scope,
            vec![FipsMeshPeerConfig {
                participant_pubkey: alice_pubkey.clone(),
                endpoint_npub: alice_npub.clone(),
                allowed_ips: vec![format!("{alice_ip}/32")],
            }],
            direct_udp_endpoint_config(bob_port, &alice_npub, alice_port),
        )
        .await
        .expect("bob endpoint should bind");

        let alice_to_bob = ipv4_packet(alice_ip, bob_ip);
        send_with_retry(&alice_runtime, &alice_to_bob).await;
        let received =
            tokio::time::timeout(Duration::from_secs(5), bob_runtime.recv_tunnel_packet())
                .await
                .expect("Bob should receive Alice packet")
                .expect("receive packet")
                .expect("packet should pass Bob admission");
        assert_eq!(received.source_pubkey, alice_pubkey);
        assert_eq!(received.bytes, alice_to_bob);

        let bob_to_alice = ipv4_packet(bob_ip, alice_ip);
        send_with_retry(&bob_runtime, &bob_to_alice).await;
        let received =
            tokio::time::timeout(Duration::from_secs(5), alice_runtime.recv_tunnel_packet())
                .await
                .expect("Alice should receive Bob packet")
                .expect("receive packet")
                .expect("packet should pass Alice admission");
        assert_eq!(received.source_pubkey, bob_pubkey);
        assert_eq!(received.bytes, bob_to_alice);

        alice_runtime.shutdown().await.expect("shutdown alice");
        bob_runtime.shutdown().await.expect("shutdown bob");
    }

    #[test]
    fn endpoint_config_uses_client_posture_and_configured_peers_only() {
        let keys = Keys::generate();
        let participant_pubkey = keys.public_key().to_hex();
        let peer = FipsMeshPeerConfig::from_participant_pubkey(
            &participant_pubkey,
            vec!["10.44.1.2/32".to_string()],
        )
        .expect("peer config");
        let relays = vec!["wss://relay.example".to_string()];

        let config = fips_endpoint_config("nostr-vpn:test", &relays, &[peer], None);

        assert!(config.node.discovery.nostr.enabled);
        assert!(!config.node.discovery.nostr.advertise);
        assert_eq!(
            config.node.discovery.nostr.policy,
            fips_endpoint::NostrDiscoveryPolicy::ConfiguredOnly
        );
        assert_eq!(config.node.discovery.nostr.app, "nostr-vpn:test");
        let udp = match config.transports.udp {
            fips_endpoint::TransportInstances::Single(udp) => udp,
            _ => panic!("expected one UDP transport"),
        };
        assert!(udp.outbound_only());
        assert!(!udp.advertise_on_nostr());
        assert!(!udp.accept_connections());
        assert_eq!(config.peers.len(), 1);
        assert!(config.peers[0].via_nostr);
    }

    #[test]
    fn endpoint_config_advertises_app_owned_direct_endpoint_for_tunnel_runtime() {
        let keys = Keys::generate();
        let participant_pubkey = keys.public_key().to_hex();
        let peer = FipsMeshPeerConfig::from_participant_pubkey(
            &participant_pubkey,
            vec!["10.44.1.2/32".to_string()],
        )
        .expect("peer config");
        let relays = vec!["wss://relay.example".to_string()];
        let transport = FipsEndpointTransportConfig {
            listen_port: 51820,
            advertised_endpoint: "192.168.50.20:51820".to_string(),
            stun_servers: vec!["stun:stun.example.org:3478".to_string()],
        };

        let config = fips_endpoint_config("nostr-vpn:test", &relays, &[peer], Some(&transport));

        assert!(config.node.discovery.nostr.enabled);
        assert!(config.node.discovery.nostr.advertise);
        assert_eq!(
            config.node.discovery.nostr.policy,
            fips_endpoint::NostrDiscoveryPolicy::ConfiguredOnly
        );
        assert_eq!(
            config.node.discovery.nostr.stun_servers,
            vec!["stun:stun.example.org:3478".to_string()]
        );
        let udp = match config.transports.udp {
            fips_endpoint::TransportInstances::Single(udp) => udp,
            _ => panic!("expected one UDP transport"),
        };
        assert_eq!(udp.bind_addr.as_deref(), Some("0.0.0.0:51820"));
        assert!(!udp.outbound_only());
        assert!(udp.advertise_on_nostr());
        assert!(udp.accept_connections());
        assert_eq!(udp.external_addr.as_deref(), Some("192.168.50.20:51820"));
        assert_eq!(config.peers.len(), 1);
        assert!(config.peers[0].via_nostr);
    }
}
