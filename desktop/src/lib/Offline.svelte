<script lang="ts">
  // Соединение с ядром потеряно: оверлей поверх рабочего стола, пока опрос статуса не вернёт ответ.
  import Mark from './Mark.svelte';
  import { t, fmt } from './i18n.svelte';
  let { since, attempts, onretry }: { since: number; attempts: number; onretry: () => void } = $props();
  let now = $state(Date.now());
  $effect(() => { const t = setInterval(() => now = Date.now(), 1000); return () => clearInterval(t); });
  const secs = $derived(Math.max(0, Math.round((now - since) / 1000)));
  const dur = $derived(fmt.duration(secs));
</script>

<div class="offline" role="alert" aria-live="assertive">
  <div class="glow g1"></div><div class="glow g2"></div>
  <div class="box">
    <div class="mark"><Mark size={96} mono="#A2B4FF" /></div>
    <h1>{t('offline.title')}</h1>
    <p>{t('offline.text', { dur, n: attempts })}</p>
    <p class="muted">{t('offline.reasons')}</p>
    <button onclick={onretry}>{t('offline.retry')}</button>
  </div>
</div>

<style>
  .offline { position: fixed; inset: 0; z-index: 5000; display: grid; place-items: center; background: rgba(14, 16, 24, .92); backdrop-filter: blur(14px); -webkit-backdrop-filter: blur(14px); color: #F4F7FF; font-family: 'Space Grotesk', system-ui, sans-serif; overflow: hidden; }
  .glow { position: absolute; border-radius: 50%; filter: blur(80px); opacity: .5; animation: drift 18s ease-in-out infinite alternate; }
  .g1 { width: 60vw; height: 60vw; left: 55%; top: -20%; background: rgba(91, 140, 255, .35); }
  .g2 { width: 50vw; height: 50vw; left: -15%; top: 55%; background: rgba(122, 92, 255, .3); animation-delay: -9s; }
  @keyframes drift { from { transform: translate(0, 0); } to { transform: translate(-6vw, 5vh); } }
  .box { position: relative; text-align: center; max-width: 520px; padding: 32px; }
  .mark { display: inline-block; animation: pulse 2.4s ease-in-out infinite; }
  @keyframes pulse { 0%, 100% { opacity: .55; transform: scale(.98); } 50% { opacity: 1; transform: scale(1); } }
  h1 { margin: 18px 0 8px; font-size: 1.6rem; font-weight: 600; letter-spacing: -.01em; }
  p { margin: 6px 0; line-height: 1.5; color: #C9D0F0; }
  .muted { color: #8E97B8; font-size: .9rem; }
  button { margin-top: 18px; padding: 10px 18px; border-radius: 10px; border: 1px solid #3B4470; background: #1a1f33; color: #F4F7FF; font: inherit; cursor: pointer; }
  button:hover { border-color: #5B8CFF; }
  @media (prefers-reduced-motion: reduce) { .glow, .mark { animation: none; } }
</style>
