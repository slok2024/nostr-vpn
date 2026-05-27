import Foundation

struct AppState: Decodable {
    var rev: UInt64 = 0
    var error = ""
    var appVersion = ""
    var platform = ""
    var mobile = true
    var vpnControlSupported = false
    var runtimeStatusDetail = ""
    var vpnEnabled = false
    var vpnActive = false
    var vpnStatus = "Disconnected"
    var daemonRunning = false
    var ownNpub = ""
    var nodeName = ""
    var selfMagicDnsName = ""
    var tunnelIp = ""
    var endpoint = ""
    var listenPort: Int = 0
    var relays: [RelayState] = []
    var activeNetworkInvite = ""
    var connectedPeerCount: UInt64 = 0
    var expectedPeerCount: UInt64 = 0
    var fipsConnectedPeerCount: UInt64 = 0
    var fipsRosterPeerCount: UInt64 = 0
    var nonFipsRosterPeerCount: UInt64 = 0
    var meshReady = false
    var exitNode = ""
    var exitNodeLeakProtection = false
    var exitNodeActive = false
    var exitNodeBlocked = false
    var exitNodeStatusText = ""
    var advertiseExitNode = false
    var advertisedRoutes: [String] = []
    var wireguardExitEnabled = false
    var wireguardExitConfigured = false
    var wireguardExitInterface = ""
    var wireguardExitAddress = ""
    var wireguardExitPrivateKey = ""
    var wireguardExitPeerPublicKey = ""
    var wireguardExitPeerPresharedKey = ""
    var wireguardExitEndpoint = ""
    var wireguardExitAllowedIps = ""
    var wireguardExitDns = ""
    var wireguardExitMtu: Int = 0
    var wireguardExitPersistentKeepaliveSecs: Int = 0
    var wireguardExitConfig = ""
    var connectToNonRosterFipsPeers = true
    var fipsNostrDiscoveryEnabled = true
    var fipsBootstrapEnabled = true
    var magicDnsSuffix = ""
    var magicDnsStatus = ""
    var autoconnect = false
    var inviteBroadcastActive = false
    var inviteBroadcastRemainingSecs: UInt64 = 0
    var nearbyDiscoveryActive = false
    var nearbyDiscoveryRemainingSecs: UInt64 = 0
    var configPath = ""
    var networks: [NetworkState] = []
    var lanPeers: [LanPeerState] = []
    var health: [HealthIssue] = []

    var activeNetwork: NetworkState? {
        networks.first(where: { $0.enabled })
    }

    enum CodingKeys: String, CodingKey {
        case rev, error, appVersion, platform, mobile, vpnControlSupported
        case runtimeStatusDetail, vpnEnabled, vpnActive, vpnStatus, daemonRunning
        case ownNpub, nodeName, selfMagicDnsName, tunnelIp, endpoint, listenPort, relays, activeNetworkInvite
        case connectedPeerCount, expectedPeerCount
        case fipsConnectedPeerCount, fipsRosterPeerCount, nonFipsRosterPeerCount
        case meshReady, exitNode, exitNodeLeakProtection
        case exitNodeActive, exitNodeBlocked, exitNodeStatusText, advertiseExitNode
        case advertisedRoutes
        case wireguardExitEnabled, wireguardExitConfigured, wireguardExitInterface, wireguardExitAddress
        case wireguardExitPrivateKey, wireguardExitPeerPublicKey, wireguardExitPeerPresharedKey
        case wireguardExitEndpoint, wireguardExitAllowedIps, wireguardExitDns
        case wireguardExitMtu, wireguardExitPersistentKeepaliveSecs, wireguardExitConfig
        case connectToNonRosterFipsPeers
        case fipsNostrDiscoveryEnabled, fipsBootstrapEnabled
        case magicDnsSuffix, magicDnsStatus, autoconnect
        case inviteBroadcastActive, inviteBroadcastRemainingSecs
        case nearbyDiscoveryActive, nearbyDiscoveryRemainingSecs, configPath
        case networks, lanPeers, health
    }

    init() {}

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        rev = container.uint64(.rev)
        error = container.string(.error)
        appVersion = container.string(.appVersion)
        platform = container.string(.platform)
        mobile = container.bool(.mobile, default: true)
        vpnControlSupported = container.bool(.vpnControlSupported)
        runtimeStatusDetail = container.string(.runtimeStatusDetail)
        vpnEnabled = container.bool(.vpnEnabled)
        vpnActive = container.bool(.vpnActive)
        vpnStatus = container.string(.vpnStatus, default: "Disconnected")
        daemonRunning = container.bool(.daemonRunning)
        ownNpub = container.string(.ownNpub)
        nodeName = container.string(.nodeName)
        selfMagicDnsName = container.string(.selfMagicDnsName)
        tunnelIp = container.string(.tunnelIp)
        endpoint = container.string(.endpoint)
        listenPort = container.int(.listenPort)
        relays = container.array(.relays)
        activeNetworkInvite = container.string(.activeNetworkInvite)
        connectedPeerCount = container.uint64(.connectedPeerCount)
        expectedPeerCount = container.uint64(.expectedPeerCount)
        fipsConnectedPeerCount = container.uint64(.fipsConnectedPeerCount)
        fipsRosterPeerCount = container.uint64(.fipsRosterPeerCount)
        nonFipsRosterPeerCount = container.uint64(.nonFipsRosterPeerCount)
        meshReady = container.bool(.meshReady)
        exitNode = container.string(.exitNode)
        exitNodeLeakProtection = container.bool(.exitNodeLeakProtection)
        exitNodeActive = container.bool(.exitNodeActive)
        exitNodeBlocked = container.bool(.exitNodeBlocked)
        exitNodeStatusText = container.string(.exitNodeStatusText)
        advertiseExitNode = container.bool(.advertiseExitNode)
        advertisedRoutes = container.array(.advertisedRoutes)
        wireguardExitEnabled = container.bool(.wireguardExitEnabled)
        wireguardExitConfigured = container.bool(.wireguardExitConfigured)
        wireguardExitInterface = container.string(.wireguardExitInterface)
        wireguardExitAddress = container.string(.wireguardExitAddress)
        wireguardExitPrivateKey = container.string(.wireguardExitPrivateKey)
        wireguardExitPeerPublicKey = container.string(.wireguardExitPeerPublicKey)
        wireguardExitPeerPresharedKey = container.string(.wireguardExitPeerPresharedKey)
        wireguardExitEndpoint = container.string(.wireguardExitEndpoint)
        wireguardExitAllowedIps = container.string(.wireguardExitAllowedIps)
        wireguardExitDns = container.string(.wireguardExitDns)
        wireguardExitMtu = container.int(.wireguardExitMtu)
        wireguardExitPersistentKeepaliveSecs = container.int(.wireguardExitPersistentKeepaliveSecs)
        wireguardExitConfig = container.string(.wireguardExitConfig)
        connectToNonRosterFipsPeers = container.bool(.connectToNonRosterFipsPeers, default: true)
        fipsNostrDiscoveryEnabled = container.bool(.fipsNostrDiscoveryEnabled, default: true)
        fipsBootstrapEnabled = container.bool(.fipsBootstrapEnabled, default: true)
        magicDnsSuffix = container.string(.magicDnsSuffix)
        magicDnsStatus = container.string(.magicDnsStatus)
        autoconnect = container.bool(.autoconnect)
        inviteBroadcastActive = container.bool(.inviteBroadcastActive)
        inviteBroadcastRemainingSecs = container.uint64(.inviteBroadcastRemainingSecs)
        nearbyDiscoveryActive = container.bool(.nearbyDiscoveryActive)
        nearbyDiscoveryRemainingSecs = container.uint64(.nearbyDiscoveryRemainingSecs)
        configPath = container.string(.configPath)
        networks = container.array(.networks)
        lanPeers = container.array(.lanPeers)
        health = container.array(.health)
    }
}

struct RelayState: Decodable, Identifiable {
    var url = ""
    var status = "unknown"
    var enabled = true
    var id: String { url }
    var connected: Bool { enabled && status == "connected" }
}

struct NetworkState: Decodable, Identifiable {
    var id = ""
    var name = ""
    var enabled = false
    var networkId = ""
    var localIsAdmin = false
    var joinRequestsEnabled = false
    var inviteInviterNpub = ""
    var outboundJoinRequest: OutboundJoinRequest?
    var inboundJoinRequests: [InboundJoinRequest] = []
    var onlineCount: UInt64 = 0
    var expectedCount: UInt64 = 0
    var participants: [ParticipantState] = []

    var displayName: String {
        name.isEmpty ? "Private network" : name
    }

    enum CodingKeys: String, CodingKey {
        case id, name, enabled, networkId, localIsAdmin, joinRequestsEnabled
        case inviteInviterNpub, outboundJoinRequest, inboundJoinRequests
        case onlineCount, expectedCount, participants
    }

    init() {}

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = container.string(.id)
        name = container.string(.name)
        enabled = container.bool(.enabled)
        networkId = container.string(.networkId)
        localIsAdmin = container.bool(.localIsAdmin)
        joinRequestsEnabled = container.bool(.joinRequestsEnabled)
        inviteInviterNpub = container.string(.inviteInviterNpub)
        outboundJoinRequest = try? container.decodeIfPresent(OutboundJoinRequest.self, forKey: .outboundJoinRequest)
        inboundJoinRequests = container.array(.inboundJoinRequests)
        onlineCount = container.uint64(.onlineCount)
        expectedCount = container.uint64(.expectedCount)
        participants = container.array(.participants)
    }
}

struct ParticipantState: Decodable, Identifiable {
    var id: String { pubkeyHex.isEmpty ? npub : pubkeyHex }
    var npub = ""
    var pubkeyHex = ""
    var alias = ""
    var magicDnsAlias = ""
    var magicDnsName = ""
    var tunnelIp = ""
    var isAdmin = false
    var reachable = false
    var offersExitNode = false
    var fipsEndpointNpub = ""
    var fipsEndpointHints: [String] = []
    var fipsTransportAddr = ""
    var fipsTransportType = ""
    var fipsSrttMs: UInt64 = 0
    var fipsPacketsSent: UInt64 = 0
    var fipsPacketsRecv: UInt64 = 0
    var fipsBytesSent: UInt64 = 0
    var fipsBytesRecv: UInt64 = 0
    var state = ""
    var meshState = ""
    var statusText = ""
    var lastSeenText = ""

    var displayName: String {
        if !magicDnsName.isEmpty { return magicDnsName }
        if !alias.isEmpty { return alias }
        return "Device"
    }

    enum CodingKeys: String, CodingKey {
        case npub, pubkeyHex, alias, magicDnsAlias, magicDnsName, tunnelIp
        case isAdmin, reachable, offersExitNode
        case fipsEndpointNpub, fipsEndpointHints, fipsTransportAddr, fipsTransportType, fipsSrttMs
        case fipsPacketsSent, fipsPacketsRecv, fipsBytesSent, fipsBytesRecv
        case state, meshState, statusText, lastSeenText, lastSignalText
    }

    init() {}

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        npub = container.string(.npub)
        pubkeyHex = container.string(.pubkeyHex)
        alias = container.string(.alias)
        magicDnsAlias = container.string(.magicDnsAlias)
        magicDnsName = container.string(.magicDnsName)
        tunnelIp = container.string(.tunnelIp)
        isAdmin = container.bool(.isAdmin)
        reachable = container.bool(.reachable)
        offersExitNode = container.bool(.offersExitNode)
        fipsEndpointNpub = container.string(.fipsEndpointNpub)
        fipsEndpointHints = container.array(.fipsEndpointHints)
        fipsTransportAddr = container.string(.fipsTransportAddr)
        fipsTransportType = container.string(.fipsTransportType)
        fipsSrttMs = container.uint64(.fipsSrttMs)
        fipsPacketsSent = container.uint64(.fipsPacketsSent)
        fipsPacketsRecv = container.uint64(.fipsPacketsRecv)
        fipsBytesSent = container.uint64(.fipsBytesSent)
        fipsBytesRecv = container.uint64(.fipsBytesRecv)
        state = container.string(.state)
        meshState = container.string(.meshState)
        statusText = container.string(.statusText)
        lastSeenText = container.string(.lastSeenText, default: container.string(.lastSignalText))
    }
}

struct OutboundJoinRequest: Decodable {
    var recipientNpub = ""
    var requestedAtText = ""
}

struct InboundJoinRequest: Decodable, Identifiable {
    var id: String { requesterNpub }
    var requesterNpub = ""
    var requesterNodeName = ""
    var requestedAtText = ""
}

struct LanPeerState: Decodable, Identifiable {
    var id: String { invite.isEmpty ? npub : invite }
    var npub = ""
    var nodeName = ""
    var networkName = ""
    var invite = ""
    var lastSeenText = ""
}

struct HealthIssue: Decodable, Identifiable {
    var id: String { code + summary }
    var code = ""
    var severity = ""
    var summary = ""
    var detail = ""
}

struct QrMatrix: Decodable {
    var width = 0
    var cells: [Bool] = []
    var error = ""
}

struct QrDecodeResult: Decodable {
    var value = ""
    var error = ""
}

private extension KeyedDecodingContainer {
    func string(_ key: Key, default defaultValue: String = "") -> String {
        (try? decodeIfPresent(String.self, forKey: key)) ?? defaultValue
    }

    func bool(_ key: Key, default defaultValue: Bool = false) -> Bool {
        (try? decodeIfPresent(Bool.self, forKey: key)) ?? defaultValue
    }

    func int(_ key: Key, default defaultValue: Int = 0) -> Int {
        (try? decodeIfPresent(Int.self, forKey: key)) ?? defaultValue
    }

    func uint64(_ key: Key, default defaultValue: UInt64 = 0) -> UInt64 {
        (try? decodeIfPresent(UInt64.self, forKey: key)) ?? defaultValue
    }

    func array<T: Decodable>(_ key: Key) -> [T] {
        (try? decodeIfPresent([T].self, forKey: key)) ?? []
    }
}
