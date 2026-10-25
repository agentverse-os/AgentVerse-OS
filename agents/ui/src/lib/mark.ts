// Session mark (docs/agents-ux.md §4.8): a deterministic small icon from the id — 12 hues × 8 shapes, the same everywhere.
// Nothing is stored: a 32-bit FNV-1a hash of the id → hue and shape.
export const MARK_SHAPES = ['circle', 'square', 'diamond', 'triangle', 'hexagon', 'star', 'drop', 'cross'] as const;
export type MarkShape = (typeof MARK_SHAPES)[number];
/** Shapes in viewBox 0 0 16 16 as a single
 *  path */
export const MARK_PATHS: Record<MarkShape, string> = {
  circle: 'M8 1.5a6.5 6.5 0 1 1 0 13a6.5 6.5 0 1 1 0-13z',
  square: 'M4 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2z',
  diamond: 'M8 1.2L14.8 8L8 14.8L1.2 8z',
  triangle: 'M8 1.6L15 14H1z',
  hexagon: 'M8 1.2l6 3.4v6.8l-6 3.4l-6-3.4V4.6z',
  star: 'M8 1.2l2 4.3l4.7.5l-3.5 3.2l1 4.6L8 11.5l-4.2 2.3l1-4.6L1.3 6l4.7-.5z',
  drop: 'M8 1.3c3 3.6 5.2 6.3 5.2 8.7a5.2 5.2 0 0 1-10.4 0C2.8 7.6 5 4.9 8 1.3z',
  cross: 'M5.5 1.5h5v4h4v5h-4v4h-5v-4h-4v-5h4z',
};
export function fnv1a(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 0x01000193) >>> 0; }
  return h >>> 0;
}
export function markOf(id: string): { shape: MarkShape; hue: number; path: string } {
  const h = fnv1a(id);
  const hue = (h % 12) * 30;
  const shape = MARK_SHAPES[(h >>> 8) % MARK_SHAPES.length];
  return { shape, hue, path: MARK_PATHS[shape] };
}
