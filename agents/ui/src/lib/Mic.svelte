<!-- Push-to-talk dictation button (docs/agents-ux.md §4.7): hold — recording, release — recognition and onText(text).
     Keyboard: Space/Enter — start, again — stop. Without a provider key it leads to Settings → Voice. -->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { ui, msg } from './state.svelte';
  import { voice, Recorder, StreamRecorder, transcribe, canRecord, canStream, micDenialHint, MAX_MS, MIN_MS, fmtTime } from './voice.svelte';
  import { t } from './i18n';
  import Icon from './Icon.svelte';
  import { holdWake, dropWake } from './wakelock';

  // onPartial — interim text of streaming dictation (§4.7, slice 72): an empty string removes it from the
  // field
  let { onText, onPartial, label = '', cls = 'composer-attach', title = '' }: { onText: (text: string, ms: number) => void; onPartial?: (text: string) => void; label?: string; cls?: string; title?: string } = $props();

  let phase = $state<'idle' | 'recording' | 'transcribing'>('idle');
  let elapsed = $state(0);
  let live = $state<'' | 'on' | 'off'>('');   // streaming mode: working / did not come up (recording as a batch)
  let note = $state<{ text: string; kind: 'hint' | 'bad'; setup?: boolean } | null>(null);
  let pressed = false; let startedAt = 0; let seq = 0;
  let rec: Recorder | StreamRecorder | null = null;
  let timer: ReturnType<typeof setInterval> | null = null; let noteTimer: ReturnType<typeof setTimeout> | null = null;

  function say(text: string, kind: 'hint' | 'bad' = 'hint', setup = false) {
    note = { text, kind, setup }; if (noteTimer) clearTimeout(noteTimer);
    noteTimer = setTimeout(() => { note = null; }, kind === 'bad' ? 7000 : 3500);
  }
  function toSettings() { ui.settingsTab = 'voice'; ui.section = 'settings'; ui.popen = false; }

  async function begin() {
    if (phase !== 'idle') return;
    if (!voice.ready) { say(t('voice.noKey'), 'hint', true); return; }
    if (!canRecord()) { say(t('voice.unsupported'), 'bad'); return; }
    const my = ++seq; note = null; phase = 'recording'; elapsed = 0; startedAt = performance.now(); live = ''; holdWake('ptt');   // the screen does not go dark while dictating (§9.9)
    const streaming = !!voice.settings?.stream && canStream() && !!onPartial;
    const r: Recorder | StreamRecorder = streaming ? new StreamRecorder(voice.settings?.language ?? '', (text) => { if (my === seq && phase === 'recording') onPartial?.(text); }, voice.settings?.provider) : new Recorder();
    rec = r;
    try { await r.start(); }
    catch (e) {
      if (my !== seq) return; rec = null; phase = 'idle'; void dropWake('ptt');
      const why = await micDenialHint(e);
      say(why === 'blocked' ? t('voice.noMicBlocked') : why === 'system' ? t('voice.noMicSystem') : why === 'device' ? t('voice.noMicDevice') : why === 'busy' ? t('voice.micBusy') : why === 'prompt' ? t('voice.noMic') : msg(e), 'bad'); return;
    }
    if (my !== seq) return;
    if (r instanceof StreamRecorder) live = r.failed ? 'off' : 'on';
    startedAt = performance.now();
    if (!pressed && !keyboard) { r.cancel(); rec = null; phase = 'idle'; void dropWake('ptt'); say(t('voice.tooEarly')); return; }
    if (timer) clearInterval(timer);
    timer = setInterval(() => { elapsed = performance.now() - startedAt; if (elapsed >= MAX_MS) finish(); }, 200);
  }
  async function finish() {
    if (phase !== 'recording') return;
    const my = ++seq; keyboard = false;
    if (timer) { clearInterval(timer); timer = null; }
    const dur = performance.now() - startedAt; const r = rec; rec = null; note = null; void dropWake('ptt');
    if (dur < MIN_MS) { r?.cancel(); if (r instanceof StreamRecorder) onPartial?.(''); phase = 'idle'; say(t('voice.hold')); return; }   // a short tap — the "hold" hint
    if (r && !r.micReady) { r.cancel(); if (r instanceof StreamRecorder) onPartial?.(''); phase = 'idle'; say(t('voice.tooEarly')); return; }   // held, but released while the browser was asking for permission
    if (r instanceof StreamRecorder) {
      phase = 'transcribing'; const t0 = performance.now();
      const res = await r.stop();
      if (my !== seq) return;
      if (res.streamed) { if (!res.text?.trim()) { onPartial?.(''); say(t('voice.nothing')); } else onText(res.text, performance.now() - t0); phase = 'idle'; return; }
      // the stream broke with interim text — keep what we heard; without text — the fallback batch path
      if (res.text?.trim()) { onText(res.text, performance.now() - t0); say(t('voice.streamBroke', { err: res.failed }), 'bad'); phase = 'idle'; return; }
      onPartial?.('');
      await batch(my, res.blob, res.failed); return;
    }
    const blob = r ? await r.stop() : new Blob();
    await batch(my, blob, '');
  }
  async function batch(my: number, blob: Blob, streamErr: string) {
    if (!blob.size) { phase = 'idle'; say(t('voice.nothing')); return; }
    phase = 'transcribing';
    try {
      const res = await transcribe(blob, voice.settings?.language ?? '');
      if (my !== seq) return;
      if (!res.text.trim()) say(t('voice.nothing')); else { onText(res.text, res.ms); if (streamErr) say(t('voice.streamFallback', { err: streamErr }), 'hint'); }
    } catch (e) { if (my === seq) say(msg(e), 'bad'); }
    finally { if (my === seq) phase = 'idle'; }
  }
  function abort() { seq++; if (timer) { clearInterval(timer); timer = null; } rec?.cancel(); rec = null; void dropWake('ptt'); if (phase === 'recording') onPartial?.(''); phase = 'idle'; pressed = false; keyboard = false; }

  let keyboard = false;
  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    if (!voice.ready) return; // a regular click leads to the settings
    e.preventDefault(); pressed = true;
    try { (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId); } catch {}
    // a safeguard without pointer capture: releasing anywhere on the page also finishes the
    // recording
    window.addEventListener('pointerup', up, { once: true }); window.addEventListener('pointercancel', up, { once: true });
    if (!keyboard) begin();
  }
  function up() { if (!pressed) return; pressed = false; if (!keyboard) finish(); }
  function key(e: KeyboardEvent) {
    if (e.key !== ' ' && e.key !== 'Enter') return;
    e.preventDefault();
    if (!voice.ready) { toSettings(); return; }
    if (phase === 'recording') { finish(); return; }
    if (phase === 'idle') { keyboard = true; begin(); say(t('voice.stopKey')); }
  }
  async function click(e: MouseEvent) {
    if (voice.ready) { e.preventDefault(); return; }
    if (!voice.settings) { await voice.load(); if (voice.ready) { say(t('voice.hold')); return; } }
    toSettings();
  }
  const vis = () => { if (document.visibilityState === 'hidden' && phase === 'recording') finish(); };
  $effect(() => { document.addEventListener('visibilitychange', vis); voice.load(); return () => document.removeEventListener('visibilitychange', vis); });
  onDestroy(() => { abort(); if (noteTimer) clearTimeout(noteTimer); });
</script>

<span class="ptt" class:rec={phase === 'recording'} class:busy={phase === 'transcribing'}>
  {#if phase === 'recording'}
    <span class="ptt-pill rec" role="status" data-live={live}><span class="dot"></span>{fmtTime(elapsed)} / {fmtTime(MAX_MS)} · {live === 'on' ? t('voice.liveText') : live === 'off' ? t('voice.liveOff') : t('voice.release')}</span>
  {:else if phase === 'transcribing'}
    <span class="ptt-pill" role="status"><Icon name="refresh" size="s" /> {t('voice.transcribing')}</span>
  {:else if note}
    <span class="ptt-pill {note.kind}" role={note.kind === 'bad' ? 'alert' : 'status'}>{note.text}{#if note.setup} · <button type="button" class="link" onclick={toSettings}>{t('voice.setup')}</button>{/if}</span>
  {/if}
  <button type="button" class="ptt-btn {cls}" class:on={phase === 'recording'} aria-label={label || t('voice.mic')} title={title || t('voice.mic')} aria-pressed={phase === 'recording'} disabled={phase === 'transcribing'}
    onpointerdown={down} onpointerup={up} onpointercancel={up} onlostpointercapture={up} oncontextmenu={(e) => e.preventDefault()} onkeydown={key} onclick={click}>
    <Icon name={phase === 'transcribing' ? 'refresh' : 'mic'} size="s" />{#if label}<span class="lbl">{label}</span>{/if}
  </button>
</span>
