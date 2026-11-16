// Hub -> 全画面の「AI CLI の登録が変わった」通知（WebSocket の providers_updated）。
// 別のタブ・別の端末・API・CLI で登録を足す・直す・消すと、Hub が開いている画面すべてへ
// 送る。画面は document のイベントに直して、新しいセッションの一覧と設定の一覧が読み直す。
// 受け取りは ws-client.ts、読み直しは新しいセッションの一覧と、設定の AI 一覧。

export const PROVIDERS_UPDATED_EVENT = 'providers-updated';

export interface ProvidersUpdatedDetail {
  ids: string[];
  action: string;
}

// WebSocket のメッセージを、画面内のイベントの詳細へ直す。中身が崩れていても読み直し自体は
// 要るので、ids が配列でなければ空、action が文字列でなければ空にして返す。
export function providersUpdatedDetail(message: { providers?: unknown; reason?: unknown }): ProvidersUpdatedDetail {
  const ids = Array.isArray(message.providers)
    ? message.providers.filter((id): id is string => typeof id === 'string' && id.length > 0)
    : [];
  const action = typeof message.reason === 'string' ? message.reason : '';
  return { ids, action };
}
