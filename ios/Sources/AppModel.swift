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
    private let supportDir: URL?
    private var refreshTask: Task<Void, Never>?
    private var copyClearTask: Task<Void, Never>?
    private var launchAutomationHandled = false

    init() {
        supportDir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
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
        debugLog("init args=\(ProcessInfo.processInfo.arguments)")
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
        let launchAutomationHandled = runLaunchAutomationIfRequested()
        if !launchAutomationHandled, state.autoconnect, !state.vpnEnabled, activeNetwork != nil {
            debugLog("autoconnect starting PacketTunnel")
            setVpnEnabled(true)
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

    func toggleVpn() {
        setVpnEnabled(!state.vpnEnabled)
    }

    private func setVpnEnabled(_ enabled: Bool, force: Bool = false) {
        debugLog("setVpnEnabled enabled=\(enabled) force=\(force) stateEnabled=\(state.vpnEnabled)")
        Task {
            if enabled {
                guard force || !state.vpnEnabled else {
                    debugLog("connect skipped: already enabled")
                    return
                }
                let tunnelConfigJson = core.mobileTunnelConfigJson()
                debugLog("mobileTunnelConfigJson len=\(tunnelConfigJson.count)")
                if state.vpnEnabled {
                    statusMessage = "Turning VPN on"
                } else {
                    dispatch(NativeActions.connectVpn(), status: "Turning VPN on")
                }
                debugLog("starting PacketTunnel stateEnabled=\(state.vpnEnabled) network=\(activeNetwork?.id ?? "nil")")
                do {
                    try await vpnController.start(
                        state: state,
                        network: activeNetwork,
                        tunnelConfigJson: tunnelConfigJson
                    )
                    debugLog("PacketTunnel start returned success")
                } catch {
                    dispatch(NativeActions.disconnectVpn(), status: "Turning VPN off")
                    statusMessage = error.localizedDescription
                    debugLog("PacketTunnel start failed: \(String(describing: error))")
                }
            } else {
                guard force || state.vpnEnabled else {
                    debugLog("disconnect skipped: already disabled")
                    return
                }
                if state.vpnEnabled {
                    dispatch(NativeActions.disconnectVpn(), status: "Turning VPN off")
                }
                do {
                    try await vpnController.stop()
                    debugLog("PacketTunnel stop returned success")
                } catch {
                    statusMessage = error.localizedDescription
                    debugLog("PacketTunnel stop failed: \(String(describing: error))")
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
        debugLog("handle url=\(url.absoluteString)")
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

    private func runLaunchAutomationIfRequested() -> Bool {
        guard !launchAutomationHandled else {
            return false
        }
        launchAutomationHandled = true

        let rawArguments = ProcessInfo.processInfo.arguments
        let arguments = Set(rawArguments)
        debugLog("launch automation args=\(Array(arguments).sorted())")
        if arguments.contains("--nvpn-debug-exit-probe") {
            Task {
                await runDebugExitProbe(arguments: rawArguments)
            }
            return true
        }
        if arguments.contains("--nvpn-connect") {
            setVpnEnabled(true, force: true)
            return true
        }
        if arguments.contains("--nvpn-disconnect") {
            setVpnEnabled(false, force: true)
            return true
        }
        return false
    }

    private func runDebugExitProbe(arguments: [String]) async {
        #if DEBUG
        let urlString = Self.argumentValue(after: "--nvpn-debug-fetch-url", in: arguments)
            ?? "https://am.i.mullvad.net/json"
        let resultName = Self.argumentValue(after: "--nvpn-debug-result", in: arguments)
            ?? "debug-exit-probe.json"
        let waitSeconds = Self.argumentValue(after: "--nvpn-debug-wait-seconds", in: arguments)
            .flatMap(Double.init) ?? 12
        let exitNode = Self.argumentValue(after: "--nvpn-debug-exit-node", in: arguments)
        let clearExit = arguments.contains("--nvpn-debug-clear-exit")
        var result: [String: Any] = [
            "url": urlString,
            "startedAt": ISO8601DateFormatter().string(from: Date()),
        ]

        await stopVpnForDebugProbe()

        if let exitNode, !exitNode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            dispatch(NativeActions.updateSettings([
                "exitNode": exitNode,
                "wireguardExitEnabled": false,
            ]))
        } else if clearExit {
            dispatch(NativeActions.updateSettings([
                "exitNode": "",
                "wireguardExitEnabled": false,
            ]))
        }
        refresh()

        if let error = await startVpnForDebugProbe() {
            result["startError"] = error
        }

        if waitSeconds > 0 {
            try? await Task.sleep(nanoseconds: UInt64(waitSeconds * 1_000_000_000))
        }
        refresh()
        result["exitNode"] = state.exitNode
        result["vpnEnabled"] = state.vpnEnabled
        result["vpnActive"] = state.vpnActive
        result["connectedPeerCount"] = state.connectedPeerCount
        result["expectedPeerCount"] = state.expectedPeerCount

        for (key, value) in await fetchDebugProbe(urlString: urlString) {
            result[key] = value
        }
        result["finishedAt"] = ISO8601DateFormatter().string(from: Date())
        writeDebugProbeResult(result, name: resultName)
        #endif
    }

    private func stopVpnForDebugProbe() async {
        refresh()
        guard state.vpnEnabled else {
            return
        }
        dispatch(NativeActions.disconnectVpn())
        do {
            try await vpnController.stop()
        } catch {
            debugLog("debug probe stop failed: \(String(describing: error))")
        }
        try? await Task.sleep(nanoseconds: 2_000_000_000)
        refresh()
    }

    private func startVpnForDebugProbe() async -> String? {
        let tunnelConfigJson = core.mobileTunnelConfigJson()
        if !state.vpnEnabled {
            dispatch(NativeActions.connectVpn())
        }
        do {
            try await vpnController.start(
                state: state,
                network: activeNetwork,
                tunnelConfigJson: tunnelConfigJson
            )
            return nil
        } catch {
            dispatch(NativeActions.disconnectVpn())
            let message = String(describing: error)
            debugLog("debug probe start failed: \(message)")
            return message
        }
    }

    private func fetchDebugProbe(urlString: String) async -> [String: Any] {
        var result: [String: Any] = [:]
        guard let url = URL(string: urlString) else {
            result["fetchError"] = "Invalid URL"
            return result
        }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 20
        configuration.timeoutIntervalForResource = 25
        let session = URLSession(configuration: configuration)
        do {
            let (data, response) = try await session.data(from: url)
            if let http = response as? HTTPURLResponse {
                result["statusCode"] = http.statusCode
            }
            if let body = String(data: data, encoding: .utf8) {
                result["body"] = String(body.prefix(4096))
            } else {
                result["byteCount"] = data.count
            }
        } catch {
            result["fetchError"] = String(describing: error)
        }
        return result
    }

    private func writeDebugProbeResult(_ result: [String: Any], name: String) {
        guard let supportDir else {
            return
        }
        let safeName = name
            .split(separator: "/")
            .last
            .map(String.init) ?? "debug-exit-probe.json"
        let url = supportDir.appendingPathComponent(safeName)
        guard JSONSerialization.isValidJSONObject(result),
              let data = try? JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys])
        else {
            return
        }
        try? data.write(to: url, options: .atomic)
        debugLog("debug probe wrote \(url.path)")
    }

    private static func argumentValue(after name: String, in arguments: [String]) -> String? {
        guard let index = arguments.firstIndex(of: name) else {
            return nil
        }
        let valueIndex = arguments.index(after: index)
        guard valueIndex < arguments.endIndex else {
            return nil
        }
        return arguments[valueIndex]
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

    private func debugLog(_ message: String) {
        #if DEBUG
        guard let supportDir else {
            return
        }
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
