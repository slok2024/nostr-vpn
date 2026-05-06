use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fips_endpoint::{
    Config as FipsConfig, ConnectPolicy, FipsEndpoint, NostrDiscoveryPolicy,
    PeerConfig as FipsPeerConfig, TransportInstances, UdpConfig,
};
use nostr_vpn_core::config::{AppConfig, derive_mesh_tunnel_ip, maybe_autoconfigure_node};
use nostr_vpn_core::fips_mesh::{FipsMeshPeerConfig, FipsMeshRuntime};
use serde::{Deserialize, Serialize};
use tokio::runtime::{Builder as RuntimeBuilder, Runtime};
use tokio::sync::mpsc as tokio_mpsc;
use tokio::task::JoinHandle;

const DEFAULT_MOBILE_MTU: u16 = 1280;
const TUNNEL_CHANNEL_CAPACITY: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MobileTunnelConfig {
    pub(crate) identity_nsec: String,
    pub(crate) network_id: String,
    pub(crate) relays: Vec<String>,
    pub(crate) local_address: String,
    pub(crate) mtu: u16,
    pub(crate) peers: Vec<FipsMeshPeerConfig>,
    pub(crate) route_targets: Vec<String>,
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

        let local_address = derive_mesh_tunnel_ip(&network_id, &own_pubkey)
            .map(|tunnel_ip| local_interface_address_for_tunnel(&tunnel_ip))
            .unwrap_or_else(|| local_interface_address_for_tunnel(&app.node.tunnel_ip));

        Ok(Self {
            identity_nsec: app.nostr.secret_key.clone(),
            network_id,
            relays: app.nostr.relays.clone(),
            local_address,
            mtu: DEFAULT_MOBILE_MTU,
            peers,
            route_targets,
            error: String::new(),
        })
    }
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
        let (endpoint, outbound_tx, inbound_rx, tasks) =
            runtime.block_on(Self::start_async(config))?;
        Ok(Self {
            runtime,
            endpoint: Some(endpoint),
            outbound_tx,
            inbound_rx: Mutex::new(inbound_rx),
            tasks,
        })
    }

    async fn start_async(
        config: MobileTunnelConfig,
    ) -> Result<(
        Arc<FipsEndpoint>,
        tokio_mpsc::Sender<Vec<u8>>,
        mpsc::Receiver<Vec<u8>>,
        Vec<JoinHandle<()>>,
    )> {
        let scope = format!("nostr-vpn:{}", config.network_id.trim());
        let endpoint = FipsEndpoint::builder()
            .config(fips_endpoint_config(&scope, &config.relays, &config.peers))
            .identity_nsec(config.identity_nsec)
            .discovery_scope(scope)
            .without_system_tun()
            .bind()
            .await
            .context("failed to bind mobile FIPS endpoint")?;
        let endpoint = Arc::new(endpoint);
        let mesh = Arc::new(RwLock::new(FipsMeshRuntime::new(config.peers)));
        let (outbound_tx, mut outbound_rx) =
            tokio_mpsc::channel::<Vec<u8>>(TUNNEL_CHANNEL_CAPACITY);
        let (inbound_tx, inbound_rx) = mpsc::sync_channel::<Vec<u8>>(TUNNEL_CHANNEL_CAPACITY);

        let send_task = {
            let endpoint = Arc::clone(&endpoint);
            let mesh = Arc::clone(&mesh);
            tokio::spawn(async move {
                while let Some(packet) = outbound_rx.recv().await {
                    let outgoing = mesh
                        .read()
                        .ok()
                        .and_then(|mesh| mesh.route_outbound_packet(&packet));
                    if let Some(outgoing) = outgoing {
                        let _ = endpoint.send(outgoing.endpoint_npub, outgoing.bytes).await;
                    }
                }
            })
        };

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

        Ok((
            endpoint,
            outbound_tx,
            inbound_rx,
            vec![send_task, recv_task],
        ))
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

impl Drop for MobileTunnel {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        let tasks = std::mem::take(&mut self.tasks);
        let endpoint = self.endpoint.take();
        self.runtime.block_on(async move {
            for task in tasks {
                let _ = task.await;
            }
            if let Some(endpoint) = endpoint
                && let Ok(endpoint) = Arc::try_unwrap(endpoint)
            {
                let _ = endpoint.shutdown().await;
            }
        });
    }
}

fn fips_endpoint_config(
    scope: &str,
    relays: &[String],
    peers: &[FipsMeshPeerConfig],
) -> FipsConfig {
    let mut config = FipsConfig::new();
    config.node.discovery.nostr.enabled = !relays.is_empty();
    config.node.discovery.nostr.advertise = false;
    config.node.discovery.nostr.policy = NostrDiscoveryPolicy::ConfiguredOnly;
    config.node.discovery.nostr.app = scope.to_string();
    if !relays.is_empty() {
        config.node.discovery.nostr.advert_relays = relays.to_vec();
        config.node.discovery.nostr.dm_relays = relays.to_vec();
    }
    config.transports.udp = TransportInstances::Single(UdpConfig {
        outbound_only: Some(true),
        accept_connections: Some(false),
        advertise_on_nostr: Some(false),
        public: Some(false),
        ..UdpConfig::default()
    });
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

fn empty_config() -> MobileTunnelConfig {
    MobileTunnelConfig {
        identity_nsec: String::new(),
        network_id: String::new(),
        relays: Vec::new(),
        local_address: String::new(),
        mtu: DEFAULT_MOBILE_MTU,
        peers: Vec::new(),
        route_targets: Vec::new(),
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
}
