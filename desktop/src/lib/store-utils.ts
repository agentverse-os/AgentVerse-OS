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
  return i; // URL или emoji
}
export function isImg(i: string | null) { return !!i && (i.startsWith('/') || i.startsWith('http')); }
export function titleOf(a: AppView) { return a.manifest.title ?? a.name; }
/** Описание на языке интерфейса: `i18n.<lang>.description` манифеста, иначе `description` (у импортированных — английское из upstream). */
export function descOf(a: AppView): string { return a.manifest.i18n?.[i18n.lang]?.description ?? a.manifest.description ?? ''; }
/** Причина из списка «Рекомендуем» на языке интерфейса. */
export function reasonOf(a: AppView): string { const r = a.manifest.recommended; return r ? (r.i18n?.[i18n.lang]?.reason ?? r.reason) : ''; }
/** Базовое имя без суффикса источника — чтобы группировать дубликаты в одну строку. */
export function baseName(a: AppView) { return a.name.replace(/-(coolify|umbrel)$/, ''); }

/** URL для кнопки «Открыть»: адрес приложения плюс `login.path` (например, страница панели управления). */
/** Адрес приложения с тем же хостом, по которому открыт Desktop: если Desktop открыт по IP (телефон без MagicDNS), приложения тоже идут по IP —
 *  edge отдаёт один и тот же сертификат и маршруты для всех своих адресов. */
export function sameHost(url: string): string { try { const u = new URL(url); if (location.hostname && u.hostname !== location.hostname && location.protocol === 'https:') u.hostname = location.hostname; return u.toString(); } catch { return url; } }
export function openUrl(a: AppView): string { const p = a.manifest.login?.path?.replace(/^\//, '') ?? ''; return sameHost((a.url ?? '') + p); }
