// Оформление Desktop: как в настольной ОС — тема, акцент, фон, масштаб, плотность, скругления, шрифт, контраст, анимации.
// Источник истины — ядро (/api/settings/desktop), копия в localStorage для мгновенного применения до ответа сети.
import { api } from './api';
import { i18n, type LangSetting } from './i18n.svelte';

export type ThemeMode = 'dark' | 'light' | 'system';
export interface Appearance {
  mode: ThemeMode;
  accent: string;              // hex
  background: string;          // id пресета фона или 'none' | 'custom'
  backgroundUrl: string;       // для 'custom'
  backgroundBlur: number;      // 0..20 px
  textScale: number;           // 0.85..1.3
  density: 'compact' | 'normal' | 'comfortable';
  radius: 'sharp' | 'normal' | 'round';
  font: 'system' | 'humanist' | 'grotesk' | 'mono';
  contrast: 'normal' | 'high';
  reduceMotion: boolean;
  transparency: boolean;       // полупрозрачные панели поверх фона
  iconSize: 'small' | 'medium' | 'large';
  showWorkspaceIps: boolean;
  /** Закреплённые системные приложения на панели задач; по умолчанию панель показывает только открытые окна. */
  taskbarPinned?: boolean;
  /** Заставка по бездействию, минуты; 0 — выключена. */
  screensaver?: number;
  /** Поверхности: flat — плоские панели, glass — стекло (сильное размытие, блики, светлые кромки), oled — чистый чёрный. */
  surface?: 'flat' | 'glass' | 'oled';
  /** Язык интерфейса Desktop (общий для всех устройств); auto — по языку браузера. */
  language?: LangSetting;
}

export const DEFAULTS: Appearance = {
  mode: 'dark', accent: '#5BBFCB', background: 'nebula', backgroundUrl: '', backgroundBlur: 0, textScale: 1, density: 'normal',
  radius: 'normal', font: 'system', contrast: 'normal', reduceMotion: false, transparency: true, iconSize: 'medium', showWorkspaceIps: true, taskbarPinned: false, screensaver: 10, surface: 'flat', language: 'auto',
};

export const ACCENTS = ['#5BBFCB', '#4F8EF7', '#7C6CF6', '#D65DB1', '#EB5757', '#F2994A', '#F2C94C', '#6FCF97', '#2FB39B', '#9AA3A8'];
type Layers = { dark: string; light: string; top?: { dark: string; light: string } };
/** top — второй слой поверх, ползунок размытия трогает его впятеро слабее (знак остаётся резким); portrait — те же слои для вертикального экрана (телефон):
 *  альбомная картинка в режиме cover на телефоне обрезается до средней трети, и знак с надписью в кадр не попадают. */
/** label — ключ словаря (i18n/messages/theme.ts): показывать через t(b.label). */
export const BACKGROUNDS: ({ id: string; label: string; portrait?: Layers } & Layers)[] = [
  { id: 'none', label: 'theme.bg.none', dark: 'none', light: 'none' },
  { id: 'black', label: 'theme.bg.black', dark: '#000000', light: '#ffffff' },
  { id: 'monolith', label: 'theme.bg.monolith', dark: "url('/wallpaper-monolith-bg.webp') center / cover no-repeat #0B0D14", light: "url('/wallpaper-monolith-light-bg.webp') center / cover no-repeat #F4F7FF", top: { dark: "url('/wallpaper-monolith-mark.webp') center / cover no-repeat", light: "url('/wallpaper-monolith-mark.webp') center / cover no-repeat" },
    portrait: { dark: "url('/wallpaper-monolith-bg-portrait.webp') center / cover no-repeat #0B0D14", light: "url('/wallpaper-monolith-light-bg-portrait.webp') center / cover no-repeat #F4F7FF", top: { dark: "url('/wallpaper-monolith-mark-portrait.webp') center / cover no-repeat", light: "url('/wallpaper-monolith-mark-portrait.webp') center / cover no-repeat" } } },
  { id: 'monolith-lockup', label: 'theme.bg.monolithLockup', dark: "url('/wallpaper-monolith-lockup-bg.webp') center / cover no-repeat #0B0D14", light: "url('/wallpaper-monolith-lockup-light-bg.webp') center / cover no-repeat #F4F7FF", top: { dark: "url('/wallpaper-monolith-lockup.webp') center / cover no-repeat", light: "url('/wallpaper-monolith-lockup-light.webp') center / cover no-repeat" },
    portrait: { dark: "url('/wallpaper-monolith-lockup-bg-portrait.webp') center / cover no-repeat #0B0D14", light: "url('/wallpaper-monolith-lockup-light-bg-portrait.webp') center / cover no-repeat #F4F7FF", top: { dark: "url('/wallpaper-monolith-lockup-portrait.webp') center / cover no-repeat", light: "url('/wallpaper-monolith-lockup-light-portrait.webp') center / cover no-repeat" } } },
  { id: 'monolith-quiet', label: 'theme.bg.monolithQuiet', dark: "url('/wallpaper-monolith-quiet.webp') center / cover no-repeat #0B0D14", light: "url('/wallpaper-monolith-light-bg.webp') center / cover no-repeat #F4F7FF" },
  { id: 'nebula', label: 'theme.bg.nebula', dark: 'radial-gradient(1200px 800px at 15% 10%, #1b2a33 0%, transparent 60%), radial-gradient(900px 600px at 85% 90%, #23202f 0%, transparent 55%), #121416', light: 'radial-gradient(1200px 800px at 15% 10%, #dbeef2 0%, transparent 60%), radial-gradient(900px 600px at 85% 90%, #ece8f7 0%, transparent 55%), #f4f6f7' },
  { id: 'dusk', label: 'theme.bg.dusk', dark: 'linear-gradient(160deg, #141a22 0%, #1c1630 60%, #0f1315 100%)', light: 'linear-gradient(160deg, #eef3f8 0%, #f3edf9 60%, #ffffff 100%)' },
  { id: 'forest', label: 'theme.bg.forest', dark: 'linear-gradient(180deg, #0f1a16 0%, #121416 70%)', light: 'linear-gradient(180deg, #e9f4ee 0%, #f6f8f7 70%)' },
  { id: 'graphite', label: 'theme.bg.graphite', dark: 'linear-gradient(135deg, #1a1d21, #0e1012)', light: 'linear-gradient(135deg, #f7f7f8, #e9ebee)' },
  { id: 'custom', label: 'theme.bg.custom', dark: 'none', light: 'none' },
];
const FONTS: Record<Appearance['font'], string> = {
  system: 'system-ui, -apple-system, "Segoe UI", Roboto, sans-serif',
  humanist: '"Segoe UI", "Trebuchet MS", Verdana, system-ui, sans-serif',
  grotesk: 'Inter, "Helvetica Neue", Arial, system-ui, sans-serif',
  mono: 'ui-monospace, "JetBrains Mono", Menlo, Consolas, monospace',
};
const KEY = 'cloudos.appearance.v1';

export function load(): Appearance {
  try { return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(KEY) ?? '{}') }; } catch { return { ...DEFAULTS }; }
}
export function saveLocal(a: Appearance) { try { localStorage.setItem(KEY, JSON.stringify(a)); } catch {} }
export async function saveRemote(a: Appearance) { try { await api.putDesktopSettings({ appearance: a }); } catch {} }
export async function loadRemote(): Promise<Appearance | null> {
  try { const s = await api.desktopSettings(); return s?.appearance ? { ...DEFAULTS, ...s.appearance } : null; } catch { return null; }
}

function hexToRgb(hex: string): [number, number, number] {
  const h = hex.replace('#', ''); const n = parseInt(h.length === 3 ? h.split('').map(c => c + c).join('') : h, 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}
/** Контраст текста на акценте: тёмный текст на светлом акценте и наоборот. */
function onAccent(hex: string) { const [r, g, b] = hexToRgb(hex); return (0.299 * r + 0.587 * g + 0.114 * b) > 150 ? '#0b1416' : '#ffffff'; }

const PORTRAIT = '(orientation: portrait)';
function isPortrait() { return typeof matchMedia === 'function' && matchMedia(PORTRAIT).matches; }

export function effectiveMode(a: Appearance): 'dark' | 'light' {
  if (a.mode !== 'system') return a.mode;
  return matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}

/** Применить оформление к документу: атрибуты и CSS-переменные. */
export function apply(a: Appearance) {
  const root = document.documentElement;
  i18n.set(a.language ?? 'auto');
  const mode = effectiveMode(a);
  root.dataset.theme = mode;
  root.dataset.density = a.density;
  root.dataset.radius = a.radius;
  root.dataset.contrast = a.contrast;
  root.dataset.motion = a.reduceMotion ? 'reduce' : 'normal';
  root.dataset.glass = a.transparency ? 'on' : 'off';
  root.dataset.icons = a.iconSize;
  root.dataset.surface = a.surface ?? 'flat';
  root.style.setProperty('--accent', a.accent);
  root.style.setProperty('--on-accent', onAccent(a.accent));
  const [r, g, b] = hexToRgb(a.accent);
  root.style.setProperty('--accent-rgb', `${r} ${g} ${b}`);
  root.style.setProperty('--scale', String(a.textScale));
  root.style.setProperty('--font', FONTS[a.font]);
  const preset = BACKGROUNDS.find(b => b.id === a.background) ?? BACKGROUNDS.find(b => b.id === DEFAULTS.background) ?? BACKGROUNDS[0];
  const bg: Layers = isPortrait() && preset.portrait ? preset.portrait : preset;
  const wall = a.background === 'custom' && a.backgroundUrl ? `url("${a.backgroundUrl.replace(/"/g, '')}") center / cover no-repeat fixed` : bg[mode];
  root.style.setProperty('--wallpaper', wall);
  root.style.setProperty('--wallpaper-blur', `${a.backgroundBlur}px`);
  root.style.setProperty('--wallpaper-top', a.background !== 'custom' && bg.top ? bg.top[mode] : 'none');
  root.style.colorScheme = mode;
  document.querySelector('meta[name=theme-color]')?.setAttribute('content', a.surface === 'oled' ? '#000000' : mode === 'dark' ? '#121416' : '#f4f6f7');
}

/** Следить за системной темой (режим system) и поворотом экрана (портретные слои обоев). */
export function watchSystem(get: () => Appearance) {
  const mq = matchMedia('(prefers-color-scheme: light)');
  const h = () => { if (get().mode === 'system') apply(get()); };
  mq.addEventListener('change', h);
  const or = matchMedia(PORTRAIT);
  const ho = () => apply(get());
  or.addEventListener('change', ho);
  return () => { mq.removeEventListener('change', h); or.removeEventListener('change', ho); };
}

/** Готовые темы: набор настроек поверх текущего оформления. После выбора всё ниже можно крутить дальше. label и hint — ключи словаря. */
export const THEMES: { id: string; label: string; hint: string; set: Partial<Appearance> }[] = [
  { id: 'monolith', label: 'theme.t.monolith', hint: 'theme.t.monolith.hint', set: { mode: 'dark', accent: '#5B8CFF', background: 'monolith', backgroundBlur: 12, surface: 'glass', transparency: true, radius: 'round', contrast: 'normal' } },
  { id: 'glass-dark', label: 'theme.t.glassDark', hint: 'theme.t.glassDark.hint', set: { mode: 'dark', accent: '#7C6CF6', background: 'monolith-quiet', backgroundBlur: 8, surface: 'glass', transparency: true, radius: 'round', contrast: 'normal' } },
  { id: 'glass-light', label: 'theme.t.glassLight', hint: 'theme.t.glassLight.hint', set: { mode: 'light', accent: '#4F8EF7', background: 'monolith', backgroundBlur: 10, surface: 'glass', transparency: true, radius: 'round', contrast: 'normal' } },
  { id: 'classic', label: 'theme.t.classic', hint: 'theme.t.classic.hint', set: { mode: 'dark', accent: '#5BBFCB', background: 'nebula', backgroundBlur: 0, surface: 'flat', transparency: true, radius: 'normal', contrast: 'normal' } },
  { id: 'oled', label: 'theme.t.oled', hint: 'theme.t.oled.hint', set: { mode: 'dark', accent: '#A2B4FF', background: 'black', backgroundBlur: 0, surface: 'oled', transparency: false, radius: 'normal', contrast: 'normal' } },
  { id: 'contrast-dark', label: 'theme.t.contrastDark', hint: 'theme.t.contrastDark.hint', set: { mode: 'dark', accent: '#F2C94C', background: 'black', backgroundBlur: 0, surface: 'flat', transparency: false, radius: 'sharp', contrast: 'high', textScale: 1.1 } },
  { id: 'contrast-light', label: 'theme.t.contrastLight', hint: 'theme.t.contrastLight.hint', set: { mode: 'light', accent: '#1F3FB5', background: 'black', backgroundBlur: 0, surface: 'flat', transparency: false, radius: 'sharp', contrast: 'high', textScale: 1.1 } },
];
export function themeMatches(a: Appearance, t: (typeof THEMES)[number]) {
  return Object.entries(t.set).every(([k, v]) => (a as unknown as Record<string, unknown>)[k] === v);
}
