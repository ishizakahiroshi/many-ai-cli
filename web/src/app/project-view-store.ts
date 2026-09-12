// project-view-store.ts — 箱ごとの記憶の読み書き。
//
// 由来: docs/local/plan_project-box-open-and-session-strip_c4_state-memory.md
//
// 保存先は既存の user_prefs（/api/user-prefs）。**新しい API を作らない。**
// エンドポイントを増やすと「PUT は全体置換」「200ms の debounce」という 2 つの性質を
// 二重に持つことになる。ここは setUserPref を呼ぶだけで、送信のタイミングは持たない。
//
// 守っていること 3 つ。どれも「記憶が静かに潰れる」側の失敗を防いでいる。
//
//  1. **保存は利用者の操作からだけ呼ぶ。** 一覧の変化からは呼ばない。接続が切れると
//     ws-client.ts:222 が sessions.clear() して一覧を描き直すので、そこを「箱が消えた」
//     と読んで保存すると、切断のたびに記憶が消える。
//  2. **復元中は保存しない。** 復元が保存を呼ぶと、落とした先の値（先頭セッション・
//     既定タブ）が記憶を潰す。記憶していたセッションが消えていたときだけ、
//     呼び出し側が意図して上書きする。
//  3. **帯の高さ（C2）と multi の範囲（C3）はここへ載せない。** どちらも端末ごとで、
//     それぞれの localStorage キーが正本。

import { sanitizeProjectViews, withProjectView } from './project-view-memory.js';
import type { ProjectViewMap } from './project-view-memory.js';
import {
  STORAGE_OPEN_PROJECT_KEY,
  STORAGE_PROJECT_VIEWS_KEY,
  setUserPref,
} from './user-prefs.js';

/** user_prefs のパス。_USER_PREFS_PATH_TO_LS に載っている名前と一致させること。 */
const PATH_PROJECT_VIEWS = 'project_views';
const PATH_OPEN_PROJECT = 'open_project';

// 復元の最中は保存を止める。入れ子で呼ばれても数えられるようカウンタで持つ。
let restoreDepth = 0;

export function isRestoringProjectView(): boolean {
  return restoreDepth > 0;
}

/** fn の実行中は saveProjectView / saveOpenProjectKey を無効にする。 */
export function runWithProjectViewRestore<T>(fn: () => T): T {
  restoreDepth++;
  try {
    return fn();
  } finally {
    restoreDepth = Math.max(0, restoreDepth - 1);
  }
}

/** いまの記憶。サーバー値は起動時のミラーで localStorage へ入っているのでここだけ読む。 */
export function readProjectViews(): ProjectViewMap {
  try {
    const raw = localStorage.getItem(STORAGE_PROJECT_VIEWS_KEY);
    if (!raw) return {};
    return sanitizeProjectViews(JSON.parse(raw)) || {};
  } catch (_) {
    return {};
  }
}

export function readProjectView(key: string): ProjectViewMap[string] | undefined {
  if (!key) return undefined;
  return readProjectViews()[key];
}

/** 最後に開いていた箱のキー。'' は「どの箱も開いていない」＝正常な状態。 */
export function readOpenProjectKey(): string {
  try {
    return localStorage.getItem(STORAGE_OPEN_PROJECT_KEY) || '';
  } catch (_) {
    return '';
  }
}

/** 箱ごとの記憶を 1 件書く。localStorage へ書いてから 200ms の debounce で PUT される。 */
export function saveProjectView(key: string, sessionId: unknown, tab: unknown): void {
  if (isRestoringProjectView()) return;
  if (!key) return;
  const next = withProjectView(readProjectViews(), key, { session_id: sessionId, tab });
  setUserPref(PATH_PROJECT_VIEWS, next);
}

/** 最後に開いていた箱を書く。null / '' はどの箱も開いていない状態として保存する。 */
export function saveOpenProjectKey(key: string | null): void {
  if (isRestoringProjectView()) return;
  setUserPref(PATH_OPEN_PROJECT, key || '');
}
