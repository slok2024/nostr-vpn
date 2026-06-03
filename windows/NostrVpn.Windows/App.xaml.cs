using System.Windows;
using System.Reflection;
using System.Text.Json;
using NostrVpn.Windows.Services;
using NostrVpn.Windows.ViewModels;

namespace NostrVpn.Windows;

public partial class App : System.Windows.Application
{
    private SingleInstanceService? _singleInstance;
    private AppViewModel? _viewModel;
    private MainWindow? _window;
    private TrayService? _tray;

    public static bool IsQuitting { get; private set; }

    protected override void OnStartup(System.Windows.StartupEventArgs e)
    {
        _singleInstance = SingleInstanceService.ClaimOrSignal(e.Args);
        if (_singleInstance is null)
        {
            Shutdown();
            return;
        }

        base.OnStartup(e);
        if (e.Args.Contains("--nvpn-e2e-update-check", StringComparer.OrdinalIgnoreCase))
        {
            _ = RunUpdateE2EAsync(e.Args);
            return;
        }

        _viewModel = new AppViewModel();
        _window = new MainWindow(_viewModel);
        _tray = new TrayService();
        _tray.Attach(_viewModel, ShowMainWindow, Quit);
        _singleInstance.Start(args => Dispatcher.Invoke(() => HandleLaunchArgs(args, forceShow: true)));

        HandleLaunchArgs(e.Args, forceShow: false);

        if (!e.Args.Contains("--hidden", StringComparer.OrdinalIgnoreCase))
        {
            ShowMainWindow();
        }
    }

    private async Task RunUpdateE2EAsync(string[] args)
    {
        var resultPath = Environment.GetEnvironmentVariable("NVPN_UPDATE_E2E_RESULT_PATH");
        var install = args.Contains("--nvpn-e2e-install-update", StringComparer.OrdinalIgnoreCase);
        var ok = false;
        object result;
        try
        {
            var service = new UpdateService();
            var currentVersion = Environment.GetEnvironmentVariable("NVPN_UPDATE_E2E_CURRENT_VERSION");
            if (string.IsNullOrWhiteSpace(currentVersion))
            {
                currentVersion = Assembly.GetExecutingAssembly().GetName().Version?.ToString(3) ?? "0.0.0";
            }
            var check = await service.CheckAsync(currentVersion);
            string? downloadedPath = null;
            long? downloadedBytes = null;
            if (install)
            {
                if (!check.UseCoreDownload)
                {
                    throw new InvalidOperationException("no Windows update asset selected");
                }
                downloadedPath = await service.DownloadWithCoreAsync(currentVersion);
                downloadedBytes = new FileInfo(downloadedPath).Length;
            }
            ok = true;
            result = new
            {
                ok = true,
                platform = "windows",
                available = check.Available,
                tag = check.Tag,
                assetName = check.AssetName,
                assetUrl = check.AssetUrl?.ToString(),
                downloadedPath,
                downloadedBytes
            };
        }
        catch (Exception error)
        {
            result = new
            {
                ok = false,
                platform = "windows",
                error = error.Message
            };
        }

        var json = JsonSerializer.Serialize(result, new JsonSerializerOptions { WriteIndented = true });
        if (!string.IsNullOrWhiteSpace(resultPath))
        {
            var parent = Path.GetDirectoryName(resultPath);
            if (!string.IsNullOrWhiteSpace(parent))
            {
                Directory.CreateDirectory(parent);
            }
            await File.WriteAllTextAsync(resultPath, json);
        }
        else
        {
            Console.WriteLine(json);
        }
        Shutdown(ok ? 0 : 1);
    }

    protected override void OnExit(System.Windows.ExitEventArgs e)
    {
        _singleInstance?.Dispose();
        _tray?.Dispose();
        _viewModel?.Dispose();
        base.OnExit(e);
    }

    private void HandleLaunchArgs(IEnumerable<string> args, bool forceShow)
    {
        var launchArgs = args.ToArray();
        foreach (var arg in launchArgs.Where(arg => arg.StartsWith("nvpn://", StringComparison.OrdinalIgnoreCase)))
        {
            _viewModel?.HandleDeepLink(arg);
        }

        if (forceShow
            && !launchArgs.Contains("--hidden", StringComparer.OrdinalIgnoreCase))
        {
            ShowMainWindow();
        }
    }

    private void ShowMainWindow()
    {
        _window ??= new MainWindow(_viewModel ?? new AppViewModel());
        if (_window.WindowState == System.Windows.WindowState.Minimized)
        {
            _window.WindowState = System.Windows.WindowState.Normal;
        }
        _window.Show();
        _window.Activate();
    }

    private void Quit()
    {
        IsQuitting = true;
        Shutdown();
    }
}
