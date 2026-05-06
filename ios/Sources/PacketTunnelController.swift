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
    private let providerBundleIdentifier = "to.iris.nvpn.ios.PacketTunnel"

    func start(state: AppState, network: NetworkState?, tunnelConfigJson: String) async throws {
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
        manager.protocolConfiguration = proto
        manager.localizedDescription = "Nostr VPN"
        manager.isEnabled = true
        try await save(manager)
        try await reload(manager)
        try manager.connection.startVPNTunnel(options: [:])
    }

    func stop() async throws {
        let manager = try await loadOrCreateManager()
        manager.connection.stopVPNTunnel()
    }

    private func loadOrCreateManager() async throws -> NETunnelProviderManager {
        let managers = try await loadAllManagers()
        if let existing = managers.first(where: { manager in
            (manager.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier
                == providerBundleIdentifier
        }) {
            return existing
        }
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
}
