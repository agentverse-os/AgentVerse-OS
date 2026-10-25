<script module lang="ts">
  type EditorTab = 'agents' | 'routines' | 'skills';
  type EditorDraft = { value: Record<string, unknown>; baseline: string };
  // Kept in this browser tab, including while the section is unmounted. No prompts or secrets are written to storage.
  const editorDrafts = new Map<string, EditorDraft>();
  const newEditors = new Set<EditorTab>();
  const lastSelection: Record<EditorTab, string | null> = { agents: null, routines: null, skills: null };
</script>

<script lang="ts">
  // The "Agents" section (docs/agents-ux.md §7): employees (name, brief, model, workspace, mode, memory), their routines
  // (task, trigger — schedule / webhook / event / manual, budget, notifications) and runs (each one a session in the
  // agent's workspace).
  import { untrack, onDestroy } from 'svelte';
  import { ui, data, money, ago, when, runIcon, runClass, msg, MODELS_FOR, ENGINES } from '../state.svelte';
  import { agents, type AgentDef, type Routine, type Run, type Skill } from '../api';
  import { toast } from '../toast.svelte';
  import { t, tn } from '../i18n';
  import Icon from '../Icon.svelte';
  const TABS = $derived<['agents' | 'routines' | 'runs' | 'skills', string][]>([['agents', t('rt.tabAgents')], ['routines', t('rt.tabRoutines')], ['skills', t('rt.tabSkills')], ['runs', t('rt.tabRuns')]]);
  const PRESETS = $derived<[string, string][]>([['0 3 * * *', t('rt.preDaily')], ['0 9 * * 1-5', t('rt.preWeekdays')], ['0 * * * *', t('rt.preHourly')], ['*/30 * * * *', t('rt.preHalfHour')], ['0 8 * * 1', t('rt.preMonday')]]);
  const EVENTS = ['', 'agent', 'app', 'update', 'workspace', 'project', 'grant', 'system', 'host'];
  // Codex: the union of the GPT-6/5.6 line levels from model/list (Luna has no ultra — agentd does not send a foreign level and leaves a note); Claude — as in
  // the composer
  const EFFORTS: Record<string, string[]> = { claude: ['', 'low', 'medium', 'high', 'xhigh', 'max', 'ultracode'], codex: ['', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'], gemini: [''], pi: [''] };
  const blankAgent = (): Partial<AgentDef> => ({ name: '', brief: '', provider: 'claude', model: '', account: '', workspace: data.running[0]?.name ?? '', mode: 'ask', effort: '', rules: { allow: [], deny: [] }, memory: '', remember: true, place: '', can_mail: false, can_routine: true, memory_budget: 6000, reflect: false, skills: [] });
  const lines = (s: string) => s.split('\n').map(x => x.trim()).filter(Boolean);
  const blankRoutine = (): Partial<Routine> => ({ name: '', agent: data.agentsList[0]?.id ?? '', prompt: '', trigger: { kind: 'schedule', cron: '0 9 * * 1-5', event: '', contains: '' }, budget: { run_usd: 1, day_usd: 5 }, notify: 'ask_error', enabled: true, mode: '' });
  const blankSkill = (): Partial<Skill> => ({ name: '', description: '', body: '' });
  let aForm = $state<Partial<AgentDef>>(blankAgent());
  let rForm = $state<Partial<Routine>>(blankRoutine());
  let sForm = $state<Partial<Skill>>(blankSkill());
  let drafting = $state(false);
  let fresh = $state(false);
  let busy = $state(false);
  let running = $state(false);
  let activeKey = $state(''); let baseline = $state('');
  const tab = $derived(ui.routinesTab);
  const selAgent = $derived(data.agentsList.find(a => a.id === ui.routinesSel) ?? null);
  const selRoutine = $derived(data.routines.find(r => r.id === ui.routinesSel) ?? null);
  const selSkill = $derived(data.skills.find(k => k.name === ui.routinesSel) ?? null);
  const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value));
  const clean = <T extends object>(o: T): Partial<T> => Object.fromEntries(Object.entries(o).filter(([k]) => !['routines', 'last_run', 'agent_name', 'workspace_name', 'schedule_text', 'next_at', 'last', 'created_at', 'updated_at', 'last_run_at', 'last_status', 'token', 'reflected_at', 'agents', 'source'].includes(k))) as Partial<T>;
  const currentForm = () => tab === 'agents' ? aForm : tab === 'skills' ? sForm : rForm;
  const signature = (form: object) => JSON.stringify(clean(form));
  const dirty = $derived(tab !== 'runs' && !!activeKey && signature(currentForm()) !== baseline);
  const editorVisible = $derived(fresh || (tab === 'agents' ? !!aForm.id && !!activeKey : tab === 'routines' ? !!rForm.id && !!activeKey : tab === 'skills' ? !!sForm.name && !!activeKey : false));
  function rememberCurrent() { if (activeKey && tab !== 'runs') editorDrafts.set(activeKey, { value: clone(currentForm()) as Record<string, unknown>, baseline }); }
  function setForm(tb: EditorTab, form: Record<string, unknown>) { if (tb === 'agents') aForm = form; else if (tb === 'routines') rForm = form; else sForm = form; }
  // A refresh updates clean editors, but never replaces a dirty draft, even when list membership changes.
  $effect(() => {
    const id = ui.routinesSel; const tb = ui.routinesTab;
    const found = tb === 'agents' ? data.agentsList.find(a => a.id === id) : tb === 'routines' ? data.routines.find(r => r.id === id) : tb === 'skills' ? data.skills.find(k => k.name === id) : null;
    const server = found ? clone(found) as unknown as Record<string, unknown> : null;
    untrack(() => {
      if (tb === 'runs') return;
      const key = `${tb}:${id ?? 'new'}`; const cached = editorDrafts.get(key);
      if (key === activeKey && (dirty || busy || running)) return;
      const useDraft = cached && cached.baseline !== signature(cached.value);
      const value = useDraft ? cached.value : server ?? cached?.value;
      fresh = id === null && newEditors.has(tb);
      if (!value && !fresh) { activeKey = ''; baseline = ''; return; }
      const initial = value ?? (tb === 'agents' ? blankAgent() : tb === 'skills' ? blankSkill() : blankRoutine());
      setForm(tb, clone(initial) as Record<string, unknown>); activeKey = key;
      baseline = useDraft ? cached.baseline : signature(initial);
      lastSelection[tb] = id;
    });
  });
  $effect(() => { if (activeKey && tab !== 'runs') { const value = clone(currentForm()) as Record<string, unknown>; const saved = baseline; untrack(() => editorDrafts.set(activeKey, { value, baseline: saved })); } });
  onDestroy(rememberCurrent);
  function select(id: string) { rememberCurrent(); if (tab !== 'runs') lastSelection[tab] = id; ui.routinesSel = id; fresh = false; }
  function switchTab(next: typeof ui.routinesTab) { if (next === tab) return; rememberCurrent(); ui.routinesTab = next; ui.routinesSel = next === 'runs' ? null : lastSelection[next]; fresh = next !== 'runs' && ui.routinesSel === null && newEditors.has(next); activeKey = ''; }
  function startNew() { if (busy || running) return; rememberCurrent(); if (tab === 'runs') switchTab('routines'); const tb = ui.routinesTab as EditorTab; newEditors.add(tb); lastSelection[tb] = null; ui.routinesSel = null; fresh = true; const key = `${tb}:new`; const cached = editorDrafts.get(key); const value = cached?.value ?? (tb === 'agents' ? blankAgent() : tb === 'skills' ? blankSkill() : blankRoutine()); setForm(tb, clone(value) as Record<string, unknown>); activeKey = key; baseline = cached?.baseline ?? signature(value); }
  function discard() {
    if (busy || running || (dirty && !window.confirm(t('sx.discardConfirm')))) return;
    editorDrafts.delete(activeKey);
    if (fresh && tab !== 'runs') { newEditors.delete(tab); fresh = false; activeKey = ''; baseline = ''; }
    else { const saved = tab === 'agents' ? selAgent : tab === 'skills' ? selSkill : selRoutine; if (saved && tab !== 'runs') { setForm(tab, clone(saved) as unknown as Record<string, unknown>); baseline = signature(saved); } }
  }
  function committed(tb: EditorTab, saved: AgentDef | Routine | Skill, key: string, sent: string) {
    const id = tb === 'skills' ? (saved as Skill).name : (saved as AgentDef | Routine).id;
    const current = activeKey === key && tab === tb ? currentForm() : editorDrafts.get(key)?.value;
    const value = current && signature(current) !== sent ? { ...clone(current), ...(tb === 'skills' ? {} : { id }) } : clone(saved);
    const savedBaseline = signature(saved); const nextKey = `${tb}:${id}`;
    editorDrafts.set(nextKey, { value: value as Record<string, unknown>, baseline: savedBaseline }); if (nextKey !== key) editorDrafts.delete(key);
    newEditors.delete(tb);
    if (activeKey === key && tab === tb) { setForm(tb, value as Record<string, unknown>); baseline = savedBaseline; activeKey = nextKey; ui.routinesSel = id; lastSelection[tb] = id; fresh = false; }
  }
  async function saveAgent() {
    if (busy || drafting || !aForm.name?.trim()) return null; busy = true; const snapshot = clone(aForm); const key = activeKey; const sent = signature(snapshot);
    try {
      const body = clean(snapshot) as Partial<AgentDef>;
      const saved = snapshot.id ? await agents.agentUpdate(snapshot.id, body) : await agents.agentCreate(body);
      committed('agents', saved, key, sent); data.agentsList = [...data.agentsList.filter(a => a.id !== saved.id), saved];
      await data.loadRoutines(); toast(t('rt.saved')); return saved;
    } catch (e) { toast(msg(e), 'bad'); return null; } finally { busy = false; }
  }
  async function deleteAgent() {
    if (busy || running || !aForm.id || !window.confirm(t('rt.deleteAgentConfirm', { name: aForm.name }))) return; const id = aForm.id; const key = activeKey; busy = true;
    try { await agents.agentDelete(id); editorDrafts.delete(key); lastSelection.agents = null; if (activeKey === key) { ui.routinesSel = null; fresh = false; activeKey = ''; } await data.loadRoutines(); toast(t('toast.removed'), 'info'); } catch (e) { toast(msg(e), 'bad'); } finally { busy = false; }
  }
  async function draft() {
    if (busy || running || drafting) return; const description = window.prompt(t('rt.draftPrompt')); if (!description?.trim()) return; const key = activeKey; const workspace = aForm.workspace; const original = { name: aForm.name, brief: aForm.brief };
    drafting = true;
    try { const d = await agents.agentDraft(description.trim(), workspace || undefined); if (activeKey === key) { if (aForm.name === original.name) aForm.name = d.name; if (aForm.brief === original.brief) aForm.brief = d.brief; } else { const saved = editorDrafts.get(key); if (saved) saved.value = { ...saved.value, ...(saved.value.name === original.name ? { name: d.name } : {}), ...(saved.value.brief === original.brief ? { brief: d.brief } : {}) }; } toast(t('rt.draftDone')); } catch (e) { toast(msg(e), 'bad'); } finally { drafting = false; }
  }
  function toggleSkill(name: string) { const cur = aForm.skills ?? []; aForm.skills = cur.includes(name) ? cur.filter(x => x !== name) : [...cur, name]; }
  async function toggleMail() { try { await agents.saveSettings({ mail_paused: !data.mailPaused }); await data.loadRoutines(); } catch (e) { toast(msg(e), 'bad'); } }
  async function saveSkill() {
    if (busy || !sForm.name?.trim() || !sForm.body?.trim()) return; busy = true; const snapshot = clone(sForm); const key = activeKey; const sent = signature(snapshot); const existing = !fresh;
    try {
      const saved = existing ? await agents.skillUpdate(snapshot.name!, { description: snapshot.description ?? '', body: snapshot.body ?? '' }) : await agents.skillCreate({ name: snapshot.name ?? '', description: snapshot.description ?? '', body: snapshot.body ?? '' });
      committed('skills', saved, key, sent); data.skills = [...data.skills.filter(s => s.name !== saved.name), saved]; await data.loadRoutines(); toast(t('rt.saved'));
    } catch (e) { toast(msg(e), 'bad'); } finally { busy = false; }
  }
  async function deleteSkill() {
    if (busy || running || fresh || !sForm.name || !window.confirm(t('rt.deleteSkillConfirm', { name: sForm.name }))) return; const name = sForm.name; const key = activeKey; busy = true;
    try { await agents.skillDelete(name); editorDrafts.delete(key); lastSelection.skills = null; if (activeKey === key) { ui.routinesSel = null; fresh = false; activeKey = ''; } await data.loadRoutines(); toast(t('toast.removed'), 'info'); } catch (e) { toast(msg(e), 'bad'); } finally { busy = false; }
  }
  async function assign() {
    if (busy || running || drafting || !aForm.id) return;
    const prompt = window.prompt(t('rt.assignPrompt', { name: aForm.name })); if (!prompt?.trim()) return;
    const key = activeKey; const id = aForm.id; const needsSave = dirty; running = true;
    try { const saved = needsSave ? await saveAgent() : null; if (needsSave && !saved) return; const r = await agents.agentRun(saved?.id ?? id, prompt.trim()); toast(t('rt.assigned', { st: stLabel(r.run.status) }), r.run.status === 'running' ? 'ok' : 'bad'); await data.loadRoutines(); if (tab === 'agents' && (activeKey === key || activeKey === `agents:${saved?.id}`)) switchTab('runs'); } catch (e) { toast(msg(e), 'bad'); } finally { running = false; }
  }
  async function saveRoutine() {
    if (busy || !rForm.name?.trim() || !rForm.agent || !rForm.prompt?.trim()) return null; busy = true; const snapshot = clone(rForm); const key = activeKey; const sent = signature(snapshot);
    try {
      const body = clean(snapshot) as Partial<Routine>;
      const saved = snapshot.id ? await agents.routineUpdate(snapshot.id, body) : await agents.routineCreate(body);
      committed('routines', saved, key, sent); data.routines = [...data.routines.filter(r => r.id !== saved.id), saved]; await data.loadRoutines(); toast(t('rt.saved')); return saved;
    } catch (e) { toast(msg(e), 'bad'); return null; } finally { busy = false; }
  }
  async function deleteRoutine() {
    if (busy || running || !rForm.id || !window.confirm(t('rt.deleteRoutineConfirm', { name: rForm.name }))) return; const id = rForm.id; const key = activeKey; busy = true;
    try { await agents.routineDelete(id); editorDrafts.delete(key); lastSelection.routines = null; if (activeKey === key) { ui.routinesSel = null; fresh = false; activeKey = ''; } await data.loadRoutines(); toast(t('toast.removed'), 'info'); } catch (e) { toast(msg(e), 'bad'); } finally { busy = false; }
  }
  async function runNow() { if (busy || running || !rForm.id || !rForm.name?.trim() || !rForm.agent || !rForm.prompt?.trim()) return; const id = rForm.id; const needsSave = dirty; running = true; try { const saved = needsSave ? await saveRoutine() : null; if (needsSave && !saved) return; const r = await agents.routineRun(saved?.id ?? id); toast(t('rt.started', { st: stLabel(r.run.status) + (r.run.reason ? ' — ' + r.run.reason : '') }), r.run.status === 'running' ? 'ok' : 'bad'); await data.loadRoutines(); } catch (e) { toast(msg(e), 'bad'); } finally { running = false; } }
  const hookUrl = (token?: string) => (token ? `${location.origin}/agents/api/hooks/${token}` : '');
  async function copyHook() { try { await navigator.clipboard.writeText(hookUrl(rForm.token)); toast(t('rt.copied'), 'info'); } catch {} }
  const stLabel = (st: string) => t(`rt.st.${st}`);
  const trLabel = (r: Partial<Routine>) => r.trigger?.kind === 'schedule' ? cronText(r.trigger.cron) : t(`rt.tr.${r.trigger?.kind ?? 'manual'}`) + (r.trigger?.kind === 'event' && r.trigger.event ? ` ${r.trigger.event}` : '');
  /** Schedule label — the same forms as in the service (routines.rs describe). */
  function cronText(expr = ''): string {
    const f = expr.trim().split(/\s+/); if (f.length !== 5) return expr;
    const num = (x: string) => /^\d+$/.test(x); const hm = (h: string, m: string) => `${h.padStart(2, '0')}:${m.padStart(2, '0')}`;
    const [m, h, dom, mon, dow] = f;
    if (num(m) && num(h) && dom === '*' && mon === '*') { if (dow === '*') return t('rt.cronDaily', { t: hm(h, m) }); if (dow === '1-5') return t('rt.cronWeekdays', { t: hm(h, m) }); if (dow === '0,6' || dow === '6,0') return t('rt.cronWeekend', { t: hm(h, m) }); }
    if (num(m) && h === '*' && dom === '*' && mon === '*' && dow === '*') return t('rt.cronHourly', { m: m.padStart(2, '0') });
    if (m.startsWith('*/') && h === '*' && dom === '*' && mon === '*' && dow === '*') return t('rt.cronEveryMin', { n: m.slice(2) });
    if (m === '0' && h.startsWith('*/') && dom === '*' && mon === '*' && dow === '*') return t('rt.cronEveryH', { n: h.slice(2) });
    return expr;
  }
  const accountsFor = (provider?: string) => data.accounts.filter(a => (provider === 'codex' ? ['codex', 'openai'].includes(a.provider) : ['claude', 'anthropic'].includes(a.provider)));
  const agentName = (id?: string) => data.agentsList.find(a => a.id === id)?.name ?? '—';
  const dur = (r: Run) => `${Math.max(0, Math.round(((r.ended_at ?? Date.now() / 1000) - r.started_at) / 60))} ${t('rt.min')}`;
  const history = $derived(rForm.id ? data.runs.filter(r => r.routine === rForm.id) : []);
</script>

{#snippet draftNotice()}
  {#if dirty}<div class="draft-notice" role="status"><span><b>{t('sx.unsaved')}</b><span class="hint">{t('sx.draftSaved')}</span></span><button class="sm ghost" disabled={busy || running} onclick={discard}>{t('sx.discard')}</button></div>{/if}
{/snippet}

{#snippet runRow(r: Run, withWho: boolean)}
  <div class="tr" data-run={r.id}>
    <span class="ic {runClass(r.status)}" title={stLabel(r.status)}><Icon name={runIcon(r.status)} size="s" /></span>
    <span class="when muted sm num">{when(r.started_at)}</span>
    <span class="who">{#if withWho}<b>{r.routine_name ?? r.agent_name}</b> · {r.agent_name} · {r.project} ·{' '}{/if}{stLabel(r.status)}{#if r.status !== 'skipped'}{' '}· {dur(r)} · {money(r.cost_usd)}{/if}{' '}· <span class="muted">{t(`rt.tr.${r.trigger}`)}{#if r.trigger === 'mail'}{' '}{r.from_agent ? t('rt.fromAgent', { name: agentName(r.from_agent) }) : t('rt.fromOwner')}{/if}</span></span>
    <span>{#if r.session}<button class="xs ghost" onclick={() => ui.goto({ project: r.project, kind: 'session', id: r.session! })}>{t('rt.openSession')} <Icon name="arrowUR" size="s" /></button>{/if}</span>
    {#if r.summary || r.reason}<span class="sum" title={r.summary ?? r.reason ?? ''}>{r.summary || r.reason}</span>{/if}
  </div>
{/snippet}

<main class="main routines-main">
  <div class="top rt-top">
    <button class="icon ghost burger" aria-label={t('nav.sections')} onclick={() => (ui.popen = !ui.popen)}><Icon name="menu" /></button>
    <div class="title"><h1>{t('rt.title')}</h1><span class="muted sm hide-m">· {t('rt.hint')}</span></div>
    <span class="seg">{#each TABS as [k, l] (k)}<button class:active={tab === k} aria-pressed={tab === k} onclick={() => switchTab(k)}>{l}</button>{/each}</span>
    <span class="sp"></span>
    {#if tab === 'agents'}<span class="inl hide-m mailsw" title={t('rt.mailHint')}><button class="switch" class:on={!data.mailPaused} aria-pressed={!data.mailPaused} aria-label={t('rt.mailHint')} onclick={toggleMail}></button><span class="muted sm">{data.mailPaused ? t('rt.mailPaused') : t('rt.mailOn')}</span></span>{/if}
    {#if tab !== 'runs'}<button class="sm primary" disabled={busy || running || drafting} aria-label={tab === 'agents' ? t('rt.newAgent') : tab === 'skills' ? t('rt.newSkill') : t('rt.newRoutine')} onclick={startNew}><Icon name="plus" size="s" /> <span class="hide-m">{tab === 'agents' ? t('rt.newAgent') : tab === 'skills' ? t('rt.newSkill') : t('rt.newRoutine')}</span></button>{/if}
  </div>
  <div class="content page">
    {#if data.routinesError}<div class="editor-error" role="alert"><span>{t('sx.loadFailed')} {data.routinesError}</span><button class="sm" disabled={data.routinesLoading} onclick={() => data.loadRoutines()}>{t('sx.retry')}</button></div>{/if}
    {#if data.routinesLoading && !data.agentsList.length && !data.routines.length && !data.skills.length}<p class="muted sm" role="status">{t('common.loading')}</p>{/if}
    {#if tab === 'agents'}
      <div class="rt">
        <div class="rt-list">
          {#each data.agentsList as a (a.id)}
            <button class="rt-item card-s" class:active={ui.routinesSel === a.id} data-agent={a.id} onclick={() => select(a.id)}>
              <span class="av {a.provider}"><Icon name={a.provider} size="s" /></span>
              <span class="nm">{a.name}</span>
              <span class="st">{#if a.last_run}<span class="dot {runClass(a.last_run.status)}"></span> {ago(a.last_run.started_at)}{:else}{t('rt.neverAgent')}{/if}</span>
              <span class="sub">{ENGINES.find(e => e.id === a.provider)?.title ?? a.provider}{a.model ? ' · ' + a.model : ''} · {a.workspace} · {t(`rt.mode${a.mode === 'ask' ? 'Ask' : a.mode === 'auto' ? 'Auto' : 'Bypass'}`)} · {tn('rt.routines', a.routines ?? 0)}</span>
              {#if a.brief}<span class="brief">«{a.brief}»</span>{/if}
            </button>
          {/each}
          {#if !data.agentsList.length}<div class="empty card-s"><span class="ic"><Icon name="claude" size="l" /></span><b>{t('rt.noAgents')}</b><span>{t('rt.noAgentsHint')}</span></div>{/if}
        </div>
        {#if editorVisible}
          <div class="card-s fs rt-form">
            <h3>{aForm.id ? aForm.name : t('rt.newAgent')}</h3>
            {@render draftNotice()}
            <fieldset class="frm" disabled={busy || running}>
              <span>{t('rt.name')}</span><input class="sm" aria-label={t('rt.name')} bind:value={aForm.name} placeholder={t('rt.phName')} />
              <span>{t('rt.provider')}</span><span class="radios">{#each ENGINES as e (e.id)}<label><input type="radio" name="prov" value={e.id} bind:group={aForm.provider} /> {e.title}</label>{/each}</span>
              <span>{t('rt.model')}</span><select class="sm" aria-label={t('rt.model')} bind:value={aForm.model}><option value="">{t('rt.modelDefault')}</option>{#each MODELS_FOR[aForm.provider ?? 'claude'] ?? [] as m (m)}<option value={m}>{m}</option>{/each}</select>
              <span>{t('rt.account')}</span><select class="sm" aria-label={t('rt.account')} bind:value={aForm.account}><option value="">{t('rt.accountGate')}</option>{#each accountsFor(aForm.provider) as acc (acc.id)}<option value={acc.id}>{acc.label}</option>{/each}</select>
              <span>{t('rt.workspace')}</span><select class="sm" aria-label={t('rt.workspace')} bind:value={aForm.workspace}>{#each data.projects as p (p.name)}<option value={p.name}>{p.name}{p.status !== 'running' ? ` (${p.status})` : ''}</option>{/each}</select>
              <span>{t('rt.mode')}</span><span class="radios"><label title={t('rt.modeAskHint')}><input type="radio" name="mode" value="ask" bind:group={aForm.mode} /> {t('rt.modeAsk')}</label><label title={t('rt.modeAutoHint')}><input type="radio" name="mode" value="auto" bind:group={aForm.mode} /> {t('rt.modeAuto')}</label><label title={t('rt.modeBypassHint')}><input type="radio" name="mode" value="bypass" bind:group={aForm.mode} /> {t('rt.modeBypass')}</label></span>
              <span>{t('rt.effort')}</span><select class="sm" aria-label={t('rt.effort')} bind:value={aForm.effort}>{#each EFFORTS[aForm.provider ?? 'claude'] ?? [''] as e (e)}<option value={e}>{e || 'auto'}</option>{/each}</select>
              <span>{t('rt.brief')}<div class="hint">{t('rt.briefHint')}</div></span><span class="stack" style="gap: 6px"><textarea aria-label={t('rt.brief')} bind:value={aForm.brief} placeholder={t('rt.phBrief')}></textarea><span class="inl"><button class="xs draft" disabled={drafting} onclick={draft}><Icon name="sparkles" size="s" /> {t('rt.draft')}{drafting ? '…' : ''}</button></span></span>
              {#if aForm.provider === 'claude'}
                <span>{t('rt.rules')}<div class="hint">{t('rt.rulesHint')}</div></span>
                <span class="stack" style="gap: 6px">
                  <textarea class="mem" rows="3" aria-label={t('sx.allowRules')} placeholder="Bash(npm test:*)&#10;Read&#10;Edit" value={(aForm.rules?.allow ?? []).join('\n')} oninput={(e) => (aForm.rules = { allow: lines((e.target as HTMLTextAreaElement).value), deny: aForm.rules?.deny ?? [] })}></textarea>
                  <span class="hint">{t('rt.rulesDeny')}</span>
                  <textarea class="mem" rows="2" aria-label={t('sx.denyRules')} placeholder="Bash(git push:*)" value={(aForm.rules?.deny ?? []).join('\n')} oninput={(e) => (aForm.rules = { allow: aForm.rules?.allow ?? [], deny: lines((e.target as HTMLTextAreaElement).value) })}></textarea>
                </span>
              {/if}
              <span>{t('rt.memory')}<div class="hint">{t('rt.memoryHint')}</div></span><textarea class="mem memory" aria-label={t('rt.memory')} bind:value={aForm.memory}></textarea>
              <span></span><label class="row" style="gap: 8px; cursor: pointer"><input type="checkbox" bind:checked={aForm.remember} /> {t('rt.remember')}</label>
              <span>{t('rt.memoryBudget')}</span><span class="inl"><input class="sm num" type="number" min="500" max="20000" step="500" aria-label={t('rt.memoryBudget')} bind:value={aForm.memory_budget} /><label class="row" style="gap: 8px; cursor: pointer"><input type="checkbox" bind:checked={aForm.reflect} /> {t('rt.reflect')}</label></span>
              <span>{t('rt.place')}<div class="hint">{t('rt.placeHint')}</div></span><input class="sm mono place" aria-label={t('rt.place')} bind:value={aForm.place} placeholder="apps/site" />
              <span>{t('rt.rights')}</span><span class="stack" style="gap: 4px"><label class="row" style="gap: 8px; cursor: pointer"><input type="checkbox" class="can-mail" bind:checked={aForm.can_mail} /> {t('rt.canMail')}</label><label class="row" style="gap: 8px; cursor: pointer"><input type="checkbox" class="can-routine" bind:checked={aForm.can_routine} /> {t('rt.canRoutine')}</label></span>
              <span>{t('rt.skills')}<div class="hint">{t('rt.skillsHint')}</div></span>
              <span class="stack skills" style="gap: 4px">
                {#each data.skills as k (k.name)}<label class="row" style="gap: 8px; cursor: pointer" title={k.description}><input type="checkbox" data-skill={k.name} checked={(aForm.skills ?? []).includes(k.name)} onchange={() => toggleSkill(k.name)} /> <span class="mono">{k.name}</span><span class="muted sm"> — {k.description}</span></label>{/each}
                {#if !data.skills.length}<span class="hint">{t('rt.noSkills')}</span>{/if}
              </span>
            </fieldset>
            <div class="acts"><button class="sm primary" disabled={busy || running || drafting || !aForm.name?.trim()} onclick={saveAgent}>{aForm.id ? t('common.save') : t('rt.create')}</button>{#if aForm.id}<button class="sm" disabled={busy || running || drafting || !aForm.name?.trim()} onclick={assign}><Icon name="send" size="s" /> {running ? t('sx.running') : dirty ? t('sx.saveAssign') : t('rt.assign')}</button>{/if}<span class="sp"></span>{#if aForm.id}<button class="sm ghost danger" disabled={busy || running} onclick={deleteAgent}>{t('rt.deleteAgent')}</button>{:else}<button class="sm ghost" disabled={busy || running} onclick={discard}>{t('rt.cancel')}</button>{/if}</div>
          </div>
        {/if}
      </div>
    {:else if tab === 'routines'}
      <div class="rt">
        <div class="rt-list">
          {#each data.routines as r (r.id)}
            <button class="rt-item card-s" class:active={ui.routinesSel === r.id} data-routine={r.id} onclick={() => select(r.id)}>
              <span class="dot {r.enabled ? (r.last?.status === 'running' ? 'working' : 'ok') : ''}"></span>
              <span class="nm">{r.name}</span>
              <span class="st">{#if r.last}<span class="dot {runClass(r.last.status)}"></span> {ago(r.last.started_at)} · {money(r.last.cost_usd)}{:else}{t('rt.never')}{/if}</span>
              <span class="sub">{r.agent_name} · {r.workspace} · {trLabel(r)}{#if r.next_at} · {t('rt.next')} {when(r.next_at)}{/if}{#if !r.enabled} · {t('rt.disabled')}{/if}</span>
            </button>
          {/each}
          {#if !data.routines.length}<div class="empty card-s"><span class="ic"><Icon name="repeat" size="l" /></span><b>{t('rt.noRoutines')}</b><span>{t('rt.noRoutinesHint')}</span></div>{/if}
        </div>
        {#if editorVisible}
          <div class="card-s fs rt-form">
            <h3>{rForm.id ? rForm.name : t('rt.newRoutine')}</h3>
            {#if !data.agentsList.length}<p>{t('rt.noAgentsForRoutine')}</p>{/if}
            {@render draftNotice()}
            <fieldset class="frm" disabled={busy || running}>
              <span>{t('rt.routineName')}</span><input class="sm" aria-label={t('rt.routineName')} bind:value={rForm.name} placeholder={t('rt.phRoutine')} />
              <span>{t('rt.agent')}</span><select class="sm" aria-label={t('rt.agent')} bind:value={rForm.agent}>{#each data.agentsList as a (a.id)}<option value={a.id}>{a.name} · {a.workspace}</option>{/each}</select>
              <span>{t('rt.prompt')}<div class="hint">{t('rt.promptHint')}</div></span><textarea aria-label={t('rt.prompt')} bind:value={rForm.prompt} placeholder={t('rt.phPrompt')}></textarea>
              <span>{t('rt.trigger')}</span>
              <span class="stack" style="gap: 8px">
                <span class="radios">{#each [['schedule', t('rt.trSchedule')], ['webhook', t('rt.trWebhook')], ['event', t('rt.trEvent')], ['manual', t('rt.trManual')]] as [k, l] (k)}<label><input type="radio" name="trig" value={k} bind:group={rForm.trigger!.kind} /> {l}</label>{/each}</span>
                {#if rForm.trigger?.kind === 'schedule'}
                  <span class="inl"><input class="sm mono" style="width: 150px" aria-label={t('rt.trigger')} bind:value={rForm.trigger!.cron} placeholder="0 3 * * *" /><select class="sm" aria-label={t('rt.presets')} value="" onchange={(e) => { const v = (e.target as HTMLSelectElement).value; if (v) rForm.trigger!.cron = v; (e.target as HTMLSelectElement).value = ''; }}><option value="">{t('rt.presets')}</option>{#each PRESETS as [c, l] (c)}<option value={c}>{l}</option>{/each}</select><span class="hint">{cronText(rForm.trigger?.cron)}{#if selRoutine?.next_at && selRoutine.trigger.cron === rForm.trigger?.cron} · {t('rt.next')} {when(selRoutine.next_at)}{/if}</span></span>
                  <span class="hint">{t('rt.cronHint')}</span><span class="hint">{t('sx.scheduleZone', { zone: Intl.DateTimeFormat().resolvedOptions().timeZone })}</span>
                {:else if rForm.trigger?.kind === 'webhook'}
                  {#if rForm.token}<span class="inl"><span class="hook" title={hookUrl(rForm.token)}>{hookUrl(rForm.token)}</span><button class="xs" onclick={copyHook}><Icon name="copy" size="s" /> {t('rt.copy')}</button></span>{:else}<span class="hint">{t('rt.hookAfterSave')}</span>{/if}
                  <span class="hint">{t('rt.hookHint')}</span>
                {:else if rForm.trigger?.kind === 'event'}
                  <span class="inl"><select class="sm" aria-label={t('rt.trEvent')} bind:value={rForm.trigger!.event}>{#each EVENTS as k (k)}<option value={k}>{k || t('rt.eventAny')}</option>{/each}</select><span class="hint">{t('rt.contains')}</span><input class="sm" style="width: 180px" aria-label={t('rt.contains')} bind:value={rForm.trigger!.contains} placeholder={t('rt.phContains')} /></span>
                {/if}
              </span>
              <span>{t('rt.budget')}<div class="hint">{t('rt.noLimit')}</div></span><span class="inl"><input class="sm num" type="number" min="0" step="0.5" aria-label={t('rt.perRun')} bind:value={rForm.budget!.run_usd} /> $ {t('rt.perRun')}<input class="sm num" type="number" min="0" step="1" aria-label={t('rt.perDay')} bind:value={rForm.budget!.day_usd} /> $ {t('rt.perDay')}</span>
              <span>{t('rt.notify')}</span><span class="radios">{#each [['ask_error', t('rt.nAskError')], ['always', t('rt.nAlways')], ['never', t('rt.nNever')]] as [k, l] (k)}<label><input type="radio" name="ntf" value={k} bind:group={rForm.notify} /> {l}</label>{/each}</span>
              <span>{t('rt.routineMode')}</span><select class="sm rmode" aria-label={t('rt.routineMode')} bind:value={rForm.mode}><option value="">{t('rt.modeAsAgent')}</option><option value="ask">{t('rt.modeAsk')}</option><option value="auto">{t('rt.modeAuto')}</option><option value="bypass">{t('rt.modeBypass')}</option></select>
              <span>{t('rt.enabled')}</span><span><button class="switch" class:on={rForm.enabled} aria-pressed={!!rForm.enabled} aria-label={t('rt.enabled')} onclick={() => (rForm.enabled = !rForm.enabled)}></button></span>
            </fieldset>
            <div class="acts"><button class="sm primary" disabled={busy || running || !rForm.name?.trim() || !rForm.agent || !rForm.prompt?.trim()} onclick={saveRoutine}>{rForm.id ? t('common.save') : t('rt.create')}</button>{#if rForm.id}<button class="sm" disabled={busy || running || !rForm.name?.trim() || !rForm.agent || !rForm.prompt?.trim()} onclick={runNow}><Icon name="play" size="s" /> {running ? t('sx.running') : dirty ? t('sx.saveRun') : t('rt.runNow')}</button>{/if}<span class="sp"></span>{#if rForm.id}<button class="sm ghost danger" disabled={busy || running} onclick={deleteRoutine}>{t('rt.deleteRoutine')}</button>{:else}<button class="sm ghost" disabled={busy || running} onclick={discard}>{t('rt.cancel')}</button>{/if}</div>
            {#if rForm.id}
              <div class="h-sec" style="margin-top: 8px">{t('rt.history')}</div>
              <div class="runs card-s" style="overflow: hidden">{#each history as r (r.id)}{@render runRow(r, false)}{/each}{#if !history.length}<div class="muted sm" style="padding: 12px 14px">{t('rt.never')}</div>{/if}</div>
            {/if}
          </div>
        {/if}
      </div>
    {:else if tab === 'skills'}
      <div class="rt">
        <div class="rt-list">
          {#each data.skills as k (k.name)}
            <button class="rt-item card-s" class:active={ui.routinesSel === k.name} data-skill={k.name} onclick={() => select(k.name)}>
              <span class="av"><Icon name="book" size="s" /></span>
              <span class="nm mono">{k.name}</span>
              <span class="st">{k.source === 'user' ? t('rt.skillOwner') : t('rt.skillFromAgent')}</span>
              <span class="sub">{k.description}{#if k.agents?.length} · {t('rt.skillUsedBy')}: {k.agents.join(', ')}{/if}</span>
            </button>
          {/each}
          {#if !data.skills.length}<div class="empty card-s"><span class="ic"><Icon name="book" size="l" /></span><b>{t('rt.noSkills')}</b><span>{t('rt.noSkillsHint')}</span></div>{/if}
        </div>
        {#if editorVisible}
          <div class="card-s fs rt-form">
            <h3>{!fresh ? sForm.name : t('rt.newSkill')}</h3>
            {@render draftNotice()}
            <fieldset class="frm" disabled={busy || running}>
              <span>{t('rt.skillName')}</span><input class="sm mono" aria-label={t('rt.skillName')} bind:value={sForm.name} placeholder="run-tests" disabled={!fresh} />
              <span>{t('rt.skillDesc')}</span><input class="sm" aria-label={t('rt.skillDesc')} bind:value={sForm.description} />
              <span>{t('rt.skillBody')}<div class="hint">{t('rt.noSkillsHint')}</div></span><textarea class="mem memory" aria-label={t('rt.skillBody')} bind:value={sForm.body}></textarea>
            </fieldset>
            <div class="acts"><button class="sm primary" disabled={busy || running || !sForm.name?.trim() || !sForm.body?.trim()} onclick={saveSkill}>{!fresh ? t('common.save') : t('rt.create')}</button><span class="sp"></span>{#if !fresh}<button class="sm ghost danger" disabled={busy || running} onclick={deleteSkill}>{t('rt.deleteSkill')}</button>{:else}<button class="sm ghost" disabled={busy || running} onclick={discard}>{t('rt.cancel')}</button>{/if}</div>
          </div>
        {/if}
      </div>
    {:else}
      <div class="runs card-s" style="overflow: hidden; max-width: 1100px">
        {#each data.runs as r (r.id)}{@render runRow(r, true)}{/each}
        {#if !data.runs.length}<div class="empty"><span class="ic"><Icon name="play" size="l" /></span><b>{t('rt.noRuns')}</b><span>{t('rt.noRunsHint')}</span></div>{/if}
      </div>
    {/if}
  </div>
</main>

<style>
  fieldset.frm { border: 0; margin: 0; padding: 0; min-width: 0; }
  .draft-notice { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin-bottom: 14px; padding: 10px; background: var(--bg-3); border: 1px solid var(--line-2); border-radius: var(--r-2); font-size: 12px; }
  .draft-notice > span { display: flex; flex-direction: column; gap: 4px; flex: 1 1 200px; }
  .draft-notice b { color: var(--warn); }
  .editor-error { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; padding-bottom: 14px; color: var(--bad); font-size: 13px; }
  .editor-error > span { flex: 1 1 240px; overflow-wrap: anywhere; }
  .rt-top .seg { min-width: 0; overflow-x: auto; }
  .rt-top .seg button { flex: 0 0 auto; white-space: nowrap; }
  @media (pointer: coarse) { .routines-main button:not(.switch), .routines-main input:not([type="radio"]):not([type="checkbox"]), .routines-main select { min-height: 44px; } .routines-main .radios label, .routines-main label.row { min-height: 44px; } }
  @media (max-width: 599px) { .rt-top { gap: 6px; } .rt-top .seg { flex: 1; } .rt-top > .sp { display: none; } .rt-top > button.primary { min-width: 44px; } .rt-item .st { max-width: 120px; overflow: hidden; text-overflow: ellipsis; } .draft-notice > span { flex-basis: 100%; } }
</style>
