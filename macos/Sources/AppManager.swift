import AppKit
import CoreImage
import Darwin
import Foundation
import SwiftUI
import UniformTypeIdentifiers

private let githubUpdateManifestUrl = URL(string: "https://api.github.com/repos/mmalmi/nostr-vpn/releases/latest")!
private let defaultUpdateManifestUrl = URL(string: "https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Fnostr-vpn/latest/release.json")!
private let updateRequestTimeout: TimeInterval = 8
private let updateUserAgent = "nvpn-updater"
private let defaultUpdatePollIntervalNanoseconds: UInt64 = 6 * 60 * 60 * 1_000_000_000

@MainActor
final class AppManager: ObservableObject {
    @Published private(set) var state: NativeAppState
    @Published private(set) var actionInFlight = false
    @Published private(set) var actionStatus = ""
    @Published private(set) var copiedValue: CopyValue?
    @Published private(set) var copiedPeerNpub: String?
    @Published private(set) var serviceSettling = false
    @Published private(set) var updateChecking = false
    @Published private(set) var updateInstalling = false
    @Published private(set) var updateAvailable = false
    @Published private(set) var updateVersion = ""
    @Published private(set) var updateStatus = ""
    @Published var autoCheckUpdates = UserDefaults.standard.object(forKey: "updates.autoCheck") as? Bool ?? true {
        didSet {
            UserDefaults.standard.set(autoCheckUpdates, forKey: "updates.autoCheck")
            if autoCheckUpdates {
                startAutomaticUpdateChecks()
            } else {
                stopAutomaticUpdateChecks()
            }
        }
    }
    @Published var autoInstallUpdates = UserDefaults.standard.bool(forKey: "updates.autoInstall") {
        didSet {
            UserDefaults.standard.set(autoInstallUpdates, forKey: "updates.autoInstall")
        }
    }
    @Published var inviteInput = ""

    private let app: FfiApp
    private var refreshTask: Task<Void, Never>?
    private var copyClearTask: Task<Void, Never>?
    private var actionStatusClearTask: Task<Void, Never>?
    private var serviceSettlementTask: Task<Void, Never>?
    private var updateTask: Task<Void, Never>?
    private var updatePollTask: Task<Void, Never>?
    private var startupUrlsDrained = false
    private var startupUpdateCheckDone = false
    private var updateAssetUrl: URL?
    private let updateManifestUrls: [URL] = {
        if let overrideUrl = ProcessInfo.processInfo.environment["NVPN_UPDATE_MANIFEST_URL"]
            .flatMap(URL.init(string:)) {
            return [overrideUrl]
        }
        return [githubUpdateManifestUrl, defaultUpdateManifestUrl]
    }()
    let launchedHidden: Bool

    private static var updatePollIntervalNanoseconds: UInt64 {
        if let raw = ProcessInfo.processInfo.environment["NVPN_UPDATE_POLL_SECONDS"],
           let seconds = Double(raw),
           seconds > 0 {
            return UInt64(seconds * 1_000_000_000)
        }
        return defaultUpdatePollIntervalNanoseconds
    }

    init() {
        let dataDir = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first?
            .appendingPathComponent("nvpn", isDirectory: true)
            .path ?? ""
        // Pass empty so the FFI falls back to its own CARGO_PKG_VERSION
        // (workspace-inherited). Avoids drift between MARKETING_VERSION in the
        // xcodeproj and the bundled nvpn binary.
        let app = FfiApp(dataDir: dataDir, appVersion: "")
        app.setPrivilegedCommandRunner(runner: AuthorizationServicesPrivilegedCommandRunner())
        self.app = app
        self.state = app.state()
        self.launchedHidden = CommandLine.arguments.contains("--autostart")
            || ProcessInfo.processInfo.environment["NVPN_AUTOSTART"] == "1"
    }

    var activeNetwork: NativeNetworkState? {
        state.networks.first(where: { $0.enabled }) ?? state.networks.first
    }

    var inactiveNetworks: [NativeNetworkState] {
        state.networks.filter { !$0.enabled }
    }

    var serviceUpdateRecommended: Bool {
        Self.serviceUpdateRecommended(in: state)
    }

    var vpnSwitchEnabled: Bool {
        state.vpnEnabled
    }

    /// Status line shown next to the VPN switch in the header and the tray.
    /// Single source of truth so both stay in sync.
    var vpnStatusText: String {
        if actionInFlight, !actionStatus.isEmpty {
            return actionStatus
        }
        if state.exitNodeBlocked {
            return state.exitNodeStatusText.isEmpty ? "Internet blocked" : state.exitNodeStatusText
        }
        if state.exitNodeActive, !state.exitNodeStatusText.isEmpty {
            return state.exitNodeStatusText
        }
        if state.vpnActive {
            return state.vpnStatus.isEmpty ? "VPN on" : state.vpnStatus
        }
        if state.vpnEnabled {
            return state.vpnStatus.isEmpty ? "Turning on" : state.vpnStatus
        }
        if Self.serviceUpdateRecommended(in: state) {
            return "Background service needs update"
        }
        return "Off"
    }

    var updateInstallEnabled: Bool {
        updateAvailable && updateAssetUrl != nil && !updateChecking && !updateInstalling
    }

    func start() {
        drainStartupUrls()
        startAutomaticUpdateChecks()
        guard refreshTask == nil else {
            return
        }
        refreshTask = Task { [weak self] in
            while !Task.isCancelled {
                self?.refresh()
                try? await Task.sleep(nanoseconds: 1_500_000_000)
            }
        }
    }

    deinit {
        refreshTask?.cancel()
        copyClearTask?.cancel()
        actionStatusClearTask?.cancel()
        serviceSettlementTask?.cancel()
        updateTask?.cancel()
        updatePollTask?.cancel()
    }

    func refresh() {
        let app = app
        Task {
            let nextState = await Task.detached {
                app.refresh()
            }.value
            await MainActor.run {
                self.state = nextState
                self.maybePromptServiceUpdate(nextState)
            }
        }
    }

    func dispatch(
        _ action: NativeAppAction,
        status: String = "",
        successStatus: String = "",
        settleService: Bool = false
    ) {
        guard !actionInFlight else {
            return
        }
        actionStatusClearTask?.cancel()
        actionInFlight = true
        actionStatus = status
        let app = app
        Task {
            let nextState = await Task.detached {
                app.dispatch(action: action)
            }.value
            await MainActor.run {
                self.state = nextState
                self.actionInFlight = false
                self.actionStatus = nextState.error.isEmpty ? successStatus : nextState.error
                self.maybePromptServiceUpdate(nextState)
                if nextState.error.isEmpty, !successStatus.isEmpty {
                    self.clearActionStatus(after: 3)
                }
                if settleService {
                    self.startServiceSettlementPolling()
                }
            }
        }
    }

    func toggleVpn() {
        let enabled = !state.vpnEnabled
        dispatch(
            enabled ? .connectVpn : .disconnectVpn,
            status: enabled ? "Turning VPN on" : "Turning VPN off"
        )
    }

    func copy(_ value: String, as copied: CopyValue? = nil, peerNpub: String? = nil) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(value, forType: .string)
        copiedValue = copied
        copiedPeerNpub = peerNpub
        copyClearTask?.cancel()
        copyClearTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            await MainActor.run {
                guard !Task.isCancelled else {
                    return
                }
                self?.copiedValue = nil
                self?.copiedPeerNpub = nil
            }
        }
    }

    func share(_ value: String) {
        guard let contentView = NSApp.keyWindow?.contentView else {
            copy(value, as: .invite)
            return
        }
        let picker = NSSharingServicePicker(items: [value])
        picker.show(relativeTo: contentView.bounds, of: contentView, preferredEdge: .minY)
    }

    func handle(url: URL) {
        let raw = url.absoluteString
        if raw.starts(with: "nvpn://invite/") {
            importInvite(raw)
            return
        }

        guard url.scheme == "nvpn", url.host == "debug" else {
            return
        }
        let action = url.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        switch action {
        case "tick":
            dispatch(.tick, status: "Refreshing")
        case "request-join":
            let networkId = queryValue("networkId", in: url) ?? queryValue("network", in: url) ?? activeNetwork?.id
            if let networkId {
                requestNetworkJoin(networkId: networkId)
            }
        case "accept-join":
            let networkId = queryValue("networkId", in: url) ?? queryValue("network", in: url) ?? activeNetwork?.id
            let requester = queryValue("requester", in: url)
                ?? queryValue("requesterNpub", in: url)
                ?? activeNetwork?.inboundJoinRequests.first?.requesterNpub
            if let networkId, let requester {
                acceptJoinRequest(networkId: networkId, requesterNpub: requester)
            }
        default:
            break
        }
    }

    func importInvite(_ invite: String) {
        let trimmed = invite.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        // Clear immediately so the field reflects "import in flight" rather
        // than holding the same text the user just submitted (and so a stale
        // invite from a prior session doesn't quietly re-fire).
        inviteInput = ""
        dispatch(.importNetworkInvite(invite: trimmed), status: "Importing invite")
    }

    func chooseInviteQrImage() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.begin { [weak self] response in
            guard response == .OK, let url = panel.url else {
                return
            }
            Task { @MainActor in
                self?.importInviteFromQrImage(url)
            }
        }
    }

    func importInviteFromQrImage(_ url: URL) {
        do {
            let invite = try decodeQrCode(from: url)
            importInvite(invite)
        } catch {
            actionStatus = error.localizedDescription
        }
    }

    func saveNodeSettings(
        nodeName: String,
        endpoint: String,
        tunnelIp: String,
        listenPort: String,
        magicDnsSuffix: String
    ) {
        let parsedPort = UInt16(listenPort.trimmingCharacters(in: .whitespacesAndNewlines))
        dispatch(.updateSettings(patch: settingsPatch(
            nodeName: nodeName,
            endpoint: endpoint,
            tunnelIp: tunnelIp,
            listenPort: parsedPort,
            magicDnsSuffix: magicDnsSuffix
        )), status: "Saving device settings")
    }

    func setAdvertiseExitNode(_ enabled: Bool) {
        dispatch(.updateSettings(patch: settingsPatch(advertiseExitNode: enabled)), status: "Saving routing")
    }

    func setWireGuardExitEnabled(_ enabled: Bool) {
        dispatch(.updateSettings(patch: settingsPatch(wireguardExitEnabled: enabled)), status: "Saving WireGuard")
    }

    func saveWireGuardExitConfig(_ config: String) {
        dispatch(.updateSettings(patch: settingsPatch(wireguardExitConfig: config)), status: "Saving WireGuard")
    }

    func saveWireGuardExitSettings(
        interface: String,
        address: String,
        privateKey: String,
        peerPublicKey: String,
        peerPresharedKey: String,
        endpoint: String,
        allowedIps: String,
        dns: String,
        mtu: String,
        keepalive: String
    ) {
        dispatch(.updateSettings(patch: settingsPatch(
            wireguardExitInterface: interface,
            wireguardExitAddress: address,
            wireguardExitPrivateKey: privateKey,
            wireguardExitPeerPublicKey: peerPublicKey,
            wireguardExitPeerPresharedKey: peerPresharedKey,
            wireguardExitEndpoint: endpoint,
            wireguardExitAllowedIps: allowedIps,
            wireguardExitDns: dns,
            wireguardExitMtu: UInt16(mtu.trimmingCharacters(in: .whitespacesAndNewlines)),
            wireguardExitPersistentKeepaliveSecs: UInt16(keepalive.trimmingCharacters(in: .whitespacesAndNewlines))
        )), status: "Saving WireGuard")
    }

    func setAutoconnect(_ enabled: Bool) {
        dispatch(.updateSettings(patch: settingsPatch(autoconnect: enabled)), status: "Saving VPN option")
    }

    func setLaunchOnStartup(_ enabled: Bool) {
        do {
            try configureLaunchAgent(enabled: enabled)
            dispatch(.updateSettings(patch: settingsPatch(launchOnStartup: enabled)), status: "Saving startup option")
        } catch {
            actionStatus = error.localizedDescription
        }
    }

    func setCloseToTray(_ enabled: Bool) {
        dispatch(.updateSettings(patch: settingsPatch(closeToTrayOnClose: enabled)), status: "Saving menu bar option")
    }

    func setAdvertisedRoutes(_ routes: String) {
        dispatch(.updateSettings(patch: settingsPatch(advertisedRoutes: routes)), status: "Saving routes")
    }

    func setExitNode(_ npub: String) {
        dispatch(.updateSettings(patch: settingsPatch(exitNode: npub)), status: "Saving exit node")
    }

    func selectDirectExit() {
        dispatch(
            .updateSettings(patch: settingsPatch(exitNode: "", wireguardExitEnabled: false)),
            status: "Saving exit node"
        )
    }

    func selectWireGuardUpstreamExit() {
        dispatch(
            .updateSettings(patch: settingsPatch(exitNode: "", wireguardExitEnabled: true)),
            status: "Saving exit node"
        )
    }

    func selectPeerExit(_ npub: String) {
        dispatch(
            .updateSettings(patch: settingsPatch(exitNode: npub, wireguardExitEnabled: false)),
            status: "Saving exit node"
        )
    }

    func setExitNodeLeakProtection(_ enabled: Bool) {
        dispatch(.updateSettings(patch: settingsPatch(exitNodeLeakProtection: enabled)), status: "Saving exit protection")
    }

    func addParticipant(networkId: String, npub: String, alias: String? = nil) {
        let trimmed = npub.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            let trimmedAlias = alias?.trimmingCharacters(in: .whitespacesAndNewlines)
            dispatch(
                .addParticipant(networkId: networkId, npub: trimmed, alias: trimmedAlias?.isEmpty == false ? trimmedAlias : nil),
                status: "Adding participant"
            )
        }
    }

    func renameNetwork(networkId: String, name: String) {
        dispatch(.renameNetwork(networkId: networkId, name: name), status: "Renaming network")
    }

    func setNetworkMeshId(networkId: String, meshId: String) {
        dispatch(.setNetworkMeshId(networkId: networkId, meshId: meshId), status: "Saving mesh ID")
    }

    func setNetworkEnabled(networkId: String, enabled: Bool) {
        dispatch(.setNetworkEnabled(networkId: networkId, enabled: enabled), status: enabled ? "Activating network" : "Disabling network")
    }

    func setJoinRequests(networkId: String, enabled: Bool) {
        dispatch(.setNetworkJoinRequestsEnabled(networkId: networkId, enabled: enabled), status: "Saving join request setting")
    }

    func requestNetworkJoin(networkId: String) {
        dispatch(.requestNetworkJoin(networkId: networkId), status: "Requesting network join")
    }

    func acceptJoinRequest(networkId: String, requesterNpub: String) {
        dispatch(.acceptJoinRequest(networkId: networkId, requesterNpub: requesterNpub), status: "Accepting join request")
    }

    func setParticipantAlias(npub: String, alias: String) {
        dispatch(.setParticipantAlias(npub: npub, alias: alias), status: "Saving alias")
    }

    func toggleAdmin(networkId: String, participant: NativeParticipantState) {
        if participant.isAdmin {
            dispatch(.removeAdmin(networkId: networkId, npub: participant.npub), status: "Removing admin")
        } else {
            dispatch(.addAdmin(networkId: networkId, npub: participant.npub), status: "Adding admin")
        }
    }

    func removeParticipant(networkId: String, npub: String) {
        dispatch(.removeParticipant(networkId: networkId, npub: npub), status: "Removing participant")
    }

    func addNetwork(_ name: String) {
        dispatch(.addNetwork(name: name.trimmingCharacters(in: .whitespacesAndNewlines)), status: "Adding network")
    }

    func manualAddNetwork(adminNpub: String, meshNetworkId: String) {
        let admin = adminNpub.trimmingCharacters(in: .whitespacesAndNewlines)
        let mesh = meshNetworkId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !admin.isEmpty, !mesh.isEmpty else { return }
        dispatch(.manualAddNetwork(adminNpub: admin, meshNetworkId: mesh), status: "Adding network")
    }

    func removeNetwork(_ networkId: String) {
        dispatch(.removeNetwork(networkId: networkId), status: "Deleting network")
    }

    func installCli() {
        dispatch(.installCli, status: "Installing CLI")
    }

    func uninstallCli() {
        dispatch(.uninstallCli, status: "Uninstalling CLI")
    }

    func installService() {
        let installing = serviceUpdateRecommended ? "Updating service" : state.serviceInstalled ? "Reinstalling service" : "Installing service"
        let installed = serviceUpdateRecommended ? "Service updated" : state.serviceInstalled ? "Service reinstalled" : "Service installed"
        dispatch(.installSystemService, status: installing, successStatus: installed, settleService: true)
    }

    func enableService() {
        dispatch(.enableSystemService, status: "Enabling service", successStatus: "Service enabled", settleService: true)
    }

    func disableService() {
        dispatch(.disableSystemService, status: "Disabling service", successStatus: "Service disabled", settleService: true)
    }

    func uninstallService() {
        dispatch(.uninstallSystemService, status: "Uninstalling service", successStatus: "Service uninstalled", settleService: true)
    }

    func startInviteBroadcast() {
        dispatch(.startInviteBroadcast, status: "Broadcasting invite")
    }

    func stopInviteBroadcast() {
        dispatch(.stopInviteBroadcast, status: "Stopped broadcasting")
    }

    func startNearbyDiscovery() {
        dispatch(.startNearbyDiscovery, status: "Looking for nearby")
    }

    func stopNearbyDiscovery() {
        dispatch(.stopNearbyDiscovery, status: "Stopped looking")
    }

    func checkForUpdates(manual: Bool = true) {
        guard !updateChecking else {
            return
        }
        updateTask?.cancel()
        updateChecking = true
        if manual {
            updateStatus = "Checking for updates"
        }
        updateTask = Task { [weak self] in
            guard let self else {
                return
            }
            do {
                let check = try await self.fetchUpdateCheck()
                await MainActor.run {
                    self.applyUpdateCheck(check, manual: manual)
                }
            } catch {
                await MainActor.run {
                    self.updateChecking = false
                    if manual {
                        self.updateStatus = error.localizedDescription
                    }
                }
            }
        }
    }

    private func startAutomaticUpdateChecks() {
        guard autoCheckUpdates else {
            stopAutomaticUpdateChecks()
            return
        }
        if !startupUpdateCheckDone {
            startupUpdateCheckDone = true
            checkForUpdates(manual: false)
        }
        guard updatePollTask == nil else {
            return
        }
        updatePollTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: Self.updatePollIntervalNanoseconds)
                guard let self, !Task.isCancelled else {
                    return
                }
                guard self.autoCheckUpdates else {
                    self.stopAutomaticUpdateChecks()
                    return
                }
                self.checkForUpdates(manual: false)
            }
        }
    }

    private func stopAutomaticUpdateChecks() {
        updatePollTask?.cancel()
        updatePollTask = nil
    }

    func installUpdate() {
        guard let updateAssetUrl else {
            updateStatus = "No macOS update asset found"
            return
        }
        guard !updateInstalling else {
            return
        }
        updateInstalling = true
        updateStatus = "Downloading \(updateVersion)"
        Task { [weak self] in
            guard let self else {
                return
            }
            do {
                let savedUrl = try await self.downloadUpdateAsset(from: updateAssetUrl)
                try await MainActor.run {
                    try self.installDownloadedUpdate(savedUrl)
                }
            } catch {
                await MainActor.run {
                    self.updateInstalling = false
                    self.updateStatus = error.localizedDescription
                }
            }
        }
    }

    private func fetchUpdateCheck() async throws -> UpdateCheck {
        let manifestUrls = await MainActor.run { self.updateManifestUrls }
        var lastError: Error?
        for manifestUrl in manifestUrls {
            do {
                let data = try await loadUpdateData(from: manifestUrl)
                let manifest = try JSONDecoder().decode(ReleaseManifest.self, from: data)
                let currentVersion = await MainActor.run { self.state.appVersion }
                let asset = manifest.preferredMacAsset()
                let assetUrl = asset.flatMap { URL(string: $0.path, relativeTo: manifestUrl)?.absoluteURL }
                return UpdateCheck(
                    manifest: manifest,
                    asset: asset,
                    assetUrl: assetUrl,
                    isNewer: versionIsNewer(manifest.tag, than: currentVersion)
                )
            } catch {
                lastError = error is DecodingError ? UpdateFetchError.malformedManifest : error
            }
        }
        throw lastError ?? UpdateFetchError.malformedManifest
    }

    @MainActor
    private func applyUpdateCheck(_ check: UpdateCheck, manual: Bool, allowAutoInstall: Bool = true) {
        updateChecking = false
        updateAvailable = check.isNewer
        updateVersion = check.manifest.tag
        updateAssetUrl = check.isNewer ? check.assetUrl : nil
        if check.isNewer {
            updateStatus = check.assetUrl == nil ? "Update \(check.manifest.tag) found without a macOS asset" : "Update \(check.manifest.tag) available"
            if allowAutoInstall, autoInstallUpdates, check.assetUrl != nil {
                installUpdate()
            }
        } else if manual {
            updateStatus = "Up to date"
        } else {
            updateStatus = ""
        }
    }

    private func downloadUpdateAsset(from assetUrl: URL) async throws -> URL {
        let downloadedUrl: URL
        if assetUrl.isFileURL {
            downloadedUrl = FileManager.default.temporaryDirectory
                .appendingPathComponent("nostr-vpn-update-download-\(UUID().uuidString)")
            try FileManager.default.copyItem(at: assetUrl, to: downloadedUrl)
        } else {
            (downloadedUrl, _) = try await URLSession.shared.download(from: assetUrl)
        }
        return try moveDownloadedUpdate(downloadedUrl, from: assetUrl)
    }

    private func decodeQrCode(from url: URL) throws -> String {
        guard let image = CIImage(contentsOf: url) else {
            throw QrImportError.unreadableImage
        }
        let detector = CIDetector(
            ofType: CIDetectorTypeQRCode,
            context: nil,
            options: [CIDetectorAccuracy: CIDetectorAccuracyHigh]
        )
        let features = detector?.features(in: image) ?? []
        for feature in features {
            if let qr = feature as? CIQRCodeFeature,
               let message = qr.messageString?.trimmingCharacters(in: .whitespacesAndNewlines),
               !message.isEmpty {
                return message
            }
        }
        throw QrImportError.noQrCode
    }

    private func drainStartupUrls() {
        guard !startupUrlsDrained else {
            return
        }
        startupUrlsDrained = true
        for argument in CommandLine.arguments where argument.starts(with: "nvpn://") {
            if let url = URL(string: argument) {
                handle(url: url)
            }
        }
    }

    private func configureLaunchAgent(enabled: Bool) throws {
        let manager = FileManager.default
        let agentsDir = manager.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/LaunchAgents", isDirectory: true)
        let plistUrl = agentsDir.appendingPathComponent("to.iris.nvpn.macos.plist")
        if enabled {
            guard let executable = Bundle.main.executableURL?.path else {
                throw LaunchAgentError.missingExecutable
            }
            try manager.createDirectory(at: agentsDir, withIntermediateDirectories: true)
            try launchAgentPlist(executable: executable).write(to: plistUrl, atomically: true, encoding: .utf8)
            _ = runLaunchctl(["bootstrap", "gui/\(getuid())", plistUrl.path])
        } else {
            _ = runLaunchctl(["bootout", "gui/\(getuid())", plistUrl.path])
            if manager.fileExists(atPath: plistUrl.path) {
                try manager.removeItem(at: plistUrl)
            }
        }
    }

    private func installDownloadedUpdate(_ archiveUrl: URL) throws {
        updateStatus = "Installing \(updateVersion)"
        if archiveUrl.lastPathComponent.hasSuffix(".app.tar.gz") {
            let unpackDir = FileManager.default.temporaryDirectory
                .appendingPathComponent("NostrVpnUpdate-\(UUID().uuidString)", isDirectory: true)
            try FileManager.default.createDirectory(at: unpackDir, withIntermediateDirectories: true)
            try runProcess("/usr/bin/tar", arguments: ["-xzf", archiveUrl.path, "-C", unpackDir.path])
            guard let newApp = findAppBundle(in: unpackDir) else {
                throw UpdateError.missingAppBundle
            }
            let script = try updateInstallScript()
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/bin/sh")
            process.arguments = [script.path, Bundle.main.bundleURL.path, newApp.path]
            try process.run()
            NSApp.terminate(nil)
        } else {
            NSWorkspace.shared.activateFileViewerSelecting([archiveUrl])
            updateInstalling = false
            updateStatus = "Downloaded \(archiveUrl.lastPathComponent)"
        }
    }

    private func startServiceSettlementPolling() {
        serviceSettlementTask?.cancel()
        serviceSettling = true
        serviceSettlementTask = Task { [weak self] in
            for _ in 0..<8 {
                guard !Task.isCancelled else {
                    return
                }
                try? await Task.sleep(nanoseconds: 700_000_000)
                guard let self else {
                    return
                }
                let app = await MainActor.run { self.app }
                let nextState = await Task.detached {
                    app.refresh()
                }.value
                await MainActor.run {
                    self.state = nextState
                    self.maybePromptServiceUpdate(nextState)
                }
            }
            await MainActor.run {
                self?.serviceSettling = false
            }
        }
    }

    private func clearActionStatus(after seconds: UInt64) {
        let statusToClear = actionStatus
        actionStatusClearTask?.cancel()
        actionStatusClearTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: seconds * 1_000_000_000)
            await MainActor.run {
                guard self?.actionStatus == statusToClear else {
                    return
                }
                self?.actionStatus = ""
            }
        }
    }

    private func maybePromptServiceUpdate(_ nextState: NativeAppState) {
        guard Self.serviceUpdateRecommended(in: nextState), !actionInFlight else {
            return
        }
        if actionStatus.isEmpty {
            actionStatus = "Background service needs update"
        }
    }

    private static func serviceUpdateRecommended(in state: NativeAppState) -> Bool {
        state.serviceInstalled
            && !state.serviceBinaryVersion.isEmpty
            && !state.expectedServiceBinaryVersion.isEmpty
            && state.serviceBinaryVersion != state.expectedServiceBinaryVersion
    }
}

struct ReleaseManifest: Decodable {
    let tag: String
    let assets: [ReleaseAsset]

    private enum CodingKeys: String, CodingKey {
        case tag
        case tagName = "tag_name"
        case assets
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        if let tag = try container.decodeIfPresent(String.self, forKey: .tag) {
            self.tag = tag
        } else if let tag = try container.decodeIfPresent(String.self, forKey: .tagName) {
            self.tag = tag
        } else {
            throw DecodingError.keyNotFound(
                CodingKeys.tag,
                DecodingError.Context(codingPath: decoder.codingPath, debugDescription: "Missing release tag")
            )
        }
        self.assets = try container.decode([ReleaseAsset].self, forKey: .assets)
    }

    func preferredMacAsset() -> ReleaseAsset? {
        assets.first { $0.name.hasSuffix("-macos-arm64.app.tar.gz") }
            ?? assets.first { $0.name.hasSuffix("-macos-arm64.dmg") }
            ?? assets.first { $0.name.hasSuffix("-macos-arm64.zip") }
    }
}

struct ReleaseAsset: Decodable {
    let name: String
    let path: String

    private enum CodingKeys: String, CodingKey {
        case name
        case path
        case browserDownloadUrl = "browser_download_url"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.name = try container.decode(String.self, forKey: .name)
        if let path = try container.decodeIfPresent(String.self, forKey: .path) {
            self.path = path
        } else if let path = try container.decodeIfPresent(String.self, forKey: .browserDownloadUrl) {
            self.path = path
        } else {
            throw DecodingError.keyNotFound(
                CodingKeys.path,
                DecodingError.Context(codingPath: decoder.codingPath, debugDescription: "Missing release asset URL")
            )
        }
    }
}

struct UpdateCheck {
    let manifest: ReleaseManifest
    let asset: ReleaseAsset?
    let assetUrl: URL?
    let isNewer: Bool
}

enum CopyValue {
    case pubkey
    case meshId
    case invite
    case peerNpub
}

enum QrImportError: LocalizedError {
    case unreadableImage
    case noQrCode

    var errorDescription: String? {
        switch self {
        case .unreadableImage:
            return "Could not read the selected image."
        case .noQrCode:
            return "No QR invite was found in the selected image."
        }
    }
}

enum LaunchAgentError: LocalizedError {
    case missingExecutable

    var errorDescription: String? {
        switch self {
        case .missingExecutable:
            return "App executable was not found."
        }
    }
}

enum UpdateError: LocalizedError {
    case missingAppBundle

    var errorDescription: String? {
        switch self {
        case .missingAppBundle:
            return "Downloaded update did not contain Nostr VPN.app."
        }
    }
}

enum UpdateFetchError: LocalizedError {
    case httpStatus(code: Int)
    case malformedManifest

    var errorDescription: String? {
        switch self {
        case .httpStatus(let code):
            if code == 404 {
                return "No release manifest published yet."
            }
            return "Update server returned HTTP \(code)."
        case .malformedManifest:
            return "Update manifest is not valid JSON."
        }
    }
}

enum UpdateE2EError: LocalizedError {
    case noAsset

    var errorDescription: String? {
        switch self {
        case .noAsset:
            return "No macOS update asset was selected."
        }
    }
}

func runUpdateE2ECommandIfRequested() {
    let arguments = Set(CommandLine.arguments)
    guard arguments.contains("--nvpn-e2e-update-check") else {
        return
    }

    let result = runUpdateE2ECommand(install: arguments.contains("--nvpn-e2e-install-update"))
    writeUpdateE2EResult(result)
    exit((result["ok"] as? Bool) == true ? EXIT_SUCCESS : EXIT_FAILURE)
}

private func runUpdateE2ECommand(install: Bool) -> [String: Any] {
    do {
        let (manifestUrl, data) = try loadConfiguredUpdateManifestBlocking()
        let manifest = try JSONDecoder().decode(ReleaseManifest.self, from: data)
        let currentVersion = ProcessInfo.processInfo.environment["NVPN_UPDATE_E2E_CURRENT_VERSION"]
            ?? Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String
            ?? ""
        let asset = manifest.preferredMacAsset()
        let assetUrl = asset.flatMap { URL(string: $0.path, relativeTo: manifestUrl)?.absoluteURL }
        let isNewer = versionIsNewer(manifest.tag, than: currentVersion)
        var downloadedPath: String?
        var preparedAppPath: String?

        if install {
            guard let assetUrl else {
                throw UpdateE2EError.noAsset
            }
            let savedUrl = try downloadUpdateAssetBlocking(from: assetUrl)
            downloadedPath = savedUrl.path
            preparedAppPath = try prepareDownloadedUpdateForE2E(savedUrl)?.path
        }

        return [
            "ok": true,
            "platform": "macos",
            "available": isNewer,
            "tag": manifest.tag,
            "assetName": asset?.name ?? NSNull(),
            "assetUrl": assetUrl?.absoluteString ?? NSNull(),
            "downloadedPath": downloadedPath ?? NSNull(),
            "preparedAppPath": preparedAppPath ?? NSNull()
        ]
    } catch {
        return [
            "ok": false,
            "platform": "macos",
            "error": error.localizedDescription
        ]
    }
}

private func configuredUpdateManifestUrls() -> [URL] {
    if let overrideUrl = ProcessInfo.processInfo.environment["NVPN_UPDATE_MANIFEST_URL"]
        .flatMap(URL.init(string:)) {
        return [overrideUrl]
    }
    return [githubUpdateManifestUrl, defaultUpdateManifestUrl]
}

private func loadConfiguredUpdateManifestBlocking() throws -> (URL, Data) {
    var lastError: Error?
    for manifestUrl in configuredUpdateManifestUrls() {
        do {
            return (manifestUrl, try loadUpdateDataBlocking(from: manifestUrl))
        } catch {
            lastError = error
        }
    }
    throw lastError ?? UpdateFetchError.malformedManifest
}

private func loadUpdateDataBlocking(from url: URL) throws -> Data {
    if url.isFileURL {
        return try Data(contentsOf: url)
    }
    let request = updateURLRequest(for: url)
    let semaphore = DispatchSemaphore(value: 0)
    var resultData: Data?
    var resultResponse: URLResponse?
    var resultError: Error?
    let session = URLSession(configuration: updateURLSessionConfiguration())
    let task = session.dataTask(with: request) { data, response, error in
        resultData = data
        resultResponse = response
        resultError = error
        semaphore.signal()
    }
    task.resume()
    if semaphore.wait(timeout: .now() + updateRequestTimeout) == .timedOut {
        task.cancel()
        throw URLError(.timedOut)
    }
    if let resultError {
        throw resultError
    }
    if let http = resultResponse as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
        throw UpdateFetchError.httpStatus(code: http.statusCode)
    }
    return resultData ?? Data()
}

private func downloadUpdateAssetBlocking(from assetUrl: URL) throws -> URL {
    let downloadedUrl = FileManager.default.temporaryDirectory
        .appendingPathComponent("nostr-vpn-update-download-\(UUID().uuidString)")
    if assetUrl.isFileURL {
        try FileManager.default.copyItem(at: assetUrl, to: downloadedUrl)
    } else {
        let data = try Data(contentsOf: assetUrl)
        try data.write(to: downloadedUrl)
    }
    return try moveDownloadedUpdate(downloadedUrl, from: assetUrl)
}

private func prepareDownloadedUpdateForE2E(_ archiveUrl: URL) throws -> URL? {
    guard archiveUrl.lastPathComponent.hasSuffix(".app.tar.gz") else {
        return nil
    }
    let unpackDir = FileManager.default.temporaryDirectory
        .appendingPathComponent("NostrVpnUpdateE2E-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: unpackDir, withIntermediateDirectories: true)
    try runProcess("/usr/bin/tar", arguments: ["-xzf", archiveUrl.path, "-C", unpackDir.path])
    guard let appBundle = findAppBundle(in: unpackDir) else {
        throw UpdateError.missingAppBundle
    }
    return appBundle
}

private func writeUpdateE2EResult(_ result: [String: Any]) {
    let data = (try? JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys]))
        ?? Data("{}".utf8)
    if let path = ProcessInfo.processInfo.environment["NVPN_UPDATE_E2E_RESULT_PATH"], !path.isEmpty {
        let url = URL(fileURLWithPath: path)
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? data.write(to: url)
    } else if let text = String(data: data, encoding: .utf8) {
        print(text)
    }
}

private func queryValue(_ name: String, in url: URL) -> String? {
    URLComponents(url: url, resolvingAgainstBaseURL: false)?
        .queryItems?
        .first(where: { $0.name == name })?
        .value?
        .trimmingCharacters(in: .whitespacesAndNewlines)
}

private func launchAgentPlist(executable: String) -> String {
    """
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
    <plist version="1.0">
    <dict>
        <key>Label</key>
        <string>to.iris.nvpn.macos</string>
        <key>ProgramArguments</key>
        <array>
            <string>\(xmlEscaped(executable))</string>
            <string>--autostart</string>
        </array>
        <key>RunAtLoad</key>
        <true/>
    </dict>
    </plist>
    """
}

private func xmlEscaped(_ value: String) -> String {
    value
        .replacingOccurrences(of: "&", with: "&amp;")
        .replacingOccurrences(of: "<", with: "&lt;")
        .replacingOccurrences(of: ">", with: "&gt;")
        .replacingOccurrences(of: "\"", with: "&quot;")
}

private func runLaunchctl(_ arguments: [String]) -> Bool {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
    process.arguments = arguments
    do {
        try process.run()
        process.waitUntilExit()
        return process.terminationStatus == 0
    } catch {
        return false
    }
}

private func moveDownloadedUpdate(_ downloadedUrl: URL, from assetUrl: URL) throws -> URL {
    let fileName = assetUrl.lastPathComponent.isEmpty ? "nostr-vpn-update" : assetUrl.lastPathComponent
    let destination = FileManager.default.temporaryDirectory
        .appendingPathComponent("NostrVpnDownloads", isDirectory: true)
        .appendingPathComponent(fileName)
    try FileManager.default.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
    if FileManager.default.fileExists(atPath: destination.path) {
        try FileManager.default.removeItem(at: destination)
    }
    try FileManager.default.moveItem(at: downloadedUrl, to: destination)
    return destination
}

private func loadUpdateData(from url: URL) async throws -> Data {
    if url.isFileURL {
        return try Data(contentsOf: url)
    }
    let session = URLSession(configuration: updateURLSessionConfiguration())
    let (data, response) = try await session.data(for: updateURLRequest(for: url))
    if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
        throw UpdateFetchError.httpStatus(code: http.statusCode)
    }
    return data
}

private func updateURLSessionConfiguration() -> URLSessionConfiguration {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.timeoutIntervalForRequest = updateRequestTimeout
    configuration.timeoutIntervalForResource = updateRequestTimeout
    return configuration
}

private func updateURLRequest(for url: URL) -> URLRequest {
    var request = URLRequest(url: url, timeoutInterval: updateRequestTimeout)
    request.httpMethod = "GET"
    if url.host == "api.github.com" {
        request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        request.setValue(updateUserAgent, forHTTPHeaderField: "User-Agent")
    }
    return request
}

private func versionIsNewer(_ candidate: String, than current: String) -> Bool {
    let left = versionParts(candidate)
    let right = versionParts(current)
    for index in 0..<max(left.count, right.count) {
        let leftValue = index < left.count ? left[index] : 0
        let rightValue = index < right.count ? right[index] : 0
        if leftValue != rightValue {
            return leftValue > rightValue
        }
    }
    return false
}

private func versionParts(_ value: String) -> [Int] {
    value
        .trimmingCharacters(in: CharacterSet(charactersIn: "vV "))
        .split { !$0.isNumber }
        .map { Int($0) ?? 0 }
}

private func runProcess(_ executable: String, arguments: [String]) throws {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: executable)
    process.arguments = arguments
    try process.run()
    process.waitUntilExit()
    if process.terminationStatus != 0 {
        throw CocoaError(.executableLoad)
    }
}

private func findAppBundle(in directory: URL) -> URL? {
    guard let enumerator = FileManager.default.enumerator(
        at: directory,
        includingPropertiesForKeys: [.isDirectoryKey],
        options: [.skipsHiddenFiles]
    ) else {
        return nil
    }
    for case let url as URL in enumerator where url.pathExtension == "app" {
        if url.lastPathComponent == "Nostr VPN.app" || url.lastPathComponent == "NostrVpnMac.app" {
            return url
        }
    }
    return nil
}

private func updateInstallScript() throws -> URL {
    let script = FileManager.default.temporaryDirectory
        .appendingPathComponent("nostr-vpn-install-update-\(UUID().uuidString).sh")
    let contents = """
    #!/bin/sh
    set -eu
    current_app="$1"
    new_app="$2"
    sleep 1
    rm -rf "$current_app"
    ditto "$new_app" "$current_app"
    open "$current_app"
    """
    try contents.write(to: script, atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
    return script
}

func settingsPatch(
    nodeName: String? = nil,
    endpoint: String? = nil,
    tunnelIp: String? = nil,
    listenPort: UInt16? = nil,
    exitNode: String? = nil,
    exitNodeLeakProtection: Bool? = nil,
    advertiseExitNode: Bool? = nil,
    advertisedRoutes: String? = nil,
    wireguardExitEnabled: Bool? = nil,
    wireguardExitInterface: String? = nil,
    wireguardExitAddress: String? = nil,
    wireguardExitPrivateKey: String? = nil,
    wireguardExitPeerPublicKey: String? = nil,
    wireguardExitPeerPresharedKey: String? = nil,
    wireguardExitEndpoint: String? = nil,
    wireguardExitAllowedIps: String? = nil,
    wireguardExitDns: String? = nil,
    wireguardExitMtu: UInt16? = nil,
    wireguardExitPersistentKeepaliveSecs: UInt16? = nil,
    wireguardExitConfig: String? = nil,
    magicDnsSuffix: String? = nil,
    autoconnect: Bool? = nil,
    launchOnStartup: Bool? = nil,
    closeToTrayOnClose: Bool? = nil
) -> SettingsPatch {
    SettingsPatch(
        nodeName: nodeName,
        endpoint: endpoint,
        tunnelIp: tunnelIp,
        listenPort: listenPort,
        exitNode: exitNode,
        exitNodeLeakProtection: exitNodeLeakProtection,
        advertiseExitNode: advertiseExitNode,
        advertisedRoutes: advertisedRoutes,
        wireguardExitEnabled: wireguardExitEnabled,
        wireguardExitInterface: wireguardExitInterface,
        wireguardExitAddress: wireguardExitAddress,
        wireguardExitPrivateKey: wireguardExitPrivateKey,
        wireguardExitPeerPublicKey: wireguardExitPeerPublicKey,
        wireguardExitPeerPresharedKey: wireguardExitPeerPresharedKey,
        wireguardExitEndpoint: wireguardExitEndpoint,
        wireguardExitAllowedIps: wireguardExitAllowedIps,
        wireguardExitDns: wireguardExitDns,
        wireguardExitMtu: wireguardExitMtu,
        wireguardExitPersistentKeepaliveSecs: wireguardExitPersistentKeepaliveSecs,
        wireguardExitConfig: wireguardExitConfig,
        magicDnsSuffix: magicDnsSuffix,
        autoconnect: autoconnect,
        launchOnStartup: launchOnStartup,
        closeToTrayOnClose: closeToTrayOnClose
    )
}
