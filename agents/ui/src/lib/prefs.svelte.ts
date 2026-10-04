// Предпочтения оформления: тема (тёмная / светлая / как система), акцент, плотность. Живут в браузере (localStorage),
// применяются атрибутами на <html> — тема приложения не зависит от темы рабочего стола.
export type ThemePref = 'dark' | 'light' | 'system';
export type Accent = 'blue' | 'violet' | 'green' | 'orange';
export type Density = 'normal' | 'compact';
import { lang, type LangSetting } from './lang.svelte';

const ls = (k: string) => { try { return localStorage.getItem(k); } catch { return null; } };
const sysDark = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: dark)') : null;

class Prefs {
  theme = $state<ThemePref>((ls('agents.theme') as ThemePref) || 'dark');
  accent = $state<Accent>((ls('agents.accent') as Accent) || 'blue');
  density = $state<Density>((ls('agents.density') as Density) || 'normal');
  /** Язык интерфейса: auto — язык Desktop (если задан явно в его оформлении), иначе язык браузера. */
  get lang(): LangSetting { return lang.setting; }
  setLang(v: LangSetting) { lang.set(v); }
  /** Эффективная тема с учётом «как система». */
  get effective(): 'dark' | 'light' { return this.theme === 'system' ? (sysDark?.matches ? 'dark' : 'light') : this.theme; }
  apply() {
    const r = document.documentElement; const t = this.effective;
    r.dataset.theme = t; r.style.colorScheme = t; r.dataset.accent = this.accent; r.dataset.density = this.density;
  }
  set<K extends 'theme' | 'accent' | 'density'>(k: K, v: this[K]) {
    (this as any)[k] = v;
    try { localStorage.setItem('agents.' + k, String(v)); } catch {}
    this.apply();
  }
  toggleTheme() { this.set('theme', this.effective === 'dark' ? 'light' : 'dark'); }
}
export const prefs = new Prefs();
sysDark?.addEventListener('change', () => { if (prefs.theme === 'system') prefs.apply(); });
