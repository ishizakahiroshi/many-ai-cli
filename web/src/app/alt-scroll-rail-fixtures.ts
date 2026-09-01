import assert from 'node:assert/strict';
import test from 'node:test';
import {
  ALT_RAIL_MIN_SLIDER_PX,
  ALT_RAIL_MIN_VIRTUAL_NOTCHES,
  altRailApplyNotches,
  altRailGeometry,
  altRailInitialState,
  altRailNotchTotal,
  altRailNotchesFromSliderTop,
  altRailStep,
} from './alt-scroll-rail.js';

const TRACK = 300;
const LARGE_TRACK = 1200;

test('alt-rail: 未操作でも下限ノッチ数ぶんのスライダーが最下部に出る', () => {
  const s = altRailInitialState();
  assert.equal(altRailNotchTotal(s), ALT_RAIL_MIN_VIRTUAL_NOTCHES);
  const g = altRailGeometry(s, TRACK);
  assert.equal(g.sliderHeight, ALT_RAIL_MIN_SLIDER_PX);
  // 最下部 = スライダー下端がトラック下端に一致する。
  assert.equal(g.sliderTop + g.sliderHeight, TRACK);
});

test('alt-rail: 上へ遡るほど範囲が広がりスライダーが縮む', () => {
  let s = altRailInitialState();
  for (let i = 0; i < 40; i++) s = altRailStep(s, -1);
  assert.equal(s.notchesUp, 40);
  const total = altRailNotchTotal(s);
  assert.ok(total > 40, `到達済み 40 ノッチより広い範囲になる: ${total}`);
  const g = altRailGeometry(s, LARGE_TRACK);
  assert.ok(g.sliderHeight < LARGE_TRACK / ALT_RAIL_MIN_VIRTUAL_NOTCHES);
  // 到達済みの先にも余地を残すので、上端には張り付かない（＝まだ上へ遡れる）。
  assert.ok(g.sliderTop > 0, `上端に張り付かない: ${g.sliderTop}`);
});

test('alt-rail: 最上部までドラッグしても、その先へ遡る余地が残る', () => {
  const s = altRailApplyNotches(altRailInitialState(), 3);
  // 上端へドラッグ = notchesUp が total - 1 まで進む。
  const dragged = altRailApplyNotches(s, altRailNotchesFromSliderTop(s, TRACK, 0));
  assert.ok(dragged.notchesUp > s.notchesUp);
  assert.ok(altRailGeometry(dragged, TRACK).sliderTop > 0);
});

test('alt-rail: 下へ戻しても広がった範囲は縮まない', () => {
  let s = altRailInitialState();
  for (let i = 0; i < 9; i++) s = altRailStep(s, -1);
  const total = altRailNotchTotal(s);
  for (let i = 0; i < 9; i++) s = altRailStep(s, 1);
  assert.equal(s.notchesUp, 0);
  assert.equal(s.notchesUpMax, 9);
  assert.equal(altRailNotchTotal(s), total);
  const g = altRailGeometry(s, TRACK);
  assert.equal(g.sliderTop + g.sliderHeight, TRACK);
});

test('alt-rail: notchesUp は 0 未満にならない', () => {
  let s = altRailInitialState();
  s = altRailStep(s, 1);
  assert.equal(s.notchesUp, 0);
  assert.equal(altRailApplyNotches(s, -5).notchesUp, 0);
});

test('alt-rail: トラックが極端に低いときも最小高さで潰れない', () => {
  const s = altRailApplyNotches(altRailInitialState(), 20);
  const g = altRailGeometry(s, 60);
  assert.equal(g.sliderHeight, ALT_RAIL_MIN_SLIDER_PX);
  assert.ok(g.sliderTop >= 0);
  assert.ok(g.sliderTop + g.sliderHeight <= 60);
});

test('alt-rail: トラック高が 0 でも例外にならない', () => {
  const g = altRailGeometry(altRailInitialState(), 0);
  assert.equal(g.sliderHeight, 0);
  assert.equal(g.sliderTop, 0);
  assert.equal(altRailNotchesFromSliderTop(altRailInitialState(), 0, 10), 0);
});

test('alt-rail: ドラッグの往復でノッチ数が復元する', () => {
  const s = altRailApplyNotches(altRailInitialState(), 5);
  for (let notches = 0; notches <= 5; notches++) {
    const moved = altRailApplyNotches(s, notches);
    const { sliderTop } = altRailGeometry(moved, TRACK);
    assert.equal(altRailNotchesFromSliderTop(s, TRACK, sliderTop), notches);
  }
});

test('alt-rail: トラック外へドラッグしても両端でクランプする', () => {
  const s = altRailApplyNotches(altRailInitialState(), 5);
  assert.equal(altRailNotchesFromSliderTop(s, TRACK, -999), altRailNotchTotal(s) - 1);
  assert.equal(altRailNotchesFromSliderTop(s, TRACK, 9999), 0);
});
