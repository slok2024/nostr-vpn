import { expect, test, type APIRequestContext, type Page } from '@playwright/test';

type ParticipantView = {
  npub: string;
  isAdmin: boolean;
  magicDnsAlias: string;
};

type NetworkView = {
  id: string;
  name: string;
  enabled: boolean;
  networkId: string;
  participants: ParticipantView[];
};

type UiState = {
  vpnStatus: string;
  activeNetworkInvite: string;
  nodeName: string;
  magicDnsSuffix: string;
  autoconnect: boolean;
  networks: NetworkView[];
};

type QrMatrix = {
  width: number;
  cells: boolean[];
};

test.describe.configure({ mode: 'serial' });

async function postJson<T>(
  request: APIRequestContext,
  path: string,
  data?: unknown,
): Promise<T> {
  const response = await request.post(path, data === undefined ? undefined : { data });
  expect(response.ok(), `${path} returned ${response.status()}`).toBeTruthy();
  return (await response.json()) as T;
}

function activeNetwork(state: UiState): NetworkView {
  const network = state.networks.find((candidate) => candidate.enabled) ?? state.networks[0];
  expect(network, 'expected at least one network').toBeTruthy();
  return network;
}

function byName(state: UiState, name: string): NetworkView {
  const network = state.networks.find((candidate) => candidate.name === name);
  expect(network, `expected network named ${name}`).toBeTruthy();
  return network!;
}

async function expectNoConsoleErrors(page: Page, action: () => Promise<void>) {
  const errors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') {
      errors.push(message.text());
    }
  });
  page.on('pageerror', (error) => errors.push(error.message));

  await action();

  expect(errors).toEqual([]);
}

test('bundled UI loads, navigates, renders QR, and stays responsive', async ({ page }) => {
  await expectNoConsoleErrors(page, async () => {
    await page.goto('/');
    await expect(page).toHaveTitle('Nostr VPN');
    await expect(page.locator('.hero')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Devices' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Add Network' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Add Device' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Connect' })).toBeVisible();

    await page.getByRole('button', { name: 'Add Device' }).click();
    await expect(page.getByRole('heading', { name: 'Add Device' })).toBeVisible();
    await expect(page.locator('.qr-frame')).toBeVisible();
    expect(await page.locator('.qr-grid span.dark').count()).toBeGreaterThan(0);
    await page.getByRole('button', { name: 'Done' }).click();

    await page.getByRole('button', { name: 'Add Network' }).click();
    await expect(page.getByRole('heading', { name: 'Add Network' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Join Network' })).toBeVisible();
    await page.getByRole('button', { name: 'Done' }).click();

    await page.getByRole('button', { name: 'Exit Nodes' }).click();
    await expect(page.getByRole('heading', { name: 'Route' })).toBeVisible();

    await page.getByRole('button', { name: 'Settings' }).click();
    await expect(page.getByRole('heading', { name: 'This Device' })).toBeVisible();

    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto('/');
    await expect(page.locator('.hero')).toBeVisible();
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });
});

test('API supports the Umbrel web config action surface', async ({ request }) => {
  const peerNpub = process.env.NVPN_UMBREL_WEB_PEER_NPUB;
  test.skip(!peerNpub, 'NVPN_UMBREL_WEB_PEER_NPUB is required for participant actions');

  let state = await postJson<UiState>(request, '/api/tick');
  const originalNetwork = activeNetwork(state);
  expect(originalNetwork.networkId).not.toBe('nostr-vpn');
  expect(originalNetwork.networkId).toMatch(/^[0-9a-f]{16}$/);

  const qr = await postJson<QrMatrix>(request, '/api/qr_matrix', {
    text: state.activeNetworkInvite,
  });
  expect(qr.width).toBeGreaterThan(0);
  expect(qr.cells.length).toBe(qr.width * qr.width);
  expect(qr.cells.some(Boolean)).toBeTruthy();

  state = await postJson<UiState>(request, '/api/update_settings', {
    nodeName: 'Umbrel Web E2E',
    magicDnsSuffix: 'e2e.nvpn',
    autoconnect: true,
  });
  expect(state.nodeName).toBe('Umbrel Web E2E');
  expect(state.magicDnsSuffix).toBe('e2e.nvpn');
  expect(state.autoconnect).toBeTruthy();

  state = await postJson<UiState>(request, '/api/add_network', { name: 'E2E Work' });
  let workNetwork = byName(state, 'E2E Work');

  state = await postJson<UiState>(request, '/api/rename_network', {
    networkId: workNetwork.id,
    name: 'E2E Renamed',
  });
  workNetwork = byName(state, 'E2E Renamed');

  state = await postJson<UiState>(request, '/api/set_network_mesh_id', {
    networkId: workNetwork.id,
    meshId: 'umbrel-web-e2e',
  });
  workNetwork = byName(state, 'E2E Renamed');
  expect(workNetwork.networkId).toBe('umbrel-web-e2e');

  state = await postJson<UiState>(request, '/api/set_network_enabled', {
    networkId: workNetwork.id,
    enabled: true,
  });
  workNetwork = byName(state, 'E2E Renamed');
  expect(workNetwork.enabled).toBeTruthy();

  state = await postJson<UiState>(request, '/api/add_participant', {
    networkId: workNetwork.id,
    npub: peerNpub,
    alias: 'Peer One',
  });
  workNetwork = byName(state, 'E2E Renamed');
  expect(workNetwork.participants.some((participant) => participant.npub === peerNpub)).toBeTruthy();

  state = await postJson<UiState>(request, '/api/set_participant_alias', {
    npub: peerNpub,
    alias: 'Peer Renamed',
  });
  workNetwork = byName(state, 'E2E Renamed');
  expect(
    workNetwork.participants.find((participant) => participant.npub === peerNpub)?.magicDnsAlias,
  ).toBe('peer-renamed');

  state = await postJson<UiState>(request, '/api/add_admin', {
    networkId: workNetwork.id,
    npub: peerNpub,
  });
  workNetwork = byName(state, 'E2E Renamed');
  expect(workNetwork.participants.find((participant) => participant.npub === peerNpub)?.isAdmin).toBeTruthy();

  state = await postJson<UiState>(request, '/api/remove_admin', {
    networkId: workNetwork.id,
    npub: peerNpub,
  });
  workNetwork = byName(state, 'E2E Renamed');
  expect(workNetwork.participants.find((participant) => participant.npub === peerNpub)?.isAdmin).toBeFalsy();

  state = await postJson<UiState>(request, '/api/remove_participant', {
    networkId: workNetwork.id,
    npub: peerNpub,
  });
  workNetwork = byName(state, 'E2E Renamed');
  expect(workNetwork.participants.some((participant) => participant.npub === peerNpub)).toBeFalsy();

  state = await postJson<UiState>(request, '/api/start_nearby_discovery');
  expect(state.vpnStatus).toContain('LAN pairing is not available');

  state = await postJson<UiState>(request, '/api/set_network_enabled', {
    networkId: originalNetwork.id,
    enabled: true,
  });
  expect(activeNetwork(state).id).toBe(originalNetwork.id);

  state = await postJson<UiState>(request, '/api/remove_network', {
    networkId: workNetwork.id,
  });
  expect(state.networks.some((network) => network.id === workNetwork.id)).toBeFalsy();
});
