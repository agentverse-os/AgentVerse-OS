// Workspace memory (ADR-009, docs/agents-ux.md §4.13): sizes, time spans and the label hint in the current language
import { t } from './i18n';
import type { WsMemory } from './api';

/** 2.6 GiB / 858 MiB */
export function memBytes(n?: number | null): string {
  const b = n ?? 0; const g = b / 1073741824;
  return g >= 1 ? t('mem.gib', { n: g.toFixed(1) }) : t('mem.mib', { n: Math.round(b / 1048576) });
}
/** 45 s / 2 min / 48 h */
export function memSpan(secs?: number | null): string {
  const s = Math.max(0, Math.round(secs ?? 0));
  return s >= 7200 ? t('mem.hours', { n: Math.floor(s / 3600) }) : s >= 120 ? t('mem.mins', { n: Math.floor(s / 60) }) : t('mem.secs', { n: s });
}
/** The hint of the tree label: used of the limit, the stall share, the largest process */
export function memTip(m: WsMemory): string {
  const head = t('mem.tip', { used: memBytes(m.used), limit: memBytes(m.limit), pct: Math.round(m.full60) });
  return m.top ? `${head} · ${t('mem.tipTop', { cmd: m.top.cmd, rss: memBytes(m.top.rss) })}` : head;
}
