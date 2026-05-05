import { encodeInvitePayload } from './invite-code.js'
import type { LanPeerView, NetworkView, UiState } from './types'

declare const __APP_VERSION__: string

const APP_VERSION = __APP_VERSION__

export const composeMagicDnsName = (alias: string, suffix: string) =>
  suffix.trim().length > 0 ? `${alias}.${suffix}` : alias

export const normalizeAlias = (value: string) =>
  value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9-]+/g, '-')
    .replace(/^-+|-+$/g, '')

export const emptyMockJoinRequestState = () => ({
  joinRequestsEnabled: true,
  inviteInviterNpub: '',
  outboundJoinRequest: null,
  inboundJoinRequests: [],
})

export const pseudoHexFromNpub = (npub: string) => {
  const seed = npub
    .replace(/^npub1/i, '')
    .replace(/[^a-z0-9]/gi, '')
    .toLowerCase()
  return (seed + 'a'.repeat(64)).slice(0, 64)
}

const countExpectedPeers = (network: NetworkView) =>
  network.enabled
    ? network.participants.filter((participant) => participant.state !== 'local').length
    : 0

const countOnlinePeers = (network: NetworkView) =>
  network.enabled
    ? network.participants.filter((participant) => participant.state === 'online').length
    : 0

const countExpectedDevices = (network: NetworkView) =>
  network.enabled ? countExpectedPeers(network) + 1 : 0

const countOnlineDevices = (network: NetworkView, sessionActive: boolean) =>
  network.enabled ? countOnlinePeers(network) + Number(sessionActive) : 0

export const defaultMockLanPeers = (): LanPeerView[] => [
  {
    npub: 'npub1x8teht3pj2zhq6e4l6s5zh2fcn0vzrp3d8zjls74g7zq5qemk3dq3wlp5m',
    nodeName: 'home-server',
    endpoint: '192.168.1.20:51820',
    networkName: 'Home',
    networkId: 'mesh-home',
    invite: encodeInvitePayload({
      v: 2,
      networkName: 'Home',
      networkId: 'mesh-home',
      inviterNpub: 'npub1x8teht3pj2zhq6e4l6s5zh2fcn0vzrp3d8zjls74g7zq5qemk3dq3wlp5m',
      admins: ['npub1x8teht3pj2zhq6e4l6s5zh2fcn0vzrp3d8zjls74g7zq5qemk3dq3wlp5m'],
      participants: ['npub1x8teht3pj2zhq6e4l6s5zh2fcn0vzrp3d8zjls74g7zq5qemk3dq3wlp5m'],
      relays: ['wss://temp.iris.to', 'wss://relay.damus.io'],
    }),
    lastSeenText: '2s ago',
  },
]

export const mockState: UiState = {
  platform: 'desktop',
  mobile: false,
  vpnSessionControlSupported: true,
  cliInstallSupported: true,
  startupSettingsSupported: true,
  trayBehaviorSupported: true,
  runtimeStatusDetail: '',
  daemonRunning: false,
  sessionActive: false,
  relayConnected: false,
  cliInstalled: false,
  serviceSupported: true,
  serviceEnablementSupported: true,
  serviceInstalled: false,
  serviceDisabled: false,
  serviceRunning: false,
  serviceStatusDetail: 'Background service is not installed',
  sessionStatus: 'Install background service to turn VPN on from the app',
  appVersion: APP_VERSION,
  daemonBinaryVersion: APP_VERSION,
  serviceBinaryVersion: APP_VERSION,
  configPath: '~/.config/nvpn/config.toml',
  ownNpub: 'npub1akgu9lxldpt32lnjf97k005a4kgasewmvsrmkpzqeff39ssev0ssd6t3u',
  ownPubkeyHex: 'f'.repeat(64),
  networkId: 'mockmesh1234',
  activeNetworkInvite: '',
  nodeId: 'mock-node',
  nodeName: 'nostr-vpn-node',
  selfMagicDnsName: 'nostr-vpn-node.nvpn',
  endpoint: '192.168.1.4:51820',
  tunnelIp: '10.44.0.1/32',
  listenPort: 51820,
  exitNode: '',
  advertiseExitNode: false,
  advertisedRoutes: [],
  effectiveAdvertisedRoutes: [],
  magicDnsSuffix: 'nvpn',
  magicDnsStatus: 'System DNS active for .nvpn via 127.0.0.1:1053',
  autoconnect: true,
  lanPairingActive: true,
  lanPairingRemainingSecs: 11 * 60 + 42,
  launchOnStartup: true,
  closeToTrayOnClose: true,
  connectedPeerCount: 0,
  expectedPeerCount: 0,
  meshReady: false,
  health: [],
  network: {
    defaultInterface: 'en0',
    primaryIpv4: '192.168.1.4',
    primaryIpv6: 'fd00::4',
    gatewayIpv4: '192.168.1.1',
    gatewayIpv6: 'fd00::1',
    captivePortal: false,
  },
  portMapping: {
    upnp: { state: 'unknown', detail: 'not checked' },
    natPmp: { state: 'unknown', detail: 'not checked' },
    pcp: { state: 'unknown', detail: 'not checked' },
  },
  networks: [
    {
      id: 'network-1',
      name: 'Network 1',
      enabled: true,
      networkId: 'mockmesh1234',
      localIsAdmin: true,
      adminNpubs: ['npub1akgu9lxldpt32lnjf97k005a4kgasewmvsrmkpzqeff39ssev0ssd6t3u'],
      ...emptyMockJoinRequestState(),
      onlineCount: 0,
      expectedCount: 0,
      participants: [],
    },
  ],
  relays: [
    { url: 'wss://temp.iris.to', state: 'unknown', statusText: 'not checked' },
    { url: 'wss://relay.damus.io', state: 'unknown', statusText: 'not checked' },
    { url: 'wss://relay.snort.social', state: 'unknown', statusText: 'not checked' },
  ],
  relaySummary: { up: 0, down: 0, checking: 0, unknown: 3 },
  lanPeers: defaultMockLanPeers(),
}

let mockLanPairingEndsAt = Date.now() + mockState.lanPairingRemainingSecs * 1000
let mockAutostartEnabled = true

export const setMockLanPairingEndsAt = (value: number | null) => {
  mockLanPairingEndsAt = value
}

export const getMockAutostartEnabled = () => mockAutostartEnabled

export const setMockAutostartEnabled = (enabled: boolean) => {
  mockAutostartEnabled = enabled
}

export const cloneMockState = () => structuredClone(mockState)

export const mockActiveNetwork = () =>
  mockState.networks.find((network) => network.enabled) ?? mockState.networks[0]

export const buildMockActiveNetworkInvite = () => {
  const activeNetwork = mockActiveNetwork()
  if (!activeNetwork) {
    return ''
  }

  return encodeInvitePayload({
    v: 2,
    networkName: activeNetwork.name,
    networkId: activeNetwork.networkId,
    inviterNpub:
      activeNetwork.inviteInviterNpub || activeNetwork.adminNpubs[0] || mockState.ownNpub,
    admins: activeNetwork.adminNpubs,
    participants: activeNetwork.participants.map((participant) => participant.npub),
    relays: mockState.relays.map((relay) => relay.url),
  })
}

export const syncMockNetworkAdminState = (network: NetworkView): NetworkView => {
  const adminSet = new Set(network.adminNpubs)
  return {
    ...network,
    localIsAdmin: adminSet.has(mockState.ownNpub),
    inviteInviterNpub:
      network.inviteInviterNpub && adminSet.has(network.inviteInviterNpub)
        ? network.inviteInviterNpub
        : network.adminNpubs[0] || '',
    participants: network.participants.map((participant) => ({
      ...participant,
      isAdmin: adminSet.has(participant.npub),
    })),
  }
}

export const buildMockSelfMagicDnsName = () => {
  const alias = normalizeAlias(mockState.nodeName)
  return alias ? composeMagicDnsName(alias, mockState.magicDnsSuffix) : ''
}

export const activateMockNetwork = (networkId: string) => {
  mockState.networks = mockState.networks.map((network) => ({
    ...network,
    enabled: network.id === networkId,
  }))
}

export const nextMockNetworkId = () => {
  const index = mockState.networks.length + 1
  let id = `network-${index}`
  let suffix = 2
  while (mockState.networks.some((network) => network.id === id)) {
    id = `network-${index}-${suffix}`
    suffix += 1
  }
  return id
}

export const mockRequiresServiceSetup = () =>
  mockState.serviceSupported && !mockState.serviceInstalled && !mockState.daemonRunning

export const updateMockRelaySummary = () => {
  mockState.relaySummary = {
    up: mockState.relays.filter((relay) => relay.state === 'up').length,
    down: mockState.relays.filter((relay) => relay.state === 'down').length,
    checking: mockState.relays.filter((relay) => relay.state === 'checking').length,
    unknown: mockState.relays.filter((relay) => relay.state === 'unknown').length,
  }
}

export const computeMockEffectiveAdvertisedRoutes = () => {
  const effective = [...mockState.advertisedRoutes]
  if (mockState.advertiseExitNode) {
    for (const route of ['0.0.0.0/0', '::/0']) {
      if (!effective.includes(route)) {
        effective.push(route)
      }
    }
  }
  return effective
}

export const recomputeMockConnectivity = () => {
  mockState.networks = mockState.networks.map((network) => ({
    ...network,
    outboundJoinRequest:
      network.outboundJoinRequest &&
      network.participants.some(
        (participant) =>
          participant.npub === network.outboundJoinRequest?.recipientNpub &&
          participant.state === 'online',
      )
        ? null
        : network.outboundJoinRequest,
  }))

  mockState.networks = mockState.networks.map((network) => ({
    ...network,
    onlineCount: countOnlineDevices(network, mockState.sessionActive),
    expectedCount: countExpectedDevices(network),
  }))

  const activeNetwork = mockActiveNetwork()
  mockState.networkId = activeNetwork?.networkId || mockState.networkId
  mockState.activeNetworkInvite = buildMockActiveNetworkInvite()
  mockState.connectedPeerCount = activeNetwork ? countOnlinePeers(activeNetwork) : 0
  mockState.expectedPeerCount = activeNetwork ? countExpectedPeers(activeNetwork) : 0
  mockState.meshReady =
    mockState.expectedPeerCount > 0 &&
    mockState.connectedPeerCount >= mockState.expectedPeerCount
}

export const refreshMockLanPairing = () => {
  if (mockLanPairingEndsAt === null) {
    mockState.lanPairingActive = false
    mockState.lanPairingRemainingSecs = 0
    mockState.lanPeers = []
    return
  }

  const remainingSecs = Math.max(
    Math.ceil((mockLanPairingEndsAt - Date.now()) / 1000),
    0,
  )
  if (remainingSecs === 0) {
    mockLanPairingEndsAt = null
    mockState.lanPairingActive = false
    mockState.lanPairingRemainingSecs = 0
    mockState.lanPeers = []
    return
  }

  mockState.lanPairingActive = true
  mockState.lanPairingRemainingSecs = remainingSecs
  if (mockState.lanPeers.length === 0) {
    mockState.lanPeers = defaultMockLanPeers()
  }
}

export const asResult = async () => {
  recomputeMockConnectivity()
  refreshMockLanPairing()
  mockState.selfMagicDnsName = buildMockSelfMagicDnsName()
  mockState.effectiveAdvertisedRoutes = computeMockEffectiveAdvertisedRoutes()
  return cloneMockState()
}
