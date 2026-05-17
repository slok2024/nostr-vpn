import AppKit
import CoreImage
import SwiftUI

struct RootView: View {
    @ObservedObject var manager: AppManager

    @State private var nodeName = ""
    @State private var endpoint = ""
    @State private var tunnelIp = ""
    @State private var listenPort = ""
    @State private var magicDnsSuffix = ""
    @State private var wireguardExitConfig = ""
    @State private var networkNameInput = ""
    @State private var deviceSearch = ""
    @State private var exitNodeSearch = ""
    @State private var selectedDevicePubkeyHex: String?
    @State private var networkNameDrafts: [String: String] = [:]
    @State private var participantAliasDrafts: [String: String] = [:]
    @State private var savedNetworksExpanded = false
    @State private var pendingNetworkRemoval: NativeNetworkState?
    @State private var pendingParticipantRemoval: PendingParticipantRemoval?
    @State private var addByDeviceIdInput = ""
    @State private var addByDeviceIdAlias = ""
    @State private var diagnosticsExpanded = false
    @State private var showingQrScanner = false
    @State private var selectedSidebarItem: SidebarItem? = .devices
    @State private var shownNetworkId: String?
    @State private var addNetworkPresented = false
    @State private var addDevicePresented = false
    @State private var manualJoinExpanded = false
    @State private var manualJoinAdminId = ""
    @State private var manualJoinMeshId = ""
    @State private var lastSyncedNodeName = ""
    @State private var lastSyncedEndpoint = ""
    @State private var lastSyncedTunnelIp = ""
    @State private var lastSyncedListenPort: UInt32 = 0
    @State private var lastSyncedMagicDnsSuffix = ""
    @State private var lastSyncedWireguardExitConfig: String? = nil
    @State private var lastSyncedParticipantAliases: [String: String] = [:]

    private var state: NativeAppState {
        manager.state
    }

    private var activeNetwork: NativeNetworkState? {
        manager.activeNetwork
    }

    private var shownNetwork: NativeNetworkState? {
        if let shownNetworkId,
           let network = state.networks.first(where: { $0.id == shownNetworkId }) {
            return network
        }
        return activeNetwork
    }

    var body: some View {
        VStack(spacing: 0) {
            headerBar
            if manager.serviceUpdateRecommended {
                serviceUpdateStripe
            }
            if manager.updateAvailable {
                updateStripe
            }
            Divider()
            HStack(spacing: 0) {
                sidebar
                    .frame(width: 250)
                Divider()
                detailPane
            }
        }
        .ignoresSafeArea(.container, edges: .top)
        .onAppear(perform: syncDrafts)
        .onChange(of: state.rev) { _, _ in
            syncDrafts()
        }
        .sheet(isPresented: $showingQrScanner) {
            QRCodeScannerSheet { code in
                manager.importInvite(code)
                showingQrScanner = false
            }
        }
        .sheet(isPresented: $addNetworkPresented) {
            addNetworkSheetContent
        }
        .sheet(isPresented: $addDevicePresented) {
            if let network = shownNetwork {
                addDeviceSheetContent(network)
            }
        }
    }

    private var addNetworkSheetContent: some View {
        VStack(alignment: .leading, spacing: 0) {
            sheetTitleBar("Add Network", systemImage: "plus.circle") {
                addNetworkPresented = false
            }
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    createNetworkSection
                    joinNetworkSection(activeNetwork)
                }
                .padding(18)
            }
        }
        .frame(width: 560, height: 620)
    }

    private func addDeviceSheetContent(_ network: NativeNetworkState) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            sheetTitleBar("Add Device", systemImage: "plus") {
                addDevicePresented = false
            }
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    inviteSection(network)
                    manualPairingInfoSection(network)
                    addByDeviceIdSection(network)
                }
                .padding(18)
            }
        }
        .frame(width: 560, height: 620)
    }

    /// Shown to the admin in the Add Device sheet so they can dictate the
    /// two values another device needs to join manually: the admin's own
    /// Device ID + the network ID. The other device pastes both into Join
    /// Network → Add manually. Both sides still have to add each other for
    /// the pairing to complete.
    private func manualPairingInfoSection(_ network: NativeNetworkState) -> some View {
        surface {
            sectionHeader("For Manual Join", systemImage: "keyboard")
            Text("If the other device can't scan or paste an invite, share these two values. They'll enter them under Join Network → Add manually. You still need to add their Device ID below.")
                .font(.caption)
                .foregroundStyle(.secondary)
            detailValueRow("Your Device ID", state.ownNpub)
            detailValueRow("Network ID", network.networkId)
        }
    }

    private func sheetTitleBar(_ title: String, systemImage: String, close: @escaping () -> Void) -> some View {
        HStack(spacing: 10) {
            Label(title, systemImage: systemImage)
                .font(.title3.weight(.semibold))
            Spacer()
            Button("Done", action: close)
                .keyboardShortcut(.cancelAction)
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 12)
    }

    private var headerBar: some View {
        HStack(spacing: 18) {
            headerIdentity
            Spacer(minLength: 0)
            headerVpnControl
        }
        .padding(.leading, 104)
        .padding(.trailing, 18)
        .frame(height: 44)
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private var serviceUpdateStripe: some View {
        HStack(spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Color.orange)
            Text(serviceUpdateStripeText)
                .font(.callout)
                .foregroundStyle(.primary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 12)
            Button {
                manager.installService()
            } label: {
                Text(manager.serviceSettling ? "Updating…" : "Update")
            }
            .controlSize(.small)
            .disabled(!state.serviceSupported || manager.actionInFlight || manager.serviceSettling)
        }
        .padding(.leading, 104)
        .padding(.trailing, 18)
        .padding(.vertical, 6)
        .background(Color(nsColor: .underPageBackgroundColor))
        .overlay(alignment: .bottom) {
            Divider()
        }
    }

    private var serviceUpdateStripeText: String {
        let installed = state.serviceBinaryVersion.trimmingCharacters(in: .whitespacesAndNewlines)
        let expected = state.expectedServiceBinaryVersion.trimmingCharacters(in: .whitespacesAndNewlines)
        if installed.isEmpty || expected.isEmpty {
            return "Background service needs update to match the app"
        }
        return "Background service is on v\(installed); update to match app v\(expected)"
    }

    private var updateStripe: some View {
        HStack(spacing: 10) {
            Image(systemName: "arrow.down.circle.fill")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(.secondary)
            Text(updateStripeText)
                .font(.callout)
                .foregroundStyle(.primary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 12)
            Toggle("Install automatically", isOn: $manager.autoInstallUpdates)
                .toggleStyle(.checkbox)
                .font(.caption)
                .foregroundStyle(.secondary)
            Button {
                manager.installUpdate()
            } label: {
                Text(manager.updateInstalling ? "Installing…" : "Install")
            }
            .controlSize(.small)
            .disabled(!manager.updateInstallEnabled)
        }
        .padding(.leading, 104)
        .padding(.trailing, 18)
        .padding(.vertical, 6)
        .background(Color(nsColor: .underPageBackgroundColor))
        .overlay(alignment: .bottom) {
            Divider()
        }
    }

    private var updateStripeText: String {
        let current = state.appVersion.trimmingCharacters(in: .whitespacesAndNewlines)
        if current.isEmpty {
            return "Update available: \(manager.updateVersion)"
        }
        return "Update available: \(manager.updateVersion) (you're on \(current))"
    }

    private var systemVersionLabel: String {
        let app = state.appVersion.trimmingCharacters(in: .whitespacesAndNewlines)
        let daemon = state.daemonBinaryVersion.trimmingCharacters(in: .whitespacesAndNewlines)
        switch (app.isEmpty, daemon.isEmpty) {
        case (true, true): return ""
        case (false, true): return "gui v\(app)"
        case (true, false): return "daemon v\(daemon)"
        case (false, false) where app == daemon: return "v\(app)"
        case (false, false): return "gui v\(app) · daemon v\(daemon)"
        }
    }

    private var headerIdentity: some View {
        HStack(spacing: 6) {
            if let shownNetwork, state.networks.count > 1 {
                networkStatusDot(shownNetwork)
            }
            Picker("", selection: headerNetworkSelection) {
                ForEach(state.networks, id: \.id) { network in
                    Text(displayName(network))
                        .tag(network.id)
                }
            }
            .labelsHidden()
            .pickerStyle(.menu)
            .disabled(state.networks.isEmpty)
            .frame(maxWidth: 160, alignment: .leading)

            Button {
                addNetworkPresented = true
            } label: {
                Image(systemName: "plus")
                    .font(.system(size: 11, weight: .semibold))
            }
            .buttonStyle(.borderless)
            .help("Add network")
        }
        .frame(width: 220, alignment: .leading)
    }

    private var headerNetworkSelection: Binding<String> {
        Binding(
            get: { shownNetwork?.id ?? state.networks.first?.id ?? "" },
            set: { networkId in
                guard !networkId.isEmpty else { return }
                shownNetworkId = networkId
                selectedSidebarItem = .devices
            }
        )
    }

    private func networkStatusDot(_ network: NativeNetworkState) -> some View {
        Circle()
            .fill(network.enabled ? Color.green : Color.secondary.opacity(0.55))
            .frame(width: 7, height: 7)
    }

    private var headerVpnControl: some View {
        HStack(spacing: 8) {
            Text(headerVpnStatusText)
                .font(.caption2)
                .foregroundStyle(headerStatusTextColor)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: 150, alignment: .trailing)
                .layoutPriority(1)
            if headerStatusDotVisible {
                Circle()
                    .fill(headerStatusColor)
                    .frame(width: 7, height: 7)
            }
            headerVpnSwitch
        }
        .help(manager.vpnSwitchEnabled ? "Turn VPN off" : "Turn VPN on")
    }

    private var headerVpnSwitch: some View {
        let disabled = manager.actionInFlight || !state.vpnControlSupported || activeNetwork == nil
        return Button {
            manager.toggleVpn()
        } label: {
            ZStack(alignment: manager.vpnSwitchEnabled ? .trailing : .leading) {
                Capsule()
                    .fill(manager.vpnSwitchEnabled ? Color.accentColor : Color(nsColor: .tertiaryLabelColor).opacity(0.45))
                    .frame(width: 52, height: 26)
                Circle()
                    .fill(Color.white)
                    .frame(width: 22, height: 22)
                    .shadow(color: .black.opacity(0.22), radius: 1, y: 1)
                    .padding(2)
            }
            .frame(width: 52, height: 26)
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .accessibilityLabel(manager.vpnSwitchEnabled ? "Turn VPN off" : "Turn VPN on")
        .accessibilityValue(manager.vpnSwitchEnabled ? "On" : "Off")
    }

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 5) {
            sidebarButton(.devices, "Devices", "circle.grid.2x2.fill")
            sidebarButton(.routing, "Exit Nodes", "arrow.triangle.branch")
            sidebarButton(.settings, "Settings", "gearshape")
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 10)
        .padding(.top, 32)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color(nsColor: .controlBackgroundColor))
    }

    private func sidebarButton(_ item: SidebarItem, _ title: String, _ systemImage: String) -> some View {
        let selected = (selectedSidebarItem ?? .devices) == item
        return Button {
            selectedSidebarItem = item
        } label: {
            Label(title, systemImage: systemImage)
                .font(.subheadline.weight(.semibold))
                .labelStyle(.titleAndIcon)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                .frame(height: 32)
                .background(selected ? Color.accentColor : Color.clear, in: RoundedRectangle(cornerRadius: 7))
                .foregroundStyle(selected ? Color.white : Color.primary)
                .contentShape(RoundedRectangle(cornerRadius: 7))
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder
    private var detailPane: some View {
        switch selectedSidebarItem ?? .devices {
        case .devices:
            if let shownNetwork {
                devicesPane(shownNetwork)
            } else {
                setupPane
            }
        case .routing:
            pageScroll {
                pageTitle("Exit Nodes", "arrow.triangle.branch")
                if let shownNetwork {
                    routingSection(shownNetwork)
                }
            }
        case .settings:
            pageScroll {
                pageTitle("Settings", "gearshape")
                settingsSection
            }
        }
    }

    private func pageScroll<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                content()
            }
            .padding(.horizontal, 28)
            .padding(.top, 28)
            .padding(.bottom, 32)
            .frame(maxWidth: 920, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .topLeading)
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private func pageTitle(_ title: String, _ systemImage: String) -> some View {
        Label(title, systemImage: systemImage)
            .font(.system(size: 24, weight: .semibold))
    }

    private func devicesPane(_ network: NativeNetworkState) -> some View {
        HStack(spacing: 0) {
            deviceListColumn(network)
                .frame(minWidth: 290, idealWidth: 330, maxWidth: 360)
            Divider()
            deviceDetailColumn(network)
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private var setupPane: some View {
        pageScroll {
            pageTitle("Add Network", "plus.circle")
            createNetworkSection
            joinNetworkSection(nil)
        }
    }

    private func deviceListColumn(_ network: NativeNetworkState) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 3) {
                        Text("Devices")
                            .font(.system(size: 24, weight: .semibold))
                        Text(deviceAvailabilityText(network))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Spacer()
                    deviceHeaderActions(network)
                }
                TextField("Search", text: $deviceSearch)
                    .textFieldStyle(.roundedBorder)
            }
            .padding(.horizontal, 20)
            .padding(.top, 24)
            .padding(.bottom, 4)

            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    let participants = visibleParticipants(network)
                    if participants.isEmpty {
                        emptyRow("No matching devices", systemImage: "circle.dotted")
                    } else {
                        VStack(alignment: .leading, spacing: 6) {
                            Text(displayName(network))
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .padding(.horizontal, 10)
                            ForEach(participants, id: \.pubkeyHex) { participant in
                                deviceListRow(participant, network: network)
                            }
                        }
                    }

                    joinRequestsSection(network)
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 24)
            }
        }
    }

    private func deviceHeaderActions(_ network: NativeNetworkState) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) {
                activateNetworkButton(network)
                addDeviceButton(network)
            }
            HStack(spacing: 8) {
                activateNetworkButton(network, compact: true)
                addDeviceButton(network, compact: true)
            }
        }
    }

    @ViewBuilder
    private func activateNetworkButton(_ network: NativeNetworkState, compact: Bool = false) -> some View {
        if !network.enabled {
            Button {
                activateNetwork(network)
            } label: {
                if compact {
                    Image(systemName: "checkmark.circle.fill")
                } else {
                    Label("Activate", systemImage: "checkmark.circle.fill")
                }
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.small)
            .disabled(manager.actionInFlight)
            .help("Activate this network")
        }
    }

    @ViewBuilder
    private func addDeviceButton(_ network: NativeNetworkState, compact: Bool = false) -> some View {
        if network.localIsAdmin {
            Button {
                addDevicePresented = true
            } label: {
                if compact {
                    Image(systemName: "plus")
                } else {
                    Label("Add device", systemImage: "plus")
                }
            }
            .controlSize(.small)
            .help("Add device to this network")
        }
    }

    private func deviceListRow(_ participant: NativeParticipantState, network: NativeNetworkState) -> some View {
        let selected = selectedParticipant(in: network)?.pubkeyHex == participant.pubkeyHex
        return Button {
            selectedDevicePubkeyHex = participant.pubkeyHex
        } label: {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 7) {
                    connectivityDot(participant, size: 8)
                    Text(deviceName(participant))
                        .font(.subheadline.weight(.semibold))
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    if isSelf(participant) {
                        badge("This device", style: selected ? .selected : .ok)
                    }
                    if participant.isAdmin {
                        badge("Admin", style: selected ? .selected : .muted)
                    }
                    if participant.offersExitNode {
                        badge("Exit", style: selected ? .selected : .warn)
                    }
                    if isFipsRouted(participant) {
                        badge("via mesh", style: selected ? .selected : .muted)
                    }
                }
                HStack(spacing: 6) {
                    if !deviceSubtitle(participant).isEmpty {
                        Text(deviceSubtitle(participant))
                    }
                    if !cleanIp(participant.tunnelIp).isEmpty {
                        Text(cleanIp(participant.tunnelIp))
                    }
                }
                .font(.caption)
                .foregroundStyle(selected ? Color.white.opacity(0.78) : Color.secondary)
                .lineLimit(1)
            }
            .foregroundStyle(selected ? Color.white : Color.primary)
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                selected ? Color.accentColor : Color.clear,
                in: RoundedRectangle(cornerRadius: 7)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func deviceDetailColumn(_ network: NativeNetworkState) -> some View {
        ScrollView {
            if let participant = selectedParticipant(in: network) {
                VStack(alignment: .leading, spacing: 22) {
                    deviceDetailHeader(participant, network: network)
                    if network.localIsAdmin && !isSelf(participant) {
                        deviceAdminSection(participant, network: network)
                    }
                    deviceAddressesSection(participant)
                    deviceConnectivitySection(participant)
                }
                .padding(.horizontal, 22)
                .padding(.top, 26)
                .padding(.bottom, 30)
                .frame(maxWidth: 640, alignment: .topLeading)
                .frame(maxWidth: .infinity, alignment: .topLeading)
            } else {
                VStack(alignment: .leading, spacing: 10) {
                    Text("Devices")
                        .font(.system(size: 24, weight: .semibold))
                    emptyRow("No devices yet", systemImage: "circle.dotted")
                }
                .padding(28)
                .frame(maxWidth: .infinity, alignment: .topLeading)
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private func deviceDetailHeader(_ participant: NativeParticipantState, network: NativeNetworkState) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(deviceName(participant))
                        .font(.system(size: 24, weight: .semibold))
                        .lineLimit(2)
                    if isSelf(participant) || participant.isAdmin || participant.offersExitNode || participant.reachable {
                        HStack(spacing: 6) {
                            if isSelf(participant) {
                                badge("This device", style: .ok)
                            }
                            if participant.isAdmin {
                                badge("Admin", style: .muted)
                            }
                            if participant.offersExitNode {
                                badge("Exit", style: .warn)
                            }
                            if isDirectFipsPeer(participant) {
                                badge("direct connection", style: .ok)
                            } else if isFipsRouted(participant) {
                                badge("via mesh", style: .muted)
                            }
                        }
                    }
                }
                Spacer()
                HStack(spacing: 7) {
                    connectivityDot(participant, size: 8)
                    Text(deviceStatusText(participant))
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private func deviceAdminSection(_ participant: NativeParticipantState, network: NativeNetworkState) -> some View {
        surface {
            sectionHeader("Manage Device", systemImage: "person.badge.key")

            HStack(spacing: 8) {
                label("Name")
                TextField("Name", text: participantAliasBinding(participant))
                Button {
                    manager.setParticipantAlias(
                        npub: participant.npub,
                        alias: participantAliasDrafts[participant.pubkeyHex] ?? participant.magicDnsAlias
                    )
                } label: {
                    Label("Save", systemImage: "checkmark")
                }
                .disabled(manager.actionInFlight)
            }

            deviceActionButtons(participant, network: network)
        }
    }

    private func deviceAddressesSection(_ participant: NativeParticipantState) -> some View {
        surface {
            Text("Addresses")
                .font(.headline)
            detailValueRow("MagicDNS", deviceMagicDnsName(participant))
            detailValueRow("VPN IP", cleanIp(participant.tunnelIp))
            detailValueRow("Device ID", participant.npub)
        }
    }

    private func deviceConnectivitySection(_ participant: NativeParticipantState) -> some View {
        surface {
            Text("Connectivity")
                .font(.headline)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 130), alignment: .leading)], alignment: .leading, spacing: 12) {
                metric("Role", deviceRoleText(participant))
                metric("State", deviceStatusText(participant))
                metric("FIPS path", fipsPathText(participant))
                metric("Last seen", participant.lastSeenText.isEmpty ? "-" : participant.lastSeenText)
                metric("Sent", formatBytes(participant.txBytes))
                metric("Received", formatBytes(participant.rxBytes))
            }
            if !participant.statusText.isEmpty {
                Text(participant.statusText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
        }
    }

    private func deviceActionButtons(_ participant: NativeParticipantState, network: NativeNetworkState) -> some View {
        HStack(spacing: 6) {
            Button {
                manager.toggleAdmin(networkId: network.id, participant: participant)
            } label: {
                Label(
                    participant.isAdmin ? "Remove admin" : "Make admin",
                    systemImage: participant.isAdmin ? "star.slash" : "star"
                )
            }
            .disabled(manager.actionInFlight)
            .help(participant.isAdmin ? "Remove admin" : "Make admin")
            Button(role: .destructive) {
                pendingParticipantRemoval = PendingParticipantRemoval(
                    networkId: network.id,
                    npub: participant.npub,
                    deviceName: deviceName(participant)
                )
            } label: {
                Label("Remove", systemImage: "trash")
            }
            .disabled(isSelf(participant) || manager.actionInFlight)
            .help("Remove device")
        }
        .controlSize(.small)
        .confirmationDialog(
            "Remove \(pendingParticipantRemoval?.deviceName ?? "device")?",
            isPresented: Binding(
                get: { pendingParticipantRemoval != nil },
                set: { if !$0 { pendingParticipantRemoval = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button("Remove", role: .destructive) {
                if let target = pendingParticipantRemoval {
                    manager.removeParticipant(networkId: target.networkId, npub: target.npub)
                }
                pendingParticipantRemoval = nil
            }
            Button("Cancel", role: .cancel) { pendingParticipantRemoval = nil }
        } message: {
            Text("This removes the device from the network's roster. They keep the network locally but won't be in this roster anymore.")
        }
    }

    private func detailValueRow(_ title: String, _ value: String) -> some View {
        let displayValue = value.isEmpty ? "-" : value
        return VStack(alignment: .leading, spacing: 4) {
            Text(displayValue)
                .font(.subheadline.weight(.semibold))
                .lineLimit(1)
                .truncationMode(.middle)
                .textSelection(.enabled)
            HStack(spacing: 6) {
                Text(title)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                if !value.isEmpty {
                    Button {
                        manager.copy(value)
                    } label: {
                        Image(systemName: "doc.on.doc")
                    }
                    .buttonStyle(.borderless)
                    .help("Copy")
                }
            }
        }
        .padding(.vertical, 3)
    }

    @ViewBuilder
    private func joinRequestsSection(_ network: NativeNetworkState) -> some View {
        if !network.inboundJoinRequests.isEmpty {
            surface {
                sectionHeader("Join Requests", systemImage: "person.badge.plus")
                ForEach(network.inboundJoinRequests, id: \.requesterPubkeyHex) { request in
                    HStack(spacing: 10) {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(request.requesterNodeName.isEmpty ? "New device" : request.requesterNodeName)
                                .font(.headline)
                            Text("\(request.requesterNpub) · \(request.requestedAtText)")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                        Spacer()
                        copyButton(value: request.requesterNpub, copied: .peerNpub, peerNpub: request.requesterNpub, systemImage: "doc.on.doc")
                        Button(role: .destructive) {
                            manager.rejectJoinRequest(networkId: network.id, requesterNpub: request.requesterNpub)
                        } label: {
                            Text("Reject")
                        }
                        .disabled(!network.localIsAdmin || manager.actionInFlight)
                        Button("Accept") {
                            manager.acceptJoinRequest(networkId: network.id, requesterNpub: request.requesterNpub)
                        }
                        .disabled(!network.localIsAdmin || manager.actionInFlight)
                    }
                    .padding(.vertical, 4)
                }
            }
        }
    }

    private func inviteSection(_ network: NativeNetworkState) -> some View {
        let invite = network.enabled ? state.activeNetworkInvite : ""
        return surface {
            HStack(alignment: .top, spacing: 18) {
                InviteQRCodeView(invite: invite)
                    .frame(width: 150, height: 150)
                VStack(alignment: .leading, spacing: 12) {
                    sectionHeader("Invite Devices", systemImage: "qrcode")
                    Text("Your invite")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    HStack(spacing: 8) {
                        Text(invite.isEmpty ? "No invite" : invite)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .textSelection(.enabled)
                            .padding(.horizontal, 10)
                            .frame(height: 32)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 6))
                        copyButton(value: invite, copied: .invite, systemImage: "doc.on.doc")
                            .disabled(invite.isEmpty)
                        Button {
                            manager.share(invite)
                        } label: {
                            Label("Share", systemImage: "square.and.arrow.up")
                        }
                        .disabled(invite.isEmpty)
                    }
                    HStack {
                        Toggle("Allow join requests", isOn: Binding(
                            get: { network.joinRequestsEnabled },
                            set: { manager.setJoinRequests(networkId: network.id, enabled: $0) }
                        ))
                        .disabled(!network.localIsAdmin || manager.actionInFlight)
                        .help("Allow devices with an invite to request access")
                        badge(network.joinRequestsEnabled ? "Allowed" : "Blocked", style: network.joinRequestsEnabled ? .ok : .muted)
                        Spacer()
                        Button {
                            state.inviteBroadcastActive ? manager.stopInviteBroadcast() : manager.startInviteBroadcast()
                        } label: {
                            Label(
                                state.inviteBroadcastActive
                                    ? "Broadcasting · \(formatRemaining(state.inviteBroadcastRemainingSecs))"
                                    : "Broadcast invite",
                                systemImage: state.inviteBroadcastActive ? "stop.circle" : "dot.radiowaves.left.and.right"
                            )
                        }
                        .disabled(manager.actionInFlight || !network.enabled)
                    }
                }
            }
        }
    }

    private func addByDeviceIdSection(_ network: NativeNetworkState) -> some View {
        let trimmed = addByDeviceIdInput.trimmingCharacters(in: .whitespacesAndNewlines)
        let invalid = !trimmed.isEmpty && !isValidDeviceId(trimmed)
        return surface {
            sectionHeader("Add by Device ID", systemImage: "plus")
            Text("Manual pairing: enter the other device's Device ID. They also need to add yours.")
                .font(.caption)
                .foregroundStyle(.secondary)
            HStack(spacing: 8) {
                TextField("Device ID", text: $addByDeviceIdInput)
                    .textFieldStyle(.roundedBorder)
                    .overlay(
                        RoundedRectangle(cornerRadius: 6)
                            .stroke(Color.red, lineWidth: invalid ? 1 : 0)
                    )
                TextField("Name (optional)", text: $addByDeviceIdAlias)
                    .textFieldStyle(.roundedBorder)
                    .frame(maxWidth: 200)
                Button {
                    manager.addParticipant(networkId: network.id, npub: trimmed, alias: addByDeviceIdAlias)
                    addByDeviceIdInput = ""
                    addByDeviceIdAlias = ""
                } label: {
                    Label("Add", systemImage: "plus")
                }
                .disabled(trimmed.isEmpty || invalid || manager.actionInFlight)
            }
            if invalid {
                Text("Not a valid device ID")
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        }
    }

    private var createNetworkSection: some View {
        surface {
            sectionHeader("Create Network", systemImage: "plus.circle")
            HStack(spacing: 8) {
                TextField("Network name", text: $networkNameInput)
                    .onSubmit {
                        addNetwork()
                        finishCreateNetwork()
                    }
                Button {
                    addNetwork(defaultName: "My Network")
                    finishCreateNetwork()
                } label: {
                    Label("Create", systemImage: "plus")
                }
                .disabled(manager.actionInFlight)
            }
        }
    }

    /// Land on the new network's Devices view right after Create. The
    /// sidebar may have been on Routing/Settings; from the Add Network
    /// sheet we also have to dismiss. Both are no-ops if already in the
    /// target state.
    private func finishCreateNetwork() {
        addNetworkPresented = false
        selectedSidebarItem = .devices
    }

    private func joinNetworkSection(_ network: NativeNetworkState?) -> some View {
        surface {
            sectionHeader("Join Network", systemImage: "arrow.down.circle")
            Text("Paste invite code")
                .font(.caption)
                .foregroundStyle(.secondary)
            HStack(spacing: 8) {
                TextField("nvpn://invite/…", text: $manager.inviteInput)
                    .onChange(of: manager.inviteInput) { _, newValue in
                        // Auto-import when the field becomes a valid invite —
                        // saves the user a click. importInvite clears the
                        // field, which prevents re-firing.
                        let trimmed = newValue.trimmingCharacters(in: .whitespacesAndNewlines)
                        if trimmed.lowercased().hasPrefix("nvpn://invite/") {
                            manager.importInvite(trimmed)
                        }
                    }
                    .onSubmit {
                        manager.importInvite(manager.inviteInput)
                    }
                Button {
                    pasteInviteFromClipboard()
                } label: {
                    Label("Paste", systemImage: "doc.on.clipboard")
                }
                Button {
                    showingQrScanner = true
                } label: {
                    Label("Scan", systemImage: "camera.viewfinder")
                }
                Button {
                    manager.chooseInviteQrImage()
                } label: {
                    Label("From file", systemImage: "qrcode.viewfinder")
                }
            }
            if let network {
                if network.outboundJoinRequest != nil {
                    badge("Join requested", style: .warn)
                } else if !network.inviteInviterNpub.isEmpty {
                    Button {
                        manager.requestNetworkJoin(networkId: network.id)
                    } label: {
                        Label("Request Access", systemImage: "person.badge.plus")
                    }
                    .disabled(manager.actionInFlight)
                }
            }

            manualJoinDisclosure

            Divider()

            HStack {
                Text("Nearby invites")
                    .font(.subheadline.weight(.medium))
                Spacer()
                Button {
                    state.nearbyDiscoveryActive ? manager.stopNearbyDiscovery() : manager.startNearbyDiscovery()
                } label: {
                    Label(
                        state.nearbyDiscoveryActive
                            ? "Listening · \(formatRemaining(state.nearbyDiscoveryRemainingSecs))"
                            : "Look for nearby",
                        systemImage: state.nearbyDiscoveryActive ? "stop.circle" : "dot.radiowaves.left.and.right"
                    )
                }
                .disabled(manager.actionInFlight)
            }

            if state.nearbyDiscoveryActive && state.lanPeers.isEmpty {
                emptyRow("No nearby invites yet", systemImage: "wifi")
            } else {
                ForEach(state.lanPeers, id: \.invite) { peer in
                    HStack {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(peer.nodeName.isEmpty ? peer.npub : peer.nodeName)
                            Text(peer.networkName)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("Join") {
                            manager.importInvite(peer.invite)
                        }
                    }
                    .padding(.vertical, 4)
                }
            }
        }
    }

    private var manualJoinDisclosure: some View {
        let admin = manualJoinAdminId.trimmingCharacters(in: .whitespacesAndNewlines)
        let mesh = manualJoinMeshId.trimmingCharacters(in: .whitespacesAndNewlines)
        let adminInvalid = !admin.isEmpty && !isValidDeviceId(admin)
        let canSubmit = !admin.isEmpty && !mesh.isEmpty && !adminInvalid
        return DisclosureGroup("Add manually", isExpanded: $manualJoinExpanded) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Both sides have to add each other. Get the admin's Device ID and the network ID from them, then have the admin add your Device ID on their Add device page.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextField("Admin Device ID", text: $manualJoinAdminId)
                    .textFieldStyle(.roundedBorder)
                    .overlay(
                        RoundedRectangle(cornerRadius: 6)
                            .stroke(Color.red, lineWidth: adminInvalid ? 1 : 0)
                    )
                if adminInvalid {
                    Text("Not a valid device ID")
                        .font(.caption)
                        .foregroundStyle(.red)
                }
                TextField("Network ID", text: $manualJoinMeshId)
                    .textFieldStyle(.roundedBorder)
                Button {
                    manager.manualAddNetwork(adminNpub: admin, meshNetworkId: mesh)
                    manualJoinAdminId = ""
                    manualJoinMeshId = ""
                    manualJoinExpanded = false
                } label: {
                    Label("Add", systemImage: "plus")
                }
                .disabled(!canSubmit || manager.actionInFlight)
            }
            .padding(.top, 6)
        }
        .font(.subheadline)
    }

    private func pasteInviteFromClipboard() {
        if let text = NSPasteboard.general.string(forType: .string) {
            manager.inviteInput = text.trimmingCharacters(in: .whitespacesAndNewlines)
        }
    }

    private func formatRemaining(_ seconds: UInt64) -> String {
        if seconds == 0 { return "off" }
        let minutes = seconds / 60
        if minutes == 0 { return "\(seconds)s" }
        let secs = seconds % 60
        return secs == 0 ? "\(minutes)m" : String(format: "%dm%02ds", minutes, secs)
    }

    private func routingSection(_ network: NativeNetworkState) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            surface {
                sectionHeader("Exit Nodes", systemImage: "arrow.triangle.branch")
                TextField("Search devices", text: $exitNodeSearch)
                    .textFieldStyle(.roundedBorder)

                VStack(spacing: 8) {
                    routeChoice(
                        title: "Direct",
                        subtitle: "Use normal internet routing",
                        selected: !state.wireguardExitEnabled && state.exitNode.isEmpty,
                        enabled: true
                    ) {
                        manager.selectDirectExit()
                    }

                    routeChoice(
                        title: "WireGuard upstream",
                        subtitle: wireguardUpstreamSubtitle,
                        selected: state.wireguardExitEnabled,
                        enabled: state.wireguardExitConfigured
                    ) {
                        manager.selectWireGuardUpstreamExit()
                    }

                    ForEach(exitNodeCandidates(network), id: \.pubkeyHex) { participant in
                        routeChoice(
                            title: deviceName(participant),
                            subtitle: participant.offersExitNode ? participant.statusText : "Exit not offered",
                            selected: !state.wireguardExitEnabled && state.exitNode == participant.npub,
                            enabled: participant.offersExitNode
                        ) {
                            manager.selectPeerExit(participant.npub)
                        }
                    }
                }

                Divider()

                Toggle(
                    "Offer this device as an exit node in \(shownNetworkLabel)",
                    isOn: Binding(
                        get: { state.advertiseExitNode },
                        set: { manager.setAdvertiseExitNode($0) }
                    )
                )
                .disabled(manager.actionInFlight)
            }
            wireGuardExitSettings
        }
    }

    private var shownNetworkLabel: String {
        shownNetwork.map(displayName) ?? "this network"
    }

    private var wireguardUpstreamSubtitle: String {
        if !state.wireguardExitConfigured {
            return "Paste a config below to enable"
        }
        let endpoint = state.wireguardExitEndpoint
        if endpoint.isEmpty {
            return "Configured"
        }
        return "via \(endpoint)"
    }

    private func routeChoice(
        title: String,
        subtitle: String,
        selected: Bool,
        enabled: Bool,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack {
                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(selected ? .green : .secondary)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .foregroundStyle(.primary)
                    Text(subtitle)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 9)
            .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 8))
        }
        .buttonStyle(.plain)
        .disabled(!enabled || manager.actionInFlight)
        .opacity(enabled ? 1 : 0.55)
    }

    private var settingsSection: some View {
        VStack(alignment: .leading, spacing: 14) {
            deviceSettings
            networkSettings
            systemSettings
            diagnosticsSection
        }
    }

    private var deviceSettings: some View {
        surface {
            sectionHeader("This Device", systemImage: "macbook")
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                GridRow {
                    label("Name")
                    TextField("Name", text: $nodeName)
                }
                GridRow {
                    label("Tunnel IP")
                    TextField("Tunnel IP", text: $tunnelIp)
                }
            }
            HStack(spacing: 14) {
                Toggle("Autoconnect", isOn: Binding(
                    get: { state.autoconnect },
                    set: { manager.setAutoconnect($0) }
                ))
                Toggle("Launch on startup", isOn: Binding(
                    get: { state.launchOnStartup },
                    set: { manager.setLaunchOnStartup($0) }
                ))
                .disabled(!state.startupSettingsSupported)
                Toggle("Menu bar on close", isOn: Binding(
                    get: { state.closeToTrayOnClose },
                    set: { manager.setCloseToTray($0) }
                ))
                .disabled(!state.trayBehaviorSupported)
            }
            Toggle("Block internet if exit node disconnects", isOn: Binding(
                get: { state.exitNodeLeakProtection },
                set: { manager.setExitNodeLeakProtection($0) }
            ))
            .disabled(manager.actionInFlight)
            Button {
                manager.saveNodeSettings(
                    nodeName: nodeName,
                    endpoint: endpoint,
                    tunnelIp: tunnelIp,
                    listenPort: listenPort,
                    magicDnsSuffix: magicDnsSuffix
                )
            } label: {
                Label("Save", systemImage: "checkmark")
            }
            .disabled(manager.actionInFlight)
        }
    }

    private var wireGuardExitSettings: some View {
        surface {
            sectionHeader("WireGuard Upstream", systemImage: "network")
            Text("Paste a WireGuard config from an upstream VPN provider such as Mullvad or Proton VPN.")
                .font(.caption)
                .foregroundStyle(.secondary)

            TextEditor(text: $wireguardExitConfig)
                .font(.system(.body, design: .monospaced))
                .frame(minHeight: 180)
                .padding(6)
                .background(Color(nsColor: .textBackgroundColor))
                .clipShape(RoundedRectangle(cornerRadius: 6))
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(Color(nsColor: .separatorColor))
                )

            Button {
                manager.saveWireGuardExitConfig(wireguardExitConfig)
            } label: {
                Label("Save", systemImage: "checkmark")
            }
            .disabled(manager.actionInFlight)
        }
    }

    private var networkSettings: some View {
        surface {
            HStack {
                sectionHeader("Networks", systemImage: "rectangle.stack")
                Spacer()
                TextField("New network", text: $networkNameInput)
                    .frame(width: 180)
                    .onSubmit { addNetwork() }
                Button {
                    addNetwork()
                } label: {
                    Image(systemName: "plus")
                }
                .disabled(networkNameInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || manager.actionInFlight)
            }

            if let network = shownNetwork {
                Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                    GridRow {
                        label(network.enabled ? "Active" : "Shown")
                        TextField("Name", text: networkNameBinding(network))
                        Button {
                            manager.renameNetwork(networkId: network.id, name: networkNameDrafts[network.id] ?? network.name)
                        } label: {
                            Image(systemName: "checkmark")
                        }
                        .disabled(!network.localIsAdmin || manager.actionInFlight)
                    }
                    GridRow {
                        label("Network ID")
                        Text(network.networkId)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .textSelection(.enabled)
                        copyButton(value: network.networkId, copied: .meshId, systemImage: "doc.on.doc")
                    }
                    GridRow {
                        label("Requests")
                        Toggle("", isOn: Binding(
                            get: { network.joinRequestsEnabled },
                            set: { manager.setJoinRequests(networkId: network.id, enabled: $0) }
                        ))
                        .labelsHidden()
                        .disabled(!network.localIsAdmin || manager.actionInFlight)
                        Text(network.joinRequestsEnabled ? "Allowed" : "Blocked")
                            .foregroundStyle(.secondary)
                    }
                    GridRow {
                        label("")
                        Button(role: .destructive) {
                            pendingNetworkRemoval = network
                        } label: {
                            Label("Delete this network", systemImage: "trash")
                        }
                        .buttonStyle(.borderless)
                        .disabled(manager.actionInFlight)
                        .confirmationDialog(
                            "Remove \(displayName(network))?",
                            isPresented: Binding(
                                get: { pendingNetworkRemoval?.id == network.id },
                                set: { if !$0 { pendingNetworkRemoval = nil } }
                            ),
                            titleVisibility: .visible
                        ) {
                            Button("Remove", role: .destructive) {
                                if let target = pendingNetworkRemoval {
                                    manager.removeNetwork(target.id)
                                }
                                pendingNetworkRemoval = nil
                            }
                            Button("Cancel", role: .cancel) { pendingNetworkRemoval = nil }
                        } message: {
                            Text("This deletes the network from this device.")
                        }
                    }
                }
            }

            disclosureSection(
                title: "Saved Networks",
                systemImage: "rectangle.stack",
                isExpanded: $savedNetworksExpanded,
                font: .subheadline.weight(.medium)
            ) {
                VStack(alignment: .leading, spacing: 10) {
                    if manager.inactiveNetworks.isEmpty {
                        emptyRow("No saved networks", systemImage: "rectangle.stack")
                    } else {
                        ForEach(manager.inactiveNetworks, id: \.id) { network in
                            savedNetworkRow(network)
                        }
                    }
                }
                .padding(.top, 8)
            }
        }
    }

    private func savedNetworkRow(_ network: NativeNetworkState) -> some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                TextField("Name", text: networkNameBinding(network))
                    .textFieldStyle(.plain)
                Text("\(network.onlineCount) of \(network.expectedCount) connected")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button("Activate") {
                activateNetwork(network)
            }
            .disabled(manager.actionInFlight)
            Button(role: .destructive) {
                pendingNetworkRemoval = network
            } label: {
                Image(systemName: "trash")
            }
            .disabled(manager.actionInFlight)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 8))
        .confirmationDialog(
            "Remove \(pendingNetworkRemoval.map(displayName) ?? "network")?",
            isPresented: Binding(
                get: { pendingNetworkRemoval?.id == network.id },
                set: { if !$0 { pendingNetworkRemoval = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button("Remove", role: .destructive) {
                if let target = pendingNetworkRemoval {
                    manager.removeNetwork(target.id)
                }
                pendingNetworkRemoval = nil
            }
            Button("Cancel", role: .cancel) { pendingNetworkRemoval = nil }
        } message: {
            Text("This deletes the network from this device.")
        }
    }

    private var systemSettings: some View {
        surface {
            HStack(spacing: 8) {
                sectionHeader("System", systemImage: "gearshape.2")
                if !systemVersionLabel.isEmpty {
                    Text(systemVersionLabel)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
                Spacer()
                if manager.serviceSettling || manager.updateChecking || manager.updateInstalling {
                    ProgressView()
                        .controlSize(.small)
                }
            }

            HStack(spacing: 8) {
                badge(state.serviceInstalled ? "Service installed" : "Service missing", style: state.serviceInstalled ? .ok : .warn)
                badge(state.serviceRunning ? "Running" : "Stopped", style: state.serviceRunning ? .ok : .muted)
                if manager.serviceUpdateRecommended {
                    badge("Update available", style: .warn)
                }
                badge(state.cliInstalled ? "CLI installed" : "CLI missing", style: state.cliInstalled ? .ok : .muted)
                badge(manager.updateAvailable ? "Update \(manager.updateVersion)" : "Up to date", style: manager.updateAvailable ? .warn : .ok)
            }

            if manager.serviceUpdateRecommended || !state.serviceStatusDetail.isEmpty || !manager.updateStatus.isEmpty {
                Text(firstNonEmpty(manager.updateStatus, state.serviceStatusDetail, fallback: ""))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }

            HStack {
                Button {
                    manager.installService()
                } label: {
                    Label(serviceInstallButtonTitle, systemImage: manager.serviceUpdateRecommended ? "arrow.up.circle" : "arrow.down.to.line")
                }
                .disabled(!state.serviceSupported || manager.actionInFlight || manager.serviceSettling)

                Button {
                    manager.checkForUpdates()
                } label: {
                    Label("Check Updates", systemImage: "arrow.triangle.2.circlepath")
                }
                .disabled(manager.updateChecking || manager.updateInstalling)

                Button {
                    manager.installCli()
                } label: {
                    Label(state.cliInstalled ? "Reinstall CLI" : "Install CLI", systemImage: "terminal")
                }
                .disabled(!state.cliInstallSupported || manager.actionInFlight)
            }
        }
    }

    private var diagnosticsSection: some View {
        surface {
            disclosureSection(
                title: "Diagnostics",
                systemImage: "waveform.path.ecg",
                isExpanded: $diagnosticsExpanded
            ) {
                VStack(alignment: .leading, spacing: 12) {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 170), alignment: .leading)], alignment: .leading, spacing: 10) {
                        metric("Interface", state.network.defaultInterface.isEmpty ? "unknown" : state.network.defaultInterface)
                        metric("IPv4", state.network.primaryIpv4.isEmpty ? "-" : state.network.primaryIpv4)
                        metric("IPv6", state.network.primaryIpv6.isEmpty ? "-" : state.network.primaryIpv6)
                        metric("Gateway", firstNonEmpty(state.network.gatewayIpv4, state.network.gatewayIpv6, fallback: "unknown"))
                        metric("Mapping", state.portMapping.activeProtocol.isEmpty ? "none" : state.portMapping.activeProtocol)
                        metric("External", state.portMapping.externalEndpoint.isEmpty ? "stun/direct" : state.portMapping.externalEndpoint)
                    }
                    if state.health.isEmpty {
                        emptyRow("No health warnings", systemImage: "checkmark.circle")
                    } else {
                        ForEach(state.health, id: \.code) { issue in
                            HStack(alignment: .top, spacing: 8) {
                                badge(issue.severity, style: healthStyle(issue.severity))
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(issue.summary)
                                    Text(issue.detail)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                }
                .padding(.top, 8)
            }
        }
    }

    private func disclosureSection<Content: View>(
        title: String,
        systemImage: String,
        isExpanded: Binding<Bool>,
        font: Font = .headline,
        @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.easeInOut(duration: 0.14)) {
                    isExpanded.wrappedValue.toggle()
                }
            } label: {
                HStack(spacing: 7) {
                    Image(systemName: "chevron.right")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 10)
                        .rotationEffect(.degrees(isExpanded.wrappedValue ? 90 : 0))
                    Label(title, systemImage: systemImage)
                        .font(font)
                    Spacer(minLength: 0)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(title)
            .accessibilityValue(isExpanded.wrappedValue ? "Expanded" : "Collapsed")

            if isExpanded.wrappedValue {
                content()
            }
        }
    }

    private func surface<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            content()
        }
        .padding(14)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 8))
    }

    private func sectionHeader(_ title: String, systemImage: String) -> some View {
        Label(title, systemImage: systemImage)
            .font(.headline)
    }

    private func emptyRow(_ text: String, systemImage: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: systemImage)
            Text(text)
        }
        .foregroundStyle(.secondary)
        .font(.subheadline)
        .padding(.vertical, 6)
    }

    private func label(_ text: String) -> some View {
        Text(text)
            .foregroundStyle(.secondary)
            .frame(width: 86, alignment: .leading)
    }

    private func metric(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(value.isEmpty ? "-" : value)
                .font(.subheadline.weight(.medium))
                .lineLimit(1)
                .truncationMode(.middle)
                .textSelection(.enabled)
        }
    }

    private func badge(_ text: String, style: BadgeStyle) -> some View {
        Text(text)
            .font(.caption.weight(.semibold))
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .foregroundStyle(style.foreground)
            .background(style.background, in: RoundedRectangle(cornerRadius: 6))
    }

    private func copyButton(
        value: String,
        copied: CopyValue,
        peerNpub: String? = nil,
        systemImage: String
    ) -> some View {
        Button {
            manager.copy(value, as: copied, peerNpub: peerNpub)
        } label: {
            Image(systemName: copyIndicator(copied, peerNpub: peerNpub) ? "checkmark" : systemImage)
        }
        .buttonStyle(.borderless)
    }

    private func copyIndicator(_ copied: CopyValue, peerNpub: String?) -> Bool {
        manager.copiedValue == copied && (copied != .peerNpub || manager.copiedPeerNpub == peerNpub)
    }

    private func networkNameBinding(_ network: NativeNetworkState) -> Binding<String> {
        Binding(
            get: { networkNameDrafts[network.id] ?? network.name },
            set: { networkNameDrafts[network.id] = $0 }
        )
    }

    private func activateNetwork(_ network: NativeNetworkState) {
        guard !network.enabled else { return }
        shownNetworkId = network.id
        manager.setNetworkEnabled(networkId: network.id, enabled: true)
    }

    private func participantAliasBinding(_ participant: NativeParticipantState) -> Binding<String> {
        Binding(
            get: { participantAliasDrafts[participant.pubkeyHex] ?? participant.magicDnsAlias },
            set: { participantAliasDrafts[participant.pubkeyHex] = $0 }
        )
    }

    private func addNetwork(defaultName: String = "") {
        let name = networkNameInput.trimmingCharacters(in: .whitespacesAndNewlines)
        manager.addNetwork(name.isEmpty ? defaultName : name)
        networkNameInput = ""
    }

    private func syncDrafts() {
        if let shownNetworkId,
           !state.networks.contains(where: { $0.id == shownNetworkId }) {
            self.shownNetworkId = nil
        }
        if state.nodeName != lastSyncedNodeName {
            nodeName = state.nodeName
            lastSyncedNodeName = state.nodeName
        }
        if state.endpoint != lastSyncedEndpoint {
            endpoint = state.endpoint
            lastSyncedEndpoint = state.endpoint
        }
        if state.tunnelIp != lastSyncedTunnelIp {
            tunnelIp = state.tunnelIp
            lastSyncedTunnelIp = state.tunnelIp
        }
        if state.listenPort != lastSyncedListenPort {
            listenPort = String(state.listenPort)
            lastSyncedListenPort = state.listenPort
        }
        if state.magicDnsSuffix != lastSyncedMagicDnsSuffix {
            magicDnsSuffix = state.magicDnsSuffix
            lastSyncedMagicDnsSuffix = state.magicDnsSuffix
        }
        if lastSyncedWireguardExitConfig != state.wireguardExitConfig {
            wireguardExitConfig = state.wireguardExitConfig
            lastSyncedWireguardExitConfig = state.wireguardExitConfig
        }

        for network in state.networks {
            if networkNameDrafts[network.id] == nil {
                networkNameDrafts[network.id] = network.name
            }
            for participant in network.participants {
                let key = participant.pubkeyHex
                let alias = participant.magicDnsAlias
                let previousAlias = lastSyncedParticipantAliases[key]
                if participantAliasDrafts[key] == nil || participantAliasDrafts[key] == previousAlias {
                    participantAliasDrafts[key] = alias
                }
                lastSyncedParticipantAliases[key] = alias
            }
        }

        if let network = shownNetwork {
            let participants = sortedParticipants(network)
            if let selectedDevicePubkeyHex,
               participants.contains(where: { $0.pubkeyHex == selectedDevicePubkeyHex }) {
                return
            }
            selectedDevicePubkeyHex = participants.first?.pubkeyHex
        } else {
            selectedDevicePubkeyHex = nil
        }
    }

    private func displayName(_ network: NativeNetworkState) -> String {
        network.name.isEmpty ? "Network" : network.name
    }

    /// A valid device ID is a bech32-encoded npub: `npub1` + 58 bech32 chars.
    private func isValidDeviceId(_ value: String) -> Bool {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count == 63, trimmed.hasPrefix("npub1") else { return false }
        let body = trimmed.dropFirst(5)
        let allowed: Set<Character> = Set("qpzry9x8gf2tvdw0s3jn54khce6mua7l")
        return body.allSatisfy { allowed.contains($0) }
    }

    private var headerVpnStatusText: String {
        manager.vpnStatusText
    }

    private var headerStatusDotVisible: Bool {
        state.exitNodeBlocked || state.exitNodeActive || state.vpnActive || state.vpnEnabled
    }

    private var headerStatusColor: Color {
        if state.exitNodeBlocked {
            return .red
        }
        if state.exitNodeActive || state.vpnActive {
            return .green
        }
        if state.vpnEnabled {
            return .orange
        }
        return .secondary
    }

    private var headerStatusTextColor: Color {
        state.exitNodeBlocked ? .red : .secondary
    }

    private func deviceAvailabilityText(_ network: NativeNetworkState) -> String {
        if network.expectedCount == 0 {
            return "No devices"
        }
        let deviceWord = network.expectedCount == 1 ? "device" : "devices"
        return "\(network.onlineCount) online · \(network.expectedCount) \(deviceWord)"
    }

    private var serviceInstallButtonTitle: String {
        if manager.serviceUpdateRecommended {
            return "Update Service"
        }
        return state.serviceInstalled ? "Reinstall Service" : "Install Service"
    }

    private func sortedParticipants(_ network: NativeNetworkState) -> [NativeParticipantState] {
        network.participants.sorted { lhs, rhs in
            if isSelf(lhs) != isSelf(rhs) {
                return isSelf(lhs)
            }
            if lhs.reachable != rhs.reachable {
                return lhs.reachable && !rhs.reachable
            }
            return deviceName(lhs).localizedCaseInsensitiveCompare(deviceName(rhs)) == .orderedAscending
        }
    }

    private func visibleParticipants(_ network: NativeNetworkState) -> [NativeParticipantState] {
        let needle = deviceSearch.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !needle.isEmpty else {
            return sortedParticipants(network)
        }
        return sortedParticipants(network).filter { participant in
            [
                deviceName(participant),
                participant.alias,
                participant.magicDnsAlias,
                participant.magicDnsName,
                participant.npub,
                participant.tunnelIp,
                deviceStatusText(participant),
            ].contains { $0.lowercased().contains(needle) }
        }
    }

    private func selectedParticipant(in network: NativeNetworkState) -> NativeParticipantState? {
        let participants = sortedParticipants(network)
        if let selectedDevicePubkeyHex,
           let selected = participants.first(where: { $0.pubkeyHex == selectedDevicePubkeyHex }) {
            return selected
        }
        return participants.first
    }

    private func isSelf(_ participant: NativeParticipantState) -> Bool {
        participant.npub == state.ownNpub || participant.meshState == "local"
    }

    private func deviceName(_ participant: NativeParticipantState) -> String {
        if !participant.magicDnsName.isEmpty {
            return participant.magicDnsName
        }
        if isSelf(participant), !state.selfMagicDnsName.isEmpty {
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

    private func deviceSubtitle(_ participant: NativeParticipantState) -> String {
        return ""
    }

    private func deviceMagicDnsName(_ participant: NativeParticipantState) -> String {
        if !participant.magicDnsName.isEmpty {
            return participant.magicDnsName
        }
        if isSelf(participant), !state.selfMagicDnsName.isEmpty {
            return state.selfMagicDnsName
        }
        if !participant.magicDnsAlias.isEmpty, !state.magicDnsSuffix.isEmpty {
            return "\(participant.magicDnsAlias).\(state.magicDnsSuffix)"
        }
        return ""
    }

    private func deviceRoleText(_ participant: NativeParticipantState) -> String {
        var roles: [String] = []
        if isSelf(participant) {
            roles.append("This device")
        }
        if participant.isAdmin {
            roles.append("Admin")
        }
        if participant.offersExitNode {
            roles.append("Exit node")
        }
        return roles.isEmpty ? "Member" : roles.joined(separator: ", ")
    }

    private func deviceStatusText(_ participant: NativeParticipantState) -> String {
        switch participant.state {
        case "local", "online", "present":
            return "Online"
        case "pending":
            return "Connecting"
        case "offline", "absent", "off":
            return "Offline"
        case _ where participant.reachable:
            return "Online"
        default:
            return "Unknown"
        }
    }

    private func fipsPathText(_ participant: NativeParticipantState) -> String {
        if isSelf(participant) {
            return "This device"
        }
        if isDirectFipsPeer(participant) {
            let transport = participant.fipsTransportType.isEmpty ? "" : " (\(participant.fipsTransportType.uppercased()))"
            if participant.fipsSrttMs > 0 {
                return "Direct connection\(transport), \(participant.fipsSrttMs) ms"
            }
            return "Direct connection\(transport)"
        }
        if participant.reachable {
            return "Via mesh"
        }
        if participant.state == "pending" {
            return "Connecting"
        }
        return "Offline"
    }

    private func isDirectFipsPeer(_ participant: NativeParticipantState) -> Bool {
        !isSelf(participant)
            && participant.reachable
            && !participant.fipsTransportAddr.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func isFipsRouted(_ participant: NativeParticipantState) -> Bool {
        !isSelf(participant)
            && participant.reachable
            && participant.fipsTransportAddr.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func connectivityDot(_ participant: NativeParticipantState, size: CGFloat) -> some View {
        Circle()
            .fill(connectivityTint(participant))
            .frame(width: size, height: size)
    }

    private func connectivityTint(_ participant: NativeParticipantState) -> Color {
        switch participant.state {
        case "local", "online", "present":
            return .green
        case "pending":
            return .orange
        default:
            return .secondary
        }
    }

    private func exitNodeCandidates(_ network: NativeNetworkState) -> [NativeParticipantState] {
        let needle = exitNodeSearch.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return network.participants.filter { participant in
            if isSelf(participant) {
                return false
            }
            guard !needle.isEmpty else {
                return true
            }
            return [
                participant.alias,
                participant.magicDnsAlias,
                participant.magicDnsName,
                participant.npub,
                participant.tunnelIp,
            ].contains { $0.lowercased().contains(needle) }
        }
    }

    private func badgeStyle(for state: String) -> BadgeStyle {
        switch state {
        case "local", "online", "present":
            return .ok
        case "pending":
            return .warn
        case "offline", "absent":
            return .bad
        default:
            return .muted
        }
    }

    private func healthStyle(_ severity: String) -> BadgeStyle {
        switch severity {
        case "critical":
            return .bad
        case "warning":
            return .warn
        case "info":
            return .muted
        default:
            return .muted
        }
    }
}

private struct PendingParticipantRemoval {
    let networkId: String
    let npub: String
    let deviceName: String
}

struct InviteQRCodeView: View {
    let invite: String

    var body: some View {
        if invite.isEmpty {
            RoundedRectangle(cornerRadius: 8)
                .fill(Color(nsColor: .textBackgroundColor))
                .overlay(Image(systemName: "qrcode").foregroundStyle(.secondary))
        } else if let image = qrImage(invite) {
            Image(nsImage: image)
                .interpolation(.none)
                .resizable()
                .scaledToFit()
                .padding(8)
                .background(Color.white, in: RoundedRectangle(cornerRadius: 8))
        } else {
            RoundedRectangle(cornerRadius: 8)
                .fill(Color(nsColor: .textBackgroundColor))
                .overlay(Image(systemName: "exclamationmark.triangle").foregroundStyle(.orange))
        }
    }

    private func qrImage(_ text: String) -> NSImage? {
        let data = Data(text.utf8)
        guard let filter = CIFilter(name: "CIQRCodeGenerator") else {
            return nil
        }
        filter.setValue(data, forKey: "inputMessage")
        filter.setValue("M", forKey: "inputCorrectionLevel")
        guard let output = filter.outputImage else {
            return nil
        }
        let transformed = output.transformed(by: CGAffineTransform(scaleX: 8, y: 8))
        let representation = NSCIImageRep(ciImage: transformed)
        let image = NSImage(size: representation.size)
        image.addRepresentation(representation)
        return image
    }
}

enum SidebarItem: Hashable {
    case devices
    case routing
    case settings
}

enum BadgeStyle {
    case ok
    case warn
    case bad
    case muted
    case selected

    var foreground: Color {
        switch self {
        case .ok:
            return .green
        case .warn:
            return .orange
        case .bad:
            return .red
        case .muted:
            return .secondary
        case .selected:
            return .white
        }
    }

    var background: Color {
        switch self {
        case .ok:
            return .green.opacity(0.14)
        case .warn:
            return .orange.opacity(0.14)
        case .bad:
            return .red.opacity(0.14)
        case .muted:
            return .secondary.opacity(0.12)
        case .selected:
            return .white.opacity(0.18)
        }
    }
}

private func formatBytes(_ bytes: UInt64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"]
    var value = Double(bytes)
    var unitIndex = 0
    while value >= 1024, unitIndex < units.count - 1 {
        value /= 1024
        unitIndex += 1
    }
    if unitIndex == 0 {
        return "\(bytes) B"
    }
    return String(format: "%.1f %@", value, units[unitIndex])
}

private func short(_ value: String, prefix: Int, suffix: Int) -> String {
    guard value.count > prefix + suffix + 3 else {
        return value
    }
    return "\(value.prefix(prefix))...\(value.suffix(suffix))"
}

private func cleanIp(_ value: String) -> String {
    value.split(separator: "/").first.map(String.init) ?? value
}

private func firstNonEmpty(_ values: String..., fallback: String) -> String {
    values.first { !$0.isEmpty } ?? fallback
}
