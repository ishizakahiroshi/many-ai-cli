import assert from 'node:assert/strict';
import test from 'node:test';
import {
  BUILTIN_LIGHT_XTERM_THEME,
  BUILTIN_XTERM_THEME,
  DARK_MINIMUM_CONTRAST_RATIO,
  DARK_XTERM_ANSI,
  DEFAULT_CONTRAST,
  LIGHT_MINIMUM_CONTRAST_RATIO,
  LIGHT_XTERM_ANSI,
  deriveCustomTheme,
  hslHex,
  minimumContrastRatioForMode,
  newCustomThemeId,
  resolveThemeId,
  sanitizeCustomTheme,
  sanitizeCustomThemes,
  validCustomThemeId,
  xtermThemeFromTokens,
} from './theme-tokens.js';

function luminance(hex: string): number {
  const n = parseInt(hex.slice(1), 16);
  const r = (n >> 16) & 255, g = (n >> 8) & 255, b = n & 255;
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

test('deriveCustomTheme: stronger contrast darkens dark bg and lightens text', () => {
  const weak = deriveCustomTheme('dark', 220, 10);
  const strong = deriveCustomTheme('dark', 220, 90);
  const lum = (hex: string) => {
    const n = parseInt(hex.slice(1), 16);
    const r = (n >> 16) & 255, g = (n >> 8) & 255, b = n & 255;
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  assert.ok(lum(strong.bg) < lum(weak.bg), 'strong bg should be darker');
  assert.ok(lum(strong.text) > lum(weak.text), 'strong text should be lighter');
});

test('deriveCustomTheme: hue shifts the ground away from a different hue', () => {
  const blue = deriveCustomTheme('dark', 220, DEFAULT_CONTRAST);
  const red = deriveCustomTheme('dark', 15, DEFAULT_CONTRAST);
  assert.notEqual(blue.bg, red.bg);
});

test('hslHex: 0 sat is gray', () => {
  assert.equal(hslHex(0, 0, 0), '#000000');
  assert.equal(hslHex(120, 0, 100), '#ffffff');
});

test('sanitizeCustomTheme drops builtin ids and empty names', () => {
  assert.equal(sanitizeCustomTheme({ id: 'dark', name: 'x', mode: 'dark', hue: 1, contrast: 1 }), null);
  assert.equal(sanitizeCustomTheme({ id: 'u-ok', name: '   ', mode: 'dark', hue: 1, contrast: 1 }), null);
  const ok = sanitizeCustomTheme({ id: 'u-ok', name: '濃い', mode: 'dark', hue: 400, contrast: -4 });
  assert.ok(ok);
  assert.equal(ok!.hue, 359);
  assert.equal(ok!.contrast, 0);
  assert.equal(ok!.name, '濃い');
});

test('sanitizeCustomThemes caps length and drops duplicates', () => {
  const many = Array.from({ length: 25 }, (_, i) => ({
    id: 'u-a' + i, name: 't' + i, mode: 'dark', hue: 10, contrast: 50,
  }));
  many.push({ id: 'u-a0', name: 'dup', mode: 'dark', hue: 1, contrast: 1 });
  const out = sanitizeCustomThemes(many);
  assert.equal(out.length, 20);
  assert.equal(out[0].name, 't0');
});

test('validCustomThemeId and resolveThemeId', () => {
  assert.equal(validCustomThemeId('u-abc1'), true);
  assert.equal(validCustomThemeId('light'), false);
  const list = [{ id: 'u-abc1', name: '濃い', mode: 'dark' as const, hue: 220, contrast: 80 }];
  assert.equal(resolveThemeId('u-abc1', list), 'u-abc1');
  assert.equal(resolveThemeId('u-missing', list), 'light');
  assert.equal(resolveThemeId('dark', list), 'dark');
});

test('newCustomThemeId is a valid id', () => {
  assert.equal(validCustomThemeId(newCustomThemeId()), true);
});

test('BUILTIN_LIGHT_XTERM_THEME: light background with dark foreground', () => {
  assert.ok(luminance(BUILTIN_LIGHT_XTERM_THEME.background) > 200, 'background should be near white');
  assert.ok(luminance(BUILTIN_LIGHT_XTERM_THEME.foreground) < 80, 'foreground should be dark');
});

test('BUILTIN_XTERM_THEME: dark palette keeps the previous values', () => {
  assert.equal(BUILTIN_XTERM_THEME.background, '#0d1117');
  assert.equal(BUILTIN_XTERM_THEME.foreground, '#e6edf3');
  assert.equal(BUILTIN_XTERM_THEME.black, DARK_XTERM_ANSI.black);
  assert.equal(BUILTIN_XTERM_THEME.brightWhite, DARK_XTERM_ANSI.brightWhite);
});

test('xtermThemeFromTokens: picks the ANSI palette by mode', () => {
  const tokens = deriveCustomTheme('light', 210, 50);
  const light = xtermThemeFromTokens(tokens, 'light');
  assert.equal(light.background, tokens.bgDeep);
  assert.equal(light.foreground, tokens.text);
  assert.equal(light.red, LIGHT_XTERM_ANSI.red);
  assert.equal(light.brightCyan, LIGHT_XTERM_ANSI.brightCyan);

  const darkTokens = deriveCustomTheme('dark', 220, 50);
  const dark = xtermThemeFromTokens(darkTokens, 'dark');
  assert.equal(dark.background, darkTokens.bgDeep);
  assert.equal(dark.foreground, darkTokens.text);
  assert.equal(dark.red, DARK_XTERM_ANSI.red);
  assert.equal(dark.brightCyan, DARK_XTERM_ANSI.brightCyan);
});

test('LIGHT_XTERM_ANSI is legible on white and differs from the dark palette', () => {
  for (const [key, value] of Object.entries(LIGHT_XTERM_ANSI)) {
    assert.ok(luminance(value) < 200, key + ' should be dark enough for a white background');
  }
  assert.notEqual(LIGHT_XTERM_ANSI.yellow, DARK_XTERM_ANSI.yellow);
  assert.notEqual(LIGHT_XTERM_ANSI.brightWhite, DARK_XTERM_ANSI.brightWhite);
});

test('minimumContrastRatioForMode: light raises contrast, dark stays at the xterm default', () => {
  assert.equal(minimumContrastRatioForMode('light'), LIGHT_MINIMUM_CONTRAST_RATIO);
  assert.ok(LIGHT_MINIMUM_CONTRAST_RATIO > DARK_MINIMUM_CONTRAST_RATIO);
  assert.equal(minimumContrastRatioForMode('dark'), DARK_MINIMUM_CONTRAST_RATIO);
  assert.equal(DARK_MINIMUM_CONTRAST_RATIO, 1);
});
