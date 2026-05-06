import SwiftUI

struct RootView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        TabView {
            NavigationStack {
                DevicesPage(model: model)
                    .navigationTitle("Devices")
            }
            .tabItem { Label("Devices", systemImage: "desktopcomputer") }

            NavigationStack {
                SharePage(model: model)
                    .navigationTitle("Share")
            }
            .tabItem { Label("Share", systemImage: "qrcode") }

            NavigationStack {
                RoutingPage(model: model)
                    .navigationTitle("Routing")
            }
            .tabItem { Label("Routing", systemImage: "arrow.triangle.branch") }

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

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                HeroCard(model: model)
                if !model.state.error.isEmpty || !model.statusMessage.isEmpty {
                    NoticeCard(text: model.state.error.isEmpty ? model.statusMessage : model.state.error)
                }
                if let network = model.activeNetwork {
                    ForEach(network.participants) { participant in
                        ParticipantRow(model: model, participant: participant)
                    }
                    AddDeviceCard(network: network) { npub, alias in
                        model.dispatch(
                            NativeActions.addParticipant(networkId: network.id, npub: npub, alias: alias),
                            status: "Adding device"
                        )
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
    }
}

private struct SharePage: View {
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

                NearbyCard(model: model)
            }
            .padding()
        }
        .background(AppColors.background)
    }
}

private struct RoutingPage: View {
    @ObservedObject var model: AppModel
    @State private var routes = ""

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
                    TextField("Routes", text: $routes)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .textFieldStyle(.roundedBorder)
                        .onAppear {
                            routes = model.state.advertisedRoutes.joined(separator: ", ")
                        }
                    Button("Save") {
                        model.dispatch(
                            NativeActions.updateSettings(["advertisedRoutes": routes]),
                            status: "Saving routes"
                        )
                    }
                    .buttonStyle(.bordered)
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
                RelaysCard(model: model)
                DiagnosticsCard(state: model.state)
            }
            .padding()
        }
        .background(AppColors.background)
    }
}

private struct HeroCard: View {
    @ObservedObject var model: AppModel

    var body: some View {
        AppCard {
            HStack(spacing: 14) {
                Circle()
                    .fill(AppColors.accent.opacity(0.12))
                    .frame(width: 58, height: 58)
                    .overlay(
                        Image(systemName: model.state.meshReady ? "checkmark" : "power")
                            .font(.system(size: 22, weight: .semibold))
                            .foregroundStyle(model.state.sessionActive ? AppColors.ok : AppColors.accent)
                    )
                VStack(alignment: .leading, spacing: 3) {
                    Text(model.activeNetwork?.displayName ?? "Nostr VPN")
                        .font(.title.bold())
                        .lineLimit(1)
                    Text(model.state.sessionStatus)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Text(peerSummary)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button(model.state.sessionActive ? "Connected" : "Connect") {
                    model.toggleSession()
                }
                .buttonStyle(.borderedProminent)
                .disabled(model.actionInFlight || !model.state.vpnSessionControlSupported)
            }
        }
    }

    private var peerSummary: String {
        if model.state.expectedPeerCount == 0 {
            return "No peers yet"
        }
        return "\(model.state.connectedPeerCount) of \(model.state.expectedPeerCount) connected"
    }
}

private struct ParticipantRow: View {
    @ObservedObject var model: AppModel
    let participant: ParticipantState

    var body: some View {
        AppCard {
            HStack(spacing: 12) {
                Circle()
                    .fill(participant.reachable ? AppColors.ok : Color.gray.opacity(0.35))
                    .frame(width: 12, height: 12)
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 8) {
                        Text(participant.displayName)
                            .font(.headline)
                            .lineLimit(1)
                        if participant.isAdmin {
                            Pill("Admin", tint: AppColors.accent)
                        }
                        if participant.offersExitNode {
                            Pill("Exit", tint: .orange)
                        }
                    }
                    Text(cleanIp(participant.tunnelIp))
                        .foregroundStyle(.secondary)
                    Text(participant.statusText)
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
            Text("Add Device")
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

private struct RelaysCard: View {
    @ObservedObject var model: AppModel
    @State private var relay = ""

    var body: some View {
        AppCard {
            Text("FIPS Relays")
                .font(.headline)
            ForEach(model.state.relays) { item in
                HStack {
                    VStack(alignment: .leading) {
                        Text(item.url)
                            .lineLimit(1)
                        Text(item.statusText)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Remove") {
                        model.dispatch(NativeActions.removeRelay(item.url), status: "Removing relay")
                    }
                }
            }
            HStack {
                TextField("Relay URL", text: $relay)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .textFieldStyle(.roundedBorder)
                Button("Add") {
                    model.dispatch(NativeActions.addRelay(relay), status: "Adding relay")
                    relay = ""
                }
                .disabled(relay.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
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
