import { fmt } from '../i18n.svelte';
/** Size and "how long ago" in the UI language (i18n.svelte.ts → fmt). */
export function fmtBytes(n: number) { return fmt.bytes(n); }
export function ago(iso?: string | null) { return fmt.ago(iso); }
/** SVG polyline for a 0..max mini chart */
export function spark(values: number[], max: number, w = 200, h = 40) {
  if (!values.length) return '';
  const step = w / Math.max(1, values.length - 1);
  return values.map((v, i) => `${(i * step).toFixed(1)},${(h - Math.min(1, v / (max || 1)) * h).toFixed(1)}`).join(' ');
}
