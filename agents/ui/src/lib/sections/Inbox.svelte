<script lang="ts">
  // Inbox: everything waiting for a human from all workspaces — permissions and agent questions answered right
  // here, failed and finished sessions.
  import { ui, data, actions, agentName, money, ago, when } from '../state.svelte';
  import { t } from '../i18n';
  import Icon from '../Icon.svelte';
  import Md from '../Md.svelte';
  import { api, agents, type InboxItem, type PendingReq, type MemoryCard, type LongCard } from '../api';
  import { msg } from '../state.svelte';
  import { toast } from '../toast.svelte';
  import { memBytes, memSpan } from '../memory';
  let wsFilter = $state('all');
  let picks = $state<Record<string, Record<string, string[]>>>({});
  let custom = $state<Record<string, Record<string, string>>>({});
  let pending = $state<Record<string, boolean>>({});
  let errors = $state<Record<string, string>>({});
  let resolved = $state<Record<string, boolean>>({});
  const list = $derived(data.inbox);
  const filt = (items: InboxItem[]) => items.filter(i => wsFilter === 'all' || i.project === wsFilter);
  const projects = $derived(Array.from(new Set([...(list?.waiting ?? []), ...(list?.failed ?? []), ...(list?.done ?? []), ...(list?.memory ?? []), ...(list?.long ?? [])].map(i => i.project))));
  // memory watch (ADR-009, §4.13): what it did last in a workspace and turns that have been silent for hours
  const byWs = <T extends { project: string }>(items: T[] | undefined) => (items ?? []).filter(i => wsFilter === 'all' || i.project === wsFilter);
  const memCards = $derived(byWs(list?.memory));
  const longCards = $derived(byWs(list?.long));
  let hidden = $state<Record<string, boolean>>({});
  let busyKey = $state('');
  let nowSecs = $state(Math.floor(Date.now() / 1000));
  $effect(() => { const iv = setInterval(() => (nowSecs = Math.floor(Date.now() / 1000)), 30000); return () => clearInterval(iv); });
  const memTitle = (c: MemoryCard) => (c.kind === 'killed' ? (c.note === 'manual' ? t('mem.killedManual') : t('mem.killed')) : c.kind === 'oom' ? t('mem.oom') : t('mem.standing'));
  const memNote = (c: MemoryCard) => (c.note === 'notify' ? t('mem.noteNotify') : c.note === 'small' ? t('mem.noteSmall') : c.note === 'rate' ? t('mem.noteRate') : c.note === 'host' ? t('mem.noteHost') : '');
  function memLines(c: MemoryCard): string[] {
    const pct = Math.round(c.full60); const out: string[] = [];
    if (c.kind === 'killed') { out.push(t('mem.proc', { cmd: c.cmd, rss: memBytes(c.rss), age: memSpan(c.age_secs) })); if (c.note !== 'manual' && c.stalled_secs) out.push(t('mem.stood', { secs: memSpan(c.stalled_secs), limit: memBytes(c.limit), pct })); }
    else if (c.kind === 'oom') { out.push(c.cmd ? t('mem.procOom', { cmd: c.cmd, rss: memBytes(c.rss) }) : t('mem.procUnknown')); out.push(t('mem.limitOnly', { limit: memBytes(c.limit) })); }
    else { out.push(c.live ? t('mem.standsNow', { pct, secs: memSpan(c.stalled_secs + Math.max(0, nowSecs - c.at)), limit: memBytes(c.limit) }) : t('mem.stood', { secs: memSpan(c.stalled_secs), limit: memBytes(c.limit), pct })); if (c.cmd) out.push(t('mem.largest', { cmd: c.cmd, rss: memBytes(c.rss) })); const n = memNote(c); if (n) out.push(n); }
    if (c.session_title) out.push(t('mem.session', { title: c.session_title }));
    return out;
  }
  async function dismiss(key: string) {
    hidden[key] = true;
    try { await agents.inboxDismiss(key); } catch (e) { hidden[key] = false; toast(msg(e), 'bad'); }
  }
  async function killNow(c: MemoryCard) {
    if (busyKey || !window.confirm(t('mem.killConfirm', { project: c.project }))) return;
    busyKey = c.key;
    try { const a = await api.memoryKill(c.project); toast(t('mem.killDone', { project: c.project, cmd: a.cmd })); }
    catch (e) { toast(msg(e), 'bad'); }
    finally { busyKey = ''; }
  }
  async function interrupt(c: LongCard) {
    if (busyKey) return; busyKey = c.key;
    try { await api.sessionInterrupt(c.project, c.session.id); toast(t('long.interrupted', { title: c.session.title })); }
    catch (e) { toast(msg(e), 'bad'); }
    finally { busyKey = ''; }
  }
  /** Memory limit lives in the project parameters of Desktop */
  const openLimit = () => window.open(`${location.origin}/#projects`, '_blank', 'noopener');
  $effect(() => { if (list && wsFilter !== 'all' && !projects.includes(wsFilter)) wsFilter = 'all'; });
  const requestKey = (it: InboxItem, p: PendingReq) => `${it.project}:${it.session.id}:${p.request_id}`;
  const canAnswer = (it: InboxItem, p: PendingReq) => (p.input?.questions ?? []).length > 0 && (p.input.questions as any[]).every(q => custom[requestKey(it, p)]?.[q.question]?.trim() || picks[requestKey(it, p)]?.[q.question]?.length);
  async function decide(it: InboxItem, p: PendingReq, body: { behavior: 'allow' | 'deny'; [key: string]: unknown }) {
    const key = requestKey(it, p); if (pending[key] || resolved[key]) return;
    pending[key] = true; errors[key] = '';
    try { await api.sessionPermission(it.project, it.session.id, { request_id: p.request_id, ...body }); resolved[key] = true; await Promise.all([data.loadInbox(), data.loadWs()]); }
    catch (e) { errors[key] = msg(e); }
    finally { pending[key] = false; }
  }
  const isQuestion = (p: PendingReq) => p.tool_name === 'AskUserQuestion' || p.kind === 'codex_question';
  function summary(p: PendingReq): string {
    const i = p.input ?? {};
    if (p.tool_name === 'Bash') return i.command ?? '';
    if (['Write', 'Edit', 'Read', 'MultiEdit'].includes(p.tool_name)) return i.file_path ?? '';
    const s = JSON.stringify(i); return s.length > 200 ? s.slice(0, 200) + '…' : s;
  }
  function pick(rid: string, q: string, label: string, multi: boolean) {
    const cur = picks[rid]?.[q] ?? []; const next = multi ? (cur.includes(label) ? cur.filter(l => l !== label) : [...cur, label]) : [label];
    picks[rid] = { ...(picks[rid] ?? {}), [q]: next };
  }
  function answer(it: InboxItem, p: PendingReq) {
    if (!canAnswer(it, p)) { errors[requestKey(it, p)] = t('ux.answerRequired'); return; }
    const answers: Record<string, string> = {};
    for (const q of p.input?.questions ?? []) { const own = custom[requestKey(it, p)]?.[q.question]?.trim(); answers[q.question] = own || (picks[requestKey(it, p)]?.[q.question] ?? []).join(', '); }
    decide(it, p, { behavior: 'allow', answers });
  }
  const who = (it: InboxItem) => `${it.project} · ${agentName(it.session.agent)}`;
  /** Claude Code rule suggestion on a permission request (permission_suggestions). */
  const ruleOf = (p: PendingReq): string => ((p.suggestions ?? []) as any[]).flatMap(s => (s?.rules ?? []) as any[]).map(r => (r.ruleContent ? `${r.toolName}(${r.ruleContent})` : r.toolName)).filter(Boolean).join(', ');
</script>

<main class="main">
  <div class="top">
    <button class="icon ghost burger" onclick={() => (ui.popen = !ui.popen)}><Icon name="menu" /></button>
    <div class="title"><h1>{t('sec.inbox')}</h1>{#if data.waiting.length}<span class="count warn">{data.waiting.length}</span>{/if}</div>
    <span class="sp"></span>
    {#if projects.length > 1 || wsFilter !== 'all'}
      <span class="seg hide-m"><button class:active={wsFilter === 'all'} onclick={() => (wsFilter = 'all')}>{t('panel.all')}</button>{#each projects as p (p)}<button class:active={wsFilter === p} onclick={() => (wsFilter = p)}>{p}</button>{/each}</span>
      <select class="sm only-m" aria-label={t('ux.projectFilter')} bind:value={wsFilter}><option value="all">{t('panel.all')}</option>{#each projects as p (p)}<option value={p}>{p}</option>{/each}</select>
    {/if}
  </div>
  <div class="content">
    {#if data.inboxError}<div class="load-warning" role="alert"><span>{list ? t('ux.refreshFailed') : t('ux.loadFailed')} {data.inboxError}</span><button disabled={data.inboxLoading} onclick={() => data.loadInbox()}>{t('common.retry')}</button></div>{/if}
    {#if list}
      <div class="inbox">
        <div class="col">
          <div class="h-sec">{t('inbox.waitTitle')}</div>
          {#each memCards.filter(c => !hidden[c.key]) as c (c.key)}
            <div class="item card-s mem {c.kind}" class:live={c.live} data-mem-card={c.kind} data-key={c.key} aria-busy={busyKey === c.key}>
              <div class="who"><Icon name={c.kind === 'stalled' ? 'gauge' : 'alert'} size="s" /> {ago(c.at)} · <b>{c.project}</b> · {t('mem.title')}<span class="sp"></span><button class="xs ghost mem-hide" onclick={() => dismiss(c.key)}>{t('mem.hide')}</button></div>
              <div class="what"><b>{memTitle(c)}</b></div>
              {#each memLines(c) as line, i (i)}<div class="muted sm mem-line">{line}</div>{/each}
              <div class="acts">
                {#if c.can_kill}<button class="danger mem-kill" disabled={!!busyKey} onclick={() => killNow(c)}>{t('mem.kill')}</button>{/if}
                {#if c.session}<button class="mem-open" onclick={() => ui.goto({ project: c.project, kind: 'session', id: c.session! })}>{t('mem.openSession')} <Icon name="arrowUR" size="s" /></button>{/if}
                <button class="ghost mem-limit" onclick={openLimit}>{t('mem.limit')}</button>
              </div>
            </div>
          {/each}
          {#each longCards.filter(c => !hidden[c.key]) as c (c.key)}
            <div class="item card-s long" data-long-card={c.session.id} data-key={c.key} aria-busy={busyKey === c.key}>
              <div class="who"><Icon name="gauge" size="s" /> <b>{c.project}</b> · <b>{c.session.title}</b> · {t('long.title', { time: memSpan(nowSecs - c.since) })}<span class="sp"></span><button class="xs ghost long-mute" onclick={() => dismiss(c.key)}>{t('long.mute')}</button></div>
              <div class="what">{#if c.tool}<Icon name="terminal" size="s" /><code class="block">{c.tool}</code>{:else}{t('long.noTool', { when: when(c.since) })}{/if}</div>
              <div class="acts"><button class="primary long-open" onclick={() => ui.goto({ project: c.project, kind: 'session', id: c.session.id })}>{t('long.open')}</button><button class="ghost danger long-interrupt" disabled={!!busyKey} onclick={() => interrupt(c)}>{t('long.interrupt')}</button></div>
            </div>
          {/each}
          {#each filt(list.waiting) as it (it.session.id)}
            {#each (it.pending ?? []).filter(p => !resolved[requestKey(it, p)]) as p (p.request_id)}
              <div class="item card-s" data-request={p.request_id} aria-busy={pending[requestKey(it, p)]}>
                <div class="who"><span class="dot waiting"></span> {t('inbox.waits')} · {t('ux.lastActivity', { when: ago(it.session.updated_at || it.session.created_at) })}<span>· {who(it)} · <b>{it.session.title}</b></span><span class="sp"></span><button class="xs ghost" onclick={() => ui.goto({ project: it.project, kind: 'session', id: it.session.id })}>{t('inbox.open')} <Icon name="arrowUR" size="s" /></button></div>
                {#if isQuestion(p)}
                  <div class="what"><Icon name="inbox" size="s" /> {t('inbox.asks')}</div>
                  {#each p.input?.questions ?? [] as q (q.question)}
                    <div class="what" style="font-size: 13.5px"><b>{q.header ? q.header + ': ' : ''}</b>{q.question}</div>
                    <div class="opts">{#each q.options ?? [] as o (o.label)}<button class="sm" class:active={(picks[requestKey(it, p)]?.[q.question] ?? []).includes(o.label)} aria-pressed={(picks[requestKey(it, p)]?.[q.question] ?? []).includes(o.label)} disabled={pending[requestKey(it, p)]} title={o.description} onclick={() => pick(requestKey(it, p), q.question, o.label, !!q.multiSelect)}>{o.label}</button>{/each}<input class="sm" disabled={pending[requestKey(it, p)]} aria-label={t('ux.answerField', { question: q.question })} placeholder={t('inbox.own')} style="flex: 1; min-width: 160px" value={custom[requestKey(it, p)]?.[q.question] ?? ''} oninput={(e) => { custom[requestKey(it, p)] = { ...(custom[requestKey(it, p)] ?? {}), [q.question]: (e.target as HTMLInputElement).value }; }} /></div>
                  {/each}
                  <div class="acts"><button class="primary" disabled={pending[requestKey(it, p)] || !canAnswer(it, p)} onclick={() => answer(it, p)}>{t('inbox.answer')}</button><button class="ghost" disabled={pending[requestKey(it, p)]} onclick={() => decide(it, p, { behavior: 'deny', message: t('inbox.declined') })}>{t('agents.skip')}</button></div>
                {:else}
                  <div class="what"><Icon name="shield" size="s" /><b>{p.tool_name}</b><code class="block">{summary(p)}</code></div>{#if ruleOf(p)}<div class="muted sm">{t('perm.rule')}: <code>{ruleOf(p)}</code></div>{/if}
                  <div class="acts"><button class="primary" disabled={pending[requestKey(it, p)]} onclick={() => decide(it, p, { behavior: 'allow' })}>{t('agents.allow')}</button><button disabled={pending[requestKey(it, p)]} onclick={() => decide(it, p, { behavior: 'allow', remember: true, scope: 'project' })}>{ruleOf(p) ? t('perm.remember') + ' · ' + t('perm.scopeProjectShort') : t('agents.allowAlways')}</button><button class="ghost danger" disabled={pending[requestKey(it, p)]} onclick={() => decide(it, p, { behavior: 'deny' })}>{t('agents.deny')}</button></div>
                {/if}
                {#if errors[requestKey(it, p)]}<div class="err" role="alert">{errors[requestKey(it, p)]}</div>{/if}
              </div>
            {/each}
          {/each}
          {#if !filt(list.waiting).length && !memCards.some(c => !hidden[c.key]) && !longCards.some(c => !hidden[c.key])}<div class="empty card-s"><span class="ic"><Icon name="checkc" size="l" /></span><b>{t('inbox.none')}</b><span>{t('inbox.noneHint')}</span></div>{/if}
        </div>
        <div class="col done-col">
          <div class="h-sec">{t('inbox.doneTitle')}</div>
          {#each filt(list.failed) as it (it.session.id)}
            <div class="item card-s failed"><div class="who"><span class="dot failed"></span> {t('inbox.failed')} {t('ux.lastActivity', { when: ago(it.session.updated_at || it.session.created_at) })}<span>· {who(it)} · <b>{it.session.title}</b></span><span class="sp"></span><button class="xs ghost" onclick={() => ui.goto({ project: it.project, kind: 'session', id: it.session.id })}>{t('inbox.open')} <Icon name="arrowUR" size="s" /></button></div>
              {#if it.session.last_text}<div class="what" style="color: var(--bad)"><Icon name="alert" size="s" /> {it.session.last_text}</div>{/if}
              <div class="acts"><button onclick={() => actions.resume({ project: it.project, kind: 'session', id: it.session.id })}><Icon name="play" size="s" /> {t('inbox.resume')}</button></div></div>
          {/each}
          {#each filt(list.done) as it (it.session.id)}
            <div class="item card-s done"><div class="who"><span class="dot ok"></span> {t('ux.lastActivity', { when: ago(it.session.updated_at || it.session.created_at) })}<span>· {who(it)} · <b>{it.session.title}</b></span><span class="sp"></span><button class="xs ghost" onclick={() => ui.goto({ project: it.project, kind: 'session', id: it.session.id })}>{t('inbox.open')} <Icon name="arrowUR" size="s" /></button></div>
              <div class="what"><Icon name="checkc" size="s" /> <span class="ell" style="white-space: normal"><Md rich={false} text={it.session.last_text ?? ""} inline /></span> <span class="chip num">{money(it.session.usage?.cost_usd)}</span></div></div>
          {/each}
          {#if !filt(list.failed).length && !filt(list.done).length}<div class="muted sm" style="padding: 8px">—</div>{/if}
        </div>
      </div>
    {:else if !data.inboxError}<div class="muted sm" role="status">{t('common.loading')}</div>{/if}
  </div>
</main>
