// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { actionBarShownAt, activeSessionId, approvalCandidateShape, approvalSuppressedCache, batchActiveQ, batchFreeText, batchSelections, isStaleHistoryRepaint, lastActionBarRender, maybeAutoSwitchToNextApproval, multiSelectFocusIdx, multiSelectSelections, recordAnsweredApprovalIdentity, enqueueApprovalAutoSwitch, removeApprovalAutoSwitchTarget, sequentialChoiceCache, sequentialChoiceSig, set_actionBarFocusIdx, set_batchFocusIdx, set_multiSelectFocusIdx, terminals, _approvalCtxHash, type ApprovalCandidateIdentity } from './state.js';
import { APPROVAL_KIND_OPENCODE_SHORTCUT, APPROVAL_ORIGIN_NATIVE, approvalRecordFor, approvalRecordToken, approvalViewFor, foldApproval, isApprovalFolded, isApprovalRecordAnswered, subscribeApprovalStore, unfoldApproval } from './approval-store.js';
import { inputEl, sendSubmittedText, sendSubmittedBody } from '../app.js';
import { clearSuppressPtyResize, isTerminalAtBottom, refitAndStickTerminalToBottomSoon, scrollTerminalToBottomSoon, suppressPtyResizeForInputLayout, syncPtySizeToViewportAfterLayout } from './terminal.js';
import { ws } from './ws-client.js';
import { isBatchOptions, isMultiSelectOptions } from './approval-parser.js';
import { approvalUiAdapter, clearApprovalMarkerSuppressed, noteApprovalMarkerSuppressed, setMultiQuestionBannerVisible } from './approval-ui.js';
import { actionBarNeedsRepaint, actionBarOwnedByOther, releaseActionBarOwnership } from './approval-owner.js';
import { chatHistoryCommitOutput, chatPaneAtBottom, getChatTimelineEl, pushMessage, scrollChatPaneToBottom } from './chat-history.js';
import { apiFetch } from './util.js';
import { appConfirm, playNotificationSound, showDesktopApprovalNotification } from './settings.js';
import { isActionBarCollapsed, setActionBarCollapsed, STORAGE_HIGH_RISK_CONFIRMATION_MODE_KEY } from './user-prefs.js';
import { probe } from '../debug/probe.js';

const HIGH_RISK_HOLD_MS = 1200;
const highRiskConfirmationInFlight = new Set<number>();
const singleFreeTextSendInFlight = new Set<number>();
const NATIVE_APPROVAL_SEND_COOLDOWN_MS = 500;
const nativeApprovalSendCooldowns = new Map<number, ReturnType<typeof setTimeout>>();

function beginNativeApprovalSendCooldown(sessionId: number): boolean {
  if (nativeApprovalSendCooldowns.has(sessionId)) return false;
  const timer = setTimeout(() => {
    nativeApprovalSendCooldowns.delete(sessionId);
  }, NATIVE_APPROVAL_SEND_COOLDOWN_MS);
  nativeApprovalSendCooldowns.set(sessionId, timer);
  return true;
}

function cancelNativeApprovalSendCooldown(sessionId: number): void {
  const timer = nativeApprovalSendCooldowns.get(sessionId);
  if (!timer) return;
  clearTimeout(timer);
  nativeApprovalSendCooldowns.delete(sessionId);
}

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- 承認パネル（action-bar）----
// 承認は Hub が持つ保留中の記録を描くだけ（approval-store.ts）。画面は端末の文字から承認を
// 作らない。Hub のブロックを選択肢にする純粋な処理は approval-parser.js、DOM の後始末と
// 告知は approval-ui.js にある。

export function getSequentialChoiceState(id, prompts) {
  if (!prompts || prompts.length < 2) return null;
  const sig = sequentialChoiceSig(prompts);
  let state = sequentialChoiceCache.get(id);
  if (!state || state.sig !== sig) {
    state = { sig, prompts, answers: new Map(), index: 0 };
    sequentialChoiceCache.set(id, state);
  } else {
    state.prompts = prompts;
  }
  while (state.index < state.prompts.length && state.answers.has(state.prompts[state.index].key)) {
    state.index++;
  }
  return state.index < state.prompts.length ? state : null;
}

export function sequentialChoiceOptionsForState(state) {
  if (!state) return [];
  const prompt = state.prompts[state.index];
  const question = `${prompt.key}: ${prompt.question}`;
  return prompt.options.map((opt, idx) => ({
    ...opt,
    isCurrent: idx === 0,
    _sequentialChoice: true,
    _sequentialKey: prompt.key,
    _sequentialQuestion: question,
  }));
}

export function clearSequentialChoiceState(id) {
  sequentialChoiceCache.delete(id);
}

// ---- provider 分類 helper ----
// Go 側の isAIProvider と対応する。承認検出・chat history・done summary 等の
// AI 固有機能を適用するかどうかの判定に使う。Shell provider は対象外。
export function isAIProvider(provider: string): boolean {
  switch (provider) {
    case 'claude':
    case 'codex':
    case 'copilot':
    case 'cursor-agent':
    case 'opencode':
    case 'grok':
    case 'command-code':
      return true;
    default:
      return false;
  }
}

export function isShellProvider(provider: string): boolean {
  return provider === 'shell';
}

// custom_providers:（config.yaml の玄人設定）の id 集合。built-in 7種の固定リストに
// 頼れない判定（relay 開始ボタン等）はこちらも合わせて見る
// （plan_custom-provider-extension-triage.md C2）。/api/info の custom_providers を
// 起動時に1回 fetch してキャッシュするだけで、spawn-panel.ts の同種 fetch とは独立。
export const knownCustomProviderIds = new Set<string>();

void (async function loadKnownCustomProviderIds() {
  try {
    const res = await apiFetch('/api/info');
    if (!res.ok) return;
    const info = await res.json();
    const list = Array.isArray(info?.custom_providers) ? info.custom_providers : [];
    for (const entry of list) {
      const id = entry && typeof entry.id === 'string' ? entry.id : '';
      if (id) knownCustomProviderIds.add(id);
    }
  } catch (e) {
    console.warn('custom_providers load failed', e);
  }
})();

// Go 側の sessionApprovalDetectionEligible に対応する TS 版。isAIProvider（built-in
// 限定）か、custom_providers に定義された id かのどちらかを受理する。relay 開始ボタン
// のような「AI セッションか」判定は本来これを使うべきで、isAIProvider 単独は built-in
// 限定の判定が要る箇所（フック注入相当の判定等）専用として残す。
export function isAIOrCustomProvider(provider: string): boolean {
  return isAIProvider(provider) || knownCustomProviderIds.has(provider);
}

// \x15(Ctrl+U) を行クリアとして解釈しない provider。送ると逆に悪さをする:
// - shell: PowerShell 等で行クリアにならずリテラル ^U が混入してコマンドを壊す（2026-06-13 e168426 で確認）
// - codex: Rust TUI が \x15 を行クリアとして解釈せず、続く \r がコマンド実行に至らない
//   （2026-06-21 確認。素ターミナルで \x15 無しなら Enter 1 個で実行できる）
// かつては doSend / sendQuickCommand の全送信に \x15 を前置していたが、claude /login の
// コード入力欄でもリテラル混入（OAuth 400）を起こしたため前置は全廃し、residue-sweep.ts の
// 事後掃除へ移行した。現在この判定を使うのは inputClearBtn の単独 \x15 送信と
// residue-sweep の掃除スキップの 2 箇所。
export function shouldSkipClearPrefix(provider: string): boolean {
  return provider === 'shell' || provider === 'codex';
}

function localizeOpenCodeShortcutOptions(options) {
  const labels = new Map([
    ['allow once', t('approval_opencode_once')],
    ['allow always', t('approval_opencode_always')],
    ['reject', t('approval_opencode_reject')],
  ]);
  return options.map((opt) => {
    const nativeLabel = String(opt.label || '').trim();
    const label = labels.get(nativeLabel.toLowerCase());
    if (!label) return opt;
    return {
      ...opt,
      label,
      title: nativeLabel,
      _openCodeShortcut: true,
    };
  });
}

// OpenCode の承認ダイアログは数字を受け取らず、矢印＋Enter で選ぶ。
// action-bar 上だけは 1/2/3 を意味のあるショートカットとして提供し、
// sendChoice が保持している _sendText（Enter / Right+Enter）へ変換して送る。
//
// 帯に畳んでいる間も、この数字キーは効かせる。OpenCode の承認ダイアログは数字を受け付けない
// ので、数字を端末へ渡しても答えにならない（入力欄に入り、続く Enter で CLI が今の選択肢を
// 確定する）。ここで送信文字列へ変える方が、畳む理由の「端末に直接答えたい」に近い。
export function handleOpenCodeApprovalNumberKey(sessionId, num) {
  const options = pendingApprovalOptions(sessionId);
  if (!Array.isArray(options) || !options.some(opt => opt && opt._openCodeShortcut)) return false;
  if (!options.some(opt => opt && opt.num === num)) return false;
  sendChoice(sessionId, num);
  return true;
}

// ---- 保留中の記録から描く（approval-store.ts）----
//
// パネルと、承認を読む全部の場所（承認タブ・モバイル・自動移動・マルチペイン）は、Hub の記録の
// 写しであるストアから読む（docs/local/plan_approval-display-single-source_c3_web-switch.md の C2）。
// 端末の文字から承認を作らず、切り替え・再読み込み・再接続のたびにタイマーや台帳で取り直さない。
// 描くのは「アクティブなセッションの記録が変わったとき」と「セッションを切り替えたとき」だけ。

// 選択肢の組が native（CLI の承認画面）由来か。同一性と形（shape）の計算に渡す種類。
function approvalOriginKindOf(options) {
  const first = Array.isArray(options) ? options[0] : null;
  return first && first._approvalSource === 'go_vt' ? 'native' : 'marker';
}

// 選択肢の組が持つ同一性。ストアの選択肢には Hub の candidate_key / source_epoch が焼いてあるので
// それをそのまま使う。持たない組（順次質問の 1 問ずつの選択肢）は、問いの中身から作った key と
// 記録の世代で組む（問いごとに別の候補として描き直し、記録が変われば別の候補になる）。
export function displayedApprovalIdentity(id, options): ApprovalCandidateIdentity {
  const arr = options as any;
  const kind = approvalOriginKindOf(options);
  const shape = approvalCandidateShape(id, options, kind);
  const candidateKey = String(arr?._candidateKey || '');
  const sourceEpoch = Number(arr?._sourceEpoch || 0);
  const record = approvalRecordFor(id);
  if (candidateKey && sourceEpoch > 0) {
    const recordShape = record && record.candidate_key === candidateKey ? String(record.candidate_shape || '') : '';
    return { candidateKey, sourceEpoch, shape: recordShape || shape };
  }
  return { candidateKey: `local:${_approvalCtxHash(shape)}`, sourceEpoch: Number(record?.source_epoch) || 1, shape };
}

// 回答済みの印に残す形（shape）。Hub の記録が形を持つとき（ネイティブの承認画面と文章の質問）は
// それを使い、画面で別の定義を作らない。持たないとき（承認マーカー）だけ、描いた選択肢から作る。
// 同じ形の見分けは、遡り中の描き直しの判定（approval-answered.ts の isStaleHistoryRepaint）に使う。
function answeredShapeOf(sessionId, record, options, kind) {
  const recordShape = String(record?.candidate_shape || '');
  if (recordShape) return recordShape;
  return Array.isArray(options) && options.length > 0 ? approvalCandidateShape(sessionId, options, kind) : '';
}

// OpenCode のネイティブ承認は数字キーを受けないので、ラベルを訳してショートカットの印を付ける。
// 記録ごとに 1 度だけ作る（同じ記録の間は同じ配列を返す）。
const localizedApprovalOptions = new WeakMap<object, any[]>();

function localizeRecordOptions(record, options) {
  if (!record || record.kind !== APPROVAL_KIND_OPENCODE_SHORTCUT || !Array.isArray(options)) return options;
  const cached = localizedApprovalOptions.get(record);
  if (cached) return cached;
  const localized: any = localizeOpenCodeShortcutOptions(options);
  for (const key of ['_question', '_summary', '_candidateKey', '_sourceEpoch', '_preamble', '_freeInput']) {
    if ((options as any)[key] !== undefined) localized[key] = (options as any)[key];
  }
  localizedApprovalOptions.set(record, localized);
  return localized;
}

/**
 * そのセッションでいま描く選択肢。順次質問は今の問いの選択肢、OpenCode はラベルを訳した後の選択肢。
 * 記録が無い・回答を送った・告知だけ（AskUserQuestion）・画面のパーサが読めないときは null。
 */
export function pendingApprovalOptions(id) {
  if (id === null || id === undefined) return null;
  const view = approvalViewFor(id);
  if (view.kind === 'options' && Array.isArray(view.options) && view.options.length > 0) {
    return localizeRecordOptions(view.record, view.options);
  }
  if (view.kind === 'sequential') {
    // 順次質問は 1 問ずつ描く。問いごとにパネルを描き直すため、記録の同一性は焼かない
    // （焼くと showOptions が「同じ候補を表示中」と見て次の問いを描かない）。
    // 回答済みの印は記録の同一性で付ける（noteApprovalAnswerSent）。
    const state = getSequentialChoiceState(id, view.prompts);
    return state ? sequentialChoiceOptionsForState(state) : null;
  }
  return null;
}

/** AskUserQuestion の告知（端末で答える承認）を待っているか。 */
export function isAskUserQuestionPending(id) {
  if (id === null || id === undefined) return false;
  return approvalViewFor(id).kind === 'notice';
}

function notifyApprovalViewsChanged() {
  try {
    window.dispatchEvent(new CustomEvent('approval-queue-updated'));
  } catch (_) {}
}

/**
 * アクティブなセッションの承認を、ストアの記録から描く（タイマーを待たない）。
 * 描くものが無ければ、そのセッションのパネルを片付ける。別のセッションのパネルは
 * 呼び出し側が releaseActionBarIfOwnedByOther で先に捨てる。
 * 畳んだ記録（approval-store.ts の「畳む」）はパネルを描かず、1 行の帯（#approval-fold-band）を出す。
 */
export function renderApprovalFromStore(id) {
  if (id === null || id === undefined || id !== activeSessionId) return;
  const view = approvalViewFor(id, { folded: true });
  const folded = view.kind === 'folded';
  setMultiQuestionBannerVisible(view.kind === 'notice');
  // Hub の記録のブロックを画面のパーサが弾いた。黙って何も出さないと承認待ちのまま原因が
  // 分からないので、抑止の告知を出す（Hub が弾いたときの approval_marker_suppressed と同じ告知）。
  // 告知は記録ごとに 1 回だけ出す（描き直すたびに出すと、✕ で閉じた告知が切り替えのたびに戻る）。
  // 記録が読める記録へ変わった・閉じたら、画面のパーサが出した告知（client_corrupt）は取り下げる。
  // Hub が出した告知（記録を開かずに捨てたブロック）は記録と結び付かないので、ここでは触らない。
  if (view.kind === 'unreadable') {
    const token = approvalRecordToken(view.record);
    if (unreadableNoticeFor.get(id) !== token) {
      unreadableNoticeFor.set(id, token);
      noteApprovalMarkerSuppressed(id, 'client_corrupt');
    }
  } else if (!folded) {
    unreadableNoticeFor.delete(id);
    if (approvalSuppressedCache.get(id) === 'client_corrupt') clearApprovalMarkerSuppressed(id);
  }
  const bar = document.getElementById('action-bar');
  const options = folded ? null : pendingApprovalOptions(id);
  if (bar && (!options || options.length === 0)) {
    if (bar.classList.contains('visible') && !actionBarOwnedByOther(bar, id)) {
      // 畳んだときは DOM だけを捨てる（一括回答の選択・自由入力は、開いたときに戻す）。
      if (folded) releasePanelForFold(bar, id);
      else hideActionBar(id);
    }
  }
  syncFoldedBand(id, folded);
  if (!bar || !options || options.length === 0) return;
  // 描き直しが要るか（出ていない・空・別のセッションのもの）の式は approval-owner.ts の 1 本。
  const newlyShown = actionBarNeedsRepaint(bar, id);
  approvalUiAdapter.showOptions(bar, id, options, newlyShown);
}

// 読めない記録の告知を出した記録（sessionId → 記録の同一性）。
const unreadableNoticeFor = new Map<number, string>();

// 畳んだときにパネルの DOM を捨てる。hideActionBar と違い、操作途中の状態（一括回答の選択・
// 自由入力・順次質問の進み具合）は残す。帯を押して開いたときに、そのまま続けられるように。
function releasePanelForFold(bar, id) {
  const identity = actionBarDisplayedIdentity(bar);
  bar.classList.remove('visible', 'batch', 'multi-select', 'single-tabs');
  bar.innerHTML = '';
  delete bar.dataset.approvalSessionId;
  delete bar.dataset.approvalCandidateKey;
  delete bar.dataset.approvalSourceEpoch;
  lastActionBarRender.sessionId = null;
  lastActionBarRender.sig = null;
  set_actionBarFocusIdx(-1);
  set_batchFocusIdx(-1);
  set_multiSelectFocusIdx(-1);
  removeBatchConfirmModal();
  if (id === activeSessionId) {
    suppressPtyResizeForInputLayout(350);
    const term = terminals.get(id);
    const shouldStickToBottom = !!(term && (term.autoScroll || isTerminalAtBottom(term)));
    syncPtySizeToViewportAfterLayout(id, shouldStickToBottom, 400, 'settle-after-fold', identity);
    if (shouldStickToBottom) scrollTerminalToBottomSoon(id);
  } else {
    clearSuppressPtyResize();
  }
}

// 帯に出す 1 行（承認待ちであることは帯のラベルが出すので、ここは質問の頭だけ）。
function approvalBandSummary(id) {
  if (approvalViewFor(id).kind === 'notice') return t('multi_question_banner');
  const options: any = pendingApprovalOptions(id);
  if (!Array.isArray(options) || options.length === 0) return '';
  if (isBatchOptions(options)) {
    const first = String((options[0] as any)?.title || '').trim();
    return first ? `${t('approval_batch_label', { n: options.length })} ${first}` : t('approval_batch_label', { n: options.length });
  }
  const props = options as any;
  const sequential = options.find((opt) => opt && opt._sequentialQuestion)?._sequentialQuestion;
  const text = sequential || props._question || options[0]?._question || props._summary?.command || props._preamble || '';
  return String(text).trim().split(/\r\n|\r|\n/).find((line) => line.trim() !== '') || '';
}

// 帯（#approval-fold-band）は告知バナーと同じく、端末と入力欄の間にフローで置く。
// #action-bar の中に置くと、デスクトップでは入力欄の上へ重なり（approval-dock.ts）、
// 下ドックの位置を戻しても端末の最終行を隠す。どちらも、畳む理由（裏を読みたい・
// 端末に直接答えたい）を妨げる。帯の出し入れで端末の高さが 1 回変わるので、バナーと同じく
// レイアウトが決まってから実寸を 1 回だけ送る。
// パネルのキー操作（app.ts の ←→ / Enter、一括・複数選択の専用キー）はどれも #action-bar の
// 'visible' を条件にしているので、畳んでいる間はキー入力がそのまま端末へ渡る。
function settleTerminalAfterFoldBand() {
  if (activeSessionId === null) return;
  suppressPtyResizeForInputLayout(350);
  const term = terminals.get(activeSessionId);
  const shouldStickToBottom = !!(term && (term.autoScroll || isTerminalAtBottom(term)));
  syncPtySizeToViewportAfterLayout(activeSessionId, shouldStickToBottom, 400, 'settle-after-fold-band');
}

function syncFoldedBand(id, folded) {
  const band = document.getElementById('approval-fold-band');
  if (!band) return;
  if (!folded) {
    if (!band.hidden && band.dataset.approvalSessionId === String(id)) hideFoldedBand();
    return;
  }
  const summary = approvalBandSummary(id);
  const sig = JSON.stringify({ s: id, k: approvalRecordToken(approvalRecordFor(id)), t: summary });
  if (!band.hidden && band.dataset.approvalSessionId === String(id) && band.dataset.bandSig === sig) return;
  const wasHidden = band.hidden;
  band.innerHTML = '';
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'approval-fold-band-btn';
  btn.title = t('approval_band_open_tooltip');
  btn.setAttribute('aria-expanded', 'false');
  btn.setAttribute('aria-controls', 'action-bar');
  const label = document.createElement('span');
  label.className = 'approval-fold-band-label';
  label.textContent = t('approval_band_label');
  btn.appendChild(label);
  if (summary) {
    const text = document.createElement('span');
    text.className = 'approval-fold-band-text';
    text.textContent = summary;
    btn.appendChild(text);
  }
  const hint = document.createElement('span');
  hint.className = 'approval-fold-band-hint';
  hint.textContent = t('approval_band_open');
  btn.appendChild(hint);
  btn.onclick = (e) => {
    e.preventDefault();
    e.stopPropagation();
    // モバイル幅では開くと承認シートも開く。入力欄へフォーカスするとソフトキーボードが
    // シートを押し上げるので、フォーカスしない（mobile-approval-sheet.ts と同じ幅の判定）。
    const mobile = typeof window.matchMedia === 'function' && window.matchMedia('(max-width: 720px)').matches;
    unfoldApprovalPanel(id, { focusInput: !mobile });
  };
  band.appendChild(btn);
  band.dataset.approvalSessionId = String(id);
  band.dataset.bandSig = sig;
  band.hidden = false;
  if (wasHidden) settleTerminalAfterFoldBand();
}

function hideFoldedBand() {
  const band = document.getElementById('approval-fold-band');
  if (!band || band.hidden) return;
  band.hidden = true;
  band.innerHTML = '';
  delete band.dataset.approvalSessionId;
  delete band.dataset.bandSig;
  settleTerminalAfterFoldBand();
}

/** まとめ（approval_snapshot）を適用した後に、アクティブなセッションと承認タブを描き直す。 */
export function renderApprovalAfterSnapshot() {
  renderApprovalFromStore(activeSessionId);
  notifyApprovalViewsChanged();
}

/**
 * 画面から記録へ回答を送った後に呼ぶ。本文の送信が成功した後にだけ呼ぶこと
 * （失敗したら記録を描き続け、同じ操作をやり直せるようにする）。
 *
 * 回答済みの印（approval-answered.ts）を記録の同一性で付けるので、Hub の閉じるが届くまでの
 * 間もこの記録は描かない。Hub へは approval_consumed を送り、Hub は記録を閉じる
 * （マーカーの記録は、先に届く確定したユーザーターンで閉じていることが多い）。
 * 形（shape）も残すのは、ページ送りで CLI が過去の画面を描き直したときに、回答済みの中身を
 * 描かないため（approval-answered.ts の isStaleHistoryRepaint）。
 */
export function noteApprovalAnswerSent(sessionId, sentText, shownOptions: any = undefined): boolean {
  const record = approvalRecordFor(sessionId);
  if (!record || isApprovalRecordAnswered(sessionId, record)) return true;
  const options = shownOptions !== undefined ? shownOptions : pendingApprovalOptions(sessionId);
  const kind = record.origin === APPROVAL_ORIGIN_NATIVE ? 'native' : 'marker';
  const shape = answeredShapeOf(sessionId, record, options, kind);
  recordAnsweredApprovalIdentity(sessionId, record.candidate_key, Number(record.source_epoch) || 0, shape);
  notifyApprovalViewsChanged();
  maybeAutoSwitchToNextApproval();
  if (!ws || ws.readyState !== WebSocket.OPEN) return false;
  try {
    ws.send(JSON.stringify({
      type: 'approval_consumed',
      session_id: sessionId,
      approval_sig: record.sig || '',
      approval_candidate_key: record.candidate_key,
      approval_source_epoch: Number(record.source_epoch) || 0,
      approval_source: kind === 'native' ? 'go_vt' : 'hub_marker',
      sent_text: sentText || '',
    }));
  } catch (_) {
    return false;
  }
  return true;
}

/**
 * 承認パネルを使わずに入力欄（またはクイックコマンド）から送ったときに呼ぶ。
 *
 * 回答済みの印を付けてパネルを片付けるのは、Hub がその記録を閉じると分かっているときだけ。
 *   - 送った文字が選択肢の番号か送信文字列と一致した: Hub へ approval_consumed を送る（Hub が閉じる）
 *   - マーカーと文章の質問（origin が marker）へ、空でない文章を送った: Hub は確定したユーザーターンで
 *     閉じる（internal/hub/approval_record.go の closeMarkerRecordOnSubmittedTurn）
 * それ以外（ネイティブの承認画面へ一致しない文字を送った・空の Enter）は、印を付けずパネルも残す。
 * ネイティブの記録は CLI の画面から消えたときに Hub が閉じる。印を付けて片付けると、Hub が閉じない
 * 限り「保留中」のまま画面からだけ消える（空の Enter は Hub では確定ターンにならない）。
 */
export function noteApprovalAnsweredByTyping(sessionId, rawText, sentText) {
  const options = pendingApprovalOptions(sessionId);
  if (!Array.isArray(options) || options.length === 0) return;
  const trimmed = String(rawText || '').trim();
  const matched = !isBatchOptions(options) && options.some((opt) => {
    if (!opt) return false;
    const optSend = opt._sendText || `${opt.num}`;
    return trimmed === String(opt.num) || trimmed === String(optSend).trim();
  });
  if (matched) {
    noteApprovalAnswerSent(sessionId, sentText, options);
    hideActionBar(sessionId);
    return;
  }
  const record = approvalRecordFor(sessionId);
  if (!record || record.origin === APPROVAL_ORIGIN_NATIVE || trimmed === '') return;
  recordAnsweredApprovalIdentity(sessionId, record.candidate_key, Number(record.source_epoch) || 0, answeredShapeOf(sessionId, record, options, 'marker'));
  notifyApprovalViewsChanged();
  hideActionBar(sessionId);
}

// ストアの記録が変わったときの後始末と、利用者への知らせ。
function onApprovalStoreChanged(changes) {
  let activeTouched = false;
  for (const change of changes) {
    const id = change.sessionId;
    if (approvalRecordToken(change.previous) !== approvalRecordToken(change.record)) {
      // 別の記録へ変わった（閉じた・上書きされた）。前の記録のために画面が持っていた
      // 操作途中の状態（順次質問の進み具合・一括回答の選択・自由入力）を捨てる。
      // 畳み状態は記録の同一性ごとに持つので、ここでは触らない（approval-store.ts の「畳む」）。
      clearSequentialChoiceState(id);
      batchSelections.delete(id);
      batchFreeText.delete(id);
      batchActiveQ.delete(id);
      multiSelectSelections.delete(id);
      clearSingleTabState(id);
      // 一括回答の確認モーダルは前の記録の中身を出している。残すと「送信」が何も送らない。
      if (id === activeSessionId) removeBatchConfirmModal();
    }
    if (!change.record) removeApprovalAutoSwitchTarget(id);
    // 新しい承認だけを知らせる。接続・再接続のまとめで届いた記録と、同じ世代の開き直しでは
    // 鳴らさない（approval-store.ts の announce）。
    if (change.record && change.announce && !isApprovalRecordAnswered(id, change.record)) {
      playNotificationSound();
      showDesktopApprovalNotification(id);
      enqueueApprovalAutoSwitch(id);
    }
    if (id === activeSessionId) activeTouched = true;
  }
  if (activeTouched) renderApprovalFromStore(activeSessionId);
  notifyApprovalViewsChanged();
  maybeAutoSwitchToNextApproval();
}

subscribeApprovalStore(onApprovalStoreChanged);

export function getActionBarButtons() {
  const bar = document.getElementById('action-bar');
  if (!bar) return [];
  return Array.from(bar.querySelectorAll('.action-btn'));
}

export function setActionBarFocus(idx) {
  set_actionBarFocusIdx(idx);
  getActionBarButtons().forEach((btn, i) => btn.classList.toggle('kbd-focus', i === idx));
}

// セッション切替時に、出ているパネルが切替先のものでなければ DOM だけ捨てる。
// 捨てたら true を返す。判定と DOM 操作の本体は approval-owner.ts にある。
//
// 切替先に描くものが無い・帯に畳んでいるなどで描き直さない経路があると、切替元の
// パネルが画面に残る。表示中セッションの質問だと思って回答すると、実際の送信先は
// 別セッションになる（bugfix_approval-panel-shows-other-session_2026-08-14.md）。
// 描かない理由を 1 つずつ塞ぐ形にすると理由が増えるたびに同じ穴が開くので、切替側で掃除する。
//
// hideActionBar は使えない。あれは対象セッションの操作途中の状態（一括回答の選択・自由入力）まで
// 捨てるが、切替元の承認はまだ未回答で、戻ったときにそのまま描き直す必要がある。
// ここでは DOM と描画差分キャッシュだけを捨て、承認状態には一切触らない。
export function releaseActionBarIfOwnedByOther(id) {
  // 帯も同じく、切替先のものでなければ捨てる（畳み状態には触らない）。
  const band = document.getElementById('approval-fold-band');
  if (band && !band.hidden && band.dataset.approvalSessionId !== String(id)) hideFoldedBand();
  const bar = document.getElementById('action-bar');
  if (!bar || !actionBarOwnedByOther(bar, id)) return false;
  releaseActionBarOwnership(bar);
  // 差分スキップ用キャッシュを無効化しないと、次の showActionBar が
  // 「同じものを描画済み」と判定して描き直さないことがある。
  lastActionBarRender.sessionId = null;
  lastActionBarRender.sig = null;
  set_actionBarFocusIdx(-1);
  set_batchFocusIdx(-1);
  set_multiSelectFocusIdx(-1);
  // 一括承認の確認モーダルは bar に紐づく。bar を捨てたら宙に浮くので一緒に閉じる。
  removeBatchConfirmModal();
  return true;
}

// 出ているパネルの同一性（showActionBar が dataset に焼いたもの）。PTY サイズ同期の観測に渡す。
function actionBarDisplayedIdentity(bar) {
  const candidateKey = bar?.dataset?.approvalCandidateKey || '';
  if (!candidateKey) return null;
  return { candidateKey, sourceEpoch: Number(bar.dataset.approvalSourceEpoch || 0) };
}

// パネル（と帯）を片付ける。承認があるかどうかは Hub の記録が決めるので、ここは画面の
// 後始末だけを行う（記録を閉じたり回答済みにしたりしない）。セッションの操作途中の状態（一括回答の
// 選択・自由入力）も捨てる。順次質問の進み具合と畳み状態は記録が変わるまで残す（途中の問いを
// 描き直すため・畳んだ記録を開き直さないため）。
//
// id を渡したときは、id のセッションが描いたパネルだけを捨てる。承認タブやモバイルの一覧から
// 別のセッションへ回答すると、ここへ別のセッションの id が来る。そのとき見ているセッションの
// パネルを捨てると、記録は変わらないので描き直す契機が無く、保留中のまま画面から消える
// （以前は端末の文字の読み直しと ↻ 承認が描き直していたが、どちらも撤去した）。
export function hideActionBar(id) {
  try { console.log('[approval-route] hideActionBar', { id, activeSessionId, stack: new Error().stack?.split('\n').slice(1, 5).join(' | ') }); } catch (_) {}
  const bar = document.getElementById('action-bar');
  const barOfOther = id !== undefined && !!bar && actionBarOwnedByOther(bar, id);
  const wasVisible = !barOfOther && !!(bar && bar.classList.contains('visible'));
  const hideIdentity = actionBarDisplayedIdentity(bar);
  // 回答した・記録が無くなったなど、パネルを片付けるときは帯も片付ける（畳んだまま入力欄から
  // 答えた場合も、この経路で帯が消える）。
  const band = document.getElementById('approval-fold-band');
  if (band && !band.hidden && (id === undefined || band.dataset.approvalSessionId === String(id))) hideFoldedBand();
  if (bar && !barOfOther) {
    bar.classList.remove('visible', 'batch', 'multi-select', 'single-tabs');
    bar.innerHTML = '';
    delete bar.dataset.approvalSessionId;
    delete bar.dataset.approvalCandidateKey;
    delete bar.dataset.approvalSourceEpoch;
  }
  // Hide is a layout transition too. Keep intermediate ResizeObserver frames
  // suppressed and replace any pending show settle with one final hide settle.
  if (wasVisible && id !== undefined && id === activeSessionId) {
    suppressPtyResizeForInputLayout(350);
    const term = terminals.get(id);
    const shouldStickToBottom = !!(term && (term.autoScroll || isTerminalAtBottom(term)));
    syncPtySizeToViewportAfterLayout(id, shouldStickToBottom, 400, 'settle-after-hide', hideIdentity);
  } else if (wasVisible) {
    clearSuppressPtyResize();
  }
  // 差分スキップ用キャッシュをリセット（次回 showActionBar が同一シグネチャでも再描画されるように）。
  // 別のセッションのパネルが出ているときは、そのパネルの描画状態と確認モーダルに触らない。
  if (!barOfOther) {
    lastActionBarRender.sessionId = null;
    lastActionBarRender.sig = null;
    set_actionBarFocusIdx(-1);
    set_batchFocusIdx(-1);
    set_multiSelectFocusIdx(-1);
    removeBatchConfirmModal();
  }
  if (id !== undefined) actionBarShownAt.delete(id);
  if (id !== undefined) batchSelections.delete(id);
  if (id !== undefined) batchFreeText.delete(id);
  if (id !== undefined) batchActiveQ.delete(id);
  if (id !== undefined) multiSelectSelections.delete(id);
  if (id !== undefined) {
    clearSingleTabState(id);
    // action-bar 消失でターミナル領域の高さが拡張されるため、追従中なら最下部へ再スナップする。
    // showActionBar が plan_approval-bar-scroll-resnap.md で同等の処理を持つので、その対称ケース。
    if (wasVisible && id === activeSessionId) {
      const term = terminals.get(id);
      const shouldStickToBottom = !!(term && (term.autoScroll || isTerminalAtBottom(term)));
      if (shouldStickToBottom) scrollTerminalToBottomSoon(id);
    }
    maybeAutoSwitchToNextApproval();
  }
}

// ✕（パネル右上・端末右下の「✕ 承認」・告知バナー・モバイルのシート）: その承認を 1 行の帯に
// 畳む。Hub の記録には触らないので、承認は保留のまま（「保留中」も通知も残る）。帯を押すと開く。
// 畳んだことは記録ごとに覚え、再読み込みしても同じ承認なら畳んだまま（approval-store.ts）。
export function foldApprovalPanel(id) {
  if (id === undefined || id === null) return;
  if (!foldApproval(id)) return;
  renderApprovalFromStore(id);
  notifyApprovalViewsChanged();
}

// 帯を押したとき（モバイルはシートを開いたとき）: 畳み状態を解いて、ストアの記録から描き直す。
// 記録は Hub が持っているので、本当に保留中の承認だけが開き、閉じていれば何も出ない。
export function unfoldApprovalPanel(id, opts: { focusInput?: boolean } = {}) {
  if (id === undefined || id === null) return;
  unfoldApproval(id);
  renderApprovalFromStore(id);
  notifyApprovalViewsChanged();
  if (opts.focusInput !== false) setTimeout(() => inputEl.focus(), 0);
}

// 抑止告知の「再検出」: Hub へ問い直す（approval_resync）。Hub は端末ミラーから候補を評価し直し、
// そのセッションの今の記録をこの画面にだけ送り直す（internal/hub/approval_record.go の
// handleApprovalResync）。画面の写しは Hub の記録からしか作らないので、取り直す先は Hub。
export function requestApprovalResync(id) {
  if (id === undefined || id === null) return false;
  if (!ws || ws.readyState !== WebSocket.OPEN) return false;
  try {
    ws.send(JSON.stringify({ type: 'approval_resync', session_id: id }));
  } catch (_) {
    return false;
  }
  return true;
}

// 端末右下の「✕ 承認」は、アクティブなセッションのパネルが開いている間だけ出す
// （承認が無い間・帯に畳んでいる間は出さない。畳んだ後は帯が同じ役目を持つ）。
// パネルの出し入れは経路が多いので、#action-bar の変化を見て合わせる。
function syncApprovalRecallBar() {
  const recall = document.getElementById('approval-recall-bar');
  if (!recall) return;
  const bar = document.getElementById('action-bar');
  const open = !!(bar && activeSessionId !== null && bar.classList.contains('visible') &&
    bar.children.length > 0 && bar.dataset.approvalSessionId === String(activeSessionId));
  if (recall.hidden === !open) return;
  recall.hidden = !open;
}

export function normalizeActionOptions(options) {
  const byNum = new Map();
  for (const opt of options || []) {
    if (!opt || typeof opt.num !== 'number') continue;
    const prev = byNum.get(opt.num);
    if (!prev) {
      byNum.set(opt.num, { ...opt });
      continue;
    }
    // 同一番号が再描画ノイズで重複した場合は、現在選択中フラグと長いラベルを優先する。
    byNum.set(opt.num, {
      ...prev,
      isCurrent: !!(prev.isCurrent || opt.isCurrent),
      preserveOrder: !!(prev.preserveOrder || opt.preserveOrder),
      label: (opt.label && opt.label.length > (prev.label || '').length) ? opt.label : prev.label,
    });
  }
  const normalized = Array.from(byNum.values());
  if (normalized.some(opt => opt.preserveOrder)) return normalized;
  return normalized.sort((a, b) => a.num - b.num);
}

// 折りたたみトグル（全文⇄コンパクト切替）。3 経路（単問/一括/複数選択）共通で使う。
// position:absolute なので bar 直下に append すれば footer の有無に関わらず同じ位置に出る。
function syncCollapseToggleButton(btn, collapsed = isActionBarCollapsed()) {
  btn.textContent = collapsed ? '⊞' : '⊟';
  const label = collapsed ? t('action_bar_expand') : t('action_bar_collapse');
  btn.title = label;
  btn.setAttribute('aria-label', label);
  // aria-expanded は「押したあとに見える全文表示」の状態を表す。
  btn.setAttribute('aria-expanded', collapsed ? 'false' : 'true');
  btn.setAttribute('aria-controls', 'action-bar');
}

function syncActionBarCollapseToggle(bar) {
  const btn = bar?.querySelector('.action-collapse-btn');
  if (btn instanceof HTMLButtonElement) syncCollapseToggleButton(btn);
}

function appendCollapseToggle(bar, sessionId) {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'action-collapse-btn';
  syncCollapseToggleButton(btn);
  btn.onclick = (e) => {
    e.preventDefault();
    e.stopPropagation();
    toggleActionBarCollapsed(sessionId);
  };
  bar.appendChild(btn);
}

export function toggleActionBarCollapsed(sessionId) {
  setActionBarCollapsed(!isActionBarCollapsed());
  const bar = document.getElementById('action-bar');
  if (!bar) return;
  // collapsed を各 show* の sig に含めているため、再描画させるには sig を無効化する。
  lastActionBarRender.sessionId = null;
  lastActionBarRender.sig = null;
  const cached = pendingApprovalOptions(sessionId);
  if (cached && cached.length > 0) {
    showActionBar(bar, sessionId, cached, false);
  } else {
    bar.classList.toggle('collapsed', isActionBarCollapsed());
    // options cache が無い経路でも、見た目と状態属性を同時に更新する。
  }
  syncActionBarCollapseToggle(bar);
  setTimeout(() => inputEl.focus(), 0);
}

export function showActionBar(bar, sessionId, options, forceStickToBottom = false) {
  try {
    const firstLabel = Array.isArray(options) && options[0] ? String((options[0] as any).label || (options[0] as any).title || '').slice(0, 80) : '';
    const question = options && (options as any)._question ? String((options as any)._question).slice(0, 80) : '';
    console.log('[approval-route] showActionBar', { sessionId, activeSessionId, len: Array.isArray(options) ? options.length : 0, firstLabel, question, stack: new Error().stack?.split('\n').slice(1, 6).join(' | ') });
  } catch (_) {}
  // 帯に畳んだ記録はパネルを描かない（帯は renderApprovalFromStore が描く）。
  // 記録が変われば畳み状態はストアが捨てるので、新しい承認は開いて出る。
  if (isApprovalFolded(sessionId)) return;
  const displayIdentity = displayedApprovalIdentity(sessionId, options);
  if (bar) {
    bar.dataset.approvalSessionId = String(sessionId);
    bar.dataset.approvalCandidateKey = displayIdentity.candidateKey;
    bar.dataset.approvalSourceEpoch = String(displayIdentity.sourceEpoch);
  }
  // ステール DOM 防御: lastActionBarRender が「描画済み」でも bar が空/非表示なら差分スキップを無効化。
  // （visible=true なのに children=0 の action-bar-invisible パターンへの対策）
  if (bar && lastActionBarRender.sessionId === sessionId &&
      (!bar.classList.contains('visible') || bar.children.length === 0)) {
    lastActionBarRender.sessionId = null;
    lastActionBarRender.sig = null;
  }
  if (isBatchOptions(options)) {
    showBatchActionBar(bar, sessionId, options, forceStickToBottom);
    return;
  }
  if (isMultiSelectOptions(options)) {
    showMultiSelectActionBar(bar, sessionId, options, forceStickToBottom);
    return;
  }
  // 単一質問（YES/NO・単一選択・順次質問）も質問タブUI（1タブ）へ統合する。
  // plan_choice-tab-ui.md C5。flat options を 1 セクション（_single）へ変換し、
  // 既存の showBatchActionBar に同じ params 形で渡す（描画は同関数の単一モード分岐で行う）。
  // 送る選択肢（pendingApprovalOptions）は flat のまま（sendChoice の _sendText 等が機能するため）。
  const opts = normalizeActionOptions(options);
  const sequentialQuestion = opts.find(o => o && o._sequentialQuestion)?._sequentialQuestion;
  // [MANY-AI-CLI] 単一ブロックの先頭にあった質問文（parseHubBlock が配列プロパティ _question に格納）。
  // これを承認ポップアップ内に表示し、AI が何を聞いているのかを CLI 画面を見ずに把握できるようにする。
  const hubQuestion = (options && (options as any)._question) ? String((options as any)._question).trim() : '';
  const title = sequentialQuestion || hubQuestion || 'Approval needed';
  const section = {
    num: 1,
    title,
    options: opts,
    _single: true,
    _freeInput: !!(options && (options as any)._freeInput),
    _labelKind: sequentialQuestion ? 'sequential' : 'approval',
    _labelTitle: sequentialQuestion || '',
    // 質問本文（承認系のみ；sequential は title 側で表示済み）。
	_question: hubQuestion,
	_summary: (options as any)._summary || null,
    // 配列プロパティの _preamble は [section] 変換で失われるためセクションへも引き継ぐ。
    _preamble: (options && (options as any)._preamble) ? (options as any)._preamble : '',
  };
  // 単一質問は [section] の 1 要素配列へ変換するため、配列プロパティの _preamble が
  // 失われる。明示的に引き継いでポップアップ先頭の前置き表示を効かせる。
  const sectionArr: any[] = [section];
  if (options && (options as any)._preamble) (sectionArr as any)._preamble = (options as any)._preamble;
  showBatchActionBar(bar, sessionId, sectionArr, forceStickToBottom);
}

// 単一質問タブUI（plan_choice-tab-ui.md C5）の自由入力状態。バッチの batchSelections/
// batchFreeText とは別管理（単一は selections 機構を使わず即送信のため）。
const singleFreeText = new Map<number, string>();   // sessionId → 自由入力テキスト（再描画跨ぎで保持）
const singleFreeActive = new Map<number, boolean>(); // sessionId → 自由入力欄を開いているか

function clearSingleTabState(id) {
  singleFreeText.delete(id);
  singleFreeActive.delete(id);
}

// 自由入力欄を開く（再描画して入力欄を出しフォーカスする）。
export function activateSingleFree(sessionId) {
  singleFreeActive.set(sessionId, true);
  const cached = pendingApprovalOptions(sessionId);
  if (!Array.isArray(cached) || isBatchOptions(cached) || isMultiSelectOptions(cached)) return;
  const bar = document.getElementById('action-bar');
  if (bar) showActionBar(bar, sessionId, cached); // router 経由で 1 セクションへ再変換
}

// 単一質問の自由入力テキストを外部から取得・保存するためのアクセサ。
// mobile-home.ts のインライン自由入力 UI が再描画を跨いで値を維持するために使う
// （PC 版 action-bar と同じ singleFreeText Map を共有する）。
export function getSingleFreeText(sessionId: number): string {
  return singleFreeText.get(sessionId) || '';
}
export function setSingleFreeText(sessionId: number, text: string): void {
  singleFreeText.set(sessionId, text);
}

// 自由入力テキストをそのまま送信する（確認モーダルなし）。送信後は UI を消して会話へ戻る。
export function sendSingleFreeText(sessionId) {
  if (singleFreeTextSendInFlight.has(sessionId)) return false;
  const text = (singleFreeText.get(sessionId) || '').trim();
  if (!text) return false;
  singleFreeTextSendInFlight.add(sessionId);
  const cachedOpts = pendingApprovalOptions(sessionId);
  try {
    // 本文が送れた後にだけ回答済みの印・チャット履歴・action-bar を更新する。
    // 接続断時は sendSubmittedBody が false を返し、入力 state を保持したまま戻る。
    // 自由回答は文章なのでチャット本文と同じ共通経路（ペースト包み＋確定 \r 別送）で送る。
    // 「本文+\r」同梱は内側 CLI に \r を吸収され送信不発になる（Grok 実測 2026-07-11）。
    if (!sendSubmittedBody(sessionId, text)) return false;
    chatHistoryCommitOutput(sessionId);
    pushMessage(sessionId, {
      role: 'system',
      kind: 'approval',
      rawText: text,
      meta: { kind: 'single', answer: null, label: text },
    });
    // Hub への知らせ（approval_consumed）の失敗は本文送信の成功と混同しない。
    noteApprovalAnswerSent(sessionId, `${text}\r`, cachedOpts);
  } finally {
    singleFreeTextSendInFlight.delete(sessionId);
  }
  hideActionBar(sessionId);
  setTimeout(() => inputEl.focus(), 0);
  return true;
}

// ---- 一括承認: 質問タブUI（plan_choice-tab-ui.md）----
// 質問そのものを横タブにし、選択中の 1 問分の選択肢だけを下のパネルに出す。
// 質問数が増えても縦の高さが一定で、上のターミナル（文脈）が隠れない。
// 全問回答後に「送信確認」→ モーダルで内容＋実送信文字列を確認 →「送信」で確定する。

export const BATCH_FREE = -1; // 自由入力肢を選択中であることを示す selections センチネル

// アクティブな質問タブ index を範囲内に正規化して返す（未設定/範囲外は 0）。
function getBatchActiveQ(sessionId, n) {
  let idx = batchActiveQ.get(sessionId);
  if (idx == null || idx < 0 || idx >= n) { idx = 0; batchActiveQ.set(sessionId, idx); }
  return idx;
}

// 承認ブロック直前の前置き説明（_preamble）をポップアップ先頭へ表示する。
// 既定はヘッダ＋高さ上限つきスクロール枠で文脈を即見せ（CLI をスクロールしなくて済む）、
// ヘッダクリックで枠を広げて全文を読めるようにする（'expanded' クラスのトグル）。
function appendApprovalPreamble(bar, preamble) {
  const text = String(preamble || '').trim();
  if (!text) return;
  const wrap = document.createElement('div');
  wrap.className = 'action-preamble';
  const head = document.createElement('button');
  head.type = 'button';
  head.className = 'action-preamble-head';
  head.textContent = t('approval_preamble_label');
  const body = document.createElement('div');
  body.className = 'action-preamble-body';
  head.onclick = (e) => {
    e.stopPropagation();
    wrap.classList.toggle('expanded');
    // バー上端のドラッグリサイズ（action-bar-resize.ts）が付けたインライン max-height が
    // 残っていると、CSS クラス側の max-height（5.5em/22em）より優先されて展開が効かない。
    body.style.maxHeight = '';
  };
  body.textContent = text;
  wrap.appendChild(head);
  wrap.appendChild(body);
  bar.appendChild(wrap);
}

// タブ/選択肢ボタンの圧縮表示テキスト。shortLabel を優先し、無ければ label 先頭を
// 全角8字で自動短縮する（最終的な伸縮は CSS の max-width + ellipsis が担う）。
function batchShortText(opt) {
  if (opt && opt.shortLabel) return String(opt.shortLabel);
  const s = String((opt && opt.label) || '').trim();
  return s.length > 8 ? s.slice(0, 8) + '…' : s;
}

function batchSectionAnswered(sessionId, idx) {
  const selections = batchSelections.get(sessionId);
  if (!selections) return false;
  const sel = selections[idx];
  if (sel == null) return false;
  if (sel === BATCH_FREE) {
    const ft = batchFreeText.get(sessionId) || [];
    return (ft[idx] || '').trim().length > 0;
  }
  return true;
}

function batchAllAnswered(sessionId, sections) {
  if (!sections || sections.length === 0) return false;
  for (let i = 0; i < sections.length; i++) {
    if (!batchSectionAnswered(sessionId, i)) return false;
  }
  return true;
}

// 単一質問モードの描画（showBatchActionBar から呼ばれる分岐実体・plan_choice-tab-ui.md C5）。
// タブは常に1つ。選択肢ボタン押下で確認モーダルを挟まず sendChoice で即送信する（1クリック）。
// 単一質問は横幅に余裕があるので短ラベル圧縮・詳細パネルは使わず、ボタンに全文を表示する
// （プレビュー不要で 1 クリック・誤クリック防止）。自由入力肢（「N. User specifies」）は
// 入力欄を開き、Enter で入力テキストをそのまま送る。
function showSingleSectionBar(bar, sessionId, section, ctx) {
  const { shouldStickToBottom, chatTlB, chatWasAtBottomB, forceStickToBottom } = ctx;
  const options = (section.options || []);
  const allowFree = !!section._freeInput;
  const title = section.title || 'Approval needed';
  const labelKind = section._labelKind || 'approval';
  const labelTitle = section._labelTitle || '';
  // 承認系（Yes/No・AI の番号付き選択肢）の質問本文。sequential は title 側に出すため除外。
  const question = (labelKind === 'approval' && section._question) ? String(section._question).trim() : '';
  const preamble = section._preamble ? String(section._preamble).trim() : '';
  const freeActive = allowFree && !!singleFreeActive.get(sessionId);
	const summary = section._summary || null;

  const sig = JSON.stringify({
    s: sessionId,
    mode: 'single-tabs',
    title,
    lk: labelKind,
    q: question,
    pre: preamble,
		approvalSummary: summary ? { c: summary.command, p: summary.paths, r: summary.risk, raw: summary.raw } : null,
    opts: options.map(o => ({ n: o.num, l: o.label, c: !!o.isCurrent, p: !!o.preserveOrder })),
    free: allowFree,
    fa: freeActive,
    v: bar.classList.contains('visible'),
    col: isActionBarCollapsed(),
  });
  probe('approval.draw', () => ({
    sessionId, mode: 'single-tabs', preamble, question, options,
    sigSkipped: lastActionBarRender.sessionId === sessionId && lastActionBarRender.sig === sig,
  }));
  if (lastActionBarRender.sessionId === sessionId && lastActionBarRender.sig === sig) {
    if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
    if (chatWasAtBottomB && chatTlB) requestAnimationFrame(() => scrollChatPaneToBottom(chatTlB));
    return;
  }
  lastActionBarRender.sessionId = sessionId;
  lastActionBarRender.sig = sig;
  bar.innerHTML = '';
  bar.classList.remove('batch', 'multi-select');
  bar.classList.add('single-tabs');
  batchSelections.delete(sessionId);
  multiSelectSelections.delete(sessionId);

  if (options.length === 0 && !allowFree) return;

  // ===== ラベル =====
  const label = document.createElement('span');
  label.className = 'action-bar-label';
  if (labelKind === 'sequential') {
    label.textContent = `⚠ ${labelTitle || title}`;
    label.classList.add('sequential-question-label');
    label.title = labelTitle || title;
  } else {
    label.textContent = '⚠ Approval needed';
  }
  bar.appendChild(label);

	if (summary) appendApprovalSummaryCard(bar, summary);

  // 承認ブロック直前の地の文（前置き説明）があれば先頭に表示する（バッチ/複数選択と対称）。
  // 単一質問だけここが抜けていたため、AI の質問文・文脈がポップアップに出ず CLI 画面を
  // 見ないと内容が分からなかった。
  appendApprovalPreamble(bar, preamble);

  // ===== パネル（全文選択肢 + 自由入力欄） =====
  // 単一質問モードでは質問タブ列（常に1タブで無意味）と質問見出しを出さない。
  // 質問文/承認文言は上の黄色ヘッダー（action-bar-label）に集約済みで、見出し・タブに
  // 同じ文字列を重ねると「Approval needed」が 3 連発して冗長になるため（plan_choice-tab-ui.md C5 改）。
  const pane = document.createElement('div');
  pane.className = 'action-qpane';

  // AI の質問本文を選択肢の上に全文表示する（折り返し可・バッチの action-qhead と同じ見た目）。
  // ラベル（黄色ヘッダ）は単行省略のため、長い質問はここで読めるようにする。
  if (question) {
    const qhead = document.createElement('div');
    qhead.className = 'action-qhead action-single-qhead';
    qhead.textContent = question;
    qhead.title = question;
    pane.appendChild(qhead);
  }

  // "Yes, and" 系（セッション全体許可）があればそれを推奨扱いにする（既存ロジック踏襲）。
  const isSessionAllowLabel = (s) => /during this session|allow.*session|yes.*allow/i.test(s);
  const hasSessionAllow = options.some(o => isSessionAllowLabel(o.label));
  const isRecommendedOpt = (o) => hasSessionAllow ? isSessionAllowLabel(o.label) : o.isCurrent;

  const isHighRiskApprove = (opt) => isHighRiskApprovalSelection(sessionId, opt.num);

  // 選択肢ボタンは全文表示（短ラベル圧縮なし）。.action-btn の通常スタイルで折り返す
  //（詳細パネルでのプレビューが不要になり、見て 1 クリックで送れる）。
  const optsEl = document.createElement('div');
  optsEl.className = 'action-qopts action-qopts-full';
  for (const opt of options) {
    const btn = document.createElement('button');
    const isPermanent = /don[''']t ask again/i.test(opt.label);
    const isSessionAllow = isSessionAllowLabel(opt.label);
    let cls = 'action-btn';
    if (isSessionAllow) cls += ' session-allow';
    else if (isRecommendedOpt(opt)) cls += ' current';
    if (isPermanent) cls += ' permanent';
    btn.className = cls;
    btn.textContent = `${opt.num}. ${opt.label}` + (isRecommendedOpt(opt) ? ` (${t('approval_recommended')})` : '');
    btn.title = `${opt.num}. ${opt.label}`;
    if (isHighRiskApprove(opt)) {
      btn.classList.add('high-risk-approval');
      bindHighRiskApproval(btn, sessionId, opt.num);
    } else {
      btn.onclick = () => sendChoice(sessionId, opt.num); // 即送信（確認モーダルなし）
    }
    optsEl.appendChild(btn);
  }

  // 自由入力肢（あれば）。クリックで入力欄を開き、入力後 Enter で即送信する。
  if (allowFree) {
    const fbtn = document.createElement('button');
    fbtn.className = 'action-btn' + (freeActive ? ' current' : '');
    fbtn.textContent = `N. ${t('approval_free_input')}`;
    fbtn.title = t('approval_free_input');
    fbtn.onclick = (e) => { e.stopPropagation(); activateSingleFree(sessionId); };
    optsEl.appendChild(fbtn);
  }
  pane.appendChild(optsEl);

  // 自由入力中だけ入力欄を出す（詳細パネルは廃止）。
  // 「N. 自由入力」ボタンの直後（optsEl 内）に入れて同じ行の右側に並べる。
  // pane 下へ縦積みすると余分な行高が増えるため、横並びで高さを抑える。
  if (freeActive) {
    const inp = document.createElement('input');
    inp.className = 'action-qfreein action-qfreein-inline';
    inp.type = 'text';
    inp.placeholder = t('approval_free_input_placeholder');
    inp.value = singleFreeText.get(sessionId) || '';
    inp.oninput = () => singleFreeText.set(sessionId, inp.value);
    inp.onkeydown = (e) => {
      // 日本語 IME の変換確定 Enter は、この input の送信 Enter と区別する。
      // isComposing が取れないブラウザでは keyCode 229 が最後の保険になる。
      const imeComposing = e.isComposing || (e as any).keyCode === 229;
      if (e.key === 'Enter' && !e.shiftKey && !imeComposing) {
        e.preventDefault();
        e.stopPropagation();
        sendSingleFreeText(sessionId);
      }
    };
    optsEl.appendChild(inp);
    setTimeout(() => inp.focus(), 0);
  }
  bar.appendChild(pane);

  // ✕: 1 行の帯に畳む（端末右下の「✕ 承認」と同じ foldApprovalPanel）。
  const closeBtn = document.createElement('button');
  closeBtn.className = 'action-dismiss-btn';
  closeBtn.textContent = '✕';
  closeBtn.title = t('dismiss_title');
  closeBtn.setAttribute('aria-label', t('dismiss_title'));
  closeBtn.onclick = (e) => {
    e.stopPropagation();
    foldApprovalPanel(sessionId);
  };
  bar.appendChild(closeBtn);
  appendCollapseToggle(bar, sessionId);
  bar.classList.toggle('collapsed', isActionBarCollapsed());

  if (!bar.classList.contains('visible')) suppressPtyResizeForInputLayout(350);
  // 観測用: 承認バーの出現が #terminal-area の高さを動かす主要因なので、
  bar.classList.add('visible');
  // 60 秒抑制で縮小サイズを Codex へ一切伝えないと、Codex が高い行数のまま再描画を続け
  // scrollback へ空行が化石化して表示がまばらになる。短く束ねた後、確定サイズを 1 回送る。
  syncPtySizeToViewportAfterLayout(sessionId, shouldStickToBottom);
  actionBarShownAt.set(sessionId, Date.now());
  if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
  if (chatWasAtBottomB && chatTlB) requestAnimationFrame(() => scrollChatPaneToBottom(chatTlB));
}

function highRiskConfirmationMode() {
  try {
    return localStorage.getItem(STORAGE_HIGH_RISK_CONFIRMATION_MODE_KEY) === 'dialog' ? 'dialog' : 'hold';
  } catch (_) {
    return 'hold';
  }
}

function isHighRiskApprovalSelection(sessionId, targetNum) {
  const cached = pendingApprovalOptions(sessionId);
  if (!Array.isArray(cached) || isBatchOptions(cached) || isMultiSelectOptions(cached)) return false;
  const summary = (cached as any)._summary;
  if (!summary || summary.risk !== 'high') return false;
  const opt = cached.find((item) => item && item.num === targetNum);
  if (!opt) return false;
  // Deny/reject actions remain immediate: the guard exists only to prevent an
  // accidental affirmative action from executing a destructive command.
  return !/\b(no|deny|reject|skip|cancel|abort|decline)\b|don't\s+allow/i.test(String(opt.label || ''));
}

export function bindHighRiskApproval(btn, sessionId, targetNum) {
  if (highRiskConfirmationMode() === 'dialog') {
    btn.textContent += ' (confirm)';
    btn.onclick = () => void requestHighRiskConfirmation(sessionId, targetNum);
    return;
  }

  btn.classList.add('hold-to-approve');
  btn.title = `${btn.title} — hold for 1.2 seconds`;
  let timer = null;
  const cancel = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
    btn.classList.remove('holding');
    btn.style.removeProperty('--hold-progress-duration');
  };
  btn.addEventListener('pointerdown', (event) => {
    if (event.button !== 0 || timer !== null) return;
    event.preventDefault();
    btn.setPointerCapture?.(event.pointerId);
    btn.classList.add('holding');
    btn.style.setProperty('--hold-progress-duration', `${HIGH_RISK_HOLD_MS}ms`);
    timer = window.setTimeout(() => {
      timer = null;
      btn.classList.remove('holding');
      btn.style.removeProperty('--hold-progress-duration');
      sendChoice(sessionId, targetNum, true);
    }, HIGH_RISK_HOLD_MS);
  });
  btn.addEventListener('pointerup', cancel);
  btn.addEventListener('pointercancel', cancel);
  btn.addEventListener('pointerleave', (event) => { if (event.buttons === 0) cancel(); });
  btn.onclick = (event) => { event.preventDefault(); }; // suppress synthetic click after a released hold
}

async function requestHighRiskConfirmation(sessionId, targetNum) {
  if (highRiskConfirmationInFlight.has(sessionId)) return;
  highRiskConfirmationInFlight.add(sessionId);
  try {
    const ok = await appConfirm({
      title: 'High-risk approval',
      message: 'This approval can run a destructive or externally visible operation. Continue only if you have checked the command and target paths.',
      confirmText: 'Approve',
      cancelText: 'Cancel',
      kind: 'danger',
    });
    if (ok) sendChoice(sessionId, targetNum, true);
  } finally {
    highRiskConfirmationInFlight.delete(sessionId);
  }
}

function appendApprovalSummaryCard(bar, summary) {
  const card = document.createElement('div');
  card.className = `approval-summary-card risk-${summary.risk || 'mid'}`;

  const risk = document.createElement('span');
  risk.className = 'approval-risk-badge';
  risk.textContent = summary.risk === 'high' ? 'HIGH' : summary.risk === 'low' ? 'LOW' : 'MID';
  card.appendChild(risk);

  const command = document.createElement('span');
  command.className = 'approval-summary-command';
  command.textContent = summary.command ? `${summary.command} を実行してよいか` : 'この操作を実行してよいか';
  command.title = command.textContent;
  card.appendChild(command);

  for (const path of summary.paths || []) {
    const badge = document.createElement('span');
    badge.className = 'approval-path-badge';
    badge.textContent = path;
    badge.title = path;
    card.appendChild(badge);
  }

  if (summary.raw) {
    const raw = document.createElement('details');
    raw.className = 'approval-summary-raw';
    const rawTitle = document.createElement('summary');
    rawTitle.textContent = 'raw';
    raw.appendChild(rawTitle);
    const rawBody = document.createElement('pre');
    rawBody.textContent = summary.raw;
    raw.appendChild(rawBody);
    card.appendChild(raw);
  }
  bar.appendChild(card);
}

export function showBatchActionBar(bar, sessionId, sections, forceStickToBottom = false) {
  const term = sessionId === activeSessionId ? terminals.get(sessionId) : null;
  const shouldStickToBottom = !!(term && (forceStickToBottom || term.autoScroll || isTerminalAtBottom(term)));
  const chatTlB = getChatTimelineEl();
  const chatWasAtBottomB = chatTlB ? chatPaneAtBottom(chatTlB) : false;

  // 単一質問モード（plan_choice-tab-ui.md C5）: showActionBar が flat options を
  // [{_single:true,...}] 1 セクションへ変換して渡す。バッチと同じタブ/パネル/詳細/自由入力の
  // 見た目を踏襲しつつ、タブは常に1つ・確認モーダルを出さず即送信する点だけが異なる。
  if (sections.length === 1 && (sections[0] as any)._single) {
    showSingleSectionBar(bar, sessionId, sections[0], { shouldStickToBottom, chatTlB, chatWasAtBottomB, forceStickToBottom });
    return;
  }

  // セクション数が変わったら選択状態・自由入力をリセット（前回セレクションの持ち越し防止）
  let selections = batchSelections.get(sessionId);
  if (!selections || selections.length !== sections.length) {
    selections = new Array(sections.length).fill(null);
    batchSelections.set(sessionId, selections);
  }
  let freeTexts = batchFreeText.get(sessionId);
  if (!freeTexts || freeTexts.length !== sections.length) {
    freeTexts = new Array(sections.length).fill('');
    batchFreeText.set(sessionId, freeTexts);
  }
  const activeQ = getBatchActiveQ(sessionId, sections.length);

  const sig = JSON.stringify({
    s: sessionId,
    mode: 'batch-tabs',
    sects: sections.map(sec => ({
      n: sec.num, t: sec.title, f: !!sec._freeInput,
      o: (sec.options || []).map(o => ({ n: o.num, l: o.label, s: o.shortLabel || '', c: !!o.isCurrent })),
    })),
    sel: selections,
    aq: activeQ,
    v: bar.classList.contains('visible'),
    col: isActionBarCollapsed(),
  });
  probe('approval.draw', () => ({
    sessionId, mode: 'batch-tabs', preamble: (sections as any)._preamble, question: '', options: sections,
    sigSkipped: lastActionBarRender.sessionId === sessionId && lastActionBarRender.sig === sig,
  }));
  if (lastActionBarRender.sessionId === sessionId && lastActionBarRender.sig === sig) {
    if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
    if (chatWasAtBottomB && chatTlB) requestAnimationFrame(() => scrollChatPaneToBottom(chatTlB));
    return;
  }
  lastActionBarRender.sessionId = sessionId;
  lastActionBarRender.sig = sig;
  // 再描画前に質問タブ列の横スクロール位置を覚えておき、再構築後に復元する。
  // これをしないとタブクリックで再描画が走るたびに scrollLeft が 0 に戻り、
  // ユーザーがスクロールして表示していた右側のタブ（例: Q6 をクリックしたら Q1 へ戻る）に
  // 強制的に巻き戻ってしまう。
  const prevTabsScrollLeft = (() => {
    const prev = bar.querySelector('.action-qtabs') as HTMLElement | null;
    return prev ? prev.scrollLeft : 0;
  })();
  bar.innerHTML = '';
  bar.classList.remove('single-tabs', 'multi-select');
  bar.classList.add('batch');

  const label = document.createElement('span');
  label.className = 'action-bar-label';
  label.textContent = t('approval_batch_label', { n: sections.length });
  bar.appendChild(label);

  const todoBadge = document.createElement('span');
  todoBadge.className = 'action-batch-todo-badge';
  bar.appendChild(todoBadge);

  // 承認ブロック直前の地の文（前置き説明）があれば先頭に表示する。
  appendApprovalPreamble(bar, (sections as any)._preamble);

  // ===== 質問タブ列（横スクロール・✓/未 ステータス付き） =====
  const tabsEl = document.createElement('div');
  tabsEl.className = 'action-qtabs';
  const statusEls: any[] = [];
  sections.forEach((sec, idx) => {
    const tab = document.createElement('button');
    tab.className = 'action-qtab' + (idx === activeQ ? ' active' : '');
    tab.onclick = (e) => { e.stopPropagation(); setBatchActiveQ(sessionId, idx); };
    const qn = document.createElement('span');
    qn.className = 'qn';
    // 質問番号は `Q1` 表記（選択肢番号 `1.` `2.` と視覚的に区別する）
    qn.textContent = `Q${idx + 1}`;
    const txt = document.createElement('span');
    txt.className = 'qlabel';
    txt.textContent = sec.title;
    txt.title = sec.title;
    const st = document.createElement('span');
    st.className = 'st';
    statusEls[idx] = st;
    tab.appendChild(qn); tab.appendChild(txt); tab.appendChild(st);
    tabsEl.appendChild(tab);
  });
  bar.appendChild(tabsEl);
  // 再描画前の横スクロール位置を復元してからアクティブタブを可視範囲に入れる。
  // クリックは可視範囲内のタブを押したケースなので scrollLeft 復元だけで足り、
  // キーボード（← →）で画面外へ進めた場合だけ scrollIntoView が必要分だけ動かす。
  tabsEl.scrollLeft = prevTabsScrollLeft;
  requestAnimationFrame(() => {
    const active = tabsEl.querySelector('.action-qtab.active') as HTMLElement | null;
    if (!active) return;
    // scrollIntoView は親 bar / ページ全体まで動かしてしまうので、tabsEl 内で手動補正する。
    const tabLeft = active.offsetLeft;
    const tabRight = tabLeft + active.offsetWidth;
    const viewLeft = tabsEl.scrollLeft;
    const viewRight = viewLeft + tabsEl.clientWidth;
    if (tabLeft < viewLeft) tabsEl.scrollLeft = tabLeft;
    else if (tabRight > viewRight) tabsEl.scrollLeft = tabRight - tabsEl.clientWidth;
  });

  // ===== アクティブ質問のパネル（選択肢 + 詳細 + 自由入力） =====
  const pane = document.createElement('div');
  pane.className = 'action-qpane';
  const activeSec = sections[activeQ];

  const head = document.createElement('div');
  head.className = 'action-qhead';
  head.textContent = `Q${activeQ + 1}/${sections.length} ${activeSec.title}`;
  head.title = activeSec.title;
  pane.appendChild(head);

  // 選択肢（+ 自由入力肢）
  const choices = (activeSec.options || []).slice();
  if (activeSec._freeInput) {
    choices.push({ num: BATCH_FREE, label: t('approval_free_input_full'), shortLabel: t('approval_free_input'), _free: true });
  }

  const optsEl = document.createElement('div');
  optsEl.className = 'action-qopts';
  choices.forEach((opt) => {
    const btn = document.createElement('button');
    let cls = 'action-btn batch-option action-qopt';
    if (opt.isCurrent) cls += ' current';
    if (selections[activeQ] === opt.num) cls += ' selected';
    btn.className = cls;
    const nEl = document.createElement('span');
    nEl.className = 'n';
    nEl.textContent = opt._free ? 'N' : `${opt.num}`;
    const lEl = document.createElement('span');
    lEl.className = 'opt-label';
    lEl.textContent = batchShortText(opt);
    btn.appendChild(nEl); btn.appendChild(lEl);
    btn.title = opt._free ? t('approval_free_input') : `${opt.num}. ${opt.label}`;
    btn.onclick = (e) => { e.stopPropagation(); selectBatchOption(sessionId, activeQ, opt.num); };
    optsEl.appendChild(btn);
  });
  pane.appendChild(optsEl);

  // 詳細パネル（選択中の全文 / 自由入力欄）
  const detail = document.createElement('div');
  const sel = selections[activeQ];
  // status/progress/submit を入力中に再構築せず更新する closure（後段で定義）
  let updateBatchStatus = () => {};
  if (sel != null) {
    const opt = sel === BATCH_FREE
      ? { num: BATCH_FREE, label: t('approval_free_input_full'), _free: true } as any
      : choices.find((o) => o.num === sel);
    if (opt) {
      detail.className = 'action-qdetail';
      const lab = document.createElement('span');
      lab.className = 'detail-lab';
      lab.textContent = opt._free ? t('approval_free_input') : `${opt.num}. ${batchShortText(opt)}`;
      detail.appendChild(lab);
      if (!opt._free) {
        const body = document.createElement('div');
        body.className = 'detail-body';
        body.textContent = opt.label + (opt.isCurrent ? ` (${t('approval_recommended')})` : '');
        detail.appendChild(body);
      } else {
        const inp = document.createElement('input');
        inp.className = 'action-qfreein';
        inp.type = 'text';
        inp.placeholder = t('approval_free_input_placeholder');
        inp.value = freeTexts[activeQ] || '';
        inp.oninput = () => {
          freeTexts[activeQ] = inp.value;
          // 入力中は full rebuild せずステータス/進捗/送信のみ更新（フォーカス維持）
          updateBatchStatus();
        };
        detail.appendChild(inp);
        setTimeout(() => inp.focus(), 0);
      }
    } else {
      detail.className = 'action-qdetail empty';
      detail.textContent = t('approval_batch_detail_empty');
    }
  } else {
    detail.className = 'action-qdetail empty';
    detail.textContent = t('approval_batch_detail_empty');
    if (activeSec._freeInput) {
      detail.classList.add('clickable');
      detail.onclick = (e) => { e.stopPropagation(); selectBatchOption(sessionId, activeQ, BATCH_FREE); };
    }
  }
  // ===== 詳細メッセージ＋送信バーを横並びに =====
  // 左に「クリア / 送信確認」、右にメッセージ詳細を置く。縦積みの送信バー行を無くし、
  // 承認ポップアップの高さを抑える（CLI 表示欄を広く保ち、メッセージ確認をしやすくする）。
  const detailRow = document.createElement('div');
  detailRow.className = 'action-qdetail-row';

  const actions = document.createElement('div');
  actions.className = 'action-qdetail-actions';

  const progress = document.createElement('span');
  progress.className = 'action-bar-progress';
  actions.appendChild(progress);

  const actionBtns = document.createElement('div');
  actionBtns.className = 'action-qdetail-btns';

  const clearBtn = document.createElement('button');
  clearBtn.className = 'action-clear-btn';
  clearBtn.textContent = t('approval_batch_clear');
  clearBtn.onclick = (e) => { e.stopPropagation(); clearBatchSelections(sessionId); };
  actionBtns.appendChild(clearBtn);

  const submitBtn = document.createElement('button');
  submitBtn.className = 'action-submit-btn';
  submitBtn.textContent = t('approval_batch_confirm');
  submitBtn.onclick = (e) => { e.stopPropagation(); openBatchConfirm(sessionId); };
  actionBtns.appendChild(submitBtn);

  actions.appendChild(actionBtns);
  detailRow.appendChild(actions);
  detailRow.appendChild(detail);
  pane.appendChild(detailRow);
  bar.appendChild(pane);

  // 畳む（✕）は position:absolute なので bar 直下に置けば右上に固定表示される。
  // 端末右下の「✕ 承認」と同じ foldApprovalPanel。
  const closeBatchBtn = document.createElement('button');
  closeBatchBtn.className = 'action-dismiss-btn';
  closeBatchBtn.textContent = '✕';
  closeBatchBtn.title = t('dismiss_title');
  closeBatchBtn.setAttribute('aria-label', t('dismiss_title'));
  closeBatchBtn.onclick = (e) => {
    e.stopPropagation();
    foldApprovalPanel(sessionId);
  };
  bar.appendChild(closeBatchBtn);

  // タブ ✓/未・進捗・送信ボタン活性を一括更新（自由入力の oninput からも呼ぶ）
  updateBatchStatus = () => {
    let done = 0;
    sections.forEach((sec, idx) => {
      const ok = batchSectionAnswered(sessionId, idx);
      if (ok) done++;
      const st = statusEls[idx];
      if (st) { st.textContent = ok ? '✓' : '未'; st.className = 'st ' + (ok ? 'done' : 'todo'); }
    });
    progress.textContent = t('approval_batch_progress', { done, total: sections.length });
    const todo = sections.length - done;
    todoBadge.textContent = todo > 0 ? t('approval_batch_todo_badge', { n: todo }) : t('approval_batch_all_done');
    todoBadge.classList.toggle('done', todo === 0);
    submitBtn.disabled = done < sections.length;
  };
  updateBatchStatus();

  appendCollapseToggle(bar, sessionId);
  bar.classList.toggle('collapsed', isActionBarCollapsed());
  if (!bar.classList.contains('visible')) suppressPtyResizeForInputLayout(350);
  // 観測用: 承認バーの出現が #terminal-area の高さを動かす主要因なので、
  bar.classList.add('visible');
  // 60 秒抑制で縮小サイズを Codex へ一切伝えないと、Codex が高い行数のまま再描画を続け
  // scrollback へ空行が化石化して表示がまばらになる。短く束ねた後、確定サイズを 1 回送る。
  syncPtySizeToViewportAfterLayout(sessionId, shouldStickToBottom);
  actionBarShownAt.set(sessionId, Date.now());
  if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
  if (chatWasAtBottomB && chatTlB) requestAnimationFrame(() => scrollChatPaneToBottom(chatTlB));
}

export function setBatchActiveQ(sessionId, idx) {
  batchActiveQ.set(sessionId, idx);
  const cached = pendingApprovalOptions(sessionId);
  if (isBatchOptions(cached)) {
    const bar = document.getElementById('action-bar');
    if (bar) showBatchActionBar(bar, sessionId, cached);
  }
}

export function selectBatchOption(sessionId, sectionIdx, optionNum) {
  const selections = batchSelections.get(sessionId);
  if (!selections) return;
  selections[sectionIdx] = optionNum;
  batchActiveQ.set(sessionId, sectionIdx); // 選んだ質問をアクティブに保つ（自動で別タブに飛ばさない）
  const cached = pendingApprovalOptions(sessionId);
  if (isBatchOptions(cached)) {
    const bar = document.getElementById('action-bar');
    if (bar) showBatchActionBar(bar, sessionId, cached);
  }
  // 自由入力肢は再描画後に入力欄へフォーカスする（showBatchActionBar 内）。それ以外は本体入力へ。
  if (optionNum !== BATCH_FREE) setTimeout(() => inputEl.focus(), 0);
}

export function clearBatchSelections(sessionId) {
  const cached = pendingApprovalOptions(sessionId);
  if (!isBatchOptions(cached)) return;
  batchSelections.set(sessionId, new Array(cached.length).fill(null));
  batchFreeText.set(sessionId, new Array(cached.length).fill(''));
  batchActiveQ.set(sessionId, 0);
  const bar = document.getElementById('action-bar');
  if (bar) showBatchActionBar(bar, sessionId, cached);
  setTimeout(() => inputEl.focus(), 0);
}

// 実送信文字列を組み立てる。各行「質問番号 選択肢番号」。自由入力の行は入力テキストを送る。
// 選択肢番号はエージェントが提示した実番号（ボタン表示と一致）をそのまま使う
// （1始まり位置への変換はしない。グローバル連番の場合に表示・回答・解釈がずれるため）。
function buildBatchPayload(sessionId) {
  const selections = batchSelections.get(sessionId) || [];
  const freeTexts = batchFreeText.get(sessionId) || [];
  return selections.map((sel, idx) =>
    sel === BATCH_FREE ? `${idx + 1} ${(freeTexts[idx] || '').trim()}` : `${idx + 1} ${sel}`
  ).join('\n');
}

// 確認モーダル用の人が読む形（質問タイトル＋選んだラベル/全文、自由入力は入力テキスト）。
function buildBatchReadable(sessionId, sections) {
  const selections = batchSelections.get(sessionId) || [];
  const freeTexts = batchFreeText.get(sessionId) || [];
  return sections.map((sec, idx) => {
    const sel = selections[idx];
    let val;
    if (sel === BATCH_FREE) {
      val = `${t('approval_free_input')}「${(freeTexts[idx] || '').trim()}」`;
    } else {
      const opt = (sec.options || []).find((o) => o.num === sel);
      val = opt ? `${opt.shortLabel ? opt.shortLabel + ' — ' : ''}${opt.label}` : `${sel}`;
    }
    return `Q${idx + 1} ${sec.title}\n   → ${val}`;
  }).join('\n');
}

function removeBatchConfirmModal() {
  const m = document.getElementById('action-confirm-mask');
  if (m && m.parentNode) m.parentNode.removeChild(m);
}

// 「送信確認」: 全問回答済みのときだけ、内容＋実送信文字列を確認するモーダルを開く。
export function openBatchConfirm(sessionId) {
  const cached = pendingApprovalOptions(sessionId);
  if (!isBatchOptions(cached)) return;
  if (!batchAllAnswered(sessionId, cached)) return;
  removeBatchConfirmModal();

  const mask = document.createElement('div');
  mask.className = 'action-confirm-mask aac-wheel-overlay';
  mask.id = 'action-confirm-mask';
  const modal = document.createElement('div');
  modal.className = 'action-confirm-modal';

  const h = document.createElement('h3');
  h.textContent = t('approval_confirm_title');
  modal.appendChild(h);

  const p1 = document.createElement('p');
  p1.textContent = t('approval_confirm_readable_label');
  modal.appendChild(p1);
  const readable = document.createElement('div');
  readable.className = 'action-confirm-readable';
  readable.textContent = buildBatchReadable(sessionId, cached);
  modal.appendChild(readable);

  const p2 = document.createElement('p');
  p2.textContent = t('approval_confirm_payload_label');
  modal.appendChild(p2);
  const payload = document.createElement('div');
  payload.className = 'action-confirm-payload';
  payload.textContent = buildBatchPayload(sessionId);
  modal.appendChild(payload);

  const row = document.createElement('div');
  row.className = 'action-confirm-row';
  const back = document.createElement('button');
  back.className = 'action-confirm-back';
  back.textContent = t('approval_confirm_back');
  back.onclick = (e) => { e.stopPropagation(); removeBatchConfirmModal(); setTimeout(() => inputEl.focus(), 0); };
  const go = document.createElement('button');
  go.className = 'action-confirm-go';
  go.textContent = t('approval_confirm_send');
  go.onclick = (e) => {
    e.stopPropagation();
    sendBatchChoices(sessionId);
  };
  row.appendChild(back); row.appendChild(go);
  modal.appendChild(row);

  mask.appendChild(modal);
  // 背景クリックで閉じる（戻る相当）
  mask.onclick = (e) => { if (e.target === mask) { removeBatchConfirmModal(); setTimeout(() => inputEl.focus(), 0); } };
  document.body.appendChild(mask);
  setTimeout(() => go.focus(), 0);
}

// 確定送信（モーダルの「送信」から呼ぶ）。送信後は完了メッセージを出さず UI を消して会話に戻る。
export function sendBatchChoices(sessionId) {
  const cached = pendingApprovalOptions(sessionId);
  if (!isBatchOptions(cached)) return;
  if (!batchAllAnswered(sessionId, cached)) return;
  const text = buildBatchPayload(sessionId);
  // 本文送信に成功するまで、回答済みの印・確認モーダル・入力 state を変更しない。
  if (!sendSubmittedBody(sessionId, text, { recordMobileTranscript: false })) return false;
  noteApprovalAnswerSent(sessionId, text, cached);
  // 一括回答は改行区切りの複数行（buildBatchPayload）。本文は上で共通経路へ送っている。
  removeBatchConfirmModal();
  hideActionBar(sessionId);
  setTimeout(() => inputEl.focus(), 0);
  return true;
}

export function isBatchActionBarVisible() {
  const bar = document.getElementById('action-bar');
  return !!(bar && bar.classList.contains('visible') && bar.classList.contains('batch'));
}

// 質問タブの移動（Tab / ←→）。タブを巡回するだけで選択は変えない。
export function moveBatchFocus(delta) {
  if (activeSessionId === null) return false;
  const cached = pendingApprovalOptions(activeSessionId);
  if (!isBatchOptions(cached) || cached.length === 0) return false;
  const n = cached.length;
  const cur = getBatchActiveQ(activeSessionId, n);
  setBatchActiveQ(activeSessionId, ((cur + delta) % n + n) % n);
  return true;
}

export function handleBatchNumberKey(sessionId, num) {
  const cached = pendingApprovalOptions(sessionId);
  if (!isBatchOptions(cached)) return false;
  const idx = getBatchActiveQ(sessionId, cached.length);
  const section = cached[idx];
  if (!section) return false;
  const opt = (section.options || []).find(o => o.num === num);
  if (!opt) return false;
  selectBatchOption(sessionId, idx, num);
  return true;
}

// ---- 複数選択（#multi）: 1 問で任意個 ON/OFF できるチェックボックス UI ----

export function showMultiSelectActionBar(bar, sessionId, options, forceStickToBottom = false) {
  const term = sessionId === activeSessionId ? terminals.get(sessionId) : null;
  const shouldStickToBottom = !!(term && (forceStickToBottom || term.autoScroll || isTerminalAtBottom(term)));
  const chatTlM = getChatTimelineEl();
  const chatWasAtBottomM = chatTlM ? chatPaneAtBottom(chatTlM) : false;

  let selected = multiSelectSelections.get(sessionId);
  if (!selected) {
    selected = new Set();
    multiSelectSelections.set(sessionId, selected);
    if (multiSelectFocusIdx < 0 || multiSelectFocusIdx >= options.length) set_multiSelectFocusIdx(0);
  }
  const question = (options[0] && options[0]._question) || '';

  const sig = JSON.stringify({
    s: sessionId,
    mode: 'multi',
    q: question,
    opts: options.map(o => ({ n: o.num, l: o.label })),
    sel: Array.from(selected).sort((a, b) => a - b),
    f: multiSelectFocusIdx,
    v: bar.classList.contains('visible'),
    col: isActionBarCollapsed(),
  });
  probe('approval.draw', () => ({
    sessionId, mode: 'multi', preamble: (options as any)._preamble, question, options,
    sigSkipped: lastActionBarRender.sessionId === sessionId && lastActionBarRender.sig === sig,
  }));
  if (lastActionBarRender.sessionId === sessionId && lastActionBarRender.sig === sig) {
    if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
    if (chatWasAtBottomM && chatTlM) requestAnimationFrame(() => scrollChatPaneToBottom(chatTlM));
    return;
  }
  lastActionBarRender.sessionId = sessionId;
  lastActionBarRender.sig = sig;
  bar.innerHTML = '';
  bar.classList.remove('batch', 'single-tabs');
  bar.classList.add('multi-select');

  const label = document.createElement('span');
  label.className = 'action-bar-label';
  label.textContent = question ? `⚠ ${question}` : t('approval_multi_label');
  if (question) label.title = question;
  bar.appendChild(label);

  // 承認ブロック直前の地の文（前置き説明）があれば先頭に表示する。
  appendApprovalPreamble(bar, (options as any)._preamble);

  const btnRow = document.createElement('div');
  btnRow.className = 'action-section-buttons';
  options.forEach((opt, idx) => {
    const btn = document.createElement('button');
    let cls = 'action-btn multi-option';
    const checked = selected.has(opt.num);
    if (checked) cls += ' selected';
    if (idx === multiSelectFocusIdx) cls += ' kbd-focus';
    btn.className = cls;
    btn.textContent = `${checked ? '☑' : '☐'} ${opt.num}. ${opt.label}`;
    btn.title = `${opt.num}. ${opt.label}`;
    btn.onclick = (e) => {
      e.stopPropagation();
      set_multiSelectFocusIdx(idx);
      toggleMultiSelectOption(sessionId, opt.num);
    };
    btnRow.appendChild(btn);
  });
  bar.appendChild(btnRow);

  const footer = document.createElement('div');
  footer.className = 'action-bar-footer';

  const progress = document.createElement('span');
  progress.className = 'action-bar-progress';
  progress.textContent = t('approval_multi_progress', { n: selected.size });
  footer.appendChild(progress);

  const selectAllBtn = document.createElement('button');
  selectAllBtn.className = 'action-clear-btn';
  selectAllBtn.textContent = t('approval_multi_select_all');
  selectAllBtn.disabled = selected.size === options.length;
  selectAllBtn.onclick = (e) => {
    e.stopPropagation();
    selectAllMultiSelectOptions(sessionId);
  };
  footer.appendChild(selectAllBtn);

  const clearBtn = document.createElement('button');
  clearBtn.className = 'action-clear-btn';
  clearBtn.textContent = t('approval_batch_clear');
  clearBtn.disabled = selected.size === 0;
  clearBtn.onclick = (e) => {
    e.stopPropagation();
    clearMultiSelectSelections(sessionId);
  };
  footer.appendChild(clearBtn);

  const submitBtn = document.createElement('button');
  submitBtn.className = 'action-submit-btn';
  submitBtn.textContent = t('approval_batch_submit');
  submitBtn.disabled = selected.size === 0;
  submitBtn.onclick = (e) => {
    e.stopPropagation();
    sendMultiSelectChoices(sessionId);
  };
  footer.appendChild(submitBtn);

  // 端末右下の「✕ 承認」と同じ foldApprovalPanel（1 行の帯に畳む）。
  const closeMultiBtn = document.createElement('button');
  closeMultiBtn.className = 'action-dismiss-btn';
  closeMultiBtn.textContent = '✕';
  closeMultiBtn.title = t('dismiss_title');
  closeMultiBtn.setAttribute('aria-label', t('dismiss_title'));
  closeMultiBtn.onclick = (e) => {
    e.stopPropagation();
    foldApprovalPanel(sessionId);
  };
  footer.appendChild(closeMultiBtn);

  bar.appendChild(footer);
  appendCollapseToggle(bar, sessionId);
  bar.classList.toggle('collapsed', isActionBarCollapsed());
  if (!bar.classList.contains('visible')) suppressPtyResizeForInputLayout(350);
  // 観測用: 承認バーの出現が #terminal-area の高さを動かす主要因なので、
  bar.classList.add('visible');
  // 60 秒抑制で縮小サイズを Codex へ一切伝えないと、Codex が高い行数のまま再描画を続け
  // scrollback へ空行が化石化して表示がまばらになる。短く束ねた後、確定サイズを 1 回送る。
  syncPtySizeToViewportAfterLayout(sessionId, shouldStickToBottom);
  actionBarShownAt.set(sessionId, Date.now());
  if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
  if (chatWasAtBottomM && chatTlM) requestAnimationFrame(() => scrollChatPaneToBottom(chatTlM));
}

export function isMultiSelectActionBarVisible() {
  const bar = document.getElementById('action-bar');
  return !!(bar && bar.classList.contains('visible') && bar.classList.contains('multi-select'));
}

export function toggleMultiSelectOption(sessionId, num) {
  const cached = pendingApprovalOptions(sessionId);
  if (!isMultiSelectOptions(cached)) return false;
  if (!cached.some(o => o.num === num)) return false;
  let selected = multiSelectSelections.get(sessionId);
  if (!selected) { selected = new Set(); multiSelectSelections.set(sessionId, selected); }
  if (selected.has(num)) selected.delete(num);
  else selected.add(num);
  const bar = document.getElementById('action-bar');
  if (bar) showMultiSelectActionBar(bar, sessionId, cached);
  setTimeout(() => inputEl.focus(), 0);
  return true;
}

export function clearMultiSelectSelections(sessionId) {
  const cached = pendingApprovalOptions(sessionId);
  if (!isMultiSelectOptions(cached)) return;
  multiSelectSelections.set(sessionId, new Set());
  set_multiSelectFocusIdx(0);
  const bar = document.getElementById('action-bar');
  if (bar) showMultiSelectActionBar(bar, sessionId, cached);
  setTimeout(() => inputEl.focus(), 0);
}

export function selectAllMultiSelectOptions(sessionId) {
  const cached = pendingApprovalOptions(sessionId);
  if (!isMultiSelectOptions(cached)) return;
  multiSelectSelections.set(sessionId, new Set(cached.map(o => o.num)));
  const bar = document.getElementById('action-bar');
  if (bar) showMultiSelectActionBar(bar, sessionId, cached);
  setTimeout(() => inputEl.focus(), 0);
}

export function moveMultiSelectFocus(delta) {
  if (activeSessionId === null) return false;
  const cached = pendingApprovalOptions(activeSessionId);
  if (!isMultiSelectOptions(cached) || cached.length === 0) return false;
  const n = cached.length;
  const start = multiSelectFocusIdx < 0 ? (delta > 0 ? -1 : 0) : multiSelectFocusIdx;
  set_multiSelectFocusIdx(((start + delta) % n + n) % n);
  const bar = document.getElementById('action-bar');
  if (bar) showMultiSelectActionBar(bar, activeSessionId, cached);
  return true;
}

// フォーカス中の選択肢を Space でトグルする
export function toggleMultiSelectFocused(sessionId) {
  const cached = pendingApprovalOptions(sessionId);
  if (!isMultiSelectOptions(cached)) return false;
  if (multiSelectFocusIdx < 0 || multiSelectFocusIdx >= cached.length) return false;
  const opt = cached[multiSelectFocusIdx];
  if (!opt) return false;
  return toggleMultiSelectOption(sessionId, opt.num);
}

export function handleMultiSelectNumberKey(sessionId, num) {
  const cached = pendingApprovalOptions(sessionId);
  if (!isMultiSelectOptions(cached)) return false;
  const idx = cached.findIndex(o => o.num === num);
  if (idx < 0) return false;
  set_multiSelectFocusIdx(idx);
  return toggleMultiSelectOption(sessionId, num);
}

export function sendMultiSelectChoices(sessionId) {
  const selected = multiSelectSelections.get(sessionId);
  if (!selected || selected.size === 0) return;
  const prevOpts = pendingApprovalOptions(sessionId);
  const nums = Array.from(selected).sort((a, b) => a - b);
  // 選択番号をカンマ連結で返す（例 "1,3"）。エージェントが提示した実番号をそのまま使う。
  const text = nums.join(',');
  const labelMap = new Map((isMultiSelectOptions(prevOpts) ? prevOpts : []).map(o => [o.num, o.label]));
  if (!sendSubmittedText(sessionId, `${text}\r`, { recordMobileTranscript: false })) return false;
  chatHistoryCommitOutput(sessionId);
  pushMessage(sessionId, {
    role: 'system',
    kind: 'approval',
    rawText: nums.map(n => labelMap.get(n) || `#${n}`).join(', '),
    meta: {
      kind: 'multi',
      answers: nums,
      labels: nums.map(n => labelMap.get(n) || null),
    },
  });
  noteApprovalAnswerSent(sessionId, text, prevOpts);
  hideActionBar(sessionId);
  setTimeout(() => inputEl.focus(), 0);
}

export function sendChoice(sessionId, targetNum, highRiskConfirmed = false) {
  if (!highRiskConfirmed && isHighRiskApprovalSelection(sessionId, targetNum)) {
    if (highRiskConfirmationMode() === 'dialog') void requestHighRiskConfirmation(sessionId, targetNum);
    return;
  }
  const seqState = sequentialChoiceCache.get(sessionId);
  if (seqState && seqState.index < seqState.prompts.length) {
    const previousIndex = seqState.index;
    const previousAnswers = new Map(seqState.answers);
    const prompt = seqState.prompts[seqState.index];
    seqState.answers.set(prompt.key, targetNum);
    seqState.index++;
    while (seqState.index < seqState.prompts.length && seqState.answers.has(seqState.prompts[seqState.index].key)) {
      seqState.index++;
    }

    if (seqState.index < seqState.prompts.length) {
      // 次の問いを描く（進み具合は sequentialChoiceCache が持ち、記録が変わるまで残る）。
      renderApprovalFromStore(sessionId);
      notifyApprovalViewsChanged();
      setTimeout(() => inputEl.focus(), 0);
      return;
    }

    const response = seqState.prompts
      .map(p => `${p.key}: ${seqState.answers.get(p.key)}`)
      .join('\n');
    // 改行区切りの複数行回答は共通経路（ペースト包み＋確定 \r 別送）で 1 メッセージとして送る。
    // 失敗時は回答 state を残し、再接続後に同じ確定操作をやり直せるようにする。
    if (!sendSubmittedBody(sessionId, response, { recordMobileTranscript: false })) {
      seqState.index = previousIndex;
      seqState.answers.clear();
      previousAnswers.forEach((answer, key) => seqState.answers.set(key, answer));
      return false;
    }
    // chatHistory: 複数質問への一括回答を system/approval として push
    chatHistoryCommitOutput(sessionId);
    pushMessage(sessionId, {
      role: 'system',
      kind: 'approval',
      rawText: response,
      meta: {
        kind: 'batch',
        answers: seqState.prompts.map(p => ({
          key: p.key,
          question: p.question,
          answer: seqState.answers.get(p.key),
        })),
      },
    });
    clearSequentialChoiceState(sessionId);
    noteApprovalAnswerSent(sessionId, response, null);
    hideActionBar(sessionId);
    setTimeout(() => inputEl.focus(), 0);
    return;
  }

  // 矢印移動ではなく番号直接入力で確定する（誤選択防止）
  const cachedOpts = pendingApprovalOptions(sessionId);
  const targetOpt = Array.isArray(cachedOpts) && !isBatchOptions(cachedOpts)
    ? cachedOpts.find(o => o && o.num === targetNum)
    : null;
  const choiceText = targetOpt && targetOpt._sendText ? targetOpt._sendText : `${targetNum}\r`;
  // OpenCode / Codex 等のネイティブショートカットは、1 回のクリックでキー列を
  // 送信する。送信直後に同じボタンが残ったフレームを連打すると同じキー列が PTY
  // キューへ複数回積まれるため、成功した送信に短い再送抑止を掛ける。
  const nativeShortcut = !!(targetOpt && targetOpt._sendText);
  if (nativeShortcut && !beginNativeApprovalSendCooldown(sessionId)) return false;
  const submitted = sendSubmittedText(sessionId, choiceText, { recordMobileTranscript: false });
  if (!submitted) {
    if (nativeShortcut) cancelNativeApprovalSendCooldown(sessionId);
    return false;
  }
  // chatHistory: 単問への回答を system/approval として push
  chatHistoryCommitOutput(sessionId);
  pushMessage(sessionId, {
    role: 'system',
    kind: 'approval',
    rawText: targetOpt ? (targetOpt.label || `#${targetNum}`) : `#${targetNum}`,
    meta: {
      kind: 'single',
      answer: targetNum,
      label: targetOpt ? (targetOpt.label || null) : null,
    },
  });
  // 回答済みの印を付けて Hub へ知らせる。Hub の閉じるが届くまでの間もこの記録は描かない。
  noteApprovalAnswerSent(sessionId, choiceText, cachedOpts);
  hideActionBar(sessionId);
  setTimeout(() => inputEl.focus(), 0);
}

// 代替画面の CLI を遡っている間は、回答済みと同じ形の記録を描かない（approval-ui.ts の
// isStaleHistoryRepaint）。最新の画面へ戻ったら描き直す。記録は変わっていないので、ストアの
// 変化を待っていると描かれないまま残る（遡り位置の正本は alt-scroll-rail-view.ts）。
window.addEventListener('alt-scroll-live', (ev) => {
  const id = Number((ev as CustomEvent)?.detail?.sessionId);
  if (id === activeSessionId) renderApprovalFromStore(id);
});

// 端末右下の「✕ 承認」（パネルが開いている間だけ出る）。type=module は defer 実行のため
// DOM は構築済み。
(function initApprovalRecallBar() {
  const bar = document.getElementById('action-bar');
  if (bar && typeof MutationObserver === 'function') {
    new MutationObserver(syncApprovalRecallBar).observe(bar, {
      attributes: true,
      attributeFilter: ['class', 'data-approval-session-id'],
      childList: true,
    });
  }
  syncApprovalRecallBar();
})();
document.getElementById('approval-dismiss-btn')?.addEventListener('click', () => {
  if (activeSessionId === null) return;
  foldApprovalPanel(activeSessionId);
});
