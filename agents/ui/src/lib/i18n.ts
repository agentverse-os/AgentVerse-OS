// Strings of the "Agents" app in four languages (English, Russian, Ukrainian, Spanish). Dictionaries — i18n/messages/*.ts: one
// file is one namespace, four languages, keys are type-checked against English (i18n/define.ts). Russian is the source language
// of the UI (the e2e checks look up elements by it too). The current language is the reactive state lang.svelte.ts: t() in
// markup is recomputed on a language change without a reload. Texts from the core and the service (API errors, events) are not
// translated.
import { lang, LANGS, LANG_NAMES, LOCALES, type Lang, type LangSetting } from './lang.svelte';
import type { Messages } from './i18n/define';
export { lang, LANGS, LANG_NAMES, LOCALES, type Lang, type LangSetting };

type Dict = Record<string, string>;
const modules = import.meta.glob<{ default: Messages<Dict> }>('./i18n/messages/*.ts', { eager: true });
const DICTS: Record<Lang, Dict> = { en: {}, ru: {}, uk: {}, es: {} };
for (const [file, m] of Object.entries(modules)) {
  for (const l of LANGS) {
    if (import.meta.env.DEV) for (const k of Object.keys(m.default[l])) if (k in DICTS[l]) console.warn(`i18n: ключ ${k} задан повторно (${file})`);
    Object.assign(DICTS[l], m.default[l]);
  }
}

function fmtStr(s: string, params?: Record<string, unknown>) {
  return params ? s.replace(/\{(\w+)\}/g, (m, k) => (k in params ? String(params[k] ?? '') : m)) : s;
}
/** String by key in the current language with {name} substitution; no translation — English; no key — the key
 *  itself. */
export function t(key: string, params?: Record<string, unknown>): string {
  const s = DICTS[lang.lang][key] ?? DICTS.en[key];
  if (s === undefined) { if (import.meta.env.DEV) console.warn(`i18n: нет ключа ${key}`); return fmtStr(key, params); }
  return fmtStr(s, params);
}
const rules = new Map<string, Intl.PluralRules>();
/** Plural: the key `<key>.<one|few|many|other>` per Intl.PluralRules of the current locale, {n} is substituted
 *  automatically. */
export function tn(key: string, n: number, params?: Record<string, unknown>): string {
  const loc = lang.locale;
  let pr = rules.get(loc);
  if (!pr) { pr = new Intl.PluralRules(loc); rules.set(loc, pr); }
  const d = DICTS[lang.lang];
  const pick = [`${key}.${pr.select(n)}`, `${key}.other`, `${key}.many`, `${key}.one`].find(k => k in d || k in DICTS.en) ?? `${key}.many`;
  return t(pick, { n, ...(params ?? {}) });
}
/** Formatting by the locale of the current
 *  language. */
export const fmt = {
  time: (d: Date | number) => new Date(d).toLocaleTimeString(lang.locale, { hour: '2-digit', minute: '2-digit' }),
  date: (d: Date | number, opts?: Intl.DateTimeFormatOptions) => new Date(d).toLocaleDateString(lang.locale, opts),
  /** Elapsed time (unix seconds): "just now", "5 min", "3 h", "2 d". */
  ago(ts?: number | null): string {
    if (!ts) return ''; const s = Math.max(0, Math.floor(Date.now() / 1000 - ts));
    return s < 60 ? t('time.now') : s < 3600 ? t('time.min', { n: Math.floor(s / 60) }) : s < 86400 ? t('time.h', { n: Math.floor(s / 3600) }) : t('time.d', { n: Math.floor(s / 86400) });
  },
};
export const locale = () => lang.locale;
