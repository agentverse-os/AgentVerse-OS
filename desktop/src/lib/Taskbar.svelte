<script lang="ts">
  import Mark from './Mark.svelte';
  import { wm, SYS_APPS, TASKBAR_H, winTitle, type SysApp } from './windows.svelte';
  import { t, tn, i18n } from './i18n.svelte';
  import type { Appearance } from './theme';
  import type { SystemStatus } from './api';
  let { appearance = $bindable(), status }: { appearance: Appearance; status: SystemStatus | null } = $props();
  let now = $state(new Date());
  $effect(() => { const t = setInterval(() => now = new Date(), 1000); return () => clearInterval(t); });
  const bad = $derived(status?.components.filter(c => c.status !== 'ok').length ?? 0);
  const pinned: SysApp[] = ['projects', 'store', 'files', 'system', 'settings'];
  function isImg(icon?: string | null) { return !!icon && (icon.startsWith('/') || icon.startsWith('http')); }
  function toggleTheme() { appearance = { ...appearance, mode: appearance.mode === 'light' ? 'dark' : 'light' }; }
</script>

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
  .start.active { background: var(--panel-2); border-color: var(--accent); }
  .logo { display: inline-flex; align-items: center; }
  .pinned { display: flex; gap: 2px; padding-right: 6px; border-right: 1px solid var(--line); }
  .tasks { display: flex; gap: 2px; flex: 1; overflow-x: auto; }
  .tray { display: flex; gap: 2px; align-items: center; margin-left: auto; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--muted); }
  .dot.ok { background: var(--ok); } .dot.bad { background: var(--bad); }
  .clock { flex-direction: column; gap: 0; line-height: 1.1; padding: 0 10px; }
  .clock .date { font-size: .7rem; }
  @media (max-width: 700px) { .label { display: none; } .clock .date { display: none; } .taskbar { gap: 2px; padding: 0 4px; } }
</style>
