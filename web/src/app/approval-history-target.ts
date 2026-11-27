// 承認タブの「これまでの承認」の行を押したときの行き先を決める（DOM に触らないので単体で試せる）。
//
// 台帳の session_id は Hub が振るライブの番号で、Hub を起動し直すと同じ番号が別のセッションに
// 付く。番号だけで移動すると関係のないセッションへ飛ぶので、いまの記録と同じ承認か、
// いまのセッションが始まった後に出た承認のときだけ、そのセッションを行き先にする。

/**
 * current: いまも答えを待っている承認（回答パネルを出せる）
 * past: セッションは開いているが、この行はもう答えを待っていない（チャットのその場所へ）
 * gone: この承認を出したセッションはもう開いていない（行き先が無い）
 */
export type ApprovalHistoryTarget = 'current' | 'past' | 'gone';

export function approvalHistoryTarget(row: any, session: any, record: any): ApprovalHistoryTarget {
  if (!row || !session) return 'gone';
  if (record && record.candidate_key
      && String(row.state || '') !== 'resolved'
      && record.candidate_key === row.candidate_key
      && (Number(record.source_epoch) || 0) === (Number(row.source_epoch) || 0)) {
    return 'current';
  }
  if (row.provider && session.provider && row.provider !== session.provider) return 'gone';
  const detected = Date.parse(String(row.detected_at || ''));
  const started = Date.parse(String(session.started_at || ''));
  if (!Number.isFinite(detected) || !Number.isFinite(started)) return 'gone';
  return detected >= started ? 'past' : 'gone';
}

const squash = (value: unknown) => String(value ?? '').replace(/\s+/g, '');

/**
 * 質問文を含む発言を探して添字を返す（無ければ -1）。端末の折り返しで空白と改行の入り方が
 * 変わるので、空白を除いて比べる。同じ質問が何度も出ていたら、検出時刻に近いものを選ぶ。
 */
export function findQuestionMessageIndex(messages: any[], question: string, detectedAt: string): number {
  const needle = squash(String(question || '').split(/\r\n|\r|\n/).find(line => line.trim()) || '');
  if (!needle || !Array.isArray(messages)) return -1;
  const detected = Date.parse(String(detectedAt || ''));
  let best = -1;
  let bestGap = Infinity;
  messages.forEach((msg, index) => {
    if (msg?.role === 'user') return;
    if (!squash(msg?.rawText || msg?.normalizedText).includes(needle)) return;
    const at = new Date(msg?.ts ?? NaN).getTime();
    const gap = Number.isFinite(at) && Number.isFinite(detected) ? Math.abs(at - detected) : Infinity;
    // 時刻を比べられないときは後ろ（新しい方）を残す。
    if (best < 0 || gap <= bestGap) { best = index; bestGap = gap; }
  });
  return best;
}
