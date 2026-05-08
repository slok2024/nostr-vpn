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
    val activeNetworkInvite: String = "",
    val connectedPeerCount: Long = 0,
    val expectedPeerCount: Long = 0,
    val meshReady: Boolean = false,
    val exitNode: String = "",
    val advertiseExitNode: Boolean = false,
    val advertisedRoutes: List<String> = emptyList(),
    val magicDnsSuffix: String = "",
    val magicDnsStatus: String = "",
    val autoconnect: Boolean = false,
    val lanPairingActive: Boolean = false,
    val lanPairingRemainingSecs: Long = 0,
    val networks: List<NetworkState> = emptyList(),
    val lanPeers: List<LanPeerState> = emptyList(),
    val health: List<HealthIssue> = emptyList(),
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
    val fipsTransportAddr: String = "",
    val fipsTransportType: String = "",
    val fipsSrttMs: Long = 0,
    val fipsPacketsSent: Long = 0,
    val fipsPacketsRecv: Long = 0,
    val fipsBytesSent: Long = 0,
    val fipsBytesRecv: Long = 0,
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
        activeNetworkInvite = json.optString("activeNetworkInvite"),
        connectedPeerCount = json.optLong("connectedPeerCount"),
        expectedPeerCount = json.optLong("expectedPeerCount"),
        meshReady = json.optBoolean("meshReady"),
        exitNode = json.optString("exitNode"),
        advertiseExitNode = json.optBoolean("advertiseExitNode"),
        advertisedRoutes = json.optJSONArray("advertisedRoutes").toStringList(),
        magicDnsSuffix = json.optString("magicDnsSuffix"),
        magicDnsStatus = json.optString("magicDnsStatus"),
        autoconnect = json.optBoolean("autoconnect"),
        lanPairingActive = json.optBoolean("lanPairingActive"),
        lanPairingRemainingSecs = json.optLong("lanPairingRemainingSecs"),
        networks = json.optJSONArray("networks").toNetworkList(),
        lanPeers = json.optJSONArray("lanPeers").toLanPeerList(),
        health = json.optJSONArray("health").toHealthList(),
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
        fipsTransportAddr = item.optString("fipsTransportAddr"),
        fipsTransportType = item.optString("fipsTransportType"),
        fipsSrttMs = item.optLong("fipsSrttMs"),
        fipsPacketsSent = item.optLong("fipsPacketsSent"),
        fipsPacketsRecv = item.optLong("fipsPacketsRecv"),
        fipsBytesSent = item.optLong("fipsBytesSent"),
        fipsBytesRecv = item.optLong("fipsBytesRecv"),
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
