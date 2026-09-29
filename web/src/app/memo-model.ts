// plan_memo-panel.md C3 — 作業メモ（案 C: プロジェクトに紐づくメモ）の純粋ロジック。
// DOM に触れない。グループ化・並び順・件数だけをここに置き、memo-panel.ts が描画に使う。
//
// パスの合成（joinPath）は path-links.ts にも同名の実装があるが、あちらは util.js /
// state.js / files-view.js を経由して window への副作用（window.showToast 代入等）を
// 持ち込むため、bun:test（DOM 無し）から import すると失敗する。ここでは path-detect.js
// （DOM に依存しない）だけを import し、joinPath 相当はこのファイル内に複製する。
import { isAbsolutePath } from './path-detect.js';

// C4（件数バッジ）向けの通知イベント名。ここに置く理由: session-list.ts は
// spawn-panel.ts が session-list.ts の providerIconHtml を import しており、
// memo-panel.ts は spawn-panel.ts を import する（openSpawnPanelWith）ため、
// session-list.ts が memo-panel.ts を import すると
// session-list → memo-panel → spawn-panel → session-list の循環になる
// （scripts/check-web-module-init.mjs が検出する TDZ の型）。memo-model.ts は
// path-detect.js しか import しない葉モジュールなので、session-list.ts はここから
// 直接読む。ペイロードは持たない（「取り直せ」の合図だけ。件数は共有キャッシュから読む）。
export const MEMOS_CHANGED_EVENT = 'many-ai-cli:memos-changed';

export interface Memo {
  id: string;
  text: string;
  /** git root（C1 が Hub 側で正規化）。空文字 = 「未分類」。 */
  project: string;
  done: boolean;
  created_at: string;
  updated_at: string;
  done_at?: string;
  /** Hub の memo-images/ 配下の画像名（パスではない）。画像だけのメモは text が空。 */
  images?: string[];
}

/** 1 つのメモに付けられる画像の上限（internal/hub/memo_images.go の memoImagesPerMemo と同じ）。 */
export const MEMO_IMAGES_PER_MEMO = 10;
/** 1 枚の上限（同 memoImageMaxBytes と同じ）。送る前に弾いて、待たせてから失敗させない。 */
export const MEMO_IMAGE_MAX_BYTES = 10 * 1024 * 1024;

/** 貼り付けたファイルのうち、メモに付けられる画像（Hub が受ける 4 形式）だけを返す。 */
export function pickMemoImageFiles<T extends { type: string }>(files: readonly T[]): T[] {
  return files.filter((f) => ['image/png', 'image/jpeg', 'image/gif', 'image/webp'].includes(f.type));
}

export interface MemoGroup {
  /** 空文字 = 未分類。 */
  project: string;
  open: Memo[];
  done: Memo[];
}

/**
 * メモを project（空文字="未分類"）ごとにグループ化し、plan_memo-panel.md の
 * 「概要/仕様」節が定める順で並べる:
 *   1. activeProject（選択中セッションのプロジェクト。空文字なら該当なし）
 *   2. 残りは最終更新（グループ内 updated_at の最大値）が新しい順
 *   3. 未分類（project === ''）は常に最後
 * 各グループは open（未完了）/ done（完了）に分けて返す。並び順（新しい順など）は
 * sortOpenMemos / sortDoneMemos が別に決める。
 */
export function groupMemosByProject(memos: readonly Memo[], activeProject: string): MemoGroup[] {
  const byProject = new Map<string, Memo[]>();
  for (const memo of memos) {
    const key = memo.project || '';
    const list = byProject.get(key);
    if (list) list.push(memo);
    else byProject.set(key, [memo]);
  }
  const latestUpdate = (list: Memo[]): string =>
    list.reduce((max, m) => (m.updated_at > max ? m.updated_at : max), '');
  const projects = Array.from(byProject.keys());
  projects.sort((a, b) => {
    if (a === '' && b === '') return 0;
    if (a === '') return 1;
    if (b === '') return -1;
    if (activeProject) {
      if (a === activeProject && b !== activeProject) return -1;
      if (b === activeProject && a !== activeProject) return 1;
    }
    return latestUpdate(byProject.get(b) as Memo[]).localeCompare(latestUpdate(byProject.get(a) as Memo[]));
  });
  return projects.map((project) => {
    const list = byProject.get(project) as Memo[];
    return {
      project,
      open: list.filter((m) => !m.done),
      done: list.filter((m) => m.done),
    };
  });
}

/** グループ内、未完了メモの表示順（新しく作られたものを上に）。 */
export function sortOpenMemos(memos: readonly Memo[]): Memo[] {
  return [...memos].sort((a, b) => b.created_at.localeCompare(a.created_at));
}

/** グループ内、完了メモの表示順（新しく完了したものを上に。折りたたみの中で使う）。 */
export function sortDoneMemos(memos: readonly Memo[]): Memo[] {
  return [...memos].sort((a, b) => (b.done_at || '').localeCompare(a.done_at || ''));
}

/**
 * project 単位の未完了件数。project は memo.project / SessionSnapshot.project_id と
 * 同じ表記（git root、または未分類なら空文字）で渡す。C4 のバッジがこれを使う。
 */
export function countOpenMemosForProject(memos: readonly Memo[], project: string): number {
  const key = project || '';
  return memos.filter((m) => (m.project || '') === key && !m.done).length;
}

/**
 * 本文が同じ未完了メモを返す（無ければ undefined）。パスの右クリックメニューの
 * 「作業メモに保存」（plan_path-menu-save-to-memo.md）が、同じパスを何度も積まないために使う。
 * 完了済みは対象にしない（再開位置として改めて積み直したい場合があるため）。
 */
export function findOpenMemoByText(memos: readonly Memo[], text: string): Memo | undefined {
  const target = text.trim();
  if (!target) return undefined;
  return memos.find((m) => !m.done && m.text.trim() === target);
}

// 「作業メモに保存」の受け口（plan_path-menu-save-to-memo.md）。path-links.ts は
// memo-panel.ts に import されているので、path-links.ts から memo-panel.ts を import すると
// 循環になる（MEMOS_CHANGED_EVENT と同じ理由）。memo-panel.ts が読み込み時に実体を登録し、
// path-links.ts はこの葉モジュール経由で呼ぶ。
export type MemoSaveResult = 'saved' | 'exists';
type MemoSaver = (text: string, sessionId?: number | string | null) => Promise<MemoSaveResult>;
let memoSaver: MemoSaver | null = null;

/** memo-panel.ts だけが呼ぶ。 */
export function registerMemoSaver(saver: MemoSaver): void {
  memoSaver = saver;
}

/**
 * 文字列を 1 件の作業メモとして保存する。分類先は sessionId のセッションの project
 * （Hub が決める）。同じ本文の未完了メモが既にあれば作らずに 'exists' を返す。失敗は reject。
 */
export function saveTextToMemo(text: string, sessionId?: number | string | null): Promise<MemoSaveResult> {
  if (!memoSaver) return Promise.reject(new Error('memo panel is not loaded'));
  return memoSaver(text, sessionId);
}

/**
 * メモ本文中のパス表記（findPathCandidates の結果）が指す実際のパスを求める。
 * 相対パスは、そのメモの project（書いた時点のセッションの git root）を基点に解決する。
 * appendLinkedText（path-links.ts）はライブセッションの cwd で解決する作りなので、メモの
 * 時点ではセッションが終わっていることが多いこの用途には使えない（plan_memo-panel.md C3）。
 */
export function resolveMemoPathTarget(candidateText: string, project: string): string {
  if (isAbsolutePath(candidateText)) return candidateText;
  if (!project) return candidateText;
  return joinPath(project, candidateText);
}

/**
 * session.cwd が memo.project（Hub が findGitRoot で決めた git root。git 管理外なら
 * cwd そのもの）に属するかを判定する。project_id（project_id.go の gitProjectRoot）は
 * 使わない — 判定アルゴリズムが違う（gitProjectRoot は git 管理外だと空文字、
 * findGitRoot は cwd そのものを返す）うえ、project_id は接続後に非同期で解決される値
 * なので、解決前は常に不一致になる。cwd はセッション登録時から常にある値なので、
 * ここだけで判定する（plan_memo-panel.md C4）。
 *
 * 大文字小文字は Windows パスのため区別しない。区切り文字は / と \ の両方を吸収する。
 */
export function cwdMatchesMemoProject(cwd: string, project: string): boolean {
  if (!cwd || !project) return false;
  const normalize = (p: string): string => p.replace(/\\/g, '/').toLowerCase().replace(/\/+$/, '');
  const nCwd = normalize(cwd);
  const nProject = normalize(project);
  if (nCwd === nProject) return true;
  return nCwd.startsWith(nProject + '/');
}

/** そのセッションの cwd に属する未完了メモの件数（C4: セッション一覧・ボタンのバッジ）。 */
export function countOpenMemosForCwd(memos: readonly Memo[], cwd: string): number {
  return memos.filter((m) => !m.done && cwdMatchesMemoProject(cwd, m.project)).length;
}

// ── C4: 件数バッジ用の共有キャッシュ ────────────────────────────────────
//
// memo-panel.ts が /api/memos の応答を持っているが、session-list.ts はそこを import
// できない（MEMOS_CHANGED_EVENT のコメント参照）。そのため、バッジ計算に要る最小限
// （メモの配列そのもの）をここに置き、memo-panel.ts が更新のたびに書き込む。
let sharedCache: readonly Memo[] = [];

/** memo-panel.ts が fetch 結果を反映するたびに呼ぶ。他のモジュールは呼ばない。 */
export function setSharedMemoCache(memos: readonly Memo[]): void {
  sharedCache = memos;
}

/** C4: セッション一覧の行バッジ。 */
export function openMemoCountForCwd(cwd: string): number {
  return countOpenMemosForCwd(sharedCache, cwd);
}

/** C4: メモボタン自身のバッジ（全プロジェクト合計）。 */
export function totalOpenMemoCount(): number {
  return sharedCache.filter((m) => !m.done).length;
}

// path-links.ts の joinPath / normalizePathSegments と同じ実装（コメント参照）。
function joinPath(base: string, rel: string): string {
  if (!base || !rel) return rel || base || '';
  const sep = base.includes('\\') ? '\\' : '/';
  const baseNorm = base.replace(/[\\/]+$/, '');
  return normalizePathSegments(baseNorm + sep + rel.replace(/^[\\/]+/, '').replace(/[\\/]/g, sep), sep);
}

function normalizePathSegments(path: string, sep: string): string {
  const drive = /^[A-Za-z]:/.test(path) ? path.slice(0, 2) : '';
  const rest = drive ? path.slice(2) : path;
  const rooted = rest.startsWith(sep);
  const parts = rest.split(/[\\/]+/);
  const out: string[] = [];
  for (const part of parts) {
    if (!part || part === '.') continue;
    if (part === '..' && out.length > 0 && out[out.length - 1] !== '..') out.pop();
    else if (part !== '..' || !rooted) out.push(part);
  }
  return drive + (rooted ? sep : '') + out.join(sep);
}
