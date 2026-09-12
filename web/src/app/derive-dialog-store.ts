// 派生起動（既存セッションを起点に新しいセッションを 1 本立てる操作）の送信内容を
// 決める、DOM に一切触れない純関数。
//
// spawn-confirm-store.ts と同じ分離理由でファイルを分けている: derive-dialog.ts は
// session-list.js / spawn-confirm.js を import しており、その先にトップレベルで
// document を触るモジュールがある。DOM の無い Bun テスト環境（web/tests/*.test.ts）
// から import できるのはこちらだけ。
//
// 子 plan: docs/local/plan_derived-session-launch_c3_derive-launch.md 内部 C2。
//
// 「種別で何が載るか」を目で追えるところに置くのがこのファイルの目的。引き継ぎと子は
// 呼ぶ API が別（/api/spawn と /api/sessions/:id/spawn-child）で、片方にしか無い項目
// （handoff_from / origin / role / same_tree）がある。ダイアログの DOM 側に条件分岐を
// 散らすと、欄を 1 つ足すたびに「どちらに載るのか」が読み取れなくなる。
import { effortForSpawnBody } from './spawn-confirm-store.js';
import { compatibleModelOrEmpty, isModelCompatibleWithProvider, type SpawnModelGroup } from './spawn-model-groups.js';

export type DeriveKind = 'handoff' | 'child';

// origin の値は Hub の internal/hub/child_permission.go（launchOriginUI）が正本。
// 画面から立てた子だけがこれを載せ、Hub は確認ダイアログを省いて段 1 で起動する。
export const DERIVE_ORIGIN_UI = 'ui';

export interface DeriveSelection {
  kind: DeriveKind;
  /** 起点セッション。引き継ぎでは前任、子では親。 */
  sourceSessionID: number;
  provider: string;
  model?: string;
  /** 子のみ。引き継ぎは対等な新しい親なので役割を持たない。 */
  role?: string;
  effort?: string;
  executionMode?: string;
  permissionPreset?: string;
  subscriptionProfileID?: string;
  /** 子のみ。true = worktree を使わず親の cwd で動かす。 */
  sameTree?: boolean;
  /**
   * 子のみ。「この役割では次回もこの段を使う」チェックボックスの状態
   * （子 plan: docs/local/plan_derived-session-launch_c2_permission-tiers.md 内部 C6）。
   * undefined = 欄を持たない呼び出し = Hub の記憶を触らない。true = 選んだ段を
   * その役割の記憶にする（「指定なし」で起動したなら記憶を消す）。false = 記憶を消す。
   * 引き継ぎには役割が無いので載らない。
   */
  rememberPermission?: boolean;
  /** 引き継ぎのみ。前任と同じ作業ディレクトリ。子は Hub 側が親 cwd / worktree を決める。 */
  cwd?: string;
  /** 起動前にダイアログで全文が見えている文面（親 plan 不変条件 3）。 */
  prompt: string;
  /**
   * 引き継ぎのみ。/api/spawn が 400 risk_confirmation_required を返した後の再送で
   * だけ true にする（New Session フォームと同じ作法）。子は spawn-child 側が段の表
   * から自分で決めるので、ここから送ることはない。
   */
  riskConfirmed?: boolean;
}

function trimmed(value: unknown): string {
  return String(value ?? '').trim();
}

/** 種別ごとの送信先。引き継ぎは検証済みの通常起動、子は orchestration の入口。 */
export function deriveRequestPath(selection: DeriveSelection): string {
  if (selection.kind === 'handoff') return '/api/spawn';
  return `/api/sessions/${encodeURIComponent(String(selection.sourceSessionID))}/spawn-child`;
}

/**
 * 送信 body を組み立てる。
 *
 * 省略可の項目は「空なら**キーごと**載せない」。false や空文字を送ると、その項目を
 * 知らなかった頃の呼び出しと JSON の形が変わり、Hub 側の「指定なし」が「明示的に
 * 指定なしを選んだ」に化ける（親 plan 不変条件 1）。same_tree がその代表で、
 * 未チェックのときに false を送ると orchestration.worktree_auto の設定を黙って
 * 上書きしてしまう。
 */
export function buildDeriveBody(
  selection: DeriveSelection,
  groups?: SpawnModelGroup[] | null,
): Record<string, unknown> {
  const provider = trimmed(selection.provider);
  const prompt = String(selection.prompt ?? '');
  const body: Record<string, unknown> = { provider };

  if (selection.kind === 'handoff') {
    body.cwd = trimmed(selection.cwd);
    body.initial_prompt = prompt;
    body.handoff_from = Number(selection.sourceSessionID) || 0;
    if (selection.riskConfirmed) body.risk_confirmed = true;
  } else {
    body.role = trimmed(selection.role);
    body.origin = DERIVE_ORIGIN_UI;
    if (prompt.trim()) body.initial_prompt = prompt;
    if (selection.sameTree) body.same_tree = true;
    // same_tree と違い false も意味を持つ（記憶を消す）ので、undefined のときだけ
    // キーごと落とす。欄を知らない呼び出しでは Hub の記憶は 1 バイトも動かない。
    if (typeof selection.rememberPermission === 'boolean') {
      body.remember_permission = selection.rememberPermission;
    }
  }

  const model = compatibleModelOrEmpty(groups, provider, trimmed(selection.model));
  if (model) body.model = model;
  // effort は写像のある provider でだけ載る。写像の無い provider へ前の選択が残った
  // まま送ると Hub が 400 で弾くので、ここで落とす（正本は /api/info の effort_levels）。
  const effort = effortForSpawnBody(provider, trimmed(selection.effort));
  if (effort) body.effort = effort;
  const executionMode = trimmed(selection.executionMode);
  if (executionMode) body.execution_mode = executionMode;
  const permissionPreset = trimmed(selection.permissionPreset);
  if (permissionPreset) body.permission_preset = permissionPreset;
  const subscription = trimmed(selection.subscriptionProfileID);
  if (subscription) body.subscription_profile_id = subscription;
  return body;
}

/**
 * 起点が終了していると子は立てられない（親が Hub の session 表に居ないと
 * spawn-child が 404 になる）。終了済みのセッションからは引き継ぎだけを出す。
 */
export function availableDeriveKinds(sourceLive: boolean): DeriveKind[] {
  return sourceLive ? ['handoff', 'child'] : ['handoff'];
}

/**
 * 起動ボタンを押せない理由。空文字なら押せる。
 * 引き継ぎは文面が要る（前任の看板を渡さない引き継ぎに意味が無い）。子は役割が要る
 * （Hub が role を必須にしている）。文面は空でもよい。
 */
export function deriveSubmitBlockedReason(
  selection: DeriveSelection,
  groups?: SpawnModelGroup[] | null,
): '' | 'provider' | 'role' | 'prompt' | 'model' {
  if (!trimmed(selection.provider)) return 'provider';
  const model = trimmed(selection.model);
  if (model && !isModelCompatibleWithProvider(groups, selection.provider, model)) return 'model';
  if (selection.kind === 'child') {
    return trimmed(selection.role) ? '' : 'role';
  }
  return String(selection.prompt ?? '').trim() ? '' : 'prompt';
}
