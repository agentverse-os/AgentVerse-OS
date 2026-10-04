<script lang="ts">
  // Входящие: всё, что ждёт человека, из всех workspace’ов — разрешения и вопросы агентов с ответом прямо здесь,
  // упавшие и завершившиеся сессии.
  import { ui, data, actions, agentName, money, ago } from '../state.svelte';
  import { t } from '../i18n';
  import Icon from '../Icon.svelte';
  import Md from '../Md.svelte';
  import type { InboxItem, PendingReq } from '../api';
  let wsFilter = $state('all');
  let picks = $state<Record<string, Record<string, string[]>>>({});
  let custom = $state<Record<string, Record<string, string>>>({});
  const list = $derived(data.inbox);
  const filt = (items: InboxItem[]) => items.filter(i => wsFilter === 'all' || i.project === wsFilter);
  const projects = $derived(Array.from(new Set([...(list?.waiting ?? []), ...(list?.failed ?? []), ...(list?.done ?? [])].map(i => i.project))));
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
    const answers: Record<string, string> = {};
    for (const q of p.input?.questions ?? []) { const own = custom[p.request_id]?.[q.question]?.trim(); answers[q.question] = own || (picks[p.request_id]?.[q.question] ?? []).join(', '); }
    actions.permission(it.project, it.session.id, { request_id: p.request_id, behavior: 'allow', answers });
  }
  const who = (it: InboxItem) => `${it.project} · ${agentName(it.session.agent)}`;
  /** Подсказка правила Claude Code у запроса разрешения (permission_suggestions). */
  const ruleOf = (p: PendingReq): string => ((p.suggestions ?? []) as any[]).flatMap(s => (s?.rules ?? []) as any[]).map(r => (r.ruleContent ? `${r.toolName}(${r.ruleContent})` : r.toolName)).filter(Boolean).join(', ');
</script>

<main class="main">
  <div class="top">
    <button class="icon ghost burger" onclick={() => (ui.popen = !ui.popen)}><Icon name="menu" /></button>
    <div class="title"><h1>{t('sec.inbox')}</h1>{#if data.waiting.length}<span class="count warn">{data.waiting.length}</span>{/if}</div>
    <span class="sp"></span>
    {#if projects.length > 1}
      <span class="seg hide-m"><button class:active={wsFilter === 'all'} onclick={() => (wsFilter = 'all')}>{t('panel.all')}</button>{#each projects as p (p)}<button class:active={wsFilter === p} onclick={() => (wsFilter = p)}>{p}</button>{/each}</span>
      <select class="sm only-m" bind:value={wsFilter}><option value="all">{t('panel.all')}</option>{#each projects as p (p)}<option value={p}>{p}</option>{/each}</select>
    {/if}
  </div>
  <div class="content">
    {#if data.error && !list}<div class="err">{data.error}</div>{/if}
    {#if list}
      <div class="inbox">
        <div class="col">
          <div class="h-sec">{t('inbox.waitTitle')}</div>
          {#each filt(list.waiting) as it (it.session.id)}
            {#each it.pending ?? [] as p (p.request_id)}
              <div class="item card-s">
                <div class="who"><span class="dot waiting"></span> {t('inbox.waits')} {ago(it.session.created_at)}<span>· {who(it)} · <b>{it.session.title}</b></span><span class="sp"></span><button class="xs ghost" onclick={() => ui.goto({ project: it.project, kind: 'session', id: it.session.id })}>{t('inbox.open')} <Icon name="arrowUR" size="s" /></button></div>
                {#if isQuestion(p)}
                  <div class="what"><Icon name="inbox" size="s" /> {t('inbox.asks')}</div>
                  {#each p.input?.questions ?? [] as q (q.question)}
                    <div class="what" style="font-size: 13.5px"><b>{q.header ? q.header + ': ' : ''}</b>{q.question}</div>
                    <div class="opts">{#each q.options ?? [] as o (o.label)}<button class="sm" class:active={(picks[p.request_id]?.[q.question] ?? []).includes(o.label)} title={o.description} onclick={() => pick(p.request_id, q.question, o.label, !!q.multiSelect)}>{o.label}</button>{/each}<input class="sm" placeholder={t('inbox.own')} style="flex: 1; min-width: 160px" value={custom[p.request_id]?.[q.question] ?? ''} oninput={(e) => { custom[p.request_id] = { ...(custom[p.request_id] ?? {}), [q.question]: (e.target as HTMLInputElement).value }; }} /></div>
                  {/each}
                  <div class="acts"><button class="primary" onclick={() => answer(it, p)}>{t('inbox.answer')}</button><button class="ghost" onclick={() => actions.permission(it.project, it.session.id, { request_id: p.request_id, behavior: 'deny', message: t('inbox.declined') })}>{t('agents.skip')}</button></div>
                {:else}
                  <div class="what"><Icon name="shield" size="s" /><b>{p.tool_name}</b><code class="block">{summary(p)}</code></div>{#if ruleOf(p)}<div class="muted sm">{t('perm.rule')}: <code>{ruleOf(p)}</code></div>{/if}
                  <div class="acts"><button class="primary" onclick={() => actions.permission(it.project, it.session.id, { request_id: p.request_id, behavior: 'allow' })}>{t('agents.allow')}</button><button onclick={() => actions.permission(it.project, it.session.id, { request_id: p.request_id, behavior: 'allow', remember: true, scope: 'project' })}>{ruleOf(p) ? t('perm.remember') + ' · ' + t('perm.scopeProjectShort') : t('agents.allowAlways')}</button><button class="ghost danger" onclick={() => actions.permission(it.project, it.session.id, { request_id: p.request_id, behavior: 'deny' })}>{t('agents.deny')}</button></div>
                {/if}
              </div>
            {/each}
          {/each}
          {#if !filt(list.waiting).length}<div class="empty card-s"><span class="ic"><Icon name="checkc" size="l" /></span><b>{t('inbox.none')}</b><span>{t('inbox.noneHint')}</span></div>{/if}
        </div>
        <div class="col done-col">
          <div class="h-sec">{t('inbox.doneTitle')}</div>
          {#each filt(list.failed) as it (it.session.id)}
            <div class="item card-s failed"><div class="who"><span class="dot failed"></span> {t('inbox.failed')} {ago(it.session.created_at)}<span>· {who(it)} · <b>{it.session.title}</b></span><span class="sp"></span><button class="xs ghost" onclick={() => ui.goto({ project: it.project, kind: 'session', id: it.session.id })}>{t('inbox.open')} <Icon name="arrowUR" size="s" /></button></div>
              {#if it.session.last_text}<div class="what" style="color: var(--bad)"><Icon name="alert" size="s" /> {it.session.last_text}</div>{/if}
              <div class="acts"><button onclick={() => actions.resume({ project: it.project, kind: 'session', id: it.session.id })}><Icon name="play" size="s" /> {t('inbox.resume')}</button></div></div>
          {/each}
          {#each filt(list.done) as it (it.session.id)}
            <div class="item card-s done"><div class="who"><span class="dot ok"></span> {ago(it.session.created_at)}<span>· {who(it)} · <b>{it.session.title}</b></span><span class="sp"></span><button class="xs ghost" onclick={() => ui.goto({ project: it.project, kind: 'session', id: it.session.id })}>{t('inbox.open')} <Icon name="arrowUR" size="s" /></button></div>
              <div class="what"><Icon name="checkc" size="s" /> <span class="ell" style="white-space: normal"><Md rich={false} text={it.session.last_text ?? ""} inline /></span> <span class="chip num">{money(it.session.usage?.cost_usd)}</span></div></div>
          {/each}
          {#if !filt(list.failed).length && !filt(list.done).length}<div class="muted sm" style="padding: 8px">—</div>{/if}
        </div>
      </div>
    {:else}<div class="muted sm">{t('common.loading')}</div>{/if}
  </div>
</main>
