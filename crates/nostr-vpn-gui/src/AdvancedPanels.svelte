<script lang="ts">
  import { Trash2 } from 'lucide-svelte'

  import { healthBadgeClass, healthSummaryText } from './lib/app-view'
  import { reconcileAutoOpenPanelState } from './lib/collapsible-panels.js'
  import type { SettingsPatch, UiState } from './lib/types'

  export let state: UiState
  export let relayInput = ''
  export let onAddRelay: () => Promise<void>
  export let onRemoveRelay: (relayUrl: string) => Promise<void>
  export let onUpdateSettings: (patch: SettingsPatch) => Promise<void>

  let diagnosticsOpen = false
  let previousHealthCount: number | null = null

  $: {
    const nextHealthCount = state.health.length
    diagnosticsOpen = reconcileAutoOpenPanelState(
      diagnosticsOpen,
      previousHealthCount,
      nextHealthCount,
    )
    previousHealthCount = nextHealthCount
  }
</script>

{#if state.vpnSessionControlSupported}
  <details
    class="panel collapsible-panel"
    open={diagnosticsOpen}
    on:toggle={(event) => {
      diagnosticsOpen = (event.currentTarget as HTMLDetailsElement).open
    }}
  >
    <summary class="collapsible-summary">
      <div>
        <div class="panel-kicker">Advanced</div>
        <h2>Diagnostics</h2>
      </div>
      <div class="section-meta">{healthSummaryText(state)}</div>
    </summary>

    <div class="collapsible-body diagnostics-panel">
      <div class="row status-row diagnostics-badges">
        <span class="badge muted">IF {state.network.defaultInterface || 'unknown'}</span>
        <span
          class={`badge ${
            state.network.captivePortal === true
              ? 'bad'
              : state.network.captivePortal === false
                ? 'ok'
                : 'muted'
          }`}
        >
          {state.network.captivePortal === true
            ? 'Captive portal'
            : state.network.captivePortal === false
              ? 'Open internet'
              : 'Portal unknown'}
        </span>
        <span class={`badge ${state.portMapping.activeProtocol ? 'ok' : 'muted'}`}>
          Mapping {state.portMapping.activeProtocol || 'none'}
        </span>
      </div>

      <div class="diagnostics-copy">
        <div class="config-path">
          Local addresses:
          {state.network.primaryIpv4 || 'no IPv4'}
          {#if state.network.primaryIpv6}
            | {state.network.primaryIpv6}
          {/if}
        </div>
        <div class="config-path">
          Gateway:
          {state.network.gatewayIpv4 || state.network.gatewayIpv6 || 'unknown'}
        </div>
        <div class="config-path">
          External endpoint:
          {state.portMapping.externalEndpoint || 'stun / direct only'}
        </div>
      </div>

      {#if state.health.length === 0}
        <div class="config-path" data-testid="health-empty">Daemon reports no active health warnings.</div>
      {:else}
        <div class="stack rows">
          {#each state.health as issue}
            <div class="health-card" data-testid="health-issue">
              <div class="row spread health-card-header">
                <div class="item-title">{issue.summary}</div>
                <span class={`badge ${healthBadgeClass(issue.severity)}`}>{issue.severity}</span>
              </div>
              <div class="item-sub">{issue.detail}</div>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  </details>

  <details class="panel collapsible-panel">
    <summary class="collapsible-summary">
      <div>
        <div class="panel-kicker">Advanced</div>
        <h2>FIPS Discovery</h2>
      </div>
      <div class="section-meta relay-health">
        <span class="ok-text">{state.relays.length} configured</span>
      </div>
    </summary>

    <div class="collapsible-body">
      <div class="row form-row">
        <input
          class="text-input"
          placeholder="Add discovery relay URL"
          data-testid="relay-input"
          bind:value={relayInput}
          on:keydown={(event) => event.key === 'Enter' && onAddRelay()}
        />
        <button class="btn" data-testid="relay-add" on:click={() => onAddRelay()}>Add</button>
      </div>

      <div class="stack rows">
        {#each state.relays as relay}
          <div class="item-row" data-testid="relay-row">
            <div class="item-main">
              <div class="item-title relay-url">{relay.url}</div>
              {#if relay.state !== 'unknown' && relay.statusText}
                <div class="item-sub">{relay.statusText}</div>
              {/if}
            </div>
            <span
              class={`badge ${relay.state === 'up' ? 'ok' : relay.state === 'down' ? 'bad' : relay.state === 'checking' ? 'warn' : 'muted'}`}
            >
              {relay.state}
            </span>
            <button
              class="btn ghost icon-btn"
              data-testid="relay-remove"
              title="Delete relay"
              aria-label="Delete relay"
              on:click={() => onRemoveRelay(relay.url)}
            >
              <Trash2 size={16} strokeWidth={2.2} />
            </button>
          </div>
        {/each}
      </div>
    </div>
  </details>

  <details class="panel collapsible-panel">
    <summary class="collapsible-summary">
      <div>
        <div class="panel-kicker">Connection</div>
        <h2>Session & Paths</h2>
      </div>
      <div class="section-meta">Startup</div>
    </summary>

    <div class="collapsible-body">
      <label class="toggle-row">
        <input
          type="checkbox"
          checked={state.autoconnect}
          on:change={(event) =>
            onUpdateSettings({
              autoconnect: (event.currentTarget as HTMLInputElement).checked,
            })}
        />
        <span>Auto-connect session on app start</span>
      </label>
    </div>
  </details>
{/if}
