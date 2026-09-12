// 送信履歴モーダルの描画先が、復元を開始したセッションと同じインスタンスか判定する。
// セッション番号だけでは、同じセッションのモーダルを閉じて開き直した場合に古い
// 非同期復元を区別できないため、セッション番号とインスタンス番号の両方を照合する。

export interface SentHistoryModalOwner {
  readonly sessionId: number;
  readonly instanceId: number;
}

export interface SentHistoryModalTarget {
  readonly sentHistorySessionId?: string;
  readonly sentHistoryModalInstanceId?: string;
}

export function sentHistoryModalOwnerMatches(
  target: SentHistoryModalTarget | null | undefined,
  owner: SentHistoryModalOwner,
): boolean {
  return target?.sentHistorySessionId === String(owner.sessionId)
    && target.sentHistoryModalInstanceId === String(owner.instanceId);
}
