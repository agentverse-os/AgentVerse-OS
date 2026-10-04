<script lang="ts">
  import { api, type SystemStatus, type Event, type UsageSummary } from './api';
  import { wm } from './windows.svelte';
  import { t } from './i18n.svelte';
  let status = $state<SystemStatus | null>(null);
  let events = $state<Event[]>([]);
  let usage = $state<UsageSummary | null>(null);
  let error = $state('');
  async function load() { try { [status, events] = await Promise.all([api.status(), api.events()]); error = ''; api.usageSummary(30).then(u => usage = u).catch(() => {}); } catch (e) { error = e instanceof Error ? e.message : String(e); } }
  $effect(() => { load(); const t = setInterval(load, 10000); return () => clearInterval(t); });
  const fmtK = (n: number) => n >= 1_000_000 ? (n / 1_000_000).toFixed(1) + 'M' : n >= 1000 ? (n / 1000).toFixed(1) + 'k' : String(n);
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
<h2 style="margin-top: 16px;">{t('system.usage')}</h2>
<div class="card">
  {#if usage && usage.total.turns}
    <div class="row" style="gap: 16px; flex-wrap: wrap; align-items: baseline;">
      <b style="font-size: 1.3rem;">${usage.total.cost_usd.toFixed(2)}</b><span class="muted">{t('system.usage30')}</span>
      <span class="muted">{fmtK(usage.total.input)}↑ {fmtK(usage.total.output)}↓ · cache {fmtK(usage.total.cache_read)} · {t('system.usageTurns', { n: usage.total.turns })}</span>
    </div>
    <div class="row" style="gap: 24px; align-items: flex-start; flex-wrap: wrap; margin-top: 8px;">
      <table class="usage"><thead><tr><th>{t('system.usageProject')}</th><th>$</th><th>↑</th><th>↓</th></tr></thead><tbody>{#each usage.by_project as b}<tr><td>{b.key}</td><td>{b.cost_usd.toFixed(2)}</td><td>{fmtK(b.input + b.cache_read + b.cache_create)}</td><td>{fmtK(b.output)}</td></tr>{/each}</tbody></table>
      <table class="usage"><thead><tr><th>{t('system.usageModel')}</th><th>$</th><th>{t('system.usageTurnsShort')}</th></tr></thead><tbody>{#each usage.by_model as b}<tr><td>{b.key}</td><td>{b.cost_usd.toFixed(2)}</td><td>{b.turns}</td></tr>{/each}</tbody></table>
      <table class="usage"><thead><tr><th>{t('system.usageDay')}</th><th>$</th></tr></thead><tbody>{#each usage.by_day.slice(-7) as b}<tr><td>{b.key}</td><td>{b.cost_usd.toFixed(2)}</td></tr>{/each}</tbody></table>
    </div>
  {:else}<span class="muted">{t('system.usageEmpty')}</span>{/if}
</div>
<h2 style="margin-top: 16px;">{t('system.events')}</h2>
<div class="card">
  {#each events as e}<div class="muted"><code>{e.at.slice(0, 19).replace('T', ' ')}</code> <b>{e.kind}</b> {e.subject} — {e.message}</div>{/each}
  {#if !events.length}<div class="muted">{t('common.empty')}</div>{/if}
</div>

<style>
  table.usage { border-collapse: collapse; font-size: .85rem; }
  table.usage th, table.usage td { text-align: left; padding: 2px 10px 2px 0; }
  table.usage th { color: var(--muted); font-weight: 500; }
</style>
