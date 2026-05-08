#![allow(dead_code)]

use anyhow::{Context, Result, anyhow};
use fips_endpoint::{
    Config, ConnectPolicy, FipsEndpoint, FipsEndpointError, FipsEndpointPeer, NostrDiscoveryPolicy,
    PeerAddress, PeerConfig as FipsPeerConfig, TransportInstances, UdpConfig,
};
use nostr_sdk::prelude::{PublicKey, ToBech32};
use nostr_vpn_core::config::{
    AppConfig, WireGuardExitConfig, derive_mesh_tunnel_ip, exit_node_default_routes,
    normalize_nostr_pubkey,
};
use nostr_vpn_core::data_plane::{MeshPeerStatus, PrivatePacket};
use nostr_vpn_core::fips_control::{
    FipsControlFrame, NetworkRoster, decode_fips_control_frame, encode_fips_control_frame,
};
use nostr_vpn_core::fips_mesh::{FipsMeshPeerConfig, FipsMeshRuntime};
use nostr_vpn_core::join_requests::MeshJoinRequest;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
#[cfg(target_os = "linux")]
use std::net::Ipv4Addr;
use std::net::{SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::sync::Arc;
use std::sync::RwLock;
#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "windows")]
use std::thread::{self, JoinHandle as ThreadJoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use tokio::sync::mpsc;

const FIPS_PEER_ONLINE_GRACE_SECS: u64 = 45;
const FIPS_NOSTR_DISCOVERY_APP: &str = "fips-overlay-v1";
const FIPS_ENDPOINT_CACHE_VERSION: u8 = 1;
const FIPS_ENDPOINT_CACHE_MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use boringtun::device::{Error as TunError, tun::TunSocket};
#[cfg(target_os = "windows")]
use nostr_vpn_wintun::load_wintun;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use tokio::task::JoinHandle;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use tokio::time::{Duration, sleep};
#[cfg(target_os = "windows")]
use wintun::{Adapter, MAX_RING_CAPACITY, Session};

pub(crate) struct FipsPrivateMeshRuntime {
    endpoint: FipsEndpoint,
    mesh: RwLock<FipsMeshRuntime>,
    presence: RwLock<HashMap<String, FipsPeerPresence>>,
    link_status: RwLock<HashMap<String, FipsEndpointPeer>>,
}

#[derive(Debug, Clone, Default)]
struct FipsPeerPresence {
    last_seen_at: Option<u64>,
    tx_bytes: u64,
    rx_bytes: u64,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FipsEndpointCacheState {
    pub(crate) version: u8,
    pub(crate) network_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) own_pubkey: Option<String>,
    pub(crate) updated_at: u64,
    #[serde(default)]
    pub(crate) peers: Vec<FipsEndpointCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FipsEndpointCacheEntry {
    pub(crate) participant_pubkey: String,
    pub(crate) endpoint_npub: String,
    pub(crate) transport_addr: String,
    pub(crate) transport_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) srtt_ms: Option<u64>,
    #[serde(default)]
    pub(crate) packets_sent: u64,
    #[serde(default)]
    pub(crate) packets_recv: u64,
    #[serde(default)]
    pub(crate) bytes_sent: u64,
    #[serde(default)]
    pub(crate) bytes_recv: u64,
    pub(crate) last_seen_at: u64,
}

pub(crate) fn fips_endpoint_cache_file_path(config_path: &Path) -> PathBuf {
    let parent = config_path
        .parent()
        .map_or_else(|| Path::new(".").to_path_buf(), PathBuf::from);
    parent.join("daemon.fips-cache.json")
}

pub(crate) fn read_fips_endpoint_cache(path: &Path) -> Result<Option<FipsEndpointCacheState>> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read FIPS endpoint cache {}", path.display()))?;
    let parsed = serde_json::from_str::<FipsEndpointCacheState>(&raw)
        .with_context(|| format!("failed to parse FIPS endpoint cache {}", path.display()))?;
    Ok(Some(parsed))
}

pub(crate) fn write_fips_endpoint_cache(path: &Path, state: &FipsEndpointCacheState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(state)?;
    fs::write(path, raw)
        .with_context(|| format!("failed to write FIPS endpoint cache {}", path.display()))?;
    crate::set_private_cache_file_permissions(path)?;
    Ok(())
}

pub(crate) fn write_fips_endpoint_cache_if_changed(
    path: &Path,
    state: &FipsEndpointCacheState,
    last_written_cache: &mut Option<String>,
) -> Result<bool> {
    let raw = serde_json::to_string(state)?;
    if last_written_cache.as_deref() == Some(raw.as_str()) {
        return Ok(false);
    }

    write_fips_endpoint_cache(path, state)?;
    *last_written_cache = Some(raw);
    Ok(true)
}

pub(crate) fn cached_fips_peer_endpoints(
    cache: Option<&FipsEndpointCacheState>,
    app: &AppConfig,
    network_id: &str,
    own_pubkey: Option<&str>,
    now: u64,
) -> Vec<(String, Vec<String>)> {
    let Some(cache) = cache else {
        return Vec::new();
    };
    if !fips_endpoint_cache_matches_runtime(cache, network_id, own_pubkey) {
        return Vec::new();
    }

    let participants = fips_endpoint_cache_participants(app);
    let mut endpoints = HashMap::<String, Vec<String>>::new();
    for entry in &cache.peers {
        let Some(entry) = normalized_fips_endpoint_cache_entry(entry, &participants, own_pubkey)
        else {
            continue;
        };
        if now.saturating_sub(entry.last_seen_at) > FIPS_ENDPOINT_CACHE_MAX_AGE_SECS {
            continue;
        }
        endpoints
            .entry(entry.endpoint_npub)
            .or_default()
            .push(entry.transport_addr);
    }

    let mut endpoints = endpoints.into_iter().collect::<Vec<_>>();
    for (_, addresses) in &mut endpoints {
        addresses.sort();
        addresses.dedup();
    }
    endpoints.sort_by(|left, right| left.0.cmp(&right.0));
    endpoints
}

pub(crate) fn updated_fips_endpoint_cache_state(
    existing: Option<&FipsEndpointCacheState>,
    app: &AppConfig,
    network_id: &str,
    own_pubkey: Option<&str>,
    peer_statuses: &[MeshPeerStatus],
    now: u64,
) -> Option<FipsEndpointCacheState> {
    let participants = fips_endpoint_cache_participants(app);
    let mut entries = HashMap::<FipsEndpointCacheEntryKey, FipsEndpointCacheEntry>::new();
    let previous_updated_at = existing
        .filter(|cache| fips_endpoint_cache_matches_runtime(cache, network_id, own_pubkey))
        .map(|cache| cache.updated_at);
    let mut observed_endpoint = false;

    if let Some(existing) = existing
        && fips_endpoint_cache_matches_runtime(existing, network_id, own_pubkey)
    {
        for entry in &existing.peers {
            let Some(entry) =
                normalized_fips_endpoint_cache_entry(entry, &participants, own_pubkey)
            else {
                continue;
            };
            if now.saturating_sub(entry.last_seen_at) <= FIPS_ENDPOINT_CACHE_MAX_AGE_SECS {
                entries.insert(FipsEndpointCacheEntryKey::from(&entry), entry);
            }
        }
    }

    for status in peer_statuses {
        let Some(entry) =
            fips_endpoint_cache_entry_from_status(status, &participants, own_pubkey, now)
        else {
            continue;
        };
        observed_endpoint = true;
        entries.insert(FipsEndpointCacheEntryKey::from(&entry), entry);
    }

    let mut peers = entries.into_values().collect::<Vec<_>>();
    peers.sort_by(|left, right| {
        left.participant_pubkey
            .cmp(&right.participant_pubkey)
            .then_with(|| left.endpoint_npub.cmp(&right.endpoint_npub))
            .then_with(|| left.transport_type.cmp(&right.transport_type))
            .then_with(|| left.transport_addr.cmp(&right.transport_addr))
    });
    if peers.is_empty() {
        return None;
    }

    Some(FipsEndpointCacheState {
        version: FIPS_ENDPOINT_CACHE_VERSION,
        network_id: network_id.trim().to_string(),
        own_pubkey: own_pubkey.map(str::to_string),
        updated_at: if observed_endpoint {
            now
        } else {
            previous_updated_at.unwrap_or(now)
        },
        peers,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FipsEndpointCacheEntryKey {
    participant_pubkey: String,
    endpoint_npub: String,
    transport_type: String,
    transport_addr: String,
}

impl From<&FipsEndpointCacheEntry> for FipsEndpointCacheEntryKey {
    fn from(entry: &FipsEndpointCacheEntry) -> Self {
        Self {
            participant_pubkey: entry.participant_pubkey.clone(),
            endpoint_npub: entry.endpoint_npub.clone(),
            transport_type: entry.transport_type.clone(),
            transport_addr: entry.transport_addr.clone(),
        }
    }
}

fn fips_endpoint_cache_participants(app: &AppConfig) -> HashSet<String> {
    app.participant_pubkeys_hex().into_iter().collect()
}

fn fips_endpoint_cache_matches_runtime(
    cache: &FipsEndpointCacheState,
    network_id: &str,
    own_pubkey: Option<&str>,
) -> bool {
    cache.version == FIPS_ENDPOINT_CACHE_VERSION
        && cache.network_id == network_id.trim()
        && match (&cache.own_pubkey, own_pubkey) {
            (Some(cached), Some(current)) => cached == current,
            (Some(_), None) => false,
            (None, _) => true,
        }
}

fn normalized_fips_endpoint_cache_entry(
    entry: &FipsEndpointCacheEntry,
    participants: &HashSet<String>,
    own_pubkey: Option<&str>,
) -> Option<FipsEndpointCacheEntry> {
    let participant_pubkey = normalize_nostr_pubkey(&entry.participant_pubkey).ok()?;
    if Some(participant_pubkey.as_str()) == own_pubkey
        || !participants.contains(&participant_pubkey)
    {
        return None;
    }

    let endpoint_npub = normalize_fips_endpoint_npub(&entry.endpoint_npub);
    if endpoint_npub.trim().is_empty() {
        return None;
    }

    let transport_type = entry.transport_type.trim().to_ascii_lowercase();
    if transport_type != "udp" {
        return None;
    }

    let transport_addr = entry.transport_addr.trim().to_string();
    if transport_addr.is_empty() {
        return None;
    }

    Some(FipsEndpointCacheEntry {
        participant_pubkey,
        endpoint_npub,
        transport_addr,
        transport_type,
        srtt_ms: entry.srtt_ms,
        packets_sent: entry.packets_sent,
        packets_recv: entry.packets_recv,
        bytes_sent: entry.bytes_sent,
        bytes_recv: entry.bytes_recv,
        last_seen_at: entry.last_seen_at,
    })
}

fn fips_endpoint_cache_entry_from_status(
    status: &MeshPeerStatus,
    participants: &HashSet<String>,
    own_pubkey: Option<&str>,
    now: u64,
) -> Option<FipsEndpointCacheEntry> {
    if !status.connected {
        return None;
    }

    let participant_pubkey = normalize_nostr_pubkey(&status.pubkey).ok()?;
    if Some(participant_pubkey.as_str()) == own_pubkey
        || !participants.contains(&participant_pubkey)
    {
        return None;
    }

    let endpoint_npub = normalize_fips_endpoint_npub(&status.endpoint_npub);
    if endpoint_npub.trim().is_empty() {
        return None;
    }

    let transport_type = status
        .transport_type
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if transport_type != "udp" {
        return None;
    }

    let transport_addr = status.transport_addr.as_deref()?.trim().to_string();
    if transport_addr.is_empty() {
        return None;
    }

    Some(FipsEndpointCacheEntry {
        participant_pubkey,
        endpoint_npub,
        transport_addr,
        transport_type,
        srtt_ms: status.srtt_ms,
        packets_sent: status.link_packets_sent,
        packets_recv: status.link_packets_recv,
        bytes_sent: status.link_bytes_sent,
        bytes_recv: status.link_bytes_recv,
        last_seen_at: now,
    })
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
        let scope = format!("nostr-vpn:{}", network_id.as_ref().trim());
        let endpoint_peers = fips_endpoint_peers_from_mesh(&peers, Vec::new());
        let config = fips_endpoint_config(&scope, &endpoint_peers, None);
        Self::bind_with_config(identity_nsec, scope, peers, config, Vec::new()).await
    }

    async fn bind_with_config(
        identity_nsec: impl Into<String>,
        scope: impl Into<String>,
        peers: Vec<FipsMeshPeerConfig>,
        config: Config,
        local_allowed_ips: Vec<String>,
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
            mesh: RwLock::new(FipsMeshRuntime::with_local_routes(peers, local_allowed_ips)),
            presence: RwLock::new(HashMap::new()),
            link_status: RwLock::new(HashMap::new()),
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

            if let Some(frame) = decode_fips_control_frame(&message.data)? {
                let source_pubkey = {
                    let mesh = self
                        .mesh
                        .read()
                        .map_err(|_| anyhow!("FIPS mesh route table lock poisoned"))?;
                    control_frame_source_pubkey(&mesh, message.source_npub.as_deref(), &frame)
                };
                let Some(source_pubkey) = source_pubkey else {
                    continue;
                };
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
        let link_status = self.link_status.read().ok();
        let mut statuses = self
            .mesh
            .read()
            .map(|mesh| mesh.peer_statuses())
            .unwrap_or_default();
        for status in &mut statuses {
            let peer_presence = presence
                .as_ref()
                .and_then(|presence| presence.get(&status.pubkey));
            let peer_link = link_status
                .as_ref()
                .and_then(|link_status| link_status.get(&status.pubkey));
            status.last_seen_at = peer_presence.and_then(|value| value.last_seen_at);
            status.tx_bytes = peer_presence.map(|value| value.tx_bytes).unwrap_or(0);
            status.rx_bytes = peer_presence.map(|value| value.rx_bytes).unwrap_or(0);
            let presence_connected = status.last_seen_at.is_some_and(|last_seen_at| {
                now.saturating_sub(last_seen_at) <= FIPS_PEER_ONLINE_GRACE_SECS
            });
            if let Some(peer_link) = peer_link {
                status.endpoint_npub = peer_link.npub.clone();
                status.transport_addr = peer_link.transport_addr.clone();
                status.transport_type = peer_link.transport_type.clone();
                status.srtt_ms = peer_link.srtt_ms;
                status.link_packets_sent = peer_link.packets_sent;
                status.link_packets_recv = peer_link.packets_recv;
                status.link_bytes_sent = peer_link.bytes_sent;
                status.link_bytes_recv = peer_link.bytes_recv;
            }
            status.connected = presence_connected;
            status.error = if status.connected {
                None
            } else {
                peer_presence
                    .and_then(|value| value.error.clone())
                    .or_else(|| Some("fips link pending".to_string()))
            };
        }
        statuses
    }

    pub(crate) async fn refresh_link_statuses(&self) -> Result<()> {
        let endpoint_peers = self
            .endpoint
            .peers()
            .await
            .context("failed to snapshot FIPS endpoint peers")?;
        let mesh = self
            .mesh
            .read()
            .map_err(|_| anyhow!("FIPS mesh route table lock poisoned"))?;
        let mut link_status = HashMap::new();
        for peer in endpoint_peers {
            if let Some(participant) = mesh.participant_for_endpoint_npub(&peer.npub) {
                link_status.insert(participant, peer);
            }
        }
        *self
            .link_status
            .write()
            .map_err(|_| anyhow!("FIPS mesh link status lock poisoned"))? = link_status;
        Ok(())
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

    pub(crate) fn replace_peers(
        &self,
        peers: Vec<FipsMeshPeerConfig>,
        local_allowed_ips: Vec<String>,
    ) -> Result<()> {
        *self
            .mesh
            .write()
            .map_err(|_| anyhow!("FIPS mesh route table lock poisoned"))? =
            FipsMeshRuntime::with_local_routes(peers, local_allowed_ips);
        let configured = self
            .mesh
            .read()
            .map_err(|_| anyhow!("FIPS mesh route table lock poisoned"))?
            .peer_pubkeys();
        self.presence
            .write()
            .map_err(|_| anyhow!("FIPS mesh presence lock poisoned"))?
            .retain(|participant, _| configured.iter().any(|value| value == participant));
        self.link_status
            .write()
            .map_err(|_| anyhow!("FIPS mesh link status lock poisoned"))?
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

fn control_frame_source_pubkey(
    mesh: &FipsMeshRuntime,
    source_npub: Option<&str>,
    frame: &FipsControlFrame,
) -> Option<String> {
    let source_npub = source_npub?;
    mesh.participant_for_endpoint_npub(source_npub).or_else(|| {
        matches!(frame, FipsControlFrame::JoinRequest { .. })
            .then(|| normalize_nostr_pubkey(source_npub).ok())
            .flatten()
    })
}

#[derive(Debug, Clone)]
struct FipsEndpointTransportConfig {
    listen_port: u16,
    advertised_endpoint: String,
    advertise_endpoint: bool,
    stun_servers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FipsEndpointPeerTransportConfig {
    npub: String,
    addresses: Vec<String>,
    via_nostr: bool,
}

fn fips_endpoint_config(
    _scope: &str,
    peers: &[FipsEndpointPeerTransportConfig],
    transport: Option<&FipsEndpointTransportConfig>,
) -> Config {
    let mut config = Config::new();
    config.node.control.enabled = false;
    config.dns.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.node.discovery.nostr.advertise = false;
    config.node.discovery.nostr.policy = NostrDiscoveryPolicy::ConfiguredOnly;
    config.node.discovery.nostr.share_local_candidates = false;
    config.node.discovery.nostr.app = FIPS_NOSTR_DISCOVERY_APP.to_string();
    let bind_addr = transport.map(fips_udp_bind_addr);
    let external_addr = transport.and_then(fips_udp_external_addr);
    let advertise_udp = false;
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
    config.peers = peers
        .iter()
        .filter(|peer| peer.via_nostr || !peer.addresses.is_empty())
        .map(|peer| FipsPeerConfig {
            npub: peer.npub.clone(),
            alias: None,
            addresses: peer
                .addresses
                .iter()
                .map(|address| PeerAddress::new("udp", address.clone()))
                .collect(),
            connect_policy: ConnectPolicy::AutoConnect,
            auto_reconnect: true,
            via_nostr: peer.via_nostr,
        })
        .collect();
    config
}

fn fips_endpoint_peers_from_mesh(
    mesh_peers: &[FipsMeshPeerConfig],
    static_peer_endpoints: Vec<(String, Vec<String>)>,
) -> Vec<FipsEndpointPeerTransportConfig> {
    let mut peers = HashMap::<String, FipsEndpointPeerTransportConfig>::new();
    for peer in mesh_peers {
        let npub = normalize_fips_endpoint_npub(&peer.endpoint_npub);
        peers
            .entry(npub.clone())
            .or_insert_with(|| FipsEndpointPeerTransportConfig {
                npub,
                addresses: Vec::new(),
                via_nostr: false,
            })
            .via_nostr = false;
    }

    for (npub, addresses) in static_peer_endpoints {
        let npub = normalize_fips_endpoint_npub(&npub);
        let peer = peers
            .entry(npub.clone())
            .or_insert_with(|| FipsEndpointPeerTransportConfig {
                npub,
                addresses: Vec::new(),
                via_nostr: false,
            });
        peer.addresses.extend(
            addresses
                .into_iter()
                .map(|address| address.trim().to_string())
                .filter(|address| !address.is_empty()),
        );
    }

    let mut peers = peers.into_values().collect::<Vec<_>>();
    for peer in &mut peers {
        peer.addresses.sort();
        peer.addresses.dedup();
    }
    peers.sort_by(|left, right| left.npub.cmp(&right.npub));
    peers
}

fn normalize_fips_endpoint_npub(value: &str) -> String {
    let trimmed = value.trim();
    normalize_nostr_pubkey(trimmed)
        .ok()
        .and_then(|pubkey| {
            PublicKey::from_hex(&pubkey)
                .ok()
                .and_then(|public_key| public_key.to_bech32().ok())
        })
        .unwrap_or_else(|| trimmed.to_string())
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
    pub(crate) iface: String,
    pub(crate) local_address: String,
    pub(crate) listen_port: u16,
    pub(crate) advertised_endpoint: String,
    pub(crate) advertise_endpoint: bool,
    pub(crate) stun_servers: Vec<String>,
    pub(crate) peers: Vec<FipsMeshPeerConfig>,
    endpoint_peers: Vec<FipsEndpointPeerTransportConfig>,
    pub(crate) route_targets: Vec<String>,
    pub(crate) local_advertised_routes: Vec<String>,
    pub(crate) wireguard_exit: WireGuardExitConfig,
    #[cfg(target_os = "linux")]
    pub(crate) control_plane_bypass_hosts: Vec<Ipv4Addr>,
}

impl FipsPrivateTunnelConfig {
    pub(crate) fn from_app(
        app: &AppConfig,
        network_id: &str,
        iface: impl Into<String>,
        own_pubkey: Option<&str>,
    ) -> Result<Self> {
        Self::from_app_with_cached_peer_endpoints(app, network_id, iface, own_pubkey, Vec::new())
    }

    pub(crate) fn from_app_with_cached_peer_endpoints(
        app: &AppConfig,
        network_id: &str,
        iface: impl Into<String>,
        own_pubkey: Option<&str>,
        cached_peer_endpoints: Vec<(String, Vec<String>)>,
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
        let mut static_peer_endpoints = app.fips_static_peer_endpoints();
        static_peer_endpoints.extend(cached_peer_endpoints);
        let endpoint_peers = fips_endpoint_peers_from_mesh(&peers, static_peer_endpoints);
        route_targets.sort();
        route_targets.dedup();

        Ok(Self {
            identity_nsec: app.nostr.secret_key.clone(),
            network_id: network_id.to_string(),
            iface: iface.into(),
            local_address: own_pubkey
                .and_then(|pubkey| derive_mesh_tunnel_ip(network_id, pubkey))
                .map(|tunnel_ip| local_interface_address_for_tunnel(&tunnel_ip))
                .unwrap_or_else(|| local_interface_address_for_tunnel(&app.node.tunnel_ip)),
            listen_port: app.node.listen_port,
            advertised_endpoint: app.node.endpoint.clone(),
            advertise_endpoint: app.fips_advertise_endpoint,
            stun_servers: app.nat.stun_servers.clone(),
            peers,
            endpoint_peers,
            route_targets,
            local_advertised_routes: app.effective_advertised_routes(),
            wireguard_exit: app.wireguard_exit.clone(),
            #[cfg(target_os = "linux")]
            control_plane_bypass_hosts: crate::control_plane_bypass_ipv4_hosts(app),
        })
    }

    fn local_allowed_ips(&self) -> Vec<String> {
        let mut routes = vec![self.local_address.clone()];
        routes.extend(self.local_advertised_routes.iter().cloned());
        routes.sort();
        routes.dedup();
        routes
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
            advertise_endpoint: config.advertise_endpoint,
            stun_servers: config.stun_servers.clone(),
        };
        let endpoint_config =
            fips_endpoint_config(&scope, &config.endpoint_peers, Some(&transport));
        let local_allowed_ips = config.local_allowed_ips();
        let mesh = Arc::new(
            FipsPrivateMeshRuntime::bind_with_config(
                config.identity_nsec.clone(),
                scope,
                config.peers.clone(),
                endpoint_config,
                local_allowed_ips,
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

    pub(crate) fn requires_endpoint_restart(&self, config: &FipsPrivateTunnelConfig) -> bool {
        self.config.identity_nsec != config.identity_nsec
            || self.config.network_id != config.network_id
            || self.config.listen_port != config.listen_port
            || self.config.advertised_endpoint != config.advertised_endpoint
            || self.config.advertise_endpoint != config.advertise_endpoint
            || self.config.stun_servers != config.stun_servers
            || self.config.endpoint_peers != config.endpoint_peers
    }

    pub(crate) async fn apply_config(&mut self, config: FipsPrivateTunnelConfig) -> Result<()> {
        self.mesh
            .replace_peers(config.peers.clone(), config.local_allowed_ips())?;
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

    pub(crate) async fn refresh_link_statuses(&self) -> Result<()> {
        self.mesh.refresh_link_statuses().await
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
            crate::apply_local_interface_network_with_mtu(
                &self.iface,
                &config.local_address,
                &config.route_targets,
                crate::FIPS_TUNNEL_MTU,
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

        crate::apply_local_interface_network_with_mtu(
            &self.iface,
            &config.local_address,
            &route_targets,
            crate::FIPS_TUNNEL_MTU,
        )
        .with_context(|| format!("failed to configure FIPS tunnel interface {}", self.iface))?;
        if let Err(error) = crate::flush_linux_route_cache() {
            eprintln!("fips: failed to flush linux route cache: {error}");
        }
        self.reconcile_linux_exit_node_forwarding(
            &config.local_address,
            &config.local_advertised_routes,
            &config.wireguard_exit,
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
    fn reconcile_linux_exit_node_forwarding(
        &mut self,
        local_address: &str,
        routes: &[String],
        wireguard_exit: &WireGuardExitConfig,
    ) {
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

        let wireguard_exit_iface = if route_families.ipv4 && wireguard_exit.enabled {
            let Some(source_cidr) = ipv4_tunnel_source_cidr.as_deref() else {
                self.reconcile_linux_exit_node_forwarding_cleanup();
                return;
            };
            match crate::validate_linux_wireguard_exit_config(wireguard_exit) {
                Ok(iface) => {
                    if !crate::linux_wireguard_exit_ipv6_default(wireguard_exit) {
                        route_families.ipv6 = false;
                    }
                    Some((iface, source_cidr.to_string()))
                }
                Err(error) => {
                    eprintln!("fips: WireGuard exit upstream is not ready: {error}");
                    self.reconcile_linux_exit_node_forwarding_cleanup();
                    return;
                }
            }
        } else {
            None
        };

        let ipv4_outbound_iface = if route_families.ipv4 {
            if let Some((iface, _)) = wireguard_exit_iface.as_ref() {
                Some(iface.clone())
            } else {
                match crate::linux_default_route() {
                    Ok(route) => Some(route.dev),
                    Err(error) => {
                        eprintln!("fips: failed to resolve default IPv4 route device: {error}");
                        self.reconcile_linux_exit_node_forwarding_cleanup();
                        return;
                    }
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
            if let Some((_, source_cidr)) = wireguard_exit_iface.as_ref()
                && let Err(error) =
                    self.apply_linux_wireguard_exit_upstream(wireguard_exit, source_cidr)
            {
                eprintln!("fips: failed to refresh WireGuard exit upstream: {error}");
                self.reconcile_linux_exit_node_forwarding_cleanup();
            } else if wireguard_exit_iface.is_none() {
                self.cleanup_linux_wireguard_exit_upstream();
            }
            return;
        }

        self.reconcile_linux_exit_node_forwarding_cleanup();

        if let Some((_, source_cidr)) = wireguard_exit_iface.as_ref()
            && let Err(error) =
                self.apply_linux_wireguard_exit_upstream(wireguard_exit, source_cidr)
        {
            eprintln!("fips: failed to configure WireGuard exit upstream: {error}");
            self.reconcile_linux_exit_node_forwarding_cleanup();
            return;
        }

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
    fn apply_linux_wireguard_exit_upstream(
        &mut self,
        config: &WireGuardExitConfig,
        source_cidr: &str,
    ) -> Result<()> {
        let mut preserve_created_interface = false;
        let mut previous_runtime = None;
        if let Some(runtime) = self.exit_node_runtime.wireguard_exit.as_ref()
            && (runtime.interface != config.interface || runtime.source_cidr != source_cidr)
        {
            self.cleanup_linux_wireguard_exit_upstream();
        } else if let Some(runtime) = self.exit_node_runtime.wireguard_exit.as_ref() {
            preserve_created_interface = runtime.created_interface;
            previous_runtime = Some(runtime.clone());
        }
        let mut runtime = crate::apply_linux_wireguard_exit_upstream(
            config,
            source_cidr,
            previous_runtime.as_ref(),
        )?;
        runtime.created_interface |= preserve_created_interface;
        self.exit_node_runtime.wireguard_exit = Some(runtime);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn cleanup_linux_wireguard_exit_upstream(&mut self) {
        let Some(runtime) = self.exit_node_runtime.wireguard_exit.take() else {
            return;
        };
        crate::cleanup_linux_wireguard_exit_upstream(&runtime);
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

        self.cleanup_linux_wireguard_exit_upstream();
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

#[cfg(target_os = "windows")]
pub(crate) struct FipsPrivateTunnelRuntime {
    iface: String,
    mesh: Arc<FipsPrivateMeshRuntime>,
    config: FipsPrivateTunnelConfig,
    session: Arc<Session>,
    stop: Arc<AtomicBool>,
    tun_read_thread: ThreadJoinHandle<()>,
    mesh_send_task: JoinHandle<()>,
    mesh_recv_task: JoinHandle<()>,
    event_rx: mpsc::Receiver<FipsPrivateMeshEvent>,
    interface_index: u32,
    route_targets: Vec<String>,
}

#[cfg(target_os = "windows")]
impl FipsPrivateTunnelRuntime {
    pub(crate) async fn start(config: FipsPrivateTunnelConfig) -> Result<Self> {
        let scope = format!("nostr-vpn:{}", config.network_id.trim());
        let transport = FipsEndpointTransportConfig {
            listen_port: config.listen_port,
            advertised_endpoint: config.advertised_endpoint.clone(),
            advertise_endpoint: config.advertise_endpoint,
            stun_servers: config.stun_servers.clone(),
        };
        let endpoint_config =
            fips_endpoint_config(&scope, &config.endpoint_peers, Some(&transport));
        let mesh = Arc::new(
            FipsPrivateMeshRuntime::bind_with_config(
                config.identity_nsec.clone(),
                scope,
                config.peers.clone(),
                endpoint_config,
                config.local_allowed_ips(),
            )
            .await?,
        );
        let (session, iface, interface_index) = start_windows_fips_wintun(&config)?;
        let route_targets =
            crate::windows_tunnel::apply_windows_routes(interface_index, &config.route_targets)?;

        let stop = Arc::new(AtomicBool::new(false));
        let (packet_tx, mut packet_rx) = mpsc::channel::<Vec<u8>>(1024);
        let (event_tx, event_rx) = mpsc::channel::<FipsPrivateMeshEvent>(1024);
        let tun_read_thread =
            spawn_windows_fips_tun_read_thread(stop.clone(), session.clone(), packet_tx);
        let mesh_send_task = {
            let mesh = Arc::clone(&mesh);
            tokio::spawn(async move {
                while let Some(packet) = packet_rx.recv().await {
                    let debug = windows_fips_packet_debug_enabled();
                    if debug {
                        eprintln!(
                            "fips: Windows Wintun -> mesh {} bytes {}",
                            packet.len(),
                            describe_ip_packet(&packet)
                        );
                    }
                    match mesh.send_tunnel_packet(&packet).await {
                        Ok(true) => {}
                        Ok(false) => {
                            if debug {
                                eprintln!("fips: Windows mesh route miss");
                            }
                        }
                        Err(error) => {
                            eprintln!("fips: failed to send Windows tunnel packet: {error}");
                        }
                    }
                }
            })
        };
        let mesh_recv_task =
            spawn_windows_fips_mesh_recv_task(Arc::clone(&mesh), session.clone(), event_tx);

        Ok(Self {
            iface,
            mesh,
            config,
            session,
            stop,
            tun_read_thread,
            mesh_send_task,
            mesh_recv_task,
            event_rx,
            interface_index,
            route_targets,
        })
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

    pub(crate) fn requires_endpoint_restart(&self, config: &FipsPrivateTunnelConfig) -> bool {
        self.config.identity_nsec != config.identity_nsec
            || self.config.network_id != config.network_id
            || self.config.iface != config.iface
            || self.config.local_address != config.local_address
            || self.config.listen_port != config.listen_port
            || self.config.advertised_endpoint != config.advertised_endpoint
            || self.config.advertise_endpoint != config.advertise_endpoint
            || self.config.stun_servers != config.stun_servers
            || self.config.endpoint_peers != config.endpoint_peers
    }

    pub(crate) async fn apply_config(&mut self, config: FipsPrivateTunnelConfig) -> Result<()> {
        self.mesh
            .replace_peers(config.peers.clone(), config.local_allowed_ips())?;
        if self.config.route_targets != config.route_targets {
            crate::windows_tunnel::remove_windows_routes(self.interface_index, &self.route_targets)
                .context("failed to remove stale Windows FIPS routes")?;
            self.route_targets = crate::windows_tunnel::apply_windows_routes(
                self.interface_index,
                &config.route_targets,
            )
            .context("failed to apply Windows FIPS routes")?;
        }
        self.config = config;
        Ok(())
    }

    pub(crate) async fn refresh_peer_dependent_routes(&mut self) -> Result<()> {
        Ok(())
    }

    pub(crate) async fn ping_peers(&self, network_id: &str, now: u64) -> Result<usize> {
        self.mesh.ping_peers(network_id, now).await
    }

    pub(crate) async fn refresh_link_statuses(&self) -> Result<()> {
        self.mesh.refresh_link_statuses().await
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
        let runtime = self;
        runtime.stop.store(true, Ordering::Relaxed);
        let _ = runtime.session.shutdown();
        if let Err(error) = crate::windows_tunnel::remove_windows_routes(
            runtime.interface_index,
            &runtime.route_targets,
        ) {
            eprintln!("fips: failed to remove Windows FIPS routes: {error}");
        }
        let _ = runtime.tun_read_thread.join();
        runtime.mesh_send_task.abort();
        runtime.mesh_recv_task.abort();
        let _ = runtime.mesh_send_task.await;
        let _ = runtime.mesh_recv_task.await;
        if let Ok(mesh) = Arc::try_unwrap(runtime.mesh) {
            mesh.shutdown()
                .await
                .context("failed to stop FIPS endpoint")?;
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn start_windows_fips_wintun(
    config: &FipsPrivateTunnelConfig,
) -> Result<(Arc<Session>, String, u32)> {
    let wintun = load_wintun()?;
    let adapter = Adapter::open(&wintun, &config.iface)
        .or_else(|_| Adapter::create(&wintun, &config.iface, "NostrVPN", None))
        .with_context(|| format!("failed to open or create wintun adapter {}", config.iface))?;
    let mtu = crate::platform_routing::FIPS_TUNNEL_MTU
        .parse::<usize>()
        .context("invalid FIPS tunnel MTU")?;
    adapter
        .set_mtu(mtu)
        .with_context(|| format!("failed to set MTU on wintun adapter {}", config.iface))?;
    let parsed_address = crate::windows_tunnel::windows_interface_address(&config.local_address)?;
    adapter
        .set_network_addresses_tuple(
            parsed_address.address.into(),
            parsed_address.mask.into(),
            None,
        )
        .with_context(|| format!("failed to set address on wintun adapter {}", config.iface))?;
    let interface_index = adapter
        .get_adapter_index()
        .with_context(|| format!("failed to resolve interface index for {}", config.iface))?;
    let session = Arc::new(
        adapter
            .start_session(MAX_RING_CAPACITY)
            .with_context(|| format!("failed to start wintun session for {}", config.iface))?,
    );
    Ok((session, config.iface.clone(), interface_index))
}

#[cfg(target_os = "windows")]
fn spawn_windows_fips_tun_read_thread(
    stop: Arc<AtomicBool>,
    session: Arc<Session>,
    packet_tx: mpsc::Sender<Vec<u8>>,
) -> ThreadJoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            let packet = match session.receive_blocking() {
                Ok(packet) => packet,
                Err(error) => {
                    if !stop.load(Ordering::Relaxed) {
                        eprintln!("fips: Windows Wintun receive failed: {error}");
                    }
                    break;
                }
            };
            let payload = packet.bytes().to_vec();
            drop(packet);
            if windows_fips_packet_debug_enabled() {
                eprintln!(
                    "fips: Windows Wintun read {} bytes {}",
                    payload.len(),
                    describe_ip_packet(&payload)
                );
            }
            if packet_tx.blocking_send(payload).is_err() {
                break;
            }
        }
    })
}

#[cfg(target_os = "windows")]
fn spawn_windows_fips_mesh_recv_task(
    mesh: Arc<FipsPrivateMeshRuntime>,
    session: Arc<Session>,
    event_tx: mpsc::Sender<FipsPrivateMeshEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match mesh.recv_mesh_event().await {
                Ok(Some(FipsPrivateMeshEvent::Packet(packet))) => {
                    let bytes = packet.bytes.clone();
                    if windows_fips_packet_debug_enabled() {
                        eprintln!(
                            "fips: Windows mesh -> Wintun {} bytes {}",
                            bytes.len(),
                            describe_ip_packet(&bytes)
                        );
                    }
                    if let Err(error) =
                        crate::windows_tunnel::write_tunnel_packets(&session, &[bytes])
                    {
                        eprintln!("fips: failed to write Windows tunnel packet: {error}");
                    }
                    if event_tx
                        .send(FipsPrivateMeshEvent::Packet(packet))
                        .await
                        .is_err()
                    {
                        break;
                    }
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

#[cfg(target_os = "windows")]
fn windows_fips_packet_debug_enabled() -> bool {
    std::env::var_os("NVPN_FIPS_PACKET_DEBUG").is_some()
}

#[cfg(target_os = "windows")]
fn describe_ip_packet(packet: &[u8]) -> String {
    match packet.first().map(|byte| byte >> 4) {
        Some(4) if packet.len() >= 20 => format!(
            "{} -> {}",
            std::net::Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]),
            std::net::Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19])
        ),
        Some(6) if packet.len() >= 40 => "IPv6".to_string(),
        Some(version) => format!("IPv{version} malformed"),
        None => "empty packet".to_string(),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) struct FipsPrivateTunnelRuntime;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
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

    pub(crate) fn requires_endpoint_restart(&self, _config: &FipsPrivateTunnelConfig) -> bool {
        false
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

    pub(crate) async fn refresh_link_statuses(&self) -> Result<()> {
        Ok(())
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
        FIPS_ENDPOINT_CACHE_MAX_AGE_SECS, FIPS_NOSTR_DISCOVERY_APP, FipsEndpointTransportConfig,
        FipsPrivateMeshRuntime, FipsPrivateTunnelConfig, cached_fips_peer_endpoints,
        control_frame_source_pubkey, fips_endpoint_config, fips_endpoint_peers_from_mesh,
        updated_fips_endpoint_cache_state,
    };
    use fips_endpoint::{
        Config, ConnectPolicy, PeerConfig as FipsPeerConfig, TransportInstances, UdpConfig,
    };
    use nostr_sdk::prelude::{Keys, ToBech32};
    use nostr_vpn_core::config::{AppConfig, derive_mesh_tunnel_ip};
    use nostr_vpn_core::data_plane::MeshPeerStatus;
    use nostr_vpn_core::fips_control::FipsControlFrame;
    use nostr_vpn_core::fips_control::NetworkRoster;
    use nostr_vpn_core::fips_mesh::{FipsMeshPeerConfig, FipsMeshRuntime};
    use nostr_vpn_core::join_requests::MeshJoinRequest;
    use std::collections::HashMap;
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

    fn mesh_peer_status(
        pubkey: impl AsRef<str>,
        endpoint_npub: impl AsRef<str>,
        transport_addr: Option<&str>,
        transport_type: Option<&str>,
        connected: bool,
        last_seen_at: Option<u64>,
    ) -> MeshPeerStatus {
        MeshPeerStatus {
            pubkey: pubkey.as_ref().to_string(),
            connected,
            endpoint_npub: endpoint_npub.as_ref().to_string(),
            transport_addr: transport_addr.map(str::to_string),
            transport_type: transport_type.map(str::to_string),
            srtt_ms: Some(18),
            link_packets_sent: 7,
            link_packets_recv: 8,
            link_bytes_sent: 900,
            link_bytes_recv: 1200,
            last_seen_at,
            tx_bytes: 0,
            rx_bytes: 0,
            error: None,
        }
    }

    #[test]
    fn control_frames_from_rostered_endpoint_resolve_to_participant() {
        let keys = Keys::generate();
        let participant_pubkey = keys.public_key().to_hex();
        let endpoint_npub = keys.public_key().to_bech32().expect("npub");
        let mesh = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: participant_pubkey.clone(),
            endpoint_npub: endpoint_npub.clone(),
            allowed_ips: vec!["10.44.1.2/32".to_string()],
        }]);
        let frame = FipsControlFrame::Ping {
            network_id: "network".to_string(),
            sent_at: 42,
        };

        assert_eq!(
            control_frame_source_pubkey(&mesh, Some(&endpoint_npub), &frame),
            Some(participant_pubkey)
        );
    }

    #[test]
    fn control_frames_from_unknown_endpoints_are_limited_to_join_requests() {
        let keys = Keys::generate();
        let unknown_pubkey = keys.public_key().to_hex();
        let unknown_npub = keys.public_key().to_bech32().expect("npub");
        let mesh = FipsMeshRuntime::new(Vec::new());
        let ping = FipsControlFrame::Ping {
            network_id: "network".to_string(),
            sent_at: 42,
        };
        let roster = FipsControlFrame::Roster {
            network_id: "network".to_string(),
            roster: NetworkRoster {
                network_name: "network".to_string(),
                participants: Vec::new(),
                admins: Vec::new(),
                aliases: HashMap::new(),
                signed_at: 42,
            },
        };
        let join_request = FipsControlFrame::JoinRequest {
            requested_at: 42,
            request: MeshJoinRequest {
                network_id: "network".to_string(),
                requester_node_name: "new-device".to_string(),
            },
        };

        assert!(control_frame_source_pubkey(&mesh, Some(&unknown_npub), &ping).is_none());
        assert!(control_frame_source_pubkey(&mesh, Some(&unknown_npub), &roster).is_none());
        assert_eq!(
            control_frame_source_pubkey(&mesh, Some(&unknown_npub), &join_request),
            Some(unknown_pubkey)
        );
    }

    #[test]
    fn fips_endpoint_cache_keeps_current_udp_network_members() {
        let own_keys = Keys::generate();
        let peer_keys = Keys::generate();
        let stranger_keys = Keys::generate();
        let own_pubkey = own_keys.public_key().to_hex();
        let peer_pubkey = peer_keys.public_key().to_hex();
        let peer_npub = peer_keys.public_key().to_bech32().expect("peer npub");
        let stranger_pubkey = stranger_keys.public_key().to_hex();
        let stranger_npub = stranger_keys
            .public_key()
            .to_bech32()
            .expect("stranger npub");
        let mut app = AppConfig::default();
        app.networks[0].participants = vec![own_pubkey.clone(), peer_pubkey.clone()];
        let now = 10_000;

        let statuses = vec![
            mesh_peer_status(
                &peer_pubkey,
                &peer_npub,
                Some(" 198.51.100.9:51820 "),
                Some("UDP"),
                true,
                Some(now - 5),
            ),
            mesh_peer_status(
                &stranger_pubkey,
                &stranger_npub,
                Some("198.51.100.10:51820"),
                Some("udp"),
                true,
                Some(now),
            ),
            mesh_peer_status(
                &peer_pubkey,
                &peer_npub,
                Some("198.51.100.11:51820"),
                Some("tcp"),
                true,
                Some(now),
            ),
            mesh_peer_status(
                &peer_pubkey,
                &peer_npub,
                Some("198.51.100.12:51820"),
                Some("udp"),
                false,
                Some(now),
            ),
            mesh_peer_status(
                &own_pubkey,
                &peer_npub,
                Some("198.51.100.13:51820"),
                Some("udp"),
                true,
                Some(now),
            ),
        ];

        let cache = updated_fips_endpoint_cache_state(
            None,
            &app,
            "test-network",
            Some(&own_pubkey),
            &statuses,
            now,
        )
        .expect("cache should include observed peer endpoint");

        assert_eq!(cache.peers.len(), 1);
        assert_eq!(cache.peers[0].participant_pubkey, peer_pubkey);
        assert_eq!(cache.peers[0].endpoint_npub.as_str(), peer_npub);
        assert_eq!(cache.peers[0].transport_addr, "198.51.100.9:51820");
        assert_eq!(cache.peers[0].transport_type, "udp");
        assert_eq!(cache.peers[0].last_seen_at, now);
        assert_eq!(cache.peers[0].packets_sent, 7);

        assert_eq!(
            cached_fips_peer_endpoints(Some(&cache), &app, "test-network", Some(&own_pubkey), now),
            vec![(peer_npub.clone(), vec!["198.51.100.9:51820".to_string()])]
        );
        assert!(
            cached_fips_peer_endpoints(Some(&cache), &app, "other-network", Some(&own_pubkey), now)
                .is_empty()
        );
        assert!(
            cached_fips_peer_endpoints(
                Some(&cache),
                &app,
                "test-network",
                Some(&own_pubkey),
                now + FIPS_ENDPOINT_CACHE_MAX_AGE_SECS + 1
            )
            .is_empty()
        );
    }

    #[test]
    fn fips_tunnel_config_adds_cached_peer_endpoints_to_mesh_peers() {
        let own_keys = Keys::generate();
        let peer_keys = Keys::generate();
        let own_pubkey = own_keys.public_key().to_hex();
        let peer_pubkey = peer_keys.public_key().to_hex();
        let peer_npub = peer_keys.public_key().to_bech32().expect("peer npub");
        let mut app = AppConfig::default();
        app.networks[0].participants = vec![own_pubkey.clone(), peer_pubkey];

        let config = FipsPrivateTunnelConfig::from_app_with_cached_peer_endpoints(
            &app,
            "test-network",
            "nvpn0",
            Some(&own_pubkey),
            vec![(
                peer_npub.clone(),
                vec![
                    "198.51.100.9:51820".to_string(),
                    "198.51.100.9:51820".to_string(),
                ],
            )],
        )
        .expect("FIPS tunnel config");

        let peer = config
            .endpoint_peers
            .iter()
            .find(|peer| peer.npub == peer_npub)
            .expect("cached endpoint peer");
        assert_eq!(peer.addresses, vec!["198.51.100.9:51820".to_string()]);
        assert!(!peer.via_nostr);
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

        let config =
            FipsPrivateTunnelConfig::from_app(&app, network_id, "utun-test", Some(&alice_pubkey))
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

    fn direct_udp_endpoint_config(
        local_port: u16,
        peer_npub: &str,
        peer_port: u16,
        auto_connect: bool,
    ) -> Config {
        let mut config = Config::new();
        config.transports.udp = TransportInstances::Single(UdpConfig {
            bind_addr: Some(format!("127.0.0.1:{local_port}")),
            accept_connections: Some(true),
            ..UdpConfig::default()
        });
        let mut peer = FipsPeerConfig::new(peer_npub, "udp", format!("127.0.0.1:{peer_port}"));
        if !auto_connect {
            peer.connect_policy = ConnectPolicy::Manual;
        }
        config.peers.push(peer);
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

    async fn wait_for_fips_peer(runtime: &FipsPrivateMeshRuntime, peer_npub: &str) {
        let mut last_snapshot = Vec::new();
        let mut last_error = None;
        for _ in 0..50 {
            match runtime.endpoint.peers().await {
                Ok(peers) => {
                    if peers.iter().any(|peer| {
                        peer.npub == peer_npub && peer.transport_addr.as_deref().is_some()
                    }) {
                        return;
                    }
                    last_snapshot = peers;
                }
                Err(error) => last_error = Some(error),
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!(
            "FIPS peer {peer_npub} did not establish; last snapshot: {last_snapshot:?}; last error: {}",
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "none".to_string())
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
            direct_udp_endpoint_config(alice_port, &bob_npub, bob_port, true),
            vec![format!("{alice_ip}/32")],
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
            direct_udp_endpoint_config(bob_port, &alice_npub, alice_port, false),
            vec![format!("{bob_ip}/32")],
        )
        .await
        .expect("bob endpoint should bind");

        wait_for_fips_peer(&alice_runtime, &bob_npub).await;
        wait_for_fips_peer(&bob_runtime, &alice_npub).await;

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
    fn endpoint_config_disables_nostr_discovery_without_direct_peers() {
        let keys = Keys::generate();
        let participant_pubkey = keys.public_key().to_hex();
        let peer = FipsMeshPeerConfig::from_participant_pubkey(
            &participant_pubkey,
            vec!["10.44.1.2/32".to_string()],
        )
        .expect("peer config");
        let endpoint_peers = fips_endpoint_peers_from_mesh(&[peer], Vec::new());
        let config = fips_endpoint_config("nostr-vpn:test", &endpoint_peers, None);

        assert!(!config.node.control.enabled);
        assert!(!config.dns.enabled);
        assert!(!config.node.discovery.nostr.enabled);
        assert!(!config.node.discovery.nostr.advertise);
        assert_eq!(
            config.node.discovery.nostr.policy,
            fips_endpoint::NostrDiscoveryPolicy::ConfiguredOnly
        );
        assert!(!config.node.discovery.nostr.share_local_candidates);
        assert_eq!(config.node.discovery.nostr.app, FIPS_NOSTR_DISCOVERY_APP);
        let udp = match config.transports.udp {
            fips_endpoint::TransportInstances::Single(udp) => udp,
            _ => panic!("expected one UDP transport"),
        };
        assert!(udp.outbound_only());
        assert!(!udp.advertise_on_nostr());
        assert!(!udp.accept_connections());
        assert!(config.peers.is_empty());
    }

    #[test]
    fn endpoint_config_listens_on_app_owned_direct_endpoint_without_nostr_advertise() {
        let keys = Keys::generate();
        let participant_pubkey = keys.public_key().to_hex();
        let peer = FipsMeshPeerConfig::from_participant_pubkey(
            &participant_pubkey,
            vec!["10.44.1.2/32".to_string()],
        )
        .expect("peer config");
        let transport = FipsEndpointTransportConfig {
            listen_port: 51820,
            advertised_endpoint: "192.168.50.20:51820".to_string(),
            advertise_endpoint: true,
            stun_servers: vec!["stun:stun.example.org:3478".to_string()],
        };

        let endpoint_peers = fips_endpoint_peers_from_mesh(&[peer], Vec::new());
        let config = fips_endpoint_config("nostr-vpn:test", &endpoint_peers, Some(&transport));

        assert!(!config.node.discovery.nostr.enabled);
        assert!(!config.node.discovery.nostr.advertise);
        assert_eq!(
            config.node.discovery.nostr.policy,
            fips_endpoint::NostrDiscoveryPolicy::ConfiguredOnly
        );
        assert!(!config.node.discovery.nostr.share_local_candidates);
        assert_eq!(config.node.discovery.nostr.app, FIPS_NOSTR_DISCOVERY_APP);
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
        assert!(!udp.advertise_on_nostr());
        assert!(udp.accept_connections());
        assert_eq!(udp.external_addr.as_deref(), Some("192.168.50.20:51820"));
        assert!(config.peers.is_empty());
    }

    #[test]
    fn endpoint_config_keeps_static_transit_peers_outside_mesh_routes() {
        let bob_keys = Keys::generate();
        let charlie_keys = Keys::generate();
        let bob_pubkey = bob_keys.public_key().to_hex();
        let charlie_npub = charlie_keys.public_key().to_bech32().expect("npub");
        let mesh_peer =
            FipsMeshPeerConfig::from_participant_pubkey(&bob_pubkey, vec!["10.44.1.2/32".into()])
                .expect("mesh peer");
        let endpoint_peers = fips_endpoint_peers_from_mesh(
            std::slice::from_ref(&mesh_peer),
            vec![(charlie_npub.clone(), vec!["10.203.0.12:51820".to_string()])],
        );
        let transport = FipsEndpointTransportConfig {
            listen_port: 51820,
            advertised_endpoint: "10.203.0.10:51820".to_string(),
            advertise_endpoint: false,
            stun_servers: Vec::new(),
        };

        let config = fips_endpoint_config("nostr-vpn:test", &endpoint_peers, Some(&transport));

        assert!(!config.node.discovery.nostr.enabled);
        assert_eq!(endpoint_peers.len(), 2);
        assert_eq!(config.peers.len(), 1);
        assert_eq!(config.peers[0].npub, charlie_npub);
        assert!(!config.peers[0].via_nostr);
        assert_eq!(config.peers[0].addresses.len(), 1);
        assert_eq!(config.peers[0].addresses[0].transport, "udp");
        assert_eq!(config.peers[0].addresses[0].addr, "10.203.0.12:51820");
    }
}
