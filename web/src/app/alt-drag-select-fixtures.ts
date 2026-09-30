import assert from 'node:assert/strict';
import test from 'node:test';
import { measureAltDragShift, stepAltDragAnchor, type AltDragAnchor } from './alt-drag-select.js';

// 全画面モードの Claude Code と同じ形の画面を作る: 上 7 行が本文（CLI 自身が送る）、
// 下 3 行が固定の入力欄とステータス行。本文は doc の offset 行目から並ぶ。
const PANE = 7;
const COLS = 80;
const doc = Array.from({ length: 40 }, (_, i) => `本文の ${i} 行目です（line ${i}）`);

function screen(offset: number, status = '$1.23  Sonnet 5.5'): string[] {
  const rows: string[] = [];
  for (let i = 0; i < PANE; i++) rows.push(doc[offset + i] ?? '');
  rows.push('────────────────────────────────');
  rows.push('> ');
  rows.push(status);
  return rows;
}

test('alt-drag: 下へ 3 行送った画面は moved 3', () => {
  assert.deepEqual(measureAltDragShift(screen(10), screen(13), 1), { kind: 'moved', lines: 3 });
});

test('alt-drag: 上へ 3 行送った画面は moved 3', () => {
  assert.deepEqual(measureAltDragShift(screen(10), screen(7), -1), { kind: 'moved', lines: 3 });
});

test('alt-drag: ステータス行だけ変わった画面は still（履歴の端で送りが効かない）', () => {
  assert.deepEqual(measureAltDragShift(screen(10), screen(10, '$1.24  Sonnet 5.5'), 1), { kind: 'still' });
});

test('alt-drag: 何も変わらない画面は still', () => {
  assert.deepEqual(measureAltDragShift(screen(10), screen(10), 1), { kind: 'still' });
});

test('alt-drag: 本文がまるごと別物になった画面は lost（推測で埋めない）', () => {
  const other = screen(10).map((line, i) => (i < PANE ? `別の画面 ${i} です（other ${i}）` : line));
  assert.deepEqual(measureAltDragShift(screen(10), other, 1), { kind: 'lost' });
});

test('alt-drag: 罫線の繰り返しだけではずれ幅の根拠にしない', () => {
  const rule = '────────────────────────────────';
  const before = [rule, rule, rule, rule, rule, '> ', 'status line'];
  const after = [rule, rule, rule, rule, rule, '> ', 'status line 2'];
  assert.notEqual(measureAltDragShift(before, after, 1).kind, 'moved');
});

test('alt-drag: 下へ送って起点が見えたままなら、起点を上へずらすだけ', () => {
  const step = stepAltDragAnchor(screen(10), screen(13), 1, 3, { col: 4, row: 5 }, 'unused', COLS);
  assert.deepEqual(step, { leftLines: [], anchor: { col: 4, row: 2 } });
});

test('alt-drag: 下へ送って起点が画面の上へ出たら、出た行を覚えて起点を本文の先頭へ', () => {
  const anchorPart = doc[11].slice(2);
  const step = stepAltDragAnchor(screen(10), screen(13), 1, 3, { col: 4, row: 1 }, anchorPart, COLS);
  assert.deepEqual(step, { leftLines: [anchorPart, doc[12]], anchor: { col: 0, row: 0 } });
});

test('alt-drag: 上へ送って起点が入力欄の下へ隠れたら、出た行を覚えて起点を本文の末尾へ', () => {
  const anchorPart = doc[15].slice(0, 3);
  const step = stepAltDragAnchor(screen(10), screen(7), -1, 3, { col: 6, row: 5 }, anchorPart, COLS);
  assert.deepEqual(step, { leftLines: [doc[14], anchorPart], anchor: { col: COLS, row: 6 } });
});

test('alt-drag: 行末のスクロール位置の記号が動いても、見えている起点の行を見失わない', () => {
  // CLI が行末に描く位置の記号（▐）が、送る前は 1 行目、送った後は 2 行目にある。
  const before = screen(10).map((line, i) => (i === 1 ? `${line} ▐` : line));
  const after = screen(13).map((line, i) => (i === 2 ? `${line} ▐` : line));
  assert.deepEqual(measureAltDragShift(before, after, 1), { kind: 'moved', lines: 3 });
  const step = stepAltDragAnchor(before, after, 1, 3, { col: 4, row: 5 }, 'unused', COLS);
  assert.deepEqual(step, { leftLines: [], anchor: { col: 4, row: 2 } });
});

test('alt-drag: 送った後の画面に起点から先が 1 行も残っていなければ null', () => {
  const blank = screen(10).map(() => '');
  assert.equal(stepAltDragAnchor(screen(10), blank, 1, 3, { col: 0, row: 1 }, doc[11], COLS), null);
});

test('alt-drag: 何段送っても、覚えた行と見えている選択をつなぐと元の本文と一致する', () => {
  // 11 行目の 2 文字目から下へドラッグし、3 行ずつ 5 段送ってから 32 行目の末尾で離す。
  let offset = 10;
  let anchor: AltDragAnchor = { col: 2, row: 1 };
  const carried: string[] = [];
  for (let n = 0; n < 5; n++) {
    const before = screen(offset);
    const after = screen(offset + 3);
    const shift = measureAltDragShift(before, after, 1);
    assert.equal(shift.kind, 'moved');
    if (shift.kind !== 'moved') return;
    const line = doc[offset + anchor.row];
    const step = stepAltDragAnchor(before, after, 1, shift.lines, anchor, line.slice(anchor.col), COLS);
    assert.ok(step);
    if (!step) return;
    carried.push(...step.leftLines);
    anchor = step.anchor;
    offset += 3;
  }
  // 見えている選択: 起点の行から、離した行（本文 6 行目 = doc[offset + 6]）まで。
  const visible: string[] = [];
  for (let row = anchor.row; row < PANE; row++) {
    const line = doc[offset + row];
    visible.push(row === anchor.row ? line.slice(anchor.col) : line);
  }
  const expected = [doc[11].slice(2), ...doc.slice(12, offset + PANE)];
  assert.deepEqual([...carried, ...visible], expected);
});
