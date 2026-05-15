using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using NostrVpn.Windows.Core;
using NostrVpn.Windows.ViewModels;

namespace NostrVpn.Windows;

public partial class MainWindow : Window
{
    public MainWindow(AppViewModel viewModel)
    {
        InitializeComponent();
        DataContext = viewModel;
    }

    private AppViewModel ViewModel => (AppViewModel)DataContext;

    protected override void OnClosing(System.ComponentModel.CancelEventArgs e)
    {
        if (!App.IsQuitting && ViewModel.State.CloseToTrayOnClose)
        {
            e.Cancel = true;
            Hide();
            return;
        }
        base.OnClosing(e);
    }

    private void CopyPeer_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: string npub })
        {
            ViewModel.CopyText(npub);
        }
    }

    private async void ToggleAdmin_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: NativeParticipantState participant })
        {
            await ViewModel.ToggleAdminAsync(participant);
        }
    }

    private async void RemoveParticipant_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: NativeParticipantState participant })
        {
            await ViewModel.RemoveParticipantAsync(participant);
        }
    }

    private async void SetParticipantAlias_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: NativeParticipantState participant } button
            && FindParent<Grid>(button) is { } row
            && FindChild<TextBox>(row, "AliasInput") is { } aliasInput)
        {
            await ViewModel.SetParticipantAliasAsync(participant, aliasInput.Text);
        }
    }

    private async void AcceptJoin_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: NativeInboundJoinRequestState request })
        {
            await ViewModel.AcceptJoinRequestAsync(request);
        }
    }

    private void JoinLanPeer_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: string invite })
        {
            // Setting InviteInput triggers the auto-import path in the
            // view-model, which also clears the field after dispatch.
            ViewModel.InviteInput = invite;
        }
    }

    private async void DirectExit_Click(object sender, RoutedEventArgs e)
    {
        await ViewModel.SelectDirectExitAsync();
    }

    private async void SetExitNode_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: string npub })
        {
            await ViewModel.SelectPeerExitAsync(npub);
        }
    }

    private async void AdvertiseExit_Click(object sender, RoutedEventArgs e)
    {
        if (sender is CheckBox checkBox)
        {
            await ViewModel.SetAdvertiseExitNodeAsync(checkBox.IsChecked == true);
        }
    }

    private async void ExitLeakProtection_Click(object sender, RoutedEventArgs e)
    {
        if (sender is CheckBox checkBox)
        {
            await ViewModel.SetExitNodeLeakProtectionAsync(checkBox.IsChecked == true);
        }
    }

    private async void WireguardExit_Click(object sender, RoutedEventArgs e)
    {
        await ViewModel.SelectWireGuardUpstreamExitAsync();
    }

    private async void Autoconnect_Click(object sender, RoutedEventArgs e)
    {
        if (sender is CheckBox checkBox)
        {
            await ViewModel.SetAutoconnectAsync(checkBox.IsChecked == true);
        }
    }

    private async void LaunchOnStartup_Click(object sender, RoutedEventArgs e)
    {
        if (sender is CheckBox checkBox)
        {
            await ViewModel.SetLaunchOnStartupAsync(checkBox.IsChecked == true);
        }
    }

    private async void CloseToTray_Click(object sender, RoutedEventArgs e)
    {
        if (sender is CheckBox checkBox)
        {
            await ViewModel.SetCloseToTrayAsync(checkBox.IsChecked == true);
        }
    }

    private async void JoinRequests_Click(object sender, RoutedEventArgs e)
    {
        if (sender is CheckBox checkBox && ViewModel.ActiveNetwork is { } network)
        {
            await ViewModel.SetJoinRequestsAsync(network.Id, checkBox.IsChecked == true);
        }
    }

    private async void ActivateNetwork_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: string networkId })
        {
            await ViewModel.ActivateNetworkAsync(networkId);
        }
    }

    private async void RemoveNetwork_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: string networkId })
        {
            var network = ViewModel.State.Networks.FirstOrDefault(n => n.Id == networkId);
            var name = string.IsNullOrWhiteSpace(network?.Name) ? "this network" : network!.Name;
            var result = MessageBox.Show(
                this,
                "This deletes the network from this device. You can rejoin later with the invite.",
                $"Remove {name}?",
                MessageBoxButton.OKCancel,
                MessageBoxImage.Warning,
                MessageBoxResult.Cancel);
            if (result != MessageBoxResult.OK) return;
            await ViewModel.RemoveNetworkAsync(networkId);
        }
    }

    private static T? FindParent<T>(DependencyObject child) where T : DependencyObject
    {
        var current = VisualTreeHelper.GetParent(child);
        while (current is not null)
        {
            if (current is T match)
            {
                return match;
            }
            current = VisualTreeHelper.GetParent(current);
        }
        return null;
    }

    private static T? FindChild<T>(DependencyObject parent, string name) where T : FrameworkElement
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(parent); index++)
        {
            var child = VisualTreeHelper.GetChild(parent, index);
            if (child is T element && element.Name == name)
            {
                return element;
            }
            var nested = FindChild<T>(child, name);
            if (nested is not null)
            {
                return nested;
            }
        }
        return null;
    }
}
