// Drag-and-drop: cards by the header (rearranging on the canvas), sessions from the panel
// — onto the canvas.
import type { Ref } from './state.svelte';
export const dnd = $state<{ ref: Ref | null }>({ ref: null });
