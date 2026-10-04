<script lang="ts">
  // Терминал карточки сессии (ADR-006): xterm.js поверх PTY, который живёт в cloudos-agentd внутри workspace.
  // Протокол v1: бинарные кадры — байты, текстовые — JSON (`hello`, `resize`, `ping`/`pong`, `exit`).
  // Закрытие окна или вкладки процесс не трогает: при следующем подключении agentd проигрывает хвост вывода.
  import { onMount } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';
  import { api } from './api';
  import { t } from './i18n';
  import { toast } from './toast.svelte';
  let { project, id, onexit }: { project: string; id: string; onexit?: (code: number) => void } = $props();
  let host: HTMLDivElement;
  let conn = $state<'connecting' | 'live' | 'exited' | 'lost'>('connecting');
  let exitCode = $state<number | null>(null);
  let term: Terminal | null = null;
  // телефон: шрифт мельче, чтобы TUI хватило колонок (13px на столе)
  const phone = typeof matchMedia === 'function' && matchMedia('(max-width: 599px)').matches;
  const fontSize = phone ? 11 : 13;
  let uploading = $state(0); let dragOver = $state(false);
  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));
  /** Картинка или файл из буфера обмена, перетаскивания или кнопки 📎: кладём в workspace (~/.cloudos/paste/), в TUI
   *  вставляем путь — Claude Code превращает путь к картинке во вложение [Image #N], Codex смотрит файл сам (view_image). */
  export async function attach(files: File[]) {
    files = files.filter(f => f.size > 0);
    if (!files.length || !term) return;
    uploading += files.length;
    try {
      const dir = '.cloudos/paste'; await api.fsMkdir(project, dir);
      const stamp = new Date().toISOString().replace(/[-:]/g, '').replace('T', '-').slice(0, 15);
      let n = 0;
      for (const f of files) {
        const ext = (f.name.includes('.') ? f.name.split('.').pop()! : f.type.split('/')[1] || 'bin').toLowerCase().replace(/[^a-z0-9]/g, '') || 'bin';
        const generic = !f.name || /^(image|blob|paste|photo)[\w-]*\.\w+$/i.test(f.name);
        const name = generic ? `${stamp}-${++n}.${ext}` : `${stamp}-${f.name.replace(/[^\w.\-]+/g, '_')}`;
        const r = await api.fsWrite(project, `${dir}/${name}`, f);
        term.paste(r.path); term.focus();
        await new Promise(res => setTimeout(res, 150));
      }
    } catch (e) { toast(t('cli.pasteFail', { err: msg(e) }), 'bad'); } finally { uploading -= files.length; }
  }
  function onPaste(e: ClipboardEvent) { const files = Array.from(e.clipboardData?.files ?? []); if (files.length) { e.preventDefault(); e.stopPropagation(); attach(files); } }
  function onDragOver(e: DragEvent) { if (e.dataTransfer?.types.includes('Files')) { e.preventDefault(); dragOver = true; } }
  function onDrop(e: DragEvent) { dragOver = false; const files = Array.from(e.dataTransfer?.files ?? []); if (files.length) { e.preventDefault(); e.stopPropagation(); attach(files); } }

  onMount(() => {
    const css = getComputedStyle(document.documentElement);
    const tm = new Terminal({ cursorBlink: true, fontSize, fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace', scrollback: 5000, allowProposedApi: true,
      theme: { background: '#0b0d12', foreground: css.getPropertyValue('--ink').trim() || '#e6e6e6', cursor: css.getPropertyValue('--accent').trim() || '#F2690D' } });
    const fa = new FitAddon(); tm.loadAddon(fa); tm.open(host); fa.fit(); term = tm;
    // Ctrl+V / Ctrl+Shift+V / Shift+Insert — родная вставка браузера (событие paste): картинки уходят в workspace через attach(),
    // текст вставляет сам xterm. Символ ^V в TUI не нужен: у Claude Code и Codex это «картинка из буфера обмена», которого внутри workspace нет
    tm.attachCustomKeyEventHandler((ev) => !(ev.type === 'keydown' && ((ev.ctrlKey && !ev.altKey && !ev.metaKey && ev.key.toLowerCase() === 'v') || (ev.shiftKey && ev.key === 'Insert'))));
    const enc = new TextEncoder();
    let ws: WebSocket | null = null; let closed = false; let retry = 0; let opened = false; let fails = 0;
    const send = (d: string | Uint8Array) => { if (ws?.readyState === WebSocket.OPEN) ws.send(d); };
    tm.onData((d) => send(enc.encode(d)));
    tm.onBinary((d) => send(Uint8Array.from(d, (c) => c.charCodeAt(0))));
    tm.onResize(({ cols, rows }) => send(JSON.stringify({ t: 'resize', cols, rows })));
    function connect() {
      if (closed) return;
      conn = 'connecting';
      ws = new WebSocket(api.ptyWsUrl(project, id)); ws.binaryType = 'arraybuffer';
      ws.onopen = () => { retry = 0; fails = 0; opened = true; conn = 'live'; fa.fit(); send(JSON.stringify({ t: 'resize', cols: tm.cols, rows: tm.rows })); if (!phone) tm.focus(); };
      ws.onmessage = (ev) => {
        if (typeof ev.data === 'string') {
          try { const m = JSON.parse(ev.data); if (m.t === 'exit') { conn = 'exited'; exitCode = m.code; closed = true; onexit?.(m.code); } } catch {}
        } else tm.write(new Uint8Array(ev.data));
      };
      ws.onclose = () => {
        if (closed) return;
        // PTY, которого уже нет (убран agentd): три неудачных подключения подряд без единого открытия — считаем вышедшим
        if (!opened && ++fails >= 3) { conn = 'exited'; exitCode = -1; closed = true; onexit?.(-1); return; }
        conn = 'lost'; setTimeout(connect, Math.min(1000 * 2 ** retry++, 15000));
      };
      ws.onerror = () => ws?.close();
    }
    connect();
    const ro = new ResizeObserver(() => { try { fa.fit(); } catch {} }); ro.observe(host);
    const ping = setInterval(() => send(JSON.stringify({ t: 'ping' })), 25000);
    return () => { closed = true; clearInterval(ping); ro.disconnect(); ws?.close(); tm.dispose(); term = null; };
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="term" class:dim={conn !== 'live'} class:drag={dragOver} onpastecapture={onPaste} ondragovercapture={onDragOver} ondragleave={() => (dragOver = false)} ondropcapture={onDrop}>
  <div class="host" bind:this={host}></div>
  {#if uploading}<div class="state up">📎 {t('cli.uploading')}</div>{:else if conn !== 'live'}<div class="state">{conn === 'exited' ? t('agents.exited', { code: exitCode ?? 0 }) : conn === 'lost' ? t('agents.reconnecting') : t('agents.connecting')}</div>{/if}
  {#if dragOver}<div class="dropzone">{t('cli.dropHint')}</div>{/if}
</div>

<style>
  .term { position: relative; width: 100%; height: 100%; min-height: 200px; background: #0b0d12; border-radius: var(--r-sm); overflow: hidden; }
  .host { position: absolute; inset: 6px; }
  .state { position: absolute; left: 10px; bottom: 8px; font-size: .75rem; color: var(--muted); background: rgb(0 0 0 / .55); padding: 2px 8px; border-radius: 999px; }
  .dim :global(.xterm) { opacity: .75; }
  .state.up { color: var(--fg-1); }
  /* скрытое поле ввода xterm: 16px, иначе iOS увеличивает страницу при фокусе; прокрутка буфера пальцем — родная у .xterm-viewport */
  .term :global(.xterm-helper-textarea) { font-size: 16px !important; }
  .term :global(.xterm-viewport) { touch-action: pan-y; }
  .dropzone { position: absolute; inset: 6px; display: grid; place-items: center; border: 2px dashed var(--accent); border-radius: var(--r-sm); background: rgb(0 0 0 / .5); color: var(--fg-1); font-size: .85rem; pointer-events: none; }
</style>
