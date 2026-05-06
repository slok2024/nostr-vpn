package org.nostrvpn.app.core

internal object NativeCore {
    init {
        System.loadLibrary("nostr_vpn_app_core")
    }

    external fun appNew(dataDir: String, appVersion: String): Long
    external fun appFree(handle: Long)
    external fun stateJson(handle: Long): String
    external fun refreshJson(handle: Long): String
    external fun dispatchJson(handle: Long, actionJson: String): String
    external fun qrMatrixJson(text: String): String
    external fun decodeQrImageJson(path: String): String
    external fun mobileTunnelConfigJson(dataDir: String): String
    external fun mobileTunnelNew(configJson: String): Long
    external fun mobileTunnelFree(handle: Long)
    external fun mobileTunnelSendPacket(handle: Long, packet: ByteArray, len: Int): Boolean
    external fun mobileTunnelNextPacket(handle: Long, output: ByteArray, timeoutMs: Int): Int
}
