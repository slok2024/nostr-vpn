import SwiftUI

struct RootView: View {
    @ObservedObject var model: AppModel
    @AppStorage(AppModel.vpnDisclosureAcceptedKey) private var vpnDisclosureAccepted = false
    @State private var addNetworkPresented = false
    @State private var vpnDisclosurePresented = false
    @State private var startVpnAfterDisclosure = false
    @State private var shownNetworkId: String?

    private var shownNetwork: NetworkState? {
        if let shownNetworkId,
           let network = model.state.networks.first(where: { $0.id == shownNetworkId }) {
            return network
        }
        return model.activeNetwork
    }

    var body: some View {
        Group {
            if model.activeNetwork == nil {
                NavigationStack {
                    AddNetworkPage(
                        model: model,
                        onReviewVpnDisclosure: {
                            presentVpnDisclosure(startVpnAfterAccept: true)
                        }
                    )
                        .navigationTitle("Add Network")
                        .navigationBarTitleDisplayMode(.inline)
                }
            } else {
                TabView {
                    NavigationStack {
                        DevicesPage(model: model, network: shownNetwork)
                            .toolbar { networkSwitcherToolbar }
                    }
                    .tabItem { Label("Devices", systemImage: "circle.grid.2x2.fill") }

                    NavigationStack {
                        ExitNodesPage(model: model, network: shownNetwork)
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
                AddNetworkPage(
                    model: model,
                    onCreated: { addNetworkPresented = false },
                    onReviewVpnDisclosure: {
                        presentVpnDisclosure(startVpnAfterAccept: true)
                    }
                )
                    .navigationTitle("Add Network")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { addNetworkPresented = false }
                        }
                }
            }
        }
        .sheet(isPresented: $vpnDisclosurePresented) {
            VpnDisclosureSheet {
                let shouldStartVpn = startVpnAfterDisclosure
                vpnDisclosureAccepted = true
                startVpnAfterDisclosure = false
                vpnDisclosurePresented = false
                model.markVpnDisclosureAccepted()
                if shouldStartVpn, !model.state.vpnEnabled {
                    model.toggleVpn()
                }
            }
        }
        .onAppear {
            if model.vpnDisclosurePromptVisible && !vpnDisclosureAccepted {
                presentVpnDisclosure(startVpnAfterAccept: true)
            }
        }
        .onChange(of: model.vpnDisclosurePromptVisible) { _, visible in
            if visible && !vpnDisclosureAccepted {
                presentVpnDisclosure(startVpnAfterAccept: true)
            }
        }
        .onChange(of: model.state.rev) { _, _ in
            if let shownNetworkId,
               !model.state.networks.contains(where: { $0.id == shownNetworkId }) {
                self.shownNetworkId = nil
            }
        }
    }

    @ToolbarContentBuilder
    private var networkSwitcherToolbar: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            NetworkSwitcher(
                model: model,
                shownNetwork: shownNetwork,
                shownNetworkId: $shownNetworkId,
                addNetworkPresented: $addNetworkPresented
            )
        }
        ToolbarItem(placement: .topBarTrailing) {
            ToolbarVpnSwitch(
                model: model,
                vpnDisclosureAccepted: vpnDisclosureAccepted,
                onReviewVpnDisclosure: {
                    presentVpnDisclosure(startVpnAfterAccept: true)
                }
            )
        }
    }

    private func presentVpnDisclosure(startVpnAfterAccept: Bool) {
        startVpnAfterDisclosure = startVpnAfterAccept
        vpnDisclosurePresented = true
    }
}

/// Header dropdown that shows the active network's name and lets the user
/// switch to any saved network or jump to the Add Network page. Single
/// "Add network" button when there's only one saved network and nothing
/// to switch to.
private struct NetworkSwitcher: View {
    @ObservedObject var model: AppModel
    let shownNetwork: NetworkState?
    @Binding var shownNetworkId: String?
    @Binding var addNetworkPresented: Bool

    var body: some View {
        Menu {
            ForEach(model.state.networks) { network in
                Button {
                    shownNetworkId = network.id
                } label: {
                    HStack {
                        if model.state.networks.count > 1 {
                            NetworkStatusDot(network: network)
                        }
                        Text(network.displayName)
                    }
                }
            }
            if !model.state.networks.isEmpty {
                Divider()
            }
            Button {
                addNetworkPresented = true
            } label: {
                Label("Add network", systemImage: "plus")
            }
        } label: {
            HStack(spacing: 4) {
                if let shownNetwork, model.state.networks.count > 1 {
                    NetworkStatusDot(network: shownNetwork)
                }
                Text(shownNetwork?.displayName ?? "Nostr VPN")
                    .font(.headline)
                    .lineLimit(1)
                Image(systemName: "chevron.down")
                    .font(.caption2)
            }
            .foregroundStyle(.primary)
        }
    }
}

private struct NetworkStatusDot: View {
    let network: NetworkState

    var body: some View {
        Circle()
            .fill(network.enabled ? Color.green : Color.secondary.opacity(0.55))
            .frame(width: 7, height: 7)
    }
}

/// First screen on a fresh install AND the screen reachable from the
/// header switcher's "Add network" item. Same content in both contexts:
/// create, join via invite, or pick up a nearby invite.
private struct AddNetworkPage: View {
    @ObservedObject var model: AppModel
    /// Called once the user lands on a network — used by the sheet
    /// presentation to dismiss itself so the underlying Devices tab is
    /// visible. The setup case (no active network) doesn't pass this:
    /// the root view's `if activeNetwork == nil` flips on its own.
    var onCreated: (() -> Void)? = nil
    var onReviewVpnDisclosure: () -> Void = {}

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                if !model.state.error.isEmpty || !model.statusMessage.isEmpty {
                    NoticeCard(
                        text: model.state.error.isEmpty ? model.statusMessage : model.state.error,
                        actionTitle: model.state.error.isEmpty && model.vpnDisclosurePromptVisible ? "Review" : nil,
                        action: onReviewVpnDisclosure
                    )
                }
                CreateNetworkCard(model: model, onCreated: onCreated)
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
    let network: NetworkState?
    @State private var addDevicePresented = false
    @State private var pendingNetworkRemoval: NetworkState?

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                if !model.state.error.isEmpty || shouldShowStatusNotice(model.statusMessage) {
                    NoticeCard(
                        text: model.state.error.isEmpty ? model.statusMessage : model.state.error,
                        actionTitle: nil,
                        action: {}
                    )
                }
                if let network {
                    if !network.enabled {
                        Button {
                            model.dispatch(
                                NativeActions.setNetworkEnabled(network.id, true),
                                status: "Activating network"
                            )
                        } label: {
                            Label("Activate Network", systemImage: "checkmark.circle.fill")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)
                        .disabled(model.actionInFlight)
                    }
                    if network.localIsAdmin {
                        Button {
                            addDevicePresented = true
                        } label: {
                            Label("Add device", systemImage: "plus")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.bordered)
                    }
                    ForEach(sortedParticipants(network.participants, state: model.state)) { participant in
                        ParticipantRow(model: model, network: network, participant: participant)
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
                        } reject: {
                            model.dispatch(
                                NativeActions.rejectJoinRequest(
                                    networkId: network.id,
                                    requesterNpub: request.requesterNpub
                                ),
                                status: "Rejecting request"
                            )
                        }
                    }
                    Button(role: .destructive) {
                        pendingNetworkRemoval = network
                    } label: {
                        Label("Delete network", systemImage: "trash")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                    .padding(.top, 8)
                } else {
                    NoticeCard(text: "No network")
                }
            }
            .padding()
        }
        .background(AppColors.background)
        .sheet(isPresented: $addDevicePresented) {
            if let network {
                NavigationStack {
                    AddDeviceSheet(model: model, network: network)
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
        .confirmationDialog(
            "Delete \(pendingNetworkRemoval?.displayName ?? "network")?",
            isPresented: Binding(
                get: { pendingNetworkRemoval != nil },
                set: { if !$0 { pendingNetworkRemoval = nil } }
            ),
            titleVisibility: .visible,
            presenting: pendingNetworkRemoval
        ) { network in
            Button("Delete", role: .destructive) {
                model.dispatch(NativeActions.removeNetwork(network.id), status: "Deleting network")
                pendingNetworkRemoval = nil
            }
            Button("Cancel", role: .cancel) { pendingNetworkRemoval = nil }
        } message: { _ in
            Text("Removes the network from this device. You can rejoin later with the invite.")
        }
    }

    private func shouldShowStatusNotice(_ message: String) -> Bool {
        !message.isEmpty && message != AppModel.vpnDisclosurePromptMessage
    }

}

private struct ToolbarVpnSwitch: View {
    @ObservedObject var model: AppModel
    let vpnDisclosureAccepted: Bool
    let onReviewVpnDisclosure: () -> Void

    private var enabled: Bool {
        !model.actionInFlight && model.state.vpnControlSupported && model.activeNetwork != nil
    }

    var body: some View {
        Button {
            if !model.state.vpnEnabled && !vpnDisclosureAccepted {
                model.requireVpnDisclosureReview()
                onReviewVpnDisclosure()
            } else {
                model.toggleVpn()
            }
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

private struct VpnDisclosureSheet: View {
    let acknowledge: () -> Void

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 14) {
                Text("Before Turning VPN On")
                    .font(.title2.weight(.semibold))
                    .frame(maxWidth: .infinity, alignment: .leading)
                Text("Nostr VPN is a private VPN and generic WireGuard exit-node utility. It is not a public VPN, anonymity, stealth, or consumer proxy service.")
                Text("The app uses VPN data only to operate networks you configure: device identity, peer lists, routes, exit-node settings, endpoints, invite/join metadata, traffic counters, and connection health.")
                Text("Packet traffic is encrypted. User-selected peers, relays, bridge paths, and exit nodes receive only the data needed to provide the connection you asked them to provide.")
                Text("The developer does not sell VPN data, use it for ads or tracking, or disclose it to third parties.")
                Spacer()
            }
            .font(.body)
            .foregroundStyle(.primary)
            .padding()
            .navigationTitle("VPN Data Use")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Continue", action: acknowledge)
                        .fontWeight(.semibold)
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

private struct CreateNetworkCard: View {
    @ObservedObject var model: AppModel
    var onCreated: (() -> Void)? = nil
    @State private var networkName = "My Network"

    var body: some View {
        SetupCard(title: "Create Network", systemImage: "plus.circle.fill", tint: AppColors.create) {
            VStack(alignment: .leading, spacing: 10) {
                TextField("Network name", text: $networkName)
                    .textFieldStyle(.roundedBorder)
                Button {
                    let name = networkName.trimmingCharacters(in: .whitespacesAndNewlines)
                    model.dispatch(
                        NativeActions.addNetwork(name.isEmpty ? "My Network" : name),
                        status: "Creating network"
                    )
                    networkName = "My Network"
                    onCreated?()
                } label: {
                    Label("Create", systemImage: "plus")
                        .frame(maxWidth: .infinity)
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
        SetupCard(title: "Join Network", systemImage: "arrow.down.circle.fill", tint: AppColors.join) {
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
            HStack(spacing: 10) {
                Button {
                    if let text = UIPasteboard.general.string {
                        inviteInput = text.trimmingCharacters(in: .whitespacesAndNewlines)
                    }
                } label: {
                    Label("Paste", systemImage: "doc.on.clipboard")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                Button {
                    qrScannerPresented = true
                } label: {
                    Label("Scan", systemImage: "camera.viewfinder")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
            }
            .controlSize(.regular)

            DisclosureGroup("Add manually", isExpanded: $manualExpanded) {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Both sides add each other. Enter their Device ID and network ID here, then have them add your Device ID.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    TextField("Admin Device ID", text: $manualAdminId)
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
                    Button("Add") {
                        let admin = manualAdminId.trimmingCharacters(in: .whitespacesAndNewlines)
                        let mesh = manualNetworkId.trimmingCharacters(in: .whitespacesAndNewlines)
                        model.dispatch(
                            NativeActions.manualAddNetwork(adminNpub: admin, meshNetworkId: mesh),
                            status: "Adding network"
                        )
                        manualAdminId = ""
                        manualNetworkId = ""
                        manualExpanded = false
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

/// Admin-only sheet for adding a device to YOUR network. Two paths:
/// share an invite (QR / copy / broadcast) for the other device to import,
/// or directly add by Device ID. Joining someone else's network and
/// finding nearby networks belong to the Add Network page, not here —
/// they're the "I want IN to a network" direction, not "I want THEM in
/// MY network".
private struct AddDeviceSheet: View {
    @ObservedObject var model: AppModel
    let network: NetworkState

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                if network.enabled {
                    InviteToMyNetworkCard(model: model, network: network)
                }
                ManualPairingInfoCard(model: model, network: network)
                AddDeviceCard(network: network) { npub, alias in
                    model.dispatch(
                        NativeActions.addParticipant(networkId: network.id, npub: npub, alias: alias),
                        status: "Adding device"
                    )
                }
            }
            .padding()
        }
        .background(AppColors.background)
    }
}

/// Shown to the admin in the Add Device sheet so they can dictate
/// (text/Signal/etc.) the two values another device needs to join
/// manually: the admin's own Device ID + the network ID. The other
/// device pastes both into Join Network → Add manually. Both sides
/// then have to add each other's Device IDs for the pairing to
/// complete.
private struct ManualPairingInfoCard: View {
    @ObservedObject var model: AppModel
    let network: NetworkState

    var body: some View {
        AppCard {
            Text("For manual join")
                .font(.headline)
            Text("If the other device can't scan or paste an invite, share these two values. They'll enter them under Join Network → Add manually. You still need to add their Device ID below for the pairing to complete.")
                .font(.caption)
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 4) {
                Text("Your Device ID")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                CopyLine(value: model.state.ownNpub, model: model)
            }
            VStack(alignment: .leading, spacing: 4) {
                Text("Network ID")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                CopyLine(value: network.networkId, model: model)
            }
        }
    }
}

private struct InviteToMyNetworkCard: View {
    @ObservedObject var model: AppModel
    let network: NetworkState

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
                    Toggle("Allow join requests", isOn: Binding(
                        get: { network.joinRequestsEnabled },
                        set: { enabled in
                            model.dispatch(
                                NativeActions.setJoinRequests(networkId: network.id, enabled: enabled),
                                status: "Saving join request setting"
                            )
                        }
                    ))
                    .disabled(!network.localIsAdmin || model.actionInFlight)
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
    let network: NetworkState?

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
                    if let network {
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
                DiagnosticsCard(state: model.state)
            }
            .padding()
        }
        .background(AppColors.background)
    }
}

private struct ParticipantRow: View {
    @ObservedObject var model: AppModel
    let network: NetworkState
    let participant: ParticipantState
    @State private var detailPresented = false

    var body: some View {
        Button {
            detailPresented = true
        } label: {
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
                    Image(systemName: "chevron.right")
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                }
            }
        }
        .buttonStyle(.plain)
        .sheet(isPresented: $detailPresented) {
            NavigationStack {
                DeviceDetailSheet(model: model, network: network, participant: participant)
                    .navigationTitle(deviceName(participant, state: model.state))
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Done") { detailPresented = false }
                        }
                    }
            }
        }
    }
}

private struct DeviceDetailSheet: View {
    @ObservedObject var model: AppModel
    let network: NetworkState
    let participant: ParticipantState
    @State private var aliasDraft: String = ""
    @State private var pendingRemove = false

    private var isMe: Bool { isSelf(participant, state: model.state) }
    private var localIsAdmin: Bool { network.localIsAdmin }

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                AppCard {
                    HStack(spacing: 8) {
                        Circle()
                            .fill(connectivityTint(participant, state: model.state))
                            .frame(width: 12, height: 12)
                        Text(deviceDetailStatus(participant, state: model.state))
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                        Spacer()
                        if participant.isAdmin {
                            Pill("Admin", tint: AppColors.accent)
                        }
                        if isMe {
                            Pill("This device", tint: AppColors.ok)
                        }
                        if participant.offersExitNode {
                            Pill("Exit", tint: .orange)
                        }
                    }
                }

                AppCard {
                    Text("Identity")
                        .font(.headline)
                    if !participant.magicDnsName.isEmpty {
                        labelValueRow("Magic DNS", participant.magicDnsName)
                    }
                    labelValueRow("Device ID", participant.npub, copyable: true)
                    if !participant.tunnelIp.isEmpty {
                        labelValueRow("Tunnel IP", participant.tunnelIp, copyable: true)
                    }
                    if !participant.fipsTransportAddr.isEmpty {
                        labelValueRow("Endpoint", participant.fipsTransportAddr)
                    }
                }

                if localIsAdmin && !isMe {
                    AppCard {
                        Text("Manage")
                            .font(.headline)
                        TextField("Alias", text: $aliasDraft)
                            .textFieldStyle(.roundedBorder)
                            .onAppear { aliasDraft = participant.magicDnsAlias }
                        Button {
                            model.dispatch(
                                NativeActions.setParticipantAlias(npub: participant.npub, alias: aliasDraft),
                                status: "Saving alias"
                            )
                        } label: {
                            Label("Save alias", systemImage: "checkmark")
                        }
                        .buttonStyle(.bordered)

                        Button {
                            if participant.isAdmin {
                                model.dispatch(
                                    NativeActions.removeAdmin(networkId: network.id, npub: participant.npub),
                                    status: "Removing admin"
                                )
                            } else {
                                model.dispatch(
                                    NativeActions.addAdmin(networkId: network.id, npub: participant.npub),
                                    status: "Granting admin"
                                )
                            }
                        } label: {
                            Label(participant.isAdmin ? "Remove admin" : "Make admin", systemImage: participant.isAdmin ? "person.fill.badge.minus" : "person.fill.badge.plus")
                        }
                        .buttonStyle(.bordered)

                        Button(role: .destructive) {
                            pendingRemove = true
                        } label: {
                            Label("Remove from network", systemImage: "trash")
                        }
                        .buttonStyle(.bordered)
                    }
                    .confirmationDialog(
                        "Remove \(deviceName(participant, state: model.state))?",
                        isPresented: $pendingRemove,
                        titleVisibility: .visible
                    ) {
                        Button("Remove", role: .destructive) {
                            model.dispatch(
                                NativeActions.removeParticipant(networkId: network.id, npub: participant.npub),
                                status: "Removing device"
                            )
                            pendingRemove = false
                        }
                        Button("Cancel", role: .cancel) { pendingRemove = false }
                    } message: {
                        Text("This removes the device from the network's roster. They keep the network locally but won't be in this roster anymore.")
                    }
                }
            }
            .padding()
        }
        .background(AppColors.background)
    }

    private func labelValueRow(_ label: String, _ value: String, copyable: Bool = false) -> some View {
        HStack(alignment: .top) {
            Text(label)
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                .frame(width: 90, alignment: .leading)
            Text(value)
                .font(.callout)
                .lineLimit(2)
                .truncationMode(.middle)
                .textSelection(.enabled)
            Spacer(minLength: 4)
            if copyable {
                Button {
                    model.copy(value)
                } label: {
                    Image(systemName: "doc.on.doc")
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
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
            Text("Manual pairing: enter the other device's Device ID. They also need to add yours.")
                .font(.caption)
                .foregroundStyle(.secondary)
            TextField("Device ID", text: $deviceId)
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
    let reject: () -> Void

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
                HStack(spacing: 8) {
                    Button("Reject", role: .destructive, action: reject)
                        .buttonStyle(.bordered)
                    Button("Accept", action: accept)
                        .buttonStyle(.borderedProminent)
                }
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

private struct SetupCard<Content: View>: View {
    let title: String
    let systemImage: String
    let tint: Color
    let content: Content

    init(title: String, systemImage: String, tint: Color, @ViewBuilder content: () -> Content) {
        self.title = title
        self.systemImage = systemImage
        self.tint = tint
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(title, systemImage: systemImage)
                .font(.headline)
                .symbolRenderingMode(.hierarchical)
                .foregroundStyle(tint)
            content
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(alignment: .leading) {
            RoundedRectangle(cornerRadius: 2, style: .continuous)
                .fill(tint)
                .frame(width: 4)
                .padding(.vertical, 14)
        }
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(tint.opacity(0.24), lineWidth: 1)
        }
        .tint(tint)
    }
}

private struct NoticeCard: View {
    let text: String
    var actionTitle: String? = nil
    var action: () -> Void = {}

    var body: some View {
        AppCard {
            Text(text)
                .foregroundStyle(.brown)
            if let actionTitle {
                Button(actionTitle, action: action)
                    .buttonStyle(.borderedProminent)
                    .controlSize(.regular)
            }
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
    static let create = Color.green
    static let join = Color.blue
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
    if !participant.magicDnsName.isEmpty {
        return participant.magicDnsName
    }
    if isSelf(participant, state: state), !state.selfMagicDnsName.isEmpty {
        return state.selfMagicDnsName
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
    switch participant.state {
    case "local", "online", "present":
        return "Online"
    case "pending":
        return "Connecting"
    case "offline", "absent", "off":
        return "Offline"
    default:
        return participant.reachable ? "Online" : "Unknown"
    }
}

private func deviceDetailStatus(_ participant: ParticipantState, state: AppState) -> String {
    if isSelf(participant, state: state) {
        return deviceStatus(participant, state: state)
    }
    if !participant.statusText.isEmpty {
        return participant.statusText
    }
    return deviceStatus(participant, state: state)
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
