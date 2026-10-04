<script lang="ts">
  import { wm, SYS_APPS, type SysApp } from './windows.svelte';
  import { api, type AppView, type ProjectView } from './api';
  import { sourceOf, openUrl, descOf } from './store-utils';
  import { t } from './i18n.svelte';
  import SysIcon from './SysIcon.svelte';
  import Mark from './Mark.svelte';
  import { widgets } from './widgets.svelte';
  let { look = 'default' }: { look?: 'default' | 'umbrel' } = $props();
  let apps = $state<AppView[]>([]);
  let projects = $state<ProjectView[]>([]);
  async function load() { try { [apps, projects] = await Promise.all([api.apps(), api.projects()]); } catch {} }
  $effect(() => { load(); const t = setInterval(load, 15000); return () => clearInterval(t); });
  const sys: SysApp[] = ['projects', 'agents', 'store', 'files', 'system', 'settings'];
  function icon(a: AppView) { return a.manifest.icon?.startsWith('./') ? `/api/apps/${a.name}/icon` : (a.manifest.icon ?? '📦'); }
  function isImg(i: string) { return i.startsWith('/') || i.startsWith('http'); }
  function openApp(a: AppView) { if (!a.url) return; if (a.manifest.route.open === 'iframe') wm.open(`app:${a.name}`, a.manifest.title ?? a.name, openUrl(a), icon(a)); else window.open(openUrl(a), '_blank', 'noopener'); }
  // одинарный клик — выделить (на сенсорных экранах — открыть), двойной — открыть; как в настольных ОС
  let selected = $state<string | null>(null);
  const touch = matchMedia('(pointer: coarse)').matches;
  function act(id: string, open: () => void) { if (touch) open(); else selected = id; }
  const umbrel = $derived(look === 'umbrel');
  // приветствие по времени суток, как заголовок рабочего стола umbrelOS
  let hour = $state(new Date().getHours());
  $effect(() => { const t = setInterval(() => hour = new Date().getHours(), 60000); return () => clearInterval(t); });
  // облик umbrel: сетка по центру, но виджеты (свободный слой, по умолчанию столбик справа сверху) не должны её накрывать —
  // под столбики слева и справа резервируются отступы; пока виджет тащат, отступы не пересчитываются (иначе сетка дёргается)
  let vw = $state(innerWidth);
  $effect(() => { const h = () => vw = innerWidth; addEventListener('resize', h); return () => removeEventListener('resize', h); });
  let lastReserve = { left: 0, right: 0 };
  const reserve = $derived.by(() => {
    if (!umbrel || vw < 700) return { left: 0, right: 0 };
    if (widgets.dragging) return lastReserve;
    let left = 0, right = 0;
    for (const w of widgets.items) { if (w.x + w.w / 2 > vw / 2) right = Math.max(right, vw - w.x + 12); else left = Math.max(left, w.x + w.w + 12); }
    lastReserve = { left: Math.min(left, vw * 0.45), right: Math.min(right, vw * 0.45) };
    return lastReserve;
  });
  const greeting = $derived(hour < 5 ? t('shell.greetingNight') : hour < 12 ? t('shell.greetingMorning') : hour < 18 ? t('shell.greetingDay') : t('shell.greetingEvening'));
</script>

{#snippet sysIcons()}
  {#each sys as k}
    <button class="icon" class:selected={selected === `sys:${k}`} title={t('shell.sysIconTitle', { title: t(SYS_APPS[k].title) })} onclick={() => act(`sys:${k}`, () => wm.openSys(k))} ondblclick={() => wm.openSys(k)} onkeydown={(e) => e.key === 'Enter' && wm.openSys(k)}>
      {#if umbrel}<span class="glyph sys"><SysIcon name={k} size={52} /></span>{:else}<span class="glyph emoji">{SYS_APPS[k].icon}</span>{/if}<span class="name">{t(SYS_APPS[k].title)}</span>
    </button>
  {/each}
{/snippet}
{#snippet projectIcons()}
  {#each projects as p (p.name)}
    <button class="icon" class:selected={selected === `proj:${p.name}`} title={t('shell.projectIconTitle', { name: p.name, status: p.workspace?.status ?? '—' }) + (p.spec.description ? '\n' + p.spec.description : '')}
      onclick={() => act(`proj:${p.name}`, () => wm.openSys('project', { name: p.name }, p.name))} ondblclick={() => wm.openSys('project', { name: p.name }, p.name)} onkeydown={(e) => e.key === 'Enter' && wm.openSys('project', { name: p.name }, p.name)}>
      {#if umbrel}<span class="glyph sys"><SysIcon name="project" size={52} /><span class="state" class:ok={p.workspace?.status === 'running'} class:off={p.workspace?.status === 'stopped'}></span></span>
      {:else}<span class="glyph emoji">🧩<span class="state" class:ok={p.workspace?.status === 'running'} class:off={p.workspace?.status === 'stopped'}></span></span>{/if}<span class="name">{p.name}</span>
    </button>
  {/each}
{/snippet}
{#snippet appIcons()}
  {#each apps.filter(a => a.installed) as a (a.name)}
    <button class="icon" class:selected={selected === `app:${a.name}`} title="{a.manifest.title ?? a.name} · {sourceOf(a).label}{descOf(a) ? ' · ' + descOf(a) : ''}"
      onclick={() => act(`app:${a.name}`, () => openApp(a))} ondblclick={() => openApp(a)} onkeydown={(e) => e.key === 'Enter' && openApp(a)}>
      {#if isImg(icon(a))}<img class="glyph" src={icon(a)} alt="" />{:else}<span class="glyph emoji">{icon(a)}</span>{/if}
      <span class="name">{a.manifest.title ?? a.name}</span>
    </button>
  {/each}
{/snippet}

<div class="icons" class:umbrel role="group" aria-label={t('shell.desktopAria')} style={umbrel ? `padding-left:${28 + reserve.left}px; padding-right:${28 + reserve.right}px` : ''} onpointerdown={(e) => { if (e.target === e.currentTarget) selected = null; }}>
  {#if umbrel}
    <!-- облик umbrel: логотип, приветствие и одна общая сетка (система, проекты, приложения), как в umbrelOS -->
    <div class="hdr"><Mark size={56} /><h1>{greeting}</h1></div>
    <div class="grp">{@render sysIcons()}{@render projectIcons()}{@render appIcons()}</div>
  {:else}
    <div class="grp"><span class="gl">AgentVerse OS</span>{@render sysIcons()}</div>
    {#if projects.length}<div class="grp"><span class="gl">{t('sys.projects')}</span>{@render projectIcons()}</div>{/if}
    {#if apps.some(a => a.installed)}<div class="grp"><span class="gl">{t('shell.storeApps')}</span>{@render appIcons()}</div>{/if}
  {/if}
</div>

<style>
  .icons { position: fixed; inset: 12px 12px 60px 12px; display: flex; flex-direction: column; gap: 10px; align-items: flex-start; pointer-events: none; max-width: 60vw; }
  .grp { width: 100%; display: grid; grid-template-columns: repeat(auto-fill, 96px); grid-auto-rows: 96px; gap: 6px; justify-content: start; position: relative; padding-top: 16px; }
  .gl { position: absolute; left: 6px; top: 0; font-size: .7rem; letter-spacing: .06em; text-transform: uppercase; color: var(--muted); text-shadow: 0 1px 3px rgb(0 0 0 / .6); pointer-events: none; }
  :root[data-theme="light"] .gl { text-shadow: none; }
  @media (max-width: 700px) { .icons { inset: 8px 8px auto 8px; max-height: 214px; overflow-x: auto; grid-auto-flow: column; grid-template-rows: 96px 96px; grid-template-columns: none; grid-auto-columns: 84px; } }
  .icon { pointer-events: auto; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px; width: 96px; height: 96px; background: transparent; border-color: transparent; color: var(--ink); text-shadow: 0 1px 3px rgb(0 0 0 / .6); padding: 6px 2px; }
  :root[data-theme="light"] .icon { text-shadow: 0 1px 2px rgb(255 255 255 / .8); }
  .icon:hover { background: rgb(var(--accent-rgb) / .12); border-color: rgb(var(--accent-rgb) / .35); }
  .icon.selected { background: rgb(var(--accent-rgb) / .22); border-color: rgb(var(--accent-rgb) / .6); }
  .glyph { width: 44px; height: 44px; border-radius: 12px; object-fit: cover; position: relative; }
  .glyph.emoji { display: flex; align-items: center; justify-content: center; font-size: 30px; background: var(--glass); border: 1px solid var(--line); }
  .name { font-size: .78rem; line-height: 1.2; text-align: center; max-width: 92px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .state { position: absolute; right: -3px; bottom: -3px; width: 12px; height: 12px; border-radius: 50%; background: var(--muted); border: 2px solid var(--bg); }
  .state.ok { background: var(--ok); } .state.off { background: var(--muted); }

  /* облик umbrel: логотип и приветствие сверху, сетка по центру без подписей групп, плитки 52 px с кольцом при наведении */
  .icons.umbrel { inset: 0 0 100px 0; max-width: none; align-items: center; justify-content: flex-start; gap: 4px; padding: 3vh 28px 16px; overflow: hidden auto; scrollbar-width: none; transition: padding var(--dur); }
  .hdr { display: flex; flex-direction: column; align-items: center; gap: 8px; margin-bottom: 16px; pointer-events: none; }
  .hdr h1 { font-size: clamp(20px, 2.6vw, 36px); font-weight: 700; letter-spacing: -.03em; color: rgb(255 255 255 / .92); text-shadow: 0 2px 8px rgb(0 0 0 / .5); margin: 0; }
  .umbrel .grp { width: min(100%, 1120px); grid-template-columns: repeat(auto-fill, 112px); grid-auto-rows: 108px; gap: 6px 8px; justify-content: center; padding-top: 0; }
  .umbrel .gl { display: none; }
  .umbrel .icon { width: 112px; height: 108px; gap: 8px; border-radius: 16px; text-shadow: 0 2px 4px rgb(0 0 0 / .6); color: rgb(255 255 255 / .92); }
  .umbrel .icon:hover, .umbrel .icon.selected { background: rgb(255 255 255 / .06); border-color: rgb(255 255 255 / .12); }
  .umbrel .glyph { width: 52px; height: 52px; border-radius: 14px; transition: transform .25s cubic-bezier(.2, .8, .2, 1), box-shadow .25s; }
  .umbrel .glyph.sys { display: grid; place-items: center; background: none; border: 0; }
  .umbrel .glyph.emoji { background: rgb(255 255 255 / .1); border: .5px solid rgb(255 255 255 / .18); backdrop-filter: blur(4px); font-size: 32px; }
  .umbrel .icon:hover .glyph { transform: scale(1.1); box-shadow: 0 0 0 6px rgb(255 255 255 / .25); }
  .umbrel .name { font-size: 13px; font-weight: 500; max-width: 100px; }
  .umbrel .state { border-color: rgb(0 0 0 / .7); }
  @media (max-width: 700px) { .icons.umbrel { inset: 8px 8px auto 8px; max-height: none; overflow: auto; display: flex; flex-direction: column; padding: 4px 0 0; } .hdr { margin-bottom: 8px; } .hdr h1 { font-size: 20px; } }
</style>
