<script lang="ts">
  let { values, big = false }: { values: number[]; big?: boolean } = $props();
  const w = $derived(big ? 300 : 72), h = $derived(big ? 56 : 22);
  const pts = $derived.by(() => {
    const arr = values.length >= 2 ? values : [...values, ...values].slice(0, 2).length === 2 ? [values[0] ?? 0, values[0] ?? 0] : [0, 0];
    const m = Math.max(...arr, 1e-9);
    return arr.map((v, i) => [(i / (arr.length - 1)) * w, h - 3 - (v / m) * (h - 8)] as [number, number]);
  });
  const line = $derived(pts.map(p => p.join(',')).join(' '));
  const area = $derived(`M0,${h} ${pts.map(p => `L${p[0]},${p[1]}`).join(' ')} L${w},${h} Z`);
</script>

<svg class="spark" class:big viewBox="0 0 {w} {h}" preserveAspectRatio="none"><path d={area} /><polyline points={line} /></svg>
