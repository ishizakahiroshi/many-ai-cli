// 「AI CLI が足された」知らせの、画面に触れない部分（足した印と、実行するコマンドの 1 行）。
// provider-added-notice.ts が使い、node の fixture からも読めるよう DOM と i18n を import しない。
import type { ProvidersUpdatedDetail } from './providers-updated.js';

// この画面が足した印の有効時間。これを過ぎた印は無視する（保存が失敗して残った印で、
// 後から別の経路で同じ id が足されたときの知らせを隠さないため）。
export const OWN_CREATION_WINDOW_MS = 60_000;
const COMMAND_SHOWN_MAX = 160;

const creating = new Map<string, number>();

export function markProviderCreating(id: string, now: number = Date.now()): void {
  if (id) creating.set(id, now);
}

// この画面が足した id なら true を返して印を外す。期限切れの印は外すだけで false。
export function consumeOwnCreation(
  id: string,
  now: number = Date.now(),
  marks: Map<string, number> = creating,
): boolean {
  const at = marks.get(id);
  if (at === undefined) return false;
  marks.delete(id);
  return now - at <= OWN_CREATION_WINDOW_MS;
}

// 知らせるべき id。created だけが対象で、この画面が足したものは除く。
export function idsToAnnounce(
  detail: ProvidersUpdatedDetail,
  now: number = Date.now(),
  marks: Map<string, number> = creating,
): string[] {
  if (detail.action !== 'created') return [];
  return detail.ids.filter((id) => !consumeOwnCreation(id, now, marks));
}

// 取り消しが断られたときに出す文言の鍵。取り消しは「知らせた時点の登録の版」を添えて送る
// ので、Hub が版の食い違い（409）で断ったなら、知らせたあとに登録が変わっている
// （同じ id が足し直された等）。その新しい登録を消さず、そう伝える。
export function undoFailureMessageKey(failure: {
  ok?: boolean;
  kind?: string;
  status?: number;
}): 'provider_added_undo_changed' | 'provider_added_undo_failed' {
  return failure.kind === 'http' && failure.status === 409
    ? 'provider_added_undo_changed'
    : 'provider_added_undo_failed';
}

function quoteIfNeeded(part: string): string {
  return part === '' || /\s/.test(part) ? JSON.stringify(part) : part;
}

// 登録の launch から「実行するコマンド」の 1 行を作る。長すぎるときは切る。
export function launchCommandLine(launch: unknown): string {
  if (!launch || typeof launch !== 'object') return '';
  const record = launch as { executable?: unknown; args?: unknown };
  const parts: string[] = [];
  if (typeof record.executable === 'string' && record.executable) parts.push(quoteIfNeeded(record.executable));
  if (Array.isArray(record.args)) {
    for (const arg of record.args) if (typeof arg === 'string') parts.push(quoteIfNeeded(arg));
  }
  const line = parts.join(' ');
  return line.length > COMMAND_SHOWN_MAX ? `${line.slice(0, COMMAND_SHOWN_MAX)}…` : line;
}
