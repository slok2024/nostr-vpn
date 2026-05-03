<script lang="ts">
  import { onMount } from 'svelte'
  import {
    type DownloadEvent,
    type Update,
    downloadAndInstall,
    loadPrefs,
    maybeAutoCheck,
    patchPrefs,
  } from './lib/updater'

  type Phase = 'idle' | 'downloading' | 'installed' | 'failed'

  let update: Update | null = $state(null)
  let phase: Phase = $state('idle')
  let downloaded = $state(0)
  let total: number | undefined = $state(undefined)
  let error: string | null = $state(null)
  let autoInstall = $state(loadPrefs().autoInstall)

  onMount(async () => {
    const found = await maybeAutoCheck()
    if (!found || !found.updateAvailable) return
    const prefs = loadPrefs()
    if (prefs.dismissedVersion === found.version) return
    update = found
    if (prefs.autoInstall) {
      void install(found)
    }
  })

  async function install(target: Update) {
    phase = 'downloading'
    error = null
    try {
      await downloadAndInstall(target, (event: DownloadEvent) => {
        if (event.event === 'started') {
          downloaded = 0
          total = event.data.contentLength
        } else if (event.event === 'progress') {
          downloaded = event.data.downloaded
        } else if (event.event === 'finished') {
          downloaded = event.data.total
        }
      })
      phase = 'installed'
    } catch (err) {
      phase = 'failed'
      error = err instanceof Error ? err.message : String(err)
    }
  }

  function dismiss() {
    if (update) patchPrefs({ dismissedVersion: update.version })
    update = null
  }

  function toggleAutoInstall(value: boolean) {
    autoInstall = value
    patchPrefs({ autoInstall: value })
  }

  let pct = $derived(
    total && total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : null,
  )
</script>

{#if update}
  <div class="update-banner" role="status">
    <div class="update-message">
      {#if phase === 'installed'}
        Installed {update.version}. Restart Nostr VPN to apply.
      {:else if phase === 'downloading'}
        Downloading {update.version}{pct !== null ? ` — ${pct}%` : '…'}
      {:else if phase === 'failed'}
        <span class="update-error">Update failed: {error}</span>
      {:else}
        Update available: <strong>{update.version}</strong>
        (you're on {update.currentVersion})
      {/if}
    </div>
    <label class="update-auto">
      <input
        type="checkbox"
        checked={autoInstall}
        onchange={(e) => toggleAutoInstall((e.currentTarget as HTMLInputElement).checked)}
      />
      Install automatically
    </label>
    {#if phase === 'idle'}
      <button class="btn small" onclick={() => install(update!)}>Install</button>
    {/if}
    {#if phase !== 'downloading'}
      <button class="btn-icon" aria-label="Dismiss" onclick={dismiss}>×</button>
    {/if}
  </div>
{/if}

<style>
  .update-banner {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 1rem;
    background: var(--banner-bg, rgba(0, 130, 255, 0.12));
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    font-size: 0.9rem;
  }
  .update-message {
    flex: 1;
  }
  .update-error {
    color: #ff8a8a;
  }
  .update-auto {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.8rem;
    opacity: 0.85;
  }
  .btn-icon {
    background: transparent;
    border: none;
    color: inherit;
    opacity: 0.7;
    font-size: 1.2rem;
    line-height: 1;
    cursor: pointer;
    padding: 0 0.35rem;
  }
  .btn-icon:hover {
    opacity: 1;
  }
  .btn.small {
    padding: 0.25rem 0.6rem;
    font-size: 0.8rem;
  }
</style>
