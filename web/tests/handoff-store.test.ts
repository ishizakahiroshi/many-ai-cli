import { describe, expect, test } from 'bun:test';
import {
  decideHandoffNotify,
  handoffListMatches,
  handoffNoteActionFor,
  limitingUsageWindowFromUsageStat,
  newHandoffNotifyLedger,
  normalizeHandoffNoteMode,
  orderHandoffList,
  remainingPercentFromUsageStat,
  usageWindowsFromUsageStat,
  usedPercentsFromUsageStat,
} from '../src/app/handoff-store.ts';
import type { HandoffListItem } from '../src/app/handoff-store.ts';
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

describe('limitingUsageWindowFromUsageStat', () => {
  test('keeps the Claude window identity that caused the notification', () => {
    const limiting = limitingUsageWindowFromUsageStat(usage({
      claude_5h_present: true, rl_5h_pct: 3,
      claude_7d_present: true, rl_7d_pct: 93,
    }));
    expect(limiting).toEqual({ usedPercent: 93, remainingPercent: 7, windowMinutes: 10080 });
  });

  test('keeps Codex window_minutes with the limiting percentage', () => {
    const windows = usageWindowsFromUsageStat(usage({
      provider: 'codex',
      codex_primary_present: true, codex_primary_used_pct: 94, codex_primary_window_minutes: 300,
      codex_secondary_present: true, codex_secondary_used_pct: 40, codex_secondary_window_minutes: 10080,
    }));
    expect(windows).toEqual([
      { usedPercent: 94, remainingPercent: 6, windowMinutes: 300 },
      { usedPercent: 40, remainingPercent: 60, windowMinutes: 10080 },
    ]);
    expect(limitingUsageWindowFromUsageStat(usage({
      provider: 'codex',
      codex_primary_present: true, codex_primary_used_pct: 94, codex_primary_window_minutes: 300,
      codex_secondary_present: true, codex_secondary_used_pct: 40, codex_secondary_window_minutes: 10080,
    }))).toEqual({ usedPercent: 94, remainingPercent: 6, windowMinutes: 300 });
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

describe('decideHandoffNotify (once per session x usage window)', () => {
  const both = (pct5h: number, pct7d: number): Message => usage({
    claude_5h_present: true, rl_5h_pct: pct5h,
    claude_7d_present: true, rl_7d_pct: pct7d,
  });

  test('stays quiet while every window is above the threshold', () => {
    const ledger = newHandoffNotifyLedger();
    expect(decideHandoffNotify(ledger, 3, both(50, 40), 10, 'button')).toBeNull();
  });

  test('a session without an id or without readable windows never notifies', () => {
    const ledger = newHandoffNotifyLedger();
    expect(decideHandoffNotify(ledger, 0, both(99, 99), 10, 'button')).toBeNull();
    expect(decideHandoffNotify(ledger, 3, usage({ tokens_total: 1 }), 10, 'button')).toBeNull();
  });

  test('the same window does not notify twice, even as its remaining percent keeps dropping', () => {
    const ledger = newHandoffNotifyLedger();
    const first = decideHandoffNotify(ledger, 3, both(93, 40), 10, 'button');
    expect(first?.window).toEqual({ usedPercent: 93, remainingPercent: 7, windowMinutes: 300 });
    expect(decideHandoffNotify(ledger, 3, both(96, 40), 10, 'button')).toBeNull();
  });

  test('a 7d window crossing the threshold after 5h was notified still notifies, though 5h remains the lower one', () => {
    const ledger = newHandoffNotifyLedger();
    expect(decideHandoffNotify(ledger, 3, both(97, 40), 10, 'button')?.window.windowMinutes).toBe(300);
    // 5h is still the lower window (3% left) but was already notified; 7d (8% left) is new.
    const second = decideHandoffNotify(ledger, 3, both(97, 92), 10, 'button');
    expect(second?.window).toEqual({ usedPercent: 92, remainingPercent: 8, windowMinutes: 10080 });
    expect(decideHandoffNotify(ledger, 3, both(97, 95), 10, 'button')).toBeNull();
  });

  test('two windows crossing at once make one banner for the lower one and mark both as notified', () => {
    const ledger = newHandoffNotifyLedger();
    const got = decideHandoffNotify(ledger, 3, both(95, 92), 10, 'button');
    expect(got?.window.windowMinutes).toBe(300);
    expect(decideHandoffNotify(ledger, 3, both(96, 93), 10, 'button')).toBeNull();
  });

  test('sessions are tracked independently', () => {
    const ledger = newHandoffNotifyLedger();
    expect(decideHandoffNotify(ledger, 3, both(95, 40), 10, 'button')).not.toBeNull();
    expect(decideHandoffNotify(ledger, 4, both(95, 40), 10, 'button')).not.toBeNull();
  });

  test('a window that recovers above the threshold and drops again is not re-announced', () => {
    const ledger = newHandoffNotifyLedger();
    expect(decideHandoffNotify(ledger, 3, both(95, 40), 10, 'button')).not.toBeNull();
    expect(decideHandoffNotify(ledger, 3, both(20, 40), 10, 'button')).toBeNull();
    expect(decideHandoffNotify(ledger, 3, both(95, 40), 10, 'button')).toBeNull();
  });

  test('the threshold itself counts as low (remaining <= threshold)', () => {
    const ledger = newHandoffNotifyLedger();
    expect(decideHandoffNotify(ledger, 3, both(90, 40), 10, 'button')).not.toBeNull();
  });

  test('auto asks for the memo once per session, not once per window', () => {
    const ledger = newHandoffNotifyLedger();
    const first = decideHandoffNotify(ledger, 3, both(95, 40), 10, 'auto');
    expect(first?.requestNote).toBe(true);
    const second = decideHandoffNotify(ledger, 3, both(95, 92), 10, 'auto');
    expect(second?.window.windowMinutes).toBe(10080);
    expect(second?.requestNote).toBe(false);
  });

  test('ask and off never request a memo by themselves', () => {
    const ask = newHandoffNotifyLedger();
    expect(decideHandoffNotify(ask, 3, both(95, 40), 10, handoffNoteActionFor('ask'))?.requestNote).toBe(false);
    const off = newHandoffNotifyLedger();
    expect(decideHandoffNotify(off, 3, both(95, 40), 10, handoffNoteActionFor('off'))?.requestNote).toBe(false);
  });

  test('codex windows are keyed by window_minutes', () => {
    const ledger = newHandoffNotifyLedger();
    const codex = (primary: number, secondary: number): Message => usage({
      provider: 'codex',
      codex_primary_present: true, codex_primary_used_pct: primary, codex_primary_window_minutes: 300,
      codex_secondary_present: true, codex_secondary_used_pct: secondary, codex_secondary_window_minutes: 10080,
    });
    expect(decideHandoffNotify(ledger, 3, codex(95, 40), 10, 'button')?.window.windowMinutes).toBe(300);
    expect(decideHandoffNotify(ledger, 3, codex(95, 93), 10, 'button')?.window.windowMinutes).toBe(10080);
  });
});

function row(partial: Partial<HandoffListItem> & Pick<HandoffListItem, 'sessionID'>): HandoffListItem {
  return {
    live: false,
    providerLabel: 'Claude Code',
    cwd: 'D:\\dev\\github\\public\\many-ai-cli',
    ...partial,
  };
}

describe('orderHandoffList', () => {
  test('running sessions stay above ended ones, each group newest first', () => {
    const source = [
      row({ sessionID: 64 }),
      row({ sessionID: 10, live: true }),
      row({ sessionID: 57 }),
      row({ sessionID: 40, live: true }),
    ];
    const ordered = orderHandoffList(source).map((item) => item.sessionID);
    expect(ordered).toEqual([40, 10, 64, 57]);
    expect(source.map((item) => item.sessionID)).toEqual([64, 10, 57, 40]);
  });
});

describe('handoffListMatches', () => {
  const item = row({ sessionID: 64, providerLabel: 'Codex CLI', cwd: 'D:\\dev\\github\\public\\many-ai-cli' });

  test('blank query keeps every row', () => {
    expect(handoffListMatches(item, '   ')).toBe(true);
  });

  test('number, CLI name, and folder match without case', () => {
    expect(handoffListMatches(item, '64')).toBe(true);
    expect(handoffListMatches(item, '#64')).toBe(true);
    expect(handoffListMatches(item, 'codex cli')).toBe(true);
    expect(handoffListMatches(item, 'MANY-AI-CLI')).toBe(true);
  });

  test('a number inside another id still matches, and a longer number does not', () => {
    expect(handoffListMatches(row({ sessionID: 164 }), '64')).toBe(true);
    expect(handoffListMatches(item, '640')).toBe(false);
    expect(handoffListMatches(item, 'grok')).toBe(false);
  });
});
