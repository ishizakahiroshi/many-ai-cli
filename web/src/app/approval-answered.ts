// 承認の「同一性」と「回答済み」を持つ唯一の場所。
//
// 以前はここの役目が 3 つの state に分かれていた。タイマーで失効する署名、
// ブロック全文ハッシュの恒久マーク、手動 dismiss 用の質問キーで、それぞれ「同じ質問とは何か」の
// 定義が違った。3 者の食い違いが、回答済みの承認が出戻る症状の温床になっていた
// （docs/local/plan_v0.7-scope_c2_approval-identity_c2_replay-generation.md）。
//
// 現在は candidateKey + sourceEpoch の 1 組だけで判定する。
//   - candidateKey: provider・承認種別・正規化した質問・選択肢番号と送信文字列から作る。
//     ラベルの空白・罫線・折返しは含めないので、TUI の再描画で揺れても変わらない。
//   - sourceEpoch : live prompt の世代。世代が進めば同じ質問文でも別の候補として扱うので、
//     意図的な再質問は表示される。
// どちらも Hub の保留中の記録（approval-store.ts）が持って届く。画面は組み立て直さない。
//
// このモジュールは DOM に触らない。state.ts から切り出してあるのは、承認の同一性を
// ブラウザ無しで試せるようにするため（approval-owner.ts と同じ方針）。
import { isBatchOptions } from './approval-parser.js';
import type { ApprovalOptionLike } from './state.js';

export interface ApprovalCandidateIdentity {
  candidateKey: string;
  sourceEpoch: number;
  shape: string;
}

export const answeredApprovalCandidates = new Map<number, Set<string>>(); // sessionId → `${epoch}\0${candidateKey}`
export const answeredApprovalShapeKeys = new Map<number, Map<string, string>>(); // sessionId → `${epoch}\0${shape}` -> candidateKey
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

// 選択肢の組の「中身」（provider・種別・質問・選択肢番号と送信文字列）。世代を含まない。
// 回答済みの中身をページ送りの描き直しで出さない判定（isStaleHistoryRepaint）と、
// 同一性を持たない組（順次質問の 1 問ずつの選択肢）の同一性に使う。
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

function answeredCandidateToken(identity: { candidateKey: string; sourceEpoch: number }): string {
  return `${identity.sourceEpoch}\0${identity.candidateKey}`;
}

function answeredApprovalShapeToken(sourceEpoch: number, shape: string): string {
  return `${sourceEpoch}\0${shape}`;
}

function rememberAnsweredApprovalIdentity(id: number, identity: { candidateKey: string; sourceEpoch: number; shape?: string }): void {
  let set = answeredApprovalCandidates.get(id);
  if (!set) { set = new Set<string>(); answeredApprovalCandidates.set(id, set); }
  set.add(answeredCandidateToken(identity));
  if (identity.shape) {
    let shapes = answeredApprovalShapeKeys.get(id);
    if (!shapes) { shapes = new Map<string, string>(); answeredApprovalShapeKeys.set(id, shapes); }
    shapes.set(answeredApprovalShapeToken(identity.sourceEpoch, identity.shape), identity.candidateKey);
  }
  while (set.size > ANSWERED_CANDIDATE_LIMIT) {
    const oldest = set.values().next().value;
    if (oldest === undefined) break;
    set.delete(oldest);
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

/**
 * Hub の記録（candidate_key + source_epoch）に回答済みの印を付ける。形（shape）も渡すと、
 * ページ送りの描き直しで同じ中身が届いたときに描かない判定（isStaleHistoryRepaint）に使える。
 */
export function recordAnsweredApprovalIdentity(id: number, candidateKey: string, sourceEpoch: number, shape = ''): void {
  if (!candidateKey || sourceEpoch <= 0) return;
  rememberAnsweredApprovalIdentity(id, { candidateKey, sourceEpoch, shape });
}

/**
 * Hub の記録の同一性（candidate_key + source_epoch）そのものに回答済みの印があるか。
 *
 * Hub の記録は同一性を持って届くので、選択肢から組み立て直さずにそのまま引く
 * （approval-store.ts）。回答した記録を描き直さないための判定で、印を付けるのは
 * recordAnsweredApprovalIdentity。
 */
export function isAnsweredApprovalIdentity(id: number, candidateKey: string, sourceEpoch: number): boolean {
  if (!candidateKey || !(sourceEpoch > 0)) return false;
  const set = answeredApprovalCandidates.get(id);
  return !!(set && set.has(answeredCandidateToken({ candidateKey, sourceEpoch })));
}

/**
 * この shape（provider・種別・質問・選択肢番号・送信文字列）の承認に、このセッションで
 * 一度でも回答したか。世代（sourceEpoch）は見ない。
 *
 * 通常の判定 isAnsweredApprovalIdentity は世代込みで見る。世代が進めば同じ質問でも
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
 * CLI がページ送りで過去の画面を描き直している最中に届いた、回答済みの中身か。
 *
 * 2026-08-23 に showOptions() へ入れた条件をここへ移した（bugfix_approval-bar-stale-
 * options-scroll-mismatch_2026-08-19.md）。判定式を 1 箇所に集約する。
 *
 * 遡り位置の取得だけは terminal.ts 側にあるので、呼び出し側から渡してもらう
 * （このモジュールは DOM にも xterm にも触らない方針のため）。
 */
export function isStaleHistoryRepaint(id: number, shape: string, showingHistory: boolean): boolean {
  return !!showingHistory && isAnsweredApprovalShapeAcrossEpochs(id, shape);
}

/** セッションを消した・Hub との接続をやり直したときに、そのセッションの回答済みの印を捨てる。 */
export function forgetAnsweredApprovals(id: number): void {
  answeredApprovalCandidates.delete(id);
  answeredApprovalShapeKeys.delete(id);
}

/**
 * 接続をやり直したとき（全セッションの履歴リセット）に、全セッションの回答済みの印を捨てる。
 * 半開きの接続へ送った回答は Hub に届いていないことがあり、Hub がまとめ（approval_snapshot）で
 * まだ保留中と言う記録は描き直すのが安全側。形（shape）の印は残す（遡り中の描き直し対策）。
 */
export function forgetAllAnsweredApprovals(): void {
  answeredApprovalCandidates.clear();
}

/** テスト専用。1 セッションぶんの承認同一性 state を初期状態へ戻す。 */
export function _resetApprovalAnsweredStateForTest(id: number): void {
  forgetAnsweredApprovals(id);
}
