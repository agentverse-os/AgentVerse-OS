<!-- Agent browser (docs/agents-ux.md §4.9, step 2): a screencast of the headless Chromium tab from the workspace and
     input into it. The same tab the agent uses through Playwright MCP. Lives in the sidebar (§4.10). -->
<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { api, type BrowserStatus, type BrowserTab } from './api';
  import { data, ui, msg } from './state.svelte';
  import { toast } from './toast.svelte';
  import { t } from './i18n';
  import Icon from './Icon.svelte';
  let { project }: { project: string } = $props();
  let status = $state<BrowserStatus | null>(null);
  let busy = $state(false);
  let error = $state('');
  let tabs = $state<BrowserTab[]>([]);
  let active = $state('');
  let url = $state('');
  let addr = $state('');
  let addrFocused = false;
  let frame = $state('');
  let frameW = $state(0); let frameH = $state(0);
  let frames = $state(0);
  let ws: WebSocket | null = null;
  let img = $state<HTMLImageElement | null>(null);
  let view = $state<HTMLDivElement | null>(null);
  let connected = $state(false);
  let reconnectT: ReturnType<typeof setTimeout> | null = null;
  let ro: ResizeObserver | null = null;
  let lastVp = { w: 0, h: 0 };
  const send = (m: Record<string, unknown>) => { if (ws?.readyState === WebSocket.OPEN) ws.send(JSON.stringify(m)); };
  // Design Mode (§4.9 step 3): highlight under the cursor, a click puts the element (HTML, styles, crop) into the composer
  // of the selected conversation
  let design = $state(false);
  let hoverT = 0;
  let picking = $state(false);
  function setDesign(on: boolean) { design = on; send({ kind: 'design', on }); }
  function onElement(m: any) {
    picking = false;
    if (m.error) { toast(t('design.error', { error: m.error }), 'bad'); return; }
    const styles = Object.entries(m.styles ?? {}).map(([k, v]) => `${k}: ${v}`).join('; ');
    const sizeLine = m.rect ? `${Math.round(m.rect.w)}×${Math.round(m.rect.h)} px` : '';
    const text = `${t('design.element', { url: m.url ?? url, selector: m.selector ?? m.tag ?? '?' })}${sizeLine ? ` · ${sizeLine}` : ''}\n\`\`\`html\n${(m.html ?? '').trim()}\n\`\`\`\n${styles ? `${t('design.styles')}: ${styles}\n` : ''}`;
    const detail: { text: string; image?: { name: string; type: string; data: string }; handled: boolean } = { text, handled: false };
    if (m.png) detail.image = { name: `element-${Date.now()}.png`, type: 'image/png', data: m.png };
    window.dispatchEvent(new CustomEvent('agents:design', { detail }));
    if (detail.handled) toast(t('design.picked', { selector: m.selector ?? m.tag ?? '' })); else toast(t('design.noComposer'), 'bad');
  }

  async function refresh() { try { status = await api.browserStatus(project); error = ''; } catch (e) { error = msg(e); } }
  async function enable() {
    if (busy) return; busy = true;
    try { status = await api.browserSet(project, true); await data.setBrowser(project, { agent: true }); connect(); }
    catch (e) { error = msg(e); toast(msg(e), 'bad'); } finally { busy = false; }
  }
  async function disable() {
    if (busy) return; busy = true;
    try { close(); await api.browserSet(project, false).catch(() => {}); await data.setBrowser(project, { agent: false }); toast(t('abrowser.disabledToast', { project })); }
    catch (e) { toast(msg(e), 'bad'); } finally { busy = false; }
  }
  async function install() {
    if (busy) return; busy = true;
    try { const p = await api.browserInstall(project); toast(t('abrowser.installing')); ui.open({ project, kind: 'pty', id: p.id }); }
    catch (e) { toast(msg(e), 'bad'); } finally { busy = false; }
  }
  function connect(target = '') {
    close();
    const sock = new WebSocket(api.browserWsUrl(project, target)); ws = sock;
    sock.onopen = () => { connected = true; error = ''; sendViewport(true); };
    sock.onmessage = (ev) => {
      let m: any; try { m = JSON.parse(String(ev.data)); } catch { return; }
      if (m.kind === 'frame') { frame = `data:image/jpeg;base64,${m.data}`; frameW = m.w ?? 0; frameH = m.h ?? 0; frames++; }
      else if (m.kind === 'tabs') { tabs = m.list ?? []; active = m.active ?? active; const cur = tabs.find(x => x.id === active); if (cur) { url = cur.url; if (!addrFocused) addr = cur.url; } }
      else if (m.kind === 'nav') { url = m.url ?? ''; if (!addrFocused) addr = url; }
      else if (m.kind === 'error') { error = m.error ?? ''; }
      else if (m.kind === 'element') onElement(m);
    };
    sock.onclose = () => { connected = false; if (ws === sock) { ws = null; if (data.browserOf(project).agent && !error) reconnectT = setTimeout(() => connect(active), 2000); } };
    sock.onerror = () => { try { sock.close(); } catch {} };
  }
  function close() { if (reconnectT) { clearTimeout(reconnectT); reconnectT = null; } const s = ws; ws = null; try { s?.close(); } catch {} connected = false; }
  function sendViewport(force = false) {
    const el = view; if (!el) return;
    const w = Math.max(240, Math.floor(el.clientWidth)), h = Math.max(200, Math.floor(el.clientHeight));
    if (!force && Math.abs(w - lastVp.w) < 8 && Math.abs(h - lastVp.h) < 8) return;
    lastVp = { w, h }; send({ kind: 'viewport', w, h });
  }
  // mouse coordinates: the image is shown 1:1 with the browser window (viewport = the area size), but we scale just in
  // case
  function pos(e: MouseEvent) { const el = img!; const r = el.getBoundingClientRect(); const sx = frameW ? frameW / r.width : 1, sy = frameH ? frameH / r.height : 1; return { x: Math.round((e.clientX - r.left) * sx), y: Math.round((e.clientY - r.top) * sy) }; }
  const btn = (e: MouseEvent) => (e.button === 2 ? 'right' : e.button === 1 ? 'middle' : 'left');
  const mods = (e: MouseEvent | KeyboardEvent) => (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
  function mouse(type: string, e: MouseEvent) {
    const p = pos(e);
    if (design) {
      if (type === 'mouseMoved') { const now = Date.now(); if (now - hoverT > 60) { hoverT = now; send({ kind: 'hover', x: p.x, y: p.y }); } }
      else if (type === 'mousePressed') { if (!picking) { picking = true; send({ kind: 'inspect', x: p.x, y: p.y }); } }
      return;
    }
    send({ kind: 'mouse', type, x: p.x, y: p.y, button: type === 'mouseMoved' ? 'none' : btn(e), clickCount: type === 'mouseMoved' ? 0 : e.detail || 1, modifiers: mods(e) });
  }
  function wheel(e: WheelEvent) { e.preventDefault(); const p = pos(e); send({ kind: 'mouse', type: 'mouseWheel', x: p.x, y: p.y, deltaX: Math.round(e.deltaX), deltaY: Math.round(e.deltaY), modifiers: mods(e) }); }
  const VK: Record<string, number> = { Enter: 13, Backspace: 8, Tab: 9, Escape: 27, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40, Delete: 46, Home: 36, End: 35, PageUp: 33, PageDown: 34, ' ': 32 };
  function key(type: 'keyDown' | 'keyUp', e: KeyboardEvent) {
    e.preventDefault();
    if (design) { if (e.key === 'Escape' && type === 'keyDown') setDesign(false); return; }
    const printable = e.key.length === 1 && !e.ctrlKey && !e.metaKey;
    const m: Record<string, unknown> = { kind: 'key', type: printable && type === 'keyDown' ? 'keyDown' : type, key: e.key, code: e.code, modifiers: mods(e) };
    if (VK[e.key] != null) m.windowsVirtualKeyCode = VK[e.key];
    if (printable && type === 'keyDown') { m.text = e.key; }
    send(m);
    if (printable && type === 'keyDown' && e.key !== ' ') { /* text is already in keyDown */ }
  }
  function paste(e: ClipboardEvent) { const txt = e.clipboardData?.getData('text'); if (txt) { e.preventDefault(); send({ kind: 'text', text: txt }); } }
  function go() { const u = addr.trim(); if (!u) return; send({ kind: 'nav', url: u }); }
  $effect(() => { refresh(); });
  $effect(() => { if (status?.running && data.browserOf(project).agent && !ws) connect(); });
  // the flag is on, Chromium is installed but not running (a workspace restart before the autostart) — bring it up
  // ourselves once
  let autoTried = false;
  $effect(() => { if (status && status.installed && !status.running && data.browserOf(project).agent && !busy && !autoTried) { autoTried = true; enable(); } });
  $effect(() => { const el = view; if (!el) return; ro = new ResizeObserver(() => sendViewport()); ro.observe(el); return () => ro?.disconnect(); });
  onDestroy(close);
</script>

<div class="abrowser">
  {#if error && !connected}<div class="composer-error ab-error">{error} <button class="xs" onclick={() => { error = ''; refresh(); if (status?.running) connect(); }}>{t('browser.reload')}</button></div>{/if}
  {#if !status}<div class="muted sm">{t('common.loading')}</div>
  {:else if !status.installed}
    <div class="ab-setup"><p class="muted sm">{t('abrowser.notInstalled')}</p><button class="sm primary ab-install" disabled={busy} onclick={install}><Icon name="download" size="s" /> {t('abrowser.install')}</button></div>
  {:else if !status.running || !data.browserOf(project).agent}
    <div class="ab-setup"><p class="muted sm">{t('abrowser.off')}</p><button class="sm primary ab-enable" disabled={busy} onclick={enable}><Icon name="globe" size="s" /> {busy ? t('abrowser.starting') : t('abrowser.enable')}</button></div>
  {:else}
    <div class="ab-bar">
      <select class="sm ab-tabs" aria-label={t('abrowser.tabs')} value={active} onchange={(e) => connect((e.currentTarget as HTMLSelectElement).value)}>{#each tabs as tb (tb.id)}<option value={tb.id}>{tb.title || tb.url || tb.id.slice(0, 6)}</option>{/each}</select>
      <button class="icon xs ghost" title={t('abrowser.newTab')} aria-label={t('abrowser.newTab')} onclick={() => send({ kind: 'newtab', url: 'about:blank' })}><Icon name="plus" size="s" /></button>
      <button class="icon xs ghost" title={t('abrowser.back')} aria-label={t('abrowser.back')} onclick={() => send({ kind: 'back' })}><Icon name="chevronLeft" size="s" /></button>
      <button class="icon xs ghost" title={t('abrowser.forward')} aria-label={t('abrowser.forward')} onclick={() => send({ kind: 'forward' })}><Icon name="chevronRight" size="s" /></button>
      <button class="icon xs ghost" title={t('browser.reload')} aria-label={t('browser.reload')} onclick={() => send({ kind: 'reload' })}><Icon name="refresh" size="s" /></button>
      <input class="sm mono ab-addr" bind:value={addr} aria-label={t('abrowser.address')} placeholder="http://127.0.0.1:3000" onfocus={() => (addrFocused = true)} onblur={() => { addrFocused = false; addr = url; }} onkeydown={(e) => { if (e.key === 'Enter') { go(); (e.currentTarget as HTMLInputElement).blur(); } }} />
      <button class="xs ab-design" class:primary={design} title={t('design.hint')} aria-pressed={design} onclick={() => setDesign(!design)}><Icon name="crosshair" size="s" /> <span class="hide-p">{t('design.toggle')}</span></button>
      <button class="xs ghost ab-off" disabled={busy} title={t('browser.off')} onclick={disable}><Icon name="x" size="s" /> <span class="hide-p">{t('browser.off')}</span></button>
    </div>
    {#if design}<div class="muted xs ab-design-hint" role="status">{t('design.hint')}</div>{/if}
    <div class="ab-view" class:design bind:this={view}>
      {#if frame}
        <!-- the image is the browser window itself: clicks, the wheel and the keyboard go to the page (like a terminal), hence the surface is
             application -->
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <div class="ab-surface" role="application" aria-label={t('abrowser.frameAlt')} tabindex="0"
          onmousedown={(e) => { (e.currentTarget as HTMLElement).focus(); mouse('mousePressed', e); }} onmouseup={(e) => mouse('mouseReleased', e)} onmousemove={(e) => mouse('mouseMoved', e)} onwheel={wheel} oncontextmenu={(e) => e.preventDefault()}
          onkeydown={(e) => key('keyDown', e)} onkeyup={(e) => key('keyUp', e)} onpaste={paste}>
          <img bind:this={img} class="ab-frame" src={frame} alt="" draggable="false" data-frames={frames} />
        </div>
      {:else}<div class="muted sm ab-wait" role="status">{connected ? t('abrowser.waitingFrame') : t('abrowser.connecting')}</div>{/if}
    </div>
    <div class="muted xs ab-status"><span class="mono ell">{url || '—'}</span>{#if status.mcp}<span class="ab-mcp" title={t('abrowser.mcpHint')}>· MCP</span>{/if}</div>
  {/if}
</div>

<style>
  .abrowser { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 6px; }
  .ab-bar { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; }
  .ab-tabs { max-width: 160px; }
  .ab-addr { flex: 1 1 160px; min-width: 120px; }
  .ab-view { flex: 1; min-height: 0; overflow: hidden; background: var(--bg-3); border: 1px solid var(--line-1); border-radius: var(--r-2); display: flex; align-items: flex-start; justify-content: flex-start; }
  .ab-surface { display: block; max-width: 100%; max-height: 100%; outline: none; cursor: default; user-select: none; }
  .ab-view.design .ab-surface { cursor: crosshair; }
  .ab-view.design { outline: 2px solid var(--accent); outline-offset: -2px; }
  .ab-design-hint { padding: 0 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .ab-surface:focus-visible { box-shadow: inset 0 0 0 2px var(--accent); }
  .ab-frame { display: block; max-width: 100%; max-height: 100%; -webkit-user-drag: none; pointer-events: none; }
  .ab-wait { padding: 12px; }
  .ab-setup { display: flex; flex-direction: column; gap: 8px; align-items: flex-start; padding: 6px 0; }
  .ab-status { display: flex; gap: 6px; min-width: 0; padding: 0 2px; }
  .ab-status .ell { min-width: 0; }
  .ab-error { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
</style>
