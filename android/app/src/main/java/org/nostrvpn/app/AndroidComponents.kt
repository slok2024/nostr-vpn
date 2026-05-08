package org.nostrvpn.app

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
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

@Composable
internal fun Hero(state: AppState, dispatch: (JSONObject) -> Unit) {
    AppCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(
                modifier = Modifier
                    .size(10.dp)
                    .clip(CircleShape)
                    .background(if (state.vpnActive) Ok else Color(0xFF9CA3AF)),
            )
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    localDeviceTitle(state),
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(state.vpnStatus, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(
                    peerSummary(state),
                    color = Muted,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            Switch(
                checked = state.vpnEnabled,
                enabled = state.vpnControlSupported,
                onCheckedChange = { enabled ->
                    dispatch(
                        if (enabled) {
                            NativeActions.connectVpn()
                        } else {
                            NativeActions.disconnectVpn()
                        },
                    )
                },
            )
        }
    }
}

private fun localDeviceTitle(state: AppState): String =
    state.selfMagicDnsName
        .ifBlank { state.nodeName }
        .ifBlank { "This device" }

private fun peerSummary(state: AppState): String {
    if (state.expectedPeerCount == 0L) {
        return "No peers yet"
    }
    return "${state.connectedPeerCount} of ${state.expectedPeerCount} connected"
}

@Composable
internal fun ParticipantRow(participant: ParticipantState) {
    AppCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Dot(selected = participant.reachable)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        participant.magicDnsName.ifBlank { participant.alias },
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (participant.isAdmin) Pill("Admin", Color(0xFFF5F3FF), Accent)
                    if (participant.offersExitNode) Pill("Exit", Color(0xFFFFF7ED), Color(0xFFA16207))
                }
                Text(participant.tunnelIp, color = Muted, maxLines = 1)
                Text(participant.statusText, color = Muted, style = MaterialTheme.typography.bodySmall)
            }
            CopyButton(participant.npub)
        }
    }
}

@Composable
internal fun AddParticipantCard(network: NetworkState, dispatch: (JSONObject) -> Unit) {
    AppCard {
        Text("Add Device", style = MaterialTheme.typography.titleMedium)
        AddParticipantForm(network, dispatch)
    }
}

@Composable
internal fun AddParticipantForm(network: NetworkState, dispatch: (JSONObject) -> Unit) {
    var npub by remember { mutableStateOf("") }
    var alias by remember { mutableStateOf("") }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedTextField(
            value = npub,
            onValueChange = { npub = it },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            label = { Text("npub") },
        )
        OutlinedTextField(
            value = alias,
            onValueChange = { alias = it },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            label = { Text("Name") },
        )
        Button(
            enabled = npub.isNotBlank(),
            onClick = {
                dispatch(
                    JSONObject()
                        .put("type", "add_participant")
                        .put("networkId", network.id)
                        .put("npub", npub.trim())
                        .put("alias", alias.trim().ifBlank { JSONObject.NULL }),
                )
                npub = ""
                alias = ""
            },
        ) {
            Text("Add")
        }
    }
}

@Composable
internal fun NearbyCard(state: AppState, dispatch: (JSONObject) -> Unit) {
    AppCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Nearby Devices", style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            Button(onClick = {
                dispatch(
                    if (state.lanPairingActive) {
                        NativeActions.stopLanPairing()
                    } else {
                        NativeActions.startLanPairing()
                    },
                )
            }) {
                Text(if (state.lanPairingActive) "${state.lanPairingRemainingSecs}s" else "Pair")
            }
        }
        if (state.lanPeers.isEmpty()) {
            Text("None", color = Muted)
        } else {
            state.lanPeers.forEach { peer -> LanPeerRow(peer, dispatch) }
        }
    }
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
internal fun NetworksCard(state: AppState, network: NetworkState?, dispatch: (JSONObject) -> Unit) {
    var newNetwork by remember { mutableStateOf("") }
    AppCard {
        Text("Networks", style = MaterialTheme.typography.titleMedium)
        network?.let {
            Text(it.networkId, color = Muted, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(
                    checked = it.joinRequestsEnabled,
                    onCheckedChange = { enabled ->
                        dispatch(
                            JSONObject()
                                .put("type", "set_network_join_requests_enabled")
                                .put("networkId", it.id)
                                .put("enabled", enabled),
                        )
                    },
                    enabled = it.localIsAdmin,
                )
                Text("Join requests")
            }
        }
        state.networks.filter { !it.enabled }.forEach { saved ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(saved.name.ifBlank { "Private network" }, fontWeight = FontWeight.SemiBold)
                    Text("${saved.onlineCount} of ${saved.expectedCount} connected", color = Muted)
                }
                Button(onClick = { dispatch(NativeActions.setNetworkEnabled(saved.id, true)) }) {
                    Text("Activate")
                }
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            OutlinedTextField(
                value = newNetwork,
                onValueChange = { newNetwork = it },
                modifier = Modifier.weight(1f),
                singleLine = true,
                label = { Text("New network") },
            )
            Spacer(Modifier.width(8.dp))
            Button(enabled = newNetwork.isNotBlank(), onClick = {
                dispatch(NativeActions.addNetwork(newNetwork.trim()))
                newNetwork = ""
            }) {
                Text("Add")
            }
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
    }) {
        Text("Copy")
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
        modifier = Modifier
            .clip(RoundedCornerShape(999.dp))
            .background(background)
            .padding(horizontal = 8.dp, vertical = 3.dp),
    )
}

internal val Accent = Color(0xFF7C3AED)
internal val Ok = Color(0xFF16A34A)
internal val Muted = Color(0xFF68717C)
