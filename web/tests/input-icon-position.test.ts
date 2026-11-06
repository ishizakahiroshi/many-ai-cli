import { describe, expect, test } from 'bun:test';
import {
  applyVisibleOrder, clampActionsWidth, clampIconPoint, completeIconOrder, insertionIndex,
  layoutStatusControls, moveIconInOrder, nextIconScale, orderFromPositions, parseActionsWidth,
  parseIconOrder, parseIconPositions, parseIconScales,
  INPUT_ICON_IDS, MAX_ICON_SCALE, MIN_ACTIONS_WIDTH, MIN_ICON_SCALE, MIN_STATUS_TEXT_WIDTH,
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
  test('a fixed button-area width reserves exactly that width and packs rows inside it', () => {
    const fixed = layoutStatusControls([80, 80, 80], 600, [], 200);
    // 200 から余白 18 を引いた 182 に 80+6+80=166 までが入り、3 個目は次の段へ回る。
    expect(fixed.rows).toBe(2);
    expect(fixed.reservedWidth).toBe(200);
    expect(fixed.points[0].x).toBe(600 - 8 - 166);
    expect(fixed.points[2]).toEqual({ x: 600 - 8 - 80, y: 43 });
    // 幅の指定が無ければ従来どおり、先頭の段の幅 + 18。
    expect(layoutStatusControls([80, 80, 80], 600).reservedWidth).toBe(270);
    expect(layoutStatusControls([], 600, [], 200).reservedWidth).toBe(0);
  });
  test('the button-area width stays inside the band and leaves room for the status text', () => {
    expect(clampActionsWidth(10, 800)).toBe(MIN_ACTIONS_WIDTH);
    expect(clampActionsWidth(900, 800)).toBe(800 - MIN_STATUS_TEXT_WIDTH);
    expect(clampActionsWidth(333.4, 800)).toBe(333);
    expect(clampActionsWidth(500, 100)).toBe(MIN_ACTIONS_WIDTH);
    expect(parseActionsWidth('320')).toBe(320);
    expect(parseActionsWidth('12')).toBeNull();
    expect(parseActionsWidth('abc')).toBeNull();
    expect(parseActionsWidth(null)).toBeNull();
  });
  test('the saved band order keeps known buttons once, in order, and fills in newly added ones', () => {
    expect(parseIconOrder('["send-btn","not-a-button","voice-btn","send-btn"]')).toEqual(['send-btn', 'voice-btn']);
    expect(parseIconOrder('{"a":1}')).toEqual([]);
    expect(parseIconOrder('bad json')).toEqual([]);
    expect(parseIconOrder(null)).toEqual([]);
    // 保存に無いボタンは、既定の並びで直前にあるボタンの後ろへ入る。
    const full = completeIconOrder(['buf-clear-btn', 'send-btn']);
    expect(full).toHaveLength(INPUT_ICON_IDS.length);
    expect(new Set(full).size).toBe(INPUT_ICON_IDS.length);
    expect(full.indexOf('buf-clear-btn')).toBeLessThan(full.indexOf('send-btn'));
    expect(full.indexOf('voice-btn')).toBe(full.indexOf('send-btn') + 1);
    expect(completeIconOrder([])).toEqual([...INPUT_ICON_IDS]);
  });
  test('the old saved coordinates become a reading order: upper row first, left to right', () => {
    const saved = parseIconPositions(JSON.stringify({
      'voice-btn': { x: 300, y: 8 },
      'send-btn': { x: 100, y: 10 },
      'slash-picker-btn': { x: 50, y: 60 },
      'buf-clear-btn': { x: 10, y: 62 },
    }));
    expect(orderFromPositions(saved)).toEqual(['send-btn', 'voice-btn', 'buf-clear-btn', 'slash-picker-btn']);
    expect(orderFromPositions({})).toEqual([]);
  });
  test('dropping between buttons inserts before or after the nearest one', () => {
    const slots = [{ cx: 100, cy: 20 }, { cx: 160, cy: 20 }, { cx: 220, cy: 20 }];
    expect(insertionIndex(slots, 90, 20)).toBe(0);
    expect(insertionIndex(slots, 110, 20)).toBe(1);
    expect(insertionIndex(slots, 175, 22)).toBe(2);
    expect(insertionIndex(slots, 400, 20)).toBe(3);
    expect(insertionIndex([], 10, 10)).toBe(0);
    const ids = ['a', 'b', 'c', 'd'];
    expect(moveIconInOrder(ids, 'a', 2)).toEqual(['b', 'c', 'a', 'd']);
    expect(moveIconInOrder(ids, 'd', 0)).toEqual(['d', 'a', 'b', 'c']);
    expect(moveIconInOrder(ids, 'b', 99)).toEqual(['a', 'c', 'd', 'b']);
  });
  test('reordering the visible buttons leaves the hidden ones where they were', () => {
    const full = ['a', 'h1', 'b', 'c', 'h2'];
    expect(applyVisibleOrder(full, ['c', 'a', 'b'])).toEqual(['c', 'h1', 'a', 'b', 'h2']);
    expect(applyVisibleOrder(full, ['a', 'b', 'c'])).toEqual(full);
  });
  test('restores saved sizes within range and ignores malformed ones', () => {
    expect(parseIconScales('{"send-btn":2,"voice-btn":9,"slash-picker-btn":0.1,"buf-clear-btn":1}'))
      .toEqual({ 'send-btn': 2, 'voice-btn': 4, 'slash-picker-btn': 0.5 });
    expect(parseIconScales('{"send-btn":"2","not-an-input-control":2}')).toEqual({});
    expect(parseIconScales('bad json')).toEqual({});
  });
});
