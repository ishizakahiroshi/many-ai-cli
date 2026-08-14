// --- ESM imports (generated) ---
import { t } from './i18n.js';
import { cleanCopiedText, showToast, token } from './app/util.js';
import { DEFAULT_VOICE_GRACE_SEC, STORAGE_APPROVAL_AUTO_SWITCH_KEY, STORAGE_AUTO_APPROVAL_ENABLED_KEY, STORAGE_HIGH_RISK_CONFIRMATION_MODE_KEY, STORAGE_MOBILE_VOICE_HINT_SHOWN_KEY, STORAGE_NOTIFY_SOUND_CUSTOM_KEY, STORAGE_TOOLS_LEFT_KEY, STORAGE_VOICE_WHISPER_AUTO_SUBMIT_KEY, _putUserPrefsNow, getDefaultTriggerPhrase, getDefaultWakeWordPhrase, setUserPref, setVoiceEngine } from './app/user-prefs.js';
import { DOUBLE_SEND_GUARD_MS, actionBarFocusIdx, actionBarShownAt, activeSessionId, answeredApprovalCandidates, answeredApprovalShapeKeys, answeredMarkerSigs, recordAnsweredMarkerSig, replayAnsweredApprovalTokens, approvalAutoSwitchQueue, approvalConsumedSig, approvalConsumedSigDeleteTimer, approvalRawOptionsCache, approvalSig, approvalSourceCache, approvalSourceEpochCache, approvalReplayState, approvalSuppressUntil, approvalSwitchCandidates, approvalVisibleCache, autoDismissTimers, batchSelections, composeEndSendTimer, isComposing, lastDoSendAt, maybeAutoSwitchToNextApproval, multiQuestionDismissedCache, multiQuestionLatchAt, multiQuestionVisibleCache, pendingSend, removeApprovalAutoSwitchTarget, removeFromSessionOrder, sequentialChoiceCache, sessionInputState, sessions, set_actionBarFocusIdx, set_activeSessionId, set_composeEndSendTimer, set_isComposing, set_lastDoSendAt, set_pendingSend, terminals } from './app/state.js';
import { activateSession, render, renderSessionList, switchSessionByTab } from './app/session-list.js';
import { orderSessions } from './app/state.js';
import { canFitTerminal, fitTerminalPreservingBottom, isTerminalAtBottom, refitActiveTerminalAfterLayout, refitAndStickTerminalToBottomAfterLayoutSettles, resumeTerminalBottomFollow, scrollTerminalToBottomSoon, sendResize, suppressPtyResizeForInputLayout, updateScrollLockBtn } from './app/terminal.js';
import { QUICK_CMD_SLOTS, appConfirm, appConfirmShutdown, appConfirmTypedDanger, appLegacyResetNotice, applyFontSize, applyLang, applyTheme, attachDoneSummaryNotifyToggle, attachTokenStatusbarToggle, getActiveTriggerPhrase, getQuickCommand, loadApprovalSettings, loadSlashCmdSources, loadUsageLinkSettings, quickCommandButtonId, quickCommandDefault, saveUsageLinkSettings, sessionLazyLoaded, sessionViewMode, stripTrailingTriggerPhrase, textEndsWithTriggerPhrase, updateChatCountBadge } from './app/settings.js';
import { ws } from './app/ws-client.js';
import { setMultiQuestionBannerVisible } from './app/approval-ui.js';
import { pendingSessionIds } from './app/approval-queue-tab.js';
import { scheduleDeferredEnter, scheduleAfterOutputSettle, deferredEnterMinWaitFor, cancelDeferredEnter } from './app/deferred-enter.js';
import { scheduleResidueSweep, cancelResidueSweep } from './app/residue-sweep.js';
import { cancelExpandCapture } from './app/expand-popup.js';
import { clearMobileTranscriptSession, recordMobileTranscriptUserSubmission } from './app/mobile-transcript.js';
import { approvalCheckTimers, approvalSuppressRescanTimers, cancelApprovalHintConfirm, clearSequentialChoiceState, detectApproval, getActionBarButtons, handleBatchNumberKey, handleMultiSelectNumberKey, handleOpenCodeApprovalNumberKey, hideActionBar, isBatchActionBarVisible, isMultiSelectActionBarVisible, isSelectMenuActive, isShellProvider, maybeSendDirectApprovalConsumed, moveBatchFocus, moveMultiSelectFocus, openBatchConfirm, sendMultiSelectChoices, setActionBarFocus, shouldSkipClearPrefix, toggleMultiSelectFocused } from './app/approval.js';
import { chatHistoryCommitOutput, isTranscriptBackedSession, mountChatPaneForSession, onChatHistorySessionRemoved, pushMessage, resetAllChatHistory, resetChatHistoryForSession, scrollChatPaneToBottomSoon } from './app/chat-history.js';
import { attachThumbnails, flushPendingAttach, pendingAttachFiles, updateAttachClearBtn, MAX_ATTACH_BYTES } from './app/attachments.js';
import { FilesTabManager } from './app/files-view.js';
import { getExposeStatus, fetchExposeStatus, disableExpose } from './app/host-expose.js';
import './app/detached-grid-launcher.js';
import './app/mobile-home.js';
import { mobileApprovalActiveBadgeCount } from './app/mobile-approval-sheet.js';
import { clearMobileTerminalLiteSession } from './app/mobile-terminal-lite.js';
import './app/orchestration-dashboard.js';
import './app/prompt-templates.js';
import { setActiveTab } from './app/settings.js';

export let _userAvatarUrl = '';
export let _userDisplayName = '';

// i18n ロード前のフォールバック（i18n.js が window.t を上書きするまでキーをそのまま返す）
if (typeof window.t !== 'function') window.t = (key) => key;



// ANSI エスケープシーケンスを取り除く軽量ヘルパ。
// 完全な StripANSI 実装ではなく、表示用 normalized 生成と raw 用の最低限の整形に使う。
// 制御文字や CSI / OSC を概ね除去できれば C3 に渡す材料としては十分。

document.addEventListener('i18n-ready', () => {
  document.getElementById('summary').textContent = t('registering');
  renderSessionList();
  updateInputAffordance();
});

window.addEventListener('orchestration-dashboard-open-session', (event: Event) => {
  const sessionID = Number((event as CustomEvent<{ sessionID?: number }>).detail?.sessionID);
  if (!Number.isInteger(sessionID) || !sessions.has(sessionID)) return;
  activateSession(sessionID);
  setActiveTab(sessionID, 'terminal');
});


// moved to /app/approval.js


// ---- 入力バー ----

export const inputEl = document.getElementById('input') as HTMLTextAreaElement;
export const inputClearBtn = document.getElementById('input-clear-btn');
export const pasteChipsEl = document.getElementById('paste-chips');

// ペースト折りたたみ状態
export const pastedTexts = []; // [{id, text, lineCount}]
export let pasteCounter = 0;

// Files タブは app.ts に依存しない。イベントで常に現在の入力欄へ @path を渡す。
window.addEventListener('many-ai-cli:insert-file-prompt', (event: Event) => {
  const text = String((event as CustomEvent<{ text?: string }>).detail?.text || '').trim();
  if (!text) return;
  inputEl.value += inputEl.value && !inputEl.value.endsWith('\n') ? `\n${text}` : text;
  inputEl.dispatchEvent(new Event('input', { bubbles: true }));
  inputEl.focus();
});

// Desktop palette and mobile chips use one insertion boundary so multiline
// templates retain the same focus and textarea sizing behavior as file prompts.
// detail.send はテンプレート側の「選んだら即送信」トグル。true なら入力欄へ
// 差し込んだうえで通常の送信経路（doSend）をそのまま通す。
window.addEventListener('many-ai-cli:insert-template', (event: Event) => {
  const detail = (event as CustomEvent<{ text?: string; send?: boolean }>).detail;
  const text = String(detail?.text || '');
  if (!text) return;
  inputEl.value += inputEl.value && !inputEl.value.endsWith('\n') ? `\n${text}` : text;
  inputEl.dispatchEvent(new Event('input', { bubbles: true }));
  autoExpand();
  inputEl.focus();
  if (!detail?.send) return;
  if (activeSessionId === null) {
    showToast(t('toast_template_send_no_session'));
    return;
  }
  void doSend(activeSessionId);
});

export function autoExpand(opts: any = {}) {
  const t = activeSessionId === null ? null : terminals.get(activeSessionId);
  const shouldStickToBottom = !!(t && (t.autoScroll || isTerminalAtBottom(t)));
  if (opts.suppressPtyResize) {
    suppressPtyResizeForInputLayout();
  }
  inputEl.style.height = 'auto';
  if (inputEl.value === '') {
    // 空のときは高さを CSS の min-height に任せる。Chrome は placeholder の折り返しも
    // scrollHeight に含めるため、狭い画面で placeholder が 2 行に折り返すと
    // 未入力なのにバーが 2 行分に育ってしまう。
    inputEl.style.height = '';
  } else {
    inputEl.style.height = Math.min(inputEl.scrollHeight, Math.floor(window.innerHeight * 0.3)) + 'px';
  }
  updateInputClearButton();
  refitActiveTerminalAfterLayout(shouldStickToBottom);
}

export function updateInputClearButton() {
  inputClearBtn?.classList.toggle('has-text', inputEl.value.length > 0);
}

// アクティブセッションが実行中（state === 'running'）かを返す単一ヘルパ。
// C1（プレースホルダ差し替え）・C2（送信→停止ボタン化）が共通で参照する。
export function isActiveSessionRunning() {
  if (activeSessionId === null) return false;
  const s = sessions.get(activeSessionId);
  return !!s && s.state === 'running';
}

// 停止ボタン（■）が PTY へ送る中断キーを provider 別に返す。
// grok（Grok Build CLI）は Esc では生成を中断できず Ctrl+C(0x03) のみ有効
// （Windows 実機 v0.2.93 で確認）。他 provider は従来どおり Esc(0x1b)。
export function stopKeyForActiveSession(): string {
  const s = activeSessionId !== null ? sessions.get(activeSessionId) : undefined;
  return s?.provider === 'grok' ? '\x03' : '\x1b';
}

// B1a: スマホ幅判定の単一情報源。OS キーボードの🎤誘導 placeholder の表示判定にだけ使う。
const _mobileVoiceHintMql = (typeof window !== 'undefined' && typeof window.matchMedia === 'function')
  ? window.matchMedia('(max-width: 720px)') : null;
function isMobileViewport(): boolean {
  return !!_mobileVoiceHintMql?.matches;
}
function shouldShowMobileVoiceHintPlaceholder(): boolean {
  if (!isMobileViewport()) return false;
  try { return localStorage.getItem(STORAGE_MOBILE_VOICE_HINT_SHOWN_KEY) !== '1'; }
  catch (_) { return false; }
}

// 実行中状態に応じて入力欄プレースホルダと送信ボタンの見た目／挙動を再評価する。
// WS の state 更新・セッション切替・i18n 適用後にこれを呼ぶことで停止導線を同期する。
export function updateInputAffordance() {
  const running = isActiveSessionRunning();
  // C1: 実行中は「Esc で停止」、それ以外は通常文言。data-i18n-placeholder の自動適用と
  // 競合しないよう、running 状態を見て JS から明示的に上書きする。
  // B1a: 実行中でなく・スマホ幅で・初回ヒント未表示なら、音声入力ヒントを出す（Q12 決定）。
  const stopViaCtrlC = stopKeyForActiveSession() === '\x03';
  if (running) {
    inputEl.placeholder = t(stopViaCtrlC ? 'input_placeholder_running_ctrlc' : 'input_placeholder_running');
  } else if (shouldShowMobileVoiceHintPlaceholder()) {
    inputEl.placeholder = t('mobile_voice_hint_placeholder');
  } else {
    inputEl.placeholder = t('input_placeholder');
  }
  // C2: 実行中でも入力欄にテキスト/チップ/ファイルがあれば ➤（送信）のまま。
  // 入力が空の場合のみ ■（停止）に切替える。ペースト・ファイル添付直後に送信できずもっさりする問題を解消。
  const hasContent = inputEl.value.length > 0 || pastedTexts.length > 0 || pendingAttachFiles.length > 0;
  const showStop = running && !hasContent;
  const sendBtn = document.getElementById('send-btn');
  if (sendBtn) {
    sendBtn.textContent = showStop ? '■' : '➤';
    sendBtn.classList.toggle('is-stopping', showStop);
    const title = showStop ? t(stopViaCtrlC ? 'stop_btn_title_ctrlc' : 'stop_btn_title') : t('send_btn_title');
    sendBtn.title = title;
    sendBtn.setAttribute('aria-label', title);
  }
}

export function renderPasteChips() {
  if (!pasteChipsEl) return;
  pasteChipsEl.innerHTML = '';
  pastedTexts.forEach((pt, idx) => {
    const chip = document.createElement('div');
    chip.className = 'paste-chip';

    const label = document.createElement('span');
    label.className = 'paste-chip-label';
    label.textContent = t('paste_chip_label', { id: pt.id, n: pt.lineCount });

    const expandBtn = document.createElement('button');
    expandBtn.className = 'paste-chip-expand';
    expandBtn.textContent = t('expand');
    expandBtn.title = t('expand_title_btn');
    expandBtn.addEventListener('click', () => expandPasteChip(idx));

    const removeBtn = document.createElement('button');
    removeBtn.className = 'paste-chip-remove';
    removeBtn.textContent = t('remove');
    removeBtn.title = t('remove_paste');
    removeBtn.addEventListener('click', () => removePasteChip(idx));

    chip.appendChild(label);
    chip.appendChild(expandBtn);
    chip.appendChild(removeBtn);
    pasteChipsEl.appendChild(chip);
  });
}

export function stagePastedText(text, opts: any = {}) {
  const cleaned = String(text || '');
  if (!cleaned) return false;
  const lines = cleaned.split('\n');
  if (!opts.force && lines.length <= 4 && cleaned.length <= 300) return false;
  if (pastedTexts.length >= 3) pastedTexts.shift();
  set_pasteCounter(pasteCounter + 1);
  pastedTexts.push({ id: pasteCounter, text: cleaned, lineCount: lines.length });
  renderPasteChips();
  updateInputAffordance();
  inputEl.focus();
  return true;
}

export function expandPasteChip(idx) {
  const pt = pastedTexts[idx];
  if (!pt) return;
  inputEl.value = pt.text + (inputEl.value ? '\n' + inputEl.value : '');
  pastedTexts.splice(idx, 1);
  renderPasteChips();
  autoExpand({ suppressPtyResize: true });
  inputEl.focus();
}

export function removePasteChip(idx) {
  pastedTexts.splice(idx, 1);
  renderPasteChips();
  updateInputAffordance();
}

export function clearAllPastes() {
  pastedTexts.length = 0;
  renderPasteChips();
  updateInputAffordance();
}

export function buildSendText() {
  const parts = pastedTexts.map(pt => pt.text);
  if (inputEl.value) parts.push(inputEl.value);
  return parts.join('\n');
}

// 長文ペースト（10行以上 または 1500字以上）を一時 .txt 添付へ変換して @path 参照で送るための閾値。
// 巨大なブラケットペーストを内側 CLI へ送らなくなるため、Codex の [Pasted Content] 畳み込みと、
// それに伴う確定 \r の吸収（入力欄にテキストが張り付いたまま送信されない不具合）が原理的に発生しない。
// タイミングを推測して確定 \r を撃つ（オープンループ）のではなく、そもそも畳み込みを起こさない経路へ
// 変える根本対処。閾値未満のチップは従来どおり本文へ結合する（短い貼り付けまで参照化しない）。
const PASTE_FILE_MIN_LINES = 10;
const PASTE_FILE_MIN_CHARS = 1500;

// 長文ペーストチップを一時テキストファイル化し、画像添付と同じ pendingAttachFiles 経路へ積む。
// flushPendingAttach が provider 別の inject（codex/claude は `@絶対パス `）へ変換するため、内側 CLI へ
// 渡るのは短い 1 行の @path だけになり、畳み込みも確定 \r 吸収も起きない。確定は通常の \r で済む。
// 閾値未満のチップ・入力欄テキストは pastedTexts に残し、本文として従来経路で送る（混在も自然）。
export async function stageLongPastesAsFiles() {
  for (let i = pastedTexts.length - 1; i >= 0; i--) {
    const pt = pastedTexts[i];
    if (pt.lineCount < PASTE_FILE_MIN_LINES && pt.text.length < PASTE_FILE_MIN_CHARS) continue;
    const bytes = new TextEncoder().encode(pt.text);
    // 8MB 超は添付保存に失敗してテキストを失う恐れがあるため、ファイル化せず従来のブラケットペースト経路に残す。
    if (bytes.byteLength > MAX_ATTACH_BYTES) continue;
    pendingAttachFiles.push({ buf: bytes.buffer, filename: `paste-${pt.id}.txt`, entry: {}, wrapper: null });
    pastedTexts.splice(i, 1);
  }
  renderPasteChips();
  updateInputAffordance();
}

// Ollama route で起動したセッションでは Claude Code / Codex 側の /model コマンドが
// spawn 時固定の env (ANTHROPIC_BASE_URL=http://localhost:11434 等) と整合しないため
// 純正モデルに切替えるとエラーになる。行頭 /model 入力は送信前にブロックする。
export function isOllamaModelCommandBlocked(sessionId, text) {
  const s = sessions.get(sessionId);
  if (!s || (s.route !== 'ollama' && s.route !== 'lm-studio')) return false;
  const trimmed = String(text || '').replace(/^[\s\x00-\x1f]+/, '');
  return /^\/model(\b|\s|$)/i.test(trimmed);
}

// shell（素のシェル）セッション内で AI CLI（claude/codex/copilot/cursor-agent/grok）の
// 起動コマンドを直接打つと、provider=shell 用にチューニングされた入力・承認処理
// （\x15 前置なし・マーカー未注入・shell 用承認検出）と二重ラップになり、スラッシュ
// コマンドの文字化けや承認ボタンの不動作を招く。先頭トークンが起動コマンドのときは
// 検知して provider 名を返す（パス前置・.cmd/.exe 等の拡張子も許容）。該当なしは null。
const AI_CLI_LAUNCH_RE = /^(?:[^\s]*[\\/])?(claude|codex|copilot|cursor-agent|grok)(?:\.(?:cmd|exe|bat|ps1))?(?=\s|$)/i;
// 「このまま続行」を選んだ shell セッションでは以後ナグを出さない（セッション単位で抑止）。
const aiCliLaunchNudgeSuppressed = new Set();

export function detectAiCliLaunchInShell(sessionId, text) {
  const s = sessions.get(sessionId);
  if (!s || !isShellProvider(s.provider || '')) return null;
  if (aiCliLaunchNudgeSuppressed.has(sessionId)) return null;
  const trimmed = String(text || '').replace(/^[\s\x00-\x1f]+/, '');
  const m = AI_CLI_LAUNCH_RE.exec(trimmed);
  return m ? m[1].toLowerCase() : null;
}

// 検知時に専用セッション spawn を促す。ブロックはしない：
//   「専用セッションを開く」→ spawn パネルを provider/cwd プリセットで開き、true を返す
//                            （呼び出し側は起動コマンドを shell へ送らない＝二重起動を防ぐ）
//   「このまま続行」          → false を返し（以後そのセッションでは抑止）、通常送信させる
async function maybeNudgeAiCliLaunchInShell(sessionId, text) {
  const provider = detectAiCliLaunchInShell(sessionId, text);
  if (!provider) return false;
  const ok = await appConfirm({
    title: t('shell_ai_launch_title'),
    message: t('shell_ai_launch_msg', { provider }),
    confirmText: t('shell_ai_launch_open', { provider }),
    cancelText: t('shell_ai_launch_continue'),
  });
  if (ok) {
    const cwd = sessions.get(sessionId)?.cwd || '';
    (window as any).openSpawnFor?.(provider, cwd);
    return true;
  }
  aiCliLaunchNudgeSuppressed.add(sessionId);
  return false;
}

export function clearInput() {
  inputEl.value = '';
  inputEl.style.height = 'auto';
  updateInputClearButton();
  clearAllPastes();
}

// doSend の session 単位 in-flight ロック（Enter / 音声 / autosend 再入防止）
const doSendInFlight = new Set<number>();

type DeferredSendState = 'queued' | 'sending' | 'injected' | 'acked';
let deferredEnterOverrideMs = 0;
let deferredSendSessionId: number | null = null;
let deferredSendHideTimer: ReturnType<typeof setTimeout> | null = null;
let sendFailureToastAt = 0;

function notifySendFailure(): void {
  // 切断中にキー操作が連続しても、トーストを毎回積み重ねない。
  if (Date.now() - sendFailureToastAt < 1500) return;
  sendFailureToastAt = Date.now();
  showToast(t('toast_input_send_failed'), undefined, 4500);
}

function isWebSocketSendReady(): boolean {
  return !!ws && ws.readyState === WebSocket.OPEN;
}

function setDeferredSendStatus(sessionId: number, state: DeferredSendState): void {
  deferredSendSessionId = sessionId;
  if (deferredSendHideTimer) clearTimeout(deferredSendHideTimer);
  const el = document.getElementById('deferred-send-status');
  const label = el?.querySelector('.deferred-send-label');
  if (!el || !label) return;
  const labels: Record<DeferredSendState, string> = {
    queued: '送信を準備中', sending: '入力欄の安定を待機中', injected: '確定 Enter を送信', acked: '送信しました',
  };
  el.hidden = false;
  el.dataset.state = state;
  label.textContent = labels[state];
  if (state === 'acked') {
    deferredSendHideTimer = setTimeout(() => { if (deferredSendSessionId === sessionId) el.hidden = true; }, 1100);
  }
}

function clearDeferredSendStatus(sessionId?: number): void {
  if (sessionId != null && deferredSendSessionId !== sessionId) return;
  if (deferredSendHideTimer) clearTimeout(deferredSendHideTimer);
  deferredSendHideTimer = null;
  const el = document.getElementById('deferred-send-status');
  if (el) el.hidden = true;
  deferredSendSessionId = null;
}

function effectiveDeferredEnterWait(sessionId: number): number {
  return deferredEnterOverrideMs > 0 ? deferredEnterOverrideMs : deferredEnterMinWaitFor(sessions.get(sessionId)?.provider || '');
}

// スラッシュコマンド 1 行の末尾空白を落とす。候補メニューの確定は引数を続けて打てるよう
// `cmd + ' '` を入力欄へ入れる（selectSlashItem）が、この末尾空白が付いたまま送ると
// OpenCode の TUI はコマンド候補を開かない。候補を開く条件が「先頭が / で、かつカーソル
// までに空白が 1 つも無いこと」のため（opencode 1.18.18 実機バイナリで確認）。候補が開かない
// と確定 Enter が通常送信に落ち、`/models` がコマンドではなく AI へのプロンプトとして飛ぶ。
// 末尾空白に意味を持つコマンドは wrap 対象 6 CLI に無いため provider を問わず落とす。
function trimTrailingSpaceForSlashCommand(text) {
  if (!text.startsWith('/') || text.includes('\n')) return text;
  return text.replace(/[ \t]+$/, '');
}

// 本文送信の共通判定: ブラケットペーストで包むか（deferEnter=確定 \r を出力静止後に別送するか）。
// 「本文+\r」を同一チャンクで送ると、内側 CLI がチャンク一括入力をペーストとして取り込む過程で
// 末尾 \r を確定キーと扱わず、入力欄に張り付いたまま送信されない（Grok 実測 2026-07-11:
// 全文エコー後に Enter 不発・8 分沈黙）。wrap 対象 6 CLI すべてが ?2004h（ブラケットペースト
// 受け入れ）を宣言していることをログで確認済みのため、本文はペーストで包んで正体を正しく伝える。
// 生送信（本文+\r 同梱）のまま残す例外は「キー入力として届くべきもの」だけ:
// - shell: 素の PowerShell / cmd 等はブラケットペースト未宣言がありうる（マーカーがリテラル混入する）
// - "/" 始まりの単一行: スラッシュコマンド（CLI のコマンドメニュー操作）
// - 数字のみ / 1 文字: 承認メニュー等へのホットキー応答。ペーストで包むとメニューが受理しない
//   恐れがあり、かつ数バイトのチャンクはタイプと区別されないため生送信で安全
function buildBodySubmitPart(sessionId, rawText) {
  if (rawText.includes('\n')) {
    // 複数行は provider を問わず従来どおりペースト包み（\n の途中 Enter 解釈を防ぐ）
    return { textPart: '\x1b[200~' + rawText + '\x1b[201~', deferEnter: true };
  }
  const provider = sessions.get(sessionId)?.provider || '';
  const keyLike = rawText === '' || rawText.length === 1 || /^[0-9]+$/.test(rawText) || rawText.startsWith('/');
  if (isShellProvider(provider) || keyLike) {
    return { textPart: rawText + '\r', deferEnter: false };
  }
  return { textPart: '\x1b[200~' + rawText + '\x1b[201~', deferEnter: true };
}

// 「本文」を 1 件送信する共通経路（チャット自由入力以外＝クイックコマンド・承認 UI の
// 自由回答/複数質問回答から使う）。buildBodySubmitPart の判定でペースト包み＋確定 \r
// 別送（deferred-enter）にし、キー入力相当は従来どおり本文+\r で送る。
// 直前の deferred-enter 予約が残っていると遅延 \r がこの送信の後ろに混ざるため先に消す。
export function sendSubmittedBody(sessionId, bodyText, opts: any = {}) {
  cancelDeferredEnter(sessionId, 'send_body');
  const body = trimTrailingSpaceForSlashCommand(bodyText);
  const { textPart, deferEnter } = buildBodySubmitPart(sessionId, body);
  if (!sendSubmittedText(sessionId, textPart, opts)) return false;
  if (deferEnter) scheduleDeferredEnter(sessionId, effectiveDeferredEnterWait(sessionId));
  scheduleResidueSweep(sessionId, body, deferEnter);
  return true;
}

export async function doSend(sessionId) {
  // 直前の doSend（Enter/音声/ボタン）と async 中の再入を抑止
  if (Date.now() - lastDoSendAt < DOUBLE_SEND_GUARD_MS) return;
  if (doSendInFlight.has(sessionId)) return;
  // 後続の単行送信が deferred-enter 予約をキャンセルしないと、遅延 \r / ペースト本体が
  // 次メッセージの後ろに注入される。送信確定の直前に必ず消す。
  cancelDeferredEnter(sessionId, 'do_send');
  clearDeferredSendStatus(sessionId);
  // Ollama route セッションで /model 始まりはブロック（spawn 時固定 env と不整合のため）
  if (isOllamaModelCommandBlocked(sessionId, buildSendText())) {
    showToast(t('toast_model_blocked_on_ollama'));
    return;
  }
  // 選択メニュー（claude /model 等・承認ではないカーソル駆動 TUI）表示中は、
  // チャット入力欄からの素通し注入（末尾 \r）を保留する。注入すると \r が
  // 現在カーソル選択中の項目を誤確定してしまうため。入力テキストは消さず、
  // ユーザーは下の action-bar ボタンか端末ペインで選択を解決する。
  if (isSelectMenuActive(sessionId)) {
    showToast(t('toast_select_menu_active'));
    return;
  }
  // 非接続時は長文ペーストを添付へ移したり、入力 state を変更したりする前に止める。
  // 再接続後に同じ入力をそのまま再送できるよう、送信前の state を保つ。
  if (!isWebSocketSendReady()) {
    notifySendFailure();
    return;
  }
  // shell セッション内で AI CLI 起動コマンドを検知 → 専用セッション spawn を誘導。
  // 「開く」を選べば起動コマンドは shell へ送らず spawn パネルへ切り替える（二重起動防止）。
  // flushPendingAttach より前に判定し、誘導採択時に画像 inject を無駄に消費しないようにする。
  if (await maybeNudgeAiCliLaunchInShell(sessionId, buildSendText())) {
    return;
  }
  doSendInFlight.add(sessionId);
  set_lastDoSendAt(Date.now());
  try {
    // 長文ペーストチップを一時 .txt 化して @path 参照で送る（Codex の畳み込みによる確定 \r 吸収を回避）。
    // flushPendingAttach より前に積むことで、画像添付と同一の inject 経路へ合流させる。
    await stageLongPastesAsFiles();
    const injects = await flushPendingAttach(sessionId);
    const injectPrefix = injects.join('');
    let rawText = buildSendText();
    // トリガーフレーズを末尾から除去（PTY・AI には送らない）
    const _tp = getActiveTriggerPhrase();
    if (_tp && textEndsWithTriggerPhrase(rawText, _tp)) {
      rawText = stripTrailingTriggerPhrase(rawText, _tp);
    }
    // 外部クリップボードからのペーストは CRLF(\r\n) を保持する。ブラケットペースト
    // 本体に生の \r が残ると、内側 CLI（Claude Code 等）が paste 内の \r を確定キーと
    // 誤解し、末尾に付与した本来の確定 \r が無効化されて入力欄に残る（pasted text が
    // 実行されない）。確定はこちらが末尾に付ける \r のみが担うべきなので、本文中の CR は
    // すべて LF に正規化する。入力欄で打った複数行は元々 \n のみなので影響を受けない。
    rawText = rawText.replace(/\r\n?/g, '\n');
    // 候補メニュー確定で付く末尾空白を落とす（付いたままだと内側 CLI がコマンドとして扱わない）。
    rawText = trimTrailingSpaceForSlashCommand(rawText);
    // ペースト包み・確定 \r 分離の判定は buildBodySubmitPart に共通化（クイックコマンドと共用）。
    // ブラケットペーストはテキスト部分のみに適用し、injectPrefix は前置する。
    let textPart;
    let deferEnter = false;
    if (rawText === '' && injectPrefix !== '') {
      // 画像のみ（テキストなし）: inject 末尾の \r or スペースで確定済み → 追加の \r で送信
      textPart = '\r';
    } else {
      ({ textPart, deferEnter } = buildBodySubmitPart(sessionId, rawText));
    }
    // 残骸（前回の未確定入力）への連結対策は、かつての「全送信への \x15(Ctrl+U) 盲目前置」を
    // やめ、送信後に張り付きを実測してから掃除する residue-sweep.ts へ移行した。
    // 前置方式は「相手プロンプトが Ctrl+U を行クリアと解釈する」仮定に依存し、shell / codex に
    // 加えて claude /login のコード入力欄でもリテラル ^U 混入（OAuth 400）を起こしたため。
    // 画像 inject（@path）を複数行ペーストに前置すると、@path エコー由来の早期 idle で確定 \r が
    // 前倒し発火し、内側 CLI がペースト取り込み中（「Pasting…」）のまま \r を吸収して固着する。
    // injectPrefix がある複数行ペーストでは、まず画像 inject だけ送り、取り込みが落ち着いてから
    // ペースト本体＋確定 \r を送る（下記 needPasteSplit 分岐）。それ以外は従来通り 1 書き込みにまとめる。
    const needPasteSplit = deferEnter && injectPrefix !== '';
    const textToSend = needPasteSplit ? injectPrefix : (injectPrefix + textPart);
    // 入力欄・貼り付けチップの消去は、PTY への最初の書き込みが成功してから行う。
    // sendText は接続断や send() の例外を false で返すため、失敗時は doSend の
    // 後続状態更新にも入らず、ユーザーが同じ内容を再送できる。
    if (!sendSubmittedText(sessionId, textToSend)) return false;
    clearInput();
    hideSlashMenu();
    // 送信したら次のプロンプトは別物の可能性があるため dismiss フラグ・multiQ ラッチをクリア
    multiQuestionDismissedCache.delete(sessionId);
    multiQuestionLatchAt.delete(sessionId);
    // テキスト送信で承認ポップアップをバイパスした場合、Ink 再描画による
    // 同一選択肢の再検出・再表示を防ぐため消費済み署名を保存する
    const prevOpts = approvalRawOptionsCache.get(sessionId);
    if (prevOpts) approvalConsumedSig.set(sessionId, approvalSig(prevOpts));
    recordAnsweredMarkerSig(sessionId, prevOpts);
    if (typeof maybeSendDirectApprovalConsumed === 'function') {
      maybeSendDirectApprovalConsumed(sessionId, rawText, textToSend);
    }
    hideActionBar(sessionId);
    // PTY エコーバックによる誤再表示を抑制（sendChoice と同様）
    approvalSuppressUntil.set(sessionId, Date.now() + 2000);
    setTimeout(() => {
      detectApproval(sessionId);
      maybeAutoSwitchToNextApproval();
    }, 2050);
    // chatHistory: ユーザー送信は AI ターンの境界。
    // まず蓄積中の AI 出力チャンクを即 commit してから user 入力を push する。
    chatHistoryCommitOutput(sessionId);
    if (rawText && rawText !== '' && !isTranscriptBackedSession(sessionId)) {
      pushMessage(sessionId, { role: 'user', kind: 'text', rawText });
    }
    if (sessionId === activeSessionId) {
      // 送信後は新しいターンを見る意図なので、chat/split 側もレイアウト確定まで末尾へ張り付かせる。
      scrollChatPaneToBottomSoon({ passes: 4, startedAt: Date.now() });
    }
    if (deferEnter) setDeferredSendStatus(sessionId, 'queued');
    if (deferEnter) setDeferredSendStatus(sessionId, 'sending');
    // B1a: スマホ幅で音声入力 placeholder を 1 回でも見せたユーザーが送信を完了した時点で
    // hint shown フラグを立て、以降は通常 placeholder に戻す（一度の認知で十分）。
    if (shouldShowMobileVoiceHintPlaceholder()) {
      setUserPref('mobile.voice_hint_shown', true);
      updateInputAffordance();
    }
    // Codex/OpenCode は大きいペーストを無出力で即プレースホルダ化するため、確定 \r の最低待機を
    // 長めに取り早撃ち（\r 吸収による送信不発）を防ぐ。他 provider は既定値で挙動不変。
    const enterMinWait = effectiveDeferredEnterWait(sessionId);
    if (needPasteSplit) {
      // 段1: 画像 inject（@path）の取り込み（[Image #N] 畳み込み・@ 補完ポップアップ閉じ）が
      // 出力静止で落ち着くのを待ち、段2: ペースト本体を送り、段3: 確定 \r を予約する。確定 \r の
      // 予約をペースト送出後まで遅らせることで、@path エコー由来の早期静止で \r が前倒し発火して
      // 「Pasting…」固着するのを断つ。ペースト送出以降は画像なし複数行ペーストと同一経路で確定する。
      scheduleAfterOutputSettle(sessionId, () => {
        if (!sendText(sessionId, textPart)) return false;
        scheduleDeferredEnter(sessionId, enterMinWait,
          () => setDeferredSendStatus(sessionId, 'injected'),
          () => setDeferredSendStatus(sessionId, 'acked'));
        return true;
      });
    } else if (deferEnter) {
      // 複数行ペーストの確定 \r は、内側 CLI の畳み込み・再描画が落ち着いてから別書き込みで送る。
      // 同一書き込みに含めると \r が吸収され送信されない。固定遅延では大きなペーストの取り込み時間を
      // 当てられず取りこぼすため、PTY 出力が静止するのを待ってから 1 回だけ送る（deferred-enter.ts）。
      scheduleDeferredEnter(sessionId, enterMinWait,
        () => setDeferredSendStatus(sessionId, 'injected'),
        () => setDeferredSendStatus(sessionId, 'acked'));
    }
    // 送信テキストが確定されず入力行に張り付いたままになっていないかを事後観測し、
    // 張り付きを実際に見たときだけ \x15 で掃除する（盲目前置の置き換え。residue-sweep.ts）。
    scheduleResidueSweep(sessionId, rawText, deferEnter);
  } finally {
    doSendInFlight.delete(sessionId);
  }
}

export function saveInputStateFor(id) {
  if (id === null) return;
  // サムネイル DOM は pendingAttachFiles 各エントリの wrapper 参照から復元できるため、
  // ここではコンテナから切り離すだけでよい（DocumentFragment 退避は復元が1回しか
  // 効かず、activateSession の再実行でサムネイルだけ消える不具合があった）。
  if (attachThumbnails) attachThumbnails.replaceChildren();
  sessionInputState.set(id, {
    inputValue: inputEl.value,
    pastedTextsData: [...pastedTexts],
    pendingAttachFiles: [...pendingAttachFiles],
  });
  inputEl.value = '';
  inputEl.style.height = 'auto';
  updateInputClearButton();
  pastedTexts.length = 0;
  pendingAttachFiles.length = 0;
}

export function restoreInputStateFor(id) {
  const state = sessionInputState.get(id);
  if (state) {
    inputEl.value = state.inputValue;
    pastedTexts.length = 0;
    pastedTexts.push(...state.pastedTextsData);
    pendingAttachFiles.length = 0;
    pendingAttachFiles.push(...state.pendingAttachFiles);
    if (attachThumbnails) {
      // renderPasteChips と同じく毎回再構築する冪等な復元。
      // wrapper は stage 時の prepend 順（新しい順）に戻すため配列順に prepend する。
      attachThumbnails.replaceChildren();
      for (const p of pendingAttachFiles) {
        if (p.wrapper) attachThumbnails.prepend(p.wrapper);
      }
    }
  } else {
    inputEl.value = '';
    inputEl.style.height = 'auto';
    pastedTexts.length = 0;
    pendingAttachFiles.length = 0;
    if (attachThumbnails) attachThumbnails.replaceChildren();
  }
  autoExpand();
  renderPasteChips();
  updateAttachClearBtn();
}

export function cleanupSessionInputState(id) {
  const state = sessionInputState.get(id);
  if (!state) return;
  for (const p of state.pendingAttachFiles ?? []) {
    const img = p.wrapper?.querySelector('img');
    if (img && img.src.startsWith('blob:')) URL.revokeObjectURL(img.src);
  }
  sessionInputState.delete(id);
}

export const specialKeys = {
  'ArrowUp':    '\x1b[A',
  'ArrowDown':  '\x1b[B',
  'ArrowRight': '\x1b[C',
  'ArrowLeft':  '\x1b[D',
  'Escape':     '\x1b',
};

// /config・/model など「矢印で選ぶ」タイプの Claude 側メニューが、今まさに画面に
// 出ているかを端末ビューポート末尾のヒント行から検出する。
// これらのメニュー（Ink インライン描画で alternate-screen を使わず、承認 UI としても
// 検出されない）が出ている間は、入力欄に打ちかけの文字が残っていても ↑↓←→ を
// ブラウザのカーソル移動へ横取りせず PTY へ転送する必要がある（下の specialKeys 処理で
// 矢印送出時に入力欄がクリアされるため、最初の 1 押しで残り文字も消え以後は通常経路で通る）。
// ヒント例: "Type to filter · Enter/↓ to select · ↑ to tabs · Esc to clear" /
//           "Enter/Space to change · / to search · Esc to close"
const cursorMenuHintRe = /\besc to (?:close|clear|cancel|exit)\b|(?:↑|↓|←|→)\s*to\s*(?:tabs|select|change|search)|\btype to filter\b/i;
function activeSessionShowsCursorMenu() {
  if (activeSessionId === null) return false;
  const entry = terminals.get(activeSessionId);
  const term = entry && entry.term;
  const buf = term && term.buffer && term.buffer.active;
  if (!buf) return false;
  const rows = term.rows || 24;
  const top = buf.baseY;
  for (let y = top + rows - 1; y >= top; y--) {
    const line = buf.getLine(y);
    if (!line) continue;
    const text = line.translateToString(true);
    if (text && cursorMenuHintRe.test(text)) return true;
  }
  return false;
}

// ---- スラッシュコマンドメニュー ----

// /入力補完のフォールバック（ソース未設定・取得失敗時のみ使う最小セット）。
// 通常はピッカーと同じ /api/slash-commands の英語フルリストを使う（slashCmdDynamic）。
function getSlashCommandsFallback() {
  return [
    { cmd: '/clear',    desc: t('slash_clear') },
    { cmd: '/compact',  desc: t('slash_compact') },
    { cmd: '/config',   desc: t('slash_config') },
    { cmd: '/cost',     desc: t('slash_cost') },
    { cmd: '/doctor',   desc: t('slash_doctor') },
    { cmd: '/help',     desc: t('slash_help') },
    { cmd: '/init',     desc: t('slash_init') },
    { cmd: '/login',    desc: t('slash_login') },
    { cmd: '/logout',   desc: t('slash_logout') },
    { cmd: '/model',    desc: t('slash_model') },
    { cmd: '/review',   desc: t('slash_review') },
    { cmd: '/resume',   desc: t('slash_resume') },
    { cmd: '/status',   desc: t('slash_status') },
    { cmd: '/usage',    desc: t('slash_usage') },
    { cmd: '/vim',      desc: t('slash_vim') },
  ];
}

// provider 単位の動的スラッシュコマンドキャッシュ。スラッシュピッカー（/ ▾）と
// /api/slash-commands を共有し、取得済みなら /入力補完にも英語フルリストを出す。
export const slashCmdDynamic = new Map(); // cache key -> [{cmd, desc}]
const slashCmdRetryAfter = new Map();     // cache key -> epoch ms（失敗時の再試行抑止）
const slashCmdLoading = new Set();        // 取得中の cache key

function activeProvider() {
  return sessions.get(activeSessionId)?.provider || 'claude';
}

function slashCmdCacheKey(provider, sessionId = activeSessionId) {
  return `${provider}#${sessionId || 0}`;
}

// ピッカー／/入力補完の双方から呼べるキャッシュ充填。
export function setSlashCmdCache(provider, cmds, sessionId = activeSessionId) {
  const list = (cmds || []).filter(c => c && c.cmd);
  if (list.length > 0) {
    const key = slashCmdCacheKey(provider, sessionId);
    slashCmdDynamic.set(key, list);
    slashCmdRetryAfter.delete(key);
  }
}

// /入力補完用にフルリストを遅延取得する。取得済み・取得中・抑止中は何もしない。
// 取得完了時、メニューが開いていれば再描画する。
async function ensureSlashCommands(provider, sessionId = activeSessionId) {
  const key = slashCmdCacheKey(provider, sessionId);
  if (slashCmdDynamic.has(key) || slashCmdLoading.has(key)) return;
  const retryAt = slashCmdRetryAfter.get(key) || 0;
  if (Date.now() < retryAt) return;
  slashCmdLoading.add(key);
  try {
    const sidParam = sessionId ? `&session_id=${encodeURIComponent(sessionId)}` : '';
    const resp = await fetch(`/api/slash-commands?provider=${provider}${sidParam}&token=${token}`);
    if (!resp.ok) { slashCmdRetryAfter.set(key, Date.now() + 60_000); return; }
    const data = await resp.json();
    setSlashCmdCache(provider, data.cmds, sessionId);
    if (slashCmdDynamic.has(key) && !slashMenuEl.hidden) updateSlashMenu();
  } catch (_) {
    slashCmdRetryAfter.set(key, Date.now() + 60_000);
  } finally {
    slashCmdLoading.delete(key);
  }
}

export function getSlashCommands() {
  const dyn = slashCmdDynamic.get(slashCmdCacheKey(activeProvider()));
  if (dyn && dyn.length > 0) return dyn;
  return getSlashCommandsFallback();
}

export const slashMenuEl = document.getElementById('slash-menu');
export let slashItems = [];
export let slashIndex = -1;

export function updateSlashMenu() {
  const val = inputEl.value;
  if (!val.startsWith('/') && !val.startsWith('$')) { hideSlashMenu(); return; }
  ensureSlashCommands(activeProvider(), activeSessionId); // 非同期: 取得完了時に自動で再描画
  const filtered = getSlashCommands().filter(c => c.cmd.startsWith(val));
  if (filtered.length === 0) { hideSlashMenu(); return; }
  slashItems = filtered;
  if (slashIndex >= slashItems.length) slashIndex = 0;
  if (slashIndex < 0) slashIndex = 0;
  renderSlashMenu();
}

export function renderSlashMenu() {
  slashMenuEl.innerHTML = '';
  slashItems.forEach((item, i) => {
    const div = document.createElement('div');
    div.className = 'slash-item' + (i === slashIndex ? ' selected' : '');
    const cmdSpan = document.createElement('span');
    cmdSpan.className = 'slash-cmd';
    cmdSpan.textContent = item.cmd;
    const descSpan = document.createElement('span');
    descSpan.className = 'slash-desc';
    descSpan.textContent = item.desc;
    div.appendChild(cmdSpan);
    div.appendChild(descSpan);
    div.addEventListener('mousedown', (e) => { e.preventDefault(); selectSlashItem(i); });
    slashMenuEl.appendChild(div);
  });
  slashMenuEl.hidden = false;
  scrollSlashIntoView();
}

export function hideSlashMenu() {
  slashMenuEl.hidden = true;
  slashItems = [];
  slashIndex = -1;
}

export function selectSlashItem(i) {
  if (i < 0 || i >= slashItems.length) return;
  const cmd = slashItems[i].cmd;
  // 引数を取らずダイアログを開くだけのコマンドはクイック実行用途のため、候補選択時に即送信する。
  // /models は OpenCode 側の /model 相当（TUI コマンド名 model.list）。ここに無いと Enter を
  // 2 回押す形になり、Claude の /model と操作が揃わない。
  if (activeSessionId !== null && (cmd === '/clear' || cmd === '/model' || cmd === '/models')) {
    sendQuickCommand(activeSessionId, cmd);
    clearInput();
    hideSlashMenu();
    inputEl.focus();
    return;
  }
  inputEl.value = cmd + ' ';
  hideSlashMenu();
  autoExpand();
  inputEl.focus();
}

export function scrollSlashIntoView() {
  const items = slashMenuEl.querySelectorAll('.slash-item');
  if (items[slashIndex]) items[slashIndex].scrollIntoView({ block: 'nearest' });
}

inputEl.addEventListener('input', () => {
  autoExpand({ suppressPtyResize: true }); updateSlashMenu();
  updateInputClearButton();
  updateInputAffordance();
  if (!isComposing) {
    const _tp = getActiveTriggerPhrase();
    if (_tp && activeSessionId !== null && textEndsWithTriggerPhrase(buildSendText(), _tp)) {
      doSend(activeSessionId);
    }
  }
});
inputEl.addEventListener('blur', () => setTimeout(hideSlashMenu, 150));
inputEl.addEventListener('compositionstart', () => { set_isComposing(true); });
inputEl.addEventListener('compositionend', () => {
  set_isComposing(false);
  // 自動送信トリガー: input イベントは IME 環境/ブラウザによって compositionend より
  // 前または最中にしか発火せず、isComposing=true で autosend がスキップされてしまう。
  // ここで末尾チェックして発火させる。input ハンドラ側でもチェックされるが、
  // doSend 後は inputEl.value='' になるので二重送信しない。
  const _tp = getActiveTriggerPhrase();
  if (_tp && activeSessionId !== null && textEndsWithTriggerPhrase(buildSendText(), _tp)) {
    set_pendingSend(false);
    if (composeEndSendTimer !== null) {
      clearTimeout(composeEndSendTimer);
      set_composeEndSendTimer(null);
    }
    doSend(activeSessionId);
    return;
  }
  if (pendingSend) {
    set_pendingSend(false);
    set_composeEndSendTimer(setTimeout(() => {
      set_composeEndSendTimer(null);
      if (activeSessionId === null) return;
      doSend(activeSessionId);
    }, 0));
  }
});

inputEl.addEventListener('keydown', (e) => {
  // スラッシュメニューが開いているときはメニュー操作を優先
  if (!slashMenuEl.hidden && slashItems.length > 0) {
    if (e.key === 'ArrowUp') {
      slashIndex = (slashIndex - 1 + slashItems.length) % slashItems.length;
      renderSlashMenu();
      e.preventDefault(); return;
    }
    if (e.key === 'ArrowDown') {
      slashIndex = (slashIndex + 1) % slashItems.length;
      renderSlashMenu();
      e.preventDefault(); return;
    }
    if (e.key === 'Enter' || e.key === 'Tab') {
      selectSlashItem(slashIndex);
      e.preventDefault(); return;
    }
    if (e.key === 'Escape') {
      hideSlashMenu();
      e.preventDefault(); return;
    }
  }

  if (activeSessionId === null) return;

  // バッチ承認モード（複数質問の一括回答）の専用キー処理。
  // 入力が空のときのみ作動し、通常の文字入力・IME と競合しないようにする。
  // 送信確認モーダル表示中は action-bar の専用キー処理を行わない（モーダル側で操作する）。
  if (inputEl.value === '' && !e.isComposing && isBatchActionBarVisible() && !document.getElementById('action-confirm-mask')) {
    if (e.key === 'Tab' && slashMenuEl.hidden) {
      moveBatchFocus(e.shiftKey ? -1 : 1);
      e.preventDefault(); return;
    }
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      moveBatchFocus(e.key === 'ArrowRight' ? 1 : -1);
      e.preventDefault(); return;
    }
    if (e.key === ' ') {
      moveBatchFocus(1);
      e.preventDefault(); return;
    }
    if (/^[0-9]$/.test(e.key) && !e.ctrlKey && !e.metaKey && !e.altKey) {
      if (handleBatchNumberKey(activeSessionId, parseInt(e.key, 10))) {
        e.preventDefault(); return;
      }
    }
    // Enter で「送信確認」モーダルを開く（即送信はしない）。全問回答済みのときだけ開く。
    if (e.key === 'Enter' && !e.shiftKey) {
      openBatchConfirm(activeSessionId);
      e.preventDefault(); return;
    }
  }

  // 複数選択（#multi）の専用キー処理。入力欄が空のときのみ作動。
  // ←→↑↓ でフォーカス移動、Space でフォーカス中の選択肢を ON/OFF、
  // 数字キーで該当選択肢をトグル、Enter でまとめて送信。
  if (inputEl.value === '' && !e.isComposing && isMultiSelectActionBarVisible()) {
    if ((e.key === 'Tab' && slashMenuEl.hidden)) {
      moveMultiSelectFocus(e.shiftKey ? -1 : 1);
      e.preventDefault(); return;
    }
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
      moveMultiSelectFocus(1);
      e.preventDefault(); return;
    }
    if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
      moveMultiSelectFocus(-1);
      e.preventDefault(); return;
    }
    if (e.key === ' ') {
      toggleMultiSelectFocused(activeSessionId);
      e.preventDefault(); return;
    }
    if (/^[0-9]$/.test(e.key) && !e.ctrlKey && !e.metaKey && !e.altKey) {
      if (handleMultiSelectNumberKey(activeSessionId, parseInt(e.key, 10))) {
        e.preventDefault(); return;
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      sendMultiSelectChoices(activeSessionId);
      e.preventDefault(); return;
    }
  }

  // OpenCode の Allow once / Allow always / Reject はネイティブ側では
  // 矢印＋Enter 操作だが、承認バーでは 1/2/3 で即決できるようにする。
  // OpenCode の承認を検出しているときだけ変換するため、通常の入力は横取りしない。
  if (inputEl.value === '' && !e.isComposing && /^[1-3]$/.test(e.key) &&
      !e.ctrlKey && !e.metaKey && !e.altKey &&
      handleOpenCodeApprovalNumberKey(activeSessionId, parseInt(e.key, 10))) {
    e.preventDefault(); return;
  }

  // 複数質問プロンプト（AskUserQuestion 等の複数選択）はバナー表示のみで
  // action-bar を出さないため、ターミナルへ直接キーを送って操作する。
  // ↑↓←→/Esc は specialKeys で転送されるが、複数選択のチェックボックス
  // トグルに必須のスペースは転送経路が無く入力欄へ空白が入るだけだった。
  // 複数質問検出中・入力欄が空・修飾なしのスペースに限り PTY へ転送する。
  if (e.key === ' ' && inputEl.value === '' && !e.isComposing &&
      !e.ctrlKey && !e.metaKey && !e.altKey &&
      multiQuestionVisibleCache.get(activeSessionId)) {
    sendText(activeSessionId, ' ');
    e.preventDefault(); return;
  }

  // Tab でセッション切り替え（スラッシュメニューが閉じているとき）
  if (e.key === 'Tab' && !e.isComposing && slashMenuEl.hidden) {
    switchSessionByTab(e.shiftKey);
    e.preventDefault(); return;
  }

  // action-bar 表示中 + 入力なし → ←→ キーでボタン間移動
  if ((e.key === 'ArrowLeft' || e.key === 'ArrowRight') && inputEl.value === '') {
    const bar = document.getElementById('action-bar');
    if (bar && bar.classList.contains('visible')) {
      const btns = getActionBarButtons();
      if (btns.length > 0) {
        if (actionBarFocusIdx < 0) set_actionBarFocusIdx(0);
        const delta = e.key === 'ArrowRight' ? 1 : -1;
        setActionBarFocus((actionBarFocusIdx + delta + btns.length) % btns.length);
        e.preventDefault(); return;
      }
    }
  }

  if (specialKeys[e.key]) {
    // 入力テキストあり + 矢印キーはブラウザのカーソル移動に委譲する。
    // ただし /config・/model 等の矢印駆動メニューが画面に出ている間は、打ちかけの
    // 文字が残っていても矢印を PTY へ転送する（そうしないとメニューのタブ切替・項目選択が
    // 一切効かなくなる。alternate-screen も承認 UI 検出も使えないためヒント行で判定する）。
    if (inputEl.value !== '' && (e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'ArrowUp' || e.key === 'ArrowDown')
        && !activeSessionShowsCursorMenu()) return;
    // 実行中 + 入力欄が空の Esc は停止操作とみなし、停止ボタン（■）と同じ
    // provider 別停止キーを送る（grok は Esc 非対応・Ctrl+C のみ有効のため）。
    const keyText = (e.key === 'Escape' && inputEl.value === '' && isActiveSessionRunning())
      ? stopKeyForActiveSession()
      : specialKeys[e.key];
    sendText(activeSessionId, keyText);
    inputEl.value = ''; // TUI 操作中の誤入力を流さないようにクリア
    updateInputClearButton();
    e.preventDefault(); return;
  }
  if (e.ctrlKey && e.key === 'c') {
    // xterm.js 選択中はクリップボードにコピーして SIGINT を送らない
    const xt = terminals.get(activeSessionId);
    if (xt?.term.hasSelection()) {
      const text = cleanCopiedText(xt.term.getSelection());
      if (text) {
        navigator.clipboard.writeText(text).catch(() => {});
        stagePastedText(text, { force: true });
      }
      e.preventDefault(); return;
    }
    // ブラウザ側の通常テキスト選択中もコピーに委譲
    if (window.getSelection()?.toString().length > 0) return;
    sendText(activeSessionId, '\x03'); e.preventDefault(); return;
  }
  if (e.ctrlKey && e.key === 'd') { sendText(activeSessionId, '\x04'); e.preventDefault(); return; }
  // ctrl+o: Claude Code の折りたたみ展開（ターミナル直接操作と同等）
  if (e.ctrlKey && e.key === 'o') { sendText(activeSessionId, '\x0f'); e.preventDefault(); return; }
  if (e.key === 'Enter') {
    if (isMobileViewport()) {
      // Mobile C2 uses button-only send. Enter stays a textarea newline here,
      // so action-bar Enter execution and IME pendingSend remain desktop-only.
      autoExpand({ suppressPtyResize: true });
      return;
    }
    if (e.isComposing || isComposing) { set_pendingSend(true); return; } // IME確定後に送信
    if (e.shiftKey) { autoExpand(); return; } // Shift+Enter: 改行
    // action-bar 表示中かつ入力が空 → フォーカス中ボタン（未指定なら先頭）を実行
    const bar = document.getElementById('action-bar');
    if (bar && bar.classList.contains('visible') && inputEl.value.trim() === '') {
      const shownAt = actionBarShownAt.get(activeSessionId) || 0;
      // /model 送信直後の Enter が action-bar 初期選択を即確定してしまう事故を防ぐ。
      if (Date.now() - shownAt < 300) { e.preventDefault(); return; }
      const btns = getActionBarButtons();
      const targetBtn = actionBarFocusIdx >= 0 ? btns[actionBarFocusIdx] : btns[0];
      if (targetBtn) { targetBtn.click(); e.preventDefault(); return; }
    }
    // compositionend が既に doSend をスケジュール済みの場合はキャンセル（二重送信防止）
    if (composeEndSendTimer !== null) {
      clearTimeout(composeEndSendTimer);
      set_composeEndSendTimer(null);
    }
    set_pendingSend(false);
    doSend(activeSessionId);
    e.preventDefault();
  }
});

// ツール群 左右切替ボタン
(function initToolsFlip() {
  const wrap = document.getElementById('input-wrap');
  const btn = document.getElementById('tools-flip-btn');
  const inputArea = document.getElementById('input-area');
  const inputTools = document.getElementById('input-tools');
  const promptTemplateWrap = document.getElementById('prompt-template-wrap');
  if (!wrap || !btn || !inputArea || !inputTools || !promptTemplateWrap) return;

  const applyToolsPosition = (isLeft) => {
    wrap.classList.toggle('tools-left', isLeft);
    const voiceBtn = document.getElementById('voice-btn');
    const sendBtn = document.getElementById('send-btn');
    if (isLeft) {
      wrap.append(btn, inputTools);
      if (voiceBtn) wrap.append(voiceBtn);
      wrap.append(promptTemplateWrap);
      if (sendBtn) wrap.append(sendBtn);
      if (inputClearBtn) wrap.append(inputClearBtn);
      wrap.append(inputArea);
    } else {
      wrap.append(inputArea);
      if (inputClearBtn) wrap.append(inputClearBtn);
      if (sendBtn) wrap.append(sendBtn);
      if (voiceBtn) wrap.append(voiceBtn);
      wrap.append(promptTemplateWrap, inputTools, btn);
    }
  };

  applyToolsPosition(localStorage.getItem(STORAGE_TOOLS_LEFT_KEY) === '1');
  btn.addEventListener('click', () => {
    const isLeft = !wrap.classList.contains('tools-left');
    applyToolsPosition(isLeft);
    localStorage.setItem(STORAGE_TOOLS_LEFT_KEY, isLeft ? '1' : '0');
  });

  // 入力欄の空白（ボタン以外＝paddingや flex 余白）をクリックしたら textarea に
  // フォーカスを移す。ネイティブでは textarea の矩形上しか focus されず、行間や
  // ボタン列左側の空きは死に領域になっていた（2026-07-02 report）。
  wrap.addEventListener('mousedown', (e) => {
    const t = e.target as HTMLElement | null;
    if (!t) return;
    if (t === inputEl) return; // textarea 自身は既定挙動
    if (t.closest('button, a, input, select, textarea, [contenteditable="true"], .paste-chip')) return;
    e.preventDefault();
    inputEl.focus();
  });
})();

(function initMobileComposer() {
  const attachBtn = document.getElementById('mobile-composer-attach-btn');
  const attachMenu = document.getElementById('mobile-composer-attach-menu');
  const fileBtn = document.getElementById('mobile-composer-file-btn');
  const cameraBtn = document.getElementById('mobile-composer-camera-btn');
  if (!attachBtn || !attachMenu) return;

  const closeAttachMenu = () => {
    attachMenu.hidden = true;
    attachBtn.setAttribute('aria-expanded', 'false');
  };
  const toggleAttachMenu = (ev?: Event) => {
    ev?.stopPropagation();
    if (!isMobileViewport()) return;
    attachMenu.hidden = !attachMenu.hidden;
    attachBtn.setAttribute('aria-expanded', attachMenu.hidden ? 'false' : 'true');
  };
  attachBtn.addEventListener('click', toggleAttachMenu);
  fileBtn?.addEventListener('click', (ev) => {
    ev.stopPropagation();
    closeAttachMenu();
    document.getElementById('attach-file-input')?.click();
  });
  cameraBtn?.addEventListener('click', (ev) => {
    ev.stopPropagation();
    closeAttachMenu();
    document.getElementById('attach-camera-input')?.click();
  });
  document.addEventListener('click', (ev) => {
    if (attachMenu.hidden) return;
    const target = ev.target as Node | null;
    if (target && (attachMenu.contains(target) || attachBtn.contains(target))) return;
    closeAttachMenu();
  });

  const syncVisualViewportOffset = () => {
    const vv = window.visualViewport;
    if (!vv || !isMobileViewport()) {
      document.documentElement.style.removeProperty('--mobile-vv-bottom-offset');
      return;
    }
    const offset = Math.max(0, window.innerHeight - vv.height - vv.offsetTop);
    document.documentElement.style.setProperty('--mobile-vv-bottom-offset', `${Math.round(offset)}px`);
  };
  if (window.visualViewport) {
    window.visualViewport.addEventListener('resize', syncVisualViewportOffset);
    window.visualViewport.addEventListener('scroll', syncVisualViewportOffset);
  }
  window.addEventListener('resize', syncVisualViewportOffset);
  syncVisualViewportOffset();
})();

inputClearBtn?.addEventListener('click', () => {
  inputEl.value = '';
  autoExpand();
  updateInputClearButton();
  if (isMobileViewport()) window.dispatchEvent(new CustomEvent('mobile-composer-idle'));
  // Web テキストエリアだけでなく内側 CLI の入力行も消す。ビジー時等に溜まった
  // 残骸（例: "/login/login..."）は web を空にしても TUI 側に残り続けるため、
  // \x15(Ctrl+U) を単独送信して行クリアする（ユーザー明示操作なのでここは前置方式のまま。
  // 送信経路の自動前置は residue-sweep.ts の事後掃除へ移行済み）。
  // ただし shell / codex は \x15 が行クリアとして機能しない（リテラル混入 or 無視）ため
  // 単独送信もスキップする。セッション未選択なら no-op。
  if (activeSessionId !== null && !shouldSkipClearPrefix(sessions.get(activeSessionId)?.provider || '')) {
    sendText(activeSessionId, '\x15');
  }
  inputEl.focus();
});

document.getElementById('send-btn').addEventListener('mousedown', () => {
  // クリック時に IME が確定中の場合、compositionend 後に送信するよう予約
  if (isComposing) set_pendingSend(true);
});
document.getElementById('send-btn').addEventListener('click', () => {
  if (activeSessionId === null) return;
  // C2: 実行中 + 入力なし → 停止（Esc）。実行中 + 入力あり → そのまま送信（Claude に割り込み）。
  const hasContent = inputEl.value.length > 0 || pastedTexts.length > 0 || pendingAttachFiles.length > 0;
  if (isActiveSessionRunning() && !hasContent) {
    sendText(activeSessionId, stopKeyForActiveSession());
    return;
  }
  if (isComposing) return; // compositionend 側で処理する
  // 直前 (DOUBLE_SEND_GUARD_MS) に doSend 済み → autosend 等の直後 click を取り込む二重送信防止
  if (Date.now() - lastDoSendAt < DOUBLE_SEND_GUARD_MS) return;
  if (composeEndSendTimer !== null) {
    // compositionend が既に doSend をスケジュール済み → タイマーキャンセルして直接実行（二重送信防止）
    clearTimeout(composeEndSendTimer);
    set_composeEndSendTimer(null);
  }
  set_pendingSend(false);
  doSend(activeSessionId);
});

for (let slot = 1; slot <= QUICK_CMD_SLOTS; slot++) {
  const btn = document.getElementById(quickCommandButtonId(slot));
  if (!btn) continue;
  btn.addEventListener('click', () => {
    if (activeSessionId === null) return;
    sendQuickCommand(activeSessionId, getQuickCommand(slot));
  });
}

export function syncMobileLayoutState() {
  const hasSession = activeSessionId !== null && sessions.size > 0;
  document.body.classList.toggle('mobile-has-session', hasSession);
  // スマホホーム画面: セッションが選択されていない場合はホームビューを表示
  const isMobile = window.matchMedia('(max-width: 720px)').matches;
  document.body.classList.toggle('mobile-home-view', isMobile && !hasSession);
  // バックボタン・ホーム画面の表示切替
  const titleBtn = document.getElementById('mobile-session-title-btn');
  const logBtn = document.getElementById('mobile-log-btn');
  const mobileHome = document.getElementById('mobile-home');
  if (titleBtn) {
    titleBtn.hidden = !(isMobile && hasSession);
    titleBtn.setAttribute('aria-expanded', String(isMobile && hasSession && document.body.classList.contains('mobile-drawer-open')));
    if (isMobile && hasSession && activeSessionId !== null) {
      const s = sessions.get(activeSessionId);
      titleBtn.textContent = s ? `#${activeSessionId} ${s.label || s.cwd?.split(/[\\/]/).filter(Boolean).pop() || s.provider || ''}` : `#${activeSessionId}`;
    }
  }
  if (logBtn) logBtn.hidden = !(isMobile && hasSession);
  if (mobileHome) mobileHome.hidden = !(isMobile && !hasSession);
  // ハンバーガーバッジ: 個別セッションビュー中に他セッションの承認待ち数を表示
  const badge = document.querySelector<HTMLElement>('#mobile-menu-btn .mobile-badge');
  if (badge) {
    if (isMobile && hasSession) {
      const pendingCount = pendingSessionIds().filter(id => id !== activeSessionId).length + mobileApprovalActiveBadgeCount();
      badge.textContent = String(pendingCount);
      badge.hidden = pendingCount === 0;
    } else {
      badge.hidden = true;
    }
  }
  if (sessions.size === 0) closeMobileSessionDrawer();
  if (isMobile) {
    // 閉じている間はフル再描画しない（タップ中に行 DOM が消える原因になっていた）。
    // 開いているときだけ更新。open 直前は openMobileSessionDrawer が force 描画する。
    if (document.body.classList.contains('mobile-drawer-open')) {
      (window as any).renderMobileSessionDrawer?.();
    }
  } else {
    // PC 幅へ戻ったらドロワーを閉じ、renderMobileSessionDrawer() が外した hidden を戻す。
    // 戻さないと PC サイドバー #session-list 内にドロワー中身が露出したまま残る。
    closeMobileSessionDrawer();
    const drawerContent = document.getElementById('mobile-drawer-content');
    if (drawerContent) drawerContent.hidden = true;
  }
}

window.addEventListener('approval-queue-updated', () => {
  syncMobileLayoutState();
});

export function openMobileSessionDrawer() {
  if (!isMobileViewport()) return;
  // force: まだ mobile-drawer-open が付く前に中身を描く
  (window as any).renderMobileSessionDrawer?.(true);
  document.body.classList.add('mobile-drawer-open');
  const btn = document.getElementById('mobile-menu-btn');
  const titleBtn = document.getElementById('mobile-session-title-btn');
  const backdrop = document.getElementById('mobile-drawer-backdrop');
  if (btn) btn.setAttribute('aria-expanded', 'true');
  if (titleBtn) titleBtn.setAttribute('aria-expanded', 'true');
  if (backdrop) backdrop.hidden = false;
}

export function closeMobileSessionDrawer() {
  document.body.classList.remove('mobile-drawer-open');
  const btn = document.getElementById('mobile-menu-btn');
  const titleBtn = document.getElementById('mobile-session-title-btn');
  const backdrop = document.getElementById('mobile-drawer-backdrop');
  if (btn) btn.setAttribute('aria-expanded', 'false');
  if (titleBtn) titleBtn.setAttribute('aria-expanded', 'false');
  if (backdrop) backdrop.hidden = true;
}

window.syncMobileLayoutState = syncMobileLayoutState;
window.closeMobileSessionDrawer = closeMobileSessionDrawer;

function mobileSessionToastTitle(id: number): string {
  const s = sessions.get(id);
  if (!s) return `#${id}`;
  const name = s.label || String(s.cwd || '').replace(/\\/g, '/').split('/').filter(Boolean).pop() || s.provider || '';
  return name ? `#${id} ${name}` : `#${id}`;
}

function activateNextMobileSession(): void {
  if (sessions.size <= 1 || activeSessionId === null) return;
  const all = orderSessions();
  const currentIdx = all.findIndex(s => s.id === activeSessionId);
  if (currentIdx === -1) return;
  const next = all[(currentIdx + 1) % all.length];
  if (!next) return;
  activateSession(next.id);
  showToast(t('session_switch_toast', { name: mobileSessionToastTitle(next.id) }));
}

function initMobileEdgeGestures(): void {
  let startX = 0;
  let startY = 0;
  let edge: 'left' | 'right' | null = null;
  let consumed = false;
  const EDGE_PX = 24;
  const MIN_DX = 54;
  // スクロールコンテナ（.mtl-chat-view / #mobile-home / .mobile-drawer-body）は
  // 起点ブロックしない: 画面端 EDGE_PX 内の touchstart は常にジェスチャー候補とし、
  // 縦スクロールとの競合は touchmove の縦優先キャンセル（|dy| > |dx| → edge=null）で防ぐ。
  const blockedSelector = [
    '#mobile-approval-sheet',
    '#mobile-approval-sheet-backdrop',
    '.mas-grab',
    '.mas-content',
  ].join(', ');
  const isGestureBlocked = (target: EventTarget | null): boolean => {
    const el = target instanceof HTMLElement ? target : null;
    return !!el?.closest(blockedSelector) || document.body.classList.contains('mobile-drawer-open') || document.body.classList.contains('mobile-approval-sheet-open');
  };

  document.addEventListener('touchstart', (ev) => {
    if (!isMobileViewport()) return;
    if (document.querySelector('.mtl-detail-overlay')) return;
    if (isGestureBlocked(ev.target)) return;
    if ((ev.target as HTMLElement | null)?.closest('input, textarea, select, a')) return;
    const touch = ev.touches[0];
    if (!touch) return;
    const width = window.innerWidth || document.documentElement.clientWidth;
    startX = touch.clientX;
    startY = touch.clientY;
    consumed = false;
    if (startX <= EDGE_PX) edge = 'left';
    else if (width - startX <= EDGE_PX) edge = 'right';
    else edge = null;
  }, { passive: true });

  document.addEventListener('touchmove', (ev) => {
    if (!edge || consumed || !isMobileViewport()) return;
    if (document.querySelector('.mtl-detail-overlay') || document.body.classList.contains('mobile-drawer-open') || document.body.classList.contains('mobile-approval-sheet-open')) { edge = null; return; }
    const touch = ev.touches[0];
    if (!touch) return;
    const dx = touch.clientX - startX;
    const dy = touch.clientY - startY;
    if (Math.abs(dy) > Math.abs(dx)) {
      edge = null;
      return;
    }
    if (edge === 'left' && dx > MIN_DX) {
      consumed = true;
      openMobileSessionDrawer();
    } else if (edge === 'right' && dx < -MIN_DX) {
      consumed = true;
      activateNextMobileSession();
    }
  }, { passive: true });

  document.addEventListener('touchend', () => {
    edge = null;
    consumed = false;
  }, { passive: true });
}

(function initMobileControls() {
  const menuBtn = document.getElementById('mobile-menu-btn');
  const backdrop = document.getElementById('mobile-drawer-backdrop');
  const spawnBtn = document.getElementById('mobile-spawn-btn');
  const titleBtn = document.getElementById('mobile-session-title-btn');
  const logBtn = document.getElementById('mobile-log-btn');
  const keyboardToggle = document.getElementById('mobile-keyboard-toggle');
  const keyboardPanel = document.getElementById('mobile-keyboard-panel');
  const keyRow = document.getElementById('mobile-key-row');
  let ctrlNext = false;

  menuBtn?.addEventListener('click', (e) => {
    e.stopPropagation();
    if (document.body.classList.contains('mobile-drawer-open')) closeMobileSessionDrawer();
    else openMobileSessionDrawer();
  });
  backdrop?.addEventListener('click', closeMobileSessionDrawer);
  spawnBtn?.addEventListener('click', (e) => {
    e.stopPropagation();
    openMobileSessionDrawer();
    document.getElementById('new-session-btn')?.click();
  });
  titleBtn?.addEventListener('click', (e) => {
    e.stopPropagation();
    openMobileSessionDrawer();
  });
  logBtn?.addEventListener('click', (e) => {
    e.stopPropagation();
    if (!isMobileViewport()) return;
    document.querySelector<HTMLElement>('.mtl-detail-icon')?.click();
  });
  keyboardToggle?.addEventListener('click', () => {
    const nextHidden = !keyboardPanel.hidden;
    keyboardPanel.hidden = nextHidden;
    keyboardToggle.setAttribute('aria-expanded', String(!nextHidden));
  });
  keyRow?.addEventListener('click', (e) => {
    const btn = e.target.closest('button[data-mobile-key]');
    if (!btn || activeSessionId === null) return;
    const key = btn.dataset.mobileKey;
    if (key === 'ctrl') {
      ctrlNext = !ctrlNext;
      btn.setAttribute('aria-pressed', String(ctrlNext));
      return;
    }
    const textByKey = {
      esc: '\x1b',
      tab: '\t',
      up: '\x1b[A',
      down: '\x1b[B',
      right: '\x1b[C',
      left: '\x1b[D',
      'ctrl-o': '\x0f',
      'ctrl-c': '\x03',
    };
    const text = textByKey[key] || '';
    if (text) sendText(activeSessionId, text);
    ctrlNext = false;
    keyRow.querySelector('[data-mobile-key="ctrl"]')?.setAttribute('aria-pressed', 'false');
    focusInputForTerminalKeys();
  });
  inputEl.addEventListener('input', () => {
    if (!ctrlNext || activeSessionId === null || inputEl.value.length === 0) return;
    const ch = inputEl.value.slice(-1).toLowerCase();
    if (ch >= 'a' && ch <= 'z') {
      inputEl.value = inputEl.value.slice(0, -1);
      updateInputClearButton();
      sendText(activeSessionId, String.fromCharCode(ch.charCodeAt(0) - 96));
      ctrlNext = false;
      keyRow?.querySelector('[data-mobile-key="ctrl"]')?.setAttribute('aria-pressed', 'false');
    }
  });
  initMobileEdgeGestures();

  // ビューポート幅変化（スマホ↔PC 切替）で DOM 状態を再同期
  const mql = window.matchMedia('(max-width: 720px)');
  mql.addEventListener('change', syncMobileLayoutState);

  // NOTE: 循環 import（state.js → session-list.js → … → app.js）により、本モジュールは
  // state.js の本体評価より前に評価されうる。ここで同期的に syncMobileLayoutState() を
  // 呼ぶと activeSessionId が TDZ（Cannot access before initialization）で落ち、モジュール
  // グラフ全体の初期化が中断して WS 登録まで連鎖死する。files-view.js の sessionsRef と
  // 同じく、評価完了後のマイクロタスクへ遅延する。
  queueMicrotask(syncMobileLayoutState);
})();

export function sendQuickCommand(sessionId, cmd) {
  // 未登録（空）スロットは送信しない（ボタンは非表示だが念のためのガード）。
  if (!cmd || !cmd.trim()) return;
  // Ollama route セッションで /model 始まりはブロック（quick-model-btn 経由含む）
  if (isOllamaModelCommandBlocked(sessionId, cmd)) {
    showToast(t('toast_model_blocked_on_ollama'));
    return;
  }
  // shell セッション内で AI CLI 起動コマンドを検知 → 専用セッション spawn を誘導。
  // confirm は非同期なので、検知時のみ判定を待ってから（誘導不採択なら）実送信する。
  if (detectAiCliLaunchInShell(sessionId, cmd)) {
    void maybeNudgeAiCliLaunchInShell(sessionId, cmd).then((handled) => {
      if (!handled) doSendQuickCommand(sessionId, cmd);
    });
    return;
  }
  doSendQuickCommand(sessionId, cmd);
}

function doSendQuickCommand(sessionId, cmd) {
  // doSend / sendChoice と同様に承認 UI 状態を Hub と同期する。
  // /clear 等で画面がリセットされた後も approvalVisibleCache=true が残ると、
  // セッションカードの "Pending" バッジが消えなくなる。
  const prevOpts = approvalRawOptionsCache.get(sessionId);
  // 本文送信が失敗した場合は承認 UI と消費済み state を保持する。
  if (!sendSubmittedBody(sessionId, cmd)) return false;
  if (prevOpts) approvalConsumedSig.set(sessionId, approvalSig(prevOpts));
  hideActionBar(sessionId);
  approvalSuppressUntil.set(sessionId, Date.now() + 2000);
  setTimeout(() => {
    detectApproval(sessionId);
    maybeAutoSwitchToNextApproval();
  }, 2050);
  // 残骸への連結対策の \x15 前置は廃止（doSend と同じく residue-sweep.ts の事後掃除へ移行）。
  // 本文の送り方（ペースト包み・確定 \r 分離）も doSend と同じ共通経路に従う。
  focusInputForTerminalKeys();
  return true;
}

export function focusInputForTerminalKeys() {
  if (activeSessionId === null || document.activeElement === inputEl) return;
  try {
    inputEl.focus({ preventScroll: true });
  } catch (_) {
    inputEl.focus();
  }
}

export function sendSubmittedText(sessionId, text, opts: any = {}) {
  if (!sendText(sessionId, text)) return false;
  if (opts.recordMobileTranscript !== false) {
    recordMobileTranscriptUserSubmission(sessionId, text);
  }
  if (isMobileViewport()) window.dispatchEvent(new CustomEvent('mobile-composer-idle'));
  // 送信操作は最新出力を見たい意図なので、スクロールアップ中でも最下部へ戻して追従を再開する
  const t = terminals.get(sessionId);
  if (t) {
    t.autoScroll = true;
    try { t.term.scrollToBottom(); } catch (_) {}
    if (sessionId === activeSessionId) {
      updateScrollLockBtn(false);
      // 承認バー表示中に送信すると、hideActionBar による action-bar 消失 + clearInput +
      // PTY echo back + Codex TUI の再描画が連続で走る。単発 RAF の scrollTerminalToBottomSoon
      // では onScroll で autoScroll=false に倒れた後の再描画フレームで viewport が上へズレ、
      // 最悪スクロールバック先頭まで戻る。force + 複数 delay の fit+snap で
      // レイアウト確定後（~220ms 内）まで最下部に張り付かせる。
      const startedAt = Date.now();
      scrollTerminalToBottomSoon(sessionId, { force: true, passes: 4, startedAt });
      refitAndStickTerminalToBottomAfterLayoutSettles(sessionId, { force: true, startedAt });
      resumeTerminalBottomFollow(sessionId, { startedAt });
    }
  }
  return true;
}

export function sendText(sessionId, text) {
  if (!isWebSocketSendReady()) {
    notifySendFailure();
    return false;
  }
  try {
    ws.send(JSON.stringify({ type: 'pty_input', session_id: sessionId, text }));
    return true;
  } catch (_) {
    notifySendFailure();
    return false;
  }
}

export function requestSessionDismiss(id) {
  // 「セッションが勝手に消える」事案の犯人特定用
  // (docs/local/bugfix_session-silent-auto-dismiss_2026-07-21.md)。
  // dismiss を投げる直前に呼び出し元スタックを console と PTY 生ログ両方へ残す。
  // 発火経路が UI × / group × / multi-pane close / auto-dismiss / snapshot completed 等
  // 複数あるため、再発時にどれが引いたかをここで確定させる。
  try {
    const stack = new Error('dismiss trace').stack || '';
    console.warn('[session-dismiss-trace] session_id=' + id + ' ts=' + new Date().toISOString() + '\n' + stack);
  } catch (_) {}
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ type: 'session_dismiss', session_id: id }));
  }
}

export function dismissSession(id) {
  if (!sessions.has(id)) return;
  requestSessionDismiss(id);
}

export function requestSessionHistoryReset(id) {
  if (!sessions.has(id)) return;
  if (!confirm(t('session_history_reset_confirm'))) return;
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ type: 'session_history_reset', session_id: id }));
  } else {
    resetLocalSessionHistory(id);
    showToast(t('session_history_reset_done'));
  }
}

export function resetTerminalHistoryForSession(id) {
  const t = terminals.get(id);
  if (!t) return;
  t.pendingChunks = [];
  t.pendingTotalBytes = 0;
  t.pendingTextTail = '';
  t.markerFilterCarry = new Uint8Array(0);
  t.screenClearSeqCarry = new Uint8Array(0);
  t.autoScroll = true;
  try { t.term.clear(); } catch (_) {}
  try { t.term.scrollToBottom(); } catch (_) {}
}

export function resetAllLocalSessionHistory() {
  sessions.forEach(s => {
    s.first_message = '';
    s.last_message = '';
  });
  terminals.forEach((_t, id) => resetTerminalHistoryForSession(id));
  approvalVisibleCache.clear();
  multiQuestionVisibleCache.clear();
  multiQuestionDismissedCache.clear();
  multiQuestionLatchAt.clear();
  sequentialChoiceCache.clear();
  approvalRawOptionsCache.clear();
  approvalSourceCache.clear();
  approvalSourceEpochCache.clear();
  approvalReplayState.clear();
  approvalConsumedSigDeleteTimer.forEach(t => clearTimeout(t));
  approvalConsumedSigDeleteTimer.clear();
  approvalConsumedSig.clear();
  answeredMarkerSigs.clear();
  answeredApprovalCandidates.clear();
  answeredApprovalShapeKeys.clear();
  replayAnsweredApprovalTokens.clear();
  approvalSwitchCandidates.clear();
  batchSelections.clear();
  approvalSuppressUntil.clear();
  approvalAutoSwitchQueue.length = 0;
  resetAllChatHistory();
  hideActionBar(undefined);
  setMultiQuestionBannerVisible(false);
  if (activeSessionId !== null) {
    updateChatCountBadge();
    if (typeof mountChatPaneForSession === 'function') mountChatPaneForSession(activeSessionId);
  }
  renderSessionList();
}

export function resetLocalSessionHistory(id) {
  const s = sessions.get(id);
  if (s) {
    s.first_message = '';
    s.last_message = '';
  }
  resetTerminalHistoryForSession(id);
  approvalVisibleCache.delete(id);
  if (multiQuestionVisibleCache.delete(id) && id === activeSessionId) {
    setMultiQuestionBannerVisible(false);
  }
  multiQuestionDismissedCache.delete(id);
  multiQuestionLatchAt.delete(id);
  clearSequentialChoiceState(id);
  approvalRawOptionsCache.delete(id);
  approvalSourceCache.delete(id);
  approvalSourceEpochCache.delete(id);
  approvalReplayState.delete(id);
  const sigTimer = approvalConsumedSigDeleteTimer.get(id);
  if (sigTimer) clearTimeout(sigTimer);
  approvalConsumedSigDeleteTimer.delete(id);
  approvalConsumedSig.delete(id);
  answeredApprovalCandidates.delete(id);
  answeredApprovalShapeKeys.delete(id);
  replayAnsweredApprovalTokens.delete(id);
  approvalSwitchCandidates.delete(id);
  batchSelections.delete(id);
  approvalSuppressUntil.delete(id);
  removeApprovalAutoSwitchTarget(id);
  resetChatHistoryForSession(id);
  if (id === activeSessionId) {
    hideActionBar(undefined);
    updateChatCountBadge();
    if (typeof mountChatPaneForSession === 'function') mountChatPaneForSession(id);
  }
  renderSessionList();
}

export function clearSessionTimerEntry(timerMap, id) {
  if (!timerMap || typeof timerMap.get !== 'function' || typeof timerMap.delete !== 'function') return;
  const timer = timerMap.get(id);
  if (timer) clearTimeout(timer);
  timerMap.delete(id);
}

export function cleanupRemovedSessionState(id) {
  try { clearSessionTimerEntry(approvalCheckTimers, id); } catch (_) {}
  try { clearSessionTimerEntry(approvalSuppressRescanTimers, id); } catch (_) {}
  try {
    const sigTimer = approvalConsumedSigDeleteTimer.get(id);
    if (sigTimer) clearTimeout(sigTimer);
    approvalConsumedSigDeleteTimer.delete(id);
  } catch (_) {}
  try { cancelDeferredEnter(id, 'session_removed'); } catch (_) {}
  try { cancelResidueSweep(id); } catch (_) {}
  try { cancelExpandCapture(id); } catch (_) {}
  try { doSendInFlight.delete(id); } catch (_) {}
  try { if (typeof window._wakewordSessionRemoved === 'function') window._wakewordSessionRemoved(id); } catch (_) {}
  // review-view.ts のターン完了カード状態（キャッシュ・dismiss 記録）を破棄する。
  // Hub 再起動でセッション ID が再利用されるため、残すと新セッションのターン
  // 番号と衝突して正当なカードが黙って抑止される（purge 経路もここを通る）。
  try { window.dispatchEvent(new CustomEvent('many-session-removed', { detail: { session_id: id } })); } catch (_) {}
}

export function removeLocalSession(id) {
  const timer = autoDismissTimers.get(id);
  if (timer) { clearTimeout(timer); autoDismissTimers.delete(id); }
  // 削除直前にセッション一覧上の「上の隣」を覚えておく。
  // active セッションを削除した際、勝手に先頭へジャンプしないよう、
  // 削除カードの直上（先頭の場合は直下）を後でアクティブ化する。
  let neighborId: number | null = null;
  if (activeSessionId === id) {
    try {
      const ordered = orderSessions();
      const idx = ordered.findIndex(s => s.id === id);
      if (idx > 0) neighborId = ordered[idx - 1].id;
      else if (idx === 0 && ordered.length > 1) neighborId = ordered[1].id;
    } catch (_) {}
  }
  cleanupRemovedSessionState(id);
  try {
    const mgr = window.multiPaneManager;
    if (mgr && typeof mgr.onSessionRemoved === 'function') mgr.onSessionRemoved(id);
  } catch (_) {}
  // sessions.delete より前に git/files タブの付け替えを試みる
  // （onSessionRemoved 内で sessionsRef を引いて代替を探すため、削除前の方が探しやすい）
  try { FilesTabManager.onSessionRemoved(id); } catch (_) {}
  // C1/C2: チャット store とビューモード state をクリーンアップ
  try { if (typeof onChatHistorySessionRemoved === 'function') onChatHistorySessionRemoved(id); } catch (_) {}
  try { clearMobileTranscriptSession(id); clearMobileTerminalLiteSession(id); } catch (_) {}
  try { if (typeof sessionViewMode !== 'undefined') sessionViewMode.delete(id); } catch (_) {}
  try { if (typeof sessionLazyLoaded !== 'undefined') sessionLazyLoaded.delete(id); } catch (_) {}
  sessions.delete(id);
  removeFromSessionOrder(id);
  const t = terminals.get(id);
  if (t) { try { t.term.dispose(); } catch (_) {} terminals.delete(id); }
  approvalVisibleCache.delete(id);
  if (multiQuestionVisibleCache.delete(id) && id === activeSessionId) {
    setMultiQuestionBannerVisible(false);
  }
  multiQuestionDismissedCache.delete(id);
  multiQuestionLatchAt.delete(id);
  removeApprovalAutoSwitchTarget(id);
  approvalRawOptionsCache.delete(id);
  approvalSourceCache.delete(id);
  approvalSourceEpochCache.delete(id);
  approvalReplayState.delete(id);
  approvalConsumedSig.delete(id);
  answeredMarkerSigs.delete(id);
  answeredApprovalCandidates.delete(id);
  answeredApprovalShapeKeys.delete(id);
  replayAnsweredApprovalTokens.delete(id);
  batchSelections.delete(id);
  clearSequentialChoiceState(id);
  cancelApprovalHintConfirm(id);
  approvalSuppressUntil.delete(id);
  cleanupSessionInputState(id);
  onChatHistorySessionRemoved(id);
  if (activeSessionId === id) {
    set_activeSessionId(null);
    syncMobileLayoutState();
    const area = document.getElementById('terminal-area');
    if (area) area.innerHTML = '';
    hideActionBar(undefined);
    setMultiQuestionBannerVisible(false);
    if (sessions.size > 0) {
      const next = (neighborId !== null && sessions.has(neighborId))
        ? neighborId
        : sessions.keys().next().value;
      activateSession(next);
      return;
    }
  }
  maybeAutoSwitchToNextApproval();
  render();
}


// セッション選択中は inputEl からフォーカスが外れたら即座に戻す
// ただし設定パネルが開いている間、またはテキスト選択操作中はフォーカスを奪わない
export let suppressFocusReclaim = false;
export let voiceActive = false;
export let voiceAudioActive = false;
export function isInteractiveFocusTarget(target) {
  if (!(target instanceof Element)) return false;
  return !!target.closest([
    'button',
    'input',
    'textarea',
    'select',
    'a',
    'label',
    '[contenteditable="true"]',
    '[role="button"]',
    '#input-bar',
    '#action-bar',
    '#attach-panel',
    '#slash-picker',
    '#settings-panel',
    '#new-session-panel',
    '#model-picker-overlay',
    '#about-panel',
    '#session-list',
    '#topbar',
    // ターミナル領域上の読み取り専用オーバーレイ。本文のドラッグ選択を
    // 入力欄フォーカス回収で潰さないため、フォーカス奪取対象外にする。
    '#grok-chat-viewer',
    '#history-viewer',
  ].join(','));
}
document.addEventListener('mousedown', () => { suppressFocusReclaim = true; });
document.addEventListener('mouseup',   () => { setTimeout(() => { suppressFocusReclaim = false; }, 300); });

inputEl.addEventListener('blur', (e) => {
  if (isInteractiveFocusTarget(e.relatedTarget)) return;
  if (suppressFocusReclaim || voiceActive) return;
  if (activeSessionId !== null && document.getElementById('settings-panel').hidden) {
    setTimeout(() => inputEl.focus(), 0);
  }
});


// moved to /app/spawn-panel.js

// C5: Detached Grid Launcher ボタン
(function () {
  const launcherBtn = document.getElementById('detached-grid-launcher-btn');
  if (!launcherBtn) return;
  launcherBtn.addEventListener('click', () => {
    if (typeof (window as any).openDetachedGridLauncher === 'function') {
      (window as any).openDetachedGridLauncher();
    }
  });
})();

(function () {
  const killAllBtns = [document.getElementById('kill-all-btn'), document.getElementById('settings-kill-all-btn')]
    .filter((button): button is HTMLButtonElement => button instanceof HTMLButtonElement);
  if (killAllBtns.length === 0) return;

  killAllBtns.forEach((killAllBtn) => killAllBtn.addEventListener('click', async () => {
    const ok = await appConfirmTypedDanger({
      title: t('kill_all_confirm_title'),
      message: t('kill_all_confirm'),
      confirmText: t('kill_all_confirm_run'),
      cancelText: t('spawn_cancel'),
      phrase: 'KILL ALL',
    });
    if (!ok) return;
    killAllBtns.forEach((button) => { button.disabled = true; });
    try {
      await fetch(`/api/kill-all?token=${token}`, { method: 'POST' });
    } catch (_) {}
    killAllBtns.forEach((button) => { button.disabled = false; });
  }));
})();

(function () {
  const shutdownBtn = document.getElementById('shutdown-btn');
  if (!shutdownBtn) return;

  shutdownBtn.addEventListener('click', async () => {
    // 外部公開（tailscale serve）中なら確認に「公開も停止」チェック（既定 ON）を出す。
    // serve は --bg で tailscaled 側に残るため、Hub 停止だけだと幽霊公開状態になる。
    let exposeActive = getExposeStatus()?.state === 'ready';
    try {
      const r = await fetchExposeStatus(true);
      if (r.ok && r.status) exposeActive = r.status.state === 'ready';
    } catch (_) { /* 取得失敗時はキャッシュ値で判断 */ }

    const result = await appConfirmShutdown({ exposeActive });
    if (!result) return;
    shutdownBtn.disabled = true;

    // graceful 経路でのみ確実に停止できる（PID kill / クラッシュでは走らない）。
    if (result.stopExpose) {
      try { await disableExpose(); } catch (_) {}
    }

    if (result.action === 'sessions') {
      try { await fetch(`/api/kill-all?token=${token}`, { method: 'POST' }); } catch (_) {}
      try { await fetch(`/api/shutdown?token=${token}`, { method: 'POST' }); } catch (_) {}
      window.close();
    } else {
      try {
        await fetch(`/api/shutdown?token=${token}`, { method: 'POST' });
      } catch (_) {}
      window.close();
    }
  });
})();

(function () {
  const idleTimeoutEl     = document.getElementById('idle-timeout-min');
  const reconnectGraceEl  = document.getElementById('reconnect-grace-min');
	const boardNotifyModeEl = document.getElementById('board-notify-mode') as HTMLSelectElement | null;
	const spawnConfirmModeEl = document.getElementById('spawn-confirm-mode') as HTMLSelectElement | null;
	const spawnConfirmProvidersEl = document.getElementById('spawn-confirm-providers') as HTMLInputElement | null;
	const spawnConfirmProvidersRow = document.getElementById('spawn-confirm-providers-row');
	const orchestrationChildTimeoutEl = document.getElementById('orchestration-child-timeout') as HTMLInputElement | null;
	const orchestrationTimeoutRespawnEl = document.getElementById('orchestration-timeout-respawn') as HTMLInputElement | null;
  const logEnabledEl               = document.getElementById('log-enabled');
  const logSessionEnabledEl        = document.getElementById('log-session-enabled');
  const logMaxSizeEl               = document.getElementById('log-max-size');
  const logMaxBackupsEl            = document.getElementById('log-max-backups');
  const logSessionRetentionDaysEl  = document.getElementById('log-session-retention-days');
  const logSessionMaxSizeEl        = document.getElementById('log-session-max-size');
  const attachRetentionDaysEl      = document.getElementById('attach-retention-days');
  const attachMaxTotalMbEl         = document.getElementById('attach-max-total-mb');

  async function loadIdleTimeout() {
    if (!idleTimeoutEl) return;
    try {
      const res = await fetch(`/api/idle-timeout?token=${token}`);
      if (!res.ok) return;
      const cfg = await res.json();
      idleTimeoutEl.value = cfg.idle_timeout_min;
    } catch (_) {}
  }

  async function saveIdleTimeout() {
    if (!idleTimeoutEl) return;
    const raw = parseInt(idleTimeoutEl.value, 10);
    const min = Number.isFinite(raw) ? Math.max(0, Math.min(1440, raw)) : 60;
    idleTimeoutEl.value = String(min);
    try {
      await fetch(`/api/idle-timeout?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ idle_timeout_min: min }),
      });
    } catch (_) {}
  }

  async function loadReconnectGrace() {
    if (!reconnectGraceEl) return;
    try {
      const res = await fetch(`/api/reconnect-grace?token=${token}`);
      if (!res.ok) return;
      const cfg = await res.json();
      const sec = Number(cfg.wrapper_reconnect_grace_sec) || 0;
      reconnectGraceEl.value = String(Math.round(sec / 60));
    } catch (_) {}
  }

  async function saveReconnectGrace() {
    if (!reconnectGraceEl) return;
    const raw = parseInt(reconnectGraceEl.value, 10);
    const min = Number.isFinite(raw) ? Math.max(0, Math.min(1440, raw)) : 60;
    reconnectGraceEl.value = String(min);
    try {
      await fetch(`/api/reconnect-grace?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ wrapper_reconnect_grace_sec: min * 60 }),
      });
    } catch (_) {}
  }

  async function loadLogConfig() {
    try {
      const res = await fetch(`/api/log-config?token=${token}`);
      if (!res.ok) return;
      const cfg = await res.json();
      logEnabledEl.checked  = cfg.enabled;
      if (logSessionEnabledEl) logSessionEnabledEl.checked = !!cfg.session_enabled;
      logMaxSizeEl.value    = cfg.max_size_mb;
      logMaxBackupsEl.value = cfg.max_backups;
      if (logSessionRetentionDaysEl) logSessionRetentionDaysEl.value = cfg.session_retention_days ?? 7;
      if (logSessionMaxSizeEl) logSessionMaxSizeEl.value = cfg.session_max_size_mb ?? 50;
      if (attachRetentionDaysEl) attachRetentionDaysEl.value = cfg.attachment_retention_days ?? 7;
      if (attachMaxTotalMbEl) attachMaxTotalMbEl.value = cfg.attachment_max_total_mb ?? 500;
      const logDirBtn = document.getElementById('log-dir-btn');
      if (logDirBtn && cfg.log_dir) {
        logDirBtn.dataset.tooltip = cfg.log_dir;
      }
      const logDirPath = document.getElementById('log-dir-path') as HTMLAnchorElement | null;
      if (logDirPath && cfg.log_dir) {
        logDirPath.textContent = cfg.log_dir;
        logDirPath.title = cfg.log_dir;
      }
      const attachDirBtn = document.getElementById('attach-dir-btn');
      if (attachDirBtn && cfg.attach_dir) {
        attachDirBtn.dataset.tooltip = cfg.attach_dir;
      }
    } catch (_) {}
  }

  async function openDirOrCopy(btn, kind) {
    const path = btn.dataset.tooltip;
    if (!path || path === t('loading')) return;
    try {
      const res = await fetch(`/api/open-dir?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ kind }),
      });
      if (res.ok) return;
    } catch (_) {}
    try {
      await navigator.clipboard.writeText(path);
      const prev = btn.dataset.tooltip;
      btn.dataset.tooltip = t('copied_to_clipboard');
      setTimeout(() => { btn.dataset.tooltip = prev; }, 1500);
    } catch (_) {}
  }

  const logDirBtn = document.getElementById('log-dir-btn');
  if (logDirBtn) {
    logDirBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      openDirOrCopy(logDirBtn, 'log');
    });
  }
  const logDirPath = document.getElementById('log-dir-path');
  if (logDirPath) {
    logDirPath.addEventListener('click', (e) => {
      e.stopPropagation();
      if (logDirBtn) openDirOrCopy(logDirBtn, 'log');
    });
  }

  const attachDirBtn = document.getElementById('attach-dir-btn');
  if (attachDirBtn) {
    attachDirBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      openDirOrCopy(attachDirBtn, 'attach');
    });
  }

  const sessionStoreResetBtn = document.getElementById('session-store-reset-btn');
  if (sessionStoreResetBtn) {
    sessionStoreResetBtn.addEventListener('click', async (e) => {
      e.stopPropagation();
      const ok = await appConfirmTypedDanger({
        title: t('settings_history_reset_confirm_title'),
        message: t('settings_history_reset_confirm_message'),
        confirmText: t('settings_history_reset_confirm_run'),
        cancelText: t('confirm_cancel'),
        phrase: 'RESET HISTORY',
      });
      if (!ok) return;
      sessionStoreResetBtn.disabled = true;
      try {
        const res = await fetch(`/api/session-store/reset?token=${token}`, { method: 'POST' });
        if (!res.ok) {
          showToast(t('settings_history_reset_failed'), sessionStoreResetBtn);
          return;
        }
        resetAllLocalSessionHistory();
        showToast(t('settings_history_reset_done'), sessionStoreResetBtn);
      } catch (_) {
        showToast(t('settings_history_reset_failed'), sessionStoreResetBtn);
      } finally {
        sessionStoreResetBtn.disabled = false;
      }
    });
  }

  const logsPurgeBtn = document.getElementById('logs-purge-btn');
  if (logsPurgeBtn) {
    logsPurgeBtn.addEventListener('click', async (e) => {
      e.stopPropagation();
      const ok = await appConfirmTypedDanger({
        title: t('settings_logs_purge_confirm_title'),
        message: t('settings_logs_purge_confirm_message'),
        confirmText: t('settings_history_reset_confirm_run'),
        cancelText: t('confirm_cancel'),
        phrase: 'PURGE LOGS',
      });
      if (!ok) return;
      logsPurgeBtn.disabled = true;
      try {
        const res = await fetch(`/api/logs/purge?token=${token}`, { method: 'POST' });
        if (!res.ok) {
          showToast(t('settings_logs_purge_failed'), logsPurgeBtn);
          return;
        }
        resetAllLocalSessionHistory();
        showToast(t('settings_logs_purge_done'), logsPurgeBtn);
      } catch (_) {
        showToast(t('settings_logs_purge_failed'), logsPurgeBtn);
      } finally {
        logsPurgeBtn.disabled = false;
      }
    });
  }

  const attachmentsPurgeBtn = document.getElementById('attachments-purge-btn');
  if (attachmentsPurgeBtn) {
    attachmentsPurgeBtn.addEventListener('click', async (e) => {
      e.stopPropagation();
      const ok = await appConfirmTypedDanger({
        title: t('settings_attachments_purge_confirm_title'),
        message: t('settings_attachments_purge_confirm_message'),
        confirmText: t('settings_history_reset_confirm_run'),
        cancelText: t('confirm_cancel'),
        phrase: 'PURGE ATTACHMENTS',
      });
      if (!ok) return;
      attachmentsPurgeBtn.disabled = true;
      try {
        const res = await fetch(`/api/attachments/purge?token=${token}`, { method: 'POST' });
        if (!res.ok) {
          showToast(t('settings_attachments_purge_failed'), attachmentsPurgeBtn);
          return;
        }
        showToast(t('settings_attachments_purge_done'), attachmentsPurgeBtn);
      } catch (_) {
        showToast(t('settings_attachments_purge_failed'), attachmentsPurgeBtn);
      } finally {
        attachmentsPurgeBtn.disabled = false;
      }
    });
  }

  (function () {
    const KEY = 'many-ai-cli.settings-section-state';
    let state = {};
    try { state = JSON.parse(localStorage.getItem(KEY) || '{}') || {}; } catch (_) { state = {}; }
    document.querySelectorAll('.settings-section[data-section]').forEach((el) => {
      const id = el.dataset.section;
      if (state[id]) el.open = true;
      el.addEventListener('toggle', () => {
        state[id] = el.open;
        try { localStorage.setItem(KEY, JSON.stringify(state)); } catch (_) {}
      });
    });
  })();

  const approvalToggleInput = document.getElementById('approval-toggle-input');
  if (approvalToggleInput) {
    approvalToggleInput.addEventListener('change', async () => {
      const endpoint = approvalToggleInput.checked ? 'enable' : 'disable';
      try {
        await fetch(`/api/approval/${endpoint}?token=${token}`, { method: 'POST' });
      } catch (_) {}
    });
  }
  const approvalAutoSwitchInput = document.getElementById('approval-auto-switch-input');
  if (approvalAutoSwitchInput) {
    approvalAutoSwitchInput.checked = localStorage.getItem(STORAGE_APPROVAL_AUTO_SWITCH_KEY) === '1';
    approvalAutoSwitchInput.addEventListener('change', () => {
      setUserPref('approval.auto_switch', approvalAutoSwitchInput.checked);
      if (approvalAutoSwitchInput.checked) maybeAutoSwitchToNextApproval();
    });
  }

	async function loadBoardNotifyMode() {
		if (!boardNotifyModeEl) return;
		try {
			const res = await fetch(`/api/orchestration-config?token=${token}`);
			if (!res.ok) return;
			const cfg = await res.json();
			const mode = String(cfg.board_notify_mode || 'queue-until-idle');
			boardNotifyModeEl.value = ['soft-notify', 'queue-until-idle', 'interrupt'].includes(mode) ? mode : 'queue-until-idle';
			if (spawnConfirmModeEl) {
				const spawnMode = String(cfg.spawn_confirm_mode || 'on');
				spawnConfirmModeEl.value = ['on', 'off', 'providers'].includes(spawnMode) ? spawnMode : 'on';
				if (spawnConfirmProvidersEl) spawnConfirmProvidersEl.value = Array.isArray(cfg.spawn_confirm_providers) ? cfg.spawn_confirm_providers.join(', ') : '';
				if (spawnConfirmProvidersRow) spawnConfirmProvidersRow.hidden = spawnConfirmModeEl.value !== 'providers';
			}
			if (orchestrationChildTimeoutEl) orchestrationChildTimeoutEl.value = String(cfg.child_timeout_seconds || 900);
			if (orchestrationTimeoutRespawnEl) orchestrationTimeoutRespawnEl.checked = Boolean(cfg.timeout_respawn);
		} catch (_) {}
	}

	async function saveBoardNotifyMode() {
		if (!boardNotifyModeEl) return;
		try {
			await fetch(`/api/orchestration-config?token=${token}`, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ board_notify_mode: boardNotifyModeEl.value, spawn_confirm_mode: spawnConfirmModeEl?.value || 'on', spawn_confirm_providers: (spawnConfirmProvidersEl?.value || '').split(',').map(v => v.trim()).filter(Boolean), child_timeout_seconds: Math.max(60, Math.min(86400, Number(orchestrationChildTimeoutEl?.value || 900))), timeout_respawn: Boolean(orchestrationTimeoutRespawnEl?.checked) }),
			});
		} catch (_) {}
	}
  const autoApprovalInput = document.getElementById('auto-approval-enabled-input');
  if (autoApprovalInput) {
    autoApprovalInput.checked = localStorage.getItem(STORAGE_AUTO_APPROVAL_ENABLED_KEY) === '1';
    autoApprovalInput.addEventListener('change', () => setUserPref('approval.auto_approval_enabled', autoApprovalInput.checked));
  }
  const highRiskModeInput = document.getElementById('approval-high-risk-mode') as HTMLSelectElement | null;
  if (highRiskModeInput) {
    highRiskModeInput.value = localStorage.getItem(STORAGE_HIGH_RISK_CONFIRMATION_MODE_KEY) === 'dialog' ? 'dialog' : 'hold';
    highRiskModeInput.addEventListener('change', () => {
      const mode = highRiskModeInput.value === 'dialog' ? 'dialog' : 'hold';
      setUserPref('approval.high_risk_confirmation_mode', mode);
    });
  }
  const autoApprovalSimulateBtn = document.getElementById('auto-approval-simulate-btn');
  if (autoApprovalSimulateBtn) {
    autoApprovalSimulateBtn.addEventListener('click', async () => {
      const result = document.getElementById('auto-approval-simulate-result');
      try {
        const res = await fetch(`/api/auto-approval/simulate?token=${encodeURIComponent(token || '')}&n=100`);
        const data = res.ok ? await res.json() : null;
        if (result) result.textContent = data ? `${data.total || 0}件中 ${data.matched || 0}件が一致（危険操作は除外）` : 'シミュレーションに失敗しました';
      } catch (_) { if (result) result.textContent = 'シミュレーションに失敗しました'; }
    });
  }

  async function saveLogConfig() {
    try {
      await fetch(`/api/log-config?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          enabled:                 logEnabledEl.checked,
          session_enabled:         !!logSessionEnabledEl?.checked,
          max_size_mb:             parseInt(logMaxSizeEl.value, 10) || 10,
          max_backups:             parseInt(logMaxBackupsEl.value, 10) || 3,
          compress:                false,
          session_retention_days:  parseInt(logSessionRetentionDaysEl?.value ?? '7', 10) || 7,
          session_max_size_mb:     parseInt(logSessionMaxSizeEl?.value ?? '50', 10),
          attachment_retention_days: parseInt(attachRetentionDaysEl?.value ?? '7', 10) || 0,
          attachment_max_total_mb:   parseInt(attachMaxTotalMbEl?.value ?? '500', 10) || 0,
        }),
      });
    } catch (_) {}
  }

  async function saveSlashCmdSources() {
    const body = {
      claude: (document.getElementById('slash-src-claude')?.value || '').trim(),
      codex:  (document.getElementById('slash-src-codex')?.value  || '').trim(),
      copilot: (document.getElementById('slash-src-copilot')?.value || '').trim(),
      'cursor-agent': (document.getElementById('slash-src-cursor-agent')?.value || '').trim(),
    };
    try {
      await fetch(`/api/slash-cmd-sources?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
    } catch (_) {}
  }

  window.__settingsSaveAll = async () => {
    await saveIdleTimeout();
    await saveReconnectGrace();
		await saveBoardNotifyMode();
    await saveLogConfig();
    await saveSlashCmdSources();
    saveUsageLinkSettings();
  };

  window.__settingsResetAll = async () => {
    applyTheme('light');
    applyFontSize('medium');
    applyLang('ja');
    setUserPref('display.theme', 'light');
    setUserPref('display.font_size', 'medium');
    setUserPref('display.lang', 'ja');

    setUserPref('trigger.enabled', false);
    setUserPref('trigger.phrase', getDefaultTriggerPhrase());
    setUserPref('voice.wake_word_enabled', false);
    setUserPref('voice.wake_word_phrase', getDefaultWakeWordPhrase());
    setVoiceEngine('browser');
    try { localStorage.removeItem(STORAGE_VOICE_WHISPER_AUTO_SUBMIT_KEY); } catch (_) {}
    setUserPref('notify_sound.enabled', false);
    setUserPref('notify_sound.type', 'default');
    try { localStorage.removeItem(STORAGE_NOTIFY_SOUND_CUSTOM_KEY); } catch (_) {}
    setUserPref('approval.auto_switch', false);
	setUserPref('approval.auto_approval_enabled', false);
    for (let slot = 1; slot <= QUICK_CMD_SLOTS; slot++) {
      setUserPref(`quick_cmds.cmd${slot}`, quickCommandDefault(slot));
      setUserPref(`quick_cmds.show${slot}`, true);
    }
    setUserPref('usage_links.claude', '');
    setUserPref('usage_links.codex', '');
    setUserPref('usage_links.copilot', '');
    setUserPref('usage_links.cursor-agent', '');
    setUserPref('usage_links.ollama', '');
    setUserPref('usage_links.opencode', '');
    setUserPref('usage_links.grok', '');
    setUserPref('voice.grace_seconds', DEFAULT_VOICE_GRACE_SEC);

    const triggerEnabled = document.getElementById('trigger-enabled');
    const triggerPhrase  = document.getElementById('trigger-phrase-input');
    const triggerRow     = document.getElementById('trigger-phrase-row');
    if (triggerEnabled) triggerEnabled.checked = false;
    if (triggerPhrase) triggerPhrase.value = getDefaultTriggerPhrase();
    if (triggerRow) triggerRow.hidden = true;

    const wakeWordEnabled = document.getElementById('wakeword-enabled');
    const wakeWordPhrase  = document.getElementById('wakeword-phrase-input');
    const wakeWordRow     = document.getElementById('wakeword-phrase-row');
    if (wakeWordEnabled) wakeWordEnabled.checked = false;
    if (wakeWordPhrase) wakeWordPhrase.value = getDefaultWakeWordPhrase();
    if (wakeWordRow) wakeWordRow.hidden = true;
    document.dispatchEvent(new CustomEvent('wakewordsettings:changed'));

    const soundEnabledEl  = document.getElementById('notify-sound-enabled');
    const soundTypeEl     = document.getElementById('notify-sound-type');
    const soundTypeRow    = document.getElementById('notify-sound-type-row');
    const soundCustomRow  = document.getElementById('notify-sound-custom-row');
    const soundFilenameEl = document.getElementById('notify-sound-filename');
    const soundFileEl     = document.getElementById('notify-sound-file');
    if (soundEnabledEl) soundEnabledEl.checked = false;
    if (soundTypeEl) soundTypeEl.value = 'default';
    if (soundTypeRow) soundTypeRow.hidden = true;
    if (soundCustomRow) soundCustomRow.hidden = true;
    if (soundFilenameEl) soundFilenameEl.textContent = '';
    if (soundFileEl) soundFileEl.value = '';

    for (let slot = 1; slot <= QUICK_CMD_SLOTS; slot++) {
      const el = document.getElementById(`quick-cmd-${slot}`);
      if (el) el.value = quickCommandDefault(slot);
      const showEl = document.getElementById(`quick-cmd-${slot}-show`);
      if (showEl) showEl.checked = true;
    }

    const voiceGraceEl = document.getElementById('voice-grace-select');
    if (voiceGraceEl) {
      voiceGraceEl.value = String(DEFAULT_VOICE_GRACE_SEC);
    }

    const approvalAutoSwitchInput = document.getElementById('approval-auto-switch-input');
    if (approvalAutoSwitchInput) approvalAutoSwitchInput.checked = false;
	const autoApprovalInput = document.getElementById('auto-approval-enabled-input');
	if (autoApprovalInput) autoApprovalInput.checked = false;
	const highRiskModeInput = document.getElementById('approval-high-risk-mode') as HTMLSelectElement | null;
	if (highRiskModeInput) highRiskModeInput.value = 'hold';
	setUserPref('approval.high_risk_confirmation_mode', 'hold');

    const idleTimeoutEl = document.getElementById('idle-timeout-min');
    const reconnectGraceEl = document.getElementById('reconnect-grace-min');
    const logEnabledEl = document.getElementById('log-enabled');
    const logMaxSizeEl = document.getElementById('log-max-size');
    const logMaxBackupsEl = document.getElementById('log-max-backups');
    if (idleTimeoutEl) idleTimeoutEl.value = '60';
    if (reconnectGraceEl) reconnectGraceEl.value = '60';
    if (logEnabledEl) logEnabledEl.checked = true;
    const logSessionEnabledEl2 = document.getElementById('log-session-enabled');
    if (logSessionEnabledEl2) logSessionEnabledEl2.checked = false;
    if (logMaxSizeEl) logMaxSizeEl.value = '10';
    if (logMaxBackupsEl) logMaxBackupsEl.value = '3';
    const logSessionRetentionDaysEl2 = document.getElementById('log-session-retention-days');
    if (logSessionRetentionDaysEl2) logSessionRetentionDaysEl2.value = '7';
    const logSessionMaxSizeEl2 = document.getElementById('log-session-max-size');
    if (logSessionMaxSizeEl2) logSessionMaxSizeEl2.value = '50';
    const attachRetentionDaysEl2 = document.getElementById('attach-retention-days');
    if (attachRetentionDaysEl2) attachRetentionDaysEl2.value = '7';
    const attachMaxTotalMbEl2 = document.getElementById('attach-max-total-mb');
    if (attachMaxTotalMbEl2) attachMaxTotalMbEl2.value = '500';

    const approvalToggleInput = document.getElementById('approval-toggle-input');
    if (approvalToggleInput) {
      approvalToggleInput.checked = false;
      try { await fetch(`/api/approval/disable?token=${token}`, { method: 'POST' }); } catch (_) {}
    }

    const slashClaudeEl = document.getElementById('slash-src-claude');
    const slashCodexEl = document.getElementById('slash-src-codex');
    const slashCopilotEl = document.getElementById('slash-src-copilot');
    const slashCursorAgentEl = document.getElementById('slash-src-cursor-agent');
    if (slashClaudeEl) slashClaudeEl.value = '';
    if (slashCodexEl) slashCodexEl.value = '';
    if (slashCopilotEl) slashCopilotEl.value = '';
    if (slashCursorAgentEl) slashCursorAgentEl.value = '';
    loadUsageLinkSettings();

    const termAppEl = document.getElementById('settings-terminal-app');
    if (termAppEl) termAppEl.value = '';

    await window.__settingsSaveAll();
    await loadApprovalSettings();
    // theme/font_size/lang を含む user_prefs をサーバへ確実に反映してからリロードする。
    // リロードしないと i18n（言語）が再描画されず、また mirror が旧サーバ値で
    // localStorage を上書きしてリセットが巻き戻るため、flush 後に reload する。
    // （従来は先頭の setLang が即リロードして reset 本体が途中で中断していた点も解消）
    try { await _putUserPrefsNow(); } catch (_) {}
    location.reload();
  };

  // 新バージョン移行時、初回ロードで一度だけ案内を出す。チェックボックスで
  // 「ログ・履歴を削除 / 添付を削除 / 今後ログを記録する」を複数選択 → 実行。
  // サーバ側が「未通知 & セッションログ無効 & 旧ログあり」と判定したときだけ表示し、
  // 表示後はフラグを立てて二度と出さない。
  (async () => {
    try {
      const res = await fetch(`/api/logs/legacy-notice?token=${token}`);
      if (!res.ok) return;
      const data = await res.json();
      if (!data.show) return;
      const choice = await appLegacyResetNotice();
      if (!choice) {
        // 閉じる/Escape: 変更なし。ただし再表示はしない（フラグだけ立てる）。
        try { await fetch(`/api/logs/legacy-notice?token=${token}`, { method: 'POST' }); } catch (_) {}
        return;
      }
      // フラグを立てつつ、選択したログ記録設定（オン/オフ）も保存する。
      try {
        await fetch(`/api/logs/legacy-notice?token=${token}`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ enable_logging: choice.enableLogging }),
        });
      } catch (_) {}
      // チェックされた削除対象だけ実行（実行中セッションのぶんは各 purge 側で保護）。
      const tasks: Promise<Response>[] = [];
      if (choice.deleteLogs) tasks.push(fetch(`/api/logs/purge?token=${token}`, { method: 'POST' }));
      if (choice.deleteAttachments) tasks.push(fetch(`/api/attachments/purge?token=${token}`, { method: 'POST' }));
      if (tasks.length === 0) {
        showToast(t('legacy_logs_notice_done'));
        return;
      }
      try {
        const results = await Promise.all(tasks);
        if (choice.deleteLogs) {
          resetAllLocalSessionHistory();
          }
        showToast(results.every(r => r.ok) ? t('legacy_logs_notice_done') : t('legacy_logs_notice_failed'));
      } catch (_) {
        showToast(t('legacy_logs_notice_failed'));
      }
    } catch (_) {}
  })();

  // 設定パネルが開かれたときにログ設定を読み込む
  document.getElementById('settings-btn').addEventListener('click', () => {
    if (!document.getElementById('settings-panel').hidden) {
      loadIdleTimeout();
      loadReconnectGrace();
		loadBoardNotifyMode();
      loadLogConfig();
      loadApprovalSettings();
      loadSlashCmdSources();
      loadUsageLinkSettings();
      attachTokenStatusbarToggle();
      attachDoneSummaryNotifyToggle();
      void loadDeferredEnterConfig();
    }
  });

  if (idleTimeoutEl) idleTimeoutEl.addEventListener('change', saveIdleTimeout);
  if (reconnectGraceEl) reconnectGraceEl.addEventListener('change', saveReconnectGrace);
	if (boardNotifyModeEl) boardNotifyModeEl.addEventListener('change', saveBoardNotifyMode);
	if (spawnConfirmModeEl) spawnConfirmModeEl.addEventListener('change', () => { if (spawnConfirmProvidersRow) spawnConfirmProvidersRow.hidden = spawnConfirmModeEl.value !== 'providers'; void saveBoardNotifyMode(); });
	if (spawnConfirmProvidersEl) spawnConfirmProvidersEl.addEventListener('change', () => void saveBoardNotifyMode());
	if (orchestrationChildTimeoutEl) orchestrationChildTimeoutEl.addEventListener('change', () => void saveBoardNotifyMode());
	if (orchestrationTimeoutRespawnEl) orchestrationTimeoutRespawnEl.addEventListener('change', () => void saveBoardNotifyMode());
  logEnabledEl.addEventListener('change', saveLogConfig);
  if (logSessionEnabledEl) logSessionEnabledEl.addEventListener('change', saveLogConfig);
  logMaxSizeEl.addEventListener('change', saveLogConfig);
  logMaxBackupsEl.addEventListener('change', saveLogConfig);
  if (logSessionRetentionDaysEl) logSessionRetentionDaysEl.addEventListener('change', saveLogConfig);
  if (logSessionMaxSizeEl) logSessionMaxSizeEl.addEventListener('change', saveLogConfig);
  if (attachRetentionDaysEl) attachRetentionDaysEl.addEventListener('change', saveLogConfig);
  if (attachMaxTotalMbEl) attachMaxTotalMbEl.addEventListener('change', saveLogConfig);

  const deferredEnterEl = document.getElementById('deferred-enter-ms') as HTMLSelectElement | null;
  async function loadDeferredEnterConfig() {
    if (!deferredEnterEl) return;
    try {
      const res = await fetch(`/api/input-config?token=${encodeURIComponent(token || '')}`);
      if (!res.ok) return;
      const cfg = await res.json();
      deferredEnterOverrideMs = Number(cfg?.deferred_enter_ms) || 0;
      deferredEnterEl.value = String(deferredEnterOverrideMs);
    } catch (_) {}
  }
  if (deferredEnterEl) {
    deferredEnterEl.addEventListener('change', async () => {
      const ms = Number(deferredEnterEl.value) || 0;
      try {
        const res = await fetch(`/api/input-config?token=${encodeURIComponent(token || '')}`, {
          method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ deferred_enter_ms: ms }),
        });
        if (!res.ok) throw new Error('save failed');
        deferredEnterOverrideMs = ms;
      } catch (_) {
        showToast('複数行ペーストの設定を保存できませんでした');
      }
    });
    void loadDeferredEnterConfig();
  }

  const deferredCancel = document.getElementById('deferred-send-cancel');
  if (deferredCancel) deferredCancel.addEventListener('click', () => {
    if (deferredSendSessionId == null) return;
    const id = deferredSendSessionId;
    cancelDeferredEnter(id, 'ui_cancel_button');
    clearDeferredSendStatus(id);
    showToast('複数行ペーストの確定送信をキャンセルしました');
  });
})();

(function () {
  const runButton = document.getElementById('doctor-run-btn') as HTMLButtonElement | null;
  const result = document.getElementById('doctor-result');
  if (!runButton || !result) return;

  function addText(parent: HTMLElement, className: string, value: string) {
    const el = document.createElement('span');
    el.className = className;
    el.textContent = value;
    parent.append(el);
  }

  async function runDoctor() {
    runButton.disabled = true;
    result.hidden = false;
    result.replaceChildren();
    addText(result, 'settings-note', t('settings_doctor_running'));
    try {
      const response = await fetch(`/api/doctor?token=${encodeURIComponent(token || '')}`);
      if (!response.ok) throw new Error(String(response.status));
      const report = await response.json();
      result.replaceChildren();
      for (const check of report.checks || []) {
        const card = document.createElement('div');
        card.className = 'doctor-check';
        addText(card, `doctor-level doctor-level--${check.level}`, `[${check.level}] ${check.name}`);
        addText(card, 'doctor-message', check.message || '');
        if (check.fix) {
          const fix = document.createElement('div');
          fix.className = 'doctor-fix';
          addText(fix, '', check.fix);
          const copy = document.createElement('button');
          copy.type = 'button';
          copy.className = 'settings-inline-btn doctor-copy';
          copy.textContent = t('settings_doctor_copy');
          copy.addEventListener('click', async () => {
            try { await navigator.clipboard.writeText(check.fix); showToast(t('settings_doctor_copied')); } catch (_) { showToast(t('settings_doctor_copy_failed')); }
          });
          fix.append(copy);
          card.append(fix);
        }
        result.append(card);
      }
    } catch (_) {
      result.replaceChildren();
      addText(result, 'settings-note settings-note-warn', t('settings_doctor_failed'));
    } finally {
      runButton.disabled = false;
    }
  }

  runButton.addEventListener('click', runDoctor);
})();

(function () {
  const resizer  = document.getElementById('sidebar-resizer');
  const sidebar  = document.getElementById('session-list');
  if (!resizer || !sidebar) return;

  const STORAGE_KEY = 'ai_cli_hub_sidebar_width';
  const MIN = 160, MAX = 520;

  const saved = parseInt(localStorage.getItem(STORAGE_KEY), 10);
  if (saved >= MIN && saved <= MAX) sidebar.style.width = saved + 'px';

  let startX = 0, startW = 0;

  function onMove(e) {
    const dx = (e.clientX || (e.touches && e.touches[0].clientX) || 0) - startX;
    const w = Math.min(MAX, Math.max(MIN, startW + dx));
    sidebar.style.width = w + 'px';
    try { localStorage.setItem(STORAGE_KEY, String(w)); } catch (_) {}
    renderSessionList();
    // ターミナルの幅変化に追従
    terminals.forEach((t, id) => {
      if (!canFitTerminal(t)) return;
      const prevCols = t.term.cols;
      const prevRows = t.term.rows;
      fitTerminalPreservingBottom(t, id);
      if (t.term.cols !== prevCols || t.term.rows !== prevRows) {
        sendResize(id, t.term.cols, t.term.rows, 'app-refit-all');
      }
    });
  }

  function onUp() {
    resizer.classList.remove('dragging');
    document.removeEventListener('mousemove', onMove);
    document.removeEventListener('mouseup', onUp);
    document.body.style.cursor = '';
    document.body.style.userSelect = '';
  }

  resizer.addEventListener('mousedown', (e) => {
    e.preventDefault();
    startX = e.clientX;
    startW = sidebar.getBoundingClientRect().width;
    resizer.classList.add('dragging');
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  });
})();

// moved to /app/voice.js

// ---- スラッシュコマンドピッカー ----
(function () {
  const pickerEl       = document.getElementById('slash-picker');
  const titleEl        = document.getElementById('slash-picker-title');
  const timeEl         = document.getElementById('slash-picker-time');
  const searchEl       = document.getElementById('slash-picker-search');
  const listEl         = document.getElementById('slash-picker-list');
  const refreshBtn     = document.getElementById('slash-picker-refresh');
  const closeBtn       = document.getElementById('slash-picker-close');
  const pickerBtn      = document.getElementById('slash-picker-btn');
  if (!pickerEl || !pickerBtn) return;

  let pickerProvider = null;
  let pickerSessionId = null;
  let pickerData     = null; // { cmds, fetched_at, source_url }

  pickerBtn.addEventListener('click', async () => {
    if (!pickerEl.hidden) { hidePicker(); return; }
    const sess = sessions.get(activeSessionId);
    const provider = sess?.provider || 'claude';
    await openPicker(provider, activeSessionId, false);
  });

  refreshBtn.addEventListener('click', async (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (pickerProvider) await openPicker(pickerProvider, pickerSessionId, true);
  });

  closeBtn.addEventListener('click', (e) => {
    e.preventDefault();
    e.stopPropagation();
    hidePicker();
  });

  searchEl.addEventListener('input', () => renderList(searchEl.value));

  searchEl.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') { hidePicker(); e.preventDefault(); }
  });

  document.addEventListener('mousedown', (e) => {
    if (!pickerEl.hidden && !pickerEl.contains(e.target) && e.target !== pickerBtn) {
      hidePicker();
    }
  });

  async function openPicker(provider, sessionId, forceRefresh) {
    pickerProvider = provider;
    pickerSessionId = sessionId || null;
    pickerEl.hidden = false;
    titleEl.textContent = provider === 'claude' ? 'Claude Code'
                        : provider === 'copilot' ? 'GitHub Copilot'
                        : provider === 'cursor-agent' ? 'Cursor Agent'
                        : 'Codex CLI';
    timeEl.textContent  = '';
    listEl.innerHTML = `<div class="slash-picker-status">${t('slash_picker_loading')}</div>`;
    searchEl.value = '';
    try {
      const method = forceRefresh ? 'POST' : 'GET';
      const sidParam = sessionId ? `&session_id=${encodeURIComponent(sessionId)}` : '';
      const resp = await fetch(`/api/slash-commands?provider=${provider}${sidParam}&token=${token}`, { method });
      if (!resp.ok) {
        const txt = await resp.text();
        if (resp.status === 404) {
          listEl.innerHTML = `<div class="slash-picker-status slash-picker-status--warn">${t('slash_picker_not_configured')}</div>`;
        } else {
          listEl.innerHTML = `<div class="slash-picker-status slash-picker-status--error">${t('slash_picker_error')}</div>`;
        }
        return;
      }
      pickerData = await resp.json();
      setSlashCmdCache(provider, pickerData.cmds, sessionId); // /入力補完と一覧を共有
      timeEl.textContent = formatAge(pickerData.fetched_at);
      renderList('');
      setTimeout(() => searchEl.focus(), 0);
    } catch (_) {
      listEl.innerHTML = `<div class="slash-picker-status slash-picker-status--error">${t('slash_picker_error')}</div>`;
    }
  }

  function renderList(filter) {
    if (!pickerData) return;
    const cmds = pickerData.cmds || [];
    const q = filter.trim().toLowerCase();
    const filtered = q
      ? cmds.filter(c => c.cmd.includes(q) || (c.desc || '').toLowerCase().includes(q))
      : cmds;
    if (filtered.length === 0) {
      listEl.innerHTML = `<div class="slash-picker-status">${t('slash_picker_empty')}</div>`;
      return;
    }
    listEl.innerHTML = '';
    for (const item of filtered) {
      const div = document.createElement('div');
      div.className = 'slash-picker-item';
      const cmdSpan = document.createElement('span');
      cmdSpan.className = 'slash-picker-cmd';
      cmdSpan.textContent = item.cmd;
      const descSpan = document.createElement('span');
      descSpan.className = 'slash-picker-desc';
      descSpan.textContent = item.desc || '';
      if (item.desc) descSpan.title = item.desc;
      div.appendChild(cmdSpan);
      div.appendChild(descSpan);
      div.addEventListener('mousedown', (e) => {
        e.preventDefault();
        if (activeSessionId !== null) sendQuickCommand(activeSessionId, item.cmd);
        hidePicker();
      });
      listEl.appendChild(div);
    }
  }

  function hidePicker() {
    pickerEl.hidden = true;
    pickerData = null;
  }

  function formatAge(iso) {
    if (!iso) return '';
    const diffMs = Date.now() - new Date(iso).getTime();
    const m = Math.floor(diffMs / 60000);
    if (m < 1)  return t('slash_picker_just_now');
    if (m < 60) return t('slash_picker_ago_min').replace('{n}', String(m));
    const h = Math.floor(m / 60);
    if (h < 24) return t('slash_picker_ago_hour').replace('{n}', String(h));
    return t('slash_picker_ago_day').replace('{n}', String(Math.floor(h / 24)));
  }
})();

// moved to /app/files-view.js

// moved to /app/git-view.js


// --- ESM cross-module setters (generated) ---
export function set__userAvatarUrl(v) { _userAvatarUrl = v; }
export function set__userDisplayName(v) { _userDisplayName = v; }
export function set_pasteCounter(v) { pasteCounter = v; }
export function set_voiceActive(v) { voiceActive = v; }
export function set_voiceAudioActive(v) { voiceAudioActive = v; }

// --- ESM window-interop publish (generated; preserves dynamic window.* lookups) ---
window.dismissSession = dismissSession;
