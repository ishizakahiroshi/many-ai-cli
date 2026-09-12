import { describe, expect, test } from 'bun:test';
import {
  handoffNoteActionFor,
  normalizeHandoffNoteMode,
  remainingPercentFromUsageStat,
  usedPercentsFromUsageStat,
} from '../src/app/handoff-store.ts';
import type { Message } from '../src/types/proto.ts';

// 子 plan: docs/local/plan_derived-session-launch_c4_handoff-routes.md 内部 C2・C3。
//
// 固定したいのは 2 つ。
//   1. 残量の帯がどの usage_stat で出て、どの usage_stat では出ないか
//      （0% と未取得を取り違えると、帯が永久に出ない or いきなり出る）
//   2. handoff.note_on_threshold の 3 値が画面で何になるか
//
// handoff-store.ts は DOM に触れないので bun:test から直接 import できる
// （derive-dialog-store.test.ts と同じ理由）。

function usage(overrides: Partial<Message> = {}): Message {
  return { type: 'usage_stat', session_id: 3, provider: 'claude', ...overrides } as Message;
}

describe('usedPercentsFromUsageStat', () => {
  test('claude reports its 5h and 7d windows', () => {
    const got = usedPercentsFromUsageStat(usage({
      claude_5h_present: true, rl_5h_pct: 92,
      claude_7d_present: true, rl_7d_pct: 40,
    }));
    expect(got).toEqual([92, 40]);
  });

  test('codex reports its primary and secondary windows', () => {
    const got = usedPercentsFromUsageStat(usage({
      provider: 'codex',
      codex_primary_present: true, codex_primary_used_pct: 55,
      codex_secondary_present: true, codex_secondary_used_pct: 96,
    }));
    expect(got).toEqual([55, 96]);
  });

  test('a percentage without its present flag is not read as 0% used', () => {
    expect(usedPercentsFromUsageStat(usage({ rl_5h_pct: 0, codex_primary_used_pct: 0 }))).toEqual([]);
  });
});

describe('remainingPercentFromUsageStat', () => {
  test('the busiest window decides the remaining percentage', () => {
    const remaining = remainingPercentFromUsageStat(usage({
      claude_5h_present: true, rl_5h_pct: 93,
      claude_7d_present: true, rl_7d_pct: 12,
    }));
    expect(remaining).toBe(7);
  });

  test('a usage_stat carrying no rate limits is unknown, not full', () => {
    expect(remainingPercentFromUsageStat(usage({ tokens_total: 1200 }))).toBeNull();
  });

  test('a window at 0% used is a real 100% remaining', () => {
    expect(remainingPercentFromUsageStat(usage({ claude_5h_present: true, rl_5h_pct: 0 }))).toBe(100);
  });
});

describe('handoff.note_on_threshold', () => {
  test('unset and misspelled values stay on the conservative default', () => {
    expect(normalizeHandoffNoteMode(undefined)).toBe('ask');
    expect(normalizeHandoffNoteMode('')).toBe('ask');
    expect(normalizeHandoffNoteMode('AUTO')).toBe('ask');
    expect(normalizeHandoffNoteMode('none')).toBe('ask');
  });

  test('the three accepted values round-trip', () => {
    expect(normalizeHandoffNoteMode('ask')).toBe('ask');
    expect(normalizeHandoffNoteMode('auto')).toBe('auto');
    expect(normalizeHandoffNoteMode('off')).toBe('off');
  });

  test('ask offers a button, auto asks by itself, off does neither', () => {
    expect(handoffNoteActionFor('ask')).toBe('button');
    expect(handoffNoteActionFor('auto')).toBe('auto');
    expect(handoffNoteActionFor('off')).toBe('none');
  });
});
