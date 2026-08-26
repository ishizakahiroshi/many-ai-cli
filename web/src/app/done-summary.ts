// done-summary.ts — 完了サマリー（[MANY-AI-CLI-DONE] ブロック）の保持と表示用整形。
//
// 端末への書き戻しをやめた（hub-marker-filter.ts の案 G）ぶんの受け皿。Hub は
// internal/hub/done_summary.go の publishDoneSummary で done_summary を
// **通知設定と無関係に** broadcast しているので、ここで保持して
// ライブ帯（terminal.ts）とセッションカード（session-list.ts）の両方へ配る。
// 端末は「見ているセッション」しか映さないので、カード側が「見ていないセッションの
// 完了に気づく」経路を受け持つ（外部通知を設定していない利用者でもここまでは届く）。
//
// DOM を触らないので、整形の純関数は node:test の fixtures から検証できる。

import type { DoneSummary } from '../types/proto.js';

const summaries = new Map<number, DoneSummary>();

/**
 * 直近の完了サマリーを差し替える。
 *
 * 意図的に「次のターンが始まったら消す」処理を持たない。ターン中はライブ帯もカードも
 * 稼働状態（running / waiting）の表示を優先するので古いサマリーは前に出ず、待機へ戻った
 * ときに「直前に何が終わったか」が読めるほうが実用的なため。消えるのはセッション破棄時だけ。
 */
export function setDoneSummary(id: number, summary: DoneSummary | undefined | null): void {
  if (!summary || !summary.text) return;
  summaries.set(id, summary);
}

export function getDoneSummary(id: number): DoneSummary | undefined {
  return summaries.get(id);
}

export function dropDoneSummary(id: number): void {
  summaries.delete(id);
}

/**
 * kind ごとの記号。文言を持たないので i18n を増やさずに成否が伝わる。
 * kind は Hub 側 classifyDoneSummary が付ける（proto.ts の DoneSummaryKind）。
 */
export function doneSummaryIcon(kind: string | undefined): string {
  switch (kind) {
    case 'failure': return '✗';
    case 'aborted': return '⏹';
    case 'needs_action': return '❓';
    // unknown は Hub のフォールバック（マーカー無しでターンが終わった）。成否を
    // 名乗れないので ✓ にも ❓ にも寄せず、続きが読めないことだけを示す記号にする。
    case 'unknown': return '…';
    default: return '✓';
  }
}

/** CSS 修飾子。未知の kind は success 扱いに寄せる（色が付かないより読める）。 */
export function doneSummaryKindSuffix(kind: string | undefined): string {
  switch (kind) {
    case 'failure': return 'failure';
    case 'aborted': return 'aborted';
    case 'needs_action': return 'needs-action';
    case 'unknown': return 'unknown';
    default: return 'success';
  }
}

/**
 * 1 行へ畳んで長すぎる分を省略する。Hub 側で 320 文字に切られて届くが、
 * カードの狭い行はさらに短くしたいので上限を呼び出し側から渡す。
 */
export function doneSummaryLine(text: string | undefined, maxLen: number): string {
  const one = String(text || '').replace(/\s+/g, ' ').trim();
  if (maxLen <= 0 || one.length <= maxLen) return one;
  return `${one.slice(0, Math.max(1, maxLen - 1))}…`;
}

/** 記号付きの表示文字列。空サマリーでは記号だけを残さず空文字を返す。 */
export function doneSummaryDisplayText(summary: DoneSummary | undefined, maxLen: number): string {
  if (!summary) return '';
  const line = doneSummaryLine(summary.text, maxLen);
  if (!line) return '';
  return `${doneSummaryIcon(summary.kind)} ${line}`;
}
