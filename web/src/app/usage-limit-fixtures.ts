import assert from 'node:assert/strict';
import test from 'node:test';
import {
  clampPercent,
  normalizeRemainingPercent,
  remainingPercent,
  resetState,
  sortUsageWindows,
  usageSeverity,
  usedPercent,
  windowLabel,
} from './usage-limit.js';

test('usage limit helpers normalize provider semantics and preserve zero', () => {
  assert.equal(normalizeRemainingPercent(37, 'used'), 63);
  assert.equal(normalizeRemainingPercent(63, 'remaining'), 63);
  assert.equal(normalizeRemainingPercent(37, 'unknown'), null);
  assert.equal(clampPercent(-5), 0);
  assert.equal(clampPercent(105), 100);
  assert.equal(remainingPercent({ used_percent: 0 }), 100);
  assert.equal(remainingPercent({ remaining_percent: 0 }), 0);
  assert.equal(remainingPercent({ used_percent: 37, remaining_percent: 63 }), 63);
  assert.equal(usedPercent({ remaining_percent: 63 }), 37);
});

test('usage limit helpers label and sort arbitrary windows', () => {
  assert.deepEqual(windowLabel(300), { kind: 'five_hour' });
  assert.deepEqual(windowLabel(10080), { kind: 'weekly' });
  assert.deepEqual(windowLabel(90), { kind: 'minutes', amount: 90 });
  const windows = sortUsageWindows([
    { used_percent: 20, window_minutes: 10080 },
    { used_percent: 10, window_minutes: 300 },
    { used_percent: 30, window_minutes: 90 },
  ]);
  assert.deepEqual(windows.map((window) => window.window_minutes), [90, 300, 10080]);
});

test('usage limit helpers expose severity and reset staleness', () => {
  assert.equal(usageSeverity(26), 'normal');
  assert.equal(usageSeverity(25), 'warning');
  assert.equal(usageSeverity(10), 'danger');
  assert.equal(usageSeverity(0), 'danger');
  const now = 1_800_000_000_000;
  assert.deepEqual(resetState((now + 90 * 60_000) / 1000, now), {
    epoch: now + 90 * 60_000,
    stale: false,
    remainingMs: 90 * 60_000,
  });
  assert.equal(resetState((now - 1) / 1000, now)?.stale, true);
});
