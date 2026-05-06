package org.nostrvpn.app.vpn

import android.content.Intent
import android.net.VpnService
import android.os.ParcelFileDescriptor
import org.json.JSONObject
import org.nostrvpn.app.core.NativeCore
import java.io.FileInputStream
import java.io.FileOutputStream
import java.util.concurrent.atomic.AtomicBoolean

class NostrVpnService : VpnService() {
    private val running = AtomicBoolean(false)
    private var tunnelHandle: Long = 0
    private var tunnelInterface: ParcelFileDescriptor? = null
    private var readThread: Thread? = null
    private var writeThread: Thread? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_DISCONNECT -> {
                stopTunnel()
                stopSelf()
            }
            else -> startTunnel(intent?.getStringExtra(EXTRA_CONFIG_JSON).orEmpty())
        }
        return START_STICKY
    }

    override fun onDestroy() {
        stopTunnel()
        super.onDestroy()
    }

    private fun startTunnel(configJson: String) {
        if (configJson.isBlank()) {
            stopSelf()
            return
        }
        stopTunnel()

        val config = JSONObject(configJson)
        if (config.optString("error").isNotBlank()) {
            stopSelf()
            return
        }

        val descriptor = buildVpnInterface(config) ?: run {
            stopSelf()
            return
        }
        val handle = NativeCore.mobileTunnelNew(configJson)
        if (handle == 0L) {
            descriptor.close()
            stopSelf()
            return
        }

        tunnelInterface = descriptor
        tunnelHandle = handle
        running.set(true)
        readThread = Thread({ readTunLoop(descriptor, handle) }, "nvpn-tun-read").also { it.start() }
        writeThread = Thread({ writeTunLoop(descriptor, handle) }, "nvpn-tun-write").also { it.start() }
    }

    private fun buildVpnInterface(config: JSONObject): ParcelFileDescriptor? {
        val builder = Builder()
            .setSession("Nostr VPN")
            .setMtu(config.optInt("mtu", 1280))
            .setBlocking(true)

        val local = parseCidr(config.optString("localAddress", "10.44.0.1/32")) ?: return null
        builder.addAddress(local.address, local.prefix)

        val routes = config.optJSONArray("routeTargets")
        if (routes != null) {
            for (index in 0 until routes.length()) {
                val route = parseCidr(routes.optString(index)) ?: continue
                builder.addRoute(route.address, route.prefix)
            }
        }

        return builder.establish()
    }

    private fun readTunLoop(descriptor: ParcelFileDescriptor, handle: Long) {
        val input = FileInputStream(descriptor.fileDescriptor)
        val buffer = ByteArray(65_535)
        while (running.get()) {
            val count = try {
                input.read(buffer)
            } catch (_: Exception) {
                break
            }
            if (count <= 0) {
                break
            }
            NativeCore.mobileTunnelSendPacket(handle, buffer, count)
        }
    }

    private fun writeTunLoop(descriptor: ParcelFileDescriptor, handle: Long) {
        val output = FileOutputStream(descriptor.fileDescriptor)
        val buffer = ByteArray(65_535)
        while (running.get()) {
            val count = NativeCore.mobileTunnelNextPacket(handle, buffer, 1_000)
            if (count > 0) {
                try {
                    output.write(buffer, 0, count)
                } catch (_: Exception) {
                    break
                }
            } else if (count < 0) {
                break
            }
        }
    }

    private fun stopTunnel() {
        running.set(false)
        val descriptor = tunnelInterface
        tunnelInterface = null
        descriptor?.close()
        val currentThread = Thread.currentThread()
        val threads = listOf(readThread, writeThread)
        readThread = null
        writeThread = null
        threads.forEach { it?.interrupt() }
        threads.forEach { thread ->
            if (thread != null && thread != currentThread) {
                try {
                    thread.join(1_500)
                } catch (_: InterruptedException) {
                    currentThread.interrupt()
                }
            }
        }
        val handle = tunnelHandle
        tunnelHandle = 0
        if (handle != 0L) {
            NativeCore.mobileTunnelFree(handle)
        }
    }

    private fun parseCidr(value: String): Cidr? {
        val parts = value.trim().split("/", limit = 2)
        val address = parts.firstOrNull()?.takeIf { it.isNotBlank() } ?: return null
        val prefix = parts.getOrNull(1)?.toIntOrNull() ?: 32
        if (prefix !in 0..32) {
            return null
        }
        return Cidr(address, prefix)
    }

    private data class Cidr(val address: String, val prefix: Int)

    companion object {
        const val ACTION_CONNECT = "org.nostrvpn.app.vpn.CONNECT"
        const val ACTION_DISCONNECT = "org.nostrvpn.app.vpn.DISCONNECT"
        const val EXTRA_CONFIG_JSON = "configJson"
    }
}
