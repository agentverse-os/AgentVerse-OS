<script lang="ts">
  // Раздел «Агенты» (docs/agents-ux.md §7): сотрудники (имя, бриф, модель, workspace, режим, память), их рутины (задание,
  // триггер — расписание / вебхук / событие / вручную, бюджет, уведомления) и запуски (каждый — сессия в workspace агента).
  import { untrack } from 'svelte';
  import { ui, data, money, ago, when, runIcon, runClass, msg, MODELS_FOR, ENGINES } from '../state.svelte';
  import { agents, type AgentDef, type Routine, type Run, type Skill } from '../api';
  import { toast } from '../toast.svelte';
  import { t, tn } from '../i18n';
  import Icon from '../Icon.svelte';
  const TABS = $derived<['agents' | 'routines' | 'runs' | 'skills', string][]>([['agents', t('rt.tabAgents')], ['routines', t('rt.tabRoutines')], ['skills', t('rt.tabSkills')], ['runs', t('rt.tabRuns')]]);
  const PRESETS = $derived<[string, string][]>([['0 3 * * *', t('rt.preDaily')], ['0 9 * * 1-5', t('rt.preWeekdays')], ['0 * * * *', t('rt.preHourly')], ['*/30 * * * *', t('rt.preHalfHour')], ['0 8 * * 1', t('rt.preMonday')]]);
  const EVENTS = ['', 'agent', 'app', 'update', 'workspace', 'project', 'grant', 'system', 'host'];
  const EFFORTS: Record<string, string[]> = { claude: ['', 'low', 'medium', 'high', 'xhigh', 'max'], codex: ['', 'low', 'medium', 'high', 'xhigh'], gemini: [''], pi: [''] };
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
  const tab = $derived(ui.routinesTab);
  const selAgent = $derived(data.agentsList.find(a => a.id === ui.routinesSel) ?? null);
  const selRoutine = $derived(data.routines.find(r => r.id === ui.routinesSel) ?? null);
  const selSkill = $derived(data.skills.find(k => k.name === ui.routinesSel) ?? null);
  // форма подхватывает выбранную запись; опрос раз в 10 с её не сбрасывает (следим за выбором и числом записей, не за данными)
  $effect(() => {
    const id = ui.routinesSel; const tb = ui.routinesTab; const na = data.agentsList.length; const nr = data.routines.length; const ns = data.skills.length;
    untrack(() => {
      if (tb === 'agents') { const a = data.agentsList.find(x => x.id === id); if (a) { aForm = { ...a }; fresh = false; } else if (!fresh) aForm = blankAgent(); }
      if (tb === 'routines') { const r = data.routines.find(x => x.id === id); if (r) { rForm = { ...r, trigger: { ...r.trigger }, budget: { ...r.budget }, mode: r.mode ?? '' }; fresh = false; } else if (!fresh) rForm = blankRoutine(); }
      if (tb === 'skills') { const k = data.skills.find(x => x.name === id); if (k) { sForm = { ...k }; fresh = false; } else if (!fresh) sForm = blankSkill(); }
      void na; void nr; void ns;
    });
  });
  function select(id: string) { ui.routinesSel = id; fresh = false; }
  function startNew() { ui.routinesSel = null; fresh = true; if (tab === 'agents') aForm = blankAgent(); else if (tab === 'skills') sForm = blankSkill(); else { ui.routinesTab = 'routines'; rForm = blankRoutine(); } }
  const clean = <T extends object>(o: T): Partial<T> => Object.fromEntries(Object.entries(o).filter(([k]) => !['routines', 'last_run', 'agent_name', 'workspace_name', 'schedule_text', 'next_at', 'last', 'created_at', 'updated_at', 'last_run_at', 'last_status', 'token', 'reflected_at', 'agents'].includes(k))) as Partial<T>;
  async function saveAgent() {
    busy = true;
    try {
      const body = clean(aForm) as Partial<AgentDef>;
      const saved = aForm.id ? await agents.agentUpdate(aForm.id, body) : await agents.agentCreate(body);
      await data.loadRoutines(); select(saved.id); toast(t('rt.saved'));
    } catch (e) { toast(msg(e), 'bad'); } finally { busy = false; }
  }
  async function deleteAgent() {
    if (!aForm.id || !window.confirm(t('rt.deleteAgentConfirm', { name: aForm.name }))) return;
    try { await agents.agentDelete(aForm.id); ui.routinesSel = null; fresh = false; await data.loadRoutines(); toast(t('toast.removed'), 'info'); } catch (e) { toast(msg(e), 'bad'); }
  }
  async function draft() {
    const description = window.prompt(t('rt.draftPrompt')); if (!description?.trim()) return;
    drafting = true;
    try { const d = await agents.agentDraft(description.trim(), aForm.workspace || undefined); aForm.name = d.name; aForm.brief = d.brief; toast(t('rt.draftDone')); } catch (e) { toast(msg(e), 'bad'); } finally { drafting = false; }
  }
  function toggleSkill(name: string) { const cur = aForm.skills ?? []; aForm.skills = cur.includes(name) ? cur.filter(x => x !== name) : [...cur, name]; }
  async function toggleMail() { try { await agents.saveSettings({ mail_paused: !data.mailPaused }); await data.loadRoutines(); } catch (e) { toast(msg(e), 'bad'); } }
  async function saveSkill() {
    busy = true;
    try {
      const saved = selSkill && !fresh ? await agents.skillUpdate(selSkill.name, { description: sForm.description ?? '', body: sForm.body ?? '' }) : await agents.skillCreate({ name: sForm.name ?? '', description: sForm.description ?? '', body: sForm.body ?? '' });
      await data.loadRoutines(); select(saved.name); toast(t('rt.saved'));
    } catch (e) { toast(msg(e), 'bad'); } finally { busy = false; }
  }
  async function deleteSkill() {
    if (!selSkill || !window.confirm(t('rt.deleteSkillConfirm', { name: selSkill.name }))) return;
    try { await agents.skillDelete(selSkill.name); ui.routinesSel = null; fresh = false; await data.loadRoutines(); toast(t('toast.removed'), 'info'); } catch (e) { toast(msg(e), 'bad'); }
  }
  async function assign() {
    if (!aForm.id) { toast(t('rt.saveFirst'), 'bad'); return; }
    const prompt = window.prompt(t('rt.assignPrompt', { name: aForm.name })); if (!prompt?.trim()) return;
    try { const r = await agents.agentRun(aForm.id, prompt.trim()); toast(t('rt.assigned', { st: stLabel(r.run.status) }), r.run.status === 'running' ? 'ok' : 'bad'); await data.loadRoutines(); ui.routinesTab = 'runs'; } catch (e) { toast(msg(e), 'bad'); }
  }
  async function saveRoutine() {
    busy = true;
    try {
      const body = clean(rForm) as Partial<Routine>;
      const saved = rForm.id ? await agents.routineUpdate(rForm.id, body) : await agents.routineCreate(body);
      await data.loadRoutines(); select(saved.id); toast(t('rt.saved'));
    } catch (e) { toast(msg(e), 'bad'); } finally { busy = false; }
  }
  async function deleteRoutine() {
    if (!rForm.id || !window.confirm(t('rt.deleteRoutineConfirm', { name: rForm.name }))) return;
    try { await agents.routineDelete(rForm.id); ui.routinesSel = null; fresh = false; await data.loadRoutines(); toast(t('toast.removed'), 'info'); } catch (e) { toast(msg(e), 'bad'); }
  }
  async function runNow(id: string) { try { const r = await agents.routineRun(id); toast(t('rt.started', { st: stLabel(r.run.status) + (r.run.reason ? ' — ' + r.run.reason : '') }), r.run.status === 'running' ? 'ok' : 'bad'); await data.loadRoutines(); } catch (e) { toast(msg(e), 'bad'); } }
  async function toggle(r: Routine) { try { await agents.routineUpdate(r.id, { enabled: !r.enabled }); await data.loadRoutines(); } catch (e) { toast(msg(e), 'bad'); } }
  const hookUrl = (token?: string) => (token ? `${location.origin}/agents/api/hooks/${token}` : '');
  async function copyHook() { try { await navigator.clipboard.writeText(hookUrl(rForm.token)); toast(t('rt.copied'), 'info'); } catch {} }
  const stLabel = (st: string) => t(`rt.st.${st}`);
  const trLabel = (r: Partial<Routine>) => r.trigger?.kind === 'schedule' ? cronText(r.trigger.cron) : t(`rt.tr.${r.trigger?.kind ?? 'manual'}`) + (r.trigger?.kind === 'event' && r.trigger.event ? ` ${r.trigger.event}` : '');
  /** Подпись расписания — те же формы, что в сервисе (routines.rs describe). */
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

{#snippet runRow(r: Run, withWho: boolean)}
  <div class="tr" data-run={r.id}>
    <span class="ic {runClass(r.status)}" title={stLabel(r.status)}><Icon name={runIcon(r.status)} size="s" /></span>
    <span class="when muted sm num">{when(r.started_at)}</span>
    <span class="who">{#if withWho}<b>{r.routine_name ?? r.agent_name}</b> · {r.agent_name} · {r.project} ·{' '}{/if}{stLabel(r.status)}{#if r.status !== 'skipped'}{' '}· {dur(r)} · {money(r.cost_usd)}{/if}{' '}· <span class="muted">{t(`rt.tr.${r.trigger}`)}{#if r.trigger === 'mail'}{' '}{r.from_agent ? t('rt.fromAgent', { name: agentName(r.from_agent) }) : t('rt.fromOwner')}{/if}</span></span>
    <span>{#if r.session}<button class="xs ghost" onclick={() => ui.goto({ project: r.project, kind: 'session', id: r.session! })}>{t('rt.openSession')} <Icon name="arrowUR" size="s" /></button>{/if}</span>
    {#if r.summary || r.reason}<span class="sum" title={r.summary ?? r.reason ?? ''}>{r.summary || r.reason}</span>{/if}
  </div>
{/snippet}

<main class="main">
  <div class="top">
    <button class="icon ghost burger" onclick={() => (ui.popen = !ui.popen)}><Icon name="menu" /></button>
    <div class="title"><h1>{t('rt.title')}</h1><span class="muted sm hide-m">· {t('rt.hint')}</span></div>
    <span class="seg">{#each TABS as [k, l] (k)}<button class:active={tab === k} onclick={() => { ui.routinesTab = k; fresh = false; }}>{l}</button>{/each}</span>
    <span class="sp"></span>
    {#if tab === 'agents'}<span class="inl hide-m mailsw" title={t('rt.mailHint')}><button class="switch" class:on={!data.mailPaused} aria-pressed={!data.mailPaused} onclick={toggleMail}></button><span class="muted sm">{data.mailPaused ? t('rt.mailPaused') : t('rt.mailOn')}</span></span>{/if}
    {#if tab !== 'runs'}<button class="sm primary" onclick={startNew}><Icon name="plus" size="s" /> <span class="hide-m">{tab === 'agents' ? t('rt.newAgent') : tab === 'skills' ? t('rt.newSkill') : t('rt.newRoutine')}</span></button>{/if}
  </div>
  <div class="content">
    {#if data.error && !data.agentsList.length && !data.routines.length}<div class="err" style="margin-bottom: 10px">{data.error}</div>{/if}
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
        {#if fresh || selAgent}
          <div class="card-s fs rt-form">
            <h3>{aForm.id ? aForm.name : t('rt.newAgent')}</h3>
            <div class="frm">
              <span>{t('rt.name')}</span><input class="sm" bind:value={aForm.name} placeholder={t('rt.phName')} />
              <span>{t('rt.provider')}</span><span class="radios">{#each ENGINES as e (e.id)}<label><input type="radio" name="prov" value={e.id} bind:group={aForm.provider} /> {e.title}</label>{/each}</span>
              <span>{t('rt.model')}</span><select class="sm" bind:value={aForm.model}><option value="">{t('rt.modelDefault')}</option>{#each MODELS_FOR[aForm.provider ?? 'claude'] ?? [] as m (m)}<option value={m}>{m}</option>{/each}</select>
              <span>{t('rt.account')}</span><select class="sm" bind:value={aForm.account}><option value="">{t('rt.accountGate')}</option>{#each accountsFor(aForm.provider) as acc (acc.id)}<option value={acc.id}>{acc.label}</option>{/each}</select>
              <span>{t('rt.workspace')}</span><select class="sm" bind:value={aForm.workspace}>{#each data.projects as p (p.name)}<option value={p.name}>{p.name}{p.status !== 'running' ? ` (${p.status})` : ''}</option>{/each}</select>
              <span>{t('rt.mode')}</span><span class="radios"><label title={t('rt.modeAskHint')}><input type="radio" name="mode" value="ask" bind:group={aForm.mode} /> {t('rt.modeAsk')}</label><label title={t('rt.modeAutoHint')}><input type="radio" name="mode" value="auto" bind:group={aForm.mode} /> {t('rt.modeAuto')}</label><label title={t('rt.modeBypassHint')}><input type="radio" name="mode" value="bypass" bind:group={aForm.mode} /> {t('rt.modeBypass')}</label></span>
              <span>{t('rt.effort')}</span><select class="sm" bind:value={aForm.effort}>{#each EFFORTS[aForm.provider ?? 'claude'] ?? [''] as e (e)}<option value={e}>{e || 'auto'}</option>{/each}</select>
              <span>{t('rt.brief')}<div class="hint">{t('rt.briefHint')}</div></span><span class="stack" style="gap: 6px"><textarea bind:value={aForm.brief} placeholder={t('rt.phBrief')}></textarea><span class="inl"><button class="xs draft" disabled={drafting} onclick={draft}><Icon name="sparkles" size="s" /> {t('rt.draft')}{drafting ? '…' : ''}</button></span></span>
              {#if aForm.provider === 'claude'}
                <span>{t('rt.rules')}<div class="hint">{t('rt.rulesHint')}</div></span>
                <span class="stack" style="gap: 6px">
                  <textarea class="mem" rows="3" placeholder="Bash(npm test:*)&#10;Read&#10;Edit" value={(aForm.rules?.allow ?? []).join('\n')} oninput={(e) => (aForm.rules = { allow: lines((e.target as HTMLTextAreaElement).value), deny: aForm.rules?.deny ?? [] })}></textarea>
                  <span class="hint">{t('rt.rulesDeny')}</span>
                  <textarea class="mem" rows="2" placeholder="Bash(git push:*)" value={(aForm.rules?.deny ?? []).join('\n')} oninput={(e) => (aForm.rules = { allow: aForm.rules?.allow ?? [], deny: lines((e.target as HTMLTextAreaElement).value) })}></textarea>
                </span>
              {/if}
              <span>{t('rt.memory')}<div class="hint">{t('rt.memoryHint')}</div></span><textarea class="mem memory" bind:value={aForm.memory}></textarea>
              <span></span><label class="row" style="gap: 8px; cursor: pointer"><input type="checkbox" bind:checked={aForm.remember} /> {t('rt.remember')}</label>
              <span>{t('rt.memoryBudget')}</span><span class="inl"><input class="sm num" type="number" min="500" max="20000" step="500" bind:value={aForm.memory_budget} /><label class="row" style="gap: 8px; cursor: pointer"><input type="checkbox" bind:checked={aForm.reflect} /> {t('rt.reflect')}</label></span>
              <span>{t('rt.place')}<div class="hint">{t('rt.placeHint')}</div></span><input class="sm mono place" bind:value={aForm.place} placeholder="apps/site" />
              <span>{t('rt.rights')}</span><span class="stack" style="gap: 4px"><label class="row" style="gap: 8px; cursor: pointer"><input type="checkbox" class="can-mail" bind:checked={aForm.can_mail} /> {t('rt.canMail')}</label><label class="row" style="gap: 8px; cursor: pointer"><input type="checkbox" class="can-routine" bind:checked={aForm.can_routine} /> {t('rt.canRoutine')}</label></span>
              <span>{t('rt.skills')}<div class="hint">{t('rt.skillsHint')}</div></span>
              <span class="stack skills" style="gap: 4px">
                {#each data.skills as k (k.name)}<label class="row" style="gap: 8px; cursor: pointer" title={k.description}><input type="checkbox" data-skill={k.name} checked={(aForm.skills ?? []).includes(k.name)} onchange={() => toggleSkill(k.name)} /> <span class="mono">{k.name}</span><span class="muted sm"> — {k.description}</span></label>{/each}
                {#if !data.skills.length}<span class="hint">{t('rt.noSkills')}</span>{/if}
              </span>
            </div>
            <div class="acts"><button class="sm primary" disabled={busy || !aForm.name?.trim()} onclick={saveAgent}>{aForm.id ? t('common.save') : t('rt.create')}</button>{#if aForm.id}<button class="sm" onclick={assign}><Icon name="send" size="s" /> {t('rt.assign')}</button>{/if}<span class="sp"></span>{#if aForm.id}<button class="sm ghost danger" onclick={deleteAgent}>{t('rt.deleteAgent')}</button>{:else}<button class="sm ghost" onclick={() => (fresh = false)}>{t('rt.cancel')}</button>{/if}</div>
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
        {#if fresh || selRoutine}
          <div class="card-s fs rt-form">
            <h3>{rForm.id ? rForm.name : t('rt.newRoutine')}</h3>
            {#if !data.agentsList.length}<p>{t('rt.noAgentsForRoutine')}</p>{/if}
            <div class="frm">
              <span>{t('rt.routineName')}</span><input class="sm" bind:value={rForm.name} placeholder={t('rt.phRoutine')} />
              <span>{t('rt.agent')}</span><select class="sm" bind:value={rForm.agent}>{#each data.agentsList as a (a.id)}<option value={a.id}>{a.name} · {a.workspace}</option>{/each}</select>
              <span>{t('rt.prompt')}<div class="hint">{t('rt.promptHint')}</div></span><textarea bind:value={rForm.prompt} placeholder={t('rt.phPrompt')}></textarea>
              <span>{t('rt.trigger')}</span>
              <span class="stack" style="gap: 8px">
                <span class="radios">{#each [['schedule', t('rt.trSchedule')], ['webhook', t('rt.trWebhook')], ['event', t('rt.trEvent')], ['manual', t('rt.trManual')]] as [k, l] (k)}<label><input type="radio" name="trig" value={k} bind:group={rForm.trigger!.kind} /> {l}</label>{/each}</span>
                {#if rForm.trigger?.kind === 'schedule'}
                  <span class="inl"><input class="sm mono" style="width: 150px" bind:value={rForm.trigger!.cron} placeholder="0 3 * * *" /><select class="sm" value="" onchange={(e) => { const v = (e.target as HTMLSelectElement).value; if (v) rForm.trigger!.cron = v; (e.target as HTMLSelectElement).value = ''; }}><option value="">{t('rt.presets')}</option>{#each PRESETS as [c, l] (c)}<option value={c}>{l}</option>{/each}</select><span class="hint">{cronText(rForm.trigger?.cron)}{#if selRoutine?.next_at && selRoutine.trigger.cron === rForm.trigger?.cron} · {t('rt.next')} {when(selRoutine.next_at)}{/if}</span></span>
                  <span class="hint">{t('rt.cronHint')}</span>
                {:else if rForm.trigger?.kind === 'webhook'}
                  {#if rForm.token}<span class="inl"><span class="hook" title={hookUrl(rForm.token)}>{hookUrl(rForm.token)}</span><button class="xs" onclick={copyHook}><Icon name="copy" size="s" /> {t('rt.copy')}</button></span>{:else}<span class="hint">{t('rt.hookAfterSave')}</span>{/if}
                  <span class="hint">{t('rt.hookHint')}</span>
                {:else if rForm.trigger?.kind === 'event'}
                  <span class="inl"><select class="sm" bind:value={rForm.trigger!.event}>{#each EVENTS as k (k)}<option value={k}>{k || t('rt.eventAny')}</option>{/each}</select><span class="hint">{t('rt.contains')}</span><input class="sm" style="width: 180px" bind:value={rForm.trigger!.contains} placeholder={t('rt.phContains')} /></span>
                {/if}
              </span>
              <span>{t('rt.budget')}<div class="hint">{t('rt.noLimit')}</div></span><span class="inl"><input class="sm num" type="number" min="0" step="0.5" bind:value={rForm.budget!.run_usd} /> $ {t('rt.perRun')}<input class="sm num" type="number" min="0" step="1" bind:value={rForm.budget!.day_usd} /> $ {t('rt.perDay')}</span>
              <span>{t('rt.notify')}</span><span class="radios">{#each [['ask_error', t('rt.nAskError')], ['always', t('rt.nAlways')], ['never', t('rt.nNever')]] as [k, l] (k)}<label><input type="radio" name="ntf" value={k} bind:group={rForm.notify} /> {l}</label>{/each}</span>
              <span>{t('rt.routineMode')}</span><select class="sm rmode" bind:value={rForm.mode}><option value="">{t('rt.modeAsAgent')}</option><option value="ask">{t('rt.modeAsk')}</option><option value="auto">{t('rt.modeAuto')}</option><option value="bypass">{t('rt.modeBypass')}</option></select>
              <span>{t('rt.enabled')}</span><span><button class="switch" class:on={rForm.enabled} aria-pressed={!!rForm.enabled} onclick={() => (rForm.enabled = !rForm.enabled)}></button></span>
            </div>
            <div class="acts"><button class="sm primary" disabled={busy || !rForm.name?.trim() || !rForm.agent} onclick={saveRoutine}>{rForm.id ? t('common.save') : t('rt.create')}</button>{#if rForm.id}<button class="sm" onclick={() => runNow(rForm.id!)}><Icon name="play" size="s" /> {t('rt.runNow')}</button>{/if}<span class="sp"></span>{#if rForm.id}<button class="sm ghost danger" onclick={deleteRoutine}>{t('rt.deleteRoutine')}</button>{:else}<button class="sm ghost" onclick={() => (fresh = false)}>{t('rt.cancel')}</button>{/if}</div>
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
        {#if fresh || selSkill}
          <div class="card-s fs rt-form">
            <h3>{selSkill && !fresh ? selSkill.name : t('rt.newSkill')}</h3>
            <div class="frm">
              <span>{t('rt.skillName')}</span><input class="sm mono" bind:value={sForm.name} placeholder="run-tests" disabled={!!selSkill && !fresh} />
              <span>{t('rt.skillDesc')}</span><input class="sm" bind:value={sForm.description} />
              <span>{t('rt.skillBody')}<div class="hint">{t('rt.noSkillsHint')}</div></span><textarea class="mem memory" bind:value={sForm.body}></textarea>
            </div>
            <div class="acts"><button class="sm primary" disabled={busy || !sForm.name?.trim() || !sForm.body?.trim()} onclick={saveSkill}>{selSkill && !fresh ? t('common.save') : t('rt.create')}</button><span class="sp"></span>{#if selSkill && !fresh}<button class="sm ghost danger" onclick={deleteSkill}>{t('rt.deleteSkill')}</button>{:else}<button class="sm ghost" onclick={() => (fresh = false)}>{t('rt.cancel')}</button>{/if}</div>
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
