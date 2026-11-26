<script lang="ts">
  import { openUrl, sourceOf, iconOf } from './store-utils';
  import { wm, SYS_APPS, type SysApp } from './windows.svelte';
  import { widgets } from './widgets.svelte';
  import { api, type AppView, type ProjectView } from './api';
  import { t } from './i18n.svelte';
  import AppIcon, { refOf } from './AppIcon.svelte';
  import Glyph from './Glyph.svelte';
  import { device } from './device.svelte';
  let apps = $state<AppView[]>([]);
  let projects = $state<ProjectView[]>([]);
  let q = $state('');
  let input: HTMLInputElement | undefined = $state();
  $effect(() => { if (wm.startOpen) { q = ''; api.apps().then(a => apps = a).catch(() => {}); api.projects().then(p => projects = p).catch(() => {}); setTimeout(() => input?.focus(), 30); } });
  const sys = Object.entries(SYS_APPS).filter(([k]) => k !== 'appdetail' && k !== 'project' && k !== 'appready' && k !== 'reader') as [SysApp, { title: string; icon: string }][];
  const s = $derived(q.trim().toLowerCase());
  const installed = $derived(apps.filter(a => a.installed && (!s || a.name.includes(s) || (a.manifest.title ?? '').toLowerCase().includes(s))));
  const catalog = $derived(s ? apps.filter(a => !a.installed && (a.name.includes(s) || (a.manifest.title ?? '').toLowerCase().includes(s))).slice(0, 8) : []);
  const projs = $derived(projects.filter(p => !s || p.name.includes(s) || (p.spec.description ?? '').toLowerCase().includes(s)));  // the note is searched too
  const sysShown = $derived(sys.filter(([, m]) => !s || t(m.title).toLowerCase().includes(s)));
  function openApp(a: AppView) {
    if (!a.url) return;
    if (a.manifest.route.open === 'iframe') wm.open(`app:${a.name}`, a.manifest.title ?? a.name, openUrl(a), iconOf(a)); else window.open(openUrl(a), '_blank', 'noopener');
    wm.startOpen = false;
  }
  // Enter in the search field opens the first match: an installed app, then a system app, a project, an app from the Store
  function first() {
    if (!s) return;
    const a = installed.find(x => x.url);
    if (a) return openApp(a);
    if (sysShown.length) return wm.openSys(sysShown[0][0]);
    if (projs.length) return wm.openSys('project', { name: projs[0].name }, projs[0].name);
    if (catalog.length) return wm.openSys('appdetail', { name: catalog[0].name }, catalog[0].manifest.title ?? catalog[0].name);
  }
</script>

{#if wm.startOpen}
  <!-- the phone drawer fills the screen between the bars and needs no backdrop: the status bar and the navigation stay clickable -->
  {#if !device.phone}<div class="backdrop" role="presentation" onclick={() => wm.startOpen = false}></div>{/if}
  <div class="menu" class:drawer={device.phone} role="menu" aria-label={t('shell.startMenuAria')}>
    <label class="find"><Glyph name="search" size={18} /><input bind:this={input} placeholder={t('shell.searchPlaceholder')} bind:value={q} onkeydown={(e) => { if (e.key === 'Escape') wm.startOpen = false; else if (e.key === 'Enter') first(); }} /></label>
    <div class="cols">
      <div class="col">
        {#if !s || sysShown.length}<div class="muted h">{t('sys.system')}</div>{/if}
        <div class="sysgrid">
          {#each sysShown as [k, m]}
            <button class="item" onclick={() => wm.openSys(k)}><AppIcon icon={{ sys: k }} size={36} /><span class="tx">{t(m.title)}</span></button>
          {/each}
        </div>
        {#if !s || projs.length}<div class="muted h">{t('sys.projects')}</div>{/if}
        {#each projs as p}
          <button class="item" onclick={() => wm.openSys('project', { name: p.name }, p.name)} title={t('shell.wsStatus', { status: p.workspace?.status ?? '—' }) + (p.spec.description ? '\n' + p.spec.description : '')}><AppIcon icon={{ sys: 'project' }} size={28} /><span class="tx">{p.name}</span> <span class="badge {p.workspace?.status === 'running' ? 'ok' : ''}">{p.workspace?.status ?? '—'}</span>{#if p.spec.description}<span class="muted note">{p.spec.description}</span>{/if}</button>
          {#each p.workspace?.apps ?? [] as a}
            {#if p.workspace?.status === 'running'}<button class="item sub" onclick={() => { wm.open(`ws:${p.name}:${a.slug}`, `${p.name} · ${a.display_name}`, a.url, '🧩'); wm.startOpen = false; }}>↳ {a.display_name}</button>{/if}
          {/each}
        {/each}
        {#if !projs.length && !s}<div class="muted item">{t('shell.noProjects')}</div>{/if}
      </div>
      <div class="col">
        {#if !s || installed.length}<div class="muted h">{t('shell.installedApps')}</div>{/if}
        {#each installed as a}
          <button class="item" onclick={() => openApp(a)} disabled={!a.url}>
            <AppIcon icon={refOf(iconOf(a))} size={28} label={a.manifest.title ?? a.name} /><span class="tx">{a.manifest.title ?? a.name}</span>
            {#if a.state && a.state !== 'running'}<span class="badge warn">{a.state}</span>{/if}
          </button>
        {/each}
        {#if !installed.length && !s}<div class="muted item">{t('shell.nothingInstalled')}</div>{/if}
        {#if s && !installed.length && !catalog.length && !projs.length && !sysShown.length}<div class="muted item">{t('shell.nothingFound')}</div>{/if}
        {#if catalog.length}
          <div class="muted h">{t('shell.inStore')}</div>
          {#each catalog as a}<button class="item" onclick={() => wm.openSys('appdetail', { name: a.name }, a.manifest.title ?? a.name)}><AppIcon icon={refOf(iconOf(a))} size={28} label={a.manifest.title ?? a.name} /><span class="tx">{a.manifest.title ?? a.name}</span> <span class="badge src" style="color:{sourceOf(a).color}">{sourceOf(a).label}</span> <span class="muted">{t('shell.installEllipsis')}</span></button>{/each}
        {/if}
      </div>
    </div>
    <div class="foot row">
      <button class="primary" onclick={() => { widgets.picker = true; wm.startOpen = false; }}>{t('shell.addWidget')}</button>
      <button onclick={() => { wm.closeAll(); wm.startOpen = false; }}>{t('ctx.closeAll')}</button>
      <button onclick={() => location.reload()}>{t('ctx.reload')}</button>
      <span style="flex:1"></span>
      <a href="/api/ca.crt" class="muted">{t('shell.caCert')}</a>
    </div>
  </div>
{/if}

<style>
  /* The launcher: search on top, system apps and projects on the left, installed apps on the right; one column when the panel
     itself is narrow (a container query). It opens above the start button — at the left edge of the taskbar, centered in the
     umbrel look, as a full-screen drawer on the phone. */
  .backdrop { position: fixed; inset: 0; z-index: 2400; }
  .menu { position: fixed; left: max(8px, calc((100vw - var(--shelf-max)) / 2)); bottom: calc(var(--shell-b) + 6px); width: min(820px, calc(100vw - 16px)); max-height: min(78vh, 760px, calc(100dvh - var(--shell-b) - 20px)); display: flex; flex-direction: column; gap: 10px; padding: 12px;
    background: var(--pop-bg); backdrop-filter: var(--pop-blur); -webkit-backdrop-filter: var(--pop-blur); border: 1px solid var(--line-2); border-radius: var(--r-lg); box-shadow: var(--shadow-3); z-index: 2450; container: launcher / inline-size;
    transition: opacity .18s var(--ease), transform .18s var(--ease); }
  @starting-style { .menu { opacity: 0; transform: translateY(10px) scale(.985); } }
  :global(:root[data-look="umbrel"]) .menu:not(.drawer) { left: 50%; transform: translateX(-50%); bottom: calc(var(--shell-b) + 2px); border-width: .5px; border-color: rgb(255 255 255 / .12); padding: 14px; }
  @starting-style { :global(:root[data-look="umbrel"]) .menu:not(.drawer) { transform: translateX(-50%) translateY(10px) scale(.985); } }
  .find { display: flex; align-items: center; gap: 10px; padding: 0 12px; height: 42px; border-radius: var(--r-sm); background: color-mix(in oklab, var(--ink) 6%, transparent); border: 1px solid var(--line); color: var(--muted); flex: none; transition: border-color var(--dur), box-shadow var(--dur); }
  .find:focus-within { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
  .find input { flex: 1; height: 100%; padding: 0; background: transparent; border: 0; box-shadow: none; outline: 0; font-size: 1rem; border-radius: 0; }
  :global(:root[data-look="umbrel"]) .find { border-radius: 999px; padding: 0 16px; }
  :global(:root[data-look="umbrel"]) .find input { background: transparent; border: 0; padding: 0; }
  :global(:root[data-look="umbrel"]) .find input:focus-visible { outline: 0; }
  .cols { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 6px 18px; overflow: auto; min-height: 0; padding-right: 2px; align-content: start; }
  .col { min-width: 0; }
  .h { font-size: .7rem; font-weight: 600; text-transform: uppercase; letter-spacing: .08em; margin: 10px 8px 4px; }
  .col > .h:first-child { margin-top: 2px; }
  /* system apps — a grid of tiles (the label under the icon, up to two lines), everything else — rows */
  .sysgrid { display: grid; grid-template-columns: repeat(auto-fill, minmax(74px, 1fr)); gap: 2px; }
  .sysgrid .item { flex-direction: column; justify-content: flex-start; gap: 6px; min-height: 78px; padding: 8px 2px 6px; text-align: center; }
  .sysgrid .tx { white-space: normal; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; font-size: .75rem; line-height: 1.2; overflow-wrap: anywhere; }
  .item { display: flex; align-items: center; gap: 10px; width: 100%; min-height: 40px; text-align: left; background: transparent; border-color: transparent; padding: 5px 8px; border-radius: var(--r-sm); font-weight: 500; min-width: 0; }
  button.item:hover { background: var(--fill-2); border-color: transparent; }
  .item .tx { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .item.sub { min-height: 32px; padding-left: 46px; font-size: .86rem; font-weight: 400; color: var(--muted); }
  .item .note { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: .84rem; font-weight: 400; }
  .item .badge { flex: none; }
  .foot { border-top: 1px solid var(--line); padding-top: 10px; flex: none; }
  @container launcher (max-width: 600px) { .cols { grid-template-columns: minmax(0, 1fr); } }
  /* the phone: the app drawer between the status bar and the navigation */
  .menu.drawer { left: env(safe-area-inset-left, 0px); right: calc(var(--m-nav-r) + env(safe-area-inset-right, 0px)); top: calc(var(--m-status) + env(safe-area-inset-top, 0px)); bottom: calc(var(--m-nav-b) + env(safe-area-inset-bottom, 0px)); width: auto; max-height: none; border-radius: 0; border-width: 0; background: var(--bg); backdrop-filter: none; -webkit-backdrop-filter: none; }
  @starting-style { .menu.drawer { opacity: 0; transform: translateY(14px); } }
  @media (pointer: coarse) { .item { min-height: 44px; } .item.sub { min-height: 40px; } .foot button { min-height: 40px; } }
  .drawer .item { min-height: 48px; }
  .drawer .sysgrid .item { min-height: 84px; }
  .drawer .foot button { min-height: 40px; }
</style>
