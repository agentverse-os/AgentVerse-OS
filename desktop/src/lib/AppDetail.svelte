<script lang="ts">
  import { api, type AppView } from './api';
  import { wm } from './windows.svelte';
  import { sourceOf, iconOf, isImg, titleOf, openUrl, descOf, reasonOf } from './store-utils';
  import { marked } from 'marked';
  import DOMPurify from 'dompurify';
  import Dropdown from './Dropdown.svelte';
  let { name, initialTab }: { name: string; initialTab?: string } = $props();
  import AccessPanel from './AccessPanel.svelte';
  import RemoveDialog from './RemoveDialog.svelte';
  import InstallDialog from './InstallDialog.svelte';
  let installing = $state(false);
  import { needsOf } from './access';
  import { toast } from './toast.svelte';
  import { t, fmt } from './i18n.svelte';
  let removing = $state(false);
  let app = $state<AppView | null>(null);
  let error = $state('');
  let repairNote = $state('');
  let busy = $state('');
  let logs = $state('');
  let readme = $state('');
  let form = $state<Record<string, string>>({});
  let shot = $state<number | null>(null);
  let tab = $state<'about' | 'settings' | 'config' | 'tech' | 'links' | 'snapshots'>('about');
  let snaps = $state<import('./api').RestorePoint[] | null>(null);
  async function loadSnaps() { try { snaps = await api.appSnapshots(name); } catch (e) { error = String(e); snaps = []; } }
  let others = $state<AppView[]>([]);
  let linkTarget = $state('');
  $effect(() => { if (initialTab === 'settings' || initialTab === 'tech' || initialTab === 'config') tab = initialTab; });
  let ov = $state<{ env: string; compose: string; effective_compose: string } | null>(null);
  let ovDirty = $state(false);
  async function loadOv() { try { ov = await api.overrides(name); ovDirty = false; } catch (e) { error = String(e); } }
  let revealed = $state<Record<string, boolean>>({});
  async function load() {
    try {
      const all = await api.apps(); app = all.find(a => a.name === name) ?? null; others = all.filter(a => a.installed && a.name !== name);
      if (app) { form = { ...app.settings_values }; if (app.manifest.readme && !readme) readme = DOMPurify.sanitize(await marked.parse(await api.readme(name), { async: false }) as string); }
      else error = t('app.notFound', { name });
    } catch (e) { error = e instanceof Error ? e.message : String(e); }
  }
  $effect(() => { load(); });
  // результат запроса (AppView) применяем сразу — полный load() перечитывает весь каталог и приходит на секунды позже
  async function run(label: string, f: () => Promise<unknown>) { busy = label; try { const r = await f(); if (r && typeof r === 'object' && 'manifest' in r) { app = r as AppView; form = { ...app.settings_values }; } error = ''; } catch (e) { error = e instanceof Error ? e.message : String(e); } finally { busy = ''; await load(); } }
  function open(a: AppView) { if (!a.url) return; if (a.manifest.route.open === 'iframe') wm.open(`app:${a.name}`, titleOf(a), openUrl(a), iconOf(a)); else window.open(openUrl(a), '_blank', 'noopener'); }
  function key(e: KeyboardEvent) {
    const cur = shot; if (cur === null || !app) return;
    const n = app.manifest.gallery.length;
    if (e.key === 'Escape') shot = null; else if (e.key === 'ArrowRight') shot = (cur + 1) % n; else if (e.key === 'ArrowLeft') shot = (cur - 1 + n) % n;
  }
</script>

<svelte:window onkeydown={key} />
{#if error}<div class="card" style="border-color: var(--bad); margin-bottom: 10px;">{error}</div>{/if}
{#if app?.installed && app.state && ['restarting', 'down', 'exited', 'dead', 'unhealthy'].includes(app.state)}
  <div class="card" style="border-color: var(--bad); margin-bottom: 10px;">
    <b>{t('app.brokenTitle', { state: app.state ?? '' })}</b> {t('app.brokenText')}
    <div class="row" style="margin-top: 6px;">
      <button onclick={() => { tab = 'tech'; api.logs(name, 200).then(t => logs = t).catch(e => error = String(e)); }}>{t('app.showLogs')}</button>
      <button onclick={() => run(t('app.busyRepair'), async () => { const r = await api.repair(name); repairNote = r.length ? t('app.repaired', { list: r.join('; ') }) : t('app.repairNothing'); })}>{t('app.repair')}</button>
      <button onclick={() => run(t('app.busyRestart'), () => api.install(name))}>{t('app.restart')}</button>
    </div>
    {#if repairNote}<div class="muted small" style="margin-top: 4px;">{repairNote}</div>{/if}
  </div>
{/if}
{#if removing && app}<RemoveDialog {app} onclose={() => removing = false} onstart={(_, label) => busy = label} ondone={() => { removing = false; busy = ''; load(); }} />{/if}
{#if installing && app}<InstallDialog {app} onclose={() => installing = false} ondone={(v) => { installing = false; app = v; wm.openSys('appready', { name }, `${t('sys.appready')} · ${titleOf(v)}`); load(); }} />{/if}
{#if app}
  {@const src = sourceOf(app)}
  {@const ic = iconOf(app)}
  <header class="hero">
    {#if isImg(ic)}<img class="big" src={ic} alt="" />{:else}<div class="big emoji">{ic ?? '📦'}</div>{/if}
    <div class="hero-meta">
      <h2>{titleOf(app)} {#if app.installed}<span class="badge {app.state === 'running' ? 'ok' : 'warn'}">{app.state ?? t('app.installedBadge')}</span>{/if}</h2>
      <div class="muted">{descOf(app)}</div>
      {#if app.manifest.recommended}<div class="small pickreason">★ {reasonOf(app)}</div>{/if}
      <div class="chips">
        <span class="badge src" style="border-color:{src.color}; color:{src.color}">{src.label}</span>
        {#if app.manifest.recommended}<span class="badge pickb" title={t('app.pickRank', { rank: app.manifest.recommended.rank })}>{t('app.pick')}</span>{/if}
        {#if app.manifest.upstream?.version}<span class="badge">v{app.manifest.upstream.version}</span>{/if}
        {#each app.manifest.upstream?.categories ?? [] as c}<span class="badge">{c}</span>{/each}
        {#if app.manifest.type === 'system'}<span class="badge">system</span>{/if}
        {#if app.manifest.provides.length}<span class="badge ok">provides {app.manifest.provides.join(', ')}</span>{/if}
        {#if app.manifest.requires.length}<span class="badge warn">requires {app.manifest.requires.join(', ')}</span>{/if}
        {#if app.manifest.host_docker_socket}<span class="badge warn" title={t('app.dockerSock')}>docker.sock</span>{/if}
      </div>
      {#if app.installed}
        <div class="accessbox"><AccessPanel {app} compact manage onchange={(v) => { app = v; form = { ...v.settings_values }; }} /></div>
      {:else}
        {@const needs = needsOf(app)}
        {#if needs.length}<div class="muted small">{t('app.needs', { list: needs.join(' · ') })}</div>{/if}
      {/if}
      <div class="actions">
        {#if app.installed}
          {#if app.url}<button class="primary" onclick={() => open(app!)}>{t('common.open')}{app.manifest.route.open === 'newtab' ? ' ↗' : ''}</button>{/if}
          <button disabled={!!busy} onclick={() => { tab = 'tech'; api.logs(name, 200).then(t => logs = t).catch(e => error = String(e)); }}>{t('app.logs')}</button>
          <button onclick={() => wm.openSys('files', { root: 'apps', path: name }, `${t('sys.files')} · ${titleOf(app!)}`)}>{t('sys.files')}</button>
          <button class="danger" disabled={!!busy} onclick={() => removing = true}>{#if busy}<span class="spin"></span> {busy}{:else}{t('app.remove')}{/if}</button>
        {:else}
          <button class="primary" disabled={!!busy} onclick={() => { if (app!.manifest.settings.some(s => s.required)) { installing = true; } else { run(t('app.busyInstall'), () => toast.track(t('app.installingTitle', { title: titleOf(app!) }), progress => api.installWait(name, s => { busy = s; progress(s); }), v => { wm.openSys('appready', { name }, `${t('sys.appready')} · ${titleOf(v)}`); return { text: t('app.installedBadge'), action: { label: t('common.open'), run: () => open(v) } }; })); } }}>{busy || t('common.install')}</button>
          {#if app.manifest.settings.length}<button onclick={() => tab = 'settings'}>{t('app.configureFirst')}</button>{/if}
        {/if}
      </div>
    </div>
  </header>

  {#if app.manifest.gallery.length}
    <div class="gallery">
      {#each app.manifest.gallery as g, i}
        <button class="shot" onclick={() => shot = i} aria-label={t('app.shot', { n: i + 1 })}><img src={g} alt={t('app.shot', { n: i + 1 })} loading="lazy" /></button>
      {/each}
    </div>
  {/if}
  {#if shot !== null}
    <div class="lightbox" role="dialog" aria-label={t('app.shotOne')} tabindex="-1" onclick={() => shot = null} onkeydown={key}>
      <img src={app.manifest.gallery[shot]} alt="" />
      <div class="lb-nav"><span class="muted">{shot + 1} / {app.manifest.gallery.length} · {t('app.lbHint')}</span></div>
    </div>
  {/if}

  <nav class="tabs">
    <button class:active={tab === 'about'} onclick={() => tab = 'about'}>{t('app.tabAbout')}</button>
    {#if app.manifest.settings.length}<button class:active={tab === 'settings'} onclick={() => tab = 'settings'}>{t('app.tabSettings', { n: app.manifest.settings.length })}</button>{/if}
    <button class:active={tab === 'config'} onclick={() => { tab = 'config'; if (!ov) loadOv(); }}>{t('app.tabConfig')}</button>
    <button class:active={tab === 'tech'} onclick={() => tab = 'tech'}>{t('app.tabTech')}</button>
    {#if app.installed}<button class:active={tab === 'snapshots'} onclick={() => { tab = 'snapshots'; loadSnaps(); }}>{t('app.tabSnapshots')}</button>{/if}
    {#if app.installed}<button class:active={tab === 'links'} onclick={() => tab = 'links'}>{t('app.tabLinks')}{#if app.links_to.length + app.links_from.length}<span class="badge" style="margin-left:6px">{app.links_to.length + app.links_from.length}</span>{/if}</button>{/if}
  </nav>

  {#if tab === 'about'}
    {#if readme}<div class="readme">{@html readme}</div>{:else}<div class="muted">{t('app.noReadme')}</div>{/if}
    <dl class="facts">
      {#if app.manifest.upstream?.author}<dt>{t('app.author')}</dt><dd>{app.manifest.upstream.author}</dd>{/if}
      {#if app.manifest.website}<dt>{t('app.website')}</dt><dd><a href={app.manifest.website} target="_blank" rel="noopener">{app.manifest.website}</a></dd>{/if}
      {#if app.manifest.upstream?.source && app.manifest.upstream.source !== app.manifest.website}<dt>{t('app.source')}</dt><dd><a href={app.manifest.upstream.source} target="_blank" rel="noopener">{app.manifest.upstream.source}</a></dd>{/if}
      {#if app.manifest.tags.length}<dt>{t('app.tags')}</dt><dd>{app.manifest.tags.join(', ')}</dd>{/if}
      <dt>{t('app.catalog')}</dt><dd>{src.label}{#if app.manifest.upstream?.template} · {app.manifest.upstream.template}{/if}</dd>
    </dl>
    {#if app.manifest.upstream?.release_notes}<h3>{t('app.releaseNotes')}</h3><div class="muted" style="white-space: pre-wrap;">{app.manifest.upstream.release_notes}</div>{/if}
  {:else if tab === 'settings'}
    <div class="settings">
      {#each app.manifest.settings as f}
        <label class="setting">
          <span>{f.label ?? f.env} <span class="muted">{f.env}{f.required ? ' *' : ''}</span></span>
          {#if f.type === 'boolean'}<Dropdown bind:value={form[f.env]} options={[{ value: 'true', label: t('app.optYes') }, { value: 'false', label: t('app.optNo') }]} />
          {:else if f.options?.length}<Dropdown bind:value={form[f.env]} options={f.options.map(o => ({ value: o.value, label: o.label }))} />
          {:else}
            <span class="row" style="gap:4px">
              <input style="flex:1" type={(f.type === 'password' || f.type === 'random') && !revealed[f.env] ? 'password' : f.type === 'number' ? 'number' : 'text'} bind:value={form[f.env]} placeholder={f.type} />
              {#if f.type === 'password' || f.type === 'random'}
                <button type="button" title={t('app.reveal')} onclick={async () => { if (form[f.env] === '••••••') { try { form[f.env] = (await api.reveal(name, f.env)).value; } catch (e) { error = String(e); } } revealed[f.env] = !revealed[f.env]; }}>{revealed[f.env] ? '🙈' : '👁'}</button>
                <button type="button" title={t('app.copyTitle')} onclick={async () => { let v = form[f.env]; if (v === '••••••') { v = (await api.reveal(name, f.env)).value; } navigator.clipboard?.writeText(v); }}>⧉</button>
              {/if}
            </span>{/if}
          {#if f.hint}<span class="muted hint">{f.hint}</span>{/if}
        </label>
      {/each}
    </div>
    <div class="row" style="margin-top: 10px;">
      <button class="primary" disabled={!!busy} onclick={() => run(app!.installed ? t('app.busyApply') : t('app.busySave'), () => api.settings(name, form))}>{busy || (app.installed ? t('app.apply') : t('common.save'))}</button>
      <span class="muted">{t('app.secretsMasked')}</span>
    </div>
  {:else if tab === 'snapshots'}
    <p class="muted small">{t('app.snapsText')}</p>
    {#if snaps === null}<div class="muted">{t('common.loading')}</div>
    {:else if !snaps.length}<div class="card muted">{t('app.snapsEmpty')}</div>
    {:else}
      <table class="facts-table"><tbody>
        {#each snaps as s (s.snapshot)}
          <tr><td>{fmt.dateTime(s.created * 1000)}</td><td><span class="badge">{s.kind}</span></td><td><code class="small">{s.snapshot}</code></td>
            <td><button class="danger" disabled={!!busy} onclick={() => { if (confirm(t('app.restoreConfirm', { title: titleOf(app!), date: fmt.dateTime(s.created * 1000) }))) run(t('app.busyRestore'), () => api.appRestore(name, s.snapshot)); }}>{t('app.rollback')}</button></td></tr>
        {/each}
      </tbody></table>
    {/if}
  {:else if tab === 'links'}
    {@const candidates = others.filter(o => !app!.links_to.includes(o.name))}
    <p class="muted small">{t('app.linksText')}</p>
    <dl class="facts">
      <dt>{t('app.internalUrl')}</dt><dd class="row">{#if app.internal_url}<code>{app.internal_url}</code><button title={t('app.copyTitle')} onclick={() => navigator.clipboard?.writeText(app!.internal_url ?? '')}>⧉</button><span class="muted small">{t('app.internalUrlHint')}</span>{:else}—{/if}</dd>
      <dt>{t('app.linksTo')}</dt><dd>
        {#each app.links_to as p}<div class="row" style="margin-bottom:4px"><b>{p}</b><button class="danger" disabled={!!busy} onclick={() => run(t('app.busyUnlink'), () => api.unlink(name, p))}>{t('app.unlink')}</button></div>{/each}
        {#if !app.links_to.length}<span class="muted">{t('app.linksNone')}</span>{/if}
        <div class="row" style="margin-top:6px">
          <Dropdown bind:value={linkTarget} ariaLabel={t('app.provider')} options={[{ value: '', label: t('app.linkTo') }, ...candidates.map(c => ({ value: c.name, label: titleOf(c) }))]} />
          <button class="primary" disabled={!linkTarget || !!busy} onclick={() => run(t('app.busyLink'), async () => { const v = await api.link(name, linkTarget); linkTarget = ''; return v; })}>{busy || t('app.link')}</button>
        </div>
      </dd>
      {#if Object.keys(app.link_env).length}<dt>{t('app.linkEnv')}</dt><dd><pre class="envpre">{Object.entries(app.link_env).map(([k, v]) => `${k}=${v}`).join('\n')}</pre><span class="muted small">{t('app.linkEnvHint')}</span></dd>{/if}
      <dt>{t('app.linksFrom')}</dt><dd>{app.links_from.length ? app.links_from.join(', ') : '—'}</dd>
      <dt>{t('app.sharedNet')}</dt><dd class="row">
        <button class:primary={app.shared_net} disabled={!!busy} onclick={() => run(app!.shared_net ? t('app.busyLeave') : t('app.busyLink'), () => api.setShared(name, !app!.shared_net))}>{busy || (app.shared_net ? t('app.sharedLeave') : t('app.sharedJoin'))}</button>
        <span class="muted small">{t('app.sharedHint1')} <code>&lt;{t('app.sharedName')}&gt;.apps</code>{t('app.sharedHint2')} <code>{name}.apps</code></span></dd>
    </dl>
  {:else if tab === 'config'}
    {#if ov}
      <p class="muted" style="margin: 0 0 8px;">{t('app.overridesIntro')} (<code>store/{name}/user.env</code>, <code>compose.override.yaml</code>) {t('app.overridesOutro')}</p>
      <div class="cfg">
        <label>{t('app.overridesEnv')} (<code>$&#123;VAR&#125;</code>) {t('app.overridesEnvHint')}<textarea rows="8" spellcheck="false" bind:value={ov.env} oninput={() => ovDirty = true} placeholder="OPENROUTER_API_KEY=sk-…&#10;TZ=Europe/Kyiv"></textarea></label>
        <label>{t('app.overridesCompose')}<textarea rows="12" spellcheck="false" bind:value={ov.compose} oninput={() => ovDirty = true} placeholder={"services:\n  " + (app.manifest.endpoint.service) + ":\n    environment:\n      SOME_FLAG: 'true'"}></textarea></label>
      </div>
      <div class="row" style="margin-top: 8px;">
        <button class="primary" disabled={!ovDirty || !!busy} onclick={() => run(app!.installed ? t('app.busyApply') : t('app.busySave'), async () => { await api.putOverrides(name, { env: ov!.env, compose: ov!.compose }); await loadOv(); })}>{busy || (app.installed ? t('app.apply') : t('common.save'))}</button>
        <button onclick={() => wm.openSys('files', { root: 'store', path: name }, `${t('sys.files')} · store/${name}`)}>{t('app.manifestFiles')}</button>
        <button onclick={() => wm.openSys('files', { root: 'apps', path: name }, `${t('sys.files')} · ${titleOf(app!)}`)}>{t('app.dataFiles')}</button>
      </div>
      <details style="margin-top: 10px;"><summary class="muted">{t('app.effectiveCompose')}</summary><pre>{ov.effective_compose}</pre></details>
    {:else}<div class="muted">{t('common.loading')}</div>{/if}
  {:else}
    <dl class="facts">
      <dt>{t('app.openIn')}</dt><dd class="row" style="gap:6px">
        <button class:primary={app.manifest.route.open === 'iframe'} onclick={() => run(t('app.busySave'), () => api.setOpen(name, 'iframe'))}>{t('app.openIframe')}</button>
        <button class:primary={app.manifest.route.open === 'newtab'} onclick={() => run(t('app.busySave'), () => api.setOpen(name, 'newtab'))}>{t('app.openNewtab')}</button>
        <span class="muted small">{t('app.openHint')}</span></dd>
      <dt>{t('app.route')}</dt><dd>{app.manifest.route.mode}{#if app.port} · {t('app.port', { port: app.port })}{/if}{#if app.url} · <a href={app.url} target="_blank" rel="noopener">{app.url}</a>{/if}</dd>
      <dt>Endpoint</dt><dd>{app.manifest.endpoint.service}:{app.manifest.endpoint.port} {t('app.inNetwork')} <code>{app.name}-net</code></dd>
      <dt>{t('app.data')}</dt><dd><code>tank/apps/{app.name}</code></dd>
      {#if app.granted_to.length}<dt>{t('app.grantedTo')}</dt><dd>{app.granted_to.join(', ')}</dd>{/if}
      <dt>{t('app.manifest')}</dt><dd><code>store/{app.name}/manifest.yaml</code> · origin {app.manifest.origin}</dd>
    </dl>
    {#each app.manifest.notes as n}<div class="muted">· {n}</div>{/each}
    {#if logs}<h3 style="margin-top: 12px;">{t('app.logs')}</h3><pre>{logs}</pre>{/if}
  {/if}
{:else if !error}
  <div class="muted">{t('common.loading')}</div>
{/if}

<style>
  .facts-table { width: 100%; border-collapse: collapse; } .facts-table td { padding: 6px 8px; border-bottom: 1px solid var(--line); vertical-align: middle; }

  .hero { display: flex; gap: 16px; align-items: flex-start; }
  .big { width: 88px; height: 88px; border-radius: 18px; object-fit: cover; flex: none; background: var(--panel-2); }
  .big.emoji { display: flex; align-items: center; justify-content: center; font-size: 52px; }
  .hero-meta { min-width: 0; flex: 1; display: flex; flex-direction: column; gap: 6px; }
  .hero h2 { margin: 0; font-size: 1.35rem; }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; }
  .actions { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 4px; }
  .gallery { display: flex; gap: 8px; overflow-x: auto; margin: 14px 0 4px; padding-bottom: 4px; scroll-snap-type: x mandatory; }
  .shot { flex: none; padding: 0; border-radius: var(--r-sm); overflow: hidden; scroll-snap-align: start; background: var(--panel-2); }
  .shot img { display: block; height: 160px; width: auto; max-width: 320px; object-fit: cover; }
  .lightbox { position: fixed; inset: 0; z-index: 5000; background: rgb(0 0 0 / .85); display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; cursor: zoom-out; }
  .lightbox img { max-width: 96vw; max-height: 88vh; border-radius: var(--r); }
  .tabs { display: flex; gap: 4px; margin: 14px 0 10px; border-bottom: 1px solid var(--line); }
  .tabs button { background: transparent; border-color: transparent; border-radius: var(--r-sm) var(--r-sm) 0 0; color: var(--muted); }
  .tabs button.active { color: var(--ink); border-color: var(--line); border-bottom-color: var(--panel); background: var(--panel-2); }
  .readme { font-size: .95rem; line-height: 1.55; overflow-wrap: anywhere; }
  .readme :global(img) { max-width: 100%; border-radius: var(--r-sm); }
  .readme :global(h1), .readme :global(h2), .readme :global(h3) { font-size: 1.05rem; margin: 12px 0 6px; }
  .readme :global(pre) { white-space: pre-wrap; }
  .facts { display: grid; grid-template-columns: max-content 1fr; gap: 4px 14px; margin: 12px 0 0; font-size: .9rem; }
  .facts dt { color: var(--muted); } .facts dd { margin: 0; overflow-wrap: anywhere; }
  .settings { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 10px; }
  .setting { display: flex; flex-direction: column; gap: 4px; font-size: .9rem; }
  .hint { font-size: .8rem; }
  .envpre { margin: 0; font-size: .8rem; white-space: pre-wrap; word-break: break-all; background: var(--panel-2); padding: 6px 8px; border-radius: var(--r-sm); }
  .pickb { border-color: #F2B84B; color: #F2B84B; }
  .pickreason { color: #F2B84B; }
  .accessbox { margin: 4px 0 2px; padding: 8px 10px; border: 1px solid var(--line); border-radius: var(--r-sm); background: color-mix(in srgb, var(--panel) 60%, transparent); }
  .secrets { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; font-size: .85rem; }
  .secret { display: inline-flex; gap: 6px; align-items: center; background: var(--panel-2); border-radius: var(--r-sm); padding: 4px 8px; }
  .secret button { padding: 0 6px; }
  .cfg { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  .cfg label { display: flex; flex-direction: column; gap: 4px; font-size: .85rem; color: var(--muted); }
  .cfg textarea { font: .82rem/1.4 ui-monospace, Menlo, Consolas, monospace; background: var(--panel-2); color: var(--ink); border: 1px solid var(--line); border-radius: var(--r-sm); padding: 8px; resize: vertical; }
  @media (max-width: 900px) { .cfg { grid-template-columns: 1fr; } }
  @media (max-width: 700px) { .hero { flex-direction: column; } .facts { grid-template-columns: 1fr; } .actions button { min-height: 40px; flex: 1 1 auto; } .shot img { height: 120px; } }
</style>
