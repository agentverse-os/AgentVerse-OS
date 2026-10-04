// Строки приложения «Агенты» на четырёх языках (английский, русский, украинский, испанский). Словари — i18n/messages/*.ts: один
// файл — одно пространство имён, четыре языка, ключи проверяются типом по английскому (i18n/define.ts). Русский — исходный
// язык интерфейса (по нему же ищут элементы e2e-проверки). Текущий язык — реактивное состояние lang.svelte.ts: t() в разметке
// пересчитывается при смене языка без перезагрузки. Тексты из ядра и сервиса (ошибки API, события) не переводятся.
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
/** Строка по ключу на текущем языке с подстановкой {name}; нет перевода — английский; нет ключа — сам ключ. */
export function t(key: string, params?: Record<string, unknown>): string {
  const s = DICTS[lang.lang][key] ?? DICTS.en[key];
  if (s === undefined) { if (import.meta.env.DEV) console.warn(`i18n: нет ключа ${key}`); return fmtStr(key, params); }
  return fmtStr(s, params);
}
const rules = new Map<string, Intl.PluralRules>();
/** Множественное число: ключ `<key>.<one|few|many|other>` по Intl.PluralRules текущей локали, {n} подставляется сам. */
export function tn(key: string, n: number, params?: Record<string, unknown>): string {
  const loc = lang.locale;
  let pr = rules.get(loc);
  if (!pr) { pr = new Intl.PluralRules(loc); rules.set(loc, pr); }
  const d = DICTS[lang.lang];
  const pick = [`${key}.${pr.select(n)}`, `${key}.other`, `${key}.many`, `${key}.one`].find(k => k in d || k in DICTS.en) ?? `${key}.many`;
  return t(pick, { n, ...(params ?? {}) });
}
/** Форматирование по локали текущего языка. */
export const fmt = {
  time: (d: Date | number) => new Date(d).toLocaleTimeString(lang.locale, { hour: '2-digit', minute: '2-digit' }),
  date: (d: Date | number, opts?: Intl.DateTimeFormatOptions) => new Date(d).toLocaleDateString(lang.locale, opts),
  /** Сколько прошло (секунды unix): «только что», «5 мин», «3 ч», «2 дн». */
  ago(ts?: number | null): string {
    if (!ts) return ''; const s = Math.max(0, Math.floor(Date.now() / 1000 - ts));
    return s < 60 ? t('time.now') : s < 3600 ? t('time.min', { n: Math.floor(s / 60) }) : s < 86400 ? t('time.h', { n: Math.floor(s / 3600) }) : t('time.d', { n: Math.floor(s / 86400) });
  },
};
export const locale = () => lang.locale;
