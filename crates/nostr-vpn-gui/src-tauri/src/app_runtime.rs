use super::*;
#[cfg(target_os = "ios")]
use std::io::Write as _;

#[derive(Debug, Clone, Default)]
pub(crate) struct RelayStatus {
    pub(crate) state: String,
    pub(crate) status_text: String,
}

pub(crate) use nostr_vpn_app_core::{
    DaemonRuntimeState, InboundJoinRequestView, LanPeerView, NetworkView, OutboundJoinRequestView,
    ParticipantView, RelaySummary, RelayView, RuntimePlatform, SettingsPatch, TrayExitNodeEntry,
    TrayMenuItemSpec, TrayNetworkGroup, TrayRuntimeState, UiState, current_runtime_capabilities,
    current_runtime_platform,
};

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistConfigOutcome {
    SavedLocally,
    ReloadedRunningDaemon,
}

impl PersistConfigOutcome {
    pub(crate) fn needs_explicit_daemon_reload(self) -> bool {
        matches!(self, Self::SavedLocally)
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct CliStatusResponse {
    pub(crate) daemon: CliDaemonStatus,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct CliServiceStatusResponse {
    pub(crate) supported: bool,
    pub(crate) installed: bool,
    #[serde(default)]
    pub(crate) disabled: bool,
    pub(crate) loaded: bool,
    pub(crate) running: bool,
    pub(crate) pid: Option<u32>,
    pub(crate) label: String,
    pub(crate) plist_path: String,
    #[serde(default)]
    pub(crate) binary_path: String,
    #[serde(default)]
    pub(crate) binary_version: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct CliDaemonStatus {
    pub(crate) running: bool,
    pub(crate) state: Option<DaemonRuntimeState>,
}

pub(crate) fn within_peer_online_grace(
    last_handshake_at: Option<SystemTime>,
    now: SystemTime,
) -> bool {
    let Some(last_handshake_at) = last_handshake_at else {
        return false;
    };
    now.duration_since(last_handshake_at)
        .map(|elapsed| elapsed.as_secs() <= PEER_ONLINE_GRACE_SECS)
        .unwrap_or(false)
}

pub(crate) fn within_peer_presence_grace(
    last_signal_seen_at: Option<SystemTime>,
    now: SystemTime,
) -> bool {
    let Some(last_signal_seen_at) = last_signal_seen_at else {
        return false;
    };
    now.duration_since(last_signal_seen_at)
        .map(|elapsed| elapsed.as_secs() <= PEER_PRESENCE_GRACE_SECS)
        .unwrap_or(false)
}

pub(crate) fn peer_offers_exit_node(routes: &[String]) -> bool {
    routes
        .iter()
        .any(|route| route == "0.0.0.0/0" || route == "::/0")
}

impl Drop for NvpnBackend {
    fn drop(&mut self) {
        #[cfg(target_os = "android")]
        let _ = self.android_session.stop();
        self.stop_lan_pairing();
    }
}

pub(crate) async fn run_lan_pairing_loop(
    tx: mpsc::Sender<LanPairingSignal>,
    stop_flag: Arc<AtomicBool>,
    own_npub: String,
    node_name: String,
    endpoint: String,
    invite: String,
) {
    let started_at = Instant::now();
    let multicast = std::net::Ipv4Addr::new(
        LAN_PAIRING_ADDR[0],
        LAN_PAIRING_ADDR[1],
        LAN_PAIRING_ADDR[2],
        LAN_PAIRING_ADDR[3],
    );
    let target = std::net::SocketAddr::from((LAN_PAIRING_ADDR, LAN_PAIRING_PORT));

    let std_socket =
        match std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, LAN_PAIRING_PORT)) {
            Ok(socket) => socket,
            Err(_) => return,
        };

    if std_socket
        .join_multicast_v4(&multicast, &std::net::Ipv4Addr::UNSPECIFIED)
        .is_err()
    {
        return;
    }

    if std_socket.set_nonblocking(true).is_err() {
        return;
    }

    let socket = match tokio::net::UdpSocket::from_std(std_socket) {
        Ok(socket) => socket,
        Err(_) => return,
    };

    let mut announce_interval = tokio::time::interval(Duration::from_secs(3));
    let mut idle_interval = tokio::time::interval(Duration::from_millis(250));
    let mut buffer = [0_u8; LAN_PAIRING_BUFFER_BYTES];

    loop {
        if stop_flag.load(Ordering::Relaxed)
            || started_at.elapsed() >= Duration::from_secs(LAN_PAIRING_DURATION_SECS)
        {
            return;
        }

        tokio::select! {
            _ = announce_interval.tick() => {
                let message = LanAnnouncement {
                    v: LAN_PAIRING_ANNOUNCEMENT_VERSION,
                    npub: own_npub.clone(),
                    node_name: node_name.clone(),
                    endpoint: endpoint.clone(),
                    invite: invite.clone(),
                    timestamp: unix_timestamp(),
                };

                if let Ok(encoded) = serde_json::to_vec(&message) {
                    let _ = socket.send_to(&encoded, target).await;
                }
            }
            recv = socket.recv_from(&mut buffer) => {
                if let Ok((len, _)) = recv
                    && let Some(signal) = decode_lan_pairing_announcement(&buffer[..len], &own_npub)
                {
                    let _ = tx.send(signal);
                }
            }
            _ = idle_interval.tick() => {}
        }
    }
}

pub(crate) struct AppState {
    pub(crate) backend: Arc<Mutex<NvpnBackend>>,
    pub(crate) last_tray_runtime_state: Arc<Mutex<TrayRuntimeState>>,
}

#[cfg(test)]
pub(crate) fn tauri_protocol_request_path(uri: &tauri::http::Uri, origin: &str) -> String {
    let request_uri = uri.to_string();
    let request_path = request_uri
        .split(&['?', '#'][..])
        .next()
        .unwrap_or_default()
        .strip_prefix(origin)
        .unwrap_or_default()
        .trim_start_matches('/');

    if request_path.is_empty() {
        "index.html".to_string()
    } else {
        request_path.to_string()
    }
}

#[cfg(target_os = "ios")]
pub(crate) fn reset_ios_probe() {
    let _ = std::fs::write(std::env::temp_dir().join("nvpn-ios-probe.log"), b"");
}

#[cfg(target_os = "ios")]
pub(crate) fn write_ios_probe(message: impl AsRef<str>) {
    let log_path = std::env::temp_dir().join("nvpn-ios-probe.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = writeln!(file, "{}", message.as_ref());
    }
}

#[cfg(target_os = "ios")]
pub(crate) fn ios_force_connect_requested() -> bool {
    env_flag_is_truthy(NVPN_IOS_FORCE_CONNECT_ENV)
}
