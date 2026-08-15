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
}

export interface ApprovalCandidateIdentity {
  candidateKey: string;
  sourceEpoch: number;
  shape: string;
}

export const approvalSourceCache = new Map<number, ApprovalSourceState>(); // sessionId → current approval provenance/identity
export const approvalSourceEpochCache = new Map<number, number>(); // sessionId → logical live prompt generation
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

/** テスト専用。1 セッションぶんの承認同一性 state を初期状態へ戻す。 */
export function _resetApprovalAnsweredStateForTest(id: number): void {
  answeredApprovalCandidates.delete(id);
  answeredApprovalShapeKeys.delete(id);
  replayAnsweredApprovalTokens.delete(id);
  approvalSourceEpochCache.delete(id);
  approvalSourceCache.delete(id);
  approvalReplayState.delete(id);
}
