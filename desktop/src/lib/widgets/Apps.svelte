<script lang="ts">
  import { api, type AppView } from '../api';
  import { wm } from '../windows.svelte';
  import { t } from '../i18n.svelte';
  import { iconOf, isImg, titleOf, openUrl, descOf } from '../store-utils';
  let apps = $state<AppView[]>([]);
  $effect(() => { const f = () => api.apps().then(a => apps = a.filter(x => x.installed && x.url)).catch(() => {}); f(); const t = setInterval(f, 20000); return () => clearInterval(t); });
  function open(a: AppView) { if (!a.url) return; if (a.manifest.route.open === 'iframe') wm.open(`app:${a.name}`, titleOf(a), openUrl(a), iconOf(a)); else window.open(openUrl(a), '_blank', 'noopener'); }
</script>
<div class="apps">
  {#each apps as a (a.name)}
    {@const ic = iconOf(a)}
    <button class="a" title={descOf(a)} onclick={() => open(a)}>{#if isImg(ic)}<img src={ic} alt="" />{:else}<span class="e">{ic ?? '📦'}</span>{/if}<span>{titleOf(a)}</span></button>
  {/each}
  {#if !apps.length}<button class="a" onclick={() => wm.openSys('store')}>🛍️ <span>{t('widgets.apps.openStore')}</span></button>{/if}
</div>
<style>
  .apps { display: flex; flex-wrap: wrap; gap: 6px; }
  .a { display: flex; flex-direction: column; align-items: center; gap: 4px; width: 72px; padding: 6px 4px; background: transparent; border-color: transparent; font-size: .72rem; }
  .a img, .a .e { width: 32px; height: 32px; border-radius: 8px; object-fit: cover; font-size: 24px; display: flex; align-items: center; justify-content: center; }
  .a span:last-child { max-width: 68px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
