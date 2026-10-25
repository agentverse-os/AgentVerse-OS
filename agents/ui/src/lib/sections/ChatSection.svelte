<script lang="ts">
  // The "Chat" section (docs/agents-ux.md §6): quick conversations in the service workspace scratch — the chat list in the
  // panel, on the right the same session card as in "Workplaces" (without Git); "move to a workspace" takes the chat into a
  // project together with its files.
  import { ui, data, actions, live, statusText, shortModel, money, ago, activity, msg, stopLabel, modeLabel, accountMenu, engineIcon, engineTitle, ENGINES } from '../state.svelte';
  import { agents, download, downloadUrl, fileName, type SessionInfo, type SimpleChat as SimpleChatT } from '../api';
  import SimpleChatView from '../SimpleChat.svelte';
  import { menu } from '../menu.svelte';
  import { toast } from '../toast.svelte';
  import { t } from '../i18n';
  import { untrack } from 'svelte';
  import Icon from '../Icon.svelte';
  import SessionPane from '../SessionPane.svelte';
  const SCRATCH = 'scratch';
  // engines of the service workspace: the installed ones (the agentd catalog), until loaded — Claude
  // Code and Codex
  const scratchEngines = () => (data.engines[SCRATCH]?.filter(e => e.installed).map(e => ({ id: e.id, title: e.title, icon: e.icon })) ?? ENGINES.filter(e => e.id === 'claude' || e.id === 'codex'));
  $effect(() => { if (ready && !data.engines[SCRATCH]) data.loadEngines(SCRATCH); });
  const chat = $derived(data.chat);
  const ready = $derived(!!chat && chat.status === 'running' && chat.reachable);
  const sessions = $derived([...(chat?.sessions ?? [])].sort((a, b) => Number(live(b)) - Number(live(a)) || activity(b) - activity(a)));
  const engine = $derived(ui.chatEngine);
  const cur = $derived(engine === 'advanced' ? sessions.find(s => s.id === ui.chatSel) ?? null : null);
  const curSimple = $derived(engine === 'simple' ? data.chats.find(c => c.id === ui.chatSel) ?? null : null);
  const listError = $derived(engine === 'simple' ? data.chatsError : data.chatError);
  const listLoaded = $derived(engine === 'simple' ? data.chatsLoaded : !!chat);
  function setEngine(e: 'simple' | 'advanced') { ui.setChatEngine(e); }
  $effect(() => { if (engine === 'simple' && !data.chatRoutes && !data.chatRoutesLoading && !data.chatRoutesError) untrack(() => data.loadChatRoutes()); });
  async function newSimple(incognito = false) {
    if (creating) return; creating = true;
    try { const c = await agents.chatNew({ incognito }); data.chats = [c, ...data.chats.filter(x => x.id !== c.id)]; data.chatsLoaded = true; await data.loadChat(); ui.setChatEngine('simple'); ui.selectChat(c.id); ui.popen = false; } catch (e) { toast(msg(e), 'bad'); } finally { creating = false; }
  }
  async function removeSimple(c: SimpleChatT) {
    if (!window.confirm(t('chat.deleteConfirm', { title: c.title || t('chat.untitled') }))) return;
    try { await agents.chatRemove(c.id); if (ui.chatSel === c.id) ui.selectChat(null); await data.loadChat(); toast(t('toast.removed'), 'info'); } catch (e) { toast(msg(e), 'bad'); }
  }
  async function keepSimple(c: SimpleChatT) { try { await agents.chatKeep(c.id); await data.loadChat(); toast(t('chat.kept')); } catch (e) { toast(msg(e), 'bad'); } }
  function handoffMenu(el: HTMLElement, c: SimpleChatT) {
    menu.show(el, [{ header: true, label: t('chat.handoffHint') }, ...scratchEngines().map(e => ({ icon: e.icon, label: e.title, on: () => handoff(c, e.id) }))]);
  }
  async function handoff(c: SimpleChatT, agent: string) {
    if (!ready) { toast(t('chat.noScratch'), 'bad'); return; }
    if (creating) return; creating = true;
    try { const r = await agents.chatHandoff(c.id, agent); if (data.chat) data.chat = { ...data.chat, sessions: [r.session, ...data.chat.sessions.filter(s => s.id !== r.session.id)] }; toast(t('chat.handoffDone')); await data.loadChat(); ui.setChatEngine('advanced'); ui.selectChat(r.session.id); } catch (e) { toast(msg(e), 'bad'); } finally { creating = false; }
  }
  function simpleMenu(el: HTMLElement, c: SimpleChatT) {
    menu.show(el, [
      { icon: 'arrowUR', label: t('chat.handoff') + '…', disabled: creating || !ready, on: () => setTimeout(() => handoffMenu(el, c), 80) },
      ...(c.incognito ? [{ icon: 'download', label: t('chat.keep'), on: () => keepSimple(c) }] : []),
      ...(c.incognito ? [] : [{ icon: 'download', label: t('sess.menu.exportMd'), on: () => exportChat(c, 'md') }, { icon: 'download', label: t('sess.menu.exportJson'), on: () => exportChat(c, 'json') }]),
      { icon: 'trash', label: t('chat.delete'), danger: true, on: () => removeSimple(c) },
    ]);
  }
  async function exportChat(c: SimpleChatT, fmt: 'md' | 'json') {
    const name = fileName(c.title || c.id, fmt);
    try { await download(downloadUrl.chat(c.id, fmt), name); toast(t('toast.downloaded', { name })); } catch (e) { toast(t('toast.downloadFail', { msg: e instanceof Error ? e.message : String(e) }), 'bad'); }
  }
  const subSimple = (c: SimpleChatT) => `${shortModel(c.model) || c.model} · ${money(c.cost_usd)} · ${ago(c.updated_at)}${c.incognito ? ' · ' + t('chat.incognito') : ''}`;
  let creating = $state(false);
  let ensuring = $state(false);
  const ref = (s: SessionInfo) => ({ project: SCRATCH, kind: 'session' as const, id: s.id });
  function stateText(): string {
    if (!chat) return t('common.loading');
    if (!chat.exists) return chat.busy ? t('chat.creating') : t('chat.noScratch');
    if (chat.status === 'running') return chat.reachable ? t('chat.ready') : t('chat.noAgentd');
    if (chat.busy || chat.status === 'starting') return chat.exists && chat.busy && chat.status !== 'starting' && chat.status !== 'stopped' ? t('chat.creating') : t('chat.starting');
    return t('chat.stopped');
  }
  async function ensure() { if (ensuring) return; ensuring = true; try { data.chat = await agents.chatEnsure(); toast(chat?.exists ? t('chat.starting') : t('chat.creating'), 'info'); } catch (e) { toast(msg(e), 'bad'); } finally { ensuring = false; } }
  async function newChat(agent: string) {
    if (creating || !ready) return; creating = true;
    try {
      const st = (await agents.settings().catch(() => null)) as any;
      const cache = agent === 'claude' ? st?.defaults?.cache_claude : undefined;
      const s = await agents.chatCreate({ agent, model: st?.defaults?.[agent] || undefined, permission_mode: st?.defaults?.[`perm_${agent}`] || undefined, cache_ttl: cache && cache !== 'auto' ? cache : undefined });
      if (data.chat) data.chat = { ...data.chat, sessions: [s, ...data.chat.sessions.filter(x => x.id !== s.id)] };
      await data.loadChat(); await data.loadWs(); ui.setChatEngine('advanced'); ui.selectChat(s.id); ui.popen = false;
    } catch (e) { toast(msg(e), 'bad'); } finally { creating = false; }
  }
  function moveMenu(el: HTMLElement) {
    const run = data.running; if (!run.length) { toast(t('chat.noWs'), 'bad'); return; }
    menu.show(el, [{ header: true, label: t('chat.moveTo') }, ...run.map(p => ({ icon: 'grid', label: p.name, on: () => move(p.name) }))]);
  }
  async function move(to: string) {
    if (!cur) return; if (!window.confirm(t('chat.moveConfirm', { title: cur.title, ws: to }))) return;
    try { const r = await agents.chatMove(cur.id, to); toast(t('chat.moved', { ws: to })); ui.selectChat(null); await Promise.all([data.loadChat(), data.loadWs()]); ui.goto({ project: to, kind: 'session', id: r.session.id }); }
    catch (e) { toast(msg(e), 'bad'); }
  }
  async function stopOrRemove(s: SessionInfo) { await actions.stopOrRemove(ref(s)); const loaded = await data.loadChat(); if (loaded && !data.chat?.sessions.some(x => x.id === s.id) && ui.chatSel === s.id) ui.selectChat(null); }
  function chatMenu(el: HTMLElement, s: SessionInfo) {
    const dead = !live(s);
    menu.show(el, [
      { icon: 'arrowUR', label: t('chat.moveTo') + '…', on: () => { ui.selectChat(s.id); setTimeout(() => moveMenu(el), 80); } },
      ...(dead ? [{ icon: 'play', label: t('sess.menu.resume'), on: () => actions.resume(ref(s)).then(() => data.loadChat()) }] : []),
      { icon: 'git', label: t('sess.menu.fork'), on: () => actions.fork(ref(s)).then(() => data.loadChat()) },
      { icon: dead ? 'trash' : 'square', label: stopLabel(s, true), danger: dead, on: () => stopOrRemove(s) },
      ...accountMenu(ref(s)),
    ]);
  }
  const sub = (s: SessionInfo) => `${statusText(s)} · ${shortModel(s.model) || s.agent} · ${money(s.usage?.cost_usd)} · ${ago(activity(s))}`;
</script>

<aside class="panel">
  <div class="p-head"><h2>{t('chat.title')}</h2>
    {#if engine === 'simple'}
      <button class="icon sm ghost" aria-label={t('chat.newSimple')} data-tip={t('chat.newSimple')} disabled={creating} onclick={() => newSimple(false)}><span class="av simple"><Icon name="zap" size="s" /></span></button>
      <button class="icon sm ghost" aria-label={t('chat.newIncognito')} data-tip={t('chat.newIncognito')} disabled={creating} onclick={() => newSimple(true)}><span class="av incog"><Icon name="eyeOff" size="s" /></span></button>
    {:else}
      {#each scratchEngines() as e (e.id)}<button class="icon sm ghost" aria-label={e.id === 'claude' ? t('chat.newClaude') : e.id === 'codex' ? t('chat.newCodex') : t('menu.chatWith', { name: e.title })} data-tip={e.id === 'claude' ? t('chat.newClaude') : e.id === 'codex' ? t('chat.newCodex') : t('menu.chatWith', { name: e.title })} disabled={!ready || creating} onclick={() => newChat(e.id)}><span class="av {e.id}"><Icon name={e.icon} size="s" /></span></button>{/each}
    {/if}
  </div>
  <div class="p-body">
    {#if data.chatLoading && !listLoaded}<p class="muted sm" role="status">{t('common.loading')}</p>{/if}
    {#if engine === 'simple'}
      {#if data.chatsLoaded && !data.chats.length}<div class="empty" style="padding: 24px 12px"><b>{t('chat.noneSimple')}</b><span>{t('chat.noneSimpleHint')}</span></div>{/if}
      {#each data.chats as c (c.id)}
        <button class="sess" class:active={ui.chatSel === c.id} data-chat={c.id} onclick={() => { ui.selectChat(c.id); ui.popen = false; }} oncontextmenu={(e) => { e.preventDefault(); simpleMenu(e.currentTarget as HTMLElement, c); }}>
          <span class="av {c.incognito ? 'incog' : 'simple'}"><Icon name={c.incognito ? 'eyeOff' : 'zap'} size="s" /></span>
          <span class="t">{c.title || t('chat.untitled')}</span>
          <span class="st"><span class="dot ok"></span></span>
          <span class="sub">{subSimple(c)}</span>
        </button>
      {/each}
    {:else}
    {#if chat && !ready}
      <div class="empty" style="padding: 24px 12px"><span class="ic"><Icon name="chat" size="l" /></span><b>{stateText()}</b>
        {#if !chat.exists && !chat.busy}<span>{t('chat.noScratchHint')}</span><button class="sm primary" style="margin-top: 8px" disabled={ensuring} onclick={ensure}>{t('chat.create')}</button>
        {:else if chat.exists && chat.status !== 'running' && !chat.busy}<button class="sm primary" style="margin-top: 8px" disabled={ensuring} onclick={ensure}>{t('chat.start')}</button>{/if}
        {#if chat.error}<span class="err">{chat.error}</span>{/if}
      </div>
    {:else if chat && !sessions.length}
      <div class="empty" style="padding: 24px 12px"><b>{t('chat.none')}</b><span>{t('chat.noneHint')}</span></div>
    {/if}
    {#each sessions as s (s.id)}
      <button class="sess" class:active={ui.chatSel === s.id} class:exited={!live(s)} data-id={s.id} onclick={() => { ui.selectChat(s.id); ui.popen = false; }} oncontextmenu={(e) => { e.preventDefault(); chatMenu(e.currentTarget as HTMLElement, s); }}>
        <span class="av {s.agent}"><Icon name={engineIcon(s.agent)} size="s" /></span>
        <span class="t">{s.title}</span>
        <span class="st"><span class="dot {s.status === 'waiting' ? 'waiting' : s.status === 'error' ? 'failed' : live(s) ? (s.status === 'working' ? 'working' : 'ok') : ''}"></span></span>
        <span class="sub">{sub(s)}</span>
      </button>
    {/each}
    {/if}
  </div>
  <div class="p-foot">
    <div class="row"><span>{engine === 'simple' ? t('chat.simple') : SCRATCH} · {t('chat.today')}</span><span class="sp"></span><b class="num" style="color: var(--fg-1)">{money(chat?.cost_today)}</b></div>
    <div class="row"><span class="dot {ready ? 'ok' : chat?.busy ? 'working' : 'failed'}"></span><span>{engine === 'simple' ? data.chatsError ? t('cx.loadFailed') : data.chatsLoaded ? t('cx.ready') : t('common.loading') : stateText()}</span></div>
  </div>
</aside>

<main class="main chat-main">
  <div class="top chat-top" class:busy={!!cur && !curSimple}>
    <button class="icon ghost burger" aria-label={t('cx.openList')} aria-expanded={ui.popen} onclick={() => (ui.popen = !ui.popen)}><Icon name="menu" /></button>
    <div class="title"><h1>{t('chat.title')}</h1><span class="muted sm hide-m">· {t('chat.hint')}</span></div>
    <span class="engine seg" title={t('chat.engineHint')}><button class:active={engine === 'simple'} aria-pressed={engine === 'simple'} data-engine="simple" onclick={() => setEngine('simple')}><Icon name="zap" size="s" /> {t('chat.simple')}</button><button class:active={engine === 'advanced'} aria-pressed={engine === 'advanced'} data-engine="advanced" onclick={() => setEngine('advanced')}><Icon name="terminal" size="s" /> {t('chat.advanced')}</button></span>
    <span class="sp"></span>
    {#if curSimple}
      {#if curSimple.incognito}<button class="sm keep header-action" aria-label={t('chat.keep')} onclick={() => keepSimple(curSimple!)}><Icon name="download" size="s" /> <span class="hide-m">{t('chat.keep')}</span></button>{/if}
      <button class="sm handoff header-action" disabled={creating || !ready} aria-label={t('chat.handoff')} aria-haspopup="menu" onclick={(e) => { e.stopPropagation(); handoffMenu(e.currentTarget as HTMLElement, curSimple!); }}><Icon name="arrowUR" size="s" /> <span class="hide-m">{t('chat.handoff')}</span></button>
      <button class="icon sm ghost header-action" aria-label={t('sec.more')} aria-haspopup="menu" onclick={(e) => { e.stopPropagation(); simpleMenu(e.currentTarget as HTMLElement, curSimple!); }}><Icon name="more" /></button>
    {:else if cur}
      <button class="sm hide-p" onclick={(e) => { e.stopPropagation(); moveMenu(e.currentTarget as HTMLElement); }}><Icon name="arrowUR" size="s" /> <span class="hide-m">{t('chat.moveTo')}</span><span class="only-m">{t('chat.move')}</span></button>
      {#if !live(cur)}<button class="sm primary hide-p" onclick={() => actions.resume(ref(cur!)).then(() => data.loadChat())}>{t('thread.resume')}</button>{/if}
      <button class="sm hide-p {live(cur) ? 'danger' : 'ghost'}" onclick={() => stopOrRemove(cur!)}>{stopLabel(cur)}</button>
      <button class="icon sm ghost header-action" aria-label={t('sec.more')} aria-haspopup="menu" onclick={(e) => { e.stopPropagation(); chatMenu(e.currentTarget as HTMLElement, cur!); }}><Icon name="more" /></button>
    {/if}
  </div>
  <div class="content chat-content" style="padding: 0">
    {#if listError}<div class="chat-load-error" role="alert"><span><b>{listLoaded ? t('cx.stale') : t('cx.loadFailed')}</b><span>{listError}</span></span><button class="sm" disabled={data.chatLoading} onclick={() => data.loadChat()}>{t('common.retry')}</button></div>{/if}
    {#if curSimple}
      <div class="thread">
        <div class="t-head">
          <span class="av {curSimple.incognito ? 'incog' : 'simple'} lg"><Icon name={curSimple.incognito ? 'eyeOff' : 'zap'} /></span>
          <div class="thread-heading"><h2 class="ell" title={curSimple.title || t('chat.untitled')}>{curSimple.title || t('chat.untitled')}</h2><div class="muted sm ell thread-summary" title={`${t('chat.simple')} · ${curSimple.model}`}>{t('chat.simple')} · {curSimple.model}{#if curSimple.created_at}<span class="thread-detail"> · {t('thread.created')} {ago(curSimple.created_at)}</span>{/if}</div></div>
          <span class="sp"></span>
          <div class="meta hide-m"><span class="chip mono"><Icon name="cpu" size="s" /> {curSimple.model}</span>{#if curSimple.incognito}<span class="chip"><Icon name="eyeOff" size="s" /> {t('chat.incognito')}</span>{/if}</div>
        </div>
        <div class="t-body">{#key curSimple.id}<SimpleChatView id={curSimple.id} onchange={() => data.loadChat()} />{/key}</div>
      </div>
    {:else if engine === 'simple'}
      <div class="empty" style="height: 100%">
        <span class="ic"><Icon name="zap" size="l" /></span>
        <b>{!listLoaded ? (listError ? t('cx.loadFailed') : t('common.loading')) : data.chats.length ? t('chat.pick') : t('chat.noneSimple')}</b><span style="max-width: 460px">{data.chats.length ? t('chat.pickHint') : t('chat.noneSimpleHint')}</span>
        <div class="row" style="gap: 8px; margin-top: 8px"><button class="sm primary" disabled={creating} onclick={() => newSimple(false)}><Icon name="zap" size="s" /> {t('chat.newSimple')}</button><button class="sm" disabled={creating} onclick={() => newSimple(true)}><Icon name="eyeOff" size="s" /> {t('chat.newIncognito')}</button></div>
      </div>
    {:else if cur}
      <div class="thread">
        <div class="t-head">
          <span class="av {cur.agent} lg"><Icon name={engineIcon(cur.agent)} /></span>
          <div class="thread-heading"><h2 class="ell" title={cur.title}>{cur.title}</h2><div class="muted sm ell thread-summary" title={`${SCRATCH} · ${statusText(cur)}`}>{SCRATCH}<span class="thread-detail"> · {cur.cwd}</span> · {statusText(cur)}{#if cur.created_at}<span class="thread-detail"> · {t('thread.created')} {ago(cur.created_at)}</span>{/if}</div></div>
          <span class="sp"></span>
          <div class="meta hide-m">{#if cur.model}<span class="chip mono"><Icon name="cpu" size="s" /> {cur.model}</span>{/if}<span class="chip"><Icon name="shield" size="s" /> {modeLabel(cur.permission_mode || (cur.agent === 'codex' ? 'on-request' : 'default'))}</span><span class="chip num">{money(cur.usage?.cost_usd)}</span></div>
        </div>
        <div class="t-body">{#key cur.id}<SessionPane project={SCRATCH} id={cur.id} kind="session" cwd={cur.cwd} hide={['git']} onchange={() => data.loadChat()} />{/key}</div>
      </div>
    {:else}
      <div class="empty" style="height: 100%">
        <span class="ic"><Icon name="chat" size="l" /></span>
        {#if !chat}<b>{listError ? t('cx.loadFailed') : t('common.loading')}</b>
        {:else if !ready}<b>{stateText()}</b><span style="max-width: 460px">{t('chat.noScratchHint')}</span>
          {#if chat && !chat.busy && (!chat.exists || chat.status !== 'running')}<button class="sm primary" style="margin-top: 8px" disabled={ensuring} onclick={ensure}>{chat.exists ? t('chat.start') : t('chat.create')}</button>{/if}
        {:else}<b>{sessions.length ? t('chat.pick') : t('chat.none')}</b><span style="max-width: 460px">{sessions.length ? t('chat.pickHint') : t('chat.noneHint')}</span>
          <div class="row" style="gap: 8px; margin-top: 8px"><button class="sm primary" disabled={!ready || creating} onclick={() => newChat('claude')}><Icon name="claude" size="s" /> {t('chat.newClaude')}</button><button class="sm" disabled={!ready || creating} onclick={() => newChat('codex')}><Icon name="codex" size="s" /> {t('chat.newCodex')}</button></div>
        {/if}
      </div>
    {/if}
  </div>
</main>

<style>
  .chat-content { display: flex; flex-direction: column; }
  .chat-content > .thread { flex: 1; min-height: 0; height: auto; }
  .chat-content > .empty { flex: 1; height: auto !important; }
  .thread-heading { min-width: 0; }
  .chat-load-error { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; flex-shrink: 0; padding: 10px 14px; border-bottom: 1px solid var(--line-2); font-size: 12px; color: var(--bad); background: var(--bg-2); }
  .chat-load-error > span { display: flex; flex-direction: column; gap: 4px; flex: 1 1 200px; overflow-wrap: anywhere; }
  @media (max-width: 599px) {
    .chat-top { gap: 4px; flex-wrap: nowrap; }
    .chat-top .engine.seg { flex: 1; min-width: 0; }
    .chat-top .engine button { flex: 1; min-width: 0; padding: 0 6px; white-space: nowrap; }
    .chat-top > .sp, .chat-top > .keep { display: none; }
    .chat-top > .header-action { flex-shrink: 0; min-width: 44px; width: 44px; min-height: 44px; padding: 0; }
    .chat-content .t-head { gap: 8px; padding: 10px 12px; }
    .chat-content .t-head > .av, .chat-content .t-head > .sp { display: none; }
    .chat-content .t-head h2 { font-size: 15px; }
    .chat-load-error button { min-height: 44px; }
  }
  @media (pointer: coarse) and (max-height: 500px) and (max-width: 1099px) {
    .chat-content .t-head { flex-wrap: nowrap; gap: 8px; padding: 6px 12px; margin-bottom: 6px; }
    .chat-content .t-head > .av, .chat-content .t-head > .sp, .chat-content .t-head .meta { display: none; }
    .chat-content .t-head h2 { font-size: 15px; }
    .chat-top .title, .chat-top > .hide-p { display: none; }
  }
  @media (max-width: 599px) {
    .chat-content .t-head { flex-wrap: nowrap; gap: 6px; padding: 4px 6px; margin-bottom: 2px; min-height: 32px; flex-shrink: 0; }
  }
  @media (max-width: 599px), (pointer: coarse) and (max-height: 500px) and (max-width: 1099px) {
    .thread-heading { display: flex; flex: 1; align-items: center; gap: 8px; }
    .thread-heading h2 { flex: 1; min-width: 0; }
    .thread-summary { flex: 0 1 auto; min-width: 0; max-width: 45%; }
    .thread-detail { display: none; }
  }
  @media (max-width: 380px) { .chat-top > .handoff { display: none; } }
</style>
