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

    private let app: FfiApp?
    private let fixtureMode: Bool
    private var refreshTask: Task<Void, Never>?
    private var copyClearTask: Task<Void, Never>?
    private var actionStatusClearTask: Task<Void, Never>?
    private var serviceSettlementTask: Task<Void, Never>?
    private var updateTask: Task<Void, Never>?
    private var updatePollTask: Task<Void, Never>?
    private var refreshInFlight = false
    private var refreshPending = false
    private var startupUrlsDrained = false
    private var startupUpdateCheckDone = false
    private var updateAssetUrl: URL?
    private var updateUsesCoreDownload = false
    private let updateManifestUrls: [URL] = {
        if let overrideUrl = ProcessInfo.processInfo.environment["NVPN_UPDATE_MANIFEST_URL"]
            .flatMap(URL.init(string:)) {
            return [overrideUrl]
        }
        return [defaultUpdateManifestUrl, githubUpdateManifestUrl]
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
        let fixtureMode = Self.fixtureModeRequested()
        self.fixtureMode = fixtureMode
        self.launchedHidden = CommandLine.arguments.contains("--hidden") && !fixtureMode
        if fixtureMode {
            self.app = nil
            self.state = Self.screenshotFixtureState()
            self.autoCheckUpdates = false
            return
        }

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
    }

    var activeNetwork: NativeNetworkState? {
        state.networks.first(where: { $0.enabled })
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
        updateAvailable && updateUsesCoreDownload && !updateChecking && !updateInstalling
    }

    func start() {
        drainStartupUrls()
        guard !fixtureMode else {
            return
        }
        syncLaunchAgentWithSettings()
        startAutomaticUpdateChecks()
        guard refreshTask == nil else {
            return
        }
        refreshTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                await self?.performRefresh()
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
        Task { @MainActor [weak self] in
            await self?.performRefresh()
        }
    }

    private func performRefresh() async {
        guard let app else {
            return
        }
        if refreshInFlight {
            refreshPending = true
            return
        }
        refreshInFlight = true
        defer {
            refreshInFlight = false
        }

        repeat {
            refreshPending = false
            let nextState = await Task.detached {
                app.refresh()
            }.value
            guard !Task.isCancelled else {
                return
            }
            state = nextState
            maybePromptServiceUpdate(nextState)
        } while refreshPending && !Task.isCancelled
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
        guard let app else {
            actionStatus = successStatus
            if !successStatus.isEmpty {
                clearActionStatus(after: 3)
            }
            return
        }
        actionStatusClearTask?.cancel()
        actionInFlight = true
        actionStatus = status
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
        let item: Any = URL(string: value) ?? value
        let picker = NSSharingServicePicker(items: [item])
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

    func chooseWireGuardConfigFile() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.plainText, .data, .item]
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.begin { [weak self] response in
            guard response == .OK, let url = panel.url else {
                return
            }
            Task { @MainActor in
                self?.importWireGuardConfigFile(url)
            }
        }
    }

    func importWireGuardConfigFile(_ url: URL) {
        do {
            let didStartAccess = url.startAccessingSecurityScopedResource()
            defer {
                if didStartAccess {
                    url.stopAccessingSecurityScopedResource()
                }
            }
            let config = try String(contentsOf: url, encoding: .utf8)
            guard !config.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                actionStatus = "Selected WireGuard config is empty."
                return
            }
            saveWireGuardExitConfig(config)
        } catch {
            actionStatus = error.localizedDescription
        }
    }

    func saveDeviceSettings(
        nodeName: String,
        endpoint: String,
        tunnelIp: String,
        listenPort: String
    ) {
        let parsedPort = UInt16(listenPort.trimmingCharacters(in: .whitespacesAndNewlines))
        dispatch(.updateSettings(patch: settingsPatch(
            nodeName: nodeName,
            endpoint: endpoint,
            tunnelIp: tunnelIp,
            listenPort: parsedPort
        )), status: "Saving device settings")
    }

    func saveFipsHostInboundTcpPorts(_ value: String) {
        dispatch(
            .updateSettings(patch: settingsPatch(fipsHostInboundTcpPorts: value)),
            status: "Saving FIPS option"
        )
    }

    @discardableResult
    func addRelay(_ value: String) -> Bool {
        guard let url = Self.normalizedRelayUrl(value) else {
            return false
        }
        var lists = relayLists()
        lists.disabled.removeAll { $0 == url }
        if !lists.enabled.contains(url) {
            lists.enabled.append(url)
        }
        saveRelayLists(lists)
        return true
    }

    func setRelay(_ url: String, enabled: Bool) {
        guard let url = Self.normalizedRelayUrl(url) else {
            return
        }
        var lists = relayLists()
        lists.enabled.removeAll { $0 == url }
        lists.disabled.removeAll { $0 == url }
        if enabled {
            lists.enabled.append(url)
        } else {
            lists.disabled.append(url)
        }
        saveRelayLists(lists)
    }

    func deleteRelay(_ url: String) {
        guard let url = Self.normalizedRelayUrl(url) else {
            return
        }
        var lists = relayLists()
        lists.enabled.removeAll { $0 == url }
        lists.disabled.removeAll { $0 == url }
        saveRelayLists(lists)
    }

    private func relayLists() -> (enabled: [String], disabled: [String]) {
        let enabled = state.relays
            .filter(\.enabled)
            .compactMap { Self.normalizedRelayUrl($0.url) }
        let disabled = state.relays
            .filter { !$0.enabled }
            .compactMap { Self.normalizedRelayUrl($0.url) }
        return (
            enabled: Self.uniqueRelayList(enabled),
            disabled: Self.uniqueRelayList(disabled).filter { !enabled.contains($0) }
        )
    }

    private func saveRelayLists(_ lists: (enabled: [String], disabled: [String])) {
        let enabled = Self.uniqueRelayList(lists.enabled)
        let disabled = Self.uniqueRelayList(lists.disabled).filter { !enabled.contains($0) }
        dispatch(
            .updateSettings(patch: settingsPatch(relays: enabled, disabledRelays: disabled)),
            status: "Saving relays"
        )
    }

    private static func normalizedRelayUrl(_ value: String) -> String? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return nil
        }
        if trimmed.hasPrefix("ws://") || trimmed.hasPrefix("wss://") {
            return trimmed
        }
        return "wss://\(trimmed)"
    }

    private static func uniqueRelayList(_ values: [String]) -> [String] {
        var seen = Set<String>()
        return values.filter { seen.insert($0).inserted }
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

    func setFipsHostTunnel(_ enabled: Bool) {
        dispatch(.updateSettings(patch: settingsPatch(fipsHostTunnelEnabled: enabled)), status: "Saving FIPS option")
    }

    func setConnectToNonRosterFipsPeers(_ enabled: Bool) {
        dispatch(.updateSettings(patch: settingsPatch(connectToNonRosterFipsPeers: enabled)), status: "Saving FIPS option")
    }

    func setFipsNostrDiscoveryEnabled(_ enabled: Bool) {
        dispatch(.updateSettings(patch: settingsPatch(fipsNostrDiscoveryEnabled: enabled)), status: "Saving FIPS option")
    }

    func setFipsBootstrapEnabled(_ enabled: Bool) {
        dispatch(.updateSettings(patch: settingsPatch(fipsBootstrapEnabled: enabled)), status: "Saving FIPS option")
    }

    func setLaunchOnStartup(_ enabled: Bool) {
        do {
            try configureLaunchAgent(enabled: enabled, loadCurrentSession: true)
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

    func resetNetworkInvite(networkId: String) {
        dispatch(.resetNetworkInvite(networkId: networkId), status: "Resetting invite")
    }

    func requestNetworkJoin(networkId: String) {
        dispatch(.requestNetworkJoin(networkId: networkId), status: "Requesting network join")
    }

    func acceptJoinRequest(networkId: String, requesterNpub: String) {
        dispatch(.acceptJoinRequest(networkId: networkId, requesterNpub: requesterNpub), status: "Accepting join request")
    }

    func rejectJoinRequest(networkId: String, requesterNpub: String) {
        dispatch(.rejectJoinRequest(networkId: networkId, requesterNpub: requesterNpub), status: "Rejecting join request")
    }

    func setParticipantAlias(npub: String, alias: String) {
        dispatch(.setParticipantAlias(npub: npub, alias: alias), status: "Saving alias")
    }

    func setParticipantEndpointHints(npub: String, endpointHints: [String]) {
        dispatch(
            .setParticipantEndpointHints(npub: npub, endpointHints: endpointHints),
            status: "Saving address hints"
        )
    }

    func toggleAdmin(networkId: String, participant: NativeParticipantState) {
        if participant.isAdmin {
            dispatch(.removeAdmin(networkId: networkId, npub: participant.npub), status: "Removing admin")
        } else {
            dispatch(.addAdmin(networkId: networkId, npub: participant.npub), status: "Adding admin")
        }
    }

    func removeParticipant(networkId: String, npub: String) {
        dispatch(.removeParticipant(networkId: networkId, npub: npub), status: "Removing device")
    }

    func addNetwork(_ name: String) {
        dispatch(.addNetwork(name: name.trimmingCharacters(in: .whitespacesAndNewlines)), status: "Adding network")
    }

    func manualAddNetwork(adminNpub: String, meshNetworkId: String) {
        let admin = adminNpub.trimmingCharacters(in: .whitespacesAndNewlines)
        let mesh = normalizeNetworkIdInput(meshNetworkId)
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
        dispatch(.startInviteBroadcast, status: "Sharing nearby")
    }

    func stopInviteBroadcast() {
        dispatch(.stopInviteBroadcast, status: "Stopped nearby sharing")
    }

    func startNearbyDiscovery() {
        dispatch(.startNearbyDiscovery, status: "Finding nearby")
    }

    func stopNearbyDiscovery() {
        dispatch(.stopNearbyDiscovery, status: "Stopped looking")
    }

    func checkForUpdates(manual: Bool = true) {
        guard !fixtureMode else {
            updateStatus = manual ? "Fixture mode" : ""
            return
        }
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
                    self.updateUsesCoreDownload = false
                    self.updateAssetUrl = nil
                    if manual {
                        self.updateStatus = error.localizedDescription
                    }
                }
            }
        }
    }

    private func startAutomaticUpdateChecks() {
        guard !fixtureMode else {
            return
        }
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
        if updateUsesCoreDownload {
            installCoreDownloadedUpdate()
            return
        }
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
        let currentVersion = await MainActor.run { self.state.appVersion }
        let result = try await runCoreUpdateCheck(currentVersion: currentVersion)
        let asset = result.asset.isEmpty ? nil : ReleaseAsset(name: result.asset, path: result.url ?? "")
        return UpdateCheck(
            manifest: ReleaseManifest(tag: result.tag, assets: asset.map { [$0] } ?? []),
            asset: asset,
            assetUrl: nil,
            isNewer: result.available,
            usesCoreDownload: result.available && asset != nil
        )
    }

    @MainActor
    private func applyUpdateCheck(_ check: UpdateCheck, manual: Bool, allowAutoInstall: Bool = true) {
        updateChecking = false
        updateAvailable = check.isNewer
        updateVersion = check.manifest.tag
        updateUsesCoreDownload = check.isNewer && check.usesCoreDownload
        updateAssetUrl = check.isNewer ? check.assetUrl : nil
        if check.isNewer {
            let hasAsset = check.usesCoreDownload
            updateStatus = hasAsset ? "Update \(check.manifest.tag) available" : "Update \(check.manifest.tag) found without a macOS asset"
            if allowAutoInstall, autoInstallUpdates, hasAsset {
                installUpdate()
            }
        } else if manual {
            updateStatus = "Up to date"
        } else {
            updateStatus = ""
        }
    }

    private func installCoreDownloadedUpdate() {
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
                let downloadDir = FileManager.default.temporaryDirectory
                    .appendingPathComponent("NostrVpnDownloads", isDirectory: true)
                let currentVersion = await MainActor.run { self.state.appVersion }
                let result = try await runCoreUpdateDownload(
                    currentVersion: currentVersion,
                    downloadDir: downloadDir
                )
                guard let path = result.path, !path.isEmpty else {
                    throw UpdateError.missingDownloadedPath
                }
                try await MainActor.run {
                    try self.installDownloadedUpdate(URL(fileURLWithPath: path))
                }
            } catch {
                await MainActor.run {
                    self.updateInstalling = false
                    self.updateStatus = error.localizedDescription
                }
            }
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

    private func syncLaunchAgentWithSettings() {
        do {
            try configureLaunchAgent(
                enabled: state.startupSettingsSupported && state.launchOnStartup,
                loadCurrentSession: false
            )
        } catch {
            actionStatus = error.localizedDescription
        }
    }

    private func configureLaunchAgent(enabled: Bool, loadCurrentSession: Bool) throws {
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
            if loadCurrentSession {
                _ = runLaunchctl(["bootstrap", "gui/\(getuid())", plistUrl.path])
            }
        } else {
            if loadCurrentSession {
                _ = runLaunchctl(["bootout", "gui/\(getuid())", plistUrl.path])
            }
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
                await self.performRefresh()
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

    private static func fixtureModeRequested() -> Bool {
        let arguments = Set(CommandLine.arguments)
        if arguments.contains("--nvpn-fixture-mode") || arguments.contains("--nvpn-screenshot-fixture") {
            return true
        }
        let raw = ProcessInfo.processInfo.environment["NVPN_MACOS_FIXTURE_MODE"] ?? ""
        return ["1", "true", "yes", "on"].contains(raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased())
    }

    private static func screenshotFixtureState() -> NativeAppState {
        let version = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "4.0.31"
        let selfNpub = "npub12mh8r6uetvj9fptwua9gtng0ycz8uvzn0vmxsaddk7vfegeu29ts3qngt3"
        let macbookNpub = "npub1q0n4g9trrsyqezfgrsg6txtedmhgv0h5apur2a4yxgq4z0kgejmsjeclxl"
        let iphoneNpub = "npub1tmhvkh3ktx7dw06fxntuzmvc9r20wxnrapzd056240s42jtpyzps645uyh"
        let androidNpub = "npub1h988xqzvhu98t0n22ys7etjcg7ca33s78kda4mu5pfathz9xklmqxx6rg0"
        let ubuntuNpub = "npub1dxlnwd78xjhec3hlzc8qwjuc98w47hv89khggu8yk0ynpkw4czxsuccs3u"
        let joinNpub = "npub1xnvumr6snuvl7tcwll3kz9wny4fzjh9uwhmu4d6hc2hwuxrk6vqq8kukg4"
        let nearbyNpub = "npub1saweqehm9a5gsn7xgcqjpmf2ungfj74wxawwvehn38s0mxtzx80qq8a5xa"
        let selfHex = "56ee71eb995b2454856ee74a85cd0f26047e30537b366875adb7989ca33c5157"
        let macbookHex = "03e75415631c080c89281c11a599796eee863ef4e8783576a43201513ec8ccb7"
        let iphoneHex = "5eeecb5e3659bcd73f4934d7c16d9828d4f71a63e844d7d34aabe15549612083"
        let androidHex = "b94e73004cbf0a75be6a5121ecae5847b1d8c61e3d9bdaef940a7abb88a6b7f6"
        let ubuntuHex = "69bf3737c734af9c46ff160e074b9829dd5f5d872dae8470e4b3c930d9d5c08d"
        let joinHex = "34d9cd8f509f19ff2f0effe36115d32552295cbc75f7cab757c2aeee1876d300"
        let networkId = "demo-mesh"

        let local = NativeParticipantState(
            npub: selfNpub,
            pubkeyHex: selfHex,
            alias: "mini",
            magicDnsAlias: "mini",
            magicDnsName: "mini.nvpn",
            tunnelIp: "10.44.195.20",
            isAdmin: true,
            reachable: true,
            txBytes: 842_112,
            rxBytes: 1_302_804,
            advertisedRoutes: [],
            offersExitNode: false,
            fipsEndpointNpub: selfNpub,
            fipsEndpointHints: [],
            fipsTransportAddr: "192.0.2.57:51820",
            fipsTransportType: "udp",
            fipsSrttMs: 0,
            fipsPacketsSent: 0,
            fipsPacketsRecv: 0,
            fipsBytesSent: 0,
            fipsBytesRecv: 0,
            fipsDirectProbePending: false,
            fipsDirectProbeAfterMs: 0,
            state: "local",
            meshState: "local",
            statusText: "local",
            lastSeenText: "self"
        )
        let macbook = NativeParticipantState(
            npub: macbookNpub,
            pubkeyHex: macbookHex,
            alias: "macbook",
            magicDnsAlias: "macbook",
            magicDnsName: "macbook.nvpn",
            tunnelIp: "10.44.178.166",
            isAdmin: false,
            reachable: true,
            txBytes: 290_144,
            rxBytes: 211_776,
            advertisedRoutes: [],
            offersExitNode: false,
            fipsEndpointNpub: macbookNpub,
            fipsEndpointHints: [],
            fipsTransportAddr: "",
            fipsTransportType: "mesh",
            fipsSrttMs: 38,
            fipsPacketsSent: 4_284,
            fipsPacketsRecv: 4_103,
            fipsBytesSent: 628_104,
            fipsBytesRecv: 602_920,
            fipsDirectProbePending: false,
            fipsDirectProbeAfterMs: 0,
            state: "online",
            meshState: "via mesh",
            statusText: "via mesh, 38 ms",
            lastSeenText: "now"
        )
        let iphone = NativeParticipantState(
            npub: iphoneNpub,
            pubkeyHex: iphoneHex,
            alias: "iphone",
            magicDnsAlias: "iphone",
            magicDnsName: "iphone.nvpn",
            tunnelIp: "10.44.181.12",
            isAdmin: false,
            reachable: true,
            txBytes: 118_272,
            rxBytes: 91_440,
            advertisedRoutes: [],
            offersExitNode: false,
            fipsEndpointNpub: iphoneNpub,
            fipsEndpointHints: [],
            fipsTransportAddr: "192.0.2.74:52283",
            fipsTransportType: "udp",
            fipsSrttMs: 9,
            fipsPacketsSent: 2_016,
            fipsPacketsRecv: 1_988,
            fipsBytesSent: 244_992,
            fipsBytesRecv: 239_872,
            fipsDirectProbePending: false,
            fipsDirectProbeAfterMs: 0,
            state: "online",
            meshState: "direct",
            statusText: "direct, 9 ms",
            lastSeenText: "1s ago"
        )
        let android = NativeParticipantState(
            npub: androidNpub,
            pubkeyHex: androidHex,
            alias: "android",
            magicDnsAlias: "android",
            magicDnsName: "android.nvpn",
            tunnelIp: "10.44.191.30",
            isAdmin: false,
            reachable: true,
            txBytes: 76_112,
            rxBytes: 64_928,
            advertisedRoutes: [],
            offersExitNode: false,
            fipsEndpointNpub: androidNpub,
            fipsEndpointHints: [],
            fipsTransportAddr: "192.0.2.92:47120",
            fipsTransportType: "udp",
            fipsSrttMs: 18,
            fipsPacketsSent: 1_104,
            fipsPacketsRecv: 1_079,
            fipsBytesSent: 130_288,
            fipsBytesRecv: 127_416,
            fipsDirectProbePending: false,
            fipsDirectProbeAfterMs: 0,
            state: "online",
            meshState: "direct",
            statusText: "direct, 18 ms",
            lastSeenText: "2s ago"
        )
        let ubuntu = NativeParticipantState(
            npub: ubuntuNpub,
            pubkeyHex: ubuntuHex,
            alias: "ubuntu",
            magicDnsAlias: "ubuntu",
            magicDnsName: "ubuntu.nvpn",
            tunnelIp: "10.44.202.44",
            isAdmin: false,
            reachable: true,
            txBytes: 5_820_192,
            rxBytes: 4_911_240,
            advertisedRoutes: ["10.88.0.0/16"],
            offersExitNode: true,
            fipsEndpointNpub: ubuntuNpub,
            fipsEndpointHints: [],
            fipsTransportAddr: "203.0.113.44:51820",
            fipsTransportType: "udp",
            fipsSrttMs: 22,
            fipsPacketsSent: 18_244,
            fipsPacketsRecv: 18_031,
            fipsBytesSent: 8_100_904,
            fipsBytesRecv: 7_900_512,
            fipsDirectProbePending: false,
            fipsDirectProbeAfterMs: 0,
            state: "online",
            meshState: "direct",
            statusText: "direct, 22 ms",
            lastSeenText: "3s ago"
        )
        let joinRequest = NativeInboundJoinRequestState(
            requesterNpub: joinNpub,
            requesterPubkeyHex: joinHex,
            requesterNodeName: "ipad",
            requestedAtText: "2m ago"
        )
        let network = NativeNetworkState(
            id: "demo",
            name: "Home Mesh",
            enabled: true,
            networkId: networkId,
            localIsAdmin: true,
            joinRequestsEnabled: true,
            inviteInviterNpub: "",
            adminNpubs: [selfNpub],
            outboundJoinRequest: nil,
            inboundJoinRequests: [joinRequest],
            onlineCount: 5,
            expectedCount: 5,
            admins: [selfNpub],
            participants: [local, macbook, iphone, android, ubuntu]
        )

        return NativeAppState(
            rev: 1,
            platform: "macos",
            mobile: false,
            vpnControlSupported: true,
            cliInstallSupported: true,
            startupSettingsSupported: true,
            trayBehaviorSupported: true,
            runtimeStatusDetail: "Screenshot fixture",
            appVersion: version,
            configPath: "/Users/demo/Library/Application Support/nvpn/config.toml",
            error: "",
            cliInstalled: true,
            serviceSupported: true,
            serviceEnablementSupported: true,
            serviceInstalled: true,
            serviceDisabled: false,
            serviceRunning: true,
            serviceStatusDetail: "Background service running (nvpn), pid 4242",
            daemonRunning: true,
            vpnEnabled: true,
            vpnActive: true,
            vpnStatus: "Connected",
            daemonBinaryVersion: version,
            serviceBinaryVersion: version,
            expectedServiceBinaryVersion: version,
            ownNpub: selfNpub,
            ownPubkeyHex: selfHex,
            nodeId: "mini",
            nodeName: "mini",
            selfMagicDnsName: "mini.nvpn",
            endpoint: "203.0.113.8:51820",
            tunnelIp: "10.44.195.20",
            listenPort: 51820,
            relays: [
                NativeRelayState(url: "wss://relay.damus.io", status: "connected", enabled: true),
                NativeRelayState(url: "wss://relay.nostr.band", status: "unknown", enabled: true)
            ],
            networkId: networkId,
            activeNetworkInvite: "nvpn://invite/demo-mesh",
            exitNode: "",
            exitNodeLeakProtection: true,
            exitNodeActive: false,
            exitNodeBlocked: false,
            exitNodeStatusText: "",
            advertiseExitNode: false,
            advertisedRoutes: [],
            effectiveAdvertisedRoutes: [],
            wireguardExitEnabled: false,
            wireguardExitConfigured: true,
            wireguardExitInterface: "utun-demo",
            wireguardExitAddress: "10.8.0.2/32",
            wireguardExitPrivateKey: "demo-private-key",
            wireguardExitPeerPublicKey: "demo-peer-key",
            wireguardExitPeerPresharedKey: "",
            wireguardExitEndpoint: "demo-wireguard.invalid:51820",
            wireguardExitAllowedIps: "0.0.0.0/0",
            wireguardExitDns: "1.1.1.1",
            wireguardExitMtu: 1280,
            wireguardExitPersistentKeepaliveSecs: 25,
            wireguardExitConfig: "",
            fipsHostTunnelEnabled: false,
            connectToNonRosterFipsPeers: true,
            fipsNostrDiscoveryEnabled: true,
            fipsBootstrapEnabled: false,
            fipsBootstrapPeers: [:],
            fipsBootstrapPeerDefaults: [:],
            fipsHostInboundTcpPorts: "",
            magicDnsSuffix: "nvpn",
            magicDnsStatus: "Serving .nvpn names",
            autoconnect: true,
            inviteBroadcastActive: true,
            inviteBroadcastRemainingSecs: 417,
            nearbyDiscoveryActive: true,
            nearbyDiscoveryRemainingSecs: 417,
            launchOnStartup: true,
            closeToTrayOnClose: true,
            connectedPeerCount: 4,
            expectedPeerCount: 4,
            fipsConnectedPeerCount: 4,
            fipsRosterPeerCount: 4,
            nonFipsRosterPeerCount: 0,
            meshReady: true,
            health: [],
            network: NativeNetworkSummary(
                defaultInterface: "en0",
                primaryIpv4: "192.0.2.57",
                primaryIpv6: "",
                gatewayIpv4: "192.0.2.1",
                gatewayIpv6: "",
                changedAt: 0,
                captivePortal: "false"
            ),
            portMapping: NativePortMappingStatus(
                upnp: NativeProbeStatus(state: "ok", detail: "mapped"),
                natPmp: NativeProbeStatus(state: "unknown", detail: ""),
                pcp: NativeProbeStatus(state: "unknown", detail: ""),
                activeProtocol: "upnp",
                externalEndpoint: "203.0.113.8:51820",
                gateway: "192.0.2.1",
                goodUntil: 0
            ),
            networks: [network],
            lanPeers: [
                NativeLanPeerState(
                    npub: nearbyNpub,
                    nodeName: "nearby-macbook",
                    endpoint: "192.0.2.44:51820",
                    networkName: "Nearby Mesh",
                    networkId: "nearby-mesh",
                    invite: "nvpn://invite/nearby-mesh",
                    lastSeenText: "3s ago"
                )
            ]
        )
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

    init(tag: String, assets: [ReleaseAsset]) {
        self.tag = tag
        self.assets = assets
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

    init(name: String, path: String) {
        self.name = name
        self.path = path
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
    let usesCoreDownload: Bool
}

struct CoreUpdateResult: Decodable {
    let available: Bool
    let tag: String
    let asset: String
    let source: String
    let verified: Bool
    let url: String?
    let path: String?
    let error: String?

    enum CodingKeys: String, CodingKey {
        case available
        case tag
        case asset
        case source
        case verified
        case url
        case path
        case error
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.available = try container.decodeIfPresent(Bool.self, forKey: .available) ?? false
        self.tag = try container.decodeIfPresent(String.self, forKey: .tag) ?? ""
        self.asset = try container.decodeIfPresent(String.self, forKey: .asset) ?? ""
        self.source = try container.decodeIfPresent(String.self, forKey: .source) ?? ""
        self.verified = try container.decodeIfPresent(Bool.self, forKey: .verified) ?? false
        self.url = try container.decodeIfPresent(String.self, forKey: .url)
        self.path = try container.decodeIfPresent(String.self, forKey: .path)
        self.error = try container.decodeIfPresent(String.self, forKey: .error)
    }
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
    case coreUpdaterFailed(String)
    case coreUpdaterOutputInvalid
    case missingDownloadedPath

    var errorDescription: String? {
        switch self {
        case .missingAppBundle:
            return "Downloaded update did not contain Nostr VPN.app."
        case .coreUpdaterFailed(let message):
            return message.isEmpty ? "Update check failed." : message
        case .coreUpdaterOutputInvalid:
            return "Updater returned invalid output."
        case .missingDownloadedPath:
            return "Updater did not return a downloaded file."
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
    return [defaultUpdateManifestUrl, githubUpdateManifestUrl]
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
            <string>--hidden</string>
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

@_silgen_name("nostr_vpn_update_check_json")
private func nostrVpnUpdateCheckJson(
    _ currentVersion: UnsafePointer<CChar>?,
    _ mode: UnsafePointer<CChar>?,
    _ source: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("nostr_vpn_update_download_json")
private func nostrVpnUpdateDownloadJson(
    _ currentVersion: UnsafePointer<CChar>?,
    _ mode: UnsafePointer<CChar>?,
    _ source: UnsafePointer<CChar>?,
    _ downloadDir: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("nostr_vpn_string_free")
private func nostrVpnStringFree(_ value: UnsafeMutablePointer<CChar>?)

private func runCoreUpdateCheck(currentVersion: String) async throws -> CoreUpdateResult {
    try await Task.detached {
        try runCoreUpdateCheckBlocking(currentVersion: currentVersion)
    }.value
}

private func runCoreUpdateDownload(currentVersion: String, downloadDir: URL) async throws -> CoreUpdateResult {
    try await Task.detached {
        try runCoreUpdateDownloadBlocking(currentVersion: currentVersion, downloadDir: downloadDir)
    }.value
}

private func runCoreUpdateCheckBlocking(currentVersion: String) throws -> CoreUpdateResult {
    let json = try currentVersion.withCString { current in
        try "app".withCString { mode in
            try "auto".withCString { source in
                try takeRustString(nostrVpnUpdateCheckJson(current, mode, source))
            }
        }
    }
    return try decodeCoreUpdateResult(json)
}

private func runCoreUpdateDownloadBlocking(currentVersion: String, downloadDir: URL) throws -> CoreUpdateResult {
    let json = try currentVersion.withCString { current in
        try "app".withCString { mode in
            try "auto".withCString { source in
                try downloadDir.path.withCString { dir in
                    try takeRustString(nostrVpnUpdateDownloadJson(current, mode, source, dir))
                }
            }
        }
    }
    return try decodeCoreUpdateResult(json)
}

private func takeRustString(_ pointer: UnsafeMutablePointer<CChar>?) throws -> String {
    guard let pointer else {
        throw UpdateError.coreUpdaterOutputInvalid
    }
    defer { nostrVpnStringFree(pointer) }
    return String(cString: pointer)
}

private func decodeCoreUpdateResult(_ json: String) throws -> CoreUpdateResult {
    let data = Data(json.utf8)
    let result = try JSONDecoder().decode(CoreUpdateResult.self, from: data)
    if let error = result.error, !error.isEmpty {
        throw UpdateError.coreUpdaterFailed(error)
    }
    return result
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
    relays: [String]? = nil,
    disabledRelays: [String]? = nil,
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
    fipsHostTunnelEnabled: Bool? = nil,
    connectToNonRosterFipsPeers: Bool? = nil,
    fipsNostrDiscoveryEnabled: Bool? = nil,
    fipsBootstrapEnabled: Bool? = nil,
    fipsBootstrapPeers: [String: [String]]? = nil,
    fipsHostInboundTcpPorts: String? = nil,
    autoconnect: Bool? = nil,
    launchOnStartup: Bool? = nil,
    closeToTrayOnClose: Bool? = nil
) -> SettingsPatch {
    SettingsPatch(
        nodeName: nodeName,
        endpoint: endpoint,
        tunnelIp: tunnelIp,
        listenPort: listenPort,
        relays: relays,
        disabledRelays: disabledRelays,
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
        fipsHostTunnelEnabled: fipsHostTunnelEnabled,
        connectToNonRosterFipsPeers: connectToNonRosterFipsPeers,
        fipsNostrDiscoveryEnabled: fipsNostrDiscoveryEnabled,
        fipsBootstrapEnabled: fipsBootstrapEnabled,
        fipsBootstrapPeers: fipsBootstrapPeers,
        fipsHostInboundTcpPorts: fipsHostInboundTcpPorts,
        autoconnect: autoconnect,
        launchOnStartup: launchOnStartup,
        closeToTrayOnClose: closeToTrayOnClose
    )
}

private func normalizeNetworkIdInput(_ value: String) -> String {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    let compactScalars = trimmed.unicodeScalars.filter {
        !$0.properties.isWhitespace && $0 != "-"
    }
    let compact = String(String.UnicodeScalarView(compactScalars))
    if compact.isEmpty && trimmed.unicodeScalars.allSatisfy({ $0.properties.isWhitespace || $0 == "-" }) {
        return ""
    }
    return !compact.isEmpty && isHexString(compact) ? compact.lowercased() : trimmed
}

private func isHexString(_ value: String) -> Bool {
    !value.isEmpty && value.unicodeScalars.allSatisfy { scalar in
        (48...57).contains(Int(scalar.value))
            || (65...70).contains(Int(scalar.value))
            || (97...102).contains(Int(scalar.value))
    }
}
