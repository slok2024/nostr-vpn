package org.nostrvpn.app

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.sin
import org.json.JSONObject
import org.nostrvpn.app.core.AppState
import org.nostrvpn.app.core.NativeActions
import org.nostrvpn.app.core.NetworkState
import org.nostrvpn.app.core.ParticipantState
import org.nostrvpn.app.core.activeNetwork

private enum class Page(val title: String) {
    Devices("Devices"),
    ExitNodes("Exit Nodes"),
    Settings("Settings"),
}

@Composable
internal fun NostrVpnTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = lightColorScheme(
            primary = Color(0xFF8B5CF6),
            secondary = Color(0xFF22D3EE),
            background = Color(0xFFF6F7F8),
            surface = Color.White,
            onPrimary = Color.White,
            onSecondary = Color(0xFF111827),
            onBackground = Color(0xFF17202A),
            onSurface = Color(0xFF17202A),
        ),
        content = content,
    )
}

@Composable
internal fun NostrVpnApp(
    state: AppState,
    qrJson: (String) -> JSONObject,
    dispatch: (JSONObject) -> Unit,
) {
    var page by remember { mutableStateOf(Page.Devices) }
    var showAddDevice by remember { mutableStateOf(false) }
    val network = state.activeNetwork
    Scaffold(
        containerColor = Color(0xFFF6F7F8),
        topBar = {
            MobileTopBar(
                title = page.title,
                state = state,
                network = network,
                dispatch = dispatch,
                onAddDevice = if (page == Page.Devices) {
                    { showAddDevice = true }
                } else {
                    null
                },
            )
        },
        bottomBar = {
            NavigationBar(containerColor = Color.White) {
                Page.entries.forEach { item ->
                    NavigationBarItem(
                        selected = page == item,
                        onClick = { page = item },
                        icon = { NavIcon(item, selected = page == item) },
                        label = { Text(item.title) },
                    )
                }
            }
        },
    ) { padding ->
        LazyColumn(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
            contentPadding = PaddingValues(18.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (state.error.isNotBlank()) {
                item { Notice(state.error) }
            }
            when (page) {
                Page.Devices -> devicesPage(state, network, dispatch)
                Page.ExitNodes -> exitNodesPage(state, network, dispatch)
                Page.Settings -> settingsPage(state, network, dispatch)
            }
        }
    }
    if (showAddDevice) {
        AddDevicesDialog(
            state = state,
            network = network,
            qrJson = qrJson,
            dispatch = dispatch,
            onDismiss = { showAddDevice = false },
        )
    }
}

@Composable
private fun MobileTopBar(
    title: String,
    state: AppState,
    network: NetworkState?,
    dispatch: (JSONObject) -> Unit,
    onAddDevice: (() -> Unit)?,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(Color.White)
            .padding(horizontal = 18.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
            Text(networkTitle(network), color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        if (onAddDevice != null) {
            FilledIconButton(onClick = onAddDevice) {
                PlusIcon()
            }
            Spacer(Modifier.width(10.dp))
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

@Composable
private fun PlusIcon() {
    Canvas(Modifier.size(18.dp)) {
        val strokeWidth = 2.6.dp.toPx()
        val center = size.width / 2f
        drawLine(
            Color.White,
            Offset(center, 2.dp.toPx()),
            Offset(center, size.height - 2.dp.toPx()),
            strokeWidth = strokeWidth,
            cap = StrokeCap.Round,
        )
        drawLine(
            Color.White,
            Offset(2.dp.toPx(), center),
            Offset(size.width - 2.dp.toPx(), center),
            strokeWidth = strokeWidth,
            cap = StrokeCap.Round,
        )
    }
}

@Composable
private fun NavIcon(page: Page, selected: Boolean) {
    val color = if (selected) Accent else Color(0xFF17202A)
    Canvas(modifier = Modifier.size(28.dp)) {
        val strokeWidth = 2.6.dp.toPx()
        val stroke = Stroke(width = strokeWidth, cap = StrokeCap.Round)
        when (page) {
            Page.Devices -> {
                val radius = 3.6.dp.toPx()
                val gap = 5.4.dp.toPx()
                val center = Offset(size.width / 2f, size.height / 2f)
                for (x in listOf(-gap, gap)) {
                    for (y in listOf(-gap, gap)) {
                        drawCircle(color, radius, Offset(center.x + x, center.y + y))
                    }
                }
            }
            Page.ExitNodes -> {
                val top = Offset(size.width / 2f, 5.5.dp.toPx())
                val joint = Offset(size.width / 2f, 13.dp.toPx())
                val left = Offset(8.dp.toPx(), 22.dp.toPx())
                val right = Offset(20.dp.toPx(), 22.dp.toPx())
                drawLine(color, top, joint, strokeWidth = strokeWidth, cap = StrokeCap.Round)
                drawLine(color, joint, left, strokeWidth = strokeWidth, cap = StrokeCap.Round)
                drawLine(color, joint, right, strokeWidth = strokeWidth, cap = StrokeCap.Round)
                drawCircle(color, 2.7.dp.toPx(), top)
                drawCircle(color, 2.7.dp.toPx(), left)
                drawCircle(color, 2.7.dp.toPx(), right)
            }
            Page.Settings -> {
                val center = Offset(size.width / 2f, size.height / 2f)
                val inner = 8.6.dp.toPx()
                val outer = 12.1.dp.toPx()
                repeat(8) { index ->
                    val angle = index * PI.toFloat() / 4f
                    val start = Offset(center.x + cos(angle) * inner, center.y + sin(angle) * inner)
                    val end = Offset(center.x + cos(angle) * outer, center.y + sin(angle) * outer)
                    drawLine(color, start, end, strokeWidth = strokeWidth, cap = StrokeCap.Round)
                }
                drawCircle(color, 6.7.dp.toPx(), center, style = stroke)
                drawCircle(color, 2.4.dp.toPx(), center)
            }
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.devicesPage(
    state: AppState,
    network: NetworkState?,
    dispatch: (JSONObject) -> Unit,
) {
    if (network == null) {
        item { EmptyCard("No network") }
        return
    }
    item { DeviceListHeader(state, network) }
    items(sortedParticipants(network.participants, state), key = { it.pubkeyHex.ifBlank { it.npub } }) { participant ->
        ParticipantRow(state, participant)
    }
    items(network.inboundJoinRequests, key = { it.requesterNpub }) { request ->
        AppCard {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(request.requesterNodeName.ifBlank { "Join request" }, fontWeight = FontWeight.SemiBold)
                    Text(request.requestedAtText, color = Muted, style = MaterialTheme.typography.bodySmall)
                }
                Button(onClick = {
                    dispatch(
                        JSONObject()
                            .put("type", "accept_join_request")
                            .put("networkId", network.id)
                            .put("requesterNpub", request.requesterNpub),
                    )
                }) {
                    Text("Accept")
                }
            }
        }
    }
}

@Composable
private fun DeviceListHeader(
    state: AppState,
    network: NetworkState,
) {
    Column {
        Text(networkTitle(network), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
        Text(deviceCountText(network), color = Muted, style = MaterialTheme.typography.bodySmall)
    }
}

private fun sortedParticipants(participants: List<ParticipantState>, state: AppState): List<ParticipantState> =
    participants.sortedWith(
        compareByDescending<ParticipantState> { it.isSelf(state) }
            .thenByDescending { it.reachable }
            .thenBy(String.CASE_INSENSITIVE_ORDER) { it.deviceName(state) },
    )

private fun ParticipantState.isSelf(state: AppState): Boolean =
    (state.ownNpub.isNotBlank() && npub == state.ownNpub) || meshState == "local"

private fun ParticipantState.deviceName(state: AppState): String {
    if (isSelf(state) && state.nodeName.isNotBlank()) return state.nodeName
    if (magicDnsName.isNotBlank()) return magicDnsName
    if (alias.isNotBlank()) return alias
    if (magicDnsAlias.isNotBlank()) return magicDnsAlias
    if (npub.length <= 19) return npub.ifBlank { "Device" }
    return "${npub.take(12)}...${npub.takeLast(6)}"
}

private fun deviceCountText(network: NetworkState): String {
    if (network.expectedCount == 0L) return "This device"
    val word = if (network.expectedCount == 1L) "device" else "devices"
    return "${network.onlineCount} online - ${network.expectedCount} $word"
}

@Composable
private fun AddDevicesDialog(
    state: AppState,
    network: NetworkState?,
    qrJson: (String) -> JSONObject,
    dispatch: (JSONObject) -> Unit,
    onDismiss: () -> Unit,
) {
    var inviteInput by remember { mutableStateOf("") }
    val context = androidx.compose.ui.platform.LocalContext.current
    val clipboard = remember(context) {
        context.getSystemService(android.content.ClipboardManager::class.java)
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add Device") },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("Invite Devices", style = MaterialTheme.typography.titleMedium)
                if (state.activeNetworkInvite.isNotBlank()) {
                    QrCode(invite = state.activeNetworkInvite, qrJson = qrJson)
                    Text("Your invite", color = Muted, style = MaterialTheme.typography.bodySmall)
                    CopyLine(state.activeNetworkInvite)
                }
                Button(onClick = {
                    dispatch(
                        if (state.inviteBroadcastActive) {
                            NativeActions.stopInviteBroadcast()
                        } else {
                            NativeActions.startInviteBroadcast()
                        },
                    )
                }) {
                    Text(
                        if (state.inviteBroadcastActive) {
                            "Broadcasting · ${formatDialogRemaining(state.inviteBroadcastRemainingSecs)}"
                        } else {
                            "Broadcast invite"
                        },
                    )
                }

                Spacer(modifier = Modifier.height(8.dp))
                Text("Join Network", style = MaterialTheme.typography.titleMedium)
                Text("Paste invite code", color = Muted, style = MaterialTheme.typography.bodySmall)
                OutlinedTextField(
                    value = inviteInput,
                    onValueChange = { newValue ->
                        inviteInput = newValue
                        // Auto-import on a recognisable invite — saves the
                        // user a tap. Clearing prevents re-firing.
                        val trimmed = newValue.trim()
                        if (trimmed.startsWith("nvpn://invite/", ignoreCase = true)) {
                            dispatch(NativeActions.importInvite(trimmed))
                            inviteInput = ""
                        }
                    },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    label = { Text("nvpn://invite/…") },
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(onClick = {
                        val item = clipboard?.primaryClip?.getItemAt(0)?.coerceToText(context)
                        item?.toString()?.let { inviteInput = it.trim() }
                    }) {
                        Text("Paste")
                    }
                    Button(
                        enabled = inviteInput.isNotBlank(),
                        onClick = {
                            dispatch(NativeActions.importInvite(inviteInput.trim()))
                            inviteInput = ""
                        },
                    ) {
                        Text("Import")
                    }
                }
                if (network?.outboundJoinRequest == true) {
                    Pill("Join requested", Color(0xFFFFF7ED), Color(0xFF9A3412))
                } else if (!network?.inviteInviterNpub.isNullOrBlank()) {
                    Button(onClick = {
                        dispatch(JSONObject().put("type", "request_network_join").put("networkId", network!!.id))
                    }) {
                        Text("Request Access")
                    }
                }
                if (network?.localIsAdmin == true) {
                    Text("Manual", style = MaterialTheme.typography.titleMedium)
                    AddParticipantForm(network, dispatch)
                }
                NearbyCard(state, dispatch)
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) {
                Text("Done")
            }
        },
    )
}

private fun formatDialogRemaining(seconds: Long): String {
    if (seconds <= 0) return "off"
    val minutes = seconds / 60
    if (minutes == 0L) return "${seconds}s"
    val secs = seconds % 60
    return if (secs == 0L) "${minutes}m" else "${minutes}m%02ds".format(secs)
}

private fun androidx.compose.foundation.lazy.LazyListScope.exitNodesPage(
    state: AppState,
    network: NetworkState?,
    dispatch: (JSONObject) -> Unit,
) {
    item {
        AppCard {
            Text("Exit Node", style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(10.dp))

            // The daemon clears the *other* side automatically when
            // both would otherwise be set (see
            // `settings_patch_enforces_exit_node_mutual_exclusion`),
            // so the WG and peer rows only push the field they own.
            // "Direct" still needs to flip both explicitly — neither
            // is a conflict the daemon resolves.
            val directSelected = !state.wireguardExitEnabled && state.exitNode.isBlank()
            ExitNodeRow(
                title = "Direct",
                subtitle = "No exit node — your own internet",
                selected = directSelected,
                enabled = true,
                onClick = {
                    dispatch(
                        NativeActions.updateSettings(
                            "exitNode" to "",
                            "wireguardExitEnabled" to false,
                        ),
                    )
                },
            )

            val wgSubtitle =
                if (!state.wireguardExitConfigured) {
                    "No WireGuard config saved yet"
                } else if (state.wireguardExitEndpoint.isBlank()) {
                    "Configured"
                } else {
                    state.wireguardExitEndpoint
                }
            ExitNodeRow(
                title = "WireGuard upstream",
                subtitle = wgSubtitle,
                selected = state.wireguardExitEnabled,
                enabled = state.wireguardExitConfigured,
                onClick = {
                    dispatch(NativeActions.updateSettings("wireguardExitEnabled" to true))
                },
            )

            network?.participants.orEmpty().filter { it.offersExitNode }.forEach { participant ->
                ExitNodeRow(
                    title = participant.magicDnsName.ifBlank { participant.alias },
                    subtitle = participant.npub,
                    selected = !state.wireguardExitEnabled && state.exitNode == participant.npub,
                    enabled = true,
                    onClick = {
                        dispatch(NativeActions.updateSettings("exitNode" to participant.npub))
                    },
                )
            }
        }
    }
    item {
        AppCard {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(
                    checked = state.advertiseExitNode,
                    onCheckedChange = { enabled ->
                        dispatch(NativeActions.updateSettings("advertiseExitNode" to enabled))
                    },
                )
                Text("Offer exit node")
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(
                    checked = state.exitNodeLeakProtection,
                    onCheckedChange = { enabled ->
                        dispatch(NativeActions.updateSettings("exitNodeLeakProtection" to enabled))
                    },
                )
                Text("Block internet if exit node disconnects")
            }
        }
    }
    item { WireGuardSettingsCard(state, dispatch) }
}

private fun androidx.compose.foundation.lazy.LazyListScope.settingsPage(
    state: AppState,
    network: NetworkState?,
    dispatch: (JSONObject) -> Unit,
) {
    item { DeviceSettingsCard(state, dispatch) }
    item { NetworksCard(state, network, dispatch) }
    item { DiagnosticsCard(state) }
}
