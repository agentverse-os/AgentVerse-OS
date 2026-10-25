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
  import { t } from './i18n.svelte';
  let { project, id, onexit }: { project: string; id: string; onexit?: (code: number) => void } = $props();
  let host: HTMLDivElement;
  let conn = $state<'connecting' | 'live' | 'exited' | 'lost'>('connecting');
  let exitCode = $state<number | null>(null);

  onMount(() => {
    const css = getComputedStyle(document.documentElement);
    const term = new Terminal({ cursorBlink: true, fontSize: 13, fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace', scrollback: 5000, allowProposedApi: true,
      theme: { background: '#0b0d12', foreground: css.getPropertyValue('--ink').trim() || '#e6e6e6', cursor: css.getPropertyValue('--accent').trim() || '#F2690D' } });
    const fit = new FitAddon(); term.loadAddon(fit); term.open(host); fit.fit();
    const enc = new TextEncoder();
    let ws: WebSocket | null = null; let closed = false; let retry = 0;
    const send = (d: string | Uint8Array) => { if (ws?.readyState === WebSocket.OPEN) ws.send(d); };
    term.onData((d) => send(enc.encode(d)));
    term.onBinary((d) => send(Uint8Array.from(d, (c) => c.charCodeAt(0))));
    term.onResize(({ cols, rows }) => send(JSON.stringify({ t: 'resize', cols, rows })));
    function connect() {
      if (closed) return;
      conn = 'connecting';
      ws = new WebSocket(api.ptyWsUrl(project, id)); ws.binaryType = 'arraybuffer';
      ws.onopen = () => { retry = 0; conn = 'live'; fit.fit(); send(JSON.stringify({ t: 'resize', cols: term.cols, rows: term.rows })); term.focus(); };
      ws.onmessage = (ev) => {
        if (typeof ev.data === 'string') {
          try { const m = JSON.parse(ev.data); if (m.t === 'exit') { conn = 'exited'; exitCode = m.code; closed = true; onexit?.(m.code); } } catch {}
        } else term.write(new Uint8Array(ev.data));
      };
      ws.onclose = () => { if (closed) return; conn = 'lost'; setTimeout(connect, Math.min(1000 * 2 ** retry++, 15000)); };
      ws.onerror = () => ws?.close();
    }
    connect();
    const ro = new ResizeObserver(() => { try { fit.fit(); } catch {} }); ro.observe(host);
    const ping = setInterval(() => send(JSON.stringify({ t: 'ping' })), 25000);
    return () => { closed = true; clearInterval(ping); ro.disconnect(); ws?.close(); term.dispose(); };
  });
</script>

<div class="term" class:dim={conn !== 'live'}>
  <div class="host" bind:this={host}></div>
  {#if conn !== 'live'}<div class="state">{conn === 'exited' ? t('agents.exited', { code: exitCode ?? 0 }) : conn === 'lost' ? t('agents.reconnecting') : t('agents.connecting')}</div>{/if}
</div>

<style>
  .term { position: relative; width: 100%; height: 100%; min-height: 200px; background: #0b0d12; border-radius: var(--r-sm); overflow: hidden; }
  .host { position: absolute; inset: 6px; }
  .state { position: absolute; left: 10px; bottom: 8px; font-size: .75rem; color: var(--muted); background: rgb(0 0 0 / .55); padding: 2px 8px; border-radius: 999px; }
  .dim :global(.xterm) { opacity: .75; }
</style>
