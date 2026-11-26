<script lang="ts">
  import { api, type SystemStatus, type Event, type UsageSummary, type HostView, type PressureConfig } from './api';
  import { wm } from './windows.svelte';
  import { t, fmt } from './i18n.svelte';
  import { toast } from './toast.svelte';
  let status = $state<SystemStatus | null>(null);
  let events = $state<Event[]>([]);
  let usage = $state<UsageSummary | null>(null);
  let error = $state('');
  // owner's servers (ADR-008): the list, the busy row action, the "add" form and the install command shown after creation or a re-issued code
  let hosts = $state<HostView[]>([]);
  let hostBusy = $state('');
  let hostName = $state('');
  let hostNote = $state('');
  let install = $state<{ name: string; cmd: string } | null>(null);
  let uninstall = $state<{ name: string; cmd: string } | null>(null);
  const loadHosts = () => api.hosts().then(h => hosts = h).catch(() => {});
  // memory guard (ADR-009): the stand-wide rule and the projects where processes are never ended
  let pc = $state<PressureConfig | null>(null);
  let pcProjects = $state<string[]>([]);
  $effect(() => { api.pressure().then(p => pc = p).catch(() => {}); api.projects().then(p => pcProjects = p.map(x => x.name)).catch(() => {}); });
  async function savePc(patch: Record<string, unknown>) {
    try { pc = await api.pressurePut(patch); } catch (e) { toast.push({ kind: 'error', title: t('system.memTitle'), text: e instanceof Error ? e.message : String(e) }); api.pressure().then(p => pc = p).catch(() => {}); }
  }
  const numOf = (e: Event_, lo: number, hi: number) => Math.min(hi, Math.max(lo, Number((e.currentTarget as HTMLInputElement).value) || lo));
  type Event_ = { currentTarget: EventTarget | null };
  async function load() { try { [status, events] = await Promise.all([api.status(), api.events()]); error = ''; api.usageSummary(30).then(u => usage = u).catch(() => {}); loadHosts(); } catch (e) { error = e instanceof Error ? e.message : String(e); } }
  $effect(() => { load(); const t = setInterval(load, 10000); return () => clearInterval(t); });
  const fmtK = (n: number) => n >= 1_000_000 ? (n / 1_000_000).toFixed(1) + 'M' : n >= 1000 ? (n / 1000).toFixed(1) + 'k' : String(n);
  /** A server action: `key` marks the busy row, an error goes to a toast, the list is reloaded afterwards. */
  async function hostAct(key: string, f: () => Promise<unknown>) {
    hostBusy = key;
    try { await f(); } catch (e) { toast.push({ kind: 'error', title: t('system.hosts'), text: e instanceof Error ? e.message : String(e) }); }
    finally { hostBusy = ''; loadHosts(); }
  }
  const put = (v: HostView) => { hosts = hosts.map(h => h.name === v.name ? v : h); };
  const probe = (h: HostView) => hostAct(`${h.name}:probe`, async () => put(await api.hostProbe(h.name)));
  const code = (h: HostView) => hostAct(`${h.name}:code`, async () => { const v = await api.hostCode(h.name); install = { name: v.name, cmd: v.install ?? '' }; });
  const remove = (h: HostView) => { if (!window.confirm(t('system.hostsRemoveConfirm', { name: h.name }))) return; hostAct(`${h.name}:remove`, async () => { await api.hostRemove(h.name); if (install?.name === h.name) install = null; uninstall = { name: h.name, cmd: h.uninstall }; toast.push({ kind: 'ok', title: t('system.hostsRemoved', { name: h.name }) }); }); };
  // the switch (ADR-008): off — the core stops agentd on the server and leaves it alone; on — probes again (agentd must be started there)
  const toggle = (h: HostView) => hostAct(`${h.name}:toggle`, async () => { const v = await api.hostEdit(h.name, { enabled: !!h.disabled }); put(v); toast.push(v.disabled ? { kind: 'ok', title: t('system.hostsSwitchedOff', { name: h.name }) } : v.status === 'online' ? { kind: 'ok', title: t('system.hostsSwitchedOn', { name: h.name }) } : { kind: 'error', title: t('system.hostsSwitchedOnOffline', { name: h.name }) }); });
  // the core pushes its embedded agentd, the server restarts it: the fingerprint is re-read by a probe a few seconds later; track() already shows the error toast
  const update = (h: HostView) => hostAct(`${h.name}:update`, () => toast.track(t('system.hostsUpdating', { name: h.name }), () => api.hostUpdate(h.name), () => { setTimeout(() => probe(h), 5000); return { text: t('system.hostsUpdated') }; }).catch(() => {}));
  const add = () => { const name = hostName.trim(); if (name) hostAct('add', async () => { const v = await api.hostAdd(name, hostNote.trim()); install = { name: v.name, cmd: v.install ?? '' }; hostName = ''; hostNote = ''; }); };
  async function copyText(text: string) { try { await navigator.clipboard.writeText(text); toast.push({ kind: 'ok', title: t('system.hostsCopied') }); } catch { toast.push({ kind: 'error', title: t('system.hostsCopyFailed') }); } }
  const copy = () => { if (install) copyText(install.cmd); };
  const behind = (h: HostView) => h.status === 'online' && !!h.agentd_version && h.agentd_version !== h.core_agentd_version;
  const seen = (iso: string) => { const a = fmt.ago(iso); return a === t('time.now') ? a : t('system.hostsLastSeen', { ago: a }); };
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
<h2 style="margin-top: 16px;">{t('system.hosts')}</h2>
<div class="card">
  <div class="muted" style="margin-bottom: 8px;">{t('system.hostsHint')}</div>
  {#each hosts as h (h.name)}
    <div class="host">
      <div class="row" style="justify-content: space-between;">
        <div class="row">
          <button class="switch host-switch" class:on={!h.disabled} aria-pressed={!h.disabled} aria-label={t('system.hostsSwitch')} title={h.disabled ? t('system.hostsSwitchOnHint') : t('system.hostsSwitchOffHint')} disabled={!!hostBusy || !h.addr} onclick={() => toggle(h)}></button>
          <b>{h.name}</b>
          {#if h.addr}<code>{h.addr}:{h.port}</code>{/if}
          {#if h.disabled}<span class="muted">{t('system.hostsDisabled')}</span>{:else if h.status === 'online'}<span class="badge ok">{t('system.hostsOnline')}</span>{:else if h.status === 'offline'}<span class="badge bad">{t('system.hostsOffline')}</span>{:else}<span class="muted">{t('system.hostsWaiting')}</span>{/if}
          {#if behind(h)}<span class="badge warn">{t('system.hostsBehind')}</span> <button class="primary" disabled={!!hostBusy} onclick={() => update(h)}>{t('system.hostsUpdateAgentd')}</button>{/if}
        </div>
        <div class="row">
          <button disabled={!!hostBusy || !h.addr || h.disabled} onclick={() => probe(h)}>{hostBusy === `${h.name}:probe` ? t('system.hostsProbing') : t('system.hostsProbe')}</button>
          <button disabled={!!hostBusy} onclick={() => code(h)}>{t('system.hostsCode')}</button>
          <button class="danger" disabled={!!hostBusy} onclick={() => remove(h)}>{t('system.hostsRemove')}</button>
        </div>
      </div>
      {#if h.addr}
        <div class="muted">
          {#if h.os || h.arch}{[h.os, h.arch].filter(Boolean).join(' ')} · {/if}
          {#if h.user}{t('system.hostsUser', { user: h.hostname ? `${h.user}@${h.hostname}` : h.user })} · {/if}
          {#if h.agentd_version}<span title={h.agentd_version}>{t('system.hostsAgentd', { version: h.agentd_version.slice(0, 8) })}</span> · {/if}
          {h.llm ? t('system.hostsLlm', { provider: h.llm }) : t('system.hostsLlmNone')}
          {#if h.status === 'offline' && h.last_seen} · {seen(h.last_seen)}{/if}
        </div>
        {#if h.status === 'offline' && h.last_error}<div class="muted">{h.last_error}</div>{/if}
      {/if}
      {#if h.note}<div class="muted">{h.note}</div>{/if}
    </div>
  {:else}
    <div class="muted">{t('system.hostsEmpty')}</div>
  {/each}
  {#if install}
    <div class="install">
      <b>{t('system.hostsInstallTitle', { name: install.name })}</b>
      <div class="row" style="margin-top: 6px;"><code class="cmd">{install.cmd}</code><button onclick={copy}>{t('system.hostsCopy')}</button></div>
      <div class="muted" style="margin-top: 6px;">{t('system.hostsInstallHint')}</div>
    </div>
  {/if}
  {#if uninstall}
    <div class="install">
      <b>{t('system.hostsUninstallTitle', { name: uninstall.name })}</b>
      <div class="row" style="margin-top: 6px;"><code class="cmd">{uninstall.cmd}</code><button onclick={() => copyText(uninstall!.cmd)}>{t('system.hostsCopy')}</button></div>
      <div class="muted" style="margin-top: 6px;">{t('system.hostsUninstallHint')}</div>
    </div>
  {/if}
  <h3 style="margin-top: 12px;">{t('system.hostsAdd')}</h3>
  <div class="row">
    <input placeholder={t('system.hostsName')} bind:value={hostName} maxlength={40} onkeydown={(e) => { if (e.key === 'Enter') add(); }} style="min-width: 220px;" />
    <input placeholder={t('system.hostsNote')} bind:value={hostNote} maxlength={500} onkeydown={(e) => { if (e.key === 'Enter') add(); }} style="flex: 1; min-width: 220px;" />
    <button class="primary" disabled={!!hostBusy || !hostName.trim()} onclick={add}>{hostBusy === 'add' ? t('system.hostsAdding') : t('system.hostsAddBtn')}</button>
  </div>
</div>
{#if pc}
<h2 style="margin-top: 16px;">{t('system.memTitle')}</h2>
<div class="card memg">
  <div class="mrow"><button class="host-switch" class:on={pc.enabled} role="switch" aria-checked={pc.enabled} aria-label={t('system.memWatch')} onclick={() => savePc({ enabled: !pc!.enabled })}></button><span>{t('system.memWatch')}</span></div>
  <div class="mrow" class:off={!pc.enabled}>
    <span class="muted">{t('system.memOnStall')}</span>
    <span class="seg">
      <button class:primary={pc.action === 'kill'} disabled={!pc.enabled} onclick={() => savePc({ action: 'kill' })}>{t('system.memKill')}</button>
      <button class:primary={pc.action === 'notify'} disabled={!pc.enabled} onclick={() => savePc({ action: 'notify' })}>{t('system.memNotify')}</button>
    </span>
  </div>
  <div class="mrow" class:off={!pc.enabled}>
    <span class="muted">{t('system.memThreshold')}</span>
    <input type="number" min="20" max="100" step="5" value={pc.full_pct} disabled={!pc.enabled} aria-label={t('system.memThreshold')} onchange={(e) => savePc({ full_pct: numOf(e, 20, 100) })} /><span class="muted">%</span>
    <span class="muted">{t('system.memLonger')}</span>
    <input type="number" min="0.5" max="60" step="0.5" value={Math.round(pc.hold_secs / 6) / 10} disabled={!pc.enabled} aria-label={t('system.memLonger')} onchange={(e) => savePc({ hold_secs: Math.round(numOf(e, 0.5, 60) * 60) })} /><span class="muted">{t('system.memMin')}</span>
  </div>
  {#if pcProjects.length}
    <div class="mrow" class:off={!pc.enabled || pc.action !== 'kill'}>
      <span class="muted">{t('system.memExempt')}</span>
      {#each pcProjects as name}
        {@const ex = pc.projects?.[name]?.action === 'notify'}
        <label class="badge {ex ? 'warn' : ''}"><input type="checkbox" checked={ex} disabled={!pc.enabled || pc.action !== 'kill'} onchange={(e) => savePc({ projects: { [name]: (e.currentTarget as HTMLInputElement).checked ? { ...(pc!.projects?.[name] ?? {}), action: 'notify' } : null } })} /> {name}</label>
      {/each}
    </div>
  {/if}
  <div class="mrow">
    <span class="muted">{t('system.memLongTurn')}</span>
    <input type="number" min="0" max="168" step="0.25" value={pc.long_turn_hours} aria-label={t('system.memLongTurn')} onchange={(e) => { const v = numOf(e, 0, 168); savePc({ long_turn_hours: v > 0 && v < 0.25 ? 0.25 : v }); }} /><span class="muted">{t('system.memHours')}</span>
  </div>
  <div class="muted small">{t('system.memHint')}</div>
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
      <div class="tscroll"><table class="usage"><thead><tr><th>{t('system.usageProject')}</th><th>$</th><th>↑</th><th>↓</th></tr></thead><tbody>{#each usage.by_project as b}<tr><td>{b.key}</td><td>{b.cost_usd.toFixed(2)}</td><td>{fmtK(b.input + b.cache_read + b.cache_create)}</td><td>{fmtK(b.output)}</td></tr>{/each}</tbody></table></div>
      <div class="tscroll"><table class="usage"><thead><tr><th>{t('system.usageModel')}</th><th>$</th><th>{t('system.usageTurnsShort')}</th></tr></thead><tbody>{#each usage.by_model as b}<tr><td>{b.key}</td><td>{b.cost_usd.toFixed(2)}</td><td>{b.turns}</td></tr>{/each}</tbody></table></div>
      <div class="tscroll"><table class="usage"><thead><tr><th>{t('system.usageDay')}</th><th>$</th></tr></thead><tbody>{#each usage.by_day.slice(-7) as b}<tr><td>{b.key}</td><td>{b.cost_usd.toFixed(2)}</td></tr>{/each}</tbody></table></div>
    </div>
  {:else}<span class="muted">{t('system.usageEmpty')}</span>{/if}
</div>
<h2 style="margin-top: 16px;">{t('system.events')}</h2>
<div class="card">
  {#each events as e}<div class="muted"><code>{e.at.slice(0, 19).replace('T', ' ')}</code> <b>{e.kind}</b> {e.subject} — {e.message}</div>{/each}
  {#if !events.length}<div class="muted">{t('common.empty')}</div>{/if}
</div>

<style>
  /* the server switch (ADR-008): a pill, on = accent */
  .host-switch { position: relative; width: 36px; height: 20px; border-radius: 999px; background: var(--muted, #666); border: 0; padding: 0; cursor: pointer; flex: none; opacity: .55; }
  .host-switch::after { content: ''; position: absolute; top: 2px; left: 2px; width: 16px; height: 16px; border-radius: 50%; background: #fff; transition: transform .15s; }
  .host-switch.on { background: var(--accent); opacity: 1; }
  .host-switch.on::after { transform: translateX(16px); }
  .host-switch:disabled { cursor: default; opacity: .35; }
  .host { padding: 8px 0; border-top: 1px solid color-mix(in srgb, var(--line) 50%, transparent); }
  .install { margin-top: 10px; padding: 10px 12px; border: 1px solid var(--accent); border-radius: var(--r-sm); }
  .cmd { flex: 1; min-width: 0; padding: 6px 8px; background: var(--panel-2); border-radius: var(--r-sm); white-space: pre-wrap; word-break: break-all; user-select: all; }
  /* memory guard: rows of a label and its controls, wrapping on a narrow window */
  .memg { display: flex; flex-direction: column; gap: calc(10px * var(--sp)); }
  .mrow { display: flex; flex-wrap: wrap; align-items: center; gap: calc(8px * var(--sp)); }
  .mrow.off { opacity: .55; }
  .mrow input[type=number] { width: 5.2em; }
  .mrow .badge { cursor: pointer; gap: 5px; }
  .seg { display: inline-flex; flex-wrap: wrap; gap: 6px; }
  .small { font-size: .8rem; }
  table.usage { border-collapse: collapse; font-size: .85rem; }
  table.usage th, table.usage td { text-align: left; padding: 2px 10px 2px 0; }
  table.usage th { color: var(--muted); font-weight: 500; }
</style>
