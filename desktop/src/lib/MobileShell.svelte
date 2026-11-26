<script lang="ts">
  // Mobile shell (3.13: the phone is the "remote" mode): status bar, home screen (search, icon grid, widgets),
  // full-screen apps, bottom navigation back/home/recents, the app drawer. The same shell serves an unfolded foldable and
  // a small tablet (the grids get more columns) and a phone on its side (no status bar, the navigation is a rail on the right).
  import { wm, SYS_APPS, winTitle, type SysApp } from './windows.svelte';
  import { t, i18n } from './i18n.svelte';
  import { api, type AppView, type ProjectView, type SystemStatus } from './api';
  import { iconOf, titleOf, openUrl } from './store-utils';
  import Widgets from './Widgets.svelte';
  import StartMenu from './StartMenu.svelte';
  import AppIcon, { refOf, winIcon } from './AppIcon.svelte';
  import Glyph from './Glyph.svelte';
  import type { Appearance } from './theme';
  let { appearance = $bindable(), status }: { appearance: Appearance; status: SystemStatus | null } = $props();
  let apps = $state<AppView[]>([]);
  let projects = $state<ProjectView[]>([]);
  let now = $state(new Date());
  let recents = $state(false);
  async function load() { try { [apps, projects] = await Promise.all([api.apps(), api.projects()]); } catch {} }
  $effect(() => { load(); const t = setInterval(load, 15000); const c = setInterval(() => now = new Date(), 1000); return () => { clearInterval(t); clearInterval(c); }; });
  const sys: SysApp[] = ['projects', 'store', 'files', 'system', 'settings'];
  const activeWin = $derived(wm.wins.find(w => w.id === wm.active && !w.minimized) ?? null);
  const bad = $derived(status?.components.filter(c => c.status !== 'ok').length ?? 0);
  const time = $derived(now.toLocaleTimeString(i18n.locale, { hour: '2-digit', minute: '2-digit' }));
  function openApp(a: AppView) { if (!a.url) return; if (a.manifest.route.open === 'iframe') wm.open(`app:${a.name}`, titleOf(a), openUrl(a), iconOf(a)); else window.open(openUrl(a), '_blank', 'noopener'); }
  function home() { for (const w of wm.wins) w.minimized = true; wm.active = null; wm.save(); recents = false; wm.startOpen = false; }
  function back() { if (recents) { recents = false; return; } if (wm.startOpen) { wm.startOpen = false; return; } if (activeWin) { activeWin.minimized = true; wm.active = null; wm.save(); } }
</script>

<div class="mobile">
  <header class="status">
    <span class="clock">{time}</span>
    <span class="muted date">{now.toLocaleDateString(i18n.locale, { weekday: 'short', day: 'numeric', month: 'short' })}</span>
    <span class="spacer"></span>
    <button class="ico plain" title={t('shell.theme')} aria-label={t('shell.theme')} onclick={() => appearance = { ...appearance, mode: appearance.mode === 'light' ? 'dark' : 'light' }}><Glyph name={appearance.mode === 'light' ? 'sun' : 'moon'} size={19} /></button>
    <button class="ico plain" title={t('shell.healthLow')} aria-label={t('shell.healthLow')} onclick={() => wm.openSys('system')}><span class="dot" class:ok={bad === 0} class:bad={bad > 0}></span></button>
  </header>

  {#if !activeWin}
    <main class="home" class:hidden={recents}>
      <button class="search" onclick={() => wm.startOpen = true}><Glyph name="search" size={18} /> {t('shell.searchApps')}</button>
      <div class="grid">
        {#each sys as k}<button class="tile" onclick={() => wm.openSys(k)}><AppIcon icon={{ sys: k }} size="var(--tsz)" /><span class="n">{t(SYS_APPS[k].title)}</span></button>{/each}
        {#each projects as p (p.name)}<button class="tile" onclick={() => wm.openSys('project', { name: p.name }, p.name)}><AppIcon icon={{ sys: 'project' }} size="var(--tsz)" status={p.workspace?.status === 'running' ? 'ok' : 'off'} /><span class="n">{p.name}</span></button>{/each}
        {#each apps.filter(a => a.installed) as a (a.name)}
          <button class="tile" onclick={() => openApp(a)}><AppIcon icon={refOf(iconOf(a))} size="var(--tsz)" label={titleOf(a)} /><span class="n">{titleOf(a)}</span></button>
        {/each}
      </div>
      <Widgets mobile />
    </main>
  {/if}

  {#if recents}
    <div class="recents" role="dialog" aria-label={t('shell.recents')}>
      <div class="rhead"><b>{t('shell.openApps')}</b>{#if wm.wins.length}<button onclick={() => { wm.closeAll(); recents = false; }}>{t('shell.closeAll')}</button>{/if}</div>
      <div class="rgrid">
        {#each wm.wins as w (w.id)}
          <div class="rc">
            <button class="rc-open" onclick={() => { wm.focus(w.id); wm.save(); recents = false; }}>
              <AppIcon icon={winIcon(w)} size={36} label={winTitle(w)} />
              <span class="rt"><b>{winTitle(w)}</b>{#if w.url}<span class="muted small">{w.url.replace(/^https?:\/\//, '')}</span>{/if}</span>
            </button>
            <button class="rc-close plain" title={t('common.closeTitle')} aria-label={t('common.closeTitle')} onclick={() => wm.close(w.id)}><Glyph name="close" size={18} /></button>
          </div>
        {/each}
      </div>
      {#if !wm.wins.length}<div class="muted">{t('shell.nothingOpen')}</div>{/if}
    </div>
  {/if}

  <StartMenu />

  <nav class="navbar" aria-label={t('shell.navAria')}>
    <button class="nb" onclick={back} title={t('shell.navBack')} aria-label={t('shell.navBack')}><Glyph name="back" size={24} /></button>
    <button class="nb" onclick={home} title={t('shell.navHome')} aria-label={t('shell.navHome')}><Glyph name="home" size={23} /></button>
    <button class="nb" onclick={() => { recents = !recents; wm.startOpen = false; }} title={t('shell.recents')} aria-label={t('shell.recents')} class:active={recents}><Glyph name="recents" size={22} />{#if wm.wins.length}<span class="cnt">{wm.wins.length}</span>{/if}</button>
    <span class="railclock">{time}</span>
  </nav>
</div>

<style>
  /* the shell is above the windows (z 2000) but transparent to touches — the windows under it are clickable; its own elements enable pointer-events.
     The geometry is the --m-* tokens of app.css plus the safe-area insets (a notch, the gesture bar). */
  .mobile { --st: calc(var(--m-status) + env(safe-area-inset-top, 0px)); --nb: calc(var(--m-nav-b) + env(safe-area-inset-bottom, 0px)); --nr: calc(var(--m-nav-r) + env(safe-area-inset-right, 0px)); --sl: env(safe-area-inset-left, 0px);
    position: fixed; inset: 0; display: flex; flex-direction: column; z-index: 2000; pointer-events: none; }
  .mobile > :global(*) { pointer-events: auto; } /* :global — otherwise Svelte scopes the rule to its own elements, and the child components (the drawer) are left without events */
  .status { display: flex; align-items: center; gap: 8px; height: var(--st); padding: env(safe-area-inset-top, 0px) calc(8px + env(safe-area-inset-right, 0px)) 0 calc(14px + var(--sl)); font-size: .9rem; background: var(--shelf-bg); backdrop-filter: var(--shelf-blur); -webkit-backdrop-filter: var(--shelf-blur); flex: none; }
  :global(:root[data-look="umbrel"]) .status { background: var(--glass); backdrop-filter: var(--glass-blur); -webkit-backdrop-filter: var(--glass-blur); }
  .status .clock { font-weight: 700; font-variant-numeric: tabular-nums; }
  .status .date { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .spacer { flex: 1; }
  .ico { display: grid; place-items: center; width: 40px; height: 40px; padding: 0; border-radius: 50%; background: transparent; border-color: transparent; color: var(--ink); flex: none; }
  .ico:hover { background: var(--fill); border-color: transparent; }
  .dot { display: inline-block; width: 10px; height: 10px; border-radius: 50%; background: var(--muted); } .dot.ok { background: var(--ok); box-shadow: 0 0 0 3px color-mix(in oklab, var(--ok) 22%, transparent); } .dot.bad { background: var(--bad); box-shadow: 0 0 0 3px color-mix(in oklab, var(--bad) 22%, transparent); }

  .home { --tsz: clamp(52px, 14.5vw, 64px); flex: 1; min-height: 0; overflow: hidden auto; overscroll-behavior: contain; padding: 10px calc(14px + var(--nr)) calc(var(--nb) + 14px) calc(14px + var(--sl)); display: flex; flex-direction: column; gap: 14px; }
  .home.hidden { display: none; }
  .search { display: flex; align-items: center; gap: 10px; width: 100%; max-width: 640px; margin-inline: auto; text-align: left; min-height: 46px; padding: 0 16px; border-radius: 999px; background: var(--glass); backdrop-filter: var(--glass-blur); -webkit-backdrop-filter: var(--glass-blur); border-color: var(--line); color: var(--muted); font-size: .95rem; font-weight: 400; flex: none; }
  /* the icon grid fills the width: four columns on a phone, more on an unfolded foldable and a tablet */
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(calc(var(--tsz) + 22px), 1fr)); gap: 12px 2px; flex: none; }
  .tile { display: flex; flex-direction: column; align-items: center; gap: 7px; min-width: 0; background: transparent; border-color: transparent; border-radius: var(--r); padding: 6px 2px; color: var(--ink); text-shadow: 0 1px 3px rgb(0 0 0 / .55); --icon-ring: rgb(10 12 16 / .7); }
  .tile:hover { background: transparent; border-color: transparent; }
  .tile:active { transform: scale(.94); }
  .tile > :global(.aicon) { filter: drop-shadow(0 4px 10px rgb(0 0 0 / .3)); }
  :global(:root[data-theme="light"]) .tile { text-shadow: 0 1px 2px rgb(255 255 255 / .8); --icon-ring: rgb(255 255 255 / .9); }
  .n { font-size: .76rem; font-weight: 500; line-height: 1.2; text-align: center; max-width: 100%; overflow: hidden; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow-wrap: anywhere; }

  .recents { position: fixed; inset: var(--st) var(--nr) var(--nb) var(--sl); z-index: 1900; overflow: auto; padding: 14px; display: flex; flex-direction: column; gap: 12px; background: var(--bg); }
  .rhead { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-height: 40px; }
  .rhead button { min-height: 40px; }
  .rgrid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 280px), 1fr)); gap: 8px; }
  .rc { display: flex; align-items: stretch; background: var(--card); border: 1px solid var(--card-line); border-radius: var(--r); overflow: hidden; min-width: 0; }
  .rc-open { flex: 1; min-width: 0; display: flex; gap: 12px; align-items: center; text-align: left; min-height: 60px; padding: 10px 12px; background: transparent; border: 0; border-radius: 0; }
  .rc-open:hover { background: var(--fill); }
  .rt { display: flex; flex-direction: column; min-width: 0; }
  .rt b, .rt .small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rc-close { display: grid; place-items: center; width: 52px; padding: 0; background: transparent; border: 0; border-left: 1px solid var(--card-line); border-radius: 0; color: var(--muted); flex: none; }
  .rc-close:hover { background: color-mix(in oklab, var(--bad) 14%, transparent); color: var(--bad); }
  .small { font-size: .78rem; }

  .navbar { position: fixed; left: 0; right: 0; bottom: 0; z-index: 2200; display: grid; grid-template-columns: 1fr 1fr 1fr; height: var(--nb); padding: 0 var(--nr) env(safe-area-inset-bottom, 0px) var(--sl); background: var(--shelf-bg); backdrop-filter: var(--shelf-blur); -webkit-backdrop-filter: var(--shelf-blur); border-top: 1px solid var(--line); }
  :global(:root[data-look="umbrel"]) .navbar { background: var(--glass); backdrop-filter: var(--glass-blur); -webkit-backdrop-filter: var(--glass-blur); border-top: .5px solid rgb(255 255 255 / .1); }
  .nb { position: relative; display: grid; place-items: center; min-width: 48px; min-height: 48px; padding: 0; background: transparent; border: 0; border-radius: 0; color: var(--ink); }
  .nb:hover { background: transparent; }
  .nb:active { background: var(--fill); transform: none; }
  .nb.active { color: var(--accent-ink); }
  .cnt { position: absolute; top: calc(50% - 20px); left: calc(50% + 8px); min-width: 17px; height: 17px; display: grid; place-items: center; font-size: .66rem; font-weight: 700; background: var(--accent); color: var(--on-accent); border-radius: 999px; padding: 0 4px; }
  .railclock { display: none; }

  /* a phone on its side: no status bar, the navigation is a rail on the right with the clock on top */
  @media (orientation: landscape) and (max-height: 520px) {
    .status { display: none; }
    .home { padding-top: calc(10px + env(safe-area-inset-top, 0px)); padding-bottom: calc(12px + env(safe-area-inset-bottom, 0px)); }
    .navbar { left: auto; top: 0; width: var(--nr); height: auto; grid-template-columns: none; grid-template-rows: auto 1fr 1fr 1fr; padding: calc(8px + env(safe-area-inset-top, 0px)) env(safe-area-inset-right, 0px) env(safe-area-inset-bottom, 0px) 0; border-top: 0; border-left: 1px solid var(--line); }
    .railclock { display: block; order: -1; text-align: center; font-size: .78rem; font-weight: 700; font-variant-numeric: tabular-nums; padding-bottom: 4px; }
    .cnt { top: calc(50% - 20px); }
  }
</style>
