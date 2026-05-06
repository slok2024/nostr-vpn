using Microsoft.Win32;

namespace NostrVpn.Windows.Services;

public static class StartupService
{
    private const string RunKeyPath = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string ProtocolKeyPath = @"Software\Classes\nvpn";
    private const string AppName = "Nostr VPN";

    public static void SetLaunchOnStartup(bool enabled)
    {
        using var key = Registry.CurrentUser.CreateSubKey(RunKeyPath);
        if (enabled)
        {
            key.SetValue(AppName, $"\"{Environment.ProcessPath}\" --autostart");
        }
        else
        {
            key.DeleteValue(AppName, throwOnMissingValue: false);
        }
    }

    public static void RegisterDeepLinkProtocol()
    {
        var exe = Environment.ProcessPath;
        if (string.IsNullOrWhiteSpace(exe))
        {
            return;
        }

        using var key = Registry.CurrentUser.CreateSubKey(ProtocolKeyPath);
        key.SetValue("", "URL:Nostr VPN");
        key.SetValue("URL Protocol", "");

        using var command = Registry.CurrentUser.CreateSubKey($@"{ProtocolKeyPath}\shell\open\command");
        command.SetValue("", $"\"{exe}\" \"%1\"");
    }
}
