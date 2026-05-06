package org.nostrvpn.app.core

import org.json.JSONObject

class AppCoreClient(private val dataDir: String, appVersion: String) : AutoCloseable {
    private var handle: Long = NativeCore.appNew(dataDir, appVersion)

    fun state(): AppState = parseAppState(NativeCore.stateJson(requireHandle()))

    fun refresh(): AppState = parseAppState(NativeCore.refreshJson(requireHandle()))

    fun dispatch(action: JSONObject): AppState =
        parseAppState(NativeCore.dispatchJson(requireHandle(), action.toString()))

    fun qrMatrix(invite: String): JSONObject = JSONObject(NativeCore.qrMatrixJson(invite))

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
    fun connectSession() = action("connect_session")
    fun disconnectSession() = action("disconnect_session")
    fun importInvite(invite: String) = action("import_network_invite", "invite" to invite)
    fun startLanPairing() = action("start_lan_pairing")
    fun stopLanPairing() = action("stop_lan_pairing")
    fun addRelay(relay: String) = action("add_relay", "relay" to relay)
    fun removeRelay(relay: String) = action("remove_relay", "relay" to relay)
    fun addNetwork(name: String) = action("add_network", "name" to name)
    fun setNetworkEnabled(networkId: String, enabled: Boolean) =
        action("set_network_enabled", "networkId" to networkId, "enabled" to enabled)

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
