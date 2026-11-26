<script lang="ts">
  import { wm, SYS_APPS, type SysApp } from './windows.svelte';
  import { api, type AppView, type ProjectView } from './api';
  import { sourceOf, openUrl, descOf, iconOf } from './store-utils';
  import { t } from './i18n.svelte';
  import AppIcon, { refOf } from './AppIcon.svelte';
  import Mark from './Mark.svelte';
  import { widgets } from './widgets.svelte';
  let { look = 'default' }: { look?: 'default' | 'umbrel' } = $props();
  let apps = $state<AppView[]>([]);
  let projects = $state<ProjectView[]>([]);
  async function load() { try { [apps, projects] = await Promise.all([api.apps(), api.projects()]); } catch {} }
  $effect(() => { load(); const t = setInterval(load, 15000); return () => clearInterval(t); });
  const sys: SysApp[] = ['projects', 'agents', 'store', 'files', 'system', 'settings'];
  function openApp(a: AppView) { if (!a.url) return; if (a.manifest.route.open === 'iframe') wm.open(`app:${a.name}`, a.manifest.title ?? a.name, openUrl(a), iconOf(a)); else window.open(openUrl(a), '_blank', 'noopener'); }
  // a single click — select (on touch screens — open), a double click — open; as in desktop OSes
  let selected = $state<string | null>(null);
  const touch = matchMedia('(pointer: coarse)').matches;
  function act(id: string, open: () => void) { if (touch) open(); else selected = id; }
  const umbrel = $derived(look === 'umbrel');
  // a greeting by time of day, like the umbrelOS desktop heading
  let hour = $state(new Date().getHours());
  $effect(() => { const t = setInterval(() => hour = new Date().getHours(), 60000); return () => clearInterval(t); });
  // the widgets (a free layer, a column on the top right by default) must not cover the icons: in the umbrel look the centered
  // grid keeps margins for the widget columns on the left and right, in the regular look the icon area ends before the right
  // column; while a widget is being dragged the margins are not recomputed (otherwise the grid jitters)
  let lastReserve = { left: 0, right: 0 };
  const reserve = $derived.by(() => {
    const vw = wm.vw;
    if (vw < 700) return { left: 0, right: 0 };
    if (widgets.dragging) return lastReserve;
    let left = 0, right = 0;
    for (const w of widgets.fitted.values()) { if (w.x + w.w / 2 > vw / 2) right = Math.max(right, vw - w.x + 12); else left = Math.max(left, w.x + w.w + 12); }
    lastReserve = { left: Math.min(left, vw * 0.45), right: Math.min(right, vw * 0.45) };
    return lastReserve;
  });
  const greeting = $derived(hour < 5 ? t('shell.greetingNight') : hour < 12 ? t('shell.greetingMorning') : hour < 18 ? t('shell.greetingDay') : t('shell.greetingEvening'));
  const wsState = (p: ProjectView) => p.workspace?.status === 'running' ? 'ok' : 'off';
</script>

{#snippet sysIcons()}
  {#each sys as k}
    <button class="icon" class:selected={selected === `sys:${k}`} title={t('shell.sysIconTitle', { title: t(SYS_APPS[k].title) })} onclick={() => act(`sys:${k}`, () => wm.openSys(k))} ondblclick={() => wm.openSys(k)} onkeydown={(e) => e.key === 'Enter' && wm.openSys(k)}>
      <span class="glyph sys"><AppIcon icon={{ sys: k }} size="var(--isz)" /></span><span class="name">{t(SYS_APPS[k].title)}</span>
    </button>
  {/each}
{/snippet}
{#snippet projectIcons()}
  {#each projects as p (p.name)}
    <button class="icon" class:selected={selected === `proj:${p.name}`} title={t('shell.projectIconTitle', { name: p.name, status: p.workspace?.status ?? '—' }) + (p.spec.description ? '\n' + p.spec.description : '')}
      onclick={() => act(`proj:${p.name}`, () => wm.openSys('project', { name: p.name }, p.name))} ondblclick={() => wm.openSys('project', { name: p.name }, p.name)} onkeydown={(e) => e.key === 'Enter' && wm.openSys('project', { name: p.name }, p.name)}>
      <span class="glyph sys"><AppIcon icon={{ sys: 'project' }} size="var(--isz)" status={wsState(p)} /></span><span class="name">{p.name}</span>
    </button>
  {/each}
{/snippet}
{#snippet appIcons()}
  {#each apps.filter(a => a.installed) as a (a.name)}
    <button class="icon" class:selected={selected === `app:${a.name}`} title="{a.manifest.title ?? a.name} · {sourceOf(a).label}{descOf(a) ? ' · ' + descOf(a) : ''}"
      onclick={() => act(`app:${a.name}`, () => openApp(a))} ondblclick={() => openApp(a)} onkeydown={(e) => e.key === 'Enter' && openApp(a)}>
      <span class="glyph"><AppIcon icon={refOf(iconOf(a))} size="var(--isz)" label={a.manifest.title ?? a.name} /></span>
      <span class="name">{a.manifest.title ?? a.name}</span>
    </button>
  {/each}
{/snippet}

<div class="icons" class:umbrel role="group" aria-label={t('shell.desktopAria')} style={umbrel ? `padding-left:${28 + reserve.left}px; padding-right:${28 + reserve.right}px` : `--res-r:${reserve.right}px`} onpointerdown={(e) => { if (e.target === e.currentTarget) selected = null; }}>
  {#if umbrel}
    <!-- the umbrel look: the logo, the greeting and one shared grid (system, projects, apps), as in umbrelOS -->
    <div class="hdr"><Mark size={56} /><h1>{greeting}</h1></div>
    <div class="grp">{@render sysIcons()}{@render projectIcons()}{@render appIcons()}</div>
  {:else}
    <div class="grp"><span class="gl">AgentVerse OS</span>{@render sysIcons()}</div>
    {#if projects.length}<div class="grp"><span class="gl">{t('sys.projects')}</span>{@render projectIcons()}</div>{/if}
    {#if apps.some(a => a.installed)}<div class="grp"><span class="gl">{t('shell.storeApps')}</span>{@render appIcons()}</div>{/if}
  {/if}
</div>

<style>
  /* The desktop: icons in a fluid grid. The cell is derived from the icon size (--icon: the "icon size" setting), so the grid keeps
     its proportions at any size; a long list scrolls inside the desktop, never under the taskbar. */
  .icons { --isz: var(--icon); --cell: calc(var(--isz) + 44px); position: fixed; inset: 14px auto calc(var(--shell-b) + 10px) 14px; width: min(58vw, 760px, 100vw - var(--res-r, 0px) - 20px); display: flex; flex-direction: column; gap: 14px; align-items: flex-start; overflow: hidden auto; scrollbar-width: none; overscroll-behavior: contain; }
  .icons::-webkit-scrollbar { display: none; }
  .grp { width: 100%; display: grid; grid-template-columns: repeat(auto-fill, var(--cell)); grid-auto-rows: calc(var(--isz) + 50px); gap: 4px; justify-content: start; position: relative; padding-top: 20px; }
  .gl { position: absolute; left: 8px; top: 0; font-size: .68rem; font-weight: 600; letter-spacing: .09em; text-transform: uppercase; color: color-mix(in oklab, var(--ink) 62%, transparent); text-shadow: 0 1px 3px rgb(0 0 0 / .6); pointer-events: none; }
  :global(:root[data-theme="light"]) .gl { text-shadow: none; }
  .icon { display: flex; flex-direction: column; align-items: center; justify-content: flex-start; gap: 7px; width: var(--cell); padding: 8px 4px 4px; background: transparent; border-color: transparent; border-radius: var(--r-sm); color: var(--ink); text-shadow: 0 1px 3px rgb(0 0 0 / .65); --icon-ring: rgb(10 12 16 / .7); }
  :global(:root[data-theme="light"]) .icon { text-shadow: 0 1px 2px rgb(255 255 255 / .85); --icon-ring: rgb(255 255 255 / .9); }
  .icon:hover { background: color-mix(in oklab, var(--ink) 9%, transparent); border-color: color-mix(in oklab, var(--ink) 12%, transparent); }
  .icon.selected { background: var(--accent-soft); border-color: var(--accent-line); }
  .icon:active { transform: scale(.97); }
  .glyph { display: grid; place-items: center; width: var(--isz); height: var(--isz); flex: none; filter: drop-shadow(0 4px 10px rgb(0 0 0 / .32)); transition: transform .22s var(--ease); }
  .icon:hover .glyph { transform: translateY(-2px); }
  .name { font-size: .78rem; font-weight: 500; line-height: 1.22; text-align: center; max-width: 100%; overflow: hidden; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow-wrap: anywhere; text-wrap: balance; }

  /* the umbrel look: the logo and greeting on top, the grid centered without group labels, bigger tiles with a ring on hover */
  .icons.umbrel { --isz: calc(var(--icon) + 4px); --cell: calc(var(--isz) + 60px); inset: 0 0 var(--shell-b) 0; width: auto; align-items: center; justify-content: flex-start; gap: 4px; padding: clamp(14px, 3.4vh, 40px) 28px 16px; transition: padding var(--dur); }
  .hdr { display: flex; flex-direction: column; align-items: center; gap: 8px; margin-bottom: clamp(8px, 2vh, 20px); pointer-events: none; }
  .hdr h1 { font-size: clamp(1.35rem, 1rem + 1.5vw, 2.4rem); font-weight: 700; letter-spacing: -.03em; color: rgb(255 255 255 / .94); text-shadow: 0 2px 8px rgb(0 0 0 / .5); margin: 0; }
  .umbrel .grp { width: min(100%, 1180px); grid-auto-rows: calc(var(--isz) + 54px); gap: 6px 4px; justify-content: center; padding-top: 0; }
  .umbrel .gl { display: none; }
  .umbrel .icon { gap: 9px; padding-top: 10px; border-radius: 18px; text-shadow: 0 2px 4px rgb(0 0 0 / .6); color: rgb(255 255 255 / .94); --icon-ring: rgb(0 0 0 / .7); }
  .umbrel .icon:hover, .umbrel .icon.selected { background: rgb(255 255 255 / .07); border-color: rgb(255 255 255 / .12); }
  .umbrel .glyph { border-radius: 24%; transition: transform .25s var(--ease), box-shadow .25s; }
  .umbrel .icon:hover .glyph { transform: scale(1.08); box-shadow: 0 0 0 5px rgb(255 255 255 / .22); }
  .umbrel .name { font-size: .86rem; font-weight: 500; }
  @media (max-height: 560px) { .icons.umbrel .hdr :global(svg) { display: none; } }
</style>
