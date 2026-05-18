namespace NostrVpn.Windows.Core;

public sealed class NativeAppState
{
    public ulong Rev { get; set; }
    public string Platform { get; set; } = "";
    public bool Mobile { get; set; }
    public bool VpnControlSupported { get; set; }
    public bool CliInstallSupported { get; set; }
    public bool StartupSettingsSupported { get; set; }
    public bool TrayBehaviorSupported { get; set; }
    public string RuntimeStatusDetail { get; set; } = "";
    public string AppVersion { get; set; } = "";
    public string ConfigPath { get; set; } = "";
    public string Error { get; set; } = "";
    public bool CliInstalled { get; set; }
    public bool ServiceSupported { get; set; }
    public bool ServiceEnablementSupported { get; set; }
    public bool ServiceInstalled { get; set; }
    public bool ServiceDisabled { get; set; }
    public bool ServiceRunning { get; set; }
    public string ServiceStatusDetail { get; set; } = "";
    public bool DaemonRunning { get; set; }
    public bool VpnEnabled { get; set; }
    public bool VpnActive { get; set; }
    public string VpnStatus { get; set; } = "";
    public string DaemonBinaryVersion { get; set; } = "";
    public string ServiceBinaryVersion { get; set; } = "";
    public string OwnNpub { get; set; } = "";
    public string OwnPubkeyHex { get; set; } = "";
    public string NodeId { get; set; } = "";
    public string NodeName { get; set; } = "";
    public string SelfMagicDnsName { get; set; } = "";
    public string Endpoint { get; set; } = "";
    public string TunnelIp { get; set; } = "";
    public uint ListenPort { get; set; }
    public string NetworkId { get; set; } = "";
    public string ActiveNetworkInvite { get; set; } = "";
    public string ExitNode { get; set; } = "";
    public bool ExitNodeLeakProtection { get; set; }
    public bool ExitNodeActive { get; set; }
    public bool ExitNodeBlocked { get; set; }
    public string ExitNodeStatusText { get; set; } = "";
    public bool AdvertiseExitNode { get; set; }
    public List<string> AdvertisedRoutes { get; set; } = [];
    public List<string> EffectiveAdvertisedRoutes { get; set; } = [];
    public bool WireguardExitEnabled { get; set; }
    public bool WireguardExitConfigured { get; set; }
    public string WireguardExitInterface { get; set; } = "";
    public string WireguardExitAddress { get; set; } = "";
    public string WireguardExitPrivateKey { get; set; } = "";
    public string WireguardExitPeerPublicKey { get; set; } = "";
    public string WireguardExitPeerPresharedKey { get; set; } = "";
    public string WireguardExitEndpoint { get; set; } = "";
    public string WireguardExitAllowedIps { get; set; } = "";
    public string WireguardExitDns { get; set; } = "";
    public ushort WireguardExitMtu { get; set; }
    public ushort WireguardExitPersistentKeepaliveSecs { get; set; }
    public string WireguardExitConfig { get; set; } = "";
    public string MagicDnsSuffix { get; set; } = "";
    public string MagicDnsStatus { get; set; } = "";
    public bool Autoconnect { get; set; }
    public bool InviteBroadcastActive { get; set; }
    public ulong InviteBroadcastRemainingSecs { get; set; }
    public bool NearbyDiscoveryActive { get; set; }
    public ulong NearbyDiscoveryRemainingSecs { get; set; }
    public bool LaunchOnStartup { get; set; }
    public bool CloseToTrayOnClose { get; set; }
    public ulong ConnectedPeerCount { get; set; }
    public ulong ExpectedPeerCount { get; set; }
    public bool MeshReady { get; set; }
    public List<NativeHealthIssue> Health { get; set; } = [];
    public NativeNetworkSummary Network { get; set; } = new();
    public NativePortMappingStatus PortMapping { get; set; } = new();
    public List<NativeNetworkState> Networks { get; set; } = [];
    public List<NativeLanPeerState> LanPeers { get; set; } = [];
}

public sealed class NativeNetworkState
{
    public string Id { get; set; } = "";
    public string Name { get; set; } = "";
    public bool Enabled { get; set; }
    public string NetworkId { get; set; } = "";
    public bool LocalIsAdmin { get; set; }
    public bool JoinRequestsEnabled { get; set; }
    public string InviteInviterNpub { get; set; } = "";
    public List<string> AdminNpubs { get; set; } = [];
    public NativeOutboundJoinRequestState? OutboundJoinRequest { get; set; }
    public List<NativeInboundJoinRequestState> InboundJoinRequests { get; set; } = [];
    public ulong OnlineCount { get; set; }
    public ulong ExpectedCount { get; set; }
    public List<string> Admins { get; set; } = [];
    public List<NativeParticipantState> Participants { get; set; } = [];
}

public sealed class NativeParticipantState
{
    public string Npub { get; set; } = "";
    public string PubkeyHex { get; set; } = "";
    public string Alias { get; set; } = "";
    public string MagicDnsAlias { get; set; } = "";
    public string MagicDnsName { get; set; } = "";
    public string TunnelIp { get; set; } = "";
    public bool IsAdmin { get; set; }
    public bool Reachable { get; set; }
    public ulong TxBytes { get; set; }
    public ulong RxBytes { get; set; }
    public List<string> AdvertisedRoutes { get; set; } = [];
    public bool OffersExitNode { get; set; }
    public string FipsEndpointNpub { get; set; } = "";
    public string FipsTransportAddr { get; set; } = "";
    public string FipsTransportType { get; set; } = "";
    public ulong FipsSrttMs { get; set; }
    public ulong FipsPacketsSent { get; set; }
    public ulong FipsPacketsRecv { get; set; }
    public ulong FipsBytesSent { get; set; }
    public ulong FipsBytesRecv { get; set; }
    public string State { get; set; } = "";
    public string MeshState { get; set; } = "";
    public string StatusText { get; set; } = "";
    public string LastSeenText { get; set; } = "";
    [System.Text.Json.Serialization.JsonIgnore]
    public bool IsSelf { get; set; }
    public string DisplayName => FirstNonEmpty(
        MagicDnsName,
        Alias,
        MagicDnsAlias,
        ShortText(Npub, 12, 6));
    public string CleanTunnelIp => TunnelIp.Split('/')[0].Trim();
    public string MagicDnsDisplay => FirstNonEmpty(MagicDnsName, MagicDnsAlias, "-");
    public string LastSeenDisplay => string.IsNullOrWhiteSpace(LastSeenText) ? "-" : LastSeenText;
    public string TxBytesDisplay => FormatBytes(TxBytes);
    public string RxBytesDisplay => FormatBytes(RxBytes);
    public string RoleText
    {
        get
        {
            var roles = new List<string>();
            if (IsSelf)
            {
                roles.Add("This device");
            }
            if (IsAdmin)
            {
                roles.Add("Admin");
            }
            if (OffersExitNode)
            {
                roles.Add("Exit node");
            }
            return roles.Count == 0 ? "Member" : string.Join(", ", roles);
        }
    }
    public string ConnectivityStateText
    {
        get
        {
            return State.ToLowerInvariant() switch
            {
                "off" => "Off",
                "local" or "online" or "present" => "Online",
                "pending" => "Connecting",
                "offline" => "Offline",
                _ when Reachable => "Online",
                _ => "Unknown",
            };
        }
    }
    public string StatusDetailText => string.IsNullOrWhiteSpace(StatusText) ? ConnectivityStateText : StatusText;
    public bool IsFipsDirect => Reachable
        && !string.Equals(State, "local", StringComparison.OrdinalIgnoreCase)
        && !string.IsNullOrWhiteSpace(FipsTransportAddr);
    public bool IsFipsRouted => Reachable
        && !string.Equals(State, "local", StringComparison.OrdinalIgnoreCase)
        && string.IsNullOrWhiteSpace(FipsTransportAddr);
    public string FipsPathText
    {
        get
        {
            if (string.Equals(State, "local", StringComparison.OrdinalIgnoreCase))
            {
                return "This device";
            }
            if (IsFipsDirect)
            {
                var transport = string.IsNullOrWhiteSpace(FipsTransportType) ? "" : $" ({FipsTransportType.ToUpperInvariant()})";
                return FipsSrttMs > 0 ? $"Direct connection{transport}, {FipsSrttMs} ms" : $"Direct connection{transport}";
            }
            if (IsFipsRouted)
            {
                return FipsSrttMs > 0 ? $"Via mesh, {FipsSrttMs} ms" : "Via mesh";
            }
            if (string.Equals(State, "pending", StringComparison.OrdinalIgnoreCase))
            {
                return "Connecting";
            }
            return "Offline";
        }
    }

    private static string FirstNonEmpty(params string[] values)
    {
        foreach (var value in values)
        {
            if (!string.IsNullOrWhiteSpace(value))
            {
                return value;
            }
        }
        return "";
    }

    private static string ShortText(string value, int prefix, int suffix)
    {
        if (value.Length <= prefix + suffix + 3)
        {
            return value;
        }
        return $"{value[..prefix]}...{value[^suffix..]}";
    }

    private static string FormatBytes(ulong bytes)
    {
        string[] units = ["B", "KB", "MB", "GB", "TB"];
        var value = (double)bytes;
        var unitIndex = 0;
        while (value >= 1024 && unitIndex < units.Length - 1)
        {
            value /= 1024;
            unitIndex++;
        }
        return unitIndex == 0 ? $"{bytes} B" : $"{value:0.0} {units[unitIndex]}";
    }
}

public sealed class NativeOutboundJoinRequestState
{
    public string RecipientNpub { get; set; } = "";
    public string RecipientPubkeyHex { get; set; } = "";
    public string RequestedAtText { get; set; } = "";
}

public sealed class NativeInboundJoinRequestState
{
    public string RequesterNpub { get; set; } = "";
    public string RequesterPubkeyHex { get; set; } = "";
    public string RequesterNodeName { get; set; } = "";
    public string RequestedAtText { get; set; } = "";
}

public sealed class NativeLanPeerState
{
    public string Npub { get; set; } = "";
    public string NodeName { get; set; } = "";
    public string Endpoint { get; set; } = "";
    public string NetworkName { get; set; } = "";
    public string NetworkId { get; set; } = "";
    public string Invite { get; set; } = "";
    public string LastSeenText { get; set; } = "";
}

public sealed class NativeHealthIssue
{
    public string Code { get; set; } = "";
    public string Severity { get; set; } = "";
    public string Summary { get; set; } = "";
    public string Detail { get; set; } = "";
}

public sealed class NativeNetworkSummary
{
    public string DefaultInterface { get; set; } = "";
    public string PrimaryIpv4 { get; set; } = "";
    public string PrimaryIpv6 { get; set; } = "";
    public string GatewayIpv4 { get; set; } = "";
    public string GatewayIpv6 { get; set; } = "";
    public ulong ChangedAt { get; set; }
    public string CaptivePortal { get; set; } = "";
}

public sealed class NativeProbeStatus
{
    public string State { get; set; } = "";
    public string Detail { get; set; } = "";
}

public sealed class NativePortMappingStatus
{
    public NativeProbeStatus Upnp { get; set; } = new();
    public NativeProbeStatus NatPmp { get; set; } = new();
    public NativeProbeStatus Pcp { get; set; } = new();
    public string ActiveProtocol { get; set; } = "";
    public string ExternalEndpoint { get; set; } = "";
    public string Gateway { get; set; } = "";
    public ulong GoodUntil { get; set; }
}

public sealed class QrMatrix
{
    public int Width { get; set; }
    public List<bool> Cells { get; set; } = [];
    public string Error { get; set; } = "";
}

public sealed class QrDecodeResult
{
    public string Value { get; set; } = "";
    public string Error { get; set; } = "";
}
