// Voice (docs/agents-ux.md §4.7): dictation settings from the server (without keys), push-to-talk recording via
// MediaRecorder, recognition on the service and text insertion at the cursor position. The button — Mic.svelte, the
// settings tab — Settings.svelte.
import { agents, type VoiceSettings, type VoicePatch, type VoiceResult } from './api';
import { msg } from './state.svelte';

class Voice {
  settings = $state<VoiceSettings | null>(null);
  loading = $state(false);
  error = $state('');
  private loaded = false;
  /** There is a key for the chosen provider — the microphone button records, otherwise it
   *  leads to the settings */
  get ready() { return !!this.settings?.ready; }
  async load(force = false) {
    if (this.loading || (this.loaded && !force)) return;
    this.loading = true;
    try { this.settings = await agents.voice(); this.error = ''; this.loaded = true; } catch (e) { this.error = msg(e); } finally { this.loading = false; }
  }
  async save(patch: VoicePatch) { this.settings = await agents.voicePut(patch); this.loaded = true; return this.settings; }
  /** Read-aloud (slice 64): which text is playing now (message key) and whether loading is in progress; one player
   *  per app */
  speaking = $state<string | null>(null);
  speakLoading = $state<string | null>(null);
  private audio: HTMLAudioElement | null = null;
  private url = '';
  private resolvePlay: (() => void) | null = null;
  stop() {
    if (this.audio) { try { this.audio.pause(); } catch {} this.audio = null; }
    if (this.url) { URL.revokeObjectURL(this.url); this.url = ''; }
    this.speaking = null; this.speakLoading = null;
    const r = this.resolvePlay; this.resolvePlay = null; r?.();
    this.mediaSession(false);
  }
  /** Media Session (§9.9): while read-aloud is playing it can be stopped from the lock screen and the
   *  headphones */
  private mediaSession(on: boolean) {
    try {
      const ms = navigator.mediaSession; if (!ms) return;
      if (on) {
        ms.metadata = new MediaMetadata({ title: 'Читаю ответ', artist: 'AgentVerse Agents', artwork: [{ src: '/agents/icon-192.png', sizes: '192x192', type: 'image/png' }, { src: '/agents/icon-512.png', sizes: '512x512', type: 'image/png' }] });
        ms.playbackState = 'playing';
        for (const a of ['pause', 'stop'] as const) { try { ms.setActionHandler(a, () => this.clearQueue()); } catch {} }
        try { ms.setActionHandler('play', null); } catch {}
      } else {
        ms.playbackState = 'none'; ms.metadata = null;
        for (const a of ['pause', 'stop'] as const) { try { ms.setActionHandler(a, null); } catch {} }
      }
    } catch {}
  }
  /** Play a blob to the end (or until stop()) */
  private playBlob(key: string, blob: Blob): Promise<void> {
    return new Promise((resolve) => {
      this.stop();
      this.url = URL.createObjectURL(blob); const a = new Audio(this.url); this.audio = a; this.speaking = key;
      const done = () => { if (this.audio === a) this.stop(); else resolve(); };
      this.resolvePlay = resolve;
      a.onended = done; a.onerror = done;
      a.play().then(() => this.mediaSession(true)).catch(() => done());
    });
  }
  /** A repeated call with the same key — stop; with another — replace (manual read-aloud interrupts the auto
   *  queue) */
  async speak(key: string, text: string): Promise<void> {
    if (this.speaking === key || this.speakLoading === key) { this.stop(); return; }
    this.clearQueue(); this.speakLoading = key;
    try {
      const blob = await agents.voiceSpeak(text);
      if (this.speakLoading !== key) return;
      this.speakLoading = null;
      void this.playBlob(key, blob);   // the promise resolves when playback starts; the end — via `speaking`
    } catch (e) { this.stop(); throw e; }
  }
  /** Read-aloud as the answer goes (§4.7, slice 68): sentences are queued as they appear, the next one loads while the
   * current one plays; `auto` — the toggle in the composer's "Options" menu, stored in
   * the browser */
  auto = $state<boolean>((() => { try { return localStorage.getItem('agents.autoSpeak') === '1'; } catch { return false; } })());
  setAuto(v: boolean) { this.auto = v; try { localStorage.setItem('agents.autoSpeak', v ? '1' : '0'); } catch {} if (!v) this.clearQueue(); }
  reading = $state(false);
  lastError = $state('');
  private queue: { key: string; text: string; blob?: Promise<Blob> }[] = [];
  private seen = new Set<string>();
  private pumping = false;
  enqueue(key: string, text: string) {
    if (this.seen.has(key)) return; this.seen.add(key); if (this.seen.size > 1000) this.seen.clear();
    this.queue.push({ key, text }); void this.pump();
  }
  clearQueue() { this.queue = []; this.reading = false; this.stop(); }
  private async pump() {
    if (this.pumping) return; this.pumping = true; this.reading = true;
    try {
      while (this.queue.length) {
        const item = this.queue[0];
        item.blob ??= agents.voiceSpeak(item.text);
        const next = this.queue[1]; if (next && !next.blob) { next.blob = agents.voiceSpeak(next.text); next.blob.catch(() => {}); }
        let blob: Blob;
        try { blob = await item.blob; } catch (e) { this.lastError = msg(e); if (this.queue[0] === item) this.queue.shift(); if (!this.queue.length) break; continue; }
        if (this.queue[0] !== item) continue;
        await this.playBlob(item.key, blob);
        if (this.queue[0] === item) this.queue.shift();
      }
    } finally { this.pumping = false; this.reading = this.queue.length > 0; if (this.queue.length) void this.pump(); }
  }
}

/** Answer text without markdown for "Share" and the like (§9.9): code blocks stay as text, links — "text (address)" */
export function plainText(md: string): string {
  return md
    .replace(/```[\w-]*\n?([\s\S]*?)```/g, (_m, code: string) => code.trimEnd())
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, '$1 ($2)')
    .replace(/^#{1,6}\s+/gm, '').replace(/^\s*>\s?/gm, '').replace(/^\s*[-*+]\s+/gm, '• ').replace(/^\s*\|.*\|\s*$/gm, (row) => row.trim().replace(/^\||\|$/g, '').split('|').map(c => c.trim()).join(' · '))
    .replace(/^\s*[-:| ]+\s*$/gm, '')
    .replace(/(\*\*|__)(.*?)\1/g, '$2').replace(/(\*|_)(?=\S)(.*?)(?<=\S)\1/g, '$2').replace(/`([^`]*)`/g, '$1')
    .replace(/\n{3,}/g, '\n\n').trim();
}

/** Finished sentences of the answer text from position `from`: without code blocks, the end is . ! ? … or a line break;
 *  returns the sentences and
 * the position up to which the text was parsed (a tail without an end mark stays for the next call or the final
 * flush) */
export function sentencesFrom(text: string, from: number): { chunks: string[]; end: number } {
  const chunks: string[] = []; let i = from; let start = from; let fence = (text.slice(0, from).match(/```/g) || []).length % 2 === 1;
  const push = (a: number, b: number) => { const t = text.slice(a, b).replace(/```[\s\S]*?```/g, ' (код) ').trim(); if (!fence && /[\p{L}\p{N}]{3,}/u.test(t)) chunks.push(t); };
  while (i < text.length) {
    if (text.startsWith('```', i)) { if (!fence) push(start, i); fence = !fence; i += 3; start = i; continue; }
    const c = text[i];
    if (c === '\n' || ((c === '.' || c === '!' || c === '?' || c === '…') && (i + 1 >= text.length ? false : /\s/.test(text[i + 1])))) {
      const endAt = c === '\n' ? i : i + 1;
      if (!fence) push(start, endAt);
      i = endAt + (c === '\n' ? 1 : 0); start = i; continue;
    }
    i++;
  }
  return { chunks, end: start };
}
export const voice = new Voice();

/** Recording limit — two minutes (auto-stop); shorter than 400 ms is an accidental tap,
 *  not recognized */
export const MAX_MS = 120_000;
export const MIN_MS = 400;

export function pickMime(): string {
  if (typeof MediaRecorder === 'undefined') return '';
  for (const m of ['audio/webm;codecs=opus', 'audio/webm', 'audio/mp4', 'audio/ogg;codecs=opus']) { try { if (MediaRecorder.isTypeSupported(m)) return m; } catch {} }
  return '';
}
export const canRecord = () => typeof navigator !== 'undefined' && !!navigator.mediaDevices?.getUserMedia && typeof MediaRecorder !== 'undefined';

/** Why the microphone was refused: the site permission state (Permissions API) and the browser error text → a hint
 *  key */
export async function micDenialHint(e: unknown): Promise<'blocked' | 'system' | 'device' | 'busy' | 'prompt' | 'other'> {
  const name = (e as { name?: string })?.name ?? ''; const m = e instanceof Error ? e.message : String(e);
  if (name === 'NotFoundError' || /NotFound|not found|no device/i.test(m)) return 'device';
  if (name === 'NotReadableError' || name === 'AbortError' || /NotReadable|in use|busy|could not start/i.test(m)) return 'busy';
  if (name === 'NotAllowedError' || /NotAllowed|Permission|denied/i.test(m)) {
    if (/system/i.test(m)) return 'system';
    try { const st = await navigator.permissions?.query({ name: 'microphone' as PermissionName }); if (st?.state === 'denied') return 'blocked'; } catch {}
    return 'prompt';
  }
  return 'other';
}

/** One recording: the microphone is taken for the recording and released right after (the browser indicator
 *  goes off). */
export class Recorder {
  private stream: MediaStream | null = null;
  private owned = true;
  private rec: MediaRecorder | null = null;
  private chunks: Blob[] = [];
  private cancelled = false;
  /** The microphone is already obtained (the permission
   *  request passed) */
  micReady = false;
  mime = '';
  async start(): Promise<void> {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
    // the button was released while the browser was asking for permission: access is granted and remembered, and the tracks are released right away
    // — otherwise the microphone stays "on" until a reload
    if (this.cancelled) { stream.getTracks().forEach((t) => t.stop()); return; }
    this.stream = stream; this.micReady = true; this.owned = true; this.begin();
  }
  /** Record a foreign stream (streaming dictation): the tracks are released by the
   *  stream owner */
  attach(stream: MediaStream) { this.stream = stream; this.owned = false; this.begin(); }
  private begin() {
    if (!this.stream) return;
    this.mime = pickMime();
    this.rec = new MediaRecorder(this.stream, this.mime ? { mimeType: this.mime } : undefined);
    this.chunks = [];
    this.rec.ondataavailable = (e) => { if (e.data.size) this.chunks.push(e.data); };
    this.rec.start(250);
  }
  /** Stop and collect the recording; an empty Blob if the recording never
   *  started */
  stop(): Promise<Blob> {
    return new Promise((res) => {
      const r = this.rec;
      if (!r) { this.release(); res(new Blob()); return; }
      const done = () => { const type = this.mime || r.mimeType || 'audio/webm'; const b = new Blob(this.chunks, { type: type.split(';')[0] }); this.release(); res(b); };
      if (r.state === 'inactive') { done(); return; }
      r.onstop = done;
      try { r.stop(); } catch { done(); }
    });
  }
  cancel() { this.cancelled = true; try { if (this.rec && this.rec.state !== 'inactive') { this.rec.onstop = null; this.rec.stop(); } } catch {} this.release(); }
  private release() { if (this.owned) this.stream?.getTracks().forEach((t) => t.stop()); this.stream = null; this.rec = null; this.chunks = []; }
}

export const transcribe = (audio: Blob, lang = ''): Promise<VoiceResult> => agents.voiceTranscribe(audio, lang);

/** Streaming recognition (§4.7, slice 72): an AudioWorklet cuts the microphone into PCM16 chunks of the needed rate and
 *  sends them over WebSocket
 * to the service, which holds the connection to the provider; interim text arrives as you speak. A MediaRecorder always
 * records in parallel — if the stream did not come up or broke without a result, the same recording goes as a batch to
 * /transcribe. */
export const canStream = () => canRecord() && typeof AudioWorkletNode !== 'undefined' && typeof AudioContext !== 'undefined' && typeof WebSocket !== 'undefined';
const WORKLET_SRC = `class PcmCapture extends AudioWorkletProcessor {
  constructor(o) { super(); const p = o.processorOptions || {}; this.target = p.targetRate || 16000; this.chunk = Math.round(this.target * (p.chunkMs || 100) / 1000); this.ratio = sampleRate / this.target; this.pos = 0; this.last = 0; this.out = new Int16Array(this.chunk); this.n = 0; this.stopped = false; this.port.onmessage = (e) => { if (e.data === 'stop') { this.flush(); this.stopped = true; } }; }
  flush() { if (!this.n) return; const b = this.out.slice(0, this.n).buffer; this.port.postMessage(b, [b]); this.out = new Int16Array(this.chunk); this.n = 0; }
  process(inputs) { if (this.stopped) return false; const ch = inputs[0] && inputs[0][0]; if (!ch || !ch.length) return true;
    // linear resampling from the context rate to the provider rate; pos — the fractional position in the current block, last — the last sample of
    // the previous one
    let pos = this.pos; const len = ch.length;
    while (pos < len) { const i = Math.floor(pos); const f = pos - i; const a = i < 0 ? this.last : ch[i]; const b = i + 1 < len ? ch[i + 1] : ch[len - 1]; let v = a + (b - a) * f; v = v < -1 ? -1 : v > 1 ? 1 : v; this.out[this.n++] = v < 0 ? v * 32768 : v * 32767; if (this.n >= this.chunk) this.flush(); pos += this.ratio; }
    this.pos = pos - len; this.last = ch[len - 1]; return true; }
}
registerProcessor('pcm-capture', PcmCapture);`;
let workletUrl = '';
function workletModule(): string { if (!workletUrl) workletUrl = URL.createObjectURL(new Blob([WORKLET_SRC], { type: 'application/javascript' })); return workletUrl; }

/** Cutting PCM16 of the needed rate from the context's Float32 blocks (linear resampling) — for the fallback path
 *  via
 * ScriptProcessorNode when the AudioWorklet did not come up (old WebViews, headless Chromium in
 * e2e) */
export class PcmChunker {
  private pos = 0; private out: Int16Array; private n = 0; private readonly ratio: number; private readonly chunk: number;
  constructor(srcRate: number, target: number, chunkMs = 100) { this.ratio = srcRate / target; this.chunk = Math.max(1, Math.round(target * chunkMs / 1000)); this.out = new Int16Array(this.chunk); }
  feed(ch: Float32Array): ArrayBuffer[] {
    const res: ArrayBuffer[] = []; let pos = this.pos; const len = ch.length; if (!len) return res;
    while (pos < len) {
      const i = Math.floor(pos); const f = pos - i; const a = ch[i]; const b = i + 1 < len ? ch[i + 1] : a; let v = a + (b - a) * f; v = v < -1 ? -1 : v > 1 ? 1 : v;
      this.out[this.n++] = v < 0 ? v * 32768 : v * 32767;
      if (this.n >= this.chunk) { res.push(this.out.buffer as ArrayBuffer); this.out = new Int16Array(this.chunk); this.n = 0; }
      pos += this.ratio;
    }
    this.pos = pos - len; return res;
  }
  flush(): ArrayBuffer | null { if (!this.n) return null; const b = this.out.slice(0, this.n).buffer as ArrayBuffer; this.n = 0; return b; }
}

export type StreamResult = { text: string | null; blob: Blob; streamed: boolean; failed: string };
export class StreamRecorder {
  private stream: MediaStream | null = null;
  private ctx: AudioContext | null = null;
  private node: AudioWorkletNode | null = null;
  private sp: ScriptProcessorNode | null = null;
  private chunker: PcmChunker | null = null;
  private src: MediaStreamAudioSourceNode | null = null;
  private ws: WebSocket | null = null;
  private fallback: Recorder | null = null;
  private finalText: string | null = null;
  private finalWait: (() => void) | null = null;
  private closed = false;     // the socket to the service is closed
  private stopping = false;   // stop()/cancel() were called: do not bring the capture up again
  private ready = false;
  /** Chunks recorded before the service's `ready` answer (connecting to the provider takes up to a second or so): they go
   *  first */
  private pending: ArrayBuffer[] = [];
  /** The last interim text; `failed` — why the stream does not work (the field is empty while all is well) */
  partial = '';
  failed = '';
  rate = 0;
  constructor(private lang: string, private onPartial: (text: string) => void, provider = 'elevenlabs') { this.rate = provider === 'openai' ? 24_000 : 16_000; }
  /** The microphone is already obtained (the permission request passed); `ready` below is the service's
   *  answer that the stream is ready */
  micReady = false;
  async start(): Promise<void> {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
    if (this.stopping) { stream.getTracks().forEach((t) => t.stop()); return; }   // released during the permission request — access granted, the tracks go
    this.stream = stream; this.micReady = true;
    this.fallback = new Recorder(); this.fallback.attach(this.stream);
    // the capture starts right away at the provider's expected rate while the socket comes up in parallel: the first
    // words are not lost
    const assumed = this.rate; const cap = this.capture().catch((e) => { this.failed = this.failed || msg(e); });
    try { await this.open(); }
    catch (e) { this.failed = msg(e); this.pending = []; this.teardownStream(); return; }
    if (this.rate !== assumed) { this.teardownStream(); await cap; this.pending = []; void this.capture().catch((e) => { this.failed = this.failed || msg(e); }); this.ready = true; }
  }
  private open(): Promise<void> {
    return new Promise((resolve, reject) => {
      let settled = false;
      const ws = new WebSocket(agents.voiceStreamUrl(this.lang)); ws.binaryType = 'arraybuffer'; this.ws = ws;
      const timer = setTimeout(() => { if (!settled) { settled = true; reject(new Error('поток: сервис не ответил')); try { ws.close(); } catch {} } }, 12000);
      ws.onmessage = (e) => {
        let v: { kind?: string; text?: string; rate?: number } = {};
        try { v = JSON.parse(String(e.data)); } catch { return; }
        if (v.kind === 'ready') {
          const serverRate = v.rate ?? 16000;
          if (serverRate === this.rate) { for (const b of this.pending) { try { ws.send(b); } catch {} } this.pending = []; this.ready = true; }
          else { this.pending = []; this.rate = serverRate; }   // not the rate we expected: start() restarts the capture and opens sending
          if (!settled) { settled = true; clearTimeout(timer); resolve(); }
        }
        else if (v.kind === 'partial') { this.partial = v.text ?? ''; this.onPartial(this.partial); }
        else if (v.kind === 'final') { this.finalText = v.text ?? ''; this.finalWait?.(); }
        else if (v.kind === 'error') { this.failed = v.text ?? 'поток оборвался'; if (!settled) { settled = true; clearTimeout(timer); reject(new Error(this.failed)); } this.finalWait?.(); }
      };
      ws.onerror = () => { if (!settled) { settled = true; clearTimeout(timer); reject(new Error('поток: соединение не открылось')); } };
      ws.onclose = () => { this.closed = true; if (!settled) { settled = true; clearTimeout(timer); reject(new Error('поток: соединение закрыто')); } if (this.finalText === null && !this.failed) this.failed = 'поток оборвался'; this.finalWait?.(); };
    });
  }
  private push(buf: ArrayBuffer) {
    if (this.failed || this.closed) return;
    if (!this.ready) { if (this.pending.length < 1200) this.pending.push(buf); return; }   // up to 2 minutes in 100 ms steps
    const ws = this.ws; if (ws && ws.readyState === WebSocket.OPEN) ws.send(buf);
  }
  /** Capture: an AudioWorklet, and if the module did not come up within 1.5 s (no support, headless) — a ScriptProcessorNode with
   *  the same cutting */
  private async capture(): Promise<void> {
    if (!this.stream || this.closed || this.stopping) return;
    const ctx = new AudioContext(); this.ctx = ctx; const rate = this.rate || 16000;
    this.src = ctx.createMediaStreamSource(this.stream);
    let worklet = false;
    if (ctx.audioWorklet && typeof AudioWorkletNode !== 'undefined') {
      try { await Promise.race([ctx.audioWorklet.addModule(workletModule()), new Promise<void>((_, rej) => setTimeout(() => rej(new Error('worklet timeout')), 1500))]); worklet = true; } catch {}
    }
    if (this.closed || this.stopping || !this.ctx) { this.teardownStream(); return; }   // stopped while waiting for the module
    if (worklet) {
      this.node = new AudioWorkletNode(ctx, 'pcm-capture', { numberOfInputs: 1, numberOfOutputs: 0, processorOptions: { targetRate: rate, chunkMs: 100 } });
      this.node.port.onmessage = (e) => this.push(e.data as ArrayBuffer);
      this.src.connect(this.node);
    } else {
      const sp = ctx.createScriptProcessor(4096, 1, 1); this.sp = sp; const ch = new PcmChunker(ctx.sampleRate, rate, 100); this.chunker = ch;
      sp.onaudioprocess = (e) => { for (const b of ch.feed(e.inputBuffer.getChannelData(0))) this.push(b); };
      const mute = ctx.createGain(); mute.gain.value = 0;   // the processor must lead to the output, but the microphone does not go to the speakers
      this.src.connect(sp); sp.connect(mute); mute.connect(ctx.destination);
    }
    if (ctx.state === 'suspended') { try { await ctx.resume(); } catch {} }
  }
  private teardownStream() {
    try { this.node?.port.postMessage('stop'); } catch {}
    if (this.sp) { try { this.sp.onaudioprocess = null; } catch {} const tail = this.chunker?.flush(); if (tail) this.push(tail); }
    try { this.src?.disconnect(); } catch {}
    try { this.node?.disconnect(); } catch {}
    try { this.sp?.disconnect(); } catch {}
    const c = this.ctx; this.ctx = null; this.node = null; this.sp = null; this.chunker = null; this.src = null;
    if (c) { c.close().catch(() => {}); }
  }
  /** Stop: close the capture, ask for the result (up to 20 s), collect the fallback recording; the microphone is
   *  released here */
  async stop(): Promise<StreamResult> {
    this.stopping = true; this.teardownStream();
    const ws = this.ws;
    if (ws && ws.readyState === WebSocket.OPEN && !this.failed) {
      await new Promise<void>((res) => setTimeout(res, 120));   // the tail of chunks from the worklet
      try { ws.send(JSON.stringify({ kind: 'stop' })); } catch {}
      if (this.finalText === null && !this.failed) {
        await new Promise<void>((res) => { this.finalWait = res; setTimeout(res, 20000); });
        this.finalWait = null;
      }
    }
    try { ws?.close(); } catch {}
    this.ws = null;
    const blob = this.fallback ? await this.fallback.stop() : new Blob(); this.fallback = null;
    this.stream?.getTracks().forEach((t) => t.stop()); this.stream = null;
    if (this.finalText !== null) return { text: this.finalText, blob, streamed: true, failed: '' };
    return { text: this.partial || null, blob, streamed: false, failed: this.failed || 'поток не прислал итог' };
  }
  cancel() {
    this.stopping = true; this.closed = true; this.teardownStream();
    try { this.ws?.close(); } catch {} this.ws = null;
    this.fallback?.cancel(); this.fallback = null;
    this.stream?.getTracks().forEach((t) => t.stop()); this.stream = null;
  }
}

/** Insertion at the cursor position with spaces around; returns the new value and the cursor position after the
 *  insertion */
export function insertAt(cur: string, sel: [number, number], piece: string): { value: string; caret: number } {
  const p = piece.trim();
  if (!p) return { value: cur, caret: sel[1] };
  const s = Math.max(0, Math.min(sel[0], cur.length)), e = Math.max(s, Math.min(sel[1], cur.length));
  const before = cur.slice(0, s), after = cur.slice(e);
  const sep1 = before && !/\s$/.test(before) ? ' ' : '';
  const sep2 = after && !/^\s/.test(after) ? ' ' : '';
  const head = before + sep1 + p;
  return { value: head + sep2 + after, caret: head.length };
}

/** Interim text of streaming dictation in the field (§4.7, slice 72): the region [start, end) is the inserted piece together
 *  with the spaces
 * around it; every update removes the previous region and inserts the new one by the same rule, empty text only removes. */
export type Interim = { start: number; end: number };
export function applyInterim(cur: string, sel: [number, number], region: Interim | null, piece: string): { value: string; region: Interim | null; caret: number } {
  let base = cur, s = Math.max(0, Math.min(sel[0], cur.length)), e = Math.max(s, Math.min(sel[1], cur.length));
  if (region) { const rs = Math.min(region.start, cur.length), re = Math.min(Math.max(region.end, rs), cur.length); base = cur.slice(0, rs) + cur.slice(re); s = e = rs; }
  const p = piece.trim();
  if (!p) return { value: base, region: null, caret: s };
  const r = insertAt(base, [s, e], p);
  const after = base.slice(e);
  return { value: r.value, region: { start: s, end: r.value.length - after.length }, caret: r.caret };
}

export const fmtTime = (ms: number) => { const s = Math.floor(ms / 1000); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };

/** Voice command from dictation (§4.7, slice 69): the whole phrase is a short command, otherwise `null` (the text is inserted
 *  as is).
 * `text:` / "текст:" at the start — forced insertion of the
 * remainder. */
export type VoiceCommand = { kind: 'send' | 'stop' | 'allow' | 'deny' | 'clear' | 'read' | 'new'; arg?: string } | { kind: 'text'; text: string };
export function parseCommand(raw: string): VoiceCommand | null {
  const t0 = raw.trim();
  const forced = t0.match(/^(?:текст|text)\s*[:：]\s*(.+)$/is);
  if (forced) return { kind: 'text', text: forced[1].trim() };
  const t = t0.toLowerCase().replace(/[.!?,;:…"'«»]+/g, ' ').replace(/\s+/g, ' ').trim().replace(/\s*(пожалуйста|please)$/u, '').replace(/^(пожалуйста|please)\s+/u, '').trim();
  if (!t || t.split(' ').length > 6) return null;
  const is = (...alts: string[]) => alts.includes(t);
  if (is('отправь', 'отправить', 'отправляй', 'send', 'send it')) return { kind: 'send' };
  if (is('стоп', 'останови', 'остановить', 'прерви', 'прервать', 'stop', 'stop it')) return { kind: 'stop' };
  if (is('разреши', 'разрешить', 'разрешаю', 'да разреши', 'allow', 'allow it', 'yes allow')) return { kind: 'allow' };
  if (is('запрети', 'запретить', 'отклони', 'отклонить', 'deny', 'deny it', 'reject')) return { kind: 'deny' };
  if (is('очисти', 'очистить', 'очисти поле', 'очистить поле', 'clear', 'clear it')) return { kind: 'clear' };
  if (is('озвучь', 'озвучить', 'прочитай', 'прочитай ответ', 'прочитать ответ', 'read it', 'read the reply', 'read aloud')) return { kind: 'read' };
  const m = t.match(/^(?:нов(?:ая|ую|ый) (?:беседа|беседу|тред|сессия|сессию)|new (?:thread|session|chat)) (?:в|in) (.+)$/u);
  if (m) return { kind: 'new', arg: m[1].trim() };
  return null;
}
/** Workspace name from what was said: exact, case-insensitive, by prefix or by substring; with several matches — null */
const TRANSLIT: Record<string, string> = { а: 'a', б: 'b', в: 'v', г: 'g', д: 'd', е: 'e', ё: 'e', ж: 'zh', з: 'z', и: 'i', й: 'j', к: 'k', л: 'l', м: 'm', н: 'n', о: 'o', п: 'p', р: 'r', с: 's', т: 't', у: 'u', ф: 'f', х: 'h', ц: 'c', ч: 'ch', ш: 'sh', щ: 'sch', ъ: '', ы: 'y', ь: '', э: 'e', ю: 'yu', я: 'ya' };
const latin = (s: string) => s.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, '').replace(/[а-яё]/g, c => TRANSLIT[c] ?? c);
export function matchProject(spoken: string, names: string[]): string | null {
  const q = latin(spoken); if (!q) return null;
  const exact = names.filter(n => latin(n) === q); if (exact.length === 1) return exact[0];
  const starts = names.filter(n => latin(n).startsWith(q) || q.startsWith(latin(n))); if (starts.length === 1) return starts[0];
  const inc = names.filter(n => latin(n).includes(q) || q.includes(latin(n))); if (inc.length === 1) return inc[0];
  // said in Russian ("энерджи хаб" → enerdzhihab): a match of the first four letters after
  // transliteration
  const head = q.slice(0, 4); if (head.length === 4) { const h = names.filter(n => latin(n).startsWith(head)); if (h.length === 1) return h[0]; }
  return null;
}
