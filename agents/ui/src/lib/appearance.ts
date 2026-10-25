/** Appearance choices are independent of the global light/dark mode. */
export type ColorScheme = 'classic' | 'graphite' | 'nord' | 'dracula' | 'monokai' | 'solarized' | 'contrast' | 'matrix' | 'cyber';
export type UiFont = 'inter' | 'system' | 'humanist' | 'mono';
export type CodeFont = 'jetbrains' | 'system' | 'consolas';
export type TextTone = 'soft' | 'warm' | 'cool' | 'scheme' | 'custom';
export type TextWeight = 'normal' | 'medium';

type PreviewColors = { bg: string; panel: string; fg: string; accent: string };
type Scheme = {
  id: ColorScheme;
  label: string;
  hint: string;
  dark: PreviewColors;
  light: PreviewColors;
};

export const SCHEMES: readonly Scheme[] = [
  { id: 'classic', label: 'Classic', hint: 'look.scheme.classic.hint',
    dark: { bg: '#0a0c10', panel: '#141821', fg: '#e8ebf1', accent: '#6d9cff' },
    light: { bg: '#eef1f6', panel: '#ffffff', fg: '#151a24', accent: '#3d6fe0' } },
  { id: 'graphite', label: 'Graphite', hint: 'look.scheme.graphite.hint',
    dark: { bg: '#111315', panel: '#1d2024', fg: '#edf0f4', accent: '#96b4f4' },
    light: { bg: '#f0f1f3', panel: '#ffffff', fg: '#202329', accent: '#315caf' } },
  { id: 'nord', label: 'Nord', hint: 'look.scheme.nord.hint',
    dark: { bg: '#242933', panel: '#2e3440', fg: '#eceff4', accent: '#88c0d0' },
    light: { bg: '#e8eef2', panel: '#ffffff', fg: '#273344', accent: '#2b6176' } },
  { id: 'dracula', label: 'Dracula', hint: 'look.scheme.dracula.hint',
    dark: { bg: '#1d1e28', panel: '#282a36', fg: '#f8f8f2', accent: '#bd93f9' },
    light: { bg: '#f1edf8', panel: '#ffffff', fg: '#30263e', accent: '#7740a9' } },
  { id: 'monokai', label: 'Monokai', hint: 'look.scheme.monokai.hint',
    dark: { bg: '#1d1e19', panel: '#272a23', fg: '#f8f8ee', accent: '#a6e22e' },
    light: { bg: '#f1f0e8', panel: '#fffffb', fg: '#2c3024', accent: '#406014' } },
  { id: 'solarized', label: 'Solarized', hint: 'look.scheme.solarized.hint',
    dark: { bg: '#002b36', panel: '#083b46', fg: '#f3eedf', accent: '#8fc2d6' },
    light: { bg: '#eee8d5', panel: '#fff9e9', fg: '#263e45', accent: '#145e73' } },
  { id: 'contrast', label: 'Contrast', hint: 'look.scheme.contrast.hint',
    dark: { bg: '#000000', panel: '#111111', fg: '#ffffff', accent: '#9bc8ff' },
    light: { bg: '#ffffff', panel: '#ffffff', fg: '#000000', accent: '#004c99' } },
  { id: 'matrix', label: 'Matrix', hint: 'look.scheme.matrix.hint',
    dark: { bg: '#030b07', panel: '#0b1911', fg: '#b9d8c1', accent: '#55f28a' },
    light: { bg: '#e5f2e9', panel: '#f8fff9', fg: '#203f2a', accent: '#116c39' } },
  { id: 'cyber', label: 'Cyber', hint: 'look.scheme.cyber.hint',
    dark: { bg: '#0b0a18', panel: '#18162b', fg: '#d8cbe5', accent: '#f3db46' },
    light: { bg: '#eeecf9', panel: '#fdfbff', fg: '#302444', accent: '#774c00' } },
];

/** Swatch colors match the final CSS overrides in both brightness modes. */
export const ACCENTS = [
  { id: 'blue', key: 'look.accentBlue', dark: '#8fbbff', light: '#2950a6' },
  { id: 'violet', key: 'look.accentViolet', dark: '#bbaeff', light: '#523db6' },
  { id: 'green', key: 'look.accentGreen', dark: '#53dda0', light: '#10623a' },
  { id: 'orange', key: 'look.accentOrange', dark: '#ffbc73', light: '#854011' },
] as const;

/** Scheme and custom use these neutral colors only as a fallback preview. */
export const TEXT_TONES: readonly { id: TextTone; key: string; dark: string; light: string }[] = [
  { id: 'soft', key: 'look.text.soft', dark: '#c6ceda', light: '#243044' },
  { id: 'warm', key: 'look.text.warm', dark: '#d7cbb5', light: '#493c2b' },
  { id: 'cool', key: 'look.text.cool', dark: '#b6cadb', light: '#29485e' },
  { id: 'scheme', key: 'look.text.scheme', dark: '#e8ebf1', light: '#151a24' },
  { id: 'custom', key: 'look.text.custom', dark: '#c6ceda', light: '#243044' },
];

export const UI_FONTS: readonly { id: UiFont; label: string; stack: string }[] = [
  { id: 'inter', label: 'look.font.inter', stack: '"Inter", ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif' },
  { id: 'system', label: 'look.font.system', stack: 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif' },
  { id: 'humanist', label: 'look.font.humanist', stack: '"Trebuchet MS", "Segoe UI", "Gill Sans", sans-serif' },
  { id: 'mono', label: 'look.font.mono', stack: '"JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace' },
];

export const CODE_FONTS: readonly { id: CodeFont; label: string; stack: string }[] = [
  { id: 'jetbrains', label: 'look.codeFont.jetbrains', stack: '"JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace' },
  { id: 'system', label: 'look.codeFont.system', stack: 'ui-monospace, SFMono-Regular, Menlo, monospace' },
  { id: 'consolas', label: 'look.codeFont.consolas', stack: 'Consolas, "Liberation Mono", "Courier New", monospace' },
];

/** WCAG relative contrast for opaque six-digit hexadecimal colors. */
export function contrastRatio(first: string, second: string): number {
  const luminance = (color: string) => {
    if (!/^#[\da-f]{6}$/i.test(color)) throw new Error('Expected a six-digit hexadecimal color');
    const channels = [1, 3, 5].map(index => {
      const value = Number.parseInt(color.slice(index, index + 2), 16) / 255;
      return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
    });
    return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
  };
  const a = luminance(first), b = luminance(second);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

/**
 * Keep a requested text color whenever it is readable on every supplied surface.
 * Otherwise find the smallest RGB blend toward white (dark mode) or black (light).
 * Secondary text uses the available contrast budget toward the average surface.
 * `contrast` is the primary text's lowest contrast across the supplied surfaces.
 */
export function deriveTextColors(color: string, backgrounds: string[], mode: 'dark' | 'light'): {
  primary: string; secondary: string; muted: string; adjusted: boolean; contrast: number;
} {
  const normalize = (value: string) => {
    if (!/^#[\da-f]{6}$/i.test(value)) throw new Error('Expected a six-digit hexadecimal color');
    return value.toLowerCase();
  };
  const requested = normalize(color);
  if (!backgrounds.length) throw new Error('At least one background color is required');
  const surfaces = backgrounds.map(normalize);
  const channels = (hex: string) => [1, 3, 5].map(index => Number.parseInt(hex.slice(index, index + 2), 16));
  const hex = (rgb: number[]) => '#' + rgb.map(value => Math.round(value).toString(16).padStart(2, '0')).join('');
  const mix = (from: string, toward: string, amount: number) => {
    const first = channels(from), second = channels(toward);
    return hex(first.map((value, index) => value + (second[index] - value) * amount));
  };
  const minimum = (value: string) => Math.min(...surfaces.map(background => contrastRatio(value, background)));
  const threshold = 4.5;
  let primary = requested;
  if (minimum(primary) < threshold) {
    const endpoint = mode === 'dark' ? '#ffffff' : '#000000';
    if (minimum(endpoint) < threshold) throw new Error('Background colors do not support the selected text mode');
    let low = 0, high = 1;
    for (let step = 0; step < 24; step++) {
      const middle = (low + high) / 2;
      if (minimum(mix(requested, endpoint, middle)) >= threshold) high = middle;
      else low = middle;
    }
    primary = mix(requested, endpoint, high);
  }
  const backgroundChannels = surfaces.map(channels);
  const average = hex([0, 1, 2].map(index => backgroundChannels.reduce((sum, surface) => sum + surface[index], 0) / surfaces.length));
  const soften = (amount: number) => {
    const target = mix(primary, average, amount);
    if (minimum(target) >= threshold) return target;
    let low = 0, high = amount;
    for (let step = 0; step < 24; step++) {
      const middle = (low + high) / 2;
      if (minimum(mix(primary, average, middle)) >= threshold) low = middle;
      else high = middle;
    }
    return mix(primary, average, low);
  };
  return {
    primary,
    secondary: soften(0.18),
    muted: soften(0.36),
    adjusted: primary !== requested,
    contrast: minimum(primary),
  };
}
