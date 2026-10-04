// Перетаскивание: карточки за шапку (перестановка на холсте), сессии из панели — на холст.
import type { Ref } from './state.svelte';
export const dnd = $state<{ ref: Ref | null }>({ ref: null });
