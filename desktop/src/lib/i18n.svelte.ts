// Локализация Desktop (английский, русский, украинский, испанский). Словари — i18n/messages/*.ts: один файл — одно пространство имён,
// четыре языка, ключи ru/uk/es проверяются типом по en (i18n/define.ts). Текущий язык — реактивное состояние: t() в разметке
// пересчитывается при смене языка без перезагрузки. Источник языка — оформление (Appearance.language, общее для всех устройств
// пользователя, theme.ts → apply → i18n.set); 'auto' — по языку браузера, неизвестный язык → английский.
// Русские строки словаря — исходные тексты интерфейса, по ним же ищут элементы e2e-тесты (контексты Playwright с locale ru-RU).
// Тексты, которые приходят из ядра (события, ошибки API, состояния), пока не переводятся — это следующий шаг на стороне cloudd.
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
/** Названия языков на самих языках — не переводятся. */
export const LANG_NAMES: Record<Lang, string> = { en: 'English', ru: 'Русский', uk: 'Українська', es: 'Español' };

/** Язык браузера: первый из navigator.languages, для которого есть словарь; иначе английский. */
export function detectLang(): Lang {
  const list = typeof navigator === 'undefined' ? [] : (navigator.languages?.length ? navigator.languages : [navigator.language]);
  for (const l of list) { const p = (l || '').toLowerCase().slice(0, 2); if (p in DICTS) return p as Lang; }
  return 'en';
}

class I18n {
  setting = $state<LangSetting>('auto');
  lang = $state<Lang>(detectLang());
  /** BCP 47 для Intl (даты, числа, множественное число). */
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

/** Строка по ключу на текущем языке с подстановкой {name}; нет перевода — английский; нет ключа — сам ключ (и предупреждение в dev). */
export function t(key: string, params?: Record<string, string | number>): string {
  const s = DICTS[i18n.lang][key] ?? DICTS.en[key];
  if (s === undefined) { if (import.meta.env.DEV) console.warn(`i18n: нет ключа ${key}`); return key; }
  return fill(s, params);
}

const rules = new Map<string, Intl.PluralRules>();
/** Множественное число: ключ `<key>.<one|few|many|other>` по Intl.PluralRules текущей локали, {n} подставляется сам. */
export function tn(key: string, n: number, params?: Record<string, string | number>): string {
  const loc = i18n.locale;
  let pr = rules.get(loc);
  if (!pr) { pr = new Intl.PluralRules(loc); rules.set(loc, pr); }
  const k = `${key}.${pr.select(n)}`;
  return t(k in DICTS[i18n.lang] || k in DICTS.en ? k : `${key}.other`, { n, ...params });
}

type DateLike = Date | string | number;
/** Форматирование по локали текущего языка — вместо toLocale*() без аргументов (те берут язык браузера, а не Desktop). */
export const fmt = {
  date: (d: DateLike, opts?: Intl.DateTimeFormatOptions) => new Date(d).toLocaleDateString(i18n.locale, opts),
  time: (d: DateLike, opts?: Intl.DateTimeFormatOptions) => new Date(d).toLocaleTimeString(i18n.locale, opts ?? { hour: '2-digit', minute: '2-digit' }),
  dateTime: (d: DateLike, opts?: Intl.DateTimeFormatOptions) => new Date(d).toLocaleString(i18n.locale, opts),
  number: (n: number, opts?: Intl.NumberFormatOptions) => n.toLocaleString(i18n.locale, opts),
  /** Размер: 1024-кратные единицы с локализованным суффиксом (B/KB/MB/GB/TB → Б/КБ/МБ/ГБ/ТБ). */
  bytes(n: number, digits = 1): string {
    if (!Number.isFinite(n)) return '';
    if (n < 1024) return `${Math.round(n)} ${t('unit.B')}`;
    const units = ['unit.KB', 'unit.MB', 'unit.GB', 'unit.TB'];
    let v = n / 1024, i = 0;
    while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
    return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(digits)} ${t(units[i])}`;
  },
  /** Сколько прошло — коротко: «только что», «5 мин», «3 ч», «2 д». */
  ago(iso?: string | null): string {
    if (!iso) return '';
    const d = (Date.now() - new Date(iso).getTime()) / 1000;
    if (d < 60) return t('time.now');
    if (d < 3600) return t('time.min', { n: Math.floor(d / 60) });
    if (d < 86400) return t('time.h', { n: Math.floor(d / 3600) });
    return t('time.d', { n: Math.floor(d / 86400) });
  },
  /** Длительность в секундах: «42 с» или «3 мин 5 с». */
  duration(secs: number): string {
    const s = Math.max(0, Math.round(secs));
    return s < 60 ? t('time.sec', { n: s }) : t('time.minSec', { m: Math.floor(s / 60), s: s % 60 });
  },
};
