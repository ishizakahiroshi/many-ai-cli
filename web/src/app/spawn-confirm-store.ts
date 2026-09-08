// 子セッション起動の確認待ちを保持する、DOM に一切触れない純粋なストア。
//
// spawn-confirm.ts（ダイアログの markup 生成・イベント配線）から意図的に分離している。
// spawn-confirm.ts は session-list.js を import しており、それが terminal.ts /
// settings.ts / chat-history.ts 等の巨大な import グラフを引き込む。その中には
// `document.addEventListener(...)` のようなトップレベル副作用があるモジュールが含まれ、
// DOM の無い Bun テスト環境（web/tests/*.test.ts、bun:test）で import すると
// ReferenceError で落ちる。ここに置いた純粋関数だけは、その import グラフに触れずに
// 単体テストできる（web/tests/spawn-confirm-store.test.ts）。
//
// plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md C3。

// 子が実際に起動する承認設定。Hub 側で applyChildApprovalDefaults を通して作った
// ものが provider ごとに入っている（internal/hub/orchestration.go の
// childApprovalPreview）。全項目が空なら「Hub は何も足さない」＝ CLI 既定のまま。
export interface ChildApproval {
  permissionMode: string;
  sandbox: string;
  askForApproval: string;
  riskConfirmed: boolean;
}

export interface SpawnConfirmationRecord {
  id: string;
  parentId: number;
  role: string;
  provider: string;
  model: string;
  cwd: string;
  initialPrompt: string;
  requestedAtMs: number;
  // provider をキーにするのは、ダイアログが承認前に provider を差し替えられるため。
  // 選び直した瞬間に、その provider の実効権限へ表示を切り替える。
  approval: Record<string, ChildApproval>;
}

export interface SpawnConfirmationClosedController {
  applyClosed: (m: any) => void;
}

// Hub がまだ持っている（決定が付いていない）確認の一覧。
// spawn_confirmation_requested（新規要求 / 再接続時の再送のどちらも同じ型）で追加し、
// spawn_confirmation_closed で取り除く。
const pendingSpawnConfirmations = new Map<string, SpawnConfirmationRecord>();

// 現在ダイアログとして開いている確認 ID → その結果反映コールバック。
const openDialogControllers = new Map<string, SpawnConfirmationClosedController>();

export function recordFromMessage(m: any): SpawnConfirmationRecord {
  return {
    id: String(m?.spawn_confirmation_id || ''),
    parentId: Number(m?.session_id || 0),
    role: String(m?.role || ''),
    provider: String(m?.provider || ''),
    model: String(m?.model || ''),
    cwd: String(m?.cwd || ''),
    initialPrompt: String(m?.initial_prompt || ''),
    requestedAtMs: Number(m?.spawn_requested_at_ms || 0) || Date.now(),
    approval: approvalsFromMessage(m?.spawn_child_approval),
  };
}

// 旧 Hub から届いた（このフィールドを持たない）メッセージでも落ちないよう、
// 欠けていれば空の表として扱う。呼び出し側は「エントリが無い＝不明」ではなく
// 「何も足されない」と読める形に寄せている。
function approvalsFromMessage(raw: any): Record<string, ChildApproval> {
  const out: Record<string, ChildApproval> = {};
  if (!raw || typeof raw !== 'object') return out;
  for (const [provider, value] of Object.entries(raw as Record<string, any>)) {
    const key = String(provider || '').trim();
    if (!key) continue;
    out[key] = {
      permissionMode: String(value?.permission_mode || ''),
      sandbox: String(value?.sandbox || ''),
      askForApproval: String(value?.ask_for_approval || ''),
      riskConfirmed: Boolean(value?.risk_confirmed),
    };
  }
  return out;
}

// spawn_confirmation_requested を受けたらストアへ積む。同じ ID の再送（UI 再接続時の
// resend）では単に上書きする。
export function noteSpawnConfirmationRequested(m: any): void {
  const record = recordFromMessage(m);
  if (!record.id) return;
  pendingSpawnConfirmations.set(record.id, record);
}

// parentId の保留件数（サイドバーの印・× 無効化の両方から使う）。
export function pendingSpawnConfirmationCount(parentId: number): number {
  let n = 0;
  for (const rec of pendingSpawnConfirmations.values()) {
    if (rec.parentId === parentId) n++;
  }
  return n;
}

// parentId の保留のうち、isOpen(id) が false のものだけを対象に、最も古い
// （requestedAtMs が最小の）ものを 1 件返す。「開いている確認をもう一度開かない」
// 「古い順に開く」の 2 つのルールを、任意の Map を渡して検証できるようにしている。
export function selectOldestPendingConfirmation(
  records: Map<string, SpawnConfirmationRecord>,
  parentId: number,
  isOpen: (id: string) => boolean,
): SpawnConfirmationRecord | null {
  let best: SpawnConfirmationRecord | null = null;
  for (const rec of records.values()) {
    if (rec.parentId !== parentId) continue;
    if (isOpen(rec.id)) continue;
    if (!best || rec.requestedAtMs < best.requestedAtMs) best = rec;
  }
  return best;
}

// spawn-confirm.ts の openNextSpawnConfirmationFor から使う、モジュール内シングルトンの
// ストアに対する薄いラッパー。
export function selectOldestPendingConfirmationFor(parentId: number): SpawnConfirmationRecord | null {
  return selectOldestPendingConfirmation(pendingSpawnConfirmations, parentId, (id) => openDialogControllers.has(id));
}

export function registerDialogController(id: string, controller: SpawnConfirmationClosedController): void {
  if (!id) return;
  openDialogControllers.set(id, controller);
}

export function unregisterDialogController(id: string): void {
  openDialogControllers.delete(id);
}

export function isDialogControllerOpen(id: string): boolean {
  return openDialogControllers.has(id);
}

// spawn_confirmation_closed を受けたらストアから外し、開いているダイアログが
// あればそちらへ結果を渡す。開いていなければ（サイドバーの印を消すだけで）何もしない。
export function closeSpawnConfirmation(m: any): void {
  const id = String(m?.spawn_confirmation_id || '');
  if (!id) return;
  pendingSpawnConfirmations.delete(id);
  const controller = openDialogControllers.get(id);
  if (controller) controller.applyClosed(m);
}

// Hub 再起動検出時にローカル状態を破棄する purgeLocalStateForHubRestart から呼ぶ。
// 開いているダイアログも道連れで閉じる（保留自体が Hub 上で消えているため）。
export function clearAllSpawnConfirmationsForHubRestart(): void {
  pendingSpawnConfirmations.clear();
  for (const [id, controller] of Array.from(openDialogControllers.entries())) {
    controller.applyClosed({ spawn_confirmation_id: id, reason: 'parent_gone' });
  }
}

// After POST /spawn-confirm returns 2xx the Hub has accepted the decision, but
// the dialog still waits for spawn_confirmation_closed. That broadcast can be
// lost if the UI WebSocket is down or stale. HTTP 200 is written before
// performSpawn, so a missing close event must not leave the overlay stuck in
// deciding with Escape disabled and no Close button.
export const SPAWN_CONFIRM_CLOSED_FALLBACK_MS = 4000;

export type SpawnConfirmHttpDecision = {
  waitForCloseBroadcast: boolean;
  fallbackMs: number;
  terminalReason: '' | 'expired' | 'decided_elsewhere' | 'submit_failed';
};

export function spawnConfirmDecisionFromHttp(ok: boolean, status: number): SpawnConfirmHttpDecision {
  if (ok) {
    return { waitForCloseBroadcast: true, fallbackMs: SPAWN_CONFIRM_CLOSED_FALLBACK_MS, terminalReason: '' };
  }
  if (status === 404) {
    return { waitForCloseBroadcast: false, fallbackMs: 0, terminalReason: 'expired' };
  }
  if (status === 409) {
    return { waitForCloseBroadcast: false, fallbackMs: 0, terminalReason: 'decided_elsewhere' };
  }
  return { waitForCloseBroadcast: false, fallbackMs: 0, terminalReason: 'submit_failed' };
}

