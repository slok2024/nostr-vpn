using NostrVpn.Windows.Core;

namespace NostrVpn.Windows.Services;

public sealed class UpdateService
{
    public static bool SkipOpen => Environment.GetEnvironmentVariable("NVPN_UPDATE_SKIP_OPEN") == "1";

    public async Task<UpdateResult> CheckAsync(string currentVersion)
    {
        var update = await Task.Run(() => AppCoreClient.CheckUpdate(currentVersion));
        var assetUrl = string.IsNullOrWhiteSpace(update.Url) ? null : new Uri(update.Url);
        var hasAsset = !string.IsNullOrWhiteSpace(update.Asset);

        return new UpdateResult(
            update.Available,
            update.Tag,
            assetUrl,
            string.IsNullOrWhiteSpace(update.Asset) ? null : update.Asset,
            update.Available
                ? hasAsset ? $"Update {update.Tag} available" : $"Update {update.Tag} found without a Windows asset"
                : "Up to date",
            UseCoreDownload: update.Available && hasAsset);
    }

    public async Task<string> DownloadWithCoreAsync(string currentVersion)
    {
        var downloadDir = Environment.GetEnvironmentVariable("NVPN_UPDATE_DOWNLOAD_DIR");
        if (string.IsNullOrWhiteSpace(downloadDir))
        {
            downloadDir = Path.Combine(Path.GetTempPath(), "NostrVpnDownloads");
        }
        Directory.CreateDirectory(downloadDir);

        var update = await Task.Run(() => AppCoreClient.DownloadUpdate(currentVersion, downloadDir));
        if (string.IsNullOrWhiteSpace(update.Path))
        {
            throw new InvalidOperationException("updater did not return a downloaded file");
        }
        return update.Path;
    }
}

public sealed record UpdateResult(
    bool Available,
    string Tag,
    Uri? AssetUrl,
    string? AssetName,
    string Message,
    bool UseCoreDownload);
