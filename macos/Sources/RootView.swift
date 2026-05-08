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
    @State private var participantInput = ""
    @State private var participantAliasInput = ""
    @State private var networkNameInput = ""
    @State private var deviceSearch = ""
    @State private var exitNodeSearch = ""
    @State private var selectedDevicePubkeyHex: String?
    @State private var networkNameDrafts: [String: String] = [:]
    @State private var participantAliasDrafts: [String: String] = [:]
    @State private var manageDevicesExpanded = false
    @State private var savedNetworksExpanded = false
    @State private var advancedSettingsExpanded = false
    @State private var diagnosticsExpanded = false
    @State private var showingQrScanner = false
    @State private var selectedSidebarItem: SidebarItem? = .devices
    @State private var lastSyncedRev: UInt64 = 0

    private var state: NativeAppState {
        manager.state
    }

    private var activeNetwork: NativeNetworkState? {
        manager.activeNetwork
    }

    var body: some View {
        VStack(spacing: 0) {
            headerBar
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

    private var headerIdentity: some View {
        Text(activeNetwork.map(displayName) ?? "Nostr VPN")
            .font(.caption.weight(.semibold))
            .lineLimit(1)
            .truncationMode(.tail)
            .frame(width: 180, alignment: .leading)
    }

    private var headerVpnControl: some View {
        HStack(spacing: 10) {
            Text(headerVpnStatusText)
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(width: 96, alignment: .trailing)
            headerVpnSwitch
        }
        .help(manager.vpnSwitchEnabled ? "Turn VPN off" : "Turn VPN on")
    }

    private var headerVpnSwitch: some View {
        let disabled = manager.actionInFlight || !state.vpnControlSupported
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
            if let activeNetwork {
                devicesPane(activeNetwork)
            } else {
                emptyDevicesPane
            }
        case .sharing:
            pageScroll {
                pageTitle("Share", "qrcode")
                if let activeNetwork {
                    inviteSection(activeNetwork)
                    lanPairingSection
                }
            }
        case .routing:
            pageScroll {
                pageTitle("Exit Nodes", "arrow.triangle.branch")
                if let activeNetwork {
                    routingSection(activeNetwork)
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

    private var emptyDevicesPane: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Devices")
                .font(.system(size: 24, weight: .semibold))
            emptyRow("No network selected", systemImage: "circle.dotted")
        }
        .padding(28)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private func deviceListColumn(_ network: NativeNetworkState) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 3) {
                        Text("Devices")
                            .font(.system(size: 24, weight: .semibold))
                        Text(peerAvailabilityText)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Spacer()
                    Button {
                        selectedSidebarItem = .sharing
                    } label: {
                        Image(systemName: "plus")
                    }
                    .disabled(!network.localIsAdmin && network.inviteInviterNpub.isEmpty)
                    .help("Add device")
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

                    if network.localIsAdmin {
                        manageDevicesSection(network)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 24)
            }
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
                    if participant.isAdmin {
                        badge("Admin", style: selected ? .selected : .muted)
                    }
                    if participant.offersExitNode {
                        badge("Exit", style: selected ? .selected : .warn)
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
                Text(deviceName(participant))
                    .font(.system(size: 24, weight: .semibold))
                    .lineLimit(2)
                Spacer()
                HStack(spacing: 7) {
                    connectivityDot(participant, size: 8)
                    Text(deviceStatusText(participant))
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            }
            if network.localIsAdmin && !isSelf(participant) {
                deviceActionButtons(participant, network: network)
            }
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
                manager.removeParticipant(networkId: network.id, npub: participant.npub)
            } label: {
                Label("Remove", systemImage: "trash")
            }
            .disabled(isSelf(participant) || manager.actionInFlight)
            .help("Remove device")
        }
        .controlSize(.small)
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

    private func manageDevicesSection(_ network: NativeNetworkState) -> some View {
        disclosureSection(
            title: "Manage Devices",
            systemImage: "slider.horizontal.3",
            isExpanded: $manageDevicesExpanded,
            font: .subheadline.weight(.medium)
        ) {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    TextField("Device ID", text: $participantInput)
                        .onSubmit(addParticipantToActiveNetwork)
                    TextField("Name", text: $participantAliasInput)
                        .frame(maxWidth: 160)
                        .onSubmit(addParticipantToActiveNetwork)
                    Button {
                        addParticipantToActiveNetwork()
                    } label: {
                        Image(systemName: "plus")
                    }
                    .disabled(participantInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || manager.actionInFlight)
                }

                ForEach(network.participants, id: \.pubkeyHex) { participant in
                    HStack(spacing: 8) {
                        Text(deviceName(participant))
                            .frame(width: 150, alignment: .leading)
                            .lineLimit(1)
                        TextField("Name", text: participantAliasBinding(participant))
                        Button {
                            manager.setParticipantAlias(
                                npub: participant.npub,
                                alias: participantAliasDrafts[participant.pubkeyHex] ?? participant.magicDnsAlias
                            )
                        } label: {
                            Image(systemName: "checkmark")
                        }
                        .disabled(manager.actionInFlight)
                        Button {
                            manager.toggleAdmin(networkId: network.id, participant: participant)
                        } label: {
                            Image(systemName: participant.isAdmin ? "star.fill" : "star")
                        }
                        .disabled(manager.actionInFlight)
                        Button(role: .destructive) {
                            manager.removeParticipant(networkId: network.id, npub: participant.npub)
                        } label: {
                            Image(systemName: "trash")
                        }
                        .disabled(isSelf(participant) || manager.actionInFlight)
                    }
                }
            }
            .padding(.top, 8)
        }
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
        surface {
            HStack(alignment: .top, spacing: 18) {
                InviteQRCodeView(invite: state.activeNetworkInvite)
                    .frame(width: 150, height: 150)
                VStack(alignment: .leading, spacing: 12) {
                    sectionHeader("Invite Devices", systemImage: "qrcode")
                    HStack(spacing: 8) {
                        Text(state.activeNetworkInvite.isEmpty ? "No invite" : state.activeNetworkInvite)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .textSelection(.enabled)
                            .padding(.horizontal, 10)
                            .frame(height: 32)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 6))
                        copyButton(value: state.activeNetworkInvite, copied: .invite, systemImage: "doc.on.doc")
                            .disabled(state.activeNetworkInvite.isEmpty)
                        Button {
                            manager.share(state.activeNetworkInvite)
                        } label: {
                            Image(systemName: "square.and.arrow.up")
                        }
                        .disabled(state.activeNetworkInvite.isEmpty)
                    }
                    HStack(spacing: 8) {
                        TextField("Paste invite", text: $manager.inviteInput)
                            .onSubmit {
                                manager.importInvite(manager.inviteInput)
                            }
                        Button {
                            manager.importInvite(manager.inviteInput)
                        } label: {
                            Image(systemName: "arrow.down")
                        }
                        Button {
                            showingQrScanner = true
                        } label: {
                            Image(systemName: "camera.viewfinder")
                        }
                        Button {
                            manager.chooseInviteQrImage()
                        } label: {
                            Image(systemName: "qrcode.viewfinder")
                        }
                    }
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
            }
        }
    }

    private var lanPairingSection: some View {
        surface {
            HStack {
                sectionHeader("Nearby Devices", systemImage: "dot.radiowaves.left.and.right")
                Spacer()
                Button {
                    state.lanPairingActive ? manager.stopLanPairing() : manager.startLanPairing()
                } label: {
                    Label(
                        state.lanPairingActive ? formatSeconds(state.lanPairingRemainingSecs) : "Pair Nearby",
                        systemImage: state.lanPairingActive ? "stop.circle" : "plus.circle"
                    )
                }
                .disabled(manager.actionInFlight)
            }

            if state.lanPeers.isEmpty {
                emptyRow("No nearby invites", systemImage: "wifi")
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
                        selected: state.exitNode.isEmpty,
                        enabled: true
                    ) {
                        manager.setExitNode("")
                    }

                    ForEach(exitNodeCandidates(network), id: \.pubkeyHex) { participant in
                        routeChoice(
                            title: deviceName(participant),
                            subtitle: participant.offersExitNode ? participant.statusText : "Exit not offered",
                            selected: state.exitNode == participant.npub,
                            enabled: participant.offersExitNode
                        ) {
                            manager.setExitNode(participant.npub)
                        }
                    }
                }

                Divider()

                Toggle("Offer this device as an exit node", isOn: Binding(
                    get: { state.advertiseExitNode },
                    set: { manager.setAdvertiseExitNode($0) }
                ))
                .disabled(manager.actionInFlight)
            }
        }
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

            disclosureSection(
                title: "Advanced",
                systemImage: "slider.horizontal.3",
                isExpanded: $advancedSettingsExpanded
            ) {
                VStack(alignment: .leading, spacing: 14) {
                    diagnosticsSection
                }
                .padding(.top, 8)
            }
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

    private var networkSettings: some View {
        surface {
            HStack {
                sectionHeader("Networks", systemImage: "rectangle.stack")
                Spacer()
                TextField("New network", text: $networkNameInput)
                    .frame(width: 180)
                    .onSubmit(addNetwork)
                Button {
                    addNetwork()
                } label: {
                    Image(systemName: "plus")
                }
                .disabled(networkNameInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || manager.actionInFlight)
            }

            if let network = activeNetwork {
                Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                    GridRow {
                        label("Active")
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
                        label("Join")
                        Toggle("", isOn: Binding(
                            get: { network.joinRequestsEnabled },
                            set: { manager.setJoinRequests(networkId: network.id, enabled: $0) }
                        ))
                        .labelsHidden()
                        .disabled(!network.localIsAdmin || manager.actionInFlight)
                        Text(network.joinRequestsEnabled ? "Open" : "Closed")
                            .foregroundStyle(.secondary)
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
                manager.setNetworkEnabled(networkId: network.id, enabled: true)
            }
            Button(role: .destructive) {
                manager.removeNetwork(network.id)
            } label: {
                Image(systemName: "trash")
            }
            .disabled(manager.actionInFlight)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 8))
    }

    private var systemSettings: some View {
        surface {
            HStack {
                sectionHeader("System", systemImage: "gearshape.2")
                Spacer()
                if manager.serviceSettling || manager.updateChecking || manager.updateInstalling {
                    ProgressView()
                        .controlSize(.small)
                }
            }

            HStack(spacing: 8) {
                badge(state.serviceInstalled ? "Service installed" : "Service missing", style: state.serviceInstalled ? .ok : .warn)
                badge(state.serviceRunning ? "Running" : "Stopped", style: state.serviceRunning ? .ok : .muted)
                if manager.serviceRepairRecommended {
                    badge("Repair available", style: .warn)
                }
                badge(state.cliInstalled ? "CLI installed" : "CLI missing", style: state.cliInstalled ? .ok : .muted)
                badge(manager.updateAvailable ? "Update \(manager.updateVersion)" : "Current", style: manager.updateAvailable ? .warn : .ok)
            }

            if manager.serviceRepairRecommended || !state.serviceStatusDetail.isEmpty || !manager.updateStatus.isEmpty {
                Text(firstNonEmpty(manager.updateStatus, state.serviceStatusDetail, fallback: ""))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }

            HStack {
                Button {
                    manager.installService()
                } label: {
                    Label(serviceInstallButtonTitle, systemImage: manager.serviceRepairRecommended ? "wrench.and.screwdriver" : "arrow.down.to.line")
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

    private func participantAliasBinding(_ participant: NativeParticipantState) -> Binding<String> {
        Binding(
            get: { participantAliasDrafts[participant.pubkeyHex] ?? participant.magicDnsAlias },
            set: { participantAliasDrafts[participant.pubkeyHex] = $0 }
        )
    }

    private func addParticipantToActiveNetwork() {
        guard let network = activeNetwork else {
            return
        }
        manager.addParticipant(networkId: network.id, npub: participantInput, alias: participantAliasInput)
        participantInput = ""
        participantAliasInput = ""
    }

    private func addNetwork() {
        manager.addNetwork(networkNameInput)
        networkNameInput = ""
    }

    private func syncDrafts() {
        guard lastSyncedRev != state.rev else {
            return
        }
        lastSyncedRev = state.rev
        nodeName = state.nodeName
        endpoint = state.endpoint
        tunnelIp = state.tunnelIp
        listenPort = String(state.listenPort)
        magicDnsSuffix = state.magicDnsSuffix

        for network in state.networks {
            networkNameDrafts[network.id] = network.name
            for participant in network.participants {
                participantAliasDrafts[participant.pubkeyHex] = participant.magicDnsAlias
            }
        }

        if let network = activeNetwork {
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

    private var headerVpnStatusText: String {
        if manager.actionInFlight, !manager.actionStatus.isEmpty {
            return manager.actionStatus
        }
        if state.vpnActive {
            return state.vpnStatus.isEmpty ? "VPN on" : state.vpnStatus
        }
        if state.vpnEnabled {
            return state.vpnStatus.isEmpty ? "Turning on" : state.vpnStatus
        }
        if manager.serviceRepairRecommended {
            return "Service needs repair"
        }
        return "Off"
    }

    private var peerAvailabilityText: String {
        if state.expectedPeerCount == 0 {
            return "No devices"
        }
        let deviceWord = state.expectedPeerCount == 1 ? "device" : "devices"
        return "\(state.connectedPeerCount) online · \(state.expectedPeerCount) \(deviceWord)"
    }

    private var serviceInstallButtonTitle: String {
        if manager.serviceRepairRecommended {
            return "Repair Service"
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
        if isSelf(participant), !state.nodeName.isEmpty {
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

    private func deviceSubtitle(_ participant: NativeParticipantState) -> String {
        if isSelf(participant) {
            return "This device"
        }
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
        if participant.state == "off" {
            return "Off"
        }
        if isSelf(participant) {
            return "Self"
        }
        switch participant.state {
        case "local", "online":
            return "Online"
        case "pending":
            return "Connecting"
        case "offline":
            return "Offline"
        default:
            return "Unknown"
        }
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
    case sharing
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

private func formatSeconds(_ seconds: UInt64) -> String {
    "\(seconds / 60):\(String(format: "%02d", seconds % 60))"
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
