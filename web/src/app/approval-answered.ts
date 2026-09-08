// 承認の「同一性」と「回答済み」を持つ唯一の場所。
//
// 以前はここの役目が 3 つの state に分かれていた。タイマーで失効する署名
// (approvalConsumedSig)、ブロック全文ハッシュの恒久マーク (answeredMarkerSigs)、
// 手動 dismiss 用の質問キー (approvalQuestionKey) で、それぞれ「同じ質問とは何か」の
// 定義が違った。3 者の食い違いが、回答済みの承認が出戻る症状の温床になっていた
// （docs/local/plan_v0.7-scope_c2_approval-identity_c2_replay-generation.md）。
//
// 現在は candidateKey + sourceEpoch の 1 組だけで判定する。
//   - candidateKey: provider・承認種別・正規化した質問・選択肢番号と送信文字列から作る。
//     ラベルの空白・罫線・折返しは含めないので、TUI の再描画で揺れても変わらない。
//   - sourceEpoch : live prompt の世代。replay と reflow では進まない。世代が進めば
//     同じ質問文でも別の候補として扱うので、意図的な再質問は表示される。
//
// このモジュールは DOM に触らない。state.ts から切り出してあるのは、承認の同一性を
// ブラウザ無しで試せるようにするため（approval-owner.ts と同じ方針）。
import { isBatchOptions } from './approval-parser.js';
import type { ApprovalOptionLike } from './state.js';

export interface ApprovalSourceState {
  source?: string;
  sig?: string;
  kind?: string;
  detectedAt?: string;
  candidateKey?: string;
  sourceEpoch?: number;
  shape?: string;
  /** Hub 側の供給元ラベル（approval_marker の approval_source。'transcript' / 'go_vt'）。 */
  hubSource?: string;
}

export interface ApprovalCandidateIdentity {
  candidateKey: string;
  sourceEpoch: number;
  shape: string;
}

export const approvalSourceCache = new Map<number, ApprovalSourceState>(); // sessionId → current approval provenance/identity
export const approvalSourceEpochCache = new Map<number, number>(); // sessionId → logical live prompt generation
export const hubMarkerDeliveredEpoch = new Map<number, number>(); // sessionId → epoch in which Hub last delivered a marker
export const approvalReplayState = new Map<number, { replayEpoch: number; pending: boolean }>(); // sessionId → replay gate
export const answeredApprovalCandidates = new Map<number, Set<string>>(); // sessionId → `${epoch}\0${candidateKey}`
export const answeredApprovalShapeKeys = new Map<number, Map<string, string>>(); // sessionId → `${epoch}\0${shape}` -> candidateKey
export const replayAnsweredApprovalTokens = new Map<number, Set<string>>(); // replay-only suppression restored from Hub
const ANSWERED_CANDIDATE_LIMIT = 400;

// provider は sessions（state.ts）にしか無いが、そこを import すると app 全体を
// 引き込んでしまう。参照方向を逆にして、state.ts 側から解決関数を差してもらう。
let resolveProvider: (id: number) => string = () => '';

export function setApprovalProviderResolver(resolver: (id: number) => string): void {
  resolveProvider = resolver;
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
  const provider = normalizeApprovalCandidateText(resolveProvider(id)).toLowerCase();
  const question = candidateQuestionText(options, questionOverride);
  const flat = isBatchOptions(options)
    ? options.flatMap((section: any) => section?.options || [])
    : (options || []);
  const entries = flat
    .map((opt: any) => `${Number(opt?.num)}:${normalizeApprovalCandidateText(opt?._sendText || opt?.send_text)}`)
    .sort();
  return `${provider}\n${normalizeApprovalCandidateText(kind).toLowerCase()}\n${question}\n${entries.join('\n')}`;
}

export function approvalCandidateIdentity(id: number, options: ApprovalOptionLike[] | any[], kind = 'fallback', questionOverride?: unknown): ApprovalCandidateIdentity {
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
    // Candidate+epoch history is kept across generations on purpose: replay in
    // the old generation stays suppressed, while an intentionally repeated
    // prompt in the new generation is shown because its token differs.
    approvalSourceEpochCache.set(id, next);
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

function defaultKindFor(id: number): string {
  return approvalSourceCache.get(id)?.source === 'go_vt' ? 'native' : 'marker';
}

export function recordAnsweredApprovalCandidate(id: number, options: any, kind?: string, questionOverride?: unknown): ApprovalCandidateIdentity | null {
  if (!Array.isArray(options) || options.length === 0) return null;
  const identity = approvalCandidateIdentity(id, options, kind || defaultKindFor(id), questionOverride);
  if (!identity.candidateKey) return null;
  rememberAnsweredApprovalIdentity(id, identity);
  annotateApprovalIdentity(options, identity);
  return identity;
}

export function isAnsweredApprovalCandidate(id: number, options: any, kind?: string, questionOverride?: unknown): boolean {
  if (!Array.isArray(options) || options.length === 0) return false;
  const identity = approvalCandidateIdentity(id, options, kind || defaultKindFor(id), questionOverride);
  const set = answeredApprovalCandidates.get(id);
  return !!(set && set.has(answeredCandidateToken(identity)));
}

/**
 * この shape（provider・種別・質問・選択肢番号・送信文字列）の承認に、このセッションで
 * 一度でも回答したか。世代（sourceEpoch）は見ない。
 *
 * 通常の判定 isAnsweredApprovalCandidate は世代込みで見る。世代が進めば同じ質問でも
 * 新しい候補として出すのが仕様だからで、それはこのファイル冒頭のルールどおり。この関数は
 * その世代の縛りを外して「同じ中身に答えたことがあるか」だけを見る。
 *
 * 用途は 1 つだけ。CLI がページ送りで過去の画面を描き直している間に届いた候補を採らない
 * 条件に使う（approval-ui.ts の showOptions）。回答済み state を新しく増やさず、既存の
 * 台帳を別の角度から引くだけにしてある（承認の同一性は 1 本という規約のため）。
 */
export function isAnsweredApprovalShapeAcrossEpochs(id: number, shape: string): boolean {
  if (!shape) return false;
  const shapes = answeredApprovalShapeKeys.get(id);
  if (!shapes) return false;
  const suffix = `\0${shape}`;
  for (const token of shapes.keys()) {
    if (token.endsWith(suffix)) return true;
  }
  return false;
}

/**
 * Hub がこの世代のマーカー承認を配信したことを記録する。
 *
 * 回答済みかどうかとは別の話なので、承認の同一性 state（candidateKey + sourceEpoch）は
 * 増やしていない。ここが持つのは「今の世代でどちらの検出層が正本か」という provenance で、
 * 既存の approvalSourceCache と同じ種類のもの。approvalSourceCache と分けてあるのは、
 * あちらが hideActionBar で消えるのに対し、こちらは世代が変わるまで残す必要があるため。
 */
export function noteHubMarkerDelivered(id: number, sourceEpoch: number): void {
  if (!id || !(sourceEpoch > 0)) return;
  const current = hubMarkerDeliveredEpoch.get(id) || 0;
  if (sourceEpoch > current) hubMarkerDeliveredEpoch.set(id, sourceEpoch);
}

/**
 * 今の世代について Hub のマーカー検出が正本か。
 *
 * true の間、ブラウザ側のローカル走査（pendingTextTail 由来）は承認候補を新しく立てない。
 * ローカル走査は PTY の生テキストを継ぎ足した文字列を見ており、Ink の差分再描画では
 * 書き換わらなかった文字が届かないため、同じ質問から Hub と違う本文を作ることがある。
 * 実測（2026-08-26 / mer session #5）: 回答の 6 秒後に「SSH して」が「SSH て」になった
 * 再パースが回答済み台帳を外し、回答済みの承認が再点灯して 保留中 が下がらなくなった。
 * Hub は本物の VT グリッドを持つので同じ入力で候補キーが揺れていない。
 *
 * 世代で切ってあるのが要点。Hub がこの世代で何も配信していない場面
 * （開始マーカーが画面外にあってブロックを抽出できない等）では false のままなので、
 * ローカル走査は従来どおり最後の砦として働く。
 */
export function isHubMarkerAuthoritative(id: number): boolean {
  const delivered = hubMarkerDeliveredEpoch.get(id) || 0;
  return delivered > 0 && delivered >= getApprovalSourceEpoch(id);
}

/**
 * CLI がページ送りで過去の画面を描き直している最中に届いた、回答済みの中身か。
 *
 * 2026-08-23 に showOptions() へ入れた条件をここへ移した（bugfix_approval-bar-stale-
 * options-scroll-mismatch_2026-08-19.md）。当時は描画判定にしか無かったため、パネルは
 * 出ないのに 保留中 バッジ・通知音・auto-switch だけが立ち、パネルが無いので閉じることも
 * できなかった。受け口（handleGoApprovalDetected / handleHubApprovalMarker）が状態を
 * 触る前にも同じ判定を通すため、判定式を 1 箇所に集約する。
 *
 * 遡り位置の取得だけは terminal.ts 側にあるので、呼び出し側から渡してもらう
 * （このモジュールは DOM にも xterm にも触らない方針のため）。
 */
export function isStaleHistoryRepaint(id: number, shape: string, showingHistory: boolean): boolean {
  return !!showingHistory && isAnsweredApprovalShapeAcrossEpochs(id, shape);
}

/** テスト専用。1 セッションぶんの承認同一性 state を初期状態へ戻す。 */
export function _resetApprovalAnsweredStateForTest(id: number): void {
  answeredApprovalCandidates.delete(id);
  answeredApprovalShapeKeys.delete(id);
  replayAnsweredApprovalTokens.delete(id);
  approvalSourceEpochCache.delete(id);
  approvalSourceCache.delete(id);
  approvalReplayState.delete(id);
  hubMarkerDeliveredEpoch.delete(id);
}
