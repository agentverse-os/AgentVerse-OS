<script lang="ts">
  import { api } from '../api';
  import { fmtBytes, spark } from './util';
  import { wm } from '../windows.svelte';
  import { t, tn } from '../i18n.svelte';
  let { config, w }: { config: Record<string, any>; w: number } = $props();
  let m = $state<any>(null);
  let error = $state('');
  async function load() { try { m = await (await fetch('/api/monitor')).json(); error = ''; } catch (e) { error = String(e); } }
  $effect(() => { load(); const t = setInterval(load, 5000); return () => clearInterval(t); });
  const memPct = $derived(m ? Math.round(m.host.mem_used / m.host.mem_total * 100) : 0);
  const compact = $derived(w < 340);
  function uptime(s: number) { const d = Math.floor(s / 86400), h = Math.floor(s % 86400 / 3600), mi = Math.floor(s % 3600 / 60); return d ? t('widgets.monitor.uptimeDH', { d, h }) : t('widgets.monitor.uptimeHM', { h, m: mi }); }
</script>

{#if error}<div class="muted">{t('widgets.monitor.noData', { error })}</div>
{:else if !m}<div class="muted">{t('widgets.monitor.collecting')}</div>
{:else}
  <div class="mon" class:compact>
    <div class="gauges">
      <button class="gauge" onclick={() => wm.openSys('system')} title={t('widgets.monitor.openSystem')}>
        <svg viewBox="0 0 200 40" preserveAspectRatio="none"><polyline points={spark(m.history_cpu, 100)} fill="none" stroke="var(--accent)" stroke-width="2" /></svg>
        <div class="val"><b>{m.host.cpu_pct.toFixed(0)}%</b><span class="muted">{t('widgets.monitor.cpuLine', { cores: tn('widgets.monitor.cores', m.cores), load: m.host.load1.toFixed(2) })}</span></div>
      </button>
      <button class="gauge" onclick={() => wm.openSys('system')} title={t('widgets.monitor.openSystem')}>
        <svg viewBox="0 0 200 40" preserveAspectRatio="none"><polyline points={spark(m.history_mem, m.host.mem_total)} fill="none" stroke="var(--ok)" stroke-width="2" /></svg>
        <div class="val"><b>{memPct}%</b><span class="muted">RAM · {fmtBytes(m.host.mem_used)} / {fmtBytes(m.host.mem_total)}</span></div>
      </button>
    </div>
    <div class="disks">
      {#each m.disks as d}
        {@const used = d.total - d.avail}
        <div class="disk" title={d.fs}><span class="muted">{d.mount}</span><div class="bar"><i style="width:{d.total ? used / d.total * 100 : 0}%"></i></div><span>{t('widgets.monitor.free', { size: fmtBytes(d.avail) })}</span></div>
      {/each}
      <div class="muted small">{t('widgets.monitor.summary', { uptime: uptime(m.host.uptime_s), containers: m.containers.length, instances: m.instances.length })}</div>
    </div>
    {#if !compact}
      <div class="top">
        <div class="muted small">{t('widgets.monitor.topMem')}</div>
        {#each [...m.instances.map((i: any) => ({ name: `🧩 ${i.name}`, cpu: i.cpu_pct, mem: i.mem_used })), ...m.containers.map((c: any) => ({ name: c.app ?? c.name, cpu: c.cpu_pct, mem: c.mem_used }))].sort((a, b) => b.mem - a.mem).slice(0, 6) as r}
          <div class="row-t"><span class="name">{r.name}</span><span class="muted">{r.cpu.toFixed(1)}%</span><span>{fmtBytes(r.mem)}</span></div>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .mon { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; height: 100%; }
  .mon.compact { grid-template-columns: 1fr; }
  .gauges { display: flex; flex-direction: column; gap: 8px; }
  .gauge { display: block; text-align: left; padding: 6px 8px; background: var(--panel-2); border-color: transparent; }
  .gauge svg { width: 100%; height: 36px; display: block; }
  .val { display: flex; flex-wrap: wrap; align-items: baseline; column-gap: 6px; }  /* зазор gap, а не пробелом: в flex ведущий пробел схлопывается («9%CPU») */
  .val b { font-size: 1.2rem; }
  .val span { min-width: 0; }
  .disks { display: flex; flex-direction: column; gap: 6px; font-size: .8rem; }
  .disk { display: flex; flex-direction: column; gap: 2px; }
  .bar { height: 6px; background: var(--panel-2); border-radius: 3px; overflow: hidden; } .bar i { display: block; height: 100%; background: var(--accent); }
  .small { font-size: .75rem; }
  .top { grid-column: 1 / -1; font-size: .8rem; }
  .row-t { display: grid; grid-template-columns: 1fr auto auto; gap: 8px; padding: 1px 0; border-bottom: 1px solid color-mix(in srgb, var(--line) 50%, transparent); }
  .row-t .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
