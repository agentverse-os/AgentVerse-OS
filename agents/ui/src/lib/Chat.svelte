<script lang="ts">
  // Карточка сессии агента, представление Chat (ADR-006): журнал сессии из cloudos-agentd по WebSocket (сначала всё после
  // последнего известного seq, потом живое), сообщения, вызовы инструментов с результатами, карточки разрешений и вопросов
  // агента с ответом отсюда, композер с фото. Процесс живёт в workspace — окно и вкладку можно закрывать.
  import { onMount, tick } from 'svelte';
  import { byProvider, api, agents, type SessionInfo, type SessionEvent, type Scheduled , type GateModel, type Skill } from './api';
  import { actions, when, ui } from './state.svelte';
  import { t, tn } from './i18n';
  import Icon from './Icon.svelte';
  import { adapterFor, type Ctx } from './adapters';
  import { menu } from './menu.svelte';
  import { toast } from './toast.svelte';
  import type { Caps } from './api';
  import Md from './Md.svelte';
  let { project, id, onchange }: { project: string; id: string; onchange?: () => void } = $props();
  let events = $state<SessionEvent[]>([]);
  let info = $state<SessionInfo | null>(null);
  let conn = $state<'connecting' | 'live' | 'lost'>('connecting');
  let text = $state('');
  let images = $state<{ name: string; type: string; data: string; url: string }[]>([]);
  let error = $state('');
  // правка отправленного сообщения (композер в режиме правки), очередь и отложенные сообщения, история ввода, черновик
  let editing = $state<{ seq: number } | null>(null);
  const queued = $derived(info?.queued ?? []);
  let scheduled = $state<Scheduled[]>([]);
  async function loadScheduled() { try { scheduled = (await agents.scheduled(project, id)).scheduled.filter(s => s.status === 'pending'); } catch {} }
  const DRAFT = `agents.draft.${project}:${id}`;
  try { const d = localStorage.getItem(DRAFT); if (d) text = d; } catch {}
  $effect(() => { const v = text; try { if (v.trim()) localStorage.setItem(DRAFT, v); else localStorage.removeItem(DRAFT); } catch {} });
  const hist = $derived(events.filter(e => e.kind === 'user' && !e.parent_tool_use_id && !taskNote(e)).map(e => cleanUser(userText(e))).filter(Boolean));
  let histIdx = $state<number | null>(null);
  function startEdit(e: SessionEvent) { editing = { seq: e.seq }; text = cleanUser(userText(e)); tick().then(() => { autoGrow(); ta?.focus(); }); }
  function cancelEdit() { editing = null; text = ''; tick().then(autoGrow); }
  function forkAt(e: SessionEvent) { actions.fork({ project, kind: 'session', id }, e.seq); }
  async function unqueue(qid: string) { try { await api.sessionUnqueue(project, id, qid); await refresh(); } catch (e) { error = msg(e); } }
  const qtext = (q: { content: any }) => (Array.isArray(q.content) ? q.content.filter((b: any) => b.type === 'text').map((b: any) => b.text).join(' ') : String(q.content ?? ''));
  function schedMenu(el: HTMLElement) {
    const body = text.trim(); if (!body) return;
    const at = (secs: number) => Math.floor(Date.now() / 1000) + secs;
    const tomorrow = () => { const d = new Date(); d.setDate(d.getDate() + 1); d.setHours(9, 0, 0, 0); return Math.floor(d.getTime() / 1000); };
    menu.show(el, [
      { header: true, label: t('sched.btn') },
      { icon: 'calendar', label: t('sched.in15'), on: () => schedule(at(15 * 60)) }, { icon: 'calendar', label: t('sched.in1h'), on: () => schedule(at(3600)) }, { icon: 'calendar', label: t('sched.in8h'), on: () => schedule(at(8 * 3600)) },
      { icon: 'calendar', label: t('sched.tomorrow'), on: () => schedule(tomorrow()) },
      { icon: 'edit', label: t('sched.custom'), on: () => { const v = window.prompt(t('sched.prompt')); if (!v) return; const ts = Date.parse(v.trim().replace(' ', 'T')); if (!ts) { error = t('sched.bad'); return; } schedule(Math.floor(ts / 1000)); } },
    ]);
  }
  async function schedule(atTs: number) {
    const body = text.trim(); if (!body) return;
    try { const sm = await agents.scheduleCreate({ project, session: id, content: body, at: atTs }); text = ''; editing = null; tick().then(autoGrow); await loadScheduled(); toast(t('sched.created', { when: when(sm.at) })); } catch (e) { error = msg(e); }
  }
  async function unschedule(sid: string) { try { await agents.scheduleDelete(sid); await loadScheduled(); } catch (e) { error = msg(e); } }
  let log: HTMLDivElement;
  let lastSeq = 0;
  // адаптер провайдера (Claude Code / Codex): модели, усилие, режимы, команды и подписи инструментов — разные
  let caps = $state<Caps | null>(null);
  const adapter = $derived(adapterFor(info?.agent));
  let ta = $state<HTMLTextAreaElement | undefined>(undefined);
  let popup = $state<{ kind: 'cmd' | 'file'; items: { label: string; hint?: string; desc?: string; insert: string }[]; sel: number; start: number; end: number; loading?: boolean } | null>(null);
  let ctxProvider = $state<Ctx | null>(null);
  let now = $state(Date.now());
  let ctxTimer: ReturnType<typeof setTimeout> | undefined;
  // журнал: при открытии показываем конец сразу (реплей без промежуточных прокруток), «прилипаем» к низу только пока
  // читатель внизу; при чтении истории новые события считаются и предлагаются кнопкой «вниз»
  let stick = $state(true);
  let unread = $state(0);
  let replayLeft = 0;
  let replayDone = false;
  const WINDOW = 150;
  /** Сколько событий журнала грузится за раз (хвост); более старые — по кнопке «показать ещё» страницами */
  const HISTORY = 4000;
  let olderCount = $state(0);
  let showAll = $state(false);
  async function loadOlder() {
    const from = Math.max(0, olderCount - HISTORY); const before = log?.scrollHeight ?? 0;
    try { const older = await api.sessionEvents(project, id, from, olderCount); if (older.length) { events = [...older.filter(o => o.seq <= olderCount), ...events]; olderCount = from; showAll = true; await tick(); if (log) log.scrollTop += log.scrollHeight - before; } else olderCount = 0; } catch (e) { error = msg(e); }
  }
  // сообщения сабагентов (parent_tool_use_id) сворачиваются под карточку вызова Agent, в общую ленту не идут
  const top = $derived(events.filter(e => !e.parent_tool_use_id));
  const subs = $derived.by(() => { const m: Record<string, SessionEvent[]> = {}; for (const e of events) if (e.parent_tool_use_id && (e.kind === 'assistant' || e.kind === 'tool_result')) (m[e.parent_tool_use_id] ??= []).push(e); return m; });
  const visible = $derived(showAll || top.length <= WINDOW ? top : top.slice(top.length - WINDOW));
  const hiddenCount = $derived(top.length - visible.length);
  // переход к сообщению из поиска (срез 34): догрузить историю до seq, показать всё окно, прокрутить и подсветить
  async function focusSeq(seq: number) {
    for (let i = 0; i < 30; i++) { const first = events[0]?.seq ?? 0; if (first <= seq || olderCount === 0) break; await loadOlder(); }
    showAll = true; await tick(); await new Promise((r) => setTimeout(r, 80));
    const el = log?.querySelector(`[data-seq="${seq}"]`) as HTMLElement | null;
    if (el) { stick = false; el.scrollIntoView({ block: 'center' }); el.classList.add('hit'); setTimeout(() => el.classList.remove('hit'), 3000); }
  }
  $effect(() => { const f = ui.focusSeq; if (!f || f.id !== id) return; const seq = f.seq; const h = setTimeout(() => { ui.focusSeq = null; void focusSeq(seq); }, 300); return () => clearTimeout(h); });
  // фоновые задачи после хода: по событиям task_started … task_notification / task_updated с конечным статусом
  const runningTasks = $derived.by(() => { const m = new Map<string, SessionEvent>(); for (const e of events) { if (e.kind !== 'task') continue; if (e.subtype === 'task_started') m.set(e.task_id, e); else if (e.subtype === 'task_notification' || ['completed', 'failed', 'killed', 'stopped', 'cancelled', 'error'].includes(e.status)) m.delete(e.task_id); } return info?.alive ? [...m.values()] : []; });
  async function stopTask(taskId: string) { try { await api.sessionTaskStop(project, id, taskId); } catch (e) { error = msg(e); } }
  const taskIcon = (tt?: string | null) => (tt === 'local_agent' || tt === 'agent' ? 'claude' : tt === 'workflow' ? 'layers' : tt === 'monitor' ? 'eye' : 'terminal');
  const elapsed = (ts?: number) => { const s = Math.max(0, Math.floor((now - (ts ?? now)) / 1000)); return s < 60 ? `${s} s` : `${Math.floor(s / 60)} ${t('rt.min')}`; };
  /** Служебные сообщения, которые CLI подкладывает в беседу как реплики пользователя: <task-notification> после фоновой
   *  задачи (итог, вызовы инструментов, длительность) — своя строка вместо сырого XML; <system-reminder> — вырезается. */
  const userText = (e: SessionEvent) => (e.content ?? []).filter((b: any) => b.type === 'text').map((b: any) => String(b.text ?? '')).join('\n');
  const cleanUser = (s: string) => s.replace(/<system-reminder>[\s\S]*?<\/system-reminder>/g, '').trim();
  function taskNote(e: SessionEvent): { status: string; summary: string; result: string; meta: string } | null {
    const text = userText(e); if (!/^\s*<task-notification>/.test(text)) return null;
    const tag = (n: string) => text.match(new RegExp(`<${n}>([\\s\\S]*?)<\\/${n}>`))?.[1]?.trim() ?? '';
    const tools = tag('tool_uses'), ms = Number(tag('duration_ms') || 0);
    const meta = [tools ? `${tools} tools` : '', ms ? (ms >= 60000 ? `${Math.round(ms / 60000)} ${t('rt.min')}` : `${Math.round(ms / 1000)} s`) : ''].filter(Boolean).join(' · ');
    return { status: tag('status'), summary: tag('summary'), result: tag('result') || tag('output') || '', meta };
  }
  /** Подсказка правила Claude Code для запроса разрешения: Bash(git diff:*), Read… */
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
  function onLogScroll() { if (!log) return; const atBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 48; if (atBottom !== stick) stick = atBottom; if (atBottom) unread = 0; }
  async function showEarlier() { const before = log?.scrollHeight ?? 0; showAll = true; await tick(); if (log) log.scrollTop += log.scrollHeight - before; }
  function jumpDown() { stick = true; unread = 0; scroll(); }
  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));

  onMount(() => {
    let ws: WebSocket | null = null; let closed = false; let retry = 0;
    loadScheduled(); const schedT = setInterval(loadScheduled, 30000);
    // история — одним HTTP-запросом (тысячи событий за один рендер, без покадровой «промотки»), WebSocket — только живое
    (async () => {
      try {
        // у длинных бесед (тысячи событий) берём хвост журнала: с начала грузился бы лимит в 5000 и открывалась середина
        let from = 0;
        try { const r = await api.sessionGet(project, id); info = r.info; if (r.caps) caps = r.caps; from = Math.max(0, (r.info.last_seq ?? 0) - HISTORY); } catch {}
        const list = await api.sessionEvents(project, id, from);
        if (!closed && list.length) { events = list; lastSeq = list[list.length - 1].seq; olderCount = from; replayDone = true; await tick(); scroll(); }
      } catch {}
      connect();
    })();
    function connect() {
      if (closed) return;
      conn = 'connecting';
      ws = new WebSocket(api.sessionWsUrl(project, id, lastSeq));
      ws.onopen = () => { retry = 0; conn = 'live'; };
      ws.onmessage = (ev) => {
        let m: any; try { m = JSON.parse(ev.data); } catch { return; }
        if (m.kind === 'hello') { info = m.info; replayLeft = m.replay ?? 0; if (!replayLeft) { replayDone = true; scroll(); } refresh(); scheduleCtx(); return; }
        if (m.kind === 'pong') return;
        if (m.kind === 'lagged') { api.sessionEvents(project, id, lastSeq).then(add).catch(() => {}); return; }
        add([m]);
      };
      ws.onclose = () => { if (closed) return; conn = 'lost'; setTimeout(connect, Math.min(1000 * 2 ** retry++, 15000)); };
      ws.onerror = () => ws?.close();
    }
    function add(list: SessionEvent[]) {
      let changed = false, fresh = 0;
      // после правки нумерация журнала откатывается: отметку history_truncated принимаем с любым seq и режем ленту по ней
      for (const e of list) { if (e.seq > lastSeq || e.kind === 'history_truncated') { if (e.kind === 'history_truncated') { events = events.filter(x => x.seq <= e.at); } events.push(e); lastSeq = e.seq; if (e.kind === 'queued' || e.kind === 'unqueued') refreshSoon(); if (e.kind === 'user' && !e.parent_tool_use_id && scheduled.length) loadScheduled(); changed = true; if (replayLeft > 0) replayLeft--; else if (e.kind === 'task' || e.kind === 'user' || e.kind === 'assistant' || e.kind === 'history_truncated' || e.kind === 'permission_request') fresh++; } }
      if (!changed) return;
      // реплей истории: без обновлений состояния на каждое событие (иначе шторм запросов), одна прокрутка и одно
      // обновление по его окончании; живые события — прокрутка только если читатель внизу
      if (replayLeft > 0) return;
      if (replayLeft === 0 && !replayDone) { replayDone = true; refresh(); scheduleCtx(); scroll(); return; }
      if (list.some(e => e.kind === 'status' || e.kind === 'result' || e.kind === 'exit' || e.kind === 'init' || e.kind === 'initialized' || e.kind === 'model' || e.kind === 'effort' || e.kind === 'permission_mode')) refresh();
      if (list.some(e => e.kind === 'result' || e.kind === 'initialized')) scheduleCtx();
      if (stick) scroll(); else unread += fresh;
    }
    const ping = setInterval(() => { if (ws?.readyState === WebSocket.OPEN) ws.send('{"kind":"ping"}'); }, 25000);
    const tick10 = setInterval(() => (now = Date.now()), 10000);
    return () => { closed = true; clearInterval(schedT); clearInterval(ping); clearInterval(tick10); clearTimeout(ctxTimer); ws?.close(); };
  });
  let changeT: ReturnType<typeof setTimeout> | undefined;
  let refreshT: ReturnType<typeof setTimeout> | undefined;
  function refreshSoon() { clearTimeout(refreshT); refreshT = setTimeout(() => { refresh(); loadScheduled(); }, 300); }
  async function refresh() {
    try {
      const r = await api.sessionGet(project, id); info = r.info; if (r.caps) caps = r.caps; clearTimeout(changeT); changeT = setTimeout(() => onchange?.(), 700);
      // журнал стал короче (правка из другой вкладки или до подключения) — перечитать ленту целиком
      if ((r.info.last_seq ?? 0) < lastSeq) { const list = await api.sessionEvents(project, id, 0); events = list; lastSeq = list.length ? list[list.length - 1].seq : 0; olderCount = 0; await tick(); scroll(); }
    } catch {}
  }
  /** Точный контекст у Claude — control `get_context_usage` (summary: без вызовов API); зовём после хода, не чаще раза в 1.5 с. */
  function scheduleCtx() { clearTimeout(ctxTimer); ctxTimer = setTimeout(fetchCtx, 1500); }
  // долгий ход с десятками вызовов инструментов: чип контекста обновляется и по ходу, раз в 20 с
  $effect(() => { if (!working) return; const i = setInterval(fetchCtx, 20000); return () => clearInterval(i); });
  async function fetchCtx() {
    if (adapter.id !== 'claude' || !info?.alive) return;
    try {
      const r = await api.sessionControl(project, id, { subtype: 'get_context_usage', detail: 'summary' }); const c = r?.response ?? r;
      if (c?.maxTokens) ctxProvider = { used: c.totalTokens ?? 0, window: c.maxTokens, pct: c.percentage ?? Math.round(((c.totalTokens ?? 0) / c.maxTokens) * 100), source: 'provider', threshold: c.autoCompactThreshold ?? undefined, categories: (c.categories ?? []).filter((x: any) => x.kind === 'used').map((x: any) => ({ name: x.name, tokens: x.tokens })) } as Ctx;
    } catch {}
  }
  async function scroll() { await tick(); if (log) log.scrollTop = log.scrollHeight; }

  const status = $derived(info?.status ?? 'connecting');
  const pendingIds = $derived(new Set(events.filter(e => e.kind === 'permission_result').map(e => e.request_id)));
  async function send() {
    const body = text.trim(); if (!body && !images.length) return;
    popup = null; error = '';
    const m = !images.length ? body.match(/^\/([\w-]+)\s*([\s\S]*)$/) : null;
    if (m) { try { if (await runCommand(m[1], m[2].trim())) { text = ''; tick().then(autoGrow); return; } } catch (e) { error = msg(e); return; } }
    const content: unknown[] = [];
    for (const im of images) content.push({ type: 'image', source: { type: 'base64', media_type: im.type, data: im.data } });
    // навыки, ещё не переданные в этой сессии — блоком перед вопросом (один раз на сессию)
    const pendingSkills = editing ? [] : skillSel.filter((n) => !skillSent.includes(n));
    if (pendingSkills.length && !skillsAll.length) { try { skillsAll = (await agents.skills()).skills; } catch {} }
    const block = pendingSkills.length ? skillsBlock(pendingSkills) : '';
    const bodyText = block && body ? `${block}\n---\n${body}` : body;
    if (bodyText) content.push({ type: 'text', text: bodyText });
    if (block) { skillSent = [...skillSent, ...pendingSkills]; skillsPersist(); }
    try {
      if (editing) { const seq = editing.seq; editing = null; await api.sessionEdit(project, id, { seq, content }); text = ''; images = []; tick().then(autoGrow); await refresh(); return; }
      // ход идёт — в очередь agentd, уйдёт само после ответа (даже без открытого браузера)
      if (working && !images.length) { await api.sessionQueue(project, id, content); text = ''; tick().then(autoGrow); await refresh(); return; }
      // процесс не жив (завершён, упал или только что отключили TUI и статус ещё не обновился) — сначала возобновить
      if (status === 'exited' || status === 'error' || info?.alive === false) {
        await api.sessionCreate(project, { resume: id });
        for (let i = 0; i < 20; i++) { await refresh(); if (info?.alive) break; await new Promise(r => setTimeout(r, 500)); }
      }
      await api.sessionPrompt(project, id, content); text = ''; images = []; error = ''; tick().then(autoGrow);
    } catch (e) { error = msg(e); }
  }
  function attach(e: Event) {
    const files = Array.from((e.target as HTMLInputElement).files ?? []);
    for (const f of files) {
      const r = new FileReader();
      r.onload = () => { const url = String(r.result); images.push({ name: f.name, type: f.type || 'image/jpeg', data: url.split(',')[1] ?? '', url }); };
      r.readAsDataURL(f);
    }
    (e.target as HTMLInputElement).value = '';
  }
  async function answer(req: SessionEvent, behavior: 'allow' | 'deny', extra: Record<string, unknown> = {}) {
    try { await api.sessionPermission(project, id, { request_id: req.request_id, behavior, ...extra }); } catch (e) { error = msg(e); }
  }
  // ответы на AskUserQuestion: выбранные варианты по вопросу (multiSelect — несколько) или свой текст
  let picks = $state<Record<string, Record<string, string[]>>>({});
  let custom = $state<Record<string, Record<string, string>>>({});
  function pick(rid: string, q: string, label: string, multi: boolean) {
    const cur = picks[rid]?.[q] ?? [];
    const next = multi ? (cur.includes(label) ? cur.filter(l => l !== label) : [...cur, label]) : [label];
    picks[rid] = { ...(picks[rid] ?? {}), [q]: next };
  }
  function submitAnswers(req: SessionEvent) {
    const answers: Record<string, string> = {};
    for (const q of req.input?.questions ?? []) {
      const own = custom[req.request_id]?.[q.question]?.trim();
      answers[q.question] = own || (picks[req.request_id]?.[q.question] ?? []).join(', ');
    }
    answer(req, 'allow', { answers });
  }
  async function interrupt() { try { await api.sessionInterrupt(project, id); } catch (e) { error = msg(e); } }
  async function resume() { try { await api.sessionCreate(project, { resume: id }); await refresh(); } catch (e) { error = msg(e); } }
  function summary(name: string, input: any): string { return adapter.toolSummary(name, input); }
  function resultText(content: any): string {
    if (typeof content === 'string') return content;
    if (Array.isArray(content)) return content.map((b: any) => (b.type === 'text' ? b.text : b.type === 'tool_result' ? resultText(b.content) : '')).join('\n');
    return content ? JSON.stringify(content) : '';
  }
  const toolResults = $derived.by(() => {
    const m: Record<string, { text: string; error: boolean }> = {};
    for (const e of events) if (e.kind === 'tool_result') for (const b of e.content ?? []) if (b.type === 'tool_result') m[b.tool_use_id] = { text: resultText(b.content), error: !!b.is_error };
    return m;
  });
  function fmtTokens(n?: number) { if (n == null) return '—'; if (n >= 1e6) return (n / 1e6).toFixed(1).replace(/\.0$/, '') + 'M'; if (n >= 1000) return (n / 1000).toFixed(n >= 10000 ? 0 : 1).replace(/\.0$/, '') + 'k'; return String(n); }
  // модель, усилие и режим — меню в композере (как у CloudCLI); подписи и списки даёт адаптер, факты — caps из agentd
  const working = $derived(status === 'working' || status === 'starting');
  const modeValue = $derived(info?.permission_mode ?? caps?.current_mode ?? (info?.agent === 'codex' ? 'on-request' : 'default'));
  async function setModel(v: string | null, ctxWindow?: number | null) { if ((v || null) === (info?.model ?? null)) return; try { await api.sessionModel(project, id, v || null, ctxWindow ?? null); await refresh(); } catch (e) { error = msg(e); } }
  async function setMode(v: string) { try { await api.sessionMode(project, id, v); await refresh(); } catch (e) { error = msg(e); } }
  async function setEffort(v: string | null) { try { await api.sessionEffort(project, id, v); await refresh(); } catch (e) { error = msg(e); } }
  // модели gate (провайдера llm проекта): группа в меню моделей — любая из них принимается CLI как id модели
  let gateModels = $state<{ app: string; title: string; models: GateModel[] }[]>([]); let gateAt = 0;
  async function loadGateModels() { if (Date.now() - gateAt < 60000) return; gateAt = Date.now(); try { gateModels = (await api.gateModels(project)).targets; } catch {} }
  $effect(() => { if (info) loadGateModels(); });
  const gateItems = (cur: string | null) => {
    const all = gateModels.flatMap(tg => tg.models);
    const groups = byProvider(all);
    if (!groups.length) return [];
    const win = (n?: number | null) => (n ? (n >= 1e6 ? `${Math.round(n / 1e5) / 10}M` : `${Math.round(n / 1000)}k`) : '');
    // модели gate по каналам учёток (Claude Code, Codex, Antigravity, Devin…); id настоящий — Claude Code и Codex принимают его через gate
    return [{ header: true, label: t('cmp.gateModels') }, ...groups.flatMap(g => [{ sub: true, label: `${g.title} · ${g.models.length}` },
      ...g.models.map(m => { const label = m.name && m.name !== m.id ? m.name : m.id; return { label, hint: [label !== m.id ? m.id : '', win(m.context)].filter(Boolean).join(' · ') || undefined, checked: cur === m.id, on: () => setModel(m.id, m.context ?? null) }; })])];
  };
  function modelMenu(el: HTMLElement) {
    const cur = info?.model ?? null;
    if (adapter.id === 'codex') {
      menu.show(el, [{ header: true, label: t('cmp.selectModel') }, { label: 'Default', hint: t('cmp.modelDefaultSet'), checked: !cur, on: () => setModel(null) },
        ...adapter.models(caps).filter(m => m.value).map(m => ({ label: m.label, hint: m.desc, checked: cur === m.value, on: () => setModel(m.value) })), ...gateItems(cur)], { search: true, placeholder: t('chat.modelSearch') });
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
  // контекст: у Claude — get_context_usage, иначе оценка адаптера по журналу; кэш промпта — из последнего result
  const ctx = $derived<Ctx | null>(ctxProvider ?? adapter.context(caps, events, info?.model));
  // попап «Режим и усилие» (как ⇧+Tab в Claude Code): режимы с описанием и ползунок усилия
  let modesOpen = $state(false);
  // навыки в беседе (срез 36, §4.3.10): выбор на сессию в браузере; тексты ещё не переданных уходят блоком перед сообщением
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
  /** Блок навыков как у рутин (routines.rs): заголовок, затем «## имя — описание» и тело; после «---» — сам вопрос. */
  function skillsBlock(names: string[]): string {
    const parts = names.map((n) => skillsAll.find((x) => x.name === n)).filter((x): x is Skill => !!x).map((x) => `## ${x.name} — ${x.description.trim()}\n${x.body.trim()}\n`);
    return parts.length ? `${SKILLS_HEAD}\n${parts.join('')}` : '';
  }
  /** Сообщение владельца с блоком навыков — в ленте свёрнуто: имена навыков и сам вопрос. */
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
  async function applyThinking() { const n = parseInt(thinkingInput, 10); try { await api.sessionThinking(project, id, Number.isFinite(n) && n > 0 ? n : null); await refresh(); } catch (e) { error = msg(e); } }
  async function toggleFast() { try { await api.sessionTier(project, id, info?.service_tier === 'fast' ? null : 'fast'); await refresh(); } catch (e) { error = msg(e); } }
  function ctxMenu(el: HTMLElement) {
    if (!ctx) return;
    const cats = (ctx as any).categories as { name: string; tokens: number }[] | undefined;
    menu.show(el, [{ header: true, label: t('cmp.contextDetails', { used: fmtTokens(ctx.used), window: fmtTokens(ctx.window), pct: ctx.pct }) + (ctx.threshold ? ` · ${t('cmp.autoCompactAt', { n: fmtTokens(ctx.threshold) })}` : '') },
      ...(cats ?? []).map(c => ({ label: c.name, hint: fmtTokens(c.tokens), on: () => {} })),
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
  // слэш-команды и «@» файлы — попап над полем ввода; /model /effort /mode перехватываются и идут через API сессии
  function autoGrow() { if (!ta) return; ta.style.height = 'auto'; ta.style.height = Math.min(ta.scrollHeight, 180) + 'px'; }
  function onInput() { histIdx = null; autoGrow(); updatePopup(); }
  function updatePopup() {
    const pos = ta?.selectionStart ?? text.length; const before = text.slice(0, pos);
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
    text = text.slice(0, popup.start) + it.insert + text.slice(popup.end); popup = null;
    tick().then(() => { ta?.focus(); ta?.setSelectionRange(caret, caret); autoGrow(); updatePopup(); });
  }
  function onKey(e: KeyboardEvent) {
    if (popup) {
      if (e.key === 'ArrowDown') { e.preventDefault(); popup.sel = (popup.sel + 1) % Math.max(1, popup.items.length); return; }
      if (e.key === 'ArrowUp') { e.preventDefault(); popup.sel = (popup.sel - 1 + popup.items.length) % Math.max(1, popup.items.length); return; }
      if ((e.key === 'Enter' || e.key === 'Tab') && popup.items.length) { e.preventDefault(); pickPopup(popup.items[popup.sel]); return; }
      if (e.key === 'Escape') { e.preventDefault(); popup = null; return; }
    }
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); }
    // ↑/↓ в пустом поле листают свои прошлые сообщения; Esc прерывает ход или отменяет правку; Tab переключает режим разрешений
    if ((e.key === 'ArrowUp' || e.key === 'ArrowDown') && hist.length && (histIdx != null || (!text.trim() && e.key === 'ArrowUp'))) {
      e.preventDefault();
      const next = e.key === 'ArrowUp' ? (histIdx == null ? hist.length - 1 : Math.max(0, histIdx - 1)) : (histIdx == null ? null : histIdx + 1);
      if (next == null || next >= hist.length) { histIdx = null; text = ''; } else { histIdx = next; text = hist[next]; }
      tick().then(autoGrow); return;
    }
    if (e.key === 'Escape') { e.preventDefault(); if (editing) cancelEdit(); else if (working) interrupt(); return; }
    if (e.key === 'Tab' && !e.shiftKey) { const modes = adapter.modes(caps).filter(m => !m.hidden && !(m.value === 'bypassPermissions' && caps?.bypass_allowed === false)).map(m => m.value); if (modes.length > 1) { e.preventDefault(); setMode(modes[(Math.max(0, modes.indexOf(modeValue)) + 1) % modes.length]); } }
  }
  /** Команда из строки: локальные (модель, усилие, режим) — через API сессии; остальные у Claude — самому CLI (/compact, /context, /cost…). */
  async function runCommand(name: string, arg: string): Promise<boolean> {
    const cmd = adapter.commands(caps).find(c => c.name === name);
    if (cmd?.local) {
      if (name === 'model') { await setModel(arg || null); return true; }
      if (name === 'effort') { if (arg && !adapter.effortsAll.includes(arg)) { error = t('cmp.badArg', { v: arg }); return true; } await setEffort(arg === 'auto' ? null : arg || null); return true; }
      if (name === 'mode') { if (!adapter.modes(caps).some(m => m.value === arg)) { error = t('cmp.badArg', { v: arg }); return true; } await setMode(arg); return true; }
    }
    if (!cmd && adapter.id === 'codex') { error = t('cmp.unknownCmd', { agent: adapter.name, cmd: '/' + name }); return true; }
    return false;
  }
  function sendText(s: string) { text = s; send(); }
</script>

<svelte:window onclick={() => { modesOpen = false; effortOpen = false; skillsOpen = false; }} onkeydown={(e) => { if (e.key === 'Escape') { modesOpen = false; effortOpen = false; skillsOpen = false; } }} />
<div class="chat">
  {#if runningTasks.length}
    <div class="tasks" title={t('task.strip')}><span class="muted xs">⏳ {t('task.strip')}</span>{#each runningTasks as tk (tk.task_id)}<span class="chip task" title={tk.description ?? ''}><Icon name={taskIcon(tk.task_type)} size="s" /> <span class="ell">{tk.description || tk.task_type || tk.task_id}</span> <span class="muted">{elapsed(tk.ts)}</span><button class="x" title={t('task.stop')} onclick={() => stopTask(tk.task_id)}>×</button></span>{/each}</div>
  {/if}
  <div class="log" bind:this={log} onscroll={onLogScroll}>
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
          {#if b.type === 'text' && b.text?.trim()}<div class="m agent" data-seq={e.seq} class:wide={/(^|\n)\s*(\||```|#{1,4} |[-*] |\d+\. )/.test(b.text)}><Md text={b.text} /></div>
          {:else if b.type === 'thinking' && b.thinking}<details class="m think"><summary>💭 {t('agents.thinking')}</summary><div class="txt muted">{b.thinking}</div></details>
          {:else if b.type === 'tool_use'}
            <details class="m tool" class:err={toolResults[b.id]?.error}><summary><b>{adapter.toolLabel(b.name)}</b> <span class="muted mono">{summary(b.name, b.input)}</span>{#if toolResults[b.id]} <span class="badge {toolResults[b.id].error ? 'bad' : 'ok'}">{toolResults[b.id].error ? t('agents.toolError') : t('agents.toolDone')}</span>{:else if working} <span class="badge run"><span class="spin"></span>{t('cmp.running')}</span>{/if}</summary>
              <pre class="mono">{JSON.stringify(b.input, null, 1)}</pre>{#if toolResults[b.id]}<pre class="mono res">{toolResults[b.id].text.slice(0, 4000)}</pre>{/if}
              {#if subs[b.id]?.length}<details class="sub"><summary>🤖 {t('sub.title')} · {tn('sub.messages', subs[b.id].length)}</summary>{#each subs[b.id] as se (se.seq)}{#if se.kind === 'assistant'}{#each se.content ?? [] as sb}{#if sb.type === 'text' && sb.text?.trim()}<div class="sm"><Md text={sb.text} /></div>{:else if sb.type === 'tool_use'}<div class="muted mono xs">▸ {adapter.toolLabel(sb.name)} {summary(sb.name, sb.input)}</div>{/if}{/each}{/if}{/each}</details>{/if}</details>
          {/if}
        {/each}
      {:else if e.kind === 'permission_request'}
        <div class="m perm" class:done={pendingIds.has(e.request_id)}>
          {#if e.question}
            <div class="ph">❓ {t('agents.question')}</div>
            {#each e.input?.questions ?? [] as q}
              <div class="q"><b>{q.header ? q.header + ': ' : ''}</b>{q.question}</div>
              {#if !pendingIds.has(e.request_id)}
                <div class="opts">{#each q.options ?? [] as o}<button class="mini" class:active={(picks[e.request_id]?.[q.question] ?? []).includes(o.label)} title={o.description} onclick={() => pick(e.request_id, q.question, o.label, !!q.multiSelect)}>{o.label}</button>{/each}</div>
                <input class="own" placeholder={t('agents.ownAnswer')} value={custom[e.request_id]?.[q.question] ?? ''} oninput={(ev) => { custom[e.request_id] = { ...(custom[e.request_id] ?? {}), [q.question]: (ev.target as HTMLInputElement).value }; }} />
              {/if}
            {/each}
            {#if !pendingIds.has(e.request_id)}<div class="row" style="gap:6px; margin-top:6px;"><button class="primary mini" onclick={() => submitAnswers(e)}>{t('agents.answer')}</button><button class="mini" onclick={() => answer(e, 'deny', { message: t('inbox.declined') })}>{t('agents.skip')}</button></div>{/if}
          {:else}
            <div class="ph"><Icon name="shield" size="s" /> {t('agents.permission')}: <b>{e.tool_name}</b>{#if ruleOf(e)}<span class="chip mono rule" title={t('perm.rule')}>{ruleOf(e)}</span>{/if}</div>
            {#if e.title || e.description}<div class="muted">{e.title ?? ''} {e.description ?? ''}</div>{/if}
            <pre class="mono">{summary(e.tool_name, e.input) || JSON.stringify(e.input, null, 1)}</pre>
            {#if e.decision_reason}<div class="muted sm">{e.decision_reason}</div>{/if}
            {#if !pendingIds.has(e.request_id)}
              <div class="row" style="gap:6px; margin-top:6px;">
                <button class="primary mini" onclick={() => answer(e, 'allow')}>{t('agents.allow')}</button>
                {#if ruleOf(e)}<button class="mini" onclick={(ev) => { ev.stopPropagation(); rememberMenu(ev.currentTarget as HTMLElement, e); }}>{t('perm.remember')} ▾</button>{:else if e.provider !== 'acp' || (e.options ?? []).some((o: any) => o.kind === 'allow_always')}<button class="mini" onclick={() => answer(e, 'allow', { remember: true })}>{t('agents.allowAlways')}</button>{/if}
                <button class="mini danger" onclick={() => answer(e, 'deny')}>{t('agents.deny')}</button>
              </div>
            {/if}
          {/if}
        </div>
      {:else if e.kind === 'permission_result'}
        <div class="m sys muted">{e.behavior === 'allow' ? '✅' : '⛔'} {e.tool_name} · {e.behavior === 'allow' ? t('agents.allowed') : t('agents.denied')}{e.auto ? ' · ' + t('perm.auto') : e.remember ? ' · ' + t('agents.remembered') + (e.rules?.length ? ': ' + e.rules.join(', ') + ' (' + scopeLabel(e.scope) + ')' : '') : ''}</div>
      {:else if e.kind === 'history_truncated'}
        <div class="m sys muted cut">✂ {t('edit.cut')}</div>
      {:else if e.kind === 'forked'}
        <div class="m sys muted">⑂ {t('fork.row', { from: (e.from ?? '').slice(0, 8) })}</div>
      {:else if e.kind === 'task'}
        {#if e.subtype === 'task_started'}<div class="m sys muted task">⏳ {t('task.started')}: {e.description || e.task_type || e.task_id}</div>
        {:else if e.subtype === 'task_notification' || ['completed', 'failed', 'killed', 'stopped', 'cancelled', 'error'].includes(e.status)}<div class="m sys muted task">{e.status === 'completed' ? '✅ ' + t('task.done') : ['killed', 'stopped', 'cancelled'].includes(e.status) ? '⏹ ' + t('task.stopped') : '⚠️ ' + t('task.failed')}{e.summary ? ': ' + e.summary : ''}{#if e.output_file} · <span class="mono xs">{t('task.output')}: {e.output_file}</span>{/if}</div>{/if}
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
    {#if working}<div class="m agent typing" title={t('cmp.typing')}><span></span><span></span><span></span></div>{/if}
    {#if !events.length}<div class="muted" style="padding: 12px;">{conn === 'live' ? t('agents.emptyChat') : t('agents.connecting')}</div>{/if}
  </div>
  {#if !stick && unread > 0}<button class="jump" onclick={jumpDown}>↓ {tn('chat.unread', unread)}</button>{/if}
  {#if error}<div class="muted" style="color: var(--bad);">{error}</div>{/if}
  <div class="composer" class:busy={working}>
    {#if images.length}<div class="thumbs">{#each images as im, i}<span class="thumb"><img src={im.url} alt={im.name} /><button class="x" onclick={() => images.splice(i, 1)}>×</button></span>{/each}</div>{/if}
    {#if editing}<div class="editbar"><Icon name="edit" size="s" /> {t('edit.banner', { n: editing.seq })}<span class="sp"></span><button class="xs ghost" onclick={cancelEdit}>{t('rt.cancel')}</button></div>{/if}
    {#each queued as q (q.id)}<div class="qcard"><span class="muted xs">⏳ {t('queue.label')}</span><button class="txt ell asl" title={t('queue.edit')} onclick={() => { const v = qtext(q); unqueue(q.id).then(() => { text = v; tick().then(() => { autoGrow(); ta?.focus(); }); }); }}>{qtext(q)}</button><button class="x" title={t('rt.cancel')} onclick={() => unqueue(q.id)}>×</button></div>{/each}
    {#each scheduled as sm (sm.id)}<div class="qcard sched"><span class="muted xs">🕒 {t('sched.label')} · {when(sm.at)}</span><span class="txt ell">{sm.content}</span><button class="x" title={t('rt.cancel')} onclick={() => unschedule(sm.id)}>×</button></div>{/each}
    {#if popup}
      <div class="popup" role="listbox">
        <div class="pk muted">{popup.kind === 'cmd' ? t('cmp.commands') : t('cmp.files')}{#if popup.loading} · {t('common.loading')}{/if}</div>
        {#each popup.items as it, i (it.insert)}<button class="pi" class:sel={i === popup.sel} role="option" aria-selected={i === popup.sel} onmousedown={(e) => e.preventDefault()} onclick={() => pickPopup(it)}><span class="pn mono">{it.label}</span>{#if it.hint}<span class="ph mono muted">{it.hint}</span>{/if}{#if it.desc}<span class="pd muted">{it.desc}</span>{/if}</button>{/each}
        {#if !popup.items.length && !popup.loading}<div class="pk muted">{t('cmp.noMatches')}</div>{/if}
      </div>
    {/if}
    <div class="in">
      <label class="attach" title={t('agents.attach')}><Icon name="clip" /><input type="file" accept="image/*" multiple onchange={attach} /></label>
      <textarea bind:this={ta} rows="1" bind:value={text} oninput={onInput} onkeydown={onKey} onclick={updatePopup} placeholder={status === 'exited' ? (info?.tui ? t('agents.tuiHint') : t('agents.exitedHint')) : status === 'waiting' ? t('agents.waitingHint') : t('cmp.placeholderShort')}></textarea>
      {#if working}<button class="icon stop" onclick={interrupt} title={t('agents.stopTurn')}><Icon name="square" size="s" /></button>{/if}
      {#if !editing}<button class="icon sched" title={t('sched.btn')} disabled={!text.trim()} onclick={(e) => { e.stopPropagation(); schedMenu(e.currentTarget as HTMLElement); }}><Icon name="calendar" size="s" /></button>{/if}
      <button class="icon send" class:primary={!!text.trim() || !!images.length} onclick={send} disabled={!text.trim() && !images.length} title={editing ? t('edit.title') : working ? t('queue.hint') : t('cmp.sendHint')}><Icon name={editing ? 'edit' : 'send'} size="s" /></button>
    </div>
    {#if effortOpen && adapter.id === 'codex'}
      <div class="effpop" role="dialog" aria-label="Effort" onclick={(e) => e.stopPropagation()}>
        <div class="ep-h"><Icon name="zap" size="s" /><span class="sp"></span><span class="ep-cur"><b>{info?.effort ?? caps?.default_effort ?? 'auto'}</b><span class="muted">{adapter.modelLabel(info?.model, caps)}</span></span><span class="sp"></span><button class="icon xs ghost" title={t('cmp.effortReset')} onclick={() => setEffort(null)}><Icon name="refresh" size="s" /></button></div>
        <input class="range" type="range" min="0" max={Math.max(0, effortSteps.length - 1)} step="1" value={effortIdx >= 0 ? effortIdx : Math.max(0, effortSteps.indexOf(caps?.default_effort ?? ''))} style="--p: {effortSteps.length > 1 ? Math.round(((effortIdx >= 0 ? effortIdx : Math.max(0, effortSteps.indexOf(caps?.default_effort ?? ''))) / (effortSteps.length - 1)) * 100) : 100}%" oninput={(e) => setEffort(effortSteps[Number((e.target as HTMLInputElement).value)])} />
        <div class="ep-steps">{#each effortSteps as e, i (e)}<button class="ep-step" class:on={(info?.effort ?? caps?.default_effort) === e} onclick={() => setEffort(e)}>{e}</button>{/each}</div>
        <div class="muted xs ep-hint">{adapter.efforts(caps).find(e => e.value === (info?.effort ?? caps?.default_effort))?.hint ?? ''}</div>
        {#if caps?.tiers?.length}
          {#each caps.tiers as tr (tr.id)}<label class="ep-tier"><button class="switch" class:on={info?.service_tier === tr.id} aria-pressed={info?.service_tier === tr.id} onclick={() => api.sessionTier(project, id, info?.service_tier === tr.id ? null : tr.id).then(refresh).catch((e) => (error = msg(e)))}></button><span><b>{tr.name}</b><span class="muted xs">{tr.description ?? ''}</span></span></label>{/each}
        {/if}
      </div>
    {/if}
    {#if skillsOpen}
      <div class="modes skills-pop" role="dialog" aria-label={t('cmp.skills')} onclick={(e) => e.stopPropagation()}>
        <div class="mh"><span>{t('cmp.skills')}</span><span class="sp"></span><button class="icon xs ghost" onclick={() => (skillsOpen = false)}><Icon name="x" size="s" /></button></div>
        {#each skillsAll as sk (sk.name)}
          <label class="mode sk" data-skill={sk.name}><input type="checkbox" checked={skillSel.includes(sk.name)} onchange={() => toggleSkill(sk.name)} /><span class="mi-t"><b>{sk.name}</b><span class="muted">{sk.description}</span></span>{#if skillSent.includes(sk.name)}<span class="muted xs">{t('cmp.skillSent')}</span>{/if}</label>
        {:else}<div class="muted sm" style="padding: 8px 10px">{t('cmp.skillsEmpty')}</div>{/each}
        <div class="muted xs" style="padding: 6px 10px 8px">{t('cmp.skillsHint')}</div>
      </div>
    {/if}
    {#if modesOpen}
      <div class="modes" role="dialog" aria-label={t('cmp.modesTitle')} onclick={(e) => e.stopPropagation()}>
        <div class="mh"><span>{t('cmp.modesTitle')}</span><span class="sp"></span><span class="muted xs">{adapter.name}</span><button class="icon xs ghost" onclick={() => (modesOpen = false)}><Icon name="x" size="s" /></button></div>
        {#each adapter.modes(caps).filter(m => !m.hidden) as m (m.value)}
          {@const blocked = m.value === 'bypassPermissions' && caps?.bypass_allowed === false}
          <button class="mode" class:on={modeValue === m.value} class:dis={blocked} disabled={blocked} onclick={() => { setMode(m.value); }}>
            <span class="mi-ic"><Icon name={m.value === 'plan' ? 'eye' : m.value === 'auto' ? 'zap' : m.value === 'bypassPermissions' ? 'alert' : m.value === 'acceptEdits' ? 'edit' : m.value === 'never' ? 'zap' : 'shield'} size="s" /></span>
            <span class="mi-t"><b>{m.label}</b><span class="muted">{blocked ? t('cmp.bypassBlocked') : m.hint}</span></span>
            {#if modeValue === m.value}<Icon name="check" size="s" />{/if}
          </button>
        {/each}
        {#if adapter.id === 'claude'}
          <!-- Claude Code: одна строка Effort со слайдером, как в его пикере (⇧+Tab); верхняя ступень — ultracode -->
          <div class="eff">
            <span class="mi-ic"><Icon name="zap" size="s" /></span>
            <span class="mi-t"><b>Effort <span class="muted">({info?.effort ? `${info.effort}${adapter.efforts(caps).find(e => e.value === info?.effort)?.hint ? ' — ' + adapter.efforts(caps).find(e => e.value === info?.effort)?.hint : ''}` : 'auto'})</span></b></span>
            <span class="slider mini" role="group" aria-label="Effort">
              {#each effortSteps as e, i (e)}<button class="dot" class:on={effortIdx === i} class:past={effortIdx >= 0 && i < effortIdx} title={e} onclick={() => setEffort(e)}></button>{/each}
            </span>
            {#if info?.effort}<button class="icon xs ghost" title={t('cmp.effortReset')} onclick={() => setEffort(null)}><Icon name="refresh" size="s" /></button>{/if}
          </div>
          <div class="eff sub">
            <span class="mi-ic"><Icon name="refresh" size="s" /></span>
            <span class="mi-t"><b>{t('cmp.cacheTtl')} <span class="muted">({info?.cache_ttl ?? 'auto'})</span></b><span class="muted">{t('cmp.cacheTtlHint')}</span></span>
            <span class="seg" role="group">
              {#each [['auto', 'auto'], ['5m', '5m'], ['1h', '1h']] as [v, l] (v)}<button class:on={(info?.cache_ttl ?? 'auto') === v} onclick={() => api.sessionCache(project, id, v === 'auto' ? null : v).then(refresh).catch((e) => (error = msg(e)))}>{l}</button>{/each}
            </span>
          </div>
        {:else}
          <!-- Codex: попап режимов одобрений; усилие и tier — в своём попапе (кнопка усилия) -->
        {/if}
      </div>
    {/if}
    <div class="tools">
      <span class="st" title={conn === 'lost' ? t('agents.reconnecting') : ''}><span class="dot {working ? 'working' : status === 'waiting' ? 'waiting' : status === 'idle' ? 'ok' : ''}"></span> {working ? t('cmp.typing') : status === 'waiting' ? t('agents.waiting') : status === 'idle' ? t('agents.idle') : status === 'exited' ? t('st.exited') : status === 'error' ? t('st.failed') : conn === 'lost' ? t('agents.reconnecting') : t('agents.connecting')}</span>
      <button class="ctl" title={t('cmp.modelHint')} onclick={(e) => { e.stopPropagation(); modelMenu(e.currentTarget as HTMLElement); }}><Icon name={adapter.icon} size="s" /><span class="lbl">{adapter.modelLabel(info?.model, caps)}{#if adapter.id === 'codex' && (info?.effort || caps?.default_effort)} <em class="acc">{info?.effort ?? caps?.default_effort}</em>{/if}</span></button>
      <button class="ctl" title={t('cmp.modesHint')} onclick={(e) => { e.stopPropagation(); effortOpen = false; modesOpen = !modesOpen; }}><Icon name="shield" size="s" /><span class="lbl">{adapter.modeLabel(modeValue, caps)}</span></button>
      {#if caps?.features?.effort ?? true}<button class="ctl" title={t('cmp.effortHint')} onclick={(e) => { e.stopPropagation(); if (adapter.id === 'codex') { modesOpen = false; effortOpen = !effortOpen; } else { effortOpen = false; modesOpen = !modesOpen; } }}><Icon name="zap" size="s" /><span class="lbl">{adapter.id === 'codex' && !info?.effort ? t('cmp.selectEffort') : adapter.effortLabel(info?.effort)}</span></button>{/if}
      {#if caps?.mcp !== undefined && caps?.mcp !== null}<button class="ctl mcp" title={t('cmp.mcpHint')} onclick={(e) => { e.stopPropagation(); ui.openMcp(project); }}><Icon name="cpu" size="s" /><span class="lbl">MCP · {caps.mcp}</span></button>{/if}
      <button class="ctl skills" class:on={skillSel.length > 0} title={t('cmp.skillsHint')} onclick={(e) => { e.stopPropagation(); skillsOpenToggle(); }}><Icon name="sparkles" size="s" /><span class="lbl">{t('cmp.skills')}{skillSel.length ? ` · ${skillSel.length}` : ''}</span></button>
      <span class="sp"></span>
      {#if ctx}<button class="ctl ctx" class:warn={ctx.threshold ? ctx.used >= ctx.threshold * 0.9 : ctx.pct >= 80} title={t('cmp.contextHint')} onclick={(e) => { e.stopPropagation(); ctxMenu(e.currentTarget as HTMLElement); }}><span class="ring" style="--p: {Math.min(100, ctx.pct)}"></span><span class="lbl num">{ctx.pct}% · {fmtTokens(ctx.used)}/{fmtTokens(ctx.window)}</span></button>{/if}
      {#if cache}<span class="chip num" title={t('cmp.cacheHint')}><Icon name="refresh" size="s" /> {t('cmp.cache')}<span class="hide-p"> {fmtTokens(cache.read)}↺{#if cache.write} · +{fmtTokens(cache.write)}{/if}</span>{#if cache.ttl} · {cache.oneH ? t('cmp.cache1h') : t('cmp.cache5m')} · {cacheLeft > 0 ? mmss(cacheLeft) : t('cmp.cacheExpired')}{/if}</span>{/if}
    </div>
  </div>
</div>

<style>
  .chat { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 6px; }
  .log { flex: 1; min-height: 0; overflow: auto; display: flex; flex-direction: column; gap: 10px; padding: 4px 2px; }
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
  /* flex-shrink 0: с overflow hidden блок иначе сжимается до полоски, когда журнал короткой карточки переполнен */
  .m.perm { align-self: stretch; flex-shrink: 0; max-width: 100%; border-color: var(--line-2); position: relative; overflow: hidden; padding-left: 15px; }
  .m.perm::before { content: ""; position: absolute; left: 0; top: 0; bottom: 0; width: 3px; background: var(--warn); }
  .m.perm.done { opacity: .55; } .m.perm.done::before { background: var(--fg-3); }
  .ph { font-weight: 600; margin-bottom: 6px; display: flex; align-items: center; gap: 6px; }
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
  .composer { border: 1px solid var(--line-2); border-radius: var(--r-3); background: var(--bg-4); padding: 6px 8px 6px; display: flex; flex-direction: column; gap: 4px; transition: border-color var(--dur), box-shadow var(--dur); }
  .composer:focus-within { border-color: rgb(var(--accent-rgb) / .6); box-shadow: 0 0 0 3px rgb(var(--accent-rgb) / .14); }
  .composer textarea { flex: 1; border: 0; background: transparent; padding: 4px 6px; min-height: 34px; max-height: 160px; resize: none; }
  .composer textarea:focus { box-shadow: none; }
  .composer .row > button { height: 30px; }
  .composer .row > button.primary { border-radius: 999px; padding: 0 14px; }
  .attach { cursor: pointer; width: 28px; height: 28px; display: grid; place-items: center; border-radius: var(--r-1); color: var(--fg-2); flex: none; }
  .attach:hover { background: var(--bg-3); color: var(--fg-1); }
  .attach input { display: none; }
  .thumbs { display: flex; gap: 6px; flex-wrap: wrap; }
  .thumb { position: relative; }
  .thumb img { width: 56px; height: 56px; object-fit: cover; border-radius: 8px; border: 1px solid var(--line-2); }
  .thumb .x { position: absolute; top: -6px; right: -6px; width: 20px; height: 20px; padding: 0; border-radius: 50%; font-size: 12px; line-height: 1; }
  .sm { font-size: 12px; }
  .tools { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; font-size: 12px; color: var(--fg-3); }
  .sp { flex: 1; }
  .ctl { display: inline-flex; align-items: center; gap: 4px; }
  .ctl input, .ctl select { height: 26px; font-size: 12px; padding: 0 6px; background: transparent; border-color: transparent; color: var(--fg-1); border-radius: 999px; box-shadow: none; }
  .ctl input:hover, .ctl select:hover, .ctl input:focus, .ctl select:focus { border-color: var(--line-1); background: var(--bg-3); }
  .ctl input.model { width: 160px; font-family: var(--mono); }
  .usage { white-space: nowrap; }
  /* композер v2: попап команд и файлов, круглая отправка, панель инструментов с меню, кольцо контекста, чипы */
  .composer { position: relative; }
  .composer.busy { border-color: rgb(var(--accent-rgb) / .35); }
  .in { display: flex; align-items: flex-end; gap: 6px; }
  .in textarea { flex: 1; border: 0; background: transparent; padding: 6px 6px; min-height: 32px; max-height: 180px; resize: none; line-height: 1.45; }
  .in textarea:focus { box-shadow: none; }
  .in button.icon { width: 32px; height: 32px; border-radius: 50%; flex: none; }
  .in button.send:not(.primary) { color: var(--fg-3); }
  .in button.stop { color: var(--bad); }
  .popup { position: absolute; left: 6px; right: 6px; bottom: calc(100% + 6px); max-height: 300px; overflow: auto; background: var(--bg-2); border: 1px solid var(--line-2); border-radius: var(--r-2); box-shadow: var(--shadow-2, 0 12px 32px rgb(0 0 0 / .35)); padding: 4px; z-index: 5; display: flex; flex-direction: column; }
  .popup .pk { font-size: 11px; padding: 4px 8px; }
  .popup .pi { display: grid; grid-template-columns: auto auto 1fr; align-items: baseline; gap: 8px; text-align: left; height: auto; min-height: 28px; padding: 4px 8px; border: 0; background: transparent; border-radius: var(--r-1); color: var(--fg-1); }
  .popup .pi.sel, .popup .pi:hover { background: var(--accent-soft); }
  .popup .pn { font-size: 12.5px; white-space: nowrap; } .popup .ph { font-size: 11.5px; white-space: nowrap; }
  .popup .pd { font-size: 11.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tools { display: flex; align-items: center; gap: 4px 6px; flex-wrap: wrap; font-size: 12px; color: var(--fg-3); padding-top: 2px; }
  .tools .st { display: inline-flex; align-items: center; gap: 5px; white-space: nowrap; margin-right: 2px; }
  .tools .ctl { display: inline-flex; align-items: center; gap: 5px; height: 24px; padding: 0 8px; border-radius: 999px; border: 1px solid transparent; background: transparent; color: var(--fg-2); font-size: 12px; max-width: 200px; }
  .tools .ctl .lbl { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tools .ctl:hover { background: var(--bg-3); color: var(--fg-1); border-color: var(--line-1); }
  .tools .ctl.ctx.warn { color: var(--warn); }
  .tools .chip { display: inline-flex; align-items: center; gap: 4px; height: 24px; padding: 0 8px; border-radius: 999px; background: var(--bg-3); color: var(--fg-3); font-size: 11.5px; white-space: nowrap; }
  .ring { width: 14px; height: 14px; border-radius: 50%; flex: none; background: conic-gradient(var(--accent) calc(var(--p) * 1%), var(--line-3) 0); -webkit-mask: radial-gradient(circle at center, transparent 4px, #000 4.5px); mask: radial-gradient(circle at center, transparent 4px, #000 4.5px); }
  .ctx.warn .ring { background: conic-gradient(var(--warn) calc(var(--p) * 1%), var(--line-3) 0); }
  .m.cmd { align-self: stretch; max-width: 100%; background: var(--bg-4); padding: 8px 12px; }
  .m.cmd .ph { font-size: 11.5px; font-weight: 500; margin-bottom: 4px; }
  .m.cmd .out { max-height: 360px; overflow: auto; font-size: 12.5px; }
  /* ответ с таблицей, кодом, заголовками или списком — во всю ширину журнала */
  .m.agent.wide { max-width: 100%; align-self: stretch; }
  .m.typing { display: inline-flex; gap: 4px; padding: 10px 14px; }
  .m.typing span { width: 6px; height: 6px; border-radius: 50%; background: var(--fg-3); animation: blink 1.2s infinite ease-in-out; }
  .m.typing span:nth-child(2) { animation-delay: .2s; } .m.typing span:nth-child(3) { animation-delay: .4s; }
  @keyframes blink { 0%, 80%, 100% { opacity: .25; transform: translateY(0); } 40% { opacity: 1; transform: translateY(-2px); } }
  .badge.run { background: var(--accent-soft); color: var(--accent); gap: 5px; }
  .spin { width: 10px; height: 10px; border-radius: 50%; border: 2px solid currentColor; border-right-color: transparent; animation: spin .8s linear infinite; display: inline-block; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .m.sys.turn { flex-wrap: wrap; }
  /* попап «Режим и усилие» */
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
  .slider .step { display: flex; flex-direction: column; align-items: center; gap: 3px; flex: 1 1 0; min-width: 0; }
  .slider .step i { font-style: normal; font-family: var(--mono); font-size: 9.5px; color: var(--fg-3); }
  .slider .dot { width: 14px; height: 14px; border-radius: 50%; border: 0; padding: 0; background: var(--line-3); }
  .slider .dot.past { background: rgb(var(--accent-rgb) / .45); } .slider .dot.on { background: var(--accent); box-shadow: 0 0 0 3px rgb(var(--accent-rgb) / .25); }
  .modes .eff.sub { border-top: 0; margin-top: 0; padding-top: 6px; }
  .slider.mini { flex-basis: auto; margin-left: auto; padding: 6px 8px; gap: 8px; flex-direction: row; }
  .slider.mini .dot { width: 12px; height: 12px; }
  .tools .ctl .acc { font-style: normal; color: var(--accent); margin-left: 2px; }
  /* попап усилия Codex: имя уровня, модель, градиентный слайдер, ступени, tier */
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
  .modes .eff .reset { margin-left: 36px; align-self: flex-start; }
  .seg { display: inline-flex; border: 1px solid var(--line-2); border-radius: 999px; overflow: hidden; flex: none; }
  .seg button { height: 24px; padding: 0 10px; border: 0; background: transparent; color: var(--fg-3); font-family: var(--mono); font-size: 11.5px; }
  .seg button.on { background: var(--accent-soft); color: var(--accent); }
  .think { width: 96px; height: 26px; }
  @media (max-width: 599px) { .slider .step i { font-size: 8.5px; } }
  .earlier { align-self: center; height: 26px; padding: 0 12px; border-radius: 999px; font-size: 12px; color: var(--fg-3); background: var(--bg-3); border: 1px solid var(--line-1); }
  .earlier:hover { color: var(--fg-1); }
  .jump { position: absolute; left: 50%; transform: translateX(-50%); bottom: 118px; z-index: 4; height: 28px; padding: 0 12px; border-radius: 999px; font-size: 12px; background: var(--accent); color: var(--accent-fg); border: 0; box-shadow: var(--shadow-2, 0 6px 18px rgb(0 0 0 / .35)); }
  .chat { position: relative; }
  @media (max-width: 599px) {
    .hide-p { display: none; }
    .m { max-width: 92%; }
    .tools .ctl { max-width: 130px; padding: 0 6px; }
    .tools .sp { flex-basis: 100%; height: 0; }
  }
</style>
