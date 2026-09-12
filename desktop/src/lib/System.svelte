<script lang="ts">
  import { api, type SystemStatus, type Event } from './api';
  import { wm } from './windows.svelte';
  import { t } from './i18n.svelte';
  let status = $state<SystemStatus | null>(null);
  let events = $state<Event[]>([]);
  let error = $state('');
  async function load() { try { [status, events] = await Promise.all([api.status(), api.events()]); error = ''; } catch (e) { error = e instanceof Error ? e.message : String(e); } }
  $effect(() => { load(); const t = setInterval(load, 10000); return () => clearInterval(t); });
</script>

<div class="row" style="justify-content: space-between; align-items: baseline;"><h2>{t('sys.system')}</h2>{#if status}<span class="muted">AgentVerse OS {status.version}{#if status.build && status.build !== 'dev'}{' · '}{status.build}{/if}{' · '}<button onclick={() => wm.openSys('updates')}>⬆️ {t('sys.updates')}</button> <button onclick={() => wm.openSys('setup')}>🧭 {t('sys.setup')}</button></span>{/if}</div>
{#if error}<div class="card" style="border-color: var(--bad); margin-bottom: 12px;">{error}</div>{/if}
{#if status}
  <div class="grid">
    {#each status.components as c}
      <div class="card"><div class="row" style="justify-content: space-between;"><h3>{c.name}</h3><span class="badge {c.status==='ok'?'ok':'bad'}">{c.status}</span></div><div class="muted">{c.detail}</div></div>
    {/each}
  </div>
{/if}
<h2 style="margin-top: 16px;">{t('system.events')}</h2>
<div class="card">
  {#each events as e}<div class="muted"><code>{e.at.slice(0, 19).replace('T', ' ')}</code> <b>{e.kind}</b> {e.subject} — {e.message}</div>{/each}
  {#if !events.length}<div class="muted">{t('common.empty')}</div>{/if}
</div>
