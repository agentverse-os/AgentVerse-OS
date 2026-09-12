<script lang="ts">
  import Toasts from './lib/Toasts.svelte';
  import Offline from './lib/Offline.svelte';
  import Screensaver from './lib/Screensaver.svelte';
  import { bootDone, bootMessage, bootActive } from './lib/boot';
  // Оболочка AgentVerse OS (3.12): рабочий стол с обоями и иконками, окна поверх, панель задач с меню запуска.
  import Windows from './lib/Windows.svelte';
  import Taskbar from './lib/Taskbar.svelte';
  import StartMenu from './lib/StartMenu.svelte';
  import DesktopIcons from './lib/DesktopIcons.svelte';
  import Widgets from './lib/Widgets.svelte';
  import { widgets } from './lib/widgets.svelte';
  import MobileShell from './lib/MobileShell.svelte';
  import { device } from './lib/device.svelte';
  import { wm, type SysApp } from './lib/windows.svelte';
  import { api, type SystemStatus } from './lib/api';
  import { apply, load, loadRemote, watchSystem, saveLocal, saveRemote, type Appearance } from './lib/theme';
  import { t, tn } from './lib/i18n.svelte';

  let status = $state<SystemStatus | null>(null);
  let ctx = $state<{ x: number; y: number } | null>(null);
  // Доверяет ли браузер сертификату AgentVerse OS: Service Worker регистрируется только при полностью доверенном сертификате, «исключение»
  // не считается. Без доверия не работают webview в code-server (панели расширений, в т.ч. Claude Code), PWA и приложения на других портах.
  let certWarn = $state(false);
  $effect(() => {
    if (!('serviceWorker' in navigator) || localStorage.getItem('cloudos.certwarn.until') && Number(localStorage.getItem('cloudos.certwarn.until')) > Date.now()) return;
    navigator.serviceWorker.register('/sw.js', { scope: '/' }).catch((e: unknown) => { const m = String(e); if (m.includes('SSL') || m.includes('certificate') || m.includes('SecurityError')) certWarn = true; });
  });
  function dismissCert() { certWarn = false; localStorage.setItem('cloudos.certwarn.until', String(Date.now() + 7 * 86400_000)); }
  // связь с ядром: первый ответ снимает экран загрузки; без ответа дольше 25 с — оверлей «нет связи», ответ — снимает.
  // Порог по времени, а не по числу промахов: один опрос с повторами может тянуться дольше интервала (в WebKit особенно).
  let failures = $state(0);
  let offlineSince = $state(0);
  let lastOk = Date.now();
  let polling = false;
  async function poll() {
    if (polling) return;
    polling = true;
    try {
      status = await api.status();
      failures = 0; offlineSince = 0; lastOk = Date.now(); bootDone();
    } catch (e) {
      status = null; failures += 1;
      if (bootActive()) bootMessage(failures < 2 ? t('boot.retry') : t('boot.down', { n: failures }), failures >= 2);
    } finally { polling = false; }
  }
  $effect(() => {
    poll();
    const t = setInterval(poll, 10000);
    const w = setInterval(() => { if (!bootActive() && !offlineSince && Date.now() - lastOk > 25000) offlineSince = lastOk; }, 1000);
    return () => { clearInterval(t); clearInterval(w); };
  });
  const saverStatus = $derived(status ? t('saver.ok', { windows: tn('saver.windows', wm.wins.length) }) : t('common.noCore'));

  // оформление: сразу из localStorage, затем из ядра (общее для всех устройств); изменения из панели задач — сохраняются
  const initial = load();
  let appearance = $state<Appearance>(initial);
  apply(initial);
  let remoteLoaded = false;
  $effect(() => { loadRemote().then(r => { if (r) appearance = r; remoteLoaded = true; }); });
  $effect(() => { const a = $state.snapshot(appearance); apply(a); saveLocal(a); if (remoteLoaded) saveRemote(a); });
  $effect(() => watchSystem(() => appearance));

  // глубокие ссылки: /#projects, /#store, /#system, /#settings, /#app=<name> открывают окно
  const sysApps: SysApp[] = ['projects', 'store', 'system', 'settings', 'files', 'vault', 'setup', 'updates'];
  function openFromHash() {
    const h = location.hash.slice(1);
    if (!h) return;
    if (sysApps.includes(h as SysApp)) wm.openSys(h as SysApp);
    else if (h.startsWith('app=')) wm.openSys('appdetail', { name: h.slice(4) }, h.slice(4));
    else if (h.startsWith('ready=')) wm.openSys('appready', { name: h.slice(6) });
    history.replaceState(null, '', location.pathname);
  }
  $effect(() => { openFromHash(); window.addEventListener('hashchange', openFromHash); return () => window.removeEventListener('hashchange', openFromHash); });
  // первый визит из этого браузера: пока мастер первого запуска не пройден (ядро, kv setup.done) — открыть его на любом устройстве;
  // иначе на десктопе — Проекты (телефон стартует с домашнего экрана). Решение принимается по первому ответу ядра.
  const firstVisit = !location.hash && !localStorage.getItem('cloudos.firstrun');
  let firstOpened = false;
  $effect(() => {
    if (!firstVisit || firstOpened || !status) return;
    firstOpened = true; localStorage.setItem('cloudos.firstrun', '1');
    if (status.setup_done === false) wm.openSys('setup');
    else if (!device.phone && !wm.wins.length) wm.openSys('projects');
  });

  // клавиатура: Alt+Tab — следующее окно, Esc — закрыть меню, Meta/Ctrl+Space — меню запуска, Alt+F4/Ctrl+W не трогаем (браузер)
  function keys(e: KeyboardEvent) {
    if (e.key === 'Tab' && e.altKey) { e.preventDefault(); wm.cycle(); }
    else if (e.key === 'Escape') { wm.startOpen = false; ctx = null; widgets.picker = false; }
    else if (e.key === ' ' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); wm.startOpen = !wm.startOpen; }
  }
</script>

<svelte:window onkeydown={keys} />
{#if offlineSince && !bootActive()}<Offline since={offlineSince} attempts={failures} onretry={poll} />{/if}
<Screensaver minutes={appearance.screensaver ?? 10} statusText={saverStatus} />
{#if device.phone}
  <div class="desktop" role="application" aria-label="AgentVerse OS">
    {#if certWarn && !wm.wins.some(w => !w.minimized)}<div class="certwarn phone"><span>{t('cert.phone')} <a href="/api/ca.crt">{t('cert.download')}</a> · <button class="link" onclick={() => wm.openSys('settings')}>{t('cert.how')}</button></span><button class="x" title={t('cert.hide')} onclick={dismissCert}>✕</button></div>{/if}
    <MobileShell bind:appearance {status} />
    <Windows bind:appearance />
    <Toasts />
  </div>
{:else}
<div class="desktop" role="application" aria-label="AgentVerse OS" oncontextmenu={(e) => { if ((e.target as HTMLElement).closest('.win, .taskbar, .menu, .widget, input, textarea, a')) return; e.preventDefault(); ctx = { x: e.clientX, y: e.clientY }; }} onclick={() => ctx = null}>
  {#if certWarn}<div class="certwarn"><span>{t('cert.desktop')} <a href="/api/ca.crt">{t('cert.download')}</a> · <button class="link" onclick={() => wm.openSys('settings')}>{t('cert.howLong')}</button></span><button class="x" title={t('cert.hide')} onclick={dismissCert}>✕</button></div>{/if}
  <DesktopIcons />
  <Widgets />
  <Windows bind:appearance />
  <Toasts />
  {#if ctx}
    <ul class="ctx" role="menu" style="left:{Math.min(ctx.x, innerWidth - 240)}px; top:{Math.min(ctx.y, innerHeight - 200)}px">
      <li role="menuitem"><button onclick={() => { widgets.picker = true; ctx = null; }}>{t('ctx.addWidget')}</button></li>
      <li role="menuitem"><button onclick={() => { wm.openSys('settings'); ctx = null; }}>{t('ctx.appearance')}</button></li>
      <li role="menuitem"><button onclick={() => { wm.closeAll(); ctx = null; }}>{t('ctx.closeAll')}</button></li>
      <li role="menuitem"><button onclick={() => { location.reload(); }}>{t('ctx.reload')}</button></li>
    </ul>
  {/if}
  <StartMenu />
  <Taskbar bind:appearance {status} />
</div>
{/if}

<style>
  .certwarn { position: fixed; bottom: 58px; left: 50%; transform: translateX(-50%); z-index: 1900; width: max-content; max-width: min(92vw, 860px); display: flex; gap: 10px; align-items: center; padding: 8px 12px; border-radius: var(--r-sm, 8px); background: color-mix(in srgb, var(--bad, #c33) 18%, var(--panel, #222)); border: 1px solid var(--bad, #c33); color: var(--fg, #eee); font-size: .9rem; line-height: 1.35; }
  .certwarn.phone { bottom: 66px; z-index: 2100; }
  .certwarn a { color: inherit; text-decoration: underline; }
  .certwarn .link { background: none; border: 0; padding: 0; color: inherit; text-decoration: underline; cursor: pointer; font: inherit; }
  .certwarn .x { background: none; border: 0; color: inherit; cursor: pointer; font-size: 1rem; padding: 0 4px; }

  .desktop { position: fixed; inset: 0; overflow: hidden; }
  .ctx { position: fixed; z-index: 3000; list-style: none; margin: 0; padding: 4px; background: var(--glass); backdrop-filter: blur(16px); -webkit-backdrop-filter: blur(16px); border: 1px solid var(--line); border-radius: var(--r-sm); box-shadow: var(--shadow); min-width: 220px; }
  .ctx button { width: 100%; text-align: left; background: transparent; border-color: transparent; padding: 8px 10px; }
  .ctx button:hover { background: var(--panel-2); }
</style>
