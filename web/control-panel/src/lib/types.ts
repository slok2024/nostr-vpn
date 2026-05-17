export type HealthSeverity = 'info' | 'warning' | 'critical';

export type HealthIssue = {
  code: string;
  severity: HealthSeverity;
  summary: string;
  detail: string;
};

export type ProbeStatus = {
  state: string;
  detail?: string;
};

export type PortMappingStatus = {
  upnp: ProbeStatus;
  natPmp: ProbeStatus;
  pcp: ProbeStatus;
  activeProtocol?: string | null;
  externalEndpoint?: string | null;
  gateway?: string | null;
  goodUntil?: number | null;
};

export type NetworkSummary = {
  defaultInterface?: string | null;
  primaryIpv4?: string | null;
  primaryIpv6?: string | null;
  gatewayIpv4?: string | null;
  gatewayIpv6?: string | null;
  changedAt?: number | null;
  captivePortal?: boolean | null;
};

export type ParticipantView = {
  npub: string;
  pubkeyHex: string;
  alias: string;
  isAdmin: boolean;
  tunnelIp: string;
  magicDnsAlias: string;
  magicDnsName: string;
  txBytes: number;
  rxBytes: number;
  advertisedRoutes: string[];
  offersExitNode: boolean;
  fipsEndpointNpub: string;
  fipsTransportAddr: string;
  fipsTransportType: string;
  fipsSrttMs?: number | null;
  fipsPacketsSent: number;
  fipsPacketsRecv: number;
  fipsBytesSent: number;
  fipsBytesRecv: number;
  state: string;
  meshState: string;
  statusText: string;
  lastSeenText: string;
};

export type OutboundJoinRequestView = {
  recipientNpub: string;
  recipientPubkeyHex: string;
  requestedAtText: string;
};

export type InboundJoinRequestView = {
  requesterNpub: string;
  requesterPubkeyHex: string;
  requesterNodeName: string;
  requestedAtText: string;
};

export type NetworkView = {
  id: string;
  name: string;
  enabled: boolean;
  networkId: string;
  localIsAdmin: boolean;
  adminNpubs: string[];
  joinRequestsEnabled: boolean;
  inviteInviterNpub: string;
  outboundJoinRequest?: OutboundJoinRequestView | null;
  inboundJoinRequests: InboundJoinRequestView[];
  onlineCount: number;
  expectedCount: number;
  participants: ParticipantView[];
};

export type LanPeer = {
  invite?: string;
  nodeName?: string;
  networkName?: string;
  lastSeenText?: string;
};

export type UiState = {
  platform: string;
  mobile: boolean;
  vpnControlSupported: boolean;
  cliInstallSupported: boolean;
  startupSettingsSupported: boolean;
  trayBehaviorSupported: boolean;
  runtimeStatusDetail: string;
  daemonRunning: boolean;
  vpnEnabled: boolean;
  vpnActive: boolean;
  cliInstalled: boolean;
  serviceSupported: boolean;
  serviceEnablementSupported: boolean;
  serviceInstalled: boolean;
  serviceDisabled: boolean;
  serviceRunning: boolean;
  serviceStatusDetail: string;
  vpnStatus: string;
  appVersion: string;
  daemonBinaryVersion: string;
  configPath: string;
  ownNpub: string;
  ownPubkeyHex: string;
  networkId: string;
  activeNetworkInvite: string;
  nodeId: string;
  nodeName: string;
  selfMagicDnsName: string;
  endpoint: string;
  tunnelIp: string;
  listenPort: number;
  exitNode: string;
  advertiseExitNode: boolean;
  advertisedRoutes: string[];
  effectiveAdvertisedRoutes: string[];
  magicDnsSuffix: string;
  magicDnsStatus: string;
  autoconnect: boolean;
  inviteBroadcastActive: boolean;
  inviteBroadcastRemainingSecs: number;
  nearbyDiscoveryActive: boolean;
  nearbyDiscoveryRemainingSecs: number;
  launchOnStartup: boolean;
  closeToTrayOnClose: boolean;
  connectedPeerCount: number;
  expectedPeerCount: number;
  meshReady: boolean;
  health: HealthIssue[];
  network: NetworkSummary;
  portMapping: PortMappingStatus;
  networks: NetworkView[];
  lanPeers: LanPeer[];
};

export type QrMatrix = {
  width: number;
  cells: boolean[];
};
