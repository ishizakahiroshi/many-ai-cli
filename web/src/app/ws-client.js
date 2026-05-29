// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- WebSocket 自動再接続（指数バックオフ） ----
let ws = null;
let _wsIntentionalClose = false; // ページ遷移など意図的クローズ時は再接続しない
let _wsRetryDelay = 500; // 初期バックオフ ms
const _wsRetryMax = 10000; // 上限 ms

function syncElapsedTimer() {
  const shouldRun = !!(ws && ws.readyState === WebSocket.OPEN && activeSessionId !== null && !document.hidden);
  if (shouldRun) {
    if (!_elapsedTimerInterval) {
      _elapsedTimerInterval = setInterval(() => updateMainTabStatus(), 1000);
    }
    updateMainTabStatus();
    return;
  }
  if (_elapsedTimerInterval) {
    clearInterval(_elapsedTimerInterval);
    _elapsedTimerInterval = null;
  }
}

document.addEventListener('visibilitychange', syncElapsedTimer);

function _sendRegister() {
  const area = document.getElementById('terminal-area');
  // clientWidth/Height が「ゼロではないが極小（レイアウト未確定 / 親が
  // display:none 直後など）」のときに `0 || 200` のフォールバックが効かず、
  // cols=1〜13 のような不正値が Hub の lastUICols として記録される。
  // その値で spawn された新セッションは極狭幅 PTY で起動し、内部で強制改行
  // された出力を吐く。xterm 側で再 fit しても過去行はリフローされず、
  // 1 行 5〜7 文字のような折り返しが残り続ける症状になる。
  // 安全しきい値未満は既定値（200×50）にフォールバック。
  // 実寸は attach 後 whenLayoutReady → sendResize で送り直される。
  const cw = area ? area.clientWidth : 0;
  const ch = area ? area.clientHeight : 0;
  const cols = cw >= 120 ? Math.floor(cw / 7.5) : 200;
  const rows = ch >= 80  ? Math.floor(ch / 16)  : 50;
  ws.send(JSON.stringify({ type: 'register', role: 'ui', token, cols, rows, ui_active_session_id: activeSessionId || 0 }));
}

function sessionLayoutSnapshot(s) {
  if (!s) return '';
  return [
    s.provider || '',
    s.display_name || '',
    s.cwd || '',
    s.project || '',
    s.branch || '',
    s.label || '',
    s.shell || '',
    s.started_at || '',
    s.first_message || '',
    s.last_message || '',
    s.log_path || '',
    s.jsonl_path || '',
    s.model || '',
    s.route || '',
    s.end_reason || '',
  ].join('\x1f');
}

function _connectWs() {
  const _ws = new WebSocket(`ws://${location.host}/ws`);
  ws = _ws;
  _ws.onerror = () => { document.getElementById('summary').textContent = t('ws_error'); };
  _ws.onclose = (e) => {
    if (_elapsedTimerInterval) { clearInterval(_elapsedTimerInterval); _elapsedTimerInterval = null; }
    sessions.clear();
    autoDismissTimers.forEach(t => clearTimeout(t));
    autoDismissTimers.clear();
    activeSessionId = null;
    updateShellBadge(null);
    updateTabNotification(0);
    const area = document.getElementById('terminal-area');
    if (area) area.innerHTML = '';
    hideActionBar(undefined);
    renderSessionList();
    if (_wsIntentionalClose) return;
    // 指数バックオフで自動再接続
    document.getElementById('summary').textContent = t('ws_close', { code: e.code });
    const nsBtn = document.getElementById('new-session-btn');
    if (nsBtn) { nsBtn.disabled = true; document.getElementById('new-session-panel').hidden = true; }
    document.getElementById('reconnect-btn').hidden = false;
    const jitter = Math.random() * 200;
    const delay = Math.min(_wsRetryMax, _wsRetryDelay) + jitter;
    _wsRetryDelay = Math.min(_wsRetryMax, _wsRetryDelay * 2);
    setTimeout(() => {
      if (_wsIntentionalClose) return;
      _connectWs();
    }, delay);
  };
  _ws.onopen = () => {
    _wsRetryDelay = 500; // 再接続成功でバックオフリセット
    document.getElementById('summary').textContent = t('registering');
    const nsBtn = document.getElementById('new-session-btn');
    if (nsBtn) nsBtn.disabled = false;
    document.getElementById('reconnect-btn').hidden = true;
    _sendRegister();
  };
  _ws.onmessage = (ev) => {
    let m;
    try { m = JSON.parse(ev.data); } catch { return; }
    let fastRenderSessionId = null;

  if (m.type === 'pty_data') {
    const id = m.session_id;
    ensureTerminal(id);
    const t = terminals.get(id);
    const binary = atob(m.data);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    const isActive = id === activeSessionId;
    const isLiveRendered = isActive || isSessionLiveRenderedInMultiPane(id);

    // マーカーを先にデコードして検出し、xterm.js / スキャナにはマーカー除去済みバイトを渡す
    let textChunk = '';
    let approvalTextChunk = '';
    let xtermBytes = bytes;
    let hasMarker = false;
    try {
      textChunk = (t.textDecoder || utf8Decoder).decode(bytes, { stream: true });
      approvalTextChunk = textChunk;
      if (textChunk.includes(CHAT_HISTORY_USER_TURN_MARKER)) {
        hasMarker = true;
        approvalTextChunk = textChunk.split(CHAT_HISTORY_USER_TURN_MARKER).join('');
        xtermBytes = utf8Encoder.encode(approvalTextChunk);
      }
    } catch (_) {}

    if (isLiveRendered && !t.pendingFlushActive) {
      writePTYChunk(id, t.term, xtermBytes, () => {
        if (t.autoScroll) t.term.scrollToBottom();
      });
    } else {
      // 非アクティブセッションは everAttached に関わらず pendingChunks に溜める。
      // 承認検出は scanBuffer ではなく pendingTextTail ベースで行うため、
      // xterm のライブ書き込みを非アクティブ中は止めてよい。
      // セッション切替時に attachTerminal → flushPending で一括 xterm 書き込みする。
      queuePendingTerminalChunk(id, xtermBytes);
    }
    trackApprovalHintFromChunk(id, xtermBytes, approvalTextChunk);
    if (isLiveRendered) scheduleApprovalCheck(id);

    // chatHistory: マーカー検出でターン境界を確定し AI 出力を commit する
    if (hasMarker) {
      const parts = textChunk.split(CHAT_HISTORY_USER_TURN_MARKER);
      for (let i = 0; i < parts.length; i++) {
        if (parts[i]) chatHistoryAppendOutput(id, parts[i]);
        if (i < parts.length - 1) chatHistoryCommitOutputOrSeed(id);
      }
    } else if (textChunk) {
      chatHistoryAppendOutput(id, textChunk);
    }
    return;
  }

  if (m.type === 'approval_patterns_updated') {
    showToast(t('toast_approval_patterns_updated'));
    if (window.approvalPatternsUI && typeof window.approvalPatternsUI.onOfficialUpdated === 'function') {
      window.approvalPatternsUI.onOfficialUpdated(Array.isArray(m.providers) ? m.providers : []);
    }
    return;
  }

  if (m.type === 'approval_detected') {
    handleGoApprovalDetected(m);
    return;
  }

  if (m.type === 'approval_cleared') {
    handleGoApprovalCleared(m);
    return;
  }

  if (m.type === 'session_history_reset') {
    if (m.session_id) resetLocalSessionHistory(m.session_id);
    else resetAllLocalSessionHistory();
    showToast(t('session_history_reset_done'));
    return;
  }

  if (m.type === 'snapshot') {
    const arr = typeof m.sessions === 'string' ? JSON.parse(m.sessions) : m.sessions;
    (arr || []).forEach(s => {
      if (s.state === 'completed') {
        requestSessionDismiss(s.id);
        return;
      }
      s.project = deriveProjectKeyFromCwd(s.cwd);
      sessions.set(s.id, s);
      addToSessionOrder(s.id);
    });
    document.getElementById('summary').textContent = t('connected') || '接続済み';
    renderSessionList();
    checkApprovalOnStartup();
    syncElapsedTimer();
  } else if (m.type === 'session_update') {
    if (m.state === 'completed') {
      requestSessionDismiss(m.session_id);
      removeLocalSession(m.session_id);
      return;
    }
    const isNew = !sessions.has(m.session_id);
    const cur = sessions.get(m.session_id) || { id: m.session_id };
    const beforeLayout = sessionLayoutSnapshot(cur);
    if (m.provider)        cur.provider        = m.provider;
    if (m.display_name)    cur.display_name    = m.display_name;
    if (m.cwd)            { cur.cwd = m.cwd; cur.project = deriveProjectKeyFromCwd(m.cwd); }
    if (m.branch !== undefined) cur.branch      = m.branch;
    if (m.label !== undefined) cur.label       = m.label;
    if (m.shell)           cur.shell           = m.shell;
    if (m.state)           cur.state           = m.state;
    if (m.last_output_at)  cur.last_output_at  = m.last_output_at;
    if (m.started_at)      cur.started_at      = m.started_at;
    if (m.first_message)   cur.first_message   = m.first_message;
    if (m.last_message)    cur.last_message    = m.last_message;
    if (m.log_path)        cur.log_path        = m.log_path;
    if (m.jsonl_path)      cur.jsonl_path      = m.jsonl_path;
    if (m.model !== undefined) cur.model       = m.model;
    if (m.route !== undefined) cur.route       = m.route;
    sessions.set(m.session_id, cur);
    if (!isNew && beforeLayout === sessionLayoutSnapshot(cur) && (m.state || m.last_output_at)) {
      fastRenderSessionId = m.session_id;
    }
    // C4: セッション state 変化をマルチペインバッジに反映
    if (m.state) {
      const mgr = window.multiPaneManager;
      if (mgr && typeof mgr.updateSlotBadge === 'function') {
        const badgeStatus = m.state === 'waiting' ? 'waiting'
                          : m.state === 'running' ? 'running'
                          : 'standby';
        mgr.updateSlotBadge(m.session_id, badgeStatus);
      }
    }
    // C5: 新規セッションは非★グループ末尾（forceToFront=false）
    // 既存セッションの再登録（isNew=false）は位置を変えない
    addToSessionOrder(m.session_id, false);
    if (isNew && pendingAutoSwitch) {
      pendingAutoSwitch = false;
      activateSession(m.session_id);
    }
  } else if (m.type === 'session_end') {
    if ((m.state || 'disconnected') === 'completed') {
      requestSessionDismiss(m.session_id);
      removeLocalSession(m.session_id);
      return;
    }
    const s = sessions.get(m.session_id);
    if (s) {
      s.state = m.state || 'disconnected';
      if (m.reason) s.end_reason = m.reason;
    }
    cancelApprovalHintConfirm(m.session_id);
    approvalVisibleCache.delete(m.session_id);
    if (multiQuestionVisibleCache.delete(m.session_id) && m.session_id === activeSessionId) {
      setMultiQuestionBannerVisible(false);
    }
    removeApprovalAutoSwitchTarget(m.session_id);
    maybeAutoSwitchToNextApproval();
    const deadStates = ['error', 'disconnected'];
    if (deadStates.includes(m.state) && !autoDismissTimers.has(m.session_id)) {
      const timer = setTimeout(() => {
        autoDismissTimers.delete(m.session_id);
        dismissSession(m.session_id);
      }, 5000);
      autoDismissTimers.set(m.session_id, timer);
    }
  } else if (m.type === 'session_removed') {
    removeLocalSession(m.session_id);
  }

  if (fastRenderSessionId !== null && renderSessionStateUpdate(fastRenderSessionId)) return;
  render();
  }; // end _ws.onmessage
} // end _connectWs

document.getElementById('reconnect-btn').addEventListener('click', async () => {
  const btn = document.getElementById('reconnect-btn');
  btn.disabled = true;
  btn.textContent = t('reconnect_checking') || '確認中...';
  try {
    // トークン不要の疎通確認（401でも「Hub起動中」と判断）
    await fetch('/', { signal: AbortSignal.timeout(2000) });
    location.reload();
  } catch (_) {
    btn.textContent = '↺ ' + (t('reconnect') || '再接続');
    btn.disabled = false;
    document.getElementById('summary').textContent = t('hub_stopped') || 'Hub停止中 — any-ai-cli serve で再起動してください';
  }
});

// WS 初回接続
_connectWs();
