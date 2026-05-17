package org.nostrvpn.app

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.Switch
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import org.json.JSONObject
import org.nostrvpn.app.core.AppState
import org.nostrvpn.app.core.LanPeerState
import org.nostrvpn.app.core.NativeActions
import org.nostrvpn.app.core.NetworkState
import org.nostrvpn.app.core.ParticipantState

internal fun networkTitle(network: NetworkState?): String =
    network?.name?.ifBlank { "Private network" } ?: "No network"

@Composable
internal fun ParticipantRow(
    state: AppState,
    participant: ParticipantState,
    network: NetworkState? = null,
    dispatch: ((JSONObject) -> Unit)? = null,
) {
    val isSelf = participant.isSelf(state)
    var detailOpen by remember { mutableStateOf(false) }
    AppCard {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.clickable { detailOpen = true },
        ) {
            Dot(selected = if (isSelf) state.vpnActive else participant.reachable)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    participant.displayName(state),
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    if (isSelf) Pill("This device", Color(0xFFECFDF5), Ok)
                    if (participant.isAdmin) Pill("Admin", Color(0xFFF5F3FF), Accent)
                    if (participant.offersExitNode) Pill("Exit", Color(0xFFFFF7ED), Color(0xFFA16207))
                    if (participant.isFipsRouted(state)) Pill("Routed", Color(0xFFF1F5F9), Muted)
                }
                Text(participant.subtitle(isSelf), color = Muted, maxLines = 1)
                Text(participant.statusLabel(state), color = Muted, style = MaterialTheme.typography.bodySmall)
            }
            Text("›", color = Muted)
        }
    }
    if (detailOpen) {
        DeviceDetailDialog(
            state = state,
            participant = participant,
            network = network,
            dispatch = dispatch,
            onDismiss = { detailOpen = false },
        )
    }
}

@Composable
private fun DeviceDetailDialog(
    state: AppState,
    participant: ParticipantState,
    network: NetworkState?,
    dispatch: ((JSONObject) -> Unit)?,
    onDismiss: () -> Unit,
) {
    val isSelf = participant.isSelf(state)
    val manageNetwork = network?.takeIf { it.localIsAdmin && !isSelf }
    val manageDispatch = dispatch
    var aliasDraft by remember { mutableStateOf(participant.magicDnsAlias) }
    var pendingRemove by remember { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(participant.displayName(state)) },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Text(participant.detailStatusLabel(state), color = Muted)
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    if (isSelf) Pill("This device", Color(0xFFECFDF5), Ok)
                    if (participant.isAdmin) Pill("Admin", Color(0xFFF5F3FF), Accent)
                    if (participant.offersExitNode) Pill("Exit", Color(0xFFFFF7ED), Color(0xFFA16207))
                }
                if (participant.magicDnsName.isNotBlank()) {
                    Text("Magic DNS", style = MaterialTheme.typography.labelMedium, color = Muted)
                    Text(participant.magicDnsName)
                }
                Text("Device ID", style = MaterialTheme.typography.labelMedium, color = Muted)
                CopyLine(participant.npub)
                if (participant.tunnelIp.isNotBlank()) {
                    Text("Tunnel IP", style = MaterialTheme.typography.labelMedium, color = Muted)
                    CopyLine(participant.tunnelIp)
                }
                if (manageNetwork != null && manageDispatch != null) {
                    Spacer(Modifier.height(8.dp))
                    OutlinedTextField(
                        value = aliasDraft,
                        onValueChange = { aliasDraft = it },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        label = { Text("Alias") },
                    )
                    Button(onClick = {
                        manageDispatch(JSONObject().put("type", "set_participant_alias")
                            .put("npub", participant.npub).put("alias", aliasDraft))
                    }) { Text("Save alias") }
                    OutlinedButton(onClick = {
                        val type = if (participant.isAdmin) "remove_admin" else "add_admin"
                        manageDispatch(JSONObject().put("type", type)
                            .put("networkId", manageNetwork.id).put("npub", participant.npub))
                    }) {
                        Text(if (participant.isAdmin) "Remove admin" else "Make admin")
                    }
                    OutlinedButton(onClick = { pendingRemove = true }) {
                        Text("Remove from network")
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Done") }
        },
    )
    if (pendingRemove && manageNetwork != null && manageDispatch != null) {
        AlertDialog(
            onDismissRequest = { pendingRemove = false },
            title = { Text("Remove ${participant.displayName(state)}?") },
            text = { Text("This removes the device from the network's roster. They keep the network locally but won't be in this roster anymore.") },
            confirmButton = {
                TextButton(onClick = {
                    manageDispatch(JSONObject().put("type", "remove_participant")
                        .put("networkId", manageNetwork.id).put("npub", participant.npub))
                    pendingRemove = false
                    onDismiss()
                }) { Text("Remove") }
            },
            dismissButton = {
                TextButton(onClick = { pendingRemove = false }) { Text("Cancel") }
            },
        )
    }
}

@Composable
internal fun AddParticipantForm(network: NetworkState, dispatch: (JSONObject) -> Unit) {
    var deviceId by remember { mutableStateOf("") }
    var alias by remember { mutableStateOf("") }
    val trimmedDeviceId = deviceId.trim()
    val showError = trimmedDeviceId.isNotEmpty() && !isValidDeviceId(trimmedDeviceId)
    val canSubmit = trimmedDeviceId.isNotEmpty() && !showError
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedTextField(
            value = deviceId,
            onValueChange = { deviceId = it },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            label = { Text("Device ID") },
            isError = showError,
            supportingText = if (showError) {
                { Text("Not a valid device ID") }
            } else {
                null
            },
        )
        OutlinedTextField(
            value = alias,
            onValueChange = { alias = it },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            label = { Text("Name") },
        )
        Button(
            enabled = canSubmit,
            onClick = {
                dispatch(
                    JSONObject()
                        .put("type", "add_participant")
                        .put("networkId", network.id)
                        .put("npub", trimmedDeviceId)
                        .put("alias", alias.trim().ifBlank { JSONObject.NULL }),
                )
                deviceId = ""
                alias = ""
            },
        ) {
            Text("Add")
        }
    }
}

private val BECH32_BODY_CHARSET: Set<Char> = "qpzry9x8gf2tvdw0s3jn54khce6mua7l".toSet()

/** A valid device ID is a bech32-encoded npub: `npub1` + 58 bech32 chars. */
internal fun isValidDeviceId(value: String): Boolean {
    val trimmed = value.trim()
    if (trimmed.length != 63 || !trimmed.startsWith("npub1")) return false
    return trimmed.substring(5).all { it in BECH32_BODY_CHARSET }
}

@Composable
internal fun NearbyCard(state: AppState, dispatch: (JSONObject) -> Unit) {
    AppCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Nearby invites", style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            Button(onClick = {
                dispatch(
                    if (state.nearbyDiscoveryActive) {
                        NativeActions.stopNearbyDiscovery()
                    } else {
                        NativeActions.startNearbyDiscovery()
                    },
                )
            }) {
                Text(
                    if (state.nearbyDiscoveryActive) {
                        "Listening · ${formatRemaining(state.nearbyDiscoveryRemainingSecs)}"
                    } else {
                        "Look for nearby"
                    },
                )
            }
        }
        if (state.lanPeers.isEmpty()) {
            Text(
                if (state.nearbyDiscoveryActive) "No nearby invites yet" else "Tap above to look for nearby devices",
                color = Muted,
            )
        } else {
            state.lanPeers.forEach { peer -> LanPeerRow(peer, dispatch) }
        }
    }
}

private fun formatRemaining(seconds: Long): String {
    if (seconds <= 0) return "off"
    val minutes = seconds / 60
    if (minutes == 0L) return "${seconds}s"
    val secs = seconds % 60
    return if (secs == 0L) "${minutes}m" else "${minutes}m%02ds".format(secs)
}

@Composable
internal fun LanPeerRow(peer: LanPeerState, dispatch: (JSONObject) -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 8.dp)) {
        Column(Modifier.weight(1f)) {
            Text(peer.nodeName.ifBlank { peer.networkName }, fontWeight = FontWeight.SemiBold)
            Text(peer.lastSeenText, color = Muted, style = MaterialTheme.typography.bodySmall)
        }
        Button(onClick = { dispatch(NativeActions.importInvite(peer.invite)) }) {
            Text("Join")
        }
    }
}

@Composable
internal fun DeviceSettingsCard(state: AppState, dispatch: (JSONObject) -> Unit) {
    var nodeName by remember(state.nodeName) { mutableStateOf(state.nodeName) }
    var endpoint by remember(state.endpoint) { mutableStateOf(state.endpoint) }
    var tunnelIp by remember(state.tunnelIp) { mutableStateOf(state.tunnelIp) }
    var port by remember(state.listenPort) { mutableStateOf(state.listenPort.toString()) }
    AppCard {
        Text("This Device", style = MaterialTheme.typography.titleMedium)
        OutlinedTextField(nodeName, { nodeName = it }, Modifier.fillMaxWidth(), singleLine = true, label = { Text("Name") })
        OutlinedTextField(tunnelIp, { tunnelIp = it }, Modifier.fillMaxWidth(), singleLine = true, label = { Text("Tunnel IP") })
        OutlinedTextField(endpoint, { endpoint = it }, Modifier.fillMaxWidth(), singleLine = true, label = { Text("Endpoint") })
        OutlinedTextField(port, { port = it }, Modifier.fillMaxWidth(), singleLine = true, label = { Text("Port") })
        Row(verticalAlignment = Alignment.CenterVertically) {
            Checkbox(
                checked = state.autoconnect,
                onCheckedChange = { enabled -> dispatch(NativeActions.updateSettings("autoconnect" to enabled)) },
            )
            Text("Autoconnect")
        }
        Button(onClick = {
            dispatch(
                NativeActions.updateSettings(
                    "nodeName" to nodeName,
                    "endpoint" to endpoint,
                    "tunnelIp" to tunnelIp,
                    "listenPort" to port.toIntOrNull(),
                ),
            )
        }) {
            Text("Save")
        }
    }
}

@Composable
internal fun ExitNodeRow(
    title: String,
    subtitle: String,
    selected: Boolean,
    enabled: Boolean,
    onClick: () -> Unit,
) {
    val alpha = if (enabled) 1f else 0.5f
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(enabled = enabled, onClick = onClick)
                .padding(vertical = 6.dp)
                .alpha(alpha),
    ) {
        Text(
            if (selected) "●" else "○",
            color =
                if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline,
            style = MaterialTheme.typography.titleLarge,
        )
        Spacer(Modifier.width(10.dp))
        Column(modifier = Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyMedium)
            if (subtitle.isNotEmpty()) {
                Text(
                    subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.outline,
                    maxLines = 1,
                )
            }
        }
    }
}

@Composable
internal fun WireGuardSettingsCard(state: AppState, dispatch: (JSONObject) -> Unit) {
    var config by remember(state.wireguardExitConfig) { mutableStateOf(state.wireguardExitConfig) }

    AppCard {
        Text("WireGuard Upstream", style = MaterialTheme.typography.titleMedium)
        Text(
            "Paste a WireGuard config from an upstream VPN provider such as Mullvad or Proton VPN.",
            color = Muted,
            style = MaterialTheme.typography.bodySmall,
        )
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Enabled", modifier = Modifier.weight(1f))
            Switch(
                checked = state.wireguardExitEnabled,
                onCheckedChange = { enabled ->
                    dispatch(NativeActions.updateSettings("wireguardExitEnabled" to enabled))
                },
            )
        }
        OutlinedTextField(
            config,
            { config = it },
            Modifier.fillMaxWidth(),
            minLines = 8,
            label = { Text("Config") },
        )
        Button(onClick = {
            dispatch(
                NativeActions.updateSettings(
                    "wireguardExitConfig" to config,
                ),
            )
        }) {
            Text("Save")
        }
    }
}

@Composable
internal fun DiagnosticsCard(state: AppState) {
    AppCard {
        Text("Diagnostics", style = MaterialTheme.typography.titleMedium)
        Metric("Runtime", state.runtimeStatusDetail.ifBlank { state.platform })
        Metric("MagicDNS", state.magicDnsStatus)
        Metric("Version", state.appVersion)
        state.health.forEach { issue ->
            Text(issue.severity, color = Color(0xFFA16207), fontWeight = FontWeight.SemiBold)
            Text(issue.summary)
            if (issue.detail.isNotBlank()) Text(issue.detail, color = Muted)
        }
    }
}

@Composable
internal fun QrCode(invite: String, qrJson: (String) -> JSONObject) {
    val qr = remember(invite) { qrJson(invite) }
    val width = qr.optInt("width")
    val cells = qr.optJSONArray("cells")
    Canvas(
        modifier = Modifier
            .size(132.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(Color.White),
    ) {
        drawRect(Color.White)
        if (width <= 0 || cells == null) return@Canvas
        val quiet = 3
        val modules = width + quiet * 2
        val cell = size.minDimension / modules
        for (y in 0 until width) {
            for (x in 0 until width) {
                if (cells.optBoolean(y * width + x)) {
                    drawRect(
                        color = Color(0xFF111827),
                        topLeft = androidx.compose.ui.geometry.Offset((x + quiet) * cell, (y + quiet) * cell),
                        size = Size(cell, cell),
                    )
                }
            }
        }
    }
}

@Composable
internal fun AppCard(content: @Composable ColumnScope.() -> Unit) {
    Card(
        colors = CardDefaults.cardColors(containerColor = Color.White),
        shape = RoundedCornerShape(8.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
            content = content,
        )
    }
}

@Composable
internal fun EmptyCard(text: String) {
    AppCard { Text(text, color = Muted) }
}

@Composable
internal fun Notice(text: String) {
    AppCard { Text(text, color = Color(0xFF9A3412)) }
}

@Composable
internal fun CopyLine(value: String) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(value, modifier = Modifier.weight(1f), color = Muted, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
        CopyButton(value)
    }
}

@Composable
internal fun CopyButton(value: String) {
    val context = LocalContext.current
    TextButton(enabled = value.isNotBlank(), onClick = {
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        clipboard.setPrimaryClip(ClipData.newPlainText("Nostr VPN", value))
    }, modifier = Modifier.widthIn(min = 64.dp)) {
        Text("Copy", maxLines = 1, softWrap = false)
    }
}

@Composable
internal fun Metric(label: String, value: String) {
    Row {
        Text(label, color = Muted, modifier = Modifier.width(88.dp))
        Text(value.ifBlank { "-" }, modifier = Modifier.weight(1f), maxLines = 2, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
internal fun Dot(selected: Boolean) {
    Box(
        modifier = Modifier
            .size(if (selected) 12.dp else 8.dp)
            .clip(CircleShape)
            .background(if (selected) Ok else Color(0xFFD1D5DB)),
    )
}

@Composable
internal fun Pill(text: String, background: Color, foreground: Color) {
    Text(
        text = text,
        color = foreground,
        style = MaterialTheme.typography.labelSmall,
        maxLines = 1,
        softWrap = false,
        modifier = Modifier
            .clip(RoundedCornerShape(999.dp))
            .background(background)
            .padding(horizontal = 8.dp, vertical = 3.dp),
    )
}

internal val Accent = Color(0xFF7C3AED)
internal val Ok = Color(0xFF16A34A)
internal val Muted = Color(0xFF68717C)

private fun ParticipantState.isSelf(state: AppState): Boolean =
    (state.ownNpub.isNotBlank() && npub == state.ownNpub) || meshState == "local"

private fun ParticipantState.displayName(state: AppState): String {
    if (magicDnsName.isNotBlank()) return magicDnsName
    if (isSelf(state) && state.selfMagicDnsName.isNotBlank()) return state.selfMagicDnsName
    if (alias.isNotBlank()) return alias
    if (magicDnsAlias.isNotBlank()) return magicDnsAlias
    return npub.shortNpub()
}

private fun ParticipantState.subtitle(isSelf: Boolean): String {
    val ip = tunnelIp.substringBefore("/")
    return if (isSelf) {
        if (ip.isBlank()) "This device" else "This device - $ip"
    } else {
        ip
    }
}

private fun ParticipantState.statusLabel(appState: AppState): String {
    if (isSelf(appState)) return if (appState.vpnEnabled) "This device" else "Off"
    return when (state) {
        "local", "online", "present" -> "Online"
        "pending" -> "Connecting"
        "offline", "absent", "off" -> "Offline"
        else -> if (reachable) "Online" else "Unknown"
    }
}

private fun ParticipantState.detailStatusLabel(appState: AppState): String {
    if (isSelf(appState)) return statusLabel(appState)
    return when {
        statusText.isNotBlank() -> statusText
        else -> statusLabel(appState)
    }
}

private fun ParticipantState.isFipsRouted(state: AppState): Boolean =
    !isSelf(state) && reachable && fipsTransportAddr.isBlank()

private fun String.shortNpub(): String {
    if (isBlank()) return "Device"
    if (length <= 19) return this
    return "${take(12)}...${takeLast(6)}"
}
