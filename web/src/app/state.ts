// --- ESM imports (generated) ---
import { STORAGE_APPROVAL_AUTO_SWITCH_KEY, STORAGE_GROUP_ORDER_KEY, STORAGE_ORDER_KEY, STORAGE_PROJECT_FAVORITES_KEY, setUserPref } from './user-prefs.js';
import { activateSession } from './session-list.js';
import { isBatchOptions } from './approval-parser.js';
import type { SessionSnapshot } from '../types/proto.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

export interface TerminalEntry {
  term?: any;
  fitAddon?: any;
  container?: HTMLElement | null;
  pendingChunks: Uint8Array[];
  pendingTotalBytes?: number;
  pendingFlushActive?: boolean;
  pendingFlushSeq?: number;
  pendingFlushWatchdog?: ReturnType<typeof setTimeout> | null;
  webglAddon?: { dispose?: () => void } | null;
  pendingTextTail?: string;
  textDecoder?: TextDecoder;
  markerFilterCarry?: Uint8Array;
  reverseVideoFilterCarry?: Uint8Array;
  screenClearSeqCarry?: Uint8Array;
  liveStatusText?: string;
  // 更新が途切れたら稼働中→待機表示(idle)へ切替えるためのタイマー。
  // タイマーが生きている間＝稼働中、null＝待機（idle）。枠自体は消さず常時表示する。
  liveStatusHideTimer?: ReturnType<typeof setTimeout> | null;
  // ライブ進捗行の列アドレス再構成用（部分更新を 1 行へ組み立てる）
  liveLineRow?: number | null;   // 現在組み立て中の行番号（端末スクロールで動く）
  liveLineCells?: string[];      // 列 → 文字のスパース配列（1 列 1 要素 / 1-based）
  // compact（Claude /compact）中の経過秒表示用。中間 % が PTY に来ないため自前で発番する。
  compactingSince?: number | null;  // compact 開始時刻(ms)。経過秒の起点。null＝非 compact
  compactSeenAt?: number;           // 最後に compact フレームを観測した時刻(ms)。解除判定用
  compactDetectTail?: string;       // 直前チャンク末尾。検出語のチャンク境界分断対策の繰り越し
  autoScroll?: boolean;
  everAttached?: boolean;
  scrollHandlerInstalled?: boolean;
  scrollDisposable?: { dispose?: () => void };
  [key: string]: any;
}

export interface ApprovalOptionLike {
  num: number;
  label?: string;
  title?: string;
  isCurrent?: boolean;
  preserveOrder?: boolean;
  _ctx?: string;
  _sendText?: string;
  options?: ApprovalOptionLike[];
  [key: string]: any;
}

export interface SequentialChoicePrompt {
  key: string;
  question: string;
  options: ApprovalOptionLike[];
}

export const sessions = new Map<number, SessionSnapshot>();
export const terminals = new Map<number, TerminalEntry>(); // sessionId -> terminal state
export const approvalVisibleCache = new Map<number, boolean>();
export const multiQuestionVisibleCache = new Map<number, boolean>(); // sessionId → bool（Claude Code AskUserQuestion 等の複数質問 UI が画面に出ているか）
export const multiQuestionDismissedCache = new Map<number, boolean>(); // sessionId → bool（banner の ✕ ボタンで誤検出を手動 dismiss した状態。次の PTY 送信でクリア）
export const multiQuestionLatchAt = new Map<number, number>(); // sessionId → epoch ms（複数質問 UI のタブ行を最後にライブ検出した時刻。Ink 部分再描画でタブ行が一瞬窓から外れても multiQ 終了に倒さないためのデバウンス基準）
export const approvalSuppressedCache = new Map<number, string>(); // sessionId → 抑止理由（marker_leak / option_start / duplicate_option / box_rule / client_corrupt）。承認マーカーが構造破損で配信されなかったことの告知用
export const approvalSuppressedDismissedCache = new Map<number, boolean>(); // sessionId → bool（抑止告知バナーを ✕ で閉じた状態。新しい抑止／正常マーカー到着でクリア）
export const sequentialChoiceCache = new Map<number, any>(); // sessionId → { sig, prompts, answers, index }
export const approvalRawOptionsCache = new Map<number, ApprovalOptionLike[] | any[]>(); // sessionId → approval options
export interface ApprovalSourceState {
  source?: string;
  sig?: string;
  kind?: string;
  detectedAt?: string;
  candidateKey?: string;
  sourceEpoch?: number;
  shape?: string;
}
export const approvalSourceCache = new Map<number, ApprovalSourceState>(); // sessionId → current approval provenance/identity
export const approvalSourceEpochCache = new Map<number, number>(); // sessionId → logical live prompt generation
export const approvalReplayState = new Map<number, { replayEpoch: number; pending: boolean }>(); // sessionId → replay gate
export const approvalConsumedSig = new Map<number, string>(); // sessionId → 消費済み承認の署名（doSend でテキスト送信した場合の再表示防止）
// Candidate+epoch is the primary answered state. It is bounded per session and
// intentionally independent from display labels/block hashes.
export const answeredApprovalCandidates = new Map<number, Set<string>>(); // sessionId → `${epoch}\0${candidateKey}`
export const answeredApprovalShapeKeys = new Map<number, Map<string, string>>(); // sessionId → `${epoch}\0${shape}` -> candidateKey
export const replayAnsweredApprovalTokens = new Map<number, Set<string>>(); // replay-only suppression restored from Hub
const ANSWERED_CANDIDATE_LIMIT = 400;
// [MANY-AI-CLI] マーカー質問の「回答済み」恒久マーク（sessionId → ブロック全文ハッシュの集合）。
// approvalConsumedSig はタイマー失効する短期抑制で、Ink の SIGWINCH 再描画（タブ切替時など）で
// 画面に残った回答済みマーカーブロックが再流入すると失効後に再表示されてしまう。
// こちらは時間に依存せず、一度回答した [MANY-AI-CLI] ブロック（質問文＋全選択肢のハッシュ）を
// セッション存続中ずっと記録し、同一ブロックの検出を完全にスキップする。
// 質問文も含めたハッシュなので「Yes/No」等ラベルが同一でも別質問なら別ハッシュになり誤抑制しない。
// ネイティブ承認（go_vt / Codex 等のファイル編集・コマンド許可）は対象外＝毎回出す。
export const answeredMarkerSigs = new Map<number, Set<string>>(); // sessionId → Set<blockSig>
const ANSWERED_MARKER_SIG_LIMIT = 200; // 1 セッションあたりの保持上限（無制限増加防止）

export function recordAnsweredMarkerSig(id: number, opts: any): void {
  const sig = opts && opts[0] && opts[0]._blockSig;
  if (!sig) return;
  let set = answeredMarkerSigs.get(id);
  if (!set) { set = new Set<string>(); answeredMarkerSigs.set(id, set); }
  set.add(sig);
  // 上限超過時は最古から間引く（Set は挿入順を保持する）。
  while (set.size > ANSWERED_MARKER_SIG_LIMIT) {
    const oldest = set.values().next().value;
    if (oldest === undefined) break;
    set.delete(oldest);
  }
}

export function isAnsweredMarkerSig(id: number, opts: any): boolean {
  const sig = opts && opts[0] && opts[0]._blockSig;
  if (!sig) return false;
  const set = answeredMarkerSigs.get(id);
  return !!(set && set.has(sig));
}

function normalizeApprovalCandidateText(value: unknown): string {
  return String(value || '')
    .replace(/\u001b\[[0-?]*[ -/]*[@-~]/g, '')
    .replace(/\s+/g, ' ')
    .trim();
}

function candidateQuestionText(options: any[], questionOverride?: unknown): string {
  if (questionOverride && normalizeApprovalCandidateText(questionOverride)) {
    return normalizeApprovalCandidateText(questionOverride);
  }
  const arr: any = options as any;
  if (arr?._question) return normalizeApprovalCandidateText(arr._question);
  if (arr?.[0]?._question) return normalizeApprovalCandidateText(arr[0]._question);
  if (isBatchOptions(options)) {
    return options.map((section: any) => normalizeApprovalCandidateText(section?.title)).filter(Boolean).join('\n');
  }
  const sequential = (options || []).find((opt: any) => opt?._sequentialQuestion)?._sequentialQuestion;
  return normalizeApprovalCandidateText(sequential);
}

// This shape mirrors the Go candidate contract conceptually. The wire-provided
// candidate key is preferred whenever available; the local fallback is only
// used for old Hub messages and browser-only parser paths.
export function approvalCandidateShape(id: number, options: ApprovalOptionLike[] | any[], kind = 'fallback', questionOverride?: unknown): string {
  const provider = normalizeApprovalCandidateText(sessions.get(id)?.provider || '').toLowerCase();
  const question = candidateQuestionText(options, questionOverride);
  const flat = isBatchOptions(options)
    ? options.flatMap((section: any) => section?.options || [])
    : (options || []);
  const entries = flat
    .map((opt: any) => `${Number(opt?.num)}:${normalizeApprovalCandidateText(opt?._sendText || opt?.send_text)}`)
    .sort();
  return `${provider}\n${normalizeApprovalCandidateText(kind).toLowerCase()}\n${question}\n${entries.join('\n')}`;
}

export function approvalCandidateIdentity(id: number, options: ApprovalOptionLike[] | any[], kind = 'fallback', questionOverride?: unknown): { candidateKey: string; sourceEpoch: number; shape: string } {
  const arr: any = options as any;
  const explicitKey = String(arr?._candidateKey || arr?.[0]?._candidateKey || '').trim();
  const explicitEpoch = Number(arr?._sourceEpoch || arr?.[0]?._sourceEpoch || 0);
  const shape = approvalCandidateShape(id, options, kind, questionOverride);
  const source = approvalSourceCache.get(id);
  const currentEpoch = getApprovalSourceEpoch(id);
  const sourceEpoch = explicitEpoch > currentEpoch
    ? explicitEpoch
    : Math.max(currentEpoch, source?.shape === shape ? Number(source.sourceEpoch || 0) : 0, 1);
  const answeredShapeKey = answeredApprovalShapeKeys.get(id)?.get(answeredApprovalShapeToken(sourceEpoch, shape));
  const candidateKey = explicitKey || (source?.candidateKey && source.shape === shape ? source.candidateKey : '') || answeredShapeKey || `local:${_approvalCtxHash(shape)}`;
  return { candidateKey, sourceEpoch: sourceEpoch > 0 ? sourceEpoch : 1, shape };
}

export function annotateApprovalIdentity(options: any, identity: { candidateKey: string; sourceEpoch: number }): any {
  if (!options || typeof options !== 'object') return options;
  try {
    options._candidateKey = identity.candidateKey;
    options._sourceEpoch = identity.sourceEpoch;
    for (const section of options) {
      if (!section || typeof section !== 'object') continue;
      section._candidateKey = identity.candidateKey;
      section._sourceEpoch = identity.sourceEpoch;
      for (const option of (section.options || [])) {
        if (option && typeof option === 'object') {
          option._candidateKey = identity.candidateKey;
          option._sourceEpoch = identity.sourceEpoch;
        }
      }
    }
  } catch (_) {}
  return options;
}

export function getApprovalSourceEpoch(id: number): number {
  const current = Number(approvalSourceEpochCache.get(id) || 0);
  if (current > 0) return current;
  approvalSourceEpochCache.set(id, 1);
  return 1;
}

export function noteApprovalSourceEpoch(id: number, epoch: unknown): number {
  const next = Number(epoch || 0);
  if (!Number.isFinite(next) || next <= 0) return getApprovalSourceEpoch(id);
  const current = Number(approvalSourceEpochCache.get(id) || 0);
  if (next > current) {
    approvalSourceEpochCache.set(id, next);
    // The legacy short sig is intentionally cleared only at a new live
    // generation. Candidate+epoch history remains intact, so replay in the
    // old generation stays suppressed while an intentionally repeated prompt
    // can be shown.
    approvalConsumedSig.delete(id);
  }
  else if (current === 0) approvalSourceEpochCache.set(id, next);
  return Number(approvalSourceEpochCache.get(id) || next);
}

export function beginApprovalReplay(id: number, replayEpoch: unknown): void {
  const next = Number(replayEpoch || 0) || 1;
  const current = approvalReplayState.get(id);
  if (!current || next >= current.replayEpoch) {
    approvalReplayState.set(id, { replayEpoch: Math.max(next, current?.replayEpoch || 0), pending: true });
  } else {
    current.pending = true;
  }
}

export function finishApprovalReplay(id: number, replayEpoch: unknown, sourceEpoch?: unknown, consumedCandidateKey = '', consumedShape = ''): boolean {
  const next = Number(replayEpoch || 0) || 1;
  const current = approvalReplayState.get(id);
  if (current && next < current.replayEpoch) return false;
  approvalReplayState.set(id, { replayEpoch: Math.max(next, current?.replayEpoch || 0), pending: false });
  const liveEpoch = noteApprovalSourceEpoch(id, sourceEpoch);
  if (consumedCandidateKey && consumedShape) {
    // The shape is associated with the generation currently restored in the
    // browser. The Hub's consumed epoch is retained on the wire for audit, but
    // using the live replay epoch here prevents an old scrollback candidate
    // from being mistaken for a future prompt until a fresh Hub candidate is
    // explicitly announced.
    recordAnsweredApprovalIdentity(id, consumedCandidateKey, liveEpoch, consumedShape, true);
  }
  return true;
}

export function isApprovalReplayPending(id: number): boolean {
  return !!approvalReplayState.get(id)?.pending;
}

export function approvalCandidateDebugKey(candidateKey: unknown): string {
  return _approvalCtxHash(String(candidateKey || '')).slice(0, 10);
}

function answeredCandidateToken(identity: { candidateKey: string; sourceEpoch: number }): string {
  return `${identity.sourceEpoch}\0${identity.candidateKey}`;
}

function answeredApprovalShapeToken(sourceEpoch: number, shape: string): string {
  return `${sourceEpoch}\0${shape}`;
}

function rememberAnsweredApprovalIdentity(id: number, identity: { candidateKey: string; sourceEpoch: number; shape?: string }, replayOnly = false): void {
  let set = answeredApprovalCandidates.get(id);
  if (!set) { set = new Set<string>(); answeredApprovalCandidates.set(id, set); }
  const token = answeredCandidateToken(identity);
  set.add(token);
  if (identity.shape) {
    let shapes = answeredApprovalShapeKeys.get(id);
    if (!shapes) { shapes = new Map<string, string>(); answeredApprovalShapeKeys.set(id, shapes); }
    shapes.set(answeredApprovalShapeToken(identity.sourceEpoch, identity.shape), identity.candidateKey);
  }
  if (replayOnly) {
    let replayTokens = replayAnsweredApprovalTokens.get(id);
    if (!replayTokens) { replayTokens = new Set<string>(); replayAnsweredApprovalTokens.set(id, replayTokens); }
    replayTokens.add(token);
  }
  while (set.size > ANSWERED_CANDIDATE_LIMIT) {
    const oldest = set.values().next().value;
    if (oldest === undefined) break;
    set.delete(oldest);
    replayAnsweredApprovalTokens.get(id)?.delete(oldest);
    const separator = oldest.indexOf('\0');
    const oldestEpoch = separator >= 0 ? oldest.slice(0, separator) : '';
    const oldestKey = separator >= 0 ? oldest.slice(separator + 1) : '';
    const shapes = answeredApprovalShapeKeys.get(id);
    if (shapes && oldestEpoch && oldestKey) {
      for (const [shapeToken, shapeKey] of shapes) {
        if (shapeKey === oldestKey && shapeToken.startsWith(`${oldestEpoch}\0`)) shapes.delete(shapeToken);
      }
      if (shapes.size === 0) answeredApprovalShapeKeys.delete(id);
    }
  }
}

export function recordAnsweredApprovalIdentity(id: number, candidateKey: string, sourceEpoch: number, shape = '', replayOnly = false): void {
  if (!candidateKey || sourceEpoch <= 0) return;
  rememberAnsweredApprovalIdentity(id, { candidateKey, sourceEpoch, shape }, replayOnly);
}

export function clearReplayAnsweredApprovalCandidate(id: number, identity: { candidateKey: string; sourceEpoch: number; shape?: string }): void {
  const token = answeredCandidateToken(identity);
  const replayTokens = replayAnsweredApprovalTokens.get(id);
  if (!replayTokens?.has(token)) return;
  replayTokens.delete(token);
  if (replayTokens.size === 0) replayAnsweredApprovalTokens.delete(id);
  answeredApprovalCandidates.get(id)?.delete(token);
  if (answeredApprovalCandidates.get(id)?.size === 0) answeredApprovalCandidates.delete(id);
  if (identity.shape) {
    const shapes = answeredApprovalShapeKeys.get(id);
    shapes?.delete(answeredApprovalShapeToken(identity.sourceEpoch, identity.shape));
    if (shapes?.size === 0) answeredApprovalShapeKeys.delete(id);
  }
}

export function recordAnsweredApprovalCandidate(id: number, options: any, kind?: string, questionOverride?: unknown): { candidateKey: string; sourceEpoch: number; shape: string } | null {
  if (!Array.isArray(options) || options.length === 0) return null;
  const identity = approvalCandidateIdentity(id, options, kind || (approvalSourceCache.get(id)?.source === 'go_vt' ? 'native' : 'marker'), questionOverride);
  if (!identity.candidateKey) return null;
  rememberAnsweredApprovalIdentity(id, identity);
  annotateApprovalIdentity(options, identity);
  return identity;
}

export function isAnsweredApprovalCandidate(id: number, options: any, kind?: string, questionOverride?: unknown): boolean {
  if (!Array.isArray(options) || options.length === 0) return false;
  const identity = approvalCandidateIdentity(id, options, kind || (approvalSourceCache.get(id)?.source === 'go_vt' ? 'native' : 'marker'), questionOverride);
  const set = answeredApprovalCandidates.get(id);
  return !!(set && set.has(answeredCandidateToken(identity)));
}
export const batchSelections = new Map<number, number[]>(); // sessionId → number[]（セクションごとの選択番号、未選択は null、自由入力選択中は -1）
export const batchFreeText = new Map<number, string[]>(); // sessionId → string[]（質問タブUIの自由入力テキスト、セクションごと）
export const batchActiveQ = new Map<number, number>(); // sessionId → アクティブな質問タブ index（質問タブUI）
export let batchFocusIdx = -1; // 現在フォーカス中のバッチセクション index（-1: 未フォーカス / 範囲外）
export const multiSelectSelections = new Map<number, Set<number>>(); // sessionId → Set<選択番号>（複数選択 #multi の ON 状態）
export let multiSelectFocusIdx = -1; // 現在フォーカス中の複数選択肢 index（-1: 未フォーカス）
export const approvalConsumedSigDeleteTimer = new Map<number, ReturnType<typeof setTimeout>>(); // sessionId → timer（sig を debounce 型で削除するためのタイマー）
export const approvalSwitchCandidates = new Map<number, any>(); // sessionId → { sig, options, firstSeenAt }（表示中の承認と異なる選択肢が検出されたときの安定性チェック用）
export const APPROVAL_PENDING_TEXT_TAIL_LIMIT = 12000;

// 承認選択肢の sig を計算。Ink の再描画やスクロールバック残骸による
// label の微妙な差異（前後空白、空白の重複、truncate 位置）を吸収するため normalize する。
// (Y:1/N:0) Yes/No プロンプトはどれも同じ label を持つため、_ctx に質問文ハッシュを
// 載せて区別する（連続する別質問が同一 sig で誤抑制されないように）。
export function approvalSig(options: ApprovalOptionLike[] | any[]): string {
  if (isBatchOptions(options)) {
    return JSON.stringify(options.map(s => ({
      n: s.num,
      t: String(s.title || '').replace(/\s+/g, ' ').slice(0, 80),
      o: (s.options || []).map((o: ApprovalOptionLike) => `${o.num}:${String(o.label || '').trim().replace(/\s+/g, ' ').slice(0, 80)}`),
    })));
  }
  return JSON.stringify((options || []).map(o => {
    const lbl = String(o.label || '').trim().replace(/\s+/g, ' ').slice(0, 80);
    const ctx = o && o._ctx ? `|${o._ctx}` : '';
    return `${o.num}:${lbl}${ctx}`;
  }));
}

// シンプルな文字列ハッシュ (djb2)。承認質問文の同一性判定に使う。
export function _approvalCtxHash(s: unknown): string {
  const text = String(s || '').replace(/\s+/g, ' ').trim();
  let h = 5381;
  for (let i = 0; i < text.length; i++) {
    h = (((h << 5) + h) + text.charCodeAt(i)) | 0;
  }
  return (h >>> 0).toString(36);
}

// 手動 dismiss 抑止用の「質問アイデンティティ」。
// Grok 差分再描画では option ラベルが欠け・重複して approvalSig が揺れるが、
// 質問文（_question）は比較的安定するため、同一質問の再表示抑止に使う。
// 質問が取れない経路（ネイティブ承認等）は options の approvalSig にフォールバック。
export function approvalQuestionKey(options: ApprovalOptionLike[] | any[] | null | undefined): string {
  if (!options || !Array.isArray(options) || options.length === 0) return '';
  const arrQ = (options as any)._question;
  if (arrQ && String(arrQ).trim()) {
    return 'q:' + _approvalCtxHash(String(arrQ).replace(/\s+/g, ' ').trim().slice(0, 200));
  }
  const first = options[0] as any;
  if (first && first._multiSelect && first._question && String(first._question).trim()) {
    return 'q:' + _approvalCtxHash(String(first._question).replace(/\s+/g, ' ').trim().slice(0, 200));
  }
  if (isBatchOptions(options)) {
    const titles = options
      .map((s: any) => String(s?.title || '').replace(/\s+/g, ' ').trim())
      .filter(Boolean)
      .join('\n');
    if (titles) return 'b:' + _approvalCtxHash(titles.slice(0, 400));
  }
  return 'o:' + approvalSig(options);
}

export function sequentialChoiceSig(prompts: SequentialChoicePrompt[] | any[]): string {
  return _approvalCtxHash((prompts || []).map(p => `${p.key}:${p.question}:${p.options.map((o: ApprovalOptionLike) => `${o.num}.${o.label}`).join('|')}`).join('\n'));
}
export const approvalHintConfirmTimers = new Map<number, ReturnType<typeof setTimeout>>(); // sessionId → timer（生バイト検出を短時間 debounce してチカチカを防ぐ）
export const approvalHintConfirmTrusted = new Map<number, boolean>(); // sessionId → true: marker/plainYesNo 由来の信頼性の高い検出（fallback に上書きさせない）
export const sessionInputState = new Map<number, any>(); // sessionId → { inputValue, pastedTextsData, pendingAttachFiles }（サムネイルは各エントリの wrapper から再構築）

// =========================================================================
// chatHistory store (plan_chat-history-subview.md §C1)
//
// セッションごとのチャットを保持する in-memory store。
// C2 (タブ切替) / C3 (吹き出しレンダラ) が購読して描画する。
//
// メッセージ shape:
//   { id, ts, role, kind, rawText, normalizedText, attachments, tool, meta }
//   role  : 'user' | 'ai' | 'system'
//   kind  : 'text' | 'attach' | 'approval' | 'tool'
//   rawText        : 生 PTY テキスト（StripANSI 適用済み / D16: raw 切替用）
//   normalizedText : 軽い正規化を適用したレンダリング用テキスト (D15)
//   attachments    : kind='attach' のとき [{path?, filename?, kind:'image'|'file'}]
//   tool           : kind='tool' のとき { name, args, ... }（C3 で扱う）
//   meta           : 任意の付随情報（approval の question/answer など）
//
// API:
//   pushMessage(sid, msg)    : メッセージを追加し subscriber 通知
//   getMessages(sid)         : メッセージ配列の浅いコピーを取得
//   subscribe(sid, cb)       : 変化通知購読 (unsubscribe 関数を返す)
//   onSessionRemoved(sid)    : ストア + subscriber を破棄
// =========================================================================

export const chatHistory = new Map<number, any[]>();              // sid → Message[]
export const chatHistorySubs = new Map<number, Set<(...args: any[]) => void>>();          // sid → Set<callback>
export const chatHistoryIdSeq = new Map<number, number>();         // sid → 次に振る連番 (1 始まり)
export const chatHistoryOutputBuffers = new Map<number, any>(); // sid → { rawChunks:[], lastTs }
export const chatHistoryAutoCommitTimers = new Map<number, ReturnType<typeof setTimeout>>(); // sid → timerId
// Go 側 chatHistoryUserTurnMarker と一致させること
export const CHAT_HISTORY_USER_TURN_MARKER = "\x1b]47777;user-turn\x07";

export const autoDismissTimers = new Map<number, ReturnType<typeof setTimeout>>(); // sessionId → timer
export const approvalSuppressUntil = new Map<number, number>(); // sessionId → timestamp (sendChoice 後の誤再表示を抑制)
export const approvalAutoSwitchQueue: number[] = [];
export const utf8Decoder = new TextDecoder('utf-8');
export const utf8Encoder = new TextEncoder();

export let activeSessionId: number | null = null;
export let isComposing = false;       // IMEコンポジション状態
export let pendingSend = false;       // IME確定後に送信するフラグ
export let composeEndSendTimer: ReturnType<typeof setTimeout> | null = null; // compositionend が doSend をスケジュール済みの場合のタイマーID
export let lastDoSendAt = 0;          // 直前の doSend 実行時刻（二重送信防止の短時間ガード用）
export const DOUBLE_SEND_GUARD_MS = 100;
export const SIDEBAR_COLLAPSED_WIDTH_THRESHOLD = 180;
// action-bar の点滅防止用
// - lastActionBarRender: 前回描画した内容のシグネチャ（同一なら DOM 再構築をスキップ）
export const lastActionBarRender: { sessionId: number | null; sig: string | null } = { sessionId: null, sig: null };
export let _elapsedTimerInterval: ReturnType<typeof setInterval> | null = null;
export let dragSrcId: number | null = null;
export let dragSrcGroupKey: string | null = null;
export let dragOverCardEl: Element | null = null;

function readStorageArray(key: string): any[] {
  try {
    const value = JSON.parse(localStorage.getItem(key) || '[]');
    return Array.isArray(value) ? value : [];
  } catch (_) {
    return [];
  }
}

export function isSessionLiveRenderedInMultiPane(id: number): boolean {
  const multiView = document.getElementById('multi-view');
  const mgr = window.multiPaneManager;
  if (!multiView || multiView.hidden || !mgr || !Array.isArray(mgr.slots)) return false;
  const t = terminals.get(id);
  if (!t || !t.everAttached || !t.container || !t.container.isConnected) return false;
  if (!multiView.contains(t.container)) return false;
  return mgr.slots.some((slot: any) => slot && slot.session && slot.session.id === id);
}
export let dragOverGroupEl: Element | null = null;
export let pendingAutoSwitch = false;
export let actionBarFocusIdx = -1;
export let approvalAutoSwitchInProgress = false;
export const actionBarShownAt = new Map<number, number>(); // sessionId -> timestamp(ms), Enter即確定ガード用

export let projectFavorites: string[] = readStorageArray(STORAGE_PROJECT_FAVORITES_KEY);
// sessionOrder はセッション ID の数値配列。sessions は数値キーの Map なので、
// 文字列で保存された旧値をそのまま載せると sessions.has() が全て false になり
// 手動並び順が丸ごと無視される。読み込み時点で数値へ寄せておく。
export let sessionOrder: number[] = readStorageArray(STORAGE_ORDER_KEY)
  .map((v) => (typeof v === 'number' ? v : parseInt(String(v), 10)))
  .filter((v) => Number.isInteger(v));
export let groupOrder: string[] = readStorageArray(STORAGE_GROUP_ORDER_KEY);
export const collapsedGroups = new Set<string>();

export function saveProjectFavorites() {
  setUserPref('project_favorites', projectFavorites);
}

export function saveGroupOrder() {
  setUserPref('group_order', groupOrder);
}

export function saveSessionOrder() {
  setUserPref('session_order', sessionOrder);
}

// cwd からプロジェクトキーを派生する。
// renderSessionList のグループ化（末尾セグメント）と同じ規則で揃え、
// FilesTabManager の可視性判定（curSess.project 参照）が一貫して機能するようにする。
export function deriveProjectKeyFromCwd(cwd: unknown): string {
  if (!cwd) return '';
  const name = String(cwd).replace(/\\/g, '/').split('/').filter(p => p.length > 0).pop() || '';
  return name;
}

export function addToSessionOrder(id: number, forceToFront = false): void {
  const idx = sessionOrder.indexOf(id);
  if (idx !== -1) {
    if (forceToFront) { sessionOrder.splice(idx, 1); sessionOrder.unshift(id); }
  } else {
    // C5: 新規セッションは非★グループの末尾スロットに入るよう末尾追加（push）。
    // forceToFront=true の場合のみ先頭追加（非新規の既存セッション再登録用）。
    if (forceToFront) {
      sessionOrder.unshift(id);
    } else {
      sessionOrder.push(id);
    }
  }
}

export function removeFromSessionOrder(id: number): void {
  const idx = sessionOrder.indexOf(id);
  if (idx !== -1) { sessionOrder.splice(idx, 1); saveSessionOrder(); }
}

// orderSessions は全モジュール共通のセッション整列ロジック（C9: 旧 getSortedSessions /
// getOrderedSessions / multi-pane フォールバックの三重定義を 1 つに集約）。
// 📌 ピン留めセッションを先頭に、残りを sessionOrder 順（未登録セッションは末尾）で並べる。
// sessions / sessionOrder 未定義時は空配列または id 昇順へフォールバックする
// （旧 multi-pane.js の防御的フォールバックを継承）。
export function orderSessions(): SessionSnapshot[] {
  if (typeof sessions === 'undefined') return [];
  if (typeof sessionOrder === 'undefined') {
    return Array.from(sessions.values()).sort((a, b) => a.id - b.id);
  }
  const orderedIds = sessionOrder.filter(id => sessions.has(id));
  sessions.forEach((s) => {
    if (!orderedIds.includes(s.id)) orderedIds.push(s.id);
  });
  const ordered = orderedIds.map(id => sessions.get(id)).filter(Boolean) as SessionSnapshot[];
  return [...ordered.filter(s => s.pinned), ...ordered.filter(s => !s.pinned)];
}
window.orderSessions = orderSessions;
// 後方互換エイリアス: multi-pane.js など window.getSortedSessions 参照箇所のために残す。
window.getSortedSessions = orderSessions;

export function isApprovalAutoSwitchEnabled() {
  return localStorage.getItem(STORAGE_APPROVAL_AUTO_SWITCH_KEY) === '1';
}

export function isCurrentSessionHoldingApprovalFocus() {
  if (activeSessionId === null) return false;
  return !!approvalVisibleCache.get(activeSessionId);
}

export function removeApprovalAutoSwitchTarget(sessionId: number): void {
  for (let i = approvalAutoSwitchQueue.length - 1; i >= 0; i--) {
    if (approvalAutoSwitchQueue[i] === sessionId) approvalAutoSwitchQueue.splice(i, 1);
  }
  // C4: 承認解決時にマルチペインバッジをセッション state に合わせて更新
  const mgr = window.multiPaneManager;
  if (mgr && typeof mgr.updateSlotBadge === 'function') {
    const s = sessions.get(sessionId);
    const badgeStatus = (s && s.state === 'running') ? 'running' : 'standby';
    mgr.updateSlotBadge(sessionId, badgeStatus);
  }
}

export function maybeAutoSwitchToNextApproval() {
  if (!isApprovalAutoSwitchEnabled()) return;
  if (approvalAutoSwitchInProgress) return;
  if (activeSessionId === null) return;
  if (isCurrentSessionHoldingApprovalFocus()) return;
  const bar = document.getElementById('action-bar');
  if (bar && bar.classList.contains('visible')) return;
  const panel = document.getElementById('settings-panel');
  if (panel && !panel.hidden) return;

  while (approvalAutoSwitchQueue.length > 0) {
    const nextId = approvalAutoSwitchQueue[0];
    if (!sessions.has(nextId) || !approvalVisibleCache.get(nextId) || nextId === activeSessionId) {
      approvalAutoSwitchQueue.shift();
      continue;
    }
    approvalAutoSwitchQueue.shift();
    approvalAutoSwitchInProgress = true;
    try {
      activateSession(nextId);
    } finally {
      approvalAutoSwitchInProgress = false;
    }
    return;
  }
}

export function enqueueApprovalAutoSwitch(sessionId: number): void {
  if (!isApprovalAutoSwitchEnabled()) return;
  if (sessionId === activeSessionId) return;
  if (!sessions.has(sessionId)) return;
  if (!approvalAutoSwitchQueue.includes(sessionId)) {
    approvalAutoSwitchQueue.push(sessionId);
  }
  // C4: 承認待ちになったときマルチペインバッジを 'waiting' に更新
  const mgr = window.multiPaneManager;
  if (mgr && typeof mgr.updateSlotBadge === 'function') {
    mgr.updateSlotBadge(sessionId, 'waiting');
  }
  maybeAutoSwitchToNextApproval();
}

// --- ESM cross-module setters (generated) ---
export function set__elapsedTimerInterval(v: ReturnType<typeof setInterval> | null) { _elapsedTimerInterval = v; }
export function set_actionBarFocusIdx(v: number) { actionBarFocusIdx = v; }
export function set_activeSessionId(v: number | null) {
  try { console.log('[approval-route] set_activeSessionId', { from: activeSessionId, to: v, stack: new Error().stack?.split('\n').slice(1, 5).join(' | ') }); } catch (_) {}
  activeSessionId = v;
}
export function set_batchFocusIdx(v: number) { batchFocusIdx = v; }
export function set_multiSelectFocusIdx(v: number) { multiSelectFocusIdx = v; }
export function set_composeEndSendTimer(v: ReturnType<typeof setTimeout> | null) { composeEndSendTimer = v; }
export function set_dragOverCardEl(v: Element | null) { dragOverCardEl = v; }
export function set_dragOverGroupEl(v: Element | null) { dragOverGroupEl = v; }
export function set_dragSrcGroupKey(v: string | null) { dragSrcGroupKey = v; }
export function set_dragSrcId(v: number | null) { dragSrcId = v; }
export function set_groupOrder(v: string[]) { groupOrder = v; }
export function set_isComposing(v: boolean) { isComposing = v; }
export function set_lastDoSendAt(v: number) { lastDoSendAt = v; }
export function set_pendingAutoSwitch(v: boolean) { pendingAutoSwitch = v; }
export function set_pendingSend(v: boolean) { pendingSend = v; }

// --- ESM window-interop publish (generated; preserves dynamic window.* lookups) ---
window.terminals = terminals;
// activeSessionId は実行中に変化するため、素の代入だと評価時の初期値 (null) を焼き付けて
// 二度と更新されない。加えて chat-history.js が同名を「setter 無しの getter」として先に
// 定義する評価順のため、素の代入は strict モードで
//   TypeError: Cannot set property activeSessionId which has only a getter
// を投げ、state.js の評価が中断して後続モジュール (spawn-panel 等) が読み込まれなくなる。
// configurable な live getter として定義し、値の最新性と評価順非依存の両方を満たす。
Object.defineProperty(window, 'activeSessionId', {
  configurable: true,
  get() { return activeSessionId; },
});
