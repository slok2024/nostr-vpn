use serde::{Deserialize, Serialize};

use crate::state::SettingsPatch;

#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
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
    /// Manual pairing: the joiner enters the admin's Device ID + mesh
    /// network id from out-of-band. We just add a local network with the
    /// admin seeded as participant + admin and let mesh discovery converge
    /// once the admin adds us back. No join request is queued — both sides
    /// are expected to add each other directly.
    ManualAddNetwork {
        admin_npub: String,
        mesh_network_id: String,
    },
    /// Start broadcasting our active-network invite over LAN multicast/broadcast.
    StartInviteBroadcast,
    StopInviteBroadcast,
    /// Start listening for nearby invites (populates `lan_peers`).
    StartNearbyDiscovery,
    StopNearbyDiscovery,
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
        let encoded = r#"{"type":"update_settings","patch":{"nodeName":"office","listenPort":51821,"exitNodeLeakProtection":true,"advertiseExitNode":true,"wireguardExitEnabled":true,"wireguardExitEndpoint":"198.51.100.20:51830","wireguardExitConfig":"[Interface]\nPrivateKey = client\nAddress = 10.0.0.2/32\n\n[Peer]\nPublicKey = peer\nAllowedIPs = 0.0.0.0/0\nEndpoint = vpn.example.test:51820"}}"#;

        let action = serde_json::from_str::<NativeAppAction>(encoded).expect("parse action");
        match action {
            NativeAppAction::UpdateSettings { patch } => {
                assert_eq!(patch.node_name.as_deref(), Some("office"));
                assert_eq!(patch.listen_port, Some(51821));
                assert_eq!(patch.exit_node_leak_protection, Some(true));
                assert_eq!(patch.advertise_exit_node, Some(true));
                assert_eq!(patch.wireguard_exit_enabled, Some(true));
                assert_eq!(
                    patch.wireguard_exit_endpoint.as_deref(),
                    Some("198.51.100.20:51830")
                );
                assert!(
                    patch
                        .wireguard_exit_config
                        .as_deref()
                        .is_some_and(|config| config.contains("[Interface]"))
                );
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }
}
