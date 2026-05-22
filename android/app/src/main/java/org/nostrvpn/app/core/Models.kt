package org.nostrvpn.app.core

import org.json.JSONArray
import org.json.JSONObject

data class AppState(
    val rev: Long = 0,
    val error: String = "",
    val appVersion: String = "",
    val platform: String = "",
    val mobile: Boolean = true,
    val vpnControlSupported: Boolean = false,
    val runtimeStatusDetail: String = "",
    val vpnEnabled: Boolean = false,
    val vpnActive: Boolean = false,
    val vpnStatus: String = "Disconnected",
    val daemonRunning: Boolean = false,
    val ownNpub: String = "",
    val nodeName: String = "",
    val selfMagicDnsName: String = "",
    val tunnelIp: String = "",
    val endpoint: String = "",
    val listenPort: Int = 0,
    val relays: List<RelayState> = emptyList(),
    val activeNetworkInvite: String = "",
    val connectedPeerCount: Long = 0,
    val expectedPeerCount: Long = 0,
    val fipsConnectedPeerCount: Long = 0,
    val fipsRosterPeerCount: Long = 0,
    val nonFipsRosterPeerCount: Long = 0,
    val meshReady: Boolean = false,
    val exitNode: String = "",
    val exitNodeLeakProtection: Boolean = false,
    val exitNodeActive: Boolean = false,
    val exitNodeBlocked: Boolean = false,
    val exitNodeStatusText: String = "",
    val advertiseExitNode: Boolean = false,
    val advertisedRoutes: List<String> = emptyList(),
    val wireguardExitEnabled: Boolean = false,
    val wireguardExitConfigured: Boolean = false,
    val wireguardExitInterface: String = "",
    val wireguardExitAddress: String = "",
    val wireguardExitPrivateKey: String = "",
    val wireguardExitPeerPublicKey: String = "",
    val wireguardExitPeerPresharedKey: String = "",
    val wireguardExitEndpoint: String = "",
    val wireguardExitAllowedIps: String = "",
    val wireguardExitDns: String = "",
    val wireguardExitMtu: Int = 0,
    val wireguardExitPersistentKeepaliveSecs: Int = 0,
    val wireguardExitConfig: String = "",
    val magicDnsSuffix: String = "",
    val magicDnsStatus: String = "",
    val autoconnect: Boolean = false,
    val inviteBroadcastActive: Boolean = false,
    val inviteBroadcastRemainingSecs: Long = 0,
    val nearbyDiscoveryActive: Boolean = false,
    val nearbyDiscoveryRemainingSecs: Long = 0,
    val networks: List<NetworkState> = emptyList(),
    val lanPeers: List<LanPeerState> = emptyList(),
    val health: List<HealthIssue> = emptyList(),
)

data class RelayState(
    val url: String = "",
    val status: String = "unknown",
    val enabled: Boolean = true,
)

data class NetworkState(
    val id: String = "",
    val name: String = "",
    val enabled: Boolean = false,
    val networkId: String = "",
    val localIsAdmin: Boolean = false,
    val joinRequestsEnabled: Boolean = false,
    val inviteInviterNpub: String = "",
    val outboundJoinRequest: Boolean = false,
    val inboundJoinRequests: List<InboundJoinRequest> = emptyList(),
    val onlineCount: Long = 0,
    val expectedCount: Long = 0,
    val participants: List<ParticipantState> = emptyList(),
)

data class ParticipantState(
    val npub: String = "",
    val pubkeyHex: String = "",
    val alias: String = "",
    val magicDnsAlias: String = "",
    val magicDnsName: String = "",
    val tunnelIp: String = "",
    val isAdmin: Boolean = false,
    val reachable: Boolean = false,
    val offersExitNode: Boolean = false,
    val fipsEndpointNpub: String = "",
    val fipsEndpointHints: List<String> = emptyList(),
    val fipsTransportAddr: String = "",
    val fipsTransportType: String = "",
    val fipsSrttMs: Long = 0,
    val fipsPacketsSent: Long = 0,
    val fipsPacketsRecv: Long = 0,
    val fipsBytesSent: Long = 0,
    val fipsBytesRecv: Long = 0,
    val state: String = "",
    val meshState: String = "",
    val statusText: String = "",
    val lastSeenText: String = "",
)

data class InboundJoinRequest(
    val requesterNpub: String = "",
    val requesterNodeName: String = "",
    val requestedAtText: String = "",
)

data class LanPeerState(
    val nodeName: String = "",
    val networkName: String = "",
    val invite: String = "",
    val lastSeenText: String = "",
)

data class HealthIssue(
    val severity: String = "",
    val summary: String = "",
    val detail: String = "",
)

val AppState.activeNetwork: NetworkState?
    get() = networks.firstOrNull { it.enabled } ?: networks.firstOrNull()

fun parseAppState(jsonText: String): AppState {
    val json = JSONObject(jsonText.ifBlank { "{}" })
    return AppState(
        rev = json.optLong("rev"),
        error = json.optString("error"),
        appVersion = json.optString("appVersion"),
        platform = json.optString("platform"),
        mobile = json.optBoolean("mobile", true),
        vpnControlSupported = json.optBoolean("vpnControlSupported"),
        runtimeStatusDetail = json.optString("runtimeStatusDetail"),
        vpnEnabled = json.optBoolean("vpnEnabled"),
        vpnActive = json.optBoolean("vpnActive"),
        vpnStatus = json.optString("vpnStatus", "Disconnected"),
        daemonRunning = json.optBoolean("daemonRunning"),
        ownNpub = json.optString("ownNpub"),
        nodeName = json.optString("nodeName"),
        selfMagicDnsName = json.optString("selfMagicDnsName"),
        tunnelIp = json.optString("tunnelIp"),
        endpoint = json.optString("endpoint"),
        listenPort = json.optInt("listenPort"),
        relays = json.optJSONArray("relays").toRelayList(),
        activeNetworkInvite = json.optString("activeNetworkInvite"),
        connectedPeerCount = json.optLong("connectedPeerCount"),
        expectedPeerCount = json.optLong("expectedPeerCount"),
        fipsConnectedPeerCount = json.optLong("fipsConnectedPeerCount"),
        fipsRosterPeerCount = json.optLong("fipsRosterPeerCount"),
        nonFipsRosterPeerCount = json.optLong("nonFipsRosterPeerCount"),
        meshReady = json.optBoolean("meshReady"),
        exitNode = json.optString("exitNode"),
        exitNodeLeakProtection = json.optBoolean("exitNodeLeakProtection"),
        exitNodeActive = json.optBoolean("exitNodeActive"),
        exitNodeBlocked = json.optBoolean("exitNodeBlocked"),
        exitNodeStatusText = json.optString("exitNodeStatusText"),
        advertiseExitNode = json.optBoolean("advertiseExitNode"),
        advertisedRoutes = json.optJSONArray("advertisedRoutes").toStringList(),
        wireguardExitEnabled = json.optBoolean("wireguardExitEnabled"),
        wireguardExitConfigured = json.optBoolean("wireguardExitConfigured"),
        wireguardExitInterface = json.optString("wireguardExitInterface"),
        wireguardExitAddress = json.optString("wireguardExitAddress"),
        wireguardExitPrivateKey = json.optString("wireguardExitPrivateKey"),
        wireguardExitPeerPublicKey = json.optString("wireguardExitPeerPublicKey"),
        wireguardExitPeerPresharedKey = json.optString("wireguardExitPeerPresharedKey"),
        wireguardExitEndpoint = json.optString("wireguardExitEndpoint"),
        wireguardExitAllowedIps = json.optString("wireguardExitAllowedIps"),
        wireguardExitDns = json.optString("wireguardExitDns"),
        wireguardExitMtu = json.optInt("wireguardExitMtu"),
        wireguardExitPersistentKeepaliveSecs = json.optInt("wireguardExitPersistentKeepaliveSecs"),
        wireguardExitConfig = json.optString("wireguardExitConfig"),
        magicDnsSuffix = json.optString("magicDnsSuffix"),
        magicDnsStatus = json.optString("magicDnsStatus"),
        autoconnect = json.optBoolean("autoconnect"),
        inviteBroadcastActive = json.optBoolean("inviteBroadcastActive"),
        inviteBroadcastRemainingSecs = json.optLong("inviteBroadcastRemainingSecs"),
        nearbyDiscoveryActive = json.optBoolean("nearbyDiscoveryActive"),
        nearbyDiscoveryRemainingSecs = json.optLong("nearbyDiscoveryRemainingSecs"),
        networks = json.optJSONArray("networks").toNetworkList(),
        lanPeers = json.optJSONArray("lanPeers").toLanPeerList(),
        health = json.optJSONArray("health").toHealthList(),
    )
}

private fun JSONArray?.toRelayList(): List<RelayState> = mapObjects { item ->
    RelayState(
        url = item.optString("url"),
        status = item.optString("status", "unknown"),
        enabled = item.optBoolean("enabled", true),
    )
}

private fun JSONArray?.toNetworkList(): List<NetworkState> = mapObjects { item ->
    NetworkState(
        id = item.optString("id"),
        name = item.optString("name"),
        enabled = item.optBoolean("enabled"),
        networkId = item.optString("networkId"),
        localIsAdmin = item.optBoolean("localIsAdmin"),
        joinRequestsEnabled = item.optBoolean("joinRequestsEnabled"),
        inviteInviterNpub = item.optString("inviteInviterNpub"),
        outboundJoinRequest = !item.isNull("outboundJoinRequest"),
        inboundJoinRequests = item.optJSONArray("inboundJoinRequests").toInboundJoinRequestList(),
        onlineCount = item.optLong("onlineCount"),
        expectedCount = item.optLong("expectedCount"),
        participants = item.optJSONArray("participants").toParticipantList(),
    )
}

private fun JSONArray?.toParticipantList(): List<ParticipantState> = mapObjects { item ->
    ParticipantState(
        npub = item.optString("npub"),
        pubkeyHex = item.optString("pubkeyHex"),
        alias = item.optString("alias"),
        magicDnsAlias = item.optString("magicDnsAlias"),
        magicDnsName = item.optString("magicDnsName"),
        tunnelIp = item.optString("tunnelIp"),
        isAdmin = item.optBoolean("isAdmin"),
        reachable = item.optBoolean("reachable"),
        offersExitNode = item.optBoolean("offersExitNode"),
        fipsEndpointNpub = item.optString("fipsEndpointNpub"),
        fipsEndpointHints = item.optJSONArray("fipsEndpointHints").toStringList(),
        fipsTransportAddr = item.optString("fipsTransportAddr"),
        fipsTransportType = item.optString("fipsTransportType"),
        fipsSrttMs = item.optLong("fipsSrttMs"),
        fipsPacketsSent = item.optLong("fipsPacketsSent"),
        fipsPacketsRecv = item.optLong("fipsPacketsRecv"),
        fipsBytesSent = item.optLong("fipsBytesSent"),
        fipsBytesRecv = item.optLong("fipsBytesRecv"),
        state = item.optString("state"),
        meshState = item.optString("meshState"),
        statusText = item.optString("statusText"),
        lastSeenText = item.optString("lastSeenText", item.optString("lastSignalText")),
    )
}

private fun JSONArray?.toInboundJoinRequestList(): List<InboundJoinRequest> = mapObjects { item ->
    InboundJoinRequest(
        requesterNpub = item.optString("requesterNpub"),
        requesterNodeName = item.optString("requesterNodeName"),
        requestedAtText = item.optString("requestedAtText"),
    )
}

private fun JSONArray?.toLanPeerList(): List<LanPeerState> = mapObjects { item ->
    LanPeerState(
        nodeName = item.optString("nodeName"),
        networkName = item.optString("networkName"),
        invite = item.optString("invite"),
        lastSeenText = item.optString("lastSeenText"),
    )
}

private fun JSONArray?.toHealthList(): List<HealthIssue> = mapObjects { item ->
    HealthIssue(
        severity = item.optString("severity"),
        summary = item.optString("summary"),
        detail = item.optString("detail"),
    )
}

private fun JSONArray?.toStringList(): List<String> {
    if (this == null) return emptyList()
    return List(length()) { index -> optString(index) }.filter { it.isNotBlank() }
}

private fun <T> JSONArray?.mapObjects(convert: (JSONObject) -> T): List<T> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            optJSONObject(index)?.let { add(convert(it)) }
        }
    }
}
