<script lang="ts">
  import { api, type Event } from '../api';
  import { t } from '../i18n.svelte';
  let { config }: { config: Record<string, any> } = $props();
  let ev = $state<Event[]>([]);
  $effect(() => { const f = () => api.events().then(e => ev = e.slice(0, config.count ?? 12)).catch(() => {}); f(); const t = setInterval(f, 8000); return () => clearInterval(t); });
</script>
<div class="ev">
  {#each ev as e}<div><code>{e.at.slice(11, 19)}</code> <b>{e.kind}</b> {e.subject} <span class="muted">— {e.message}</span></div>{/each}
  {#if !ev.length}<div class="muted">{t('common.empty')}</div>{/if}
</div>
<style>.ev { font-size: .8rem; display: flex; flex-direction: column; gap: 3px; } .ev div { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }</style>
