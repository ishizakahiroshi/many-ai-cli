import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DEFAULT_CONTRAST,
  deriveCustomTheme,
  hslHex,
  newCustomThemeId,
  resolveThemeId,
  sanitizeCustomTheme,
  sanitizeCustomThemes,
  validCustomThemeId,
} from './theme-tokens.js';

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
