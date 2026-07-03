import assert from 'node:assert/strict';
import test from 'node:test';
import {
  filterCursorHideBlocksPure,
  MAX_CURSOR_HIDE_BUF,
  type CursorHideFilterState,
} from './cursor-hide-filter.js';

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8');

function bytes(s: string): Uint8Array { return encoder.encode(s); }
function str(b: Uint8Array): string { return decoder.decode(b); }
function initialState(altScreen = false): CursorHideFilterState {
  return {
    carry: new Uint8Array(0),
    inBlock: false,
    blockBuf: [],
    hasAbsPos: false,
    hasNewline: false,
    altScreen,
  };
}

const HIDE = '\x1b[?25l';
const SHOW = '\x1b[?25h';
const ALT_ENTER = '\x1b[?1049h';
const ALT_EXIT = '\x1b[?1049l';

// Grok Build 実セッション（s6・2026-07-03）と同形の「CUP 絶対移動・LF なし」応答ブロック。
// 本文テキストは合成データ（実プロンプト/実応答は fixture に貼らない）。
const GROK_STYLE_BODY = `\x1b[38;2;200;200;200m\x1b[48;2;20;20;20m\x1b[1m\x1b[12;6Hきょうは 2099年1月2日(土) です\x1b[31;7H`;
const GROK_STYLE_BLOCK = `${HIDE}${GROK_STYLE_BODY}${SHOW}`;

// Claude 相当の「CUP・LF なし」スピナー更新ブロック（メインバッファで破棄されるべき対象）。
const SPINNER_BLOCK = `${HIDE}\x1b[38;2;215;119;87m\x1b[28;1H* Reticulating… 3s\x1b[31;3H${SHOW}`;

test('メインバッファ: CUP あり・LF なしブロックは従来どおり破棄される', () => {
  const { out, state, events } = filterCursorHideBlocksPure(bytes(`a${SPINNER_BLOCK}b`), initialState());
  const text = str(out);
  assert.equal(text, 'ab');
  assert.equal(events.length, 1);
  assert.equal(events[0].kind, 'filter-discard');
  assert.equal(events[0].hasAbsPos, true);
  assert.equal(state.altScreen, false);
  assert.equal(state.inBlock, false);
});

test('alt buffer 突入後: 同じ形のブロックが破棄されず素通しされる（Grok 応答本文）', () => {
  const input = bytes(`${ALT_ENTER}${GROK_STYLE_BLOCK}`);
  const { out, state, events } = filterCursorHideBlocksPure(input, initialState());
  const text = str(out);
  assert.equal(text.includes('きょうは 2099年1月2日(土) です'), true);
  assert.equal(text.includes(HIDE), true);
  assert.equal(text.includes(SHOW), true);
  assert.equal(text.startsWith(ALT_ENTER), true);
  assert.equal(events.length, 1);
  assert.equal(events[0].kind, 'block-passthrough-alt');
  // ライブ進捗行の抽出用に blockBuf がイベントへ渡ること
  assert.equal(str(new Uint8Array(events[0].blockBuf)).includes('きょうは'), true);
  assert.equal(state.altScreen, true);
});

test('alt buffer 退出後: 破棄分岐が復活する', () => {
  const input = bytes(`${ALT_ENTER}${ALT_EXIT}${SPINNER_BLOCK}`);
  const { out, state, events } = filterCursorHideBlocksPure(input, initialState());
  const text = str(out);
  assert.equal(text, `${ALT_ENTER}${ALT_EXIT}`);
  assert.equal(events.length, 1);
  assert.equal(events[0].kind, 'filter-discard');
  assert.equal(state.altScreen, false);
});

test('alt 状態はチャンクを跨いで保持される', () => {
  const first = filterCursorHideBlocksPure(bytes(ALT_ENTER), initialState());
  assert.equal(first.state.altScreen, true);
  const second = filterCursorHideBlocksPure(bytes(GROK_STYLE_BLOCK), first.state);
  assert.equal(str(second.out).includes('きょうは 2099年1月2日(土) です'), true);
  assert.equal(second.events[0].kind, 'block-passthrough-alt');
});

test('チャンク境界で分断された ?1049h も carry で検出される', () => {
  const whole = bytes(ALT_ENTER);
  const cut = 4; // '\x1b[?1' | '049h'
  const first = filterCursorHideBlocksPure(whole.slice(0, cut), initialState());
  assert.equal(first.out.length, 0);
  assert.equal(first.state.carry.length, cut);
  const second = filterCursorHideBlocksPure(whole.slice(cut), first.state);
  assert.equal(second.state.altScreen, true);
  assert.equal(str(second.out), ALT_ENTER);
});

test('回帰: LF 入りブロック（本文描画）は alt 状態に関係なく通過する', () => {
  const block = `${HIDE}\x1b[5;1Hline1\nline2${SHOW}`;
  for (const alt of [false, true]) {
    const { out, events } = filterCursorHideBlocksPure(bytes(block), initialState(alt));
    const text = str(out);
    assert.equal(text.includes('line1\nline2'), true, `alt=${alt} text=${JSON.stringify(text)}`);
    assert.equal(events[0].kind, 'block-passthrough-lf');
  }
});

test('回帰: CUP なしブロック（cursor home 等の初期化）は通過する', () => {
  const block = `${HIDE}\x1b[Hbanner${SHOW}`;
  const { out, events } = filterCursorHideBlocksPure(bytes(block), initialState());
  assert.equal(str(out), block);
  assert.equal(events[0].kind, 'block-passthrough-show');
});

test('回帰: バッファ上限超過ブロックは通過する', () => {
  const filler = 'x'.repeat(MAX_CURSOR_HIDE_BUF + 16);
  const block = `${HIDE}\x1b[3;3H${filler}`;
  const { out, events } = filterCursorHideBlocksPure(bytes(block), initialState());
  assert.equal(str(out).includes(filler.slice(0, MAX_CURSOR_HIDE_BUF)), true);
  assert.equal(events[0].kind, 'block-passthrough-overflow');
});

test('回帰: ブロック途中でチャンクが切れても discard 判定が維持される', () => {
  const whole = bytes(`${SPINNER_BLOCK}tail`);
  const cut = bytes(HIDE).length + 10; // ブロック本文の途中
  const first = filterCursorHideBlocksPure(whole.slice(0, cut), initialState());
  const second = filterCursorHideBlocksPure(whole.slice(cut), first.state);
  assert.equal(str(first.out) + str(second.out), 'tail');
  assert.equal(second.events[0].kind, 'filter-discard');
});
