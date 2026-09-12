<script lang="ts">
  import { api, type SystemStatus } from '../api';
  import { wm } from '../windows.svelte';
  import { t } from '../i18n.svelte';
  let s = $state<SystemStatus | null>(null);
  $effect(() => { const f = () => api.status().then(x => s = x).catch(() => s = null); f(); const t = setInterval(f, 10000); return () => clearInterval(t); });
</script>
{#if s}
  <div class="grid">
    {#each s.components as c}<button class="c" title={c.detail ?? ''} onclick={() => wm.openSys('system')}><span class="dot" class:ok={c.status === 'ok'} class:bad={c.status !== 'ok'}></span>{c.name}<span class="muted small">{c.detail?.slice(0, 14)}</span></button>{/each}
  </div>
{:else}<div class="muted">{t('common.noCore')}</div>{/if}
<style>
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); gap: 6px; }
  .c { display: flex; gap: 6px; align-items: center; background: var(--panel-2); border-color: transparent; padding: 6px 8px; font-size: .85rem; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--muted); } .dot.ok { background: var(--ok); } .dot.bad { background: var(--bad); }
  .small { font-size: .7rem; margin-left: auto; }
</style>
