package org.nostrvpn.app

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
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
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.HorizontalDivider
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
    scanQr: () -> Unit,
    dispatch: (JSONObject) -> Unit,
) {
    var page by remember { mutableStateOf(Page.Devices) }
    var showAddDevice by remember { mutableStateOf(false) }
    var showAddNetwork by remember { mutableStateOf(false) }
    var pendingNetworkRemoval by remember { mutableStateOf<NetworkState?>(null) }
    var shownNetworkId by remember { mutableStateOf<String?>(null) }
    val activeNetwork = state.activeNetwork
    val network = state.networks.firstOrNull { it.id == shownNetworkId } ?: activeNetwork
    Scaffold(
        containerColor = Color(0xFFF6F7F8),
        topBar = {
            MobileTopBar(
                state = state,
                network = network,
                dispatch = dispatch,
                onSelectNetwork = { shownNetworkId = it },
                onAddNetwork = { showAddNetwork = true },
            )
        },
        bottomBar = {
            // Bottom nav only makes sense once a network exists. With no
            // network the only meaningful action is Add Network, which we
            // surface as the entire screen body.
            if (network != null) {
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
            if (network == null) {
                addNetworkBody(state, scanQr, dispatch)
            } else {
                when (page) {
                    Page.Devices -> devicesPage(
                        state,
                        network,
                        scanQr,
                        dispatch,
                        onAddDevice = { showAddDevice = true },
                        onDeleteNetwork = { pendingNetworkRemoval = network },
                    )
                    Page.ExitNodes -> exitNodesPage(state, network, dispatch)
                    Page.Settings -> settingsPage(state, network, dispatch)
                }
            }
        }
    }
    if (showAddDevice && network != null) {
        AddDevicesDialog(
            state = state,
            network = network,
            qrJson = qrJson,
            dispatch = dispatch,
            onDismiss = { showAddDevice = false },
        )
    }
    if (showAddNetwork) {
        AddNetworkDialog(
            state = state,
            scanQr = scanQr,
            dispatch = dispatch,
            onDismiss = { showAddNetwork = false },
            onCreated = {
                // Land on the new network's Devices view: dismiss the
                // dialog and reset the nav to Devices in case the user
                // was on Exit Nodes or Settings when they tapped Add
                // network.
                showAddNetwork = false
                page = Page.Devices
            },
        )
    }
    pendingNetworkRemoval?.let { target ->
        AlertDialog(
            onDismissRequest = { pendingNetworkRemoval = null },
            title = { Text("Delete ${target.name.ifBlank { "network" }}?") },
            text = { Text("Removes the network from this device.") },
            confirmButton = {
                TextButton(onClick = {
                    dispatch(NativeActions.removeNetwork(target.id))
                    pendingNetworkRemoval = null
                }) {
                    Text("Delete", color = Color(0xFFB00020))
                }
            },
            dismissButton = {
                TextButton(onClick = { pendingNetworkRemoval = null }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun MobileTopBar(
    state: AppState,
    network: NetworkState?,
    dispatch: (JSONObject) -> Unit,
    onSelectNetwork: (String) -> Unit,
    onAddNetwork: () -> Unit,
) {
    var menuExpanded by remember { mutableStateOf(false) }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(Color.White)
            .padding(horizontal = 18.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.weight(1f)) {
            // Single dropdown: switch network OR add a new one. Whole row
            // tappable so it reads as a "current network: name" affordance.
            Row(
                modifier = Modifier.clickable { menuExpanded = true },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (state.networks.size > 1) {
                    NetworkStatusDot(network)
                    Spacer(Modifier.width(8.dp))
                }
                Text(
                    networkTitle(network),
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Spacer(Modifier.width(6.dp))
                Text("▾", color = Muted)
            }
            DropdownMenu(expanded = menuExpanded, onDismissRequest = { menuExpanded = false }) {
                state.networks.forEach { saved ->
                    DropdownMenuItem(
                        text = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                if (state.networks.size > 1) {
                                    NetworkStatusDot(saved)
                                    Spacer(Modifier.width(8.dp))
                                }
                                Text(networkTitle(saved))
                            }
                        },
                        onClick = {
                            menuExpanded = false
                            onSelectNetwork(saved.id)
                        },
                    )
                }
                if (state.networks.isNotEmpty()) {
                    HorizontalDivider()
                }
                DropdownMenuItem(
                    text = { Text("Add network") },
                    onClick = {
                        menuExpanded = false
                        onAddNetwork()
                    },
                )
            }
        }
        Switch(
            checked = state.vpnEnabled,
            enabled = state.vpnControlSupported && network != null,
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
private fun NetworkStatusDot(network: NetworkState?) {
    Canvas(modifier = Modifier.size(8.dp)) {
        drawCircle(if (network?.enabled == true) Color(0xFF16A34A) else Color(0xFF9CA3AF))
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
    network: NetworkState,
    @Suppress("UNUSED_PARAMETER") scanQr: () -> Unit,
    dispatch: (JSONObject) -> Unit,
    onAddDevice: () -> Unit,
    onDeleteNetwork: () -> Unit,
) {
    if (network.localIsAdmin) {
        item {
            Button(
                onClick = onAddDevice,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Add device")
            }
        }
    }
    items(sortedParticipants(network.participants, state), key = { it.pubkeyHex.ifBlank { it.npub } }) { participant ->
        ParticipantRow(state, participant, network = network, dispatch = dispatch)
    }
    items(network.inboundJoinRequests, key = { it.requesterNpub }) { request ->
        AppCard {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(request.requesterNodeName.ifBlank { "Join request" }, fontWeight = FontWeight.SemiBold)
                    Text(request.requestedAtText, color = Muted, style = MaterialTheme.typography.bodySmall)
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(onClick = {
                        dispatch(NativeActions.rejectJoinRequest(network.id, request.requesterNpub))
                    }) {
                        Text("Reject", color = Color(0xFFB00020))
                    }
                    Button(onClick = {
                        dispatch(NativeActions.acceptJoinRequest(network.id, request.requesterNpub))
                    }) {
                        Text("Accept")
                    }
                }
            }
        }
    }
    item {
        OutlinedButton(
            onClick = onDeleteNetwork,
            modifier = Modifier
                .fillMaxWidth()
                .padding(top = 8.dp),
        ) {
            Text("Delete network", color = Color(0xFFB00020))
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
    if (magicDnsName.isNotBlank()) return magicDnsName
    if (isSelf(state) && state.selfMagicDnsName.isNotBlank()) return state.selfMagicDnsName
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
private fun NetworkSetupCard(
    state: AppState,
    scanQr: () -> Unit,
    dispatch: (JSONObject) -> Unit,
    onCreated: (() -> Unit)? = null,
) {
    var networkName by remember { mutableStateOf("My Network") }
    var inviteInput by remember { mutableStateOf("") }
    val context = androidx.compose.ui.platform.LocalContext.current
    val clipboard = remember(context) {
        context.getSystemService(android.content.ClipboardManager::class.java)
    }

    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SetupChoiceCard("Create Network", Color(0xFF16A34A)) {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(
                    value = networkName,
                    onValueChange = { networkName = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    label = { Text("Network name") },
                )
                Button(
                    onClick = {
                        dispatch(NativeActions.addNetwork(networkName.trim().ifBlank { "My Network" }))
                        networkName = "My Network"
                        onCreated?.invoke()
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text("Create")
                }
            }
        }

        SetupChoiceCard("Join Network", Color(0xFF2563EB)) {
            OutlinedTextField(
                value = inviteInput,
                onValueChange = { newValue ->
                    inviteInput = newValue
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
                OutlinedButton(
                    onClick = {
                        val item = clipboard?.primaryClip?.getItemAt(0)?.coerceToText(context)
                        item?.toString()?.let { inviteInput = it.trim() }
                    },
                    modifier = Modifier.weight(1f),
                ) {
                    Text("Paste")
                }
                OutlinedButton(
                    onClick = scanQr,
                    modifier = Modifier.weight(1f),
                ) {
                    Text("Scan")
                }
            }

            // Manual join: hand off admin device id + mesh network id directly
            // to the core's `manual_add_network` action. Both sides add each
            // other's Device ID out-of-band; no join request is queued here.
            var manualExpanded by remember { mutableStateOf(false) }
            var manualAdminId by remember { mutableStateOf("") }
            var manualNetworkId by remember { mutableStateOf("") }
            TextButton(onClick = { manualExpanded = !manualExpanded }) {
                Text(if (manualExpanded) "Add manually ▴" else "Add manually ▾")
            }
            if (manualExpanded) {
                val adminTrim = manualAdminId.trim()
                val meshTrim = manualNetworkId.trim()
                val adminInvalid = adminTrim.isNotEmpty() && !isValidDeviceId(adminTrim)
                val canSubmit = adminTrim.isNotEmpty() && meshTrim.isNotEmpty() && !adminInvalid
                Text(
                    "Both sides have to add each other. Get the admin's Device ID and the network ID from them, then have the admin add your Device ID on their Add device page.",
                    style = MaterialTheme.typography.bodySmall,
                    color = Muted,
                )
                OutlinedTextField(
                    value = manualAdminId,
                    onValueChange = { manualAdminId = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    label = { Text("Admin Device ID") },
                    isError = adminInvalid,
                    supportingText = if (adminInvalid) {
                        { Text("Not a valid device ID") }
                    } else {
                        null
                    },
                )
                OutlinedTextField(
                    value = manualNetworkId,
                    onValueChange = { manualNetworkId = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    label = { Text("Network ID") },
                )
                Button(
                    enabled = canSubmit,
                    onClick = {
                        dispatch(NativeActions.manualAddNetwork(adminTrim, meshTrim))
                        manualAdminId = ""
                        manualNetworkId = ""
                        manualExpanded = false
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text("Add")
                }
            }
        }
    }
}

@Composable
private fun SetupChoiceCard(
    title: String,
    accent: Color,
    content: @Composable ColumnScope.() -> Unit,
) {
    AppCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Canvas(Modifier.size(10.dp)) {
                drawCircle(accent)
            }
            Spacer(Modifier.width(8.dp))
            Text(title, style = MaterialTheme.typography.titleMedium, color = accent)
        }
        content()
    }
}

// LazyListScope wrapper for the Add Network body, used as the entire
// screen content when there is no active network. Mirrors the in-dialog
// content we show when the user picks "Add network" from the header
// switcher with an existing network already in place.
private fun androidx.compose.foundation.lazy.LazyListScope.addNetworkBody(
    state: AppState,
    scanQr: () -> Unit,
    dispatch: (JSONObject) -> Unit,
) {
    item { NetworkSetupCard(state, scanQr, dispatch) }
    item { NearbyCard(state, dispatch) }
}

@Composable
private fun AddNetworkDialog(
    state: AppState,
    scanQr: () -> Unit,
    dispatch: (JSONObject) -> Unit,
    onDismiss: () -> Unit,
    onCreated: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add Network") },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                NetworkSetupCard(state, scanQr, dispatch, onCreated = onCreated)
                NearbyCard(state, dispatch)
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Done") }
        },
    )
}

/// Admin-only sheet for adding a device to YOUR network. Two paths:
/// share an invite, or directly add by Device ID. Joining someone
/// else's network and finding nearby networks belong to Add Network,
/// not here.
@Composable
private fun AddDevicesDialog(
    state: AppState,
    network: NetworkState,
    qrJson: (String) -> JSONObject,
    dispatch: (JSONObject) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add Device") },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (network.enabled) {
                    Text("Invite to my network", style = MaterialTheme.typography.titleMedium)
                    Text(
                        "Share this code with another device to give it access to your network.",
                        style = MaterialTheme.typography.bodySmall,
                        color = Muted,
                    )
                    if (state.activeNetworkInvite.isNotBlank()) {
                        QrCode(invite = state.activeNetworkInvite, qrJson = qrJson)
                        CopyLine(state.activeNetworkInvite)
                    }
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Switch(
                            checked = network.joinRequestsEnabled,
                            onCheckedChange = { enabled ->
                                dispatch(NativeActions.setJoinRequests(network.id, enabled))
                            },
                        )
                        Spacer(Modifier.width(8.dp))
                        Text("Allow join requests")
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
                }

                Spacer(modifier = Modifier.height(8.dp))
                Text("For manual join", style = MaterialTheme.typography.titleMedium)
                Text(
                    "If the other device can't scan or paste an invite, share these two values. They'll enter them under Join Network → Add manually. You still need to add their Device ID below.",
                    style = MaterialTheme.typography.bodySmall,
                    color = Muted,
                )
                Text("Your Device ID", style = MaterialTheme.typography.bodySmall, color = Muted)
                CopyLine(state.ownNpub)
                Text("Network ID", style = MaterialTheme.typography.bodySmall, color = Muted)
                CopyLine(network.networkId)

                Spacer(modifier = Modifier.height(8.dp))
                Text("Add by Device ID", style = MaterialTheme.typography.titleMedium)
                Text(
                    "Manual pairing: enter the other device's Device ID. They also need to add yours.",
                    style = MaterialTheme.typography.bodySmall,
                    color = Muted,
                )
                AddParticipantForm(network, dispatch)
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
                val name = network?.name?.ifBlank { null } ?: "this network"
                Text("Offer exit node in $name")
            }
        }
    }
    item { WireGuardSettingsCard(state, dispatch) }
}

private fun androidx.compose.foundation.lazy.LazyListScope.settingsPage(
    state: AppState,
    @Suppress("UNUSED_PARAMETER") network: NetworkState?,
    dispatch: (JSONObject) -> Unit,
) {
    item { DeviceSettingsCard(state, dispatch) }
    item {
        AppCard {
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
    item { DiagnosticsCard(state) }
}
