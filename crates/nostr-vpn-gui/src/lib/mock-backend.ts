import type { NetworkView, SettingsPatch } from './types'
import { decodeInvitePayload, determineInviteImportTarget } from './invite-code.js'
import {
  activateMockNetwork,
  asResult,
  composeMagicDnsName,
  defaultMockLanPeers,
  emptyMockJoinRequestState,
  getMockAutostartEnabled,
  mockActiveNetwork,
  mockRequiresServiceSetup,
  mockState,
  nextMockNetworkId,
  normalizeAlias,
  pseudoHexFromNpub,
  setMockAutostartEnabled,
  setMockLanPairingEndsAt,
  syncMockNetworkAdminState,
  updateMockRelaySummary,
} from './mock-state.js'

type MockNetworkInvite = {
  v: number
  networkName: string
  networkId: string
  inviterNpub: string
  admins: string[]
  participants: string[]
  relays: string[]
}

export const isTauriRuntime = () =>
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

export const tickMock = () => asResult()
export const connectSessionMock = () => {
  if (mockRequiresServiceSetup()) {
    throw new Error('Install background service to turn VPN on from the app')
  }
  mockState.sessionActive = true
  mockState.daemonRunning = true
  mockState.serviceDisabled = false
  mockState.serviceRunning = mockState.serviceInstalled
  mockState.relayConnected = true
  mockState.sessionStatus = 'Daemon running'
  mockState.relays = mockState.relays.map((relay) => ({
    ...relay,
    state: 'up',
    statusText: 'connected (mock)',
  }))
  mockState.networks = mockState.networks.map((network) => ({
    ...network,
    participants: network.participants.map((participant) => ({
      ...participant,
      state: participant.state === 'local' ? 'local' : 'online',
      presenceState: participant.state === 'local' ? 'local' : 'present',
      statusText: participant.state === 'local' ? 'local' : 'online (seen 0s ago)',
      lastSignalText: participant.state === 'local' ? 'self' : 'nostr seen 0s ago',
    })),
  }))
  updateMockRelaySummary()
  return asResult()
}

export const disconnectSessionMock = () => {
  mockState.sessionActive = false
  mockState.daemonRunning = true
  mockState.serviceDisabled = false
  mockState.serviceRunning = mockState.serviceInstalled
  mockState.relayConnected = false
  mockState.sessionStatus = 'Paused'
  mockState.relays = mockState.relays.map((relay) => ({
    ...relay,
    state: 'unknown',
    statusText: 'not checked',
  }))
  mockState.networks = mockState.networks.map((network) => ({
    ...network,
    participants: network.participants.map((participant) => ({
      ...participant,
      state: participant.state === 'local' ? 'local' : 'unknown',
      presenceState: participant.state === 'local' ? 'local' : 'unknown',
      statusText: participant.state === 'local' ? 'local' : 'unknown',
      lastSignalText: participant.state === 'local' ? 'self' : 'nostr unseen',
    })),
  }))
  updateMockRelaySummary()
  return asResult()
}

export const installCliMock = () => asResult()
export const uninstallCliMock = () => asResult()

export const installSystemServiceMock = () => {
  mockState.serviceInstalled = true
  mockState.serviceDisabled = false
  mockState.serviceRunning = true
  mockState.daemonRunning = true
  mockState.serviceStatusDetail = 'Background service running (mock)'
  mockState.sessionStatus = 'Daemon running'
  return asResult()
}

export const enableSystemServiceMock = installSystemServiceMock

export const disableSystemServiceMock = () => {
  mockState.serviceInstalled = true
  mockState.serviceDisabled = true
  mockState.serviceRunning = false
  mockState.sessionActive = false
  mockState.daemonRunning = false
  mockState.relayConnected = false
  mockState.serviceStatusDetail = 'Background service is installed but disabled in launchd'
  mockState.sessionStatus = 'Background service is disabled in launchd'
  return asResult()
}

export const uninstallSystemServiceMock = () => {
  mockState.serviceInstalled = false
  mockState.serviceDisabled = false
  mockState.serviceRunning = false
  mockState.sessionActive = false
  mockState.daemonRunning = false
  mockState.relayConnected = false
  mockState.serviceStatusDetail = 'Background service is not installed'
  mockState.sessionStatus = 'Install background service to turn VPN on from the app'
  return asResult()
}

export const addNetworkMock = (name: string) => {
  const index = mockState.networks.length + 1
  const normalized = name.trim() || `Network ${index}`
  const id = nextMockNetworkId()
  mockState.networks.push({
    id,
    name: normalized,
    enabled: false,
    networkId: id.replace(/-/g, ''),
    localIsAdmin: true,
    adminNpubs: [mockState.ownNpub],
    ...emptyMockJoinRequestState(),
    onlineCount: 0,
    expectedCount: 0,
    participants: [],
  })
  return asResult()
}

export const renameNetworkMock = (networkId: string, name: string) => {
  mockState.networks = mockState.networks.map((network) =>
    network.id === networkId ? { ...network, name: name.trim() || network.name } : network,
  )
  return asResult()
}

export const setNetworkMeshIdMock = (networkId: string, meshId: string) => {
  mockState.networks = mockState.networks.map((network) =>
    network.id === networkId
      ? { ...network, networkId: meshId.trim() || network.networkId }
      : network,
  )
  return asResult()
}

export const removeNetworkMock = (networkId: string) => {
  if (mockState.networks.length <= 1) {
    return asResult()
  }
  mockState.networks = mockState.networks.filter((network) => network.id !== networkId)
  if (!mockState.networks.some((network) => network.enabled) && mockState.networks[0]) {
    activateMockNetwork(mockState.networks[0].id)
  }
  return asResult()
}

export const setNetworkEnabledMock = (networkId: string, enabled: boolean) => {
  if (enabled) {
    activateMockNetwork(networkId)
  }
  return asResult()
}

export const setNetworkJoinRequestsEnabledMock = (networkId: string, enabled: boolean) => {
  mockState.networks = mockState.networks.map((network) =>
    network.id === networkId ? { ...network, joinRequestsEnabled: enabled } : network,
  )
  return asResult()
}

export const requestNetworkJoinMock = (networkId: string) => {
  mockState.networks = mockState.networks.map((network) => {
    if (network.id !== networkId || !network.inviteInviterNpub) {
      return network
    }

    const recipient =
      network.participants.find((participant) => participant.npub === network.inviteInviterNpub) ??
      null

    return {
      ...network,
      outboundJoinRequest: {
        recipientNpub: network.inviteInviterNpub,
        recipientPubkeyHex: recipient?.pubkeyHex ?? pseudoHexFromNpub(network.inviteInviterNpub),
        requestedAtText: '0s ago',
      },
    }
  })
  mockState.sessionStatus = 'Join request sent'
  return asResult()
}

const upsertMockParticipant = (networkId: string, npub: string, alias = '') => {
  const target = mockState.networks.find((network) => network.id === networkId)
  if (!target || target.participants.some((participant) => participant.npub === npub)) {
    return
  }

  const pubkeyHex = pseudoHexFromNpub(npub)
  const aliasCandidate = normalizeAlias(alias)
  const magicDnsAlias = aliasCandidate.length > 0 ? aliasCandidate : `peer-${pubkeyHex.slice(0, 10)}`

  target.participants.push({
    npub,
    pubkeyHex,
    isAdmin: target.adminNpubs.includes(npub),
    tunnelIp: '10.44.0.2/32',
    magicDnsAlias,
    magicDnsName: composeMagicDnsName(magicDnsAlias, mockState.magicDnsSuffix),
    txBytes: 0,
    rxBytes: 0,
    advertisedRoutes: [],
    offersExitNode: false,
    state: 'unknown',
    presenceState: 'absent',
    statusText: 'no signal yet',
    lastSignalText: 'nostr unseen',
  })
}

export const addParticipantMock = (networkId: string, npub: string, alias = '') => {
  upsertMockParticipant(networkId, npub, alias)
  return asResult()
}

export const addAdminMock = (networkId: string, npub: string) => {
  upsertMockParticipant(networkId, npub)
  mockState.networks = mockState.networks.map((network) =>
    network.id === networkId
      ? syncMockNetworkAdminState({
          ...network,
          adminNpubs: [...new Set([...network.adminNpubs, npub])],
        })
      : network,
  )
  mockState.sessionStatus = 'Admin saved'
  return asResult()
}

export const importNetworkInviteMock = (invite: string) => {
  const parsed = decodeInvitePayload(invite) as MockNetworkInvite
  const activeNetwork = mockActiveNetwork()
  if (!activeNetwork) {
    return asResult()
  }

  const importTarget = determineInviteImportTarget(
    mockState.networks,
    activeNetwork.id,
    parsed.networkId,
  )
  let targetNetwork =
    (importTarget.networkId &&
      mockState.networks.find((network) => network.id === importTarget.networkId)) ||
    null

  if (importTarget.mode === 'create') {
    const id = nextMockNetworkId()
    targetNetwork = {
      id,
      name: parsed.networkName.trim() || `Network ${mockState.networks.length + 1}`,
      enabled: false,
      networkId: parsed.networkId.trim() || id.replace(/-/g, ''),
      localIsAdmin: false,
      adminNpubs: [],
      ...emptyMockJoinRequestState(),
      onlineCount: 0,
      expectedCount: 0,
      participants: [],
    }
    mockState.networks.push(targetNetwork)
  }

  if (!targetNetwork) {
    targetNetwork = activeNetwork
  }

  if (
    parsed.networkName.trim() &&
    (targetNetwork.participants.length === 0 || /^Network \d+/.test(targetNetwork.name))
  ) {
    targetNetwork.name = parsed.networkName.trim()
  }
  if (parsed.networkId.trim()) {
    targetNetwork.networkId = parsed.networkId.trim()
  }
  targetNetwork.adminNpubs = [...new Set([...(targetNetwork.adminNpubs || []), ...parsed.admins])]
  targetNetwork.inviteInviterNpub = parsed.inviterNpub
  activateMockNetwork(targetNetwork.id)
  for (const participant of parsed.participants) {
    upsertMockParticipant(targetNetwork.id, participant)
  }
  targetNetwork = syncMockNetworkAdminState(targetNetwork)
  mockState.networks = mockState.networks.map((network) =>
    network.id === targetNetwork?.id ? targetNetwork : network,
  )
  if (targetNetwork.inviteInviterNpub) {
    const recipient =
      targetNetwork.participants.find(
        (participant) => participant.npub === targetNetwork?.inviteInviterNpub,
      ) ?? null
    targetNetwork.outboundJoinRequest = {
      recipientNpub: targetNetwork.inviteInviterNpub,
      recipientPubkeyHex: recipient?.pubkeyHex ?? pseudoHexFromNpub(targetNetwork.inviteInviterNpub),
      requestedAtText: '0s ago',
    }
    mockState.sessionActive = true
  }
  for (const relay of parsed.relays) {
    const normalizedRelay = relay.trim()
    if (normalizedRelay && !mockState.relays.some((entry) => entry.url === normalizedRelay)) {
      mockState.relays.push({
        url: normalizedRelay,
        state: 'unknown',
        statusText: 'not checked',
      })
    }
  }
  updateMockRelaySummary()
  mockState.sessionStatus = targetNetwork.inviteInviterNpub
    ? `Invite imported and join request sent for ${parsed.networkName.trim() || targetNetwork.name}`
    : `Invite imported for ${parsed.networkName.trim() || targetNetwork.name}`
  return asResult()
}

export const startLanPairingMock = () => {
  setMockLanPairingEndsAt(Date.now() + 15 * 60 * 1000)
  mockState.lanPeers = defaultMockLanPeers()
  return asResult()
}

export const stopLanPairingMock = () => {
  setMockLanPairingEndsAt(null)
  mockState.lanPeers = []
  return asResult()
}

export const setParticipantAliasMock = (npub: string, alias: string) => {
  const normalized = normalizeAlias(alias)
  mockState.networks = mockState.networks.map((network) => ({
    ...network,
    participants: network.participants.map((participant) => {
      if (participant.npub !== npub) {
        return participant
      }

      const magicDnsAlias = normalized || participant.magicDnsAlias
      return {
        ...participant,
        magicDnsAlias,
        magicDnsName: composeMagicDnsName(magicDnsAlias, mockState.magicDnsSuffix),
      }
    }),
  }))
  return asResult()
}

export const removeParticipantMock = (networkId: string, npub: string) => {
  mockState.networks = mockState.networks.map((network) => {
    if (network.id !== networkId) {
      return network
    }
    if (network.adminNpubs.includes(npub) && network.adminNpubs.length <= 1) {
      return network
    }
    return syncMockNetworkAdminState({
      ...network,
      adminNpubs: network.adminNpubs.filter((admin) => admin !== npub),
      participants: network.participants.filter((participant) => participant.npub !== npub),
    })
  })
  return asResult()
}

export const removeAdminMock = (networkId: string, npub: string) => {
  mockState.networks = mockState.networks.map((network) => {
    if (network.id !== networkId || network.adminNpubs.length <= 1) {
      return network
    }
    return syncMockNetworkAdminState({
      ...network,
      adminNpubs: network.adminNpubs.filter((admin) => admin !== npub),
    })
  })
  mockState.sessionStatus = 'Admin removed'
  return asResult()
}

export const acceptJoinRequestMock = (networkId: string, requesterNpub: string) => {
  upsertMockParticipant(networkId, requesterNpub)
  mockState.networks = mockState.networks.map((network) =>
    network.id === networkId
      ? {
          ...network,
          inboundJoinRequests: network.inboundJoinRequests.filter(
            (request) => request.requesterNpub !== requesterNpub,
          ),
        }
      : network,
  )
  mockState.sessionStatus = 'Join request accepted'
  return asResult()
}

export const addRelayMock = (relay: string) => {
  if (!mockState.relays.some((entry) => entry.url === relay)) {
    mockState.relays.push({ url: relay, state: 'unknown', statusText: 'not checked' })
    updateMockRelaySummary()
  }
  return asResult()
}

export const removeRelayMock = (relay: string) => {
  if (mockState.relays.length > 1) {
    mockState.relays = mockState.relays.filter((entry) => entry.url !== relay)
    updateMockRelaySummary()
  }
  return asResult()
}

export const updateSettingsMock = (patch: SettingsPatch) => {
  if (patch.nodeName !== undefined) {
    mockState.nodeName = patch.nodeName
  }
  if (patch.endpoint !== undefined) {
    mockState.endpoint = patch.endpoint
  }
  if (patch.tunnelIp !== undefined) {
    mockState.tunnelIp = patch.tunnelIp
  }
  if (patch.magicDnsSuffix !== undefined) {
    mockState.magicDnsSuffix = patch.magicDnsSuffix
    mockState.magicDnsStatus =
      patch.magicDnsSuffix.trim().length > 0
        ? `System DNS active for .${patch.magicDnsSuffix} via 127.0.0.1:1053`
        : 'Local DNS only on 127.0.0.1:1053 (set suffix for system split-dns)'
    mockState.networks = mockState.networks.map((network) => ({
      ...network,
      participants: network.participants.map((participant) => ({
        ...participant,
        magicDnsName: composeMagicDnsName(participant.magicDnsAlias, mockState.magicDnsSuffix),
      })),
    }))
  }
  if (patch.listenPort !== undefined) {
    mockState.listenPort = patch.listenPort
  }
  if (patch.exitNode !== undefined) {
    mockState.exitNode = patch.exitNode.trim()
  }
  if (patch.advertiseExitNode !== undefined) {
    mockState.advertiseExitNode = patch.advertiseExitNode
  }
  if (patch.advertisedRoutes !== undefined) {
    mockState.advertisedRoutes = patch.advertisedRoutes
      .split(',')
      .map((value) => value.trim())
      .filter((value) => value.length > 0)
  }
  if (patch.autoconnect !== undefined) {
    mockState.autoconnect = patch.autoconnect
  }
  if (patch.launchOnStartup !== undefined) {
    mockState.launchOnStartup = patch.launchOnStartup
  }
  if (patch.closeToTrayOnClose !== undefined) {
    mockState.closeToTrayOnClose = patch.closeToTrayOnClose
  }
  return asResult()
}

export const isAutostartEnabledMock = async () => getMockAutostartEnabled()

export const setAutostartEnabledMock = async (enabled: boolean) => {
  setMockAutostartEnabled(enabled)
  return true
}
