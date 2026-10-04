<script lang="ts">
  import { ICONS } from './icons';
  let { name, size = 'm' }: { name: string; size?: 's' | 'm' | 'l' } = $props();
</script>

<svg class="i {size === 's' ? 's' : size === 'l' ? 'l' : ''}" viewBox="0 0 24 24" aria-hidden="true">{@html ICONS[name] ?? ''}</svg>
