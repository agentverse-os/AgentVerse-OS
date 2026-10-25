// Appearance of the whole web app; stored in the browser independently of
// Desktop.
import { SCHEMES, UI_FONTS, CODE_FONTS, TEXT_TONES, deriveTextColors, type ColorScheme, type UiFont, type CodeFont, type TextTone, type TextWeight } from './appearance';
import { lang, type LangSetting } from './lang.svelte';

export type ThemePref = 'dark' | 'light' | 'system';
export type Accent = 'scheme' | 'blue' | 'violet' | 'green' | 'orange';
export type Density = 'normal' | 'compact';
type AppearanceValues = {
  theme: ThemePref; scheme: ColorScheme; accent: Accent; density: Density;
  uiFont: UiFont; codeFont: CodeFont;
  textTone: TextTone; textDark: string; textLight: string; textWeight: TextWeight;
};
type PreferenceKey = keyof AppearanceValues;
const DEFAULTS: AppearanceValues = {
  theme: 'dark', scheme: 'classic', accent: 'scheme', density: 'normal', uiFont: 'inter', codeFont: 'jetbrains',
  textTone: 'soft', textDark: '#c6ceda', textLight: '#243044', textWeight: 'normal',
};
const OPTIONS: { [K in Exclude<PreferenceKey, 'textDark' | 'textLight'>]: readonly AppearanceValues[K][] } = {
  theme: ['dark', 'light', 'system'], scheme: SCHEMES.map(s => s.id),
  accent: ['scheme', 'blue', 'violet', 'green', 'orange'], density: ['normal', 'compact'],
  uiFont: UI_FONTS.map(f => f.id), codeFont: CODE_FONTS.map(f => f.id),
  textTone: TEXT_TONES.map(tone => tone.id), textWeight: ['normal', 'medium'],
};
const keys = Object.keys(DEFAULTS) as PreferenceKey[];
const ls = (key: string) => { try { return localStorage.getItem(key); } catch { return null; } };
function valid<K extends PreferenceKey>(key: K, value: unknown): AppearanceValues[K] {
  if (key === 'textDark' || key === 'textLight') {
    return (typeof value === 'string' && /^#[\da-f]{6}$/i.test(value.trim()) ? value.trim().toLowerCase() : DEFAULTS[key]) as AppearanceValues[K];
  }
  return OPTIONS[key as keyof typeof OPTIONS].some(option => option === value) ? value as AppearanceValues[K] : DEFAULTS[key];
}
function load(): AppearanceValues {
  return Object.fromEntries(keys.map(key => [key, valid(key, ls('agents.' + key))])) as AppearanceValues;
}
function save(values: Partial<AppearanceValues>) {
  try { for (const [key, value] of Object.entries(values)) localStorage.setItem('agents.' + key, value); } catch {}
}
const initial = load();
const sysDark = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: dark)') : null;

// Include tinted message/selection surfaces as well as the five base surfaces.
function textBackgrounds(css: CSSStyleDeclaration, mode: 'dark' | 'light'): string[] {
  const fallback = mode === 'dark' ? '#1a1f2b' : '#ffffff';
  const base = [0, 1, 2, 3, 4].map(index => {
    const value = css.getPropertyValue('--bg-' + index).trim();
    return /^#[\da-f]{6}$/i.test(value) ? value : fallback;
  });
  const overlay = css.getPropertyValue('--accent-soft').trim().match(/^rgba?\(([^)]+)\)$/);
  if (!overlay) return base;
  const values = overlay[1].replace(/[,/]/g, ' ').trim().split(/\s+/).map(Number);
  if (values.length !== 4 || values.some(value => !Number.isFinite(value))) return base;
  const alpha = Math.max(0, Math.min(1, values[3]));
  return [...base, ...base.map(background => '#' + [1, 3, 5].map((position, index) => {
    const channel = Number.parseInt(background.slice(position, position + 2), 16);
    return Math.round(channel * (1 - alpha) + Math.max(0, Math.min(255, values[index])) * alpha).toString(16).padStart(2, '0');
  }).join(''))];
}

class Prefs {
  theme = $state<ThemePref>(initial.theme);
  scheme = $state<ColorScheme>(initial.scheme);
  accent = $state<Accent>(initial.accent);
  density = $state<Density>(initial.density);
  uiFont = $state<UiFont>(initial.uiFont);
  codeFont = $state<CodeFont>(initial.codeFont);
  textTone = $state<TextTone>(initial.textTone);
  textDark = $state(initial.textDark);
  textLight = $state(initial.textLight);
  textWeight = $state<TextWeight>(initial.textWeight);
  private textColors = $state({ primary: DEFAULTS.textDark, secondary: DEFAULTS.textDark, muted: DEFAULTS.textDark, contrast: 4.5, adjusted: false });
  private systemDark = $state(sysDark?.matches ?? false);
  get lang(): LangSetting { return lang.setting; }
  setLang(value: LangSetting) { lang.set(value); }
  get effective(): 'dark' | 'light' { return this.theme === 'system' ? (this.systemDark ? 'dark' : 'light') : this.theme; }
  get appliedTextColor() { return this.textColors.primary; }
  get textContrast() { return this.textColors.contrast; }
  get textAdjusted() { return this.textColors.adjusted; }
  apply() {
    const root = document.documentElement;
    root.dataset.theme = this.effective; root.style.colorScheme = this.effective;
    root.dataset.scheme = this.scheme; root.dataset.accent = this.accent; root.dataset.density = this.density;
    root.dataset.uiFont = this.uiFont; root.dataset.codeFont = this.codeFont;
    root.dataset.textTone = this.textTone; root.dataset.textWeight = this.textWeight;
    for (const token of ['--fg-1', '--fg-2', '--fg-3']) root.style.removeProperty(token);
    const css = getComputedStyle(root);
    const tone = TEXT_TONES.find(tone => tone.id === this.textTone) ?? TEXT_TONES[0];
    const requested = this.textTone === 'custom' ? (this.effective === 'dark' ? this.textDark : this.textLight)
      : this.textTone === 'scheme' ? css.getPropertyValue('--fg-1').trim() : tone[this.effective];
    this.textColors = deriveTextColors(requested, textBackgrounds(css, this.effective), this.effective);
    root.style.setProperty('--fg-1', this.textColors.primary);
    root.style.setProperty('--fg-2', this.textColors.secondary);
    root.style.setProperty('--fg-3', this.textColors.muted);
    root.style.setProperty('--text-weight', this.textWeight === 'medium' ? '500' : '400');
    root.style.setProperty('--strong-weight', this.textWeight === 'medium' ? '700' : '600');
    const background = css.getPropertyValue('--bg-0').trim();
    if (background) document.querySelector('meta[name="theme-color"]')?.setAttribute('content', background);
  }
  set<K extends PreferenceKey>(key: K, value: AppearanceValues[K]) {
    const next: Partial<AppearanceValues> = { [key]: valid(key, value) };
    // Choosing a color preset applies its accent; the mode and fonts stay as chosen.
    if (key === 'scheme') next.accent = 'scheme';
    Object.assign(this, next); save(next); this.apply();
  }
  resetAppearance() { Object.assign(this, DEFAULTS); save(DEFAULTS); this.apply(); }
  reload() { Object.assign(this, load()); this.apply(); }
  systemChanged(dark: boolean) { this.systemDark = dark; if (this.theme === 'system') this.apply(); }
  toggleTheme() { this.set('theme', this.effective === 'dark' ? 'light' : 'dark'); }
}
export const prefs = new Prefs();
sysDark?.addEventListener('change', event => prefs.systemChanged(event.matches));
if (typeof window !== 'undefined') window.addEventListener('storage', event => {
  if (event.key === null || keys.some(key => event.key === 'agents.' + key)) prefs.reload();
});
