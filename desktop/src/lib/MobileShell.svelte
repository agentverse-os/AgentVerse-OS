<script lang="ts">
  // Мобильная оболочка (3.13: телефон — режим «пульт»): статус-бар, домашний экран (поиск, виджеты, сетка иконок),
  // приложения на весь экран, нижняя навигация назад/домой/недавние, ящик приложений.
  import { wm, SYS_APPS, winTitle, type SysApp } from './windows.svelte';
  import { t, i18n } from './i18n.svelte';
  import { NAV_H, STATUS_H } from './device.svelte';
  import { api, type AppView, type ProjectView, type SystemStatus } from './api';
  import { iconOf, isImg, titleOf, openUrl } from './store-utils';
  import Widgets from './Widgets.svelte';
  import StartMenu from './StartMenu.svelte';
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
  function openApp(a: AppView) { if (!a.url) return; if (a.manifest.route.open === 'iframe') wm.open(`app:${a.name}`, titleOf(a), openUrl(a), iconOf(a)); else window.open(openUrl(a), '_blank', 'noopener'); }
  function home() { for (const w of wm.wins) w.minimized = true; wm.active = null; wm.save(); recents = false; wm.startOpen = false; }
  function back() { if (recents) { recents = false; return; } if (wm.startOpen) { wm.startOpen = false; return; } if (activeWin) { activeWin.minimized = true; wm.active = null; wm.save(); } }
</script>

<div class="mobile" style="--nav:{NAV_H}px; --status:{STATUS_H}px">
  <header class="status">
    <span class="clock">{now.toLocaleTimeString(i18n.locale, { hour: '2-digit', minute: '2-digit' })}</span>
    <span class="muted">{now.toLocaleDateString(i18n.locale, { weekday: 'short', day: 'numeric', month: 'short' })}</span>
    <span class="spacer"></span>
    <button class="ico" title={t('shell.theme')} onclick={() => appearance = { ...appearance, mode: appearance.mode === 'light' ? 'dark' : 'light' }}>{appearance.mode === 'light' ? '☀️' : '🌙'}</button>
    <button class="ico" title={t('shell.healthLow')} onclick={() => wm.openSys('system')}><span class="dot" class:ok={bad === 0} class:bad={bad > 0}></span></button>
  </header>

  {#if !activeWin}
    <main class="home" class:hidden={recents}>
      <button class="search" onclick={() => wm.startOpen = true}>🔍 {t('shell.searchApps')}</button>
      <div class="grid">
        {#each sys as k}<button class="tile" onclick={() => wm.openSys(k)}><span class="g emoji">{SYS_APPS[k].icon}</span><span class="n">{t(SYS_APPS[k].title)}</span></button>{/each}
        {#each projects as p (p.name)}<button class="tile" onclick={() => wm.openSys('project', { name: p.name }, p.name)}><span class="g emoji">🧩<span class="st" class:ok={p.workspace?.status === 'running'}></span></span><span class="n">{p.name}</span></button>{/each}
        {#each apps.filter(a => a.installed) as a (a.name)}
          {@const ic = iconOf(a)}
          <button class="tile" onclick={() => openApp(a)}>{#if isImg(ic)}<img class="g" src={ic} alt="" />{:else}<span class="g emoji">{ic ?? '📦'}</span>{/if}<span class="n">{titleOf(a)}</span></button>
        {/each}
      </div>
      <Widgets mobile />
    </main>
  {/if}

  {#if recents}
    <div class="recents" role="dialog" aria-label={t('shell.recents')}>
      <div class="row" style="justify-content: space-between;"><b>{t('shell.openApps')}</b>{#if wm.wins.length}<button onclick={() => { wm.closeAll(); recents = false; }}>{t('shell.closeAll')}</button>{/if}</div>
      {#each wm.wins as w (w.id)}
        <div class="rc">
          <button class="rc-open" onclick={() => { wm.focus(w.id); wm.save(); recents = false; }}>
            {#if isImg(w.icon ?? null)}<img src={w.icon} alt="" />{:else}<span class="emoji">{w.icon ?? "▢"}</span>{/if}
            <span><b>{winTitle(w)}</b>{#if w.url}<br /><span class="muted small">{w.url.replace(/^https?:\/\//, '')}</span>{/if}</span>
          </button>
          <button class="rc-close" title={t('common.closeTitle')} onclick={() => wm.close(w.id)}>✕</button>
        </div>
      {/each}
      {#if !wm.wins.length}<div class="muted">{t('shell.nothingOpen')}</div>{/if}
    </div>
  {/if}

  <StartMenu />

  <nav class="navbar" style="height: var(--nav)" aria-label={t('shell.navAria')}>
    <button onclick={back} title={t('shell.navBack')}>◁</button>
    <button onclick={home} title={t('shell.navHome')}>○</button>
    <button onclick={() => { recents = !recents; wm.startOpen = false; }} title={t('shell.recents')} class:active={recents}>▢{#if wm.wins.length}<span class="cnt">{wm.wins.length}</span>{/if}</button>
  </nav>
</div>

<style>
  /* оболочка выше окон (z 2000), но прозрачна для касаний — окна под ней кликабельны; свои элементы включают pointer-events */
  .mobile { position: fixed; inset: 0; display: flex; flex-direction: column; z-index: 2000; pointer-events: none; }
  .mobile > :global(*) { pointer-events: auto; } /* :global — иначе Svelte ограничит правило своими элементами, и дочерние компоненты (ящик) останутся без событий */
  .status { display: flex; align-items: center; gap: 8px; height: var(--status); padding: env(safe-area-inset-top) 12px 0; font-size: .9rem; background: var(--glass); backdrop-filter: blur(12px); -webkit-backdrop-filter: blur(12px); }
  .status .clock { font-weight: 700; }
  .spacer { flex: 1; }
  .ico { background: transparent; border-color: transparent; padding: 2px 6px; min-height: 32px; }
  .dot { display: inline-block; width: 10px; height: 10px; border-radius: 50%; background: var(--muted); } .dot.ok { background: var(--ok); } .dot.bad { background: var(--bad); }
  .home { flex: 1; overflow: auto; padding: 4px 12px calc(var(--nav) + 12px); display: flex; flex-direction: column; gap: 12px; }
  .home.hidden { display: none; }
  .search { width: 100%; text-align: left; padding: 12px 14px; border-radius: 999px; background: var(--glass); color: var(--muted); font-size: .95rem; }
  .grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px 6px; }
  .tile { display: flex; flex-direction: column; align-items: center; gap: 6px; background: transparent; border-color: transparent; padding: 6px 2px; color: var(--ink); text-shadow: 0 1px 3px rgb(0 0 0 / .5); }
  :root[data-theme="light"] .tile { text-shadow: none; }
  .g { width: 56px; height: 56px; border-radius: 16px; object-fit: cover; position: relative; }
  .g.emoji { display: flex; align-items: center; justify-content: center; font-size: 34px; background: var(--glass); border: 1px solid var(--line); }
  .st { position: absolute; right: -2px; bottom: -2px; width: 12px; height: 12px; border-radius: 50%; background: var(--muted); border: 2px solid var(--bg); } .st.ok { background: var(--ok); }
  .n { font-size: .75rem; line-height: 1.15; text-align: center; max-width: 84px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .recents { position: fixed; inset: var(--status) 0 var(--nav) 0; z-index: 1900; overflow: auto; padding: 12px; display: flex; flex-direction: column; gap: 8px; background: var(--bg); }
  .rc { display: flex; gap: 6px; align-items: stretch; }
  .rc-open { flex: 1; display: flex; gap: 10px; align-items: center; text-align: left; padding: 10px 12px; }
  .rc-open img { width: 32px; height: 32px; border-radius: 8px; } .rc-open .emoji { font-size: 24px; }
  .rc-close { min-width: 48px; }
  .small { font-size: .78rem; }
  .navbar { position: fixed; left: 0; right: 0; bottom: 0; z-index: 2200; display: grid; grid-template-columns: 1fr 1fr 1fr; background: var(--glass); backdrop-filter: blur(16px); -webkit-backdrop-filter: blur(16px); border-top: 1px solid var(--line); padding-bottom: env(safe-area-inset-bottom); }
  .navbar button { background: transparent; border: 0; border-radius: 0; font-size: 1.4rem; color: var(--ink); position: relative; }
  .navbar button.active { color: var(--accent); }
  .cnt { position: absolute; top: 6px; right: calc(50% - 22px); font-size: .65rem; background: var(--accent); color: var(--on-accent); border-radius: 999px; padding: 0 5px; }
</style>
