import Foundation
import SwiftUI
import UIKit

@MainActor
final class AppModel: ObservableObject {
    @Published var state: AppState
    @Published var actionInFlight = false
    @Published var statusMessage = ""
    @Published var copiedValue = ""

    private let core: NativeCoreClient
    private let vpnController = PacketTunnelController()
    private var refreshTask: Task<Void, Never>?
    private var copyClearTask: Task<Void, Never>?
    private var launchAutomationHandled = false

    init() {
        let supportDir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first?
            .appendingPathComponent("Nostr VPN", isDirectory: true)
        if let supportDir {
            try? FileManager.default.createDirectory(at: supportDir, withIntermediateDirectories: true)
            Self.seedMobileConfig(in: supportDir, deviceName: Self.deviceName())
        }
        // Pass empty so the FFI falls back to its own CARGO_PKG_VERSION
        // (workspace-inherited). Avoids drift between MARKETING_VERSION in the
        // xcodeproj and the bundled nvpn binary.
        core = NativeCoreClient(dataDir: supportDir?.path ?? "", appVersion: "")
        state = core.state()
    }

    deinit {
        refreshTask?.cancel()
        core.close()
    }

    var activeNetwork: NetworkState? {
        state.activeNetwork
    }

    func start() {
        guard refreshTask == nil else {
            return
        }
        refreshTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 2_000_000_000)
                self?.refresh()
            }
        }
        runLaunchAutomationIfRequested()
    }

    func refresh() {
        state = core.refresh()
    }

    func dispatch(_ action: [String: Any], status: String = "") {
        guard !actionInFlight else {
            return
        }
        actionInFlight = true
        statusMessage = status
        state = core.dispatch(action)
        actionInFlight = false
        statusMessage = state.error
    }

    func toggleVpn() {
        setVpnEnabled(!state.vpnEnabled)
    }

    private func setVpnEnabled(_ enabled: Bool, force: Bool = false) {
        Task {
            if enabled {
                guard force || !state.vpnEnabled else {
                    return
                }
                let tunnelConfigJson = core.mobileTunnelConfigJson()
                if state.vpnEnabled {
                    statusMessage = "Turning VPN on"
                } else {
                    dispatch(NativeActions.connectVpn(), status: "Turning VPN on")
                }
                do {
                    try await vpnController.start(
                        state: state,
                        network: activeNetwork,
                        tunnelConfigJson: tunnelConfigJson
                    )
                } catch {
                    dispatch(NativeActions.disconnectVpn(), status: "Turning VPN off")
                    statusMessage = error.localizedDescription
                }
            } else {
                guard force || state.vpnEnabled else {
                    return
                }
                if state.vpnEnabled {
                    dispatch(NativeActions.disconnectVpn(), status: "Turning VPN off")
                }
                do {
                    try await vpnController.stop()
                } catch {
                    statusMessage = error.localizedDescription
                }
            }
        }
    }

    func importInvite(_ invite: String) {
        let trimmed = invite.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        dispatch(NativeActions.importInvite(trimmed), status: "Importing")
    }

    func handle(url: URL) {
        let raw = url.absoluteString
        if raw.lowercased().hasPrefix("nvpn://invite/") {
            importInvite(raw)
            return
        }

        guard url.scheme == "nvpn", url.host == "debug" else {
            return
        }

        let action = url.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        if action == "tick" {
            refresh()
        } else if action == "connect" {
            setVpnEnabled(true, force: true)
        } else if action == "disconnect" {
            setVpnEnabled(false, force: true)
        }
    }

    private func runLaunchAutomationIfRequested() {
        guard !launchAutomationHandled else {
            return
        }
        launchAutomationHandled = true

        let arguments = Set(ProcessInfo.processInfo.arguments)
        if arguments.contains("--nvpn-connect") {
            setVpnEnabled(true, force: true)
        } else if arguments.contains("--nvpn-disconnect") {
            setVpnEnabled(false, force: true)
        }
    }

    func qrMatrix(for invite: String) -> QrMatrix {
        core.qrMatrix(invite: invite)
    }

    func copy(_ value: String) {
        guard !value.isEmpty else {
            return
        }
        UIPasteboard.general.string = value
        copiedValue = value
        copyClearTask?.cancel()
        copyClearTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            await MainActor.run {
                if self?.copiedValue == value {
                    self?.copiedValue = ""
                }
            }
        }
    }

    private static func seedMobileConfig(in supportDir: URL, deviceName: String) {
        let name = deviceName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else {
            return
        }

        let config = supportDir.appendingPathComponent("config.toml")
        guard !FileManager.default.fileExists(atPath: config.path) else {
            return
        }

        let escaped = name
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        try? "node_name = \"\(escaped)\"\n".write(to: config, atomically: true, encoding: .utf8)
    }

    private static func deviceName() -> String {
        let preferred = UIDevice.current.name.trimmingCharacters(in: .whitespacesAndNewlines)
        if !preferred.isEmpty {
            return preferred
        }

        let model = UIDevice.current.model.trimmingCharacters(in: .whitespacesAndNewlines)
        return model.isEmpty ? "iOS device" : model
    }
}
