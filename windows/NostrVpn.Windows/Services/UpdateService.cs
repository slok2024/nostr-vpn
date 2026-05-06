using System.Net.Http;
using System.Text.Json;

namespace NostrVpn.Windows.Services;

public sealed class UpdateService
{
    private static readonly Uri ManifestUri = new("https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases/nostr-vpn/latest/release.json");
    private static readonly HttpClient Http = new();
    private static readonly JsonSerializerOptions JsonOptions = new() { PropertyNameCaseInsensitive = true };

    public async Task<UpdateResult> CheckAsync(string currentVersion)
    {
        var json = await Http.GetStringAsync(ManifestUri);
        var manifest = JsonSerializer.Deserialize<ReleaseManifest>(json, JsonOptions)
            ?? throw new InvalidOperationException("release manifest was empty");
        var asset = PreferredWindowsAsset(manifest.Assets);
        var available = VersionIsNewer(manifest.Tag, currentVersion);
        return new UpdateResult(
            available,
            manifest.Tag,
            asset?.Url is null ? null : new Uri(ManifestUri, asset.Url),
            available
                ? asset is null ? $"Update {manifest.Tag} found without a Windows asset" : $"Update {manifest.Tag} available"
                : "Up to date");
    }

    private static ReleaseAsset? PreferredWindowsAsset(IEnumerable<ReleaseAsset> assets)
    {
        var arch = Environment.GetEnvironmentVariable("PROCESSOR_ARCHITECTURE") ?? "";
        var preferred = arch.Contains("ARM64", StringComparison.OrdinalIgnoreCase)
            ? "windows-arm64-setup.exe"
            : "windows-x64-setup.exe";
        return assets.FirstOrDefault(asset => asset.Name.EndsWith(preferred, StringComparison.OrdinalIgnoreCase))
            ?? assets.FirstOrDefault(asset => asset.Name.EndsWith("windows-x64-setup.exe", StringComparison.OrdinalIgnoreCase));
    }

    private static bool VersionIsNewer(string candidate, string current)
    {
        var normalizedCandidate = candidate.Trim().TrimStart('v', 'V');
        var normalizedCurrent = current.Trim().TrimStart('v', 'V');
        return Version.TryParse(normalizedCandidate, out var candidateVersion)
            && Version.TryParse(normalizedCurrent, out var currentVersion)
            && candidateVersion > currentVersion;
    }
}

public sealed record UpdateResult(bool Available, string Tag, Uri? AssetUrl, string Message);

public sealed class ReleaseManifest
{
    public string Tag { get; set; } = "";
    public List<ReleaseAsset> Assets { get; set; } = [];
}

public sealed class ReleaseAsset
{
    public string Name { get; set; } = "";
    public string Path { get; set; } = "";
    public string Url => Path;
}
