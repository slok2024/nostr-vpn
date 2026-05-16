use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use tower_http::services::{ServeDir, ServeFile};

mod invite;
mod network_views;
mod nvpn_cli;
mod ui_models;
mod ui_types;

pub(crate) use crate::ui_models::{
    current_unix_timestamp, is_already_running_message, is_not_running_message,
    nvpn_gui_iface_override, to_npub,
};
pub(crate) use crate::ui_types::CliStatusResponse;

use crate::invite::{
    apply_network_invite_to_active_network, parse_network_invite, preferred_join_request_recipient,
};
use crate::nvpn_cli::{
    connect_vpn_inner, default_config_path, disconnect_vpn_inner, discover_static_dir,
    ensure_config_exists, fetch_cli_status, load_config, resolve_nvpn_cli_path,
};
use crate::ui_models::{
    bad_request, build_ui_state, finalize_config_change, internal_error,
    local_join_request_listener_enabled, parse_advertised_routes_input, parse_exit_node_input,
    set_action_status, update_config_and_reload as update_config_and_reload_impl,
};
use crate::ui_types::{
    AliasRequest, InviteRequest, JoinRequestAction, NameRequest, NetworkEnabledRequest,
    NetworkIdRequest, NetworkMeshRequest, NetworkNameRequest, NetworkPeerRequest,
    ParticipantRequest, SettingsPatch, UiState,
};
use nostr_vpn_core::config::{AppConfig, PendingOutboundJoinRequest, normalize_nostr_pubkey};

const NVPN_BIN_ENV: &str = "NVPN_CLI_PATH";
const NETWORK_INVITE_PREFIX: &str = "nvpn://invite/";
const NETWORK_INVITE_VERSION: u8 = 3;
const DEFAULT_STATIC_DIR: &str = "/usr/share/nostr-vpn/web";

#[derive(Debug, Parser)]
#[command(name = "nvpn-web")]
#[command(about = "HTTP API for the nostr-vpn web UI")]
struct Args {
    #[arg(long, default_value = "0.0.0.0:8081")]
    listen: SocketAddr,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    nvpn: Option<PathBuf>,
    #[arg(long)]
    static_dir: Option<PathBuf>,
}

#[derive(Clone)]
struct ServerState {
    config_path: PathBuf,
    nvpn_bin: PathBuf,
    action_status: Arc<Mutex<String>>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiError>;

fn update_config_and_reload(
    state: &ServerState,
    update: impl FnOnce(&mut AppConfig) -> Result<String>,
) -> ApiResult<Json<UiState>> {
    Ok(Json(
        update_config_and_reload_impl(state, update).map_err(internal_error)?,
    ))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "nostr_vpn_web=info".into()),
        )
        .init();

    let Args {
        listen,
        config,
        nvpn,
        static_dir,
    } = Args::parse();
    let config_path = config.unwrap_or_else(default_config_path);
    ensure_config_exists(&config_path)?;
    let nvpn_bin = resolve_nvpn_cli_path(nvpn)?;
    let static_dir = static_dir.or_else(discover_static_dir);

    let state = ServerState {
        config_path,
        nvpn_bin,
        action_status: Arc::new(Mutex::new(String::new())),
    };

    let mut app = Router::new()
        .route("/api/health", get(health))
        .route("/api/tick", post(tick))
        .route("/api/connect_vpn", post(connect_vpn))
        .route("/api/disconnect_vpn", post(disconnect_vpn))
        .route("/api/add_network", post(add_network))
        .route("/api/rename_network", post(rename_network))
        .route("/api/set_network_mesh_id", post(set_network_mesh_id))
        .route("/api/remove_network", post(remove_network))
        .route("/api/set_network_enabled", post(set_network_enabled))
        .route(
            "/api/set_network_join_requests_enabled",
            post(set_network_join_requests_enabled),
        )
        .route("/api/request_network_join", post(request_network_join))
        .route("/api/add_participant", post(add_participant))
        .route("/api/add_admin", post(add_admin))
        .route("/api/import_network_invite", post(import_network_invite))
        .route("/api/start_invite_broadcast", post(start_invite_broadcast))
        .route("/api/stop_invite_broadcast", post(stop_invite_broadcast))
        .route("/api/start_nearby_discovery", post(start_nearby_discovery))
        .route("/api/stop_nearby_discovery", post(stop_nearby_discovery))
        .route("/api/remove_participant", post(remove_participant))
        .route("/api/remove_admin", post(remove_admin))
        .route("/api/accept_join_request", post(accept_join_request))
        .route("/api/reject_join_request", post(reject_join_request))
        .route("/api/set_participant_alias", post(set_participant_alias))
        .route("/api/update_settings", post(update_settings))
        .with_state(state.clone());

    if let Some(static_dir) = static_dir {
        let index_path = static_dir.join("index.html");
        if !index_path.exists() {
            return Err(anyhow!(
                "static web UI directory is missing {}",
                index_path.display()
            ));
        }
        tracing::info!("serving static web UI from {}", static_dir.display());
        app = app.fallback_service(
            ServeDir::new(static_dir).not_found_service(ServeFile::new(index_path)),
        );
    } else {
        tracing::info!("static web UI disabled");
    }

    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!("nostr-vpn web api listening on {}", listen);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn tick(State(state): State<ServerState>) -> ApiResult<Json<UiState>> {
    Ok(Json(build_ui_state(&state).map_err(internal_error)?))
}

async fn connect_vpn(State(state): State<ServerState>) -> ApiResult<Json<UiState>> {
    connect_vpn_inner(&state).map_err(bad_request)?;
    set_action_status(&state, "Daemon running");
    Ok(Json(build_ui_state(&state).map_err(internal_error)?))
}

async fn disconnect_vpn(State(state): State<ServerState>) -> ApiResult<Json<UiState>> {
    disconnect_vpn_inner(&state).map_err(bad_request)?;
    set_action_status(&state, "Paused");
    Ok(Json(build_ui_state(&state).map_err(internal_error)?))
}

async fn add_network(
    State(state): State<ServerState>,
    Json(request): Json<NameRequest>,
) -> ApiResult<Json<UiState>> {
    update_config_and_reload(&state, |config| {
        config.add_network(&request.name);
        Ok("Network saved.".to_string())
    })
}

async fn rename_network(
    State(state): State<ServerState>,
    Json(request): Json<NetworkNameRequest>,
) -> ApiResult<Json<UiState>> {
    update_config_and_reload(&state, |config| {
        config.rename_network(&request.network_id, &request.name)?;
        Ok("Network renamed.".to_string())
    })
}

async fn set_network_mesh_id(
    State(state): State<ServerState>,
    Json(request): Json<NetworkMeshRequest>,
) -> ApiResult<Json<UiState>> {
    update_config_and_reload(&state, |config| {
        config.set_network_mesh_id(&request.network_id, &request.mesh_id)?;
        Ok("Mesh ID updated.".to_string())
    })
}

async fn remove_network(
    State(state): State<ServerState>,
    Json(request): Json<NetworkIdRequest>,
) -> ApiResult<Json<UiState>> {
    update_config_and_reload(&state, |config| {
        config.remove_network(&request.network_id)?;
        Ok("Network removed.".to_string())
    })
}

async fn set_network_enabled(
    State(state): State<ServerState>,
    Json(request): Json<NetworkEnabledRequest>,
) -> ApiResult<Json<UiState>> {
    update_config_and_reload(&state, |config| {
        config.set_network_enabled(&request.network_id, request.enabled)?;
        Ok(if request.enabled {
            "Network activated.".to_string()
        } else {
            "Network updated.".to_string()
        })
    })
}

async fn set_network_join_requests_enabled(
    State(state): State<ServerState>,
    Json(request): Json<NetworkEnabledRequest>,
) -> ApiResult<Json<UiState>> {
    let mut config = load_config(&state.config_path).map_err(internal_error)?;
    config
        .set_network_join_requests_enabled(&request.network_id, request.enabled)
        .map_err(bad_request)?;
    finalize_config_change(&state, &mut config).map_err(bad_request)?;
    if local_join_request_listener_enabled(&config) {
        connect_vpn_inner(&state).map_err(bad_request)?;
    }
    set_action_status(
        &state,
        if request.enabled {
            "Join requests enabled."
        } else {
            "Join requests disabled."
        },
    );
    Ok(Json(build_ui_state(&state).map_err(internal_error)?))
}

async fn request_network_join(
    State(state): State<ServerState>,
    Json(request): Json<NetworkIdRequest>,
) -> ApiResult<Json<UiState>> {
    let mut config = load_config(&state.config_path).map_err(internal_error)?;
    let network = config
        .network_by_id(&request.network_id)
        .ok_or_else(|| ApiError::bad_request("network not found"))?
        .clone();

    let mut recipients = network.admins.clone();
    recipients.sort();
    recipients.dedup();
    if recipients.is_empty() {
        return Err(ApiError::bad_request(
            "this network was not imported from an invite",
        ));
    }

    let primary_recipient = preferred_join_request_recipient(&network)
        .or_else(|| recipients.first().cloned())
        .ok_or_else(|| ApiError::bad_request("this network was not imported from an invite"))?;

    if let Some(existing) = &network.outbound_join_request
        && existing.recipient == primary_recipient
    {
        return Ok(Json(build_ui_state(&state).map_err(internal_error)?));
    }

    if let Some(target) = config.network_by_id_mut(&request.network_id) {
        target.outbound_join_request = Some(PendingOutboundJoinRequest {
            recipient: primary_recipient.clone(),
            requested_at: current_unix_timestamp(),
        });
    }

    finalize_config_change(&state, &mut config).map_err(bad_request)?;
    let status = fetch_cli_status(&state).ok();
    if status.as_ref().is_none_or(|value| !value.daemon.running) {
        connect_vpn_inner(&state).map_err(bad_request)?;
        set_action_status(&state, "Join request queued and FIPS mesh started.");
    } else {
        set_action_status(&state, "Join request queued for FIPS delivery.");
    }
    Ok(Json(build_ui_state(&state).map_err(internal_error)?))
}

async fn add_participant(
    State(state): State<ServerState>,
    Json(request): Json<ParticipantRequest>,
) -> ApiResult<Json<UiState>> {
    update_config_and_reload(&state, |config| {
        let normalized =
            config.add_participant_to_network(&request.network_id, request.npub.trim())?;
        if let Some(alias) = request.alias.as_deref()
            && !alias.trim().is_empty()
        {
            config.set_peer_alias(&normalized, alias)?;
        }
        Ok("Participant saved.".to_string())
    })
}

async fn add_admin(
    State(state): State<ServerState>,
    Json(request): Json<NetworkPeerRequest>,
) -> ApiResult<Json<UiState>> {
    update_config_and_reload(&state, |config| {
        config.add_admin_to_network(&request.network_id, &request.npub)?;
        Ok("Admin saved.".to_string())
    })
}

async fn import_network_invite(
    State(state): State<ServerState>,
    Json(request): Json<InviteRequest>,
) -> ApiResult<Json<UiState>> {
    update_config_and_reload(&state, |config| {
        let invite = parse_network_invite(&request.invite)?;
        apply_network_invite_to_active_network(config, &invite)?;
        let network_name = config
            .active_network_opt()
            .map(|network| network.name.clone())
            .unwrap_or_else(|| "network".to_string());
        Ok(format!("Invite imported for {network_name}."))
    })
}

async fn start_invite_broadcast(State(state): State<ServerState>) -> ApiResult<Json<UiState>> {
    lan_pairing_unavailable(&state)
}

async fn stop_invite_broadcast(State(state): State<ServerState>) -> ApiResult<Json<UiState>> {
    lan_pairing_unavailable(&state)
}

async fn start_nearby_discovery(State(state): State<ServerState>) -> ApiResult<Json<UiState>> {
    lan_pairing_unavailable(&state)
}

async fn stop_nearby_discovery(State(state): State<ServerState>) -> ApiResult<Json<UiState>> {
    lan_pairing_unavailable(&state)
}

fn lan_pairing_unavailable(state: &ServerState) -> ApiResult<Json<UiState>> {
    set_action_status(
        state,
        "LAN pairing is not available in the Umbrel web build yet.",
    );
    Ok(Json(build_ui_state(state).map_err(internal_error)?))
}

async fn remove_participant(
    State(state): State<ServerState>,
    Json(request): Json<NetworkPeerRequest>,
) -> ApiResult<Json<UiState>> {
    update_config_and_reload(&state, |config| {
        let normalized = normalize_nostr_pubkey(&request.npub)?;
        config.remove_participant_from_network(&request.network_id, &normalized)?;
        if let Some(network) = config.network_by_id_mut(&request.network_id) {
            if network.invite_inviter == normalized {
                network.invite_inviter.clear();
            }
            if network
                .outbound_join_request
                .as_ref()
                .is_some_and(|pending| pending.recipient == normalized)
            {
                network.outbound_join_request = None;
            }
            network
                .inbound_join_requests
                .retain(|pending| pending.requester != normalized);
        }
        Ok("Participant removed.".to_string())
    })
}

async fn remove_admin(
    State(state): State<ServerState>,
    Json(request): Json<NetworkPeerRequest>,
) -> ApiResult<Json<UiState>> {
    update_config_and_reload(&state, |config| {
        let normalized = normalize_nostr_pubkey(&request.npub)?;
        config.remove_admin_from_network(&request.network_id, &normalized)?;
        Ok("Admin removed.".to_string())
    })
}

async fn accept_join_request(
    State(state): State<ServerState>,
    Json(request): Json<JoinRequestAction>,
) -> ApiResult<Json<UiState>> {
    let mut config = load_config(&state.config_path).map_err(internal_error)?;
    let requester = normalize_nostr_pubkey(&request.requester_npub).map_err(bad_request)?;
    let requester_node_name = config
        .network_by_id(&request.network_id)
        .and_then(|network| {
            network
                .inbound_join_requests
                .iter()
                .find(|pending| pending.requester == requester)
                .map(|pending| pending.requester_node_name.clone())
        })
        .unwrap_or_default();
    config
        .add_participant_to_network(&request.network_id, &requester)
        .map_err(bad_request)?;
    if !requester_node_name.trim().is_empty() {
        let _ = config.set_peer_alias(&requester, &requester_node_name);
    }
    if let Some(network) = config.network_by_id_mut(&request.network_id) {
        network
            .inbound_join_requests
            .retain(|pending| pending.requester != requester);
    }
    finalize_config_change(&state, &mut config).map_err(bad_request)?;
    let status = fetch_cli_status(&state).ok();
    if status.as_ref().is_none_or(|value| !value.daemon.running) {
        connect_vpn_inner(&state).map_err(bad_request)?;
        set_action_status(&state, "Join request accepted and VPN started.");
    } else {
        set_action_status(&state, "Join request accepted.");
    }
    Ok(Json(build_ui_state(&state).map_err(internal_error)?))
}

async fn reject_join_request(
    State(state): State<ServerState>,
    Json(request): Json<JoinRequestAction>,
) -> ApiResult<Json<UiState>> {
    update_config_and_reload(&state, |config| {
        config.reject_inbound_join_request(&request.network_id, &request.requester_npub)?;
        Ok("Join request rejected.".to_string())
    })
}

async fn set_participant_alias(
    State(state): State<ServerState>,
    Json(request): Json<AliasRequest>,
) -> ApiResult<Json<UiState>> {
    update_config_and_reload(&state, |config| {
        config.set_peer_alias(&request.npub, &request.alias)?;
        Ok("Alias saved.".to_string())
    })
}

async fn update_settings(
    State(state): State<ServerState>,
    Json(patch): Json<SettingsPatch>,
) -> ApiResult<Json<UiState>> {
    update_config_and_reload(&state, |config| {
        if let Some(node_name) = patch.node_name {
            config.node_name = node_name;
        }
        if let Some(endpoint) = patch.endpoint {
            config.node.endpoint = endpoint;
        }
        if let Some(tunnel_ip) = patch.tunnel_ip {
            config.node.tunnel_ip = tunnel_ip;
        }
        if let Some(listen_port) = patch.listen_port {
            if listen_port == 0 {
                return Err(anyhow!("listen port must be > 0"));
            }
            config.node.listen_port = listen_port;
        }
        if let Some(exit_node) = patch.exit_node {
            config.exit_node = parse_exit_node_input(&exit_node)?;
        }
        if let Some(advertise_exit_node) = patch.advertise_exit_node {
            config.node.advertise_exit_node = advertise_exit_node;
        }
        if let Some(advertised_routes) = patch.advertised_routes {
            config.node.advertised_routes = parse_advertised_routes_input(&advertised_routes)?;
        }
        if let Some(magic_dns_suffix) = patch.magic_dns_suffix {
            config.magic_dns_suffix = magic_dns_suffix;
        }
        if let Some(autoconnect) = patch.autoconnect {
            config.autoconnect = autoconnect;
        }
        if let Some(launch_on_startup) = patch.launch_on_startup {
            config.launch_on_startup = launch_on_startup;
        }
        if let Some(close_to_tray_on_close) = patch.close_to_tray_on_close {
            config.close_to_tray_on_close = close_to_tray_on_close;
        }
        Ok("Settings saved.".to_string())
    })
}
