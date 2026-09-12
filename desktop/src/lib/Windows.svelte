<script lang="ts">
  import { wm, TASKBAR_H, winTitle, type Win } from './windows.svelte';
  import { t } from './i18n.svelte';
  import { device, NAV_H, STATUS_H } from './device.svelte';
  import Projects from './Projects.svelte';
  import Store from './Store.svelte';
  import System from './System.svelte';
  import Settings from './Settings.svelte';
  import Setup from './Setup.svelte';
  import Updates from './Updates.svelte';
  import AppDetail from './AppDetail.svelte';
  import AppReady from './AppReady.svelte';
  import Vault from './Vault.svelte';
  import Reader from './Reader.svelte';
  import AccessPanel from './AccessPanel.svelte';
  import { api, type AppView } from './api';
  import ProjectWindow from './ProjectWindow.svelte';
  import Files from './Files.svelte';
  import type { Appearance } from './theme';
  let { appearance = $bindable() }: { appearance: Appearance } = $props();
  let drag: { id: string; kind: 'move' | 'resize'; x: number; y: number } | null = null;
  function down(e: PointerEvent, w: Win, kind: 'move' | 'resize') {
    if ((e.target as HTMLElement).closest('button, a')) return;
    drag = { id: w.id, kind, x: e.clientX, y: e.clientY }; wm.focus(w.id);
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) {
    if (!drag) return;
    const dx = e.clientX - drag.x, dy = e.clientY - drag.y; drag.x = e.clientX; drag.y = e.clientY;
    if (drag.kind === 'move') wm.move(drag.id, dx, dy); else wm.resize(drag.id, dx, dy);
  }
  function up() { if (drag) { wm.save(); drag = null; } }
  // телефон: видно только активное окно; без активного — домашний экран
  const visible = $derived(device.phone ? wm.wins.filter(w => !w.minimized && w.id === wm.active) : wm.wins.filter(w => !w.minimized));
  function isImg(icon?: string | null) { return !!icon && (icon.startsWith('/') || icon.startsWith('http')); }

  // 🔑 данные для входа в окне приложения (docs/userflow-apps.md, шаг 3); при первом открытии приложения с известным паролем — раскрывается само
  let keyOpen = $state<Record<string, boolean>>({});
  let keyApp = $state<Record<string, AppView>>({});
  const seenApp = new Set<string>();
  async function toggleKey(id: string) {
    if (!keyApp[id]) { try { keyApp[id] = await api.app(id.slice(4)); } catch { return; } }
    keyOpen[id] = !keyOpen[id];
  }
  $effect(() => {
    // закрытое окно: сбросить состояние поповера, чтобы при повторном открытии он не всплывал сам
    const ids = new Set(wm.wins.map(w => w.id));
    for (const id of Object.keys(keyOpen)) if (!ids.has(id)) { delete keyOpen[id]; seenApp.add(id); }
    for (const w of wm.wins) {
      if (!w.id.startsWith('app:') || seenApp.has(w.id)) continue;
      seenApp.add(w.id);
      const n = w.id.slice(4);
      let shown: string | null = null;
      try { shown = localStorage.getItem('cloudos.access.shown.' + n); } catch {}
      if (shown) continue;
      api.app(n).then(a => {
        const m = a.manifest.login?.mode;
        if (m === 'generated' || m === 'default') { keyApp[w.id] = a; keyOpen[w.id] = true; try { localStorage.setItem('cloudos.access.shown.' + n, '1'); } catch {} }
      }).catch(() => {});
    }
  });
</script>

{#each visible as w (w.id)}
  <div class="win" class:active={wm.active === w.id} class:max={w.maximized || device.phone} class:mobile={device.phone} role="dialog" aria-label={winTitle(w)} tabindex="-1"
    style={w.maximized || device.phone ? `z-index:${w.z}; --tb:${device.phone ? NAV_H : TASKBAR_H}px; --status:${STATUS_H}px` : `transform: translate(${w.x}px, ${w.y}px); width:${w.w}px; height:${w.h}px; z-index:${w.z}`}
    onpointerdown={() => wm.focus(w.id)}>
    {#if device.phone}
      <header class="bar mbar" role="toolbar" tabindex="-1" aria-label={t('shell.mbarAria')}>
        <button class="mback" title={t('shell.winHome')} onclick={() => wm.minimize(w.id)}>‹</button>
        {#if isImg(w.icon)}<img src={w.icon} alt="" />{:else if w.icon}<span class="emoji">{w.icon}</span>{/if}
        <span class="title">{winTitle(w)}</span>
        <span class="spacer"></span>
        {#if w.id.startsWith('app:')}<button class="key" class:on={keyOpen[w.id]} title={t('shell.loginDetails')} onclick={() => toggleKey(w.id)}>🔑</button>{/if}
        {#if w.url}<a href={w.url} target="_blank" rel="noopener" title={t('shell.openInBrowser')}><button>↗</button></a>{/if}
        <button class="close" title={t('common.close')} onclick={() => wm.close(w.id)}>✕</button>
      </header>
    {:else}
    <header class="bar" role="toolbar" tabindex="-1" aria-label={t('shell.barAria')} onpointerdown={(e) => down(e, w, 'move')} onpointermove={move} onpointerup={up} ondblclick={() => wm.toggleMax(w.id)}>
      {#if isImg(w.icon)}<img src={w.icon} alt="" />{:else if w.icon}<span class="emoji">{w.icon}</span>{/if}
      <span class="title">{winTitle(w)}</span>
      {#if w.url}<span class="url muted">{w.url.replace(/^https?:\/\//, '')}</span>{/if}
      <span class="spacer"></span>
      <!-- кнопки в духе macOS: цветные кружки, глифы проявляются при наведении на группу; порядок прежний — ключ, новая вкладка, свернуть, развернуть, закрыть -->
      <div class="ctl" role="group" aria-label={t('shell.controlsAria')}>
        {#if w.id.startsWith('app:')}<button class="key" class:on={keyOpen[w.id]} title={t('shell.loginDetails')} aria-label={t('shell.loginDetails')} onclick={() => toggleKey(w.id)}>
          <svg viewBox="0 0 10 10" aria-hidden="true"><circle cx="3.4" cy="6.6" r="1.9" fill="none" stroke="currentColor" stroke-width="1.3"/><path d="M4.8 5.2 8.4 1.6M7.2 2.8l1.2 1.2M6 4l1 1" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/></svg></button>{/if}
        {#if w.url}<a class="ext" href={w.url} target="_blank" rel="noopener" title={t('shell.openNewTab')} aria-label={t('shell.openNewTab')}>
          <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M3 7l4-4M4.2 3H7v2.8" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg></a>{/if}
        <button class="tl min" title={t('shell.minimize')} aria-label={t('shell.minimize')} onclick={() => wm.minimize(w.id)}>
          <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M2 5h6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg></button>
        <button class="tl max" title={w.maximized ? t('shell.restore') : t('shell.maximize')} aria-label={w.maximized ? t('shell.restore') : t('shell.maximize')} onclick={() => wm.toggleMax(w.id)}>
          {#if w.maximized}<svg viewBox="0 0 10 10" aria-hidden="true"><path d="M1.6 1.6h3.6L1.6 5.2ZM8.4 8.4H4.8l3.6-3.6Z" fill="currentColor"/></svg>
          {:else}<svg viewBox="0 0 10 10" aria-hidden="true"><path d="M1.6 8.4V4.8l3.6 3.6ZM8.4 1.6v3.6L4.8 1.6Z" fill="currentColor"/></svg>{/if}</button>
        <button class="tl close" title={t('common.close')} aria-label={t('common.close')} onclick={() => wm.close(w.id)}>
          <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M2.6 2.6l4.8 4.8M7.4 2.6 2.6 7.4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg></button>
      </div>
    </header>
    {/if}
    {#if keyOpen[w.id] && keyApp[w.id]}
      <div class="keypop" role="dialog" aria-label={t('shell.loginDetailsAria')}>
        <div class="row" style="justify-content: space-between"><b>{t('shell.loginDetails')}</b><button title={t('common.closeTitle')} onclick={() => keyOpen[w.id] = false}>✕</button></div>
        <AccessPanel app={keyApp[w.id]} compact />
      </div>
    {/if}
    {#if w.kind === 'iframe'}
      <iframe src={w.url} title={winTitle(w)} allow="clipboard-read; clipboard-write; fullscreen; camera; microphone" sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-downloads allow-modals"></iframe>
    {:else}
      <div class="body">
        {#if w.component === 'projects'}<Projects showIps={appearance.showWorkspaceIps} />
        {:else if w.component === 'store'}<Store />
        {:else if w.component === 'system'}<System />
        {:else if w.component === 'settings'}<Settings bind:appearance />
        {:else if w.component === 'appdetail'}<AppDetail name={String(w.props?.name ?? '')} initialTab={w.props?.tab ? String(w.props.tab) : undefined} />
        {:else if w.component === 'appready'}<AppReady name={String(w.props?.name ?? '')} />
        {:else if w.component === 'vault'}<Vault />
        {:else if w.component === 'setup'}<Setup />
        {:else if w.component === 'updates'}<Updates />
        {:else if w.component === 'reader'}<Reader link={String(w.props?.link ?? '')} feed={w.props?.feed ? String(w.props.feed) : undefined} />
        {:else if w.component === 'project'}<ProjectWindow name={String(w.props?.name ?? '')} showIps={appearance.showWorkspaceIps} />
        {:else if w.component === 'files'}<Files root={String(w.props?.root ?? 'apps')} path={String(w.props?.path ?? '')} />
        {/if}
      </div>
    {/if}
    {#if !w.maximized && !device.phone}<div class="grip" role="separator" aria-label={t('shell.resize')} onpointerdown={(e) => down(e, w, 'resize')} onpointermove={move} onpointerup={up}></div>{/if}
  </div>
{/each}

<style>
  .win { position: fixed; top: 0; left: 0; display: flex; flex-direction: column; background: var(--glass); backdrop-filter: blur(16px); -webkit-backdrop-filter: blur(16px); border: 1px solid var(--line); border-radius: var(--r); box-shadow: var(--shadow); overflow: hidden; opacity: .97; transition: opacity var(--dur); }
  .win.active { opacity: 1; border-color: color-mix(in srgb, var(--accent) 60%, var(--line)); }
  .win.max { inset: 0 0 var(--tb) 0; width: auto !important; height: auto !important; transform: none !important; border-radius: 0; }
  .win.mobile { inset: var(--status) 0 var(--tb) 0; border: 0; border-radius: 0; opacity: 1; background: var(--bg); backdrop-filter: none; -webkit-backdrop-filter: none; }
  .mbar { cursor: default; min-height: 48px; padding: 6px 10px; }
  .mbar .title { font-size: 1rem; overflow: hidden; text-overflow: ellipsis; }
  .mbar button { min-width: 40px; min-height: 36px; font-size: 1rem; }
  .mback { font-size: 1.4rem !important; line-height: 1; }
  .bar { display: flex; align-items: center; gap: 8px; padding: 6px 8px; background: var(--panel-2); border-bottom: 1px solid var(--line); cursor: grab; user-select: none; touch-action: none; flex: none; }
  .bar img { width: 18px; height: 18px; border-radius: 4px; }
  .bar .emoji { font-size: 15px; }
  .bar .title { font-weight: 600; font-size: .9rem; white-space: nowrap; }
  .bar .url { font-size: .75rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 40%; }
  .bar .spacer { flex: 1; }
  .bar button { padding: 0 8px; line-height: 1.6; font-size: .85rem; }
  .bar button.key.on { background: color-mix(in srgb, var(--accent) 30%, transparent); }
  /* кнопки окна как в macOS: кружки 14 px, зона нажатия шире кружка, глифы видны при наведении на группу (у ключа и ↗ — всегда: их смысл не передаётся цветом) */
  .bar .ctl { display: flex; align-items: center; gap: 8px; margin: 0 2px 0 6px; }
  .bar .ctl button, .bar .ctl a.ext { position: relative; width: 14px; height: 14px; padding: 0; border-radius: 50%; border: 1px solid rgb(0 0 0 / .22); display: grid; place-items: center; line-height: 0; color: rgb(0 0 0 / .62);
    background: #C3C8D6; box-shadow: inset 0 1px 0 rgb(255 255 255 / .28); transition: background var(--dur), filter var(--dur), color var(--dur); }
  .bar .ctl button::after, .bar .ctl a.ext::after { content: ""; position: absolute; inset: -6px; }  /* зона нажатия 26 px */
  .bar .ctl svg { width: 9px; height: 9px; transition: opacity var(--dur); }
  .bar .ctl .tl svg { opacity: 0; }
  .bar .ctl:hover .tl svg, .bar .ctl .tl:focus-visible svg { opacity: 1; }
  .bar .ctl .min { background: #FEBC2E; }
  .bar .ctl .max { background: #28C840; }
  .bar .ctl .close { background: #FF5F57; }
  .bar .ctl button:hover, .bar .ctl a.ext:hover { filter: brightness(1.1); border-color: rgb(0 0 0 / .3); }
  .bar .ctl button:active { transform: none; filter: brightness(.85); }
  .bar .ctl .key.on { background: var(--accent); color: var(--on-accent); }
  .win:not(.active) .bar .ctl .tl { background: color-mix(in srgb, var(--ink) 22%, var(--panel-2)); }  /* у неактивного окна светофор гаснет */
  .win:not(.active) .bar .ctl .key:not(.on), .win:not(.active) .bar .ctl a.ext { background: color-mix(in srgb, var(--ink) 30%, var(--panel-2)); color: color-mix(in srgb, var(--ink) 70%, transparent); }
  :root[data-theme="light"] .bar .ctl button, :root[data-theme="light"] .bar .ctl a.ext { border-color: rgb(0 0 0 / .16); }
  :root[data-contrast="high"] .bar .ctl .tl svg { opacity: 1; }
  .keypop { position: absolute; top: 42px; right: 8px; width: min(440px, calc(100% - 16px)); z-index: 3; background: var(--panel); border: 1px solid var(--line); border-radius: var(--r-sm); box-shadow: var(--shadow); padding: 10px 12px; display: flex; flex-direction: column; gap: 8px; }
  .win.mobile .keypop { top: calc(var(--status) + 50px); }
  .mbar .close:hover { border-color: var(--bad); color: var(--bad); }
  iframe { flex: 1; border: 0; background: #fff; }
  .body { flex: 1; overflow: auto; padding: calc(14px * var(--sp)); }
  .grip { position: absolute; right: 0; bottom: 0; width: 18px; height: 18px; cursor: nwse-resize; background: linear-gradient(135deg, transparent 50%, var(--line) 50%); touch-action: none; }
</style>
