// 保留中の承認の、画面側の写し。
//
// Hub は「このセッションでいま答えを待っている承認」を 1 件の記録として持つ
// （internal/hub/approval_record.go の冒頭が規則の正本）。画面はその記録を
// approval_snapshot（接続したときのまとめ）と approval_state（開く・閉じるの差分）で
// 受け取ってここへ写し、描くだけにする。端末の文字から承認を作らない
// （docs/local/plan_approval-display-single-source.md の原則 1）。
//
// 以前の画面は Hub から一度きりの通知しか受け取らず、端末の文字を読んで自分で記録を作り、
// タイマーで取り直していた。別のセッションを見ている間に届いた承認が、切り替えても
// 描かれない行き止まりはそこから出た（bugfix_approval-panel-blank-on-switch_2026-09-23.md）。
// ここは見ているセッションかどうかに関係なく記録を持つので、切り替えた時点で描ける。
//
// 規則:
//   - セッションごとに記録 1 件と版番号を持つ。まとめが届いたら丸ごと置き換える。
//     前の接続で持っていた記録を残すと、閉じた承認が再接続のたびに出戻る。
//   - 差分は版番号が手元より新しいときだけ適用する。Hub は配信をロックの外で行うので、
//     開く・閉じるが逆順に届くことがある。古い版を捨てれば、閉じた記録が遅れて届いた
//     「開く」で生き返らない。
//   - 閉じるは「その版では記録が無い」という意味なので、手元の記録を外す。開いたことが
//     届いていない記録の閉じる（Hub が自動承認した記録は、開いたことを画面へ送らない）は、
//     版番号を進めるだけになる。
//   - 承認の同一性は Hub の candidate_key + source_epoch だけを使う（approval-answered.ts）。
//     回答を送った記録を描き直さない判定も、approval-answered.ts の回答済みの印を引く。
//     ここに 2 つ目の「同じ質問とは何か」を作らない。
//   - DOM に触らない。何を描くかの判断（approvalViewForRecord）も純粋な関数にして、
//     approval-store-fixtures.ts で固定する。
//   - 畳んだこと（パネルの ✕）も記録ごとにここで覚える（下の「畳む」節）。畳んでも記録と
//     「保留中」は変えない。その画面の見え方だけの状態なので Hub へは送らない。
import type { ApprovalRecord, ApprovalSessionState, ApprovalState } from '../types/proto.js';
import { extractHubMarkerApproval, extractSequentialChoicePrompts, normalizeVtCursorOps } from './approval-parser.js';
import { annotateApprovalIdentity, isAnsweredApprovalIdentity } from './approval-answered.js';

// 記録の origin（internal/hub/approval_record.go の approvalRecordOrigin*）。
export const APPROVAL_ORIGIN_NATIVE = 'native';

// 記録の kind のうち、画面が描き方を変えるもの（internal/hub の approvalKind* と同じ値）。
// ネイティブの承認画面の kind（native / native_codex_shortcut など）は選択肢の形が同じなので
// 個別に持たない。native_opencode_shortcut だけはラベルの翻訳に使う。
export const APPROVAL_KIND_MARKER = 'marker';
export const APPROVAL_KIND_ASK_USER_QUESTION = 'ask_user_question';
export const APPROVAL_KIND_SEQUENTIAL_CHOICE = 'sequential_choice';
export const APPROVAL_KIND_HUB_CHOICE = 'hub_choice';
export const APPROVAL_KIND_OPENCODE_SHORTCUT = 'native_opencode_shortcut';

interface StoreEntry {
  version: number;
  record: ApprovalRecord | null;
}

export interface ApprovalStoreChange {
  sessionId: number;
  /** 変化の後の記録（無ければ null）。 */
  record: ApprovalRecord | null;
  /** 変化の前の記録（無ければ null）。 */
  previous: ApprovalRecord | null;
  /**
   * 利用者へ知らせる新しい記録か（音・デスクトップ通知・承認への自動移動）。
   * 差分で開いた記録のうち、このページで直前に見ていた記録と同一性が違うものだけが true。
   * まとめ（接続・再接続）で届いた記録と、同じ世代で開き直した記録は false にする。
   * Hub 側の通知も同じ世代の開き直しでは鳴り直さない（approvalNotificationIDLocked）。
   */
  announce: boolean;
}

export type ApprovalStoreListener = (changes: ApprovalStoreChange[]) => void;

const entries = new Map<number, StoreEntry>();
// セッションごとに最後に見た記録の同一性。announce の判定だけに使う。
const seenRecordTokens = new Map<number, string>();
const listeners = new Set<ApprovalStoreListener>();

/** 記録の同一性（candidate_key + source_epoch）を 1 つの文字列にしたもの。記録が無ければ ''。 */
export function approvalRecordToken(record: ApprovalRecord | null | undefined): string {
  if (!record || !record.candidate_key) return '';
  return `${Number(record.source_epoch) || 0}\0${record.candidate_key}`;
}

function notify(changes: ApprovalStoreChange[]): void {
  if (changes.length === 0) return;
  for (const listener of Array.from(listeners)) {
    try {
      listener(changes);
    } catch (err) {
      console.warn('approval store listener failed', err);
    }
  }
}

/** 記録が変わるたびに呼ばれる。戻り値を呼ぶと購読をやめる。 */
export function subscribeApprovalStore(listener: ApprovalStoreListener): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/**
 * 接続したときのまとめ（approval_snapshot）で全セッションの記録を置き換える。
 * 記録の無いセッションも並んで届くので、まとめに無い記録は残さない。
 * 版番号も比べずに置き換える（Hub を起動し直すと版番号は 0 から数え直すため）。
 */
export function applyApprovalSnapshot(states: ApprovalSessionState[] | null | undefined): ApprovalStoreChange[] {
  const next = new Map<number, StoreEntry>();
  for (const state of states || []) {
    const id = Number(state?.session_id);
    if (!Number.isFinite(id) || id <= 0) continue;
    next.set(id, { version: Number(state.version) || 0, record: state.record || null });
  }
  const changes: ApprovalStoreChange[] = [];
  for (const id of new Set([...entries.keys(), ...next.keys()])) {
    const previous = entries.get(id)?.record || null;
    const record = next.get(id)?.record || null;
    if (record) seenRecordTokens.set(id, approvalRecordToken(record));
    if (approvalRecordToken(previous) === approvalRecordToken(record)) continue;
    changes.push({ sessionId: id, record, previous, announce: false });
  }
  entries.clear();
  for (const [id, entry] of next) entries.set(id, entry);
  // まとめに無い記録の畳み状態は、もう描かれることが無いので捨てる（接続していない間に
  // 閉じた記録・Hub を起動し直す前の記録の分）。
  pruneFolds(new Set(Array.from(next, ([id, entry]) => foldKeyFor(id, entry.record)).filter(Boolean)));
  notify(changes);
  return changes;
}

/**
 * 1 セッションの記録の開閉（approval_state）を適用する。手元より古い版は捨てる。
 * 見た目の変わらない適用（知らない記録の閉じる・同じ記録の開き直し）は null を返し、
 * 購読者へも知らせない。
 */
export function applyApprovalState(sessionId: number, state: ApprovalState | null | undefined): ApprovalStoreChange | null {
  const id = Number(sessionId);
  if (!Number.isFinite(id) || id <= 0 || !state) return null;
  const version = Number(state.version) || 0;
  const current = entries.get(id);
  if (current && version <= current.version) return null;
  const previous = current?.record || null;
  // 閉じるは「この版では記録が無い」。開く・閉じるのどちらか一方だけが入って届く。
  const record = state.open || null;
  entries.set(id, { version, record });
  if (approvalRecordToken(previous) === approvalRecordToken(record)) return null;
  // 閉じても畳み状態は捨てない。ネイティブの承認は画面から一時的に消えて閉じ、同じ候補・同じ世代で
  // 開き直すことがあり、それは同じ承認なので畳んだまま出す（原則 5）。キーに世代を含むので、
  // 残した項目が別の記録に当たることは無い。掃除はまとめの刈り込みと件数の上限で行う。
  let announce = false;
  if (record) {
    const token = approvalRecordToken(record);
    announce = seenRecordTokens.get(id) !== token;
    seenRecordTokens.set(id, token);
  }
  const change: ApprovalStoreChange = { sessionId: id, record, previous, announce };
  notify([change]);
  return change;
}

/** セッションが消えたときに、そのセッションの記録を捨てる。 */
export function forgetApprovalSession(sessionId: number): ApprovalStoreChange | null {
  const id = Number(sessionId);
  const previous = entries.get(id)?.record || null;
  entries.delete(id);
  seenRecordTokens.delete(id);
  dropFold(foldKeyFor(id, previous));
  if (!previous) return null;
  const change: ApprovalStoreChange = { sessionId: id, record: null, previous, announce: false };
  notify([change]);
  return change;
}

/** Hub の起動し直しで、全セッションの記録を捨てる（直後に届くまとめで埋め直す）。 */
export function resetApprovalStore(): void {
  const changes: ApprovalStoreChange[] = [];
  for (const [id, entry] of entries) {
    if (entry.record) changes.push({ sessionId: id, record: null, previous: entry.record, announce: false });
  }
  entries.clear();
  seenRecordTokens.clear();
  notify(changes);
}

/** そのセッションの保留中の記録（無ければ null）。回答を送った記録も含む。 */
export function approvalRecordFor(sessionId: number): ApprovalRecord | null {
  return entries.get(Number(sessionId))?.record || null;
}

/** 手元の版番号（届いていなければ 0）。 */
export function approvalStoreVersion(sessionId: number): number {
  return entries.get(Number(sessionId))?.version || 0;
}

/** この記録に画面から回答を送ったか（Hub の閉じるが届くまでの間は描かない）。 */
export function isApprovalRecordAnswered(sessionId: number, record: ApprovalRecord | null | undefined): boolean {
  if (!record) return false;
  return isAnsweredApprovalIdentity(Number(sessionId), record.candidate_key, Number(record.source_epoch) || 0);
}

// ---- 畳む（パネルの ✕）----
//
// ✕ はパネルを 1 行の帯に畳む。帯を押せば開く（docs/local/plan_approval-display-single-source.md
// の B1・B2）。承認があるかどうかは Hub の記録が決めるので、畳んでも記録・「保留中」・通知は
// 変えず、Hub へも何も送らない。畳んだ承認は、この画面のパネル・自動移動・モバイルのシートで
// 開かない（承認タブの一覧には残る）。
//
// 覚え方:
//   - 記録ごとに覚える。キーは「Hub の起動ごとの ID + セッション番号 + 記録の同一性
//     （candidate_key + source_epoch）」。同じ質問でもセッションが違えば別に扱う。
//     セッション番号は Hub を起動し直すと振り直されるので、起動ごとの ID（snapshot の
//     hub_instance）を前に付けて、起動し直した後の別の承認を畳んだまま出さない。
//   - 保存先はタブごとの sessionStorage。再読み込みしても同じ承認なら畳んだまま、別のタブ・
//     別の端末とは共有しない（原則 5: 畳む・開くはその画面の見え方だけを変える）。
//   - 記録が閉じても項目は消さない。ネイティブの承認は画面から一時的に消えて閉じ、同じ候補・
//     同じ世代で開き直すことがあり、それは同じ承認なので畳んだまま出す。キーに世代を含むので、
//     残した項目が別の記録に当たることは無い。消すのは、まとめ（approval_snapshot）に無い記録の分・
//     セッションが消えたとき・件数の上限を超えたとき。
const FOLD_STORAGE_KEY = 'ai_cli_hub_approval_folded';
const FOLD_LIMIT = 200;

/** 畳み状態の保存先（sessionStorage と同じ形）。fixture からは差し替える。 */
export interface ApprovalFoldStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

function defaultFoldStorage(): ApprovalFoldStorage | null {
  try {
    return typeof sessionStorage === 'undefined' ? null : sessionStorage;
  } catch (_) {
    return null;
  }
}

let foldStorage: ApprovalFoldStorage | null = defaultFoldStorage();
let foldScope = '';
// 保存先の写し（畳んだ順）。初めて使うときに保存先から読む。
let folds: string[] | null = null;

function loadFolds(): string[] {
  if (folds) return folds;
  folds = [];
  try {
    const raw = JSON.parse(foldStorage?.getItem(FOLD_STORAGE_KEY) || '[]');
    if (Array.isArray(raw)) folds = raw.filter((key) => typeof key === 'string' && key !== '').slice(-FOLD_LIMIT);
  } catch (_) {
    folds = [];
  }
  return folds;
}

function saveFolds(list: string[]): void {
  folds = list.slice(-FOLD_LIMIT);
  try { foldStorage?.setItem(FOLD_STORAGE_KEY, JSON.stringify(folds)); } catch (_) {}
}

function foldKeyFor(sessionId: number, record: ApprovalRecord | null | undefined): string {
  const token = approvalRecordToken(record);
  if (!token) return '';
  return `${foldScope}\0${Number(sessionId)}\0${token}`;
}

function dropFold(key: string): void {
  if (!key) return;
  const list = loadFolds();
  if (list.includes(key)) saveFolds(list.filter((item) => item !== key));
}

function pruneFolds(liveKeys: Set<string>): void {
  const list = loadFolds();
  const kept = list.filter((key) => liveKeys.has(key));
  if (kept.length !== list.length) saveFolds(kept);
}

/**
 * Hub の起動ごとの ID（snapshot の hub_instance）。まとめ（approval_snapshot）を適用する前に
 * 渡すこと。Hub はまとめより先に snapshot を送る（internal/hub/ui_broadcast.go）。
 */
export function setApprovalFoldScope(scope: string): void {
  foldScope = String(scope || '');
}

/** この記録を畳んでいるか。 */
export function isApprovalRecordFolded(sessionId: number, record: ApprovalRecord | null | undefined): boolean {
  const key = foldKeyFor(sessionId, record);
  return !!key && loadFolds().includes(key);
}

/** そのセッションの保留中の記録を畳んでいるか。 */
export function isApprovalFolded(sessionId: number): boolean {
  return isApprovalRecordFolded(sessionId, approvalRecordFor(sessionId));
}

/** そのセッションの保留中の記録を畳む。記録が無ければ何もしない。畳んだら true。 */
export function foldApproval(sessionId: number): boolean {
  const key = foldKeyFor(sessionId, approvalRecordFor(sessionId));
  if (!key) return false;
  const list = loadFolds();
  if (list.includes(key)) return false;
  saveFolds([...list, key]);
  return true;
}

/** そのセッションの保留中の記録を開く。開いたら true。 */
export function unfoldApproval(sessionId: number): boolean {
  const key = foldKeyFor(sessionId, approvalRecordFor(sessionId));
  if (!key || !loadFolds().includes(key)) return false;
  dropFold(key);
  return true;
}

/** テスト専用。畳み状態の保存先を差し替え、写しを読み直させる。 */
export function _setApprovalFoldStorageForTest(storage: ApprovalFoldStorage | null): void {
  foldStorage = storage;
  folds = null;
}

// ---- 何を描くか ----

export type ApprovalViewKind = 'none' | 'folded' | 'notice' | 'options' | 'sequential' | 'unreadable';

export interface ApprovalView {
  kind: ApprovalViewKind;
  record: ApprovalRecord | null;
  /** kind === 'options' のときの選択肢（単問・一括・複数選択のどれも既存の描画関数が読む形）。 */
  options?: any[];
  /** kind === 'sequential' のときの順次質問（見出しと選択肢）。 */
  prompts?: any[];
}

interface ApprovalRecordContent {
  kind: 'notice' | 'options' | 'sequential' | 'unreadable';
  options?: any[];
  prompts?: any[];
}

// 記録は届いた後に書き換えない（Hub 側も同じ）ので、解いた中身を記録ごとに覚えておく。
// 承認タブは 1 秒ごとに全セッションの選択肢を読むので、そのたびにブロックを解かない。
const contentCache = new WeakMap<ApprovalRecord, ApprovalRecordContent>();

/** Hub から届いた選択肢（proto.ApprovalOption）を、画面の描画関数が読む形にする。 */
export function approvalOptionsFromWire(raw: unknown): any[] {
  return (Array.isArray(raw) ? raw : [])
    .map((opt: any) => ({
      num: Number(opt?.num),
      label: String(opt?.label || '').trim(),
      isCurrent: !!opt?.is_current,
      preserveOrder: !!opt?.preserve_order,
      _sendText: opt?.send_text || undefined,
    }))
    .filter((opt) => Number.isFinite(opt.num) && opt.label);
}

/** ネイティブの承認の要約（proto.ApprovalSummary）を承認カードが読む形にする。 */
export function approvalSummaryFromWire(raw: any, fallbackRaw?: unknown, fallbackQuestion?: unknown): any {
  if (!raw || typeof raw !== 'object') return null;
  const risk = raw.risk === 'low' || raw.risk === 'high' ? raw.risk : 'mid';
  const command = String(raw.command || fallbackQuestion || '').trim();
  const paths = Array.isArray(raw.paths)
    ? raw.paths.map((path: unknown) => String(path || '').trim()).filter(Boolean).slice(0, 4)
    : [];
  const context = String(raw.raw || fallbackRaw || '').trim();
  if (!command && !context) return null;
  return { command, paths, risk, raw: context };
}

// Hub の記録のブロックは端末ミラーの行かトランスクリプトの本文で、端末の制御文字を含まない。
// 念のためカーソル移動の変換と CSI の除去だけを通してから行に分ける。
// 承認タブの履歴（Hub の台帳の行）も同じブロックなので、これで行に分ける（解釈器を 2 本にしない）。
export function hubBlockLines(block: unknown): string[] {
  return normalizeVtCursorOps(String(block || ''))
    .split(/\r\n|\r|\n/)
    .map((line: string) => line.replace(/\u001b\[[0-?]*[ -/]*[@-~]/g, ''));
}

function withIdentity(options: any[], record: ApprovalRecord): any[] {
  return annotateApprovalIdentity(options, {
    candidateKey: String(record.candidate_key || ''),
    sourceEpoch: Number(record.source_epoch) || 0,
  });
}

function recordContent(record: ApprovalRecord): ApprovalRecordContent {
  const cached = contentCache.get(record);
  if (cached) return cached;
  const content = buildRecordContent(record);
  contentCache.set(record, content);
  return content;
}

function buildRecordContent(record: ApprovalRecord): ApprovalRecordContent {
  const kind = String(record.kind || '');
  if (kind === APPROVAL_KIND_ASK_USER_QUESTION) return { kind: 'notice' };

  if (kind === APPROVAL_KIND_MARKER) {
    const options = extractHubMarkerApproval(hubBlockLines(record.block));
    if (!Array.isArray(options) || options.length === 0) return { kind: 'unreadable' };
    return { kind: 'options', options: withIdentity(options, record) };
  }

  if (kind === APPROVAL_KIND_SEQUENTIAL_CHOICE) {
    const prompts = extractSequentialChoicePrompts(hubBlockLines(record.block));
    if (!Array.isArray(prompts) || prompts.length === 0) return { kind: 'unreadable' };
    return { kind: 'sequential', prompts };
  }

  const options = approvalOptionsFromWire(record.options);
  if (options.length === 0) {
    // 選択肢の無いネイティブの記録は、端末で答えるものとして告知だけにする。
    return record.origin === APPROVAL_ORIGIN_NATIVE ? { kind: 'notice' } : { kind: 'unreadable' };
  }
  const question = String(record.question || '').trim();
  if (question) (options as any)._question = question;
  if (kind === APPROVAL_KIND_HUB_CHOICE) {
    // 旧形式の選択は「N. User specifies」を持つ（approval_text_question.go の extractHubChoiceQuestion）。
    (options as any)._freeInput = true;
  }
  if (record.origin === APPROVAL_ORIGIN_NATIVE) {
    const summary = approvalSummaryFromWire(record.summary, record.context, record.question);
    if (summary) (options as any)._summary = summary;
    for (const option of options) {
      option._approvalSource = 'go_vt';
      option._approvalSig = String(record.sig || '');
    }
  }
  return { kind: 'options', options: withIdentity(options, record) };
}

/**
 * 記録から何を描くかを決める（純粋な関数）。
 *   answered: この記録に画面から回答を送った。Hub の閉じるが届くまで描かない。
 *   folded:   利用者がパネルを帯に畳んだ（isApprovalRecordFolded）。帯だけを描く。
 */
export function approvalViewForRecord(record: ApprovalRecord | null | undefined, opts: { answered?: boolean; folded?: boolean } = {}): ApprovalView {
  if (!record || opts.answered) return { kind: 'none', record: null };
  if (opts.folded) return { kind: 'folded', record };
  const content = recordContent(record);
  return { kind: content.kind, record, options: content.options, prompts: content.prompts };
}

/**
 * セッションの記録から何を描くかを決める。回答済みの判定は approval-answered.ts を引く。
 * 畳み状態は既定では見ない（承認タブ・回答の送信・モバイルのシートの中身は、畳んでいても
 * 選択肢を読む）。パネルを描くときだけ { folded: true } を渡し、畳んでいれば帯にする。
 */
export function approvalViewFor(sessionId: number, opts: { folded?: boolean } = {}): ApprovalView {
  const record = approvalRecordFor(sessionId);
  return approvalViewForRecord(record, {
    answered: isApprovalRecordAnswered(sessionId, record),
    folded: !!opts.folded && isApprovalRecordFolded(sessionId, record),
  });
}

/**
 * 利用者の答えを待っている記録があるか。回答を送った記録（Hub の閉じるを待っている間）は
 * 含めない。承認タブ・自動移動・モバイルの「保留中」の並びはこれで決める。
 * 「保留中」の表示そのもの（セッション一覧・サマリー）は Hub の state を読む。
 */
export function isApprovalPending(sessionId: number): boolean {
  const record = approvalRecordFor(sessionId);
  return !!record && !isApprovalRecordAnswered(sessionId, record);
}

/**
 * テスト専用。ストアを空にし、購読者も外す（ページの読み込み直しに当たる）。
 * 畳み状態の保存先は残し、写しだけを捨てて読み直させる。
 */
export function _resetApprovalStoreForTest(): void {
  entries.clear();
  seenRecordTokens.clear();
  listeners.clear();
  foldScope = '';
  folds = null;
}
