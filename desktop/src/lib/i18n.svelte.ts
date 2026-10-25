// Desktop localization (English, Russian, Ukrainian, Spanish). Dictionaries — i18n/messages/*.ts: one file is one namespace, four
// languages, the ru/uk/es keys are type-checked against en (i18n/define.ts). The current language is reactive state: t() in markup is
// recomputed on a language change without a reload. The language source is the appearance (Appearance.language, shared across all of
// the user's devices, theme.ts → apply → i18n.set); 'auto' — by the browser language, an unknown language → English. The Russian
// dictionary strings are the source texts of the UI; the e2e tests look up elements by them too (Playwright contexts with locale
// ru-RU). Texts that come from the core (events, API errors, states) are not translated yet — that is the next step on the cloudd
// side.
import { LANGS, type Lang, type LangSetting, type Messages } from './i18n/define';
export { LANGS, type Lang, type LangSetting };

type Dict = Record<string, string>;
const modules = import.meta.glob<{ default: Messages<Dict> }>('./i18n/messages/*.ts', { eager: true });
const DICTS: Record<Lang, Dict> = { en: {}, ru: {}, uk: {}, es: {} };
for (const [file, m] of Object.entries(modules)) {
  for (const l of LANGS) {
    if (import.meta.env.DEV) for (const k of Object.keys(m.default[l])) if (k in DICTS[l]) console.warn(`i18n: ключ ${k} задан повторно (${file})`);
    Object.assign(DICTS[l], m.default[l]);
  }
}

export const LOCALES: Record<Lang, string> = { en: 'en-US', ru: 'ru-RU', uk: 'uk-UA', es: 'es-ES' };
/** Language names in the languages themselves — not
 *  translated. */
export const LANG_NAMES: Record<Lang, string> = { en: 'English', ru: 'Русский', uk: 'Українська', es: 'Español' };

/** Browser language: the first of navigator.languages that has a dictionary; otherwise English. */
export function detectLang(): Lang {
  const list = typeof navigator === 'undefined' ? [] : (navigator.languages?.length ? navigator.languages : [navigator.language]);
  for (const l of list) { const p = (l || '').toLowerCase().slice(0, 2); if (p in DICTS) return p as Lang; }
  return 'en';
}

class I18n {
  setting = $state<LangSetting>('auto');
  lang = $state<Lang>(detectLang());
  /** BCP 47 for Intl (dates, numbers, plurals). */
  get locale() { return LOCALES[this.lang]; }
  set(s: LangSetting) {
    const lang: Lang = s === 'auto' || !(s in DICTS) ? detectLang() : s;
    if (this.setting !== s) this.setting = s;
    if (this.lang !== lang) this.lang = lang;
    if (typeof document !== 'undefined' && document.documentElement.lang !== lang) document.documentElement.lang = lang;
  }
}
export const i18n = new I18n();

function fill(s: string, p?: Record<string, string | number>) { return p ? s.replace(/\{(\w+)\}/g, (m, k) => (k in p ? String(p[k]) : m)) : s; }

/** String by key in the current language with {name} substitution; no translation — English; no key — the key itself (and a warning in
 *  dev). */
export function t(key: string, params?: Record<string, string | number>): string {
  const s = DICTS[i18n.lang][key] ?? DICTS.en[key];
  if (s === undefined) { if (import.meta.env.DEV) console.warn(`i18n: нет ключа ${key}`); return key; }
  return fill(s, params);
}

const rules = new Map<string, Intl.PluralRules>();
/** Plural: the key `<key>.<one|few|many|other>` per Intl.PluralRules of the current locale, {n} is substituted
 *  automatically. */
export function tn(key: string, n: number, params?: Record<string, string | number>): string {
  const loc = i18n.locale;
  let pr = rules.get(loc);
  if (!pr) { pr = new Intl.PluralRules(loc); rules.set(loc, pr); }
  const k = `${key}.${pr.select(n)}`;
  return t(k in DICTS[i18n.lang] || k in DICTS.en ? k : `${key}.other`, { n, ...params });
}

type DateLike = Date | string | number;
/** Formatting by the locale of the current language — instead of toLocale*() without arguments (those take the browser
 *  language, not Desktop's). */
export const fmt = {
  date: (d: DateLike, opts?: Intl.DateTimeFormatOptions) => new Date(d).toLocaleDateString(i18n.locale, opts),
  time: (d: DateLike, opts?: Intl.DateTimeFormatOptions) => new Date(d).toLocaleTimeString(i18n.locale, opts ?? { hour: '2-digit', minute: '2-digit' }),
  dateTime: (d: DateLike, opts?: Intl.DateTimeFormatOptions) => new Date(d).toLocaleString(i18n.locale, opts),
  number: (n: number, opts?: Intl.NumberFormatOptions) => n.toLocaleString(i18n.locale, opts),
  /** Size: 1024-based units with a localized suffix (B/KB/MB/GB/TB → Б/КБ/МБ/ГБ/ТБ). */
  bytes(n: number, digits = 1): string {
    if (!Number.isFinite(n)) return '';
    if (n < 1024) return `${Math.round(n)} ${t('unit.B')}`;
    const units = ['unit.KB', 'unit.MB', 'unit.GB', 'unit.TB'];
    let v = n / 1024, i = 0;
    while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
    return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(digits)} ${t(units[i])}`;
  },
  /** Elapsed time, short: "just now", "5 min", "3 h", "2 d". */
  ago(iso?: string | null): string {
    if (!iso) return '';
    const d = (Date.now() - new Date(iso).getTime()) / 1000;
    if (d < 60) return t('time.now');
    if (d < 3600) return t('time.min', { n: Math.floor(d / 60) });
    if (d < 86400) return t('time.h', { n: Math.floor(d / 3600) });
    return t('time.d', { n: Math.floor(d / 86400) });
  },
  /** Duration in seconds: "42 s" or "3 min 5 s". */
  duration(secs: number): string {
    const s = Math.max(0, Math.round(secs));
    return s < 60 ? t('time.sec', { n: s }) : t('time.minSec', { m: Math.floor(s / 60), s: s % 60 });
  },
};
