// Current language of the "Agents" app — reactive state (like Desktop i18n): t() in markup is recomputed on a language
// change without a reload. Source: the "language" setting in the appearance (localStorage agents.lang; auto — the Desktop
// language from the core if set explicitly, otherwise the browser language; an unknown language → English).
import { LANGS, type Lang, type LangSetting } from './i18n/define';
export { LANGS, type Lang, type LangSetting };
export const LOCALES: Record<Lang, string> = { en: 'en-US', ru: 'ru-RU', uk: 'uk-UA', es: 'es-ES' };
/** Language names in the languages themselves — not
 *  translated. */
export const LANG_NAMES: Record<Lang, string> = { en: 'English', ru: 'Русский', uk: 'Українська', es: 'Español' };
const ls = (k: string) => { try { return localStorage.getItem(k); } catch { return null; } };
const isLang = (s: unknown): s is Lang => typeof s === 'string' && (LANGS as readonly string[]).includes(s);

/** Browser language: the first of navigator.languages that has a dictionary; otherwise English. */
export function detectLang(): Lang {
  const list = typeof navigator === 'undefined' ? [] : (navigator.languages?.length ? navigator.languages : [navigator.language]);
  for (const l of list) { const p = (l || '').toLowerCase().slice(0, 2); if (isLang(p)) return p; }
  return 'en';
}

class LangState {
  setting = $state<LangSetting>((ls('agents.lang') as LangSetting) || 'auto');
  /** Desktop language from the core (appearance → language) if set explicitly; picked up at
   *  start. */
  desktop = $state<Lang | null>(null);
  lang = $state<Lang>(detectLang());
  get locale() { return LOCALES[this.lang]; }
  constructor() { this.recompute(); }
  recompute() {
    const s = this.setting;
    const lang: Lang = isLang(s) ? s : (this.desktop ?? detectLang());
    if (this.lang !== lang) this.lang = lang;
    if (typeof document !== 'undefined' && document.documentElement.lang !== lang) document.documentElement.lang = lang;
  }
  set(s: LangSetting) {
    this.setting = isLang(s) ? s : 'auto';
    try { localStorage.setItem('agents.lang', this.setting); } catch {}
    this.recompute();
  }
  setDesktop(l: unknown) { this.desktop = isLang(l) ? l : null; this.recompute(); }
}
export const lang = new LangState();
