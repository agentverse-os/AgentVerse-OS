<script lang="ts">
  import { wm, SYS_APPS, type SysApp } from './windows.svelte';
  import { api, type AppView, type ProjectView } from './api';
  import { sourceOf, openUrl, descOf } from './store-utils';
  import { t } from './i18n.svelte';
  let apps = $state<AppView[]>([]);
  let projects = $state<ProjectView[]>([]);
  async function load() { try { [apps, projects] = await Promise.all([api.apps(), api.projects()]); } catch {} }
  $effect(() => { load(); const t = setInterval(load, 15000); return () => clearInterval(t); });
  const sys: SysApp[] = ['projects', 'store', 'files', 'system', 'settings'];
  function icon(a: AppView) { return a.manifest.icon?.startsWith('./') ? `/api/apps/${a.name}/icon` : (a.manifest.icon ?? '📦'); }
  function isImg(i: string) { return i.startsWith('/') || i.startsWith('http'); }
  function openApp(a: AppView) { if (!a.url) return; if (a.manifest.route.open === 'iframe') wm.open(`app:${a.name}`, a.manifest.title ?? a.name, openUrl(a), icon(a)); else window.open(openUrl(a), '_blank', 'noopener'); }
  // одинарный клик — выделить (на сенсорных экранах — открыть), двойной — открыть; как в настольных ОС
  let selected = $state<string | null>(null);
  const touch = matchMedia('(pointer: coarse)').matches;
  function act(id: string, open: () => void) { if (touch) open(); else selected = id; }
</script>

<div class="icons" role="group" aria-label={t('shell.desktopAria')} onpointerdown={(e) => { if (e.target === e.currentTarget) selected = null; }}>
  <div class="grp"><span class="gl">AgentVerse OS</span>
    {#each sys as k}
      <button class="icon" class:selected={selected === `sys:${k}`} title={t('shell.sysIconTitle', { title: t(SYS_APPS[k].title) })} onclick={() => act(`sys:${k}`, () => wm.openSys(k))} ondblclick={() => wm.openSys(k)} onkeydown={(e) => e.key === 'Enter' && wm.openSys(k)}>
        <span class="glyph emoji">{SYS_APPS[k].icon}</span><span class="name">{t(SYS_APPS[k].title)}</span>
      </button>
    {/each}
  </div>
  {#if projects.length}<div class="grp"><span class="gl">{t('sys.projects')}</span>
    {#each projects as p (p.name)}
      <button class="icon" class:selected={selected === `proj:${p.name}`} title={t('shell.projectIconTitle', { name: p.name, status: p.workspace?.status ?? '—' })}
        onclick={() => act(`proj:${p.name}`, () => wm.openSys('project', { name: p.name }, p.name))} ondblclick={() => wm.openSys('project', { name: p.name }, p.name)} onkeydown={(e) => e.key === 'Enter' && wm.openSys('project', { name: p.name }, p.name)}>
        <span class="glyph emoji">🧩<span class="state" class:ok={p.workspace?.status === 'running'} class:off={p.workspace?.status === 'stopped'}></span></span><span class="name">{p.name}</span>
      </button>
    {/each}
  </div>{/if}
  {#if apps.some(a => a.installed)}<div class="grp"><span class="gl">{t('shell.storeApps')}</span>
    {#each apps.filter(a => a.installed) as a (a.name)}
      <button class="icon" class:selected={selected === `app:${a.name}`} title="{a.manifest.title ?? a.name} · {sourceOf(a).label}{descOf(a) ? ' · ' + descOf(a) : ''}"
        onclick={() => act(`app:${a.name}`, () => openApp(a))} ondblclick={() => openApp(a)} onkeydown={(e) => e.key === 'Enter' && openApp(a)}>
        {#if isImg(icon(a))}<img class="glyph" src={icon(a)} alt="" />{:else}<span class="glyph emoji">{icon(a)}</span>{/if}
        <span class="name">{a.manifest.title ?? a.name}</span>
      </button>
    {/each}
  </div>{/if}
</div>

<style>
  .icons { position: fixed; inset: 12px 12px 60px 12px; display: flex; flex-direction: column; gap: 10px; align-items: flex-start; pointer-events: none; max-width: 60vw; }
  .grp { width: 100%; display: grid; grid-template-columns: repeat(auto-fill, 96px); grid-auto-rows: 96px; gap: 6px; justify-content: start; position: relative; padding-top: 16px; }
  .gl { position: absolute; left: 6px; top: 0; font-size: .7rem; letter-spacing: .06em; text-transform: uppercase; color: var(--muted); text-shadow: 0 1px 3px rgb(0 0 0 / .6); pointer-events: none; }
  :root[data-theme="light"] .gl { text-shadow: none; }
  @media (max-width: 700px) { .icons { inset: 8px 8px auto 8px; max-height: 214px; overflow-x: auto; grid-auto-flow: column; grid-template-rows: 96px 96px; grid-template-columns: none; grid-auto-columns: 84px; } }
  .icon { pointer-events: auto; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px; width: 96px; height: 96px; background: transparent; border-color: transparent; color: var(--ink); text-shadow: 0 1px 3px rgb(0 0 0 / .6); }
  :root[data-theme="light"] .icon { text-shadow: 0 1px 2px rgb(255 255 255 / .8); }
  .icon:hover { background: rgb(var(--accent-rgb) / .12); border-color: rgb(var(--accent-rgb) / .35); }
  .icon.selected { background: rgb(var(--accent-rgb) / .22); border-color: rgb(var(--accent-rgb) / .6); }
  .glyph { width: 44px; height: 44px; border-radius: 12px; object-fit: cover; position: relative; }
  .glyph.emoji { display: flex; align-items: center; justify-content: center; font-size: 30px; background: var(--glass); border: 1px solid var(--line); }
  .name { font-size: .78rem; line-height: 1.2; text-align: center; max-width: 92px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .state { position: absolute; right: -3px; bottom: -3px; width: 12px; height: 12px; border-radius: 50%; background: var(--muted); border: 2px solid var(--bg); }
  .state.ok { background: var(--ok); } .state.off { background: var(--muted); }
</style>
