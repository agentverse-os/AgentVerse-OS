<script lang="ts">
  // Agent session card, the Chat view (ADR-006): the session journal from cloudos-agentd over WebSocket (first everything
  // after the last known seq, then live), messages, tool calls with results, permission and agent question cards answered
  // from here, a composer with photos. The process lives in the workspace — the window and the tab can be closed.
  import { onMount, tick, untrack } from 'svelte';
  import { codeDraft } from './codeDrafts.svelte';
  import { byProvider, api, agents, OfflineError, TimeoutError, type SessionInfo, type SessionEvent, type Scheduled , type GateModel, type Skill } from './api';
  import { actions, when, ui, phone, touch, same, data, money } from './state.svelte';
  import { t, tn } from './i18n';
  import Icon from './Icon.svelte';
  import { adapterFor, type Ctx } from './adapters';
  import { menu } from './menu.svelte';
  import { toast } from './toast.svelte';
  import type { Caps } from './api';
  import Md from './Md.svelte';
  import Mic from './Mic.svelte';
  import { insertAt, applyInterim, voice, sentencesFrom, parseCommand, matchProject, plainText, type VoiceCommand, type Interim } from './voice.svelte';
  import { holdWake, dropWake } from './wakelock';
  import { enqueueOutbox, listOutbox, delOutbox, flushOutbox, type OutboxItem } from './outbox';
  import { getChat, putChat, delChat, PAGE } from './chatCache';
  let { project, id, onchange }: { project: string; id: string; onchange?: () => void } = $props();
  let events = $state<SessionEvent[]>([]);
  let info = $state<SessionInfo | null>(null);
  let conn = $state<'connecting' | 'live' | 'lost'>('connecting');
  let connectedOnce = $state(false);
  const DRAFT = untrack(() => `agents.draft.${project}:${id}`);
  const draftState = codeDraft(DRAFT);
  // editing a sent message (the composer in edit mode), the queue and scheduled messages, input history, draft
  const queued = $derived(info?.queued ?? []);
  let scheduled = $state<Scheduled[]>([]);
  async function loadScheduled() { try { scheduled = (await agents.scheduled(project, id)).scheduled.filter(s => s.status === 'pending'); } catch {} }
  const hist = $derived(events.filter(e => e.kind === 'user' && !e.parent_tool_use_id && !taskNote(e)).map(e => cleanUser(userText(e))).filter(Boolean));
  let histIdx = $state<number | null>(null);
  function startEdit(e: SessionEvent) { if (draftState.sending) return; if (!draftState.editing) draftState.beforeEdit = { text: draftState.text, images: [...draftState.images] }; draftState.editing = { seq: e.seq }; draftState.text = cleanUser(userText(e)); draftState.images = []; tick().then(() => { autoGrow(); ta?.focus(); }); }
  function cancelEdit() { if (draftState.sending) return; draftState.editing = null; draftState.text = draftState.beforeEdit?.text ?? ''; draftState.images = draftState.beforeEdit?.images ?? []; draftState.beforeEdit = null; tick().then(autoGrow); }
  function forkAt(e: SessionEvent) { actions.fork({ project, kind: 'session', id }, e.seq); }
  /** Answer read-aloud (§4.7, slice 64): without a provider key — a hint to the
   *  settings */
  async function speakMsg(key: string, text: string) {
    if (!voice.settings) { try { await voice.load(); } catch {} }
    if (!voice.ready) { toast(t('voice.speakNotReady'), 'bad'); ui.section = 'settings'; ui.settingsTab = 'voice'; return; }
    try { await voice.speak(key, text); } catch (e) { toast(msg(e), 'bad'); }
  }
  async function unqueue(qid: string) { try { await api.sessionUnqueue(project, id, qid); await refresh(); return true; } catch (e) { draftState.error = msg(e); return false; } }
  const qtext = (q: { content: any }) => (Array.isArray(q.content) ? q.content.filter((b: any) => b.type === 'text').map((b: any) => b.text).join(' ') : String(q.content ?? ''));
  async function editQueued(q: { id: string; content: any }) {
    if (draftState.sending || draftState.editing || draftState.text.trim() || draftState.images.length) { draftState.error = t('cmp.finishDraft'); return; }
    if (!await unqueue(q.id)) return;
    draftState.text = qtext(q);
    draftState.images = (Array.isArray(q.content) ? q.content : []).filter((b: any) => b.type === 'image' && b.source?.type === 'base64').map((b: any, i: number) => ({ name: `image-${i + 1}`, type: b.source.media_type, data: b.source.data, url: `data:${b.source.media_type};base64,${b.source.data}` }));
    tick().then(() => { autoGrow(); ta?.focus(); });
  }
  /** Extra settings are available at any width; the main ones stay next to the field on a wide screen. */
  function moreMenu(el: HTMLElement) {
    menu.show(el, [
      { icon: 'shield', label: t('cmp.modeHint'), hint: adapter.modeLabel(modeValue, caps), on: () => { effortOpen = false; skillsOpen = false; modesOpen = true; } },
      ...(caps?.features?.effort ?? true ? [{ icon: 'zap', label: t('cmp.reasoning'), hint: adapter.effortLabel(info?.effort), on: () => { skillsOpen = false; if (adapter.id === 'codex') { modesOpen = false; effortOpen = true; } else { effortOpen = false; modesOpen = true; } } }] : []),
      { sep: true },
      { icon: 'sparkles', label: t('cmp.skills') + (skillSel.length ? ` · ${skillSel.length}` : ''), on: () => skillsOpenToggle() },
      { icon: 'volume', label: t('voice.autoRead'), checked: voice.auto, on: () => voice.setAuto(!voice.auto) },
      ...(caps?.mcp !== undefined && caps?.mcp !== null ? [{ icon: 'cpu', label: `MCP · ${caps.mcp}`, on: () => ui.openMcp(project) }] : []),
      ...(ctx ? [{ icon: 'cpu', label: t('cmp.contextDetails', { used: fmtTokens(ctx.used), window: fmtTokens(ctx.window), pct: ctx.pct }), on: () => setTimeout(() => ctxMenu(el), 80) }] : []),
      ...(cache ? [{ icon: 'refresh', label: t('cmp.cacheShort', { read: fmtTokens(cache.read), write: fmtTokens(cache.write) }), hint: cache.ttl ? `${cache.oneH ? t('cmp.cache1h') : t('cmp.cache5m')} · ${cacheLeft > 0 ? mmss(cacheLeft) : t('cmp.cacheExpired')}` : '', on: () => {} }] : []),
      ...(draftState.text.trim() && !draftState.editing ? [{ sep: true }, { icon: 'calendar', label: t('sched.btn'), on: () => setTimeout(() => schedMenu(el), 80) }] : []),
    ]);
  }
  function schedMenu(el: HTMLElement) {
    const body = draftState.text.trim(); if (!body) return;
    const at = (secs: number) => Math.floor(Date.now() / 1000) + secs;
    const tomorrow = () => { const d = new Date(); d.setDate(d.getDate() + 1); d.setHours(9, 0, 0, 0); return Math.floor(d.getTime() / 1000); };
    menu.show(el, [
      { header: true, label: t('sched.btn') },
      { icon: 'calendar', label: t('sched.in15'), on: () => schedule(at(15 * 60)) }, { icon: 'calendar', label: t('sched.in1h'), on: () => schedule(at(3600)) }, { icon: 'calendar', label: t('sched.in8h'), on: () => schedule(at(8 * 3600)) },
      { icon: 'calendar', label: t('sched.tomorrow'), on: () => schedule(tomorrow()) },
      { icon: 'edit', label: t('sched.custom'), on: () => { const v = window.prompt(t('sched.prompt')); if (!v) return; const ts = Date.parse(v.trim().replace(' ', 'T')); if (!ts) { draftState.error = t('sched.bad'); return; } schedule(Math.floor(ts / 1000)); } },
    ]);
  }
  async function schedule(atTs: number) {
    const body = draftState.text.trim(); if (!body) return;
    try { const sm = await agents.scheduleCreate({ project, session: id, content: body, at: atTs }); draftState.text = ''; draftState.editing = null; tick().then(autoGrow); await loadScheduled(); toast(t('sched.created', { when: when(sm.at) })); } catch (e) { draftState.error = msg(e); }
  }
  async function unschedule(sid: string) { try { await agents.scheduleDelete(sid); await loadScheduled(); } catch (e) { draftState.error = msg(e); } }
  let log: HTMLDivElement;
  let lastSeq = 0;
  // provider adapter (Claude Code / Codex): models, effort, modes, commands and tool labels differ
  let caps = $state<Caps | null>(null);
  const adapter = $derived(adapterFor(info?.agent));
  let ta = $state<HTMLTextAreaElement | undefined>(undefined);
  let attachmentInput = $state<HTMLInputElement | null>(null);
  let cameraInput = $state<HTMLInputElement | null>(null);
  let popup = $state<{ kind: 'cmd' | 'file'; items: { label: string; hint?: string; desc?: string; insert: string }[]; sel: number; start: number; end: number; loading?: boolean } | null>(null);
  let ctxProvider = $state<Ctx | null>(null);
  let now = $state(Date.now());
  let ctxTimer: ReturnType<typeof setTimeout> | undefined;
  // journal: on open the end is shown right away (a replay without intermediate scrolls), we "stick" to the bottom
  // only while the reader is at the bottom; while reading history new events are counted and offered with a "down"
  // button
  let stick = $state(true);
  let positioningLog = false;
  let unread = $state(0);
  /** Live answer text (slice 39): `delta` chunks over WebSocket without the journal; cleared by a finished
   *  message */
  let live = $state('');
  let replayLeft = 0;
  let replayDone = false;
  const WINDOW = 150;
  /** How many journal events are loaded at a time (the tail); older ones — by the "show more" button in
   *  pages */
  // feed on the device (§4.3.16): pages of PAGE events, a cache in IndexedDB, a lightweight
  // replay
  const HISTORY = PAGE;
  let olderCount = $state(0);
  let importV: number | null = null;
  let fromCache = $state(false); let cachedAt = $state(0); let verified = $state(false); let staleShown = $state(false); let offline = $state(false);
  let saveT: ReturnType<typeof setTimeout> | undefined;
  function scheduleSave(ms = 1500) { clearTimeout(saveT); saveT = setTimeout(saveCache, ms); }
  // $state.snapshot: rune proxies cannot be structured-cloned (DataCloneError in IndexedDB)
  function saveCache() { clearTimeout(saveT); if (events.length) void putChat(project, id, importV, $state.snapshot(events) as SessionEvent[]); }
  /** "Show whole": the full event from the server replaces the
   *  lightweight one */
  let expanded = $state<Set<number>>(new Set());
  async function expandEvent(seq: number) {
    try { const full = await api.sessionEvent(project, id, seq); events = events.map(e => (e.seq === seq ? full : e)); expanded = new Set([...expanded, seq]); scheduleSave(); } catch (e) { toast(msg(e), 'bad'); }
  }
  let showAll = $state(false);
  async function loadOlder() {
    const from = Math.max(0, olderCount - HISTORY); const before = log?.scrollHeight ?? 0;
    try { const older = await api.sessionEvents(project, id, from, olderCount, true); if (older.length) { events = [...older.filter(o => o.seq <= olderCount), ...events]; olderCount = from; showAll = true; await tick(); if (log) log.scrollTop += log.scrollHeight - before; } else olderCount = 0; } catch (e) { draftState.error = msg(e); }
  }
  // subagent messages (parent_tool_use_id) fold under the Agent call card and do not go into the main feed
  const top = $derived(events.filter(e => !e.parent_tool_use_id));
  const subs = $derived.by(() => { const m: Record<string, SessionEvent[]> = {}; for (const e of events) if (e.parent_tool_use_id && (e.kind === 'assistant' || e.kind === 'tool_result')) (m[e.parent_tool_use_id] ??= []).push(e); return m; });
  const visible = $derived(showAll || top.length <= WINDOW ? top : top.slice(top.length - WINDOW));
  const hiddenCount = $derived(top.length - visible.length);
  // jump to a message from search (slice 34): load the history up to seq, show the whole window, scroll and
  // highlight
  async function focusSeq(seq: number) {
    for (let i = 0; i < 60; i++) { const first = events[0]?.seq ?? 0; if (first <= seq || olderCount === 0) break; await loadOlder(); }
    showAll = true; await tick(); await new Promise((r) => setTimeout(r, 80));
    const el = log?.querySelector(`[data-seq="${seq}"]`) as HTMLElement | null;
    if (el) { stick = false; el.scrollIntoView({ block: 'center' }); el.classList.add('hit'); setTimeout(() => el.classList.remove('hit'), 3000); }
  }
  $effect(() => { const f = ui.focusSeq; if (!f || f.id !== id) return; const seq = f.seq; const h = setTimeout(() => { ui.focusSeq = null; void focusSeq(seq); }, 300); return () => clearTimeout(h); });
  // background tasks after a turn: by the task_started … task_notification / task_updated events with a final
  // status
  const runningTasks = $derived.by(() => { const m = new Map<string, SessionEvent>(); for (const e of events) { if (e.kind !== 'task') continue; if (e.subtype === 'task_started') m.set(e.task_id, e); else if (e.subtype === 'task_notification' || ['completed', 'failed', 'killed', 'stopped', 'cancelled', 'error'].includes(e.status)) m.delete(e.task_id); } return info?.alive ? [...m.values()] : []; });
  async function stopTask(taskId: string) { try { await api.sessionTaskStop(project, id, taskId); } catch (e) { draftState.error = msg(e); } }
  const taskIcon = (tt?: string | null) => (tt === 'local_agent' || tt === 'agent' ? 'claude' : tt === 'workflow' ? 'layers' : tt === 'monitor' ? 'eye' : 'terminal');
  const elapsed = (ts?: number) => { const s = Math.max(0, Math.floor((now - (ts ?? now)) / 1000)); return s < 60 ? `${s} s` : `${Math.floor(s / 60)} ${t('rt.min')}`; };
  /** Service messages the CLI slips into the conversation as user messages: <task-notification> after a background
   * task (result, tool calls, duration) — its own line instead of raw XML; <system-reminder> is cut out. */
  const userText = (e: SessionEvent) => (e.content ?? []).filter((b: any) => b.type === 'text').map((b: any) => String(b.text ?? '')).join('\n');
  const cleanUser = (s: string) => s.replace(/<system-reminder>[\s\S]*?<\/system-reminder>/g, '').trim();
  function taskNote(e: SessionEvent): { status: string; summary: string; result: string; meta: string; id: string } | null {
    const text = userText(e); if (!/^\s*<task-notification>/.test(text)) return null;
    const tag = (n: string) => text.match(new RegExp(`<${n}>([\\s\\S]*?)<\\/${n}>`))?.[1]?.trim() ?? '';
    const tools = tag('tool_uses'), ms = Number(tag('duration_ms') || 0);
    const meta = [tools ? `${tools} tools` : '', ms ? (ms >= 60000 ? `${Math.round(ms / 60000)} ${t('rt.min')}` : `${Math.round(ms / 1000)} s`) : ''].filter(Boolean).join(' · ');
    return { status: tag('status'), summary: tag('summary'), result: tag('result') || tag('output') || '', meta, id: tag('task-id') || tag('task_id') };
  }
  /** Tasks that already have a <task-notification> note in the feed: their system lines "task stopped/finished" are not duplicated */
  const notedTasks = $derived.by(() => { const s = new Set<string>(); for (const e of events) { if (e.kind !== 'user') continue; const n = taskNote(e); if (n?.id) s.add(n.id); } return s; });
  /** Claude Code rule suggestion for a permission request: Bash(git diff:*), Read… */
  const ruleOf = (e: SessionEvent): string => ((e.suggestions ?? []) as any[]).flatMap(s => (s?.rules ?? []) as any[]).map(r => (r.ruleContent ? `${r.toolName}(${r.ruleContent})` : r.toolName)).filter(Boolean).join(', ');
  const scopeLabel = (s?: string | null) => (s === 'session' ? t('perm.scopeSession') : s === 'user' ? t('perm.scopeUser') : t('perm.scopeProject'));
  function rememberMenu(el: HTMLElement, e: SessionEvent) {
    const rule = ruleOf(e);
    menu.show(el, [
      { header: true, label: rule ? `${t('perm.rule')}: ${rule}` : t('perm.remember') },
      { icon: 'zap', label: t('perm.scopeSession'), on: () => answer(e, 'allow', { remember: true, scope: 'session' }) },
      { icon: 'folder', label: t('perm.scopeProject'), on: () => answer(e, 'allow', { remember: true, scope: 'project' }) },
      { icon: 'key', label: t('perm.scopeUser'), on: () => answer(e, 'allow', { remember: true, scope: 'user' }) },
    ]);
  }
  function onLogScroll() { if (!log || positioningLog) return; const atBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 48; if (atBottom !== stick) stick = atBottom; if (atBottom) unread = 0; }
  async function showEarlier() { const before = log?.scrollHeight ?? 0; showAll = true; await tick(); if (log) log.scrollTop += log.scrollHeight - before; }
  function jumpDown() { stick = true; unread = 0; scroll(); }
  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));

  onMount(() => {
    let ws: WebSocket | null = null; let closed = false; let retry = 0; let reconnectT: ReturnType<typeof setTimeout> | null = null; let lastMsgAt = 0;
    // §9.10: the network came back or the tab is visible again — reconnect right away, not after
    // the 1…15 s pause
    const kick = () => {
      if (closed) return;
      if (conn === 'live') {
        // the socket counts as alive — check with a ping: no frame within 5 s (half-open after a network change) — close, onclose reconnects
        // without a pause
        const sock = ws; if (!sock || sock.readyState !== WebSocket.OPEN) return;
        const before = lastMsgAt; try { sock.send('{"kind":"ping"}'); } catch {}
        setTimeout(() => { if (ws === sock && lastMsgAt === before) { retry = 0; try { sock.close(); } catch {} } }, 5000);
        return;
      }
      if (reconnectT) { clearTimeout(reconnectT); reconnectT = null; }
      retry = 0;
      if (ws && ws.readyState === WebSocket.CONNECTING) return;
      if (ws) { const old = ws; ws = null; old.onclose = null; old.onmessage = null; old.onerror = null; try { old.close(); } catch {} }   // the old socket must not schedule its own reconnect
      connect();
    };
    const onOnline = () => kick(); const onVisible = () => { if (document.visibilityState === 'visible') kick(); };
    window.addEventListener('online', onOnline); document.addEventListener('visibilitychange', onVisible);
    loadScheduled(); const schedT = setInterval(loadScheduled, 30000);
    // history — with one HTTP request (thousands of events in one render, no frame-by-frame "fast-forward"), WebSocket —
    // only live
    (async () => {
      // 1. the device cache: instant rendering unless the hub snapshot says the journal was rebuilt or is
      // shorter
      try {
        const cached = await getChat(project, id);
        const snap = data.infoOf({ project, kind: 'session', id }) as SessionInfo | undefined;
        if (cached && !closed && (!snap || ((snap.import_v ?? null) === cached.import_v && (snap.last_seq ?? 0) >= cached.last_seq))) {
          events = cached.events; lastSeq = cached.last_seq; olderCount = Math.max(0, cached.first_seq - 1); importV = cached.import_v; fromCache = true;
          cachedAt = cached.events[cached.events.length - 1]?.ts ?? cached.saved_at; replayDone = true; await tick(); scroll();
          setTimeout(() => { if (!verified) staleShown = true; }, 1000);   // the "as of …" mark — only if the reconciliation drags on
        }
      } catch {}
      // 2. reconciliation with the server: import_v and last_seq; without a cache — the
      // last page (lite)
      try {
        const r = await api.sessionGet(project, id); info = r.info; if (r.caps) caps = r.caps;
        const srvLast = r.info.last_seq ?? 0; const srvImport = r.info.import_v ?? null;
        if (fromCache && (srvImport !== importV || srvLast < lastSeq)) { fromCache = false; void delChat(project, id); events = []; lastSeq = 0; olderCount = 0; replayDone = false; }
        if (!fromCache) {
          const from = Math.max(0, srvLast - HISTORY);
          const list = await api.sessionEvents(project, id, from, undefined, true);
          if (!closed && list.length) { events = list; lastSeq = list[list.length - 1].seq; olderCount = from; replayDone = true; await tick(); scroll(); }
        }
        importV = srvImport; verified = true; staleShown = false; offline = false; scheduleSave();
      } catch (e) {
        if (String(msg(e)).includes('404')) { void delChat(project, id); }
        else if (fromCache) { offline = true; staleShown = true; }
      }
      connect();
    })();
    function connect() {
      if (closed) return;
      conn = 'connecting';
      const sock = new WebSocket(api.sessionWsUrl(project, id, lastSeq, true)); ws = sock;
      sock.onopen = () => { if (ws !== sock) return; retry = 0; connectedOnce = true; conn = 'live'; lastMsgAt = Date.now(); };
      sock.onmessage = (ev) => {
        if (ws !== sock) return;
        lastMsgAt = Date.now();
        let m: any; try { m = JSON.parse(ev.data); } catch { return; }
        if (m.kind === 'delta') { if (m.reset) live = ''; if (m.text) live += m.text; if (stick) scroll(); return; }
        if (m.kind === 'hello') { info = m.info; replayLeft = m.replay ?? 0; verified = true; staleShown = false; offline = false; if (!replayLeft) { replayDone = true; scroll(); } refresh(); scheduleCtx(); return; }
        if (m.kind === 'pong') return;
        if (m.kind === 'lagged') { api.sessionEvents(project, id, lastSeq, undefined, true).then(add).catch(() => {}); return; }
        add([m]);
      };
      sock.onclose = () => { if (closed || ws !== sock) return; conn = 'lost'; if (fromCache && !verified) { offline = true; staleShown = true; } if (reconnectT) clearTimeout(reconnectT); reconnectT = setTimeout(() => { reconnectT = null; connect(); }, Math.min(1000 * 2 ** retry++, 15000)); };
      sock.onerror = () => { try { sock.close(); } catch {} };
    }
    function add(list: SessionEvent[]) {
      let changed = false, fresh = 0;
      // after an edit the journal numbering rolls back: the history_truncated mark is accepted with any seq and the feed
      // is cut by it
      for (const e of list) { if (e.seq > lastSeq || e.kind === 'history_truncated') { if (e.kind === 'history_truncated') { events = events.filter(x => x.seq <= e.at); } events.push(e); lastSeq = e.seq; if (e.kind === 'assistant' && voice.auto && !e.local) { const ft = (e.content ?? []).filter((b: any) => b.type === 'text').map((b: any) => b.text ?? '').join('\n'); if (ft.trim()) speakLive(ft, turnGen, true); } if (e.kind === 'assistant' || e.kind === 'result' || e.kind === 'exit' || e.kind === 'user') live = ''; if (e.kind === 'queued' || e.kind === 'unqueued') refreshSoon(); if (e.kind === 'user' && !e.parent_tool_use_id && scheduled.length) loadScheduled(); changed = true; if (replayLeft > 0) replayLeft--; else if (e.kind === 'task' || e.kind === 'user' || e.kind === 'assistant' || e.kind === 'history_truncated' || e.kind === 'permission_request') fresh++; } }
      if (!changed) return;
      scheduleSave();
      // history replay: no state updates per event (otherwise a request storm), one scroll and one update when
      // it ends; live events — a scroll only if the reader is at the bottom
      if (replayLeft > 0) return;
      if (replayLeft === 0 && !replayDone) { replayDone = true; refresh(); scheduleCtx(); scroll(); return; }
      if (list.some(e => e.kind === 'status' || e.kind === 'result' || e.kind === 'exit' || e.kind === 'init' || e.kind === 'initialized' || e.kind === 'model' || e.kind === 'effort' || e.kind === 'permission_mode')) refresh();
      if (list.some(e => e.kind === 'result' || e.kind === 'initialized')) scheduleCtx();
      if (stick) scroll(); else unread += fresh;
    }
    // ping every 25 s; silence watchdog: a missed pong (40 s without frames by the next tick) means a half-open socket, close it to reconnect
    const ping = setInterval(() => { if (ws?.readyState !== WebSocket.OPEN) return; if (Date.now() - lastMsgAt > 40000) { retry = 0; try { ws.close(); } catch {} return; } ws.send('{"kind":"ping"}'); }, 25000);
    const tick10 = setInterval(() => (now = Date.now()), 10000);
    const onHide = () => { if (document.visibilityState === 'hidden') saveCache(); };
    document.addEventListener('visibilitychange', onHide);
    return () => { closed = true; clearInterval(schedT); clearInterval(ping); clearInterval(tick10); clearTimeout(ctxTimer); if (reconnectT) clearTimeout(reconnectT); window.removeEventListener('online', onOnline); document.removeEventListener('visibilitychange', onVisible); document.removeEventListener('visibilitychange', onHide); saveCache(); void dropWake(`turn:${id}`); ws?.close(); };
  });
  let changeT: ReturnType<typeof setTimeout> | undefined;
  let refreshT: ReturnType<typeof setTimeout> | undefined;
  function refreshSoon() { clearTimeout(refreshT); refreshT = setTimeout(() => { refresh(); loadScheduled(); }, 300); }
  async function refresh() {
    try {
      const r = await api.sessionGet(project, id); info = r.info; if (r.caps) caps = r.caps; clearTimeout(changeT); changeT = setTimeout(() => onchange?.(), 700);
      // the journal got shorter (an edit from another tab or before connecting) — re-read the whole
      // feed
      if ((r.info.last_seq ?? 0) < lastSeq) { void delChat(project, id); const from = Math.max(0, (r.info.last_seq ?? 0) - PAGE); const list = await api.sessionEvents(project, id, from, undefined, true); events = list; lastSeq = list.length ? list[list.length - 1].seq : 0; olderCount = from; importV = r.info.import_v ?? null; await tick(); scroll(); scheduleSave(); }
    } catch {}
  }
  /** Exact context for Claude — control `get_context_usage` (summary: no API calls); called after a turn, at most once per 1.5 s. */
  function scheduleCtx() { clearTimeout(ctxTimer); ctxTimer = setTimeout(fetchCtx, 1500); }
  // a long turn with dozens of tool calls: the context chip also refreshes as it goes, every 20
  // s
  $effect(() => { if (!working) return; const i = setInterval(fetchCtx, 20000); return () => clearInterval(i); });
  async function fetchCtx() {
    if (adapter.id !== 'claude' || !info?.alive) return;
    try {
      const r = await api.sessionControl(project, id, { subtype: 'get_context_usage', detail: 'summary' }); const c = r?.response ?? r;
      if (c?.maxTokens) ctxProvider = { used: c.totalTokens ?? 0, window: c.maxTokens, pct: c.percentage ?? Math.round(((c.totalTokens ?? 0) / c.maxTokens) * 100), source: 'provider', threshold: c.autoCompactThreshold ?? undefined, categories: (c.categories ?? []).filter((x: any) => x.kind === 'used').map((x: any) => ({ name: x.name, tokens: x.tokens })) } as Ctx;
    } catch {}
  }
  async function scroll() { await tick(); if (log && stick && !positioningLog) log.scrollTop = log.scrollHeight; }

  const status = $derived(info?.status ?? 'connecting');
  const working = $derived(status === 'working' || status === 'starting');
  // §9.9: the screen does not go dark while a turn is
  // running in the open conversation
  $effect(() => { if (working) holdWake(`turn:${id}`); else void dropWake(`turn:${id}`); });
  // "Share" an answer — only where Web Share exists (phones, Safari)
  const canShare = typeof navigator !== 'undefined' && typeof navigator.share === 'function';
  async function shareText(md: string) { try { await navigator.share({ text: plainText(md) }); } catch (e) { if (!/Abort/i.test(String(e))) toast(msg(e), 'bad'); } }
  // share_target: what was shared from the phone — into this conversation's draft as soon as it is
  // open in Thread
  $effect(() => {
    const sh = data.pendingShare; if (!sh) return;
    untrack(() => {
      data.pendingShare = null;
      if (sh.text) insertText(sh.text);
      for (const f of sh.files) {
        if (f.type.startsWith('image/')) { draftState.images.push({ name: f.name, type: f.type, data: f.data, url: `data:${f.type};base64,${f.data}` }); }
        else { try { const txt = atob(f.data); if (txt.trim()) insertText(`\n${f.name}:\n${txt.slice(0, 20000)}`); } catch {} }
      }
      toast(t('share.added'), 'info');
    });
  });
  // outbox without a network (§9.9): this conversation's entries, sent when
  // the connection returns
  let outbox = $state<OutboxItem[]>([]);
  async function loadOutbox() { outbox = (await listOutbox()).filter(o => o.project === project && o.session === id); }
  async function dropOutbox(oid: string) { await delOutbox(oid); await loadOutbox(); }
  $effect(() => { if (conn === 'live') untrack(() => { loadOutbox().then(() => { if (outbox.some(o => !o.error)) flushOutbox(() => toast(t('outbox.sent', { n: 1 }), 'info')).then(() => loadOutbox()); }); }); });
  $effect(() => { const h = () => { void loadOutbox(); }; window.addEventListener('agents:outbox', h); return () => window.removeEventListener('agents:outbox', h); });
  const pendingIds = $derived(new Set([...events.filter(e => e.kind === 'permission_result').map(e => e.request_id), ...Object.keys(draftState.permissionSent).filter(id => draftState.permissionSent[id])]));
  const pendingRequest = $derived(status === 'waiting' ? [...events].reverse().find(e => e.kind === 'permission_request' && !pendingIds.has(e.request_id)) : undefined);
  const queueing = $derived(!draftState.editing && (working || status === 'waiting'));
  const resuming = $derived(!draftState.editing && !queueing && (status === 'exited' || status === 'error' || info?.alive === false));
  const sendLabel = $derived(draftState.sending ? t('cmp.sending') : draftState.editing ? t('cmp.saveEdit') : queueing ? t('cmp.queueAction') : resuming ? t('cmp.resumeSend') : t('agents.send'));
  const connectionLabel = $derived(conn !== 'live' ? (connectedOnce || conn === 'lost' ? t('agents.reconnecting') : t('agents.connecting')) : draftState.sending ? t('cmp.sending') : working ? t('cmp.typing') : status === 'waiting' ? t('agents.waiting') : status === 'exited' ? t('st.exited') : status === 'error' ? t('st.failed') : t('agents.idle'));
  async function jumpDecision() {
    if (!pendingRequest) { ui.section = 'inbox'; return; }
    positioningLog = true; stick = false; showAll = true;
    try {
      await tick(); await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
      const el = log?.querySelector(`[data-request="${CSS.escape(pendingRequest.request_id)}"]`) as HTMLElement | null;
      if (el && log) {
        log.scrollTop += el.getBoundingClientRect().top - log.getBoundingClientRect().top - Math.max(0, (log.clientHeight - el.clientHeight) / 2);
        (el.querySelector('button:not(:disabled), input') as HTMLElement | null)?.focus({ preventScroll: true });
      }
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    } finally { positioningLog = false; stick = false; }
  }
  async function send() {
    const draft = draftState.text, attachments = [...draftState.images], edit = draftState.editing, enqueue = queueing;
    const body = draft.trim(); if (draftState.sending || draftState.readingImages || (!body && !attachments.length)) return;
    // no connection — into the outbox (§9.9): goes out by itself when the network returns; a history edit and the
    // queue wait for a live socket. The socket may still count as alive in the first seconds after the network
    // vanishes — look at navigator.onLine too
    const offline = conn !== 'live' || (typeof navigator !== 'undefined' && navigator.onLine === false);
    const toOutbox = async () => {
      const content: unknown[] = attachments.map(im => ({ type: 'image', source: { type: 'base64', media_type: im.type, data: im.data } }));
      if (body) content.push({ type: 'text', text: body });
      await enqueueOutbox(project, id, content, body || `[${attachments.length} img]`);
      if (draftState.text === draft) draftState.text = ''; draftState.images = draftState.images.filter(im => !attachments.includes(im));
      await loadOutbox(); toast(t('outbox.queued'), 'info'); tick().then(autoGrow);
    };
    if (offline) { if (edit || enqueue) return; await toOutbox(); return; }
    draftState.sending = true; popup = null; draftState.error = '';
    try {
      const m = !attachments.length ? body.match(/^\/([\w-]+)\s*([\s\S]*)$/) : null;
      if (m && await runCommand(m[1], m[2].trim())) { if (!draftState.error && draftState.text === draft) draftState.text = ''; tick().then(autoGrow); return; }
      const content: unknown[] = attachments.map(im => ({ type: 'image', source: { type: 'base64', media_type: im.type, data: im.data } }));
      const pendingSkills = edit ? [] : skillSel.filter(n => !skillSent.includes(n));
      if (pendingSkills.length && !skillsAll.length) skillsAll = (await agents.skills()).skills;
      const block = pendingSkills.length ? skillsBlock(pendingSkills) : '';
      const bodyText = block && body ? `${block}\n---\n${body}` : body;
      if (bodyText) content.push({ type: 'text', text: bodyText });
      if (edit) await api.sessionEdit(project, id, { seq: edit.seq, content });
      // The queue accepts both text and images; permissions are not bypassed by a new
      // message.
      else if (enqueue) await api.sessionQueue(project, id, content);
      else {
        if (status === 'exited' || status === 'error' || info?.alive === false) {
        await api.sessionCreate(project, { resume: id });
        for (let i = 0; i < 20; i++) { await refresh(); if (info?.alive) break; await new Promise(r => setTimeout(r, 500)); }
        }
        await api.sessionPrompt(project, id, content);
      }
      // Clear only the confirmed send, not new text typed during the request.
      if (draftState.text === draft) draftState.text = '';
      draftState.images = draftState.images.filter(im => !attachments.includes(im));
      if (edit && draftState.editing === edit) { draftState.editing = null; if (!draftState.text) draftState.text = draftState.beforeEdit?.text ?? ''; draftState.images = [...(draftState.beforeEdit?.images ?? []), ...draftState.images]; draftState.beforeEdit = null; }
      if (block) { skillSent = [...new Set([...skillSent, ...pendingSkills])]; skillsPersist(); }
      tick().then(autoGrow); await refresh();
    } catch (e) {
      // the network vanished mid-send — not an error but the outbox (if it is a regular
      // message)
      if (e instanceof OfflineError && !edit && !enqueue) { draftState.sending = false; await toOutbox(); return; }
      // the request hung: the core may have accepted it — do not resend, ask to look at the feed instead (the draft
      // stays)
      if (e instanceof TimeoutError) { draftState.error = t('cmp.sendUnconfirmed'); return; }
      draftState.error = `${t('cmp.sendFailed')} ${msg(e)}`;
    }
    finally { draftState.sending = false; }
  }
  /** Images as attachments: from the 📎 button, from the clipboard (Ctrl+V / ⌘V) and by drag-and-drop; non-images are
   *  skipped with a hint */
  function addImages(list: Iterable<File>) {
    let skipped = 0;
    for (const f of list) {
      if (!f.type.startsWith('image/')) { skipped++; continue; }
      const r = new FileReader();
      draftState.readingImages++;
      r.onload = () => { const url = String(r.result); draftState.images.push({ name: f.name || `pasted-${Date.now()}.png`, type: f.type || 'image/png', data: url.split(',')[1] ?? '', url }); };
      r.onerror = () => { draftState.error = t('cmp.attachmentFailed', { name: f.name }); };
      r.onloadend = () => { draftState.readingImages--; };
      try { r.readAsDataURL(f); } catch { draftState.readingImages--; draftState.error = t('cmp.attachmentFailed', { name: f.name }); }
    }
    if (skipped) toast(t('cmp.imagesOnly'), 'info');
  }
  function attach(e: Event) { addImages(Array.from((e.target as HTMLInputElement).files ?? [])); (e.target as HTMLInputElement).value = ''; }
  function onPaste(e: ClipboardEvent) {
    const files = Array.from(e.clipboardData?.items ?? []).filter(i => i.kind === 'file').map(i => i.getAsFile()).filter((f): f is File => !!f);
    if (!files.length) return;
    e.preventDefault(); addImages(files);
  }
  function onDrop(e: DragEvent) { e.preventDefault(); dragging = false; if (e.dataTransfer?.files?.length) addImages(Array.from(e.dataTransfer.files)); }
  let dragging = $state(false);
  async function answer(req: SessionEvent, behavior: 'allow' | 'deny', extra: Record<string, unknown> = {}) {
    const rid = req.request_id; if (draftState.permissionPending[rid] || pendingIds.has(rid)) return;
    draftState.permissionPending[rid] = true; draftState.permissionErrors[rid] = '';
    try { await api.sessionPermission(project, id, { request_id: rid, behavior, ...extra }); draftState.permissionSent[rid] = true; }
    catch (e) { draftState.permissionErrors[rid] = msg(e); }
    finally { draftState.permissionPending[rid] = false; }
  }
  // answers to AskUserQuestion: the chosen options per question (multiSelect — several) or free text
  let picks = $state<Record<string, Record<string, string[]>>>({});
  let custom = $state<Record<string, Record<string, string>>>({});
  function pick(rid: string, q: string, label: string, multi: boolean) {
    const cur = picks[rid]?.[q] ?? [];
    const next = multi ? (cur.includes(label) ? cur.filter(l => l !== label) : [...cur, label]) : [label];
    picks[rid] = { ...(picks[rid] ?? {}), [q]: next };
  }
  const canAnswer = (req: SessionEvent) => (req.input?.questions ?? []).length > 0 && (req.input.questions as any[]).every(q => custom[req.request_id]?.[q.question]?.trim() || picks[req.request_id]?.[q.question]?.length);
  function submitAnswers(req: SessionEvent) {
    if (!canAnswer(req)) { draftState.permissionErrors[req.request_id] = t('ux.answerRequired'); return; }
    const answers: Record<string, string> = {};
    for (const q of req.input?.questions ?? []) {
      const own = custom[req.request_id]?.[q.question]?.trim();
      answers[q.question] = own || (picks[req.request_id]?.[q.question] ?? []).join(', ');
    }
    answer(req, 'allow', { answers });
  }
  async function interrupt() { try { await api.sessionInterrupt(project, id); } catch (e) { draftState.error = msg(e); } }
  async function resume() { try { await api.sessionCreate(project, { resume: id }); await refresh(); } catch (e) { draftState.error = msg(e); } }
  function summary(name: string, input: any): string { return adapter.toolSummary(name, input); }
  function resultText(content: any): string {
    if (typeof content === 'string') return content;
    if (Array.isArray(content)) return content.map((b: any) => (b.type === 'text' ? b.text : b.type === 'tool_result' ? resultText(b.content) : '')).join('\n');
    return content ? JSON.stringify(content) : '';
  }
  const toolResults = $derived.by(() => {
    const m: Record<string, { text: string; error: boolean; truncated: boolean; full: number; seq: number }> = {};
    for (const e of events) if (e.kind === 'tool_result') for (const b of e.content ?? []) if (b.type === 'tool_result') m[b.tool_use_id] = { text: resultText(b.content), error: !!b.is_error, truncated: !!b.truncated, full: b.full_len ?? 0, seq: e.seq };
    return m;
  });
  function fmtTokens(n?: number) { if (n == null) return '—'; if (n >= 1e6) return (n / 1e6).toFixed(1).replace(/\.0$/, '') + 'M'; if (n >= 1000) return (n / 1000).toFixed(n >= 10000 ? 0 : 1).replace(/\.0$/, '') + 'k'; return String(n); }
  // model, effort and mode — menus in the composer (as in CloudCLI); labels and lists come from the adapter, facts —
  // from agentd caps
  $effect(() => { if (!working) live = ''; });
  // read-aloud as the answer goes (§4.7, slice 68): finished sentences of the live text go to the queue; the generation = the
  // seq of the owner's last message; the final flush — from the text of the finished message (the tail of the last chunk does
  // not reach the client: agentd flushes the live buffer right after the finished message)
  let spokenGen = -1; let spokenOffset = 0; let spokenChars = 0;
  const SPEAK_CAP = 2000;
  function speakLive(text: string, gen: number, final = false) {
    if (gen !== spokenGen) { spokenGen = gen; spokenOffset = 0; spokenChars = 0; }
    if (spokenChars >= SPEAK_CAP) return;
    const { chunks, end } = sentencesFrom(text, spokenOffset);
    const tail = final ? text.slice(end).replace(/```[\s\S]*?```/g, ' (код) ').trim() : '';
    const all = tail && /[\p{L}\p{N}]{3,}/u.test(tail) ? [...chunks, tail] : chunks;
    for (const c of all) { if (spokenChars >= SPEAK_CAP) break; spokenChars += c.length; voice.enqueue(`${id}:${gen}:${spokenOffset}:${c.length}`, c); }
    spokenOffset = final ? text.length : end;
  }
  // turn generation — the seq of the owner's last message: the same for the live text and for the final flush
  const turnGen = $derived.by(() => { for (let i = events.length - 1; i >= 0; i--) { if (events[i].kind === 'user') return events[i].seq; } return 0; });
  $effect(() => {
    const text = live; const gen = turnGen; const on = voice.auto;
    if (!on || !text) return;
    untrack(() => speakLive(text, gen));
  });
  const modeValue = $derived(info?.permission_mode ?? caps?.current_mode ?? (info?.agent === 'codex' ? 'on-request' : 'default'));
  async function setModel(v: string | null, ctxWindow?: number | null) { if ((v || null) === (info?.model ?? null)) return; try { await api.sessionModel(project, id, v || null, ctxWindow ?? null); await refresh(); } catch (e) { draftState.error = msg(e); } }
  async function setMode(v: string) { try { await api.sessionMode(project, id, v); await refresh(); } catch (e) { draftState.error = msg(e); } }
  async function setEffort(v: string | null) { try { await api.sessionEffort(project, id, v); await refresh(); } catch (e) { draftState.error = msg(e); } }
  // gate models (the project's llm provider): a group in the model menu — any of them is accepted by
  // the CLI as a model id
  let gateModels = $state<{ app: string; title: string; models: GateModel[] }[]>([]); let gateAt = 0;
  async function loadGateModels() { if (Date.now() - gateAt < 60000) return; gateAt = Date.now(); try { gateModels = (await api.gateModels(project)).targets; } catch {} }
  $effect(() => { if (info) loadGateModels(); });
  const win = (n?: number | null) => (n ? (n >= 1e6 ? `${Math.round(n / 1e5) / 10}M` : `${Math.round(n / 1000)}k`) : '');
  const gateItems = (cur: string | null) => {
    const all = gateModels.flatMap(tg => tg.models);
    const groups = byProvider(all);
    if (!groups.length) return [];
    // gate models by account channels (Claude Code, Codex, Antigravity, Devin…); the id is real — Claude Code and Codex accept it through
    // the gate
    return [{ header: true, label: t('cmp.gateModels') }, ...groups.flatMap(g => [{ sub: true, label: `${g.title} · ${g.models.length}` },
      ...g.models.map(m => { const label = m.name && m.name !== m.id ? m.name : m.id; return { label, hint: [label !== m.id ? m.id : '', win(m.context)].filter(Boolean).join(' · ') || undefined, checked: cur === m.id, on: () => setModel(m.id, m.context ?? null) }; })])];
  };
  function modelMenu(el: HTMLElement) {
    const cur = info?.model ?? null;
    if (adapter.id === 'codex') {
      menu.show(el, [{ header: true, label: t('cmp.selectModel') }, { label: 'Default', hint: t('cmp.modelDefaultSet'), checked: !cur, on: () => setModel(null) },
        ...adapter.models(caps).filter(m => m.value).map(m => ({ label: m.label, hint: [m.desc, win(data.contextOf(m.value))].filter(Boolean).join(' · ') || undefined, checked: cur === m.value, on: () => setModel(m.value, data.contextOf(m.value)) })), ...gateItems(cur)], { search: true, placeholder: t('chat.modelSearch') });
      return;
    }
    const def = adapter.models(caps).find(m => m.value === 'default');
    const defHint = def?.desc?.match(/^Use the default model \(currently (.*)\)\s*·?\s*(.*)$/i);
    menu.show(el, [{ header: true, label: t('cmp.modelHint') }, { label: t('agents.modelDefault'), hint: defHint ? [defHint[1], defHint[2]].filter(Boolean).join(' · ') : def?.desc ?? '', checked: !cur, on: () => setModel(null) },
      ...adapter.models(caps).filter(m => m.value && m.value !== 'default').map(m => ({ label: `${m.label}${m.context ? ' · ' + (m.context >= 1e6 ? '1M' : Math.round(m.context / 1000) + 'k') : ''}`, hint: m.desc ?? m.hint, checked: cur === m.value, on: () => setModel(m.value) })), ...gateItems(cur)], { search: true, placeholder: t('chat.modelSearch') });
  }
  function effortMenu(el: HTMLElement) {
    const cur = info?.effort ?? null;
    menu.show(el, [{ header: true, label: t('cmp.effortHint') }, { label: t('cmp.effortAuto'), checked: !cur, on: () => setEffort(null) }, ...adapter.efforts(caps).map(e => ({ label: e.label, hint: e.value, checked: cur === e.value, on: () => setEffort(e.value) }))]);
  }
  function modeMenu(el: HTMLElement) {
    const noBypass = caps?.bypass_allowed === false;
    menu.show(el, [{ header: true, label: t('cmp.modeHint') }, ...adapter.modes(caps).map(m => {
      const blocked = noBypass && m.value === 'bypassPermissions';
      return { label: m.label, hint: blocked ? t('cmp.bypassBlocked') : m.hint, checked: modeValue === m.value, disabled: blocked, on: () => setMode(m.value) };
    })]);
  }
  // context: for Claude — get_context_usage, otherwise the adapter's estimate from the journal; the prompt cache
  // — from the last result
  const ctx = $derived<Ctx | null>(ctxProvider ?? adapter.context(caps, events, info?.model));
  // spending limits (§8.3, slice 67): a bar from 80 % of the workspace or total limit, red on "stop" and exhaustion
  const budget = $derived(data.budgetOf(project));
  // slice 61 (§9.7): the live subscription limit chip — the last rate_limit_event (Claude as is, Codex normalized by agentd to the
  // same shape)
  type LimitWin = { id: string; pct: number; resetsAt: number | null };
  const limits = $derived.by(() => {
    for (let i = events.length - 1; i >= 0; i--) {
      const e = events[i]; if (e.kind !== 'rate_limit_event') continue;
      const inf = e.data?.rate_limit_info; if (!inf || typeof inf !== 'object') continue;
      const secs = (r: unknown) => (typeof r === 'number' ? (r > 1e10 ? Math.round(r / 1000) : r) : null);
      const wins: LimitWin[] = Object.entries((inf.unifiedWindows ?? {}) as Record<string, any>).filter(([, w]) => w && typeof w.utilization === 'number').map(([id, w]) => ({ id, pct: Math.round(w.utilization * 100), resetsAt: secs(w.resetsAt) }));
      if (!wins.length && typeof inf.utilization === 'number') wins.push({ id: String(inf.rateLimitType ?? 'five_hour'), pct: Math.round(inf.utilization * 100), resetsAt: secs(inf.resetsAt) });
      if (!wins.length) continue;
      const top = wins.reduce((a, b) => (b.pct > a.pct ? b : a));
      return { status: String(inf.status ?? 'allowed'), top, wins, plan: typeof inf.planType === 'string' ? inf.planType : null };
    }
    return null;
  });
  const winLabel = (id: string) => id === 'five_hour' ? t('lim.fiveHour') : id === 'seven_day' ? t('lim.sevenDay') : id === 'seven_day_opus' ? t('lim.sevenDayOpus') : id === 'seven_day_sonnet' ? t('lim.sevenDaySonnet') : /^w\d+$/.test(id) ? t('lim.hours', { n: Math.round(Number(id.slice(1)) / 60) }) : id;
  function resetIn(ts: number | null): string {
    if (!ts) return ''; const m = Math.round((ts * 1000 - Date.now()) / 60000); if (m <= 0) return t('lim.resetNow');
    return t('lim.resetIn', { t: m < 60 ? t('time.min', { n: m }) : m % 60 ? `${t('time.h', { n: Math.floor(m / 60) })} ${t('time.min', { n: m % 60 })}` : t('time.h', { n: Math.floor(m / 60) }) });
  }
  function limitMenu(el: HTMLElement) {
    if (!limits) return;
    menu.show(el, [{ header: true, label: t('lim.chipTitle') + (limits.plan ? ` · ${limits.plan}` : '') + (limits.status === 'rejected' ? ` · ${t('lim.chipRejected')}` : '') },
      ...limits.wins.map(w => ({ label: `${winLabel(w.id)} · ${w.pct}%`, hint: resetIn(w.resetsAt), on: () => {} })),
      { sep: true }, { icon: 'key', label: t('lim.chipAccounts'), on: () => { ui.section = 'settings'; ui.settingsTab = 'accounts'; } }]);
  }
  // the "Mode and effort" popup (like ⇧+Tab in Claude Code): modes with descriptions and
  // the effort slider
  let modesOpen = $state(false);
  // skills in the conversation (slice 36, §4.3.10): a per-session choice in the browser; the texts of ones not yet sent go
  // as a block before the message
  let skillsOpen = $state(false); let skillsAll = $state<Skill[]>([]); let skillSel = $state<string[]>([]); let skillSent = $state<string[]>([]);
  const SKILLS_HEAD = '# Навыки (как делать повторяющуюся работу)';
  function skillsLoad() { try { const v = JSON.parse(localStorage.getItem(`agents.skills.${id}`) || '{}'); skillSel = Array.isArray(v.selected) ? v.selected : []; skillSent = Array.isArray(v.sent) ? v.sent : []; } catch { skillSel = []; skillSent = []; } }
  function skillsPersist() { try { localStorage.setItem(`agents.skills.${id}`, JSON.stringify({ selected: skillSel, sent: skillSent })); } catch {} }
  $effect(() => { id; skillsLoad(); });
  async function skillsOpenToggle() { modesOpen = false; effortOpen = false; skillsOpen = !skillsOpen; if (skillsOpen && !skillsAll.length) { try { skillsAll = (await agents.skills()).skills; } catch {} } }
  function toggleSkill(name: string) {
    skillSel = skillSel.includes(name) ? skillSel.filter((x) => x !== name) : [...skillSel, name]; skillsPersist();
    if (adapter.id === 'claude' && skillSel.length) agents.skillsMaterialize(project, skillSel).catch(() => {});
  }
  /** Skills block as in routines (routines.rs): a heading, then "## name — description" and the body; after "---" —
   *  the question itself. */
  function skillsBlock(names: string[]): string {
    const parts = names.map((n) => skillsAll.find((x) => x.name === n)).filter((x): x is Skill => !!x).map((x) => `## ${x.name} — ${x.description.trim()}\n${x.body.trim()}\n`);
    return parts.length ? `${SKILLS_HEAD}\n${parts.join('')}` : '';
  }
  /** An owner's message with a skills block is folded in the feed: the skill names and the
   *  question itself. */
  function userParts(text: string): { skills: string[]; text: string } {
    if (!text.startsWith(SKILLS_HEAD)) return { skills: [], text };
    const i = text.lastIndexOf('\n---\n'); const head = i >= 0 ? text.slice(0, i) : text;
    return { skills: Array.from(head.matchAll(/^## (\S+)/gm)).map((m) => m[1]), text: i >= 0 ? text.slice(i + 5).trim() : '' };
  }
  let effortOpen = $state(false);
  const effortSteps = $derived(adapter.efforts(caps).map(e => e.value));
  const effortIdx = $derived(info?.effort ? effortSteps.indexOf(info.effort) : -1);
  let thinkingInput = $state('');
  let customOpen = $state(false);
  $effect(() => { thinkingInput = info?.max_thinking_tokens ? String(info.max_thinking_tokens) : ''; if (info?.max_thinking_tokens) customOpen = true; });
  async function applyThinking() { const n = parseInt(thinkingInput, 10); try { await api.sessionThinking(project, id, Number.isFinite(n) && n > 0 ? n : null); await refresh(); } catch (e) { draftState.error = msg(e); } }
  async function toggleFast() { try { await api.sessionTier(project, id, info?.service_tier === 'fast' ? null : 'fast'); await refresh(); } catch (e) { draftState.error = msg(e); } }
  function ctxMenu(el: HTMLElement) {
    if (!ctx) return;
    const cats = (ctx as any).categories as { name: string; tokens: number }[] | undefined;
    menu.show(el, [{ header: true, label: t('cmp.contextDetails', { used: fmtTokens(ctx.used), window: fmtTokens(ctx.window), pct: ctx.pct }) + (ctx.threshold ? ` · ${t('cmp.autoCompactAt', { n: fmtTokens(ctx.threshold) })}` : '') },
      ...(cats ?? []).map(c => ({ label: c.name, hint: fmtTokens(c.tokens), on: () => {} })),
      ...(cache ? [{ sep: true }, { icon: 'refresh', label: t('cmp.cacheLine', { read: fmtTokens(cache.read), write: fmtTokens(cache.write) }), hint: cache.ttl ? `${cache.oneH ? t('cmp.cache1h') : t('cmp.cache5m')} · ${cacheLeft > 0 ? mmss(cacheLeft) : t('cmp.cacheExpired')}` : '', on: () => {} }] : []),
      ...(adapter.supportsCompact ? [{ sep: true }, { icon: 'refresh', label: '/compact', hint: t('cmp.contextFull'), on: () => sendText('/compact') }, { icon: 'info', label: '/context', on: () => sendText('/context') }] : [])]);
  }
  const lastResult = $derived([...events].reverse().find(e => e.kind === 'result' && e.num_turns !== 0 && e.usage));
  const cache = $derived.by(() => {
    const r = lastResult; if (!r) return null; const u = r.usage;
    const read = u.cache_read_input_tokens ?? u.cachedInputTokens ?? 0, write = u.cache_creation_input_tokens ?? 0;
    if (!read && !write) return null;
    const oneH = (u.cache_creation?.ephemeral_1h_input_tokens ?? 0) > 0; const ttl = adapter.id === 'claude' ? (oneH ? 3600 : 300) : 0;
    return { read, write, oneH, ttl, at: r.ts as number };
  });
  const cacheLeft = $derived(cache?.ttl ? Math.max(0, cache.at + cache.ttl * 1000 - now) : -1);
  const mmss = (ms: number) => `${Math.floor(ms / 60000)}:${String(Math.floor((ms % 60000) / 1000)).padStart(2, '0')}`;
  // slash commands and "@" files — a popup above the input field; /model /effort /mode are intercepted and go
  // through the session API
  function autoGrow() { if (!ta) return; ta.style.height = 'auto'; ta.style.height = Math.min(ta.scrollHeight, window.innerHeight < 480 ? 72 : 180) + 'px'; }   // short screen (phone in landscape) — up to three lines
  function onInput() { histIdx = null; autoGrow(); updatePopup(); }
  /** Design Mode (§4.9 step 3): an element from the agent browser — text at the cursor position and a screenshot crop as an attachment; accepted only by the
   *  composer of the selected session */
  function onDesignPick(ev: Event) {
    const d = (ev as CustomEvent).detail as { text: string; image?: { name: string; type: string; data: string }; handled: boolean };
    if (!d || d.handled || !ui.sel || !same(ui.sel, { project, kind: 'session', id })) return;
    d.handled = true;
    insertText(d.text);
    if (d.image) { try { const bin = atob(d.image.data); const arr = new Uint8Array(bin.length); for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i); addImages([new File([arr], d.image.name, { type: d.image.type })]); } catch {} }
  }
  $effect(() => { window.addEventListener('agents:design', onDesignPick); return () => window.removeEventListener('agents:design', onDesignPick); });
  /** Dictation (§4.7): text at the cursor position, the cursor after it; on touch the field is not focused so the keyboard does
   *  not pop up */
  /** Voice command from dictation (§4.7, slice 69): a short command phrase is executed, the rest is inserted as text */
  async function voiceCommand(cmd: VoiceCommand): Promise<boolean> {
    switch (cmd.kind) {
      case 'text': insertText(cmd.text); return true;
      case 'send': if (!draftState.text.trim() && !draftState.images.length) { toast(t('vc.nothingToSend'), 'bad'); return true; } await send(); toast(t('vc.sent')); return true;
      case 'stop': if (!working) { toast(t('vc.notRunning'), 'bad'); return true; } await interrupt(); toast(t('vc.stopped')); return true;
      case 'allow': case 'deny': {
        const p = pendingRequest; if (!p) { toast(t('vc.noRequest'), 'bad'); return true; }
        await actions.permission(project, id, { request_id: p.request_id, behavior: cmd.kind }); toast(t(cmd.kind === 'allow' ? 'vc.allowed' : 'vc.denied')); return true;
      }
      case 'clear': draftState.text = ''; draftState.images = []; toast(t('vc.cleared')); return true;
      case 'read': {
        const last = [...events].reverse().find(e => e.kind === 'assistant' && !e.local && (e.content ?? []).some((b: any) => b.type === 'text' && b.text?.trim()));
        if (!last) { toast(t('vc.noReply'), 'bad'); return true; }
        const text = (last.content ?? []).filter((b: any) => b.type === 'text').map((b: any) => b.text ?? '').join('\n');
        await speakMsg(`${id}:${last.seq}`, text); return true;
      }
      case 'new': {
        const ws = matchProject(cmd.arg ?? '', data.running.map(p => p.name));
        if (!ws) { toast(t('vc.noWorkspace', { name: cmd.arg ?? '' }), 'bad'); return true; }
        toast(t('vc.creating', { ws })); await actions.createSession(ws, 'claude'); return true;
      }
    }
    return false;
  }
  function insertText(text: string) {
    const cur = draftState.text ?? ''; const s = ta?.selectionStart ?? cur.length, e = ta?.selectionEnd ?? cur.length;
    const r = insertAt(cur, [s, e], text); draftState.text = r.value;
    tick().then(() => { if (ta) { try { ta.setSelectionRange(r.caret, r.caret); } catch {} if (!touch()) ta.focus(); } autoGrow(); });
  }
  // Interim text of streaming dictation (§4.7, slice 72): a region in the field that every update replaces whole
  let interim: Interim | null = null;
  function onPartial(text: string) {
    const cur = draftState.text ?? ''; const s = ta?.selectionStart ?? cur.length, e = ta?.selectionEnd ?? cur.length;
    const r = applyInterim(cur, [s, e], interim, text); draftState.text = r.value; interim = r.region;
    tick().then(() => { if (ta) { try { ta.setSelectionRange(r.caret, r.caret); } catch {} } autoGrow(); });
  }
  function onVoice(text: string) {
    if (interim) onPartial('');   // the result replaces the interim text; a command is executed without a trace in the field
    const cmd = parseCommand(text);
    if (cmd) { voiceCommand(cmd).catch(e => toast(msg(e), 'bad')); return; }
    insertText(text);
  }
  function updatePopup() {
    const pos = ta?.selectionStart ?? draftState.text.length; const before = draftState.text.slice(0, pos);
    const slash = before.match(/^\/([\w-]*)$/);
    if (slash) {
      const q = slash[1].toLowerCase();
      const items = adapter.commands(caps).filter(c => c.name.toLowerCase().startsWith(q)).slice(0, 14).map(c => ({ label: '/' + c.name, hint: c.hint, desc: c.desc, insert: '/' + c.name + ' ' }));
      popup = items.length ? { kind: 'cmd', items, sel: 0, start: 0, end: pos } : null; return;
    }
    const at = before.match(/(?:^|\s)@([^\s@]*)$/);
    if (at && adapter.supportsFiles) { const q = at[1]; const start = pos - q.length - 1; popup = { kind: 'file', items: popup?.kind === 'file' ? popup.items : [], sel: 0, start, end: pos, loading: true }; fetchFiles(q, start, pos); return; }
    popup = null;
  }
  let fileReq = 0;
  async function fetchFiles(q: string, start: number, end: number) {
    const n = ++fileReq;
    try {
      let paths: string[] = [];
      if (adapter.id === 'claude' && info?.alive) { try { const r = await api.sessionControl(project, id, { subtype: 'file_suggestions', query: q }); paths = ((r?.response ?? r)?.suggestions ?? []).map((x: any) => x.path); } catch {} }
      if (!paths.length) { const r = await api.fsFind(project, info?.cwd ?? '', q); paths = r.suggestions.map(x => x.path); }
      if (n !== fileReq || popup?.kind !== 'file') return;
      popup = { kind: 'file', items: paths.slice(0, 14).map(x => ({ label: x, insert: '@' + x + ' ' })), sel: 0, start, end, loading: false };
    } catch { if (popup?.kind === 'file') popup = { ...popup, loading: false }; }
  }
  function pickPopup(it: { insert: string }) {
    if (!popup) return; const caret = popup.start + it.insert.length;
    draftState.text = draftState.text.slice(0, popup.start) + it.insert + draftState.text.slice(popup.end); popup = null;
    tick().then(() => { ta?.focus(); ta?.setSelectionRange(caret, caret); autoGrow(); updatePopup(); });
  }
  function onKey(e: KeyboardEvent) {
    if (popup) {
      if (e.key === 'ArrowDown') { e.preventDefault(); popup.sel = (popup.sel + 1) % Math.max(1, popup.items.length); return; }
      if (e.key === 'ArrowUp') { e.preventDefault(); popup.sel = (popup.sel - 1 + popup.items.length) % Math.max(1, popup.items.length); return; }
      if ((e.key === 'Enter' || e.key === 'Tab') && popup.items.length) { e.preventDefault(); pickPopup(popup.items[popup.sel]); return; }
      if (e.key === 'Escape') { e.preventDefault(); popup = null; return; }
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { if (touch()) return; e.preventDefault(); send(); }
    // ↑/↓ in an empty field browse your past messages; Tab outside autocompletion moves the
    // focus.
    if ((e.key === 'ArrowUp' || e.key === 'ArrowDown') && hist.length && (histIdx != null || (!draftState.text.trim() && e.key === 'ArrowUp'))) {
      e.preventDefault();
      const next = e.key === 'ArrowUp' ? (histIdx == null ? hist.length - 1 : Math.max(0, histIdx - 1)) : (histIdx == null ? null : histIdx + 1);
      if (next == null || next >= hist.length) { histIdx = null; draftState.text = ''; } else { histIdx = next; draftState.text = hist[next]; }
      tick().then(autoGrow); return;
    }
    if (e.key === 'Escape') { e.preventDefault(); if (draftState.editing) cancelEdit(); else if (working) interrupt(); return; }
  }
  /** Command from the line: local ones (model, effort, mode) — through the session API; the rest for Claude — to the CLI itself (/compact,
   *  /context, /cost…). */
  async function runCommand(name: string, arg: string): Promise<boolean> {
    const cmd = adapter.commands(caps).find(c => c.name === name);
    if (cmd?.local) {
      if (name === 'model') { await setModel(arg || null); return true; }
      if (name === 'effort') { if (arg && !adapter.effortsAll.includes(arg)) { draftState.error = t('cmp.badArg', { v: arg }); return true; } await setEffort(arg === 'auto' ? null : arg || null); return true; }
      if (name === 'mode') { if (!adapter.modes(caps).some(m => m.value === arg)) { draftState.error = t('cmp.badArg', { v: arg }); return true; } await setMode(arg); return true; }
    }
    if (!cmd && adapter.id === 'codex') { draftState.error = t('cmp.unknownCmd', { agent: adapter.name, cmd: '/' + name }); return true; }
    return false;
  }
  function sendText(s: string) { draftState.text = s; send(); }
  /** The popup above the composer is no taller than the free space between the screen header and the composer (on the phone the first
   *  item otherwise went under the header) */
  function fitAbove(el: HTMLElement) {
    const fit = () => {
      const anchor = el.parentElement?.getBoundingClientRect(); if (!anchor) return;
      const top = (document.querySelector('.main .top') as HTMLElement | null)?.getBoundingClientRect().bottom ?? 0;
      el.style.maxHeight = `${Math.max(48, Math.floor(anchor.top - top - 12))}px`;
    };
    fit(); const ro = new ResizeObserver(fit); if (el.parentElement) ro.observe(el.parentElement); window.addEventListener('resize', fit);
    return { destroy() { ro.disconnect(); window.removeEventListener('resize', fit); } };
  }
  function popoverFocus(el: HTMLElement) {
    tick().then(() => (el.querySelector('button:not(:disabled), input') as HTMLElement | null)?.focus({ preventScroll: true }));
  }
  function popoverKeys(e: KeyboardEvent) { if (e.key === 'Escape') { e.stopPropagation(); modesOpen = false; effortOpen = false; skillsOpen = false; ta?.focus(); } }
</script>

<svelte:window onclick={() => { modesOpen = false; effortOpen = false; skillsOpen = false; }} onkeydown={(e) => { if (e.key === 'Escape') { modesOpen = false; effortOpen = false; skillsOpen = false; } }} />
<div class="chat">
  {#if runningTasks.length}
    <div class="tasks" title={t('task.strip')}><span class="muted xs">⏳ {t('task.strip')}</span>{#each runningTasks as tk (tk.task_id)}<span class="chip task" title={tk.description ?? ''}><Icon name={taskIcon(tk.task_type)} size="s" /> <span class="ell">{tk.description || tk.task_type || tk.task_id}</span> <span class="muted">{elapsed(tk.ts)}</span><button class="x" title={t('task.stop')} onclick={() => stopTask(tk.task_id)}>×</button></span>{/each}</div>
  {/if}
  {#if staleShown}<div class="cached-mark" role="status"><Icon name={offline ? 'alert' : 'refresh'} size="s" /> {offline ? t('chat.cachedOffline', { when: when(Math.floor(cachedAt / 1000)) }) : t('chat.cachedAt', { when: when(Math.floor(cachedAt / 1000)) })}</div>{/if}
  <div class="log" bind:this={log} data-following={stick} onscroll={onLogScroll}>
    {#if hiddenCount > 0}<button class="earlier" onclick={showEarlier}>{t('chat.earlier', { n: hiddenCount })}</button>{:else if olderCount > 0}<button class="earlier" onclick={loadOlder}>{t('chat.earlier', { n: Math.min(olderCount, HISTORY) })}</button>{/if}
    {#each visible as e (e.seq)}
      {#if e.kind === 'user'}
        {@const note = taskNote(e)}
        {#if note}
          <details class="m sys muted task note"><summary>{note.status === 'completed' ? '✅' : note.status && note.status !== 'running' ? '⚠️' : '⏳'} {t('task.done')}{note.status ? ' · ' + note.status : ''}{note.summary ? ': ' + note.summary : ''}{note.meta ? ' · ' + note.meta : ''}</summary>{#if note.result}<pre class="mono res">{note.result.slice(0, 6000)}</pre>{/if}</details>
        {:else if cleanUser(userText(e)) || (e.content ?? []).some((b: any) => b.type === 'image')}
          <div class="m user" data-seq={e.seq}>{#each e.content ?? [] as b}{#if b.type === 'text' && cleanUser(b.text ?? '')}{@const uv = userParts(cleanUser(b.text ?? ''))}{#if uv.skills.length}<div class="sk muted xs"><Icon name="sparkles" size="s" /> {t('chat.skillsLine', { list: uv.skills.join(', ') })}</div>{/if}{#if uv.text}<div class="txt">{uv.text}</div>{/if}{:else if b.type === 'image'}<div class="muted">📷 {t('agents.image')}{b.bytes ? ` · ${Math.round(b.bytes / 1024)} KB` : ''}</div>{/if}{/each}<span class="macts">{#if adapter.id === 'claude'}<button class="ma" title={t('edit.title')} onclick={() => startEdit(e)}><Icon name="edit" size="s" /></button>{/if}<button class="ma" title={t('fork.here')} onclick={() => forkAt(e)}><Icon name="git" size="s" /></button></span></div>
        {/if}
      {:else if e.kind === 'assistant' && e.local}
        <div class="m cmd"><div class="ph muted"><Icon name="terminal" size="s" /> {t('cmp.commandOut')}</div>{#each e.content ?? [] as b}{#if b.type === 'text'}<div class="out"><Md text={b.text} /></div>{/if}{/each}</div>
      {:else if e.kind === 'assistant'}
        {#each e.content ?? [] as b}
          {#if b.type === 'text' && b.text?.trim()}{@const sk = `${id}:${e.seq}`}<div class="m agent" data-seq={e.seq} class:wide={/(^|\n)\s*(\||```|#{1,4} |[-*] |\d+\. )/.test(b.text)}><Md text={b.text} /><span class="macts"><button class="ma speak" class:on={voice.speaking === sk} class:busy={voice.speakLoading === sk} aria-pressed={voice.speaking === sk} title={voice.speaking === sk ? t('voice.stop') : t('voice.speak')} onclick={() => speakMsg(sk, b.text ?? '')}><Icon name={voice.speaking === sk ? 'square' : 'volume'} size="s" /></button>{#if canShare}<button class="ma share" title={t('chat.share')} aria-label={t('chat.share')} onclick={() => shareText(b.text ?? '')}><Icon name="share" size="s" /></button>{/if}</span></div>
          {:else if b.type === 'thinking' && b.thinking}<details class="m think"><summary>💭 {t('agents.thinking')}</summary><div class="txt muted">{b.thinking}</div></details>
          {:else if b.type === 'tool_use'}
            <details class="m tool" class:err={toolResults[b.id]?.error}><summary><b>{adapter.toolLabel(b.name)}</b> <span class="muted mono">{summary(b.name, b.input)}</span>{#if toolResults[b.id]} <span class="badge {toolResults[b.id].error ? 'bad' : 'ok'}">{toolResults[b.id].error ? t('agents.toolError') : t('agents.toolDone')}</span>{:else if working} <span class="badge run"><span class="spin"></span>{t('cmp.running')}</span>{/if}</summary>
              <pre class="mono">{JSON.stringify(b.input, null, 1)}</pre>{#if b.truncated}<button class="xs ghost expand" onclick={() => expandEvent(e.seq)}>{t('chat.showFull', { kb: Math.round((b.full_len ?? 0) / 1024) })}</button>{/if}{#if toolResults[b.id]}{@const tr = toolResults[b.id]}<pre class="mono res">{expanded.has(tr.seq) ? tr.text : tr.text.slice(0, 4000)}</pre>{#if tr.truncated || (tr.text.length > 4000 && !expanded.has(tr.seq))}<button class="xs ghost expand" onclick={() => expandEvent(tr.seq)}>{t('chat.showFull', { kb: Math.round(Math.max(tr.full, tr.text.length) / 1024) })}</button>{/if}{/if}
              {#if subs[b.id]?.length}<details class="sub"><summary>🤖 {t('sub.title')} · {tn('sub.messages', subs[b.id].length)}</summary>{#each subs[b.id] as se (se.seq)}{#if se.kind === 'assistant'}{#each se.content ?? [] as sb}{#if sb.type === 'text' && sb.text?.trim()}<div class="sm"><Md text={sb.text} /></div>{:else if sb.type === 'tool_use'}<div class="muted mono xs">▸ {adapter.toolLabel(sb.name)} {summary(sb.name, sb.input)}</div>{/if}{/each}{/if}{/each}</details>{/if}</details>
          {/if}
        {/each}
      {:else if e.kind === 'permission_request'}
        <div class="m perm" data-request={e.request_id} class:done={pendingIds.has(e.request_id)}>
          {#if e.question}
            <div class="ph">❓ {t('agents.question')}</div>
            {#each e.input?.questions ?? [] as q}
              <div class="q"><b>{q.header ? q.header + ': ' : ''}</b>{q.question}</div>
              {#if !pendingIds.has(e.request_id)}
                <div class="opts">{#each q.options ?? [] as o}<button class="mini" disabled={draftState.permissionPending[e.request_id]} aria-pressed={(picks[e.request_id]?.[q.question] ?? []).includes(o.label)} class:active={(picks[e.request_id]?.[q.question] ?? []).includes(o.label)} title={o.description} onclick={() => pick(e.request_id, q.question, o.label, !!q.multiSelect)}>{o.label}</button>{/each}</div>
                <input class="own" disabled={draftState.permissionPending[e.request_id]} aria-label={t('ux.answerField', { question: q.question })} placeholder={t('agents.ownAnswer')} value={custom[e.request_id]?.[q.question] ?? ''} oninput={(ev) => { custom[e.request_id] = { ...(custom[e.request_id] ?? {}), [q.question]: (ev.target as HTMLInputElement).value }; }} />
              {/if}
            {/each}
            {#if !pendingIds.has(e.request_id)}<div class="row" style="gap:6px; margin-top:6px; flex-wrap:wrap;"><button class="primary mini" disabled={draftState.permissionPending[e.request_id] || !canAnswer(e)} onclick={() => submitAnswers(e)}>{t('agents.answer')}</button><button class="mini" disabled={draftState.permissionPending[e.request_id]} onclick={() => answer(e, 'deny', { message: t('inbox.declined') })}>{t('agents.skip')}</button></div>{/if}
          {:else}
            <div class="ph"><Icon name="shield" size="s" /> {t('agents.permission')}: <b>{e.tool_name}</b>{#if ruleOf(e)}<span class="chip mono rule" title={t('perm.rule')}>{ruleOf(e)}</span>{/if}</div>
            {#if e.title || e.description}<div class="muted">{e.title ?? ''} {e.description ?? ''}</div>{/if}
            <pre class="mono">{summary(e.tool_name, e.input) || JSON.stringify(e.input, null, 1)}</pre>
            {#if e.decision_reason}<div class="muted sm">{e.decision_reason}</div>{/if}
            {#if !pendingIds.has(e.request_id)}
              <div class="row pacts">
                <button class="primary mini" disabled={draftState.permissionPending[e.request_id]} onclick={() => answer(e, 'allow')}>{t('agents.allow')}</button>
                {#if ruleOf(e)}<button class="mini" disabled={draftState.permissionPending[e.request_id]} onclick={(ev) => { ev.stopPropagation(); rememberMenu(ev.currentTarget as HTMLElement, e); }}>{t('perm.remember')} ▾</button>{:else if e.provider !== 'acp' || (e.options ?? []).some((o: any) => o.kind === 'allow_always')}<button class="mini" disabled={draftState.permissionPending[e.request_id]} onclick={() => answer(e, 'allow', { remember: true })}>{t('agents.allowAlways')}</button>{/if}
                <button class="mini danger" disabled={draftState.permissionPending[e.request_id]} onclick={() => answer(e, 'deny')}>{t('agents.deny')}</button>
              </div>
            {/if}
          {/if}
          {#if draftState.permissionErrors[e.request_id]}<div class="err" role="alert">{draftState.permissionErrors[e.request_id]}</div>{/if}
        </div>
      {:else if e.kind === 'permission_result'}
        <div class="m sys muted">{e.behavior === 'allow' ? '✅' : '⛔'} {e.tool_name} · {e.behavior === 'allow' ? t('agents.allowed') : t('agents.denied')}{e.auto ? ' · ' + t('perm.auto') : e.remember ? ' · ' + t('agents.remembered') + (e.rules?.length ? ': ' + e.rules.join(', ') + ' (' + scopeLabel(e.scope) + ')' : '') : ''}</div>
      {:else if e.kind === 'history_truncated'}
        <div class="m sys muted cut">✂ {t('edit.cut')}</div>
      {:else if e.kind === 'forked'}
        <div class="m sys muted">⑂ {t('fork.row', { from: (e.from ?? '').slice(0, 8) })}</div>
      {:else if e.kind === 'task'}
        {#if e.subtype === 'task_started'}<div class="m sys muted task">⏳ {t('task.started')}: {e.description || e.task_type || e.task_id}</div>
        {:else if (e.subtype === 'task_notification' || ['completed', 'failed', 'killed', 'stopped', 'cancelled', 'error'].includes(e.status)) && !notedTasks.has(e.task_id)}<div class="m sys muted task">{e.status === 'completed' ? '✅ ' + t('task.done') : ['killed', 'stopped', 'cancelled'].includes(e.status) ? '⏹ ' + t('task.stopped') : '⚠️ ' + t('task.failed')}{e.summary ? ': ' + e.summary : ''}{#if e.output_file} · <span class="mono xs">{t('task.output')}: {e.output_file}</span>{/if}</div>{/if}
      {:else if e.kind === 'result'}
        {#if e.num_turns !== 0}<div class="m sys muted turn">{e.is_error ? '⚠️ ' + (e.subtype ?? 'error') : '✓'} · {((e.duration_ms ?? 0) / 1000).toFixed(1)} s{#if e.num_turns} · {tn('agents.turns', e.num_turns)}{/if}{#if e.usage} · {fmtTokens(e.usage.input_tokens)}↑ {fmtTokens(e.usage.output_tokens)}↓{#if e.usage.cache_read_input_tokens || e.usage.cache_creation_input_tokens} · {t('cmp.cache')} {fmtTokens(e.usage.cache_read_input_tokens)}↺{#if e.usage.cache_creation_input_tokens} · +{fmtTokens(e.usage.cache_creation_input_tokens)}{/if}{/if}{/if}{#if e.total_cost_usd} · ${e.total_cost_usd.toFixed(3)}{/if}</div>{/if}
      {:else if e.kind === 'exit'}
        <div class="m sys muted">⏹ {t('agents.sessionExited')}</div>
      {:else if e.kind === 'interrupt'}
        <div class="m sys muted"><Icon name="square" size="s" /> {t('agents.interrupted')}</div>
      {:else if e.kind === 'error'}
        <div class="m sys" style="color: var(--bad);">⚠️ {e.message}</div>
      {:else if e.kind === 'model'}
        <div class="m sys muted"><Icon name="cpu" size="s" /> {e.model ?? t('agents.modelDefault')}</div>
      {:else if e.kind === 'permission_mode'}
        <div class="m sys muted"><Icon name="shield" size="s" /> {adapter.modeLabel(e.mode, caps)}</div>
      {:else if e.kind === 'effort'}
        <div class="m sys muted"><Icon name="zap" size="s" /> {t('cmp.effort')}: {adapter.effortLabel(e.effort)}</div>
      {:else if e.kind === 'thinking'}
        <div class="m sys muted"><Icon name="layers" size="s" /> {t('cmp.thinking')}: {e.max_thinking_tokens ?? 'auto'}</div>
      {:else if e.kind === 'tier'}
        <div class="m sys muted"><Icon name="zap" size="s" /> {t('cmp.fast')}: {e.tier ?? 'off'}</div>
      {:else if e.kind === 'cache_ttl'}
        <div class="m sys muted"><Icon name="refresh" size="s" /> {t('cmp.cacheTtl')}: {e.ttl ?? 'auto'}</div>
      {:else if e.kind === 'system' && e.subtype === 'permission_denied'}
        <div class="m sys muted">⛔ {JSON.stringify(e.data?.message ?? e.data).slice(0, 200)}</div>
      {:else if e.kind === 'system' && (e.subtype === 'agent_note' || e.subtype === 'agentd_restart' || e.subtype === 'recap_sent')}
        <div class="m sys muted" class:restart={e.subtype === 'agentd_restart'}><Icon name={e.subtype === 'agentd_restart' || e.subtype === 'recap_sent' ? 'refresh' : 'info'} size="s" /> {String(e.data?.message ?? '').slice(0, 300)}</div>
      {/if}
    {/each}
    {#if live}<div class="m agent live" class:wide={/(^|\n)\s*(\||```|#{1,4} |[-*] |\d+\. )/.test(live)}><Md text={live} live /><span class="caret"></span></div>
    {:else if working}<div class="m agent typing" title={t('cmp.typing')}><span></span><span></span><span></span></div>{/if}
    {#if !events.length}<div class="muted" style="padding: 12px;">{conn === 'live' ? t('agents.emptyChat') : t('agents.connecting')}</div>{/if}
  </div>
  {#if !stick && unread > 0}<button class="jump" onclick={jumpDown}>↓ {tn('chat.unread', unread)}</button>{/if}
  <div class="composer chat-composer" class:busy={working} class:dragging role="region" aria-label={t('cmp.messageLabel')} aria-busy={draftState.sending} ondragover={(e) => { if (e.dataTransfer?.types.includes('Files')) { e.preventDefault(); dragging = true; } }} ondragleave={() => (dragging = false)} ondrop={onDrop}>
    <div class="composer-state">
      <span class="composer-status" class:warn={conn !== 'live' || status === 'waiting'} role="status"><span class="dot {conn !== 'live' ? 'waiting' : working ? 'working' : status === 'waiting' ? 'waiting' : status === 'error' ? 'failed' : status === 'idle' ? 'ok' : ''}"></span>{draftState.readingImages ? t('common.loading') : connectionLabel}</span>
      <span class="composer-compact-mode" title={t('cmp.modeHint')}><Icon name="shield" size="s" /><span class="ell">{adapter.modeLabel(modeValue, caps)}</span></span>
      <button class="composer-mode-summary composer-narrow" title={t('cmp.modeHint')} aria-haspopup="dialog" aria-expanded={modesOpen} onclick={(e) => { e.stopPropagation(); skillsOpen = false; effortOpen = false; modesOpen = !modesOpen; }}><Icon name="shield" size="s" /><span class="ell">{adapter.modeLabel(modeValue, caps)}</span></button>
      {#if ctx}<button class="ctl ctx composer-context" class:warn={ctx.threshold ? ctx.used >= ctx.threshold * 0.9 : ctx.pct >= 80} title={t('cmp.contextDetails', { used: fmtTokens(ctx.used), window: fmtTokens(ctx.window), pct: ctx.pct })} aria-haspopup="menu" onclick={(e) => { e.stopPropagation(); ctxMenu(e.currentTarget as HTMLElement); }}><span class="ring" style="--p: {Math.max(0, Math.min(100, ctx.pct))}"></span><span class="lbl">{t('cmp.context')} <b class="num">{ctx.pct}%</b></span></button><span class="composer-short-context" class:warn={ctx.threshold ? ctx.used >= ctx.threshold * 0.9 : ctx.pct >= 80}><span class="ring" style="--p: {Math.max(0, Math.min(100, ctx.pct))}"></span>{t('cmp.context')} <b class="num">{ctx.pct}%</b></span>{/if}
      {#if voice.reading}<button class="ctl composer-reading" title={t('voice.readingStop')} aria-label={t('voice.readingStop')} onclick={() => voice.clearQueue()}><Icon name="volume" size="s" /><span class="lbl">{t('voice.reading')}</span><Icon name="square" size="s" /></button>{/if}
      {#if limits}<button class="ctl composer-limit" class:warn={limits.status !== 'rejected' && (limits.status === 'allowed_warning' || limits.top.pct >= 80)} class:bad={limits.status === 'rejected'} title={t('lim.chipHint', { win: winLabel(limits.top.id), pct: limits.top.pct })} aria-haspopup="menu" onclick={(e) => { e.stopPropagation(); limitMenu(e.currentTarget as HTMLElement); }}><Icon name="gauge" size="s" /><span class="lbl">{winLabel(limits.top.id)} <b class="num">{limits.top.pct}%</b></span></button>{/if}
    </div>
    {#if conn !== 'live' && connectedOnce}<div class="composer-notice" role="status"><Icon name="refresh" size="s" />{t('cmp.connectionLost')}</div>{/if}
    {#if resuming && conn === 'live'}<div class="composer-notice info"><Icon name="play" size="s" />{info?.tui ? t('agents.tuiHint') : t('agents.exitedHint')}</div>{/if}
    {#if outbox.length}<div class="composer-notice outbox" role="status"><Icon name="cloudoff" size="s" /><span>{t('outbox.waiting', { n: outbox.length })}{#if outbox.some(o => o.error)} · {t('outbox.failed', { err: outbox.find(o => o.error)?.error ?? '' })}{/if}</span>{#each outbox as o (o.id)}<button class="xs ghost outbox-drop" title={o.text} onclick={() => dropOutbox(o.id)}>{t('outbox.drop')}</button>{/each}</div>{/if}
    {#if budget}<div class="composer-notice budget" class:bad={budget.blocked} role={budget.blocked ? 'alert' : 'status'}><Icon name="alert" size="s" /><span>{budget.blocked ? t('budget.stopped', { scope: budget.scope === 'total' ? t('models.all') : budget.scope, spent: money(budget.spent), limit: money(budget.per_day) }) : t('budget.warn', { pct: Math.round(budget.pct), scope: budget.scope === 'total' ? t('models.all') : budget.scope, spent: money(budget.spent), limit: money(budget.per_day) })}</span><button class="xs ghost" onclick={() => { ui.section = 'models'; }}>{t('budget.open')}</button></div>{/if}
    {#if status === 'waiting'}<div class="composer-notice decision"><Icon name="shield" size="s" /><span>{t('cmp.needsDecision')}{#if pendingRequest?.tool_name} · {pendingRequest.question ? t('agents.question') : pendingRequest.tool_name}{/if}</span><button class="xs ghost" onclick={jumpDecision}>{t('cmp.jumpDecision')} <Icon name="arrowUR" size="s" /></button></div>{/if}
    {#if draftState.error}<div class="composer-error" role="alert">{draftState.error}</div>{/if}
    {#if draftState.images.length}<div class="thumbs">{#each draftState.images as im, i}<span class="thumb"><img src={im.url} alt={im.name} /><button class="x" aria-label={t('cmp.removeAttachment', { name: im.name })} onclick={() => draftState.images.splice(i, 1)}>×</button></span>{/each}</div>{/if}
    {#if draftState.editing}<div class="editbar"><Icon name="edit" size="s" /> {t('edit.banner', { n: draftState.editing.seq })}<span class="sp"></span><button class="xs ghost" onclick={cancelEdit}>{t('rt.cancel')}</button></div>{/if}
    {#each queued as q (q.id)}<div class="qcard"><span class="muted xs">⏳ {t('queue.label')}</span><button class="txt ell asl" title={t('queue.edit')} onclick={() => editQueued(q)}>{qtext(q)}</button><button class="x" title={t('rt.cancel')} onclick={() => unqueue(q.id)}>×</button></div>{/each}
    {#each scheduled as sm (sm.id)}<div class="qcard sched"><span class="muted xs">🕒 {t('sched.label')} · {when(sm.at)}</span><span class="txt ell">{sm.content}</span><button class="x" title={t('rt.cancel')} onclick={() => unschedule(sm.id)}>×</button></div>{/each}
    {#if popup}
      <div class="popup" role="listbox">
        <div class="pk muted">{popup.kind === 'cmd' ? t('cmp.commands') : t('cmp.files')}{#if popup.loading} · {t('common.loading')}{/if}</div>
        {#each popup.items as it, i (it.insert)}<button class="pi" class:sel={i === popup.sel} role="option" aria-selected={i === popup.sel} onmousedown={(e) => e.preventDefault()} onclick={() => pickPopup(it)}><span class="pn mono">{it.label}</span>{#if it.hint}<span class="ph mono muted">{it.hint}</span>{/if}{#if it.desc}<span class="pd muted">{it.desc}</span>{/if}</button>{/each}
        {#if !popup.items.length && !popup.loading}<div class="pk muted">{t('cmp.noMatches')}</div>{/if}
      </div>
    {/if}
    <div class="in composer-input">
      <textarea bind:this={ta} rows="2" bind:value={draftState.text} aria-label={t('cmp.messageLabel')} oninput={onInput} onkeydown={onKey} onclick={updatePopup} onpaste={onPaste} placeholder={t('cmp.placeholderPhone')}></textarea>
    </div>
    {#if effortOpen && adapter.id === 'codex'}
      <div class="effpop" role="dialog" tabindex="-1" aria-label={t('cmp.reasoning')} use:fitAbove use:popoverFocus onkeydown={popoverKeys} onclick={(e) => e.stopPropagation()}>
        <div class="ep-h"><Icon name="zap" size="s" /><span class="sp"></span><span class="ep-cur"><b>{info?.effort ?? caps?.default_effort ?? 'auto'}</b><span class="muted">{adapter.modelLabel(info?.model, caps)}</span></span><span class="sp"></span><button class="icon xs ghost" title={t('cmp.effortReset')} onclick={() => setEffort(null)}><Icon name="refresh" size="s" /></button></div>
        <input class="range" aria-label={t('cmp.reasoning')} type="range" min="0" max={Math.max(0, effortSteps.length - 1)} step="1" value={effortIdx >= 0 ? effortIdx : Math.max(0, effortSteps.indexOf(caps?.default_effort ?? ''))} style="--p: {effortSteps.length > 1 ? Math.round(((effortIdx >= 0 ? effortIdx : Math.max(0, effortSteps.indexOf(caps?.default_effort ?? ''))) / (effortSteps.length - 1)) * 100) : 100}%" oninput={(e) => setEffort(effortSteps[Number((e.target as HTMLInputElement).value)])} />
        <div class="ep-steps">{#each effortSteps as e, i (e)}<button class="ep-step" class:on={(info?.effort ?? caps?.default_effort) === e} onclick={() => setEffort(e)}>{e}</button>{/each}</div>
        <div class="muted xs ep-hint">{adapter.efforts(caps).find(e => e.value === (info?.effort ?? caps?.default_effort))?.hint ?? ''}</div>
        {#if caps?.tiers?.length}
          {#each caps.tiers as tr (tr.id)}<label class="ep-tier"><button class="switch" class:on={info?.service_tier === tr.id} aria-label={tr.name} aria-pressed={info?.service_tier === tr.id} onclick={() => api.sessionTier(project, id, info?.service_tier === tr.id ? null : tr.id).then(refresh).catch((e) => (draftState.error = msg(e)))}></button><span><b>{tr.name}</b><span class="muted xs">{tr.description ?? ''}</span></span></label>{/each}
        {/if}
      </div>
    {/if}
    {#if skillsOpen}
      <div class="modes skills-pop" role="dialog" tabindex="-1" aria-label={t('cmp.skills')} use:fitAbove use:popoverFocus onkeydown={popoverKeys} onclick={(e) => e.stopPropagation()}>
        <div class="mh"><span>{t('cmp.skills')}</span><span class="sp"></span><button class="icon xs ghost" aria-label={t('common.close')} onclick={() => (skillsOpen = false)}><Icon name="x" size="s" /></button></div>
        {#each skillsAll as sk (sk.name)}
          <label class="mode sk" data-skill={sk.name}><input type="checkbox" checked={skillSel.includes(sk.name)} onchange={() => toggleSkill(sk.name)} /><span class="mi-t"><b>{sk.name}</b><span class="muted">{sk.description}</span></span>{#if skillSent.includes(sk.name)}<span class="muted xs">{t('cmp.skillSent')}</span>{/if}</label>
        {:else}<div class="muted sm" style="padding: 8px 10px">{t('cmp.skillsEmpty')}</div>{/each}
        <div class="muted xs" style="padding: 6px 10px 8px">{t('cmp.skillsHint')}</div>
      </div>
    {/if}
    {#if modesOpen}
      <div class="modes" role="dialog" tabindex="-1" aria-label={t('cmp.modesTitle')} use:fitAbove use:popoverFocus onkeydown={popoverKeys} onclick={(e) => e.stopPropagation()}>
        <div class="mh"><span>{t('cmp.modesTitle')}</span><span class="sp"></span><span class="muted xs">{adapter.name}</span><button class="icon xs ghost" aria-label={t('common.close')} onclick={() => (modesOpen = false)}><Icon name="x" size="s" /></button></div>
        {#each adapter.modes(caps).filter(m => !m.hidden) as m (m.value)}
          {@const blocked = m.value === 'bypassPermissions' && caps?.bypass_allowed === false}
          <button class="mode" class:on={modeValue === m.value} class:dis={blocked} disabled={blocked} onclick={() => { setMode(m.value); }}>
            <span class="mi-ic"><Icon name={m.value === 'plan' ? 'eye' : m.value === 'auto' ? 'zap' : m.value === 'bypassPermissions' ? 'alert' : m.value === 'acceptEdits' ? 'edit' : m.value === 'never' ? 'zap' : 'shield'} size="s" /></span>
            <span class="mi-t"><b>{m.label}</b><span class="muted">{blocked ? t('cmp.bypassBlocked') : m.hint}</span></span>
            {#if modeValue === m.value}<Icon name="check" size="s" />{/if}
          </button>
        {/each}
        {#if adapter.id === 'claude' || (adapter.id !== 'codex' && adapter.efforts(caps).length)}
          <!-- Claude Code: one Effort row with a slider, as in its picker (⇧+Tab); the top step is ultracode.
               Antigravity (efforts from caps): the same slider, applied by an agy restart at the next message -->
          <div class="eff">
            <span class="mi-ic"><Icon name="zap" size="s" /></span>
            <span class="mi-t"><b>Effort <span class="muted">({info?.effort ? `${info.effort}${adapter.efforts(caps).find(e => e.value === info?.effort)?.hint ? ' — ' + adapter.efforts(caps).find(e => e.value === info?.effort)?.hint : ''}` : 'auto'})</span></b></span>
            <span class="slider mini" role="group" aria-label="Effort">
              {#each effortSteps as e, i (e)}<button class="dot" aria-label={t('cmp.reasoning') + ': ' + e} aria-pressed={effortIdx === i} class:on={effortIdx === i} class:past={effortIdx >= 0 && i < effortIdx} title={e} onclick={() => setEffort(e)}></button>{/each}
            </span>
            {#if info?.effort}<button class="icon xs ghost" title={t('cmp.effortReset')} onclick={() => setEffort(null)}><Icon name="refresh" size="s" /></button>{/if}
          </div>
        {/if}
        {#if adapter.id === 'claude'}
          <div class="eff sub">
            <span class="mi-ic"><Icon name="refresh" size="s" /></span>
            <span class="mi-t"><b>{t('cmp.cacheTtl')} <span class="muted">({info?.cache_ttl ?? 'auto'})</span></b><span class="muted">{t('cmp.cacheTtlHint')}</span></span>
            <span class="seg" role="group">
              {#each [['auto', 'auto'], ['5m', '5m'], ['1h', '1h']] as [v, l] (v)}<button class:on={(info?.cache_ttl ?? 'auto') === v} onclick={() => api.sessionCache(project, id, v === 'auto' ? null : v).then(refresh).catch((e) => (draftState.error = msg(e)))}>{l}</button>{/each}
            </span>
          </div>
        {:else}
          <!-- Codex: the approval modes popup; effort and tier — in their own popup (the effort
               button) -->
        {/if}
      </div>
    {/if}
    <div class="composer-toolbar">
      <div class="tools composer-controls">
        <button class="ctl model composer-control" title={t('cmp.modelHint')} aria-haspopup="menu" onclick={(e) => { e.stopPropagation(); modelMenu(e.currentTarget as HTMLElement); }}><Icon name={adapter.icon} size="s" /><span class="lbl">{adapter.modelLabel(info?.model, caps)}</span><Icon name="chevD" size="s" /></button>
        <button class="ctl mode composer-control composer-secondary" title={t('cmp.modesHint')} aria-haspopup="dialog" aria-expanded={modesOpen} onclick={(e) => { e.stopPropagation(); skillsOpen = false; effortOpen = false; modesOpen = !modesOpen; }}><Icon name="shield" size="s" /><span class="lbl">{adapter.modeLabel(modeValue, caps)}</span></button>
        {#if caps?.features?.effort ?? true}<button class="ctl effort composer-control composer-secondary" title={t('cmp.effortHint')} aria-haspopup="dialog" aria-expanded={adapter.id === 'codex' ? effortOpen : modesOpen} onclick={(e) => { e.stopPropagation(); skillsOpen = false; if (adapter.id === 'codex') { modesOpen = false; effortOpen = !effortOpen; } else { effortOpen = false; modesOpen = !modesOpen; } }}><Icon name="zap" size="s" /><span class="lbl">{adapter.effortLabel(info?.effort)}</span></button>{/if}
        {#if skillSel.length}<button class="ctl skills on composer-control composer-secondary" title={t('cmp.skillsHint')} aria-haspopup="dialog" aria-expanded={skillsOpen} onclick={(e) => { e.stopPropagation(); skillsOpenToggle(); }}><Icon name="sparkles" size="s" /><span class="lbl">{t('cmp.skills')} · {skillSel.length}</span></button>{/if}
        <button class="ctl more composer-control composer-more" title={t('cmp.parameters')} aria-label={t('cmp.parameters')} aria-haspopup="menu" onclick={(e) => { e.stopPropagation(); moreMenu(e.currentTarget as HTMLElement); }}><Icon name="settings" size="s" /><span class="more-label">{t('cmp.parameters')}{skillSel.length ? ` · ${skillSel.length}` : ''}</span></button>
      </div>
      <div class="composer-actions">
        <input bind:this={attachmentInput} type="file" accept="image/*" multiple hidden onchange={attach} />
        <Mic onText={onVoice} {onPartial} />
        <button class="attach composer-attach" title={t('agents.attach')} aria-label={t('agents.attach')} onclick={() => attachmentInput?.click()}><Icon name="clip" /></button>
        {#if phone()}<input bind:this={cameraInput} type="file" accept="image/*" capture="environment" hidden onchange={attach} /><button class="attach cam composer-attach" title={t('chat.camera')} aria-label={t('chat.camera')} onclick={() => cameraInput?.click()}><Icon name="camera" /></button>{/if}
        {#if working || status === 'waiting'}<button class="stop composer-attach" onclick={interrupt} disabled={conn !== 'live'} title={t('agents.stopTurn')} aria-label={t('agents.stopTurn')}><Icon name="square" size="s" /></button>{/if}
        <button class="send primary composer-send" class:queueing onclick={send} disabled={draftState.sending || draftState.readingImages > 0 || (conn !== 'live' && (!!draftState.editing || queueing)) || (!draftState.text.trim() && !draftState.images.length)} title={sendLabel} aria-label={sendLabel}><Icon name={draftState.sending ? 'refresh' : draftState.editing ? 'edit' : queueing ? 'plus' : 'send'} size="s" /><span class="action-label">{sendLabel}</span></button>
      </div>
    </div>
    {#if !phone()}<div class="composer-hint"><span>{t('cmp.commandsHint')}</span><span>{t('cmp.enterHint')}</span></div>{/if}
  </div>
</div>

<style>
  .chat { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 6px; }
  .log { flex: 1; min-height: 0; overflow: auto; overflow-anchor: none; display: flex; flex-direction: column; gap: 10px; padding: 4px 2px; }
  .m { max-width: 76%; border-radius: var(--r-3); padding: 9px 13px; line-height: 1.5; font-size: 13.5px; background: var(--bg-3); border: 1px solid var(--line-1); }
  .m.user { align-self: flex-end; background: var(--accent-soft); border-color: rgb(var(--accent-rgb) / .25); border-bottom-right-radius: 4px; }
  .m.agent { align-self: flex-start; border-bottom-left-radius: 4px; }
  .m.sys { align-self: center; background: transparent; border: 0; font-size: 11.5px; color: var(--fg-3); padding: 0 8px; display: flex; align-items: center; gap: 5px; }
  .m.tool { align-self: stretch; max-width: 100%; width: 100%; padding: 0; background: var(--bg-4); font-size: 12.5px; }
  .m.tool summary { display: flex; align-items: center; gap: 8px; min-height: 30px; padding: 4px 10px; cursor: pointer; list-style: none; color: var(--fg-2); }
  .m.tool summary::-webkit-details-marker { display: none; }
  .m.tool summary b { color: var(--fg-1); font-weight: 500; }
  .m.tool summary .mono { font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
  .m.tool.err { border-color: rgb(242 109 109 / .4); }
  .m.think { align-self: flex-start; font-size: 12.5px; color: var(--fg-3); background: transparent; border: 0; padding: 0 4px; max-width: 76%; }
  .m.think summary { cursor: pointer; list-style: none; } .m.think summary::-webkit-details-marker { display: none; }
  .m.think .txt { border-left: 2px solid var(--line-3); padding: 6px 10px; margin-top: 4px; }
  /* flex-shrink 0: with overflow hidden the block otherwise shrinks to a strip when the journal of a short card overflows */
  .m.perm { align-self: stretch; flex-shrink: 0; max-width: 100%; border-color: var(--line-2); position: relative; overflow: hidden; padding-left: 15px; }
  .m.perm::before { content: ""; position: absolute; left: 0; top: 0; bottom: 0; width: 3px; background: var(--warn); }
  .m.perm.done { opacity: .55; } .m.perm.done::before { background: var(--fg-3); }
  .ph { font-weight: 600; margin-bottom: 6px; display: flex; align-items: center; gap: 6px; flex-wrap: wrap; min-width: 0; }
  .ph .chip.rule { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* permission card buttons wrap: on the phone three buttons do not fit in a row, the third one went past the card edge */
  .pacts { gap: 6px; margin-top: 6px; flex-wrap: wrap; }
  .txt { white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.5; }
  .mono { font-family: var(--mono); font-size: 12px; }
  pre.mono { white-space: pre-wrap; overflow-wrap: anywhere; margin: 6px 0 0; max-height: 240px; overflow: auto; padding: 8px 10px; background: var(--bg-4); border-radius: var(--r-1); border: 1px solid var(--line-1); }
  .m.tool pre.mono { margin: 0; border: 0; border-top: 1px solid var(--line-1); border-radius: 0; }
  .res { color: var(--fg-2); }
  .badge { display: inline-flex; align-items: center; height: 20px; padding: 0 7px; border-radius: 999px; font-size: 11px; margin-left: auto; }
  .badge.ok { background: var(--ok-soft); color: var(--ok); border-color: transparent; } .badge.bad { background: var(--bad-soft); color: var(--bad); border-color: transparent; }
  summary { cursor: pointer; }
  .q { margin-top: 6px; }
  .opts { display: flex; flex-wrap: wrap; gap: 6px; margin: 6px 0; }
  .mini.active { background: var(--accent-soft); border-color: rgb(var(--accent-rgb) / .5); color: var(--accent); }
  .own { width: 100%; height: 28px; font-size: 12.5px; }
  .thumbs { display: flex; gap: 6px; flex-wrap: wrap; }
  .thumb { position: relative; }
  .thumb img { width: 56px; height: 56px; object-fit: cover; border-radius: 8px; border: 1px solid var(--line-2); }
  .thumb .x { position: absolute; top: -6px; right: -6px; width: 20px; height: 20px; padding: 0; border-radius: 50%; font-size: 12px; line-height: 1; }
  @media (pointer: coarse) { .thumb .x { min-width: 36px; min-height: 36px; } }
  .sm { font-size: 12px; }
  .popup { position: absolute; left: 6px; right: 6px; bottom: calc(100% + 6px); max-height: 300px; overflow: auto; background: var(--bg-2); border: 1px solid var(--line-2); border-radius: var(--r-2); box-shadow: var(--shadow-2, 0 12px 32px rgb(0 0 0 / .35)); padding: 4px; z-index: 5; display: flex; flex-direction: column; }
  .popup .pk { font-size: 11px; padding: 4px 8px; }
  .popup .pi { display: grid; grid-template-columns: auto auto 1fr; align-items: baseline; gap: 8px; text-align: left; height: auto; min-height: 28px; padding: 4px 8px; border: 0; background: transparent; border-radius: var(--r-1); color: var(--fg-1); }
  .popup .pi.sel, .popup .pi:hover { background: var(--accent-soft); }
  .popup .pn { font-size: 12.5px; white-space: nowrap; } .popup .ph { font-size: 11.5px; white-space: nowrap; }
  .popup .pd { font-size: 11.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ring { width: 14px; height: 14px; border-radius: 50%; flex: none; background: conic-gradient(var(--accent) calc(var(--p) * 1%), var(--line-3) 0); -webkit-mask: radial-gradient(circle at center, transparent 4px, #000 4.5px); mask: radial-gradient(circle at center, transparent 4px, #000 4.5px); }
  .ctx.warn .ring { background: conic-gradient(var(--warn) calc(var(--p) * 1%), var(--line-3) 0); }
  .m.cmd { align-self: stretch; max-width: 100%; background: var(--bg-4); padding: 8px 12px; }
  .m.cmd .ph { font-size: 11.5px; font-weight: 500; margin-bottom: 4px; }
  .m.cmd .out { max-height: 360px; overflow: auto; font-size: 12.5px; }
  /* an answer with a table, code, headings or a list — the full width of the journal */
  .m.agent.wide { max-width: 100%; align-self: stretch; }
  .m.agent.live .caret { display: inline-block; width: 2px; height: 1em; margin-left: 2px; vertical-align: text-bottom; background: var(--accent); animation: blink 1s steps(2) infinite; }
  .m.typing { display: inline-flex; gap: 4px; padding: 10px 14px; }
  .m.typing span { width: 6px; height: 6px; border-radius: 50%; background: var(--fg-3); animation: blink 1.2s infinite ease-in-out; }
  .m.typing span:nth-child(2) { animation-delay: .2s; } .m.typing span:nth-child(3) { animation-delay: .4s; }
  @keyframes blink { 0%, 80%, 100% { opacity: .25; transform: translateY(0); } 40% { opacity: 1; transform: translateY(-2px); } }
  .badge.run { background: var(--accent-soft); color: var(--accent); gap: 5px; }
  .spin { width: 10px; height: 10px; border-radius: 50%; border: 2px solid currentColor; border-right-color: transparent; animation: spin .8s linear infinite; display: inline-block; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .m.sys.turn { flex-wrap: wrap; }
  /* background task note (<task-notification>): a full-width block — the title as a line, the output below it with wrapping */
  .m.sys.task.note { display: block; align-self: stretch; max-width: 100%; padding: 6px 10px; border: 1px solid var(--line-1); border-radius: var(--r-2); background: var(--bg-3); }
  .m.sys.task.note summary { cursor: pointer; list-style: none; line-height: 1.45; }
  .m.sys.task.note summary::-webkit-details-marker { display: none; }
  .m.sys.task.note pre.res { margin: 6px 0 0; white-space: pre-wrap; word-break: break-word; overflow-wrap: anywhere; max-height: 240px; overflow: auto; font-size: 11.5px; }
  .m.sys.task:not(.note) { flex-wrap: wrap; justify-content: center; text-align: center; }
  /* the "Mode and effort" popup */
  .modes { position: absolute; left: 6px; bottom: calc(100% + 6px); width: min(460px, calc(100% - 12px)); max-height: calc(100vh - 130px); overflow: auto; background: var(--bg-2); border: 1px solid var(--line-2); border-radius: var(--r-2); box-shadow: var(--shadow-2, 0 12px 32px rgb(0 0 0 / .35)); padding: 6px; z-index: 6; display: flex; flex-direction: column; gap: 2px; }
  .modes .mh { display: flex; align-items: center; gap: 8px; padding: 4px 8px 6px; font-size: 12px; color: var(--fg-3); }
  .modes .mode, .modes .eff { display: flex; align-items: center; gap: 10px; text-align: left; height: auto; padding: 8px 10px; border: 0; background: transparent; border-radius: var(--r-1); color: var(--fg-1); font-size: 13px; }
  .modes .mode:hover { background: var(--bg-3); } .modes .mode.on { background: var(--accent-soft); } .modes .mode.dis { opacity: .45; cursor: not-allowed; }
  .modes .mi-ic { width: 26px; height: 26px; border-radius: 8px; display: grid; place-items: center; background: var(--bg-3); color: var(--fg-2); flex: none; }
  .modes .mi-t { display: flex; flex-direction: column; gap: 2px; flex: 1; min-width: 0; } .modes .mi-t .muted { font-size: 11.5px; line-height: 1.3; white-space: normal; overflow-wrap: anywhere; }
  .modes .mi-t b { white-space: normal; }
  .modes .eff { border-top: 1px solid var(--line-1); margin-top: 4px; padding-top: 10px; flex-wrap: wrap; }
  .modes .eff > .slider { flex-basis: 100%; margin-left: 0; justify-content: space-between; }
  .slider { display: inline-flex; align-items: flex-start; gap: 4px; padding: 6px 6px 2px; border-radius: 12px; background: var(--bg-3); flex: none; }
  .slider .dot { width: 14px; height: 14px; border-radius: 50%; border: 0; padding: 0; background: var(--line-3); }
  .slider .dot.past { background: rgb(var(--accent-rgb) / .45); } .slider .dot.on { background: var(--accent); box-shadow: 0 0 0 3px rgb(var(--accent-rgb) / .25); }
  .modes .eff.sub { border-top: 0; margin-top: 0; padding-top: 6px; }
  .slider.mini { flex-basis: auto; margin-left: auto; padding: 6px 8px; gap: 8px; flex-direction: row; }
  .slider.mini .dot { width: 12px; height: 12px; }
  /* Codex effort popup: level name, model, gradient slider, steps, tier */
  .effpop { position: absolute; left: 6px; bottom: calc(100% + 6px); width: min(360px, calc(100% - 12px)); background: var(--bg-2); border: 1px solid var(--line-2); border-radius: var(--r-2); box-shadow: var(--shadow-2, 0 12px 32px rgb(0 0 0 / .35)); padding: 12px 14px; z-index: 6; display: flex; flex-direction: column; gap: 10px; }
  .ep-h { display: flex; align-items: center; gap: 8px; color: var(--fg-3); }
  .ep-cur { display: flex; flex-direction: column; align-items: center; gap: 2px; } .ep-cur b { font-size: 16px; color: var(--accent); font-family: var(--mono); } .ep-cur .muted { font-size: 12px; }
  .range { -webkit-appearance: none; appearance: none; width: 100%; height: 22px; border-radius: 999px; background: linear-gradient(90deg, rgb(var(--accent-rgb) / .35), var(--accent) var(--p), var(--bg-3) var(--p)); outline: none; margin: 2px 0; }
  .range::-webkit-slider-thumb { -webkit-appearance: none; width: 26px; height: 26px; border-radius: 50%; background: #fff; box-shadow: 0 2px 8px rgb(0 0 0 / .35); cursor: pointer; }
  .range::-moz-range-thumb { width: 26px; height: 26px; border-radius: 50%; background: #fff; border: 0; cursor: pointer; }
  .ep-steps { display: flex; justify-content: space-between; }
  .ep-steps .ep-step { border: 0; background: transparent; padding: 0 2px; font-family: var(--mono); font-size: 10.5px; color: var(--fg-3); height: 18px; }
  .ep-steps .ep-step.on { color: var(--accent); font-weight: 600; }
  .ep-hint { text-align: center; min-height: 14px; }
  .ep-tier { display: flex; align-items: center; gap: 10px; border-top: 1px solid var(--line-1); padding-top: 10px; cursor: pointer; }
  .ep-tier > span { display: flex; flex-direction: column; gap: 1px; font-size: 12.5px; }
  .seg { display: inline-flex; border: 1px solid var(--line-2); border-radius: 999px; overflow: hidden; flex: none; }
  .seg button { height: 24px; padding: 0 10px; border: 0; background: transparent; color: var(--fg-3); font-family: var(--mono); font-size: 11.5px; }
  .seg button.on { background: var(--accent-soft); color: var(--accent); }
  .think { width: 96px; height: 26px; }
  .earlier { align-self: center; height: 26px; padding: 0 12px; border-radius: 999px; font-size: 12px; color: var(--fg-3); background: var(--bg-3); border: 1px solid var(--line-1); }
  .earlier:hover { color: var(--fg-1); }
  .jump { align-self: center; flex: none; z-index: 4; height: 28px; padding: 0 12px; border-radius: 999px; font-size: 12px; background: var(--accent); color: var(--accent-fg); border: 0; box-shadow: var(--shadow-2, 0 6px 18px rgb(0 0 0 / .35)); }
  .chat { position: relative; }
  @media (max-width: 599px) {
    .m { max-width: 92%; }

  }
</style>
