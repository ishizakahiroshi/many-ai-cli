// Custom Hub themes are hue + contrast on top of shipped Light/Dark.
// Accent, provider colors, and warning colors stay on the 土台 (light/dark).

export type ThemeMode = 'light' | 'dark';

export type SurfaceTokens = {
  bg: string;
  bgElev: string;
  bgElev2: string;
  bgDeep: string;
  text: string;
  textSoft: string;
  textDim: string;
  muted: string;
  muted2: string;
  line: string;
  lineSoft: string;
};

export type CustomTheme = {
  id: string;
  name: string;
  mode: ThemeMode;
  hue: number;
  contrast: number;
};

export type XtermChromeTheme = {
  background: string;
  cursor: string;
  cursorAccent: string;
  foreground: string;
  scrollbarSliderBackground: string;
  scrollbarSliderHoverBackground: string;
  scrollbarSliderActiveBackground: string;
};

export const MAX_CUSTOM_THEMES = 20;
export const MAX_CUSTOM_THEME_NAME = 16;
export const DEFAULT_HUE_DARK = 220;
export const DEFAULT_HUE_LIGHT = 210;
export const DEFAULT_CONTRAST = 50;

export const BUILTIN_XTERM_THEME: XtermChromeTheme = {
  background: '#0d1117',
  cursor: '#0d1117',
  cursorAccent: '#e6edf3',
  foreground: '#e6edf3',
  scrollbarSliderBackground: '#4f5bd5',
  scrollbarSliderHoverBackground: '#6b74ff',
  scrollbarSliderActiveBackground: '#8b92ff',
};

const SURFACE_VARS: Array<[string, keyof SurfaceTokens | 'rgba-text-soft' | 'rgba-text-hover' | 'copy-bg' | 'copy-elev' | 'copy-line' | 'copy-text' | 'copy-muted']> = [
  ['--bg', 'bg'],
  ['--bg-layer', 'copy-bg'],
  ['--bg-elev', 'bgElev'],
  ['--bg-elev2', 'bgElev2'],
  ['--bg-deep', 'bgDeep'],
  ['--bg-2', 'copy-elev'],
  ['--panel', 'copy-elev'],
  ['--bg-soft', 'rgba-text-soft'],
  ['--bg-hover', 'rgba-text-hover'],
  ['--line', 'line'],
  ['--line-soft', 'lineSoft'],
  ['--border', 'copy-line'],
  ['--text', 'text'],
  ['--text-soft', 'textSoft'],
  ['--text-dim', 'textDim'],
  ['--fg', 'copy-text'],
  ['--fg-muted', 'copy-muted'],
  ['--muted', 'muted'],
  ['--muted-2', 'muted2'],
];

let activeXtermTheme: XtermChromeTheme = BUILTIN_XTERM_THEME;

export function currentXtermTheme(): XtermChromeTheme {
  return activeXtermTheme;
}

export function setActiveXtermTheme(theme: XtermChromeTheme): void {
  activeXtermTheme = theme;
}

function clampInt(n: number, min: number, max: number): number {
  if (!Number.isFinite(n)) return min;
  return Math.max(min, Math.min(max, Math.round(n)));
}

function rgbToHex(r: number, g: number, b: number): string {
  const c = (n: number) => ('0' + Math.max(0, Math.min(255, Math.round(n))).toString(16)).slice(-2);
  return '#' + c(r) + c(g) + c(b);
}

export function hslHex(h: number, s: number, l: number): string {
  h = ((h % 360) + 360) % 360;
  s = Math.max(0, Math.min(100, s)) / 100;
  l = Math.max(0, Math.min(100, l)) / 100;
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => {
    const k = (n + h / 30) % 12;
    const c = l - a * Math.max(Math.min(k - 3, 9 - k, 1), -1);
    return Math.round(255 * c);
  };
  return rgbToHex(f(0), f(8), f(4));
}

function hexToRgb(hex: string): { r: number; g: number; b: number } {
  const h = String(hex || '').replace('#', '');
  const full = h.length === 3 ? h[0] + h[0] + h[1] + h[1] + h[2] + h[2] : h;
  return {
    r: parseInt(full.slice(0, 2), 16) || 0,
    g: parseInt(full.slice(2, 4), 16) || 0,
    b: parseInt(full.slice(4, 6), 16) || 0,
  };
}

function rgba(hex: string, a: number): string {
  const rgb = hexToRgb(hex);
  return 'rgba(' + rgb.r + ',' + rgb.g + ',' + rgb.b + ',' + a + ')';
}

export function hueName(h: number, mode: ThemeMode): string {
  const hue = ((Number(h) % 360) + 360) % 360;
  const tail = mode === 'light' ? 'paper' : 'black';
  if (hue < 25 || hue >= 335) return 'red-' + tail;
  if (hue < 55) return 'orange-' + tail;
  if (hue < 80) return 'yellow-' + tail;
  if (hue < 150) return 'green-' + tail;
  if (hue < 200) return 'teal-' + tail;
  if (hue < 255) return 'blue-' + tail;
  if (hue < 295) return 'purple-' + tail;
  return 'magenta-' + tail;
}

export function hueNameJa(h: number, mode: ThemeMode): string {
  const hue = ((Number(h) % 360) + 360) % 360;
  const tail = mode === 'light' ? '紙' : '黒';
  if (hue < 25 || hue >= 335) return '赤' + tail;
  if (hue < 55) return '橙' + tail;
  if (hue < 80) return '黄' + tail;
  if (hue < 150) return '緑' + tail;
  if (hue < 200) return '青緑' + tail;
  if (hue < 255) return '青' + tail;
  if (hue < 295) return '紫' + tail;
  return '赤紫' + tail;
}

export function contrastNameJa(c: number): string {
  if (c < 30) return '弱い';
  if (c < 45) return 'やや弱い';
  if (c <= 55) return '標準';
  if (c < 75) return 'やや強い';
  return '強い';
}

export function hueBarCss(mode: ThemeMode): string {
  const l = mode === 'light' ? 88 : 14;
  const s = mode === 'light' ? 28 : 40;
  const stops: string[] = [];
  for (let i = 0; i <= 6; i++) stops.push(hslHex(i * 60, s, l));
  return 'linear-gradient(90deg,' + stops.join(',') + ')';
}

export function deriveCustomTheme(mode: ThemeMode, hue: number, contrast: number): SurfaceTokens {
  const t = clampInt(contrast, 0, 100) / 100;
  const h = clampInt(hue, 0, 359);
  if (mode === 'dark') {
    const sat = 8 + t * 10;
    const bgL = 13 - t * 10;
    const textL = 74 + t * 20;
    const mutedL = 52 + t * 6;
    const elevL = bgL + (7 - t * 3);
    const elev2L = elevL + 5;
    const deepL = Math.max(1, bgL - 3);
    const text = hslHex(h, 10, textL);
    return {
      bg: hslHex(h, sat, bgL),
      bgElev: hslHex(h, sat, elevL),
      bgElev2: hslHex(h, sat, elev2L),
      bgDeep: hslHex(h, sat, deepL),
      text,
      textSoft: hslHex(h, 8, textL - 8),
      textDim: hslHex(h, 8, textL - 16),
      muted: hslHex(h, 8, mutedL),
      muted2: hslHex(h, 8, mutedL - 8),
      line: rgba(text, 0.12),
      lineSoft: rgba(text, 0.20),
    };
  }
  const satL = 10 + (1 - t) * 8;
  const bgL = 90 + t * 6;
  const textL = 24 - t * 16;
  const mutedL = 42 - t * 8;
  const text = hslHex(h, 18, textL);
  return {
    bg: hslHex(h, satL, bgL),
    bgElev: hslHex(h, Math.max(0, satL - 4), Math.min(100, bgL + 6)),
    bgElev2: hslHex(h, satL, bgL - 6),
    bgDeep: hslHex(h, satL, bgL - 4),
    text,
    textSoft: hslHex(h, 14, textL + 10),
    textDim: hslHex(h, 12, textL + 18),
    muted: hslHex(h, 10, mutedL),
    muted2: hslHex(h, 10, mutedL + 8),
    line: rgba(text, 0.16),
    lineSoft: rgba(text, 0.24),
  };
}

export function xtermThemeFromTokens(tokens: SurfaceTokens): XtermChromeTheme {
  return {
    background: tokens.bgDeep,
    cursor: tokens.bgDeep,
    cursorAccent: tokens.text,
    foreground: tokens.text,
    scrollbarSliderBackground: BUILTIN_XTERM_THEME.scrollbarSliderBackground,
    scrollbarSliderHoverBackground: BUILTIN_XTERM_THEME.scrollbarSliderHoverBackground,
    scrollbarSliderActiveBackground: BUILTIN_XTERM_THEME.scrollbarSliderActiveBackground,
  };
}

export function applySurfaceTokens(tokens: SurfaceTokens): void {
  const root = document.documentElement;
  for (const [cssVar, key] of SURFACE_VARS) {
    if (key === 'copy-bg') root.style.setProperty(cssVar, tokens.bg);
    else if (key === 'copy-elev') root.style.setProperty(cssVar, tokens.bgElev);
    else if (key === 'copy-line') root.style.setProperty(cssVar, tokens.line);
    else if (key === 'copy-text') root.style.setProperty(cssVar, tokens.text);
    else if (key === 'copy-muted') root.style.setProperty(cssVar, tokens.muted);
    else if (key === 'rgba-text-soft') root.style.setProperty(cssVar, rgba(tokens.text, 0.04));
    else if (key === 'rgba-text-hover') root.style.setProperty(cssVar, rgba(tokens.text, 0.06));
    else root.style.setProperty(cssVar, tokens[key]);
  }
}

export function clearSurfaceTokens(): void {
  const root = document.documentElement;
  for (const [cssVar] of SURFACE_VARS) root.style.removeProperty(cssVar);
}

export function validCustomThemeId(id: string): boolean {
  if (!id.startsWith('u-') || id.length < 4 || id.length > 32) return false;
  return /^u-[a-zA-Z0-9]+$/.test(id);
}

export function newCustomThemeId(): string {
  return 'u-' + Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
}

function cleanName(raw: unknown): string {
  const s = String(raw ?? '').replace(/[\u0000-\u001f\u007f]/g, '').trim();
  if (!s) return '';
  return [...s].slice(0, MAX_CUSTOM_THEME_NAME).join('');
}

export function sanitizeCustomTheme(raw: unknown): CustomTheme | null {
  if (!raw || typeof raw !== 'object') return null;
  const o = raw as Record<string, unknown>;
  const id = String(o.id ?? '').trim();
  if (!validCustomThemeId(id)) return null;
  const name = cleanName(o.name);
  if (!name) return null;
  const mode: ThemeMode = o.mode === 'light' ? 'light' : o.mode === 'dark' ? 'dark' : 'dark';
  return {
    id,
    name,
    mode,
    hue: clampInt(Number(o.hue), 0, 359),
    contrast: clampInt(Number(o.contrast), 0, 100),
  };
}

export function sanitizeCustomThemes(raw: unknown): CustomTheme[] {
  if (!Array.isArray(raw)) return [];
  const seen = new Set<string>();
  const out: CustomTheme[] = [];
  for (const item of raw) {
    const t = sanitizeCustomTheme(item);
    if (!t || seen.has(t.id)) continue;
    seen.add(t.id);
    out.push(t);
    if (out.length >= MAX_CUSTOM_THEMES) break;
  }
  return out;
}

export function resolveThemeId(theme: string, customs: CustomTheme[]): string {
  if (theme === 'light' || theme === 'dark') return theme;
  if (customs.some((c) => c.id === theme)) return theme;
  return 'light';
}
