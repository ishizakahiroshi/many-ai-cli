import assert from 'node:assert/strict';
import test from 'node:test';
import {
  doneSummaryDisplayText,
  doneSummaryIcon,
  doneSummaryKindSuffix,
  doneSummaryLine,
  dropDoneSummary,
  getDoneSummary,
  setDoneSummary,
} from './done-summary.js';
import type { DoneSummary } from '../types/proto.js';

function summary(text: string, kind = 'success'): DoneSummary {
  return { session_id: 1, text, kind, at: '2026-08-26T10:23:16+09:00' };
}

test('setDoneSummary: 本文が空のものは保持しない（記号だけの行を出さないため）', () => {
  dropDoneSummary(1);
  setDoneSummary(1, summary(''));
  assert.equal(getDoneSummary(1), undefined);
  setDoneSummary(1, undefined);
  assert.equal(getDoneSummary(1), undefined);
});

test('setDoneSummary: 直近のものへ差し替わり、drop で消える', () => {
  setDoneSummary(2, summary('1 回目'));
  setDoneSummary(2, summary('2 回目'));
  assert.equal(getDoneSummary(2)?.text, '2 回目');
  dropDoneSummary(2);
  assert.equal(getDoneSummary(2), undefined);
});

test('doneSummaryLine: 改行と連続空白を 1 行へ畳む', () => {
  assert.equal(doneSummaryLine('前半です。\n  後半です。', 100), '前半です。 後半です。');
  assert.equal(doneSummaryLine('  \t 前後の空白 \n', 100), '前後の空白');
  assert.equal(doneSummaryLine(undefined, 100), '');
});

test('doneSummaryLine: 上限を超えたぶんだけ省略記号へ置き換える', () => {
  assert.equal(doneSummaryLine('abcdefghij', 10), 'abcdefghij');
  assert.equal(doneSummaryLine('abcdefghijk', 10), 'abcdefghi…');
  // maxLen <= 0 は「切らない」（呼び出し側が上限を持たないケース）
  assert.equal(doneSummaryLine('abcdefghijk', 0), 'abcdefghijk');
});

test('doneSummaryIcon / doneSummaryKindSuffix: 未知の kind は success 側へ寄せる', () => {
  assert.equal(doneSummaryIcon('failure'), '✗');
  assert.equal(doneSummaryIcon('aborted'), '⏹');
  assert.equal(doneSummaryIcon('needs_action'), '❓');
  assert.equal(doneSummaryIcon('success'), '✓');
  assert.equal(doneSummaryIcon('brand-new-kind'), '✓');
  assert.equal(doneSummaryIcon(undefined), '✓');
  assert.equal(doneSummaryKindSuffix('needs_action'), 'needs-action');
  assert.equal(doneSummaryKindSuffix('brand-new-kind'), 'success');
});

test('doneSummaryDisplayText: 記号 + 1 行。中身が無ければ記号も出さない', () => {
  assert.equal(doneSummaryDisplayText(summary('直しました。', 'success'), 100), '✓ 直しました。');
  assert.equal(doneSummaryDisplayText(summary('落ちています。', 'failure'), 100), '✗ 落ちています。');
  assert.equal(doneSummaryDisplayText(summary('   '), 100), '');
  assert.equal(doneSummaryDisplayText(undefined, 100), '');
});
