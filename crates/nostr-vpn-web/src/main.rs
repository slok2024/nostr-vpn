use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use nostr_vpn_app_core::{FfiApp, NativeAppAction, NativeAppState, SettingsPatch};
use nostr_vpn_core::config::{AppConfig, maybe_autoconfigure_node};
use qrcode::QrCode;
use serde_json::{Value, json};
use tower_http::services::{ServeDir, ServeFile};

mod ui_types;

use crate::ui_types::{
    AliasRequest, InviteRequest, JoinRequestAction, NameRequest, NetworkEnabledRequest,
    NetworkIdRequest, NetworkMeshRequest, NetworkNameRequest, NetworkPeerRequest,
    ParticipantRequest, QrMatrixRequest, QrMatrixResponse,
};

const NVPN_BIN_ENV: &str = "NVPN_CLI_PATH";
const DEFAULT_STATIC_DIR: &str = "/usr/share/nostr-vpn/web";

#[derive(Debug, Parser)]
#[command(name = "nvpn-web")]
#[command(about = "HTTP API for the nostr-vpn web UI")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:8081")]
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
    core: Arc<FfiApp>,
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
            Json(json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiError>;
type UiStateResponse = Json<Value>;

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
        core: FfiApp::new_with_config_path(
            config_path,
            env!("CARGO_PKG_VERSION").to_string(),
            Some(nvpn_bin),
        ),
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
        .route("/api/qr_matrix", post(qr_matrix))
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

async fn tick(State(state): State<ServerState>) -> ApiResult<UiStateResponse> {
    state_response(state.core.refresh())
}

async fn connect_vpn(State(state): State<ServerState>) -> ApiResult<UiStateResponse> {
    dispatch(&state, NativeAppAction::ConnectVpn)
}

async fn disconnect_vpn(State(state): State<ServerState>) -> ApiResult<UiStateResponse> {
    dispatch(&state, NativeAppAction::DisconnectVpn)
}

async fn add_network(
    State(state): State<ServerState>,
    Json(request): Json<NameRequest>,
) -> ApiResult<UiStateResponse> {
    dispatch(&state, NativeAppAction::AddNetwork { name: request.name })
}

async fn rename_network(
    State(state): State<ServerState>,
    Json(request): Json<NetworkNameRequest>,
) -> ApiResult<UiStateResponse> {
    dispatch(
        &state,
        NativeAppAction::RenameNetwork {
            network_id: request.network_id,
            name: request.name,
        },
    )
}

async fn set_network_mesh_id(
    State(state): State<ServerState>,
    Json(request): Json<NetworkMeshRequest>,
) -> ApiResult<UiStateResponse> {
    dispatch(
        &state,
        NativeAppAction::SetNetworkMeshId {
            network_id: request.network_id,
            mesh_id: request.mesh_id,
        },
    )
}

async fn remove_network(
    State(state): State<ServerState>,
    Json(request): Json<NetworkIdRequest>,
) -> ApiResult<UiStateResponse> {
    dispatch(
        &state,
        NativeAppAction::RemoveNetwork {
            network_id: request.network_id,
        },
    )
}

async fn set_network_enabled(
    State(state): State<ServerState>,
    Json(request): Json<NetworkEnabledRequest>,
) -> ApiResult<UiStateResponse> {
    dispatch(
        &state,
        NativeAppAction::SetNetworkEnabled {
            network_id: request.network_id,
            enabled: request.enabled,
        },
    )
}

async fn set_network_join_requests_enabled(
    State(state): State<ServerState>,
    Json(request): Json<NetworkEnabledRequest>,
) -> ApiResult<UiStateResponse> {
    dispatch(
        &state,
        NativeAppAction::SetNetworkJoinRequestsEnabled {
            network_id: request.network_id,
            enabled: request.enabled,
        },
    )
}

async fn request_network_join(
    State(state): State<ServerState>,
    Json(request): Json<NetworkIdRequest>,
) -> ApiResult<UiStateResponse> {
    dispatch(
        &state,
        NativeAppAction::RequestNetworkJoin {
            network_id: request.network_id,
        },
    )
}

async fn add_participant(
    State(state): State<ServerState>,
    Json(request): Json<ParticipantRequest>,
) -> ApiResult<UiStateResponse> {
    dispatch(
        &state,
        NativeAppAction::AddParticipant {
            network_id: request.network_id,
            npub: request.npub,
            alias: request.alias,
        },
    )
}

async fn add_admin(
    State(state): State<ServerState>,
    Json(request): Json<NetworkPeerRequest>,
) -> ApiResult<UiStateResponse> {
    dispatch(
        &state,
        NativeAppAction::AddAdmin {
            network_id: request.network_id,
            npub: request.npub,
        },
    )
}

async fn import_network_invite(
    State(state): State<ServerState>,
    Json(request): Json<InviteRequest>,
) -> ApiResult<UiStateResponse> {
    dispatch(
        &state,
        NativeAppAction::ImportNetworkInvite {
            invite: request.invite,
        },
    )
}

async fn start_invite_broadcast(State(state): State<ServerState>) -> ApiResult<UiStateResponse> {
    dispatch(&state, NativeAppAction::StartInviteBroadcast)
}

async fn stop_invite_broadcast(State(state): State<ServerState>) -> ApiResult<UiStateResponse> {
    dispatch(&state, NativeAppAction::StopInviteBroadcast)
}

async fn start_nearby_discovery(State(state): State<ServerState>) -> ApiResult<UiStateResponse> {
    dispatch(&state, NativeAppAction::StartNearbyDiscovery)
}

async fn stop_nearby_discovery(State(state): State<ServerState>) -> ApiResult<UiStateResponse> {
    dispatch(&state, NativeAppAction::StopNearbyDiscovery)
}

async fn remove_participant(
    State(state): State<ServerState>,
    Json(request): Json<NetworkPeerRequest>,
) -> ApiResult<UiStateResponse> {
    dispatch(
        &state,
        NativeAppAction::RemoveParticipant {
            network_id: request.network_id,
            npub: request.npub,
        },
    )
}

async fn remove_admin(
    State(state): State<ServerState>,
    Json(request): Json<NetworkPeerRequest>,
) -> ApiResult<UiStateResponse> {
    dispatch(
        &state,
        NativeAppAction::RemoveAdmin {
            network_id: request.network_id,
            npub: request.npub,
        },
    )
}

async fn accept_join_request(
    State(state): State<ServerState>,
    Json(request): Json<JoinRequestAction>,
) -> ApiResult<UiStateResponse> {
    dispatch(
        &state,
        NativeAppAction::AcceptJoinRequest {
            network_id: request.network_id,
            requester_npub: request.requester_npub,
        },
    )
}

async fn reject_join_request(
    State(state): State<ServerState>,
    Json(request): Json<JoinRequestAction>,
) -> ApiResult<UiStateResponse> {
    dispatch(
        &state,
        NativeAppAction::RejectJoinRequest {
            network_id: request.network_id,
            requester_npub: request.requester_npub,
        },
    )
}

async fn set_participant_alias(
    State(state): State<ServerState>,
    Json(request): Json<AliasRequest>,
) -> ApiResult<UiStateResponse> {
    dispatch(
        &state,
        NativeAppAction::SetParticipantAlias {
            npub: request.npub,
            alias: request.alias,
        },
    )
}

async fn update_settings(
    State(state): State<ServerState>,
    Json(patch): Json<SettingsPatch>,
) -> ApiResult<UiStateResponse> {
    dispatch(&state, NativeAppAction::UpdateSettings { patch })
}

async fn qr_matrix(Json(request): Json<QrMatrixRequest>) -> ApiResult<Json<QrMatrixResponse>> {
    Ok(Json(build_qr_matrix(&request.text).map_err(bad_request)?))
}

fn dispatch(state: &ServerState, action: NativeAppAction) -> ApiResult<UiStateResponse> {
    let next = state.core.dispatch(action);
    let error = next.error.trim();
    if !error.is_empty() {
        return Err(ApiError::bad_request(error));
    }
    state_response(next)
}

fn state_response(state: NativeAppState) -> ApiResult<UiStateResponse> {
    Ok(Json(umbrel_state_value(state)?))
}

fn umbrel_state_value(state: NativeAppState) -> ApiResult<Value> {
    let mut value = serde_json::to_value(state).map_err(internal_error)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| ApiError::internal("failed to encode app state"))?;

    let vpn_enabled = object
        .get("vpnEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let vpn_active = object
        .get("vpnActive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let vpn_status = object
        .get("vpnStatus")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !vpn_enabled
        && !vpn_active
        && (vpn_status.is_empty()
            || vpn_status == "Disconnected"
            || vpn_status == "Daemon running"
            || vpn_status == "Daemon not running")
    {
        object.insert("vpnStatus".to_string(), json!("VPN off"));
    }

    object.insert("platform".to_string(), json!("umbrel"));
    object.insert("mobile".to_string(), json!(false));
    object.insert("vpnControlSupported".to_string(), json!(true));
    object.insert("cliInstallSupported".to_string(), json!(false));
    object.insert("startupSettingsSupported".to_string(), json!(false));
    object.insert("trayBehaviorSupported".to_string(), json!(false));
    object.insert("runtimeStatusDetail".to_string(), json!(""));
    object.insert("cliInstalled".to_string(), json!(false));
    object.insert("serviceSupported".to_string(), json!(false));
    object.insert("serviceEnablementSupported".to_string(), json!(false));
    object.insert("serviceInstalled".to_string(), json!(false));
    object.insert("serviceDisabled".to_string(), json!(false));
    object.insert("serviceRunning".to_string(), json!(false));
    object.insert(
        "serviceStatusDetail".to_string(),
        json!("Managed directly by the Umbrel app"),
    );

    Ok(value)
}

fn build_qr_matrix(text: &str) -> Result<QrMatrixResponse> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(QrMatrixResponse {
            width: 0,
            cells: Vec::new(),
        });
    }

    let code = QrCode::new(trimmed.as_bytes())?;
    let width = code.width();
    let cells = code
        .to_colors()
        .into_iter()
        .map(|color| matches!(color, qrcode::Color::Dark))
        .collect();
    Ok(QrMatrixResponse { width, cells })
}

fn ensure_config_exists(path: &Path) -> Result<()> {
    let mut config = if path.exists() {
        AppConfig::load(path).with_context(|| format!("failed to load {}", path.display()))?
    } else {
        AppConfig::generated()
    };
    config.ensure_defaults();
    maybe_autoconfigure_node(&mut config);
    config
        .save(path)
        .with_context(|| format!("failed to save {}", path.display()))
}

fn default_config_path() -> PathBuf {
    dirs::config_dir().map_or_else(
        || PathBuf::from("nvpn.toml"),
        |dir| dir.join("nvpn").join("config.toml"),
    )
}

fn discover_static_dir() -> Option<PathBuf> {
    let path = PathBuf::from(DEFAULT_STATIC_DIR);
    path.join("index.html").exists().then_some(path)
}

fn resolve_nvpn_cli_path(override_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return validate_executable(&path);
    }
    if let Some(path) = env::var_os(NVPN_BIN_ENV) {
        return validate_executable(&PathBuf::from(path));
    }
    if let Some(path_var) = env::var_os("PATH") {
        for dir in env::split_paths(&path_var) {
            let candidate = dir.join(nvpn_binary_name());
            if candidate.exists()
                && let Ok(validated) = validate_executable(&candidate)
            {
                return Ok(validated);
            }
        }
    }
    Err(anyhow!(
        "nvpn CLI binary not found; set {} or add nvpn to PATH",
        NVPN_BIN_ENV
    ))
}

#[cfg(target_os = "windows")]
fn nvpn_binary_name() -> &'static str {
    "nvpn.exe"
}

#[cfg(not(target_os = "windows"))]
fn nvpn_binary_name() -> &'static str {
    "nvpn"
}

fn validate_executable(path: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("failed to canonicalize {}", path.display()))?;
    let metadata = fs::metadata(&canonical)
        .with_context(|| format!("failed to inspect {}", canonical.display()))?;
    if !metadata.is_file() {
        return Err(anyhow!("{} is not a file", canonical.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(anyhow!("{} is not executable", canonical.display()));
        }
    }
    Ok(canonical)
}

fn bad_request(error: anyhow::Error) -> ApiError {
    ApiError::bad_request(error.to_string())
}

fn internal_error(error: impl std::fmt::Display) -> ApiError {
    ApiError::internal(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_listen_address_is_loopback() {
        let args = Args::parse_from(["nvpn-web"]);

        assert!(args.listen.ip().is_loopback());
        assert_eq!(args.listen.port(), 8081);
    }

    #[test]
    fn qr_matrix_encodes_invite_text() {
        let matrix = build_qr_matrix("nvpn://invite/example").expect("qr matrix");

        assert!(matrix.width > 0);
        assert_eq!(matrix.cells.len(), matrix.width * matrix.width);
        assert!(matrix.cells.iter().any(|cell| *cell));
    }

    #[test]
    fn umbrel_state_hides_desktop_service_controls() {
        let state = NativeAppState {
            platform: "desktop".to_string(),
            cli_install_supported: true,
            startup_settings_supported: true,
            tray_behavior_supported: true,
            service_supported: true,
            service_enablement_supported: true,
            service_installed: true,
            service_running: true,
            vpn_status: "Disconnected".to_string(),
            ..NativeAppState::default()
        };

        let value = umbrel_state_value(state).expect("state value");

        assert_eq!(value["platform"], "umbrel");
        assert_eq!(value["vpnStatus"], "VPN off");
        assert_eq!(value["cliInstallSupported"], false);
        assert_eq!(value["startupSettingsSupported"], false);
        assert_eq!(value["trayBehaviorSupported"], false);
        assert_eq!(value["serviceSupported"], false);
        assert_eq!(value["serviceEnablementSupported"], false);
        assert_eq!(value["serviceInstalled"], false);
        assert_eq!(value["serviceRunning"], false);
        assert_eq!(
            value["serviceStatusDetail"],
            "Managed directly by the Umbrel app"
        );
    }
}
