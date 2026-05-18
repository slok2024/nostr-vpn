import Foundation

enum ScreenshotFixtures {
    static func state() -> AppState {
        let ownNpub = fakeNpub("q")
        let macNpub = fakeNpub("p")
        let windowsNpub = fakeNpub("z")
        let linuxNpub = fakeNpub("r")
        let androidNpub = fakeNpub("t")
        let ipadNpub = fakeNpub("v")

        var state = AppState()
        state.rev = 1
        state.appVersion = appVersion
        state.platform = "iOS Simulator"
        state.mobile = true
        state.vpnControlSupported = true
        state.runtimeStatusDetail = "Fixture mode"
        state.vpnEnabled = true
        state.vpnActive = true
        state.vpnStatus = "Connected"
        state.daemonRunning = true
        state.ownNpub = ownNpub
        state.nodeName = "iPhone"
        state.selfMagicDnsName = ""
        state.tunnelIp = "10.44.0.2/32"
        state.endpoint = "local network"
        state.listenPort = 51820
        state.activeNetworkInvite = "nvpn://invite/demo-home-mesh"
        state.connectedPeerCount = 3
        state.expectedPeerCount = 4
        state.meshReady = true
        state.exitNode = macNpub
        state.exitNodeLeakProtection = true
        state.exitNodeActive = true
        state.exitNodeStatusText = "Routing through laptop"
        state.advertiseExitNode = false
        state.advertisedRoutes = ["10.44.0.0/24"]
        state.wireguardExitEnabled = false
        state.wireguardExitConfigured = true
        state.wireguardExitInterface = "utun-demo"
        state.wireguardExitAddress = "10.64.12.4/32"
        state.wireguardExitEndpoint = "demo-wireguard.invalid:51820"
        state.wireguardExitAllowedIps = "0.0.0.0/0, ::/0"
        state.wireguardExitDns = "100.64.0.1"
        state.wireguardExitMtu = 1280
        state.wireguardExitPersistentKeepaliveSecs = 25
        state.wireguardExitConfig = """
        [Interface]
        PrivateKey = demo-private-key
        Address = 10.64.12.4/32
        DNS = 100.64.0.1

        [Peer]
        PublicKey = demo-peer-key
        Endpoint = demo-wireguard.invalid:51820
        AllowedIPs = 0.0.0.0/0, ::/0
        """
        state.magicDnsSuffix = "home.mesh"
        state.magicDnsStatus = "Ready"
        state.autoconnect = true
        state.inviteBroadcastActive = false
        state.nearbyDiscoveryActive = true
        state.nearbyDiscoveryRemainingSecs = 112
        state.configPath = "Fixture data"
        state.networks = [
            network(
                ownNpub: ownNpub,
                macNpub: macNpub,
                windowsNpub: windowsNpub,
                linuxNpub: linuxNpub,
                androidNpub: androidNpub,
                ipadNpub: ipadNpub
            )
        ]
        state.lanPeers = [
            lanPeer(name: "iPadOS nearby", network: "Home Mesh", invite: "nvpn://invite/demo-ipad")
        ]
        return state
    }

    private static var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
    }

    static func dispatch(_ action: [String: Any], state original: AppState) -> AppState {
        var state = original
        state.error = ""
        state.rev += 1
        guard let type = action["type"] as? String else {
            return state
        }

        switch type {
        case "connect_vpn":
            state.vpnEnabled = true
            state.vpnActive = true
            state.vpnStatus = "Connected"
            state.connectedPeerCount = min(state.expectedPeerCount, 3)
        case "disconnect_vpn":
            state.vpnEnabled = false
            state.vpnActive = false
            state.vpnStatus = "Disconnected"
            state.connectedPeerCount = 0
        case "set_network_enabled":
            if let networkId = action["networkId"] as? String,
               let enabled = action["enabled"] as? Bool,
               let index = state.networks.firstIndex(where: { $0.id == networkId }) {
                state.networks[index].enabled = enabled
            }
        case "update_settings":
            if let patch = action["patch"] as? [String: Any] {
                applySettings(patch, to: &state)
            }
        default:
            break
        }
        return state
    }

    static func qrMatrix() -> QrMatrix {
        let width = 25
        var cells = Array(repeating: false, count: width * width)
        for y in 0..<width {
            for x in 0..<width {
                let finder = (x < 7 && y < 7)
                    || (x >= width - 7 && y < 7)
                    || (x < 7 && y >= width - 7)
                cells[y * width + x] = finder || ((x * 3 + y * 5) % 7 == 0)
            }
        }
        var matrix = QrMatrix()
        matrix.width = width
        matrix.cells = cells
        return matrix
    }

    private static func network(
        ownNpub: String,
        macNpub: String,
        windowsNpub: String,
        linuxNpub: String,
        androidNpub: String,
        ipadNpub: String
    ) -> NetworkState {
        var network = NetworkState()
        network.id = "fixture-home"
        network.name = "Home Mesh"
        network.enabled = true
        network.networkId = "mesh-demo-home"
        network.localIsAdmin = true
        network.joinRequestsEnabled = true
        network.onlineCount = 3
        network.expectedCount = 4
        network.inboundJoinRequests = []
        network.participants = [
            participant(
                npub: ownNpub,
                alias: "iPhone",
                magicDns: "",
                ip: "10.44.0.2/32",
                isAdmin: true,
                reachable: true,
                state: "local",
                meshState: "local",
                status: "This device"
            ),
            participant(
                npub: macNpub,
                alias: "Laptop",
                magicDns: "",
                ip: "10.44.0.3/32",
                isAdmin: true,
                reachable: true,
                offersExitNode: true,
                state: "online",
                transport: "LAN direct",
                status: "Online via LAN"
            ),
            participant(
                npub: windowsNpub,
                alias: "Windows PC",
                magicDns: "",
                ip: "10.44.0.4/32",
                reachable: true,
                state: "online",
                status: "Online through relay path"
            ),
            participant(
                npub: linuxNpub,
                alias: "Linux server",
                magicDns: "",
                ip: "10.44.0.5/32",
                reachable: true,
                offersExitNode: true,
                state: "online",
                status: "Online via mesh"
            ),
            participant(
                npub: androidNpub,
                alias: "Android phone",
                magicDns: "",
                ip: "10.44.0.6/32",
                reachable: false,
                state: "offline",
                status: "Last seen 18 min ago"
            ),
            participant(
                npub: ipadNpub,
                alias: "iPad",
                magicDns: "",
                ip: "10.44.0.7/32",
                reachable: false,
                state: "offline",
                status: "Last seen yesterday"
            ),
        ]
        return network
    }

    private static func participant(
        npub: String,
        alias: String,
        magicDns: String,
        ip: String,
        isAdmin: Bool = false,
        reachable: Bool,
        offersExitNode: Bool = false,
        state: String,
        meshState: String = "",
        transport: String = "",
        status: String
    ) -> ParticipantState {
        var participant = ParticipantState()
        participant.npub = npub
        participant.pubkeyHex = fakeHex(String(npub.dropFirst(5).first ?? "0"))
        participant.alias = alias
        participant.magicDnsAlias = alias.lowercased()
        participant.magicDnsName = magicDns
        participant.tunnelIp = ip
        participant.isAdmin = isAdmin
        participant.reachable = reachable
        participant.offersExitNode = offersExitNode
        participant.fipsEndpointNpub = fakeNpub("s")
        participant.fipsTransportAddr = transport
        participant.fipsTransportType = transport.isEmpty ? "" : "udp"
        participant.fipsSrttMs = reachable ? 18 : 0
        participant.fipsPacketsSent = reachable ? 1482 : 0
        participant.fipsPacketsRecv = reachable ? 1516 : 0
        participant.fipsBytesSent = reachable ? 481_280 : 0
        participant.fipsBytesRecv = reachable ? 722_944 : 0
        participant.state = state
        participant.meshState = meshState
        participant.statusText = status
        participant.lastSeenText = reachable ? "now" : "yesterday"
        return participant
    }

    private static func lanPeer(name: String, network: String, invite: String) -> LanPeerState {
        var peer = LanPeerState()
        peer.npub = fakeNpub("y")
        peer.nodeName = name
        peer.networkName = network
        peer.invite = invite
        peer.lastSeenText = "Nearby now"
        return peer
    }

    private static func applySettings(_ patch: [String: Any], to state: inout AppState) {
        if let exitNode = patch["exitNode"] as? String {
            state.exitNode = exitNode
            state.exitNodeActive = !exitNode.isEmpty
        }
        if let enabled = patch["wireguardExitEnabled"] as? Bool {
            state.wireguardExitEnabled = enabled
            if enabled {
                state.exitNode = ""
                state.exitNodeActive = false
            }
        }
        if let leakProtection = patch["exitNodeLeakProtection"] as? Bool {
            state.exitNodeLeakProtection = leakProtection
        }
        if let advertise = patch["advertiseExitNode"] as? Bool {
            state.advertiseExitNode = advertise
        }
        if let autoconnect = patch["autoconnect"] as? Bool {
            state.autoconnect = autoconnect
        }
        if let nodeName = patch["nodeName"] as? String {
            state.nodeName = nodeName
        }
        if let tunnelIp = patch["tunnelIp"] as? String {
            state.tunnelIp = tunnelIp
        }
        if let endpoint = patch["endpoint"] as? String {
            state.endpoint = endpoint
        }
        if let listenPort = patch["listenPort"] as? Int {
            state.listenPort = listenPort
        }
        if let config = patch["wireguardExitConfig"] as? String {
            state.wireguardExitConfig = config
            state.wireguardExitConfigured = !config.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    private static func fakeNpub(_ character: Character) -> String {
        "npub1" + String(repeating: String(character), count: 58)
    }

    private static func fakeHex(_ character: String) -> String {
        let scalar = character.unicodeScalars.first?.value ?? 0
        let byte = String(format: "%02x", scalar % 256)
        return String(repeating: byte, count: 32)
    }
}
