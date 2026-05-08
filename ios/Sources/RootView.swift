import SwiftUI

struct RootView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        TabView {
            NavigationStack {
                DevicesPage(model: model)
                    .navigationTitle("Devices")
            }
            .tabItem { Label("Devices", systemImage: "circle.grid.2x2.fill") }

            NavigationStack {
                ExitNodesPage(model: model)
                    .navigationTitle("Exit Nodes")
            }
            .tabItem { Label("Exit Nodes", systemImage: "arrow.triangle.branch") }

            NavigationStack {
                SettingsPage(model: model)
                    .navigationTitle("Settings")
            }
            .tabItem { Label("Settings", systemImage: "gearshape") }
        }
        .tint(.purple)
    }
}

private struct DevicesPage: View {
    @ObservedObject var model: AppModel
    @State private var addDevicePresented = false

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                if !model.state.error.isEmpty || !model.statusMessage.isEmpty {
                    NoticeCard(text: model.state.error.isEmpty ? model.statusMessage : model.state.error)
                }
                if let network = model.activeNetwork {
                    DeviceListHeader(state: model.state, network: network)
                    ForEach(sortedParticipants(network.participants, state: model.state)) { participant in
                        ParticipantRow(model: model, participant: participant)
                    }
                    ForEach(network.inboundJoinRequests) { request in
                        JoinRequestRow(request: request) {
                            model.dispatch(
                                NativeActions.acceptJoinRequest(
                                    networkId: network.id,
                                    requesterNpub: request.requesterNpub
                                ),
                                status: "Accepting request"
                            )
                        }
                    }
                } else {
                    NoticeCard(text: "No network")
                }
            }
            .padding()
        }
        .background(AppColors.background)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                HStack(spacing: 12) {
                    Button {
                        addDevicePresented = true
                    } label: {
                        Image(systemName: "plus")
                    }
                    .accessibilityLabel("Add device")

                    ToolbarVpnSwitch(model: model)
                }
            }
        }
        .sheet(isPresented: $addDevicePresented) {
            NavigationStack {
                AddDeviceSheet(model: model)
                    .navigationTitle("Add Device")
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Done") {
                                addDevicePresented = false
                            }
                        }
                    }
            }
        }
    }

}

private struct ToolbarVpnSwitch: View {
    @ObservedObject var model: AppModel

    private var enabled: Bool {
        !model.actionInFlight && model.state.vpnControlSupported
    }

    var body: some View {
        Button {
            model.toggleVpn()
        } label: {
            ZStack(alignment: model.state.vpnEnabled ? .trailing : .leading) {
                Capsule()
                    .fill(model.state.vpnEnabled ? AppColors.accent : Color.gray.opacity(0.24))
                    .frame(width: 48, height: 28)
                Circle()
                    .fill(Color.white)
                    .frame(width: 24, height: 24)
                    .shadow(color: .black.opacity(0.22), radius: 1, y: 1)
                    .padding(2)
            }
            .frame(width: 48, height: 28)
            .contentShape(Capsule())
            .opacity(enabled ? 1 : 0.55)
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .accessibilityLabel(model.state.vpnEnabled ? "Turn VPN off" : "Turn VPN on")
        .accessibilityValue(model.state.vpnEnabled ? "On" : "Off")
    }
}

private struct DeviceListHeader: View {
    let state: AppState
    let network: NetworkState

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(network.displayName)
                .font(.headline)
                .lineLimit(1)
            Text(deviceCountText)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 2)
    }

    private var deviceCountText: String {
        if state.expectedPeerCount == 0 {
            return "This device"
        }
        let word = state.expectedPeerCount == 1 ? "device" : "devices"
        return "\(state.connectedPeerCount) online - \(state.expectedPeerCount) \(word)"
    }
}

private struct AddDeviceSheet: View {
    @ObservedObject var model: AppModel
    @State private var inviteInput = ""

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                AppCard {
                    HStack(alignment: .top, spacing: 16) {
                        QrCodeView(matrix: model.qrMatrix(for: model.state.activeNetworkInvite))
                            .frame(width: 136, height: 136)
                        VStack(alignment: .leading, spacing: 10) {
                            Text("Invite Devices")
                                .font(.headline)
                            CopyLine(value: model.state.activeNetworkInvite, model: model)
                            TextField("Invite", text: $inviteInput)
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                                .textFieldStyle(.roundedBorder)
                            Button("Import") {
                                model.importInvite(inviteInput)
                                inviteInput = ""
                            }
                            .buttonStyle(.borderedProminent)
                            .disabled(inviteInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                            if !model.state.activeNetworkInvite.isEmpty {
                                ShareLink(item: model.state.activeNetworkInvite) {
                                    Label("Share", systemImage: "square.and.arrow.up")
                                }
                            }
                        }
                    }
                }

                if let network = model.activeNetwork, network.localIsAdmin {
                    AddDeviceCard(network: network) { npub, alias in
                        model.dispatch(
                            NativeActions.addParticipant(networkId: network.id, npub: npub, alias: alias),
                            status: "Adding device"
                        )
                    }
                }

                NearbyCard(model: model)
            }
            .padding()
        }
        .background(AppColors.background)
    }
}

private struct ExitNodesPage: View {
    @ObservedObject var model: AppModel

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                AppCard {
                    Text("Exit Node")
                        .font(.headline)
                    Button(model.state.exitNode.isEmpty ? "Direct" : "Use Direct") {
                        model.dispatch(NativeActions.updateSettings(["exitNode": ""]), status: "Saving route")
                    }
                    .buttonStyle(.borderedProminent)
                    if let network = model.activeNetwork {
                        ForEach(network.participants.filter(\.offersExitNode)) { participant in
                            Button(participant.displayName) {
                                model.dispatch(
                                    NativeActions.updateSettings(["exitNode": participant.npub]),
                                    status: "Saving route"
                                )
                            }
                        }
                    }
                }

                AppCard {
                    Toggle("Offer exit node", isOn: Binding(
                        get: { model.state.advertiseExitNode },
                        set: { value in
                            model.dispatch(
                                NativeActions.updateSettings(["advertiseExitNode": value]),
                                status: "Saving route"
                            )
                        }
                    ))
                }
            }
            .padding()
        }
        .background(AppColors.background)
    }
}

private struct SettingsPage: View {
    @ObservedObject var model: AppModel

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                DeviceSettingsCard(model: model)
                NetworksCard(model: model)
                DiagnosticsCard(state: model.state)
            }
            .padding()
        }
        .background(AppColors.background)
    }
}

private struct ParticipantRow: View {
    @ObservedObject var model: AppModel
    let participant: ParticipantState

    var body: some View {
        AppCard {
            HStack(spacing: 12) {
                Circle()
                    .fill(connectivityTint(participant, state: model.state))
                    .frame(width: 12, height: 12)
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 8) {
                        Text(deviceName(participant, state: model.state))
                            .font(.headline)
                            .lineLimit(1)
                        if participant.isAdmin {
                            Pill("Admin", tint: AppColors.accent)
                        }
                        if isSelf(participant, state: model.state) {
                            Pill("Self", tint: AppColors.ok)
                        }
                        if participant.offersExitNode {
                            Pill("Exit", tint: .orange)
                        }
                    }
                    Text(deviceSubtitle(participant, state: model.state))
                        .foregroundStyle(.secondary)
                    Text(deviceStatus(participant, state: model.state))
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button("Copy") {
                    model.copy(participant.npub)
                }
            }
        }
    }
}

private struct AddDeviceCard: View {
    let network: NetworkState
    let add: (String, String) -> Void
    @State private var npub = ""
    @State private var alias = ""

    var body: some View {
        AppCard {
            Text("Manual")
                .font(.headline)
            TextField("npub", text: $npub)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.roundedBorder)
            TextField("Name", text: $alias)
                .textFieldStyle(.roundedBorder)
            Button("Add") {
                add(npub.trimmingCharacters(in: .whitespacesAndNewlines), alias)
                npub = ""
                alias = ""
            }
            .buttonStyle(.borderedProminent)
            .disabled(npub.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
    }
}

private struct JoinRequestRow: View {
    let request: InboundJoinRequest
    let accept: () -> Void

    var body: some View {
        AppCard {
            HStack {
                VStack(alignment: .leading) {
                    Text(request.requesterNodeName.isEmpty ? "Join request" : request.requesterNodeName)
                        .font(.headline)
                    Text(request.requestedAtText)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button("Accept", action: accept)
                    .buttonStyle(.borderedProminent)
            }
        }
    }
}

private struct NearbyCard: View {
    @ObservedObject var model: AppModel

    var body: some View {
        AppCard {
            HStack {
                Text("Nearby Devices")
                    .font(.headline)
                Spacer()
                Button(model.state.lanPairingActive ? "\(model.state.lanPairingRemainingSecs)s" : "Pair") {
                    model.dispatch(
                        model.state.lanPairingActive ? NativeActions.stopLanPairing() : NativeActions.startLanPairing(),
                        status: "Pairing"
                    )
                }
            }
            if model.state.lanPeers.isEmpty {
                Text("None")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(model.state.lanPeers) { peer in
                    HStack {
                        VStack(alignment: .leading) {
                            Text(peer.nodeName.isEmpty ? peer.networkName : peer.nodeName)
                                .font(.subheadline.weight(.semibold))
                            Text(peer.lastSeenText)
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("Join") {
                            model.importInvite(peer.invite)
                        }
                    }
                }
            }
        }
    }
}

private struct DeviceSettingsCard: View {
    @ObservedObject var model: AppModel
    @State private var nodeName = ""
    @State private var tunnelIp = ""
    @State private var endpoint = ""
    @State private var port = ""

    var body: some View {
        AppCard {
            Text("This Device")
                .font(.headline)
            TextField("Name", text: $nodeName)
                .textFieldStyle(.roundedBorder)
            TextField("Tunnel IP", text: $tunnelIp)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.roundedBorder)
            TextField("Endpoint", text: $endpoint)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.roundedBorder)
            TextField("Port", text: $port)
                .keyboardType(.numberPad)
                .textFieldStyle(.roundedBorder)
            Toggle("Autoconnect", isOn: Binding(
                get: { model.state.autoconnect },
                set: { value in
                    model.dispatch(NativeActions.updateSettings(["autoconnect": value]), status: "Saving")
                }
            ))
            Button("Save") {
                var patch: [String: Any] = [
                    "nodeName": nodeName,
                    "tunnelIp": tunnelIp,
                    "endpoint": endpoint,
                ]
                if let listenPort = Int(port) {
                    patch["listenPort"] = listenPort
                }
                model.dispatch(NativeActions.updateSettings(patch), status: "Saving")
            }
            .buttonStyle(.borderedProminent)
        }
        .onAppear {
            nodeName = model.state.nodeName
            tunnelIp = model.state.tunnelIp
            endpoint = model.state.endpoint
            port = String(model.state.listenPort)
        }
        .onChange(of: model.state.rev) { _, _ in
            nodeName = model.state.nodeName
            tunnelIp = model.state.tunnelIp
            endpoint = model.state.endpoint
            port = String(model.state.listenPort)
        }
    }
}

private struct NetworksCard: View {
    @ObservedObject var model: AppModel
    @State private var newNetwork = ""

    var body: some View {
        AppCard {
            Text("Networks")
                .font(.headline)
            if let network = model.activeNetwork {
                CopyLine(value: network.networkId, model: model)
                Toggle("Join requests", isOn: Binding(
                    get: { network.joinRequestsEnabled },
                    set: { enabled in
                        model.dispatch(
                            NativeActions.setJoinRequests(networkId: network.id, enabled: enabled),
                            status: "Saving"
                        )
                    }
                ))
                .disabled(!network.localIsAdmin)
            }
            ForEach(model.state.networks.filter { !$0.enabled }) { network in
                HStack {
                    VStack(alignment: .leading) {
                        Text(network.displayName)
                            .font(.subheadline.weight(.semibold))
                        Text("\(network.onlineCount) of \(network.expectedCount) connected")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Activate") {
                        model.dispatch(NativeActions.setNetworkEnabled(network.id, true), status: "Activating")
                    }
                }
            }
            HStack {
                TextField("New network", text: $newNetwork)
                    .textFieldStyle(.roundedBorder)
                Button("Add") {
                    model.dispatch(NativeActions.addNetwork(newNetwork), status: "Adding network")
                    newNetwork = ""
                }
                .disabled(newNetwork.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
    }
}

private struct DiagnosticsCard: View {
    let state: AppState

    var body: some View {
        AppCard {
            Text("Diagnostics")
                .font(.headline)
            Metric("Runtime", state.runtimeStatusDetail.isEmpty ? state.platform : state.runtimeStatusDetail)
            Metric("MagicDNS", state.magicDnsStatus)
            Metric("Version", state.appVersion)
            Metric("Config", state.configPath)
            ForEach(state.health) { issue in
                VStack(alignment: .leading, spacing: 3) {
                    Text(issue.severity)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.orange)
                    Text(issue.summary)
                    if !issue.detail.isEmpty {
                        Text(issue.detail)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
    }
}

private struct AppCard<Content: View>: View {
    let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            content
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

private struct NoticeCard: View {
    let text: String

    var body: some View {
        AppCard {
            Text(text)
                .foregroundStyle(.brown)
        }
    }
}

private struct CopyLine: View {
    let value: String
    @ObservedObject var model: AppModel

    var body: some View {
        HStack {
            Text(value.isEmpty ? "-" : value)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer()
            Button(model.copiedValue == value ? "Copied" : "Copy") {
                model.copy(value)
            }
            .disabled(value.isEmpty)
        }
    }
}

private struct Metric: View {
    let label: String
    let value: String

    init(_ label: String, _ value: String) {
        self.label = label
        self.value = value
    }

    var body: some View {
        HStack(alignment: .top) {
            Text(label)
                .foregroundStyle(.secondary)
                .frame(width: 80, alignment: .leading)
            Text(value.isEmpty ? "-" : value)
                .lineLimit(2)
                .truncationMode(.middle)
        }
        .font(.footnote)
    }
}

private struct Pill: View {
    let text: String
    let tint: Color

    init(_ text: String, tint: Color) {
        self.text = text
        self.tint = tint
    }

    var body: some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(tint.opacity(0.12))
            .clipShape(Capsule())
    }
}

private struct QrCodeView: View {
    let matrix: QrMatrix

    var body: some View {
        Canvas { context, size in
            context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(.white))
            guard matrix.width > 0, matrix.cells.count == matrix.width * matrix.width else {
                return
            }
            let quiet = 3
            let modules = matrix.width + quiet * 2
            let cell = min(size.width, size.height) / CGFloat(modules)
            for y in 0..<matrix.width {
                for x in 0..<matrix.width where matrix.cells[y * matrix.width + x] {
                    let rect = CGRect(
                        x: CGFloat(x + quiet) * cell,
                        y: CGFloat(y + quiet) * cell,
                        width: cell,
                        height: cell
                    )
                    context.fill(Path(rect), with: .color(.black))
                }
            }
        }
        .background(.white)
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

private enum AppColors {
    static let background = Color(uiColor: .systemGroupedBackground)
    static let accent = Color.purple
    static let ok = Color.green
}

private func cleanIp(_ value: String) -> String {
    value.split(separator: "/").first.map(String.init) ?? value
}

private func sortedParticipants(_ participants: [ParticipantState], state: AppState) -> [ParticipantState] {
    participants.sorted { lhs, rhs in
        let lhsSelf = isSelf(lhs, state: state)
        let rhsSelf = isSelf(rhs, state: state)
        if lhsSelf != rhsSelf {
            return lhsSelf
        }
        if lhs.reachable != rhs.reachable {
            return lhs.reachable && !rhs.reachable
        }
        return deviceName(lhs, state: state).localizedCaseInsensitiveCompare(deviceName(rhs, state: state)) == .orderedAscending
    }
}

private func isSelf(_ participant: ParticipantState, state: AppState) -> Bool {
    (!state.ownNpub.isEmpty && participant.npub == state.ownNpub) || participant.meshState == "local"
}

private func deviceName(_ participant: ParticipantState, state: AppState) -> String {
    if isSelf(participant, state: state), !state.nodeName.isEmpty {
        return state.nodeName
    }
    if !participant.magicDnsName.isEmpty {
        return participant.magicDnsName
    }
    if !participant.alias.isEmpty {
        return participant.alias
    }
    if !participant.magicDnsAlias.isEmpty {
        return participant.magicDnsAlias
    }
    return short(participant.npub, prefix: 12, suffix: 6)
}

private func deviceSubtitle(_ participant: ParticipantState, state: AppState) -> String {
    let ip = cleanIp(participant.tunnelIp)
    if isSelf(participant, state: state) {
        return ip.isEmpty ? "This device" : "This device - \(ip)"
    }
    return ip
}

private func deviceStatus(_ participant: ParticipantState, state: AppState) -> String {
    if isSelf(participant, state: state) {
        return state.vpnEnabled ? "Self" : "Off"
    }
    if !participant.statusText.isEmpty {
        return participant.statusText
    }
    switch participant.state {
    case "local", "online", "present":
        return "Online"
    case "pending":
        return "Connecting"
    case "offline", "absent", "off":
        return "Offline"
    default:
        return "Unknown"
    }
}

private func connectivityTint(_ participant: ParticipantState, state: AppState) -> Color {
    if isSelf(participant, state: state) {
        return state.vpnActive ? AppColors.ok : Color.gray.opacity(0.35)
    }
    switch participant.state {
    case "local", "online", "present":
        return AppColors.ok
    case "pending":
        return .orange
    default:
        return Color.gray.opacity(0.35)
    }
}

private func short(_ value: String, prefix: Int, suffix: Int) -> String {
    guard value.count > prefix + suffix + 1 else {
        return value.isEmpty ? "Device" : value
    }
    return "\(value.prefix(prefix))...\(value.suffix(suffix))"
}
