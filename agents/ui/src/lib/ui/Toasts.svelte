<script lang="ts">
  import { toasts } from '../toast.svelte';
  import Icon from '../Icon.svelte';
</script>

<div class="toasts" aria-live="polite">
  {#each toasts.list as t (t.id)}
    <div class="toast {t.kind}"><span class="ic"><Icon name={t.kind === 'bad' ? 'x' : t.kind === 'info' ? 'info' : 'check'} size="s" /></span><span>{t.text}</span></div>
  {/each}
</div>
