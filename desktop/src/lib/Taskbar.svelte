<script lang="ts">
  import Mark from './Mark.svelte';
  import SysIcon, { type SysIconName } from './SysIcon.svelte';
  import { wm, SYS_APPS, TASKBAR_H, winTitle, type SysApp } from './windows.svelte';
  import { widgets } from './widgets.svelte';
  import { t, tn, i18n } from './i18n.svelte';
  import type { Appearance } from './theme';
  import type { SystemStatus } from './api';
  let { appearance = $bindable(), status }: { appearance: Appearance; status: SystemStatus | null } = $props();
  let now = $state(new Date());
  $effect(() => { const t = setInterval(() => now = new Date(), 1000); return () => clearInterval(t); });
  const bad = $derived(status?.components.filter(c => c.status !== 'ok').length ?? 0);
  const pinned: SysApp[] = ['projects', 'agents', 'store', 'files', 'system', 'settings'];
  function isImg(icon?: string | null) { return !!icon && (icon.startsWith('/') || icon.startsWith('http')); }
  function toggleTheme() { appearance = { ...appearance, mode: appearance.mode === 'light' ? 'dark' : 'light' }; }
  const umbrel = $derived(appearance.look === 'umbrel');
  // the umbrel look: the dock shows the pinned system apps and the open windows of the rest; system app windows are
  // drawn with their own SysIcon, apps — with the manifest image or an emoji in a tile
  const SYS_ICON: Partial<Record<string, SysIconName>> = { projects: 'projects', store: 'store', files: 'files', system: 'system', settings: 'settings', project: 'project', vault: 'vault', updates: 'updates', setup: 'setup', appdetail: 'appdetail', reader: 'reader', appready: 'appready', agents: 'agents' };
  const others = $derived(wm.wins.filter(w => !pinned.some(p => w.id === `sys:${p}`)));
</script>

{#if umbrel}
  <nav class="dock" aria-label={t('shell.dockAria')}>
    <div class="slot"><button class="item start" class:active={wm.startOpen} onclick={() => wm.startOpen = !wm.startOpen} aria-label={t('shell.startMenu')}><span class="tile"><Mark size={30} /></span></button><span class="tip">{t('shell.startMenu')}</span></div>
    <span class="sep"></span>
    {#each pinned as app}
      {@const open = wm.wins.find(w => w.id === `sys:${app}`)}
      <div class="slot"><button class="item" class:running={!!open} class:active={open && wm.active === open.id && !open.minimized} aria-label={t(SYS_APPS[app].title)} onclick={() => open ? wm.toggle(open.id) : wm.openSys(app)}>
        <SysIcon name={app} size={50} />{#if open}<span class="run"></span>{/if}
      </button><span class="tip">{t(SYS_APPS[app].title)}</span></div>
    {/each}
    {#if others.length}<span class="sep"></span>{/if}
    {#each others as w (w.id)}
      {@const sys = w.kind === 'component' && w.component ? SYS_ICON[w.component] : undefined}
      <div class="slot"><button class="item running" class:active={wm.active === w.id && !w.minimized} class:min={w.minimized} aria-label={winTitle(w)} onclick={() => wm.toggle(w.id)}>
        {#if sys}<SysIcon name={sys} size={50} />{:else if isImg(w.icon)}<img src={w.icon} alt="" />{:else}<span class="tile emoji">{w.icon ?? '📦'}</span>{/if}<span class="run"></span>
      </button><span class="tip">{winTitle(w)}</span></div>
    {/each}
    <span class="sep"></span>
    <div class="slot"><button class="item" aria-label={t('shell.addWidget')} onclick={() => widgets.picker = true}><span class="tile plus">+</span></button><span class="tip">{t('shell.addWidget')}</span></div>
    <span class="sep"></span>
    <!-- the dock's service segment instead of the tray: core health, updates, clock — covers neither windows nor
         widgets -->
    <div class="tray" role="status" aria-label={t('shell.islandAria')}>
      <button class="ib" title={t('shell.health')} onclick={() => wm.openSys('system')}><span class="dot" class:ok={bad === 0} class:bad={bad > 0}></span><span class="label">{status ? (bad ? tn('shell.failures', bad) : t('shell.coreOk')) : '…'}</span></button>
      {#if status && (status.update_available || (status.app_updates ?? 0) > 0)}<button class="ib" title={status.update_available ? t('shell.sysUpdate') : t('shell.appUpdates', { n: status.app_updates ?? 0 })} onclick={() => wm.openSys('updates')}>⬆️</button>{/if}
      <button class="ib iclock" title={now.toLocaleDateString(i18n.locale, { weekday: 'long', day: 'numeric', month: 'long' })} onclick={() => wm.openSys('settings')}>{now.toLocaleTimeString(i18n.locale, { hour: '2-digit', minute: '2-digit' })}</button>
    </div>
  </nav>
{:else}
<nav class="taskbar" style="height: {TASKBAR_H}px" aria-label={t('shell.taskbarAria')}>
  <button class="start" class:active={wm.startOpen} onclick={() => wm.startOpen = !wm.startOpen} title={t('shell.startMenu')}>
    <span class="logo"><Mark size={18} simple /></span><span class="label">AgentVerse OS</span>
  </button>
  {#if appearance.taskbarPinned}<div class="pinned">
    {#each pinned as app}
      {@const open = wm.wins.find(w => w.id === `sys:${app}`)}
      <button class:running={!!open} class:active={open && wm.active === open.id && !open.minimized} title={SYS_APPS[app].title} onclick={() => open ? wm.toggle(open.id) : wm.openSys(app)}>
        <span class="emoji">{SYS_APPS[app].icon}</span><span class="label">{t(SYS_APPS[app].title)}</span>
      </button>
    {/each}
  </div>{/if}
  <div class="tasks">
    {#each wm.wins.filter(w => !appearance.taskbarPinned || !pinned.some(p => w.id === `sys:${p}`)) as w (w.id)}
      <button class="running" class:active={wm.active === w.id && !w.minimized} class:min={w.minimized} onclick={() => wm.toggle(w.id)} title={w.url ?? winTitle(w)}>
        {#if isImg(w.icon)}<img src={w.icon} alt="" />{:else if w.icon}<span class="emoji">{w.icon}</span>{/if}<span class="label">{winTitle(w)}</span>
      </button>
    {/each}
  </div>
  <div class="tray">
    <button class="tray-btn" title={t('shell.health')} onclick={() => wm.openSys('system')}>
      <span class="dot" class:ok={bad === 0} class:bad={bad > 0}></span><span class="label muted">{status ? (bad ? tn('shell.failures', bad) : t('shell.coreOk')) : '…'}</span>
    </button>
    {#if status && (status.update_available || (status.app_updates ?? 0) > 0)}<button class="tray-btn" title={status.update_available ? t('shell.sysUpdate') : t('shell.appUpdates', { n: status.app_updates ?? 0 })} onclick={() => wm.openSys('updates')}>⬆️</button>{/if}
    <button class="tray-btn" title={t('shell.toggleTheme')} onclick={toggleTheme}>{appearance.mode === 'light' ? '☀️' : '🌙'}</button>
    <button class="tray-btn clock" title={now.toLocaleDateString(i18n.locale, { weekday: 'long', day: 'numeric', month: 'long' })} onclick={() => wm.openSys('settings')}>
      <span>{now.toLocaleTimeString(i18n.locale, { hour: '2-digit', minute: '2-digit' })}</span>
      <span class="muted date">{now.toLocaleDateString(i18n.locale, { day: '2-digit', month: '2-digit' })}</span>
    </button>
  </div>
</nav>
{/if}

<style>
  .taskbar { position: fixed; left: 0; right: 0; bottom: 0; z-index: 2000; display: flex; align-items: center; gap: 6px; padding: 0 8px; background: var(--glass); backdrop-filter: blur(16px); -webkit-backdrop-filter: blur(16px); border-top: 1px solid var(--line); }
  .taskbar button { display: flex; align-items: center; gap: 6px; height: 36px; padding: 0 10px; background: transparent; border-color: transparent; white-space: nowrap; font-size: .85rem; }
  .taskbar button:hover { background: var(--panel-2); border-color: var(--line); }
  .taskbar button.running { border-bottom: 2px solid var(--muted); border-radius: var(--r-sm) var(--r-sm) 2px 2px; }
  .taskbar button.active { background: var(--panel-2); border-bottom-color: var(--accent); }
  .taskbar button.min { opacity: .6; }
  .taskbar img { width: 18px; height: 18px; border-radius: 4px; }
  .emoji { font-size: 16px; }
  .start { font-weight: 700; }
  .taskbar .start.active { background: var(--panel-2); border-color: var(--accent); }
  .logo { display: inline-flex; align-items: center; }
  .pinned { display: flex; gap: 2px; padding-right: 6px; border-right: 1px solid var(--line); }
  .tasks { display: flex; gap: 2px; flex: 1; overflow-x: auto; }
  .tray { display: flex; gap: 2px; align-items: center; margin-left: auto; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--muted); }
  .dot.ok { background: var(--ok); } .dot.bad { background: var(--bad); }
  .taskbar .clock { flex-direction: column; gap: 0; line-height: 1.1; padding: 0 10px; }
  .taskbar .clock .date { font-size: .7rem; }
  @media (max-width: 700px) { .label { display: none; } .clock .date { display: none; } .taskbar { gap: 2px; padding: 0 4px; } }

  /* the umbrelOS dock: glass with a highlight and a deep shadow, 50 px icons that grow on hover (the neighbours — slightly), the hint
     above the icon, a dot under an open window; the service segment on the right (core health, updates, clock) replaces the tray */
  .dock { position: fixed; left: 50%; bottom: 10px; transform: translateX(-50%); z-index: 2000; display: flex; align-items: flex-end; gap: 10px; height: 74px; padding: 0 12px 12px; max-width: calc(100vw - 24px); overflow: visible;
    border-radius: 24px; background: rgb(0 0 0 / .42); background-clip: padding-box; border: .5px solid rgb(255 255 255 / .1); backdrop-filter: blur(20px) saturate(1.2) brightness(.85); -webkit-backdrop-filter: blur(20px) saturate(1.2) brightness(.85);
    box-shadow: inset 1px 0 0 rgb(255 255 255 / .04), inset -1px 0 0 rgb(255 255 255 / .04), inset 0 1px 0 rgb(255 255 255 / .2), inset 0 .5px 0 rgb(255 255 255 / .1), 0 4px 24px rgb(0 0 0 / .56); }
  .dock .sep { width: .5px; height: 40px; margin: 0 2px 5px; background: rgb(255 255 255 / .16); flex: none; }
  .slot { position: relative; display: flex; align-items: flex-end; height: 50px; }
  .dock .item { position: relative; width: 50px; height: 50px; padding: 0; border: 0; border-radius: 15px; background: transparent; box-shadow: none; display: grid; place-items: center; transform-origin: bottom center; transition: transform .18s cubic-bezier(.2, .8, .2, 1), filter var(--dur); filter: drop-shadow(0 6px 14px rgb(0 0 0 / .45)); }
  .dock .item:hover { transform: scale(1.5) translateY(-6px); z-index: 2; background: transparent; }
  .dock .slot:has(.item:hover) + .slot .item, .dock .slot:has(+ .slot .item:hover) .item { transform: scale(1.2) translateY(-3px); }
  .dock .item:focus-visible { outline: 0; filter: drop-shadow(0 6px 14px rgb(0 0 0 / .45)) brightness(1.25); }
  .dock .item.min { opacity: .55; }
  .dock .item img { width: 50px; height: 50px; border-radius: 15px; object-fit: cover; }
  .tile { display: grid; place-items: center; width: 50px; height: 50px; border-radius: 15px; background: rgb(255 255 255 / .12); border: .5px solid rgb(255 255 255 / .18); box-shadow: inset 0 1px 0 rgb(255 255 255 / .2); font-size: 26px; }
  .tile.plus { font-size: 30px; font-weight: 300; color: rgb(255 255 255 / .85); line-height: 1; }
  .dock .item.start.active .tile { background: rgb(255 255 255 / .25); }
  .run { position: absolute; left: 50%; bottom: -9px; width: 4px; height: 4px; border-radius: 50%; background: rgb(255 255 255 / .85); transform: translateX(-50%); }
  .dock .item.active .run { width: 12px; border-radius: 2px; }
  .tip { position: absolute; left: 50%; bottom: calc(100% + 30px); transform: translateX(-50%) translateY(4px); padding: 4px 10px; border-radius: 10px; background: rgb(0 0 0 / .8); color: #fff; font-size: 12px; font-weight: 500; white-space: nowrap; opacity: 0; pointer-events: none; transition: opacity .15s, transform .15s; box-shadow: 0 4px 12px rgb(0 0 0 / .4); }
  .slot:hover .tip { opacity: 1; transform: translateX(-50%) translateY(0); }
  .dock .tray { display: flex; align-items: center; gap: 2px; height: 50px; }
  .dock .ib { display: flex; align-items: center; gap: 8px; height: 32px; padding: 0 12px; border: 0; border-radius: 999px; background: rgb(255 255 255 / .08); box-shadow: none; font-size: .85rem; color: rgb(255 255 255 / .85); white-space: nowrap; }
  .dock .ib:hover { background: rgb(255 255 255 / .16); }
  .dock .iclock { font-weight: 600; font-variant-numeric: tabular-nums; }
  @media (max-width: 700px) { .dock { gap: 4px; padding: 0 8px 8px; height: 64px; } .dock .item, .slot, .tile, .dock .item img { width: 44px; height: 44px; } .dock .sep { display: none; } .dock .ib .label { display: none; } }
</style>
