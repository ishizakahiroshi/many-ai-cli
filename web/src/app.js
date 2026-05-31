// --- ESM imports (generated) ---
import { t } from './i18n.js';
import { copyCleanText, showToast, token } from './app/util.js';
import { DEFAULT_VOICE_GRACE_SEC, STORAGE_APPROVAL_AUTO_SWITCH_KEY, STORAGE_NOTIFY_SOUND_CUSTOM_KEY, STORAGE_TOOLS_LEFT_KEY, _putUserPrefsNow, getDefaultTriggerPhrase, getDefaultWakeWordPhrase, setUserPref } from './app/user-prefs.js';
import { DOUBLE_SEND_GUARD_MS, actionBarFocusIdx, actionBarShownAt, activeSessionId, approvalAutoSwitchQueue, approvalConsumedSig, approvalConsumedSigDeleteTimer, approvalRawOptionsCache, approvalSig, approvalSourceCache, approvalSuppressUntil, approvalSwitchCandidates, approvalVisibleCache, autoDismissTimers, batchSelections, composeEndSendTimer, crunchLatch, isComposing, lastDoSendAt, maybeAutoSwitchToNextApproval, multiQuestionDismissedCache, multiQuestionVisibleCache, pendingSend, removeApprovalAutoSwitchTarget, removeFromSessionOrder, sequentialChoiceCache, sessionInputState, sessions, set_actionBarFocusIdx, set_activeSessionId, set_composeEndSendTimer, set_isComposing, set_lastDoSendAt, set_pendingSend, terminals, toolOutputs } from './app/state.js';
import { activateSession, render, renderSessionList, switchSessionByTab } from './app/session-list.js';
import { appendLinkedText } from './app/path-links.js';
import { canFitTerminal, fitTerminalPreservingBottom, isTerminalAtBottom, refitActiveTerminalAfterLayout, refitAndStickTerminalToBottomAfterLayoutSettles, scrollTerminalToBottomSoon, sendResize, updateScrollLockBtn } from './app/terminal.js';
import { DEFAULT_QUICK_CMD_1, DEFAULT_QUICK_CMD_2, appConfirm, appConfirmShutdown, applyFontSize, applyLang, applyTheme, getActiveTriggerPhrase, getQuickCommand, loadApprovalSettings, loadSlashCmdSources, loadUsageLinkSettings, saveUsageLinkSettings, sessionLazyLoaded, sessionViewMode, stripTrailingTriggerPhrase, textEndsWithTriggerPhrase, updateChatCountBadge } from './app/settings.js';
import { ws } from './app/ws-client.js';
import { setMultiQuestionBannerVisible } from './app/approval-ui.js';
import { approvalCheckTimers, approvalSuppressRescanTimers, cancelApprovalHintConfirm, clearSequentialChoiceState, detectApproval, getActionBarButtons, handleBatchNumberKey, hideActionBar, isBatchActionBarVisible, maybeSendDirectApprovalConsumed, moveBatchFocus, sendBatchChoices, setActionBarFocus } from './app/approval.js';
import { chatHistoryCommitOutput, mountChatPaneForSession, onChatHistorySessionRemoved, pushMessage, resetAllChatHistory, resetChatHistoryForSession, scrollChatPaneToBottomSoon } from './app/chat-history.js';
import { attachThumbnails, flushPendingAttach, pendingAttachFiles, updateAttachClearBtn } from './app/attachments.js';
import { FilesTabManager } from './app/files-view.js';

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
});


// moved to /app/approval.js


export function renderToolOutputs(id) {
  const panel = document.getElementById('tool-outputs-panel');
  const list = document.getElementById('tool-outputs-list');
  const countEl = document.getElementById('tool-outputs-count');
  if (!panel || !list) return;

  const outputs = toolOutputs.get(id) || [];
  if (outputs.length === 0) {
    panel.hidden = true;
    return;
  }

  panel.hidden = false;
  if (countEl) countEl.textContent = t('tool_outputs_count', { n: outputs.length });

  list.innerHTML = '';
  outputs.forEach((out, idx) => {
    const item = document.createElement('details');
    item.className = 'tool-output-item';
    if (idx === 0) item.open = true; // 最新は展開して表示

    const summary = document.createElement('summary');
    summary.textContent = t('tool_output_summary', { ts: out.ts, n: out.lines.length });
    item.appendChild(summary);

    const body = document.createElement('div');
    body.className = 'tool-output-body';

    const copyBtn = document.createElement('button');
    copyBtn.type = 'button';
    copyBtn.className = 'tool-output-copy';
    copyBtn.textContent = '⧉';
    copyBtn.title = t('copy_to_clipboard');
    copyBtn.setAttribute('aria-label', t('copy_to_clipboard'));
    copyBtn.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      copyCleanText(out.lines, copyBtn).catch(() => {});
    });
    body.appendChild(copyBtn);

    const pre = document.createElement('pre');
    pre.className = 'tool-output-content';
    appendLinkedText(pre, out.lines.join('\n'), id);
    body.appendChild(pre);
    item.appendChild(body);

    list.appendChild(item);
  });
}

// ---- 入力バー ----

export const inputEl = document.getElementById('input');
export const inputClearBtn = document.getElementById('input-clear-btn');
export const pasteChipsEl = document.getElementById('paste-chips');

// ペースト折りたたみ状態
export const pastedTexts = []; // [{id, text, lineCount}]
export let pasteCounter = 0;

export function autoExpand() {
  const t = activeSessionId === null ? null : terminals.get(activeSessionId);
  const shouldStickToBottom = !!(t && (t.autoScroll || isTerminalAtBottom(t)));
  inputEl.style.height = 'auto';
  inputEl.style.height = Math.min(inputEl.scrollHeight, Math.floor(window.innerHeight * 0.3)) + 'px';
  updateInputClearButton();
  refitActiveTerminalAfterLayout(shouldStickToBottom);
}

export function updateInputClearButton() {
  inputClearBtn?.classList.toggle('has-text', inputEl.value.length > 0);
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

export function expandPasteChip(idx) {
  const pt = pastedTexts[idx];
  if (!pt) return;
  inputEl.value = pt.text + (inputEl.value ? '\n' + inputEl.value : '');
  pastedTexts.splice(idx, 1);
  renderPasteChips();
  autoExpand();
  inputEl.focus();
}

export function removePasteChip(idx) {
  pastedTexts.splice(idx, 1);
  renderPasteChips();
}

export function clearAllPastes() {
  pastedTexts.length = 0;
  renderPasteChips();
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
  set_lastDoSendAt(Date.now());
  const injects = await flushPendingAttach(sessionId);
  const injectPrefix = injects.join('');
  let rawText = buildSendText();
  // トリガーフレーズを末尾から除去（PTY・AI には送らない）
  const _tp = getActiveTriggerPhrase();
  if (_tp && textEndsWithTriggerPhrase(rawText, _tp)) {
    rawText = stripTrailingTriggerPhrase(rawText, _tp);
  }
  // 改行を含む場合はブラケットペーストモードでラップ（\n が途中 Enter と解釈されるのを防ぐ）
  // ブラケットペーストはテキスト部分のみに適用し、injectPrefix は前置する
  let textPart;
  if (rawText === '' && injectPrefix !== '') {
    // 画像のみ（テキストなし）: inject 末尾の \r or スペースで確定済み → 追加の \r で送信
    textPart = '\r';
  } else if (rawText.includes('\n')) {
    textPart = '\x1b[200~' + rawText + '\x1b[201~\r';
  } else {
    textPart = rawText + '\r';
  }
  const textToSend = injectPrefix + textPart;
  clearInput();
  hideSlashMenu();
  // 送信したら次のプロンプトは別物の可能性があるため dismiss フラグをクリア
  multiQuestionDismissedCache.delete(sessionId);
  // テキスト送信で承認ポップアップをバイパスした場合、Ink 再描画による
  // 同一選択肢の再検出・再表示を防ぐため消費済み署名を保存する
  const prevOpts = approvalRawOptionsCache.get(sessionId);
  if (prevOpts) approvalConsumedSig.set(sessionId, approvalSig(prevOpts));
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
}

export function saveInputStateFor(id) {
  if (id === null) return;
  const frag = document.createDocumentFragment();
  if (attachThumbnails) {
    while (attachThumbnails.firstChild) frag.appendChild(attachThumbnails.firstChild);
  }
  sessionInputState.set(id, {
    inputValue: inputEl.value,
    pastedTextsData: [...pastedTexts],
    pendingAttachFiles: [...pendingAttachFiles],
    thumbsFragment: frag,
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
      attachThumbnails.innerHTML = '';
      if (state.thumbsFragment) attachThumbnails.appendChild(state.thumbsFragment);
    }
  } else {
    inputEl.value = '';
    inputEl.style.height = 'auto';
    pastedTexts.length = 0;
    pendingAttachFiles.length = 0;
    if (attachThumbnails) attachThumbnails.innerHTML = '';
  }
  autoExpand();
  renderPasteChips();
  updateAttachClearBtn();
}

export function cleanupSessionInputState(id) {
  const state = sessionInputState.get(id);
  if (!state) return;
  if (state.thumbsFragment) {
    state.thumbsFragment.querySelectorAll('img').forEach(img => {
      if (img.src.startsWith('blob:')) URL.revokeObjectURL(img.src);
    });
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

export function getSlashCommands() {
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

export const slashMenuEl = document.getElementById('slash-menu');
export let slashItems = [];
export let slashIndex = -1;

export function updateSlashMenu() {
  const val = inputEl.value;
  if (!val.startsWith('/')) { hideSlashMenu(); return; }
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
  autoExpand(); updateSlashMenu();
  updateInputClearButton();
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
  if (inputEl.value === '' && !e.isComposing && isBatchActionBarVisible()) {
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
    if (e.key === 'Enter' && !e.shiftKey) {
      sendBatchChoices(activeSessionId);
      e.preventDefault(); return;
    }
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
      navigator.clipboard.writeText(xt.term.getSelection()).catch(() => {});
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
  inputEl.focus();
});

document.getElementById('send-btn').addEventListener('mousedown', () => {
  // クリック時に IME が確定中の場合、compositionend 後に送信するよう予約
  if (isComposing) set_pendingSend(true);
});
document.getElementById('send-btn').addEventListener('click', () => {
  if (activeSessionId === null) return;
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

document.getElementById('quick-clear-btn').addEventListener('click', () => {
  if (activeSessionId === null) return;
  sendQuickCommand(activeSessionId, getQuickCommand(1));
});

document.getElementById('quick-model-btn').addEventListener('click', () => {
  if (activeSessionId === null) return;
  sendQuickCommand(activeSessionId, getQuickCommand(2));
});

export function sendQuickCommand(sessionId, cmd) {
  // Ollama route セッションで /model 始まりはブロック（quick-model-btn 経由含む）
  if (isOllamaModelCommandBlocked(sessionId, cmd)) {
    showToast(t('toast_model_blocked_on_ollama'));
    return;
  }
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
  sendSubmittedText(sessionId, `${cmd}\r`);
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
  toolOutputs.clear();
  approvalVisibleCache.clear();
  multiQuestionVisibleCache.clear();
  multiQuestionDismissedCache.clear();
  sequentialChoiceCache.clear();
  approvalRawOptionsCache.clear();
  approvalSourceCache.clear();
  approvalConsumedSigDeleteTimer.forEach(t => clearTimeout(t));
  approvalConsumedSigDeleteTimer.clear();
  approvalConsumedSig.clear();
  approvalSwitchCandidates.clear();
  batchSelections.clear();
  approvalSuppressUntil.clear();
  approvalAutoSwitchQueue.length = 0;
  resetAllChatHistory();
  hideActionBar(undefined);
  setMultiQuestionBannerVisible(false);
  if (activeSessionId !== null) {
    renderToolOutputs(activeSessionId);
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
  toolOutputs.delete(id);
  approvalVisibleCache.delete(id);
  if (multiQuestionVisibleCache.delete(id) && id === activeSessionId) {
    setMultiQuestionBannerVisible(false);
  }
  multiQuestionDismissedCache.delete(id);
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
    renderToolOutputs(id);
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
  try { if (typeof crunchLatch !== 'undefined') crunchLatch.delete(id); } catch (_) {}
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
  cleanupRemovedSessionState(id);
  try {
    const mgr = window.multiPaneManager;
    if (mgr && typeof mgr.onSessionRemoved === 'function') mgr.onSessionRemoved(id);
  } catch (_) {}
  // sessions.delete より前に git/files タブの付け替えを試みる
  // （onSessionRemoved 内で sessionsRef を引いて代替を探すため、削除前の方が探しやすい）
  try { FilesTabManager.onSessionRemoved(id); } catch (_) {}
  // C1/C2: チャット履歴 store とビューモード state をクリーンアップ
  try { if (typeof onChatHistorySessionRemoved === 'function') onChatHistorySessionRemoved(id); } catch (_) {}
  try { if (typeof sessionViewMode !== 'undefined') sessionViewMode.delete(id); } catch (_) {}
  try { if (typeof sessionLazyLoaded !== 'undefined') sessionLazyLoaded.delete(id); } catch (_) {}
  sessions.delete(id);
  removeFromSessionOrder(id);
  const t = terminals.get(id);
  if (t) { try { t.term.dispose(); } catch (_) {} terminals.delete(id); }
  toolOutputs.delete(id);
  approvalVisibleCache.delete(id);
  if (multiQuestionVisibleCache.delete(id) && id === activeSessionId) {
    setMultiQuestionBannerVisible(false);
  }
  multiQuestionDismissedCache.delete(id);
  removeApprovalAutoSwitchTarget(id);
  approvalRawOptionsCache.delete(id);
  approvalSourceCache.delete(id);
  approvalConsumedSig.delete(id);
  batchSelections.delete(id);
  clearSequentialChoiceState(id);
  cancelApprovalHintConfirm(id);
  approvalSuppressUntil.delete(id);
  cleanupSessionInputState(id);
  onChatHistorySessionRemoved(id);
  if (activeSessionId === id) {
    set_activeSessionId(null);
    const area = document.getElementById('terminal-area');
    if (area) area.innerHTML = '';
    hideActionBar(undefined);
    setMultiQuestionBannerVisible(false);
    if (sessions.size > 0) {
      activateSession(sessions.keys().next().value);
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

// ---- ツール出力パネル 折りたたみ / 閉じるボタン ----

export const collapseOutputsBtn = document.getElementById('tool-outputs-collapse');
if (collapseOutputsBtn) {
  collapseOutputsBtn.addEventListener('click', () => {
    const panel = document.getElementById('tool-outputs-panel');
    if (!panel) return;
    const collapsed = panel.classList.toggle('collapsed');
    collapseOutputsBtn.textContent = collapsed ? '▶' : '▼';
  });
}

export const clearOutputsBtn = document.getElementById('tool-outputs-clear');
if (clearOutputsBtn) {
  clearOutputsBtn.addEventListener('click', () => {
    if (activeSessionId !== null) {
      toolOutputs.delete(activeSessionId);
      renderToolOutputs(activeSessionId);
    }
  });
}



// moved to /app/spawn-panel.js

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
    const result = await appConfirmShutdown();
    if (!result) return;
    shutdownBtn.disabled = true;
    if (result === 'sessions') {
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
  const logMaxSizeEl               = document.getElementById('log-max-size');
  const logMaxBackupsEl            = document.getElementById('log-max-backups');
  const logSessionRetentionDaysEl  = document.getElementById('log-session-retention-days');
  const logSessionMaxSizeEl        = document.getElementById('log-session-max-size');

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
      logMaxSizeEl.value    = cfg.max_size_mb;
      logMaxBackupsEl.value = cfg.max_backups;
      if (logSessionRetentionDaysEl) logSessionRetentionDaysEl.value = cfg.session_retention_days ?? 7;
      if (logSessionMaxSizeEl) logSessionMaxSizeEl.value = cfg.session_max_size_mb ?? 50;
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

  (function () {
    const KEY = 'any-ai-cli.settings-section-state';
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
          max_size_mb:             parseInt(logMaxSizeEl.value, 10) || 10,
          max_backups:             parseInt(logMaxBackupsEl.value, 10) || 3,
          compress:                false,
          session_retention_days:  parseInt(logSessionRetentionDaysEl?.value ?? '7', 10) || 7,
          session_max_size_mb:     parseInt(logSessionMaxSizeEl?.value ?? '50', 10),
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
    setUserPref('notify_sound.enabled', false);
    setUserPref('notify_sound.type', 'default');
    try { localStorage.removeItem(STORAGE_NOTIFY_SOUND_CUSTOM_KEY); } catch (_) {}
    setUserPref('approval.auto_switch', false);
    setUserPref('quick_cmds.cmd1', DEFAULT_QUICK_CMD_1);
    setUserPref('quick_cmds.cmd2', DEFAULT_QUICK_CMD_2);
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

    const quickCmd1El = document.getElementById('quick-cmd-1');
    const quickCmd2El = document.getElementById('quick-cmd-2');
    if (quickCmd1El) quickCmd1El.value = DEFAULT_QUICK_CMD_1;
    if (quickCmd2El) quickCmd2El.value = DEFAULT_QUICK_CMD_2;

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
    if (logMaxSizeEl) logMaxSizeEl.value = '10';
    if (logMaxBackupsEl) logMaxBackupsEl.value = '3';
    const logSessionRetentionDaysEl2 = document.getElementById('log-session-retention-days');
    if (logSessionRetentionDaysEl2) logSessionRetentionDaysEl2.value = '7';
    const logSessionMaxSizeEl2 = document.getElementById('log-session-max-size');
    if (logSessionMaxSizeEl2) logSessionMaxSizeEl2.value = '50';

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

    const foAppEl = document.getElementById('settings-file-open-app');
    const termAppEl = document.getElementById('settings-terminal-app');
    if (foAppEl) foAppEl.value = '';
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

  // 設定パネルが開かれたときにログ設定を読み込む
  document.getElementById('settings-btn').addEventListener('click', () => {
    if (!document.getElementById('settings-panel').hidden) {
      loadIdleTimeout();
      loadReconnectGrace();
      loadLogConfig();
      loadApprovalSettings();
      loadSlashCmdSources();
      loadUsageLinkSettings();
    }
  });

  if (idleTimeoutEl) idleTimeoutEl.addEventListener('change', saveIdleTimeout);
  if (reconnectGraceEl) reconnectGraceEl.addEventListener('change', saveReconnectGrace);
  logEnabledEl.addEventListener('change', saveLogConfig);
  logMaxSizeEl.addEventListener('change', saveLogConfig);
  logMaxBackupsEl.addEventListener('change', saveLogConfig);
  if (logSessionRetentionDaysEl) logSessionRetentionDaysEl.addEventListener('change', saveLogConfig);
  if (logSessionMaxSizeEl) logSessionMaxSizeEl.addEventListener('change', saveLogConfig);
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
    try { localStorage.setItem(STORAGE_KEY, w); } catch (_) {}
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
    if (m < 60) return t('slash_picker_ago_min').replace('{n}', m);
    const h = Math.floor(m / 60);
    if (h < 24) return t('slash_picker_ago_hour').replace('{n}', h);
    return t('slash_picker_ago_day').replace('{n}', Math.floor(h / 24));
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
