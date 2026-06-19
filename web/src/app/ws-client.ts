// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { showToast, token } from './util.js';
import { CHAT_HISTORY_USER_TURN_MARKER, _elapsedTimerInterval, activeSessionId, addToSessionOrder, approvalVisibleCache, autoDismissTimers, chatHistory, deriveProjectKeyFromCwd, isSessionLiveRenderedInMultiPane, maybeAutoSwitchToNextApproval, multiQuestionLatchAt, multiQuestionVisibleCache, pendingAutoSwitch, removeApprovalAutoSwitchTarget, sessions, set__elapsedTimerInterval, set_activeSessionId, set_pendingAutoSwitch, terminals, utf8Decoder, utf8Encoder } from './state.js';
import { dismissSession, removeLocalSession, requestSessionDismiss, resetAllLocalSessionHistory, resetLocalSessionHistory, updateInputAffordance } from '../app.js';
import { activateSession, render, renderSessionList, renderSessionStateUpdate, updateMainTabStatus, updateShellBadge, updateTabNotification } from './session-list.js';
import { applyRemotePtyResize, ensureTerminal, markCompactActivity, queuePendingTerminalChunk, scheduleLiveStatusExtract, syncLiveStatusDomForActive, writePTYChunk } from './terminal.js';
import { checkApprovalOnStartup } from './settings.js';
import { setMultiQuestionBannerVisible } from './approval-ui.js';
import { cancelApprovalHintConfirm, handleGoApprovalCleared, handleGoApprovalDetected, hideActionBar, isAIProvider, scheduleApprovalCheck, trackApprovalHintFromChunk } from './approval.js';
import { notifyDeferredEnterOutput } from './deferred-enter.js';
import { chatHistoryAppendOutput, chatHistoryCommitOutputOrSeed } from './chat-history.js';
import { clearChatPayloadForSession, handleChatTurnMessage, initChatPayloadUI } from './chat-payload.js';
import { handleUsageStatMessage, removeUsageCacheEntry } from './token-statusbar.js';
import { removeWorkflowSnapshot } from './workflow-modal.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- WebSocket 自動再接続（指数バックオフ） ----
export let ws = null;
export let _wsIntentionalClose = false; // ページ遷移など意図的クローズ時は再接続しない
export let _wsRetryDelay = 500; // 初期バックオフ ms
export const _wsRetryMax = 10000; // 上限 ms
let _wsReconnectTimer = null;

// ---- WS 死活監視（モバイルの half-open / ゾンビ接続対策） ----
// スマホは画面ロック・アプリ切替・Wi-Fi⇄モバイル回線のハンドオーバ等で、
// onclose を発火しないまま接続が半オープン状態になることがある。その場合
// ws.readyState は OPEN のまま固まり、session_update 等のライブ通知が届かず
// 「新規セッションが出ない（リロードすると出る）」という症状になる。
// サーバは uiPingInterval(30s) ごとに {type:'ping'} を送るため、健全な接続なら
// 最低30秒に1回は何らかのメッセージが届く。直近受信からの経過で死活判定する。
let _lastMsgAt = Date.now();
let _wsWatchdog = null;
const WS_STALE_MS = 75000; // ping 約2.5回分。これを超える無通信はゾンビとみなす
const WS_WATCHDOG_INTERVAL_MS = 15000;

// ステータスバー等から Hub 接続状態を参照するための getter。
//   'open'        : WS 接続中
//   'connecting'  : 接続試行中 / 再接続待ち
//   'closed'      : 切断（再接続予定なし）
export function wsConnectionState(): 'open' | 'connecting' | 'closed' {
  if (ws && ws.readyState === WebSocket.OPEN) return 'open';
  if (ws && ws.readyState === WebSocket.CONNECTING) return 'connecting';
  if (_wsReconnectTimer) return 'connecting';
  return 'closed';
}

// Hub プロセス起動毎の ID（snapshot の hub_instance）。Hub が再起動すると
// live session ID が 1 から振り直されるため、再接続先が別インスタンスだった
// 場合は旧 ID キーのローカル状態（chatHistory / terminals 等）を破棄しないと
// 同じ番号の別セッションに旧セッションのチャット・バッファが混入する。
let _hubInstance = null;
let _pendingOpenSessionId = parseInt(new URLSearchParams(location.search).get('session_id') || '0', 10) || 0;

const REGISTER_DEFAULT_COLS = 200;
const REGISTER_DEFAULT_ROWS = 50;
const REGISTER_MIN_USABLE_COLS = 80;
const REGISTER_MIN_USABLE_ROWS = 20;
const REGISTER_APPROX_CELL_WIDTH = 7.5;
const REGISTER_APPROX_CELL_HEIGHT = 16;

function openSessionFromNotification(sessionId) {
  const id = Number(sessionId || 0);
  if (!id) return;
  if (sessions.has(id)) {
    activateSession(id);
    return;
  }
  _pendingOpenSessionId = id;
}

if ('serviceWorker' in navigator) {
  navigator.serviceWorker.addEventListener('message', (event) => {
    const data = event.data || {};
    if (data.type === 'many-ai-cli-open-session') {
      openSessionFromNotification(data.session_id);
    }
  });
}

// Hub 再起動検出時に live session ID をキーとするローカル状態を全破棄する。
// removeLocalSession がターミナル dispose・チャット・承認系キャッシュ等の
// 個別クリーンアップを一括で行うため、既知の全 ID に対して呼ぶ。
function purgeLocalStateForHubRestart() {
  const ids = new Set([
    ...terminals.keys(),
    ...chatHistory.keys(),
    ...sessions.keys(),
  ]);
  ids.forEach(id => { try { removeLocalSession(id); } catch (_) {} });
}

export function syncElapsedTimer() {
  const shouldRun = !!(ws && ws.readyState === WebSocket.OPEN && activeSessionId !== null && !document.hidden);
  if (shouldRun) {
    if (!_elapsedTimerInterval) {
      set__elapsedTimerInterval(setInterval(() => updateMainTabStatus(), 1000));
    }
    updateMainTabStatus();
    return;
  }
  if (_elapsedTimerInterval) {
    clearInterval(_elapsedTimerInterval);
    set__elapsedTimerInterval(null);
  }
}

document.addEventListener('visibilitychange', syncElapsedTimer);

// 直近受信からの経過が WS_STALE_MS を超えていたらゾンビ接続とみなして張り直す。
// OPEN のまま固まった half-open ソケットを能動的に検出する唯一の手段。
function _wsWatchdogTick() {
  if (_wsIntentionalClose) return;
  if (!ws || ws.readyState !== WebSocket.OPEN) return;
  if (Date.now() - _lastMsgAt > WS_STALE_MS) {
    // close() → onclose ハンドラが後片付け＋自動再接続をスケジュールする。
    try { ws.close(); } catch (_) {}
  }
}

// フォアグラウンド復帰／オンライン復帰時に接続の生死を確認し、
// 死んでいれば即時に張り直す（ウォッチドッグの最大75秒待ちを回避）。
function _ensureWsAlive() {
  if (_wsIntentionalClose) return;
  if (!ws || ws.readyState === WebSocket.CLOSED || ws.readyState === WebSocket.CLOSING) {
    // 再接続待ちが入っていれば前倒しして即接続する。
    if (_wsReconnectTimer) { clearTimeout(_wsReconnectTimer); _wsReconnectTimer = null; }
    _wsRetryDelay = 500;
    _connectWs();
    return;
  }
  // OPEN でも復帰直後はゾンビの可能性がある。無通信が続いていれば張り直す。
  if (ws.readyState === WebSocket.OPEN && Date.now() - _lastMsgAt > WS_STALE_MS) {
    try { ws.close(); } catch (_) {}
  }
}

document.addEventListener('visibilitychange', () => { if (!document.hidden) _ensureWsAlive(); });
window.addEventListener('online', _ensureWsAlive);
window.addEventListener('pageshow', _ensureWsAlive);

export function _sendRegister() {
  const { cols, rows } = estimateRegisterTerminalSize();
  ws.send(JSON.stringify({ type: 'register', role: 'ui', token, cols, rows, ui_active_session_id: activeSessionId || 0 }));
}

function estimateRegisterTerminalSize(): { cols: number; rows: number } {
  const active = activeSessionId !== null ? terminals.get(activeSessionId) : null;
  const termCols = Number(active?.term?.cols || 0);
  const termRows = Number(active?.term?.rows || 0);
  if (termCols >= REGISTER_MIN_USABLE_COLS && termRows >= REGISTER_MIN_USABLE_ROWS) {
    return { cols: termCols, rows: termRows };
  }

  const area = document.getElementById('terminal-area');
  // clientWidth/Height が「ゼロではないが極小（レイアウト未確定 / 親が
  // display:none 直後など）」のときに、小さい cols が Hub の lastUICols として
  // 記録されると、新規セッションの初期 PTY 幅まで狭くなる。Provider CLI は
  // その幅でテキストをハード改行するため、後から resize しても過去行は直らない。
  const cw = area ? area.clientWidth : 0;
  const ch = area ? area.clientHeight : 0;
  const approxCols = cw > 0 ? Math.floor(cw / REGISTER_APPROX_CELL_WIDTH) : 0;
  const approxRows = ch > 0 ? Math.floor(ch / REGISTER_APPROX_CELL_HEIGHT) : 0;
  return {
    cols: approxCols >= REGISTER_MIN_USABLE_COLS ? approxCols : REGISTER_DEFAULT_COLS,
    rows: approxRows >= REGISTER_MIN_USABLE_ROWS ? approxRows : REGISTER_DEFAULT_ROWS,
  };
}

export function sessionLayoutSnapshot(s) {
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

export function _connectWs() {
  // HTTPS（Tailscale serve / リモート公開）経由では平文 ws:// が mixed-content で
  // ブロックされ snapshot を受信できずセッションカードが出ない。ページの protocol に
  // 合わせて wss:// / ws:// を選ぶ。
  const wsProto = location.protocol === 'https:' ? 'wss://' : 'ws://';
  const _ws = new WebSocket(`${wsProto}${location.host}/ws`);
  ws = _ws;
  _ws.onerror = () => { document.getElementById('summary').textContent = t('ws_error'); };
  _ws.onclose = (e) => {
    if (ws !== _ws) return;
    if (_wsWatchdog) { clearInterval(_wsWatchdog); _wsWatchdog = null; }
    if (_elapsedTimerInterval) { clearInterval(_elapsedTimerInterval); set__elapsedTimerInterval(null); }
    sessions.clear();
    autoDismissTimers.forEach(t => clearTimeout(t));
    autoDismissTimers.clear();
    set_activeSessionId(null);
    if (typeof window.syncMobileLayoutState === 'function') window.syncMobileLayoutState();
    updateShellBadge(null);
    updateTabNotification(0);
    const area = document.getElementById('terminal-area');
    if (area) area.innerHTML = '';
    hideActionBar(undefined);
    renderSessionList();
    if (_wsIntentionalClose) return;
    // 指数バックオフで自動再接続
    document.getElementById('summary').textContent = t('ws_close', { code: e.code });
    const nsBtn = document.getElementById('new-session-btn') as HTMLButtonElement | null;
    if (nsBtn) { nsBtn.disabled = true; document.getElementById('new-session-panel').hidden = true; }
    document.getElementById('reconnect-btn').hidden = false;
    const jitter = Math.random() * 200;
    const delay = Math.min(_wsRetryMax, _wsRetryDelay) + jitter;
    _wsRetryDelay = Math.min(_wsRetryMax, _wsRetryDelay * 2);
    if (_wsReconnectTimer) clearTimeout(_wsReconnectTimer);
    _wsReconnectTimer = setTimeout(() => {
      _wsReconnectTimer = null;
      if (_wsIntentionalClose) return;
      if (ws !== _ws) return;
      _connectWs();
    }, delay);
  };
  _ws.onopen = () => {
    if (_wsReconnectTimer) {
      clearTimeout(_wsReconnectTimer);
      _wsReconnectTimer = null;
    }
    _wsRetryDelay = 500; // 再接続成功でバックオフリセット
    _lastMsgAt = Date.now();
    if (_wsWatchdog) clearInterval(_wsWatchdog);
    _wsWatchdog = setInterval(_wsWatchdogTick, WS_WATCHDOG_INTERVAL_MS);
    document.getElementById('summary').textContent = t('registering');
    const nsBtn = document.getElementById('new-session-btn') as HTMLButtonElement | null;
    if (nsBtn) nsBtn.disabled = false;
    document.getElementById('reconnect-btn').hidden = true;
    _sendRegister();
    // 再接続直後に承認可視ヒントを即時再主張する（Hub 側リースの取りこぼし修復。
    // リロード後はキャッシュが空なので no-op）。
    if (window.approvalUiAdapter?.reassertApprovalHints) {
      window.approvalUiAdapter.reassertApprovalHints();
    }
  };
  _ws.onmessage = (ev) => {
    // 任意のメッセージ受信（ping 含む）で死活タイマを更新する。
    _lastMsgAt = Date.now();
    let m;
    try { m = JSON.parse(ev.data); } catch { return; }
    let fastRenderSessionId = null;

  if (m.type === 'pty_data') {
    const id = m.session_id;
    ensureTerminal(id);
    const t = terminals.get(id);
    // ensureTerminal が内部例外で terminals へ登録できなかった場合 t は undefined になり、
    // 直後の t.textDecoder 参照で TypeError → このメッセージ処理が中断する。ガードして握り潰しを防ぐ。
    if (!t) return;
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
    markCompactActivity(id, approvalTextChunk);
    if (isLiveRendered) scheduleApprovalCheck(id);
    // Codex 等 provider 別のピル内テキストを本文から抽出（Claude は既存ブロック抽出経路）。
    if (isActive) scheduleLiveStatusExtract(id);
    // chat_turn / chat_turns_snapshot は早期 return しないが、ここで早期処理する
    // ためのフックを別途下に置く。
    // 複数行ペースト送信後の確定 \r は、この出力が静止する（取り込み・再描画完了）まで遅延させる。
    // 出力が来るたび待機をリセットし、止まったら deferred-enter.ts が \r を 1 回だけ送る。
    notifyDeferredEnterOutput(id);

    // chatHistory: マーカー検出でターン境界を確定し AI 出力を commit する。
    // Shell session は chat history extraction の対象外。
    const sessionProvider = sessions.get(id)?.provider || '';
    if (isAIProvider(sessionProvider)) {
      if (hasMarker) {
        const parts = textChunk.split(CHAT_HISTORY_USER_TURN_MARKER);
        for (let i = 0; i < parts.length; i++) {
          if (parts[i]) chatHistoryAppendOutput(id, parts[i]);
          if (i < parts.length - 1) chatHistoryCommitOutputOrSeed(id);
        }
      } else if (textChunk) {
        chatHistoryAppendOutput(id, textChunk);
      }
    }
    return;
  }

  if (m.type === 'usage_stat') {
    handleUsageStatMessage(m);
    return;
  }

  if (m.type === 'approval_patterns_updated') {
    showToast(t('toast_approval_patterns_updated'));
    if (window.approvalPatternsUI && typeof window.approvalPatternsUI.onOfficialUpdated === 'function') {
      window.approvalPatternsUI.onOfficialUpdated(Array.isArray(m.providers) ? m.providers : []);
    }
    return;
  }

  if (m.type === 'input_deferred') {
    // wrapper 未接続/送信失敗で Hub が入力を保留した。再接続時に自動再送されるが、
    // ユーザーには「今すぐは届いていない」ことを知らせる。
    showToast(t('toast_input_deferred', { id: m.session_id }));
    return;
  }

  if (m.type === 'pty_resize') {
    applyRemotePtyResize(m.session_id, m.cols, m.rows);
    return;
  }

  if (m.type === 'commit_msg_suggested' || m.type === 'commit_msg_error' || m.type === 'commit_msg_progress') {
    // Git タブ「Ask AI」の結果。該当する GitGraphView インスタンスが拾う。
    try { window.dispatchEvent(new CustomEvent('many-commit-msg', { detail: m })); } catch (_) {}
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

  if (m.type === 'chat_turn' || m.type === 'chat_turns_snapshot') {
    handleChatTurnMessage(m);
    return;
  }

  if (m.type === 'snapshot') {
    let arr;
    try {
      arr = typeof m.sessions === 'string' ? JSON.parse(m.sessions) : m.sessions;
    } catch (_) {
      return;
    }
    // Hub 再起動検出: インスタンス ID が前回接続時と異なる場合、snapshot 適用前に
    // 旧 live session ID キーのローカル状態を破棄する（別セッションへの履歴混入防止）
    const inst = typeof m.hub_instance === 'string' ? m.hub_instance : '';
    if (inst && _hubInstance && inst !== _hubInstance) {
      purgeLocalStateForHubRestart();
    }
    if (inst) _hubInstance = inst;
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
    if (_pendingOpenSessionId && sessions.has(_pendingOpenSessionId)) {
      const id = _pendingOpenSessionId;
      _pendingOpenSessionId = 0;
      activateSession(id);
    }
    checkApprovalOnStartup();
    syncElapsedTimer();
  } else if (m.type === 'session_update') {
    if (m.state === 'completed') {
      requestSessionDismiss(m.session_id);
      removeLocalSession(m.session_id);
      return;
    }
    const isNew = !sessions.has(m.session_id);
    const cur: any = sessions.get(m.session_id) || { id: m.session_id };
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
    // C3: git 変更状況は git_checked=true のメッセージでのみ更新する
    // （通常の session_update では 0 を omitempty で送らないため、ここで上書きしない）。
    if (m.git_checked) {
      cur.git_files   = m.git_files   ?? 0;
      cur.git_added   = m.git_added   ?? 0;
      cur.git_deleted = m.git_deleted ?? 0;
    }
    sessions.set(m.session_id, cur);
    // 実行中⇄アイドルの遷移をアクティブセッションの入力欄／送信ボタンへ反映する
    if (m.state && m.session_id === activeSessionId) { updateInputAffordance(); syncLiveStatusDomForActive(); }
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
      // C1: detached-grid モード: バッジ更新 + セッション追加時に再描画
      const dgMgr = window.detachedGridManager;
      if (dgMgr) {
        if (typeof dgMgr.updateSlotBadge === 'function') {
          const badgeStatus = m.state === 'waiting' ? 'waiting'
                            : m.state === 'running' ? 'running'
                            : 'standby';
          dgMgr.updateSlotBadge(m.session_id, badgeStatus);
        }
        if (isNew && typeof dgMgr.onSessionsUpdated === 'function') {
          dgMgr.onSessionsUpdated();
        }
      }
    }
    // C5: 新規セッションは非★グループ末尾（forceToFront=false）
    // 既存セッションの再登録（isNew=false）は位置を変えない
    addToSessionOrder(m.session_id, false);
    if (isNew && pendingAutoSwitch) {
      set_pendingAutoSwitch(false);
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
    if (m.session_id === activeSessionId) { updateInputAffordance(); syncLiveStatusDomForActive(); }
    cancelApprovalHintConfirm(m.session_id);
    approvalVisibleCache.delete(m.session_id);
    if (multiQuestionVisibleCache.delete(m.session_id) && m.session_id === activeSessionId) {
      setMultiQuestionBannerVisible(false);
    }
    multiQuestionLatchAt.delete(m.session_id);
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
    removeUsageCacheEntry(m.session_id);
    removeWorkflowSnapshot(m.session_id);
  }

  if (fastRenderSessionId !== null && renderSessionStateUpdate(fastRenderSessionId)) return;
  render();
  }; // end _ws.onmessage
} // end _connectWs

document.getElementById('reconnect-btn').addEventListener('click', async () => {
  const btn = document.getElementById('reconnect-btn') as HTMLButtonElement;
  btn.disabled = true;
  btn.textContent = t('reconnect_checking') || '確認中...';
  try {
    // トークン不要の疎通確認（401でも「Hub起動中」と判断）
    await fetch('/', { signal: AbortSignal.timeout(2000) });
    location.reload();
  } catch (_) {
    btn.textContent = '↺ ' + (t('reconnect') || '再接続');
    btn.disabled = false;
    document.getElementById('summary').textContent = t('hub_stopped') || 'Hub停止中 — many-ai-cli serve で再起動してください';
  }
});

// WS 初回接続
_connectWs();
