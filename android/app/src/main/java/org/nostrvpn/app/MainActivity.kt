package org.nostrvpn.app

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import kotlinx.coroutines.delay
import org.json.JSONObject
import org.nostrvpn.app.core.AppCoreClient
import org.nostrvpn.app.core.NativeActions
import org.nostrvpn.app.vpn.NostrVpnService

class MainActivity : ComponentActivity() {
    private var deepLink by mutableStateOf<String?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        deepLink = intent?.dataString
        val core = AppCoreClient(filesDir.resolve("app-core").absolutePath, BuildConfig.VERSION_NAME)

        setContent {
            var state by remember { mutableStateOf(core.state()) }
            val dispatch: (JSONObject) -> Unit = { action ->
                val wasActive = state.sessionActive
                state = try {
                    core.dispatch(action)
                } catch (error: Exception) {
                    state.copy(error = error.message ?: "Android action failed")
                }
                if (!wasActive && state.sessionActive) {
                    startService(
                        Intent(this, NostrVpnService::class.java)
                            .setAction(NostrVpnService.ACTION_CONNECT)
                            .putExtra(
                                NostrVpnService.EXTRA_CONFIG_JSON,
                                core.mobileTunnelConfigJson(),
                            ),
                    )
                } else if (wasActive && !state.sessionActive) {
                    startService(
                        Intent(this, NostrVpnService::class.java)
                            .setAction(NostrVpnService.ACTION_DISCONNECT),
                    )
                }
            }

            DisposableEffect(core) {
                onDispose { core.close() }
            }
            LaunchedEffect(core) {
                while (true) {
                    delay(2_000)
                    state = try {
                        core.refresh()
                    } catch (error: Exception) {
                        state.copy(error = error.message ?: "Android refresh failed")
                    }
                }
            }
            LaunchedEffect(deepLink) {
                val invite = deepLink
                if (!invite.isNullOrBlank() && invite.startsWith("nvpn://", ignoreCase = true)) {
                    dispatch(NativeActions.importInvite(invite))
                    deepLink = null
                }
            }

            NostrVpnTheme {
                NostrVpnApp(
                    state = state,
                    qrJson = { invite -> core.qrMatrix(invite) },
                    dispatch = dispatch,
                )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        deepLink = intent.dataString
    }
}
