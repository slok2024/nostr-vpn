import SwiftUI

struct RootView: View {
    @ObservedObject var model: AppModel
    @State private var addNetworkPresented = false

    var body: some View {
        Group {
            if model.activeNetwork == nil {
                NavigationStack {
                    AddNetworkPage(model: model)
                        .navigationTitle("Add Network")
                        .navigationBarTitleDisplayMode(.inline)
                }
            } else {
                TabView {
                    NavigationStack {
                        DevicesPage(model: model)
                            .toolbar { networkSwitcherToolbar }
                    }
                    .tabItem { Label("Devices", systemImage: "circle.grid.2x2.fill") }

                    NavigationStack {
                        ExitNodesPage(model: model)
                            .navigationTitle("Exit Nodes")
                            .toolbar { networkSwitcherToolbar }
                    }
                    .tabItem { Label("Exit Nodes", systemImage: "arrow.triangle.branch") }

                    NavigationStack {
                        SettingsPage(model: model)
                            .navigationTitle("Settings")
                            .toolbar { networkSwitcherToolbar }
                    }
                    .tabItem { Label("Settings", systemImage: "gearshape") }
                }
            }
        }
        .tint(.purple)
        .sheet(isPresented: $addNetworkPresented) {
            NavigationStack {
                AddNetworkPage(model: model)
                    .navigationTitle("Add Network")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { addNetworkPresented = false }
                        }
                    }
            }
        }
    }

    @ToolbarContentBuilder
    private var networkSwitcherToolbar: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            NetworkSwitcher(model: model, addNetworkPresented: $addNetworkPresented)
        }
        ToolbarItem(placement: .topBarTrailing) {
            ToolbarVpnSwitch(model: model)
        }
    }
}

/// Header dropdown that shows the active network's name and lets the user
/// switch to any saved network or jump to the Add Network page. Single
/// "Add network" button when there's only one saved network and nothing
/// to switch to.
private struct NetworkSwitcher: View {
    @ObservedObject var model: AppModel
    @Binding var addNetworkPresented: Bool

    var body: some View {
        let active = model.activeNetwork
        let inactive = model.state.networks.filter { !$0.enabled }
        Menu {
            ForEach(inactive) { network in
                Button {
                    model.dispatch(
                        NativeActions.setNetworkEnabled(network.id, true),
                        status: "Switching to \(network.displayName)"
                    )
                } label: {
                    Label(network.displayName, systemImage: "rectangle.stack")
                }
            }
            if !inactive.isEmpty {
                Divider()
            }
            Button {
                addNetworkPresented = true
            } label: {
                Label("Add network", systemImage: "plus")
            }
        } label: {
            HStack(spacing: 4) {
                Text(active?.displayName ?? "Nostr VPN")
                    .font(.headline)
                    .lineLimit(1)
                Image(systemName: "chevron.down")
                    .font(.caption2)
            }
            .foregroundStyle(.primary)
        }
    }
}

/// First screen on a fresh install AND the screen reachable from the
/// header switcher's "Add network" item. Same content in both contexts:
/// create, join via invite, or pick up a nearby invite.
private struct AddNetworkPage: View {
    @ObservedObject var model: AppModel

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                if !model.state.error.isEmpty || !model.statusMessage.isEmpty {
                    NoticeCard(text: model.state.error.isEmpty ? model.statusMessage : model.state.error)
                }
                CreateNetworkCard(model: model)
                JoinNetworkCard(model: model)
                NearbyCard(model: model)
            }
            .padding()
        }
        .background(AppColors.background)
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
                    if network.localIsAdmin {
                        Button {
                            addDevicePresented = true
                        } label: {
                            Label("Add device", systemImage: "person.crop.circle.badge.plus")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.bordered)
                    }
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
        .sheet(isPresented: $addDevicePresented) {
            NavigationStack {
                AddDeviceSheet(model: model)
                    .navigationTitle("Add Device")
                    .navigationBarTitleDisplayMode(.inline)
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
        !model.actionInFlight && model.state.vpnControlSupported && model.activeNetwork != nil
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

private struct CreateNetworkCard: View {
    @ObservedObject var model: AppModel
    @State private var networkName = "My Network"

    var body: some View {
        AppCard {
            Text("Create Network")
                .font(.headline)
            HStack {
                TextField("Network name", text: $networkName)
                    .textFieldStyle(.roundedBorder)
                Button("Create") {
                    let name = networkName.trimmingCharacters(in: .whitespacesAndNewlines)
                    model.dispatch(
                        NativeActions.addNetwork(name.isEmpty ? "My Network" : name),
                        status: "Creating network"
                    )
                    networkName = "My Network"
                }
                .buttonStyle(.borderedProminent)
                .disabled(model.actionInFlight)
            }
        }
    }
}

private struct JoinNetworkCard: View {
    @ObservedObject var model: AppModel
    @State private var inviteInput = ""
    @State private var qrScannerPresented = false
    @State private var manualExpanded = false
    @State private var manualAdminId = ""
    @State private var manualNetworkId = ""

    private var manualAdminInvalid: Bool {
        let trimmed = manualAdminId.trimmingCharacters(in: .whitespacesAndNewlines)
        return !trimmed.isEmpty && !isValidDeviceId(trimmed)
    }

    private var canSubmitManual: Bool {
        let admin = manualAdminId.trimmingCharacters(in: .whitespacesAndNewlines)
        let mesh = manualNetworkId.trimmingCharacters(in: .whitespacesAndNewlines)
        return !admin.isEmpty && !mesh.isEmpty && isValidDeviceId(admin)
    }

    var body: some View {
        AppCard {
            Text("Join Network")
                .font(.headline)
            TextField("nvpn://invite/…", text: $inviteInput)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.roundedBorder)
                .onChange(of: inviteInput) { _, newValue in
                    let trimmed = newValue.trimmingCharacters(in: .whitespacesAndNewlines)
                    if trimmed.lowercased().hasPrefix("nvpn://invite/") {
                        model.importInvite(trimmed)
                        inviteInput = ""
                    }
                }
            HStack {
                Button {
                    if let text = UIPasteboard.general.string {
                        inviteInput = text.trimmingCharacters(in: .whitespacesAndNewlines)
                    }
                } label: {
                    Label("Paste", systemImage: "doc.on.clipboard")
                }
                Button {
                    qrScannerPresented = true
                } label: {
                    Label("Scan", systemImage: "camera.viewfinder")
                }
                Spacer()
            }

            DisclosureGroup("Add manually", isExpanded: $manualExpanded) {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Enter the admin's Device ID and the network ID. They must add your Device ID too.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    TextField("Admin Device ID (npub1…)", text: $manualAdminId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .textFieldStyle(.roundedBorder)
                        .overlay(
                            RoundedRectangle(cornerRadius: 6)
                                .stroke(Color.red, lineWidth: manualAdminInvalid ? 1 : 0)
                        )
                    if manualAdminInvalid {
                        Text("Not a valid device ID")
                            .font(.caption)
                            .foregroundStyle(.red)
                    }
                    TextField("Network ID", text: $manualNetworkId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .textFieldStyle(.roundedBorder)
                    Button("Send join request") {
                        let admin = manualAdminId.trimmingCharacters(in: .whitespacesAndNewlines)
                        let mesh = manualNetworkId.trimmingCharacters(in: .whitespacesAndNewlines)
                        if let invite = manualInviteJSON(adminNpub: admin, meshId: mesh) {
                            model.importInvite(invite)
                            manualAdminId = ""
                            manualNetworkId = ""
                            manualExpanded = false
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(!canSubmitManual)
                }
                .padding(.top, 6)
            }
            .font(.subheadline)

            if let network = model.activeNetwork {
                if network.outboundJoinRequest != nil {
                    Pill("Join requested", tint: .orange)
                } else if !network.inviteInviterNpub.isEmpty {
                    Button {
                        model.dispatch(
                            NativeActions.requestNetworkJoin(networkId: network.id),
                            status: "Requesting access"
                        )
                    } label: {
                        Label("Request Access", systemImage: "person.badge.plus")
                    }
                    .buttonStyle(.bordered)
                }
            }
        }
        .sheet(isPresented: $qrScannerPresented) {
            QRCodeScannerSheet { code in
                model.importInvite(code)
                qrScannerPresented = false
            }
        }
    }
}

/// Build a synthetic invite from an admin's Device ID + a mesh network ID,
/// for the manual-join flow. The Rust core's `parse_network_invite`
/// accepts a bare-JSON invite (no `nvpn://invite/` prefix needed) as long
/// as it has v3, a non-empty network_id, and at least one admin. This
/// gives the same result as importing the equivalent invite link: the
/// network is added locally with the admin's npub, and a join request is
/// queued so the admin can accept and propagate the roster.
func manualInviteJSON(adminNpub: String, meshId: String) -> String? {
    guard isValidDeviceId(adminNpub), !meshId.isEmpty else { return nil }
    // NetworkInvite is serde(rename_all = "camelCase") on the Rust side.
    let payload: [String: Any] = [
        "v": 3,
        "networkId": meshId,
        "inviterNpub": adminNpub,
        "admins": [adminNpub],
        "participants": [],
    ]
    guard let data = try? JSONSerialization.data(withJSONObject: payload, options: []) else {
        return nil
    }
    return String(data: data, encoding: .utf8)
}

/// Admin-only sheet for adding a device to YOUR network. Two paths:
/// share an invite (QR / copy / broadcast) for the other device to import,
/// or directly add by Device ID. Joining someone else's network and
/// finding nearby networks belong to the Add Network page, not here —
/// they're the "I want IN to a network" direction, not "I want THEM in
/// MY network".
private struct AddDeviceSheet: View {
    @ObservedObject var model: AppModel

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                InviteToMyNetworkCard(model: model)

                if let network = model.activeNetwork {
                    AddDeviceCard(network: network) { npub, alias in
                        model.dispatch(
                            NativeActions.addParticipant(networkId: network.id, npub: npub, alias: alias),
                            status: "Adding device"
                        )
                    }
                }
            }
            .padding()
        }
        .background(AppColors.background)
    }
}

private struct InviteToMyNetworkCard: View {
    @ObservedObject var model: AppModel

    var body: some View {
        AppCard {
            HStack(alignment: .top, spacing: 16) {
                QrCodeView(matrix: model.qrMatrix(for: model.state.activeNetworkInvite))
                    .frame(width: 136, height: 136)
                VStack(alignment: .leading, spacing: 10) {
                    Text("Invite to my network")
                        .font(.headline)
                    Text("Share this code with another device to give it access to your network.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    CopyLine(value: model.state.activeNetworkInvite, model: model)
                    if !model.state.activeNetworkInvite.isEmpty {
                        ShareLink(item: model.state.activeNetworkInvite) {
                            Label("Share", systemImage: "square.and.arrow.up")
                        }
                    }
                    Button {
                        if model.state.inviteBroadcastActive {
                            model.dispatch(NativeActions.stopInviteBroadcast(), status: "Stopped broadcasting")
                        } else {
                            model.dispatch(NativeActions.startInviteBroadcast(), status: "Broadcasting invite")
                        }
                    } label: {
                        Label(
                            model.state.inviteBroadcastActive
                                ? "Broadcasting · \(formatRemaining(model.state.inviteBroadcastRemainingSecs))"
                                : "Broadcast invite",
                            systemImage: model.state.inviteBroadcastActive ? "stop.circle" : "dot.radiowaves.left.and.right"
                        )
                    }
                    .buttonStyle(.bordered)
                }
            }
        }
    }

    private func formatRemaining(_ seconds: UInt64) -> String {
        if seconds == 0 { return "off" }
        let minutes = seconds / 60
        if minutes == 0 { return "\(seconds)s" }
        let secs = seconds % 60
        return secs == 0 ? "\(minutes)m" : String(format: "%dm%02ds", minutes, secs)
    }
}

private struct ExitNodesPage: View {
    @ObservedObject var model: AppModel

    private var directSelected: Bool {
        !model.state.wireguardExitEnabled && model.state.exitNode.isEmpty
    }

    private var wgSelected: Bool {
        model.state.wireguardExitEnabled
    }

    private var wgSubtitle: String {
        if !model.state.wireguardExitConfigured {
            return "No WireGuard config saved yet"
        }
        let endpoint = model.state.wireguardExitEndpoint
        return endpoint.isEmpty ? "Configured" : endpoint
    }

    // The daemon clears the *other* side automatically when there
    // would otherwise be both a peer exit AND WG upstream enabled
    // (see `settings_patch_enforces_exit_node_mutual_exclusion` in
    // ffi.rs). "Direct" needs to clear both explicitly though, since
    // there's no conflict in that case for the daemon to resolve.
    private func selectDirect() {
        model.dispatch(
            NativeActions.updateSettings(["exitNode": "", "wireguardExitEnabled": false]),
            status: "Saving route"
        )
    }

    private func selectWireGuard() {
        model.dispatch(
            NativeActions.updateSettings(["wireguardExitEnabled": true]),
            status: "Saving route"
        )
    }

    private func selectPeer(_ npub: String) {
        model.dispatch(
            NativeActions.updateSettings(["exitNode": npub]),
            status: "Saving route"
        )
    }

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                AppCard {
                    Text("Exit Node")
                        .font(.headline)
                    ExitNodeRow(
                        title: "Direct",
                        subtitle: "No exit node — your own internet",
                        selected: directSelected,
                        enabled: true,
                        action: selectDirect
                    )
                    ExitNodeRow(
                        title: "WireGuard upstream",
                        subtitle: wgSubtitle,
                        selected: wgSelected,
                        enabled: model.state.wireguardExitConfigured,
                        action: selectWireGuard
                    )
                    if let network = model.activeNetwork {
                        ForEach(network.participants.filter(\.offersExitNode)) { participant in
                            ExitNodeRow(
                                title: participant.displayName,
                                subtitle: participant.npub,
                                selected: !model.state.wireguardExitEnabled
                                    && model.state.exitNode == participant.npub,
                                enabled: true,
                                action: { selectPeer(participant.npub) }
                            )
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
                    Toggle("Block internet if exit node disconnects", isOn: Binding(
                        get: { model.state.exitNodeLeakProtection },
                        set: { value in
                            model.dispatch(
                                NativeActions.updateSettings(["exitNodeLeakProtection": value]),
                                status: "Saving route"
                            )
                        }
                    ))
                }
                WireGuardSettingsCard(model: model)
            }
            .padding()
        }
        .background(AppColors.background)
    }
}

private struct ExitNodeRow: View {
    let title: String
    let subtitle: String
    let selected: Bool
    let enabled: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .center, spacing: 12) {
                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                    .foregroundColor(selected ? AppColors.accent : .secondary)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(.body)
                        .foregroundColor(.primary)
                    if !subtitle.isEmpty {
                        Text(subtitle)
                            .font(.footnote)
                            .foregroundColor(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                Spacer()
            }
            .padding(.vertical, 6)
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .opacity(enabled ? 1.0 : 0.5)
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
                            Pill("This device", tint: AppColors.ok)
                        }
                        if participant.offersExitNode {
                            Pill("Exit", tint: .orange)
                        }
                        if isFipsRouted(participant, state: model.state) {
                            Pill("Routed", tint: .secondary)
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
    @State private var deviceId = ""
    @State private var alias = ""

    private var trimmedDeviceId: String {
        deviceId.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var deviceIdInvalid: Bool {
        !trimmedDeviceId.isEmpty && !isValidDeviceId(trimmedDeviceId)
    }

    var body: some View {
        AppCard {
            Text("Add by Device ID")
                .font(.headline)
            Text("Manual pairing: enter the other device's ID (starts with npub1). They also need to add yours.")
                .font(.caption)
                .foregroundStyle(.secondary)
            TextField("npub1…", text: $deviceId)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.roundedBorder)
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(Color.red, lineWidth: deviceIdInvalid ? 1 : 0)
                )
            if deviceIdInvalid {
                Text("Not a valid device ID")
                    .font(.caption)
                    .foregroundStyle(.red)
            }
            TextField("Name", text: $alias)
                .textFieldStyle(.roundedBorder)
            Button("Add") {
                add(trimmedDeviceId, alias)
                deviceId = ""
                alias = ""
            }
            .buttonStyle(.borderedProminent)
            .disabled(trimmedDeviceId.isEmpty || deviceIdInvalid)
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
                Text("Nearby invites")
                    .font(.headline)
                Spacer()
                Button {
                    model.dispatch(
                        model.state.nearbyDiscoveryActive ? NativeActions.stopNearbyDiscovery() : NativeActions.startNearbyDiscovery(),
                        status: "Looking for nearby"
                    )
                } label: {
                    Label(
                        model.state.nearbyDiscoveryActive
                            ? "Listening · \(formatRemaining(model.state.nearbyDiscoveryRemainingSecs))"
                            : "Look for nearby",
                        systemImage: model.state.nearbyDiscoveryActive ? "stop.circle" : "dot.radiowaves.left.and.right"
                    )
                }
                .buttonStyle(.bordered)
            }
            if model.state.lanPeers.isEmpty {
                Text(model.state.nearbyDiscoveryActive ? "No nearby invites yet" : "Tap above to look for nearby devices")
                    .foregroundStyle(.secondary)
                    .font(.footnote)
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

    private func formatRemaining(_ seconds: UInt64) -> String {
        if seconds == 0 { return "off" }
        let minutes = seconds / 60
        if minutes == 0 { return "\(seconds)s" }
        let secs = seconds % 60
        return secs == 0 ? "\(minutes)m" : String(format: "%dm%02ds", minutes, secs)
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

private struct WireGuardSettingsCard: View {
    @ObservedObject var model: AppModel
    @State private var config = ""

    var body: some View {
        AppCard {
            Text("WireGuard Upstream")
                .font(.headline)
            Text("Paste a WireGuard config from an upstream VPN provider such as Mullvad or Proton VPN.")
                .font(.footnote)
                .foregroundStyle(.secondary)
            Toggle("Enabled", isOn: Binding(
                get: { model.state.wireguardExitEnabled },
                set: { value in
                    model.dispatch(NativeActions.updateSettings(["wireguardExitEnabled": value]), status: "Saving")
                }
            ))
            TextEditor(text: $config)
                .font(.system(.body, design: .monospaced))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .frame(minHeight: 180)
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(Color.secondary.opacity(0.25))
                )
            Button("Save") {
                model.dispatch(NativeActions.updateSettings(["wireguardExitConfig": config]), status: "Saving")
            }
            .buttonStyle(.borderedProminent)
        }
        .onAppear(perform: sync)
        .onChange(of: model.state.rev) { _, _ in
            sync()
        }
    }

    private func sync() {
        config = model.state.wireguardExitConfig
    }
}

private struct NetworksCard: View {
    @ObservedObject var model: AppModel
    @State private var newNetwork = ""
    @State private var pendingRemoval: NetworkState?

    var body: some View {
        AppCard {
            Text("Networks")
                .font(.headline)
            if let network = model.activeNetwork {
                HStack {
                    Text(network.displayName)
                        .font(.subheadline.weight(.semibold))
                    Spacer()
                    Text("active")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Button(role: .destructive) {
                        pendingRemoval = network
                    } label: {
                        Image(systemName: "trash")
                    }
                    .buttonStyle(.borderless)
                }
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
                    Button(role: .destructive) {
                        pendingRemoval = network
                    } label: {
                        Image(systemName: "trash")
                    }
                    .buttonStyle(.borderless)
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
        .confirmationDialog(
            "Remove \(pendingRemoval?.displayName ?? "network")?",
            isPresented: Binding(
                get: { pendingRemoval != nil },
                set: { if !$0 { pendingRemoval = nil } }
            ),
            titleVisibility: .visible,
            presenting: pendingRemoval
        ) { network in
            Button("Remove", role: .destructive) {
                model.dispatch(NativeActions.removeNetwork(network.id), status: "Removing network")
                pendingRemoval = nil
            }
            Button("Cancel", role: .cancel) { pendingRemoval = nil }
        } message: { _ in
            Text("This deletes the network from this device. You can rejoin later with the invite.")
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
        return state.vpnEnabled ? "This device" : "Off"
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

private func isFipsRouted(_ participant: ParticipantState, state: AppState) -> Bool {
    !isSelf(participant, state: state)
        && participant.reachable
        && participant.fipsTransportAddr.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
}

private func short(_ value: String, prefix: Int, suffix: Int) -> String {
    guard value.count > prefix + suffix + 1 else {
        return value.isEmpty ? "Device" : value
    }
    return "\(value.prefix(prefix))...\(value.suffix(suffix))"
}

private let bech32BodyCharset: Set<Character> = Set("qpzry9x8gf2tvdw0s3jn54khce6mua7l")

/// A valid device ID is a bech32-encoded npub: `npub1` + 58 bech32 chars.
func isValidDeviceId(_ value: String) -> Bool {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard trimmed.count == 63, trimmed.hasPrefix("npub1") else { return false }
    return trimmed.dropFirst(5).allSatisfy { bech32BodyCharset.contains($0) }
}
