namespace NostrVpn.Windows.Core;

public sealed class NativeAppState
{
    public ulong Rev { get; set; }
    public string Platform { get; set; } = "";
    public bool Mobile { get; set; }
    public bool VpnSessionControlSupported { get; set; }
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
    public bool SessionActive { get; set; }
    public bool RelayConnected { get; set; }
    public string SessionStatus { get; set; } = "";
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
    public bool AdvertiseExitNode { get; set; }
    public List<string> AdvertisedRoutes { get; set; } = [];
    public List<string> EffectiveAdvertisedRoutes { get; set; } = [];
    public string MagicDnsSuffix { get; set; } = "";
    public string MagicDnsStatus { get; set; } = "";
    public bool Autoconnect { get; set; }
    public bool LanPairingActive { get; set; }
    public ulong LanPairingRemainingSecs { get; set; }
    public bool LaunchOnStartup { get; set; }
    public bool CloseToTrayOnClose { get; set; }
    public ulong ConnectedPeerCount { get; set; }
    public ulong ExpectedPeerCount { get; set; }
    public bool MeshReady { get; set; }
    public List<NativeHealthIssue> Health { get; set; } = [];
    public NativeNetworkSummary Network { get; set; } = new();
    public NativePortMappingStatus PortMapping { get; set; } = new();
    public List<NativeNetworkState> Networks { get; set; } = [];
    public List<NativeRelayState> Relays { get; set; } = [];
    public NativeRelaySummary RelaySummary { get; set; } = new();
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
    public string State { get; set; } = "";
    public string PresenceState { get; set; } = "";
    public string StatusText { get; set; } = "";
    public string LastSignalText { get; set; } = "";
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

public sealed class NativeRelayState
{
    public string Url { get; set; } = "";
    public string State { get; set; } = "";
    public string StatusText { get; set; } = "";
}

public sealed class NativeRelaySummary
{
    public ulong Up { get; set; }
    public ulong Down { get; set; }
    public ulong Checking { get; set; }
    public ulong Unknown { get; set; }
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
