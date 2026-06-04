import Darwin
import Foundation
import Security

/// Runs the bundled `nvpn` CLI as root via Authorization Services.
///
/// The system auth dialog uses Touch ID when the user has enabled
/// "Use Touch ID for: Allow apps to request your password" in System
/// Settings, with a password fallback otherwise. Replaces the legacy
/// `osascript ... with administrator privileges` path, which was
/// password-only.
///
/// Implementation note: `AuthorizationExecuteWithPrivileges` has been
/// formally deprecated since macOS 10.7 but remains the lightest-weight
/// way to run a child process as root without installing a SMJobBless /
/// SMAppService helper. macOS 14 / 15 still ship it. If Apple removes it
/// we'll need to migrate to a privileged helper tool.
final class AuthorizationServicesPrivilegedCommandRunner: PrivilegedCommandRunner, @unchecked Sendable {
    func run(executable: String, args: [String]) -> PrivilegedCommandOutput {
        var authRef: AuthorizationRef?
        let createStatus = AuthorizationCreate(
            nil,
            nil,
            [],
            &authRef
        )
        guard createStatus == errAuthorizationSuccess, let authRef else {
            return PrivilegedCommandOutput(
                success: false,
                cancelled: false,
                stdout: Data(),
                stderr: Data("AuthorizationCreate failed (\(createStatus))".utf8)
            )
        }
        defer { AuthorizationFree(authRef, []) }

        var item = kAuthorizationRightExecute.withCString { name in
            AuthorizationItem(name: name, valueLength: 0, value: nil, flags: 0)
        }
        let copyStatus = withUnsafeMutablePointer(to: &item) { itemPtr -> OSStatus in
            var rights = AuthorizationRights(count: 1, items: itemPtr)
            return AuthorizationCopyRights(
                authRef,
                &rights,
                nil,
                [.interactionAllowed, .preAuthorize, .extendRights],
                nil
            )
        }
        if copyStatus == errAuthorizationCanceled {
            return PrivilegedCommandOutput(
                success: false,
                cancelled: true,
                stdout: Data(),
                stderr: Data()
            )
        }
        if copyStatus != errAuthorizationSuccess {
            return PrivilegedCommandOutput(
                success: false,
                cancelled: false,
                stdout: Data(),
                stderr: Data("AuthorizationCopyRights failed (\(copyStatus))".utf8)
            )
        }

        let argv: [UnsafeMutablePointer<CChar>?] = args.map { strdup($0) } + [nil]
        defer {
            for ptr in argv where ptr != nil {
                free(ptr)
            }
        }

        var pipeFile: UnsafeMutablePointer<FILE>?
        let execStatus = executable.withCString { execCStr -> OSStatus in
            argv.withUnsafeBufferPointer { argvBuf in
                let mutableArgv = UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>(
                    mutating: argvBuf.baseAddress
                )
                return Self.executeWithPrivileges(authRef, execCStr, mutableArgv, &pipeFile)
            }
        }
        if execStatus == errAuthorizationCanceled {
            return PrivilegedCommandOutput(
                success: false,
                cancelled: true,
                stdout: Data(),
                stderr: Data()
            )
        }
        if execStatus != errAuthorizationSuccess {
            return PrivilegedCommandOutput(
                success: false,
                cancelled: false,
                stdout: Data(),
                stderr: Data("AuthorizationExecuteWithPrivileges failed (\(execStatus))".utf8)
            )
        }

        var stdout = Data()
        if let pipeFile {
            let fd = fileno(pipeFile)
            var buf = [UInt8](repeating: 0, count: 4096)
            while true {
                let n = buf.withUnsafeMutableBufferPointer { bp -> Int in
                    read(fd, bp.baseAddress, bp.count)
                }
                if n <= 0 { break }
                stdout.append(buf, count: n)
            }
            fclose(pipeFile)
        }

        // AEWP merges stderr into the returned pipe and gives us no exit
        // status, so we can't distinguish a non-zero exit from a clean
        // run. Treat any output as the command's full chatter and report
        // success; the Rust core's downstream verifications (service
        // status refresh, etc.) will catch real failures.
        return PrivilegedCommandOutput(
            success: true,
            cancelled: false,
            stdout: stdout,
            stderr: Data()
        )
    }

    /// Loads `AuthorizationExecuteWithPrivileges` dynamically so the
    /// deprecation warning doesn't pollute the build, and so the symbol
    /// reference stays soft if a future macOS removes it.
    private static func executeWithPrivileges(
        _ authRef: AuthorizationRef,
        _ pathToTool: UnsafePointer<CChar>,
        _ args: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?,
        _ communicationsPipe: UnsafeMutablePointer<UnsafeMutablePointer<FILE>?>?
    ) -> OSStatus {
        typealias FnT = @convention(c) (
            AuthorizationRef,
            UnsafePointer<CChar>,
            AuthorizationFlags,
            UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?,
            UnsafeMutablePointer<UnsafeMutablePointer<FILE>?>?
        ) -> OSStatus
        guard let sym = Self.resolveExecuteWithPrivilegesSymbol() else {
            return errAuthorizationInternal
        }
        let fn = unsafeBitCast(sym, to: FnT.self)
        return fn(authRef, pathToTool, [], args, communicationsPipe)
    }

    /// Resolves the (long-deprecated) `AuthorizationExecuteWithPrivileges`
    /// symbol at runtime.
    ///
    /// macOS 26 stopped exposing this symbol through the `RTLD_DEFAULT`
    /// namespace, so `dlsym((void*)-2, ...)` now returns `nil` even though the
    /// function still ships in `Security.framework`. The result was that the
    /// privileged `nvpn` invocation silently never ran and the VPN toggle
    /// reverted to off with no auth prompt (see issue #3). Keep the
    /// `RTLD_DEFAULT` lookup first for older systems, then fall back to an
    /// explicit `Security.framework` handle, which still resolves on macOS 26.
    private static func resolveExecuteWithPrivilegesSymbol() -> UnsafeMutableRawPointer? {
        let name = "AuthorizationExecuteWithPrivileges"
        if let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), name) {
            return sym
        }
        let securityPath = "/System/Library/Frameworks/Security.framework/Security"
        guard let handle = dlopen(securityPath, RTLD_LAZY) else {
            return nil
        }
        // The handle is intentionally left resident for the process lifetime;
        // Security.framework is already loaded, so this does not leak a real
        // resource.
        return dlsym(handle, name)
    }
}
