// Extracted from app.js. Keep classic-script global scope; no module wrapper.

const sessions = new Map();
const terminals = new Map(); // sessionId -> { term, fitAddon, container, pendingChunks, pendingTextTail, markerFilterCarry }
const approvalVisibleCache = new Map();
const multiQuestionVisibleCache = new Map(); // sessionId → bool（Claude Code AskUserQuestion 等の複数質問 UI が画面に出ているか）
const multiQuestionDismissedCache = new Map(); // sessionId → bool（banner の ✕ ボタンで誤検出を手動 dismiss した状態。次の PTY 送信でクリア）
const sequentialChoiceCache = new Map(); // sessionId → { sig, prompts, answers, index }
const approvalRawOptionsCache = new Map(); // sessionId → [{num, label, isCurrent}] または [{num, title, options}, ...]（バッチ承認）
const approvalSourceCache = new Map(); // sessionId → { source, sig, kind, detectedAt }（Go native 等の表示元）
const approvalConsumedSig = new Map(); // sessionId → 消費済み承認の署名（doSend でテキスト送信した場合の再表示防止）
const batchSelections = new Map(); // sessionId → number[]（セクションごとの選択番号、未選択は null）
let batchFocusIdx = -1; // 現在フォーカス中のバッチセクション index（-1: 未フォーカス / 範囲外）
const approvalConsumedSigDeleteTimer = new Map(); // sessionId → timer（sig を debounce 型で削除するためのタイマー）
const approvalSwitchCandidates = new Map(); // sessionId → { sig, options, firstSeenAt }（表示中の承認と異なる選択肢が検出されたときの安定性チェック用）
const APPROVAL_PENDING_TEXT_TAIL_LIMIT = 12000;

// 承認選択肢の sig を計算。Ink の再描画やスクロールバック残骸による
// label の微妙な差異（前後空白、空白の重複、truncate 位置）を吸収するため normalize する。
// (Y:1/N:0) Yes/No プロンプトはどれも同じ label を持つため、_ctx に質問文ハッシュを
// 載せて区別する（連続する別質問が同一 sig で誤抑制されないように）。
function approvalSig(options) {
  if (isBatchOptions(options)) {
    return JSON.stringify(options.map(s => ({
      n: s.num,
      t: String(s.title || '').replace(/\s+/g, ' ').slice(0, 80),
      o: (s.options || []).map(o => `${o.num}:${String(o.label || '').trim().replace(/\s+/g, ' ').slice(0, 80)}`),
    })));
  }
  return JSON.stringify((options || []).map(o => {
    const lbl = String(o.label || '').trim().replace(/\s+/g, ' ').slice(0, 80);
    const ctx = o && o._ctx ? `|${o._ctx}` : '';
    return `${o.num}:${lbl}${ctx}`;
  }));
}

// シンプルな文字列ハッシュ (djb2)。承認質問文の同一性判定に使う。
function _approvalCtxHash(s) {
  const text = String(s || '').replace(/\s+/g, ' ').trim();
  let h = 5381;
  for (let i = 0; i < text.length; i++) {
    h = (((h << 5) + h) + text.charCodeAt(i)) | 0;
  }
  return (h >>> 0).toString(36);
}

function sequentialChoiceSig(prompts) {
  return _approvalCtxHash((prompts || []).map(p => `${p.key}:${p.question}:${p.options.map(o => `${o.num}.${o.label}`).join('|')}`).join('\n'));
}
const approvalHintConfirmTimers = new Map(); // sessionId → timer（生バイト検出を短時間 debounce してチカチカを防ぐ）
const approvalHintConfirmTrusted = new Map(); // sessionId → true: marker/plainYesNo 由来の信頼性の高い検出（fallback に上書きさせない）
const toolOutputs = new Map(); // sessionId → [{uid, lines, ts}]
const sessionInputState = new Map(); // sessionId → { inputValue, pastedTextsData, pendingAttachFiles, thumbsFragment }

// =========================================================================
// chatHistory store (plan_chat-history-subview.md §C1)
//
// セッションごとのチャット履歴を保持する in-memory store。
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

const chatHistory = new Map();              // sid → Message[]
const chatHistorySubs = new Map();          // sid → Set<callback>
const chatHistoryIdSeq = new Map();         // sid → 次に振る連番 (1 始まり)
const chatHistoryOutputBuffers = new Map(); // sid → { rawChunks:[], lastTs }
const chatHistoryAutoCommitTimers = new Map(); // sid → timerId
// Go 側 chatHistoryUserTurnMarker と一致させること
const CHAT_HISTORY_USER_TURN_MARKER = "\x1b]47777;user-turn\x07";

const autoDismissTimers = new Map(); // sessionId → timer
const approvalSuppressUntil = new Map(); // sessionId → timestamp (sendChoice 後の誤再表示を抑制)
const approvalAutoSwitchQueue = [];
const utf8Decoder = new TextDecoder('utf-8');
const utf8Encoder = new TextEncoder();

let activeSessionId = null;
let isComposing = false;       // IMEコンポジション状態
let pendingSend = false;       // IME確定後に送信するフラグ
let composeEndSendTimer = null; // compositionend が doSend をスケジュール済みの場合のタイマーID
let lastDoSendAt = 0;          // 直前の doSend 実行時刻（二重送信防止の短時間ガード用）
const DOUBLE_SEND_GUARD_MS = 100;
const SIDEBAR_COLLAPSED_WIDTH_THRESHOLD = 180;
let expandCapturePending = false;
// action-bar の点滅防止用
// - lastActionBarRender: 前回描画した内容のシグネチャ（同一なら DOM 再構築をスキップ）
// - crunchLatch: detectCrunch の振動吸収用ヒステリシス（一度 found=true を観測したら CRUNCH_LATCH_MS の間は維持）
const lastActionBarRender = { sessionId: null, sig: null };
const crunchLatch = new Map();
const CRUNCH_LATCH_MS = 800;
let _elapsedTimerInterval = null;
let dragSrcId = null;
let dragSrcGroupKey = null;
let dragOverCardEl = null;

function isSessionLiveRenderedInMultiPane(id) {
  const multiView = document.getElementById('multi-view');
  const mgr = window.multiPaneManager;
  if (!multiView || multiView.hidden || !mgr || !Array.isArray(mgr.slots)) return false;
  const t = terminals.get(id);
  if (!t || !t.everAttached || !t.container || !t.container.isConnected) return false;
  if (!multiView.contains(t.container)) return false;
  return mgr.slots.some(slot => slot && slot.session && slot.session.id === id);
}
let dragOverGroupEl = null;
let pendingAutoSwitch = false;
let actionBarFocusIdx = -1;
let approvalAutoSwitchInProgress = false;
const actionBarShownAt = new Map(); // sessionId -> timestamp(ms), Enter即確定ガード用

let favorites = JSON.parse(localStorage.getItem(STORAGE_FAVORITES_KEY) || '[]');
let projectFavorites = JSON.parse(localStorage.getItem(STORAGE_PROJECT_FAVORITES_KEY) || '[]');
let sessionOrder = JSON.parse(localStorage.getItem(STORAGE_ORDER_KEY) || '[]');
let groupOrder = JSON.parse(localStorage.getItem(STORAGE_GROUP_ORDER_KEY) || '[]');
const collapsedGroups = new Set();

function saveFavorites() {
  setUserPref('favorites', favorites);
}

function saveProjectFavorites() {
  setUserPref('project_favorites', projectFavorites);
}

function saveGroupOrder() {
  setUserPref('group_order', groupOrder);
}

function saveSessionOrder() {
  setUserPref('session_order', sessionOrder);
}

// cwd からプロジェクトキーを派生する。
// renderSessionList のグループ化（末尾セグメント）と同じ規則で揃え、
// FilesTabManager の可視性判定（curSess.project 参照）が一貫して機能するようにする。
function deriveProjectKeyFromCwd(cwd) {
  if (!cwd) return '';
  const name = String(cwd).replace(/\\/g, '/').split('/').filter(p => p.length > 0).pop() || '';
  return name;
}

function addToSessionOrder(id, forceToFront = false) {
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

function removeFromSessionOrder(id) {
  const idx = sessionOrder.indexOf(id);
  if (idx !== -1) { sessionOrder.splice(idx, 1); saveSessionOrder(); }
}

// orderSessions は全モジュール共通のセッション整列ロジック（C9: 旧 getSortedSessions /
// getOrderedSessions / multi-pane フォールバックの三重定義を 1 つに集約）。
// ★（favorites 配列順）を先頭に、非★（sessionOrder 順、未登録セッションは末尾）を後に並べる。
// sessions / favorites / sessionOrder 未定義時は空配列または id 昇順へフォールバックする
// （旧 multi-pane.js の防御的フォールバックを継承）。
function orderSessions() {
  if (typeof sessions === 'undefined') return [];
  if (typeof favorites === 'undefined' || typeof sessionOrder === 'undefined') {
    return Array.from(sessions.values()).sort((a, b) => a.id - b.id);
  }
  const starredList = favorites.filter(id => sessions.has(id)).map(id => sessions.get(id));
  const orderedIds = sessionOrder.filter(id => sessions.has(id) && !favorites.includes(id));
  sessions.forEach((s) => {
    if (!favorites.includes(s.id) && !orderedIds.includes(s.id)) orderedIds.push(s.id);
  });
  const nonStarredList = orderedIds.map(id => sessions.get(id));
  return [...starredList, ...nonStarredList];
}
window.orderSessions = orderSessions;
// 後方互換エイリアス: multi-pane.js など window.getSortedSessions 参照箇所のために残す。
window.getSortedSessions = orderSessions;

function isApprovalAutoSwitchEnabled() {
  return localStorage.getItem(STORAGE_APPROVAL_AUTO_SWITCH_KEY) === '1';
}

function isCurrentSessionHoldingApprovalFocus() {
  if (activeSessionId === null) return false;
  return !!approvalVisibleCache.get(activeSessionId);
}

function removeApprovalAutoSwitchTarget(sessionId) {
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

function maybeAutoSwitchToNextApproval() {
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

function enqueueApprovalAutoSwitch(sessionId) {
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
