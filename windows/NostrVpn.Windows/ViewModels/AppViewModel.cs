using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Diagnostics;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Windows;
using System.Windows.Input;
using System.Windows.Threading;
using Microsoft.Win32;
using NostrVpn.Windows.Core;
using NostrVpn.Windows.Services;

namespace NostrVpn.Windows.ViewModels;

public enum AppPage
{
    Devices,
    Share,
    ExitNodes,
    Settings,
}

public sealed class AppViewModel : INotifyPropertyChanged, IDisposable
{
    private readonly AppCoreClient _core;
    private readonly DispatcherTimer _refreshTimer;
    private readonly UpdateService _updateService = new();
    private NativeAppState _state = new();
    private AppPage _page = AppPage.Devices;
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
    private string _magicDnsSuffix = "";
    private string _advertisedRoutes = "";
    private string _updateStatus = "";
    private bool _updateChecking;
    private bool _updateAvailable;
    private string _updateVersion = "";
    private QrMatrix _inviteQr = new();

    public AppViewModel()
    {
        var version = Assembly.GetExecutingAssembly().GetName().Version?.ToString(3) ?? "";
        var dataDir = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "Nostr VPN");
        _core = new AppCoreClient(dataDir, version);
        ApplyState(_core.State(), syncDrafts: true);

        ShowDevicesCommand = new RelayCommand(_ => Page = AppPage.Devices);
        ShowShareCommand = new RelayCommand(_ => Page = AppPage.Share);
        ShowExitNodesCommand = new RelayCommand(_ => Page = AppPage.ExitNodes);
        ShowSettingsCommand = new RelayCommand(_ => Page = AppPage.Settings);
        RefreshCommand = new AsyncRelayCommand(_ => RefreshAsync(), _ => !ActionInFlight);
        ToggleVpnCommand = new AsyncRelayCommand(_ => ToggleVpnAsync(), _ => !ActionInFlight && State.VpnControlSupported);
        CopyInviteCommand = new RelayCommand(_ => CopyText(State.ActiveNetworkInvite));
        CopyThisDeviceCommand = new RelayCommand(_ => CopyText(ThisDeviceCopyValue), _ => !string.IsNullOrWhiteSpace(ThisDeviceCopyValue));
        CopyPeerCommand = new RelayCommand(parameter => CopyText(parameter as string ?? ""));
        ImportInviteCommand = new AsyncRelayCommand(_ => ImportInviteAsync(InviteInput), _ => !ActionInFlight && !string.IsNullOrWhiteSpace(InviteInput));
        ImportQrImageCommand = new AsyncRelayCommand(_ => ImportQrImageAsync(), _ => !ActionInFlight);
        ToggleLanPairingCommand = new AsyncRelayCommand(_ => DispatchAsync(State.LanPairingActive ? NativeActions.StopLanPairing() : NativeActions.StartLanPairing(), "Pairing"));
        AddParticipantCommand = new AsyncRelayCommand(_ => AddParticipantAsync(), _ => !ActionInFlight && ActiveNetwork?.LocalIsAdmin == true && !string.IsNullOrWhiteSpace(ParticipantInput));
        SaveNodeCommand = new AsyncRelayCommand(_ => SaveNodeAsync(), _ => !ActionInFlight);
        AddNetworkCommand = new AsyncRelayCommand(_ => AddNetworkAsync(), _ => !ActionInFlight && !string.IsNullOrWhiteSpace(NetworkNameInput));
        SaveNetworkNameCommand = new AsyncRelayCommand(_ => RenameActiveNetworkAsync(), _ => !ActionInFlight && ActiveNetwork?.LocalIsAdmin == true && !string.IsNullOrWhiteSpace(NetworkNameDraft));
        SaveNetworkMeshIdCommand = new AsyncRelayCommand(_ => SaveActiveNetworkMeshIdAsync(), _ => !ActionInFlight && ActiveNetwork?.LocalIsAdmin == true && !string.IsNullOrWhiteSpace(NetworkMeshIdDraft));
        CopyNetworkIdCommand = new RelayCommand(_ => CopyText(ActiveNetwork?.NetworkId ?? ""), _ => !string.IsNullOrWhiteSpace(ActiveNetwork?.NetworkId));
        RequestNetworkJoinCommand = new AsyncRelayCommand(_ => RequestActiveNetworkJoinAsync(), _ => !ActionInFlight && CanRequestActiveNetworkJoin);
        InstallServiceCommand = new AsyncRelayCommand(_ => DispatchAsync(NativeActions.InstallSystemService(), "Installing service"), _ => !ActionInFlight && State.ServiceSupported);
        EnableServiceCommand = new AsyncRelayCommand(_ => DispatchAsync(NativeActions.EnableSystemService(), "Enabling service"), _ => !ActionInFlight && State.ServiceEnablementSupported);
        DisableServiceCommand = new AsyncRelayCommand(_ => DispatchAsync(NativeActions.DisableSystemService(), "Disabling service"), _ => !ActionInFlight && State.ServiceEnablementSupported);
        InstallCliCommand = new AsyncRelayCommand(_ => DispatchAsync(NativeActions.InstallCli(), "Installing CLI"), _ => !ActionInFlight && State.CliInstallSupported);
        CheckUpdatesCommand = new AsyncRelayCommand(_ => CheckUpdatesAsync(), _ => !UpdateChecking);

        StartupService.RegisterDeepLinkProtocol();

        _refreshTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(2) };
        _refreshTimer.Tick += async (_, _) => await RefreshAsync();
        _refreshTimer.Start();
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

    public string InviteInput { get => _inviteInput; set => SetField(ref _inviteInput, value); }
    public string ParticipantInput { get => _participantInput; set => SetField(ref _participantInput, value); }
    public string ParticipantAliasInput { get => _participantAliasInput; set => SetField(ref _participantAliasInput, value); }
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
    public string MagicDnsSuffix { get => _magicDnsSuffix; set => SetField(ref _magicDnsSuffix, value); }
    public string AdvertisedRoutes { get => _advertisedRoutes; set => SetField(ref _advertisedRoutes, value); }

    public bool UpdateChecking
    {
        get => _updateChecking;
        private set => SetField(ref _updateChecking, value);
    }

    public bool UpdateAvailable
    {
        get => _updateAvailable;
        private set => SetField(ref _updateAvailable, value);
    }

    public string UpdateVersion
    {
        get => _updateVersion;
        private set => SetField(ref _updateVersion, value);
    }

    public string UpdateStatus
    {
        get => _updateStatus;
        private set => SetField(ref _updateStatus, value);
    }

    public QrMatrix InviteQr
    {
        get => _inviteQr;
        private set => SetField(ref _inviteQr, value);
    }

    public NativeNetworkState? ActiveNetwork => State.Networks.FirstOrDefault(network => network.Enabled) ?? State.Networks.FirstOrDefault();
    public IEnumerable<NativeNetworkState> InactiveNetworks => State.Networks.Where(network => !network.Enabled);
    public string ActiveNetworkName => DisplayNetworkName(ActiveNetwork);
    public string HeroSubtitle => $"{State.ConnectedPeerCount} of {State.ExpectedPeerCount} connected";
    public string VpnButtonText => State.VpnEnabled ? "On" : "Off";
    public string VpnStatusText => string.IsNullOrWhiteSpace(State.Error) ? State.VpnStatus : State.Error;
    public string ThisDeviceCopyValue => !string.IsNullOrWhiteSpace(State.OwnNpub) ? State.OwnNpub : State.TunnelIp;
    public string LanPairingText => State.LanPairingActive ? $"{State.LanPairingRemainingSecs}s" : "Pair nearby";
    public string ServiceSummary => State.ServiceInstalled ? "Service installed" : "Service missing";
    public string CliSummary => State.CliInstalled ? "CLI installed" : "CLI missing";
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
    public ICommand ShowShareCommand { get; }
    public ICommand ShowExitNodesCommand { get; }
    public ICommand ShowSettingsCommand { get; }
    public ICommand RefreshCommand { get; }
    public ICommand ToggleVpnCommand { get; }
    public ICommand CopyInviteCommand { get; }
    public ICommand CopyThisDeviceCommand { get; }
    public ICommand CopyPeerCommand { get; }
    public ICommand ImportInviteCommand { get; }
    public ICommand ImportQrImageCommand { get; }
    public ICommand ToggleLanPairingCommand { get; }
    public ICommand AddParticipantCommand { get; }
    public ICommand SaveNodeCommand { get; }
    public ICommand AddNetworkCommand { get; }
    public ICommand SaveNetworkNameCommand { get; }
    public ICommand SaveNetworkMeshIdCommand { get; }
    public ICommand CopyNetworkIdCommand { get; }
    public ICommand RequestNetworkJoinCommand { get; }
    public ICommand InstallServiceCommand { get; }
    public ICommand EnableServiceCommand { get; }
    public ICommand DisableServiceCommand { get; }
    public ICommand InstallCliCommand { get; }
    public ICommand CheckUpdatesCommand { get; }

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

    public Task SetExitNodeAsync(string npub)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { ExitNode = npub }),
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
        return network?.LocalIsAdmin == true
            ? DispatchAsync(NativeActions.RemoveParticipant(network.Id, participant.Npub), "Removing device")
            : Task.CompletedTask;
    }

    public Task ToggleAdminAsync(NativeParticipantState participant)
    {
        var network = ActiveNetwork;
        if (network?.LocalIsAdmin != true)
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

    public async Task CheckUpdatesAsync()
    {
        UpdateChecking = true;
        UpdateStatus = "Checking for updates";
        try
        {
            var result = await _updateService.CheckAsync(State.AppVersion);
            UpdateAvailable = result.Available;
            UpdateVersion = result.Tag;
            UpdateStatus = result.Message;
            if (result.Available && result.AssetUrl is not null && !UpdateService.SkipOpen)
            {
                _ = Process.Start(new ProcessStartInfo(result.AssetUrl.ToString()) { UseShellExecute = true });
            }
        }
        catch (Exception error)
        {
            UpdateStatus = error.Message;
        }
        finally
        {
            UpdateChecking = false;
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
        _core.Dispose();
    }

    private async Task DispatchAsync(string actionJson, string status)
    {
        if (ActionInFlight)
        {
            return;
        }
        ActionInFlight = true;
        Notice = status;
        try
        {
            var state = await Task.Run(() => _core.Dispatch(actionJson));
            ApplyState(state, syncDrafts: true);
            Notice = string.IsNullOrWhiteSpace(state.Error) ? "" : state.Error;
        }
        catch (Exception error)
        {
            Notice = error.Message;
        }
        finally
        {
            ActionInFlight = false;
        }
    }

    private Task ImportInviteAsync(string invite)
    {
        var trimmed = invite.Trim();
        return string.IsNullOrEmpty(trimmed)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.ImportNetworkInvite(trimmed), "Importing invite");
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

    private void ApplyState(NativeAppState state, bool syncDrafts)
    {
        State = state;
        InviteQr = _core.QrMatrix(state.ActiveNetworkInvite);
        if (syncDrafts)
        {
            SyncDrafts(state);
        }
        CommandManager.InvalidateRequerySuggested();
    }

    private void SyncDrafts(NativeAppState state)
    {
        var active = state.Networks.FirstOrDefault(network => network.Enabled) ?? state.Networks.FirstOrDefault();
        NodeName = state.NodeName;
        Endpoint = state.Endpoint;
        TunnelIp = state.TunnelIp;
        ListenPort = state.ListenPort.ToString();
        MagicDnsSuffix = state.MagicDnsSuffix;
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
        OnPropertyChanged(nameof(ActiveNetwork));
        OnPropertyChanged(nameof(InactiveNetworks));
        OnPropertyChanged(nameof(ActiveNetworkName));
        OnPropertyChanged(nameof(HeroSubtitle));
        OnPropertyChanged(nameof(VpnButtonText));
        OnPropertyChanged(nameof(VpnStatusText));
        OnPropertyChanged(nameof(ThisDeviceCopyValue));
        OnPropertyChanged(nameof(LanPairingText));
        OnPropertyChanged(nameof(ServiceSummary));
        OnPropertyChanged(nameof(CliSummary));
        OnPropertyChanged(nameof(DiagnosticsInterface));
        OnPropertyChanged(nameof(DiagnosticsIpv4));
        OnPropertyChanged(nameof(DiagnosticsIpv6));
        OnPropertyChanged(nameof(DiagnosticsGateway));
        OnPropertyChanged(nameof(DiagnosticsMapping));
        OnPropertyChanged(nameof(DiagnosticsExternal));
        OnPropertyChanged(nameof(CanRequestActiveNetworkJoin));
        OnPropertyChanged(nameof(ActiveNetworkJoinStatus));
    }

    private static string FirstNonEmpty(string first, string second, string fallback)
    {
        if (!string.IsNullOrWhiteSpace(first))
        {
            return first;
        }
        return string.IsNullOrWhiteSpace(second) ? fallback : second;
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
