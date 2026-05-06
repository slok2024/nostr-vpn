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

    init() {
        let supportDir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first?
            .appendingPathComponent("Nostr VPN", isDirectory: true)
        if let supportDir {
            try? FileManager.default.createDirectory(at: supportDir, withIntermediateDirectories: true)
        }
        let version = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? ""
        core = NativeCoreClient(dataDir: supportDir?.path ?? "", appVersion: version)
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

    func toggleSession() {
        Task {
            if state.sessionActive {
                do {
                    try await vpnController.stop()
                } catch {
                    statusMessage = error.localizedDescription
                }
                dispatch(NativeActions.disconnectSession(), status: "Disconnecting")
            } else {
                do {
                    try await vpnController.start(
                        state: state,
                        network: activeNetwork,
                        tunnelConfigJson: core.mobileTunnelConfigJson()
                    )
                    dispatch(NativeActions.connectSession(), status: "Connecting")
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
}
