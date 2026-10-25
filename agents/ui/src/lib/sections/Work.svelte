<script lang="ts">
  // The "Workplaces" section: a panel with the workspace tree, the card canvas (Code) or one session (Thread), the
  // inspector.
  import { ui, data, actions, key, same, titleOf, agentOf, dotOf, statusText, shortModel, money, ago, phone, modeLabel, live, stopLabel, activity, TREE_LIMIT, accountMenu, ENGINES, engineIcon, type Ref } from '../state.svelte';
  import { dnd } from '../dnd.svelte';
  import { menu } from '../menu.svelte';
  import { t, tn } from '../i18n';
  import Mark from '../Mark.svelte';
  import Icon from '../Icon.svelte';
  import Card from './Card.svelte';
  import Sidebar from './Sidebar.svelte';
  import SessionPane from '../SessionPane.svelte';
  /** Worktree name from the session cwd (docs/agents-ux.md §4.3.6): the "⎇ name"
   *  chip in the tree. */
  const wtOf = (r: Ref) => (r.kind === 'session' ? ((data.infoOf(r)?.cwd ?? '').match(/\/\.worktrees\/([^/]+)/)?.[1] ?? '') : '');
  import { agents, download, downloadUrl, fileName, type SessionInfo, type PtyInfo, type SearchResult, type SearchHit } from '../api';
  import { toast } from '../toast.svelte';
  import { tick } from 'svelte';
  let { onpalette }: { onpalette: () => void } = $props();
  const q = $derived(ui.q.trim().toLowerCase());
  // message search (slice 34, §4.6): from three characters, 400 ms after the last one — the service's /search (agentd of all running workspaces +
  // simple chats)
  let hits = $state<SearchResult | null>(null); let searching = $state(false); let searchError = $state(''); let searchRetry = $state(0); let searchSeq = 0;
  $effect(() => {
    const text = ui.q.trim(); const seq = ++searchSeq; void searchRetry;
    if (text.length < 3) { hits = null; searching = false; searchError = ''; return; }
    searching = true; searchError = '';
    const h = setTimeout(async () => {
      try { const r = await agents.search(text); if (seq !== searchSeq) return; hits = r; await tick(); document.querySelector('.panel .p-body')?.scrollTo({ top: 0 }); }
      catch (e) { if (seq === searchSeq) searchError = msgOf(e); }
      finally { if (seq === searchSeq) searching = false; }
    }, 400);
    return () => clearTimeout(h);
  });
  /** Snippet with the query words highlighted — in segments, without @html (the odd parts of a split by group are
   *  the matches). */
  function marked(snippet: string, words: string[]): { t: string; hit: boolean }[] {
    if (!words.length) return [{ t: snippet, hit: false }];
    const escaped = words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/е/g, '[её]'));
    const re = new RegExp('(' + escaped.join('|') + ')', 'giu');
    return snippet.split(re).map((t, i) => ({ t, hit: i % 2 === 1 })).filter((x) => x.t !== '');
  }
  function openHit(h: SearchHit) {
    if (h.kind === 'session' && h.project) { ui.focusSeq = h.seq ? { id: h.id, seq: h.seq } : null; ui.open({ project: h.project, kind: 'session', id: h.id }); ui.view = 'thread'; }
    else { ui.focusMid = h.mid ? { chat: h.id, mid: h.mid } : null; ui.setChatEngine('simple'); ui.selectChat(h.id); ui.section = 'chat'; ui.popen = false; }
  }
  async function exportRef(r: Ref, fmt: 'md' | 'json') {
    const name = fileName(titleOf(r), fmt);
    try { await download(downloadUrl.session(r.project, r.id, fmt), name); toast(t('toast.downloaded', { name })); } catch (e) { toast(t('toast.downloadFail', { msg: msgOf(e) }), 'bad'); }
  }
  const msgOf = (e: unknown) => (e instanceof Error ? e.message : String(e));
  const shown = $derived(data.projects.filter(p => ui.filter === 'all' || (p.status === 'running' && (ui.filter === 'live' ? p.sessions.some(live) || p.ptys.some(x => x.alive) : p.sessions.some(s => s.status === 'waiting')))));
  const grouped = $derived(new Set(Object.values(ui.groups).flat()));
  const cur = $derived(data.infoOf(ui.sel));
  function items(p: { name: string; sessions: SessionInfo[]; ptys: PtyInfo[] }): Ref[] {
    // live ones first, then by last activity (conversations from disk come with their own time)
    const sessions = [...p.sessions].sort((a, b) => Number(live(b)) - Number(live(a)) || activity(b) - activity(a));
    const list: Ref[] = [...sessions.map(s => ({ project: p.name, kind: 'session' as const, id: s.id })), ...p.ptys.filter(x => !x.session).map(x => ({ project: p.name, kind: 'pty' as const, id: x.id }))];
    return q ? list.filter(r => titleOf(r).toLowerCase().includes(q)) : list;
  }
  function sub(r: Ref): string {
    const i = data.infoOf(r); if (!i) return '';
    if (r.kind === 'pty') { const p = i as PtyInfo; return `${p.clients ? tn('st.viewers', p.clients) : t('st.idle')} · ${ago(p.created_at)}`; }
    const s = i as SessionInfo; return `${statusText(s)} · ${shortModel(s.model) || agentOf(r)} · ${money(s.usage?.cost_usd)} · ${ago(activity(s))}`;
  }
  // workspace engines (agentd engines.rs): Claude Code and Codex are always there, Gemini CLI and pi — if installed; otherwise an
  // "install" item
  $effect(() => { for (const p of data.running) if (!data.engines[p.name]) data.loadEngines(p.name); });
  // the "New conversation" shortcut (§9.9): the "+" menu of the first running workspace as soon as the
  // tree is loaded
  $effect(() => {
    if (!ui.wantNew || !data.ws) return;
    const first = data.running.find(p => p.reachable) ?? data.running[0]; if (!first) return;
    ui.wantNew = false;
    tick().then(() => { const btn = document.querySelector(`.panel .ws:has(.wname) button[aria-label*="${first.name}"]`) as HTMLElement | null; if (btn) newMenu(btn, first.name); });
  });
  function newMenu(el: HTMLElement, project: string) {
    const chatLabel = (e: { id: string; title: string }) => (e.id === 'claude' ? t('menu.chatClaude') : e.id === 'codex' ? t('menu.chatCodex') : t('menu.chatWith', { name: e.title }));
    const tuiLabel = (e: { id: string; title: string }) => (e.id === 'claude' ? t('menu.tuiClaude') : e.id === 'codex' ? t('menu.tuiCodex') : t('menu.tuiWith', { name: e.title }));
    const build = () => {
      const engines = data.engines[project] ?? ENGINES.map(e => ({ ...e, installed: e.id === 'claude' || e.id === 'codex', missing: [], protocol: 'acp' as const, bin: e.id, npm: [], login_hint: '' }));
      return [
        { header: true, label: `${t('menu.newIn')} ${project}` },
        ...engines.map(e => (e.installed ? { avatar: e.icon, label: chatLabel(e), hint: e.version ?? undefined, on: () => actions.createSession(project, e.id) } : { avatar: e.icon, label: t('menu.installEngine', { name: e.title }), hint: e.npm?.join(' ') || undefined, on: () => actions.installEngine(project, e.id) })),
        { sep: true },
        { avatar: 'shell', label: t('menu.shell'), on: () => actions.createPty(project, 'shell') },
        ...engines.filter(e => e.installed).map(e => ({ avatar: e.icon, label: tuiLabel(e), on: () => actions.createPty(project, e.id) })),
        ...(data.project(project)?.apps.some(a => a.slug === 'code-server') ? [{ sep: true }, { icon: 'ext', label: t('menu.ide'), on: () => actions.ide(project) }] : []),
      ];
    };
    menu.show(el, build());
    // the catalog is re-read on every open (the agentd cache is 5 min, the install stamp invalidates it): if
    // something was installed from another tab or the phone, the menu redraws on the fly — "Install…" → "Chat with
    // …"
    const key = (list?: { id: string; installed: boolean; version?: string | null }[]) => JSON.stringify((list ?? []).map(e => [e.id, e.installed, e.version ?? '']));
    const before = key(data.engines[project]);
    void data.loadEngines(project, true).then(() => { if (menu.open && menu.anchor === el && key(data.engines[project]) !== before) menu.show(el, build()); });
  }
  function headerMenu(el: HTMLElement) {
    const run = data.running; if (!run.length) return;
    menu.show(el, run.map(p => ({ avatar: 'claude', label: `${t('menu.chatClaude')} · ${p.name}`, on: () => actions.createSession(p.name, 'claude') })));
  }
  function sessionMenu(el: HTMLElement, r: Ref) {
    const i = data.infoOf(r); const dead = !live(i);
    menu.show(el, [
      { icon: 'max', label: t('sess.menu.expand'), on: () => ui.expand(r) },
      ...(r.kind === 'session' ? [{ icon: 'edit', label: t('sess.menu.rename'), on: () => actions.rename(r) }] : []),
      { icon: 'globe', label: t('browser.menuPreview'), checked: !!data.browserOf(r.project).preview, on: () => actions.togglePreview(r.project) },
      { icon: 'monitor', label: t('abrowser.menu'), checked: !!data.browserOf(r.project).agent, on: () => actions.toggleAgentBrowser(r.project) },
      ...(r.kind === 'session' && dead ? [{ icon: 'play', label: t('sess.menu.resume'), on: () => actions.resume(r) }] : []),
      ...(r.kind === 'session' ? [{ icon: 'git', label: t('sess.menu.fork'), on: () => actions.fork(r) }] : []),
      ...(r.kind === 'session' ? [{ icon: 'download', label: t('sess.menu.exportMd'), on: () => exportRef(r, 'md') }, { icon: 'download', label: t('sess.menu.exportJson'), on: () => exportRef(r, 'json') }] : []),
      { icon: dead ? 'trash' : 'square', label: stopLabel(i, true), danger: dead, on: () => actions.stopOrRemove(r) },
      ...(!dead ? [{ icon: 'trash', label: t('sess.menu.deleteLive'), danger: true, on: () => actions.deleteSession(r) }] : []),
      ...accountMenu(r),
      ...(ui.onCanvas(r) ? [{ sep: true }, { icon: 'x', label: t('sess.menu.remove'), on: () => ui.removeCard(r) }] : []),
    ]);
  }
  function dropOnCanvas(e: DragEvent) {
    e.preventDefault(); const r = dnd.ref; dnd.ref = null; (e.currentTarget as HTMLElement).classList.remove('drop-target'); if (!r) return;
    if (!ui.onCanvas(r)) ui.addCard(r); else ui.reorder(key(r), null);
  }
</script>

<aside class="panel">
  <div class="p-head"><h2>{t('sec.work')}</h2><button class="icon sm ghost" aria-label={t('panel.new')} aria-haspopup="menu" data-tip={t('panel.new')} disabled={!data.running.length} onclick={(e) => { e.stopPropagation(); headerMenu(e.currentTarget as HTMLElement); }}><Icon name="plus" /></button></div>
  <div class="p-tools">
    <div class="search"><Icon name="search" size="s" /><input class="sm" type="search" aria-label={t('panel.search')} placeholder={t('panel.search')} bind:value={ui.q} /></div>
    <div class="filters">{#each [['all', t('panel.all')], ['live', t('panel.live')], ['wait', t('panel.wait')]] as [k, l] (k)}<button class:active={ui.filter === k} aria-pressed={ui.filter === k} onclick={() => (ui.filter = k as 'all' | 'live' | 'wait')}>{l}</button>{/each}</div>
  </div>
  <div class="p-body">
    {#if data.wsError && !data.ws}<div class="err" style="padding: 8px">{data.wsError}</div>{/if}
    {#if hits || searching || searchError}
      <div class="hits">
        <div class="hh muted xs" role="status">{#if searching}<span class="spin"></span><span>{t('cx.searching')}</span>{:else if hits}{t('search.inMessages', { n: hits.total })}{/if}</div>
        {#if searchError}<div class="search-error" role="alert"><span>{t('cx.searchFailed')} {searchError}</span><button class="sm" disabled={searching} onclick={() => searchRetry++}>{t('common.retry')}</button></div>{/if}
        {#if hits && (hits.q !== ui.q.trim() || searchError)}<p class="search-stale muted xs">{t('cx.searchPrevious', { query: hits.q })}</p>{/if}
        {#if hits && !hits.hits.length && !searching && !searchError}<div class="muted xs" style="padding: 2px 8px 6px">{t('search.none')}</div>{/if}
        {#each hits?.hits ?? [] as h (h.kind + h.id)}
          <button class="hit" data-kind={h.kind} data-id={h.id} onclick={() => openHit(h)}>
            <span class="av {h.kind === 'chat' ? 'simple' : (h.agent ?? 'claude')}"><Icon name={h.kind === 'chat' ? 'zap' : engineIcon(h.agent ?? 'claude')} size="s" /></span>
            <span class="hb">
              <span class="ht"><span class="ell">{h.title}</span><span class="muted xs"> · {h.kind === 'chat' ? t('search.simpleChat') : h.project} · {ago(Math.floor(h.ts / 1000))}</span>{#if h.count > 1}<span class="muted xs"> {t('search.more', { n: h.count - 1 })}</span>{/if}</span>
              <span class="hs xs">{#each marked(h.snippet, hits?.words ?? []) as seg, i (i)}{#if seg.hit}<mark>{seg.t}</mark>{:else}{seg.t}{/if}{/each}</span>
            </span>
          </button>
        {/each}
        {#if hits?.stopped.length}<div class="muted xs" style="padding: 2px 8px 6px">{t('search.stopped', { list: hits.stopped.join(', ') })}</div>{/if}
      </div>
    {/if}
    {#if data.ws && !data.projects.length}<div class="empty"><b>{t('panel.noProjects')}</b></div>{/if}
    {#each shown as p (p.name)}
      {@const rows = items(p)}
      {@const alive = p.sessions.filter(live).length + p.ptys.filter(x => x.alive && !x.session).length}
      {@const wait = p.sessions.some(s => s.status === 'waiting')}
      <div class="ws" class:closed={ui.closed[p.name]} class:stopped={p.status !== 'running'}>
        <div class="wrow" role="button" tabindex="0" aria-label={p.name} aria-expanded={!ui.closed[p.name]} onclick={() => (ui.closed[p.name] = !ui.closed[p.name])} onkeydown={(e) => { if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); ui.closed[p.name] = !ui.closed[p.name]; } }}>
          <Icon name="chevD" size="s" /><span class="wname" title={p.description ?? ''}>{p.name}</span>
          {#if wait}<span class="dot waiting"></span>{/if}
          {#if p.status === 'running'}
            {#if !p.reachable}<span class="badge warn" title={t('agents.noAgentd')}>agentd</span>{:else}<span class="count">{alive}</span>{/if}
            <span class="muted xs num">{money(p.cost_today)}</span>
            <button class="icon xs ghost" aria-label={`${t('panel.new')} · ${p.name}`} aria-haspopup="menu" data-tip={t('panel.new')} disabled={!p.reachable} onclick={(e) => { e.stopPropagation(); newMenu(e.currentTarget as HTMLElement, p.name); }}><Icon name="plus" size="s" /></button>
          {:else}<span class="muted xs">{p.status}</span>{/if}
        </div>
        {#if !ui.closed[p.name]}
          {#each (ui.more[p.name] || q ? rows : rows.slice(0, TREE_LIMIT)) as r (key(r))}
            <button class="sess" class:active={ui.sel && same(ui.sel, r)} class:exited={!live(data.infoOf(r))} data-id={r.id} draggable="true" ondragstart={(e) => { dnd.ref = r; e.dataTransfer?.setData('text/plain', key(r)); }} onclick={() => ui.open(r)} oncontextmenu={(e) => { e.preventDefault(); sessionMenu(e.currentTarget as HTMLElement, r); }}>
              <span class="av {agentOf(r)}"><Icon name={agentOf(r) === 'shell' ? 'terminal' : agentOf(r)} size="s" /></span>
              <span class="t">{#if r.kind === 'session'}<Mark id={r.id} size={12} /> {/if}{titleOf(r)}</span>{#if wtOf(r)}<span class="wt" title={t('git.inWorktree')}>⎇ {wtOf(r)}</span>{/if}
              <span class="st">{#if ui.onCanvas(r) && ui.view === 'code'}<span class="muted" title="Code"><Icon name="grid" size="s" /></span>{/if}<span class="dot {dotOf(r)}"></span></span>
              <span class="sub">{sub(r)}</span>
            </button>
          {/each}
          {#if rows.length > TREE_LIMIT && !q}<button class="sess-more" onclick={() => (ui.more[p.name] = !ui.more[p.name])}>{ui.more[p.name] ? t('panel.less') : t('panel.more', { n: rows.length - TREE_LIMIT })}</button>{/if}
        {/if}
      </div>
    {/each}
  </div>
  <div class="p-foot">
    {#if data.ws}<div class="row"><span>{t('panel.today')}</span><span class="sp"></span><b class="num" style="color: var(--fg-1)">{money(data.ws.today.cost_usd)}</b><span>· {tn('agents.turns', data.ws.today.turns)}</span></div>{/if}
    <div class="row"><span class="dot {data.health?.cloudd ? 'ok' : 'failed'}"></span><span>{t('panel.core')} {data.health?.cloudd ? 'ok' : t('api.offline')} · agentd {data.running.filter(p => p.reachable).length}/{data.running.length}{#if data.hubLive} · <span class="hub-live" title={t('hub.liveHint')}>{t('hub.live')}</span>{/if}</span></div>
  </div>
</aside>

<main class="main">
  <div class="top work-top">
    <button class="icon ghost burger" aria-label={t('work.openWorkspaces')} title={t('work.openWorkspaces')} aria-expanded={ui.popen} onclick={() => (ui.popen = !ui.popen)}><Icon name="menu" /></button>
    <span class="seg"><button class:active={ui.view === 'code'} aria-pressed={ui.view === 'code'} onclick={() => { ui.view = 'code'; if (ui.sel && !ui.onCanvas(ui.sel)) ui.addCard(ui.sel); }}><Icon name="cols" size="s" /> {t('top.code')}</button><button class:active={ui.view === 'thread'} aria-pressed={ui.view === 'thread'} onclick={() => (ui.view = 'thread')}><Icon name="thread" size="s" /> {t('top.thread')}</button></span>
    <span class="sp"></span>
    {#if ui.view === 'code'}
      <span class="muted sm hide-m">{t('top.cols')}</span>
      <span class="seg hide-m">{#each [0, 1, 2, 3] as n (n)}<button class:active={ui.cols === n} style="padding: 0 9px" onclick={() => (ui.cols = n as 0 | 1 | 2 | 3)}>{n || t('top.auto')}</button>{/each}</span>
    {:else if ui.sel}
      <button class="sm ghost to-canvas" aria-label={t('top.toCanvas')} onclick={() => ui.toCanvas(ui.sel!)}><Icon name="grid" size="s" /> <span class="hide-m">{t('top.toCanvas')}</span></button>
    {/if}
    <button class="icon sm ghost" class:active={ui.inspector} data-tip={t('top.inspector')} title={t('top.inspector')} aria-label={t('top.inspector')} aria-expanded={ui.inspector} onclick={() => (ui.inspector = !ui.inspector)}><Icon name="panel" /></button>
  </div>
  <div class="content work-content">
    {#if !data.ws}
      {#if data.wsError}
        <div class="empty load-state" role="alert">
          <span class="ic"><Icon name="alert" size="l" /></span>
          <b>{t('work.loadFailed')}</b><span class="load-detail">{data.wsError}</span>
          <button class="primary retry" disabled={data.wsLoading} onclick={() => data.loadWs()}><Icon name="refresh" size="s" /> {data.wsLoading ? t('common.loading') : t('common.retry')}</button>
        </div>
      {:else}
        <div class="empty load-state" role="status"><span class="spin"></span><span>{t('common.loading')}</span></div>
      {/if}
    {:else}
      {#if data.wsError}
        <div class="load-warning" class:ws-offline={!!data.wsOffline} role="status">
          <Icon name="alert" size="s" /><span>{data.wsOffline ? t('work.offlineTree', { when: new Date(data.wsOffline).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) }) : t('work.stale')}</span>
          <button class="sm retry" disabled={data.wsLoading} onclick={() => data.loadWs()}><Icon name="refresh" size="s" /> {data.wsLoading ? t('common.loading') : t('common.retry')}</button>
        </div>
      {/if}
    {#if ui.view === 'code'}
      <div class="canvas c{ui.cols}" role="list" ondragover={(e) => { if (dnd.ref) { e.preventDefault(); (e.currentTarget as HTMLElement).classList.add('drop-target'); } }} ondragleave={(e) => (e.currentTarget as HTMLElement).classList.remove('drop-target')} ondrop={dropOnCanvas}>
        {#each Object.entries(ui.groups) as [name, ids] (name)}
          {@const inG = ui.cards.filter(c => ids.includes(key(c)))}
          {#if inG.length}
            <div class="group"><div class="g-head"><Icon name="layers" size="s" /><b>{name}</b><span class="muted">· {inG.length}</span></div>{#each inG as c (key(c))}<Card card={c} />{/each}</div>
          {/if}
        {/each}
        {#each ui.cards.filter(c => !grouped.has(key(c))) as c (key(c))}<Card card={c} />{/each}
        {#if !ui.cards.length}<div class="empty" style="grid-column: 1 / -1"><span class="ic"><Icon name="grid" size="l" /></span><b>{t('canvas.empty')}</b><span>{t('canvas.emptyHint')}</span></div>{/if}
      </div>
    {:else if ui.sel && cur}
      {@const s = cur as SessionInfo}
      <div class="thread">
        <div class="t-head">
          <span class="av {agentOf(ui.sel)} lg"><Icon name={agentOf(ui.sel) === 'shell' ? 'terminal' : agentOf(ui.sel)} /></span>
          <div class="thread-heading"><h2 class="ell" title={titleOf(ui.sel)}>{#if ui.sel.kind === 'session'}<Mark id={ui.sel.id} /> {/if}{titleOf(ui.sel)}</h2><div class="muted sm ell thread-summary" title={`${ui.sel.project} · ${ui.sel.kind === 'session' ? statusText(s) : t('menu.shell')}`}>{ui.sel.project}<span class="thread-detail"> · {cur.cwd}</span> · {ui.sel.kind === 'session' ? statusText(s) : t('menu.shell')}{#if cur.created_at}<span class="thread-detail"> · {t('thread.created')} {ago(cur.created_at)}</span>{/if}</div></div>
          <span class="sp"></span>
          {#if ui.sel.kind === 'session'}<div class="meta">{#if s.model}<span class="chip mono"><Icon name="cpu" size="s" /> {s.model}</span>{/if}<span class="chip"><Icon name="shield" size="s" /> {modeLabel(s.permission_mode || (s.agent === 'codex' ? 'on-request' : 'default'))}</span><span class="chip num">{money(s.usage?.cost_usd)}</span></div>{/if}
          {#if data.project(ui.sel.project)?.apps.some(a => a.slug === 'code-server')}<button class="sm hide-m" onclick={() => actions.ide(ui.sel!.project)}>code-server <Icon name="ext" size="s" /></button>{/if}
          {#if ui.sel.kind === 'session' && !live(cur)}<button class="sm primary thread-action" onclick={() => actions.resume(ui.sel!)}>{t('thread.resume')}</button>{/if}
          <button class="sm thread-action {live(cur) ? 'danger' : 'ghost'}" onclick={() => actions.stopOrRemove(ui.sel!)}>{stopLabel(cur)}</button>
          <button class="icon sm ghost thread-menu" title={t('sec.more')} aria-label={t('sec.more')} aria-haspopup="menu" onclick={(e) => { e.stopPropagation(); sessionMenu(e.currentTarget as HTMLElement, ui.sel!); }}><Icon name="more" /></button>
        </div>
        <div class="t-body">{#key key(ui.sel)}<SessionPane project={ui.sel.project} id={ui.sel.id} kind={ui.sel.kind} cwd={cur.cwd} onchange={() => { if (!data.hubLive) data.loadWs(); }} />{/key}</div>
      </div>
    {:else}
      <div class="empty"><span class="ic"><Icon name="thread" size="l" /></span><b>{t('thread.pick')}</b><span>{t('thread.pickHint')}</span><button class="primary only-p open-workspaces" onclick={() => (ui.popen = true)}><Icon name="grid" size="s" /> {t('work.openWorkspaces')}</button><button class="sm" style="margin-top: 8px" onclick={onpalette}><Icon name="search" size="s" /> {t('top.palette').split(' · ')[0]}</button></div>
    {/if}
    {/if}
  </div>
</main>

<div class="insp-scrim" onclick={() => (ui.inspector = false)} role="presentation"></div>
<aside class="inspector"><Sidebar /></aside>

<style>
  .search-error { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 8px 10px; color: var(--bad); font-size: 12px; }
  .search-error > span { flex: 1 1 180px; overflow-wrap: anywhere; }
  .search-stale { padding: 2px 10px 6px; overflow-wrap: anywhere; margin: 0; }
  .work-content { display: flex; flex-direction: column; }
  .work-content > .canvas { flex-shrink: 0; }
  .work-content > .thread { flex: 1; height: auto; }
  .load-state { flex: 1; align-content: center; }
  .load-detail { max-width: 560px; overflow-wrap: anywhere; }
  .load-warning { flex-shrink: 0; display: flex; align-items: center; gap: 10px; margin-bottom: 12px; padding: 10px 12px; border: 1px solid var(--warn); border-radius: var(--r-2); color: var(--fg-2); background: var(--bg-2); font-size: 12px; }
  .load-warning > span { flex: 1; min-width: 0; }
  .load-warning :global(.i) { flex-shrink: 0; }
  .retry { flex-shrink: 0; }
  .thread-heading { min-width: 0; }
  @media (max-width: 599px), (pointer: coarse) and (max-height: 500px) and (max-width: 1099px) {
    .work-top { gap: 4px; }
    .work-top > .seg { min-width: 0; }
    .load-warning { flex-wrap: wrap; gap: 8px; }
    .retry, .open-workspaces { min-height: 44px; }
    .thread .t-head { gap: 8px; margin-bottom: 6px; flex-wrap: nowrap; }
    .thread .t-head > .av, .thread .t-head > .sp, .thread .t-head .meta, .thread-detail, .thread-action { display: none; }
    .thread-heading { flex: 1; display: flex; align-items: center; gap: 8px; }
    .thread .t-head h2 { flex: 1; min-width: 0; font-size: 15px; }
    .thread-summary { flex: 0 1 auto; min-width: 0; max-width: 45%; }
    .thread .t-head .thread-menu { flex-shrink: 0; width: 44px; height: 32px; min-height: 32px !important; }
  }
  /* Narrow conversation column (≤ 680px: half of the desktop, an unfolded Fold, a laptop with the inspector) — the same compact header as on
     the phone: the title + "workspace · status" + "⋯" in one row; the model/mode chips and buttons live in the composer and the menu */
  @container thread (max-width: 680px) {
    .thread .t-head { gap: 8px; margin-bottom: 6px; flex-wrap: nowrap; }
    .thread .t-head > .av, .thread .t-head > .sp, .thread .t-head .meta, .thread-detail, .thread-action { display: none; }
    .thread-heading { flex: 1; display: flex; align-items: center; gap: 8px; }
    .thread .t-head h2 { flex: 1; min-width: 0; font-size: 15px; }
    .thread-summary { flex: 0 1 auto; min-width: 0; max-width: 45%; }
    .thread .t-head .thread-menu { flex-shrink: 0; width: 44px; height: 32px; min-height: 32px !important; }
  }
  @media (max-width: 599px) {
    .work-content { padding: 4px 6px 6px; }
    .thread .t-head { gap: 6px; margin-bottom: 2px; }
  }
  @media (max-width: 380px) { .work-top > .to-canvas { display: none; } }
</style>
