<script lang="ts">
  import { api, type AppView } from './api';
  import { wm } from './windows.svelte';
  import { SOURCES, sourceOf, iconOf, isImg, titleOf, openUrl, descOf, reasonOf } from './store-utils';
  import Dropdown from './Dropdown.svelte';
  import RemoveDialog from './RemoveDialog.svelte';
  import InstallDialog from './InstallDialog.svelte';
  import { needsOf } from './access';
  import { toast } from './toast.svelte';
  import { t, tn } from './i18n.svelte';
  let loaded = $state(false);
  let removing = $state<AppView | null>(null);
  let apps = $state<AppView[]>([]);
  let busy = $state<Record<string, string>>({});
  let error = $state('');
  let q = $state('');
  let category = $state('');
  let source = $state('');
  let onlyInstalled = $state(false);
  let sort = $state<'name' | 'installed' | 'source'>('installed');
  let page = $state(60);
  async function load() { try { apps = await api.apps(); error = ''; } catch (e) { error = e instanceof Error ? e.message : String(e); } finally { loaded = true; } }
  async function run(key: string, label: string, f: () => Promise<unknown>) {
    busy = { ...busy, [key]: label };
    try { await f(); error = ''; } catch (e) { error = e instanceof Error ? e.message : String(e); } finally { const { [key]: _, ...rest } = busy; busy = rest; await load(); }
  }
  $effect(() => { load(); const t = setInterval(load, 15000); return () => clearInterval(t); });
  const categories = $derived([...new Set(apps.flatMap(a => a.manifest.upstream?.categories ?? []))].sort());
  const installedCount = $derived(apps.filter(a => a.installed).length);
  const runningCount = $derived(apps.filter(a => a.installed && a.state === 'running').length);
  const stateClass = (st?: string | null) => st === 'running' ? 'ok' : !st || st === 'unknown' || st === 'deploying' || st === 'starting' ? 'warn pulse' : st === 'restarting' ? 'bad pulse' : 'bad';
  const STATE_KEY: Record<string, string> = { running: 'store.state.running', restarting: 'store.state.restarting', down: 'store.state.stopped', exited: 'store.state.stopped', dead: 'store.state.dead', unhealthy: 'store.state.unhealthy', deploying: 'store.state.deploying', starting: 'store.state.starting', unknown: 'store.state.unknown', paused: 'store.state.paused' };
  const stateLabel = (st?: string | null) => { const k = STATE_KEY[st ?? '']; return k ? t(k) : st ?? t('store.installed'); };
  const unhealthy = (st?: string | null) => !!st && ['restarting', 'down', 'exited', 'dead', 'unhealthy', 'paused'].includes(st);
  const s = $derived(q.trim().toLowerCase());
  const shown = $derived(apps
    .filter(a => !onlyInstalled || a.installed)
    .filter(a => !source || (source === 'recommended' ? !!a.manifest.recommended : a.manifest.origin === source))
    .filter(a => !category || (a.manifest.upstream?.categories ?? []).includes(category))
    .filter(a => !s || a.name.includes(s) || titleOf(a).toLowerCase().includes(s) || descOf(a).toLowerCase().includes(s) || a.manifest.provides.some(p => p.includes(s)) || a.manifest.tags.some(t => t.toLowerCase().includes(s)) || (a.manifest.upstream?.categories ?? []).some(c => c.includes(s)))
    .sort((a, b) => (source === 'recommended' ? (a.manifest.recommended?.rank ?? 999) - (b.manifest.recommended?.rank ?? 999) : 0) || (sort === 'installed' ? Number(b.installed) - Number(a.installed) : sort === 'source' ? a.manifest.origin.localeCompare(b.manifest.origin) : 0) || titleOf(a).localeCompare(titleOf(b))));
  const visible = $derived(shown.slice(0, page));
  const picks = $derived(apps.filter(a => a.manifest.recommended).sort((a, b) => (a.manifest.recommended?.rank ?? 999) - (b.manifest.recommended?.rank ?? 999)));
  const shelf = $derived(!s && !source && !category && !onlyInstalled && picks.length > 0);
  let installing = $state<AppView | null>(null);
  function install(a: AppView) {
    if (a.manifest.settings.some(s => s.required)) { installing = a; return; } // обязательные настройки — сначала мастер
    run(a.name, t('store.installing'), () => toast.track(t('store.installingTitle', { name: titleOf(a) }), progress => api.installWait(a.name, st => { busy[a.name] = st; progress(st); }), v => { wm.openSys('appready', { name: a.name }, t('store.readyTitle', { name: titleOf(v) })); return { text: t('store.installed'), action: { label: t('common.open'), run: () => open(v) } }; }));
  }
  $effect(() => { s; category; source; onlyInstalled; page = 60; });
  function details(a: AppView) { wm.openSys('appdetail', { name: a.name }, titleOf(a)); }
  function open(a: AppView) { if (!a.url) return; if (a.manifest.route.open === 'iframe') wm.open(`app:${a.name}`, titleOf(a), openUrl(a), iconOf(a)); else window.open(openUrl(a), '_blank', 'noopener'); }
</script>

<div class="head">
  <div class="row" style="justify-content: space-between;">
    <h2 style="margin:0">Store <span class="muted">{tn('store.apps', apps.length)} · {t('store.installedCount', { n: installedCount })}{#if installedCount}{' · '}{t('store.runningCount', { n: runningCount })}{/if}</span></h2>
    <label class="badge check"><input type="checkbox" bind:checked={onlyInstalled} /> {t('store.onlyInstalled')}</label>
  </div>
  <div class="filters">
    <input class="search" placeholder={t('store.searchPlaceholder')} bind:value={q} />
    <Dropdown bind:value={source} ariaLabel={t('store.source')} options={[{ value: '', label: t('store.allSources') }, { value: 'recommended', label: t('store.recommended') }, ...Object.entries(SOURCES).map(([k, v]) => ({ value: k, label: v.label }))]} />
    <Dropdown bind:value={category} ariaLabel={t('store.category')} options={[{ value: '', label: t('store.allCategories') }, ...categories.map(c => ({ value: c, label: c }))]} />
    <Dropdown bind:value={sort} ariaLabel={t('store.sort')} options={[{ value: 'installed', label: t('store.sortInstalled') }, { value: 'name', label: t('store.sortName') }, { value: 'source', label: t('store.sortSource') }]} />
  </div>
</div>
{#if error}<div class="card" style="border-color: var(--bad); margin-bottom: 12px;">{error}</div>{/if}

{#if shelf}
  <section class="picks" aria-label={t('store.recommendedAria')}>
    <div class="row" style="justify-content: space-between; margin-bottom: 6px;"><h3 class="ptitle">{t('store.recommended')}</h3><button onclick={() => source = 'recommended'}>{t('store.allPicks', { n: picks.length })}</button></div>
    <div class="shelf">
      {#each picks as a (a.name)}
        {@const ic = iconOf(a)}
        {@const psrc = sourceOf(a)}
        <article class="pick" class:installed={a.installed}>
          <button class="hit" onclick={() => details(a)} aria-label={t('store.detailsAbout', { name: titleOf(a) })}></button>
          <div class="ptop">
            {#if isImg(ic)}<img class="picon" src={ic} alt="" />{:else}<span class="picon emoji">{ic ?? '📦'}</span>{/if}
            <div style="min-width: 0"><div class="ptit"><span class="t" title={titleOf(a)}>{titleOf(a)}</span><span class="badge src psrc" style="border-color:{psrc.color}; color:{psrc.color}">{psrc.label}</span></div><div class="muted preason">{reasonOf(a)}</div></div>
          </div>
          <div class="actions">
            {#if a.installed}{#if a.url}<button class="primary" onclick={() => open(a)}>{t('common.open')}</button>{/if}<span class="badge st {stateClass(a.state)}">{stateLabel(a.state)}</span>
            {:else}<button class="primary" disabled={!!busy[a.name]} onclick={() => install(a)}>{#if busy[a.name]}<span class="spin"></span> {busy[a.name]}{:else}{t('common.install')}{/if}</button>{/if}
          </div>
        </article>
      {/each}
    </div>
  </section>
{/if}

{#if !loaded && !apps.length}
  <div class="grid" aria-busy="true">
    {#each Array(9) as _, i (i)}
      <article class="card app sk"><div class="top"><div class="skeleton icon"></div><div class="meta"><div class="skeleton" style="height: 18px; width: 55%"></div><div class="skeleton" style="height: 12px; width: 90%; margin-top: 8px"></div></div></div><div class="skeleton" style="height: 22px; width: 40%"></div><div class="actions"><div class="skeleton" style="height: 32px; width: 110px"></div><div class="skeleton" style="height: 32px; width: 100px"></div></div></article>
    {/each}
  </div>
{/if}
<div class="grid">
  {#each visible as a (a.name)}
    {@const src = sourceOf(a)}
    {@const ic = iconOf(a)}
    <article class="card app" class:installed={a.installed} class:busy={!!busy[a.name]}>
      {#if busy[a.name]}<div class="progress" aria-hidden="true"></div>{/if}
      <button class="hit" onclick={() => details(a)} aria-label={t('store.detailsAbout', { name: titleOf(a) })}></button>
      <div class="top">
        {#if isImg(ic)}<img class="icon" src={ic} alt="" loading="lazy" />{:else}<div class="icon emoji">{ic ?? '📦'}</div>{/if}
        <div class="meta">
          <div class="row" style="gap: 6px; align-items: baseline;">
            <h3 class="title">{titleOf(a)}</h3>
            {#if busy[a.name]}<span class="badge st warn pulse">{busy[a.name]}</span>{:else if a.installed}<span class="badge st {stateClass(a.state)}" title={t('store.stackState', { state: a.state ?? '—' })}>{stateLabel(a.state)}</span>{/if}
          </div>
          <div class="muted desc">{descOf(a)}</div>
        </div>
      </div>
      <div class="chips">
        <span class="badge src" style="border-color:{src.color}; color:{src.color}" title={t('store.catalogSource')}>{src.label}</span>
        {#if a.manifest.recommended}<span class="badge pickb" title={reasonOf(a)}>{t('store.recommendedBadge')}</span>{/if}
        {#if a.manifest.provides.length}<span class="badge ok">provides {a.manifest.provides.join(', ')}</span>{/if}
        {#if a.manifest.type === 'system'}<span class="badge">system</span>{/if}
        {#if a.manifest.host_docker_socket}<span class="badge warn" title={t('store.dockerSock')}>docker.sock</span>{/if}
        {#each (a.manifest.upstream?.categories ?? []).slice(0, 2) as c}<span class="badge">{c}</span>{/each}
        {#if a.manifest.gallery.length}<span class="badge" title={tn('store.screenshots', a.manifest.gallery.length)}>🖼 {a.manifest.gallery.length}</span>{/if}
        {#if a.port}<span class="badge">:{a.port}</span>{/if}
      </div>
      {#if !a.installed}{@const needs = needsOf(a, apps)}{#if needs.length}<div class="needs muted" title={t('store.needs')}>{needs.join(' · ')}</div>{/if}{/if}
      <div class="actions">
        {#if a.installed}
          {#if a.url}<button class="primary" onclick={() => open(a)}>{t('common.open')}{a.manifest.route.open === 'newtab' ? ' ↗' : ''}</button>{/if}
          {#if unhealthy(a.state)}<button class="danger" title={t('store.unhealthyHint')} onclick={() => wm.openSys('appdetail', { name: a.name, tab: 'tech' }, titleOf(a))}>{t('store.logs')}</button>{/if}
          <button onclick={() => details(a)}>{t('common.more')}</button>
                    <button class="danger" disabled={!!busy[a.name]} onclick={() => removing = a}>{#if busy[a.name]}<span class="spin"></span> {busy[a.name]}{:else}{t('store.remove')}{/if}</button>
        {:else}
          <button class="primary" disabled={!!busy[a.name]} onclick={() => install(a)}>{#if busy[a.name]}<span class="spin"></span> {busy[a.name]}{:else}{t('common.install')}{/if}</button>
          <button onclick={() => details(a)}>{t('common.more')}</button>
        {/if}
      </div>
    </article>
  {/each}
  {#if !shown.length}<div class="card muted">{t('store.nothingFound')}</div>{/if}
</div>
{#if removing}<RemoveDialog app={removing} onclose={() => removing = null} onstart={(a, label) => busy = { ...busy, [a.name]: label }} ondone={(a) => { removing = null; const { [a.name]: _, ...rest } = busy; busy = rest; load(); }} />{/if}
{#if installing}<InstallDialog app={installing} onclose={() => installing = null} ondone={(v) => { installing = null; wm.openSys('appready', { name: v.name }, t('store.readyTitle', { name: titleOf(v) })); load(); }} />{/if}
{#if shown.length > visible.length}
  <div class="row" style="justify-content: center; margin: 14px 0;"><button onclick={() => page += 60}>{t('store.showMore', { n: shown.length - visible.length })}</button></div>
{/if}

<style>
  .needs { font-size: .78rem; margin-top: 6px; }
  .card.app { animation: appear .18s ease-out; }
  .card.app.busy { border-color: color-mix(in srgb, var(--accent) 50%, var(--line)); }
  .card.app.sk { pointer-events: none; }
  .skeleton.icon { width: 44px; height: 44px; border-radius: 12px; flex: 0 0 auto; }
  @keyframes appear { from { opacity: 0; transform: translateY(4px); } }
  .pickb { border-color: #F2B84B; color: #F2B84B; }
  .picks { margin: 0 0 14px; }
  .ptitle { margin: 0; font-size: 1rem; }
  .shelf { display: flex; gap: 10px; overflow-x: auto; padding: 2px 2px 8px; scroll-snap-type: x proximity; }
  .pick { position: relative; flex: 0 0 250px; scroll-snap-align: start; background: var(--panel); border: 1px solid color-mix(in srgb, #F2B84B 45%, var(--line)); border-radius: var(--r); padding: 10px 12px; display: flex; flex-direction: column; gap: 8px; }
  .pick .hit { border-radius: var(--r); }
  .pick { min-width: 0; overflow: hidden; }
  .pick .ptop { display: flex; gap: 10px; align-items: flex-start; position: relative; pointer-events: none; min-width: 0; }
  .pick .actions { position: relative; align-items: center; }
  .picon { width: 40px; height: 40px; border-radius: 10px; object-fit: cover; flex: 0 0 auto; }
  .picon.emoji { font-size: 28px; display: grid; place-items: center; background: var(--panel-2); }
  .ptit { font-weight: 600; display: flex; align-items: center; gap: 6px; min-width: 0; }
  .ptit .t { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ptit .psrc { font-size: .7rem; padding: 0 6px; line-height: 1.4; flex: none; }
  .pick .actions { flex-wrap: nowrap; }
  .preason { font-size: .78rem; line-height: 1.3; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .head { position: sticky; top: calc(-14px * var(--sp)); z-index: 2; background: var(--glass); backdrop-filter: blur(10px); -webkit-backdrop-filter: blur(10px); margin: calc(-14px * var(--sp)) calc(-14px * var(--sp)) 10px; padding: calc(10px * var(--sp)) calc(14px * var(--sp)); border-bottom: 1px solid var(--line); }
  .filters { display: grid; grid-template-columns: 2fr 1fr 1fr 1fr; gap: 8px; margin-top: 8px; }
  .filters .search { min-width: 0; }
  .check { display: inline-flex; gap: 6px; align-items: center; padding: 4px 10px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: calc(12px * var(--sp)); }
  .app { position: relative; display: flex; flex-direction: column; gap: 8px; }
  .app.installed { border-color: color-mix(in srgb, var(--accent) 60%, var(--line)); }
  .hit { position: absolute; inset: 0; background: transparent; border: 0; border-radius: var(--r); cursor: pointer; }
  .hit:hover { background: rgb(var(--accent-rgb) / .05); }
  .top, .chips, .actions { position: relative; z-index: 1; pointer-events: none; }
  .top button, .chips .badge, .actions button, .actions a { pointer-events: auto; }
  .top { display: flex; gap: 12px; align-items: flex-start; }
  .icon { width: var(--icon, 44px); height: var(--icon, 44px); border-radius: var(--r-sm); object-fit: cover; background: var(--panel-2); flex: none; }
  .icon.emoji { display: flex; align-items: center; justify-content: center; font-size: calc(var(--icon, 44px) * .55); }
  .meta { min-width: 0; flex: 1; }
  .title { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .desc { font-size: .85rem; line-height: 1.35; max-height: 2.7em; overflow: hidden; }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; font-size: .78rem; }
  .actions { display: flex; gap: 6px; flex-wrap: wrap; align-items: center; margin-top: auto; }
  .actions > button { flex: 0 0 auto; }
  .actions > .danger { margin-left: auto; }
  .chips { align-items: center; }
  @media (max-width: 700px) {
    .filters { grid-template-columns: 1fr 1fr; }
    .filters .search { grid-column: 1 / -1; }
    .grid { grid-template-columns: 1fr; }
    .actions button { min-height: 40px; flex: 1 1 auto; }
  }
</style>
