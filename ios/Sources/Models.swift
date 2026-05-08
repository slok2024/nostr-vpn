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
    var activeNetworkInvite = ""
    var connectedPeerCount: UInt64 = 0
    var expectedPeerCount: UInt64 = 0
    var meshReady = false
    var exitNode = ""
    var advertiseExitNode = false
    var advertisedRoutes: [String] = []
    var magicDnsSuffix = ""
    var magicDnsStatus = ""
    var autoconnect = false
    var lanPairingActive = false
    var lanPairingRemainingSecs: UInt64 = 0
    var configPath = ""
    var networks: [NetworkState] = []
    var lanPeers: [LanPeerState] = []
    var health: [HealthIssue] = []

    var activeNetwork: NetworkState? {
        networks.first(where: { $0.enabled }) ?? networks.first
    }

    enum CodingKeys: String, CodingKey {
        case rev, error, appVersion, platform, mobile, vpnControlSupported
        case runtimeStatusDetail, vpnEnabled, vpnActive, vpnStatus, daemonRunning
        case ownNpub, nodeName, selfMagicDnsName, tunnelIp, endpoint, listenPort, activeNetworkInvite
        case connectedPeerCount, expectedPeerCount, meshReady, exitNode, advertiseExitNode
        case advertisedRoutes, magicDnsSuffix, magicDnsStatus, autoconnect
        case lanPairingActive, lanPairingRemainingSecs, configPath
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
        activeNetworkInvite = container.string(.activeNetworkInvite)
        connectedPeerCount = container.uint64(.connectedPeerCount)
        expectedPeerCount = container.uint64(.expectedPeerCount)
        meshReady = container.bool(.meshReady)
        exitNode = container.string(.exitNode)
        advertiseExitNode = container.bool(.advertiseExitNode)
        advertisedRoutes = container.array(.advertisedRoutes)
        magicDnsSuffix = container.string(.magicDnsSuffix)
        magicDnsStatus = container.string(.magicDnsStatus)
        autoconnect = container.bool(.autoconnect)
        lanPairingActive = container.bool(.lanPairingActive)
        lanPairingRemainingSecs = container.uint64(.lanPairingRemainingSecs)
        configPath = container.string(.configPath)
        networks = container.array(.networks)
        lanPeers = container.array(.lanPeers)
        health = container.array(.health)
    }
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
    var fipsTransportAddr = ""
    var fipsTransportType = ""
    var fipsSrttMs: UInt64 = 0
    var fipsPacketsSent: UInt64 = 0
    var fipsPacketsRecv: UInt64 = 0
    var fipsBytesSent: UInt64 = 0
    var fipsBytesRecv: UInt64 = 0
    var state = ""
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
        case fipsEndpointNpub, fipsTransportAddr, fipsTransportType, fipsSrttMs
        case fipsPacketsSent, fipsPacketsRecv, fipsBytesSent, fipsBytesRecv
        case state, statusText, lastSeenText, lastSignalText
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
        fipsTransportAddr = container.string(.fipsTransportAddr)
        fipsTransportType = container.string(.fipsTransportType)
        fipsSrttMs = container.uint64(.fipsSrttMs)
        fipsPacketsSent = container.uint64(.fipsPacketsSent)
        fipsPacketsRecv = container.uint64(.fipsPacketsRecv)
        fipsBytesSent = container.uint64(.fipsBytesSent)
        fipsBytesRecv = container.uint64(.fipsBytesRecv)
        state = container.string(.state)
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
