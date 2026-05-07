package org.nostrvpn.app

import androidx.compose.foundation.Canvas
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
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
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
            item { Hero(state, network, dispatch) }
            if (state.error.isNotBlank()) {
                item { Notice(state.error) }
            }
            when (page) {
                Page.Devices -> devicesPage(state, network, dispatch) { showAddDevice = true }
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
    onAddDevice: () -> Unit,
) {
    item {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                "Devices",
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.weight(1f),
            )
            FilledIconButton(onClick = onAddDevice) {
                Text("+")
            }
        }
    }
    if (network == null) {
        item { EmptyCard("No network") }
        return
    }
    items(network.participants, key = { it.pubkeyHex.ifBlank { it.npub } }) { participant ->
        ParticipantRow(participant, isSelf = participant.npub == state.ownNpub && state.ownNpub.isNotBlank())
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
private fun AddDevicesDialog(
    state: AppState,
    network: NetworkState?,
    qrJson: (String) -> JSONObject,
    dispatch: (JSONObject) -> Unit,
    onDismiss: () -> Unit,
) {
    var inviteInput by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add Device") },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (state.activeNetworkInvite.isNotBlank()) {
                    QrCode(invite = state.activeNetworkInvite, qrJson = qrJson)
                    CopyLine(state.activeNetworkInvite)
                }
                OutlinedTextField(
                    value = inviteInput,
                    onValueChange = { inviteInput = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    label = { Text("Invite") },
                )
                Button(
                    enabled = inviteInput.isNotBlank(),
                    onClick = {
                        dispatch(NativeActions.importInvite(inviteInput.trim()))
                        inviteInput = ""
                    },
                ) {
                    Text("Import")
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

private fun androidx.compose.foundation.lazy.LazyListScope.exitNodesPage(
    state: AppState,
    network: NetworkState?,
    dispatch: (JSONObject) -> Unit,
) {
    item {
        AppCard {
            Text("Exit Node", style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(10.dp))
            Button(onClick = { dispatch(NativeActions.updateSettings("exitNode" to "")) }) {
                Text(if (state.exitNode.isBlank()) "Direct" else "Use Direct")
            }
            Spacer(Modifier.height(8.dp))
            network?.participants.orEmpty().filter { it.offersExitNode }.forEach { participant ->
                TextButton(onClick = {
                    dispatch(NativeActions.updateSettings("exitNode" to participant.npub))
                }) {
                    Text(participant.magicDnsName.ifBlank { participant.alias }, maxLines = 1)
                }
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
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.settingsPage(
    state: AppState,
    network: NetworkState?,
    dispatch: (JSONObject) -> Unit,
) {
    item { DeviceSettingsCard(state, dispatch) }
    item { NetworksCard(state, network, dispatch) }
    item { RelaysCard(state.relays, dispatch) }
    item { DiagnosticsCard(state) }
}
