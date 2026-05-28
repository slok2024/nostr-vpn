package org.nostrvpn.app.vpn

import android.content.Context
import android.provider.Settings

internal object VpnStartState {
    private const val PREFS = "nostr_vpn_service"
    private const val USER_WANTS_VPN = "user_wants_vpn"
    private const val LOCKDOWN_ACTIVE = "lockdown_active"
    private const val ALWAYS_ON_VPN_APP = "always_on_vpn_app"
    private const val ALWAYS_ON_VPN_LOCKDOWN = "always_on_vpn_lockdown"

    fun setUserWantsVpn(context: Context, enabled: Boolean) {
        context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit()
            .putBoolean(USER_WANTS_VPN, enabled)
            .apply()
    }

    fun userWantsVpn(context: Context): Boolean =
        context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getBoolean(USER_WANTS_VPN, false)

    fun setLockdownActive(context: Context, enabled: Boolean) {
        context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit()
            .putBoolean(LOCKDOWN_ACTIVE, enabled)
            .apply()
    }

    fun lockdownActive(context: Context): Boolean =
        context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getBoolean(LOCKDOWN_ACTIVE, false)

    fun refreshLockdownActive(context: Context): Boolean {
        val enabled = systemLockdownActiveForThisApp(context)
        setLockdownActive(context, enabled)
        return enabled
    }

    private fun systemLockdownActiveForThisApp(context: Context): Boolean =
        runCatching {
            val appContext = context.applicationContext
            val resolver = appContext.contentResolver
            Settings.Secure.getString(resolver, ALWAYS_ON_VPN_APP) == appContext.packageName &&
                Settings.Secure.getInt(resolver, ALWAYS_ON_VPN_LOCKDOWN, 0) != 0
        }.getOrDefault(lockdownActive(context))
}
