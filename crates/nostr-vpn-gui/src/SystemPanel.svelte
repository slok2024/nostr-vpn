<script lang="ts">
  import type { SettingsPatch, UiState } from './lib/types'
  import {
    type DownloadEvent,
    type Update,
    checkForUpdate,
    downloadAndInstall,
    loadPrefs,
    patchPrefs,
  } from './lib/updater'

  export let state: UiState
  export let cliActionStatus = ''
  export let autostartReady = false
  export let autostartUpdating = false
  export let cliInstallSupported = false
  export let startupSettingsSupported = false
  export let trayBehaviorSupported = false
  export let magicDnsSuffixDraft = ''
  export let endpointDraft = ''
  export let tunnelIpDraft = ''
  export let listenPortDraft = ''
  export let onInstallCli: () => Promise<void>
  export let onUninstallCli: () => Promise<void>
  export let onToggleAutostart: (enabled: boolean) => Promise<void>
  export let onUpdateSettings: (patch: SettingsPatch) => Promise<void>
  export let debounce: (key: string, fn: () => Promise<void>, delay?: number) => void

  let updaterPrefs = loadPrefs()
  let updateStatus:
    | { kind: 'idle' }
    | { kind: 'checking' }
    | { kind: 'available'; update: Update }
    | { kind: 'upToDate' }
    | { kind: 'installing'; pct: number | null }
    | { kind: 'installed'; version: string }
    | { kind: 'error'; message: string } = { kind: 'idle' }

  function applyPrefs(patch: Parameters<typeof patchPrefs>[0]) {
    updaterPrefs = patchPrefs(patch)
  }

  async function manualCheck() {
    updateStatus = { kind: 'checking' }
    try {
      const update = await checkForUpdate()
      applyPrefs({ lastCheckMs: Date.now() })
      if (update && update.updateAvailable) {
        updateStatus = { kind: 'available', update }
      } else {
        updateStatus = { kind: 'upToDate' }
      }
    } catch (err) {
      updateStatus = {
        kind: 'error',
        message: err instanceof Error ? err.message : String(err),
      }
    }
  }

  async function installUpdate(update: Update) {
    updateStatus = { kind: 'installing', pct: null }
    try {
      let total: number | undefined
      await downloadAndInstall(update, (event: DownloadEvent) => {
        if (event.event === 'started') {
          total = event.data.contentLength
          updateStatus = { kind: 'installing', pct: total ? 0 : null }
        } else if (event.event === 'progress' && total) {
          updateStatus = {
            kind: 'installing',
            pct: Math.min(100, Math.round((event.data.downloaded / total) * 100)),
          }
        }
      })
      updateStatus = { kind: 'installed', version: update.version }
    } catch (err) {
      updateStatus = {
        kind: 'error',
        message: err instanceof Error ? err.message : String(err),
      }
    }
  }
</script>

<details class="panel collapsible-panel">
  <summary class="collapsible-summary">
    <div>
      <div class="panel-kicker">System</div>
      <h2>Device & App</h2>
    </div>
    <div class="section-meta">
      {cliInstallSupported || startupSettingsSupported || trayBehaviorSupported
        ? 'Node, DNS & startup'
        : 'Node & DNS'}
    </div>
  </summary>

  <div class="collapsible-body">
    <div class="row settings-action-row">
      <div class="config-path" data-testid="app-version">Version: {state.appVersion}</div>
    </div>
    <div class="row settings-action-row">
      <div class="config-path">Config: {state.configPath}</div>
    </div>
    {#if cliInstallSupported}
      <div class="row spread settings-action-row">
        <div class="config-path">Terminal CLI</div>
        <div class="row cli-actions-row">
          <button class="btn" data-testid="install-cli-btn" on:click={() => onInstallCli()}>
            {state.cliInstalled ? 'Reinstall CLI' : 'Install CLI'}
          </button>
          <button
            class="btn ghost"
            data-testid="uninstall-cli-btn"
            on:click={() => onUninstallCli()}
            disabled={!state.cliInstalled}
          >
            Uninstall CLI
          </button>
        </div>
      </div>
      {#if cliActionStatus}
        <div class="config-path">{cliActionStatus}</div>
      {/if}
    {/if}
    <div class="config-path" data-testid="magic-dns-status">DNS: {state.magicDnsStatus}</div>

    {#if startupSettingsSupported}
      <label class="toggle-row">
        <input
          type="checkbox"
          data-testid="autostart-toggle"
          checked={state.launchOnStartup}
          disabled={!autostartReady || autostartUpdating}
          on:change={(event) =>
            onToggleAutostart((event.currentTarget as HTMLInputElement).checked)}
        />
        <span>Launch on system startup</span>
      </label>
    {/if}

    {#if trayBehaviorSupported}
      <label class="toggle-row">
        <input
          type="checkbox"
          checked={state.closeToTrayOnClose}
          on:change={(event) =>
            onUpdateSettings({
              closeToTrayOnClose: (event.currentTarget as HTMLInputElement).checked,
            })}
        />
        <span>Keep running in menu bar when window is closed</span>
      </label>
    {/if}

    <div class="updates-section">
      <div class="panel-kicker">Updates</div>
      <div class="row settings-action-row">
        <button class="btn" on:click={manualCheck} disabled={updateStatus.kind === 'checking' || updateStatus.kind === 'installing'}>
          {updateStatus.kind === 'checking' ? 'Checking…' : 'Check for updates'}
        </button>
        {#if updaterPrefs.lastCheckMs > 0}
          <span class="config-path">Last checked {new Date(updaterPrefs.lastCheckMs).toLocaleString()}</span>
        {/if}
      </div>
      <label class="toggle-row">
        <input
          type="checkbox"
          checked={updaterPrefs.autoCheck}
          disabled={updaterPrefs.autoInstall}
          on:change={(event) => applyPrefs({ autoCheck: (event.currentTarget as HTMLInputElement).checked })}
        />
        <span>Check for updates automatically</span>
      </label>
      <label class="toggle-row">
        <input
          type="checkbox"
          checked={updaterPrefs.autoInstall}
          on:change={(event) => applyPrefs({ autoInstall: (event.currentTarget as HTMLInputElement).checked })}
        />
        <span>Install updates automatically <span class="config-path">(applies on next start)</span></span>
      </label>
      {#if updateStatus.kind === 'available'}
        <div class="row settings-action-row">
          <span>Version <strong>{updateStatus.update.version}</strong> is available.</span>
          <button class="btn" on:click={() => installUpdate(updateStatus.update)}>Install now</button>
        </div>
        {#if updateStatus.update.notes}
          <div class="config-path">{updateStatus.update.notes}</div>
        {/if}
      {:else if updateStatus.kind === 'upToDate'}
        <div class="config-path">You're up to date.</div>
      {:else if updateStatus.kind === 'installing'}
        <div class="config-path">Installing{updateStatus.pct !== null ? ` — ${updateStatus.pct}%` : '…'}</div>
      {:else if updateStatus.kind === 'installed'}
        <div class="config-path">Installed {updateStatus.version}. Restart Nostr VPN to apply.</div>
      {:else if updateStatus.kind === 'error'}
        <div class="config-path" style="color:#ff8a8a">{updateStatus.message}</div>
      {/if}
    </div>

    <div class="field-grid">
      <label>
        <span>MagicDNS Suffix (Optional)</span>
        <input
          class="text-input"
          data-testid="magic-dns-suffix-input"
          bind:value={magicDnsSuffixDraft}
          on:input={() =>
            debounce('magicDnsSuffix', () =>
              onUpdateSettings({ magicDnsSuffix: magicDnsSuffixDraft }))}
        />
      </label>

      <label>
        <span>Endpoint</span>
        <input
          class="text-input"
          bind:value={endpointDraft}
          on:input={() => debounce('endpoint', () => onUpdateSettings({ endpoint: endpointDraft }))}
        />
      </label>

      <label>
        <span>Tunnel IP</span>
        <input
          class="text-input"
          bind:value={tunnelIpDraft}
          on:input={() => debounce('tunnelIp', () => onUpdateSettings({ tunnelIp: tunnelIpDraft }))}
        />
      </label>

      <label>
        <span>Listen Port</span>
        <input
          class="text-input"
          bind:value={listenPortDraft}
          on:input={() =>
            debounce('listenPort', async () => {
              const parsed = Number.parseInt(listenPortDraft, 10)
              if (!Number.isNaN(parsed) && parsed > 0 && parsed <= 65535) {
                await onUpdateSettings({ listenPort: parsed })
              }
            })}
        />
      </label>
    </div>
  </div>
</details>

<style>
  .updates-section {
    margin-top: 0.75rem;
    padding-top: 0.75rem;
    border-top: 1px solid rgba(255, 255, 255, 0.08);
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
</style>
