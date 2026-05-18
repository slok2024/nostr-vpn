namespace NostrVpn.Windows.Core;

public static class NativeActions
{
    public static string Tick() => AppCoreClient.Action(new { type = "tick" });
    public static string ConnectVpn() => AppCoreClient.Action(new { type = "connect_vpn" });
    public static string DisconnectVpn() => AppCoreClient.Action(new { type = "disconnect_vpn" });
    public static string InstallCli() => AppCoreClient.Action(new { type = "install_cli" });
    public static string InstallSystemService() => AppCoreClient.Action(new { type = "install_system_service" });
    public static string EnableSystemService() => AppCoreClient.Action(new { type = "enable_system_service" });
    public static string DisableSystemService() => AppCoreClient.Action(new { type = "disable_system_service" });
    public static string UninstallSystemService() => AppCoreClient.Action(new { type = "uninstall_system_service" });
    public static string AddNetwork(string name) => AppCoreClient.Action(new { type = "add_network", name });
    public static string ManualAddNetwork(string adminNpub, string meshNetworkId) => AppCoreClient.Action(new { type = "manual_add_network", adminNpub, meshNetworkId });
    public static string RenameNetwork(string networkId, string name) => AppCoreClient.Action(new { type = "rename_network", networkId, name });
    public static string RemoveNetwork(string networkId) => AppCoreClient.Action(new { type = "remove_network", networkId });
    public static string SetNetworkMeshId(string networkId, string meshId) => AppCoreClient.Action(new { type = "set_network_mesh_id", networkId, meshId });
    public static string SetNetworkEnabled(string networkId, bool enabled) => AppCoreClient.Action(new { type = "set_network_enabled", networkId, enabled });
    public static string SetNetworkJoinRequestsEnabled(string networkId, bool enabled) => AppCoreClient.Action(new { type = "set_network_join_requests_enabled", networkId, enabled });
    public static string RequestNetworkJoin(string networkId) => AppCoreClient.Action(new { type = "request_network_join", networkId });
    public static string AcceptJoinRequest(string networkId, string requesterNpub) => AppCoreClient.Action(new { type = "accept_join_request", networkId, requesterNpub });
    public static string RejectJoinRequest(string networkId, string requesterNpub) => AppCoreClient.Action(new { type = "reject_join_request", networkId, requesterNpub });
    public static string ImportNetworkInvite(string invite) => AppCoreClient.Action(new { type = "import_network_invite", invite });
    public static string StartInviteBroadcast() => AppCoreClient.Action(new { type = "start_invite_broadcast" });
    public static string StopInviteBroadcast() => AppCoreClient.Action(new { type = "stop_invite_broadcast" });
    public static string StartNearbyDiscovery() => AppCoreClient.Action(new { type = "start_nearby_discovery" });
    public static string StopNearbyDiscovery() => AppCoreClient.Action(new { type = "stop_nearby_discovery" });
    public static string AddParticipant(string networkId, string npub, string? alias) => AppCoreClient.Action(new { type = "add_participant", networkId, npub, alias });
    public static string RemoveParticipant(string networkId, string npub) => AppCoreClient.Action(new { type = "remove_participant", networkId, npub });
    public static string AddAdmin(string networkId, string npub) => AppCoreClient.Action(new { type = "add_admin", networkId, npub });
    public static string RemoveAdmin(string networkId, string npub) => AppCoreClient.Action(new { type = "remove_admin", networkId, npub });
    public static string SetParticipantAlias(string npub, string alias) => AppCoreClient.Action(new { type = "set_participant_alias", npub, alias });
    public static string UpdateSettings(SettingsPatch patch) => AppCoreClient.Action(new { type = "update_settings", patch });
}

public sealed class SettingsPatch
{
    public string? NodeName { get; set; }
    public string? Endpoint { get; set; }
    public string? TunnelIp { get; set; }
    public ushort? ListenPort { get; set; }
    public List<string>? Relays { get; set; }
    public List<string>? DisabledRelays { get; set; }
    public string? ExitNode { get; set; }
    public bool? ExitNodeLeakProtection { get; set; }
    public bool? AdvertiseExitNode { get; set; }
    public string? AdvertisedRoutes { get; set; }
    public bool? WireguardExitEnabled { get; set; }
    public string? WireguardExitInterface { get; set; }
    public string? WireguardExitAddress { get; set; }
    public string? WireguardExitPrivateKey { get; set; }
    public string? WireguardExitPeerPublicKey { get; set; }
    public string? WireguardExitPeerPresharedKey { get; set; }
    public string? WireguardExitEndpoint { get; set; }
    public string? WireguardExitAllowedIps { get; set; }
    public string? WireguardExitDns { get; set; }
    public ushort? WireguardExitMtu { get; set; }
    public ushort? WireguardExitPersistentKeepaliveSecs { get; set; }
    public string? WireguardExitConfig { get; set; }
    public string? MagicDnsSuffix { get; set; }
    public bool? Autoconnect { get; set; }
    public bool? LaunchOnStartup { get; set; }
    public bool? CloseToTrayOnClose { get; set; }
}
