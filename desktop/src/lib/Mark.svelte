<script lang="ts">
  // Знак Monolith — сгенерировано assets/build.py --install из той же геометрии, что assets/mark.svg. Не править руками.
  // simple — версия для 16–18 px; mono — одноцветный силуэт указанного цвета.
  let { size = 24, simple = false, mono = '' }: { size?: number; simple?: boolean; mono?: string } = $props();
  const uid = `mk${Math.random().toString(36).slice(2, 8)}`;
</script>

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width={size} height={size} role="img" aria-label="AgentVerse OS" style="display:block; flex: none;">
  {#if mono}
    <defs>
  <mask id="{uid}-cut" maskUnits="userSpaceOnUse" x="-64" y="-64" width="384" height="384">
    <rect x="-64" y="-64" width="384" height="384" fill="white"/>
    <polygon points="128,118 108,228 148,228" fill="black"/>
    <g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 0 246 172" fill="none" stroke="black" stroke-width="15" stroke-linecap="round"/></g>
  </mask>
  <mask id="{uid}-cut2" maskUnits="userSpaceOnUse" x="-64" y="-64" width="384" height="384">
    <rect x="-64" y="-64" width="384" height="384" fill="white"/>
    <g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 0 246 172" fill="none" stroke="black" stroke-width="15" stroke-linecap="round"/></g>
  </mask></defs>
    <g mask="url(#{uid}-cut)">
  <g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 1 246 172" fill="none" stroke="{mono}" stroke-width="9" stroke-linecap="round" opacity="0.85"/></g>
  <polygon points="128,18 62,228 194,228" fill="{mono}"/>
</g>
<g mask="url(#{uid}-cut2)"><polygon points="128,146 117,228 139,228" fill="{mono}"/></g>
<g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 0 246 172" fill="none" stroke="{mono}" stroke-width="9" stroke-linecap="round"/></g>
  {:else if simple}
    <defs>
  <linearGradient id="{uid}-gl" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#F4F7FF"/><stop offset="0.35" stop-color="#A2B4FF"/><stop offset="1" stop-color="#5B8CFF"/></linearGradient>
  <linearGradient id="{uid}-gr" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#A2B4FF"/><stop offset="0.4" stop-color="#7A5CFF"/><stop offset="1" stop-color="#3B2FA8"/></linearGradient>
  <linearGradient id="{uid}-gc" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#F4F7FF"/><stop offset="1" stop-color="#A2B4FF"/></linearGradient>
  <linearGradient id="{uid}-gw" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#F4F7FF"/><stop offset="0.5" stop-color="#A2B4FF"/><stop offset="1" stop-color="#7A5CFF"/></linearGradient>
  <linearGradient id="{uid}-rb" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#3B2FA8"/><stop offset="1" stop-color="#7A5CFF"/></linearGradient>
  <linearGradient id="{uid}-rf" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#5B8CFF"/><stop offset="0.55" stop-color="#A2B4FF"/><stop offset="1" stop-color="#7A5CFF"/></linearGradient>
  <mask id="{uid}-cut" maskUnits="userSpaceOnUse" x="-64" y="-64" width="384" height="384">
    <rect x="-64" y="-64" width="384" height="384" fill="white"/>
    <polygon points="128,132 112,228 144,228" fill="black"/>
    <g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 0 246 172" fill="none" stroke="black" stroke-width="26" stroke-linecap="round"/></g>
  </mask>
  <mask id="{uid}-cut2" maskUnits="userSpaceOnUse" x="-64" y="-64" width="384" height="384">
    <rect x="-64" y="-64" width="384" height="384" fill="white"/>
    <g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 0 246 172" fill="none" stroke="black" stroke-width="26" stroke-linecap="round"/></g>
  </mask></defs>
    <g mask="url(#{uid}-cut)">
  <g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 1 246 172" fill="none" stroke="url(#{uid}-rb)" stroke-width="16" stroke-linecap="round" opacity="0.85"/></g>
  <polygon points="128,18 62,228 194,228" fill="url(#{uid}-gl)"/>
</g>

<g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 0 246 172" fill="none" stroke="url(#{uid}-rf)" stroke-width="16" stroke-linecap="round"/></g>
  {:else}
    <defs>
  <linearGradient id="{uid}-gl" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#F4F7FF"/><stop offset="0.35" stop-color="#A2B4FF"/><stop offset="1" stop-color="#5B8CFF"/></linearGradient>
  <linearGradient id="{uid}-gr" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#A2B4FF"/><stop offset="0.4" stop-color="#7A5CFF"/><stop offset="1" stop-color="#3B2FA8"/></linearGradient>
  <linearGradient id="{uid}-gc" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#F4F7FF"/><stop offset="1" stop-color="#A2B4FF"/></linearGradient>
  <linearGradient id="{uid}-gw" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#F4F7FF"/><stop offset="0.5" stop-color="#A2B4FF"/><stop offset="1" stop-color="#7A5CFF"/></linearGradient>
  <linearGradient id="{uid}-rb" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#3B2FA8"/><stop offset="1" stop-color="#7A5CFF"/></linearGradient>
  <linearGradient id="{uid}-rf" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#5B8CFF"/><stop offset="0.55" stop-color="#A2B4FF"/><stop offset="1" stop-color="#7A5CFF"/></linearGradient>
  <mask id="{uid}-cut" maskUnits="userSpaceOnUse" x="-64" y="-64" width="384" height="384">
    <rect x="-64" y="-64" width="384" height="384" fill="white"/>
    <polygon points="128,118 108,228 148,228" fill="black"/>
    <g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 0 246 172" fill="none" stroke="black" stroke-width="15" stroke-linecap="round"/></g>
  </mask>
  <mask id="{uid}-cut2" maskUnits="userSpaceOnUse" x="-64" y="-64" width="384" height="384">
    <rect x="-64" y="-64" width="384" height="384" fill="white"/>
    <g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 0 246 172" fill="none" stroke="black" stroke-width="15" stroke-linecap="round"/></g>
  </mask></defs>
    <g mask="url(#{uid}-cut)">
  <g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 1 246 172" fill="none" stroke="url(#{uid}-rb)" stroke-width="9" stroke-linecap="round" opacity="0.85"/></g>
  <polygon points="128,18 62,228 128,228" fill="url(#{uid}-gl)"/><polygon points="128,18 128,228 194,228" fill="url(#{uid}-gr)"/><polygon points="128,18 121,228 135,228" fill="url(#{uid}-gc)" opacity="0.9"/>
</g>
<g mask="url(#{uid}-cut2)"><polygon points="128,146 117,228 139,228" fill="url(#{uid}-gw)"/></g>
<g transform="rotate(-10 128 172)"><path d="M 10 172 A 118 30 0 0 0 246 172" fill="none" stroke="url(#{uid}-rf)" stroke-width="9" stroke-linecap="round"/></g>
  {/if}
</svg>
