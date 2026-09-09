import assert from 'node:assert/strict';
import test from 'node:test';
import {
  encodeWheelSeq,
  initialMouseModeTrackerState,
  isX10CoordinateSafe,
  scanMouseModePure,
} from './mouse-mode-tracker.js';

const encoder = new TextEncoder();
const bytes = (text: string): Uint8Array => encoder.encode(text);

test('mouse-mode: ?1000h で追跡を有効にする', () => {
  const state = scanMouseModePure(bytes('\x1b[?1000h'), initialMouseModeTrackerState());
  assert.equal(state.tracking, true);
  assert.equal(state.sgr, false);
});

test('mouse-mode: ?1006h で SGR を有効にする', () => {
  let state = scanMouseModePure(bytes('\x1b[?1000h'), initialMouseModeTrackerState());
  state = scanMouseModePure(bytes('\x1b[?1006h'), state);
  assert.equal(state.tracking, true);
  assert.equal(state.sgr, true);
});

test('mouse-mode: ?1000l で追跡を無効に戻す', () => {
  let state = scanMouseModePure(bytes('\x1b[?1000h'), initialMouseModeTrackerState());
  state = scanMouseModePure(bytes('\x1b[?1000l'), state);
  assert.equal(state.tracking, false);
});

test('mouse-mode: チャンク境界を跨いでも検出する', () => {
  let state = scanMouseModePure(bytes('\x1b[?10'), initialMouseModeTrackerState());
  assert.equal(state.sgr, false);
  state = scanMouseModePure(bytes('06h'), state);
  assert.equal(state.sgr, true);
  assert.equal(state.carry.length, 0);
});

test('mouse-mode: 同一チャンクでは後に出た h/l が勝つ', () => {
  let state = scanMouseModePure(bytes('\x1b[?1000h\x1b[?1000l'), initialMouseModeTrackerState());
  assert.equal(state.tracking, false);
  state = scanMouseModePure(bytes('\x1b[?1000l\x1b[?1000h'), initialMouseModeTrackerState());
  assert.equal(state.tracking, true);
});

test('mouse-mode: 無関係な ?25l では状態を変えない', () => {
  const initial = { carry: new Uint8Array(0), tracking: true, sgr: true, urxvt: true };
  const state = scanMouseModePure(bytes('\x1b[?25l'), initial);
  assert.deepEqual(state, initial);
});

test('mouse-mode: SGR ホイールは中央座標で上方向をエンコードする', () => {
  assert.equal(encodeWheelSeq(-1, 80, 24, 'sgr'), '\x1b[<64;40;12M');
});

test('mouse-mode: X10 ホイールは座標を 223 で制限する', () => {
  assert.equal(
    encodeWheelSeq(1, 1000, 1000, 'x10'),
    `\x1b[M${String.fromCharCode(97)}${String.fromCharCode(255)}${String.fromCharCode(255)}`,
  );
});

test('mouse-mode: 追跡なしは従来の PageDown を維持する', () => {
  assert.equal(encodeWheelSeq(1, 80, 24, 'page'), '\x1b[6~');
});

test('mouse-mode: isX10CoordinateSafe は通常サイズの端末で安全と判定する', () => {
  assert.equal(isX10CoordinateSafe(80, 24), true);
});

test('mouse-mode: isX10CoordinateSafe は cols の中央座標が 96 になると不安全と判定する', () => {
  assert.equal(isX10CoordinateSafe(192, 24), false);
});

test('mouse-mode: isX10CoordinateSafe は中央座標 95（境界内）では安全と判定する', () => {
  assert.equal(isX10CoordinateSafe(190, 24), true);
});

test('mouse-mode: isX10CoordinateSafe は rows 側でも同じ判定になる', () => {
  assert.equal(isX10CoordinateSafe(24, 192), false);
});

test('mouse-mode: 終端の来ない CSI ? が carry 上限を超えると捨てられる', () => {
  let state = scanMouseModePure(bytes(`\x1b[?${'1'.repeat(50)}`), initialMouseModeTrackerState());
  assert.ok(state.carry.length > 0);
  state = scanMouseModePure(bytes('2'.repeat(50)), state);
  assert.equal(state.carry.length, 0);
  assert.equal(state.tracking, false);
  assert.equal(state.sgr, false);
});

test('mouse-mode: carry 上限以内の未完了シーケンスは保持され次のチャンクで完了する', () => {
  let state = scanMouseModePure(bytes('\x1b[?100'), initialMouseModeTrackerState());
  assert.ok(state.carry.length > 0 && state.carry.length <= 10);
  state = scanMouseModePure(bytes('6h'), state);
  assert.equal(state.sgr, true);
  assert.equal(state.carry.length, 0);
});
