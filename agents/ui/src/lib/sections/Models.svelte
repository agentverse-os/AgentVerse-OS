<script lang="ts">
  // Модели: расход за период (книга расходов ядра), маршруты (gate и аккаунты), таблица моделей с сортировкой, цены
  // (справочно), умолчания по агентам и лимиты расходов (настройки сервиса).
  import { ui, data, money, fmtK, MODELS_FOR } from '../state.svelte';
  import { agents, download, downloadUrl, type ModelRow, type Limit } from '../api';
  import { toast } from '../toast.svelte';
  import { t } from '../i18n';
  import Icon from '../Icon.svelte';
  import Spark from '../Spark.svelte';
  let sort = $state<{ k: keyof ModelRow; d: 1 | -1 }>({ k: 'cost_usd', d: -1 });
  let open = $state<string | null>(null);
  let agentF = $state('all');
  let q = $state('');
  let newLimit = $state<Limit>({ scope: 'total', per_day: 5, action: 'warn' });
  const m = $derived(data.models);
  const rows = $derived.by(() => {
    const list = (m?.models ?? []).filter(x => (agentF === 'all' || x.agent === agentF) && (!q.trim() || x.id.includes(q.trim())));
    return list.sort((a, b) => ((Number(a[sort.k]) || 0) - (Number(b[sort.k]) || 0)) * sort.d || a.id.localeCompare(b.id));
  });
  const byDay = $derived((m?.by_day ?? []).map(b => b.cost_usd));
  function setSort(k: keyof ModelRow) { sort = sort.k === k ? { k, d: sort.d === 1 ? -1 : 1 } : { k, d: -1 }; }
  async function setDefault(agent: string, model: string) {
    try { await agents.saveSettings({ defaults: { ...(m?.defaults ?? { claude: '', codex: '' }), [agent]: model } as any }); await data.loadModels(ui.period); toast(t('toast.saved')); } catch (e) { toast(String(e), 'bad'); }
  }
  async function saveLimits(limits: Limit[]) { try { await agents.saveSettings({ limits }); await data.loadModels(ui.period); toast(t('toast.saved')); } catch (e) { toast(String(e), 'bad'); } }
  const price = (r: ModelRow) => r.price ? (r.price.note ? r.price.note : `$${r.price.input} · $${r.price.output} · $${r.price.cache_read} · $${r.price.cache_write}`) : '—';
  const spent = (scope: string) => scope === 'total' ? (m?.total.cost_usd ?? 0) : (m?.by_project.find(b => b.key === scope)?.cost_usd ?? 0);
</script>

<main class="main">
  <div class="top">
    <button class="icon ghost burger" onclick={() => (ui.popen = !ui.popen)}><Icon name="menu" /></button>
    <div class="title"><h1>{t('models.title')}</h1></div><span class="sp"></span>
    <span class="seg">{#each [[1, t('models.today')], [7, t('models.7d')], [30, t('models.30d')]] as [d, l] (d)}<button class:active={ui.period === d} onclick={() => { ui.period = d as number; data.loadModels(d as number); }}>{l}</button>{/each}</span>
  </div>
  <div class="content">
    {#if data.error && !m}<div class="err">{data.error}</div>{/if}
    {#if m}
      <div class="kpis">
        <div class="card-s kpi"><span class="lbl">{t('models.spend')} · {m.period_days === 1 ? t('models.today') : t('models.days', { n: m.period_days })} <button class="xs ghost csv" title={t('models.csv')} onclick={() => download(downloadUrl.usageCsv(m.period_days), `usage-${m.period_days}d.csv`).then(() => toast(t('toast.downloaded', { name: `usage-${m.period_days}d.csv` }))).catch((e) => toast(t('toast.downloadFail', { msg: e instanceof Error ? e.message : String(e) }), 'bad'))}>{t('models.csv')}</button></span><div class="row"><span class="val">{money(m.total.cost_usd)}</span><span class="sp"></span>{#if byDay.length > 1}<Spark values={byDay} />{/if}</div><span class="sub">{m.total.turns} {t('models.turnsShort')} · {m.by_project.length} workspace</span></div>
        <div class="card-s kpi"><span class="lbl">{t('models.tokens')}</span><div class="row"><span class="val">{fmtK((m.total.input ?? 0) + (m.total.cache_read ?? 0))}</span></div><span class="sub">{fmtK(m.total.input)}↑ {fmtK(m.total.output)}↓ · {t('models.cacheShort')} {fmtK(m.total.cache_read)}</span></div>
        <div class="card-s kpi"><span class="lbl">{t('models.avgTurn')}</span><div class="row"><span class="val">{money(m.total.turns ? m.total.cost_usd / m.total.turns : 0)}</span></div><span class="sub">{m.models.filter(x => x.turns > 0).length} {t('models.inUse')}</span></div>
        <div class="card-s kpi"><span class="lbl">{t('models.limits')}</span><div class="row"><span class="val">{m.limits.length ? t('models.ofLimits', { a: m.limits.filter(l => spent(l.scope) < l.per_day).length, b: m.limits.length }) : '—'}</span></div><span class="sub">{m.limits.length ? m.limits.filter(l => spent(l.scope) >= l.per_day).map(l => l.scope).join(', ') || t('models.withinLimits') : t('models.noLimits')}</span></div>
      </div>
      <div class="section"><div class="h-sec">{t('models.routes')}<span class="muted" style="text-transform: none; letter-spacing: 0; font-weight: 400">{t('models.routesHint')}</span></div>
        <div class="routes">{#each m.routes as r (r.id)}<div class="card-s route"><div class="r-top"><span class="dot {r.health === 'ok' ? 'ok' : ''}"></span><b>{r.name}</b><span class="chip">{r.kind}</span></div><div class="muted sm">{r.sub ?? ''}</div>{#if r.kind !== 'gate'}<div class="muted xs">{Object.entries(m.project_defaults ?? {}).filter(([, id]) => id === r.id).map(([p]) => p).join(', ') ? t('models.defaultForList', { list: Object.entries(m.project_defaults).filter(([, id]) => id === r.id).map(([p]) => p).join(', ') }) : ''}</div>{/if}</div>{/each}</div></div>
      <div class="section"><div class="h-sec">{t('models.list')}<span class="sp"></span><div class="search hide-m" style="width: 180px"><Icon name="search" size="s" /><input class="sm" placeholder={t('models.filter')} bind:value={q} /></div><select class="sm" bind:value={agentF}><option value="all">{t('models.agentAll')}</option><option value="Claude">Claude</option><option value="Codex">Codex</option></select></div>
        <div class="tbl models"><div class="tw"><table><thead><tr><th>{t('models.model')}</th><th>{t('models.agent')}</th>
          {#each [['turns', t('models.turns')], ['input', t('models.in')], ['output', t('models.out')], ['cache_read', t('models.cache')], ['cost_usd', t('models.cost')]] as [k, l] (k)}<th class="num sort" class:sorted={sort.k === k} onclick={() => setSort(k as keyof ModelRow)}>{l}{#if sort.k === k}<Icon name={sort.d < 0 ? 'sortDesc' : 'sortAsc'} size="s" />{/if}</th>{/each}
          <th>{t('models.price')}</th><th></th></tr></thead><tbody>
          {#each rows as r (r.id)}
            <tr class="click" class:open={open === r.id} onclick={() => (open = open === r.id ? null : r.id)}><td><span class="row"><b class="mono">{r.id}</b>{#if m.defaults?.claude === r.id || m.defaults?.codex === r.id}<span class="chip accent"><Icon name="check" size="s" /> {r.agent}</span>{/if}</span></td><td><span class="row"><span class="av {r.agent.toLowerCase()}"><Icon name={r.agent === 'Codex' ? 'codex' : 'claude'} size="s" /></span>{r.agent}</span></td><td class="num">{r.turns}</td><td class="num">{fmtK(r.input)}</td><td class="num">{fmtK(r.output)}</td><td class="num">{fmtK(r.cache_read)}</td><td class="num"><b>{money(r.cost_usd)}</b></td><td class="muted sm">{price(r)}</td><td><Icon name={open === r.id ? 'chevD' : 'chevR'} size="s" /></td></tr>
            {#if open === r.id}<tr><td colspan="9" style="white-space: normal; padding: 6px 10px 14px"><div class="detail"><div class="row wrap"><b class="mono">{r.id}</b><span class="chip">{r.agent}</span><span class="sp"></span>{#if r.agent === 'Claude' || r.agent === 'Codex'}<button class="sm {m.defaults?.[r.agent.toLowerCase() as 'claude' | 'codex'] === r.id ? 'primary' : ''}" onclick={() => setDefault(r.agent.toLowerCase() as 'claude' | 'codex', r.id)}>{m.defaults?.[r.agent.toLowerCase() as 'claude' | 'codex'] === r.id ? '✓ ' + t('models.default') + ' ' + r.agent : t('models.makeDefault') + ' ' + r.agent}</button>{/if}</div>
              <div class="kv2"><span>{t('models.price')}</span><span>{price(r)} <span class="muted xs">· {t('models.priceHint')}</span></span><span>{t('models.turns')}</span><span class="num">{r.turns || t('models.noUsage')}</span></div></div></td></tr>{/if}
          {/each}
        </tbody></table></div></div>
        <div class="mcards">{#each rows as r (r.id)}<div class="card-s mcard"><div class="m-top"><span class="av {r.agent.toLowerCase()}"><Icon name={r.agent === 'Codex' ? 'codex' : 'claude'} size="s" /></span><b class="mono ell">{r.id}</b><span class="sp"></span>{#if m.defaults?.claude === r.id || m.defaults?.codex === r.id}<span class="chip accent"><Icon name="check" size="s" /></span>{/if}</div><div class="m-stats"><span>{t('models.turns')}<b>{r.turns}</b></span><span>{t('models.tokens')}<b>{fmtK(r.input)}↑</b></span><span>{t('models.spend')}<b>{money(r.cost_usd)}</b></span></div><div class="muted xs">{price(r)}</div></div>{/each}</div>
      </div>
      <div class="two section">
        <div class="card-s fs"><h3>{t('models.defaults')}</h3><p>{t('models.defaultsHint')}</p><div class="frm">
          <span>Claude</span><select class="sm" value={m.defaults?.claude ?? ''} onchange={(e) => setDefault('claude', (e.target as HTMLSelectElement).value)}><option value="">{t('agents.modelDefault')}</option>{#each MODELS_FOR.claude as x (x)}<option value={x}>{x}</option>{/each}</select>
          <span>Codex</span><select class="sm" value={m.defaults?.codex ?? ''} onchange={(e) => setDefault('codex', (e.target as HTMLSelectElement).value)}><option value="">{t('agents.modelDefault')}</option>{#each MODELS_FOR.codex as x (x)}<option value={x}>{x}</option>{/each}</select>
        </div></div>
        <div class="card-s fs"><h3>{t('models.limits')}</h3><p>{t('models.limitsHint')}</p>
          <div class="stack" style="gap: 10px">
            {#each m.limits as l, i (l.scope + i)}
              {@const p = Math.min(100, l.per_day ? (spent(l.scope) / l.per_day) * 100 : 0)}
              <div class="row"><span style="width: 96px" class:b={l.scope === 'total'}>{l.scope === 'total' ? t('models.all') : l.scope}</span><span class="bar {p >= 100 ? 'bad' : p > 80 ? 'warn' : ''}"><i style="width: {p}%"></i></span><span class="num sm" style="width: 120px; text-align: right">{money(spent(l.scope))} / {money(l.per_day)}</span><span class="muted xs hide-m">{l.action === 'stop' ? t('models.stop') : t('models.warn')}</span><button class="icon xs ghost" onclick={() => saveLimits(m.limits.filter((_, j) => j !== i))}><Icon name="x" size="s" /></button></div>
            {/each}
            <div class="row wrap"><select class="sm" bind:value={newLimit.scope}><option value="total">{t('models.all')}</option>{#each data.projects as p (p.name)}<option value={p.name}>{p.name}</option>{/each}</select><span class="money"><i>$</i><input class="sm num" type="number" min="0" step="0.5" bind:value={newLimit.per_day} /></span><span class="muted sm">{t('models.perDay')}</span><select class="sm" bind:value={newLimit.action}><option value="warn">{t('models.warn')}</option><option value="stop">{t('models.stop')}</option></select><button class="sm" onclick={() => saveLimits([...m.limits.filter(l => l.scope !== newLimit.scope), { ...newLimit }])}><Icon name="plus" size="s" /> {t('models.add')}</button></div>
          </div>
        </div>
      </div>
    {:else}<div class="muted sm">{t('common.loading')}</div>{/if}
  </div>
</main>
