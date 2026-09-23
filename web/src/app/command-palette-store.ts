// command-palette-store.ts — Ctrl+K パレットに乗せる「コマンド」の定義・絞り込み・
// 選択位置の計算を、DOM と window に一切触れない純関数として切り出したもの。
//
// derive-dialog-store.ts / turn-end-notify-store.ts と同じ分離理由: session-search-palette.ts
// は document を触るモジュール群を import しており、DOM の無い Bun テスト環境
// （web/tests/*.test.ts）から import できるのはこちら側だけ。
//
// 子 plan: docs/local/plan_ux-notify-palette-review_c3_palette.md 内部 C1。
// DOM への配線・実際の翻訳（t()）・既存関数の呼び出しは同じ子 plan の C2。

export type CommandPaletteCommandId =
  | 'next-pending-approval'
  | 'next-standby'
  | 'new-session'
  | 'open-review'
  | 'open-files'
  | 'open-git'
  | 'open-settings-notify'
  | 'open-settings-approval'
  | 'show-shortcuts';

export interface CommandPaletteCommandDef {
  id: CommandPaletteCommandId;
  /** i18n キー。翻訳は呼び出し側（C2 の session-search-palette.ts）が t() で解決する。 */
  labelKey: string;
  /** 絞り込みに使う語。日本語・英語どちらも含める（小文字で置く）。 */
  keywords: string[];
}

// 開いた直後・入力が空のときにそのまま並ぶ順序（設計の決め）:
// 承認待ち → 待機中 → 新規セッション → Review → Files → Git → 設定（通知/承認）→ ショートカット一覧。
export const COMMAND_PALETTE_COMMANDS: readonly CommandPaletteCommandDef[] = [
  { id: 'next-pending-approval', labelKey: 'palette_cmd_next_pending_approval', keywords: ['approval', 'pending', 'next', '承認', '待ち', '次'] },
  { id: 'next-standby', labelKey: 'palette_cmd_next_standby', keywords: ['standby', 'idle', 'next', '待機', '次'] },
  { id: 'new-session', labelKey: 'palette_cmd_new_session', keywords: ['new', 'session', 'spawn', '新規', 'セッション'] },
  { id: 'open-review', labelKey: 'palette_cmd_open_review', keywords: ['review', 'diff', 'レビュー', '差分'] },
  { id: 'open-files', labelKey: 'palette_cmd_open_files', keywords: ['files', 'ファイル'] },
  { id: 'open-git', labelKey: 'palette_cmd_open_git', keywords: ['git', 'branch', 'ブランチ'] },
  { id: 'open-settings-notify', labelKey: 'palette_cmd_open_settings_notify', keywords: ['notify', 'sound', 'settings', '通知', '設定'] },
  { id: 'open-settings-approval', labelKey: 'palette_cmd_open_settings_approval', keywords: ['approval', 'settings', '承認', '設定'] },
  { id: 'show-shortcuts', labelKey: 'palette_cmd_show_shortcuts', keywords: ['shortcut', 'help', 'keys', 'ショートカット', 'ヘルプ'] },
];

/**
 * コマンドの使える/使えないを決める入力。DOM やセッションの生オブジェクトではなく
 * 判定に要る値だけを渡す（組み立ては呼び出し側 C2 の責務）。
 */
export interface CommandPaletteContext {
  /** 承認待ちの候補セッション ID（表示順、0 件以上）。 */
  pendingApprovalIds: readonly number[];
  /** 待機中（standby）の候補セッション ID（表示順、0 件以上）。 */
  standbySessionIds: readonly number[];
  /** 今アクティブなセッション ID。無ければ null。 */
  activeSessionId: number | null;
  /** アクティブセッションに Files/Git/Review を開ける対象（git_root か cwd）があるか。 */
  activeSessionHasWorkdir: boolean;
}

export function isCommandPaletteCommandEnabled(id: CommandPaletteCommandId, ctx: CommandPaletteContext): boolean {
  switch (id) {
    case 'next-pending-approval':
      return ctx.pendingApprovalIds.length > 0;
    case 'next-standby':
      return ctx.standbySessionIds.length > 0;
    case 'open-review':
    case 'open-files':
    case 'open-git':
      return ctx.activeSessionId !== null && ctx.activeSessionHasWorkdir;
    case 'new-session':
    case 'open-settings-notify':
    case 'open-settings-approval':
    case 'show-shortcuts':
      return true;
    default:
      return true;
  }
}

/** 使えない理由を表す i18n キー。使える場合は null。 */
export function commandPaletteDisabledReasonKey(id: CommandPaletteCommandId, ctx: CommandPaletteContext): string | null {
  if (isCommandPaletteCommandEnabled(id, ctx)) return null;
  switch (id) {
    case 'next-pending-approval':
      return 'palette_cmd_next_pending_approval_disabled';
    case 'next-standby':
      return 'palette_cmd_next_standby_disabled';
    case 'open-review':
      return 'palette_cmd_open_review_disabled';
    case 'open-files':
      return 'palette_cmd_open_files_disabled';
    case 'open-git':
      return 'palette_cmd_open_git_disabled';
    default:
      return null;
  }
}

export interface CommandPaletteListItem {
  def: CommandPaletteCommandDef;
  enabled: boolean;
  disabledReasonKey: string | null;
}

/** 定義順で全件を、使える/使えないの判定つきで並べる。 */
export function listCommandPaletteCommands(ctx: CommandPaletteContext): CommandPaletteListItem[] {
  return COMMAND_PALETTE_COMMANDS.map((def) => {
    const enabled = isCommandPaletteCommandEnabled(def.id, ctx);
    return { def, enabled, disabledReasonKey: enabled ? null : commandPaletteDisabledReasonKey(def.id, ctx) };
  });
}

/**
 * ラベル文字列とキーワードのどちらかに一致すれば残す。空クエリは常に一致。
 * ラベルは呼び出し側（C2）が t() で訳した文字列を渡す — このモジュールは i18n に触れない。
 */
export function commandPaletteMatchesQuery(label: string, keywords: readonly string[], query: string): boolean {
  const q = query.trim().toLocaleLowerCase();
  if (!q) return true;
  if (label.toLocaleLowerCase().includes(q)) return true;
  return keywords.some((keyword) => keyword.toLocaleLowerCase().includes(q));
}

export interface CommandPaletteQueryable {
  label: string;
  keywords: readonly string[];
}

/** 絞り込み。空入力なら定義順のまま全件、入力ありならラベル・キーワードに一致するものだけ。 */
export function filterCommandPaletteItems<T extends CommandPaletteQueryable>(items: readonly T[], query: string): T[] {
  return items.filter((item) => commandPaletteMatchesQuery(item.label, item.keywords, query));
}

/**
 * 並び順の中で「今の次」を返す。末尾なら先頭へ戻る。候補が 0 件なら null。
 *
 * `excludeCurrent` は今のセッション自身を候補から外す。「次の待機中へ」は自分自身へは
 * 戻らせない設計にする一方、「次の承認待ちへ」は今のセッションも候補に含めたままにし、
 * 候補が 1 件しかないときは同じ場所に留まる（cycle back to self）挙動を許す。
 */
export function pickNextInOrder(
  orderedIds: readonly number[],
  candidateIds: Iterable<number>,
  currentId: number | null,
  opts?: { excludeCurrent?: boolean },
): number | null {
  const candidates = new Set(candidateIds);
  if (opts?.excludeCurrent && currentId !== null) candidates.delete(currentId);
  const inOrder = orderedIds.filter((id) => candidates.has(id));
  if (inOrder.length === 0) return null;
  if (currentId === null) return inOrder[0];
  const idx = inOrder.indexOf(currentId);
  if (idx === -1) return inOrder[0];
  return inOrder[(idx + 1) % inOrder.length];
}

export function pickNextPendingApprovalId(orderedIds: readonly number[], pendingIds: Iterable<number>, currentId: number | null): number | null {
  return pickNextInOrder(orderedIds, pendingIds, currentId);
}

export function pickNextStandbySessionId(orderedIds: readonly number[], standbyIds: Iterable<number>, currentId: number | null): number | null {
  return pickNextInOrder(orderedIds, standbyIds, currentId, { excludeCurrent: true });
}

/**
 * ↑↓ キーでの選択位置の移動。件数が 0 のときは 0 を返す（呼び出し側は件数 0 なら
 * そもそも選択枠を描かない）。端では回る（末尾から ↓ で先頭へ、先頭から ↑ で末尾へ）。
 * 「次のセッションへ」（pickNextInOrder）と同じ「端で止めずに回る」を選び、挙動を揃えた。
 */
export function nextCommandPaletteSelectionIndex(count: number, current: number, direction: 1 | -1): number {
  if (count <= 0) return 0;
  const normalized = ((current % count) + count) % count;
  return (normalized + direction + count) % count;
}
