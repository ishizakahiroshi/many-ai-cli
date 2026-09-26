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

/**
 * 帯をもう出したかの台帳。帯は「セッション × 使用枠（窓）」ごとに 1 回出す。
 *
 * セッション単位 1 回だと、5h の帯を出した後に猶予枠などで 7d が閾値を切っても、
 * 7d の帯が出ない（別の窓の話なのに、通知済みで塞がれる）。
 *
 * 残量が閾値を上回り直しても通知済みは解除しない。同じタブで同じ窓の帯を出し直すと、
 * 残量が閾値の付近で上下するたびに帯が点滅するため（従来の「同じタブでは再通知しない」を
 * 窓単位に細かくしただけで、出し直しの条件は変えていない）。
 */
export interface HandoffNotifyLedger {
  /** `${sessionID}:${windowMinutes}` を通知済みとして持つ。 */
  windows: Set<string>;
  /** auto でメモを依頼したセッション。窓が変わっても前任のトークンを二重に使わない。 */
  noteRequested: Set<number>;
}

export function newHandoffNotifyLedger(): HandoffNotifyLedger {
  return { windows: new Set(), noteRequested: new Set() };
}

export function handoffNotifyKey(sessionID: number, windowMinutes: number): string {
  return `${sessionID}:${windowMinutes}`;
}

export interface HandoffNotifyDecision {
  /** 帯に出す窓（閾値以下で未通知の窓のうち、残量が最少のもの）。 */
  window: HandoffUsageWindow;
  /** この帯と同時に前任へメモを依頼するか（auto のセッション初回だけ true）。 */
  requestNote: boolean;
}

/**
 * usage_stat 1 件について、いま帯を出すか・メモを依頼するかを決め、台帳へ記録する。
 *
 * 閾値以下の窓が同時に複数あれば帯は 1 枚（一番残量が少ない窓）で、閾値以下の窓は
 * すべて通知済みにする。未通知の窓だけを候補にするので、5h が既に通知済みでもなお
 * 5h のほうが 7d より残量が少ない状況で、7d が閾値を切れば 7d の帯が出る。
 */
export function decideHandoffNotify(
  ledger: HandoffNotifyLedger,
  sessionID: number,
  m: Message,
  thresholdPercent: number,
  action: HandoffNoteAction,
): HandoffNotifyDecision | null {
  if (!sessionID) return null;
  const below = usageWindowsFromUsageStat(m).filter((w) => w.remainingPercent <= thresholdPercent);
  const fresh = below.filter((w) => !ledger.windows.has(handoffNotifyKey(sessionID, w.windowMinutes)));
  if (fresh.length === 0) return null;
  for (const w of below) ledger.windows.add(handoffNotifyKey(sessionID, w.windowMinutes));
  const window = fresh.reduce((a, b) => (b.remainingPercent < a.remainingPercent ? b : a));
  const requestNote = action === 'auto' && !ledger.noteRequested.has(sessionID);
  if (requestNote) ledger.noteRequested.add(sessionID);
  return { window, requestNote };
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

/** 引き継ぎ一覧の 1 行。検索は画面に出る CLI 名とフォルダを見る。 */
export interface HandoffListItem {
  sessionID: number;
  live: boolean;
  providerLabel: string;
  cwd: string;
}

/**
 * 一覧の表示順。実行中を上、終了を下にし、それぞれの中はセッション番号の新しい順。
 * 引数の配列は並べ替えない。
 */
export function orderHandoffList<T extends Pick<HandoffListItem, 'sessionID' | 'live'>>(items: readonly T[]): T[] {
  return [...items].sort((a, b) => {
    if (a.live !== b.live) return a.live ? -1 : 1;
    return b.sessionID - a.sessionID;
  });
}

/**
 * 検索文字列が番号、画面に出る CLI 名、フォルダパスのどれかに含まれるか。
 * 空や空白だけなら全部残す。大文字と小文字は区別しない。番号は「64」でも「#64」でも当たる。
 */
export function handoffListMatches(item: HandoffListItem, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const id = String(item.sessionID);
  return [id, `#${id}`, item.providerLabel, item.cwd].join('\n').toLowerCase().includes(q);
}
