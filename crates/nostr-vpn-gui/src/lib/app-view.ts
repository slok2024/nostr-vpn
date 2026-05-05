import type { HealthIssue, NetworkView, ParticipantView, PeerState, PresenceState, UiState } from './types'

export const serviceMetaText = (state: UiState) => {
  if (!state.serviceInstalled) {
    return 'Not installed'
  }
  if (state.serviceDisabled) {
    return 'Installed but disabled'
  }
  if (state.serviceRunning) {
    return 'Installed and running'
  }
  return 'Installed'
}

export const serviceLifecycleBadgeText = (state: UiState) => {
  if (state.serviceDisabled) {
    return 'Disabled'
  }
  return state.serviceRunning ? 'Running' : 'Not running'
}

export const serviceLifecycleBadgeClass = (state: UiState) => {
  if (state.serviceDisabled) {
    return 'warn'
  }
  return state.serviceRunning ? 'ok' : 'muted'
}

export const participantBadgeClass = (state: PeerState | PresenceState) => {
  if (state === 'online' || state === 'present') {
    return 'ok'
  }
  if (state === 'pending') {
    return 'warn'
  }
  if (state === 'offline' || state === 'absent') {
    return 'bad'
  }
  return 'muted'
}

export const peerStatePriority = (state: PeerState) => {
  switch (state) {
    case 'online':
      return 0
    case 'pending':
      return 1
    case 'offline':
      return 2
    case 'checking':
      return 3
    case 'unknown':
      return 4
    case 'local':
    default:
      return 5
  }
}

export const healthBadgeClass = (severity: HealthIssue['severity']) => {
  switch (severity) {
    case 'critical':
      return 'bad'
    case 'warning':
      return 'warn'
    case 'info':
    default:
      return 'muted'
  }
}

export const healthSummaryText = (state: UiState) => {
  if (state.health.length === 0) {
    return 'No active warnings'
  }
  const critical = state.health.filter((issue) => issue.severity === 'critical').length
  const warning = state.health.filter((issue) => issue.severity === 'warning').length
  if (critical > 0) {
    return `${critical} critical`
  }
  if (warning > 0) {
    return `${warning} warning${warning === 1 ? '' : 's'}`
  }
  return `${state.health.length} info`
}

export const participantTransportBadgeText = (participant: ParticipantView) => {
  switch (participant.state) {
    case 'local':
      return 'FIPS self'
    case 'online':
      return 'FIPS reachable'
    case 'pending':
      return 'FIPS pending'
    case 'offline':
      return 'FIPS offline'
    default:
      return 'FIPS unknown'
  }
}

export const participantPresenceBadgeText = (participant: ParticipantView) => {
  switch (participant.presenceState) {
    case 'local':
      return 'Mesh self'
    case 'present':
      return 'Mesh seen'
    case 'absent':
      return 'Mesh unseen'
    default:
      return 'Mesh unknown'
  }
}

export const short = (value: string, head = 12, tail = 10) => {
  if (value.length <= head + tail + 3) {
    return value
  }

  return `${value.slice(0, head)}...${value.slice(-tail)}`
}

export const activeNetwork = (state: UiState) =>
  state.networks.find((network) => network.enabled) ?? state.networks[0]

export const inactiveNetworks = (state: UiState) => state.networks.filter((network) => !network.enabled)

export const inviteInviterParticipant = (network: NetworkView) =>
  network.inviteInviterNpub
    ? network.participants.find((participant) => participant.npub === network.inviteInviterNpub)
    : undefined

export const joinRequestButtonLabel = (network: NetworkView) => {
  if (inviteInviterParticipant(network)?.state === 'online') {
    return 'Connected'
  }
  if (network.outboundJoinRequest) {
    return 'Requested'
  }
  return 'Request Join'
}

export const joinRequestStatusText = (network: NetworkView) => {
  if (inviteInviterParticipant(network)?.state === 'online') {
    return 'Mesh connection received'
  }
  if (network.outboundJoinRequest) {
    return `Requested ${network.outboundJoinRequest.requestedAtText}`
  }
  if (!network.inviteInviterNpub) {
    return ''
  }
  return `Imported from ${network.inviteInviterNpub}. Send a FIPS join request if they have not added this device yet.`
}

export const heroStateBadgeClass = (state: UiState) => {
  if (!state.vpnSessionControlSupported) {
    return 'muted'
  }
  if ((!state.serviceInstalled || state.serviceDisabled) && !state.sessionActive) {
    return 'warn'
  }
  if (state.meshReady) {
    return 'ok'
  }
  if (state.sessionActive) {
    return 'warn'
  }
  return 'muted'
}

export const heroSubtext = (state: UiState) => {
  if (!state.vpnSessionControlSupported) {
    return state.runtimeStatusDetail
  }
  const network = activeNetwork(state)
  if ((!state.serviceInstalled || state.serviceDisabled) && !state.sessionActive) {
    return 'Install the background service for reliable startup, reconnects, and admin-free VPN switching.'
  }
  if (!state.sessionActive) {
    return `Ready to connect ${network.name}.`
  }
  if (state.expectedPeerCount === 0) {
    return `${network.name} is active, but no remote devices are configured yet.`
  }
  if (state.meshReady) {
    return `${network.name} is fully connected across ${state.connectedPeerCount}/${state.expectedPeerCount} peers.`
  }

  const remaining = Math.max(state.expectedPeerCount - state.connectedPeerCount, 0)
  return `${network.name} is waiting on ${remaining} more peer${remaining === 1 ? '' : 's'}.`
}

export const heroBadgeText = (state: UiState) => {
  if (!state.vpnSessionControlSupported) {
    return 'Preview'
  }
  if ((!state.serviceInstalled || state.serviceDisabled) && !state.sessionActive) {
    return 'Service required'
  }
  if (state.sessionActive && state.expectedPeerCount > 0) {
    return `Mesh ${state.connectedPeerCount}/${state.expectedPeerCount}`
  }
  if (state.sessionActive) {
    return 'VPN On'
  }
  return 'VPN Off'
}

export const heroDetailText = (state: UiState) => (state.vpnSessionControlSupported ? state.sessionStatus : '')

export const platformLabel = (platform: string) =>
  platform.length > 0 ? `${platform[0].toUpperCase()}${platform.slice(1)}` : 'Unknown'

export const networkPeerSummary = (network: NetworkView) => {
  const saved = `${network.participants.length} device${network.participants.length === 1 ? '' : 's'} saved`
  if (network.enabled) {
    return `${saved} • ${onlineDeviceSummary(network.onlineCount, network.expectedCount)}`
  }
  return saved
}

export const networkAdminSummary = (network: NetworkView) => {
  const count = network.adminNpubs.length
  if (count === 0) {
    return 'No admins configured'
  }
  if (network.localIsAdmin) {
    return `You can manage members • ${count} admin${count === 1 ? '' : 's'} configured`
  }
  return `Managed by ${short(network.inviteInviterNpub || network.adminNpubs[0] || '')} • ${count} admin${count === 1 ? '' : 's'} configured`
}

export const onlineDeviceSummary = (onlineCount: number, expectedCount: number) =>
  `${onlineCount}/${expectedCount} device${expectedCount === 1 ? '' : 's'} online`

export const networkHasParticipant = (network: NetworkView, npub: string) =>
  network.participants.some((participant) => participant.npub === npub)

export const exitNodeCandidates = (state: UiState) => {
  const seen = new Set<string>()
  const participants: ParticipantView[] = []

  for (const network of state.networks) {
    for (const participant of network.participants) {
      if (participant.state === 'local' || seen.has(participant.npub)) {
        continue
      }
      seen.add(participant.npub)
      participants.push(participant)
    }
  }

  return participants.sort((left, right) => {
    const exitScore = Number(right.offersExitNode) - Number(left.offersExitNode)
    if (exitScore !== 0) {
      return exitScore
    }
    const stateScore = peerStatePriority(left.state) - peerStatePriority(right.state)
    if (stateScore !== 0) {
      return stateScore
    }
    return exitNodeOptionLabel(left).localeCompare(exitNodeOptionLabel(right))
  })
}

export const exitNodeOptionLabel = (participant: ParticipantView) => {
  const base = participant.magicDnsName || participant.npub
  return participant.offersExitNode
    ? `${base} (offers private exit node)`
    : `${base} (not offering private exit node)`
}

export const filteredExitNodeCandidates = (state: UiState, query: string) => {
  const normalized = query.trim().toLowerCase()
  return exitNodeCandidates(state).filter((participant) => {
    if (!normalized) {
      return true
    }
    return (
      participant.magicDnsName.toLowerCase().includes(normalized) ||
      participant.magicDnsAlias.toLowerCase().includes(normalized) ||
      participant.npub.toLowerCase().includes(normalized) ||
      participant.tunnelIp.toLowerCase().includes(normalized)
    )
  })
}

export const selectedExitNodeParticipant = (state: UiState) =>
  exitNodeCandidates(state).find((participant) => participant.npub === state.exitNode)

export const exitNodeAvailabilityClass = (participant: ParticipantView) => {
  if (!participant.offersExitNode) {
    return 'muted'
  }
  switch (participant.state) {
    case 'online':
      return 'ok'
    case 'pending':
      return 'warn'
    case 'offline':
      return 'bad'
    default:
      return 'muted'
  }
}

export const exitNodeAvailabilityText = (participant: ParticipantView) => {
  if (!participant.offersExitNode) {
    return 'Not offered'
  }
  switch (participant.state) {
    case 'online':
      return 'Ready'
    case 'pending':
      return 'Waiting'
    case 'offline':
      return 'Offline'
    default:
      return 'Unknown'
  }
}

export const offerExitNodeStatusText = (state: UiState) => {
  const defaultRoutes = state.effectiveAdvertisedRoutes.filter(
    (route) => route === '0.0.0.0/0' || route === '::/0',
  )
  const advertised = defaultRoutes.length > 0 ? defaultRoutes.join(', ') : '0.0.0.0/0, ::/0'

  if (state.advertiseExitNode) {
    return `Will advertise default routes: ${advertised}`
  }

  return 'Turn this on to offer this device as a private exit node.'
}

export const additionalRoutesStatusText = (state: UiState) => {
  if (state.advertisedRoutes.length === 0) {
    return 'Optional extra LAN or subnet routes. Not needed for exit-node traffic.'
  }

  return `Currently advertising extra routes: ${state.advertisedRoutes.join(', ')}`
}

export const routingSectionMetaText = (state: UiState) => {
  if (state.exitNode && state.advertiseExitNode) {
    return 'Using remote exit + sharing local exit'
  }
  if (state.exitNode) {
    return 'Using remote exit'
  }
  if (state.advertiseExitNode) {
    return 'Sharing local private exit'
  }

  return 'Direct mesh'
}

export const routingModeStatusText = (state: UiState) => {
  if (state.exitNode && state.advertiseExitNode) {
    return 'Your internet-bound traffic uses the selected peer while this device also advertises private default routes to peers.'
  }
  if (state.exitNode) {
    return selectedExitNodeStatusText(state)
  }
  if (state.advertiseExitNode) {
    return 'This device is offering private default-route traffic to peers while your own internet-bound traffic stays local.'
  }

  return 'Internet-bound traffic stays local; only mesh routes are used.'
}

export const selectedExitNodeStatusText = (state: UiState) => {
  if (!state.exitNode) {
    return 'Internet-bound traffic stays local; only mesh routes are used.'
  }

  const selected = selectedExitNodeParticipant(state)
  if (!selected) {
    return 'Selected exit node is not present in the current network view.'
  }

  const label = selected.magicDnsName || selected.npub
  if (!selected.offersExitNode) {
    return `${label} is selected, but it is not offering exit-node traffic right now.`
  }

  switch (selected.state) {
    case 'online':
      return `${label} is selected and ready to carry internet-bound traffic.`
    case 'pending':
      return `${label} is selected, but FIPS reachability is still pending.`
    case 'offline':
      return `${label} is selected, but it is currently offline.`
    default:
      return `${label} is selected; availability is still being checked.`
  }
}

export const selectedExitNodeBadgeClass = (state: UiState) => {
  if (!state.exitNode) {
    return 'muted'
  }

  const selected = selectedExitNodeParticipant(state)
  if (!selected || !selected.offersExitNode) {
    return 'bad'
  }

  switch (selected.state) {
    case 'online':
      return 'ok'
    case 'pending':
      return 'warn'
    case 'offline':
      return 'bad'
    default:
      return 'muted'
  }
}

export const selectedExitNodeBadgeText = (state: UiState) => {
  const selected = selectedExitNodeParticipant(state)
  if (!selected) {
    return 'Exit node unavailable'
  }

  const label = selected.magicDnsName || selected.magicDnsAlias || short(selected.npub, 12, 10)
  return `Exit ${label}`
}

export const formatTrafficBytes = (bytes: number) => {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return '0 B'
  }

  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unitIndex = 0
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024
    unitIndex += 1
  }
  const digits = unitIndex === 0 || value >= 100 ? 0 : value >= 10 ? 1 : 2
  return `${value.toFixed(digits)} ${units[unitIndex]}`
}

export const participantTrafficText = (participant: ParticipantView) =>
  `rx ${formatTrafficBytes(participant.rxBytes)} · tx ${formatTrafficBytes(participant.txBytes)}`

export const formatTrafficRate = (bytesPerSecond: number) =>
  `${formatTrafficBytes(bytesPerSecond)}/s`

export const formatCountdown = (totalSecs: number) => {
  const minutes = Math.floor(totalSecs / 60)
  const seconds = totalSecs % 60
  return `${minutes}:${seconds.toString().padStart(2, '0')}`
}

export const lanPairingHelpText = (state: UiState) =>
  state.lanPairingActive
    ? 'Nearby devices can join this mesh directly while pairing is active.'
    : 'Broadcast this invite on the local network for 15 minutes so nearby devices can join.'
