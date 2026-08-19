import assert from 'node:assert/strict';
import test from 'node:test';
import {
  LONGPROC_SEC,
  LONGPROC_SEVERE_SEC,
  STALL_SEC,
  formatLongprocDuration,
  longprocBadgeClass,
  longprocStatus,
  transcriptStalledSec,
} from './longproc.js';

const NOW = Date.parse('2026-08-19T17:40:00Z');
const iso = (secondsAgo: number) => new Date(NOW - secondsAgo * 1000).toISOString();

test('longproc: not running (elapsed null) never shows a badge', () => {
  const s = longprocStatus(null, iso(9999), NOW);
  assert.equal(s.level, 'none');
  assert.equal(s.stalledSec, 0);
});

test('longproc: below the 5 minute threshold stays quiet', () => {
  assert.equal(longprocStatus(LONGPROC_SEC - 1, undefined, NOW).level, 'none');
});

test('longproc: 5 minutes warns, 15 minutes escalates', () => {
  assert.equal(longprocStatus(LONGPROC_SEC, undefined, NOW).level, 'warn');
  assert.equal(longprocStatus(LONGPROC_SEVERE_SEC, undefined, NOW).level, 'severe');
});

test('longproc: a stalled transcript outranks elapsed time', () => {
  // 経過はまだ 6 分でも、transcript が 10 分伸びていなければ停滞のほうが強い証拠。
  const s = longprocStatus(LONGPROC_SEC + 60, iso(STALL_SEC), NOW);
  assert.equal(s.level, 'stalled');
  assert.equal(s.stalledSec, STALL_SEC);
});

test('longproc: a growing transcript keeps a long turn at warn, not stalled', () => {
  // 38 分走っていても transcript が 1 分前に伸びていれば「重いだけ」。
  const s = longprocStatus(38 * 60, iso(60), NOW);
  assert.equal(s.level, 'severe');
  assert.equal(s.stalledSec, 60);
});

test('longproc: an unknown transcript time never reports a stall', () => {
  // Hub がまだ transcript を特定できていないケース。停滞 0 として扱い誤報を出さない。
  assert.equal(transcriptStalledSec(undefined, NOW), 0);
  assert.equal(transcriptStalledSec('', NOW), 0);
  assert.equal(transcriptStalledSec('not-a-date', NOW), 0);
  assert.equal(longprocStatus(60 * 60, undefined, NOW).level, 'severe');
});

test('longproc: a future timestamp clamps to zero instead of going negative', () => {
  assert.equal(transcriptStalledSec(iso(-120), NOW), 0);
});

test('longproc: duration format switches to hours past 60 minutes', () => {
  assert.equal(formatLongprocDuration(0), '0m');
  assert.equal(formatLongprocDuration(59), '0m');
  assert.equal(formatLongprocDuration(38 * 60), '38m');
  assert.equal(formatLongprocDuration(60 * 60), '1h00m');
  assert.equal(formatLongprocDuration(95 * 60), '1h35m');
});

test('longproc: badge class carries the level', () => {
  assert.equal(longprocBadgeClass('card-longproc', 'warn'), 'card-longproc');
  assert.equal(longprocBadgeClass('card-longproc', 'severe'), 'card-longproc is-severe');
  assert.equal(longprocBadgeClass('card-longproc', 'stalled'), 'card-longproc is-stalled');
});
