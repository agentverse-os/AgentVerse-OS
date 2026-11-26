<script lang="ts">
  import { wm, winTitle, type Win, type SnapZone, type Rect } from './windows.svelte';
  import { t } from './i18n.svelte';
  import { device } from './device.svelte';
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
  import Agents from './Agents.svelte';
  import AppIcon, { winIcon } from './AppIcon.svelte';
  import Glyph from './Glyph.svelte';
  import type { Appearance } from './theme';
  let { appearance = $bindable() }: { appearance: Appearance } = $props();

  // Move by the title bar, resize by any edge or corner. A window dragged to the left or right edge snaps to a half of the
  // screen, to the top — maximizes; a snapped or maximized window picked up by its title returns to its floating size.
  type Drag = { id: string; kind: 'move' | 'resize'; dir: string; x: number; y: number; sx: number; sy: number; pending: boolean };
  let drag: Drag | null = null;
  let preview = $state<SnapZone | 'max' | null>(null);
  function down(e: PointerEvent, w: Win, kind: 'move' | 'resize', dir = '') {
    if (e.pointerType === 'mouse' && e.button !== 0) return;
    if ((e.target as HTMLElement).closest('button, a')) return;
    wm.focus(w.id);
    const pending = kind === 'move' && (w.maximized || !!w.snap);
    if (!pending) wm.adopt(w.id);
    drag = { id: w.id, kind, dir, x: e.clientX, y: e.clientY, sx: e.clientX, sy: e.clientY, pending };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) {
    if (!drag) return;
    const w = wm.wins.find(x => x.id === drag!.id);
    if (!w) return;
    const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
    if (drag.kind === 'resize') { drag.x = e.clientX; drag.y = e.clientY; wm.resizeDir(w.id, drag.dir, dx, dy); return; }
    if (drag.pending) {
      if (Math.hypot(e.clientX - drag.sx, e.clientY - drag.sy) < 8) return;
      const from = wm.rect(w);
      const ratio = Math.min(0.9, Math.max(0.1, (e.clientX - from.x) / from.w));
      w.maximized = false; w.snap = null;
      const r = wm.rect(w);
      w.w = r.w; w.h = r.h; w.x = Math.round(e.clientX - r.w * ratio); w.y = Math.max(0, Math.round(e.clientY - 18));
      drag.pending = false; drag.x = e.clientX; drag.y = e.clientY;
      return;
    }
    drag.x = e.clientX; drag.y = e.clientY;
    wm.move(w.id, dx, dy);
    preview = e.clientY <= 3 ? 'max' : e.clientX <= 5 ? 'left' : e.clientX >= wm.vw - 6 ? 'right' : null;
  }
  function up() {
    if (!drag) return;
    if (drag.kind === 'move' && preview) { if (preview === 'max') wm.toggleMax(drag.id); else wm.snap(drag.id, preview); }
    wm.save(); drag = null; preview = null;
  }
  const previewRect = $derived<Rect | null>(preview === 'max' ? wm.workArea() : preview ? wm.zoneRect(preview) : null);

  // Layout picker: hovering the maximize button offers the snap zones of this screen (thirds on wide ones)
  let picker = $state<string | null>(null);
  let pickTimer: ReturnType<typeof setTimeout> | undefined;
  const ZONE: Record<SnapZone, [number, number]> = { left: [0, .5], right: [.5, 1], l3: [0, 1 / 3], c3: [1 / 3, 2 / 3], r3: [2 / 3, 1], l23: [0, 2 / 3], r23: [1 / 3, 1] };
  function pickSoon(e: PointerEvent, id: string) { if (e.pointerType !== 'mouse') return; clearTimeout(pickTimer); pickTimer = setTimeout(() => picker = id, 380); }
  function pickLater() { clearTimeout(pickTimer); pickTimer = setTimeout(() => picker = null, 260); }
  function pickKeep() { clearTimeout(pickTimer); }
  function choose(id: string, z: SnapZone | 'max') { clearTimeout(pickTimer); picker = null; if (z === 'max') { const w = wm.wins.find(x => x.id === id); if (w && !w.maximized) wm.toggleMax(id); } else wm.snap(id, z); }

  // phone: only the active window is visible; without an active one —
  // the home screen
  const visible = $derived(device.phone ? wm.wins.filter(w => !w.minimized && w.id === wm.active) : wm.wins.filter(w => !w.minimized));
  function frame(w: Win) {
    if (device.phone) return `z-index:${w.z}`;
    const r = wm.rect(w);
    return `transform: translate(${r.x}px, ${r.y}px); width:${r.w}px; height:${r.h}px; z-index:${w.z}`;
  }

  // 🔑 sign-in data in the app window (docs/userflow-apps.md, step 3); on the first open of an app with a known password it expands by itself
  let keyOpen = $state<Record<string, boolean>>({});
  let keyApp = $state<Record<string, AppView>>({});
  const seenApp = new Set<string>();
  async function toggleKey(id: string) {
    if (!keyApp[id]) { try { keyApp[id] = await api.app(id.slice(4)); } catch { return; } }
    keyOpen[id] = !keyOpen[id];
  }
  $effect(() => {
    // a closed window: reset the popover state so it does not pop up by itself on a reopen
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

{#if previewRect}<div class="snap-preview" style="transform: translate({previewRect.x}px, {previewRect.y}px); width:{previewRect.w}px; height:{previewRect.h}px"></div>{/if}

{#each visible as w (w.id)}
  <div class="win" class:active={wm.active === w.id} class:max={w.maximized || device.phone} class:snapped={!!w.snap && !w.maximized && !device.phone} class:mobile={device.phone} class:settle={wm.settling === w.id} role="dialog" aria-label={winTitle(w)} tabindex="-1"
    style={frame(w)} onpointerdown={() => wm.focus(w.id)}>
    {#if device.phone}
      <header class="bar mbar" role="toolbar" tabindex="-1" aria-label={t('shell.mbarAria')}>
        <button class="mback" title={t('shell.winHome')} aria-label={t('shell.winHome')} onclick={() => wm.minimize(w.id)}><Glyph name="back" size={22} /></button>
        <AppIcon icon={winIcon(w)} size={24} label={winTitle(w)} />
        <span class="title">{winTitle(w)}</span>
        <span class="spacer"></span>
        {#if w.id.startsWith('app:')}<button class="key" class:on={keyOpen[w.id]} title={t('shell.loginDetails')} aria-label={t('shell.loginDetails')} onclick={() => toggleKey(w.id)}><Glyph name="key" size={20} /></button>{/if}
        {#if w.url}<a class="ext" href={w.url} target="_blank" rel="noopener" title={t('shell.openInBrowser')} aria-label={t('shell.openInBrowser')}><Glyph name="external" size={20} /></a>{/if}
        <button class="close" title={t('common.close')} aria-label={t('common.close')} onclick={() => wm.close(w.id)}><Glyph name="close" size={20} /></button>
      </header>
    {:else}
    <header class="bar" role="toolbar" tabindex="-1" aria-label={t('shell.barAria')} onpointerdown={(e) => down(e, w, 'move')} onpointermove={move} onpointerup={up} onpointercancel={up} ondblclick={() => wm.toggleMax(w.id)}>
      <AppIcon icon={winIcon(w)} size={20} label={winTitle(w)} />
      <span class="title">{winTitle(w)}</span>
      {#if w.url}<span class="url">{w.url.replace(/^https?:\/\//, '')}</span>{/if}
      <span class="spacer"></span>
      <!-- macOS-style buttons: colored circles, the glyphs appear on hovering the group; the order is as before — key, new tab, minimize, maximize, close -->
      <div class="ctl" role="group" aria-label={t('shell.controlsAria')}>
        {#if w.id.startsWith('app:')}<button class="key" class:on={keyOpen[w.id]} title={t('shell.loginDetails')} aria-label={t('shell.loginDetails')} onclick={() => toggleKey(w.id)}>
          <svg viewBox="0 0 10 10" aria-hidden="true"><circle cx="3.4" cy="6.6" r="1.9" fill="none" stroke="currentColor" stroke-width="1.3"/><path d="M4.8 5.2 8.4 1.6M7.2 2.8l1.2 1.2M6 4l1 1" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/></svg></button>{/if}
        {#if w.url}<a class="ext" href={w.url} target="_blank" rel="noopener" title={t('shell.openNewTab')} aria-label={t('shell.openNewTab')}>
          <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M3 7l4-4M4.2 3H7v2.8" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg></a>{/if}
        <button class="tl min" title={t('shell.minimize')} aria-label={t('shell.minimize')} onclick={() => wm.minimize(w.id)}>
          <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M2 5h6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg></button>
        <button class="tl max" title={w.maximized ? t('shell.restore') : t('shell.maximize')} aria-label={w.maximized ? t('shell.restore') : t('shell.maximize')} onclick={() => { clearTimeout(pickTimer); picker = null; wm.toggleMax(w.id); }}
          onpointerenter={(e) => pickSoon(e, w.id)} onpointerleave={pickLater}>
          {#if w.maximized}<svg viewBox="0 0 10 10" aria-hidden="true"><path d="M1.6 1.6h3.6L1.6 5.2ZM8.4 8.4H4.8l3.6-3.6Z" fill="currentColor"/></svg>
          {:else}<svg viewBox="0 0 10 10" aria-hidden="true"><path d="M1.6 8.4V4.8l3.6 3.6ZM8.4 1.6v3.6L4.8 1.6Z" fill="currentColor"/></svg>{/if}</button>
        <button class="tl close" title={t('common.close')} aria-label={t('common.close')} onclick={() => wm.close(w.id)}>
          <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M2.6 2.6l4.8 4.8M7.4 2.6 2.6 7.4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg></button>
      </div>
    </header>
    {#if picker === w.id}
      <div class="layouts" role="menu" tabindex="-1" aria-label={t('shell.maximize')} onpointerenter={pickKeep} onpointerleave={pickLater}>
        {#each wm.zones as z}
          <button class="snap" class:on={w.snap === z} role="menuitem" aria-label={z} onclick={() => choose(w.id, z)}><i style="left:{ZONE[z][0] * 100}%; width:{(ZONE[z][1] - ZONE[z][0]) * 100}%"></i></button>
        {/each}
        <button class="snap" class:on={w.maximized} role="menuitem" aria-label={t('shell.maximize')} onclick={() => choose(w.id, 'max')}><i style="left:0; width:100%"></i></button>
      </div>
    {/if}
    {/if}
    {#if keyOpen[w.id] && keyApp[w.id]}
      <div class="keypop" role="dialog" aria-label={t('shell.loginDetailsAria')}>
        <div class="row" style="justify-content: space-between"><b>{t('shell.loginDetails')}</b><button class="plain" title={t('common.closeTitle')} aria-label={t('common.closeTitle')} onclick={() => keyOpen[w.id] = false}><Glyph name="close" size={16} /></button></div>
        <AccessPanel app={keyApp[w.id]} compact />
      </div>
    {/if}
    {#if w.kind === 'iframe'}
      <iframe src={w.url} title={winTitle(w)} allow="clipboard-read; clipboard-write; fullscreen; camera; microphone" sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-downloads allow-modals"></iframe>
    {:else}
      <div class="body" class:flush={w.component === 'agents'}>
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
        {:else if w.component === 'agents'}<Agents />
        {/if}
      </div>
    {/if}
    {#if !w.maximized && !w.snap && !device.phone}
      {#each ['n', 's', 'e', 'w', 'ne', 'nw', 'se', 'sw'] as dir}
        <div class="rz {dir}" class:grip={dir === 'se'} role="separator" aria-label={dir === 'se' ? t('shell.resize') : undefined} aria-hidden={dir === 'se' ? undefined : 'true'}
          onpointerdown={(e) => down(e, w, 'resize', dir)} onpointermove={move} onpointerup={up} onpointercancel={up}></div>
      {/each}
    {/if}
  </div>
{/each}

<style>
  /* the window is drawn from the --win-* / --bar-* tokens (app.css), so a theme changes the material, not this file */
  .win { position: fixed; top: 0; left: 0; display: flex; flex-direction: column; background: var(--win-bg); backdrop-filter: var(--win-blur); -webkit-backdrop-filter: var(--win-blur); border: 1px solid var(--win-line); border-radius: var(--win-r); box-shadow: var(--shadow-2); overflow: hidden; outline: 0; container: winbox / inline-size; }
  .win.active { box-shadow: var(--win-shadow); border-color: color-mix(in oklab, var(--accent) 34%, var(--win-line)); }
  /* a window appears with a short fade and grow; `scale` is its own property, so it does not fight the translate of the frame */
  .win { transition: opacity calc(var(--dur) * 1.1) var(--ease), scale calc(var(--dur) * 1.1) var(--ease); }
  @starting-style { .win { opacity: 0; scale: .975; } }
  /* maximize, restore and snap move the frame smoothly; dragging and resizing stay instant */
  .win.settle { transition: transform calc(var(--dur) * 1.4) var(--ease), width calc(var(--dur) * 1.4) var(--ease), height calc(var(--dur) * 1.4) var(--ease), border-radius var(--dur); }
  .win.max { border-radius: 0; border-width: 0 0 1px; }
  .win.snapped { border-radius: var(--win-r); }
  .win.mobile { width: auto; height: auto; transform: none; scale: 1; inset: calc(var(--m-status) + env(safe-area-inset-top, 0px)) calc(var(--m-nav-r) + env(safe-area-inset-right, 0px)) calc(var(--m-nav-b) + env(safe-area-inset-bottom, 0px)) env(safe-area-inset-left, 0px); border: 0; border-radius: 0; background: var(--bg); backdrop-filter: none; -webkit-backdrop-filter: none; box-shadow: none; }

  .bar { display: flex; align-items: center; gap: 9px; min-height: 40px; padding: 0 10px 0 12px; background: var(--bar-bg); border-bottom: 1px solid var(--line); cursor: grab; user-select: none; -webkit-user-select: none; touch-action: none; flex: none; }
  .bar:active { cursor: grabbing; }
  .bar .title { font-weight: 600; font-size: .9rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; letter-spacing: -.01em; }
  .win:not(.active) .bar .title { color: var(--muted); }
  .bar .url { font-size: .73rem; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; max-width: 42%; padding: 1px 8px; border-radius: 999px; background: var(--fill); }
  .bar .spacer { flex: 1; min-width: 8px; }
  @container winbox (max-width: 520px) { .bar .url { display: none; } }

  /* window buttons as in macOS: 14 px circles, the hit area wider than the circle, the glyphs visible on hovering the group (for the key and ↗ — always: their
     meaning is not conveyed by color) */
  .bar .ctl { display: flex; align-items: center; gap: 8px; margin: 0 2px 0 6px; flex: none; }
  .bar .ctl button, .bar .ctl a.ext { position: relative; width: 14px; height: 14px; padding: 0; border-radius: 50%; border: 1px solid rgb(0 0 0 / .22); display: grid; place-items: center; line-height: 0; color: rgb(0 0 0 / .62);
    background: #C3C8D6; box-shadow: inset 0 1px 0 rgb(255 255 255 / .28); transition: background var(--dur), filter var(--dur), color var(--dur); }
  .bar .ctl button::after, .bar .ctl a.ext::after { content: ""; position: absolute; inset: -6px; }  /* 26 px hit area */
  .bar .ctl svg { width: 9px; height: 9px; transition: opacity var(--dur); }
  .bar .ctl .tl svg { opacity: 0; }
  .bar .ctl:hover .tl svg, .bar .ctl .tl:focus-visible svg { opacity: 1; }
  .bar .ctl .min { background: #FEBC2E; }
  .bar .ctl .max { background: #28C840; }
  .bar .ctl .close { background: #FF5F57; }
  .bar .ctl button:hover, .bar .ctl a.ext:hover { filter: brightness(1.1); border-color: rgb(0 0 0 / .3); }
  .bar .ctl button:active { transform: none; filter: brightness(.85); }
  .bar .ctl .key.on { background: var(--accent); color: var(--on-accent); }
  .win:not(.active) .bar .ctl .tl { background: color-mix(in oklab, var(--ink) 24%, var(--panel-2)); }  /* on an inactive window the traffic lights go out */
  .win:not(.active) .bar .ctl .key:not(.on), .win:not(.active) .bar .ctl a.ext { background: color-mix(in oklab, var(--ink) 30%, var(--panel-2)); color: color-mix(in oklab, var(--ink) 70%, transparent); }
  :global(:root[data-theme="light"]) .bar .ctl button, :global(:root[data-theme="light"]) .bar .ctl a.ext { border-color: rgb(0 0 0 / .16); }
  :global(:root[data-contrast="high"]) .bar .ctl .tl svg { opacity: 1; }
  /* a touch screen: bigger circles, the glyphs always visible */
  @media (pointer: coarse) {
    .bar { min-height: 46px; }
    .bar .ctl { gap: 14px; margin-right: 6px; }
    .bar .ctl button, .bar .ctl a.ext { width: 20px; height: 20px; }
    .bar .ctl button::after, .bar .ctl a.ext::after { inset: -9px; }
    .bar .ctl svg { width: 11px; height: 11px; }
    .bar .ctl .tl svg { opacity: 1; }
  }

  /* the umbrel look: a "sheet" — no title strip, a bold title, round glass buttons instead of the traffic lights; a maximized
     sheet floats with margins */
  :global(:root[data-look="umbrel"]) .win.active { border-color: rgb(255 255 255 / .16); }
  :global(:root[data-look="umbrel"]) .win.max:not(.mobile) { border-radius: var(--win-r); border-width: 1px; }
  :global(:root[data-look="umbrel"]) .bar:not(.mbar) { border-bottom: 0; min-height: 48px; padding: 6px 12px 0 18px; }
  :global(:root[data-look="umbrel"]) .bar:not(.mbar) .title { font-size: 1.05rem; font-weight: 700; letter-spacing: -.02em; }
  :global(:root[data-look="umbrel"]) .bar .ctl { gap: 6px; }
  :global(:root[data-look="umbrel"]) .bar .ctl button, :global(:root[data-look="umbrel"]) .bar .ctl a.ext { width: 26px; height: 26px; background: rgb(255 255 255 / .1); border: .5px solid rgb(255 255 255 / .12); color: rgb(255 255 255 / .85); box-shadow: inset 0 .5px 0 rgb(255 255 255 / .2); }
  :global(:root[data-look="umbrel"]) .bar .ctl button::after, :global(:root[data-look="umbrel"]) .bar .ctl a.ext::after { inset: -3px; }
  :global(:root[data-look="umbrel"]) .bar .ctl svg, :global(:root[data-look="umbrel"]) .bar .ctl .tl svg { opacity: 1; width: 11px; height: 11px; }
  :global(:root[data-look="umbrel"]) .bar .ctl button:hover, :global(:root[data-look="umbrel"]) .bar .ctl a.ext:hover { background: rgb(255 255 255 / .18); filter: none; border-color: rgb(255 255 255 / .2); }
  :global(:root[data-look="umbrel"]) .bar .ctl .close:hover { background: #E22C2C; color: #fff; }
  :global(:root[data-look="umbrel"]) .bar .ctl .key.on { background: var(--accent); color: #fff; }
  :global(:root[data-look="umbrel"]) .win:not(.active) .bar .ctl .tl, :global(:root[data-look="umbrel"]) .win:not(.active) .bar .ctl .key:not(.on), :global(:root[data-look="umbrel"]) .win:not(.active) .bar .ctl a.ext { background: rgb(255 255 255 / .06); color: rgb(255 255 255 / .5); }
  :global(:root[data-look="umbrel"]) .body:not(.flush) { --bpx: calc(18px * var(--sp)); --bpt: calc(8px * var(--sp)); padding-bottom: calc(18px * var(--sp)); }

  /* phone: a full-screen app with a toolbar — back to the home screen, the title, the actions */
  .mbar { cursor: default; min-height: 52px; padding: 4px 6px; gap: 8px; touch-action: auto; }
  .mbar .title { font-size: 1.02rem; }
  .mbar button, .mbar a.ext { display: grid; place-items: center; width: 44px; height: 44px; padding: 0; border: 0; border-radius: 50%; background: transparent; color: var(--ink); box-shadow: none; flex: none; }
  .mbar button:hover, .mbar a.ext:hover { background: var(--fill); }
  .mbar .key.on { background: var(--accent-soft); color: var(--accent-ink); }
  .mbar .close:hover { color: var(--bad); }

  /* layout picker under the maximize button */
  .layouts { position: absolute; top: 38px; right: 8px; z-index: 5; display: flex; flex-wrap: wrap; gap: 6px; max-width: 232px; padding: 8px; background: var(--pop-bg); backdrop-filter: var(--pop-blur); -webkit-backdrop-filter: var(--pop-blur); border: 1px solid var(--line-2); border-radius: var(--r-sm); box-shadow: var(--pop-shadow); }
  .layouts .snap { position: relative; width: 48px; height: 30px; padding: 0; border-radius: 5px; background: var(--fill); border: 1px solid var(--line-2); overflow: hidden; }
  .layouts .snap i { position: absolute; top: 2px; bottom: 2px; margin: 0 2px; border-radius: 3px; background: color-mix(in oklab, var(--ink) 34%, transparent); transition: background var(--dur); }
  .layouts .snap:hover i, .layouts .snap.on i { background: var(--accent); }
  .snap-preview { position: fixed; top: 0; left: 0; z-index: 9; border-radius: var(--win-r); background: var(--accent-soft); border: 1.5px solid var(--accent-line); pointer-events: none; transition: transform var(--dur) var(--ease), width var(--dur) var(--ease), height var(--dur) var(--ease); }

  .keypop { position: absolute; top: 46px; right: 8px; width: min(440px, calc(100% - 16px)); z-index: 3; background: var(--pop-bg); backdrop-filter: var(--pop-blur); -webkit-backdrop-filter: var(--pop-blur); border: 1px solid var(--line-2); border-radius: var(--pop-r); box-shadow: var(--pop-shadow); padding: 10px 12px; display: flex; flex-direction: column; gap: 8px; }
  .keypop .plain { display: grid; place-items: center; width: 28px; height: 28px; padding: 0; border-radius: 50%; background: transparent; border-color: transparent; }
  .keypop .plain:hover { background: var(--fill-2); }
  .win.mobile .keypop { top: 56px; }

  iframe { flex: 1; border: 0; background: #fff; min-height: 0; }
  /* the content adapts to the window, not to the screen: system apps query this container (@container win) */
  /* --bpx / --bpt are the body paddings: a full-bleed element inside (a sticky toolbar) takes them back with negative margins */
  .body { --bpx: calc(16px * var(--sp)); --bpt: calc(16px * var(--sp)); flex: 1; min-height: 0; overflow: auto; padding: var(--bpt) var(--bpx) calc(16px * var(--sp)); container: win / inline-size; }
  .body.flush { padding: 0; overflow: hidden; display: flex; }
  .body.flush > :global(.agents) { flex: 1; min-height: 0; }

  /* resize handles: thin strips along the edges and squares in the corners; the bottom-right one is drawn as a grip */
  .rz { position: absolute; z-index: 4; touch-action: none; }
  .rz.n, .rz.s { left: 12px; right: 12px; height: 6px; cursor: ns-resize; } .rz.n { top: 0; } .rz.s { bottom: 0; }
  .rz.e, .rz.w { top: 12px; bottom: 12px; width: 6px; cursor: ew-resize; } .rz.e { right: 0; } .rz.w { left: 0; }
  .rz.ne, .rz.nw, .rz.se, .rz.sw { width: 14px; height: 14px; }
  .rz.ne { top: 0; right: 0; cursor: nesw-resize; } .rz.nw { top: 0; left: 0; cursor: nwse-resize; } .rz.sw { bottom: 0; left: 0; cursor: nesw-resize; }
  .rz.se { right: 0; bottom: 0; width: 18px; height: 18px; cursor: nwse-resize; }
  .rz.grip::after { content: ""; position: absolute; right: 4px; bottom: 4px; width: 8px; height: 8px; border-right: 2px solid color-mix(in oklab, var(--ink) 30%, transparent); border-bottom: 2px solid color-mix(in oklab, var(--ink) 30%, transparent); border-bottom-right-radius: 4px; }
  @media (pointer: coarse) { .rz.se { width: 28px; height: 28px; } .rz.n, .rz.s { height: 10px; } .rz.e, .rz.w { width: 10px; } }
</style>
