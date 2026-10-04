<script lang="ts">
  // Приложение «Агенты» (ADR-006, этап 2): собственный app shell со своей тёмной темой — одинаково отдельной вкладкой
  // (/?app=agents) и внутри окна Desktop (токены OS переопределены на корне, поэтому Chat/Term/кнопки следуют теме приложения).
  // Слева дерево workspace'ов с сессиями и терминалами из cloudos-agentd, сверху режимы Agent · Code · Thread · Chat,
  // в центре холст карточек (Code: несколько сессий разных workspace'ов рядом, ширина/высота, перетаскивание) или одна
  // сессия на весь экран (Thread). Раскладка холста хранится в ядре (kv agents.layout) и общая для всех устройств.
  import { onMount } from 'svelte';
  import { api, type ProjectView, type PtyInfo, type SessionInfo, type Account, type UsageSummary } from './api';
  import SessionPane from './SessionPane.svelte';
  import { wm } from './windows.svelte';
  import { device } from './device.svelte';
  import { t, tn } from './i18n.svelte';
  type Ref = { project: string; kind: 'pty' | 'session'; id: string };
  type Card = Ref & { w: 1 | 2; h: 1 | 2 };
  type Mode = 'agent' | 'code' | 'thread' | 'chat';
  const standalone = new URLSearchParams(location.search).get('app') === 'agents';
  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));
  const key = (r: Ref) => `${r.project}:${r.id}`;
  const same = (a: Ref, b: Ref) => a.project === b.project && a.id === b.id;

  let projects = $state<ProjectView[]>([]);
  let ptys = $state<Record<string, PtyInfo[]>>({});
  let sessions = $state<Record<string, SessionInfo[]>>({});
  let reach = $state<Record<string, boolean>>({});
  let accounts = $state<Account[]>([]);
  let defaults = $state<Record<string, string>>({});
  let usage = $state<UsageSummary | null>(null);
  let error = $state('');
  let busy = $state('');
  let mode = $state<Mode>(device.phone ? 'thread' : 'code');
  let sel = $state<Ref | null>(null);
  let cards = $state<Card[]>([]);
  let cols = $state<0 | 1 | 2 | 3>(0);
  let collapsed = $state<Record<string, boolean>>({});
  let sideOpen = $state(false);
  let menuFor = $state('');
  let flash = $state('');
  let dragIdx = $state(-1);
  let notify = $state<{ app: string | null; server: string | null; topic: string; user: string; password: string | null; token: string | null } | null>(null);
  let notifyShown = $state(false);

  // локальная память вкладки (режим, выбор, раскладка до ответа ядра); раскладка холста — в ядре, общая для устройств
  const LS = 'cloudos.agents.ui';
  try {
    const u = JSON.parse(localStorage.getItem(LS) || '{}');
    if (u.mode) mode = u.mode; if (u.sel) sel = u.sel; if (Array.isArray(u.cards)) cards = u.cards; if (u.cols != null) cols = u.cols; if (u.collapsed) collapsed = u.collapsed;
  } catch {}
  $effect(() => { const u = { mode, sel, cards, cols, collapsed }; try { localStorage.setItem(LS, JSON.stringify(u)); } catch {} });
  let loaded = false; let lastSaved = ''; let saveT: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const snap = JSON.stringify({ cards, cols });
    if (!loaded || snap === lastSaved) return;
    clearTimeout(saveT); saveT = setTimeout(() => { lastSaved = snap; api.agentsLayoutPut(JSON.parse(snap)).catch(() => {}); }, 800);
  });
  onMount(() => {
    api.agentsLayoutGet().then(l => {
      if (l && Array.isArray(l.cards)) { cards = l.cards; cols = l.cols ?? 0; lastSaved = JSON.stringify({ cards: l.cards, cols: l.cols ?? 0 }); }
    }).catch(() => {}).finally(() => { loaded = true; });
  });

  const running = $derived(projects.filter(p => p.workspace?.status === 'running'));
  const stopped = $derived(projects.filter(p => p.workspace?.status !== 'running'));
  async function load() {
    try { projects = await api.projects(); error = ''; } catch (e) { error = msg(e); return; }
    try { const a = await api.accounts(); accounts = a.accounts; defaults = a.defaults; } catch {}
    for (const p of projects.filter(p => p.workspace?.status === 'running')) {
      try { [sessions[p.name], ptys[p.name]] = await Promise.all([api.sessions(p.name), api.ptys(p.name)]); reach[p.name] = true; }
      catch { reach[p.name] = false; sessions[p.name] = sessions[p.name] ?? []; ptys[p.name] = ptys[p.name] ?? []; }
    }
    // карточки исчезнувших сессий снимаем с холста; недоступный agentd — не повод
    const next = cards.filter(c => reach[c.project] !== true || infoOf(c));
    if (next.length !== cards.length) cards = next;
  }
  async function loadUsage() { try { usage = await api.usageSummary(1); } catch {} }
  $effect(() => { load(); const i = setInterval(load, 5000); return () => clearInterval(i); });
  $effect(() => { loadUsage(); const i = setInterval(loadUsage, 60000); return () => clearInterval(i); });

  function infoOf(r: Ref): SessionInfo | PtyInfo | null {
    return r.kind === 'pty' ? ptys[r.project]?.find(p => p.id === r.id) ?? null : sessions[r.project]?.find(s => s.id === r.id) ?? null;
  }
  function titleOf(r: Ref): string { const i = infoOf(r); return !i ? '…' : r.kind === 'pty' ? ((i as PtyInfo).title || (i as PtyInfo).cmd.join(' ')) : (i as SessionInfo).title; }
  function glyph(r: Ref) { if (r.kind === 'pty') return '⌨'; const a = (infoOf(r) as SessionInfo | null)?.agent; return a === 'codex' ? '◎' : '✳'; }
  function dot(r: Ref) {
    const i = infoOf(r); if (!i) return '';
    if (r.kind === 'pty') return i.alive ? 'ok' : '';
    const s = i as SessionInfo; return s.status === 'waiting' ? 'wait' : s.alive ? (s.status === 'working' ? 'work' : 'ok') : '';
  }
  function hint(r: Ref) {
    const i = infoOf(r); if (!i) return '';
    if (r.kind === 'pty') return i.alive ? ((i as PtyInfo).clients ? tn('agents.clients', (i as PtyInfo).clients) : '') : t('agents.exitedShort');
    const s = i as SessionInfo; return s.status === 'waiting' ? '✋' + (s.pending ? ' ' + s.pending : '') : s.status === 'working' ? '⏳' : s.alive ? '' : t('agents.exitedShort');
  }
  const onCanvas = (r: Ref) => cards.some(c => same(c, r));
  const current = $derived(sel ? infoOf(sel) : null);

  // ---- навигация ----
  function open(r: Ref) { sel = r; if (mode === 'code') addCard(r); else mode = 'thread'; sideOpen = false; }
  function addCard(r: Ref) { if (!onCanvas(r)) cards = [...cards, { ...r, w: 1, h: 1 }]; focusCard(r); }
  function focusCard(r: Ref) {
    flash = key(r); setTimeout(() => { document.querySelector(`[data-card="${key(r)}"]`)?.scrollIntoView({ block: 'nearest', behavior: 'smooth' }); }, 60); setTimeout(() => { if (flash === key(r)) flash = ''; }, 1600);
  }
  function removeCard(r: Ref) { cards = cards.filter(c => !same(c, r)); }
  function expand(r: Ref) { sel = r; mode = 'thread'; }
  function toCanvas(r: Ref) { mode = 'code'; addCard(r); }
  function resize(r: Ref, w?: 1 | 2, h?: 1 | 2) { cards = cards.map(c => (same(c, r) ? { ...c, w: w ?? c.w, h: h ?? c.h } : c)); }
  function setMode(m: Mode) { if (m === 'agent' || m === 'chat') return; mode = m; if (m === 'code' && sel && !onCanvas(sel)) addCard(sel); }
  function drop(i: number, e: DragEvent) {
    e.preventDefault();
    if (dragIdx >= 0) { const arr = [...cards]; const [m] = arr.splice(dragIdx, 1); arr.splice(i, 0, m); cards = arr; dragIdx = -1; return; }
    const raw = e.dataTransfer?.getData('application/x-cloudos-ref'); if (!raw) return;
    try { const r = JSON.parse(raw) as Ref; if (!onCanvas(r)) { const arr = [...cards]; arr.splice(i, 0, { ...r, w: 1, h: 1 }); cards = arr; } focusCard(r); } catch {}
  }
  function dropCanvas(e: DragEvent) { drop(cards.length, e); }

  // ---- действия ----
  // TUI-агенты стартуют через интерактивную login-оболочку: она читает /etc/bash.bashrc и profile.d — там PATH к CLI и env gate
  const CMD: Record<'shell' | 'claude' | 'codex', string[] | undefined> = { shell: undefined, claude: ['bash', '-lic', 'exec claude'], codex: ['bash', '-lic', 'exec codex'] };
  async function createPty(project: string, kind: 'shell' | 'claude' | 'codex') {
    busy = project; menuFor = '';
    try { const p = await api.ptyCreate(project, { cmd: CMD[kind], title: kind === 'shell' ? t('agents.newShell') : kind === 'claude' ? 'Claude Code' : 'Codex' }); await load(); open({ project, kind: 'pty', id: p.id }); }
    catch (e) { error = msg(e); } finally { busy = ''; }
  }
  // карточка Chat: провайдер Claude (stream-json) или Codex (app-server) — один контракт сессии в agentd
  async function createChat(project: string, agent: 'claude' | 'codex') {
    busy = project; menuFor = '';
    try { const s = await api.sessionCreate(project, { agent, title: agent === 'codex' ? 'Codex' : 'Claude' }); await load(); open({ project, kind: 'session', id: s.id }); }
    catch (e) { error = msg(e); } finally { busy = ''; }
  }
  async function setAccount(project: string, id: string) {
    try { await api.projectAccount(project, id || null); const d = { ...defaults }; if (id) d[project] = id; else delete d[project]; defaults = d; } catch (e) { error = msg(e); }
  }
  async function reassign(r: Ref, account: string) { try { await api.sessionAccount(r.project, r.id, account || null); await load(); } catch (e) { error = msg(e); } }
  async function changeModel(r: Ref) {
    const cur = (infoOf(r) as SessionInfo | null)?.model ?? '';
    const m = window.prompt(t('agents.modelPrompt'), cur); if (m === null) return;
    try { await api.sessionModel(r.project, r.id, m.trim() || null); await load(); } catch (e) { error = msg(e); }
  }
  async function stop(r: Ref) {
    try {
      if (r.kind === 'pty') await api.ptyKill(r.project, r.id); else await api.sessionDelete(r.project, r.id);
      await load();
      if (!infoOf(r)) { removeCard(r); if (sel && same(sel, r)) sel = null; }
    } catch (e) { error = msg(e); }
  }
  function ide(project: string) {
    const p = projects.find(p => p.name === project); const a = p?.workspace?.apps.find(a => a.slug === 'code-server'); if (!p || !a) return;
    if (standalone) window.open(a.url, '_blank'); else wm.open(`ws:${p.name}:${a.slug}`, `${p.name} · ${a.display_name}`, a.url, '🧩');
  }
  const hasIde = (project: string) => !!projects.find(p => p.name === project)?.workspace?.apps.some(a => a.slug === 'code-server');
  async function showNotify() { try { notify = await api.agentsNotify(); notifyShown = true; } catch (e) { error = msg(e); } }
  function copy(s: string) { try { navigator.clipboard?.writeText(s); } catch {} }
  function dragRef(e: DragEvent, r: Ref) { e.dataTransfer?.setData('application/x-cloudos-ref', JSON.stringify(r)); }
</script>

<svelte:window onclick={() => (menuFor = '')} onkeydown={(e) => { if (e.key === 'Escape') { menuFor = ''; sideOpen = false; } }} />

<div class="agents" class:side-open={sideOpen} data-mode={mode}>
  <aside class="side">
    <div class="side-head"><span>{t('agents.workspaces')}</span>{#if standalone}<a class="foot-link" href="/" title={t('agents.openDesktop')}>Desktop ↗</a>{:else}<a class="foot-link" href="/?app=agents" target="_blank" rel="noopener" title={t('agents.openStandalone')}>↗</a>{/if}</div>
    <div class="tree">
      {#if error}<div class="err">{error}</div>{/if}
      {#if !running.length}<div class="muted hint">{t('agents.noRunning')}</div>{/if}
      {#each running as p (p.name)}
        {@const items = [...(sessions[p.name] ?? []).map(s => ({ project: p.name, kind: 'session' as const, id: s.id })), ...(ptys[p.name] ?? []).filter(s => !s.session).map(s => ({ project: p.name, kind: 'pty' as const, id: s.id }))]}
        {@const live = items.filter(r => infoOf(r)?.alive).length}
        <div class="proj" class:menu-open={menuFor === p.name}>
          <div class="prow">
            <button class="chev" onclick={() => (collapsed[p.name] = !collapsed[p.name])} aria-label="toggle">{collapsed[p.name] ? '▸' : '▾'}</button>
            <span class="pname" title={p.spec.description ?? ''}>{p.name}</span>
            {#if reach[p.name] === false}<span class="badge warn" title={t('agents.noAgentd')}>agentd</span>{:else if live}<span class="cnt">{live}</span>{/if}
            <button class="add" disabled={busy === p.name || reach[p.name] === false} title={t('agents.new')} onclick={(e) => { e.stopPropagation(); menuFor = menuFor === p.name ? '' : p.name; }}>＋</button>
          </div>
          {#if menuFor === p.name}
            <div class="menu" role="menu" onclick={(e) => e.stopPropagation()}>
              <button onclick={() => createChat(p.name, 'claude')}>💬 Claude</button>
              <button onclick={() => createChat(p.name, 'codex')}>💬 Codex</button>
              <button onclick={() => createPty(p.name, 'shell')}>⌨ {t('agents.newShell')}</button>
              <button onclick={() => createPty(p.name, 'claude')}>⌨ Claude</button>
              <button onclick={() => createPty(p.name, 'codex')}>⌨ Codex</button>
              {#if accounts.length}
                <label class="acc">{t('agents.account')}
                  <select value={defaults[p.name] ?? ''} onchange={(e) => setAccount(p.name, (e.target as HTMLSelectElement).value)}>
                    <option value="">{t('agents.accountGate')}</option>
                    {#each accounts as a (a.id)}<option value={a.id}>{a.label}</option>{/each}
                  </select>
                </label>
              {/if}
            </div>
          {/if}
          {#if !collapsed[p.name]}
            {#each items as r (key(r))}
              {@const i = infoOf(r)}
              <button class="sess" data-id={r.id} class:active={sel && same(sel, r)} class:dead={!i?.alive} draggable="true" ondragstart={(e) => dragRef(e, r)} onclick={() => open(r)} title={r.kind === 'session' ? ((i as SessionInfo | null)?.last_text ?? '') : ''}>
                <span class="dot {dot(r)}"></span><span class="g">{glyph(r)}</span><span class="stitle">{titleOf(r)}</span>
                {#if onCanvas(r) && mode === 'code'}<span class="onc" title="Code">▦</span>{/if}
                <span class="muted sm">{hint(r)}</span>
              </button>
            {/each}
          {/if}
        </div>
      {/each}
      {#each stopped as p (p.name)}
        <div class="proj off"><div class="prow"><span class="chev">·</span><span class="pname">{p.name}</span><span class="muted sm">{p.workspace?.status ?? '—'}</span></div></div>
      {/each}
    </div>
    <div class="side-foot">
      {#if usage}<div class="row" style="justify-content: space-between;"><span>{t('agents.today')}</span><b>${usage.total.cost_usd.toFixed(2)} · {tn('agents.turns', usage.total.turns)}</b></div>{/if}
      {#if !notifyShown}<button class="link" onclick={showNotify}>🔔 {t('agents.notifyPhone')}</button>
      {:else if notify}
        {#if notify.app}
          <div>{t('agents.notifyHow')}</div>
          <div class="kv"><span>ntfy</span><code>{notify.server ?? '—'}</code></div>
          <div class="kv"><span>{t('agents.notifyTopic')}</span><code>{notify.topic}</code></div>
          <div class="kv"><span>{t('agents.notifyUser')}</span><code>{notify.user}</code></div>
          <div class="kv"><span>{t('agents.notifyPassword')}</span><code>{notify.password ?? '—'}</code>{#if notify.password}<button class="mini" onclick={() => copy(notify!.password!)}>⧉</button>{/if}</div>
        {:else}<div>{t('agents.notifyNoNtfy')}</div>{/if}
      {/if}
    </div>
  </aside>
  {#if sideOpen}<div class="scrim" onclick={() => (sideOpen = false)} role="presentation"></div>{/if}

  <div class="top">
    <button class="burger" onclick={(e) => { e.stopPropagation(); sideOpen = !sideOpen; }} title={t('agents.sidebar')}>☰</button>
    {#if standalone}<span class="brand">AgentVerse OS</span>{/if}
    <div class="modes" role="tablist">
      <button role="tab" disabled title={t('agents.modeAgentHint')}>Agent</button>
      <button role="tab" class:active={mode === 'code'} title={t('agents.modeCodeHint')} onclick={() => setMode('code')}>Code</button>
      <button role="tab" class:active={mode === 'thread'} title={t('agents.modeThreadHint')} onclick={() => setMode('thread')}>Thread</button>
      <button role="tab" disabled title={t('agents.modeChatHint')}>Chat</button>
    </div>
    {#if mode === 'code'}
      <div class="cols" title={t('agents.cols')}>
        {#each [0, 1, 2, 3] as n}<button class:active={cols === n} onclick={() => (cols = n as 0 | 1 | 2 | 3)}>{n === 0 ? t('agents.colsAuto') : n}</button>{/each}
      </div>
    {/if}
  </div>

  <div class="main">
    {#if mode === 'code'}
      <div class="canvas c{cols}" ondragover={(e) => e.preventDefault()} ondrop={dropCanvas} role="list">
        {#each cards as c, i (key(c))}
          {@const info = infoOf(c)}
          <div class="card" class:w2={c.w === 2} class:h2={c.h === 2} class:flash={flash === key(c)} class:active={sel && same(sel, c)} data-card={key(c)} role="listitem" ondragover={(e) => e.preventDefault()} ondrop={(e) => drop(i, e)}>
            <div class="chead" draggable="true" ondragstart={(e) => { dragIdx = i; e.dataTransfer?.setData('text/plain', key(c)); }} ondragend={() => (dragIdx = -1)} onclick={() => (sel = c)} role="presentation">
              <span class="dot {dot(c)}"></span><span class="g">{glyph(c)}</span><span class="ttl">{titleOf(c)}</span><span class="sub">{c.project}</span>
              {#if c.kind === 'session'}<button class="chip model" title={t('agents.modelHint')} onclick={(e) => { e.stopPropagation(); changeModel(c); }}>{(info as SessionInfo | null)?.model ?? t('agents.modelDefault')}</button>{/if}
              <span class="sp"></span>
              {#if cols !== 1}<button class="ico" title={t('agents.wider')} onclick={(e) => { e.stopPropagation(); resize(c, c.w === 2 ? 1 : 2); }}>⇔</button>{/if}
              <button class="ico" title={t('agents.taller')} onclick={(e) => { e.stopPropagation(); resize(c, undefined, c.h === 2 ? 1 : 2); }}>↕</button>
              <button class="ico" title={t('agents.expand')} onclick={(e) => { e.stopPropagation(); expand(c); }}>⤢</button>
              <button class="ico danger" title={info?.alive ? t('agents.kill') : t('agents.remove')} onclick={(e) => { e.stopPropagation(); stop(c); }}>{info?.alive ? '■' : '🗑'}</button>
              <button class="ico" title={t('agents.fromCanvas')} onclick={(e) => { e.stopPropagation(); removeCard(c); }}>✕</button>
            </div>
            <div class="cbody">
              {#if !info}<div class="muted hint">{reach[c.project] === false ? t('agents.noAgentd') : t('agents.connecting')}</div>
              {:else}<SessionPane project={c.project} id={c.id} kind={c.kind} cwd={info.cwd} onchange={() => load()} compact />{/if}
            </div>
          </div>
        {/each}
        {#if !cards.length}<div class="empty muted">{t('agents.canvasEmpty')}</div>{/if}
      </div>
    {:else if sel && current}
      <div class="thread">
        <div class="head">
          <span class="dot {dot(sel)}"></span><span class="g">{glyph(sel)}</span><b>{titleOf(sel)}</b><span class="muted">· {sel.project} · {current.cwd}</span>
          {#if sel.kind === 'session'}<button class="chip model" title={t('agents.modelHint')} onclick={() => changeModel(sel!)}>🧠 {(current as SessionInfo).model ?? t('agents.modelDefault')}</button>{/if}
          <span class="sp"></span>
          {#if !device.phone}<button class="mini" title={t('agents.modeCodeHint')} onclick={() => toCanvas(sel!)}>▦ {t('agents.toCanvas')}</button>{/if}
          {#if hasIde(sel.project)}<button class="mini" onclick={() => ide(sel!.project)}>{t('agents.openIde')} ↗</button>{/if}
          {#if sel.kind === 'session' && accounts.length}
            <select class="mini" title={t('agents.accountReassign')} value={(current as SessionInfo).account ?? ''} onchange={(e) => reassign(sel!, (e.target as HTMLSelectElement).value)}>
              <option value="">{t('agents.accountGate')}</option>
              {#each accounts as a (a.id)}<option value={a.id}>{a.label}</option>{/each}
            </select>
          {/if}
          <button class="mini danger" onclick={() => stop(sel!)}>{current.alive ? t('agents.kill') : t('agents.remove')}</button>
        </div>
        {#key key(sel)}
          <SessionPane project={sel.project} id={sel.id} kind={sel.kind} cwd={current.cwd} onchange={() => load()} />
        {/key}
      </div>
    {:else}
      <div class="empty muted">{t('agents.pick')}</div>
    {/if}
  </div>
</div>

<style>
  /* Своя тема приложения. Токены OS переопределяются на корне, чтобы общие компоненты (Chat, Term, кнопки) следовали ей. */
  .agents {
    --ag-bg: #0b0d12; --ag-panel: #0f1117; --ag-panel-2: #161a22; --ag-line: #232937; --ag-ink: #e6e8ee; --ag-muted: #8b93a7;
    --ag-accent: #7aa2ff; --ag-accent-rgb: 122 162 255; --ag-ok: #3fb950; --ag-warn: #e3b341; --ag-bad: #f0625d;
    --bg: var(--ag-bg); --panel: var(--ag-panel); --panel-2: var(--ag-panel-2); --line: var(--ag-line); --ink: var(--ag-ink); --muted: var(--ag-muted);
    --accent: var(--ag-accent); --accent-rgb: var(--ag-accent-rgb); --on-accent: #0b0d12; --ok: var(--ag-ok); --warn: var(--ag-warn); --bad: var(--ag-bad);
    --r: 12px; --r-sm: 8px; --glass: var(--ag-panel); --shadow: none; --sp: 1; --font: Inter, ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
    font: 14px/1.45 var(--font); color: var(--ag-ink); background: var(--ag-bg);
    display: grid; grid-template-columns: 264px minmax(0, 1fr); grid-template-rows: 46px minmax(0, 1fr); height: 100%; min-height: 0; position: relative; overflow: hidden;
  }
  .agents :global(button) { border-radius: 8px; }
  .agents :global(button:hover) { border-color: rgb(var(--ag-accent-rgb) / .6); }
  .muted { color: var(--ag-muted); }
  .sm { font-size: 11.5px; }
  .row { display: flex; align-items: center; gap: 8px; }
  .sp { flex: 1; }
  .err { color: var(--ag-bad); font-size: 12px; padding: 4px 8px; }
  .hint { font-size: 12.5px; padding: 8px; }
  .badge.warn { font-size: 10.5px; color: var(--ag-warn); border: 1px solid rgb(227 179 65 / .4); border-radius: 999px; padding: 0 6px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: #3a4150; flex: none; }
  .dot.ok { background: var(--ag-ok); }
  .dot.work { background: var(--ag-accent); animation: pulse 1.2s infinite; }
  .dot.wait { background: var(--ag-warn); box-shadow: 0 0 0 3px rgb(227 179 65 / .2); }
  @keyframes pulse { 50% { opacity: .35; } }
  .g { width: 16px; text-align: center; color: var(--ag-muted); font-size: 13px; flex: none; }

  /* сайдбар */
  .side { grid-row: 1 / 3; display: flex; flex-direction: column; min-height: 0; border-right: 1px solid var(--ag-line); background: var(--ag-panel); }
  .side-head { display: flex; align-items: center; justify-content: space-between; padding: 13px 14px 8px; font-size: 11.5px; font-weight: 600; letter-spacing: .06em; text-transform: uppercase; color: var(--ag-muted); }
  .foot-link { color: var(--ag-muted); text-decoration: none; font-weight: 500; letter-spacing: 0; text-transform: none; }
  .foot-link:hover { color: var(--ag-ink); }
  .tree { flex: 1; overflow: auto; padding: 0 8px 8px; }
  .proj { margin-bottom: 2px; position: relative; }
  .proj.off { opacity: .5; }
  .prow { display: flex; align-items: center; gap: 6px; padding: 5px 4px 5px 2px; border-radius: 8px; }
  .prow:hover { background: var(--ag-panel-2); }
  .chev { background: transparent; border: 0; padding: 0; width: 18px; color: var(--ag-muted); font-size: 11px; }
  .pname { flex: 1; min-width: 0; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cnt { font-size: 11px; color: var(--ag-muted); background: var(--ag-panel-2); border-radius: 999px; padding: 0 7px; }
  .add { width: 22px; height: 22px; padding: 0; line-height: 1; font-size: 15px; background: transparent; border-color: transparent; color: var(--ag-muted); }
  .add:hover, .menu-open .add { background: var(--ag-panel-2); color: var(--ag-ink); }
  .menu { position: absolute; right: 4px; top: 30px; z-index: 30; min-width: 190px; background: var(--ag-panel); border: 1px solid var(--ag-line); border-radius: 10px; padding: 6px; display: grid; gap: 2px; box-shadow: 0 14px 40px rgb(0 0 0 / .55); }
  .menu button { text-align: left; background: transparent; border-color: transparent; padding: 6px 10px; font-size: 13px; }
  .menu button:hover { background: var(--ag-panel-2); }
  .menu .acc { display: flex; flex-direction: column; gap: 4px; font-size: 11.5px; color: var(--ag-muted); padding: 6px 10px 4px; border-top: 1px solid var(--ag-line); margin-top: 4px; }
  .menu .acc select { font-size: 12px; padding: 3px 6px; }
  .sess { display: flex; align-items: center; gap: 7px; width: 100%; text-align: left; background: transparent; border: 1px solid transparent; padding: 5px 8px 5px 20px; border-radius: 8px; font-size: 13px; }
  .sess:hover { background: var(--ag-panel-2); border-color: transparent; }
  .sess.active { background: rgb(var(--ag-accent-rgb) / .14); border-color: rgb(var(--ag-accent-rgb) / .35); }
  .sess.dead { opacity: .55; }
  .stitle { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .onc { font-size: 10px; color: var(--ag-muted); }
  .side-foot { border-top: 1px solid var(--ag-line); padding: 10px 12px; font-size: 12px; color: var(--ag-muted); display: flex; flex-direction: column; gap: 6px; }
  .side-foot b { color: var(--ag-ink); font-weight: 600; }
  .link { background: none; border: 0; padding: 0; color: inherit; cursor: pointer; font: inherit; text-align: left; }
  .link:hover { color: var(--ag-ink); }
  .kv { display: flex; gap: 6px; align-items: center; margin-top: 2px; }
  .kv span { min-width: 60px; }
  .kv code { user-select: all; font-size: 11.5px; }
  .scrim { display: none; }

  /* верхняя панель */
  .top { grid-column: 2; display: flex; align-items: center; gap: 10px; padding: 0 12px; border-bottom: 1px solid var(--ag-line); background: var(--ag-panel); }
  .burger { display: none; background: transparent; border-color: transparent; padding: 4px 8px; }
  .brand { font-weight: 700; letter-spacing: .01em; font-size: 13px; color: var(--ag-muted); }
  .modes { margin: 0 auto; display: flex; background: var(--ag-panel-2); border-radius: 10px; padding: 3px; gap: 2px; }
  .modes button { background: transparent; border-color: transparent; padding: 4px 16px; border-radius: 8px; color: var(--ag-muted); font-weight: 600; font-size: 13px; }
  .modes button.active { background: var(--ag-bg); color: var(--ag-ink); box-shadow: 0 1px 2px rgb(0 0 0 / .5); }
  .modes button:disabled { opacity: .4; cursor: not-allowed; }
  .cols { display: flex; gap: 2px; }
  .cols button { background: transparent; border-color: transparent; color: var(--ag-muted); padding: 2px 8px; font-size: 12px; }
  .cols button.active { color: var(--ag-ink); background: var(--ag-panel-2); }

  /* холст */
  .main { grid-column: 2; min-height: 0; overflow: auto; padding: 12px; }
  .canvas { display: grid; gap: 12px; grid-auto-rows: 420px; grid-auto-flow: dense; min-height: 100%; align-content: start; }
  .canvas.c0 { grid-template-columns: repeat(auto-fill, minmax(400px, 1fr)); }
  .canvas.c1 { grid-template-columns: minmax(0, 1fr); }
  .canvas.c2 { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .canvas.c3 { grid-template-columns: repeat(3, minmax(0, 1fr)); }
  .card { display: flex; flex-direction: column; min-height: 0; min-width: 0; background: var(--ag-panel); border: 1px solid var(--ag-line); border-radius: 12px; overflow: hidden; transition: box-shadow .3s; }
  .canvas:not(.c1) .card.w2 { grid-column: span 2; }
  .card.h2 { grid-row: span 2; }
  .card.active { border-color: #2e3646; }
  .card.flash { box-shadow: 0 0 0 2px var(--ag-accent); }
  .chead { display: flex; align-items: center; gap: 7px; padding: 6px 8px 6px 12px; border-bottom: 1px solid var(--ag-line); background: var(--ag-panel-2); cursor: grab; user-select: none; font-size: 13px; }
  .chead:active { cursor: grabbing; }
  .ttl { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sub { color: var(--ag-muted); font-size: 12px; white-space: nowrap; }
  .chip { font-size: 11.5px; padding: 1px 8px; border-radius: 999px; background: transparent; color: var(--ag-muted); max-width: 200px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ico { width: 24px; height: 24px; padding: 0; line-height: 1; font-size: 12px; background: transparent; border-color: transparent; color: var(--ag-muted); flex: none; }
  .ico:hover { background: var(--ag-bg); color: var(--ag-ink); }
  .ico.danger:hover { color: var(--ag-bad); }
  .cbody { flex: 1; min-height: 0; padding: 8px; display: flex; }
  .cbody :global(.pane) { flex: 1; min-height: 0; }
  .empty { grid-column: 1 / -1; display: grid; place-items: center; min-height: 240px; text-align: center; padding: 24px; max-width: 560px; margin: 0 auto; font-size: 14px; }

  /* одна сессия */
  .thread { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 8px; }
  .thread :global(.pane) { flex: 1; min-height: 0; }
  .head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .head .model { max-width: 260px; }
  .mini { font-size: 12.5px; padding: 3px 9px; }
  .mini.danger { color: var(--ag-bad); }
  select.mini { font-size: 12.5px; padding: 2px 6px; max-width: 220px; }

  @media (max-width: 900px) {
    .agents { grid-template-columns: minmax(0, 1fr); }
    .side { position: absolute; top: 0; bottom: 0; left: 0; width: min(300px, 86vw); z-index: 40; transform: translateX(-100%); transition: transform .2s; box-shadow: 0 0 40px rgb(0 0 0 / .6); }
    .side-open .side { transform: none; }
    .scrim { display: block; position: absolute; inset: 0; z-index: 30; background: rgb(0 0 0 / .45); }
    .top, .main { grid-column: 1; }
    .burger { display: inline-block; }
    .brand { display: none; }
    .canvas.c0, .canvas.c2, .canvas.c3 { grid-template-columns: minmax(0, 1fr); }
    .canvas:not(.c1) .card.w2 { grid-column: auto; }
    .main { padding: 8px; }
  }
</style>
