using System.Drawing;
using System.Drawing.Drawing2D;
using System.Runtime.InteropServices;
using System.Windows.Forms;
using NostrVpn.Windows.Core;
using NostrVpn.Windows.ViewModels;

namespace NostrVpn.Windows.Services;

public sealed class TrayService : IDisposable
{
    private readonly NotifyIcon _notifyIcon;
    private readonly Icon _normalIcon;
    private readonly Icon _blockedIcon;
    private AppViewModel? _viewModel;
    private Action? _showWindow;
    private Action? _quit;

    public TrayService()
    {
        _normalIcon = LoadIcon();
        _blockedIcon = CreateBlockedIcon(_normalIcon);
        _notifyIcon = new NotifyIcon
        {
            Icon = _normalIcon,
            Text = "Nostr VPN",
            Visible = true,
        };
        _notifyIcon.DoubleClick += (_, _) => _showWindow?.Invoke();
    }

    public void Attach(AppViewModel viewModel, Action showWindow, Action quit)
    {
        _viewModel = viewModel;
        _showWindow = showWindow;
        _quit = quit;
        viewModel.PropertyChanged += (_, _) => Update();
        Update();
    }

    public void Update()
    {
        if (_viewModel is null)
        {
            return;
        }

        _notifyIcon.Text = TruncateTrayText(TrayText(_viewModel));
        _notifyIcon.Icon = _viewModel.State.ExitNodeBlocked ? _blockedIcon : _normalIcon;
        _notifyIcon.ContextMenuStrip?.Dispose();
        _notifyIcon.ContextMenuStrip = BuildMenu(_viewModel);
    }

    public void Dispose()
    {
        _notifyIcon.Visible = false;
        _notifyIcon.Dispose();
        _normalIcon.Dispose();
        _blockedIcon.Dispose();
    }

    private ContextMenuStrip BuildMenu(AppViewModel viewModel)
    {
        // 1. VPN toggle (first), 2. device-name section, 3. network/exit
        // submenus, 4. open/quit. See macOS TrayController.swift for the
        // canonical layout shared across platforms.
        var menu = new ContextMenuStrip();

        var vpnToggle = Item("Nostr VPN", async (_, _) => await viewModel.ToggleVpnAsync(),
            viewModel.State.VpnControlSupported && !viewModel.ActionInFlight);
        vpnToggle.Checked = viewModel.State.VpnEnabled;
        menu.Items.Add(vpnToggle);

        menu.Items.Add(new ToolStripSeparator());

        // Device-name section header + Copy Device ID.
        menu.Items.Add(Item(DeviceDisplayName(viewModel), (_, _) => { }, false));
        var copyDeviceId = Item("Copy Device ID",
            (_, _) => viewModel.CopyText(viewModel.State.OwnNpub),
            !string.IsNullOrWhiteSpace(viewModel.State.OwnNpub));
        menu.Items.Add(copyDeviceId);

        menu.Items.Add(new ToolStripSeparator());

        var network = viewModel.ActiveNetwork;
        if (network is not null)
        {
            var devices = new ToolStripMenuItem(string.IsNullOrWhiteSpace(network.Name) ? "Network Devices" : network.Name);
            foreach (var participant in network.Participants)
            {
                devices.DropDownItems.Add(Item(ParticipantMenuTitle(participant), (_, _) => viewModel.CopyText(participant.Npub)));
            }
            menu.Items.Add(devices);

            var exitNodes = new ToolStripMenuItem("Exit Node");
            if (!string.IsNullOrWhiteSpace(viewModel.State.ExitNodeStatusText))
            {
                exitNodes.DropDownItems.Add(Item(viewModel.State.ExitNodeStatusText, (_, _) => { }, false));
            }
            var offerExit = Item("Offer This Device",
                async (_, _) => await viewModel.SetAdvertiseExitNodeAsync(!viewModel.State.AdvertiseExitNode));
            offerExit.Checked = viewModel.State.AdvertiseExitNode;
            exitNodes.DropDownItems.Add(offerExit);
            exitNodes.DropDownItems.Add(new ToolStripSeparator());
            var noExit = Item("No exit node", async (_, _) => await viewModel.SetExitNodeAsync(""));
            noExit.Checked = string.IsNullOrWhiteSpace(viewModel.State.ExitNode);
            exitNodes.DropDownItems.Add(noExit);
            foreach (var participant in network.Participants.Where(participant => participant.OffersExitNode))
            {
                var item = Item(DeviceName(participant), async (_, _) => await viewModel.SetExitNodeAsync(participant.Npub));
                item.Checked = viewModel.State.ExitNode == participant.Npub;
                exitNodes.DropDownItems.Add(item);
            }
            menu.Items.Add(exitNodes);
        }

        menu.Items.Add(new ToolStripSeparator());
        menu.Items.Add(Item("Open Nostr VPN", (_, _) => _showWindow?.Invoke()));
        menu.Items.Add(Item("Quit", (_, _) => _quit?.Invoke()));
        return menu;
    }

    private static string DeviceDisplayName(AppViewModel viewModel)
    {
        var state = viewModel.State;
        if (!string.IsNullOrWhiteSpace(state.SelfMagicDnsName))
        {
            return state.SelfMagicDnsName;
        }
        if (!string.IsNullOrWhiteSpace(state.NodeName))
        {
            return state.NodeName;
        }
        var tunnelIp = state.TunnelIp?.Trim();
        if (!string.IsNullOrEmpty(tunnelIp) && tunnelIp != "-")
        {
            return tunnelIp;
        }
        return "This Device";
    }

    private static ToolStripMenuItem Item(string text, EventHandler onClick, bool enabled = true)
    {
        var item = new ToolStripMenuItem(text) { Enabled = enabled };
        item.Click += onClick;
        return item;
    }

    private static Icon LoadIcon()
    {
        foreach (var filename in new[] { "nostr-vpn-tray.ico", "nostr-vpn.ico" })
        {
            var iconPath = Path.Combine(AppContext.BaseDirectory, "Assets", filename);
            if (File.Exists(iconPath))
            {
                return new Icon(iconPath);
            }
        }

        return (Icon)SystemIcons.Application.Clone();
    }

    private static Icon CreateBlockedIcon(Icon baseIcon)
    {
        using var bitmap = baseIcon.ToBitmap();
        using var graphics = Graphics.FromImage(bitmap);
        graphics.SmoothingMode = SmoothingMode.AntiAlias;
        var diameter = Math.Max(6, Math.Min(bitmap.Width, bitmap.Height) / 3);
        var x = bitmap.Width - diameter - 1;
        var y = 1;
        using var brush = new SolidBrush(Color.FromArgb(220, 38, 38));
        graphics.FillEllipse(brush, x, y, diameter, diameter);
        var handle = bitmap.GetHicon();
        try
        {
            return (Icon)Icon.FromHandle(handle).Clone();
        }
        finally
        {
            DestroyIcon(handle);
        }
    }

    private static string TrayText(AppViewModel viewModel)
    {
        var status = !string.IsNullOrWhiteSpace(viewModel.State.ExitNodeStatusText)
            ? viewModel.State.ExitNodeStatusText
            : viewModel.State.VpnStatus;
        return $"Nostr VPN - {status}";
    }

    private static string TruncateTrayText(string value)
    {
        return value.Length <= 63 ? value : value[..60] + "...";
    }

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool DestroyIcon(IntPtr handle);

    private static string ParticipantMenuTitle(NativeParticipantState participant)
    {
        var name = DeviceName(participant);
        return string.IsNullOrWhiteSpace(participant.TunnelIp) || participant.TunnelIp == "-"
            ? name
            : $"{name} ({participant.TunnelIp})";
    }

    private static string DeviceName(NativeParticipantState participant)
    {
        if (!string.IsNullOrWhiteSpace(participant.MagicDnsName))
        {
            return participant.MagicDnsName;
        }
        if (!string.IsNullOrWhiteSpace(participant.Alias))
        {
            return participant.Alias;
        }
        return participant.Npub.Length > 16
            ? $"{participant.Npub[..10]}...{participant.Npub[^6..]}"
            : participant.Npub;
    }
}
