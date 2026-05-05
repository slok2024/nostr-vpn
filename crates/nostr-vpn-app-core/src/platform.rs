use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePlatform {
    Desktop,
    Android,
    Ios,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRuntimeCapabilities {
    pub platform: String,
    pub mobile: bool,
    pub vpn_session_control_supported: bool,
    pub cli_install_supported: bool,
    pub startup_settings_supported: bool,
    pub tray_behavior_supported: bool,
    pub runtime_status_detail: String,
}

#[must_use]
pub const fn current_runtime_platform() -> RuntimePlatform {
    if cfg!(target_os = "android") {
        RuntimePlatform::Android
    } else if cfg!(target_os = "ios") {
        RuntimePlatform::Ios
    } else {
        RuntimePlatform::Desktop
    }
}

#[must_use]
pub fn current_runtime_capabilities() -> NativeRuntimeCapabilities {
    runtime_capabilities_for(
        current_runtime_platform(),
        cfg!(all(target_os = "ios", target_abi = "sim")),
    )
}

#[must_use]
pub fn runtime_capabilities_for(
    platform: RuntimePlatform,
    ios_simulator: bool,
) -> NativeRuntimeCapabilities {
    match platform {
        RuntimePlatform::Desktop => NativeRuntimeCapabilities {
            platform: "desktop".to_string(),
            mobile: false,
            vpn_session_control_supported: true,
            cli_install_supported: true,
            startup_settings_supported: true,
            tray_behavior_supported: true,
            runtime_status_detail: String::new(),
        },
        RuntimePlatform::Android => NativeRuntimeCapabilities {
            platform: "android".to_string(),
            mobile: true,
            vpn_session_control_supported: true,
            cli_install_supported: false,
            startup_settings_supported: false,
            tray_behavior_supported: false,
            runtime_status_detail: "Android native VPN control is available; desktop service management is unavailable.".to_string(),
        },
        RuntimePlatform::Ios => NativeRuntimeCapabilities {
            platform: "ios".to_string(),
            mobile: true,
            vpn_session_control_supported: !ios_simulator,
            cli_install_supported: false,
            startup_settings_supported: false,
            tray_behavior_supported: false,
            runtime_status_detail: if ios_simulator {
                "iOS Simulator does not provide NetworkExtension VPN control; use a physical device for Packet Tunnel testing."
            } else {
                "iOS Packet Tunnel integration is available; desktop service management is unavailable."
            }
            .to_string(),
        },
    }
}

#[must_use]
pub fn runtime_capabilities_json(platform: &str, ios_simulator: bool) -> String {
    let platform = match platform {
        "android" => RuntimePlatform::Android,
        "ios" | "iphone" => RuntimePlatform::Ios,
        _ => RuntimePlatform::Desktop,
    };
    serde_json::to_string(&runtime_capabilities_for(platform, ios_simulator))
        .unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_capabilities_enable_desktop_shell_features() {
        let capabilities = runtime_capabilities_for(RuntimePlatform::Desktop, false);

        assert!(!capabilities.mobile);
        assert!(capabilities.cli_install_supported);
        assert!(capabilities.startup_settings_supported);
        assert!(capabilities.tray_behavior_supported);
    }

    #[test]
    fn ios_simulator_disables_vpn_session_control() {
        let capabilities = runtime_capabilities_for(RuntimePlatform::Ios, true);

        assert!(capabilities.mobile);
        assert!(!capabilities.vpn_session_control_supported);
        assert!(capabilities.runtime_status_detail.contains("iOS Simulator"));
    }
}
