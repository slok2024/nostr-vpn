package org.nostrvpn.app.vpn

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.VpnService
import android.net.wifi.WifiManager
import android.os.Build
import android.os.ParcelFileDescriptor
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import org.nostrvpn.app.MainActivity
import org.nostrvpn.app.R
import org.nostrvpn.app.appCoreDataDir
import org.nostrvpn.app.core.NativeCore
import org.nostrvpn.app.seedMobileConfig
import java.io.FileInputStream
import java.io.FileOutputStream
import java.net.Inet4Address
import java.util.concurrent.atomic.AtomicBoolean

class NostrVpnService : VpnService() {
    private val running = AtomicBoolean(false)
    private var tunnelHandle: Long = 0
    private var tunnelInterface: ParcelFileDescriptor? = null
    private var readThread: Thread? = null
    private var writeThread: Thread? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var multicastLock: WifiManager.MulticastLock? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        return when (intent?.action) {
            ACTION_DISCONNECT -> {
                VpnStartState.setUserWantsVpn(this, false)
                stopTunnel()
                stopServiceForeground()
                stopSelf()
                START_NOT_STICKY
            }
            ACTION_CONNECT -> {
                VpnStartState.setUserWantsVpn(this, true)
                startTunnel(
                    intent.getStringExtra(EXTRA_CONFIG_JSON).orEmpty(),
                    foregroundRequired = true,
                ).stickyResult()
            }
            ACTION_RESTORE -> {
                if (!VpnStartState.userWantsVpn(this)) {
                    stopSelf()
                    START_NOT_STICKY
                } else {
                    startTunnel(
                        persistedTunnelConfigJson(),
                        foregroundRequired = true,
                    ).stickyResult()
                }
            }
            VpnService.SERVICE_INTERFACE -> {
                // Android starts the service with this action for OS Always-on VPN.
                // Treat that as a real request to restore the tunnel from disk,
                // not as an empty interactive connect intent.
                VpnStartState.setUserWantsVpn(this, true)
                startTunnel(
                    persistedTunnelConfigJson(),
                    foregroundRequired = false,
                ).stickyResult()
            }
            else -> {
                if (VpnStartState.userWantsVpn(this)) {
                    startTunnel(
                        persistedTunnelConfigJson(),
                        foregroundRequired = true,
                    ).stickyResult()
                } else {
                    START_NOT_STICKY
                }
            }
        }
    }

    override fun onDestroy() {
        stopTunnel()
        stopServiceForeground()
        super.onDestroy()
    }

    override fun onRevoke() {
        VpnStartState.setUserWantsVpn(this, false)
        stopTunnel()
        stopServiceForeground()
        super.onRevoke()
    }

    private fun startTunnel(configJson: String, foregroundRequired: Boolean): Boolean {
        val foregroundStarted = if (foregroundRequired) {
            startServiceForeground()
        } else {
            false
        }
        if (foregroundRequired && !foregroundStarted) {
            stopSelf()
            return false
        }
        if (configJson.isBlank()) {
            return failStart(foregroundStarted, "VPN config is empty")
        }
        NativeCore.initializeAndroidContext(applicationContext)

        val config = try {
            JSONObject(configJson)
        } catch (error: Exception) {
            return failStart(foregroundStarted, "VPN config JSON could not be parsed", error)
        }
        val configError = config.optString("error")
        if (configError.isNotBlank()) {
            return failStart(foregroundStarted, configError)
        }
        val lockdownActive = currentLockdownActive()
        VpnStartState.setLockdownActive(this, lockdownActive)
        if (lockdownActive && !config.hasDefaultRoute()) {
            Log.w(
                "NostrVpnService",
                "Android VPN lockdown is active without a default VPN route; non-nvpn internet will be blocked",
            )
        }
        config.put("dnsForwarders", currentUnderlyingDnsServers())
        val tunnelConfigJson = config.toString()

        stopTunnel()
        if (!foregroundStarted) {
            publishTunnelNotification()
        }
        reconcileMulticastLock(config)

        val descriptor = buildVpnInterface(config) ?: run {
            releaseMulticastLock()
            stopServiceForeground()
            stopSelf()
            return false
        }
        val handle = NativeCore.mobileTunnelNew(tunnelConfigJson)
        if (handle == 0L) {
            descriptor.close()
            releaseMulticastLock()
            stopServiceForeground()
            stopSelf()
            return false
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
        Log.i(
            "NostrVpnService",
            "WG upstream socket fd from native runtime: $wgSocketFd (-1 means WG upstream not running)",
        )
        if (wgSocketFd >= 0) {
            val protected_ = protect(wgSocketFd)
            Log.i(
                "NostrVpnService",
                "VpnService.protect(wgSocketFd=$wgSocketFd) returned $protected_",
            )
            if (!protected_) {
                Log.w(
                    "NostrVpnService",
                    "protect(fd) failed — WG upstream may loop into the VPN tun",
                )
            }
        }

        registerUnderlyingNetworkUpdates()
        readThread = Thread({ readTunLoop(descriptor, handle) }, "nvpn-tun-read").also { it.start() }
        writeThread = Thread({ writeTunLoop(descriptor, handle) }, "nvpn-tun-write").also { it.start() }
        return true
    }

    private fun Boolean.stickyResult(): Int =
        if (this) START_STICKY else START_NOT_STICKY

    private fun failStart(
        foregroundStarted: Boolean,
        message: String,
        error: Throwable? = null,
    ): Boolean {
        if (error == null) {
            Log.w("NostrVpnService", message)
        } else {
            Log.w("NostrVpnService", message, error)
        }
        if (!running.get()) {
            if (foregroundStarted) {
                stopServiceForeground()
            } else {
                clearTunnelNotification()
            }
            stopSelf()
        }
        return false
    }

    private fun persistedTunnelConfigJson(): String {
        NativeCore.initializeAndroidContext(applicationContext)
        val dataDir = appCoreDataDir(this)
        seedMobileConfig(dataDir)
        return NativeCore.mobileTunnelConfigJson(dataDir.absolutePath)
    }

    private fun buildVpnInterface(config: JSONObject): ParcelFileDescriptor? {
        // Note: we deliberately do NOT call `allowBypass()` here.
        // Bypassable VPNs are also the only ones for which Android
        // suppresses the persistent key icon in the status bar — so
        // marking ours bypassable would silently hide the only signal
        // users have that the VPN is actually running. We already
        // protect the boringtun UDP socket via `protect(fd)` below,
        // which is the only socket that actually needs to escape the
        // tun, so allowBypass() doesn't buy us anything anyway.
        val builder = Builder()
            .setSession("Nostr VPN")
            .setConfigureIntent(configureIntent())
            .setMtu(config.optInt("mtu", 1150))
            .setBlocking(true)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            builder.setMetered(false)
        }

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
        addDnsServers(builder, config)

        // When WG upstream or a Nostr peer exit is on, the Rust runtime
        // expanded routeTargets to 0.0.0.0/0 so all traffic enters the tun.
        // Android doesn't have an `excludedRoutes` equivalent — we
        // rely on `protect(socketFd)` for WG upstream instead (called below
        // after the tunnel handle is created). The excludedRoutes JSON field
        // is therefore informational on Android for that mode; the actual
        // escape mechanism is the protected socket.

        return runCatching {
            builder.establish()
        }.onFailure { error ->
            Log.w("NostrVpnService", "Failed to establish Android VPN interface", error)
        }.getOrNull()
    }

    @Suppress("DEPRECATION")
    private fun currentUnderlyingNetworks(): Array<Network> {
        val connectivity = getSystemService(ConnectivityManager::class.java) ?: return emptyArray()
        val candidates = linkedSetOf<Network>()
        connectivity.activeNetwork?.let { candidates.add(it) }
        candidates.addAll(connectivity.allNetworks)
        return candidates.filter { network ->
            val capabilities = connectivity.getNetworkCapabilities(network) ?: return@filter false
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
                !capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
        }.toTypedArray()
    }

    private fun currentUnderlyingDnsServers(): JSONArray {
        val servers = JSONArray()
        val connectivity = getSystemService(ConnectivityManager::class.java) ?: return servers
        currentUnderlyingNetworks().forEach { network ->
            connectivity.getLinkProperties(network)?.dnsServers.orEmpty()
                .filterIsInstance<Inet4Address>()
                .mapNotNull { it.hostAddress }
                .filter { it.isNotBlank() }
                .forEach { servers.put(it) }
        }
        return servers
    }

    private fun excludeOwnProcess(builder: Builder) {
        try {
            builder.addDisallowedApplication(packageName)
        } catch (_: PackageManager.NameNotFoundException) {
            // The package must exist for a running service; ignore impossible platform races.
        }
    }

    private fun addDnsServers(builder: Builder, config: JSONObject) {
        val servers = config.optJSONArray("dnsServers") ?: return
        val magicDnsServer = config.optString("magicDnsServer").trim()
        val selected = mutableListOf<String>()
        for (index in 0 until servers.length()) {
            val server = servers.optString(index).trim()
            if (server.isEmpty()) continue
            selected.add(server)
        }
        val effectiveServers = if (magicDnsServer.isNotEmpty() && selected.any { it == magicDnsServer }) {
            listOf(magicDnsServer)
        } else {
            selected
        }
        for (server in effectiveServers) {
            runCatching {
                builder.addDnsServer(server)
            }.onFailure { error ->
                Log.w("NostrVpnService", "Ignoring invalid VPN DNS server: $server", error)
            }
        }
    }

    private fun currentLockdownActive(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && runCatching {
            isLockdownEnabled
        }.getOrDefault(false)

    private fun JSONObject.hasDefaultRoute(): Boolean {
        val routes = optJSONArray("routeTargets") ?: return false
        for (index in 0 until routes.length()) {
            if (routes.optString(index).trim() == "0.0.0.0/0") {
                return true
            }
        }
        return false
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

    private fun reconcileMulticastLock(config: JSONObject) {
        if (!shouldHoldMulticastLock(config)) {
            releaseMulticastLock()
            return
        }
        acquireMulticastLock()
    }

    private fun shouldHoldMulticastLock(config: JSONObject): Boolean {
        if (!config.optBoolean("shareLocalCandidates", false)) return false
        if (!config.optBoolean("nostrDiscoveryEnabled", false)) return false
        val hasPeers = (config.optJSONArray("peers")?.length() ?: 0) > 0
        val joinRequestsEnabled = config.optBoolean("joinRequestsEnabled", false)
        val pendingJoinRequest =
            config.optString("pendingJoinRequestRecipient").isNotBlank() &&
                config.optLong("pendingJoinRequestedAt", 0) != 0L
        return hasPeers || joinRequestsEnabled || pendingJoinRequest
    }

    private fun acquireMulticastLock() {
        if (multicastLock != null) return
        val wifi = applicationContext.getSystemService(WifiManager::class.java) ?: return
        multicastLock = wifi.createMulticastLock("nostr-vpn-lan-discovery").apply {
            setReferenceCounted(false)
            runCatching { acquire() }
        }
    }

    private fun releaseMulticastLock() {
        val lock = multicastLock ?: return
        multicastLock = null
        runCatching {
            if (lock.isHeld) {
                lock.release()
            }
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
        releaseMulticastLock()
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

    private fun startServiceForeground(): Boolean {
        createNotificationChannel()
        val notification = tunnelNotification()
        return runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                startForeground(
                    NOTIFICATION_ID,
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE,
                )
            } else {
                startForeground(NOTIFICATION_ID, notification)
            }
        }.onFailure { error ->
            Log.w("NostrVpnService", "Failed to start foreground VPN notification", error)
        }.isSuccess
    }

    private fun publishTunnelNotification() {
        createNotificationChannel()
        runCatching {
            getSystemService(NotificationManager::class.java).notify(
                NOTIFICATION_ID,
                tunnelNotification(),
            )
        }.onFailure { error ->
            Log.w("NostrVpnService", "Failed to publish VPN notification", error)
        }
    }

    private fun stopServiceForeground() {
        stopForeground(STOP_FOREGROUND_REMOVE)
        clearTunnelNotification()
    }

    private fun clearTunnelNotification() {
        runCatching {
            getSystemService(NotificationManager::class.java).cancel(NOTIFICATION_ID)
        }
    }

    private fun configureIntent(): PendingIntent {
        return PendingIntent.getActivity(
            this,
            2,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
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
        const val ACTION_RESTORE = "org.nostrvpn.app.vpn.RESTORE"
        const val EXTRA_CONFIG_JSON = "configJson"
        private const val NOTIFICATION_CHANNEL_ID = "vpn"
        private const val NOTIFICATION_ID = 7001

        fun startRestore(context: Context) {
            val intent = Intent(context, NostrVpnService::class.java)
                .setAction(ACTION_RESTORE)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }
    }
}
