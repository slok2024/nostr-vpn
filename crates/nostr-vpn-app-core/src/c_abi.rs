use std::ffi::{CStr, CString, c_char};
use std::ptr;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use image::ImageReader;
use qrcode::QrCode;
use serde::Serialize;

use crate::{FfiApp, NativeAppAction, NativeAppState};

pub struct NvpnAppHandle {
    app: Arc<FfiApp>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QrMatrixResult {
    width: usize,
    cells: Vec<bool>,
    error: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QrDecodeResult {
    value: String,
    error: String,
}

#[unsafe(no_mangle)]
pub extern "C" fn nostr_vpn_app_new(
    data_dir: *const c_char,
    app_version: *const c_char,
) -> *mut NvpnAppHandle {
    let data_dir = c_string_lossy(data_dir);
    let app_version = c_string_lossy(app_version);
    Box::into_raw(Box::new(NvpnAppHandle {
        app: FfiApp::new(data_dir, app_version),
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn nostr_vpn_app_free(handle: *mut NvpnAppHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn nostr_vpn_app_state_json(handle: *const NvpnAppHandle) -> *mut c_char {
    let state = app_from_handle(handle).map_or_else(
        |error| error_state(error.to_string()),
        |handle| handle.app.state(),
    );
    json_string(&state)
}

#[unsafe(no_mangle)]
pub extern "C" fn nostr_vpn_app_refresh_json(handle: *const NvpnAppHandle) -> *mut c_char {
    let state = app_from_handle(handle).map_or_else(
        |error| error_state(error.to_string()),
        |handle| handle.app.refresh(),
    );
    json_string(&state)
}

#[unsafe(no_mangle)]
pub extern "C" fn nostr_vpn_app_dispatch_json(
    handle: *const NvpnAppHandle,
    action_json: *const c_char,
) -> *mut c_char {
    let state = app_from_handle(handle).map_or_else(
        |error| error_state(error.to_string()),
        |handle| {
            let action_json = c_string_lossy(action_json);
            match serde_json::from_str::<NativeAppAction>(&action_json) {
                Ok(action) => handle.app.dispatch(action),
                Err(error) => {
                    let mut state = handle.app.state();
                    state.error = format!("invalid native action JSON: {error}");
                    state
                }
            }
        },
    );
    json_string(&state)
}

#[unsafe(no_mangle)]
pub extern "C" fn nostr_vpn_qr_matrix_json(text: *const c_char) -> *mut c_char {
    let result = qr_matrix(&c_string_lossy(text)).unwrap_or_else(|error| QrMatrixResult {
        width: 0,
        cells: Vec::new(),
        error: error.to_string(),
    });
    json_string(&result)
}

#[unsafe(no_mangle)]
pub extern "C" fn nostr_vpn_decode_qr_image_json(path: *const c_char) -> *mut c_char {
    let result = decode_qr_image(&c_string_lossy(path)).map_or_else(
        |error| QrDecodeResult {
            value: String::new(),
            error: error.to_string(),
        },
        |value| QrDecodeResult {
            value,
            error: String::new(),
        },
    );
    json_string(&result)
}

#[unsafe(no_mangle)]
pub extern "C" fn nostr_vpn_string_free(value: *mut c_char) {
    if value.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(value));
    }
}

fn app_from_handle<'a>(handle: *const NvpnAppHandle) -> Result<&'a NvpnAppHandle> {
    if handle.is_null() {
        return Err(anyhow!("native app handle is null"));
    }
    Ok(unsafe { &*handle })
}

fn c_string_lossy(value: *const c_char) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}

fn json_string(value: &impl Serialize) -> *mut c_char {
    match serde_json::to_string(value) {
        Ok(json) => into_c_string(json),
        Err(error) => into_c_string(format!(r#"{{"error":"{error}"}}"#)),
    }
}

fn into_c_string(value: String) -> *mut c_char {
    match CString::new(value.replace('\0', "")) {
        Ok(value) => value.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

fn error_state(error: String) -> NativeAppState {
    NativeAppState {
        error,
        ..NativeAppState::default()
    }
}

fn qr_matrix(text: &str) -> Result<QrMatrixResult> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(QrMatrixResult {
            width: 0,
            cells: Vec::new(),
            error: String::new(),
        });
    }

    let code = QrCode::new(trimmed.as_bytes()).context("failed to encode QR invite")?;
    let width = code.width();
    let cells = code
        .to_colors()
        .into_iter()
        .map(|color| matches!(color, qrcode::Color::Dark))
        .collect();
    Ok(QrMatrixResult {
        width,
        cells,
        error: String::new(),
    })
}

fn decode_qr_image(path: &str) -> Result<String> {
    let image = ImageReader::open(path)
        .with_context(|| format!("failed to open QR image {path}"))?
        .decode()
        .with_context(|| format!("failed to decode QR image {path}"))?
        .to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(image);
    let grids = prepared.detect_grids();
    for grid in grids {
        let (_, content) = grid.decode().context("failed to read QR payload")?;
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    Err(anyhow!("no QR code found in image"))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn c_abi_returns_json_state_and_action_errors() {
        let data_dir = temp_data_dir();
        let data_dir = CString::new(data_dir.to_string_lossy().as_bytes()).expect("data dir");
        let version = CString::new("test").expect("version");
        let handle = nostr_vpn_app_new(data_dir.as_ptr(), version.as_ptr());
        assert!(!handle.is_null());

        let state_json = take_string(nostr_vpn_app_state_json(handle));
        let state: serde_json::Value = serde_json::from_str(&state_json).expect("state JSON");
        assert!(state.get("rev").is_some());
        assert!(state.get("ownNpub").is_some());

        let bad_action = CString::new(r#"{"type":"missing"}"#).expect("action");
        let state_json = take_string(nostr_vpn_app_dispatch_json(handle, bad_action.as_ptr()));
        let state: serde_json::Value = serde_json::from_str(&state_json).expect("error state JSON");
        assert!(
            state["error"]
                .as_str()
                .is_some_and(|error| error.contains("invalid native action JSON"))
        );

        nostr_vpn_app_free(handle);
    }

    #[test]
    fn qr_matrix_reports_cells_for_invite_text() {
        let text = CString::new("nvpn://invite/example").expect("text");
        let json = take_string(nostr_vpn_qr_matrix_json(text.as_ptr()));
        let value: serde_json::Value = serde_json::from_str(&json).expect("matrix JSON");
        let width = value["width"].as_u64().expect("width");
        assert!(width > 0);
        assert_eq!(
            value["cells"].as_array().expect("cells").len(),
            (width * width) as usize
        );
        assert_eq!(value["error"], "");
    }

    fn temp_data_dir() -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("nostr-vpn-c-abi-{}-{stamp}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn take_string(value: *mut c_char) -> String {
        assert!(!value.is_null());
        let text = unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned();
        nostr_vpn_string_free(value);
        text
    }
}
