// Общее для виджета «Новости» и читалки: тип записи, кеш прочитанных лент, цвет источника, раскодирование сущностей.
export interface NewsItem { title: string; link: string; published?: string | null; source: string; summary?: string | null; image?: string | null; content?: string | null; author?: string | null; feed: string }
/** Записи по ссылке — читалка берёт статью отсюда, а после перезагрузки страницы перечитывает ленту. */
export const newsCache = new Map<string, NewsItem>();
export function decode(t: string | null | undefined): string { if (!t) return ''; const el = document.createElement('textarea'); el.innerHTML = t; return el.value; }
/** Короткое имя источника: у «AI | The Verge» и «Все статьи подряд / ИИ / Хабр» смысл несёт последний сегмент, у «Хабр — новости» — первый. */
export function shortSource(s: string): string {
  let t = s.trim();
  if (t.includes(' | ')) t = t.split(' | ').pop()!.trim();
  else if (t.includes(' / ')) t = t.split(' / ').pop()!.trim();
  else t = t.split(/\s[–—:]\s|\s-\s/)[0].trim();
  return t.length > 22 ? t.slice(0, 21) + '…' : t;
}
/** Устойчивый цвет источника из хеша имени. */
export function sourceColor(s: string): string { let h = 0; for (const c of s) h = (h * 31 + c.charCodeAt(0)) >>> 0; return `hsl(${h % 360} 60% 55%)`; }
const READ_KEY = 'cloudos.news.read';
export function readSet(): Set<string> { try { return new Set(JSON.parse(localStorage.getItem(READ_KEY) ?? '[]')); } catch { return new Set(); } }
export function markRead(link: string) { try { const s = readSet(); s.add(link); const arr = [...s]; localStorage.setItem(READ_KEY, JSON.stringify(arr.slice(-500))); } catch {} }
/** Анонс без служебного мусора Hacker News («Article URL: … Comments URL: … Points: …»). */
export function cleanSummary(s: string | null | undefined): string {
  const t = decode(s ?? '').trim();
  if (!t || /^Article URL:/i.test(t)) return '';
  return t.replace(/\s*(Article URL|Comments URL|Points|# Comments):.*$/i, '').trim();
}
/** Ключ для склейки дубликатов: одна новость из разных лент. */
export function titleKey(t: string): string { return decode(t).toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim().slice(0, 80); }
