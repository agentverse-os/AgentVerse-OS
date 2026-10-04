<script lang="ts">
  // Карточка сессии агента, представление Chat (ADR-006): журнал сессии из cloudos-agentd по WebSocket (сначала всё после
  // последнего известного seq, потом живое), сообщения, вызовы инструментов с результатами, карточки разрешений и вопросов
  // агента с ответом отсюда, композер с фото. Процесс живёт в workspace — окно и вкладку можно закрывать.
  import { onMount, tick } from 'svelte';
  import { api, type SessionInfo, type SessionEvent } from './api';
  import { t, tn } from './i18n.svelte';
  let { project, id, onchange }: { project: string; id: string; onchange?: () => void } = $props();
  let events = $state<SessionEvent[]>([]);
  let info = $state<SessionInfo | null>(null);
  let conn = $state<'connecting' | 'live' | 'lost'>('connecting');
  let text = $state('');
  let images = $state<{ name: string; type: string; data: string; url: string }[]>([]);
  let error = $state('');
  let log: HTMLDivElement;
  let lastSeq = 0;
  let replayLeft = 0; let replayDone = false;
  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));

  onMount(() => {
    let ws: WebSocket | null = null; let closed = false; let retry = 0;
    // история — одним HTTP-запросом (без покадровой «промотки» реплея), WebSocket — только живое
    (async () => {
      try {
        const list = await api.sessionEvents(project, id, 0);
        if (!closed && list.length) { events = list; lastSeq = list[list.length - 1].seq; replayDone = true; await tick(); scroll(); }
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
        if (m.kind === 'hello') { info = m.info; replayLeft = m.replay ?? 0; if (!replayLeft) refresh(); return; }
        if (m.kind === 'pong') return;
        if (m.kind === 'lagged') { api.sessionEvents(project, id, lastSeq).then(add).catch(() => {}); return; }
        add([m]);
      };
      ws.onclose = () => { if (closed) return; conn = 'lost'; setTimeout(connect, Math.min(1000 * 2 ** retry++, 15000)); };
      ws.onerror = () => ws?.close();
    }
    function add(list: SessionEvent[]) {
      let changed = false;
      for (const e of list) { if (e.seq > lastSeq) { events.push(e); lastSeq = e.seq; changed = true; if (replayLeft > 0) replayLeft--; } }
      if (!changed) return;
      // реплей истории: без refresh на каждое событие (иначе шторм запросов к ядру), одно обновление по его окончании
      if (replayLeft > 0) return;
      if (!replayDone) { replayDone = true; refresh(); scroll(); return; }
      if (list.some(e => e.kind === 'status' || e.kind === 'result' || e.kind === 'exit' || e.kind === 'init')) refresh();
      scroll();
    }
    const ping = setInterval(() => { if (ws?.readyState === WebSocket.OPEN) ws.send('{"kind":"ping"}'); }, 25000);
    return () => { closed = true; clearInterval(ping); ws?.close(); };
  });
  let changeT: ReturnType<typeof setTimeout> | undefined;
  async function refresh() { try { info = (await api.sessionGet(project, id)).info; clearTimeout(changeT); changeT = setTimeout(() => onchange?.(), 700); } catch {} }
  async function scroll() { await tick(); if (log) log.scrollTop = log.scrollHeight; }

  const status = $derived(info?.status ?? 'connecting');
  const pendingIds = $derived(new Set(events.filter(e => e.kind === 'permission_result').map(e => e.request_id)));
  async function send() {
    const body = text.trim(); if (!body && !images.length) return;
    const content: unknown[] = [];
    for (const im of images) content.push({ type: 'image', source: { type: 'base64', media_type: im.type, data: im.data } });
    if (body) content.push({ type: 'text', text: body });
    try { await api.sessionPrompt(project, id, content); text = ''; images = []; error = ''; } catch (e) { error = msg(e); }
  }
  function onKey(e: KeyboardEvent) { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); } }
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
  function summary(name: string, input: any): string {
    if (!input) return '';
    if (name === 'Bash') return input.command ?? '';
    if (['Write', 'Edit', 'Read', 'MultiEdit'].includes(name)) return input.file_path ?? '';
    if (name === 'Glob' || name === 'Grep') return input.pattern ?? '';
    if (name === 'WebFetch') return input.url ?? '';
    const s = JSON.stringify(input); return s.length > 160 ? s.slice(0, 160) + '…' : s;
  }
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
  function fmtTokens(n?: number) { return n == null ? '—' : n >= 1000 ? (n / 1000).toFixed(1) + 'k' : String(n); }
  // модель и режим разрешений — прямо в композере (как у CloudCLI): у Claude через control-запросы, у Codex со следующим ходом
  const modelHints = $derived(info?.agent === 'codex' ? ['gpt-6-astra', 'gpt-5.5', 'gpt-5-codex', 'gpt-oss-120b-medium'] : ['claude-fable-5-1', 'claude-opus-5-5', 'claude-sonnet-5', 'claude-haiku-4-5-20251001']);
  const modes = $derived(info?.agent === 'codex'
    ? [{ v: 'on-request', l: t('agents.modeOnRequest') }, { v: 'untrusted', l: t('agents.modeUntrusted') }, { v: 'never', l: t('agents.modeNever') }]
    : [{ v: 'default', l: t('agents.modeDefault') }, { v: 'acceptEdits', l: t('agents.modeAcceptEdits') }, { v: 'plan', l: t('agents.modePlan') }, { v: 'bypassPermissions', l: t('agents.modeBypass') }]);
  const modeValue = $derived(info?.permission_mode ?? (info?.agent === 'codex' ? 'on-request' : 'default'));
  async function setModel(v: string) { if ((v.trim() || null) === (info?.model ?? null)) return; try { await api.sessionModel(project, id, v.trim() || null); await refresh(); } catch (e) { error = msg(e); } }
  async function setMode(v: string) { try { await api.sessionMode(project, id, v); await refresh(); } catch (e) { error = msg(e); } }
</script>

<div class="chat">
  <div class="log" bind:this={log}>
    {#each events as e (e.seq)}
      {#if e.kind === 'user'}
        <div class="m user">{#each e.content ?? [] as b}{#if b.type === 'text'}<div class="txt">{b.text}</div>{:else if b.type === 'image'}<div class="muted">📷 {t('agents.image')}{b.bytes ? ` · ${Math.round(b.bytes / 1024)} KB` : ''}</div>{/if}{/each}</div>
      {:else if e.kind === 'assistant'}
        {#each e.content ?? [] as b}
          {#if b.type === 'text' && b.text?.trim()}<div class="m agent"><div class="txt">{b.text}</div></div>
          {:else if b.type === 'thinking' && b.thinking}<details class="m think"><summary>💭 {t('agents.thinking')}</summary><div class="txt muted">{b.thinking}</div></details>
          {:else if b.type === 'tool_use'}
            <details class="m tool" class:err={toolResults[b.id]?.error}><summary><b>{b.name}</b> <span class="muted mono">{summary(b.name, b.input)}</span>{#if toolResults[b.id]} <span class="badge {toolResults[b.id].error ? 'bad' : 'ok'}">{toolResults[b.id].error ? t('agents.toolError') : t('agents.toolDone')}</span>{/if}</summary>
              <pre class="mono">{JSON.stringify(b.input, null, 1)}</pre>{#if toolResults[b.id]}<pre class="mono res">{toolResults[b.id].text.slice(0, 4000)}</pre>{/if}</details>
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
            {#if !pendingIds.has(e.request_id)}<div class="row" style="gap:6px; margin-top:6px;"><button class="primary mini" onclick={() => submitAnswers(e)}>{t('agents.answer')}</button><button class="mini" onclick={() => answer(e, 'deny', { message: 'Пользователь отказался отвечать' })}>{t('agents.skip')}</button></div>{/if}
          {:else}
            <div class="ph">🔐 {t('agents.permission')}: <b>{e.tool_name}</b></div>
            {#if e.title || e.description}<div class="muted">{e.title ?? ''} {e.description ?? ''}</div>{/if}
            <pre class="mono">{summary(e.tool_name, e.input) || JSON.stringify(e.input, null, 1)}</pre>
            {#if e.decision_reason}<div class="muted sm">{e.decision_reason}</div>{/if}
            {#if !pendingIds.has(e.request_id)}
              <div class="row" style="gap:6px; margin-top:6px;">
                <button class="primary mini" onclick={() => answer(e, 'allow')}>{t('agents.allow')}</button>
                <button class="mini" onclick={() => answer(e, 'allow', { remember: true })}>{t('agents.allowAlways')}</button>
                <button class="mini danger" onclick={() => answer(e, 'deny')}>{t('agents.deny')}</button>
              </div>
            {/if}
          {/if}
        </div>
      {:else if e.kind === 'permission_result'}
        <div class="m sys muted">{e.behavior === 'allow' ? '✅' : '⛔'} {e.tool_name} · {e.behavior === 'allow' ? t('agents.allowed') : t('agents.denied')}{e.remember ? ' · ' + t('agents.remembered') : ''}</div>
      {:else if e.kind === 'result'}
        <div class="m sys muted">{e.is_error ? '⚠️ ' + (e.subtype ?? 'error') : '✓'} · {((e.duration_ms ?? 0) / 1000).toFixed(1)} s · {tn('agents.turns', e.num_turns ?? 1)} · {fmtTokens(e.usage?.input_tokens)}↑ {fmtTokens(e.usage?.output_tokens)}↓{#if e.usage?.cache_read_input_tokens} · cache {fmtTokens(e.usage.cache_read_input_tokens)}{/if} · ${(e.total_cost_usd ?? 0).toFixed(4)}</div>
      {:else if e.kind === 'exit'}
        <div class="m sys muted">⏹ {t('agents.sessionExited')}</div>
      {:else if e.kind === 'interrupt'}
        <div class="m sys muted">✋ {t('agents.interrupted')}</div>
      {:else if e.kind === 'error'}
        <div class="m sys" style="color: var(--bad);">⚠️ {e.message}</div>
      {:else if e.kind === 'model'}
        <div class="m sys muted">🧠 {e.model ?? t('agents.modelDefault')}</div>
      {:else if e.kind === 'permission_mode'}
        <div class="m sys muted">🔐 {e.mode}</div>
      {:else if e.kind === 'system' && e.subtype === 'permission_denied'}
        <div class="m sys muted">⛔ {JSON.stringify(e.data?.message ?? e.data).slice(0, 200)}</div>
      {/if}
    {/each}
    {#if !events.length}<div class="muted" style="padding: 12px;">{conn === 'live' ? t('agents.emptyChat') : t('agents.connecting')}</div>{/if}
  </div>
  {#if error}<div class="muted" style="color: var(--bad);">{error}</div>{/if}
  <div class="composer">
    {#if images.length}<div class="thumbs">{#each images as im, i}<span class="thumb"><img src={im.url} alt={im.name} /><button class="x" onclick={() => images.splice(i, 1)}>×</button></span>{/each}</div>{/if}
    <div class="row" style="gap: 6px; align-items: flex-end;">
      <label class="attach" title={t('agents.attach')}>📎<input type="file" accept="image/*" multiple onchange={attach} /></label>
      <textarea rows="2" bind:value={text} onkeydown={onKey} placeholder={status === 'exited' ? t('agents.exitedHint') : status === 'waiting' ? t('agents.waitingHint') : t('agents.placeholder')} disabled={status === 'exited' || status === 'error'}></textarea>
      {#if status === 'working' || status === 'starting'}<button onclick={interrupt} title={t('agents.stopTurn')}>■</button>{/if}
      {#if status === 'exited' || status === 'error'}<button class="primary" onclick={resume}>{t('agents.resume')}</button>{:else}<button class="primary" onclick={send} disabled={!text.trim() && !images.length}>{t('agents.send')}</button>{/if}
    </div>
    <div class="tools">
      <span class="muted sm">{conn === 'live' ? '' : conn === 'lost' ? t('agents.reconnecting') + ' · ' : ''}{status === 'working' ? '⏳ ' + t('agents.working') : status === 'waiting' ? '✋ ' + t('agents.waiting') : status === 'idle' ? t('agents.idle') : status}</span>
      <span class="sp"></span>
      <label class="ctl" title={t('agents.modelHint')}>🧠<input class="model" list="models-{id}" value={info?.model ?? ''} placeholder={t('agents.modelDefault')} onchange={(e) => setModel((e.target as HTMLInputElement).value)} /><datalist id="models-{id}">{#each modelHints as m (m)}<option value={m}></option>{/each}</datalist></label>
      <label class="ctl" title={t('agents.modeHint')}>🔐<select class="mode" value={modeValue} onchange={(e) => setMode((e.target as HTMLSelectElement).value)}>{#each modes as m (m.v)}<option value={m.v}>{m.l}</option>{/each}</select></label>
      {#if info?.usage?.turns}<span class="muted sm usage" title={t('agents.usageHint')}>{fmtTokens(info.usage.input + info.usage.cache_read)}↑ {fmtTokens(info.usage.output)}↓ · ${info.usage.cost_usd.toFixed(3)}</span>{/if}
    </div>
  </div>
</div>

<style>
  .chat { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 6px; }
  .log { flex: 1; min-height: 0; overflow: auto; display: flex; flex-direction: column; gap: 8px; padding: 6px 2px; }
  .m { max-width: 92%; border-radius: var(--r); padding: 8px 12px; background: var(--panel-2); border: 1px solid var(--line); }
  .m.user { align-self: flex-end; background: rgb(var(--accent-rgb) / .18); border-color: rgb(var(--accent-rgb) / .35); }
  .m.agent { align-self: flex-start; }
  .m.sys { align-self: center; background: transparent; border: 0; font-size: .78rem; padding: 2px 8px; }
  .m.tool, .m.think { align-self: flex-start; font-size: .85rem; width: 92%; }
  .m.tool.err { border-color: var(--bad); }
  .m.perm { align-self: stretch; max-width: 100%; border-color: var(--warn, #e0a83a); }
  .m.perm.done { opacity: .55; }
  .ph { font-weight: 600; margin-bottom: 4px; }
  .txt { white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.45; }
  .mono { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: .78rem; white-space: pre-wrap; overflow-wrap: anywhere; margin: 6px 0 0; max-height: 240px; overflow: auto; }
  .res { border-top: 1px dashed var(--line); padding-top: 6px; color: var(--muted); }
  summary { cursor: pointer; }
  .q { margin-top: 6px; }
  .opts { display: flex; flex-wrap: wrap; gap: 4px; margin: 4px 0; }
  .mini { font-size: .78rem; padding: 3px 8px; }
  .mini.active { background: rgb(var(--accent-rgb) / .25); border-color: var(--accent); }
  .mini.danger { color: var(--bad); }
  .own { width: 100%; font-size: .82rem; }
  .composer { display: flex; flex-direction: column; gap: 4px; }
  .composer textarea { flex: 1; resize: vertical; min-height: 44px; }
  .attach { cursor: pointer; padding: 6px 8px; border: 1px solid var(--line); border-radius: var(--r-sm); background: var(--panel-2); }
  .attach input { display: none; }
  .thumbs { display: flex; gap: 6px; flex-wrap: wrap; }
  .thumb { position: relative; }
  .thumb img { width: 56px; height: 56px; object-fit: cover; border-radius: 8px; border: 1px solid var(--line); }
  .thumb .x { position: absolute; top: -6px; right: -6px; width: 20px; height: 20px; padding: 0; border-radius: 50%; font-size: 12px; line-height: 1; }
  .sm { font-size: .74rem; }
  .tools { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .sp { flex: 1; }
  .ctl { display: inline-flex; align-items: center; gap: 4px; font-size: .74rem; color: var(--muted); }
  .ctl input, .ctl select { font-size: .74rem; padding: 2px 6px; background: transparent; border-color: transparent; color: var(--ink); max-width: 200px; }
  .ctl input:hover, .ctl select:hover, .ctl input:focus, .ctl select:focus { border-color: var(--line); background: var(--panel-2); }
  .ctl input.model { width: 170px; }
  .usage { white-space: nowrap; }
</style>
