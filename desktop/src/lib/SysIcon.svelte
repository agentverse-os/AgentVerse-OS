<script lang="ts">
  // Иконки системных приложений для облика umbrel: скруглённый квадрат с градиентом и белым глифом в мягкой тени —
  // как «объёмные» иконки дока umbrelOS (файлы — тёплая папка, магазин — синяя сумка, настройки — тёмные ползунки).
  // В обычном облике системные приложения показываются эмодзи (windows.svelte.ts → SYS_APPS.icon).
  export type SysIconName = 'projects' | 'store' | 'files' | 'system' | 'settings' | 'project' | 'vault' | 'updates' | 'setup' | 'appdetail' | 'reader' | 'appready' | 'agents';
  let { name, size = 48 }: { name: SysIconName; size?: number } = $props();
  const uid = Math.random().toString(36).slice(2, 8);
  const GRAD: Record<SysIconName, [string, string]> = {
    projects: ['#8B6CFF', '#4B2BD6'], store: ['#4DA3FF', '#1E5BFF'], files: ['#FF9A5C', '#E23D8C'], system: ['#43D7A6', '#118C6A'],
    settings: ['#6F7C99', '#2F3A54'], project: ['#7C8CFF', '#3B4FD9'], vault: ['#FFC857', '#D9781A'], updates: ['#5CE0B8', '#1C8C9E'],
    setup: ['#FF8A7A', '#C43A5B'], appdetail: ['#A9B6C8', '#5C6B82'], reader: ['#F7D46B', '#D2823A'], appready: ['#FFC857', '#D9781A'],
    agents: ['#FF7A59', '#C2185B'],
  };
  const [c1, c2] = $derived(GRAD[name] ?? GRAD.appdetail);
</script>

<svg class="sysicon" width={size} height={size} viewBox="0 0 64 64" aria-hidden="true">
  <defs>
    <linearGradient id="g{uid}" x1="0" y1="0" x2="0.4" y2="1"><stop offset="0" stop-color={c1} /><stop offset="1" stop-color={c2} /></linearGradient>
    <linearGradient id="h{uid}" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity=".35" /><stop offset=".5" stop-color="#fff" stop-opacity="0" /></linearGradient>
    <filter id="s{uid}" x="-20%" y="-20%" width="140%" height="150%"><feDropShadow dx="0" dy="1.5" stdDeviation="1.2" flood-color="#000" flood-opacity=".35" /></filter>
  </defs>
  <rect x="2" y="2" width="60" height="60" rx="15" fill="url(#g{uid})" />
  <rect x="2" y="2" width="60" height="60" rx="15" fill="url(#h{uid})" />
  <rect x="2.5" y="2.5" width="59" height="59" rx="14.5" fill="none" stroke="#fff" stroke-opacity=".18" />
  <g fill="#fff" filter="url(#s{uid})">
    {#if name === 'projects'}
      <rect x="15" y="14" width="34" height="9" rx="3" /><rect x="15" y="27" width="34" height="9" rx="3" opacity=".85" /><rect x="15" y="40" width="34" height="9" rx="3" opacity=".7" />
      <circle cx="20" cy="18.5" r="1.9" fill={c2} /><circle cx="20" cy="31.5" r="1.9" fill={c2} /><circle cx="20" cy="44.5" r="1.9" fill={c2} />
    {:else if name === 'agents'}
      <rect x="14" y="21" width="36" height="27" rx="8" /><circle cx="26" cy="34" r="4" fill={c2} /><circle cx="38" cy="34" r="4" fill={c2} /><rect x="30" y="12" width="4" height="9" rx="2" /><circle cx="32" cy="10.5" r="3" /><rect x="25" y="42" width="14" height="2.5" rx="1.25" fill={c2} opacity=".8" />
    {:else if name === 'store'}
      <path d="M17 24h30l-2.4 24.2a4 4 0 0 1-4 3.8H23.4a4 4 0 0 1-4-3.8Z" />
      <path d="M24 26v-4a8 8 0 0 1 16 0v4" fill="none" stroke="#fff" stroke-width="3.5" stroke-linecap="round" />
      <path d="M24.5 36c2 4.2 5 6 7.5 6s5.5-1.8 7.5-6" fill="none" stroke={c2} stroke-width="3" stroke-linecap="round" />
    {:else if name === 'files'}
      <path d="M12 21a4 4 0 0 1 4-4h11l4 4h17a4 4 0 0 1 4 4v3H12Z" opacity=".75" />
      <path d="M12 27h40a3 3 0 0 1 3 3l-2.4 16a4 4 0 0 1-4 3.4H17.4a4 4 0 0 1-4-3.4L11 30a3 3 0 0 1 1-3Z" />
    {:else if name === 'system'}
      <path d="M15 42a17 17 0 1 1 34 0" fill="none" stroke="#fff" stroke-width="5" stroke-linecap="round" />
      <path d="M32 42 42 28" stroke="#fff" stroke-width="4" stroke-linecap="round" /><circle cx="32" cy="42" r="4" />
    {:else if name === 'settings'}
      <rect x="14" y="19" width="36" height="4" rx="2" opacity=".9" /><rect x="14" y="30" width="36" height="4" rx="2" opacity=".9" /><rect x="14" y="41" width="36" height="4" rx="2" opacity=".9" />
      <rect x="20" y="15" width="7" height="12" rx="3.5" /><rect x="36" y="26" width="7" height="12" rx="3.5" /><rect x="24" y="37" width="7" height="12" rx="3.5" />
    {:else if name === 'project'}
      <path d="M32 13 50 23v18L32 51 14 41V23Z" opacity=".95" /><path d="M32 13v20l18-10" fill={c2} opacity=".35" /><path d="M32 33v18l-18-10" fill={c2} opacity=".55" />
    {:else if name === 'vault' || name === 'appready'}
      <circle cx="26" cy="30" r="9" fill="none" stroke="#fff" stroke-width="5" /><path d="M33 33l14 14M41 41l4-4M45 45l4-4" fill="none" stroke="#fff" stroke-width="5" stroke-linecap="round" />
    {:else if name === 'updates'}
      <path d="M32 46V20M20 31l12-12 12 12" fill="none" stroke="#fff" stroke-width="5.5" stroke-linecap="round" stroke-linejoin="round" />
    {:else if name === 'setup'}
      <circle cx="32" cy="32" r="17" fill="none" stroke="#fff" stroke-width="4" /><path d="M39 25 35 35l-10 4 4-10Z" /><circle cx="32" cy="32" r="2.2" fill={c2} />
    {:else if name === 'reader'}
      <rect x="15" y="16" width="34" height="32" rx="4" /><rect x="20" y="22" width="12" height="9" rx="1.5" fill={c2} opacity=".6" /><rect x="35" y="22" width="9" height="3" rx="1.5" fill={c2} opacity=".6" /><rect x="35" y="28" width="9" height="3" rx="1.5" fill={c2} opacity=".6" /><rect x="20" y="36" width="24" height="3" rx="1.5" fill={c2} opacity=".6" />
    {:else}
      <path d="M32 14 49 23v18l-17 9-17-9V23Z" fill="none" stroke="#fff" stroke-width="4.5" stroke-linejoin="round" /><path d="M15 23l17 9 17-9M32 32v18" fill="none" stroke="#fff" stroke-width="4.5" stroke-linejoin="round" />
    {/if}
  </g>
</svg>

<style>
  .sysicon { display: block; flex: none; }
</style>
