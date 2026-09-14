// 引き継ぎの帯（残量が閾値を切ったときの通知）が何を見て何を出すかを決める、
// DOM に一切触れない純関数。
//
// derive-dialog-store.ts と同じ分離理由でファイルを分けている: handoff.ts は
// derive-dialog.js を import しており、その先にトップレベルで document を触る
// モジュールがある。DOM の無い Bun テスト環境（web/tests/*.test.ts）から import
// できるのはこちらだけ。
//
// 子 plan: docs/local/plan_derived-session-launch_c4_handoff-routes.md 内部 C2・C3。
//
// **残量の判定はブラウザ側に置いたままにする。** 実測値は usage_stat に既に乗って
// いる（internal/hub/usage_stat.go）ので、Hub に残量を見張る仕組みを増やさない。
import type { Message } from '../types/proto.js';

export interface HandoffUsageWindow {
  usedPercent: number;
  remainingPercent: number;
  windowMinutes: number;
}

/**
 * usage_stat に乗っている使用枠を、通知表示に必要な期間と残量を保ったまま集める。
 */
export function usageWindowsFromUsageStat(m: Message): HandoffUsageWindow[] {
  const out: HandoffUsageWindow[] = [];
  const push = (present: unknown, used: unknown, windowMinutes: number): void => {
    if (!present || typeof used !== 'number' || !Number.isFinite(used)) return;
    const normalizedMinutes = Number.isFinite(windowMinutes) && windowMinutes > 0 ? Math.round(windowMinutes) : 0;
    out.push({ usedPercent: used, remainingPercent: 100 - used, windowMinutes: normalizedMinutes });
  };
  push(m.claude_5h_present, m.rl_5h_pct, 300);
  push(m.claude_7d_present, m.rl_7d_pct, 10080);
  push(m.codex_primary_present, m.codex_primary_used_pct, Number(m.codex_primary_window_minutes || 0));
  push(m.codex_secondary_present, m.codex_secondary_used_pct, Number(m.codex_secondary_window_minutes || 0));
  return out;
}

/**
 * usage_stat に乗っている「使用率%」を全部集める。
 *
 * present フラグを必ず併せて見るのは、0%（まだ 1 度も使っていない）と未取得
 * （その provider がそのフィールドを送っていない）を区別するため。未取得を 0% と
 * 読むと残量 100% に化け、帯が永久に出なくなる。
 */
export function usedPercentsFromUsageStat(m: Message): number[] {
  return usageWindowsFromUsageStat(m).map((window) => window.usedPercent);
}

/** 一番残量が少なく、通知を発火させた使用枠。 */
export function limitingUsageWindowFromUsageStat(m: Message): HandoffUsageWindow | null {
  const windows = usageWindowsFromUsageStat(m);
  if (windows.length === 0) return null;
  return windows.reduce((limiting, window) => (
    window.remainingPercent < limiting.remainingPercent ? window : limiting
  ));
}

/**
 * 残量%。一番使っている窓（5h / 7d / primary / secondary のうち最大）で決める。
 * 使用率が 1 つも読めない usage_stat では null を返す（「残量 100%」ではない）。
 */
export function remainingPercentFromUsageStat(m: Message): number | null {
  return limitingUsageWindowFromUsageStat(m)?.remainingPercent ?? null;
}

/** handoff.note_on_threshold の 3 値（正本は internal/config/config.go）。 */
export type HandoffNoteMode = 'ask' | 'auto' | 'off';

/** 未知の値・空は ask（人が押したときだけ依頼する）に倒す。Hub 側と同じ既定。 */
export function normalizeHandoffNoteMode(value: unknown): HandoffNoteMode {
  const raw = String(value ?? '').trim();
  if (raw === 'auto') return 'auto';
  if (raw === 'off') return 'off';
  return 'ask';
}

/**
 * 帯を出す瞬間に、引き継ぎメモについて何をするか。
 *
 * - button: ボタンを出す。押されたときだけ依頼する（既定）
 * - auto:   帯と同時に 1 回だけ依頼する。ボタンは出さない（押す先がもう無い）
 * - none:   何も出さない・何もしない
 */
export type HandoffNoteAction = 'button' | 'auto' | 'none';

export function handoffNoteActionFor(mode: HandoffNoteMode): HandoffNoteAction {
  if (mode === 'auto') return 'auto';
  if (mode === 'off') return 'none';
  return 'button';
}
