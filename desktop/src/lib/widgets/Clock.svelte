<script lang="ts">
  import { t, i18n } from '../i18n.svelte';
  let { config, onconfig }: { config: Record<string, any>; onconfig: (p: Record<string, any>) => void } = $props();
  let now = $state(new Date());
  let settings = $state(false);
  $effect(() => { const t = setInterval(() => now = new Date(), 1000); return () => clearInterval(t); });
  const tzOpts = $derived(config.tz ? { timeZone: config.tz } : {});
  function fmt(d: Date) { try { return d.toLocaleTimeString(i18n.locale, { hour: '2-digit', minute: '2-digit', ...(config.seconds ? { second: '2-digit' } : {}), ...tzOpts }); } catch { return d.toLocaleTimeString(i18n.locale); } }
  function fmtDate(d: Date) { try { return d.toLocaleDateString(i18n.locale, { weekday: 'long', day: 'numeric', month: 'long', ...tzOpts }); } catch { return d.toLocaleDateString(i18n.locale); } }
</script>
{#if settings}
  <div class="row"><input placeholder={t('widgets.clock.tzPlaceholder')} value={config.tz} onchange={(e) => onconfig({ tz: (e.currentTarget as HTMLInputElement).value })} style="flex:1" /></div>
  <label class="row muted"><input type="checkbox" checked={config.seconds} onchange={(e) => onconfig({ seconds: (e.currentTarget as HTMLInputElement).checked })} /> {t('widgets.clock.seconds')}</label>
  <button onclick={() => settings = false}>{t('common.done')}</button>
{:else}
  <button class="clock" ondblclick={() => settings = true} title={t('widgets.clock.dblSettings')}>
    <div class="time">{fmt(now)}</div>
    <div class="muted date">{fmtDate(now)}{config.tz ? ` · ${config.tz}` : ''}</div>
  </button>
{/if}
<style>
  .clock { display: flex; flex-direction: column; align-items: flex-start; width: 100%; height: 100%; background: transparent; border-color: transparent; padding: 4px; }
  .time { font-size: 2.6rem; font-weight: 700; letter-spacing: .02em; line-height: 1; }
  .date { font-size: .9rem; margin-top: 6px; }
</style>
