<!-- Session mark (§4.8): 14 px, color via --mark-h; the lightness is set in app.css separately for the dark and light
     themes -->
<script lang="ts">
  import { markOf } from './mark';
  let { id, size = 14 }: { id: string; size?: number } = $props();
  const m = $derived(markOf(id));
</script>

<svg class="smark" data-shape={m.shape} style="--mark-h: {m.hue}; width: {size}px; height: {size}px" viewBox="0 0 16 16" aria-hidden="true"><path d={m.path} /></svg>
