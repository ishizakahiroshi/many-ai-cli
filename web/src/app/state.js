// --- ESM imports (generated) ---
import { STORAGE_APPROVAL_AUTO_SWITCH_KEY, STORAGE_FAVORITES_KEY, STORAGE_GROUP_ORDER_KEY, STORAGE_ORDER_KEY, STORAGE_PROJECT_FAVORITES_KEY, setUserPref } from './user-prefs.js';
import { activateSession } from './session-list.js';
import { isBatchOptions } from './approval-parser.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

export const sessions = new Map();
export const terminals = new Map(); // sessionId -> { term, fitAddon, container, pendingChunks, pendingTextTail, markerFilterCarry }
export const approvalVisibleCache = new Map();
export const multiQuestionVisibleCache = new Map(); // sessionId → bool（Claude Code AskUserQuestion 等の複数質問 UI が画面に出ているか）
export const multiQuestionDismissedCache = new Map(); // sessionId → bool（banner の ✕ ボタンで誤検出を手動 dismiss した状態。次の PTY 送信でクリア）
export const sequentialChoiceCache = new Map(); // sessionId → { sig, prompts, answers, index }
export const approvalRawOptionsCache = new Map(); // sessionId → [{num, label, isCurrent}] または [{num, title, options}, ...]（バッチ承認）
export const approvalSourceCache = new Map(); // sessionId → { source, sig, kind, detectedAt }（Go native 等の表示元）
export const approvalConsumedSig = new Map(); // sessionId → 消費済み承認の署名（doSend でテキスト送信した場合の再表示防止）
export const batchSelections = new Map(); // sessionId → number[]（セクションごとの選択番号、未選択は null）
export let batchFocusIdx = -1; // 現在フォーカス中のバッチセクション index（-1: 未フォーカス / 範囲外）
export const approvalConsumedSigDeleteTimer = new Map(); // sessionId → timer（sig を debounce 型で削除するためのタイマー）
export const approvalSwitchCandidates = new Map(); // sessionId → { sig, options, firstSeenAt }（表示中の承認と異なる選択肢が検出されたときの安定性チェック用）
export const APPROVAL_PENDING_TEXT_TAIL_LIMIT = 12000;

// 承認選択肢の sig を計算。Ink の再描画やスクロールバック残骸による
// label の微妙な差異（前後空白、空白の重複、truncate 位置）を吸収するため normalize する。
// (Y:1/N:0) Yes/No プロンプトはどれも同じ label を持つため、_ctx に質問文ハッシュを
// 載せて区別する（連続する別質問が同一 sig で誤抑制されないように）。
export function approvalSig(options) {
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
export function _approvalCtxHash(s) {
  const text = String(s || '').replace(/\s+/g, ' ').trim();
  let h = 5381;
  for (let i = 0; i < text.length; i++) {
    h = (((h << 5) + h) + text.charCodeAt(i)) | 0;
  }
  return (h >>> 0).toString(36);
}

export function sequentialChoiceSig(prompts) {
  return _approvalCtxHash((prompts || []).map(p => `${p.key}:${p.question}:${p.options.map(o => `${o.num}.${o.label}`).join('|')}`).join('\n'));
}
export const approvalHintConfirmTimers = new Map(); // sessionId → timer（生バイト検出を短時間 debounce してチカチカを防ぐ）
export const approvalHintConfirmTrusted = new Map(); // sessionId → true: marker/plainYesNo 由来の信頼性の高い検出（fallback に上書きさせない）
export const toolOutputs = new Map(); // sessionId → [{uid, lines, ts}]
export const sessionInputState = new Map(); // sessionId → { inputValue, pastedTextsData, pendingAttachFiles, thumbsFragment }

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

export const chatHistory = new Map();              // sid → Message[]
export const chatHistorySubs = new Map();          // sid → Set<callback>
export const chatHistoryIdSeq = new Map();         // sid → 次に振る連番 (1 始まり)
export const chatHistoryOutputBuffers = new Map(); // sid → { rawChunks:[], lastTs }
export const chatHistoryAutoCommitTimers = new Map(); // sid → timerId
// Go 側 chatHistoryUserTurnMarker と一致させること
export const CHAT_HISTORY_USER_TURN_MARKER = "\x1b]47777;user-turn\x07";

export const autoDismissTimers = new Map(); // sessionId → timer
export const approvalSuppressUntil = new Map(); // sessionId → timestamp (sendChoice 後の誤再表示を抑制)
export const approvalAutoSwitchQueue = [];
export const utf8Decoder = new TextDecoder('utf-8');
export const utf8Encoder = new TextEncoder();

export let activeSessionId = null;
export let isComposing = false;       // IMEコンポジション状態
export let pendingSend = false;       // IME確定後に送信するフラグ
export let composeEndSendTimer = null; // compositionend が doSend をスケジュール済みの場合のタイマーID
export let lastDoSendAt = 0;          // 直前の doSend 実行時刻（二重送信防止の短時間ガード用）
export const DOUBLE_SEND_GUARD_MS = 100;
export const SIDEBAR_COLLAPSED_WIDTH_THRESHOLD = 180;
export let expandCapturePending = false;
// action-bar の点滅防止用
// - lastActionBarRender: 前回描画した内容のシグネチャ（同一なら DOM 再構築をスキップ）
// - crunchLatch: detectCrunch の振動吸収用ヒステリシス（一度 found=true を観測したら CRUNCH_LATCH_MS の間は維持）
export const lastActionBarRender = { sessionId: null, sig: null };
export const crunchLatch = new Map();
export const CRUNCH_LATCH_MS = 800;
export let _elapsedTimerInterval = null;
export let dragSrcId = null;
export let dragSrcGroupKey = null;
export let dragOverCardEl = null;

function readStorageArray(key) {
  try {
    const value = JSON.parse(localStorage.getItem(key) || '[]');
    return Array.isArray(value) ? value : [];
  } catch (_) {
    return [];
  }
}

export function isSessionLiveRenderedInMultiPane(id) {
  const multiView = document.getElementById('multi-view');
  const mgr = window.multiPaneManager;
  if (!multiView || multiView.hidden || !mgr || !Array.isArray(mgr.slots)) return false;
  const t = terminals.get(id);
  if (!t || !t.everAttached || !t.container || !t.container.isConnected) return false;
  if (!multiView.contains(t.container)) return false;
  return mgr.slots.some(slot => slot && slot.session && slot.session.id === id);
}
export let dragOverGroupEl = null;
export let pendingAutoSwitch = false;
export let actionBarFocusIdx = -1;
export let approvalAutoSwitchInProgress = false;
export const actionBarShownAt = new Map(); // sessionId -> timestamp(ms), Enter即確定ガード用

export let favorites = readStorageArray(STORAGE_FAVORITES_KEY);
export let projectFavorites = readStorageArray(STORAGE_PROJECT_FAVORITES_KEY);
export let sessionOrder = readStorageArray(STORAGE_ORDER_KEY);
export let groupOrder = readStorageArray(STORAGE_GROUP_ORDER_KEY);
export const collapsedGroups = new Set();

export function saveFavorites() {
  setUserPref('favorites', favorites);
}

export function saveProjectFavorites() {
  setUserPref('project_favorites', projectFavorites);
}

export function saveGroupOrder() {
  setUserPref('group_order', groupOrder);
}

export function saveSessionOrder() {
  setUserPref('session_order', sessionOrder);
}

// cwd からプロジェクトキーを派生する。
// renderSessionList のグループ化（末尾セグメント）と同じ規則で揃え、
// FilesTabManager の可視性判定（curSess.project 参照）が一貫して機能するようにする。
export function deriveProjectKeyFromCwd(cwd) {
  if (!cwd) return '';
  const name = String(cwd).replace(/\\/g, '/').split('/').filter(p => p.length > 0).pop() || '';
  return name;
}

export function addToSessionOrder(id, forceToFront = false) {
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

export function removeFromSessionOrder(id) {
  const idx = sessionOrder.indexOf(id);
  if (idx !== -1) { sessionOrder.splice(idx, 1); saveSessionOrder(); }
}

// orderSessions は全モジュール共通のセッション整列ロジック（C9: 旧 getSortedSessions /
// getOrderedSessions / multi-pane フォールバックの三重定義を 1 つに集約）。
// ★（favorites 配列順）を先頭に、非★（sessionOrder 順、未登録セッションは末尾）を後に並べる。
// sessions / favorites / sessionOrder 未定義時は空配列または id 昇順へフォールバックする
// （旧 multi-pane.js の防御的フォールバックを継承）。
export function orderSessions() {
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

export function isApprovalAutoSwitchEnabled() {
  return localStorage.getItem(STORAGE_APPROVAL_AUTO_SWITCH_KEY) === '1';
}

export function isCurrentSessionHoldingApprovalFocus() {
  if (activeSessionId === null) return false;
  return !!approvalVisibleCache.get(activeSessionId);
}

export function removeApprovalAutoSwitchTarget(sessionId) {
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

export function maybeAutoSwitchToNextApproval() {
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

export function enqueueApprovalAutoSwitch(sessionId) {
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

// --- ESM cross-module setters (generated) ---
export function set__elapsedTimerInterval(v) { _elapsedTimerInterval = v; }
export function set_actionBarFocusIdx(v) { actionBarFocusIdx = v; }
export function set_activeSessionId(v) { activeSessionId = v; }
export function set_batchFocusIdx(v) { batchFocusIdx = v; }
export function set_composeEndSendTimer(v) { composeEndSendTimer = v; }
export function set_dragOverCardEl(v) { dragOverCardEl = v; }
export function set_dragOverGroupEl(v) { dragOverGroupEl = v; }
export function set_dragSrcGroupKey(v) { dragSrcGroupKey = v; }
export function set_dragSrcId(v) { dragSrcId = v; }
export function set_groupOrder(v) { groupOrder = v; }
export function set_isComposing(v) { isComposing = v; }
export function set_lastDoSendAt(v) { lastDoSendAt = v; }
export function set_pendingAutoSwitch(v) { pendingAutoSwitch = v; }
export function set_pendingSend(v) { pendingSend = v; }

// --- ESM window-interop publish (generated; preserves dynamic window.* lookups) ---
window.terminals = terminals;
// activeSessionId は実行中に変化するため、素の代入だと評価時の初期値 (null) を焼き付けて
// 二度と更新されない。加えて chat-history.js が同名を「setter 無しの getter」として先に
// 定義する評価順のため、素の代入は strict モードで
//   TypeError: Cannot set property activeSessionId which has only a getter
// を投げ、state.js の評価が中断して後続モジュール (spawn-panel 等) が読み込まれなくなる。
// configurable な live getter として定義し、値の最新性と評価順非依存の両方を満たす。
Object.defineProperty(window, 'activeSessionId', {
  configurable: true,
  get() { return activeSessionId; },
});
