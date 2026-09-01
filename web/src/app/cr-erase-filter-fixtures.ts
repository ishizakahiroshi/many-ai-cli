import assert from 'node:assert/strict';
import test from 'node:test';
import {
  filterBareCarriageReturnPure,
  type CarriageReturnFilterState,
} from './cr-erase-filter.js';

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8');

function bytes(s: string): Uint8Array { return encoder.encode(s); }
function str(b: Uint8Array): string { return decoder.decode(b); }
function initialState(altScreen = false): CarriageReturnFilterState {
  return { carry: new Uint8Array(0), altScreen };
}

const EL = '\x1b[K';
const ALT_ENTER = '\x1b[?1049h';
const ALT_EXIT = '\x1b[?1049l';

test('main buffer の単独 CR には EL を挿入する', () => {
  const { out } = filterBareCarriageReturnPure(bytes('100%\rok'), initialState());
  assert.equal(str(out), `100%\r${EL}ok`);
});

test('\\r\\n の CR には挿入しない', () => {
  const { out } = filterBareCarriageReturnPure(bytes('line\r\nnext'), initialState());
  assert.equal(str(out), 'line\r\nnext');
});

// Linux Hub の Claude 黒画面（2026-09-01）。alt buffer 上で「1 行描く → \r で行頭へ →
// カーソル下移動で次の行」と描くため、\r の直後へ EL を入れると描いた行が毎回消える。
test('alt screen 中は EL を挿入しない', () => {
  const input = `${ALT_ENTER}Claude Code v2.1.251\r\x1b[1BOpus 5\r\x1b[1B`;
  const { out, state } = filterBareCarriageReturnPure(bytes(input), initialState());
  assert.equal(str(out), input);
  assert.equal(state.altScreen, true);
});

test('alt screen を抜けたら再び EL を挿入する', () => {
  const input = `${ALT_ENTER}a\r${ALT_EXIT}b\rc`;
  const { out, state } = filterBareCarriageReturnPure(bytes(input), initialState());
  assert.equal(str(out), `${ALT_ENTER}a\r${ALT_EXIT}b\r${EL}c`);
  assert.equal(state.altScreen, false);
});

test('alt 状態はチャンクを跨いで持ち越す', () => {
  const first = filterBareCarriageReturnPure(bytes(`${ALT_ENTER}row1\r`), initialState());
  assert.equal(first.state.altScreen, true);
  const second = filterBareCarriageReturnPure(bytes('row2\rrow3'), first.state);
  assert.equal(str(second.out), '\rrow2\rrow3');
  assert.equal(second.state.altScreen, true);
});

test('チャンク末尾の CR は carry し、次チャンクが LF なら EL を挿入しない', () => {
  const first = filterBareCarriageReturnPure(bytes('done\r'), initialState());
  assert.equal(str(first.out), 'done');
  assert.equal(str(first.state.carry), '\r');
  const second = filterBareCarriageReturnPure(bytes('\nnext'), first.state);
  assert.equal(str(second.out), '\r\nnext');
});

test('チャンク末尾の CR は carry し、次チャンクが LF でなければ EL を挿入する', () => {
  const first = filterBareCarriageReturnPure(bytes('done\r'), initialState());
  const second = filterBareCarriageReturnPure(bytes('ok'), first.state);
  assert.equal(str(second.out), `\r${EL}ok`);
});

test('?1049h がチャンク跨ぎで分割されても alt 判定が効く', () => {
  const first = filterBareCarriageReturnPure(bytes('head\x1b[?10'), initialState());
  assert.equal(str(first.out), 'head');
  assert.equal(str(first.state.carry), '\x1b[?10');
  assert.equal(first.state.altScreen, false);
  const second = filterBareCarriageReturnPure(bytes('49hrow\rnext'), first.state);
  assert.equal(str(second.out), `${ALT_ENTER}row\rnext`);
  assert.equal(second.state.altScreen, true);
});

// 回帰の芯: alt buffer 上の描画バイト数が挿入で増えない = 行が消えない。
test('Claude 風の alt 描画は 1 バイトも増えない', () => {
  const frame = `${ALT_ENTER}\x1b[2J\x1b[H` + Array.from({ length: 8 }, (_, i) => `line${i}\r\x1b[1B`).join('');
  const { out } = filterBareCarriageReturnPure(bytes(frame), initialState());
  assert.equal(out.length, bytes(frame).length);
});

// main buffer 側の防御は落とさない（Codex 等の進捗行の残留対策）。
test('main buffer の連続する上書き行には毎回 EL が入る', () => {
  const { out } = filterBareCarriageReturnPure(bytes('10%\r20%\r5%\r'), initialState());
  assert.equal(str(out), `10%\r${EL}20%\r${EL}5%`);
});
