package org.nostrvpn.app.vpn

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.VpnService
import android.os.Build
import android.os.ParcelFileDescriptor
import org.json.JSONObject
import org.nostrvpn.app.MainActivity
import org.nostrvpn.app.R
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
    private var networkCallback: ConnectivityManager.NetworkCallback? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_DISCONNECT -> {
                stopTunnel()
                stopServiceForeground()
                stopSelf()
            }
            else -> startTunnel(intent?.getStringExtra(EXTRA_CONFIG_JSON).orEmpty())
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        stopTunnel()
        stopServiceForeground()
        super.onDestroy()
    }

    private fun startTunnel(configJson: String) {
        if (configJson.isBlank()) {
            stopSelf()
            return
        }
        stopTunnel()

        val config = try {
            JSONObject(configJson)
        } catch (_: Exception) {
            stopSelf()
            return
        }
        if (config.optString("error").isNotBlank()) {
            stopSelf()
            return
        }
        startServiceForeground()

        val descriptor = buildVpnInterface(config) ?: run {
            stopServiceForeground()
            stopSelf()
            return
        }
        val handle = NativeCore.mobileTunnelNew(configJson)
        if (handle == 0L) {
            descriptor.close()
            stopServiceForeground()
            stopSelf()
            return
        }

        tunnelInterface = descriptor
        tunnelHandle = handle
        running.set(true)

        // If the user has WG upstream enabled, the boringtun runtime
        // owns a UDP socket that talks to the Mullvad/Proton server.
        // That socket has to escape the VPN tun (otherwise the
        // encrypted UDP loops back into our own tunnel), which on
        // Android means calling VpnService.protect(socketFd). The
        // Rust side exposes the fd via the JNI binding below; -1 means
        // WG upstream isn't running so there's nothing to protect.
        val wgSocketFd = NativeCore.mobileTunnelWgSocketFd(handle)
        android.util.Log.i(
            "NostrVpnService",
            "WG upstream socket fd from native runtime: $wgSocketFd (-1 means WG upstream not running)",
        )
        if (wgSocketFd >= 0) {
            val protected_ = protect(wgSocketFd)
            android.util.Log.i(
                "NostrVpnService",
                "VpnService.protect(wgSocketFd=$wgSocketFd) returned $protected_",
            )
            if (!protected_) {
                android.util.Log.w(
                    "NostrVpnService",
                    "protect(fd) failed — WG upstream may loop into the VPN tun",
                )
            }
        }

        registerUnderlyingNetworkUpdates()
        readThread = Thread({ readTunLoop(descriptor, handle) }, "nvpn-tun-read").also { it.start() }
        writeThread = Thread({ writeTunLoop(descriptor, handle) }, "nvpn-tun-write").also { it.start() }
    }

    private fun buildVpnInterface(config: JSONObject): ParcelFileDescriptor? {
        val builder = Builder()
            .setSession("Nostr VPN")
            .setMtu(config.optInt("mtu", 1280))
            .setBlocking(true)
            .allowBypass()

        val underlyingNetworks = currentUnderlyingNetworks()
        if (underlyingNetworks.isNotEmpty()) {
            builder.setUnderlyingNetworks(underlyingNetworks)
        }
        excludeOwnProcess(builder)

        val local = parseCidr(config.optString("localAddress", "10.44.0.1/32")) ?: return null
        builder.addAddress(local.address, local.prefix)

        val routes = config.optJSONArray("routeTargets")
        if (routes != null) {
            for (index in 0 until routes.length()) {
                val route = parseCidr(routes.optString(index)) ?: continue
                builder.addRoute(route.address, route.prefix)
            }
        }

        // When WG upstream is on, the Rust runtime expanded
        // routeTargets to 0.0.0.0/0 so all traffic enters the tun.
        // Android doesn't have an `excludedRoutes` equivalent — we
        // rely on `protect(socketFd)` instead (called below after the
        // tunnel handle is created). The excludedRoutes JSON field
        // is therefore informational on Android; the actual escape
        // mechanism is the protected socket.

        return builder.establish()
    }

    private fun currentUnderlyingNetworks(): Array<Network> {
        val connectivity = getSystemService(ConnectivityManager::class.java) ?: return emptyArray()
        val network = connectivity.activeNetwork ?: return emptyArray()
        val capabilities = connectivity.getNetworkCapabilities(network) ?: return emptyArray()
        if (!capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)) {
            return emptyArray()
        }
        if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) {
            return emptyArray()
        }
        return arrayOf(network)
    }

    private fun excludeOwnProcess(builder: Builder) {
        try {
            builder.addDisallowedApplication(packageName)
        } catch (_: PackageManager.NameNotFoundException) {
            // The package must exist for a running service; ignore impossible platform races.
        }
    }

    private fun registerUnderlyingNetworkUpdates() {
        unregisterUnderlyingNetworkUpdates()
        val connectivity = getSystemService(ConnectivityManager::class.java) ?: return
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                refreshUnderlyingNetworks()
            }

            override fun onLost(network: Network) {
                refreshUnderlyingNetworks()
            }

            override fun onCapabilitiesChanged(
                network: Network,
                networkCapabilities: NetworkCapabilities,
            ) {
                refreshUnderlyingNetworks()
            }
        }
        try {
            connectivity.registerDefaultNetworkCallback(callback)
            networkCallback = callback
            refreshUnderlyingNetworks()
        } catch (_: RuntimeException) {
            networkCallback = null
        }
    }

    private fun unregisterUnderlyingNetworkUpdates() {
        val callback = networkCallback ?: return
        networkCallback = null
        val connectivity = getSystemService(ConnectivityManager::class.java) ?: return
        try {
            connectivity.unregisterNetworkCallback(callback)
        } catch (_: RuntimeException) {
            // The callback may already be gone during service teardown.
        }
    }

    private fun refreshUnderlyingNetworks() {
        val networks = currentUnderlyingNetworks()
        setUnderlyingNetworks(networks.takeIf { it.isNotEmpty() })
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
        unregisterUnderlyingNetworkUpdates()
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

    private fun startServiceForeground() {
        createNotificationChannel()
        val notification = tunnelNotification()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun stopServiceForeground() {
        stopForeground(STOP_FOREGROUND_REMOVE)
    }

    private fun createNotificationChannel() {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                NOTIFICATION_CHANNEL_ID,
                getString(R.string.app_name),
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                setShowBadge(false)
            },
        )
    }

    private fun tunnelNotification(): Notification {
        val openAppIntent = packageManager.getLaunchIntentForPackage(packageName)
            ?: Intent(this, MainActivity::class.java)
        val openApp = PendingIntent.getActivity(
            this,
            0,
            openAppIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val disconnect = PendingIntent.getService(
            this,
            1,
            Intent(this, NostrVpnService::class.java).setAction(ACTION_DISCONNECT),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return Notification.Builder(this, NOTIFICATION_CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_launcher_monochrome)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(getString(R.string.vpn_notification_connected))
            .setContentIntent(openApp)
            .setOngoing(true)
            .setCategory(Notification.CATEGORY_SERVICE)
            .addAction(
                Notification.Action.Builder(
                    Icon.createWithResource(this, R.drawable.ic_launcher_monochrome),
                    getString(R.string.vpn_notification_disconnect),
                    disconnect,
                ).build(),
            )
            .build()
    }

    private data class Cidr(val address: String, val prefix: Int)

    companion object {
        const val ACTION_CONNECT = "org.nostrvpn.app.vpn.CONNECT"
        const val ACTION_DISCONNECT = "org.nostrvpn.app.vpn.DISCONNECT"
        const val EXTRA_CONFIG_JSON = "configJson"
        private const val NOTIFICATION_CHANNEL_ID = "vpn"
        private const val NOTIFICATION_ID = 7001
    }
}
