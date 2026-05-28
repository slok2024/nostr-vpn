import Foundation
import SwiftUI
import UIKit

@MainActor
final class AppModel: ObservableObject {
    nonisolated static let appGroupIdentifier = "group.to.iris.nvpn"
    private static let configFileName = "config.toml"
    private static let mobileRuntimeStateFileName = "mobile-runtime-state.json"
    static let vpnDisclosureAcceptedKey = "vpnDisclosureAccepted"
    static let vpnDisclosurePromptMessage = "Review VPN data use before turning VPN on."

    @Published var state: AppState
    @Published var actionInFlight = false
    @Published var statusMessage = ""
    @Published var copiedValue = ""
    @Published var vpnDisclosurePromptVisible = false

    private let core: NativeCoreClient?
    private let vpnController = PacketTunnelController()
    private let supportDir: URL?
    private let fixtureMode: Bool
    private var refreshTask: Task<Void, Never>?
    private var copyClearTask: Task<Void, Never>?
    private var tunnelConfigSyncTask: Task<Void, Never>?
    private var launchAutomationHandled = false
    private var tunnelStateRefreshInFlight = false

    init() {
        fixtureMode = Self.fixtureModeRequested()
        if fixtureMode {
            supportDir = nil
            core = nil
            state = ScreenshotFixtures.state()
            return
        }

        supportDir = Self.supportDirectory()
        if let supportDir {
            try? FileManager.default.createDirectory(at: supportDir, withIntermediateDirectories: true)
            Self.seedMobileConfig(in: supportDir, deviceName: Self.deviceName())
        }
        // Pass empty so the FFI falls back to its own CARGO_PKG_VERSION
        // (workspace-inherited). Avoids drift between MARKETING_VERSION in the
        // xcodeproj and the bundled nvpn binary.
        let client = NativeCoreClient(dataDir: supportDir?.path ?? "", appVersion: "")
        core = client
        state = client.state()
        debugLog("init args=\(ProcessInfo.processInfo.arguments)")
    }

    deinit {
        refreshTask?.cancel()
        tunnelConfigSyncTask?.cancel()
        core?.close()
    }

    var activeNetwork: NetworkState? {
        state.activeNetwork
    }

    func start() {
        guard !fixtureMode else {
            return
        }
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
            if UserDefaults.standard.bool(forKey: Self.vpnDisclosureAcceptedKey) {
                setVpnEnabled(true)
            } else {
                requireVpnDisclosureReview()
            }
        }
    }

    func refresh() {
        guard let core else {
            state.rev += 1
            return
        }
        state = core.refresh()
        refreshTunnelSidecarState()
    }

    func dispatch(_ action: [String: Any], status: String = "") {
        guard !actionInFlight else {
            return
        }
        let actionType = action["type"] as? String ?? ""
        actionInFlight = true
        statusMessage = status
        if fixtureMode {
            state = ScreenshotFixtures.dispatch(action, state: state)
        } else if let core {
            state = core.dispatch(action)
        }
        actionInFlight = false
        statusMessage = state.error
        debugLog(
            "dispatch action=\(actionType) error=\(!state.error.isEmpty) vpn=\(state.vpnEnabled)/\(state.vpnActive) network=\(activeNetwork?.id ?? "nil")"
        )
        if state.error.isEmpty && actionRequiresPacketTunnelConfigSync(actionType) {
            schedulePacketTunnelConfigSync(reason: actionType)
        }
    }

    func toggleVpn() {
        setVpnEnabled(!state.vpnEnabled)
    }

    func requireVpnDisclosureReview() {
        vpnDisclosurePromptVisible = true
        statusMessage = Self.vpnDisclosurePromptMessage
    }

    func markVpnDisclosureAccepted() {
        UserDefaults.standard.set(true, forKey: Self.vpnDisclosureAcceptedKey)
        vpnDisclosurePromptVisible = false
        if statusMessage == Self.vpnDisclosurePromptMessage {
            statusMessage = ""
        }
    }

    private func setVpnEnabled(_ enabled: Bool, force: Bool = false) {
        debugLog("setVpnEnabled enabled=\(enabled) force=\(force) stateEnabled=\(state.vpnEnabled)")
        if fixtureMode {
            state.vpnEnabled = enabled
            state.vpnActive = enabled
            state.vpnStatus = enabled ? "Connected" : "Disconnected"
            state.connectedPeerCount = enabled ? min(state.expectedPeerCount, 3) : 0
            state.fipsConnectedPeerCount = enabled ? min(state.fipsRosterPeerCount, 3) : 0
            state.rev += 1
            statusMessage = ""
            return
        }
        guard let core else {
            statusMessage = "Native core unavailable"
            return
        }
        Task {
            if enabled {
                guard force || !state.vpnEnabled else {
                    debugLog("connect skipped: already enabled")
                    return
                }
                let tunnelConfigJson = core.mobileTunnelConfigJson()
                let providerOptionsConfigJson = core.mobileTunnelProviderOptionsConfigJson()
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
                        tunnelConfigJson: tunnelConfigJson,
                        providerOptionsConfigJson: providerOptionsConfigJson
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

    private func schedulePacketTunnelConfigSync(reason: String) {
        guard !fixtureMode else {
            return
        }
        guard state.vpnEnabled || state.vpnActive else {
            debugLog("PacketTunnel config sync skipped reason=\(reason) vpn off")
            return
        }
        tunnelConfigSyncTask?.cancel()
        tunnelConfigSyncTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 250_000_000)
            await self?.syncPacketTunnelConfig(reason: reason)
        }
    }

    private func syncPacketTunnelConfig(reason: String) async {
        guard let core else {
            statusMessage = "Native core unavailable"
            return
        }
        guard state.vpnEnabled || state.vpnActive else {
            debugLog("PacketTunnel config sync aborted reason=\(reason) vpn off")
            return
        }
        let tunnelConfigJson = core.mobileTunnelConfigJson()
        let providerOptionsConfigJson = core.mobileTunnelProviderOptionsConfigJson()
        debugLog(
            "PacketTunnel config sync begin reason=\(reason) configLen=\(tunnelConfigJson.count) network=\(activeNetwork?.id ?? "nil")"
        )
        statusMessage = "Updating VPN"
        do {
            try await vpnController.stop()
            debugLog("PacketTunnel config sync stop returned")
        } catch {
            debugLog("PacketTunnel config sync stop failed: \(String(describing: error))")
        }
        try? await Task.sleep(nanoseconds: 500_000_000)
        do {
            try await vpnController.start(
                state: state,
                network: activeNetwork,
                tunnelConfigJson: tunnelConfigJson,
                providerOptionsConfigJson: providerOptionsConfigJson
            )
            debugLog("PacketTunnel config sync start returned")
            refresh()
            statusMessage = state.error
        } catch {
            dispatch(NativeActions.disconnectVpn(), status: "Turning VPN off")
            statusMessage = error.localizedDescription
            debugLog("PacketTunnel config sync start failed: \(String(describing: error))")
        }
    }

    private func actionRequiresPacketTunnelConfigSync(_ type: String) -> Bool {
        switch type {
        case "import_network_invite",
             "request_network_join",
             "manual_add_network",
             "set_network_enabled",
             "set_network_mesh_id",
             "set_network_join_requests_enabled",
             "add_participant",
             "set_participant_endpoint_hints",
             "add_admin",
             "remove_participant",
             "remove_admin",
             "accept_join_request",
             "reject_join_request":
            return true
        default:
            return false
        }
    }

    func importInvite(_ invite: String) {
        let trimmed = invite.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        debugLog("importInvite len=\(trimmed.count)")
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
        result["fipsConnectedPeerCount"] = state.fipsConnectedPeerCount
        result["fipsRosterPeerCount"] = state.fipsRosterPeerCount
        result["nonFipsRosterPeerCount"] = state.nonFipsRosterPeerCount

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
        guard let core else {
            return "Native core unavailable"
        }
        let tunnelConfigJson = core.mobileTunnelConfigJson()
        let providerOptionsConfigJson = core.mobileTunnelProviderOptionsConfigJson()
        if !state.vpnEnabled {
            dispatch(NativeActions.connectVpn())
        }
        do {
            try await vpnController.start(
                state: state,
                network: activeNetwork,
                tunnelConfigJson: tunnelConfigJson,
                providerOptionsConfigJson: providerOptionsConfigJson
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

    nonisolated private static func argumentValue(after name: String, in arguments: [String]) -> String? {
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
        if fixtureMode {
            return ScreenshotFixtures.qrMatrix()
        }
        return core?.qrMatrix(invite: invite) ?? QrMatrix()
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

        let config = supportDir.appendingPathComponent(configFileName)
        guard !FileManager.default.fileExists(atPath: config.path) else {
            return
        }

        let escaped = name
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        try? "node_name = \"\(escaped)\"\n".write(to: config, atomically: true, encoding: .utf8)
    }

    private func refreshTunnelSidecarState() {
        guard state.vpnEnabled, !tunnelStateRefreshInFlight else {
            return
        }
        tunnelStateRefreshInFlight = true
        Task { [weak self] in
            let runtimeJson = await self?.vpnController.runtimeStateJson()
            let appConfigToml = await self?.vpnController.takeAppConfigToml()
            await MainActor.run {
                guard let self else {
                    return
                }
                self.tunnelStateRefreshInFlight = false
                var wrote = false
                if let appConfigToml, self.writeTunnelAppConfigIfNeeded(appConfigToml) {
                    wrote = true
                }
                if let runtimeJson, self.writeTunnelRuntimeStateIfNeeded(runtimeJson) {
                    wrote = true
                }
                if wrote {
                    self.state = self.core?.refresh() ?? self.state
                }
            }
        }
    }

    private func writeTunnelRuntimeStateIfNeeded(_ json: String) -> Bool {
        let trimmed = json.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty,
              let data = trimmed.data(using: .utf8),
              (try? JSONSerialization.jsonObject(with: data)) != nil
        else {
            return false
        }
        return writeSupportFileIfChanged(data, name: Self.mobileRuntimeStateFileName)
    }

    private func writeTunnelAppConfigIfNeeded(_ toml: String) -> Bool {
        let trimmed = toml.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.hasPrefix("# failed") else {
            return false
        }
        guard let data = toml.data(using: .utf8) else {
            return false
        }
        return writeSupportFileIfChanged(data, name: Self.configFileName)
    }

    private func writeSupportFileIfChanged(_ data: Data, name: String) -> Bool {
        guard let supportDir else {
            return false
        }
        try? FileManager.default.createDirectory(at: supportDir, withIntermediateDirectories: true)
        let url = supportDir.appendingPathComponent(name)
        if let existing = try? Data(contentsOf: url), existing == data {
            return false
        }
        do {
            try data.write(to: url, options: .atomic)
            debugLog("wrote tunnel sidecar file \(name)")
            return true
        } catch {
            debugLog("failed to write tunnel sidecar file \(name): \(String(describing: error))")
            return false
        }
    }

    nonisolated static func supportDirectory() -> URL? {
        let legacy = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first?
            .appendingPathComponent("Nostr VPN", isDirectory: true)
        guard let shared = FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: appGroupIdentifier)?
            .appendingPathComponent("Nostr VPN", isDirectory: true)
        else {
            return legacy
        }
        migrateLegacySupportDirectory(from: legacy, to: shared)
        return shared
    }

    nonisolated private static func migrateLegacySupportDirectory(from legacy: URL?, to shared: URL) {
        guard let legacy, legacy.path != shared.path else {
            return
        }
        let manager = FileManager.default
        guard manager.fileExists(atPath: legacy.path) else {
            return
        }
        try? manager.createDirectory(at: shared, withIntermediateDirectories: true)
        guard let items = try? manager.contentsOfDirectory(at: legacy, includingPropertiesForKeys: nil)
        else {
            return
        }
        for item in items {
            let destination = shared.appendingPathComponent(item.lastPathComponent)
            guard !manager.fileExists(atPath: destination.path) else {
                continue
            }
            try? manager.copyItem(at: item, to: destination)
        }
    }

    private static func deviceName() -> String {
        let preferred = UIDevice.current.name.trimmingCharacters(in: .whitespacesAndNewlines)
        if !preferred.isEmpty {
            return preferred
        }

        let model = UIDevice.current.model.trimmingCharacters(in: .whitespacesAndNewlines)
        return model.isEmpty ? "iOS device" : model
    }

    nonisolated static func screenshotTabArgument() -> String? {
        argumentValue(after: "--nvpn-screenshot-tab", in: ProcessInfo.processInfo.arguments)
    }

    nonisolated private static func fixtureModeRequested() -> Bool {
        #if DEBUG
        let arguments = ProcessInfo.processInfo.arguments
        if arguments.contains("--nvpn-fixture-mode") {
            return true
        }
        let raw = ProcessInfo.processInfo.environment["NVPN_IOS_FIXTURE_MODE"] ?? ""
        return ["1", "true", "yes", "on"].contains(raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased())
        #else
        return false
        #endif
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
