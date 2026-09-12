<script lang="ts">
  import { i18n } from './i18n.svelte';
  // Заставка по бездействию: знак Monolith, часы и дата на тёмном фоне с медленным свечением. Любое действие снимает.
  // Внутри iframe-окон события не видны, поэтому пока фокус в iframe (code-server, приложения) заставка не включается.
  import Mark from './Mark.svelte';
  let { minutes, statusText = '' }: { minutes: number; statusText?: string } = $props();
  let on = $state(false);
  let now = $state(new Date());
  let timer: ReturnType<typeof setTimeout> | undefined;
  function idleMs() { const t = Number(localStorage.getItem('cloudos.screensaver.testMs') ?? ''); return t > 0 ? t : minutes * 60_000; }
  function arm() {
    clearTimeout(timer);
    if (!minutes || minutes <= 0) return;
    timer = setTimeout(() => { if (document.activeElement?.tagName === 'IFRAME' || document.hidden) { arm(); return; } on = true; }, idleMs());
  }
  function wake() { if (on) on = false; arm(); }
  $effect(() => {
    const ev: (keyof WindowEventMap)[] = ['pointermove', 'pointerdown', 'keydown', 'wheel', 'touchstart'];
    for (const e of ev) window.addEventListener(e, wake, { passive: true });
    window.addEventListener('focus', wake); window.addEventListener('blur', arm);
    const clock = setInterval(() => now = new Date(), 1000);
    arm();
    return () => { for (const e of ev) window.removeEventListener(e, wake); window.removeEventListener('focus', wake); window.removeEventListener('blur', arm); clearInterval(clock); clearTimeout(timer); };
  });
  const hh = $derived(now.toLocaleTimeString(i18n.locale, { hour: '2-digit', minute: '2-digit' }));
  const dd = $derived(now.toLocaleDateString(i18n.locale, { weekday: 'long', day: 'numeric', month: 'long' }));
</script>

{#if on}
  <div class="saver" role="presentation" onpointerdown={wake} onkeydown={wake} tabindex="-1">
    <div class="glow g1"></div><div class="glow g2"></div><div class="glow g3"></div>
    <div class="scene">
      <div class="mark"><Mark size={240} /></div>
      <div class="time">{hh}</div>
      <div class="date">{dd}</div>
      {#if statusText}<div class="status">{statusText}</div>{/if}
    </div>
    <div class="brand"><Mark size={18} simple /><span>AgentVerse <b>OS</b></span></div>
  </div>
{/if}

<style>
  .saver { position: fixed; inset: 0; z-index: 6000; background: url('/wallpaper-monolith-quiet.png') center / cover no-repeat #0B0D14; color: #F4F7FF; overflow: hidden; cursor: none; font-family: 'Space Grotesk', system-ui, sans-serif; animation: fadein .8s ease; }
  @keyframes fadein { from { opacity: 0; } to { opacity: 1; } }
  .glow { position: absolute; border-radius: 50%; filter: blur(90px); opacity: .55; animation: drift 26s ease-in-out infinite alternate; }
  .g1 { width: 55vw; height: 55vw; left: 50%; top: -15%; background: rgba(91, 140, 255, .32); }
  .g2 { width: 45vw; height: 45vw; left: -12%; top: 50%; background: rgba(122, 92, 255, .28); animation-delay: -12s; }
  .g3 { width: 30vw; height: 30vw; left: 60%; top: 60%; background: rgba(162, 180, 255, .16); animation-delay: -20s; animation-duration: 34s; }
  @keyframes drift { from { transform: translate(0, 0) scale(1); } to { transform: translate(-8vw, 6vh) scale(1.08); } }
  .scene { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; }
  .mark { animation: float 9s ease-in-out infinite; filter: drop-shadow(0 20px 60px rgba(91, 140, 255, .25)); }
  @keyframes float { 0%, 100% { transform: translateY(0); } 50% { transform: translateY(-14px); } }
  .time { margin-top: 28px; font-size: clamp(64px, 12vw, 160px); font-weight: 600; letter-spacing: -.03em; line-height: 1; }
  .date { font-size: clamp(16px, 2vw, 24px); color: #A2B4FF; text-transform: capitalize; }
  .status { margin-top: 18px; font-size: 14px; letter-spacing: .18em; text-transform: uppercase; color: #8E97B8; }
  .brand { position: absolute; left: 32px; bottom: 28px; display: flex; align-items: center; gap: 10px; font-size: 14px; color: #8E97B8; }
  .brand b { color: #A2B4FF; font-weight: 600; }
  @media (prefers-reduced-motion: reduce) { .glow, .mark { animation: none; } }
</style>
