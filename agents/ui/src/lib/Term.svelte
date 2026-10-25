<script module lang="ts">
  // Retain paths across tab changes, including an upload that finishes after the terminal unmounts.
  const pendingAttachments = $state<Record<string, string[]>>({});
</script>

<script lang="ts">
  // Session card terminal (ADR-006): xterm.js over a PTY that lives in cloudos-agentd inside the workspace.
  // Protocol v1: binary frames are bytes, text frames are JSON (`hello`, `resize`, `ping`/`pong`, `exit`).
  // Closing the window or tab does not touch the process: on the next connection agentd replays the output
  // tail.
  import { onMount } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';
  import { api } from './api';
  import { t } from './i18n';
  import { toast } from './toast.svelte';
  type Connection = 'connecting' | 'live' | 'exited' | 'lost';
  let { project, id, onexit, onconnection }: { project: string; id: string; onexit?: (code: number) => void; onconnection?: (state: Connection) => void } = $props();
  let host: HTMLDivElement;
  let conn = $state<Connection>('connecting');
  let exitCode = $state<number | null>(null);
  let term: Terminal | null = null;
  let socket: WebSocket | null = null;
  let active = false;
  const attachmentKey = $derived(`${project}:${id}`);
  const pendingPaths = $derived(pendingAttachments[attachmentKey] ?? []);
  let inserting = $state(false);
  const enc = new TextEncoder();
  function setConnection(state: Connection) {
    conn = state;
    if (term) term.options.disableStdin = state !== 'live';
    onconnection?.(state);
  }
  export function sendKey(seq: string) {
    if (conn !== 'live' || socket?.readyState !== WebSocket.OPEN) return false;
    socket.send(enc.encode(seq)); return true;
  }
  function insertPath(path: string) {
    if (!active || !term || conn !== 'live' || socket?.readyState !== WebSocket.OPEN) return false;
    term.paste(path); term.focus(); return true;
  }
  async function insertPending() {
    if (inserting) return;
    inserting = true;
    try {
      while (pendingPaths.length && insertPath(pendingPaths[0])) {
        pendingAttachments[attachmentKey] = pendingPaths.slice(1);
        await new Promise(res => setTimeout(res, 150));
      }
    } finally { inserting = false; }
  }
  // phone: a smaller font so the TUI has enough columns (13px on
  // desktop)
  const phone = typeof matchMedia === 'function' && matchMedia('(max-width: 599px)').matches;
  const fontSize = phone ? 11 : 13;
  let uploading = $state(0); let dragOver = $state(false);
  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));
  /** An image or file from the clipboard, drag-and-drop or the 📎 button: put into the workspace
   *  (~/.cloudos/paste/), into the TUI
   * we paste the path — Claude Code turns an image path into an [Image #N] attachment, Codex looks at the file itself
   * (view_image). */
  export async function attach(files: File[]) {
    files = files.filter(f => f.size > 0);
    if (!files.length || !term || !active) return;
    const owner = project; const ownerKey = attachmentKey;
    uploading += files.length;
    try {
      const dir = '.cloudos/paste'; await api.fsMkdir(owner, dir);
      const stamp = new Date().toISOString().replace(/[-:]/g, '').replace('T', '-').slice(0, 15);
      let n = 0;
      for (const f of files) {
        const ext = (f.name.includes('.') ? f.name.split('.').pop()! : f.type.split('/')[1] || 'bin').toLowerCase().replace(/[^a-z0-9]/g, '') || 'bin';
        const generic = !f.name || /^(image|blob|paste|photo)[\w-]*\.\w+$/i.test(f.name);
        const name = generic ? `${stamp}-${++n}.${ext}` : `${stamp}-${f.name.replace(/[^\w.\-]+/g, '_')}`;
        const r = await api.fsWrite(owner, `${dir}/${name}`, f);
        if (!active || !insertPath(r.path)) pendingAttachments[ownerKey] = [...(pendingAttachments[ownerKey] ?? []), r.path];
        await new Promise(res => setTimeout(res, 150));
      }
    } catch (e) { toast(t('cli.pasteFail', { err: msg(e) }), 'bad'); } finally { uploading -= files.length; }
  }
  function onPaste(e: ClipboardEvent) { const files = Array.from(e.clipboardData?.files ?? []); if (files.length) { e.preventDefault(); e.stopPropagation(); attach(files); } }
  function onDragOver(e: DragEvent) { if (e.dataTransfer?.types.includes('Files')) { e.preventDefault(); dragOver = true; } }
  function onDrop(e: DragEvent) { dragOver = false; const files = Array.from(e.dataTransfer?.files ?? []); if (files.length) { e.preventDefault(); e.stopPropagation(); attach(files); } }

  onMount(() => {
    active = true;
    const owner = project; const terminalId = id;
    const tm = new Terminal({ cursorBlink: true, disableStdin: true, fontSize, fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace', scrollback: 5000, allowProposedApi: true });
    const fa = new FitAddon(); tm.loadAddon(fa);
    const updateTheme = () => {
      const css = getComputedStyle(document.documentElement);
      const token = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
      const light = document.documentElement.dataset.theme === 'light';
      const classic = !document.documentElement.dataset.scheme || document.documentElement.dataset.scheme === 'classic';
      const ansi = (name: string, darkColor: string, lightColor: string) => classic ? (light ? lightColor : darkColor) : token(name, light ? lightColor : darkColor);
      tm.options.theme = {
        background: token('--bg-1', '#0f1218'), foreground: token('--fg-1', '#e8ebf1'),
        cursor: token('--accent', '#6d9cff'), cursorAccent: token('--bg-1', '#0f1218'),
        selectionBackground: token('--accent-soft', light ? '#3d6fe040' : '#6d9cff40'),
        black: ansi('--fg-2', '#10141c', '#394150'), brightBlack: ansi('--fg-3', '#6f7890', '#596375'),
        red: ansi('--bad', '#f26d6d', '#bc3434'), brightRed: ansi('--bad', '#ff8d8d', '#bc3434'),
        green: ansi('--ok', '#3ecf8e', '#16733e'), brightGreen: ansi('--ok', '#6ee7b4', '#16733e'),
        yellow: ansi('--warn', '#f2b84b', '#8a5a10'), brightYellow: ansi('--warn', '#ffd27d', '#8a5a10'),
        blue: ansi('--info', '#6d9cff', '#315dc5'), brightBlue: ansi('--info', '#9bbcff', '#315dc5'),
        magenta: ansi('--accent-2', '#c07cff', '#7655c1'), brightMagenta: ansi('--accent-2', '#d5a1ff', '#7655c1'),
        cyan: ansi('--codex', '#7fd1c0', '#187d87'), brightCyan: ansi('--codex', '#a8e7dc', '#187d87'),
        white: ansi('--fg-2', '#c9d0df', '#394150'), brightWhite: ansi('--fg-1', '#e8ebf1', '#151a24'),
      };
      tm.options.fontFamily = token('--mono', 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace');
      tm.options.fontWeight = token('--text-weight', '400') === '500' ? 500 : 400;
      if (tm.element) fa.fit();
    };
    updateTheme();
    tm.open(host); fa.fit(); term = tm;
    const themeObserver = new MutationObserver(updateTheme);
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'data-scheme', 'data-accent', 'data-code-font', 'style', 'class'] });
    const fitFont = () => { if (active && tm.element) fa.fit(); };
    document.fonts?.addEventListener('loadingdone', fitFont);
    // Ctrl+V / Ctrl+Shift+V / Shift+Insert — the browser's native paste (the paste event): images go into the workspace via attach(), text is
    // pasted by xterm itself. The ^V character is not needed in the TUI: for Claude Code and Codex it means "an image from the clipboard",
    // which does not exist inside the workspace
    tm.attachCustomKeyEventHandler((ev) => !(ev.type === 'keydown' && ((ev.ctrlKey && !ev.altKey && !ev.metaKey && ev.key.toLowerCase() === 'v') || (ev.shiftKey && ev.key === 'Insert'))));
    let ws: WebSocket | null = null; let closed = false; let retry = 0;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    const send = (d: string | Uint8Array) => { if (!closed && ws?.readyState === WebSocket.OPEN) ws.send(d); };
    tm.onData((d) => { if (conn === 'live') send(enc.encode(d)); });
    tm.onBinary((d) => { if (conn === 'live') send(Uint8Array.from(d, (c) => c.charCodeAt(0))); });
    tm.onResize(({ cols, rows }) => send(JSON.stringify({ t: 'resize', cols, rows })));
    function connect() {
      if (closed) return;
      setConnection('connecting');
      const next = new WebSocket(api.ptyWsUrl(owner, terminalId)); ws = next; socket = next; next.binaryType = 'arraybuffer';
      next.onopen = () => { if (closed || ws !== next) return; retry = 0; setConnection('live'); fa.fit(); send(JSON.stringify({ t: 'resize', cols: tm.cols, rows: tm.rows })); if (!phone) tm.focus(); };
      next.onmessage = (ev) => {
        if (closed || ws !== next) return;
        if (typeof ev.data === 'string') {
          try { const m = JSON.parse(ev.data); if (m.t === 'exit') { setConnection('exited'); exitCode = m.code; closed = true; onexit?.(m.code); } } catch {}
        } else tm.write(new Uint8Array(ev.data));
      };
      next.onclose = () => {
        if (closed || ws !== next) return;
        // A failed network connection does not establish that the remote process exited.
        setConnection('lost'); retryTimer = setTimeout(connect, Math.min(1000 * 2 ** retry++, 15000));
      };
      next.onerror = () => next.close();
    }
    connect();
    const ro = new ResizeObserver(() => { try { fa.fit(); } catch {} }); ro.observe(host);
    const ping = setInterval(() => send(JSON.stringify({ t: 'ping' })), 25000);
    return () => { active = false; closed = true; clearInterval(ping); clearTimeout(retryTimer); ro.disconnect(); themeObserver.disconnect(); document.fonts?.removeEventListener('loadingdone', fitFont); ws?.close(); socket = null; tm.dispose(); term = null; };
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="term" class:dim={conn !== 'live'} class:drag={dragOver} onpastecapture={onPaste} ondragovercapture={onDragOver} ondragleave={() => (dragOver = false)} ondropcapture={onDrop}>
  <div class="host" bind:this={host}></div>
  {#if uploading}<div class="state up" role="status">📎 {t('cli.uploading')}</div>{:else if conn !== 'live'}<div class="state" role="status">{conn === 'exited' ? t('agents.exited', { code: exitCode ?? 0 }) : conn === 'lost' ? t('agents.reconnecting') : t('agents.connecting')}{#if conn !== 'exited'} · {t('wsux.offlineInput')}{/if}</div>{/if}
  {#if pendingPaths.length}<div class="pending" role="status"><span>{t('wsux.pendingPaths')}</span><code>{pendingPaths.join(' · ')}</code><button disabled={conn !== 'live' || inserting} onclick={insertPending}>{t('wsux.insertPaths')} ({pendingPaths.length})</button></div>{/if}
  {#if dragOver}<div class="dropzone">{t('cli.dropHint')}</div>{/if}
</div>

<style>
  .term { position: relative; width: 100%; height: 100%; min-height: 200px; background: var(--bg-1); border-radius: var(--r-sm); overflow: hidden; }
  .host { position: absolute; inset: 6px; }
  .state { position: absolute; left: 10px; right: 10px; bottom: 8px; width: fit-content; max-width: calc(100% - 20px); font-size: .75rem; color: var(--fg-2); background: var(--bg-3); padding: 4px 8px; border-radius: 8px; }
  .pending { position: absolute; left: 10px; right: 10px; top: 10px; display: flex; flex-wrap: wrap; gap: 6px; padding: 8px; background: var(--bg-3); border: 1px solid var(--line); border-radius: 8px; color: var(--fg-2); font-size: 12px; }
  .pending code { width: 100%; overflow-wrap: anywhere; }
  .dim :global(.xterm) { opacity: .75; }
  .state.up { color: var(--fg-1); }
  /* xterm's hidden input field: 16px, otherwise iOS zooms the page on focus; finger scrolling of the buffer is native to .xterm-viewport */
  .term :global(.xterm-helper-textarea) { font-size: 16px !important; }
  .term :global(.xterm-viewport) { touch-action: pan-y; }
  .dropzone { position: absolute; inset: 6px; display: grid; place-items: center; border: 2px dashed var(--accent); border-radius: var(--r-sm); background: rgb(0 0 0 / .5); color: var(--fg-1); font-size: .85rem; pointer-events: none; }
  @media (pointer: coarse) { .pending button { min-height: 44px; } }
</style>
