using System.Windows;
using System.Windows.Controls;
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

    private async void AcceptJoin_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: NativeInboundJoinRequestState request })
        {
            await ViewModel.AcceptJoinRequestAsync(request);
        }
    }

    private async void JoinLanPeer_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: string invite })
        {
            ViewModel.InviteInput = invite;
            await Task.Delay(1);
            ViewModel.ImportInviteCommand.Execute(null);
        }
    }

    private async void DirectExit_Click(object sender, RoutedEventArgs e)
    {
        await ViewModel.SetExitNodeAsync("");
    }

    private async void SetExitNode_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: string npub })
        {
            await ViewModel.SetExitNodeAsync(npub);
        }
    }

    private async void AdvertiseExit_Click(object sender, RoutedEventArgs e)
    {
        if (sender is CheckBox checkBox)
        {
            await ViewModel.SetAdvertiseExitNodeAsync(checkBox.IsChecked == true);
        }
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
            await ViewModel.RemoveNetworkAsync(networkId);
        }
    }

    private async void RemoveRelay_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: string relay })
        {
            await ViewModel.RemoveRelayAsync(relay);
        }
    }
}
