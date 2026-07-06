// --- ESM imports (generated) ---
import { t } from './i18n.js';
import { cleanCopiedText, showToast, token } from './app/util.js';
import { DEFAULT_VOICE_GRACE_SEC, STORAGE_APPROVAL_AUTO_SWITCH_KEY, STORAGE_NOTIFY_SOUND_CUSTOM_KEY, STORAGE_TOOLS_LEFT_KEY, STORAGE_VOICE_WHISPER_AUTO_SUBMIT_KEY, _putUserPrefsNow, getDefaultTriggerPhrase, getDefaultWakeWordPhrase, setUserPref, setVoiceEngine } from './app/user-prefs.js';
import { DOUBLE_SEND_GUARD_MS, actionBarFocusIdx, actionBarShownAt, activeSessionId, answeredMarkerSigs, recordAnsweredMarkerSig, approvalAutoSwitchQueue, approvalConsumedSig, approvalConsumedSigDeleteTimer, approvalRawOptionsCache, approvalSig, approvalSourceCache, approvalSuppressUntil, approvalSwitchCandidates, approvalVisibleCache, autoDismissTimers, batchSelections, composeEndSendTimer, isComposing, lastDoSendAt, maybeAutoSwitchToNextApproval, multiQuestionDismissedCache, multiQuestionLatchAt, multiQuestionVisibleCache, pendingSend, removeApprovalAutoSwitchTarget, removeFromSessionOrder, sequentialChoiceCache, sessionInputState, sessions, set_actionBarFocusIdx, set_activeSessionId, set_composeEndSendTimer, set_isComposing, set_lastDoSendAt, set_pendingSend, terminals } from './app/state.js';
import { activateSession, render, renderSessionList, switchSessionByTab } from './app/session-list.js';
import { orderSessions } from './app/state.js';
import { canFitTerminal, fitTerminalPreservingBottom, isTerminalAtBottom, refitActiveTerminalAfterLayout, refitAndStickTerminalToBottomAfterLayoutSettles, resumeTerminalBottomFollow, scrollTerminalToBottomSoon, sendResize, suppressPtyResizeForInputLayout, updateScrollLockBtn } from './app/terminal.js';
import { QUICK_CMD_SLOTS, appConfirm, appConfirmShutdown, appLegacyResetNotice, applyFontSize, applyLang, applyTheme, attachDoneSummaryNotifyToggle, attachTokenStatusbarToggle, getActiveTriggerPhrase, getQuickCommand, loadApprovalSettings, loadSlashCmdSources, loadUsageLinkSettings, quickCommandButtonId, quickCommandDefault, saveUsageLinkSettings, sessionLazyLoaded, sessionViewMode, stripTrailingTriggerPhrase, textEndsWithTriggerPhrase, updateChatCountBadge } from './app/settings.js';
import { ws } from './app/ws-client.js';
import { setMultiQuestionBannerVisible } from './app/approval-ui.js';
import { scheduleDeferredEnter, scheduleAfterOutputSettle } from './app/deferred-enter.js';
import { approvalCheckTimers, approvalSuppressRescanTimers, cancelApprovalHintConfirm, clearSequentialChoiceState, detectApproval, getActionBarButtons, handleBatchNumberKey, handleMultiSelectNumberKey, hideActionBar, isBatchActionBarVisible, isMultiSelectActionBarVisible, isSelectMenuActive, isShellProvider, maybeSendDirectApprovalConsumed, moveBatchFocus, moveMultiSelectFocus, openBatchConfirm, sendMultiSelectChoices, setActionBarFocus, toggleMultiSelectFocused } from './app/approval.js';
import { chatHistoryCommitOutput, mountChatPaneForSession, onChatHistorySessionRemoved, pushMessage, resetAllChatHistory, resetChatHistoryForSession, scrollChatPaneToBottomSoon } from './app/chat-history.js';
import { attachThumbnails, flushPendingAttach, pendingAttachFiles, updateAttachClearBtn } from './app/attachments.js';
import { FilesTabManager } from './app/files-view.js';
import { getExposeStatus, fetchExposeStatus, disableExpose } from './app/host-expose.js';
import './app/detached-grid-launcher.js';

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


// moved to /app/approval.js


// ---- 入力バー ----

export const inputEl = document.getElementById('input') as HTMLTextAreaElement;
export const inputClearBtn = document.getElementById('input-clear-btn');
export const pasteChipsEl = document.getElementById('paste-chips');

// ペースト折りたたみ状態
export const pastedTexts = []; // [{id, text, lineCount}]
export let pasteCounter = 0;

export function autoExpand(opts: any = {}) {
  const t = activeSessionId === null ? null : terminals.get(activeSessionId);
  const shouldStickToBottom = !!(t && (t.autoScroll || isTerminalAtBottom(t)));
  if (opts.suppressPtyResize) {
    suppressPtyResizeForInputLayout();
  }
  inputEl.style.height = 'auto';
  inputEl.style.height = Math.min(inputEl.scrollHeight, Math.floor(window.innerHeight * 0.3)) + 'px';
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

// 実行中状態に応じて入力欄プレースホルダと送信ボタンの見た目／挙動を再評価する。
// WS の state 更新・セッション切替・i18n 適用後にこれを呼ぶことで停止導線を同期する。
export function updateInputAffordance() {
  const running = isActiveSessionRunning();
  // C1: 実行中は「Esc で停止」、それ以外は通常文言。data-i18n-placeholder の自動適用と
  // 競合しないよう、running 状態を見て JS から明示的に上書きする。
  inputEl.placeholder = running ? t('input_placeholder_running') : t('input_placeholder');
  // C2: 実行中でも入力欄にテキスト/チップ/ファイルがあれば ➤（送信）のまま。
  // 入力が空の場合のみ ■（停止）に切替える。ペースト・ファイル添付直後に送信できずもっさりする問題を解消。
  const hasContent = inputEl.value.length > 0 || pastedTexts.length > 0 || pendingAttachFiles.length > 0;
  const showStop = running && !hasContent;
  const sendBtn = document.getElementById('send-btn');
  if (sendBtn) {
    sendBtn.textContent = showStop ? '■' : '➤';
    sendBtn.classList.toggle('is-stopping', showStop);
    const title = showStop ? t('stop_btn_title') : t('send_btn_title');
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

// Ollama route で起動したセッションでは Claude Code / Codex 側の /model コマンドが
// spawn 時固定の env (ANTHROPIC_BASE_URL=http://localhost:11434 等) と整合しないため
// 純正モデルに切替えるとエラーになる。行頭 /model 入力は送信前にブロックする。
export function isOllamaModelCommandBlocked(sessionId, text) {
  const s = sessions.get(sessionId);
  if (!s || s.route !== 'ollama') return false;
  const trimmed = String(text || '').replace(/^[\s\x00-\x1f]+/, '');
  return /^\/model(\b|\s|$)/i.test(trimmed);
}

// shell（素のシェル）セッション内で AI CLI（claude/codex/copilot/cursor-agent）の
// 起動コマンドを直接打つと、provider=shell 用にチューニングされた入力・承認処理
// （\x15 前置なし・マーカー未注入・shell 用承認検出）と二重ラップになり、スラッシュ
// コマンドの文字化けや承認ボタンの不動作を招く。先頭トークンが起動コマンドのときは
// 検知して provider 名を返す（パス前置・.cmd/.exe 等の拡張子も許容）。該当なしは null。
const AI_CLI_LAUNCH_RE = /^(?:[^\s]*[\\/])?(claude|codex|copilot|cursor-agent)(?:\.(?:cmd|exe|bat|ps1))?(?=\s|$)/i;
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

export async function doSend(sessionId) {
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
  // shell セッション内で AI CLI 起動コマンドを検知 → 専用セッション spawn を誘導。
  // 「開く」を選べば起動コマンドは shell へ送らず spawn パネルへ切り替える（二重起動防止）。
  // flushPendingAttach より前に判定し、誘導採択時に画像 inject を無駄に消費しないようにする。
  if (await maybeNudgeAiCliLaunchInShell(sessionId, buildSendText())) {
    return;
  }
  set_lastDoSendAt(Date.now());
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
  // 改行を含む場合はブラケットペーストモードでラップ（\n が途中 Enter と解釈されるのを防ぐ）
  // ブラケットペーストはテキスト部分のみに適用し、injectPrefix は前置する
  let textPart;
  // 確定 \r をブラケットペースト終端 \x1b[201~ と同一書き込みに含めると、内側 CLI
  // （Claude Code v2 等）が大きい/複数行ペーストを [Pasted text #N] に畳み込む処理中に
  // 直後の \r を吸収し、確定キーとして登録されない（pasted text が入力欄に張り付いたまま
  // 送信されない）。複数行のときは確定 \r を本文と別書き込みに分離し、畳み込み確定後に送る。
  let deferEnter = false;
  if (rawText === '' && injectPrefix !== '') {
    // 画像のみ（テキストなし）: inject 末尾の \r or スペースで確定済み → 追加の \r で送信
    textPart = '\r';
  } else if (rawText.includes('\n')) {
    textPart = '\x1b[200~' + rawText + '\x1b[201~';
    deferEnter = true;
  } else {
    textPart = rawText + '\r';
  }
  // 内側 CLI がビジー等で入力行に残骸（前回の未確定入力）があると、本送信が
  // その後ろへ連結されてしまう（例: "残骸質問"）。sendQuickCommand と同じく
  // \x15(Ctrl+U) を先頭に置き、inject/本文を送る前に入力行を一度クリアする。
  // 入力行が空なら no-op なので無害。claude/codex/copilot/cursor 共通に送る。
  // 画像 inject（@path）を複数行ペーストに前置すると、@path エコー由来の早期 idle で確定 \r が
  // 前倒し発火し、内側 CLI がペースト取り込み中（「Pasting…」）のまま \r を吸収して固着する。
  // injectPrefix がある複数行ペーストでは、まず画像 inject だけ送り、取り込みが落ち着いてから
  // ペースト本体＋確定 \r を送る（下記 needPasteSplit 分岐）。それ以外は従来通り 1 書き込みにまとめる。
  // ただし shell provider（素のシェル）は Ink 入力欄を持たずシェル自身が行編集を担うため、
  // \x15 を本文と同一書き込みで前置すると PowerShell 等で行クリアにならずリテラル ^U が混入し
  // コマンドが壊れる（例: codex → ^Ucodex）。shell 宛ては前置せずそのまま送る。
  const clearPrefix = isShellProvider(sessions.get(sessionId)?.provider || '') ? '' : '\x15';
  const needPasteSplit = deferEnter && injectPrefix !== '';
  const textToSend = needPasteSplit ? (clearPrefix + injectPrefix) : (clearPrefix + injectPrefix + textPart);
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
  if (rawText && rawText !== '') {
    pushMessage(sessionId, { role: 'user', kind: 'text', rawText });
  }
  if (sessionId === activeSessionId) {
    // 送信後は新しいターンを見る意図なので、chat/split 側もレイアウト確定まで末尾へ張り付かせる。
    scrollChatPaneToBottomSoon({ passes: 4, startedAt: Date.now() });
  }
  sendSubmittedText(sessionId, textToSend);
  if (needPasteSplit) {
    // 段1: 画像 inject（@path）の取り込み（[Image #N] 畳み込み・@ 補完ポップアップ閉じ）が
    // 出力静止で落ち着くのを待ち、段2: ペースト本体を送り、段3: 確定 \r を予約する。確定 \r の
    // 予約をペースト送出後まで遅らせることで、@path エコー由来の早期静止で \r が前倒し発火して
    // 「Pasting…」固着するのを断つ。ペースト送出以降は画像なし複数行ペーストと同一経路で確定する。
    scheduleAfterOutputSettle(sessionId, () => {
      sendText(sessionId, textPart);
      scheduleDeferredEnter(sessionId);
    });
  } else if (deferEnter) {
    // 複数行ペーストの確定 \r は、内側 CLI の畳み込み・再描画が落ち着いてから別書き込みで送る。
    // 同一書き込みに含めると \r が吸収され送信されない。固定遅延では大きなペーストの取り込み時間を
    // 当てられず取りこぼすため、PTY 出力が静止するのを待ってから 1 回だけ送る（deferred-enter.ts）。
    scheduleDeferredEnter(sessionId);
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
export const slashCmdDynamic = new Map(); // provider -> [{cmd, desc}]
const slashCmdRetryAfter = new Map();     // provider -> epoch ms（失敗時の再試行抑止）
const slashCmdLoading = new Set();        // 取得中の provider

function activeProvider() {
  return sessions.get(activeSessionId)?.provider || 'claude';
}

// ピッカー／/入力補完の双方から呼べるキャッシュ充填。
export function setSlashCmdCache(provider, cmds) {
  const list = (cmds || []).filter(c => c && c.cmd);
  if (list.length > 0) {
    slashCmdDynamic.set(provider, list);
    slashCmdRetryAfter.delete(provider);
  }
}

// /入力補完用にフルリストを遅延取得する。取得済み・取得中・抑止中は何もしない。
// 取得完了時、メニューが開いていれば再描画する。
async function ensureSlashCommands(provider) {
  if (slashCmdDynamic.has(provider) || slashCmdLoading.has(provider)) return;
  const retryAt = slashCmdRetryAfter.get(provider) || 0;
  if (Date.now() < retryAt) return;
  slashCmdLoading.add(provider);
  try {
    const resp = await fetch(`/api/slash-commands?provider=${provider}&token=${token}`);
    if (!resp.ok) { slashCmdRetryAfter.set(provider, Date.now() + 60_000); return; }
    const data = await resp.json();
    setSlashCmdCache(provider, data.cmds);
    if (slashCmdDynamic.has(provider) && !slashMenuEl.hidden) updateSlashMenu();
  } catch (_) {
    slashCmdRetryAfter.set(provider, Date.now() + 60_000);
  } finally {
    slashCmdLoading.delete(provider);
  }
}

export function getSlashCommands() {
  const dyn = slashCmdDynamic.get(activeProvider());
  if (dyn && dyn.length > 0) return dyn;
  return getSlashCommandsFallback();
}

export const slashMenuEl = document.getElementById('slash-menu');
export let slashItems = [];
export let slashIndex = -1;

export function updateSlashMenu() {
  const val = inputEl.value;
  if (!val.startsWith('/')) { hideSlashMenu(); return; }
  ensureSlashCommands(activeProvider()); // 非同期: 取得完了時に自動で再描画
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
  // /clear と /model はクイック実行用途のため、候補選択時に即送信する
  if (activeSessionId !== null && (cmd === '/clear' || cmd === '/model')) {
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
    // 入力テキストあり + 矢印キーはブラウザのカーソル移動に委譲する
    if (inputEl.value !== '' && (e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'ArrowUp' || e.key === 'ArrowDown')) return;
    sendText(activeSessionId, specialKeys[e.key]);
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
  if (!wrap || !btn || !inputArea || !inputTools) return;

  const applyToolsPosition = (isLeft) => {
    wrap.classList.toggle('tools-left', isLeft);
    const voiceBtn = document.getElementById('voice-btn');
    const sendBtn = document.getElementById('send-btn');
    if (isLeft) {
      wrap.append(btn, inputTools);
      if (voiceBtn) wrap.append(voiceBtn);
      if (sendBtn) wrap.append(sendBtn);
      if (inputClearBtn) wrap.append(inputClearBtn);
      wrap.append(inputArea);
    } else {
      wrap.append(inputArea);
      if (inputClearBtn) wrap.append(inputClearBtn);
      if (sendBtn) wrap.append(sendBtn);
      if (voiceBtn) wrap.append(voiceBtn);
      wrap.append(inputTools, btn);
    }
  };

  applyToolsPosition(localStorage.getItem(STORAGE_TOOLS_LEFT_KEY) === '1');
  btn.addEventListener('click', () => {
    const isLeft = !wrap.classList.contains('tools-left');
    applyToolsPosition(isLeft);
    localStorage.setItem(STORAGE_TOOLS_LEFT_KEY, isLeft ? '1' : '0');
  });
})();

inputClearBtn?.addEventListener('click', () => {
  inputEl.value = '';
  autoExpand();
  updateInputClearButton();
  // Web テキストエリアだけでなく内側 CLI の入力行も消す。ビジー時等に溜まった
  // 残骸（例: "/login/login..."）は web を空にしても TUI 側に残り続けるため、
  // doSend / sendQuickCommand と同じ \x15(Ctrl+U) を単独送信して行クリアする。
  // セッション未選択なら no-op。
  if (activeSessionId !== null) sendText(activeSessionId, '\x15');
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
    sendText(activeSessionId, '\x1b');
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
  if (!hasSession) closeMobileSessionDrawer();
}

export function openMobileSessionDrawer() {
  document.body.classList.add('mobile-drawer-open');
  const btn = document.getElementById('mobile-menu-btn');
  const backdrop = document.getElementById('mobile-drawer-backdrop');
  if (btn) btn.setAttribute('aria-expanded', 'true');
  if (backdrop) backdrop.hidden = false;
}

export function closeMobileSessionDrawer() {
  document.body.classList.remove('mobile-drawer-open');
  const btn = document.getElementById('mobile-menu-btn');
  const backdrop = document.getElementById('mobile-drawer-backdrop');
  if (btn) btn.setAttribute('aria-expanded', 'false');
  if (backdrop) backdrop.hidden = true;
}

window.syncMobileLayoutState = syncMobileLayoutState;
window.closeMobileSessionDrawer = closeMobileSessionDrawer;

(function initMobileControls() {
  const menuBtn = document.getElementById('mobile-menu-btn');
  const backdrop = document.getElementById('mobile-drawer-backdrop');
  const spawnBtn = document.getElementById('mobile-spawn-btn');
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
  if (prevOpts) approvalConsumedSig.set(sessionId, approvalSig(prevOpts));
  hideActionBar(sessionId);
  approvalSuppressUntil.set(sessionId, Date.now() + 2000);
  setTimeout(() => {
    detectApproval(sessionId);
    maybeAutoSwitchToNextApproval();
  }, 2050);
  // 内側 CLI がビジー等で入力行に残骸があると、クイックコマンドが既存テキストへ
  // 連結され（例: "...質問/clear/clear"）独立コマンドにならない。Ctrl+U(\x15) で
  // 入力行を一度クリアしてから送り、連結を物理的に防ぐ。
  // shell provider は doSend と同理由で \x15 を前置しない（PowerShell 等で ^U 混入を防ぐ）。
  const qcPrefix = isShellProvider(sessions.get(sessionId)?.provider || '') ? '' : '\x15';
  sendSubmittedText(sessionId, `${qcPrefix}${cmd}\r`);
  focusInputForTerminalKeys();
}

export function focusInputForTerminalKeys() {
  if (activeSessionId === null || document.activeElement === inputEl) return;
  try {
    inputEl.focus({ preventScroll: true });
  } catch (_) {
    inputEl.focus();
  }
}

export function sendSubmittedText(sessionId, text) {
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
  sendText(sessionId, text);
}

export function sendText(sessionId, text) {
  ws.send(JSON.stringify({ type: 'pty_input', session_id: sessionId, text }));
}

export function requestSessionDismiss(id) {
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
  approvalConsumedSigDeleteTimer.forEach(t => clearTimeout(t));
  approvalConsumedSigDeleteTimer.clear();
  approvalConsumedSig.clear();
  answeredMarkerSigs.clear();
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
  const sigTimer = approvalConsumedSigDeleteTimer.get(id);
  if (sigTimer) clearTimeout(sigTimer);
  approvalConsumedSigDeleteTimer.delete(id);
  approvalConsumedSig.delete(id);
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
  try { if (typeof window._wakewordSessionRemoved === 'function') window._wakewordSessionRemoved(id); } catch (_) {}
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
  approvalConsumedSig.delete(id);
  answeredMarkerSigs.delete(id);
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
    '#topbar'
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
  const killAllBtn = document.getElementById('kill-all-btn');
  if (!killAllBtn) return;

  killAllBtn.addEventListener('click', async () => {
    const ok = await appConfirm({
      title: t('kill_all_confirm_title'),
      message: t('kill_all_confirm'),
      confirmText: t('kill_all_confirm_run'),
      cancelText: t('spawn_cancel'),
      kind: 'danger',
    });
    if (!ok) return;
    killAllBtn.disabled = true;
    try {
      await fetch(`/api/kill-all?token=${token}`, { method: 'POST' });
    } catch (_) {}
    killAllBtn.disabled = false;
  });
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
      const ok = await appConfirm({
        title: t('settings_history_reset_confirm_title'),
        message: t('settings_history_reset_confirm_message'),
        confirmText: t('settings_history_reset_confirm_run'),
        cancelText: t('confirm_cancel'),
        kind: 'danger',
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
        window.dispatchEvent(new CustomEvent('workbench-session-store-reset'));
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
      const ok = await appConfirm({
        title: t('settings_logs_purge_confirm_title'),
        message: t('settings_logs_purge_confirm_message'),
        confirmText: t('settings_history_reset_confirm_run'),
        cancelText: t('confirm_cancel'),
        kind: 'danger',
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
        window.dispatchEvent(new CustomEvent('workbench-session-store-reset'));
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
      const ok = await appConfirm({
        title: t('settings_attachments_purge_confirm_title'),
        message: t('settings_attachments_purge_confirm_message'),
        confirmText: t('settings_history_reset_confirm_run'),
        cancelText: t('confirm_cancel'),
        kind: 'danger',
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
          window.dispatchEvent(new CustomEvent('workbench-session-store-reset'));
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
      loadLogConfig();
      loadApprovalSettings();
      loadSlashCmdSources();
      loadUsageLinkSettings();
      attachTokenStatusbarToggle();
      attachDoneSummaryNotifyToggle();
    }
  });

  if (idleTimeoutEl) idleTimeoutEl.addEventListener('change', saveIdleTimeout);
  if (reconnectGraceEl) reconnectGraceEl.addEventListener('change', saveReconnectGrace);
  logEnabledEl.addEventListener('change', saveLogConfig);
  if (logSessionEnabledEl) logSessionEnabledEl.addEventListener('change', saveLogConfig);
  logMaxSizeEl.addEventListener('change', saveLogConfig);
  logMaxBackupsEl.addEventListener('change', saveLogConfig);
  if (logSessionRetentionDaysEl) logSessionRetentionDaysEl.addEventListener('change', saveLogConfig);
  if (logSessionMaxSizeEl) logSessionMaxSizeEl.addEventListener('change', saveLogConfig);
  if (attachRetentionDaysEl) attachRetentionDaysEl.addEventListener('change', saveLogConfig);
  if (attachMaxTotalMbEl) attachMaxTotalMbEl.addEventListener('change', saveLogConfig);
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
        sendResize(id, t.term.cols, t.term.rows);
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
  let pickerData     = null; // { cmds, fetched_at, source_url }

  pickerBtn.addEventListener('click', async () => {
    if (!pickerEl.hidden) { hidePicker(); return; }
    const sess = sessions.get(activeSessionId);
    const provider = sess?.provider || 'claude';
    await openPicker(provider, false);
  });

  refreshBtn.addEventListener('click', async (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (pickerProvider) await openPicker(pickerProvider, true);
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

  async function openPicker(provider, forceRefresh) {
    pickerProvider = provider;
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
      const resp = await fetch(`/api/slash-commands?provider=${provider}&token=${token}`, { method });
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
      setSlashCmdCache(provider, pickerData.cmds); // /入力補完と一覧を共有
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
