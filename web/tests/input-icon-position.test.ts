import { describe, expect, test } from 'bun:test';
import {
  clampIconPoint, layoutStatusControls, nextIconScale, parseIconPositions, parseIconScales,
  INPUT_ICON_IDS, MAX_ICON_SCALE, MIN_ICON_SCALE,
} from '../src/app/input-icon-position.js';

describe('independent input icon positions', () => {
  test('clear, templates and nested quick commands keep separate saved positions', () => {
    const saved = {
      'input-clear-btn': { x: 100, y: 10 },
      'prompt-template-toggle': { x: 140, y: 20 },
      'quick-cmd-btn-3': { x: 220, y: 30 },
      'slash-picker-btn': { x: 280, y: 40 },
      'not-an-input-control': { x: 1, y: 1 },
    };
    const restored = parseIconPositions(JSON.stringify(saved));
    expect(restored['input-clear-btn']).toEqual({ x: 100, y: 10 });
    expect(restored['prompt-template-toggle']).toEqual({ x: 140, y: 20 });
    expect(restored['quick-cmd-btn-3']).toEqual({ x: 220, y: 30 });
    expect(restored['slash-picker-btn']).toEqual({ x: 280, y: 40 });
    expect(Object.keys(restored)).toHaveLength(4);
  });
  test('every input toolbar button can be positioned, excluding menu and status actions', async () => {
    const html = await Bun.file(new URL('../src/index.html', import.meta.url)).text();
    const composer = html.slice(html.indexOf('<div id="input-wrap">'), html.indexOf('<div id="mobile-composer-attach-menu"'));
    const buttons = Array.from(composer.matchAll(/<button[^>]*\bid="([^"]+)"/g), m => m[1])
      .filter(id => id !== 'deferred-send-cancel');
    expect([...INPUT_ICON_IDS].sort()).toEqual(buttons.sort());
  });
  test('restores each icon independently and ignores malformed stored coordinates', () => {
    expect(parseIconPositions('{"send-btn":{"x":160,"y":12},"voice-btn":{"x":230,"y":40}}'))
      .toEqual({ 'send-btn': { x: 160, y: 12 }, 'voice-btn': { x: 230, y: 40 } });
    expect(parseIconPositions('{"send-btn":{"x":-1,"y":12},"voice-btn":{"x":20,"y":"40"}}')).toEqual({});
    expect(parseIconPositions('null')).toEqual({});
    expect(parseIconPositions('[1,2]')).toEqual({});
    expect(parseIconPositions('bad json')).toEqual({});
  });
  test('keeps the icon and grip reachable at every viewport edge', () => {
    expect(clampIconPoint({ x: -10, y: -30 }, 1000, 100, 32, 32)).toEqual({ x: 8, y: 8 });
    expect(clampIconPoint({ x: 2000, y: 500 }, 1000, 100, 32, 32)).toEqual({ x: 960, y: 60 });
    expect(clampIconPoint({ x: 2000, y: 500 }, 20, 20, 32, 32)).toEqual({ x: 0, y: 0 });
  });
  test('resize clamping preserves the preference for restoring a larger box', () => {
    const saved = { x: 600, y: 80 };
    expect(clampIconPoint(saved, 300, 64, 34, 34)).toEqual({ x: 258, y: 22 });
    expect(clampIconPoint(saved, 1000, 200, 34, 34)).toEqual(saved);
    expect(saved).toEqual({ x: 600, y: 80 });
  });
  test('default controls occupy the status band right side and wrap without covering status text', () => {
    const single = layoutStatusControls([80, 80, 80], 600);
    expect(single).toEqual({
      points: [{ x: 340, y: 5 }, { x: 426, y: 5 }, { x: 512, y: 5 }],
      rows: 1,
      reservedWidth: 270,
      height: 42,
    });
    const wrapped = layoutStatusControls([80, 80, 80], 330);
    expect(wrapped.points).toEqual([
      { x: 242, y: 5 }, { x: 242, y: 43 }, { x: 242, y: 81 },
    ]);
    expect(wrapped.rows).toBe(3);
    expect(wrapped.height).toBe(118);
  });
  test('an enlarged control makes its row taller so the next row does not overlap it', () => {
    const layout = layoutStatusControls([80, 80, 80], 330, [36, 144, 36]);
    expect(layout.points.map(p => p.y)).toEqual([5, 43, 189]);
    expect(layout.height).toBe(226);
  });
  test('ctrl+wheel resizes in 10% steps, returns to 100%, and stays within 50%-400%', () => {
    expect(nextIconScale(1, -100)).toBe(1.1);
    expect(nextIconScale(1.1, 100)).toBe(1);
    expect(nextIconScale(1, 100)).toBe(0.9091);
    expect(nextIconScale(3.9, -100)).toBe(MAX_ICON_SCALE);
    expect(nextIconScale(0.52, 100)).toBe(MIN_ICON_SCALE);
    expect(MIN_ICON_SCALE).toBe(0.5);
    expect(MAX_ICON_SCALE).toBe(4);
    // A small trackpad delta still moves the size instead of being rounded away.
    expect(nextIconScale(1, -2)).toBeGreaterThan(1);
  });
  test('restores saved sizes within range and ignores malformed ones', () => {
    expect(parseIconScales('{"send-btn":2,"voice-btn":9,"slash-picker-btn":0.1,"buf-clear-btn":1}'))
      .toEqual({ 'send-btn': 2, 'voice-btn': 4, 'slash-picker-btn': 0.5 });
    expect(parseIconScales('{"send-btn":"2","not-an-input-control":2}')).toEqual({});
    expect(parseIconScales('bad json')).toEqual({});
  });
});
