use serde::{Deserialize, Serialize};

use crate::state::SettingsPatch;

#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum NativeAppAction {
    GetState,
    Tick,
    ConnectVpn,
    DisconnectVpn,
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
    UpdateSettings {
        patch: SettingsPatch,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_json_uses_current_names() {
        assert_eq!(
            serde_json::to_string(&NativeAppAction::ConnectVpn).expect("serialize action"),
            r#"{"type":"connect_vpn"}"#
        );

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
        match action {
            NativeAppAction::UpdateSettings { patch } => {
                assert_eq!(patch.node_name.as_deref(), Some("office"));
                assert_eq!(patch.listen_port, Some(51821));
                assert_eq!(patch.advertise_exit_node, Some(true));
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }
}
