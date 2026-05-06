using System.Drawing;
using System.Windows.Forms;
using NostrVpn.Windows.Core;
using NostrVpn.Windows.ViewModels;

namespace NostrVpn.Windows.Services;

public sealed class TrayService : IDisposable
{
    private readonly NotifyIcon _notifyIcon;
    private AppViewModel? _viewModel;
    private Action? _showWindow;
    private Action? _quit;

    public TrayService()
    {
        _notifyIcon = new NotifyIcon
        {
            Icon = LoadIcon(),
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

        _notifyIcon.Text = TrayText(_viewModel);
        _notifyIcon.ContextMenuStrip?.Dispose();
        _notifyIcon.ContextMenuStrip = BuildMenu(_viewModel);
    }

    public void Dispose()
    {
        _notifyIcon.Visible = false;
        _notifyIcon.Dispose();
    }

    private ContextMenuStrip BuildMenu(AppViewModel viewModel)
    {
        var menu = new ContextMenuStrip();
        menu.Items.Add(Item("Open Nostr VPN", (_, _) => _showWindow?.Invoke()));
        menu.Items.Add(new ToolStripSeparator());
        menu.Items.Add(Item(viewModel.State.SessionActive ? "Disconnect VPN" : "Connect VPN", async (_, _) => await viewModel.ToggleSessionAsync(), viewModel.State.VpnSessionControlSupported));
        menu.Items.Add(Item(viewModel.State.AdvertiseExitNode ? "Stop Offering Exit" : "Offer Private Exit", async (_, _) => await viewModel.SetAdvertiseExitNodeAsync(!viewModel.State.AdvertiseExitNode)));
        menu.Items.Add(new ToolStripSeparator());
        menu.Items.Add(Item("Copy This Device", (_, _) => viewModel.CopyText(viewModel.ThisDeviceCopyValue), !string.IsNullOrWhiteSpace(viewModel.ThisDeviceCopyValue)));

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
            exitNodes.DropDownItems.Add(Item("No exit node", async (_, _) => await viewModel.SetExitNodeAsync("")));
            foreach (var participant in network.Participants.Where(participant => participant.OffersExitNode))
            {
                var item = Item(DeviceName(participant), async (_, _) => await viewModel.SetExitNodeAsync(participant.Npub));
                item.Checked = viewModel.State.ExitNode == participant.Npub;
                exitNodes.DropDownItems.Add(item);
            }
            menu.Items.Add(exitNodes);
        }

        menu.Items.Add(new ToolStripSeparator());
        menu.Items.Add(Item("Refresh", async (_, _) => await viewModel.RefreshAsync()));
        menu.Items.Add(Item("Quit", (_, _) => _quit?.Invoke()));
        return menu;
    }

    private static ToolStripMenuItem Item(string text, EventHandler onClick, bool enabled = true)
    {
        var item = new ToolStripMenuItem(text) { Enabled = enabled };
        item.Click += onClick;
        return item;
    }

    private static Icon LoadIcon()
    {
        var iconPath = Path.Combine(AppContext.BaseDirectory, "Assets", "nostr-vpn.ico");
        return File.Exists(iconPath) ? new Icon(iconPath) : SystemIcons.Application;
    }

    private static string TrayText(AppViewModel viewModel)
    {
        var status = viewModel.State.SessionActive ? "Connected" : "Disconnected";
        return $"Nostr VPN - {status}";
    }

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
