import { fmt } from '../i18n.svelte';
/** Размер и «сколько прошло» — на языке интерфейса (i18n.svelte.ts → fmt). */
export function fmtBytes(n: number) { return fmt.bytes(n); }
export function ago(iso?: string | null) { return fmt.ago(iso); }
/** SVG-полилиния для мини-графика 0..max */
export function spark(values: number[], max: number, w = 200, h = 40) {
  if (!values.length) return '';
  const step = w / Math.max(1, values.length - 1);
  return values.map((v, i) => `${(i * step).toFixed(1)},${(h - Math.min(1, v / (max || 1)) * h).toFixed(1)}`).join(' ');
}
