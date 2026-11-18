import { describe, expect, test } from 'bun:test';
import {
  LAYOUT_PLACEMENTS, LAYOUT_PRESET_MEDIA, effectiveLayoutPreset, layoutBodyClasses, readLayoutPreset,
  resolveLayoutPlace,
} from '../src/app/layout-preset.js';

describe('layout preset', () => {
  test('missing or invalid stored values resolve to bottom', () => {
    expect(readLayoutPreset(null)).toBe('bottom');
    expect(readLayoutPreset('')).toBe('bottom');
    expect(readLayoutPreset('left')).toBe('bottom');
    expect(readLayoutPreset('TOP')).toBe('bottom');
    expect(readLayoutPreset('top ')).toBe('bottom');
  });
  test('stored values map to their preset', () => {
    expect(readLayoutPreset('bottom')).toBe('bottom');
    expect(readLayoutPreset('top')).toBe('top');
  });
  test('the input area resolves to its place per preset', () => {
    expect(resolveLayoutPlace('bottom', 'input-area')).toBe('below-terminal');
    expect(resolveLayoutPlace('top', 'input-area')).toBe('above-terminal');
    expect(resolveLayoutPlace('bottom', 'unknown-part')).toBeUndefined();
  });
  test('only parts placed differently from bottom produce body classes', () => {
    expect(layoutBodyClasses('bottom')).toEqual([]);
    expect(layoutBodyClasses('top')).toEqual(['layout-input-top']);
  });
  test('top only takes effect where the screen condition matches', () => {
    expect(effectiveLayoutPreset('top', true)).toBe('top');
    // 狭い幅・タッチ主体の画面では、上寄せを選んでいても下寄せとして扱う
    expect(effectiveLayoutPreset('top', false)).toBe('bottom');
    expect(effectiveLayoutPreset('bottom', true)).toBe('bottom');
    expect(effectiveLayoutPreset('bottom', false)).toBe('bottom');
    expect(layoutBodyClasses(effectiveLayoutPreset('top', false))).toEqual([]);
  });
  test('the screen condition is the inverse of the approval bar flow-layout rule', async () => {
    // approval.css はこの条件でバーをフロー配置へ戻す。上寄せの条件がこれと重なると、
    // フロー配置のバーに上ドックの top が当たって上へずれる。
    const css = await Bun.file(new URL('../src/styles/approval.css', import.meta.url)).text();
    expect(css).toMatch(/@media \(max-width: 720px\), \(pointer: coarse\) \{\r?\n\s*#action-bar \{/);
    expect(LAYOUT_PRESET_MEDIA).toBe('(min-width: 721px) and (not (pointer: coarse))');
  });
  test('every placement row has a place for both presets', () => {
    for (const row of LAYOUT_PLACEMENTS) {
      expect(row.place.bottom).toBeTruthy();
      expect(row.place.top).toBeTruthy();
      expect(row.activeClass).toBeTruthy();
    }
  });
  test('settings has a layout section with the preset select and the moved controls', async () => {
    const html = await Bun.file(new URL('../src/index.html', import.meta.url)).text();
    const section = html.slice(
      html.indexOf('data-section="layout"'),
      html.indexOf('data-section="usage"'),
    );
    for (const id of [
      'layout-preset-select',
      'ui-side-cards-select', 'ui-side-tools-select', 'ui-side-close-select',
      'ui-side-approval-select', 'input-tools-order-reset-btn',
    ]) {
      expect(section).toContain(`id="${id}"`);
    }
    // 確認ポップアップの選択欄は「同じ側 / いつも下 / 右 / 左」の 4 値
    const approval = section.slice(section.indexOf('id="ui-side-approval-select"'));
    for (const value of ['value=""', 'value="bottom"', 'value="right"', 'value="left"']) {
      expect(approval).toContain(value);
    }
  });
});
