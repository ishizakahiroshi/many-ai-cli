import assert from 'node:assert/strict';
import test from 'node:test';
import {
  ALT_RAIL_MIN_SLIDER_PX,
  ALT_RAIL_MIN_VIRTUAL_PAGES,
  altRailApplyPages,
  altRailGeometry,
  altRailInitialState,
  altRailPageTotal,
  altRailPagesFromSliderTop,
  altRailStep,
} from './alt-scroll-rail.js';

const TRACK = 300;

test('alt-rail: 未操作でも下限ページ数ぶんのスライダーが最下部に出る', () => {
  const s = altRailInitialState();
  assert.equal(altRailPageTotal(s), ALT_RAIL_MIN_VIRTUAL_PAGES);
  const g = altRailGeometry(s, TRACK);
  assert.equal(g.sliderHeight, TRACK / ALT_RAIL_MIN_VIRTUAL_PAGES);
  // 最下部 = スライダー下端がトラック下端に一致する。
  assert.equal(g.sliderTop + g.sliderHeight, TRACK);
});

test('alt-rail: 上へ遡るほど範囲が広がりスライダーが縮む', () => {
  let s = altRailInitialState();
  for (let i = 0; i < 9; i++) s = altRailStep(s, -1);
  assert.equal(s.pagesUp, 9);
  const total = altRailPageTotal(s);
  assert.ok(total > 10, `到達済み 9 ページより広い範囲になる: ${total}`);
  const g = altRailGeometry(s, TRACK);
  assert.ok(g.sliderHeight < TRACK / ALT_RAIL_MIN_VIRTUAL_PAGES);
  // 到達済みの先にも余地を残すので、上端には張り付かない（＝まだ上へ遡れる）。
  assert.ok(g.sliderTop > 0, `上端に張り付かない: ${g.sliderTop}`);
});

test('alt-rail: 最上部までドラッグしても、その先へ遡る余地が残る', () => {
  const s = altRailApplyPages(altRailInitialState(), 3);
  // 上端へドラッグ = pagesUp が total - 1 まで進む。
  const dragged = altRailApplyPages(s, altRailPagesFromSliderTop(s, TRACK, 0));
  assert.ok(dragged.pagesUp > s.pagesUp);
  assert.ok(altRailGeometry(dragged, TRACK).sliderTop > 0);
});

test('alt-rail: 下へ戻しても広がった範囲は縮まない', () => {
  let s = altRailInitialState();
  for (let i = 0; i < 9; i++) s = altRailStep(s, -1);
  const total = altRailPageTotal(s);
  for (let i = 0; i < 9; i++) s = altRailStep(s, 1);
  assert.equal(s.pagesUp, 0);
  assert.equal(s.pagesUpMax, 9);
  assert.equal(altRailPageTotal(s), total);
  const g = altRailGeometry(s, TRACK);
  assert.equal(g.sliderTop + g.sliderHeight, TRACK);
});

test('alt-rail: pagesUp は 0 未満にならない', () => {
  let s = altRailInitialState();
  s = altRailStep(s, 1);
  assert.equal(s.pagesUp, 0);
  assert.equal(altRailApplyPages(s, -5).pagesUp, 0);
});

test('alt-rail: トラックが極端に低いときも最小高さで潰れない', () => {
  const s = altRailApplyPages(altRailInitialState(), 20);
  const g = altRailGeometry(s, 60);
  assert.equal(g.sliderHeight, ALT_RAIL_MIN_SLIDER_PX);
  assert.ok(g.sliderTop >= 0);
  assert.ok(g.sliderTop + g.sliderHeight <= 60);
});

test('alt-rail: トラック高が 0 でも例外にならない', () => {
  const g = altRailGeometry(altRailInitialState(), 0);
  assert.equal(g.sliderHeight, 0);
  assert.equal(g.sliderTop, 0);
  assert.equal(altRailPagesFromSliderTop(altRailInitialState(), 0, 10), 0);
});

test('alt-rail: ドラッグの往復でページ数が復元する', () => {
  const s = altRailApplyPages(altRailInitialState(), 5);
  for (let pages = 0; pages <= 5; pages++) {
    const moved = altRailApplyPages(s, pages);
    const { sliderTop } = altRailGeometry(moved, TRACK);
    assert.equal(altRailPagesFromSliderTop(s, TRACK, sliderTop), pages);
  }
});

test('alt-rail: トラック外へドラッグしても両端でクランプする', () => {
  const s = altRailApplyPages(altRailInitialState(), 5);
  assert.equal(altRailPagesFromSliderTop(s, TRACK, -999), altRailPageTotal(s) - 1);
  assert.equal(altRailPagesFromSliderTop(s, TRACK, 9999), 0);
});
