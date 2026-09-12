<script lang="ts">
  import { openUrl } from './store-utils';
  import { wm, SYS_APPS, TASKBAR_H, type SysApp } from './windows.svelte';
  import { sourceOf } from './store-utils';
  import { widgets } from './widgets.svelte';
  import { api, type AppView, type ProjectView } from './api';
  import { t } from './i18n.svelte';
  let apps = $state<AppView[]>([]);
  let projects = $state<ProjectView[]>([]);
  let q = $state('');
  let input: HTMLInputElement | undefined = $state();
  $effect(() => { if (wm.startOpen) { q = ''; api.apps().then(a => apps = a).catch(() => {}); api.projects().then(p => projects = p).catch(() => {}); setTimeout(() => input?.focus(), 30); } });
  const sys = Object.entries(SYS_APPS).filter(([k]) => k !== 'appdetail' && k !== 'project' && k !== 'appready' && k !== 'reader') as [SysApp, { title: string; icon: string }][];
  const s = $derived(q.trim().toLowerCase());
  const installed = $derived(apps.filter(a => a.installed && (!s || a.name.includes(s) || (a.manifest.title ?? '').toLowerCase().includes(s))));
  const catalog = $derived(s ? apps.filter(a => !a.installed && (a.name.includes(s) || (a.manifest.title ?? '').toLowerCase().includes(s))).slice(0, 8) : []);
  const projs = $derived(projects.filter(p => !s || p.name.includes(s)));
  const sysShown = $derived(sys.filter(([, m]) => !s || t(m.title).toLowerCase().includes(s)));
  function icon(a: AppView) { return a.manifest.icon?.startsWith('./') ? `/api/apps/${a.name}/icon` : (a.manifest.icon ?? '📦'); }
  function isImg(i: string) { return i.startsWith('/') || i.startsWith('http'); }
  function openApp(a: AppView) {
    if (!a.url) return;
    if (a.manifest.route.open === 'iframe') wm.open(`app:${a.name}`, a.manifest.title ?? a.name, openUrl(a), icon(a)); else window.open(openUrl(a), '_blank', 'noopener');
    wm.startOpen = false;
  }
</script>

{#if wm.startOpen}
  <div class="backdrop" role="presentation" onclick={() => wm.startOpen = false}></div>
  <div class="menu" style="bottom: {TASKBAR_H + 8}px" role="menu" aria-label={t('shell.startMenuAria')}>
    <input bind:this={input} placeholder={t('shell.searchPlaceholder')} bind:value={q} onkeydown={(e) => { if (e.key === 'Escape') wm.startOpen = false; }} />
    <div class="cols">
      <div>
        {#if !s || sysShown.length}<div class="muted h">{t('sys.system')}</div>{/if}
        {#each sysShown as [k, m]}
          <button class="item" onclick={() => wm.openSys(k)}><span class="emoji">{m.icon}</span>{t(m.title)}</button>
        {/each}
        {#if !s || projs.length}<div class="muted h" style="margin-top: 10px;">{t('sys.projects')}</div>{/if}
        {#each projs as p}
          <button class="item" onclick={() => wm.openSys('project', { name: p.name }, p.name)} title={t('shell.wsStatus', { status: p.workspace?.status ?? '—' })}><span class="emoji">🧩</span>{p.name} <span class="badge {p.workspace?.status === 'running' ? 'ok' : ''}">{p.workspace?.status ?? '—'}</span></button>
          {#each p.workspace?.apps ?? [] as a}
            {#if p.workspace?.status === 'running'}<button class="item sub" onclick={() => { wm.open(`ws:${p.name}:${a.slug}`, `${p.name} · ${a.display_name}`, a.url, '🧩'); wm.startOpen = false; }}>↳ {a.display_name}</button>{/if}
          {/each}
        {/each}
        {#if !projs.length && !s}<div class="muted item">{t('shell.noProjects')}</div>{/if}
      </div>
      <div>
        {#if !s || installed.length}<div class="muted h">{t('shell.installedApps')}</div>{/if}
        {#each installed as a}
          <button class="item" onclick={() => openApp(a)} disabled={!a.url}>
            {#if isImg(icon(a))}<img src={icon(a)} alt="" />{:else}<span class="emoji">{icon(a)}</span>{/if}{a.manifest.title ?? a.name}
            {#if a.state && a.state !== 'running'}<span class="badge warn">{a.state}</span>{/if}
          </button>
        {/each}
        {#if !installed.length && !s}<div class="muted item">{t('shell.nothingInstalled')}</div>{/if}
        {#if s && !installed.length && !catalog.length && !projs.length && !sysShown.length}<div class="muted item">{t('shell.nothingFound')}</div>{/if}
        {#if catalog.length}
          <div class="muted h" style="margin-top: 10px;">{t('shell.inStore')}</div>
          {#each catalog as a}<button class="item" onclick={() => wm.openSys('appdetail', { name: a.name }, a.manifest.title ?? a.name)}>{#if isImg(icon(a))}<img src={icon(a)} alt="" />{:else}<span class="emoji">{icon(a)}</span>{/if}{a.manifest.title ?? a.name} <span class="badge src" style="border-color:{sourceOf(a).color}; color:{sourceOf(a).color}">{sourceOf(a).label}</span> <span class="muted">{t('shell.installEllipsis')}</span></button>{/each}
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
  .backdrop { position: fixed; inset: 0; z-index: 2400; }
  .menu { position: fixed; left: 8px; width: min(760px, calc(100vw - 16px)); max-height: 70vh; display: flex; flex-direction: column; gap: 10px; padding: 12px; background: var(--glass); backdrop-filter: blur(20px); -webkit-backdrop-filter: blur(20px); border: 1px solid var(--line); border-radius: var(--r); box-shadow: var(--shadow); z-index: 2450; }
  .menu input { width: 100%; }
  .cols { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; overflow: auto; }
  .h { font-size: .75rem; text-transform: uppercase; letter-spacing: .06em; margin: 4px 0; }
  .item { display: flex; align-items: center; gap: 8px; width: 100%; text-align: left; background: transparent; border-color: transparent; padding: 6px 8px; }
  .item:hover { background: var(--panel-2); border-color: var(--line); }
  .item.sub { padding-left: 28px; font-size: .85rem; color: var(--muted); }
  .item img { width: 20px; height: 20px; border-radius: 4px; }
  .emoji { font-size: 16px; width: 20px; text-align: center; }
  .foot { border-top: 1px solid var(--line); padding-top: 8px; }
  @media (max-width: 700px) { .cols { grid-template-columns: 1fr; } .menu { left: 0; right: 0; top: 0; bottom: 56px !important; width: auto; max-height: none; border-radius: 0; } .item { min-height: 44px; } }
</style>
