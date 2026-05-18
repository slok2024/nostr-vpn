using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Diagnostics;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Windows;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;
using Microsoft.Win32;
using NostrVpn.Windows.Core;
using NostrVpn.Windows.Services;

namespace NostrVpn.Windows.ViewModels;

public enum AppPage
{
    Devices,
    AddNetwork,
    AddDevice,
    ExitNodes,
    Settings,
}

public sealed class AppViewModel : INotifyPropertyChanged, IDisposable
{
    private readonly AppCoreClient _core;
    private readonly DispatcherTimer _refreshTimer;
    private readonly DispatcherTimer _updateTimer;
    private readonly UpdateService _updateService = new();
    private NativeAppState _state = new();
    private AppPage _page = AppPage.Devices;
    private string _selectedParticipantKey = "";
    private string _shownNetworkId = "";
    private bool _actionInFlight;
    private string _notice = "";
    private string _inviteInput = "";
    private string _participantInput = "";
    private string _participantAliasInput = "";
    private string _networkNameInput = "";
    private string _networkNameDraft = "";
    private string _networkMeshIdDraft = "";
    private string _nodeName = "";
    private string _endpoint = "";
    private string _tunnelIp = "";
    private string _listenPort = "";
    private string _relayInput = "";
    private string _relaysDraft = "";
    private string _magicDnsSuffix = "";
    private string _advertisedRoutes = "";
    private string _wireguardExitConfig = "";
    private string _manualJoinAdminId = "";
    private string _manualJoinMeshId = "";
    private bool _manualJoinExpanded;
    private string _updateStatus = "";
    private Uri? _updateAssetUrl;
    private bool _updateChecking;
    private bool _updateInstalling;
    private bool _updateAvailable;
    private bool _autoInstallUpdates;
    private string _updateVersion = "";
    private QrMatrix _inviteQr = new();
    private static readonly TimeSpan UpdatePollInterval = LoadUpdatePollInterval();
    private static readonly Brush HeaderDangerBrush = new SolidColorBrush(Color.FromRgb(220, 38, 38));
    private static readonly Brush TextSecondaryBrush = new SolidColorBrush(Color.FromRgb(104, 113, 124));
    private static readonly Brush ActiveNetworkBrush = new SolidColorBrush(Color.FromRgb(22, 163, 74));
    private static readonly Brush InactiveNetworkBrush = new SolidColorBrush(Color.FromRgb(156, 163, 175));

    public AppViewModel()
    {
        var version = Assembly.GetExecutingAssembly().GetName().Version?.ToString(3) ?? "";
        var dataDir = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "Nostr VPN");
        _core = new AppCoreClient(dataDir, version);
        _autoInstallUpdates = LoadAutoInstallUpdates();
        ApplyState(_core.State(), syncDrafts: true);

        ShowDevicesCommand = new RelayCommand(_ => Page = AppPage.Devices);
        ShowAddNetworkCommand = new RelayCommand(_ => Page = AppPage.AddNetwork);
        ShowAddDeviceCommand = new RelayCommand(
            _ => Page = AppPage.AddDevice,
            _ => ActiveNetwork?.LocalIsAdmin == true);
        ShowExitNodesCommand = new RelayCommand(_ => Page = AppPage.ExitNodes);
        ShowSettingsCommand = new RelayCommand(_ => Page = AppPage.Settings);
        RefreshCommand = new AsyncRelayCommand(_ => RefreshAsync(), _ => !ActionInFlight);
        ToggleVpnCommand = new AsyncRelayCommand(_ => ToggleVpnAsync(), _ => !ActionInFlight && State.VpnControlSupported && HasActiveNetwork);
        CopyInviteCommand = new RelayCommand(_ => CopyText(State.ActiveNetworkInvite));
        CopyThisDeviceCommand = new RelayCommand(_ => CopyText(ThisDeviceCopyValue), _ => !string.IsNullOrWhiteSpace(ThisDeviceCopyValue));
        CopyPeerCommand = new RelayCommand(parameter => CopyText(parameter as string ?? ""));
        ImportInviteCommand = new AsyncRelayCommand(_ => ImportInviteAsync(InviteInput), _ => !ActionInFlight && !string.IsNullOrWhiteSpace(InviteInput));
        PasteInviteCommand = new RelayCommand(_ => PasteInviteFromClipboard(), _ => !ActionInFlight);
        ImportQrImageCommand = new AsyncRelayCommand(_ => ImportQrImageAsync(), _ => !ActionInFlight);
        ToggleInviteBroadcastCommand = new AsyncRelayCommand(_ => DispatchAsync(State.InviteBroadcastActive ? NativeActions.StopInviteBroadcast() : NativeActions.StartInviteBroadcast(), "Broadcasting invite"));
        ToggleNearbyDiscoveryCommand = new AsyncRelayCommand(_ => DispatchAsync(State.NearbyDiscoveryActive ? NativeActions.StopNearbyDiscovery() : NativeActions.StartNearbyDiscovery(), "Looking for nearby"));
        AddParticipantCommand = new AsyncRelayCommand(_ => AddParticipantAsync(), _ => !ActionInFlight && ActiveNetwork?.LocalIsAdmin == true && !string.IsNullOrWhiteSpace(ParticipantInput) && !ParticipantInputInvalid);
        SaveNodeCommand = new AsyncRelayCommand(_ => SaveNodeAsync(), _ => !ActionInFlight);
        AddRelayCommand = new AsyncRelayCommand(_ => AddRelayAsync(), _ => !ActionInFlight && !string.IsNullOrWhiteSpace(RelayInput));
        SaveRelaysCommand = new AsyncRelayCommand(_ => SaveRelaysAsync(), _ => !ActionInFlight);
        SaveWireGuardExitCommand = new AsyncRelayCommand(_ => SaveWireGuardExitAsync(), _ => !ActionInFlight);
        CreateNetworkCommand = new AsyncRelayCommand(_ => CreateNetworkAsync(), _ => !ActionInFlight);
        AddNetworkCommand = new AsyncRelayCommand(_ => AddNetworkAsync(), _ => !ActionInFlight && !string.IsNullOrWhiteSpace(NetworkNameInput));
        ManualAddNetworkCommand = new AsyncRelayCommand(
            _ => ManualAddNetworkAsync(),
            _ => !ActionInFlight && CanSubmitManualJoin);
        ToggleManualJoinCommand = new RelayCommand(_ => ManualJoinExpanded = !ManualJoinExpanded);
        ActivateInactiveNetworkCommand = new AsyncRelayCommand(
            parameter => ActivateInactiveNetworkAsync(parameter as string),
            parameter => !ActionInFlight && parameter is string id && !string.IsNullOrWhiteSpace(id));
        SaveNetworkNameCommand = new AsyncRelayCommand(_ => RenameActiveNetworkAsync(), _ => !ActionInFlight && ActiveNetwork?.LocalIsAdmin == true && !string.IsNullOrWhiteSpace(NetworkNameDraft));
        SaveNetworkMeshIdCommand = new AsyncRelayCommand(_ => SaveActiveNetworkMeshIdAsync(), _ => !ActionInFlight && ActiveNetwork?.LocalIsAdmin == true && !string.IsNullOrWhiteSpace(NetworkMeshIdDraft));
        CopyNetworkIdCommand = new RelayCommand(_ => CopyText(ActiveNetwork?.NetworkId ?? ""), _ => !string.IsNullOrWhiteSpace(ActiveNetwork?.NetworkId));
        RequestNetworkJoinCommand = new AsyncRelayCommand(_ => RequestActiveNetworkJoinAsync(), _ => !ActionInFlight && CanRequestActiveNetworkJoin);
        InstallServiceCommand = new AsyncRelayCommand(_ => DispatchAsync(NativeActions.InstallSystemService(), "Installing service"), _ => !ActionInFlight && State.ServiceSupported);
        EnableServiceCommand = new AsyncRelayCommand(_ => DispatchAsync(NativeActions.EnableSystemService(), "Enabling service"), _ => !ActionInFlight && State.ServiceEnablementSupported);
        DisableServiceCommand = new AsyncRelayCommand(_ => DispatchAsync(NativeActions.DisableSystemService(), "Disabling service"), _ => !ActionInFlight && State.ServiceEnablementSupported);
        InstallCliCommand = new AsyncRelayCommand(_ => DispatchAsync(NativeActions.InstallCli(), "Installing CLI"), _ => !ActionInFlight && State.CliInstallSupported);
        CheckUpdatesCommand = new AsyncRelayCommand(_ => CheckUpdatesAsync(), _ => !UpdateChecking && !UpdateInstalling);
        InstallUpdateCommand = new AsyncRelayCommand(_ => InstallUpdateAsync(), _ => UpdateInstallEnabled);

        StartupService.RegisterDeepLinkProtocol();

        _refreshTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(2) };
        _refreshTimer.Tick += async (_, _) => await RefreshAsync();
        _refreshTimer.Start();
        _ = CheckUpdatesAsync(manual: false);

        _updateTimer = new DispatcherTimer { Interval = UpdatePollInterval };
        _updateTimer.Tick += async (_, _) => await CheckUpdatesAsync(manual: false);
        _updateTimer.Start();
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public NativeAppState State
    {
        get => _state;
        private set
        {
            _state = value;
            OnPropertyChanged();
            RaiseDerivedStateChanged();
        }
    }

    public AppPage Page
    {
        get => _page;
        set
        {
            if (_page == value)
            {
                return;
            }
            _page = value;
            OnPropertyChanged();
        }
    }

    public bool ActionInFlight
    {
        get => _actionInFlight;
        private set
        {
            _actionInFlight = value;
            OnPropertyChanged();
            CommandManager.InvalidateRequerySuggested();
        }
    }

    public string Notice
    {
        get => _notice;
        private set
        {
            _notice = value;
            OnPropertyChanged();
        }
    }

    public string InviteInput
    {
        get => _inviteInput;
        set
        {
            if (!SetField(ref _inviteInput, value))
            {
                return;
            }
            CommandManager.InvalidateRequerySuggested();
            // Auto-import as soon as the field contains a recognisable invite —
            // saves the user a click and matches "paste and you're in" mental
            // model. We only fire on full nvpn:// URLs so partial typing
            // doesn't trigger; the import itself clears the field.
            var trimmed = (value ?? string.Empty).Trim();
            if (!ActionInFlight && LooksLikeInviteCode(trimmed))
            {
                _ = ImportInviteAsync(trimmed);
            }
        }
    }
    public string ParticipantInput
    {
        get => _participantInput;
        set
        {
            if (SetField(ref _participantInput, value))
            {
                OnPropertyChanged(nameof(ParticipantInputInvalid));
                OnPropertyChanged(nameof(ParticipantInputErrorVisibility));
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }
    public string ParticipantAliasInput { get => _participantAliasInput; set => SetField(ref _participantAliasInput, value); }
    public bool ParticipantInputInvalid
    {
        get
        {
            var trimmed = (_participantInput ?? string.Empty).Trim();
            return trimmed.Length > 0 && !IsValidDeviceId(trimmed);
        }
    }
    public Visibility ParticipantInputErrorVisibility => ParticipantInputInvalid ? Visibility.Visible : Visibility.Collapsed;

    public string ManualJoinAdminId
    {
        get => _manualJoinAdminId;
        set
        {
            if (SetField(ref _manualJoinAdminId, value))
            {
                OnPropertyChanged(nameof(ManualJoinAdminInvalid));
                OnPropertyChanged(nameof(ManualJoinAdminErrorVisibility));
                OnPropertyChanged(nameof(CanSubmitManualJoin));
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }

    public string ManualJoinMeshId
    {
        get => _manualJoinMeshId;
        set
        {
            if (SetField(ref _manualJoinMeshId, value))
            {
                OnPropertyChanged(nameof(CanSubmitManualJoin));
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }

    public bool ManualJoinExpanded
    {
        get => _manualJoinExpanded;
        set
        {
            if (SetField(ref _manualJoinExpanded, value))
            {
                OnPropertyChanged(nameof(ManualJoinExpanderToggleText));
            }
        }
    }

    public string ManualJoinExpanderToggleText => ManualJoinExpanded ? "Add manually ▴" : "Add manually ▾";

    public bool ManualJoinAdminInvalid
    {
        get
        {
            var trimmed = (_manualJoinAdminId ?? string.Empty).Trim();
            return trimmed.Length > 0 && !IsValidDeviceId(trimmed);
        }
    }

    public Visibility ManualJoinAdminErrorVisibility => ManualJoinAdminInvalid ? Visibility.Visible : Visibility.Collapsed;

    public bool CanSubmitManualJoin
    {
        get
        {
            var admin = (_manualJoinAdminId ?? string.Empty).Trim();
            var mesh = (_manualJoinMeshId ?? string.Empty).Trim();
            return admin.Length > 0 && mesh.Length > 0 && !ManualJoinAdminInvalid;
        }
    }

    public string NetworkNameInput { get => _networkNameInput; set => SetField(ref _networkNameInput, value); }
    public string NetworkNameDraft
    {
        get => _networkNameDraft;
        set
        {
            if (SetField(ref _networkNameDraft, value))
            {
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }
    public string NetworkMeshIdDraft
    {
        get => _networkMeshIdDraft;
        set
        {
            if (SetField(ref _networkMeshIdDraft, value))
            {
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }
    public string NodeName { get => _nodeName; set => SetField(ref _nodeName, value); }
    public string Endpoint { get => _endpoint; set => SetField(ref _endpoint, value); }
    public string TunnelIp { get => _tunnelIp; set => SetField(ref _tunnelIp, value); }
    public string ListenPort { get => _listenPort; set => SetField(ref _listenPort, value); }
    public string RelayInput
    {
        get => _relayInput;
        set
        {
            if (SetField(ref _relayInput, value))
            {
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }
    public string RelaysDraft { get => _relaysDraft; set => SetField(ref _relaysDraft, value); }
    public string MagicDnsSuffix { get => _magicDnsSuffix; set => SetField(ref _magicDnsSuffix, value); }
    public string AdvertisedRoutes { get => _advertisedRoutes; set => SetField(ref _advertisedRoutes, value); }
    public string WireguardExitConfig { get => _wireguardExitConfig; set => SetField(ref _wireguardExitConfig, value); }

    // Bullet-style radio indicators next to each exit-node row.
    public string DirectExitMarker =>
        (!State.WireguardExitEnabled && string.IsNullOrEmpty(State.ExitNode)) ? "●" : "○";

    public string WireguardExitMarker => State.WireguardExitEnabled ? "●" : "○";

    public string WireguardExitSubtitle
    {
        get
        {
            if (!State.WireguardExitConfigured)
            {
                return "No WireGuard config saved yet";
            }
            return string.IsNullOrWhiteSpace(State.WireguardExitEndpoint)
                ? "Configured"
                : State.WireguardExitEndpoint;
        }
    }

    public bool UpdateChecking
    {
        get => _updateChecking;
        private set
        {
            if (SetField(ref _updateChecking, value))
            {
                OnPropertyChanged(nameof(UpdateInstallEnabled));
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }

    public bool UpdateInstalling
    {
        get => _updateInstalling;
        private set
        {
            if (SetField(ref _updateInstalling, value))
            {
                OnPropertyChanged(nameof(UpdateInstallEnabled));
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }

    public bool UpdateAvailable
    {
        get => _updateAvailable;
        private set
        {
            if (SetField(ref _updateAvailable, value))
            {
                OnPropertyChanged(nameof(UpdateStripeText));
                OnPropertyChanged(nameof(UpdateInstallEnabled));
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }

    public string UpdateVersion
    {
        get => _updateVersion;
        private set
        {
            if (SetField(ref _updateVersion, value))
            {
                OnPropertyChanged(nameof(UpdateStripeText));
            }
        }
    }

    public string UpdateStatus
    {
        get => _updateStatus;
        private set => SetField(ref _updateStatus, value);
    }

    public bool AutoInstallUpdates
    {
        get => _autoInstallUpdates;
        set
        {
            if (!SetField(ref _autoInstallUpdates, value))
            {
                return;
            }
            SaveAutoInstallUpdates(value);
            if (value && UpdateInstallEnabled)
            {
                _ = InstallUpdateAsync();
            }
        }
    }

    public bool UpdateInstallEnabled => UpdateAvailable && _updateAssetUrl is not null && !UpdateChecking && !UpdateInstalling;

    public string UpdateStripeText => string.IsNullOrWhiteSpace(State.AppVersion)
        ? $"Update available: {UpdateVersion}"
        : $"Update available: {UpdateVersion} (you're on {State.AppVersion})";

    public QrMatrix InviteQr
    {
        get => _inviteQr;
        private set => SetField(ref _inviteQr, value);
    }

    private NativeNetworkState? RuntimeActiveNetwork => State.Networks.FirstOrDefault(network => network.Enabled) ?? State.Networks.FirstOrDefault();
    public NativeNetworkState? ActiveNetwork =>
        State.Networks.FirstOrDefault(network => network.Id == _shownNetworkId)
        ?? RuntimeActiveNetwork;
    public bool HasActiveNetwork => ActiveNetwork is not null;
    public string OfferExitNodeLabel
    {
        get
        {
            var name = string.IsNullOrWhiteSpace(ActiveNetwork?.Name) ? "this network" : ActiveNetwork!.Name;
            return $"Offer this device as an exit node in {name}";
        }
    }
    public IEnumerable<NativeNetworkState> InactiveNetworks => State.Networks.Where(network => !network.Enabled);
    public NativeParticipantState? SelectedParticipant
    {
        get
        {
            var network = ActiveNetwork;
            return network is null ? null : ResolveSelectedParticipant(network);
        }
        set
        {
            var nextKey = value is null ? "" : ParticipantKey(value);
            if (_selectedParticipantKey == nextKey)
            {
                return;
            }
            _selectedParticipantKey = nextKey;
            OnPropertyChanged();
            RaiseSelectedParticipantChanged();
        }
    }
    public bool SelectedParticipantCanRename => ActiveNetwork?.LocalIsAdmin == true
        && SelectedParticipant is not null;
    public bool SelectedParticipantCanManage => ActiveNetwork?.LocalIsAdmin == true
        && SelectedParticipant is { IsSelf: false };
    public string ActiveNetworkName => DisplayNetworkName(ActiveNetwork);
    public Brush ShownNetworkStatusBrush => ActiveNetwork?.Enabled == true ? ActiveNetworkBrush : InactiveNetworkBrush;
    public bool ShowNetworkStatusDot => State.Networks.Count > 1;
    public bool HasIncomingJoinRequests => State.Networks.Any(network => network.InboundJoinRequests.Count > 0);
    public bool ActiveNetworkHasIncomingJoinRequests => ActiveNetwork?.InboundJoinRequests.Count > 0;
    public bool ShowActiveNetworkInviteCard => ActiveNetwork?.Enabled == true;
    public string HeroSubtitle => $"{State.ConnectedPeerCount} of {State.ExpectedPeerCount} connected";
    public string VpnButtonText => State.VpnEnabled ? "On" : "Off";
    /// Mirrors `AppManager.vpnStatusText` on macOS so the header and the tray
    /// surface the same status string across platforms.
    public string VpnStatusText
    {
        get
        {
            if (!string.IsNullOrWhiteSpace(State.Error))
            {
                return State.Error;
            }
            if (State.ExitNodeBlocked)
            {
                return string.IsNullOrWhiteSpace(State.ExitNodeStatusText)
                    ? "Internet blocked"
                    : State.ExitNodeStatusText;
            }
            if (State.ExitNodeActive && !string.IsNullOrWhiteSpace(State.ExitNodeStatusText))
            {
                return State.ExitNodeStatusText;
            }
            if (State.VpnActive)
            {
                return string.IsNullOrWhiteSpace(State.VpnStatus) ? "VPN on" : State.VpnStatus;
            }
            if (State.VpnEnabled)
            {
                return string.IsNullOrWhiteSpace(State.VpnStatus) ? "Turning on" : State.VpnStatus;
            }
            return "Off";
        }
    }
    public Brush VpnStatusBrush => State.ExitNodeBlocked ? HeaderDangerBrush : TextSecondaryBrush;
    public string ThisDeviceCopyValue => !string.IsNullOrWhiteSpace(State.OwnNpub) ? State.OwnNpub : State.TunnelIp;
    public Visibility NoNearbyInvitesNoticeVisibility => State.NearbyDiscoveryActive && State.LanPeers.Count == 0
        ? Visibility.Visible
        : Visibility.Collapsed;
    public string InviteBroadcastButtonText => State.InviteBroadcastActive
        ? $"Broadcasting · {FormatRemaining(State.InviteBroadcastRemainingSecs)}"
        : "Broadcast invite";
    public string NearbyDiscoveryButtonText => State.NearbyDiscoveryActive
        ? $"Listening · {FormatRemaining(State.NearbyDiscoveryRemainingSecs)}"
        : "Look for nearby";

    private static string FormatRemaining(ulong seconds)
    {
        if (seconds == 0)
        {
            return "off";
        }
        var minutes = seconds / 60;
        if (minutes == 0)
        {
            return $"{seconds}s";
        }
        var remSecs = seconds % 60;
        return remSecs == 0 ? $"{minutes}m" : $"{minutes}m{remSecs:D2}s";
    }
    public string ServiceSummary => State.ServiceInstalled ? "Service installed" : "Service missing";
    public string CliSummary => State.CliInstalled ? "CLI installed" : "CLI missing";
    public string SystemVersionLabel
    {
        get
        {
            var app = State.AppVersion.Trim();
            var daemon = State.DaemonBinaryVersion.Trim();
            return (string.IsNullOrEmpty(app), string.IsNullOrEmpty(daemon)) switch
            {
                (true, true) => "",
                (false, true) => $"gui v{app}",
                (true, false) => $"daemon v{daemon}",
                (false, false) when app == daemon => $"v{app}",
                _ => $"gui v{app} · daemon v{daemon}",
            };
        }
    }
    public string DiagnosticsInterface => string.IsNullOrWhiteSpace(State.Network.DefaultInterface) ? "unknown" : State.Network.DefaultInterface;
    public string DiagnosticsIpv4 => string.IsNullOrWhiteSpace(State.Network.PrimaryIpv4) ? "-" : State.Network.PrimaryIpv4;
    public string DiagnosticsIpv6 => string.IsNullOrWhiteSpace(State.Network.PrimaryIpv6) ? "-" : State.Network.PrimaryIpv6;
    public string DiagnosticsGateway => FirstNonEmpty(State.Network.GatewayIpv4, State.Network.GatewayIpv6, "unknown");
    public string DiagnosticsMapping => string.IsNullOrWhiteSpace(State.PortMapping.ActiveProtocol) ? "none" : State.PortMapping.ActiveProtocol;
    public string DiagnosticsExternal => string.IsNullOrWhiteSpace(State.PortMapping.ExternalEndpoint) ? "stun/direct" : State.PortMapping.ExternalEndpoint;
    public bool CanRequestActiveNetworkJoin => ActiveNetwork is { OutboundJoinRequest: null } network && !string.IsNullOrWhiteSpace(network.InviteInviterNpub);
    public string ActiveNetworkJoinStatus
    {
        get
        {
            var network = ActiveNetwork;
            if (network?.OutboundJoinRequest is not null)
            {
                return "Join requested";
            }
            return CanRequestActiveNetworkJoin ? "Invite needs approval" : "";
        }
    }

    public ICommand ShowDevicesCommand { get; }
    public ICommand ShowAddNetworkCommand { get; }
    public ICommand ShowAddDeviceCommand { get; }
    public ICommand ShowExitNodesCommand { get; }
    public ICommand ShowSettingsCommand { get; }
    public ICommand RefreshCommand { get; }
    public ICommand ToggleVpnCommand { get; }
    public ICommand CopyInviteCommand { get; }
    public ICommand CopyThisDeviceCommand { get; }
    public ICommand CopyPeerCommand { get; }
    public ICommand ImportInviteCommand { get; }
    public ICommand PasteInviteCommand { get; }
    public ICommand ImportQrImageCommand { get; }
    public ICommand ToggleInviteBroadcastCommand { get; }
    public ICommand ToggleNearbyDiscoveryCommand { get; }
    public ICommand AddParticipantCommand { get; }
    public ICommand SaveNodeCommand { get; }
    public ICommand AddRelayCommand { get; }
    public ICommand SaveRelaysCommand { get; }
    public ICommand SaveWireGuardExitCommand { get; }
    public ICommand CreateNetworkCommand { get; }
    public ICommand AddNetworkCommand { get; }
    public ICommand ManualAddNetworkCommand { get; }
    public ICommand ToggleManualJoinCommand { get; }
    public ICommand ActivateInactiveNetworkCommand { get; }
    public ICommand SaveNetworkNameCommand { get; }
    public ICommand SaveNetworkMeshIdCommand { get; }
    public ICommand CopyNetworkIdCommand { get; }
    public ICommand RequestNetworkJoinCommand { get; }
    public ICommand InstallServiceCommand { get; }
    public ICommand EnableServiceCommand { get; }
    public ICommand DisableServiceCommand { get; }
    public ICommand InstallCliCommand { get; }
    public ICommand CheckUpdatesCommand { get; }
    public ICommand InstallUpdateCommand { get; }

    public async Task RefreshAsync()
    {
        if (ActionInFlight)
        {
            return;
        }
        try
        {
            var state = await Task.Run(_core.Refresh);
            ApplyState(state, syncDrafts: false);
        }
        catch (Exception error)
        {
            Notice = error.Message;
        }
    }

    public Task ToggleVpnAsync()
    {
        return DispatchAsync(
            State.VpnEnabled ? NativeActions.DisconnectVpn() : NativeActions.ConnectVpn(),
            State.VpnEnabled ? "Turning VPN off" : "Turning VPN on");
    }

    public Task SetAdvertiseExitNodeAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { AdvertiseExitNode = enabled }),
            "Saving routing");
    }

    public Task SetExitNodeLeakProtectionAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { ExitNodeLeakProtection = enabled }),
            "Saving route protection");
    }

    public Task SetWireGuardExitEnabledAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { WireguardExitEnabled = enabled }),
            "Saving WireGuard");
    }

    public Task SetExitNodeAsync(string npub)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { ExitNode = npub }),
            "Saving exit node");
    }

    // Exit-node selection. The daemon enforces mutual exclusion
    // between peer and WG (see settings_patch_enforces_exit_node_
    // mutual_exclusion in ffi.rs), so non-direct rows send only the
    // field they own. Direct flips both because that's the
    // "neither" state which doesn't trigger the daemon's conflict
    // resolution.

    public Task SelectDirectExitAsync()
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch
            {
                ExitNode = "",
                WireguardExitEnabled = false,
            }),
            "Saving exit node");
    }

    public Task SelectWireGuardUpstreamExitAsync()
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch
            {
                WireguardExitEnabled = true,
            }),
            "Saving exit node");
    }

    public Task SelectPeerExitAsync(string npub)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch
            {
                ExitNode = npub,
            }),
            "Saving exit node");
    }

    public Task SetLaunchOnStartupAsync(bool enabled)
    {
        try
        {
            StartupService.SetLaunchOnStartup(enabled);
        }
        catch (Exception error)
        {
            Notice = error.Message;
            return Task.CompletedTask;
        }
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { LaunchOnStartup = enabled }),
            "Saving startup");
    }

    public Task SetCloseToTrayAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { CloseToTrayOnClose = enabled }),
            "Saving tray behavior");
    }

    public Task SetAutoconnectAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { Autoconnect = enabled }),
            "Saving VPN option");
    }

    public Task RemoveParticipantAsync(NativeParticipantState participant)
    {
        var network = ActiveNetwork;
        if (network?.LocalIsAdmin != true || participant.IsSelf)
        {
            return Task.CompletedTask;
        }
        return DispatchAsync(NativeActions.RemoveParticipant(network.Id, participant.Npub), "Removing device");
    }

    public Task ToggleAdminAsync(NativeParticipantState participant)
    {
        var network = ActiveNetwork;
        if (network?.LocalIsAdmin != true || participant.IsSelf)
        {
            return Task.CompletedTask;
        }
        return DispatchAsync(
            participant.IsAdmin
                ? NativeActions.RemoveAdmin(network.Id, participant.Npub)
                : NativeActions.AddAdmin(network.Id, participant.Npub),
            participant.IsAdmin ? "Removing admin" : "Adding admin");
    }

    public Task ActivateNetworkAsync(string networkId)
    {
        return DispatchAsync(NativeActions.SetNetworkEnabled(networkId, true), "Activating network");
    }

    public void SelectShownNetwork(string networkId)
    {
        if (_shownNetworkId == networkId)
        {
            return;
        }
        _shownNetworkId = networkId;
        NormalizeSelectedParticipant(State);
        SyncDrafts(State);
        RaiseDerivedStateChanged();
        CommandManager.InvalidateRequerySuggested();
    }

    public Task RemoveNetworkAsync(string networkId)
    {
        return DispatchAsync(NativeActions.RemoveNetwork(networkId), "Deleting network");
    }

    public Task SetJoinRequestsAsync(string networkId, bool enabled)
    {
        return DispatchAsync(NativeActions.SetNetworkJoinRequestsEnabled(networkId, enabled), "Saving join requests");
    }

    public Task RenameActiveNetworkAsync()
    {
        var network = ActiveNetwork;
        var name = NetworkNameDraft.Trim();
        return network is null || string.IsNullOrWhiteSpace(name)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.RenameNetwork(network.Id, name), "Renaming network");
    }

    public Task SaveActiveNetworkMeshIdAsync()
    {
        var network = ActiveNetwork;
        var meshId = NetworkMeshIdDraft.Trim();
        return network is null || string.IsNullOrWhiteSpace(meshId)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.SetNetworkMeshId(network.Id, meshId), "Saving network ID");
    }

    public Task RequestActiveNetworkJoinAsync()
    {
        var network = ActiveNetwork;
        return network is null ? Task.CompletedTask : DispatchAsync(NativeActions.RequestNetworkJoin(network.Id), "Requesting access");
    }

    public Task AcceptJoinRequestAsync(NativeInboundJoinRequestState request)
    {
        var network = ActiveNetwork;
        return network?.LocalIsAdmin == true
            ? DispatchAsync(NativeActions.AcceptJoinRequest(network.Id, request.RequesterNpub), "Accepting join request")
            : Task.CompletedTask;
    }

    public Task RejectJoinRequestAsync(NativeInboundJoinRequestState request)
    {
        var network = ActiveNetwork;
        return network?.LocalIsAdmin == true
            ? DispatchAsync(NativeActions.RejectJoinRequest(network.Id, request.RequesterNpub), "Rejecting join request")
            : Task.CompletedTask;
    }

    public Task SetParticipantAliasAsync(NativeParticipantState participant, string alias)
    {
        return ActiveNetwork?.LocalIsAdmin == true
            ? DispatchAsync(NativeActions.SetParticipantAlias(participant.Npub, alias.Trim()), "Saving alias")
            : Task.CompletedTask;
    }

    public void CopyText(string value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return;
        }
        Clipboard.SetText(value);
        Notice = "Copied";
    }

    public async Task CheckUpdatesAsync(bool manual = true)
    {
        if (UpdateChecking || UpdateInstalling)
        {
            return;
        }
        UpdateChecking = true;
        if (manual)
        {
            UpdateStatus = "Checking for updates";
        }
        try
        {
            var result = await _updateService.CheckAsync(State.AppVersion);
            UpdateAvailable = result.Available;
            UpdateVersion = result.Tag;
            _updateAssetUrl = result.Available ? result.AssetUrl : null;
            OnPropertyChanged(nameof(UpdateInstallEnabled));
            CommandManager.InvalidateRequerySuggested();
            if (result.Available)
            {
                UpdateStatus = result.Message;
                if (AutoInstallUpdates && result.AssetUrl is not null)
                {
                    await InstallUpdateAsync();
                }
            }
            else if (manual)
            {
                UpdateStatus = result.Message;
            }
            else
            {
                UpdateStatus = "";
            }
        }
        catch (Exception error)
        {
            if (manual)
            {
                UpdateStatus = error.Message;
            }
        }
        finally
        {
            UpdateChecking = false;
        }
    }

    private async Task InstallUpdateAsync()
    {
        if (_updateAssetUrl is null || UpdateInstalling)
        {
            return;
        }
        UpdateInstalling = true;
        UpdateStatus = $"Downloading {UpdateVersion}";
        try
        {
            var path = await _updateService.DownloadAsync(_updateAssetUrl);
            UpdateStatus = $"Downloaded {Path.GetFileName(path)}";
            if (!UpdateService.SkipOpen)
            {
                _ = Process.Start(new ProcessStartInfo(path) { UseShellExecute = true });
            }
        }
        catch (Exception error)
        {
            UpdateStatus = error.Message;
        }
        finally
        {
            UpdateInstalling = false;
        }
    }

    public void HandleDeepLink(string url)
    {
        if (url.StartsWith("nvpn://invite/", StringComparison.OrdinalIgnoreCase))
        {
            _ = ImportInviteAsync(url);
        }
    }

    public void Dispose()
    {
        _refreshTimer.Stop();
        _updateTimer.Stop();
        _core.Dispose();
    }

    private async Task DispatchAsync(string actionJson, string status)
    {
        if (ActionInFlight)
        {
            return;
        }
        ActionInFlight = true;
        // Defer the in-progress notice so fast actions (broadcast/listen toggle,
        // copy, etc.) never flash the notice card. The card collapses when empty,
        // so showing it for ~50ms shifts the entire content below — that's what
        // looked like a flicker on the Share page when toggling broadcast/listen.
        using var noticeCts = new CancellationTokenSource();
        _ = Task.Delay(TimeSpan.FromMilliseconds(250), noticeCts.Token).ContinueWith(
            _ => Notice = status,
            CancellationToken.None,
            TaskContinuationOptions.OnlyOnRanToCompletion,
            TaskScheduler.FromCurrentSynchronizationContext());
        try
        {
            var state = await Task.Run(() => _core.Dispatch(actionJson));
            noticeCts.Cancel();
            ApplyState(state, syncDrafts: true);
            Notice = string.IsNullOrWhiteSpace(state.Error) ? "" : state.Error;
        }
        catch (Exception error)
        {
            noticeCts.Cancel();
            Notice = error.Message;
        }
        finally
        {
            ActionInFlight = false;
        }
    }

    private async Task ImportInviteAsync(string invite)
    {
        var trimmed = invite.Trim();
        if (string.IsNullOrEmpty(trimmed))
        {
            return;
        }
        await DispatchAsync(NativeActions.ImportNetworkInvite(trimmed), "Importing invite");
        // Always clear the paste field after a dispatch — keeps stale invites
        // from sticking around between sessions, and gives instant visual
        // feedback that the import was accepted.
        InviteInput = "";
    }

    private void PasteInviteFromClipboard()
    {
        try
        {
            if (Clipboard.ContainsText())
            {
                InviteInput = Clipboard.GetText().Trim();
            }
        }
        catch (Exception error)
        {
            Notice = error.Message;
        }
    }

    private static bool LooksLikeInviteCode(string value)
        => value.StartsWith("nvpn://invite/", StringComparison.OrdinalIgnoreCase);

    private const string Bech32BodyCharset = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

    /// <summary>
    /// A valid device ID is a bech32-encoded npub: "npub1" + 58 bech32 chars.
    /// </summary>
    public static bool IsValidDeviceId(string value)
    {
        if (string.IsNullOrWhiteSpace(value)) return false;
        var trimmed = value.Trim();
        if (trimmed.Length != 63 || !trimmed.StartsWith("npub1", StringComparison.Ordinal)) return false;
        for (var i = 5; i < trimmed.Length; i++)
        {
            if (Bech32BodyCharset.IndexOf(trimmed[i]) < 0) return false;
        }
        return true;
    }

    private async Task ImportQrImageAsync()
    {
        var dialog = new OpenFileDialog
        {
            Filter = "Images|*.png;*.jpg;*.jpeg;*.bmp;*.gif|All files|*.*",
            Multiselect = false,
        };
        if (dialog.ShowDialog() != true)
        {
            return;
        }
        var result = await Task.Run(() => _core.DecodeQrImage(dialog.FileName));
        if (!string.IsNullOrWhiteSpace(result.Error))
        {
            Notice = result.Error;
            return;
        }
        await ImportInviteAsync(result.Value);
    }

    private Task AddParticipantAsync()
    {
        var network = ActiveNetwork;
        if (network?.LocalIsAdmin != true)
        {
            return Task.CompletedTask;
        }
        return DispatchAsync(
            NativeActions.AddParticipant(network.Id, ParticipantInput.Trim(), string.IsNullOrWhiteSpace(ParticipantAliasInput) ? null : ParticipantAliasInput.Trim()),
            "Adding device");
    }

    private Task AddNetworkAsync()
    {
        return DispatchAsync(NativeActions.AddNetwork(NetworkNameInput.Trim()), "Adding network");
    }

    private async Task CreateNetworkAsync()
    {
        var name = string.IsNullOrWhiteSpace(NetworkNameInput) ? "My Network" : NetworkNameInput.Trim();
        NetworkNameInput = "";
        await DispatchAsync(NativeActions.AddNetwork(name), "Creating network");
        // Land on the new network's Devices view. Add Network may have
        // been the active page (or we may have been showing the
        // pre-network setup card); either way, Devices is the next
        // meaningful destination.
        Page = AppPage.Devices;
    }

    private async Task ManualAddNetworkAsync()
    {
        var admin = (ManualJoinAdminId ?? string.Empty).Trim();
        var mesh = (ManualJoinMeshId ?? string.Empty).Trim();
        if (admin.Length == 0 || mesh.Length == 0 || ManualJoinAdminInvalid)
        {
            return;
        }
        await DispatchAsync(NativeActions.ManualAddNetwork(admin, mesh), "Adding network");
        ManualJoinAdminId = "";
        ManualJoinMeshId = "";
        ManualJoinExpanded = false;
    }

    private Task ActivateInactiveNetworkAsync(string? networkId)
    {
        if (string.IsNullOrWhiteSpace(networkId))
        {
            return Task.CompletedTask;
        }
        return DispatchAsync(NativeActions.SetNetworkEnabled(networkId, true), "Switching network");
    }

    private Task SaveNodeAsync()
    {
        ushort? port = ushort.TryParse(ListenPort.Trim(), out var parsed) ? parsed : null;
        return DispatchAsync(NativeActions.UpdateSettings(new SettingsPatch
        {
            NodeName = NodeName,
            Endpoint = Endpoint,
            TunnelIp = TunnelIp,
            ListenPort = port,
            MagicDnsSuffix = MagicDnsSuffix,
        }), "Saving device");
    }

    private Task SaveRelaysAsync()
    {
        var relays = RelaysDraft
            .Split(new[] { '\r', '\n', ',', ' ', '\t' }, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
            .ToList();
        return DispatchAsync(NativeActions.UpdateSettings(new SettingsPatch
        {
            Relays = relays,
        }), "Saving relays");
    }

    private Task AddRelayAsync()
    {
        var url = NormalizeRelayUrl(RelayInput);
        if (url is null) return Task.CompletedTask;

        var (enabled, disabled) = RelayLists();
        disabled.RemoveAll(relay => relay == url);
        if (!enabled.Contains(url)) enabled.Add(url);
        RelayInput = "";
        return SaveRelayListsAsync(enabled, disabled);
    }

    public Task SetRelayEnabledAsync(NativeRelayState relay, bool enabledValue)
    {
        var url = NormalizeRelayUrl(relay.Url);
        if (url is null) return Task.CompletedTask;

        var (enabled, disabled) = RelayLists();
        enabled.RemoveAll(relayUrl => relayUrl == url);
        disabled.RemoveAll(relayUrl => relayUrl == url);
        if (enabledValue)
        {
            enabled.Add(url);
        }
        else
        {
            disabled.Add(url);
        }
        return SaveRelayListsAsync(enabled, disabled);
    }

    public Task RemoveRelayAsync(NativeRelayState relay)
    {
        var url = NormalizeRelayUrl(relay.Url);
        if (url is null) return Task.CompletedTask;

        var (enabled, disabled) = RelayLists();
        enabled.RemoveAll(relayUrl => relayUrl == url);
        disabled.RemoveAll(relayUrl => relayUrl == url);
        return SaveRelayListsAsync(enabled, disabled);
    }

    private (List<string> Enabled, List<string> Disabled) RelayLists()
    {
        var enabled = UniqueRelays(State.Relays.Where(relay => relay.Enabled).Select(relay => relay.Url));
        var disabled = UniqueRelays(State.Relays.Where(relay => !relay.Enabled).Select(relay => relay.Url));
        disabled.RemoveAll(relay => enabled.Contains(relay));
        return (enabled, disabled);
    }

    private Task SaveRelayListsAsync(List<string> enabledInput, List<string> disabledInput)
    {
        var enabled = UniqueRelays(enabledInput);
        var disabled = UniqueRelays(disabledInput);
        disabled.RemoveAll(relay => enabled.Contains(relay));
        return DispatchAsync(NativeActions.UpdateSettings(new SettingsPatch
        {
            Relays = enabled,
            DisabledRelays = disabled,
        }), "Saving relays");
    }

    private static string? NormalizeRelayUrl(string value)
    {
        var trimmed = value.Trim();
        if (string.IsNullOrWhiteSpace(trimmed)) return null;
        return trimmed.StartsWith("ws://", StringComparison.OrdinalIgnoreCase)
            || trimmed.StartsWith("wss://", StringComparison.OrdinalIgnoreCase)
            ? trimmed
            : $"wss://{trimmed}";
    }

    private static List<string> UniqueRelays(IEnumerable<string> values)
    {
        var seen = new HashSet<string>();
        return values
            .Select(NormalizeRelayUrl)
            .Where(relay => relay is not null && seen.Add(relay))
            .Select(relay => relay!)
            .ToList();
    }

    private Task SaveWireGuardExitAsync()
    {
        return DispatchAsync(NativeActions.UpdateSettings(new SettingsPatch
        {
            WireguardExitConfig = WireguardExitConfig,
        }), "Saving WireGuard");
    }

    private void ApplyState(NativeAppState state, bool syncDrafts)
    {
        TagSelfParticipants(state);
        NormalizeSelectedParticipant(state);
        State = state;
        InviteQr = _core.QrMatrix(state.ActiveNetworkInvite);
        if (syncDrafts)
        {
            SyncDrafts(state);
        }
        CommandManager.InvalidateRequerySuggested();
    }

    private static void TagSelfParticipants(NativeAppState state)
    {
        var ownNpub = state.OwnNpub;
        foreach (var network in state.Networks)
        {
            foreach (var participant in network.Participants)
            {
                participant.IsSelf =
                    string.Equals(participant.MeshState, "local", StringComparison.OrdinalIgnoreCase)
                    || (!string.IsNullOrEmpty(ownNpub) && participant.Npub == ownNpub);
            }
        }
    }

    private void SyncDrafts(NativeAppState state)
    {
        var active = state.Networks.FirstOrDefault(network => network.Id == _shownNetworkId)
            ?? state.Networks.FirstOrDefault(network => network.Enabled)
            ?? state.Networks.FirstOrDefault();
        NodeName = state.NodeName;
        Endpoint = state.Endpoint;
        TunnelIp = state.TunnelIp;
        ListenPort = state.ListenPort.ToString();
        RelaysDraft = string.Join(Environment.NewLine, state.Relays.Select(relay => relay.Url));
        MagicDnsSuffix = state.MagicDnsSuffix;
        WireguardExitConfig = state.WireguardExitConfig;
        NetworkNameDraft = active?.Name ?? "";
        NetworkMeshIdDraft = active?.NetworkId ?? "";
    }

    private static string DisplayNetworkName(NativeNetworkState? network)
    {
        if (network is null)
        {
            return "Nostr VPN";
        }
        return string.IsNullOrWhiteSpace(network.Name) ? "Private network" : network.Name;
    }

    private void RaiseDerivedStateChanged()
    {
        if (!string.IsNullOrWhiteSpace(_shownNetworkId)
            && State.Networks.All(network => network.Id != _shownNetworkId))
        {
            _shownNetworkId = "";
        }
        OnPropertyChanged(nameof(ActiveNetwork));
        OnPropertyChanged(nameof(HasActiveNetwork));
        OnPropertyChanged(nameof(OfferExitNodeLabel));
        OnPropertyChanged(nameof(InactiveNetworks));
        OnPropertyChanged(nameof(SelectedParticipant));
        RaiseSelectedParticipantChanged();
        OnPropertyChanged(nameof(ActiveNetworkName));
        OnPropertyChanged(nameof(ShownNetworkStatusBrush));
        OnPropertyChanged(nameof(ShowNetworkStatusDot));
        OnPropertyChanged(nameof(HasIncomingJoinRequests));
        OnPropertyChanged(nameof(ActiveNetworkHasIncomingJoinRequests));
        OnPropertyChanged(nameof(ShowActiveNetworkInviteCard));
        OnPropertyChanged(nameof(HeroSubtitle));
        OnPropertyChanged(nameof(VpnButtonText));
        OnPropertyChanged(nameof(VpnStatusText));
        OnPropertyChanged(nameof(VpnStatusBrush));
        OnPropertyChanged(nameof(UpdateStripeText));
        OnPropertyChanged(nameof(ThisDeviceCopyValue));
        OnPropertyChanged(nameof(InviteBroadcastButtonText));
        OnPropertyChanged(nameof(NearbyDiscoveryButtonText));
        OnPropertyChanged(nameof(NoNearbyInvitesNoticeVisibility));
        OnPropertyChanged(nameof(ServiceSummary));
        OnPropertyChanged(nameof(CliSummary));
        OnPropertyChanged(nameof(SystemVersionLabel));
        OnPropertyChanged(nameof(DiagnosticsInterface));
        OnPropertyChanged(nameof(DiagnosticsIpv4));
        OnPropertyChanged(nameof(DiagnosticsIpv6));
        OnPropertyChanged(nameof(DiagnosticsGateway));
        OnPropertyChanged(nameof(DiagnosticsMapping));
        OnPropertyChanged(nameof(DiagnosticsExternal));
        OnPropertyChanged(nameof(CanRequestActiveNetworkJoin));
        OnPropertyChanged(nameof(ActiveNetworkJoinStatus));
        OnPropertyChanged(nameof(DirectExitMarker));
        OnPropertyChanged(nameof(WireguardExitMarker));
        OnPropertyChanged(nameof(WireguardExitSubtitle));
    }

    private void RaiseSelectedParticipantChanged()
    {
        OnPropertyChanged(nameof(SelectedParticipantCanRename));
        OnPropertyChanged(nameof(SelectedParticipantCanManage));
    }

    private void NormalizeSelectedParticipant(NativeAppState state)
    {
        var network = state.Networks.FirstOrDefault(network => network.Id == _shownNetworkId)
            ?? state.Networks.FirstOrDefault(network => network.Enabled)
            ?? state.Networks.FirstOrDefault();
        if (network is null || network.Participants.Count == 0)
        {
            _selectedParticipantKey = "";
            return;
        }
        if (!string.IsNullOrWhiteSpace(_selectedParticipantKey)
            && network.Participants.Any(participant => ParticipantKey(participant) == _selectedParticipantKey))
        {
            return;
        }
        _selectedParticipantKey = SortedParticipants(network).FirstOrDefault() is { } first
            ? ParticipantKey(first)
            : "";
    }

    private NativeParticipantState? ResolveSelectedParticipant(NativeNetworkState network)
    {
        if (!string.IsNullOrWhiteSpace(_selectedParticipantKey))
        {
            var selected = network.Participants.FirstOrDefault(participant => ParticipantKey(participant) == _selectedParticipantKey);
            if (selected is not null)
            {
                return selected;
            }
        }
        return SortedParticipants(network).FirstOrDefault();
    }

    private static IEnumerable<NativeParticipantState> SortedParticipants(NativeNetworkState network)
        => network.Participants
            .OrderBy(participant => !participant.IsSelf)
            .ThenBy(participant => !participant.Reachable)
            .ThenBy(participant => participant.DisplayName, StringComparer.OrdinalIgnoreCase);

    private static string ParticipantKey(NativeParticipantState participant)
        => string.IsNullOrWhiteSpace(participant.PubkeyHex) ? participant.Npub : participant.PubkeyHex;

    private static string FirstNonEmpty(string first, string second, string fallback)
    {
        if (!string.IsNullOrWhiteSpace(first))
        {
            return first;
        }
        return string.IsNullOrWhiteSpace(second) ? fallback : second;
    }

    private static bool LoadAutoInstallUpdates()
    {
        using var key = Registry.CurrentUser.OpenSubKey(@"Software\Nostr VPN");
        return key?.GetValue("AutoInstallUpdates") is int value && value != 0;
    }

    private static void SaveAutoInstallUpdates(bool enabled)
    {
        using var key = Registry.CurrentUser.CreateSubKey(@"Software\Nostr VPN");
        key?.SetValue("AutoInstallUpdates", enabled ? 1 : 0, RegistryValueKind.DWord);
    }

    private static TimeSpan LoadUpdatePollInterval()
    {
        var raw = Environment.GetEnvironmentVariable("NVPN_UPDATE_POLL_SECONDS");
        return double.TryParse(raw, out var seconds) && seconds > 0
            ? TimeSpan.FromSeconds(seconds)
            : TimeSpan.FromHours(6);
    }

    private bool SetField<T>(ref T field, T value, [CallerMemberName] string propertyName = "")
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
        {
            return false;
        }
        field = value;
        OnPropertyChanged(propertyName);
        return true;
    }

    private void OnPropertyChanged([CallerMemberName] string propertyName = "")
    {
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
    }
}
