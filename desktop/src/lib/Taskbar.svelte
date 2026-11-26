<script lang="ts">
  import Mark from './Mark.svelte';
  import AppIcon, { winIcon } from './AppIcon.svelte';
  import Glyph from './Glyph.svelte';
  import { wm, SYS_APPS, winTitle, type SysApp } from './windows.svelte';
  import { widgets } from './widgets.svelte';
  import { t, tn, i18n } from './i18n.svelte';
  import type { Appearance } from './theme';
  import type { SystemStatus } from './api';
  let { appearance = $bindable(), status }: { appearance: Appearance; status: SystemStatus | null } = $props();
  let now = $state(new Date());
  $effect(() => { const t = setInterval(() => now = new Date(), 1000); return () => clearInterval(t); });
  const bad = $derived(status?.components.filter(c => c.status !== 'ok').length ?? 0);
  const pinned: SysApp[] = ['projects', 'agents', 'store', 'files', 'system', 'settings'];
  function toggleTheme() { appearance = { ...appearance, mode: appearance.mode === 'light' ? 'dark' : 'light' }; }
  const umbrel = $derived(appearance.look === 'umbrel');
  const hasUpdates = $derived(!!status && (status.update_available || (status.app_updates ?? 0) > 0));
  const updTitle = $derived(status?.update_available ? t('shell.sysUpdate') : t('shell.appUpdates', { n: status?.app_updates ?? 0 }));
  const healthText = $derived(status ? (bad ? tn('shell.failures', bad) : t('shell.coreOk')) : '…');
  const time = $derived(now.toLocaleTimeString(i18n.locale, { hour: '2-digit', minute: '2-digit' }));
  const dateLong = $derived(now.toLocaleDateString(i18n.locale, { weekday: 'long', day: 'numeric', month: 'long' }));
  // the umbrel look: the dock shows the pinned system apps and the open windows of the rest
  const others = $derived(wm.wins.filter(w => !pinned.some(p => w.id === `sys:${p}`)));
  // the dock never leaves the screen: the icons get smaller on a narrow screen, and if the open windows still do not fit,
  // the dock scrolls sideways (then without the hover magnification — it would be clipped)
  const d = $derived(wm.vw < 820 ? 40 : wm.vw < 1000 ? 44 : 50);
  let dock: HTMLElement | undefined = $state();
  let scroll = $state(false);
  $effect(() => { void wm.vw; void others.length; void hasUpdates; void d; const el = dock; if (!el) return; scroll = false; queueMicrotask(() => { scroll = el.scrollWidth > el.clientWidth + 1; }); });
</script>

{#if umbrel}
  <nav class="dock" class:scroll bind:this={dock} style="--d:{d}px" aria-label={t('shell.dockAria')}>
    <div class="slot"><button class="item start" class:active={wm.startOpen} onclick={() => wm.startOpen = !wm.startOpen} aria-label={t('shell.startMenu')}><span class="tile"><Mark size={Math.round(d * .6)} /></span></button><span class="tip">{t('shell.startMenu')}</span></div>
    <span class="sep"></span>
    {#each pinned as app}
      {@const open = wm.wins.find(w => w.id === `sys:${app}`)}
      <div class="slot"><button class="item" class:running={!!open} class:active={open && wm.active === open.id && !open.minimized} aria-label={t(SYS_APPS[app].title)} onclick={() => open ? wm.toggle(open.id) : wm.openSys(app)}>
        <AppIcon icon={{ sys: app }} size={d} />{#if open}<span class="run"></span>{/if}
      </button><span class="tip">{t(SYS_APPS[app].title)}</span></div>
    {/each}
    {#if others.length}<span class="sep"></span>{/if}
    {#each others as w (w.id)}
      <div class="slot"><button class="item running" class:active={wm.active === w.id && !w.minimized} class:min={w.minimized} aria-label={winTitle(w)} onclick={() => wm.toggle(w.id)}>
        <AppIcon icon={winIcon(w)} size={d} label={winTitle(w)} /><span class="run"></span>
      </button><span class="tip">{winTitle(w)}</span></div>
    {/each}
    <span class="sep"></span>
    <div class="slot addw"><button class="item" aria-label={t('shell.addWidget')} onclick={() => widgets.picker = true}><span class="tile plus"><Glyph name="plus" size={Math.round(d * .46)} stroke={2} /></span></button><span class="tip">{t('shell.addWidget')}</span></div>
    <span class="sep addw"></span>
    <!-- the dock's service segment instead of the tray: core health, updates, clock — covers neither windows nor
         widgets -->
    <div class="tray" role="status" aria-label={t('shell.islandAria')}>
      <button class="ib" title={t('shell.health')} onclick={() => wm.openSys('system')}><span class="dot" class:ok={bad === 0} class:bad={bad > 0}></span><span class="label">{healthText}</span></button>
      {#if hasUpdates}<button class="ib upd" title={updTitle} aria-label={updTitle} onclick={() => wm.openSys('updates')}><Glyph name="update" size={17} /></button>{/if}
      <button class="ib iclock" title={dateLong} onclick={() => wm.openSys('settings')}>{time}</button>
    </div>
  </nav>
{:else}
<nav class="taskbar" aria-label={t('shell.taskbarAria')}>
  <button class="start" class:active={wm.startOpen} onclick={() => wm.startOpen = !wm.startOpen} title={t('shell.startMenu')} aria-label={t('shell.startMenu')}>
    <span class="logo"><Mark size={22} simple /></span><span class="label">AgentVerse OS</span>
  </button>
  {#if appearance.taskbarPinned}<div class="pinned">
    {#each pinned as app}
      {@const open = wm.wins.find(w => w.id === `sys:${app}`)}
      <button class:running={!!open} class:active={open && wm.active === open.id && !open.minimized} title={t(SYS_APPS[app].title)} aria-label={t(SYS_APPS[app].title)} onclick={() => open ? wm.toggle(open.id) : wm.openSys(app)}>
        <AppIcon icon={{ sys: app }} size={26} /><span class="label">{t(SYS_APPS[app].title)}</span>
      </button>
    {/each}
  </div>{/if}
  <div class="tasks">
    {#each wm.wins.filter(w => !appearance.taskbarPinned || !pinned.some(p => w.id === `sys:${p}`)) as w (w.id)}
      <button class="running" class:active={wm.active === w.id && !w.minimized} class:min={w.minimized} onclick={() => wm.toggle(w.id)} title={w.url ?? winTitle(w)}>
        <AppIcon icon={winIcon(w)} size={24} label={winTitle(w)} /><span class="label">{winTitle(w)}</span>
      </button>
    {/each}
  </div>
  <div class="tray">
    <button class="tray-btn health" title={t('shell.health')} onclick={() => wm.openSys('system')}>
      <span class="dot" class:ok={bad === 0} class:bad={bad > 0}></span><span class="label muted">{healthText}</span>
    </button>
    {#if hasUpdates}<button class="tray-btn sq" title={updTitle} aria-label={updTitle} onclick={() => wm.openSys('updates')}><Glyph name="update" size={19} /><span class="pip"></span></button>{/if}
    <button class="tray-btn sq" title={t('shell.toggleTheme')} aria-label={t('shell.toggleTheme')} onclick={toggleTheme}><Glyph name={appearance.mode === 'light' ? 'sun' : 'moon'} size={19} /></button>
    <button class="tray-btn clock" title={dateLong} onclick={() => wm.openSys('settings')}>
      <span>{time}</span>
      <span class="muted date">{now.toLocaleDateString(i18n.locale, { day: '2-digit', month: '2-digit' })}</span>
    </button>
  </div>
</nav>
{/if}

<style>
  /* The taskbar is drawn from the --shelf-* tokens: attached to the bottom edge on flat surfaces, a floating shelf on glass
     (never wider than --shelf-max, so on an ultrawide screen the start button and the clock stay within reach). The labels give
     way to icons when the bar itself gets narrow (a container query), not the screen. */
  .taskbar { position: fixed; inset-inline: 0; bottom: var(--tb-gap); z-index: 2000; margin-inline: auto; width: min(100% - var(--tb-gap) * 2, var(--shelf-max)); height: var(--tb-h);
    display: flex; align-items: center; gap: 4px; padding: 0 6px; background: var(--shelf-bg); backdrop-filter: var(--shelf-blur); -webkit-backdrop-filter: var(--shelf-blur);
    border: 1px solid var(--shelf-line); border-radius: var(--shelf-r); box-shadow: var(--shelf-shadow); container: shelf / inline-size; }
  :global(:root[data-surface="glass"]) .taskbar { padding: 0 7px; }
  :global(:root:not([data-surface="glass"])) .taskbar { border-width: 1px 0 0; }
  .taskbar button { position: relative; display: flex; align-items: center; gap: 8px; height: calc(var(--tb-h) - 12px); min-width: calc(var(--tb-h) - 12px); padding: 0 10px; background: transparent; border-color: transparent; border-radius: max(var(--r-xs), calc(var(--shelf-r) - 7px)); white-space: nowrap; font-size: .86rem; flex: none; }
  .taskbar button:hover { background: var(--fill); border-color: transparent; }
  .taskbar button:active { transform: scale(.97); }
  .taskbar .label { overflow: hidden; text-overflow: ellipsis; }
  /* an open window: a short line under the icon, the active one — accent and wider */
  .taskbar button.running::after { content: ""; position: absolute; left: 50%; bottom: 1px; width: 6px; height: 3px; border-radius: 3px; background: var(--muted); transform: translateX(-50%); transition: width var(--dur) var(--ease), background var(--dur); }
  .taskbar button.active { background: var(--fill-2); }
  .taskbar button.active::after { width: 18px; background: var(--accent); }
  .taskbar button.min { opacity: .62; }
  .start { font-weight: 700; letter-spacing: -.01em; }
  .taskbar .start.active { background: var(--accent-soft); }
  .logo { display: inline-flex; align-items: center; }
  .pinned { display: flex; gap: 2px; padding-right: 6px; margin-right: 2px; border-right: 1px solid var(--line); flex: none; }
  .pinned button { padding: 0 6px; }
  .pinned .label { display: none; }
  .tasks { display: flex; gap: 2px; flex: 1; min-width: 0; overflow-x: auto; scrollbar-width: none; }
  .tasks::-webkit-scrollbar { display: none; }
  .tasks button { max-width: 200px; }
  .tray { display: flex; gap: 2px; align-items: center; margin-left: auto; flex: none; }
  .tray-btn.sq { padding: 0; justify-content: center; }
  .pip { position: absolute; top: 7px; right: 7px; width: 7px; height: 7px; border-radius: 50%; background: var(--accent); box-shadow: 0 0 0 2px var(--panel); }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--muted); flex: none; }
  .dot.ok { background: var(--ok); box-shadow: 0 0 0 3px color-mix(in oklab, var(--ok) 22%, transparent); } .dot.bad { background: var(--bad); box-shadow: 0 0 0 3px color-mix(in oklab, var(--bad) 22%, transparent); }
  .taskbar .clock { flex-direction: column; align-items: flex-end; justify-content: center; gap: 0; line-height: 1.15; padding: 0 10px; font-variant-numeric: tabular-nums; font-weight: 600; }
  .taskbar .clock .date { font-size: .7rem; font-weight: 400; }
  @container shelf (max-width: 1000px) { .start .label { display: none; } .start { padding: 0; justify-content: center; } }
  @container shelf (max-width: 860px) { .tasks .label, .health .label { display: none; } .tasks button, .health { padding: 0; justify-content: center; } }
  @container shelf (max-width: 520px) { .clock .date { display: none; } }
  /* a touch screen: taller targets */
  @media (pointer: coarse) { .taskbar button { height: calc(var(--tb-h) - 6px); min-width: 44px; } }

  /* the umbrelOS dock: glass with a highlight and a deep shadow, icons that grow on hover (the neighbours — slightly), the hint
     above the icon, a dot under an open window; the service segment on the right (core health, updates, clock) replaces the tray */
  .dock { position: fixed; left: 50%; bottom: 10px; transform: translateX(-50%); z-index: 2000; display: flex; align-items: flex-end; gap: calc(var(--d) * .2); height: calc(var(--d) + 24px); padding: 0 12px 12px; max-width: calc(100vw - 16px); overflow: visible;
    border-radius: calc(var(--d) * .48); background: var(--glass); background-clip: padding-box; border: .5px solid rgb(255 255 255 / .1); backdrop-filter: var(--glass-blur); -webkit-backdrop-filter: var(--glass-blur);
    box-shadow: inset 1px 0 0 rgb(255 255 255 / .04), inset -1px 0 0 rgb(255 255 255 / .04), inset 0 1px 0 rgb(255 255 255 / .2), inset 0 .5px 0 rgb(255 255 255 / .1), 0 4px 24px rgb(0 0 0 / .56); }
  .dock .sep { width: .5px; height: calc(var(--d) * .8); margin: 0 1px calc(var(--d) * .1); background: rgb(255 255 255 / .16); flex: none; }
  .slot { position: relative; display: flex; align-items: flex-end; height: var(--d); flex: none; }
  .dock .item { position: relative; width: var(--d); height: var(--d); padding: 0; border: 0; border-radius: 30%; background: transparent; box-shadow: none; display: grid; place-items: center; transform-origin: bottom center; transition: transform .18s var(--ease), filter var(--dur); filter: drop-shadow(0 6px 14px rgb(0 0 0 / .45)); }
  .dock .item:hover { transform: scale(1.5) translateY(-6px); z-index: 2; background: transparent; }
  .dock .slot:has(.item:hover) + .slot .item, .dock .slot:has(+ .slot .item:hover) .item { transform: scale(1.2) translateY(-3px); }
  .dock .item:focus-visible { outline: 0; filter: drop-shadow(0 6px 14px rgb(0 0 0 / .45)) brightness(1.25); }
  .dock .item.min { opacity: .55; }
  .tile { display: grid; place-items: center; width: var(--d); height: var(--d); border-radius: 30%; background: rgb(255 255 255 / .12); border: .5px solid rgb(255 255 255 / .18); box-shadow: inset 0 1px 0 rgb(255 255 255 / .2); color: rgb(255 255 255 / .88); }
  .dock .item.start.active .tile { background: rgb(255 255 255 / .25); }
  .run { position: absolute; left: 50%; bottom: -9px; width: 4px; height: 4px; border-radius: 50%; background: rgb(255 255 255 / .85); transform: translateX(-50%); transition: width var(--dur) var(--ease); }
  .dock .item.active .run { width: 12px; border-radius: 2px; }
  .tip { position: absolute; left: 50%; bottom: calc(100% + 30px); transform: translateX(-50%) translateY(4px); padding: 4px 10px; border-radius: 10px; background: rgb(0 0 0 / .8); color: #fff; font-size: 12px; font-weight: 500; white-space: nowrap; opacity: 0; pointer-events: none; transition: opacity .15s, transform .15s; box-shadow: 0 4px 12px rgb(0 0 0 / .4); }
  .slot:hover .tip { opacity: 1; transform: translateX(-50%) translateY(0); }
  .dock .tray { gap: 3px; height: var(--d); margin-left: 0; }
  .dock .ib { display: flex; align-items: center; justify-content: center; gap: 8px; height: 34px; min-width: 34px; padding: 0 12px; border: 0; border-radius: 999px; background: rgb(255 255 255 / .08); box-shadow: none; font-size: .85rem; color: rgb(255 255 255 / .86); white-space: nowrap; }
  .dock .ib:hover { background: rgb(255 255 255 / .16); }
  .dock .ib.upd { padding: 0; color: var(--accent); }
  .dock .iclock { font-weight: 600; font-variant-numeric: tabular-nums; }
  @media (max-width: 1000px) { .dock .ib .label { display: none; } .dock .ib { padding: 0 10px; } .dock { gap: 7px; padding: 0 9px 10px; } }
  @media (max-width: 820px) { .dock .addw { display: none; } }
  /* more open windows than the screen is wide: the dock scrolls, the magnification and the hints are switched off */
  .dock.scroll { overflow-x: auto; overflow-y: hidden; left: 8px; right: 8px; transform: none; max-width: none; scrollbar-width: none; align-items: center; padding-bottom: 0; }
  .dock.scroll::-webkit-scrollbar { display: none; }
  .dock.scroll .item:hover, .dock.scroll .slot:has(.item:hover) + .slot .item, .dock.scroll .slot:has(+ .slot .item:hover) .item { transform: none; }
  .dock.scroll .tip { display: none; }
  .dock.scroll .run { bottom: -7px; }
  .dock.scroll .sep { margin-bottom: 0; }
  @media (pointer: coarse) { .dock .item:hover, .dock .slot:has(.item:hover) + .slot .item, .dock .slot:has(+ .slot .item:hover) .item { transform: none; } .tip { display: none; } .dock .ib { height: 40px; min-width: 40px; } }
</style>
