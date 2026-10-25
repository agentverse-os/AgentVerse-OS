import type { AppView } from './api';
import { i18n } from './i18n.svelte';
export const SOURCES: Record<string, { label: string; color: string }> = {
  runtipi: { label: 'Runtipi', color: '#4F8EF7' },
  coolify: { label: 'Coolify', color: '#7C6CF6' },
  umbrel: { label: 'Umbrel', color: '#2FB39B' },
  manual: { label: 'AgentVerse OS', color: '#5BBFCB' },
};
export function sourceOf(a: AppView) { return SOURCES[a.manifest.origin] ?? { label: a.manifest.origin, color: '#9AA3A8' }; }
export function iconOf(a: AppView): string | null {
  const i = a.manifest.icon;
  if (!i) return null;
  if (i.startsWith('./')) return `/api/apps/${a.name}/icon`;
  return i; // a URL or an emoji
}
export function isImg(i: string | null) { return !!i && (i.startsWith('/') || i.startsWith('http')); }
export function titleOf(a: AppView) { return a.manifest.title ?? a.name; }
/** Description in the UI language: the manifest's `i18n.<lang>.description`, otherwise `description` (imported ones have the English one
 *  from upstream). */
export function descOf(a: AppView): string { return a.manifest.i18n?.[i18n.lang]?.description ?? a.manifest.description ?? ''; }
/** Reason from the "Recommended" list in the UI language. */
export function reasonOf(a: AppView): string { const r = a.manifest.recommended; return r ? (r.i18n?.[i18n.lang]?.reason ?? r.reason) : ''; }
/** Base name without the source suffix — to group duplicates into one row. */
export function baseName(a: AppView) { return a.name.replace(/-(coolify|umbrel)$/, ''); }

/** URL for the "Open" button: the app address plus `login.path` (e.g. the admin panel page). */
/** App address with the same host Desktop is opened on: if Desktop is opened by IP (a phone without MagicDNS), the apps go by IP too —
 * the edge serves the same certificate and routes for all of its addresses.
 * */
export function sameHost(url: string): string { try { const u = new URL(url); if (location.hostname && u.hostname !== location.hostname && location.protocol === 'https:') u.hostname = location.hostname; return u.toString(); } catch { return url; } }
export function openUrl(a: AppView): string { if (a.manifest.type === 'external') return a.url ?? ''; const p = a.manifest.login?.path?.replace(/^\// , '') ?? ''; return sameHost((a.url ?? '') + p); }  // an external service lives on its own host
