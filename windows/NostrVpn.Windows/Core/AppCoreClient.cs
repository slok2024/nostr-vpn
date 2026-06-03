using System.Runtime.InteropServices;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace NostrVpn.Windows.Core;

public sealed class AppCoreClient : IDisposable
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNameCaseInsensitive = true,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };

    private readonly IntPtr _handle;
    private bool _disposed;

    public AppCoreClient(string dataDir, string version)
    {
        Directory.CreateDirectory(dataDir);
        _handle = NativeMethods.AppNew(dataDir, version);
        if (_handle == IntPtr.Zero)
        {
            throw new InvalidOperationException("failed to create native app core");
        }
    }

    public NativeAppState State()
    {
        return DeserializeState(TakeString(NativeMethods.AppStateJson(_handle)));
    }

    public NativeAppState Refresh()
    {
        return DeserializeState(TakeString(NativeMethods.AppRefreshJson(_handle)));
    }

    public NativeAppState Dispatch(string actionJson)
    {
        return DeserializeState(TakeString(NativeMethods.AppDispatchJson(_handle, actionJson)));
    }

    public QrMatrix QrMatrix(string text)
    {
        var json = TakeString(NativeMethods.QrMatrixJson(text));
        return JsonSerializer.Deserialize<QrMatrix>(json, JsonOptions) ?? new QrMatrix
        {
            Error = "failed to decode QR matrix response",
        };
    }

    public QrDecodeResult DecodeQrImage(string path)
    {
        var json = TakeString(NativeMethods.DecodeQrImageJson(path));
        return JsonSerializer.Deserialize<QrDecodeResult>(json, JsonOptions) ?? new QrDecodeResult
        {
            Error = "failed to decode QR response",
        };
    }

    public static NativeUpdateResult CheckUpdate(string currentVersion, string mode = "app", string source = "auto")
    {
        var json = TakeString(NativeMethods.UpdateCheckJson(currentVersion, mode, source));
        return DecodeUpdateResult(json);
    }

    public static NativeUpdateResult DownloadUpdate(
        string currentVersion,
        string downloadDir,
        string mode = "app",
        string source = "auto")
    {
        var json = TakeString(NativeMethods.UpdateDownloadJson(currentVersion, mode, source, downloadDir));
        return DecodeUpdateResult(json);
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }
        NativeMethods.AppFree(_handle);
        _disposed = true;
    }

    public static string Action(object value)
    {
        return JsonSerializer.Serialize(value, JsonOptions);
    }

    private static NativeAppState DeserializeState(string json)
    {
        return JsonSerializer.Deserialize<NativeAppState>(json, JsonOptions) ?? new NativeAppState
        {
            Error = "failed to decode native state",
        };
    }

    private static NativeUpdateResult DecodeUpdateResult(string json)
    {
        var result = JsonSerializer.Deserialize<NativeUpdateResult>(json, JsonOptions) ?? new NativeUpdateResult
        {
            Error = "failed to decode native update response",
        };
        if (!string.IsNullOrWhiteSpace(result.Error))
        {
            throw new InvalidOperationException(result.Error);
        }
        return result;
    }

    private static string TakeString(IntPtr ptr)
    {
        if (ptr == IntPtr.Zero)
        {
            return "";
        }
        try
        {
            return Marshal.PtrToStringUTF8(ptr) ?? "";
        }
        finally
        {
            NativeMethods.StringFree(ptr);
        }
    }

    private static class NativeMethods
    {
        private const string Library = "nostr_vpn_app_core";

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, EntryPoint = "nostr_vpn_app_new")]
        public static extern IntPtr AppNew(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string dataDir,
            [MarshalAs(UnmanagedType.LPUTF8Str)] string appVersion);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, EntryPoint = "nostr_vpn_app_free")]
        public static extern void AppFree(IntPtr handle);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, EntryPoint = "nostr_vpn_app_state_json")]
        public static extern IntPtr AppStateJson(IntPtr handle);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, EntryPoint = "nostr_vpn_app_refresh_json")]
        public static extern IntPtr AppRefreshJson(IntPtr handle);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, EntryPoint = "nostr_vpn_app_dispatch_json")]
        public static extern IntPtr AppDispatchJson(
            IntPtr handle,
            [MarshalAs(UnmanagedType.LPUTF8Str)] string actionJson);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, EntryPoint = "nostr_vpn_qr_matrix_json")]
        public static extern IntPtr QrMatrixJson([MarshalAs(UnmanagedType.LPUTF8Str)] string text);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, EntryPoint = "nostr_vpn_decode_qr_image_json")]
        public static extern IntPtr DecodeQrImageJson([MarshalAs(UnmanagedType.LPUTF8Str)] string path);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, EntryPoint = "nostr_vpn_update_check_json")]
        public static extern IntPtr UpdateCheckJson(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string currentVersion,
            [MarshalAs(UnmanagedType.LPUTF8Str)] string mode,
            [MarshalAs(UnmanagedType.LPUTF8Str)] string source);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, EntryPoint = "nostr_vpn_update_download_json")]
        public static extern IntPtr UpdateDownloadJson(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string currentVersion,
            [MarshalAs(UnmanagedType.LPUTF8Str)] string mode,
            [MarshalAs(UnmanagedType.LPUTF8Str)] string source,
            [MarshalAs(UnmanagedType.LPUTF8Str)] string downloadDir);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, EntryPoint = "nostr_vpn_string_free")]
        public static extern void StringFree(IntPtr value);
    }
}
