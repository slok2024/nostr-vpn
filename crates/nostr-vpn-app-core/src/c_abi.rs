#[cfg(target_os = "android")]
use std::ffi::c_void;
use std::ffi::{CStr, CString, c_char};
use std::panic::{self, AssertUnwindSafe};
use std::ptr;
use std::sync::Arc;
#[cfg(target_os = "android")]
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use image::ImageReader;
#[cfg(target_os = "android")]
use jni::JNIEnv;
#[cfg(target_os = "android")]
use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JString};
#[cfg(target_os = "android")]
use jni::sys::{jboolean, jint, jlong, jstring};
use qrcode::QrCode;
use serde::Serialize;

use crate::mobile_tunnel::{
    MobileTunnel, mobile_debug_log, tunnel_config_json, tunnel_provider_options_config_json,
};
use crate::{FfiApp, NativeAppAction, NativeAppState};

pub struct NvpnAppHandle {
    app: Arc<FfiApp>,
}

pub struct NvpnMobileTunnelHandle {
    tunnel: MobileTunnel,
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

/// # Safety
///
/// `handle` must be null or a pointer returned by `nostr_vpn_app_new` that has not already been
/// freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nostr_vpn_app_free(handle: *mut NvpnAppHandle) {
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
pub extern "C" fn nostr_vpn_mobile_tunnel_config_json(data_dir: *const c_char) -> *mut c_char {
    let config_json = tunnel_config_json(&c_string_lossy(data_dir));
    json_raw_string(&config_json)
}

#[unsafe(no_mangle)]
pub extern "C" fn nostr_vpn_mobile_tunnel_provider_options_config_json(
    data_dir: *const c_char,
) -> *mut c_char {
    let config_json = tunnel_provider_options_config_json(&c_string_lossy(data_dir));
    json_raw_string(&config_json)
}

/// # Safety
///
/// `handle` must be a live mobile tunnel handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nostr_vpn_mobile_tunnel_runtime_state_json(
    handle: *const NvpnMobileTunnelHandle,
) -> *mut c_char {
    if handle.is_null() {
        return json_raw_string(r#"{"error":"mobile tunnel stopped"}"#);
    }
    let tunnel = unsafe { &*handle };
    match tunnel.tunnel.runtime_state_json() {
        Ok(json) => json_raw_string(&json),
        Err(error) => {
            let json = serde_json::json!({ "error": error.to_string() }).to_string();
            json_raw_string(&json)
        }
    }
}

/// # Safety
///
/// `handle` must be a live mobile tunnel handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nostr_vpn_mobile_tunnel_take_app_config_toml(
    handle: *const NvpnMobileTunnelHandle,
) -> *mut c_char {
    if handle.is_null() {
        return json_raw_string("");
    }
    let tunnel = unsafe { &*handle };
    match tunnel.tunnel.take_app_config_toml() {
        Ok(toml) => json_raw_string(&toml),
        Err(error) => json_raw_string(&format!("# failed to read mobile app config: {error}\n")),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn nostr_vpn_mobile_tunnel_new(
    config_json: *const c_char,
) -> *mut NvpnMobileTunnelHandle {
    let config_json = c_string_lossy(config_json);
    mobile_debug_log("nostr_vpn_mobile_tunnel_new enter");
    match panic::catch_unwind(AssertUnwindSafe(|| MobileTunnel::start(&config_json))) {
        Ok(Ok(tunnel)) => {
            mobile_debug_log("nostr_vpn_mobile_tunnel_new success");
            Box::into_raw(Box::new(NvpnMobileTunnelHandle { tunnel }))
        }
        Ok(Err(error)) => {
            mobile_debug_log(format!("nostr_vpn_mobile_tunnel_new error: {error:#}"));
            ptr::null_mut()
        }
        Err(payload) => {
            mobile_debug_log(format!(
                "nostr_vpn_mobile_tunnel_new panic: {}",
                panic_payload_message(&payload)
            ));
            ptr::null_mut()
        }
    }
}

/// # Safety
///
/// `handle` must be null or a pointer returned by `nostr_vpn_mobile_tunnel_new` that has not
/// already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nostr_vpn_mobile_tunnel_free(handle: *mut NvpnMobileTunnelHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

/// # Safety
///
/// `handle` must be a live mobile tunnel handle. `packet` must point to `len`
/// readable bytes for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nostr_vpn_mobile_tunnel_send_packet(
    handle: *const NvpnMobileTunnelHandle,
    packet: *const u8,
    len: usize,
) -> bool {
    if handle.is_null() || packet.is_null() || len == 0 {
        return false;
    }
    let tunnel = unsafe { &*handle };
    let packet = unsafe { std::slice::from_raw_parts(packet, len) };
    tunnel.tunnel.send_packet(packet)
}

/// Raw fd of the userspace WG upstream UDP socket, or -1 when WG
/// upstream isn't running on this tunnel. The Android host calls
/// `VpnService.protect(fd)` on this fd so the encrypted UDP escapes
/// the VPN tun. iOS instead asks for the resolved upstream route below
/// and installs it in `NEIPv4Settings.excludedRoutes`.
///
/// # Safety
///
/// `handle` must be a live mobile tunnel handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nostr_vpn_mobile_tunnel_wg_socket_fd(
    handle: *const NvpnMobileTunnelHandle,
) -> std::os::raw::c_int {
    if handle.is_null() {
        return -1;
    }
    let tunnel = unsafe { &*handle };
    tunnel.tunnel.wg_upstream_socket_fd()
}

/// Resolved IPv4 `/32` route for the userspace WG upstream UDP endpoint.
/// iOS adds this to `NEIPv4Settings.excludedRoutes` so encrypted UDP
/// continues to escape after the packet tunnel installs `0.0.0.0/0`.
/// Returns an empty string when WG upstream is not running or resolved
/// to an IPv6 endpoint.
///
/// # Safety
///
/// `handle` must be a live mobile tunnel handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nostr_vpn_mobile_tunnel_wg_excluded_route(
    handle: *const NvpnMobileTunnelHandle,
) -> *mut c_char {
    if handle.is_null() {
        return json_raw_string("");
    }
    let tunnel = unsafe { &*handle };
    json_raw_string(
        &tunnel
            .tunnel
            .wg_upstream_excluded_route()
            .unwrap_or_default(),
    )
}

/// # Safety
///
/// `handle` must be a live mobile tunnel handle. `out` must point to
/// `capacity` writable bytes for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nostr_vpn_mobile_tunnel_next_packet(
    handle: *const NvpnMobileTunnelHandle,
    out: *mut u8,
    capacity: usize,
    timeout_ms: u32,
) -> isize {
    if handle.is_null() || out.is_null() || capacity == 0 {
        return -1;
    }
    let tunnel = unsafe { &*handle };
    let out = unsafe { std::slice::from_raw_parts_mut(out, capacity) };
    match tunnel
        .tunnel
        .next_packet(out, Duration::from_millis(u64::from(timeout_ms)))
    {
        Ok(len) => isize::try_from(len).unwrap_or(-1),
        Err(_) => -1,
    }
}

/// # Safety
///
/// `value` must be null or a pointer returned by this library that has not already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nostr_vpn_string_free(value: *mut c_char) {
    if value.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(value));
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_nostrvpn_app_core_NativeCore_initializeAndroidContext(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    context: JObject<'_>,
) {
    static ANDROID_CONTEXT: OnceLock<Option<GlobalRef>> = OnceLock::new();
    ANDROID_CONTEXT.get_or_init(|| match env.new_global_ref(&context) {
        Ok(global) => {
            let Ok(vm) = env.get_java_vm() else {
                return None;
            };
            unsafe {
                ndk_context::initialize_android_context(
                    vm.get_java_vm_pointer() as *mut c_void,
                    global.as_obj().as_raw() as *mut c_void,
                );
            }
            Some(global)
        }
        Err(_) => None,
    });
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_nostrvpn_app_core_NativeCore_appNew(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
    app_version: JString<'_>,
) -> jlong {
    let data_dir = jni_string_lossy(&mut env, &data_dir);
    let app_version = jni_string_lossy(&mut env, &app_version);
    Box::into_raw(Box::new(NvpnAppHandle {
        app: FfiApp::new(data_dir, app_version),
    })) as jlong
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_nostrvpn_app_core_NativeCore_appFree(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle == 0 {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle as *mut NvpnAppHandle));
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_nostrvpn_app_core_NativeCore_stateJson(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jstring {
    jni_state_json(env, handle, |handle| handle.app.state())
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_nostrvpn_app_core_NativeCore_refreshJson(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jstring {
    jni_state_json(env, handle, |handle| handle.app.refresh())
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_nostrvpn_app_core_NativeCore_dispatchJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    action_json: JString<'_>,
) -> jstring {
    let state = app_from_jlong(handle).map_or_else(
        |error| error_state(error.to_string()),
        |handle| {
            let action_json = jni_string_lossy(&mut env, &action_json);
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
    jni_json_string(env, &state)
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_nostrvpn_app_core_NativeCore_qrMatrixJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    text: JString<'_>,
) -> jstring {
    let text = jni_string_lossy(&mut env, &text);
    let result = qr_matrix(&text).unwrap_or_else(|error| QrMatrixResult {
        width: 0,
        cells: Vec::new(),
        error: error.to_string(),
    });
    jni_json_string(env, &result)
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_nostrvpn_app_core_NativeCore_decodeQrImageJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    path: JString<'_>,
) -> jstring {
    let path = jni_string_lossy(&mut env, &path);
    let result = decode_qr_image(&path).map_or_else(
        |error| QrDecodeResult {
            value: String::new(),
            error: error.to_string(),
        },
        |value| QrDecodeResult {
            value,
            error: String::new(),
        },
    );
    jni_json_string(env, &result)
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_nostrvpn_app_core_NativeCore_mobileTunnelConfigJson(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data_dir: JString<'_>,
) -> jstring {
    let data_dir = jni_string_lossy(&mut env, &data_dir);
    jni_raw_string(env, tunnel_config_json(&data_dir))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_nostrvpn_app_core_NativeCore_mobileTunnelNew(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    config_json: JString<'_>,
) -> jlong {
    let config_json = jni_string_lossy(&mut env, &config_json);
    match MobileTunnel::start(&config_json) {
        Ok(tunnel) => Box::into_raw(Box::new(NvpnMobileTunnelHandle { tunnel })) as jlong,
        Err(_) => 0,
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_nostrvpn_app_core_NativeCore_mobileTunnelFree(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle == 0 {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle as *mut NvpnMobileTunnelHandle));
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_nostrvpn_app_core_NativeCore_mobileTunnelSendPacket(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    packet: JByteArray<'_>,
    len: jint,
) -> jboolean {
    let Some(tunnel) = tunnel_from_jlong(handle) else {
        return 0;
    };
    let Ok(mut bytes) = env.convert_byte_array(&packet) else {
        return 0;
    };
    let len = usize::try_from(len).unwrap_or(0).min(bytes.len());
    bytes.truncate(len);
    u8::from(tunnel.tunnel.send_packet(&bytes))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_nostrvpn_app_core_NativeCore_mobileTunnelNextPacket(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    out: JByteArray<'_>,
    timeout_ms: jint,
) -> jint {
    let Some(tunnel) = tunnel_from_jlong(handle) else {
        return -1;
    };
    let Ok(capacity) = env.get_array_length(&out) else {
        return -1;
    };
    let mut buffer = vec![0_u8; usize::try_from(capacity).unwrap_or(0)];
    let timeout_ms = u64::try_from(timeout_ms).unwrap_or(0);
    let len = match tunnel
        .tunnel
        .next_packet(&mut buffer, Duration::from_millis(timeout_ms))
    {
        Ok(len) => len,
        Err(_) => return -1,
    };
    if len == 0 {
        return 0;
    }
    let signed = buffer[..len]
        .iter()
        .map(|byte| i8::from_ne_bytes([*byte]))
        .collect::<Vec<_>>();
    if env.set_byte_array_region(&out, 0, &signed).is_err() {
        return -1;
    }
    jint::try_from(len).unwrap_or(-1)
}

/// Returns the raw fd of the userspace WG upstream UDP socket so the
/// VpnService can call `protect(fd)` on it. Returns -1 when WG
/// upstream isn't running on this tunnel.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_nostrvpn_app_core_NativeCore_mobileTunnelWgSocketFd(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jint {
    let Some(tunnel) = tunnel_from_jlong(handle) else {
        return -1;
    };
    tunnel.tunnel.wg_upstream_socket_fd()
}

fn app_from_handle<'a>(handle: *const NvpnAppHandle) -> Result<&'a NvpnAppHandle> {
    if handle.is_null() {
        return Err(anyhow!("native app handle is null"));
    }
    Ok(unsafe { &*handle })
}

#[cfg(target_os = "android")]
fn app_from_jlong<'a>(handle: jlong) -> Result<&'a NvpnAppHandle> {
    if handle == 0 {
        return Err(anyhow!("native app handle is null"));
    }
    Ok(unsafe { &*(handle as *const NvpnAppHandle) })
}

#[cfg(target_os = "android")]
fn tunnel_from_jlong<'a>(handle: jlong) -> Option<&'a NvpnMobileTunnelHandle> {
    if handle == 0 {
        return None;
    }
    Some(unsafe { &*(handle as *const NvpnMobileTunnelHandle) })
}

fn c_string_lossy(value: *const c_char) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(target_os = "android")]
fn jni_string_lossy(env: &mut JNIEnv<'_>, value: &JString<'_>) -> String {
    env.get_string(value).map_or_else(
        |_| String::new(),
        |value| value.to_string_lossy().into_owned(),
    )
}

#[cfg(target_os = "android")]
fn jni_state_json(
    env: JNIEnv<'_>,
    handle: jlong,
    state: impl FnOnce(&NvpnAppHandle) -> NativeAppState,
) -> jstring {
    let state = app_from_jlong(handle).map_or_else(
        |error| error_state(error.to_string()),
        |handle| state(handle),
    );
    jni_json_string(env, &state)
}

#[cfg(target_os = "android")]
fn jni_json_string(env: JNIEnv<'_>, value: &impl Serialize) -> jstring {
    let json =
        serde_json::to_string(value).unwrap_or_else(|error| format!(r#"{{"error":"{error}"}}"#));
    jni_raw_string(env, json)
}

#[cfg(target_os = "android")]
fn jni_raw_string(env: JNIEnv<'_>, value: String) -> jstring {
    env.new_string(value)
        .map_or(ptr::null_mut(), |value| value.into_raw())
}

fn json_string(value: &impl Serialize) -> *mut c_char {
    match serde_json::to_string(value) {
        Ok(json) => into_c_string(&json),
        Err(error) => into_c_string(&format!(r#"{{"error":"{error}"}}"#)),
    }
}

fn json_raw_string(value: &str) -> *mut c_char {
    into_c_string(value)
}

fn into_c_string(value: &str) -> *mut c_char {
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

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
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

        unsafe {
            nostr_vpn_app_free(handle);
        }
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
            usize::try_from(width * width).expect("matrix cell count fits usize")
        );
        assert_eq!(value["error"], "");
    }

    #[test]
    fn decode_qr_image_reads_generated_invite() {
        let invite = "nvpn://invite/example";
        let code = QrCode::new(invite.as_bytes()).expect("QR code");
        let module_count = u32::try_from(code.width()).expect("QR width fits u32");
        let scale = 8;
        let quiet_zone = 4;
        let image_width = (module_count + quiet_zone * 2) * scale;
        let mut image = image::GrayImage::from_pixel(image_width, image_width, image::Luma([255]));

        for (index, color) in code.to_colors().into_iter().enumerate() {
            if !matches!(color, qrcode::Color::Dark) {
                continue;
            }
            let x = u32::try_from(index % code.width()).expect("QR x fits u32");
            let y = u32::try_from(index / code.width()).expect("QR y fits u32");
            for dy in 0..scale {
                for dx in 0..scale {
                    image.put_pixel(
                        (x + quiet_zone) * scale + dx,
                        (y + quiet_zone) * scale + dy,
                        image::Luma([0]),
                    );
                }
            }
        }

        let dir = temp_data_dir();
        fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("invite.png");
        image.save(&path).expect("save QR image");

        let decoded = decode_qr_image(path.to_str().expect("UTF-8 path")).expect("decode QR");
        assert_eq!(decoded, invite);
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
        unsafe {
            nostr_vpn_string_free(value);
        }
        text
    }
}
