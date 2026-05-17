<script lang="ts">
  import { onMount } from 'svelte';
  import './app.css';
  import { qrMatrix, runAction, tick } from './lib/api';
  import { formatBytes, nonEmpty, remainingText, routeList, shortMiddle } from './lib/format';
  import type {
    HealthIssue,
    InboundJoinRequestView,
    NetworkView,
    ParticipantView,
    QrMatrix,
    UiState,
  } from './lib/types';

  type Tab = 'devices' | 'exit' | 'settings';
  type Tone = 'ok' | 'warn' | 'bad' | 'muted' | 'active';

  const tabs: { id: Tab; label: string }[] = [
    { id: 'devices', label: 'Devices' },
    { id: 'exit', label: 'Exit Nodes' },
    { id: 'settings', label: 'Settings' },
  ];

  let state: UiState | null = null;
  let tab: Tab = 'devices';
  let loading = true;
  let refreshing = false;
  let busyAction = '';
  let error = '';
  let notice = '';
  let qr: QrMatrix | null = null;
  let qrInvite = '';
  let settingsDirty = false;
  let participantNpub = '';
  let participantAlias = '';
  let inviteDraft = '';
  let newNetworkName = '';
  let addNetworkOpen = false;
  let addDeviceOpen = false;
  let exitSearch = '';
  let aliasDrafts: Record<string, string> = {};
  let networkNameDrafts: Record<string, string> = {};
  let meshIdDrafts: Record<string, string> = {};
  let noticeTimer: number | undefined;

  let settingsDraft = {
    nodeName: '',
    endpoint: '',
    tunnelIp: '',
    listenPort: '',
    advertisedRoutes: '',
    magicDnsSuffix: '',
    autoconnect: false,
  };

  $: activeNetwork = state ? state.networks.find((network) => network.enabled) ?? state.networks[0] ?? null : null;
  $: participants = activeNetwork?.participants ?? [];
  $: exitCandidates = participants.filter((participant) => {
    const query = exitSearch.trim().toLowerCase();
    if (!query) {
      return true;
    }
    return (
      participantName(participant).toLowerCase().includes(query) ||
      participant.npub.toLowerCase().includes(query) ||
      participant.tunnelIp.toLowerCase().includes(query)
    );
  });
  $: {
    const invite = state?.activeNetworkInvite ?? '';
    if (invite !== qrInvite) {
      qrInvite = invite;
      qr = null;
      if (invite) {
        void loadQr(invite);
      }
    }
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
    state = next;
    loading = false;
    error = '';
    if (firstState || !settingsDirty) {
      syncSettings(next);
    }
    syncNetworkDrafts(next);
    syncAliasDrafts(next);
  }

  function syncSettings(next: UiState) {
    settingsDraft = {
      nodeName: next.nodeName,
      endpoint: next.endpoint,
      tunnelIp: next.tunnelIp,
      listenPort: String(next.listenPort || ''),
      advertisedRoutes: next.advertisedRoutes.join(', '),
      magicDnsSuffix: next.magicDnsSuffix,
      autoconnect: next.autoconnect,
    };
  }

  function syncNetworkDrafts(next: UiState) {
    const names = { ...networkNameDrafts };
    const meshIds = { ...meshIdDrafts };
    for (const network of next.networks) {
      if (!(network.id in names)) {
        names[network.id] = network.name;
      }
      if (!(network.id in meshIds)) {
        meshIds[network.id] = network.networkId;
      }
    }
    networkNameDrafts = names;
    meshIdDrafts = meshIds;
  }

  function syncAliasDrafts(next: UiState) {
    const aliases = { ...aliasDrafts };
    for (const network of next.networks) {
      for (const participant of network.participants) {
        if (!(participant.npub in aliases)) {
          aliases[participant.npub] = participant.magicDnsAlias;
        }
      }
    }
    aliasDrafts = aliases;
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

  async function run(endpoint: string, payload?: unknown, label = 'Working'): Promise<boolean> {
    busyAction = label;
    error = '';
    try {
      const next = await runAction(endpoint, payload);
      applyState(next);
      setNotice(next.vpnStatus || 'Updated');
      return true;
    } catch (err) {
      error = messageOf(err);
      return false;
    } finally {
      busyAction = '';
    }
  }

  async function loadQr(invite: string) {
    try {
      const matrix = await qrMatrix(invite);
      if (qrInvite === invite) {
        qr = matrix;
      }
    } catch {
      if (qrInvite === invite) {
        qr = { width: 0, cells: [] };
      }
    }
  }

  function setNotice(value: string) {
    notice = value;
    window.clearTimeout(noticeTimer);
    noticeTimer = window.setTimeout(() => {
      notice = '';
    }, 2600);
  }

  function messageOf(value: unknown): string {
    if (value instanceof Error) {
      return value.message;
    }
    return String(value);
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
    if (stateText.includes('online') || stateText.includes('present')) {
      return 'ok';
    }
    if (stateText.includes('offline') || stateText.includes('absent')) {
      return 'bad';
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
    return nonEmpty(
      participant.magicDnsAlias || participant.magicDnsName,
      shortMiddle(participant.npub, 22),
    );
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

  async function copyText(value: string, label: string) {
    if (!value) {
      return;
    }
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      const area = document.createElement('textarea');
      area.value = value;
      area.style.position = 'fixed';
      area.style.opacity = '0';
      document.body.append(area);
      area.select();
      document.execCommand('copy');
      area.remove();
    }
    setNotice(`${label} copied`);
  }

  async function copyOwnNpub() {
    if (state?.ownNpub) {
      await copyText(state.ownNpub, 'Device ID');
    }
  }

  async function copyInvite() {
    if (state?.activeNetworkInvite) {
      await copyText(state.activeNetworkInvite, 'Invite');
    }
  }

  async function toggleVpn() {
    if (!state || !activeNetwork || !state.vpnControlSupported) {
      return;
    }
    await run(state.vpnEnabled ? '/api/disconnect_vpn' : '/api/connect_vpn', undefined, 'VPN');
  }

  async function addParticipant() {
    if (!activeNetwork) {
      return;
    }
    const npub = participantNpub.trim();
    if (!npub) {
      return;
    }
    const ok = await run(
      '/api/add_participant',
      {
        networkId: activeNetwork.id,
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

  async function requestJoin(network: NetworkView) {
    await run('/api/request_network_join', { networkId: network.id }, 'Requesting');
  }

  async function importInvite() {
    const invite = inviteDraft.trim();
    if (!invite) {
      return;
    }
    const ok = await run('/api/import_network_invite', { invite }, 'Importing');
    if (ok) {
      inviteDraft = '';
      addNetworkOpen = false;
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
      'Broadcast',
    );
  }

  async function toggleNearbyDiscovery() {
    if (!state) {
      return;
    }
    await run(
      state.nearbyDiscoveryActive ? '/api/stop_nearby_discovery' : '/api/start_nearby_discovery',
      undefined,
      'Nearby',
    );
  }

  async function addNetwork() {
    const name = newNetworkName.trim();
    if (!name) {
      return;
    }
    const ok = await run('/api/add_network', { name }, 'Adding network');
    if (ok) {
      newNetworkName = '';
      addNetworkOpen = false;
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
    const meshId = (meshIdDrafts[network.id] ?? '').trim();
    let ok = true;
    if (name && name !== network.name) {
      ok = await run('/api/rename_network', { networkId: network.id, name }, 'Saving network');
    }
    if (ok && meshId && meshId !== network.networkId) {
      ok = await run(
        '/api/set_network_mesh_id',
        { networkId: network.id, meshId },
        'Saving mesh',
      );
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
        advertisedRoutes: settingsDraft.advertisedRoutes,
        magicDnsSuffix: settingsDraft.magicDnsSuffix,
        autoconnect: settingsDraft.autoconnect,
      },
      'Saving settings',
    );
    if (ok && state) {
      settingsDirty = false;
      syncSettings(state);
    }
  }

  async function setExitNode(npub: string) {
    await run('/api/update_settings', { exitNode: npub }, 'Updating route');
  }

  async function setAdvertiseExitNode(enabled: boolean) {
    await run('/api/update_settings', { advertiseExitNode: enabled }, 'Updating exit');
  }
</script>

<svelte:head>
  <title>Nostr VPN</title>
</svelte:head>

<main class="shell">
  <aside class="sidebar">
    <div class="brand">
      <div class="brand-mark" aria-hidden="true">N</div>
      <div>
        <h1>Nostr VPN</h1>
        <p>{state?.platform ?? 'Umbrel'}</p>
      </div>
    </div>

    <nav class="nav" aria-label="Primary">
      {#each tabs as item}
        <button
          type="button"
          class:active={tab === item.id}
          aria-current={tab === item.id ? 'page' : undefined}
          on:click={() => (tab = item.id)}
        >
          {item.label}
        </button>
      {/each}
    </nav>

    <div class="sidebar-actions">
      <button type="button" class="secondary-button" on:click={() => (addNetworkOpen = true)}>
        Add Network
      </button>
      {#if activeNetwork?.localIsAdmin}
        <button type="button" class="small-button" on:click={() => (addDeviceOpen = true)}>
          Add Device
        </button>
      {/if}
    </div>

    {#if state}
      <div class="sidebar-summary">
        <span class="status-dot {heroTone(state)}"></span>
        <div>
          <strong>{state.vpnEnabled ? 'VPN on' : 'VPN off'}</strong>
          <span>{state.connectedPeerCount}/{state.expectedPeerCount} devices</span>
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
      <header class="hero">
        <div class="hero-status {heroTone(state)}" aria-hidden="true"></div>
        <div class="hero-main">
          <div class="hero-title-row">
            <h2>{activeNetwork?.name ?? 'Nostr VPN'}</h2>
            {#if activeNetwork?.localIsAdmin}
              <span class="badge muted">Admin</span>
            {/if}
          </div>
          <p>{state.vpnStatus}</p>
          <div class="badge-row">
            <span class="badge {state.vpnActive ? 'ok' : 'muted'}">
              {state.vpnActive ? 'VPN active' : 'VPN inactive'}
            </span>
            <span class="badge {state.daemonRunning ? 'ok' : 'muted'}">
              {state.daemonRunning ? 'Daemon' : 'Daemon off'}
            </span>
            <span class="badge {state.meshReady ? 'ok' : 'warn'}">
              {state.meshReady ? 'Mesh ready' : 'Mesh pending'}
            </span>
            {#if state.health.length > 0}
              <span class="badge warn">{state.health.length} health</span>
            {/if}
          </div>
        </div>
        <button
          class="primary-button"
          type="button"
          disabled={!activeNetwork || !state.vpnControlSupported || Boolean(busyAction)}
          on:click={toggleVpn}
        >
          {state.vpnEnabled ? 'Pause' : 'Connect'}
        </button>
      </header>

      <div class="identity-strip">
        <span>This device</span>
        <code>{shortMiddle(state.ownNpub, 32)}</code>
        <button type="button" class="small-button" on:click={copyOwnNpub}>
          Copy
        </button>
        {#if state.tunnelIp}
          <span class="badge muted">{state.tunnelIp}</span>
        {/if}
      </div>

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

      {#if addDeviceOpen && activeNetwork}
        <div class="modal-backdrop" role="presentation">
          <div
            class="modal-card"
            role="dialog"
            aria-modal="true"
            aria-labelledby="add-device-title"
          >
            <div class="modal-header">
              <h3 id="add-device-title">Add Device</h3>
              <button type="button" class="small-button" on:click={() => (addDeviceOpen = false)}>
                Done
              </button>
            </div>

            <div class="modal-body">
              <div class="modal-section invite-section">
                <div class="qr-frame compact">
                  {#if qr && qr.width > 0}
                    <div class="qr-grid" style={`--qr-width: ${qr.width}`}>
                      {#each qr.cells as cell, index (index)}
                        <span class:dark={cell}></span>
                      {/each}
                    </div>
                  {:else}
                    <div class="qr-empty">QR</div>
                  {/if}
                </div>
                <div class="share-copy">
                  <div class="section-heading">
                    <div>
                      <h3>Invite Devices</h3>
                      <p>{activeNetwork.name}</p>
                    </div>
                  </div>
                  <textarea readonly value={state.activeNetworkInvite} aria-label="Invite"></textarea>
                  <div class="button-row">
                    <button
                      type="button"
                      class="secondary-button"
                      disabled={!state.activeNetworkInvite}
                      on:click={copyInvite}
                    >
                      Copy
                    </button>
                    <button
                      type="button"
                      class="secondary-button"
                      on:click={toggleInviteBroadcast}
                    >
                      {state.inviteBroadcastActive
                        ? `Broadcasting ${remainingText(state.inviteBroadcastRemainingSecs)}`
                        : 'Broadcast invite'}
                    </button>
                    <label class="switch-row inline-switch">
                      <span>Allow requests</span>
                      <input
                        type="checkbox"
                        checked={activeNetwork.joinRequestsEnabled}
                        disabled={!activeNetwork.localIsAdmin || Boolean(busyAction)}
                        on:change={(event) =>
                          setJoinRequests(activeNetwork, (event.currentTarget as HTMLInputElement).checked)}
                      />
                    </label>
                  </div>
                </div>
              </div>

              <div class="modal-section">
                <div class="section-heading">
                  <div>
                    <h3>For Manual Join</h3>
                    <p>{activeNetwork.name}</p>
                  </div>
                </div>
                <div class="detail-list two-column">
                  <div>
                    <span>Your Device ID</span>
                    <strong>{shortMiddle(state.ownNpub, 36)}</strong>
                    <button type="button" class="small-button" on:click={copyOwnNpub}>Copy</button>
                  </div>
                  <div>
                    <span>Network ID</span>
                    <strong>{activeNetwork.networkId}</strong>
                    <button type="button" class="small-button" on:click={() => copyText(activeNetwork.networkId, 'Network ID')}>
                      Copy
                    </button>
                  </div>
                </div>
              </div>

              <form class="modal-section" on:submit|preventDefault={addParticipant}>
                <div class="section-heading">
                  <div>
                    <h3>Add by Device ID</h3>
                    <p>{activeNetwork.name}</p>
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
            </div>
          </div>
        </div>
      {/if}

      {#if addNetworkOpen}
        <div class="modal-backdrop" role="presentation">
          <div
            class="modal-card"
            role="dialog"
            aria-modal="true"
            aria-labelledby="add-network-title"
          >
            <div class="modal-header">
              <h3 id="add-network-title">Add Network</h3>
              <button type="button" class="small-button" on:click={() => (addNetworkOpen = false)}>
                Done
              </button>
            </div>

            <div class="modal-body">
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
                      ? `Listening ${remainingText(state.nearbyDiscoveryRemainingSecs)}`
                      : 'Look nearby'}
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
                          <button type="button" class="small-button" on:click={() => run('/api/import_network_invite', { invite: peer.invite }, 'Joining')}>
                            Join
                          </button>
                        {/if}
                      </div>
                    {/each}
                  </div>
                {/if}
              </div>
            </div>
          </div>
        </div>
      {/if}

      {#if tab === 'devices'}
        <section class="page-grid">
          <div class="panel wide">
            <div class="section-heading">
              <div>
                <h3>Devices</h3>
                <p>{activeNetwork ? `${activeNetwork.onlineCount}/${activeNetwork.expectedCount} online` : 'No network'}</p>
              </div>
              {#if activeNetwork}
                <button
                  type="button"
                  class="small-button"
                  on:click={() => setJoinRequests(activeNetwork, !activeNetwork.joinRequestsEnabled)}
                >
                  {activeNetwork.joinRequestsEnabled ? 'Requests on' : 'Requests off'}
                </button>
              {/if}
            </div>

            {#if !activeNetwork}
              <div class="empty-state">No network</div>
            {:else if participants.length === 0}
              <div class="empty-state">No devices</div>
            {:else}
              <div class="device-list">
                {#each participants as participant (participant.pubkeyHex || participant.npub)}
                  <article class="device-row">
                    <div class="device-status">
                      <span class="status-dot {participantTone(participant)}"></span>
                    </div>
                    <div class="device-main">
                      <div class="device-title">
                        <strong>{participantName(participant)}</strong>
                        {#if isSelf(participant)}
                          <span class="badge active">Self</span>
                        {/if}
                        {#if participant.isAdmin}
                          <span class="badge muted">Admin</span>
                        {/if}
                        {#if participant.offersExitNode}
                          <span class="badge warn">Exit</span>
                        {/if}
                      </div>
                      <div class="device-meta">
                        <span>{nonEmpty(participant.tunnelIp)}</span>
                        <span>{nonEmpty(participant.statusText || participant.state)}</span>
                        <span>{formatBytes(participant.txBytes)} sent</span>
                        <span>{formatBytes(participant.rxBytes)} received</span>
                      </div>
                      <div class="device-details">
                        <code>{shortMiddle(participant.npub, 36)}</code>
                        {#if participant.magicDnsName}
                          <span>{participant.magicDnsName}</span>
                        {/if}
                        {#if participant.fipsTransportType}
                          <span>{participant.fipsTransportType} {participant.fipsSrttMs ? `${participant.fipsSrttMs} ms` : ''}</span>
                        {/if}
                      </div>
                    </div>
                    <div class="device-actions">
                      <button
                        type="button"
                        class="icon-button"
                        aria-label="Copy device ID"
                        title="Copy device ID"
                        on:click={() => copyText(participant.npub, 'Device ID')}
                      >
                        C
                      </button>
                      {#if activeNetwork.localIsAdmin}
                        <input
                          class="alias-input"
                          aria-label="Device name"
                          bind:value={aliasDrafts[participant.npub]}
                          disabled={Boolean(busyAction)}
                        />
                        <button
                          type="button"
                          class="icon-button"
                          aria-label="Save device name"
                          title="Save device name"
                          on:click={() => saveAlias(participant)}
                        >
                          S
                        </button>
                      {/if}
                      {#if activeNetwork.localIsAdmin && !isSelf(participant)}
                        <button
                          type="button"
                          class="icon-button"
                          aria-label={participant.isAdmin ? 'Remove admin' : 'Make admin'}
                          title={participant.isAdmin ? 'Remove admin' : 'Make admin'}
                          on:click={() => toggleAdmin(activeNetwork, participant)}
                        >
                          {participant.isAdmin ? '-' : '+'}
                        </button>
                        <button
                          type="button"
                          class="icon-button danger"
                          aria-label="Remove device"
                          title="Remove device"
                          on:click={() => removeParticipant(activeNetwork, participant)}
                        >
                          X
                        </button>
                      {/if}
                    </div>
                  </article>
                {/each}
              </div>
            {/if}
          </div>

          <div class="panel">
            <div class="section-heading">
              <div>
                <h3>Network</h3>
                <p>{activeNetwork?.networkId ?? '-'}</p>
              </div>
              {#if activeNetwork}
                <button type="button" class="small-button" on:click={() => copyText(activeNetwork.networkId, 'Mesh ID')}>
                  Copy
                </button>
              {/if}
            </div>

            {#if activeNetwork}
              <div class="detail-list">
                <div>
                  <span>Admins</span>
                  <strong>{activeNetwork.adminNpubs.length}</strong>
                </div>
                <div>
                  <span>Join requests</span>
                  <strong>{activeNetwork.joinRequestsEnabled ? 'On' : 'Off'}</strong>
                </div>
                <div>
                  <span>Routes</span>
                  <strong>{routeList(state.effectiveAdvertisedRoutes)}</strong>
                </div>
              </div>

              {#if activeNetwork.outboundJoinRequest}
                <div class="callout">
                  <strong>Request pending</strong>
                  <span>{activeNetwork.outboundJoinRequest.requestedAtText}</span>
                </div>
              {:else if !activeNetwork.localIsAdmin && activeNetwork.adminNpubs.length > 0}
                <button type="button" class="secondary-button" on:click={() => requestJoin(activeNetwork)}>
                  Request access
                </button>
              {/if}
            {/if}
          </div>

          {#if activeNetwork && activeNetwork.inboundJoinRequests.length > 0}
            <div class="panel">
              <div class="section-heading">
                <div>
                  <h3>Requests</h3>
                  <p>{activeNetwork.inboundJoinRequests.length}</p>
                </div>
              </div>
              <div class="stack">
                {#each activeNetwork.inboundJoinRequests as request (request.requesterPubkeyHex)}
                  <div class="request-row">
                    <div>
                      <strong>{nonEmpty(request.requesterNodeName, shortMiddle(request.requesterNpub, 20))}</strong>
                      <span>{request.requestedAtText}</span>
                    </div>
                    <div class="row-actions">
                      <button type="button" class="small-button" on:click={() => acceptJoinRequest(activeNetwork, request)}>
                        Accept
                      </button>
                      <button type="button" class="small-button danger" on:click={() => rejectJoinRequest(activeNetwork, request)}>
                        Reject
                      </button>
                    </div>
                  </div>
                {/each}
              </div>
            </div>
          {/if}
        </section>
      {:else if tab === 'exit'}
        <section class="page-grid">
          <div class="panel wide">
            <div class="section-heading">
              <div>
                <h3>Route</h3>
                <p>{state.exitNode ? 'Peer exit' : 'Direct'}</p>
              </div>
            </div>
            <label>
              <span>Search</span>
              <input bind:value={exitSearch} autocomplete="off" />
            </label>

            <div class="choice-list">
              <button
                type="button"
                class:active={!state.exitNode}
                class="choice-row"
                on:click={() => setExitNode('')}
              >
                <span class="radio-dot"></span>
                <div>
                  <strong>Direct</strong>
                  <span>Normal internet route</span>
                </div>
              </button>

              {#each exitCandidates as participant (participant.pubkeyHex || participant.npub)}
                <button
                  type="button"
                  class:active={state.exitNode === participant.npub}
                  class="choice-row"
                  disabled={!participant.offersExitNode}
                  on:click={() => setExitNode(participant.npub)}
                >
                  <span class="radio-dot"></span>
                  <div>
                    <strong>{participantName(participant)}</strong>
                    <span>{participant.offersExitNode ? nonEmpty(participant.statusText, 'Exit node') : 'Exit not offered'}</span>
                  </div>
                </button>
              {/each}
            </div>
          </div>

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
            <div class="detail-list">
              <div>
                <span>Advertised</span>
                <strong>{routeList(state.effectiveAdvertisedRoutes)}</strong>
              </div>
              <div>
                <span>Selected</span>
                <strong>{state.exitNode ? shortMiddle(state.exitNode, 22) : 'Direct'}</strong>
              </div>
            </div>
          </div>
        </section>
      {:else if tab === 'settings'}
        <section class="page-grid">
          <form class="panel wide" on:submit|preventDefault={saveSettings}>
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
                <span>DNS Suffix</span>
                <input bind:value={settingsDraft.magicDnsSuffix} on:input={() => (settingsDirty = true)} />
              </label>
              <label>
                <span>Advertised Routes</span>
                <input bind:value={settingsDraft.advertisedRoutes} on:input={() => (settingsDirty = true)} />
              </label>
            </div>

            <label class="switch-row">
              <span>Autoconnect</span>
              <input
                type="checkbox"
                bind:checked={settingsDraft.autoconnect}
                on:change={() => (settingsDirty = true)}
              />
            </label>

            <div class="button-row">
              <button type="submit" class="secondary-button" disabled={Boolean(busyAction)}>
                Save
              </button>
              <span class="form-status">{state.magicDnsStatus}</span>
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
                      <button type="button" class="small-button" on:click={() => activateNetwork(network)}>
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
                <h3>Diagnostics</h3>
                <p>{state.health.length > 0 ? `${state.health.length} issues` : 'Healthy'}</p>
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
        </section>
      {/if}
    {/if}
  </section>
</main>
