import Foundation
import NetworkExtension

enum PacketTunnelControllerError: LocalizedError {
    case managerUnavailable
    case preferencesTimedOut

    var errorDescription: String? {
        switch self {
        case .managerUnavailable:
            return "VPN manager unavailable"
        case .preferencesTimedOut:
            return "VPN preferences timed out"
        }
    }
}

final class PacketTunnelController {
    private let providerBundleIdentifier = "to.iris.nvpn.PacketTunnel"

    func start(state: AppState, network: NetworkState?, tunnelConfigJson: String) async throws {
        debugLog("PacketTunnelController.start begin")
        let manager = try await loadOrCreateManager()
        let proto = (manager.protocolConfiguration as? NETunnelProviderProtocol) ?? NETunnelProviderProtocol()
        proto.providerBundleIdentifier = providerBundleIdentifier
        proto.serverAddress = network?.displayName ?? "Nostr VPN"
        proto.providerConfiguration = [
            "networkName": network?.displayName ?? "Nostr VPN",
            "tunnelIp": state.tunnelIp.isEmpty ? "10.44.0.1/32" : state.tunnelIp,
            "mtu": 1280,
            "mobileTunnelConfigJson": tunnelConfigJson,
        ]
        // Tell iOS to actually use the includedRoutes we install
        // (without this iOS sometimes lets system services bypass the
        // tunnel, which is also the only condition under which the
        // VPN status badge stays hidden).
        proto.enforceRoutes = true
        // Don't tear the tunnel down when the screen locks — for a
        // utility VPN we want it to keep running.
        proto.disconnectOnSleep = false
        manager.protocolConfiguration = proto
        manager.localizedDescription = "Nostr VPN"
        manager.isEnabled = true
        debugLog("saving preferences")
        try await save(manager)
        debugLog("reloading preferences")
        try await reload(manager)
        debugLog("calling startVPNTunnel status=\(manager.connection.status.rawValue)")
        try manager.connection.startVPNTunnel(options: [:])
        debugLog("startVPNTunnel returned status=\(manager.connection.status.rawValue)")
    }

    func stop() async throws {
        debugLog("PacketTunnelController.stop begin")
        let manager = try await loadOrCreateManager()
        manager.connection.stopVPNTunnel()
        debugLog("stopVPNTunnel returned status=\(manager.connection.status.rawValue)")
    }

    private func loadOrCreateManager() async throws -> NETunnelProviderManager {
        let managers = try await loadAllManagers()
        debugLog("loaded managers count=\(managers.count)")
        if let existing = managers.first(where: { manager in
            (manager.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier
                == providerBundleIdentifier
        }) {
            debugLog("using existing manager status=\(existing.connection.status.rawValue)")
            return existing
        }
        debugLog("creating new manager")
        return NETunnelProviderManager()
    }

    private func loadAllManagers() async throws -> [NETunnelProviderManager] {
        try await withCheckedThrowingContinuation { continuation in
            NETunnelProviderManager.loadAllFromPreferences { managers, error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: managers ?? [])
                }
            }
        }
    }

    private func save(_ manager: NETunnelProviderManager) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            manager.saveToPreferences { error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: ())
                }
            }
        }
    }

    private func reload(_ manager: NETunnelProviderManager) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            manager.loadFromPreferences { error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: ())
                }
            }
        }
    }

    private func debugLog(_ message: String) {
        #if DEBUG
        guard let supportDir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first?
            .appendingPathComponent("Nostr VPN", isDirectory: true)
        else {
            return
        }
        try? FileManager.default.createDirectory(at: supportDir, withIntermediateDirectories: true)
        let line = "[\(Date())] \(message)\n"
        guard let data = line.data(using: .utf8) else {
            return
        }
        let logUrl = supportDir.appendingPathComponent("app-debug.log")
        if FileManager.default.fileExists(atPath: logUrl.path),
           let handle = try? FileHandle(forWritingTo: logUrl)
        {
            handle.seekToEndOfFile()
            handle.write(data)
            try? handle.close()
        } else {
            try? data.write(to: logUrl)
        }
        #endif
    }
}
