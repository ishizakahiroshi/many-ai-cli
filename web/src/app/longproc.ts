// longproc.ts: 「応答が長い」の重大度判定。
//
// サイドバーのカード（session-list.ts）と入力欄上のライブ帯（terminal.ts）が
// **同じ基準**で表示するために 1 箇所へ置く。以前は両方が 300 秒の定数を別々に
// 持っていて、5 分でも 38 分でも同じ「⚠ 長時間処理中」しか出せなかった。
//
// 由来: docs/local/bugfix_codex-long-silence-not-surfaced_2026-08-19.md
// 2026-08-19、Codex セッションがツール引数の生成中に縮退ループへ落ち 38 分無音に
// なったが、重いだけのターンと画面上で区別できなかった。

/** 5 分: 重いターン（巨大 context × 高 effort）でも普通に届く範囲。 */
export const LONGPROC_SEC = 300;
/** 15 分: ここを超えたら表示を強める。 */
export const LONGPROC_SEVERE_SEC = 900;
/**
 * 10 分: provider 自身の transcript がこの時間伸びていなければ「応答が進んでいない」。
 * 経過時間より強い証拠になる。PTY 出力は TUI の再描画で途切れないが、transcript は
 * ターンが進んだときだけ伸びるため。
 */
export const STALL_SEC = 600;

export type LongprocLevel = 'none' | 'warn' | 'severe' | 'stalled';

export interface LongprocStatus {
  level: LongprocLevel;
  /** running に入ってからの経過秒。 */
  elapsedSec: number;
  /** transcript が伸びていない秒数。不明・未特定なら 0。 */
  stalledSec: number;
}

/** transcript が最後に伸びてからの経過秒。未特定（空・不正な値）は 0 を返す。 */
export function transcriptStalledSec(grewAt: string | undefined | null, nowMs: number): number {
  if (!grewAt) return 0;
  const t = Date.parse(grewAt);
  if (!Number.isFinite(t)) return 0;
  return Math.max(0, Math.floor((nowMs - t) / 1000));
}

/**
 * elapsedSec が null（running でない）なら常に 'none'。
 * transcript の停滞は経過時間より優先する（そちらのほうが強い証拠のため）。
 */
export function longprocStatus(
  elapsedSec: number | null,
  grewAt: string | undefined | null,
  nowMs: number,
): LongprocStatus {
  if (elapsedSec == null) return { level: 'none', elapsedSec: 0, stalledSec: 0 };
  const stalledSec = transcriptStalledSec(grewAt, nowMs);
  if (stalledSec >= STALL_SEC) return { level: 'stalled', elapsedSec, stalledSec };
  if (elapsedSec >= LONGPROC_SEVERE_SEC) return { level: 'severe', elapsedSec, stalledSec };
  if (elapsedSec >= LONGPROC_SEC) return { level: 'warn', elapsedSec, stalledSec };
  return { level: 'none', elapsedSec, stalledSec };
}

/**
 * 経過秒を `38m` / `1h05m` の形にする。数字と h/m だけなので i18n を挟まない
 * （ロケールごとの語順に依存しないため）。
 */
export function formatLongprocDuration(sec: number): string {
  const m = Math.max(0, Math.floor(sec / 60));
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return `${h}h${String(m % 60).padStart(2, '0')}m`;
}

/** バッジに付ける CSS クラス。'none' では呼ばない想定。 */
export function longprocBadgeClass(base: string, level: LongprocLevel): string {
  if (level === 'stalled') return `${base} is-stalled`;
  if (level === 'severe') return `${base} is-severe`;
  return base;
}
