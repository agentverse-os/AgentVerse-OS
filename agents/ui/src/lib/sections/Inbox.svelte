<script lang="ts">
  // Inbox: everything waiting for a human from all workspaces — permissions and agent questions answered right
  // here, failed and finished sessions.
  import { ui, data, actions, agentName, money, ago } from '../state.svelte';
  import { t } from '../i18n';
  import Icon from '../Icon.svelte';
  import Md from '../Md.svelte';
  import { api, type InboxItem, type PendingReq } from '../api';
  import { msg } from '../state.svelte';
  let wsFilter = $state('all');
  let picks = $state<Record<string, Record<string, string[]>>>({});
  let custom = $state<Record<string, Record<string, string>>>({});
  let pending = $state<Record<string, boolean>>({});
  let errors = $state<Record<string, string>>({});
  let resolved = $state<Record<string, boolean>>({});
  const list = $derived(data.inbox);
  const filt = (items: InboxItem[]) => items.filter(i => wsFilter === 'all' || i.project === wsFilter);
  const projects = $derived(Array.from(new Set([...(list?.waiting ?? []), ...(list?.failed ?? []), ...(list?.done ?? [])].map(i => i.project))));
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
          {#if !filt(list.waiting).length}<div class="empty card-s"><span class="ic"><Icon name="checkc" size="l" /></span><b>{t('inbox.none')}</b><span>{t('inbox.noneHint')}</span></div>{/if}
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
