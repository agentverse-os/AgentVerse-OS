// Shared by the "News" widget and the reader: the entry type, the cache of read feeds, the source color, entity
// decoding.
export interface NewsItem { title: string; link: string; published?: string | null; source: string; summary?: string | null; image?: string | null; content?: string | null; author?: string | null; feed: string }
/** Entries by link — the reader takes the article from here, and after a page reload re-reads the feed. */
export const newsCache = new Map<string, NewsItem>();
export function decode(t: string | null | undefined): string { if (!t) return ''; const el = document.createElement('textarea'); el.innerHTML = t; return el.value; }
/** Short source name: for "AI | The Verge" and "Все статьи подряд / ИИ / Хабр" the meaning is in the last segment, for "Хабр — новости" — the
 *  first. */
export function shortSource(s: string): string {
  let t = s.trim();
  if (t.includes(' | ')) t = t.split(' | ').pop()!.trim();
  else if (t.includes(' / ')) t = t.split(' / ').pop()!.trim();
  else t = t.split(/\s[–—:]\s|\s-\s/)[0].trim();
  return t.length > 22 ? t.slice(0, 21) + '…' : t;
}
/** Stable source color from the name hash. */
export function sourceColor(s: string): string { let h = 0; for (const c of s) h = (h * 31 + c.charCodeAt(0)) >>> 0; return `hsl(${h % 360} 60% 55%)`; }
const READ_KEY = 'cloudos.news.read';
export function readSet(): Set<string> { try { return new Set(JSON.parse(localStorage.getItem(READ_KEY) ?? '[]')); } catch { return new Set(); } }
export function markRead(link: string) { try { const s = readSet(); s.add(link); const arr = [...s]; localStorage.setItem(READ_KEY, JSON.stringify(arr.slice(-500))); } catch {} }
/** Teaser without the Hacker News service junk ("Article URL: … Comments URL: … Points:
 *  …"). */
export function cleanSummary(s: string | null | undefined): string {
  const t = decode(s ?? '').trim();
  if (!t || /^Article URL:/i.test(t)) return '';
  return t.replace(/\s*(Article URL|Comments URL|Points|# Comments):.*$/i, '').trim();
}
/** Key for merging duplicates: one news item from different
 *  feeds. */
export function titleKey(t: string): string { return decode(t).toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim().slice(0, 80); }
