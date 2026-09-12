<script lang="ts">
  let { config, onconfig }: { config: Record<string, any>; onconfig: (p: Record<string, any>) => void } = $props();
  import { t } from '../i18n.svelte';
  let timer: ReturnType<typeof setTimeout> | undefined;
</script>
<textarea class="notes" placeholder={t('widgets.notes.placeholder')} value={config.text} oninput={(e) => { const v = (e.currentTarget as HTMLTextAreaElement).value; clearTimeout(timer); timer = setTimeout(() => onconfig({ text: v }), 500); }}></textarea>
<style>.notes { width: 100%; height: 100%; resize: none; background: transparent; border: 0; color: var(--ink); font: .95rem/1.5 var(--font); padding: 2px; }</style>
