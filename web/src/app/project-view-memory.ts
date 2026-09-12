// project-view-memory.ts — 箱ごとの「最後に見ていた表示」の純関数。
//
// DOM にも localStorage にも fetch にも触らない（node:test から検証するため。
// multi-scope.ts / session-strip-branch.ts / sidebar-tree.ts と同じ切り分け）。
// 保存と復元の配線は project-view-store.ts と session-list.ts が持つ。
//
// 由来: docs/local/plan_project-box-open-and-session-strip_c4_state-memory.md
//
// ここが守っているのは 2 つ。どちらも「保存値をそのまま信じない」ための網。
//
//  1. **タブ名は必ず既知の 9 種へ丸める。** 保存経路が 1 本だけとは限らない
//     （config.yaml を手で書ける・古い版が別の名前を書いている）。知らない名前を
//     setActiveTab へ渡すと何も起きず、画面は前のタブのまま「復元できたのに違う」
//     という読み方のできない状態になる。
//  2. **件数とキーの長さに上限を置く。** user_prefs は PUT で全体置換されるので、
//     際限なく育つフィールドがあると設定ファイルと 1 往復ぶんの転送量が膨らむ。
//     古い箱の記憶が消えても実害は「その箱で先頭セッションが開く」だけ。

/** タブ名の正本。settings.ts の VALID_TAB_NAMES はこの配列から作る（2 か所に持たない）。 */
export const VALID_TAB_NAME_LIST = [
  'terminal', 'chat', 'split', 'files', 'git', 'multi', 'approval', 'history', 'orchestration',
] as const;

export type TabName = (typeof VALID_TAB_NAME_LIST)[number];

/** 知らないタブ名の落とし先。index.html の初期状態と同じ。 */
export const DEFAULT_TAB_NAME: TabName = 'terminal';

/** 覚えておく箱の数の上限。超えたぶんは捨てる（templates が 100 で切っているのと同型）。 */
export const PROJECT_VIEWS_MAX = 100;

/** 箱のキーの長さの上限。キーは cwd 由来のパスなので、これを超えるものは壊れた値とみなす。 */
export const PROJECT_KEY_MAX_LEN = 512;

/** 箱 1 つぶんの記憶。 */
export interface ProjectViewMemory {
  /** その箱で最後に見ていたセッション ID。 */
  session_id: number;
  /** その箱で最後に開いていたタブ名。VALID_TAB_NAME_LIST のいずれか。 */
  tab: TabName;
}

export type ProjectViewMap = Record<string, ProjectViewMemory>;

export function isValidTabName(name: unknown): name is TabName {
  return (VALID_TAB_NAME_LIST as readonly string[]).includes(String(name ?? ''));
}

/** 知らない値・壊れた値は既定のタブへ落とす。**捨てずに落とす**（復元が無言で止まらない）。 */
export function normalizeTabName(name: unknown): TabName {
  return isValidTabName(name) ? (name as TabName) : DEFAULT_TAB_NAME;
}

function normalizeSessionId(raw: unknown): number | null {
  const n = typeof raw === 'number' ? raw : parseInt(String(raw ?? ''), 10);
  if (!Number.isInteger(n) || n <= 0) return null;
  return n;
}

/**
 * 保存されている（あるいはサーバーから来た）箱ごとの記憶を検証する。
 *
 * 戻り値が null なら「まるごと捨てる」（_parseStoredUserPref の ok:false）。
 * 個々の壊れたエントリは黙って落とす＝残りは使える。
 */
export function sanitizeProjectViews(raw: unknown): ProjectViewMap | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const out: ProjectViewMap = {};
  let kept = 0;
  for (const [key, value] of Object.entries(raw as Record<string, unknown>)) {
    if (kept >= PROJECT_VIEWS_MAX) break;
    if (typeof key !== 'string' || !key || key.length > PROJECT_KEY_MAX_LEN) continue;
    if (!value || typeof value !== 'object' || Array.isArray(value)) continue;
    const sessionId = normalizeSessionId((value as Record<string, unknown>).session_id);
    if (sessionId === null) continue;
    out[key] = { session_id: sessionId, tab: normalizeTabName((value as Record<string, unknown>).tab) };
    kept++;
  }
  return out;
}

/**
 * 1 件書き足した新しい記憶を返す（元の map は書き換えない）。
 *
 * 上限に達しているときは、いちばん古い（挿入順で先頭の）別の箱を 1 件落とす。
 * 更新対象の箱は必ず残る。
 */
export function withProjectView(
  views: ProjectViewMap | null | undefined,
  key: string,
  view: { session_id: unknown; tab: unknown },
): ProjectViewMap {
  const base = sanitizeProjectViews(views) || {};
  const sessionId = normalizeSessionId(view?.session_id);
  if (typeof key !== 'string' || !key || key.length > PROJECT_KEY_MAX_LEN || sessionId === null) {
    return base;
  }
  const next: ProjectViewMap = { ...base };
  if (!(key in next)) {
    const keys = Object.keys(next);
    while (keys.length >= PROJECT_VIEWS_MAX) {
      delete next[keys.shift() as string];
    }
  }
  next[key] = { session_id: sessionId, tab: normalizeTabName(view?.tab) };
  return next;
}

export interface RestorePick {
  /** 開くセッション。null なら開けるセッションが 1 件も無い＝復元しない。 */
  sessionId: number | null;
  /** 記憶していたセッションが消えていて先頭へ落としたか。**true のときだけ保存を上書きする。** */
  fallback: boolean;
  /**
   * 開くタブ。**null は「記憶が無いので今のタブのままにする」**。既定へ落とすのとは違う。
   *
   * ここを既定（terminal）にしてしまうと、multi タブを開いたまま箱を渡り歩く使い方
   * （C3 が mgr.onOpenProjectChanged で成立させたもの）が、まだ記憶の無い箱を開いた
   * 瞬間に壊れる。記憶にあるタブ名が壊れていたときだけ既定へ落とす。
   */
  tab: TabName | null;
}

/** 記憶されているタブ名。記憶そのものが無ければ null（＝今のタブを変えない）。 */
function rememberedTab(view: unknown): TabName | null {
  if (!view || typeof view !== 'object' || Array.isArray(view)) return null;
  return normalizeTabName((view as { tab?: unknown }).tab);
}

/**
 * 記憶と「いまその箱にあるセッション」から、復元先を決める。
 *
 * - 記憶しているセッションが生きていればそれ（fallback=false）
 * - 消えていれば**その箱の先頭**（fallback=true。親 plan の C1 が既定としている挙動）
 * - 箱が空なら何もしない（sessionId=null）
 *
 * **availableIds は orderSessions() 由来の順で渡すこと。** ここでは並べ替えない
 * （サイドバー・帯と順序が食い違うと「先頭」の意味が画面と変わる）。
 */
export function pickRestoreSession(
  view: unknown,
  availableIds: readonly number[],
): RestorePick {
  const ids = Array.isArray(availableIds) ? availableIds.filter(id => Number.isInteger(id)) : [];
  const tab = rememberedTab(view);
  if (ids.length === 0) return { sessionId: null, fallback: false, tab };
  const remembered = normalizeSessionId((view as { session_id?: unknown } | null | undefined)?.session_id);
  if (remembered !== null && ids.includes(remembered)) {
    return { sessionId: remembered, fallback: false, tab };
  }
  return { sessionId: ids[0], fallback: true, tab };
}
