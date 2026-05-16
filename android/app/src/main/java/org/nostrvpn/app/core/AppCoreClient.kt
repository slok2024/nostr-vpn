package org.nostrvpn.app.core

import org.json.JSONObject

class AppCoreClient(private val dataDir: String, appVersion: String) : AutoCloseable {
    private var handle: Long = NativeCore.appNew(dataDir, appVersion)

    fun state(): AppState = parseAppState(NativeCore.stateJson(requireHandle()))

    fun refresh(): AppState = parseAppState(NativeCore.refreshJson(requireHandle()))

    fun dispatch(action: JSONObject): AppState =
        parseAppState(NativeCore.dispatchJson(requireHandle(), action.toString()))

    fun qrMatrix(invite: String): JSONObject = JSONObject(NativeCore.qrMatrixJson(invite))

    fun decodeQrImage(path: String): JSONObject = JSONObject(NativeCore.decodeQrImageJson(path))

    fun mobileTunnelConfigJson(): String = NativeCore.mobileTunnelConfigJson(dataDir)

    override fun close() {
        val current = handle
        if (current != 0L) {
            NativeCore.appFree(current)
            handle = 0
        }
    }

    private fun requireHandle(): Long {
        check(handle != 0L) { "native app core is closed" }
        return handle
    }
}

object NativeActions {
    fun connectVpn() = action("connect_vpn")
    fun disconnectVpn() = action("disconnect_vpn")
    fun importInvite(invite: String) = action("import_network_invite", "invite" to invite)
    fun startInviteBroadcast() = action("start_invite_broadcast")
    fun stopInviteBroadcast() = action("stop_invite_broadcast")
    fun startNearbyDiscovery() = action("start_nearby_discovery")
    fun stopNearbyDiscovery() = action("stop_nearby_discovery")
    fun addNetwork(name: String) = action("add_network", "name" to name)
    fun manualAddNetwork(adminNpub: String, meshNetworkId: String) =
        action("manual_add_network", "adminNpub" to adminNpub, "meshNetworkId" to meshNetworkId)

    fun setNetworkEnabled(networkId: String, enabled: Boolean) =
        action("set_network_enabled", "networkId" to networkId, "enabled" to enabled)

    fun setJoinRequests(networkId: String, enabled: Boolean) =
        action("set_network_join_requests_enabled", "networkId" to networkId, "enabled" to enabled)

    fun acceptJoinRequest(networkId: String, requesterNpub: String) =
        action("accept_join_request", "networkId" to networkId, "requesterNpub" to requesterNpub)

    fun rejectJoinRequest(networkId: String, requesterNpub: String) =
        action("reject_join_request", "networkId" to networkId, "requesterNpub" to requesterNpub)

    fun removeNetwork(networkId: String) = action("remove_network", "networkId" to networkId)

    fun updateSettings(vararg settings: Pair<String, Any?>): JSONObject =
        JSONObject()
            .put("type", "update_settings")
            .put(
                "patch",
                JSONObject().apply {
                    settings.forEach { (key, value) -> put(key, value) }
                },
            )

    private fun action(type: String, vararg fields: Pair<String, Any?>): JSONObject =
        JSONObject().put("type", type).apply {
            fields.forEach { (key, value) -> put(key, value) }
        }
}
