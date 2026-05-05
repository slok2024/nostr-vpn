use serde::{Deserialize, Serialize};

use crate::state::SettingsPatch;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum NativeAppAction {
    GetState,
    Tick,
    ConnectSession,
    DisconnectSession,
    InstallCli,
    UninstallCli,
    InstallSystemService,
    UninstallSystemService,
    EnableSystemService,
    DisableSystemService,
    AddNetwork {
        name: String,
    },
    RenameNetwork {
        network_id: String,
        name: String,
    },
    RemoveNetwork {
        network_id: String,
    },
    SetNetworkMeshId {
        network_id: String,
        mesh_id: String,
    },
    SetNetworkEnabled {
        network_id: String,
        enabled: bool,
    },
    SetNetworkJoinRequestsEnabled {
        network_id: String,
        enabled: bool,
    },
    RequestNetworkJoin {
        network_id: String,
    },
    AddParticipant {
        network_id: String,
        npub: String,
        alias: Option<String>,
    },
    AddAdmin {
        network_id: String,
        npub: String,
    },
    ImportNetworkInvite {
        invite: String,
    },
    StartLanPairing,
    StopLanPairing,
    RemoveParticipant {
        network_id: String,
        npub: String,
    },
    RemoveAdmin {
        network_id: String,
        npub: String,
    },
    AcceptJoinRequest {
        network_id: String,
        requester_npub: String,
    },
    SetParticipantAlias {
        npub: String,
        alias: String,
    },
    AddRelay {
        relay: String,
    },
    RemoveRelay {
        relay: String,
    },
    UpdateSettings {
        patch: SettingsPatch,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeAppActionDescriptor {
    pub name: &'static str,
    pub arguments: &'static [&'static str],
}

pub const ACTION_DESCRIPTORS: &[NativeAppActionDescriptor] = &[
    NativeAppActionDescriptor {
        name: "get_state",
        arguments: &[],
    },
    NativeAppActionDescriptor {
        name: "tick",
        arguments: &[],
    },
    NativeAppActionDescriptor {
        name: "connect_session",
        arguments: &[],
    },
    NativeAppActionDescriptor {
        name: "disconnect_session",
        arguments: &[],
    },
    NativeAppActionDescriptor {
        name: "install_cli",
        arguments: &[],
    },
    NativeAppActionDescriptor {
        name: "uninstall_cli",
        arguments: &[],
    },
    NativeAppActionDescriptor {
        name: "install_system_service",
        arguments: &[],
    },
    NativeAppActionDescriptor {
        name: "uninstall_system_service",
        arguments: &[],
    },
    NativeAppActionDescriptor {
        name: "enable_system_service",
        arguments: &[],
    },
    NativeAppActionDescriptor {
        name: "disable_system_service",
        arguments: &[],
    },
    NativeAppActionDescriptor {
        name: "add_network",
        arguments: &["name"],
    },
    NativeAppActionDescriptor {
        name: "rename_network",
        arguments: &["networkId", "name"],
    },
    NativeAppActionDescriptor {
        name: "remove_network",
        arguments: &["networkId"],
    },
    NativeAppActionDescriptor {
        name: "set_network_mesh_id",
        arguments: &["networkId", "meshId"],
    },
    NativeAppActionDescriptor {
        name: "set_network_enabled",
        arguments: &["networkId", "enabled"],
    },
    NativeAppActionDescriptor {
        name: "set_network_join_requests_enabled",
        arguments: &["networkId", "enabled"],
    },
    NativeAppActionDescriptor {
        name: "request_network_join",
        arguments: &["networkId"],
    },
    NativeAppActionDescriptor {
        name: "add_participant",
        arguments: &["networkId", "npub", "alias"],
    },
    NativeAppActionDescriptor {
        name: "add_admin",
        arguments: &["networkId", "npub"],
    },
    NativeAppActionDescriptor {
        name: "import_network_invite",
        arguments: &["invite"],
    },
    NativeAppActionDescriptor {
        name: "start_lan_pairing",
        arguments: &[],
    },
    NativeAppActionDescriptor {
        name: "stop_lan_pairing",
        arguments: &[],
    },
    NativeAppActionDescriptor {
        name: "remove_participant",
        arguments: &["networkId", "npub"],
    },
    NativeAppActionDescriptor {
        name: "remove_admin",
        arguments: &["networkId", "npub"],
    },
    NativeAppActionDescriptor {
        name: "accept_join_request",
        arguments: &["networkId", "requesterNpub"],
    },
    NativeAppActionDescriptor {
        name: "set_participant_alias",
        arguments: &["npub", "alias"],
    },
    NativeAppActionDescriptor {
        name: "add_relay",
        arguments: &["relay"],
    },
    NativeAppActionDescriptor {
        name: "remove_relay",
        arguments: &["relay"],
    },
    NativeAppActionDescriptor {
        name: "update_settings",
        arguments: &["patch"],
    },
];

#[must_use]
pub const fn action_descriptors() -> &'static [NativeAppActionDescriptor] {
    ACTION_DESCRIPTORS
}

#[must_use]
pub fn action_descriptors_json() -> String {
    serde_json::to_string(action_descriptors()).unwrap_or_else(|_| "[]".to_string())
}

#[must_use]
pub fn validate_action_json(action_json: &str) -> bool {
    serde_json::from_str::<NativeAppAction>(action_json).is_ok()
}

#[must_use]
pub fn normalize_action_json(action_json: &str) -> String {
    match serde_json::from_str::<NativeAppAction>(action_json) {
        Ok(action) => serde_json::to_string(&action).unwrap_or_default(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_json_uses_tauri_command_names_and_camel_case_args() {
        let action = NativeAppAction::SetNetworkEnabled {
            network_id: "net-1".to_string(),
            enabled: true,
        };

        let encoded = serde_json::to_string(&action).expect("serialize action");
        assert_eq!(
            encoded,
            r#"{"type":"set_network_enabled","networkId":"net-1","enabled":true}"#
        );
        assert_eq!(
            serde_json::from_str::<NativeAppAction>(&encoded).expect("parse action"),
            action
        );
    }

    #[test]
    fn update_settings_action_round_trips() {
        let encoded = r#"{"type":"update_settings","patch":{"nodeName":"office","listenPort":51821,"advertiseExitNode":true}}"#;

        let action = serde_json::from_str::<NativeAppAction>(encoded).expect("parse action");
        assert_eq!(
            normalize_action_json(encoded),
            serde_json::to_string(&action).unwrap()
        );
        assert!(validate_action_json(encoded));
    }

    #[test]
    fn descriptors_include_every_serialized_action_name() {
        let names = action_descriptors()
            .iter()
            .map(|descriptor| descriptor.name)
            .collect::<std::collections::BTreeSet<_>>();

        assert!(names.contains("connect_session"));
        assert!(names.contains("set_network_mesh_id"));
        assert!(names.contains("update_settings"));
        assert_eq!(names.len(), action_descriptors().len());
    }
}
