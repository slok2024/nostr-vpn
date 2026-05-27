<script lang="ts">
  import { onMount } from 'svelte';
  import './app.css';
  import { runAction, tick } from './lib/api';
  import CopyButton from './lib/CopyButton.svelte';
  import Modal from './lib/Modal.svelte';
  import QRCode from './lib/QRCode.svelte';
  import { formatBytes, nonEmpty, remainingText, shortMiddle } from './lib/format';
  import nostrVpnIcon from './lib/nostr-vpn-icon.svg';
  import type {
    HealthIssue,
    InboundJoinRequestView,
    NetworkView,
    ParticipantView,
    UiState,
  } from './lib/types';

  type Tab = 'devices' | 'exit' | 'settings';
  type Tone = 'ok' | 'warn' | 'bad' | 'muted' | 'active';

  const tabs: { id: Tab; label: string }[] = [
    { id: 'devices', label: 'Devices' },
    { id: 'exit', label: 'Exit Nodes' },
    { id: 'settings', label: 'Settings' },
  ];
  const SEARCH_VISIBILITY_THRESHOLD = 7;
  const DEVICE_ID_BODY = /^[qpzry9x8gf2tvdw0s3jn54khce6mua7l]+$/;

  let state: UiState | null = null;
  let tab: Tab = 'devices';
  let devicePane: 'list' | 'detail' = 'list';
  let loading = true;
  let refreshing = false;
  let busyAction = '';
  let error = '';
  let notice = '';
  let qrInvite = '';
  let settingsDirty = false;
  let participantNpub = '';
  let participantAlias = '';
  let inviteDraft = '';
  let manualAdminNpub = '';
  let manualNetworkId = '';
  let newNetworkName = '';
  let addNetworkOpen = false;
  let addNetworkJoinStatus = '';
  let addDeviceOpen = false;
  let diagnosticsOpen = false;
  let shownNetworkId = '';
  let selectedParticipantKey = '';
  let deviceSearch = '';
  let exitSearch = '';
  let aliasDrafts: Record<string, string> = {};
  let endpointHintDrafts: Record<string, string> = {};
  let networkNameDrafts: Record<string, string> = {};
  let meshIdDrafts: Record<string, string> = {};
  let wireguardExitConfigDraft = '';
  let wireguardDirty = false;
  let wireguardConfigFileInput: HTMLInputElement | null = null;
  let noticeTimer: number | undefined;

  let settingsDraft = {
    nodeName: '',
    endpoint: '',
    tunnelIp: '',
    listenPort: '',
    relays: '',
    advertisedRoutes: '',
    fipsHostTunnelEnabled: false,
    connectToNonRosterFipsPeers: true,
    fipsNostrDiscoveryEnabled: true,
    fipsBootstrapEnabled: true,
    fipsBootstrapPeers: '',
    fipsHostInboundTcpPorts: '',
    autoconnect: false,
  };

  // The bootstrap/transit peer list is edited as one "npub addr, addr" line per
  // peer, round-tripped to the npub -> addresses map the backend stores.
  function bootstrapPeersToText(peers: Record<string, string[]>): string {
    return Object.entries(peers ?? {})
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([npub, addrs]) => `${npub} ${addrs.join(', ')}`.trim())
      .join('\n');
  }

  function textToBootstrapPeers(text: string): Record<string, string[]> {
    const peers: Record<string, string[]> = {};
    for (const line of text.split('\n')) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      const [npub, ...rest] = trimmed.split(/\s+/);
      const addrs = rest
        .join(' ')
        .split(/[\s,]+/)
        .map((addr) => addr.trim())
        .filter(Boolean);
      if (npub && addrs.length > 0) peers[npub] = addrs;
    }
    return peers;
  }

  $: activeNetwork = state ? state.networks.find((network) => network.enabled) ?? null : null;
  $: shownNetwork = state
    ? state.networks.find((network) => network.id === shownNetworkId) ?? activeNetwork ?? state.networks[0] ?? null
    : null;
  $: addNetworkRequestNetwork =
    (shownNetwork?.outboundJoinRequest || shownNetwork?.inviteInviterNpub
      ? shownNetwork
      : null) ??
    state?.networks.find((network) => network.outboundJoinRequest || network.inviteInviterNpub) ??
    null;
  $: addNetworkJoinRequestSent = Boolean(
    addNetworkJoinStatus || addNetworkRequestNetwork?.outboundJoinRequest,
  );
  $: publicFipsAddress = state?.ownNpub ? `${state.ownNpub}.fips` : '';
  $: incomingJoinRequestCount = state
    ? state.networks.reduce((count, network) => count + network.inboundJoinRequests.length, 0)
    : 0;
  $: participants = shownNetwork?.participants ?? [];
  $: manualAdminNpubTrimmed = manualAdminNpub.trim();
  $: manualNetworkIdNormalized = normalizeNetworkIdInput(manualNetworkId);
  $: manualAdminInvalid =
    manualAdminNpubTrimmed.length > 0 && !isValidDeviceId(manualAdminNpubTrimmed);
  $: canManualAddNetwork =
    Boolean(manualAdminNpubTrimmed) &&
    Boolean(manualNetworkIdNormalized) &&
    !manualAdminInvalid;
  $: showDeviceSearch = participants.length > SEARCH_VISIBILITY_THRESHOLD;
  $: deviceSearchQuery = showDeviceSearch ? deviceSearch.trim().toLowerCase() : '';
  $: visibleParticipants = participants
    .filter((participant) => {
      const query = deviceSearchQuery;
      if (!query) {
        return true;
      }
      return (
        participantName(participant).toLowerCase().includes(query) ||
        participant.alias.toLowerCase().includes(query) ||
        participant.npub.toLowerCase().includes(query) ||
        participant.tunnelIp.toLowerCase().includes(query) ||
        deviceMagicDnsName(participant).toLowerCase().includes(query)
      );
    })
    .sort((a, b) => Number(isSelf(b)) - Number(isSelf(a)));
  $: selectedParticipant = selectedParticipantFrom(visibleParticipants, selectedParticipantKey);
  $: if (devicePane === 'detail' && !selectedParticipant) {
    devicePane = 'list';
  }
  $: allExitCandidates = participants.filter((participant) => participant.offersExitNode && !isSelf(participant));
  $: showExitSearch = allExitCandidates.length > SEARCH_VISIBILITY_THRESHOLD;
  $: exitSearchQuery = showExitSearch ? exitSearch.trim().toLowerCase() : '';
  $: exitCandidates = allExitCandidates.filter((participant) => {
    if (!participant.offersExitNode || isSelf(participant)) {
      return false;
    }
    const query = exitSearchQuery;
    if (!query) {
      return true;
    }
    return (
      participantName(participant).toLowerCase().includes(query) ||
      participant.alias.toLowerCase().includes(query) ||
      participant.npub.toLowerCase().includes(query) ||
      participant.tunnelIp.toLowerCase().includes(query)
    );
  });
  $: qrInvite = state?.activeNetworkInvite ?? '';
  $: if (addDeviceOpen && !shownNetwork?.enabled) {
    addDeviceOpen = false;
  }

  onMount(() => {
    void refresh(true);
    const refreshTimer = window.setInterval(() => {
      void refresh(false);
    }, 2500);
    return () => window.clearInterval(refreshTimer);
  });

  function applyState(next: UiState) {
    const firstState = state === null;
    shownNetworkId = preferredNetworkId(next, shownNetworkId);
    state = next;
    loading = false;
    error = '';
    if (firstState || !settingsDirty) {
      syncSettings(next);
    }
    if (firstState || !wireguardDirty) {
      syncWireGuard(next);
    }
    syncNetworkDrafts(next);
    syncAliasDrafts(next);
  }

  function preferredNetworkId(next: UiState, current: string): string {
    if (next.networks.some((network) => network.id === current)) {
      return current;
    }
    return next.networks.find((network) => network.enabled)?.id ?? next.networks[0]?.id ?? '';
  }

  function syncSettings(next: UiState) {
    const relays = next.relays ?? [];
    settingsDraft = {
      nodeName: next.nodeName,
      endpoint: next.endpoint,
      tunnelIp: next.tunnelIp,
      listenPort: String(next.listenPort || ''),
      relays: relays.map((relay) => relay.url).join('\n'),
      advertisedRoutes: next.advertisedRoutes.join(', '),
      fipsHostTunnelEnabled: next.fipsHostTunnelEnabled,
      connectToNonRosterFipsPeers: next.connectToNonRosterFipsPeers,
      fipsNostrDiscoveryEnabled: next.fipsNostrDiscoveryEnabled,
      fipsBootstrapEnabled: next.fipsBootstrapEnabled,
      fipsBootstrapPeers: bootstrapPeersToText(next.fipsBootstrapPeers),
      fipsHostInboundTcpPorts: next.fipsHostInboundTcpPorts,
      autoconnect: next.autoconnect,
    };
  }

  function resetBootstrapPeers() {
    settingsDraft.fipsBootstrapPeers = bootstrapPeersToText(
      state?.fipsBootstrapPeerDefaults ?? {},
    );
    settingsDirty = true;
  }

  function syncWireGuard(next: UiState) {
    wireguardExitConfigDraft = next.wireguardExitConfig;
  }

  function syncNetworkDrafts(next: UiState) {
    const names = { ...networkNameDrafts };
    const meshIds = { ...meshIdDrafts };
    for (const network of next.networks) {
      if (!(network.id in names)) {
        names[network.id] = network.name;
      }
      if (!(network.id in meshIds)) {
        meshIds[network.id] = displayNetworkId(network.networkId);
      }
    }
    networkNameDrafts = names;
    meshIdDrafts = meshIds;
  }

  function syncAliasDrafts(next: UiState) {
    const aliases = { ...aliasDrafts };
    const endpointHints = { ...endpointHintDrafts };
    for (const network of next.networks) {
      for (const participant of network.participants) {
        if (!(participant.npub in aliases)) {
          aliases[participant.npub] = participant.magicDnsAlias;
        }
        if (!(participant.npub in endpointHints)) {
          endpointHints[participant.npub] = (participant.fipsEndpointHints ?? []).join(', ');
        }
      }
    }
    aliasDrafts = aliases;
    endpointHintDrafts = endpointHints;
  }

  async function refresh(showSpinner: boolean) {
    if (busyAction) {
      return;
    }
    refreshing = showSpinner;
    try {
      applyState(await tick());
    } catch (err) {
      error = messageOf(err);
      loading = false;
    } finally {
      refreshing = false;
    }
  }

  async function runState(endpoint: string, payload?: unknown, label = 'Working'): Promise<UiState | null> {
    busyAction = label;
    error = '';
    try {
      const next = await runAction(endpoint, payload);
      applyState(next);
      setNotice(next.vpnStatus || 'Updated');
      return next;
    } catch (err) {
      error = messageOf(err);
      return null;
    } finally {
      busyAction = '';
    }
  }

  async function run(endpoint: string, payload?: unknown, label = 'Working'): Promise<boolean> {
    return Boolean(await runState(endpoint, payload, label));
  }

  function setNotice(value: string) {
    notice = value;
    window.clearTimeout(noticeTimer);
    noticeTimer = window.setTimeout(() => {
      notice = '';
    }, 2600);
  }

  function openAddNetwork() {
    addNetworkJoinStatus = '';
    addNetworkOpen = true;
  }

  function closeAddNetwork() {
    addNetworkOpen = false;
    addNetworkJoinStatus = '';
  }

  function messageOf(value: unknown): string {
    if (value instanceof Error) {
      return value.message;
    }
    return String(value);
  }

  function displayNetworkId(value: string): string {
    const trimmed = value.trim();
    if (trimmed.length <= 4 || !/^[0-9a-f]+$/i.test(trimmed)) {
      return trimmed;
    }
    return trimmed.match(/.{1,4}/g)?.join('-') ?? trimmed;
  }

  function normalizeNetworkIdInput(value: string): string {
    const trimmed = value.trim();
    const compact = trimmed.replace(/[\s-]/g, '');
    if (!compact && /^[\s-]*$/.test(trimmed)) {
      return '';
    }
    return compact && /^[0-9a-f]+$/i.test(compact) ? compact.toLowerCase() : trimmed;
  }

  function isValidDeviceId(value: string): boolean {
    const trimmed = value.trim();
    return (
      trimmed.length === 63 &&
      trimmed.startsWith('npub1') &&
      DEVICE_ID_BODY.test(trimmed.slice(5))
    );
  }

  function heroTone(value: UiState | null): Tone {
    if (!value) {
      return 'muted';
    }
    if (value.meshReady) {
      return 'ok';
    }
    if (value.vpnActive) {
      return 'active';
    }
    if (value.daemonRunning) {
      return 'warn';
    }
    return 'muted';
  }

  function participantTone(participant: ParticipantView): Tone {
    const stateText = `${participant.state} ${participant.meshState}`.toLowerCase();
    if (stateText.includes('online') || stateText.includes('present') || stateText.includes('local')) {
      return 'ok';
    }
    if (
      stateText.includes('offline') ||
      stateText.includes('absent') ||
      stateText.includes('off') ||
      stateText.includes('disconnected')
    ) {
      return 'muted';
    }
    return 'warn';
  }

  function issueTone(issue: HealthIssue): Tone {
    if (issue.severity === 'critical') {
      return 'bad';
    }
    if (issue.severity === 'warning') {
      return 'warn';
    }
    return 'muted';
  }

  function participantName(participant: ParticipantView): string {
    for (const value of [
      participant.magicDnsName,
      isSelf(participant) ? state?.selfMagicDnsName : '',
      participant.alias,
      participant.magicDnsAlias,
    ]) {
      const display = displayNameValue(value);
      if (display) {
        return display;
      }
    }
    return generatedDeviceName(participant);
  }

  function displayNameValue(value: string | null | undefined): string {
    const trimmed = value?.trim() ?? '';
    if (!trimmed || isGeneratedHexName(trimmed)) {
      return '';
    }
    return trimmed;
  }

  function isGeneratedHexName(value: string): boolean {
    const label = value.trim().toLowerCase().split('.')[0] ?? '';
    return /^[0-9a-f]{12,64}$/.test(label);
  }

  function generatedDeviceName(participant: ParticipantView): string {
    const source = participant.pubkeyHex || participant.npub || 'device';
    let hash = 2166136261;
    for (let index = 0; index < source.length; index += 1) {
      hash = Math.imul(hash ^ source.charCodeAt(index), 16777619) >>> 0;
    }
    return `Device ${String(hash % 10000).padStart(4, '0')}`;
  }

  function deviceMagicDnsName(participant: ParticipantView): string {
    const direct = displayNameValue(participant.magicDnsName);
    if (direct) {
      return direct;
    }
    if (isSelf(participant)) {
      const selfName = displayNameValue(state?.selfMagicDnsName);
      if (selfName) {
        return selfName;
      }
    }
    const alias = displayNameValue(participant.magicDnsAlias);
    if (alias && state?.magicDnsSuffix) {
      return `${alias}.${state.magicDnsSuffix}`;
    }
    return '';
  }

  function isActiveExitParticipant(participant: ParticipantView): boolean {
    return Boolean(
      state?.exitNodeActive &&
        state.exitNode &&
        participant.npub &&
        participant.npub === state.exitNode,
    );
  }

  function exitNodeBadgeText(participant: ParticipantView): string {
    if (!participant.offersExitNode) {
      return '';
    }
    return isActiveExitParticipant(participant) ? 'Exit active' : 'Exit offered';
  }

  function participantKey(participant: ParticipantView): string {
    return participant.pubkeyHex || participant.npub;
  }

  function selectedParticipantFrom(
    candidates: ParticipantView[],
    key: string,
  ): ParticipantView | null {
    return (
      candidates.find((participant) => participantKey(participant) === key) ??
      candidates.find(isSelf) ??
      candidates[0] ??
      null
    );
  }

  function participantSelected(participant: ParticipantView): boolean {
    return selectedParticipant
      ? participantKey(selectedParticipant) === participantKey(participant)
      : false;
  }

  function openParticipant(participant: ParticipantView) {
    selectedParticipantKey = participantKey(participant);
    devicePane = 'detail';
  }

  function showDeviceList() {
    devicePane = 'list';
  }

  function deviceRoleText(participant: ParticipantView): string {
    const roles = [];
    if (isSelf(participant)) {
      roles.push('This device');
    }
    if (participant.isAdmin) {
      roles.push('Admin');
    }
    const exitRole = exitNodeBadgeText(participant);
    if (exitRole) {
      roles.push(exitRole);
    }
    return roles.length > 0 ? roles.join(', ') : 'Member';
  }

  function fipsPathText(participant: ParticipantView): string {
    if (isSelf(participant)) {
      return 'This device';
    }
    if (isDirectFipsPeer(participant)) {
      const transport = participant.fipsTransportType.trim();
      const transportText = transport ? ` (${transport.toUpperCase()})` : '';
      return participant.fipsSrttMs
        ? `Direct connection${transportText}, ${participant.fipsSrttMs} ms`
        : `Direct connection${transportText}`;
    }
    if (participant.reachable) {
      return participant.fipsSrttMs ? `Via mesh, ${participant.fipsSrttMs} ms` : 'Via mesh';
    }
    if (participant.state === 'pending') {
      return 'Connecting';
    }
    return 'Offline';
  }

  function isDirectFipsPeer(participant: ParticipantView): boolean {
    return !isSelf(participant) && participant.reachable && participant.fipsTransportAddr.trim() !== '';
  }

  function isFipsRouted(participant: ParticipantView): boolean {
    return !isSelf(participant) && participant.reachable && participant.fipsTransportAddr.trim() === '';
  }

  function fipsPathBadgeText(participant: ParticipantView): string {
    if (isDirectFipsPeer(participant)) {
      return 'direct connection';
    }
    if (isFipsRouted(participant)) {
      return 'via mesh';
    }
    return '';
  }

  function deviceStatusText(participant: ParticipantView): string {
    const pathState = `${participant.state} ${participant.meshState}`.toLowerCase();
    if (
      participant.state === 'local' ||
      participant.meshState === 'local' ||
      pathState.includes('online') ||
      pathState.includes('present')
    ) {
      return 'Online';
    }
    if (pathState.includes('pending')) {
      return 'Connecting';
    }
    if (pathState.includes('off')) {
      return 'Off';
    }
    if (pathState.includes('offline') || pathState.includes('absent')) {
      return 'Offline';
    }
    return nonEmpty(participant.state, 'Unknown');
  }

  function deviceDetailStatusText(participant: ParticipantView): string {
    return deviceStatusText(participant);
  }

  function exitNodeStatusText(participant: ParticipantView): string {
    const status = deviceStatusText(participant);
    return status === 'Unknown' ? 'exit node' : status.toLowerCase();
  }

  function isSelf(participant: ParticipantView): boolean {
    return Boolean(
      state &&
        (participant.pubkeyHex === state.ownPubkeyHex ||
          participant.npub === state.ownNpub),
    );
  }

  function canEditNetwork(network: NetworkView): boolean {
    return network.localIsAdmin;
  }

  function handleCopied(event: CustomEvent<{ label: string }>) {
    setNotice(`${event.detail.label} copied`);
  }

  async function toggleVpn() {
    if (!state || !activeNetwork || !state.vpnControlSupported) {
      return;
    }
    await run(state.vpnEnabled ? '/api/disconnect_vpn' : '/api/connect_vpn', undefined, 'VPN');
  }

  async function addParticipant() {
    if (!shownNetwork) {
      return;
    }
    const npub = participantNpub.trim();
    if (!npub) {
      return;
    }
    const ok = await run(
      '/api/add_participant',
      {
        networkId: shownNetwork.id,
        npub,
        alias: participantAlias.trim() || null,
      },
      'Adding device',
    );
    if (ok) {
      participantNpub = '';
      participantAlias = '';
      addDeviceOpen = false;
    }
  }

  async function resetNetworkInvite(network: NetworkView) {
    if (!window.confirm('Reset this invite link? Devices with the old link will no longer be able to request access.')) {
      return;
    }
    await run('/api/reset_network_invite', { networkId: network.id }, 'Resetting invite');
  }

  async function saveAlias(participant: ParticipantView) {
    await run(
      '/api/set_participant_alias',
      {
        npub: participant.npub,
        alias: aliasDrafts[participant.npub] ?? '',
      },
      'Saving name',
    );
  }

  function endpointHintsFromDraft(value: string): string[] {
    return value
      .split(/[,\n\r\t ]+/)
      .map((item) => item.trim())
      .filter(Boolean);
  }

  async function saveEndpointHints(participant: ParticipantView) {
    const next = endpointHintsFromDraft(endpointHintDrafts[participant.npub] ?? '');
    await run(
      '/api/set_participant_endpoint_hints',
      {
        npub: participant.npub,
        endpointHints: next,
      },
      'Saving address hints',
    );
  }

  async function toggleAdmin(network: NetworkView, participant: ParticipantView) {
    await run(
      participant.isAdmin ? '/api/remove_admin' : '/api/add_admin',
      {
        networkId: network.id,
        npub: participant.npub,
      },
      participant.isAdmin ? 'Removing admin' : 'Adding admin',
    );
  }

  async function removeParticipant(network: NetworkView, participant: ParticipantView) {
    if (!window.confirm(`Remove ${participantName(participant)} from ${network.name}?`)) {
      return;
    }
    await run(
      '/api/remove_participant',
      {
        networkId: network.id,
        npub: participant.npub,
      },
      'Removing device',
    );
  }

  async function setJoinRequests(network: NetworkView, enabled: boolean) {
    await run(
      '/api/set_network_join_requests_enabled',
      {
        networkId: network.id,
        enabled,
      },
      'Updating requests',
    );
  }

  async function acceptJoinRequest(network: NetworkView, request: InboundJoinRequestView) {
    await run(
      '/api/accept_join_request',
      {
        networkId: network.id,
        requesterNpub: request.requesterNpub,
      },
      'Accepting',
    );
  }

  async function rejectJoinRequest(network: NetworkView, request: InboundJoinRequestView) {
    await run(
      '/api/reject_join_request',
      {
        networkId: network.id,
        requesterNpub: request.requesterNpub,
      },
      'Rejecting',
    );
  }

  async function importNetworkInvite(invite: string, label = 'Importing') {
    const existingIds = new Set(state?.networks.map((network) => network.id) ?? []);
    const next = await runState('/api/import_network_invite', { invite }, label);
    if (!next) {
      return false;
    }
    const createdNetwork = next.networks.find((network) => !existingIds.has(network.id));
    shownNetworkId = createdNetwork?.id ?? preferredNetworkId(next, '');
    return true;
  }

  async function importInvite() {
    const invite = inviteDraft.trim();
    if (!invite) {
      return;
    }
    const ok = await importNetworkInvite(invite, 'Importing');
    if (ok) {
      inviteDraft = '';
      closeAddNetwork();
      tab = 'devices';
    }
  }

  async function toggleInviteBroadcast() {
    if (!state) {
      return;
    }
    await run(
      state.inviteBroadcastActive ? '/api/stop_invite_broadcast' : '/api/start_invite_broadcast',
      undefined,
      state.inviteBroadcastActive ? 'Stopping nearby sharing' : 'Sharing nearby',
    );
  }

  async function toggleNearbyDiscovery() {
    if (!state) {
      return;
    }
    await run(
      state.nearbyDiscoveryActive ? '/api/stop_nearby_discovery' : '/api/start_nearby_discovery',
      undefined,
      state.nearbyDiscoveryActive ? 'Stopping nearby lookup' : 'Finding nearby',
    );
  }

  async function addNetwork() {
    const name = newNetworkName.trim();
    if (!name) {
      return;
    }
    const existingIds = new Set(state?.networks.map((network) => network.id) ?? []);
    const next = await runState('/api/add_network', { name }, 'Adding network');
    if (next) {
      const createdNetwork = next.networks.find((network) => !existingIds.has(network.id));
      shownNetworkId = createdNetwork?.id ?? preferredNetworkId(next, '');
      newNetworkName = '';
      closeAddNetwork();
      tab = 'devices';
    }
  }

  async function requestNetworkJoin() {
    const network = addNetworkRequestNetwork;
    if (!network || network.outboundJoinRequest || !network.inviteInviterNpub) {
      return;
    }
    const next = await runState(
      '/api/request_network_join',
      { networkId: network.id },
      'Requesting access',
    );
    if (next) {
      addNetworkJoinStatus = 'Join request sent';
    }
  }

  async function manualAddNetwork() {
    if (!canManualAddNetwork) {
      return;
    }
    const existingIds = new Set(state?.networks.map((network) => network.id) ?? []);
    const next = await runState(
      '/api/manual_add_network',
      {
        adminNpub: manualAdminNpubTrimmed,
        meshNetworkId: manualNetworkIdNormalized,
      },
      'Adding network',
    );
    if (next) {
      const createdNetwork = next.networks.find((network) => !existingIds.has(network.id));
      shownNetworkId = createdNetwork?.id ?? preferredNetworkId(next, '');
      manualAdminNpub = '';
      manualNetworkId = '';
      closeAddNetwork();
      tab = 'devices';
    }
  }

  async function pasteInviteFromClipboard() {
    try {
      inviteDraft = (await navigator.clipboard.readText()).trim();
    } catch (err) {
      error = messageOf(err);
    }
  }

  async function saveNetworkProfile(network: NetworkView) {
    const name = (networkNameDrafts[network.id] ?? '').trim();
    const meshId = normalizeNetworkIdInput(meshIdDrafts[network.id] ?? '');
    let ok = true;
    if (name && name !== network.name) {
      ok = await run('/api/rename_network', { networkId: network.id, name }, 'Saving network');
    }
    if (ok && meshId && meshId !== normalizeNetworkIdInput(network.networkId)) {
      ok = await run(
        '/api/set_network_mesh_id',
        { networkId: network.id, meshId },
        'Saving mesh',
      );
    }
    if (ok) {
      meshIdDrafts = { ...meshIdDrafts, [network.id]: displayNetworkId(meshId || network.networkId) };
    }
    if (ok) {
      setNotice('Network saved');
    }
  }

  async function activateNetwork(network: NetworkView) {
    await run(
      '/api/set_network_enabled',
      { networkId: network.id, enabled: true },
      'Activating network',
    );
  }

  async function removeNetwork(network: NetworkView) {
    if (!window.confirm(`Remove ${network.name}?`)) {
      return;
    }
    await run('/api/remove_network', { networkId: network.id }, 'Removing network');
  }

  async function saveSettings() {
    const listenPort = Number(settingsDraft.listenPort);
    if (!Number.isInteger(listenPort) || listenPort <= 0 || listenPort > 65535) {
      error = 'Listen port must be between 1 and 65535';
      return;
    }
    const ok = await run(
      '/api/update_settings',
      {
        nodeName: settingsDraft.nodeName,
        endpoint: settingsDraft.endpoint,
        tunnelIp: settingsDraft.tunnelIp,
        listenPort,
        relays: settingsDraft.relays
          .split(/[\s,]+/)
          .map((relay) => relay.trim())
          .filter(Boolean),
        advertisedRoutes: settingsDraft.advertisedRoutes,
        fipsHostTunnelEnabled: settingsDraft.fipsHostTunnelEnabled,
        connectToNonRosterFipsPeers: settingsDraft.connectToNonRosterFipsPeers,
        fipsNostrDiscoveryEnabled: settingsDraft.fipsNostrDiscoveryEnabled,
        fipsBootstrapEnabled: settingsDraft.fipsBootstrapEnabled,
        fipsBootstrapPeers: textToBootstrapPeers(settingsDraft.fipsBootstrapPeers),
        fipsHostInboundTcpPorts: settingsDraft.fipsHostInboundTcpPorts,
        autoconnect: settingsDraft.autoconnect,
      },
      'Saving settings',
    );
    if (ok && state) {
      settingsDirty = false;
      syncSettings(state);
    }
  }

  async function setDirectExit() {
    await run(
      '/api/update_settings',
      { exitNode: '', wireguardExitEnabled: false },
      'Updating route',
    );
  }

  async function setExitNode(npub: string) {
    await run('/api/update_settings', { exitNode: npub }, 'Updating route');
  }

  async function setWireGuardExitEnabled(enabled: boolean) {
    await run('/api/update_settings', { wireguardExitEnabled: enabled }, 'Updating route');
  }

  async function setAdvertiseExitNode(enabled: boolean) {
    await run('/api/update_settings', { advertiseExitNode: enabled }, 'Updating exit');
  }

  async function setExitNodeLeakProtection(enabled: boolean) {
    await run('/api/update_settings', { exitNodeLeakProtection: enabled }, 'Updating exit');
  }

  async function saveWireGuardExitConfig() {
    const ok = await run(
      '/api/update_settings',
      { wireguardExitConfig: wireguardExitConfigDraft },
      'Saving WireGuard',
    );
    if (ok && state) {
      wireguardDirty = false;
      syncWireGuard(state);
    }
  }

  async function importWireGuardExitConfigFile(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) {
      return;
    }
    try {
      const config = await file.text();
      if (!config.trim()) {
        error = 'Selected WireGuard config is empty.';
        return;
      }
      wireguardExitConfigDraft = config;
      wireguardDirty = true;
      await saveWireGuardExitConfig();
    } catch (err) {
      error = messageOf(err);
    } finally {
      input.value = '';
    }
  }

  function routeSummary(value: UiState): string {
    if (value.wireguardExitEnabled) {
      return 'WireGuard upstream';
    }
    if (value.exitNode) {
      return 'Peer exit';
    }
    return 'Direct';
  }

  function wireGuardExitSubtitle(value: UiState): string {
    if (!value.wireguardExitConfigured) {
      return 'No config';
    }
    return value.wireguardExitEndpoint || 'Configured';
  }

</script>

<svelte:head>
  <title>Nostr VPN</title>
</svelte:head>

<main class="shell">
  <header class="app-header">
    <div class="brand">
      <img class="brand-mark" src={nostrVpnIcon} alt="" aria-hidden="true" />
      <div>
        <h1>Nostr VPN</h1>
      </div>
    </div>

    {#if state}
      <div class="network-picker">
        <span
          class="network-state-dot"
          class:active={Boolean(shownNetwork?.enabled)}
          aria-label={shownNetwork?.enabled ? 'Selected network active' : 'Selected network inactive'}
          title={shownNetwork?.enabled ? 'Selected network active' : 'Selected network inactive'}
        ></span>
        <select bind:value={shownNetworkId} aria-label="Network">
          {#each state.networks as network (network.id)}
            <option value={network.id}>{network.name}</option>
          {/each}
        </select>
        <button
          type="button"
          class="header-icon-button"
          aria-label="Add Network"
          title="Add Network"
          on:click={openAddNetwork}
        >
          <svg aria-hidden="true" viewBox="0 0 16 16" focusable="false">
            <path d="M8 3.5v9M3.5 8h9" />
          </svg>
        </button>
      </div>

      <div class="header-vpn">
        <span class="header-vpn-text">{state.vpnStatus}</span>
        <span class="status-dot {heroTone(state)}"></span>
        <button
          type="button"
          class="vpn-switch"
          class:active={state.vpnEnabled}
          aria-label={state.vpnEnabled ? 'Turn VPN off' : 'Turn VPN on'}
          disabled={!activeNetwork || !state.vpnControlSupported || Boolean(busyAction)}
          on:click={toggleVpn}
        >
          <span></span>
        </button>
      </div>
    {/if}
  </header>

  <div class="app-body">
    <aside class="sidebar">
      <nav class="nav" aria-label="Primary">
        {#each tabs as item}
          <button
            type="button"
            class:active={tab === item.id}
            class:attention={item.id === 'devices' && incomingJoinRequestCount > 0}
            aria-current={tab === item.id ? 'page' : undefined}
            on:click={() => (tab = item.id)}
          >
            <span>{item.label}</span>
            {#if item.id === 'devices' && incomingJoinRequestCount > 0}
              <span class="nav-attention-dot" aria-hidden="true"></span>
            {/if}
          </button>
        {/each}
      </nav>

      {#if state}
        <div class="sidebar-summary">
          <span class="status-dot {heroTone(state)}"></span>
          <div>
            <strong>{state.vpnEnabled ? 'VPN on' : 'VPN off'}</strong>
            <span>
              {activeNetwork
                ? `${activeNetwork.onlineCount}/${activeNetwork.expectedCount} online`
                : `${state.connectedPeerCount}/${state.expectedPeerCount} online`}
            </span>
          </div>
        </div>
      {/if}
    </aside>

    <section class="main">
    {#if loading}
      <div class="center-state">Loading control panel</div>
    {:else if !state}
      <div class="center-state error-state">{error || 'Control panel unavailable'}</div>
    {:else}
      {#if error || notice || busyAction || refreshing}
        <div class="notice-row" class:error={Boolean(error)}>
          {#if error}
            {error}
          {:else if busyAction}
            {busyAction}
          {:else if notice}
            {notice}
          {:else if refreshing}
            Refreshing
          {/if}
        </div>
      {/if}

      {#if addDeviceOpen && shownNetwork}
        <Modal title="Add Device" titleId="add-device-title" on:close={() => (addDeviceOpen = false)}>
          <div class="modal-section invite-section">
            <div class="qr-frame compact">
              {#if qrInvite}
                <QRCode data={qrInvite} size={320} />
              {:else}
                <div class="qr-empty">QR</div>
              {/if}
            </div>
            <div class="share-copy">
              <div class="section-heading">
                <div>
                  <h3>Invite Devices</h3>
                  <p>{shownNetwork.name}</p>
                </div>
              </div>
              <div class="button-row">
                <CopyButton
                  variant="secondary"
                  value={qrInvite}
                  label="Invite"
                  text="Copy Link"
                  disabled={!qrInvite}
                  on:copied={handleCopied}
                />
                <button
                  type="button"
                  class="small-button"
                  disabled={!shownNetwork.localIsAdmin || !shownNetwork.enabled || Boolean(busyAction)}
                  on:click={() => resetNetworkInvite(shownNetwork)}
                >
                  Reset
                </button>
                <button
                  type="button"
                  class="secondary-button"
                  disabled={!shownNetwork.enabled}
                  on:click={toggleInviteBroadcast}
                >
                  {state.inviteBroadcastActive
                    ? `Sharing nearby · ${remainingText(state.inviteBroadcastRemainingSecs)}`
                    : 'Share invite nearby'}
                </button>
                <label class="switch-row inline-switch">
                  <span>Allow requests</span>
                  <input
                    type="checkbox"
                    checked={shownNetwork.joinRequestsEnabled}
                    disabled={!shownNetwork.localIsAdmin || Boolean(busyAction)}
                    on:change={(event) =>
                      setJoinRequests(shownNetwork, (event.currentTarget as HTMLInputElement).checked)}
                  />
                </label>
              </div>
            </div>
          </div>

          {#if shownNetwork.inboundJoinRequests.length > 0}
            <div class="modal-section join-requests-list">
              <div class="section-heading compact">
                <div>
                  <h3>Requests</h3>
                  <p>{shownNetwork.inboundJoinRequests.length}</p>
                </div>
              </div>
              <div class="stack">
                {#each shownNetwork.inboundJoinRequests as request (request.requesterPubkeyHex || request.requesterNpub)}
                  <div class="request-row">
                    <div>
                      <strong>{nonEmpty(request.requesterNodeName, shortMiddle(request.requesterNpub, 20))}</strong>
                      <span>{request.requestedAtText}</span>
                    </div>
                    <div class="row-actions">
                      <button type="button" class="small-button" on:click={() => acceptJoinRequest(shownNetwork, request)}>
                        Accept
                      </button>
                      <button type="button" class="small-button danger" on:click={() => rejectJoinRequest(shownNetwork, request)}>
                        Reject
                      </button>
                    </div>
                  </div>
                {/each}
              </div>
            </div>
          {/if}

          <div class="modal-section">
            <div class="section-heading">
              <div>
                <h3>For Manual Join</h3>
                <p>{shownNetwork.name}</p>
              </div>
            </div>
            <div class="detail-list two-column">
              <div>
                <span>Your Device ID</span>
                <strong>{state.ownNpub}</strong>
                <CopyButton value={state.ownNpub} label="Device ID" on:copied={handleCopied} />
              </div>
              <div>
                <span>Network ID</span>
                <strong>{displayNetworkId(shownNetwork.networkId)}</strong>
                <CopyButton value={shownNetwork.networkId} label="Network ID" on:copied={handleCopied} />
              </div>
            </div>
          </div>

          <form class="modal-section" on:submit|preventDefault={addParticipant}>
            <div class="section-heading">
              <div>
                <h3>Add by Device ID</h3>
                <p>{shownNetwork.name}</p>
              </div>
            </div>
            <div class="form-grid">
              <label>
                <span>Device ID</span>
                <input bind:value={participantNpub} autocomplete="off" />
              </label>
              <label>
                <span>Name</span>
                <input bind:value={participantAlias} autocomplete="off" />
              </label>
            </div>
            <button class="secondary-button" type="submit" disabled={Boolean(busyAction)}>
              Add
            </button>
          </form>
        </Modal>
      {/if}

      {#if addNetworkOpen}
        <Modal title="Add Network" titleId="add-network-title" on:close={closeAddNetwork}>
          <form class="modal-section" on:submit|preventDefault={addNetwork}>
            <div class="section-heading">
              <div>
                <h3>Create Network</h3>
                <p>New local roster</p>
              </div>
            </div>
            <div class="inline-form">
              <input bind:value={newNetworkName} autocomplete="off" aria-label="Network name" placeholder="Network name" />
              <button type="submit" class="small-button" disabled={Boolean(busyAction) || !newNetworkName.trim()}>
                Create
              </button>
            </div>
          </form>

          <form class="modal-section" on:submit|preventDefault={importInvite}>
            <div class="section-heading">
              <div>
                <h3>Join Network</h3>
                <p>Paste an invite</p>
              </div>
            </div>
            <label>
              <span>Invite</span>
              <textarea bind:value={inviteDraft} rows="6"></textarea>
            </label>
            <div class="button-row">
              <button class="secondary-button" type="submit" disabled={Boolean(busyAction) || !inviteDraft.trim()}>
                Join
              </button>
              <button class="small-button" type="button" on:click={pasteInviteFromClipboard}>
                Paste
              </button>
            </div>
            {#if addNetworkRequestNetwork}
              <div class="badge-row">
                {#if addNetworkJoinRequestSent}
                  <span class="badge warn">Join request sent</span>
                {:else if addNetworkRequestNetwork.inviteInviterNpub}
                  <button
                    class="secondary-button"
                    type="button"
                    disabled={Boolean(busyAction)}
                    on:click={requestNetworkJoin}
                  >
                    Request Access
                  </button>
                {/if}
              </div>
            {/if}
          </form>

          <form class="modal-section" on:submit|preventDefault={manualAddNetwork}>
            <div class="section-heading">
              <div>
                <h3>Add manually</h3>
                <p>Admin Device ID + Network ID</p>
              </div>
            </div>
            <div class="form-grid">
              <label>
                <span>Admin Device ID</span>
                <input
                  bind:value={manualAdminNpub}
                  class:invalid={manualAdminInvalid}
                  autocomplete="off"
                />
              </label>
              <label>
                <span>Network ID</span>
                <input bind:value={manualNetworkId} autocomplete="off" />
              </label>
            </div>
            {#if manualAdminInvalid}
              <div class="field-error">Not a valid device ID</div>
            {/if}
            <button
              class="secondary-button"
              type="submit"
              disabled={Boolean(busyAction) || !canManualAddNetwork}
            >
              Add
            </button>
          </form>

          <div class="modal-section">
            <div class="section-heading">
              <div>
                <h3>Nearby Invites</h3>
                <p>{state.lanPeers.length}</p>
              </div>
              <button
                type="button"
                class="small-button"
                on:click={toggleNearbyDiscovery}
              >
                {state.nearbyDiscoveryActive
                  ? `Finding nearby · ${remainingText(state.nearbyDiscoveryRemainingSecs)}`
                  : 'Find nearby'}
              </button>
            </div>
            {#if state.lanPeers.length === 0}
              <div class="empty-state">No nearby invites yet</div>
            {:else}
              <div class="stack">
                {#each state.lanPeers as peer, index (index)}
                  <div class="request-row">
                    <div>
                      <strong>{peer.nodeName || peer.networkName || 'Nearby device'}</strong>
                      <span>{peer.lastSeenText ?? ''}</span>
                    </div>
                    {#if peer.invite}
                      <button type="button" class="small-button" on:click={() => importNetworkInvite(peer.invite ?? '', 'Joining')}>
                        Join
                      </button>
                    {/if}
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        </Modal>
      {/if}

      {#if tab === 'devices'}
        <section
          class="devices-layout"
          class:showing-detail={devicePane === 'detail' && Boolean(selectedParticipant)}
        >
          <div class="device-list-column">
            <div class="list-header">
              <div>
                <h2>Devices</h2>
                <p>{shownNetwork ? `${shownNetwork.onlineCount}/${shownNetwork.expectedCount} online` : 'No network'}</p>
              </div>
              {#if shownNetwork}
                <div class="header-actions">
                  {#if !shownNetwork.enabled}
                    <button type="button" class="small-button primary" on:click={() => activateNetwork(shownNetwork)}>
                      Activate
                    </button>
                  {/if}
                  {#if shownNetwork.localIsAdmin}
                    <button
                      type="button"
                      class="small-button"
                      disabled={!shownNetwork.enabled}
                      aria-label={shownNetwork.enabled ? 'Add Device' : 'Activate network before adding a device'}
                      title={shownNetwork.enabled ? 'Add Device' : 'Activate network first'}
                      on:click={() => (addDeviceOpen = true)}
                    >
                      Add Device
                    </button>
                  {/if}
                </div>
              {/if}
            </div>

            {#if showDeviceSearch}
              <label class="search-field">
                <span>Search</span>
                <input bind:value={deviceSearch} autocomplete="off" />
              </label>
            {/if}

            <div class="device-list">
              {#if !shownNetwork}
                <div class="empty-state">No network</div>
              {:else if visibleParticipants.length === 0}
                <div class="empty-state">{deviceSearchQuery ? 'No matching devices' : 'No devices'}</div>
              {:else}
                <div class="network-label">{shownNetwork.name}</div>
                {#each visibleParticipants as participant (participant.pubkeyHex || participant.npub)}
                  <button
                    type="button"
                    class="device-list-row"
                    class:active={participantSelected(participant)}
                    on:click={() => openParticipant(participant)}
                  >
                    <span class="device-list-main">
                      <span class="device-title">
                        <span class="status-dot {participantTone(participant)}"></span>
                        <strong>{participantName(participant)}</strong>
                        {#if isSelf(participant)}
                          <span class="badge active">Self</span>
                        {/if}
                        {#if participant.isAdmin}
                          <span class="badge muted">Admin</span>
                        {/if}
                        {#if exitNodeBadgeText(participant)}
                          <span
                            class="badge"
                            class:active={isActiveExitParticipant(participant)}
                            class:warn={!isActiveExitParticipant(participant)}
                          >
                            {exitNodeBadgeText(participant)}
                          </span>
                        {/if}
                        {#if isFipsRouted(participant)}
                          <span class="badge muted">via mesh</span>
                        {/if}
                      </span>
                      <span class="device-meta">
                        <span>{nonEmpty(participant.tunnelIp)}</span>
                      </span>
                    </span>
                  </button>
                {/each}
              {/if}
            </div>

            {#if shownNetwork && shownNetwork.inboundJoinRequests.length > 0}
              <div class="join-requests-list">
                <div class="section-heading compact">
                  <div>
                    <h3>Requests</h3>
                    <p>{shownNetwork.inboundJoinRequests.length}</p>
                  </div>
                </div>
                <div class="stack">
                  {#each shownNetwork.inboundJoinRequests as request (request.requesterPubkeyHex)}
                    <div class="request-row">
                      <div>
                        <strong>{nonEmpty(request.requesterNodeName, shortMiddle(request.requesterNpub, 20))}</strong>
                        <span>{request.requestedAtText}</span>
                      </div>
                      <div class="row-actions">
                        <button type="button" class="small-button" on:click={() => acceptJoinRequest(shownNetwork, request)}>
                          Accept
                        </button>
                        <button type="button" class="small-button danger" on:click={() => rejectJoinRequest(shownNetwork, request)}>
                          Reject
                        </button>
                      </div>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}
          </div>

          <div class="device-detail-column">
            {#if !shownNetwork}
              <div class="detail-empty">
                <h2>Devices</h2>
                <div class="empty-state">No network</div>
              </div>
            {:else if selectedParticipant}
              <div class="detail-stack">
                <header class="detail-header">
                  <button
                    type="button"
                    class="small-button device-detail-back"
                    aria-label="Back to Devices"
                    on:click={showDeviceList}
                  >
                    Back
                  </button>
                  <div>
                    <h2>{participantName(selectedParticipant)}</h2>
                    {#if isSelf(selectedParticipant) || selectedParticipant.isAdmin || exitNodeBadgeText(selectedParticipant) || fipsPathBadgeText(selectedParticipant)}
                      <div class="badge-row">
                        {#if isSelf(selectedParticipant)}
                          <span class="badge active">Self</span>
                        {/if}
                        {#if selectedParticipant.isAdmin}
                          <span class="badge muted">Admin</span>
                        {/if}
                        {#if exitNodeBadgeText(selectedParticipant)}
                          <span
                            class="badge"
                            class:active={isActiveExitParticipant(selectedParticipant)}
                            class:warn={!isActiveExitParticipant(selectedParticipant)}
                          >
                            {exitNodeBadgeText(selectedParticipant)}
                          </span>
                        {/if}
                        {#if fipsPathBadgeText(selectedParticipant)}
                          <span
                            class="badge"
                            class:active={isDirectFipsPeer(selectedParticipant)}
                            class:muted={isFipsRouted(selectedParticipant)}
                          >
                            {fipsPathBadgeText(selectedParticipant)}
                          </span>
                        {/if}
                      </div>
                    {/if}
                  </div>
                  <div class="detail-status">
                    <span class="status-dot {participantTone(selectedParticipant)}"></span>
                    <span>{deviceDetailStatusText(selectedParticipant)}</span>
                  </div>
                </header>

                {#if shownNetwork.localIsAdmin}
                  <div class="detail-surface">
                    <div class="section-heading">
                      <div>
                        <h3>Manage Device</h3>
                        <p>{shownNetwork.name}</p>
                      </div>
                    </div>
                    <div class="inline-form">
                      <input
                        aria-label="Device name"
                        bind:value={aliasDrafts[selectedParticipant.npub]}
                        disabled={Boolean(busyAction)}
                      />
                      <button
                        type="button"
                        class="small-button"
                        disabled={Boolean(busyAction)}
                        on:click={() => saveAlias(selectedParticipant)}
                      >
                        Save
                      </button>
                    </div>
                    {#if !isSelf(selectedParticipant)}
                      <div class="inline-form">
                        <input
                          aria-label="FIPS address hints"
                          placeholder="host or host:port"
                          bind:value={endpointHintDrafts[selectedParticipant.npub]}
                          disabled={Boolean(busyAction)}
                        />
                        <button
                          type="button"
                          class="small-button"
                          disabled={Boolean(busyAction)}
                          on:click={() => saveEndpointHints(selectedParticipant)}
                        >
                          Save hints
                        </button>
                      </div>
                      <div class="button-row">
                        <button
                          type="button"
                          class="small-button"
                          on:click={() => toggleAdmin(shownNetwork, selectedParticipant)}
                        >
                          {selectedParticipant.isAdmin ? 'Remove admin' : 'Make admin'}
                        </button>
                        <button
                          type="button"
                          class="small-button danger"
                          on:click={() => removeParticipant(shownNetwork, selectedParticipant)}
                        >
                          Remove
                        </button>
                      </div>
                    {/if}
                  </div>
                {/if}

                <div class="detail-surface">
                  <div class="section-heading">
                    <div>
                      <h3>Addresses</h3>
                      <p>{deviceRoleText(selectedParticipant)}</p>
                    </div>
                  </div>
                  <div class="detail-list">
                    <div>
                      <span>MagicDNS</span>
                      <strong>{nonEmpty(deviceMagicDnsName(selectedParticipant))}</strong>
                    </div>
                    <div>
                      <span>VPN IP</span>
                      <strong>{nonEmpty(selectedParticipant.tunnelIp)}</strong>
                    </div>
                    <div class="detail-copy-row">
                      <div>
                        <span>Device ID</span>
                        <strong>{selectedParticipant.npub}</strong>
                      </div>
                      <CopyButton value={selectedParticipant.npub} label="Device ID" on:copied={handleCopied} />
                    </div>
                  </div>
                </div>

                <div class="detail-surface">
                  <div class="section-heading">
                    <div>
                      <h3>Connectivity</h3>
                    </div>
                  </div>
                  <div class="metric-grid">
                    <div>
                      <span>Role</span>
                      <strong>{deviceRoleText(selectedParticipant)}</strong>
                    </div>
                    <div>
                      <span>State</span>
                      <strong>{deviceStatusText(selectedParticipant)}</strong>
                    </div>
                    <div>
                      <span>FIPS path</span>
                      <strong>{fipsPathText(selectedParticipant)}</strong>
                    </div>
                    {#if selectedParticipant.fipsTransportAddr.trim()}
                      <div class="metric-wide">
                        <span>Endpoint</span>
                        <strong>{selectedParticipant.fipsTransportAddr}</strong>
                      </div>
                    {/if}
                    {#if selectedParticipant.fipsEndpointHints?.length}
                      <div class="metric-wide">
                        <span>Address hints</span>
                        <strong>{selectedParticipant.fipsEndpointHints.join(', ')}</strong>
                      </div>
                    {/if}
                    <div>
                      <span>Last seen</span>
                      <strong>{nonEmpty(selectedParticipant.lastSeenText)}</strong>
                    </div>
                    <div>
                      <span>Sent</span>
                      <strong>{formatBytes(selectedParticipant.txBytes)}</strong>
                    </div>
                    <div>
                      <span>Received</span>
                      <strong>{formatBytes(selectedParticipant.rxBytes)}</strong>
                    </div>
                    <div>
                      <span>FIPS packets</span>
                      <strong>{selectedParticipant.fipsPacketsSent}/{selectedParticipant.fipsPacketsRecv}</strong>
                    </div>
                  </div>
                </div>

              </div>
            {:else}
              <div class="detail-empty">
                <h2>Devices</h2>
                <div class="empty-state">No devices</div>
              </div>
            {/if}
          </div>
        </section>
      {:else if tab === 'exit'}
        <section class="exit-page-grid">
          <div class="panel wide">
            <div class="section-heading">
              <div>
                <h3>Route</h3>
                <p>{routeSummary(state)}</p>
              </div>
            </div>
            {#if showExitSearch}
              <label>
                <span>Search</span>
                <input bind:value={exitSearch} autocomplete="off" />
              </label>
            {/if}

            <div class="choice-list">
              <button
                type="button"
                class:active={!state.exitNode && !state.wireguardExitEnabled}
                class="choice-row"
                on:click={setDirectExit}
              >
                <span class="radio-dot"></span>
                <div>
                  <strong>Direct</strong>
                  <span>Normal internet route</span>
                </div>
              </button>

              <button
                type="button"
                class:active={state.wireguardExitEnabled}
                class="choice-row"
                disabled={!state.wireguardExitConfigured}
                on:click={() => setWireGuardExitEnabled(true)}
              >
                <span class="radio-dot"></span>
                <div>
                  <strong>WireGuard upstream</strong>
                  <span>{wireGuardExitSubtitle(state)}</span>
                </div>
              </button>

              {#each exitCandidates as participant (participant.pubkeyHex || participant.npub)}
                <button
                  type="button"
                  class:active={!state.wireguardExitEnabled && state.exitNode === participant.npub}
                  class="choice-row"
                  on:click={() => setExitNode(participant.npub)}
                >
                  <span class="radio-dot"></span>
                  <div>
                    <strong>{participantName(participant)}</strong>
                    <span>{exitNodeStatusText(participant)}</span>
                  </div>
                </button>
              {:else}
                <div class="empty-state">{exitSearchQuery ? 'No exit nodes found' : 'No exit nodes offered'}</div>
              {/each}
            </div>
          </div>

          <div class="panel-stack">
            <div class="panel">
              <div class="section-heading">
                <div>
                  <h3>This Device</h3>
                  <p>Exit node</p>
                </div>
              </div>
              <label class="switch-row">
                <span>Offer exit</span>
                <input
                  type="checkbox"
                  checked={state.advertiseExitNode}
                  on:change={(event) =>
                    setAdvertiseExitNode((event.currentTarget as HTMLInputElement).checked)}
                />
              </label>
              <label class="switch-row">
                <span>Block internet if exit node disconnects</span>
                <input
                  type="checkbox"
                  checked={state.exitNodeLeakProtection}
                  on:change={(event) =>
                    setExitNodeLeakProtection((event.currentTarget as HTMLInputElement).checked)}
                />
              </label>
            </div>

            <form class="panel wide" on:submit|preventDefault={saveWireGuardExitConfig}>
              <div class="section-heading">
                <div>
                  <h3>WireGuard Upstream</h3>
                  <p>{wireGuardExitSubtitle(state)}</p>
                </div>
              </div>
              <label class="switch-row">
                <span>Enabled</span>
                <input
                  type="checkbox"
                  checked={state.wireguardExitEnabled}
                  disabled={!state.wireguardExitConfigured}
                  on:change={(event) =>
                    setWireGuardExitEnabled((event.currentTarget as HTMLInputElement).checked)}
                />
              </label>
              <label>
                <span>Config</span>
                <textarea
                  class="code-textarea"
                  bind:value={wireguardExitConfigDraft}
                  on:input={() => (wireguardDirty = true)}
                  rows="10"
                ></textarea>
              </label>
              <div class="button-row">
                <input
                  bind:this={wireguardConfigFileInput}
                  type="file"
                  accept=".conf,.txt,text/*,application/octet-stream,*/*"
                  hidden
                  on:change={importWireGuardExitConfigFile}
                />
                <button
                  type="button"
                  class="secondary-button"
                  disabled={Boolean(busyAction)}
                  on:click={() => wireguardConfigFileInput?.click()}
                >
                  Import file
                </button>
                <button type="submit" class="secondary-button" disabled={Boolean(busyAction)}>
                  Save
                </button>
                <span class="form-status">{state.exitNodeStatusText}</span>
              </div>
            </form>
          </div>
        </section>
      {:else if tab === 'settings'}
        <section class="page-grid settings-grid">
          <form class="settings-form" on:submit|preventDefault={saveSettings}>
            <div class="panel wide">
              <div class="section-heading">
                <div>
                  <h3>This Device</h3>
                  <p>{state.selfMagicDnsName || state.nodeId}</p>
                </div>
              </div>

              <div class="form-grid">
                <label>
                  <span>Name</span>
                  <input bind:value={settingsDraft.nodeName} on:input={() => (settingsDirty = true)} />
                </label>
                <label>
                  <span>Tunnel IP</span>
                  <input bind:value={settingsDraft.tunnelIp} on:input={() => (settingsDirty = true)} />
                </label>
                <label>
                  <span>Endpoint</span>
                  <input bind:value={settingsDraft.endpoint} on:input={() => (settingsDirty = true)} />
                </label>
                <label>
                  <span>Listen Port</span>
                  <input inputmode="numeric" bind:value={settingsDraft.listenPort} on:input={() => (settingsDirty = true)} />
                </label>
                <label>
                  <span>Advertised Routes</span>
                  <input bind:value={settingsDraft.advertisedRoutes} on:input={() => (settingsDirty = true)} />
                </label>
              </div>
            </div>

            <div class="panel wide">
              <div class="section-heading">
                <div>
                  <h3>General</h3>
                  <p>App behavior and relays</p>
                </div>
              </div>

              <label class="switch-row">
                <span>Start VPN automatically</span>
                <input
                  type="checkbox"
                  bind:checked={settingsDraft.autoconnect}
                  on:change={() => (settingsDirty = true)}
                />
              </label>

              <div class="relay-list">
                {#each state.relays ?? [] as relay}
                  <div class="relay-row">
                    <span class="status-dot {relay.status === 'connected' ? 'ok' : 'muted'}"></span>
                    <span>{relay.url}</span>
                  </div>
                {/each}
              </div>

              <label>
                <span>Relays</span>
                <textarea bind:value={settingsDraft.relays} on:input={() => (settingsDirty = true)} rows="4"></textarea>
              </label>
            </div>

            <div class="panel wide">
              <div class="section-heading">
                <div>
                  <h3>FIPS</h3>
                </div>
              </div>

              <label class="switch-row">
                <span>Connect to non-roster FIPS peers</span>
                <input
                  type="checkbox"
                  bind:checked={settingsDraft.connectToNonRosterFipsPeers}
                  on:change={() => (settingsDirty = true)}
                />
              </label>

              <label class="switch-row">
                <span>Find peers over Nostr relays</span>
                <input
                  type="checkbox"
                  bind:checked={settingsDraft.fipsNostrDiscoveryEnabled}
                  on:change={() => (settingsDirty = true)}
                />
              </label>

              <label class="switch-row">
                <span>Use bootstrap servers</span>
                <input
                  type="checkbox"
                  bind:checked={settingsDraft.fipsBootstrapEnabled}
                  on:change={() => (settingsDirty = true)}
                />
              </label>

              {#if settingsDraft.fipsBootstrapEnabled}
                <label>
                  <span>Bootstrap servers</span>
                  <textarea
                    rows="4"
                    bind:value={settingsDraft.fipsBootstrapPeers}
                    on:input={() => (settingsDirty = true)}
                  ></textarea>
                </label>
                <button type="button" class="secondary-button" on:click={resetBootstrapPeers}>
                  Reset to defaults
                </button>
              {/if}
            </div>

            <div class="panel wide">
              <div class="section-heading">
                <div>
                  <h3>Public FIPS routing</h3>
                </div>
              </div>

              <p class="settings-panel-help">
                FIPS gives .fips addresses end-to-end encryption and identity-based routing. Hosts can reach each other
                without static IPs, domain names, TLS certificates, or NAT port forwarding.
                <a href="https://learn.fips.network/" target="_blank" rel="noreferrer">Learn FIPS</a>.
              </p>

              <label class="switch-row">
                <span>Route npub.fips outside VPN</span>
                <input
                  type="checkbox"
                  bind:checked={settingsDraft.fipsHostTunnelEnabled}
                  on:change={() => (settingsDirty = true)}
                />
              </label>

              <div class="dependent-settings" class:disabled={!settingsDraft.fipsHostTunnelEnabled}>
                <div class="readonly-value-row">
                  <span>Your public FIPS address</span>
                  <div class="value-with-action">
                    <input
                      aria-label="Your public FIPS address"
                      value={publicFipsAddress}
                      disabled={!settingsDraft.fipsHostTunnelEnabled}
                      readonly
                    />
                    <CopyButton
                      value={publicFipsAddress}
                      label="Public FIPS address"
                      disabled={!settingsDraft.fipsHostTunnelEnabled}
                      on:copied={handleCopied}
                    />
                  </div>
                </div>

                <label>
                  <span>Public .fips inbound TCP ports</span>
                  <input
                    bind:value={settingsDraft.fipsHostInboundTcpPorts}
                    disabled={!settingsDraft.fipsHostTunnelEnabled}
                    on:input={() => (settingsDirty = true)}
                  />
                </label>
              </div>

              <div class="settings-actions">
                <button type="submit" class="secondary-button" disabled={Boolean(busyAction) || !settingsDirty}>
                  Save settings
                </button>
              </div>
            </div>
          </form>

          <div class="panel wide">
            <div class="section-heading">
              <div>
                <h3>Networks</h3>
                <p>{state.networks.length}</p>
              </div>
            </div>
            <form class="inline-form" on:submit|preventDefault={addNetwork}>
              <input bind:value={newNetworkName} autocomplete="off" aria-label="Network name" />
              <button type="submit" class="small-button">Add</button>
            </form>
            <div class="network-list">
              {#each state.networks as network (network.id)}
                <form class="network-row" on:submit|preventDefault={() => saveNetworkProfile(network)}>
                  <div class="network-fields">
                    <label>
                      <span>Name</span>
                      <input bind:value={networkNameDrafts[network.id]} disabled={!canEditNetwork(network)} />
                    </label>
                    <label>
                      <span>Mesh ID</span>
                      <input bind:value={meshIdDrafts[network.id]} disabled={!canEditNetwork(network)} />
                    </label>
                  </div>
                  <div class="row-actions">
                    {#if network.enabled}
                      <span class="badge ok">Active</span>
                    {:else}
                      <button type="button" class="small-button primary" on:click={() => activateNetwork(network)}>
                        Activate
                      </button>
                    {/if}
                    {#if canEditNetwork(network)}
                      <button type="submit" class="small-button">Save</button>
                    {/if}
                    <button type="button" class="small-button danger" on:click={() => removeNetwork(network)}>
                      Remove
                    </button>
                  </div>
                </form>
              {/each}
            </div>
          </div>

          <div class="panel">
            <div class="section-heading">
              <div>
                <h3>System</h3>
                <p>{state.appVersion}</p>
              </div>
            </div>
            <div class="detail-list">
              <div>
                <span>Config</span>
                <strong>{state.configPath}</strong>
              </div>
              <div>
                <span>Daemon</span>
                <strong>{state.daemonBinaryVersion || '-'}</strong>
              </div>
              <div>
                <span>Interface</span>
                <strong>{state.network.defaultInterface ?? '-'}</strong>
              </div>
              <div>
                <span>Gateway</span>
                <strong>{state.network.gatewayIpv4 ?? state.network.gatewayIpv6 ?? '-'}</strong>
              </div>
              <div>
                <span>Port map</span>
                <strong>{state.portMapping.activeProtocol ?? '-'}</strong>
              </div>
              <div>
                <span>External</span>
                <strong>{state.portMapping.externalEndpoint ?? '-'}</strong>
              </div>
            </div>
          </div>

          <div class="panel diagnostics-panel">
            <button
              type="button"
              class="section-toggle"
              aria-expanded={diagnosticsOpen}
              on:click={() => (diagnosticsOpen = !diagnosticsOpen)}
            >
              <div>
                <h3>Diagnostics</h3>
                <p>{state.health.length > 0 ? `${state.health.length} issues` : 'Healthy'}</p>
              </div>
              <svg
                class:open={diagnosticsOpen}
                class="chevron-icon"
                aria-hidden="true"
                viewBox="0 0 16 16"
                focusable="false"
              >
                <path d="M4 6l4 4 4-4" />
              </svg>
            </button>
            {#if diagnosticsOpen}
              <div class="diagnostics-body">
                <div class="detail-list">
                  <div>
                    <span>Remote roster</span>
                    <strong>{state.connectedPeerCount}/{state.expectedPeerCount}</strong>
                  </div>
                  <div>
                    <span>Roster FIPS</span>
                    <strong>{state.fipsConnectedPeerCount}/{state.fipsRosterPeerCount} direct</strong>
                  </div>
                  <div>
                    <span>Other FIPS</span>
                    <strong>{state.nonFipsRosterPeerCount}</strong>
                  </div>
                </div>
                {#if state.health.length === 0}
                  <div class="empty-state">No health issues</div>
                {:else}
                  <div class="stack">
                    {#each state.health as issue (issue.code)}
                      <div class="issue-row">
                        <span class="status-dot {issueTone(issue)}"></span>
                        <div>
                          <strong>{issue.summary}</strong>
                          <span>{issue.detail}</span>
                        </div>
                      </div>
                    {/each}
                  </div>
                {/if}
              </div>
            {/if}
          </div>
        </section>
      {/if}
    {/if}
    </section>
  </div>
</main>
