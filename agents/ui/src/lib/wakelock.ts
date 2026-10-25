// Wake Lock (docs/agents-ux.md §9.9): the screen does not go dark while dictation is held or a turn is running in
// the open Thread. Holders name a reason (a key); the lock is taken while there is at least one key and released
// with the last one. When the tab goes to the background the browser releases the lock itself — on return we take
// it again if reasons remain.
type WakeLockSentinel = { release: () => Promise<void>; addEventListener: (t: 'release', cb: () => void) => void };
const holders = new Set<string>();
let sentinel: WakeLockSentinel | null = null;
let listening = false;
export const wakeLockSupported = () => typeof navigator !== 'undefined' && 'wakeLock' in navigator;
async function acquire() {
  if (sentinel || !wakeLockSupported() || typeof document === 'undefined' || document.visibilityState !== 'visible') return;
  try {
    const s = await (navigator as Navigator & { wakeLock: { request: (t: 'screen') => Promise<WakeLockSentinel> } }).wakeLock.request('screen');
    sentinel = s; s.addEventListener('release', () => { if (sentinel === s) sentinel = null; });
  } catch {}
}
function listen() {
  if (listening || typeof document === 'undefined') return; listening = true;
  document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible' && holders.size) void acquire(); });
}
export function holdWake(key: string) { holders.add(key); listen(); void acquire(); }
export async function dropWake(key: string) { holders.delete(key); if (holders.size || !sentinel) return; const s = sentinel; sentinel = null; try { await s.release(); } catch {} }
export const wakeHeld = () => !!sentinel;
