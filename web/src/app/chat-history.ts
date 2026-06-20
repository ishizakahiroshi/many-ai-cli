// --- ESM imports (generated) ---
import { escapeHtml, showToast, ti18n, token } from './util.js';
import { activeSessionId, chatHistory, chatHistoryAutoCommitTimers, chatHistoryIdSeq, chatHistoryOutputBuffers, chatHistorySubs, sessions, terminals } from './state.js';
import { _userAvatarUrl, _userDisplayName } from '../app.js';
import { activateSession, providerIconHtml, renderSessionList } from './session-list.js';
import { showPathPopup } from './path-links.js';
import { TERMINAL_SCROLLBACK_LINES, markTerminalManualScrollIntent, sendResize, updateScrollLockBtn } from './terminal.js';
import { setActiveTab, updateChatCountBadge } from './settings.js';
import { chatPane, openLightbox } from './attachments.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

export function stripAnsiBasic(s) {
  if (!s) return '';
  // CSI: ESC [ ... letter
  // OSC: ESC ] ... BEL or ESC \\
  // その他の ESC + 1 文字
  return s
    .replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, '')
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, '')
    .replace(/\x1b[PX^_][^\x1b]*\x1b\\/g, '')
    .replace(/\x1b[()][0-9A-Za-z]/g, '')
    .replace(/\x1b[=>]/g, '')
    .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, '');
}

// Claude Code / Codex の「思考中」スピナー行を判定する。
// PTY を ANSI 除去しただけだと、思考中ステータス行
// （"✳ Imploring… (12s · ↑3.2k tokens · esc to interrupt)" 等）の再描画フレームが
// 大量に連結して残る。これらは会話本文ではないので 1 行まるごと落とす。
// 誤検出を避けるため、星形 dingbat（✢✳✶✷ U+2722–U+273F）と braille に絞り、
// ✓✗ など普通に使う記号は対象にしない。Go 側 sessionlog.IsThinkingNoiseLine と同義。
const SPINNER_ANIM_RE = /[✢-✿⠀-⣿]/;
const SPINNER_ANIM_RE_G = /[✢-✿⠀-⣿]/g;

export function isThinkingNoiseLine(line) {
  const t = String(line == null ? '' : line).trim();
  if (!t) return false;
  const lower = t.toLowerCase();
  // 1) ステータスフッター（Claude/Codex とも "esc to interrupt" を必ず表示する）
  if (lower.includes('esc to interrupt')) return true;
  // 2) モード切替ヒント "auto mode on (shift+tab to cycle)"
  if (/shift\s*\+\s*tab to cycle/i.test(t)) return true;
  // 3) トークンバー "↑111.0k ↓764"
  if (t.includes('↑') && t.includes('↓')) return true;
  // 4) "thinking" + スピナーグリフ（思考アニメの再描画フレーム）
  if (lower.includes('thinking') && SPINNER_ANIM_RE.test(t)) return true;
  // 5) スピナーグリフが複数現れる断片（"Imp·rmpovri✶osviisng✶..." 等）
  const m = t.match(SPINNER_ANIM_RE_G);
  if (m && m.length >= 2) return true;
  return false;
}

// D15: 軽い正規化（行頭末 trim / prompt prefix 除去 / 連続空行圧縮）。
// コードブロック内（```...```）の trim はあえてしない。
export function normalizeChatText(raw) {
  if (!raw) return '';
  // [MANY-AI-CLI]...[/MANY-AI-CLI] / [MANY-AI-CLI-DONE]...[/MANY-AI-CLI-DONE] ブロックを除去
  // （承認マーカー・完了サマリーマーカーはチャットに表示しない。DONE は -DONE 付きで
  //  別リテラルのため、汎用の [MANY-AI-CLI] パターンに一致せず先に専用で除去する）
  let s = raw
    .replace(/\[MANY-AI-CLI-DONE\][\s\S]*?\[\/MANY-AI-CLI-DONE\]/g, '')
    .replace(/\[MANY-AI-CLI\][\s\S]*?\[\/MANY-AI-CLI\]/g, '');
  const lines = s.split(/\r?\n/);
  const out = [];
  let inFence = false;
  let blankRun = 0;
  for (let line of lines) {
    const isFenceMarker = /^\s*```/.test(line);
    if (isFenceMarker) inFence = !inFence;
    // 思考中スピナーの再描画フレーム行を丸ごと落とす（コードフェンス内・フェンス行は対象外）
    if (!inFence && !isFenceMarker && isThinkingNoiseLine(line)) continue;
    if (!inFence) {
      // prompt prefix 除去（行頭のみ）
      line = line.replace(/^[\s]*[▌>$#]\s?/, '');
      line = line.replace(/\s+$/, '');
    }
    if (!inFence && line.trim() === '') {
      blankRun++;
      if (blankRun <= 1) out.push('');
    } else {
      blankRun = 0;
      out.push(line);
    }
  }
  return out.join('\n').replace(/^\n+|\n+$/g, '');
}

export function chatHistoryNextId(sid) {
  const next = (chatHistoryIdSeq.get(sid) || 0) + 1;
  chatHistoryIdSeq.set(sid, next);
  return next;
}

export function chatHistoryNotify(sid, msg) {
  const subs = chatHistorySubs.get(sid);
  if (!subs || subs.size === 0) return;
  for (const cb of subs) {
    try { cb(msg, getMessages(sid)); }
    catch (err) { console.warn('[chatHistory] subscriber error', err); }
  }
}

export function pushMessage(sid, msg) {
  if (sid === null || sid === undefined) return null;
  if (!chatHistory.has(sid)) chatHistory.set(sid, []);
  const arr = chatHistory.get(sid);
  const raw = msg.rawText != null ? msg.rawText : (msg.text != null ? msg.text : '');
  const normalized = msg.normalizedText != null
    ? msg.normalizedText
    : normalizeChatText(raw);
  const entry = {
    id: chatHistoryNextId(sid),
    ts: msg.ts || Date.now(),
    role: msg.role || 'system',
    kind: msg.kind || 'text',
    rawText: raw,
    normalizedText: normalized,
    attachments: Array.isArray(msg.attachments) ? msg.attachments.slice() : null,
    tool: msg.tool || null,
    meta: msg.meta || null,
  };
  arr.push(entry);
  chatHistoryNotify(sid, entry);
  return entry;
}

export function getMessages(sid) {
  const arr = chatHistory.get(sid);
  return arr ? arr.slice() : [];
}

const chatHistoryStoreRestored = new Set();
const chatHistoryStoreInflight = new Set();

function hasMeaningfulLocalChat(sid) {
  const arr = chatHistory.get(sid);
  if (!arr || arr.length === 0) return false;
  if (arr.length === 1 && arr[0].role === 'user' && !String(arr[0].rawText || arr[0].normalizedText || '').trim()) {
    return false;
  }
  return true;
}

export async function restoreChatHistoryFromStore(sid, opts: any = {}) {
  if (sid === null || sid === undefined) return false;
  if (chatHistoryStoreInflight.has(sid)) return false;
  if (!opts.force && chatHistoryStoreRestored.has(sid)) return false;
  if (!opts.force && hasMeaningfulLocalChat(sid)) return false;
  chatHistoryStoreInflight.add(sid);
  try {
    const res = await fetch(`/api/session-chat?token=${encodeURIComponent(token)}&session_id=${encodeURIComponent(sid)}&limit=500`);
    if (!res.ok) return false;
    const data = await res.json();
    const messages = Array.isArray(data.messages) ? data.messages : [];
    if (messages.length === 0) {
      // DB がまだ空のタイミングで照会すると 0 件が返る。ここで「復元済み」と
      // 記録してしまうと、その後 DB に履歴が蓄積されても二度と読み直さず、
      // チャットタブが「履歴はまだありません」のままになる（DB復元ボタン
      // を押すまで解消しない）。0 件のときは印を付けず、次回の mount/開封で
      // 再照会できるようにする。
      return false;
    }
    const t = chatHistoryAutoCommitTimers.get(sid);
    if (t) { clearTimeout(t); chatHistoryAutoCommitTimers.delete(sid); }
    chatHistoryOutputBuffers.delete(sid);
    revokeChatHistoryAttachmentURLs(sid);
    chatHistory.delete(sid);
    chatHistoryIdSeq.delete(sid);
    for (const m of messages) {
      const role = m.role || 'system';
      const kind = m.kind || 'text';
      const rawText = m.rawText || m.raw_text || m.text || '';
      // normalizedText は保存値を信用せず rawText から再計算する。
      // 旧 DB に保存済みのスピナー混入テキストも、改善後の normalizeChatText で除去するため。
      let normalizedText;
      if (role === 'ai' && kind === 'text') {
        normalizedText = normalizeChatText(rawText);
        // 思考スピナーだけで実本文の無い AI メッセージ（旧 DB の残骸）は復元しない
        if (!normalizedText.trim()) continue;
      }
      pushMessage(sid, {
        ts: m.ts || Date.now(),
        role,
        kind,
        rawText,
        normalizedText, // user/attach は undefined → pushMessage が再計算
        attachments: Array.isArray(m.attachments) ? m.attachments : null,
        meta: m.meta || null,
      });
    }
    chatHistoryStoreRestored.add(sid);
    updateChatCountBadge();
    return true;
  } catch (err) {
    console.warn('[chatHistory] restore from sqlite failed', err);
    return false;
  } finally {
    chatHistoryStoreInflight.delete(sid);
  }
}

export function subscribeChatHistory(sid, cb) {
  if (typeof cb !== 'function') return () => {};
  if (!chatHistorySubs.has(sid)) chatHistorySubs.set(sid, new Set());
  const subs = chatHistorySubs.get(sid);
  subs.add(cb);
  return () => {
    const cur = chatHistorySubs.get(sid);
    if (cur) {
      cur.delete(cb);
      if (cur.size === 0) chatHistorySubs.delete(sid);
    }
  };
}

export function chatHistoryAppendOutput(sid, raw) {
  if (sid === null || sid === undefined || !raw) return;
  let buf = chatHistoryOutputBuffers.get(sid);
  if (!buf) {
    buf = { rawChunks: [], lastTs: 0 };
    chatHistoryOutputBuffers.set(sid, buf);
  }
  buf.rawChunks.push(raw);
  buf.lastTs = Date.now();
  // PTY 出力が 1.5 秒止まったら自動コミット（AI 返答完了後に手動で次の入力を待たずに表示）
  const existing = chatHistoryAutoCommitTimers.get(sid);
  if (existing) clearTimeout(existing);
  chatHistoryAutoCommitTimers.set(sid, setTimeout(() => {
    chatHistoryAutoCommitTimers.delete(sid);
    chatHistoryCommitOutput(sid);
  }, 1500));
}

export function chatHistoryCommitOutput(sid) {
  const t = chatHistoryAutoCommitTimers.get(sid);
  if (t) { clearTimeout(t); chatHistoryAutoCommitTimers.delete(sid); }
  const buf = chatHistoryOutputBuffers.get(sid);
  if (!buf) return;
  if (buf.rawChunks.length === 0) return;
  const raw = buf.rawChunks.join('');
  buf.rawChunks.length = 0;
  // 純粋な制御文字のみのチャンク（プロンプト再描画等）は無視
  const stripped = stripAnsiBasic(raw);
  if (!stripped.trim()) return;
  // spinner / progress 文字のみのチャンクは無視（例: Codex が出力する "•"）
  // stripped.trim() で前後の空白・タブを除いた上で判定（"• " など末尾スペース付きを取りこぼさないため）
  if (/^[•◦⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏\r\n]+$/.test(stripped.trim())) return;
  // 起動バナー混入防止: 何もインタラクションがない状態（メッセージ0件）の間は AI 出力を記録しない。
  // role:'user' 限定では承認ボタン回答（role:'system'）後も AI 出力が記録されないため msgs.length で判定する。
  const msgs = chatHistory.get(sid);
  if (!msgs || msgs.length === 0) return;
  // 思考中スピナーの再描画フレームだけで実本文が無いチャンクはメッセージ化しない。
  // （これを許すと normalizedText が空になり、表示が rawText のゴミへフォールバックする）
  const normalized = normalizeChatText(stripped);
  if (!normalized.trim()) return;
  pushMessage(sid, {
    role: 'ai',
    kind: 'text',
    rawText: stripped,
    normalizedText: normalized,
  });
}

// マーカー検出専用の commit。msgs.length === 0 の場合はリプレイの先頭マーカーとみなし、
// 起動バナーをバッファから捨てつつ空の user エントリを1件積んで以降の commit を解放する。
export function chatHistoryCommitOutputOrSeed(sid) {
  const msgs = chatHistory.get(sid);
  if (!msgs || msgs.length === 0) {
    const buf = chatHistoryOutputBuffers.get(sid);
    if (buf) buf.rawChunks.length = 0;
    pushMessage(sid, { role: 'user', kind: 'text', rawText: '' });
    return;
  }
  chatHistoryCommitOutput(sid);
}

export function revokeChatHistoryAttachmentURLs(sid) {
  const msgs = chatHistory.get(sid);
  if (msgs) {
    for (const msg of msgs) {
      if (msg.kind === 'attach' && Array.isArray(msg.attachments)) {
        for (const a of msg.attachments) {
          if (a.url && a.url.startsWith('blob:')) {
            try { URL.revokeObjectURL(a.url); } catch (_) {}
            a.revoked = true;
            a.url = null;
          }
        }
      }
    }
  }
}

export function onChatHistorySessionRemoved(sid) {
  const t = chatHistoryAutoCommitTimers.get(sid);
  if (t) { clearTimeout(t); chatHistoryAutoCommitTimers.delete(sid); }
  chatHistoryOutputBuffers.delete(sid);
  revokeChatHistoryAttachmentURLs(sid);
  chatHistory.delete(sid);
  chatHistorySubs.delete(sid);
  chatHistoryIdSeq.delete(sid);
  // Hub 再起動後などに同じ live ID が別セッションへ再利用された場合でも
  // SQLite からの履歴復元が「復元済み」と誤判定されないよう印を消す
  chatHistoryStoreRestored.delete(sid);
  if (_chatPaneMountedSid === sid) {
    _chatPaneMountedSid = null;
    _chatPaneRenderedMessageIds = new Set();
  }
}

export function resetChatHistoryForSession(sid) {
  const t = chatHistoryAutoCommitTimers.get(sid);
  if (t) { clearTimeout(t); chatHistoryAutoCommitTimers.delete(sid); }
  chatHistoryOutputBuffers.delete(sid);
  // attach メッセージの object URL を解放する
  revokeChatHistoryAttachmentURLs(sid);
  chatHistory.delete(sid);
  chatHistoryIdSeq.delete(sid);
  chatHistoryNotify(sid, null);
}

export function resetAllChatHistory() {
  const ids = new Set([
    ...chatHistory.keys(),
    ...chatHistoryOutputBuffers.keys(),
    ...chatHistoryAutoCommitTimers.keys(),
    ...sessions.keys(),
  ]);
  ids.forEach(id => resetChatHistoryForSession(id));
}

// C2/C3 から（および将来の拡張用に）window 経由でも触れるよう公開
if (typeof window !== 'undefined') {
  window.chatHistoryAPI = {
    getMessages,
    subscribe: subscribeChatHistory,
    push: pushMessage,
  };
}

// チャットの購読: バッジ更新用 (アクティブセッションだけ)
export let _activeChatSubUnsub = null;
export function rewireChatHistorySub(sid) {
  if (_activeChatSubUnsub) { try { _activeChatSubUnsub(); } catch (_) {} _activeChatSubUnsub = null; }
  if (sid === null || sid === undefined) return;
  if (typeof window === 'undefined' || !window.chatHistoryAPI || typeof window.chatHistoryAPI.subscribe !== 'function') return;
  _activeChatSubUnsub = window.chatHistoryAPI.subscribe(sid, (msg) => {
    if (sid !== activeSessionId) return;
    updateChatCountBadge();
    // C3: chat-pane へ増分追加。mount 状態の判定（および chat/split 表示中なのに
    // mount が欠落している場合の remount 保険）は appendMessage 側に一本化する。
    if (msg) appendMessage(sid, msg);
  });
  updateChatCountBadge();
}

if (typeof window !== 'undefined') {
  window.setActiveTab = setActiveTab;
  // C5: renderSessionList を multi-pane.js から呼び出せるよう公開（P<n> バッジ更新用）
  window.renderSessionList = renderSessionList;
  // C9: getSortedSessions の整列ロジックは state.js の orderSessions に集約済み
  // （window.getSortedSessions エイリアスも state.js で定義）。
  // C3: multi-pane.js の attachToSlot から terminals / sendResize にアクセスするための公開
  window.getTerminalEntry = function (id) { return terminals.get(id); };
  window.sendResize = sendResize;
  window.markTerminalManualScrollIntent = markTerminalManualScrollIntent;
  window.updateScrollLockBtn = updateScrollLockBtn;
  Object.defineProperty(window, 'activeSessionId', {
    configurable: true,
    get() { return activeSessionId; },
  });
}

// ─── C4: approvalUiAdapter.setApprovalVisible をラップしてマルチペインバッジを同期 ───
// approval-ui.js は app.js より後にロードされるため、window.load 後にラップする。
// これにより setApprovalVisible(id, true/false) が呼ばれるたびにバッジが更新される。
window.addEventListener('load', function () {
  const adapter = window.approvalUiAdapter;
  if (!adapter || typeof adapter.setApprovalVisible !== 'function' || adapter._c4wrapped) return;
  const orig = adapter.setApprovalVisible;
  adapter.setApprovalVisible = function (id, visible, options) {
    const result = orig.call(this, id, visible, options);
    // マルチペインのバッジを更新（'waiting' または 'running'/'idle' に切り替え）
    const mgr = window.multiPaneManager;
    if (mgr && typeof mgr.updateSlotBadge === 'function') {
      if (visible) {
        mgr.updateSlotBadge(id, 'waiting');
      } else {
        const s = sessions.get(id);
        const badgeStatus = (s && s.state === 'running') ? 'running' : 'standby';
        mgr.updateSlotBadge(id, badgeStatus);
      }
    }
    return result;
  };
  adapter._c4wrapped = true;
});

// ─── C2: バッファクリア機能 ──────────────────────────────────
// マルチモード時の scrollback 上限（localStorage で変更可能）
export function MULTI_SCROLLBACK() {
  return parseInt(localStorage.getItem('multiScrollback') ?? '150', 10);
}

export function clearBuffer(session) {
  if (!session) return;
  // session はセッションオブジェクト。term は terminals Map から取得
  const t = session.id !== undefined ? terminals.get(session.id) : null;
  if (!t || !t.term) return;
  try { t.term.clear(); } catch (_) {}
  // scrollback の縮小はマルチビュー表示中のみ。シングルモードでクリアした場合に
  // 縮小値（150 行）が残り続けると以後の履歴がほぼ遡れなくなるため標準値を維持する。
  const multiViewEl = document.getElementById('multi-view');
  const multiOpen = !!(multiViewEl && !multiViewEl.hidden);
  try { t.term.options.scrollback = multiOpen ? MULTI_SCROLLBACK() : TERMINAL_SCROLLBACK_LINES; } catch (_) {}
  if (session.id !== undefined) {
    resetChatHistoryForSession(session.id);
    if (typeof mountChatPaneForSession === 'function') mountChatPaneForSession(session.id);
  }
}


(function wireBufClearBtn() {
  const btn = document.getElementById('buf-clear-btn');
  if (!btn) return;
  btn.addEventListener('click', () => {
    // マルチモードのとき: フォーカスペインの _targetSession を使う
    const mgr = window.multiPaneManager;
    const multiView = document.getElementById('multi-view');
    const isMultiOpen = multiView && !multiView.hidden;
    if (isMultiOpen && btn._targetSession) {
      clearBuffer(btn._targetSession);
    } else if (isMultiOpen && mgr) {
      // フォーカス未設定の場合は focusedIdx のセッションを使う
      const slot = mgr.slots && mgr.slots[mgr.focusedIdx];
      if (slot && slot.session) clearBuffer(slot.session);
    } else {
      // シングルモード: activeSession
      if (activeSessionId !== null && activeSessionId !== undefined) {
        const s = sessions.get(activeSessionId);
        if (s) clearBuffer(s);
      }
    }
  });
})();

// =========================================================================
// C3: チャットメッセージレンダリング本体
// docs/local/plan_chat-history-subview.md §C3
//
// 主要関数:
//   mountChatPaneForSession(sid)     — chat-pane を再構築
//   appendMessage(sid, msg)           — 1 件 append (新規メッセージのみ)
//   renderMessageBubble(msg, opts)    — DOM 要素を返す
//   renderInlineText(text)            — path / URL / inline-code → DOM 変換
//   parseToolCallsFromOutput(text, provider) — provider 別ツール呼び出し抽出
// =========================================================================

export let _chatPaneMountedSid = null;
export let _chatPaneRenderedMessageIds = new Set();

export function getChatPaneEl() {
  return document.getElementById('chat-pane');
}

export function getChatTimelineEl() {
  const pane = getChatPaneEl();
  if (!pane) return null;
  return pane.querySelector('.chat-timeline');
}

export function chatPaneAtBottom(timeline) {
  if (!timeline) return true;
  const remain = timeline.scrollHeight - timeline.scrollTop - timeline.clientHeight;
  return remain < 60;
}

export function scrollChatPaneToBottom(timeline) {
  if (!timeline) return;
  timeline.scrollTop = timeline.scrollHeight;
}

export function scrollChatPaneToBottomSoon(opts: any = {}) {
  const passes = Math.max(1, Number(opts.passes) || 1);
  const startedAt = opts.startedAt || Date.now();
  const run = (n) => {
    const timeline = getChatTimelineEl();
    if (timeline) scrollChatPaneToBottom(timeline);
    if (n + 1 >= passes) return;
    const elapsed = Date.now() - startedAt;
    const delay = n === 0 ? 16 : Math.min(120, 24 + n * 32);
    if (elapsed + delay > 320) return;
    setTimeout(() => requestAnimationFrame(() => run(n + 1)), delay);
  };
  requestAnimationFrame(() => run(0));
}

export function getAiDisplayName(provider) {
  switch (provider) {
    case 'claude':   return ti18n('chat_ai_name_claude', 'Claude');
    case 'codex':    return ti18n('chat_ai_name_codex', 'Codex');
    case 'copilot':  return ti18n('chat_ai_name_copilot', 'Copilot');
    case 'cursor-agent': return ti18n('chat_ai_name_cursor_agent', 'Cursor Agent');
    case 'ollama':     return ti18n('chat_ai_name_ollama', 'Ollama');
    case 'lm-studio':  return ti18n('chat_ai_name_lm_studio', 'LM Studio');
    case 'opencode': return ti18n('chat_ai_name_opencode', 'OpenCode');
    case 'grok':     return ti18n('chat_ai_name_grok', 'Grok');
    default: return provider ? String(provider) : 'AI';
  }
}

export function getAiAvatarLetter(provider) {
  switch (provider) {
    case 'claude':   return 'C';
    case 'codex':    return 'X';
    case 'cursor-agent': return 'r';
    case 'ollama':     return 'O';
    case 'lm-studio':  return 'L';
    case 'opencode': return 'P';
    case 'grok':     return 'G';
    default: return 'A';
  }
}

export function getUserAvatarLetter() {
  if (_userDisplayName) {
    return [..._userDisplayName][0].toUpperCase();
  }
  const lang = (document.documentElement.lang || '').toLowerCase();
  return lang.startsWith('ja') ? 'あ' : 'Y';
}

export function formatTimestamp(ts) {
  const d = (ts instanceof Date) ? ts : new Date(ts);
  if (Number.isNaN(d.getTime())) return '';
  const pad = (n) => String(n).padStart(2, '0');
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export function formatMsgNumber(id) {
  const n = Math.max(1, Number(id) || 1);
  return '#' + String(n).padStart(3, '0');
}

// テキスト中の URL / ファイルパス / インラインコード を DOM に変換する。
// 戻り値は DocumentFragment（または Node 配列を返さず単体 Node にする）。
export function renderInlineText(text) {
  const frag = document.createDocumentFragment();
  if (!text) return frag;
  const src = String(text);

  // 全体を順次トークナイズ:
  //   - ``` で囲まれたコードブロックは行内ではここでは扱わず、上位 (renderMessageBody) で処理する
  //   - インラインコード `...` を最優先で切り出す
  //   - URL (http(s)://)
  //   - ファイルパス候補（拡張子付き、または絶対パス、または ./ ../ で始まるもの）
  //
  // 単一 regex で全パターン候補を OR して、マッチ位置順に処理する。
  //
  // 注: 簡易実装。括弧やカンマ等の終端で誤検出する可能性はあるが、C4 で
  // 右クリック時にユーザーが手動で paste 修正可能。

  // インラインコードを最初に抽出（バッククォート内は path/URL 検出をしない）
  const codeRe = /`([^`\n]+?)`/g;
  const tokens = []; // {kind:'code'|'plain', text, raw?}
  let idx = 0;
  let m;
  while ((m = codeRe.exec(src)) !== null) {
    if (m.index > idx) tokens.push({ kind: 'plain', text: src.slice(idx, m.index) });
    tokens.push({ kind: 'code', text: m[1] });
    idx = m.index + m[0].length;
  }
  if (idx < src.length) tokens.push({ kind: 'plain', text: src.slice(idx) });

  for (const tk of tokens) {
    if (tk.kind === 'code') {
      const el = document.createElement('span');
      el.className = 'code-inline';
      el.textContent = tk.text;
      frag.appendChild(el);
      continue;
    }
    // plain: URL / path を抽出
    _appendPlainWithLinks(frag, tk.text);
  }
  return frag;
}

// プレーンテキストから URL とファイルパスを抽出し、frag に追加する。
export function _appendPlainWithLinks(frag, text) {
  if (!text) return;
  // URL: http(s)://...
  // path 候補:
  //   - 絶対パス Unix: /usr/... (ただしコードブロック外)
  //   - 絶対パス Windows: C:\... or C:/...
  //   - 相対パス: ./xxx ../xxx
  //   - 拡張子付き相対: foo/bar.ext または bar.ext (拡張子に絞る)
  //
  // 安全側: 末尾の句読点 ,.;:!?) を除外する。

  // 単一の包括 regex
  const re = /(https?:\/\/[^\s<>"'`)\]]+)|((?:[a-zA-Z]:[\\/]|[.]{1,2}[\\/]|\/)[^\s<>"'`(\]]+)|([\w][\w\-/\\.]*\.[a-zA-Z]{1,8}\b)/g;
  let last = 0;
  let m;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) frag.appendChild(document.createTextNode(text.slice(last, m.index)));
    let token = m[0];
    // 末尾の句読点を分離
    let trail = '';
    while (token.length > 0 && /[.,;:!?)\]\}>]/.test(token[token.length - 1])) {
      trail = token[token.length - 1] + trail;
      token = token.slice(0, -1);
    }
    if (!token) {
      frag.appendChild(document.createTextNode(m[0]));
      last = m.index + m[0].length;
      continue;
    }
    if (m[1]) {
      // URL
      const a = document.createElement('a');
      a.className = 'url-link';
      a.href = token;
      a.target = '_blank';
      a.rel = 'noopener noreferrer';
      a.textContent = token;
      frag.appendChild(a);
    } else {
      // path
      const span = document.createElement('span');
      span.className = 'path-link';
      span.dataset.path = token;
      span.textContent = token;
      frag.appendChild(span);
    }
    if (trail) frag.appendChild(document.createTextNode(trail));
    last = m.index + m[0].length;
  }
  if (last < text.length) frag.appendChild(document.createTextNode(text.slice(last)));
}

// PTY output からツール呼び出しを抽出する（provider 別の簡易版）。
// Claude フォーマット: 行頭の `● ToolName(args)` を検出。
// Codex / 他: 同様パターンを暫定で適用（誤マッチは v2 で改善）。
//
// 戻り値: [{ name, args, body }, ...]  body は次のツール呼び出しまでの本文（任意）。
export function parseToolCallsFromOutput(text, _provider) {
  const calls = [];
  if (!text) return calls;
  const lines = String(text).split(/\r?\n/);
  const re = /^[\s•●○●○]*●\s+([A-Z][A-Za-z0-9_]*)\s*\(([^\n]*)\)\s*$/;
  let cur = null;
  for (const line of lines) {
    const m = line.match(re);
    if (m) {
      if (cur) calls.push(cur);
      cur = { name: m[1], args: m[2] || '', body: '' };
    } else if (cur) {
      // body 候補: " ⎿  ..." のような Claude のツール結果行を取り込む
      // ただし不確実なので最大 12 行までに抑える
      const bodyLines = cur.body ? cur.body.split('\n') : [];
      if (bodyLines.length < 12) {
        cur.body = cur.body ? cur.body + '\n' + line : line;
      }
    }
  }
  if (cur) calls.push(cur);
  return calls;
}

// AI メッセージ本文からツール呼び出し行を取り除いた残りテキストを返す。
export function stripToolCallLines(text) {
  if (!text) return '';
  const lines = String(text).split(/\r?\n/);
  const out = [];
  const re = /^[\s•●○●○]*●\s+([A-Z][A-Za-z0-9_]*)\s*\([^\n]*\)\s*$/;
  let skipping = false;
  for (const line of lines) {
    if (re.test(line)) { skipping = true; continue; }
    if (skipping) {
      // ⎿ で始まる結果行はツール呼び出しの一部とみなしてスキップ
      if (/^\s*⎿/.test(line)) continue;
      skipping = false;
    }
    out.push(line);
  }
  return out.join('\n').replace(/\n{3,}/g, '\n\n').trim();
}

// 1 メッセージの DOM を構築する。
export function renderMessageBubble(sid, msg) {
  const sess = (typeof sessions !== 'undefined' && sessions) ? sessions.get(sid) : null;
  const provider = (sess && sess.provider) || 'claude';
  const role = msg.role || 'system';
  const kind = msg.kind || 'text';

  const wrapEl = document.createElement('div');
  wrapEl.className = 'msg ' + role;
  wrapEl.dataset.msgId = String(msg.id);
  wrapEl.dataset.role = role;
  wrapEl.dataset.kind = kind;

  // メッセージ番号
  const numEl = document.createElement('span');
  numEl.className = 'msg-number';
  numEl.textContent = formatMsgNumber(msg.id);
  wrapEl.appendChild(numEl);

  if (role === 'system') {
    // system/approval: 中央寄せのバブル単体
    const bubble = document.createElement('div');
    bubble.className = 'bubble approval';
    const ttl = document.createElement('div');
    ttl.className = 'ttl';
    const icon = document.createElement('span');
    icon.textContent = '⚠';
    ttl.appendChild(icon);
    const title = document.createElement('span');
    title.textContent = ti18n('chat_system_approval_title', '承認待ち');
    ttl.appendChild(title);
    bubble.appendChild(ttl);

    // 質問本文（meta.kind === 'batch' なら複数質問、'single' なら単問）
    const meta = msg.meta || {};
    if (meta.kind === 'batch' && Array.isArray(meta.answers)) {
      for (const ans of meta.answers) {
        const line = document.createElement('div');
        line.appendChild(renderInlineText(String(ans.question || '')));
        bubble.appendChild(line);
      }
    } else if (msg.rawText) {
      const line = document.createElement('div');
      line.appendChild(renderInlineText(msg.normalizedText || msg.rawText));
      bubble.appendChild(line);
    }

    // 回答ライン
    const ans = document.createElement('div');
    ans.className = 'ans';
    let answerStr = '';
    if (meta.kind === 'single') {
      answerStr = meta.label ? `${meta.answer}. ${meta.label}` : String(meta.answer || msg.rawText || '');
    } else if (meta.kind === 'batch' && Array.isArray(meta.answers)) {
      answerStr = meta.answers.map(a => `${a.key}: ${a.answer}`).join(', ');
    } else {
      answerStr = msg.normalizedText || msg.rawText || '';
    }
    const timeStr = formatTimestamp(msg.ts);
    ans.textContent = ti18n('chat_approval_answered_by', `→ ${answerStr} (${timeStr} にあなたが承認)`, {
      answer: answerStr,
      time: timeStr,
    });
    bubble.appendChild(ans);

    wrapEl.appendChild(bubble);
    return wrapEl;
  }

  // user / ai 共通: avatar + bubble-wrap
  const avatar = document.createElement('div');
  avatar.className = 'avatar ' + (role === 'user' ? 'user' : ('ai ' + provider));
  if (role === 'user') {
    if (_userAvatarUrl) {
      const img = document.createElement('img');
      img.src = _userAvatarUrl;
      img.alt = getUserAvatarLetter();
      img.onerror = () => {
        img.remove();
        avatar.textContent = getUserAvatarLetter();
      };
      avatar.appendChild(img);
    } else {
      avatar.textContent = getUserAvatarLetter();
    }
  } else {
    avatar.innerHTML = providerIconHtml(provider, 30);
  }
  wrapEl.appendChild(avatar);

  const bw = document.createElement('div');
  bw.className = 'bubble-wrap';

  // meta line
  const metaEl = document.createElement('div');
  metaEl.className = 'bubble-meta';
  const ts = formatTimestamp(msg.ts);
  if (role === 'user') {
    const nameSpan = document.createElement('span');
    nameSpan.className = 'name';
    nameSpan.textContent = _userDisplayName || ti18n('chat_user', 'あなた');
    metaEl.appendChild(document.createTextNode(ts + ' · '));
    metaEl.appendChild(nameSpan);
  } else {
    const nameSpan = document.createElement('span');
    nameSpan.className = 'name';
    nameSpan.textContent = getAiDisplayName(provider);
    metaEl.appendChild(nameSpan);
    metaEl.appendChild(document.createTextNode(' · ' + ts));
    // AI メッセージのトークン・所要時間（meta 経由 or tool 経由）
    const tool = msg.tool || {};
    const m = msg.meta || {};
    const elapsed = (typeof m.elapsed_ms === 'number') ? m.elapsed_ms
                  : (typeof tool.elapsed_ms === 'number' ? tool.elapsed_ms : null);
    if (elapsed != null) {
      const s = document.createElement('span');
      s.className = 'stat';
      const sec = (elapsed / 1000).toFixed(1) + 's';
      s.innerHTML = `⏱ <b>${escapeHtml(sec)}</b>`;
      metaEl.appendChild(s);
    }
    const tokIn  = (m.tokens_in  != null) ? m.tokens_in  : (tool.tokens_in  != null ? tool.tokens_in  : null);
    const tokOut = (m.tokens_out != null) ? m.tokens_out : (tool.tokens_out != null ? tool.tokens_out : null);
    if (tokIn != null || tokOut != null) {
      const s = document.createElement('span');
      s.className = 'stat';
      const inStr  = tokIn  != null ? Number(tokIn).toLocaleString()  : '–';
      const outStr = tokOut != null ? Number(tokOut).toLocaleString() : '–';
      s.innerHTML = `🪙 in <b>${escapeHtml(inStr)}</b> / out <b>${escapeHtml(outStr)}</b>`;
      metaEl.appendChild(s);
    }
  }
  bw.appendChild(metaEl);

  // bubble 本体
  const bubble = document.createElement('div');
  bubble.className = 'bubble ' + (role === 'user' ? 'user-text' : 'ai-text');

  const content = document.createElement('div');
  content.className = 'bubble-content';

  if (kind === 'attach') {
    // attach メッセージはテキスト無し or 短い rawText のみ
    const txt = msg.normalizedText || msg.rawText || '';
    if (txt) {
      content.appendChild(renderInlineText(txt));
    } else {
      // 添付のみの場合、placeholder テキスト
      const n = Array.isArray(msg.attachments) ? msg.attachments.length : 0;
      content.textContent = ti18n('chat_attachment_count', `${n} 件の添付`, { n });
    }
  } else if (role === 'ai') {
    // AI: ツール呼び出しを抽出してから本文を表示
    const raw = msg.normalizedText || msg.rawText || '';
    const cleanText = stripToolCallLines(raw);
    if (cleanText) content.appendChild(renderInlineText(cleanText));
    bubble.appendChild(content);
    const toolCalls = parseToolCallsFromOutput(raw, provider);
    for (const tc of toolCalls) {
      bubble.appendChild(renderToolCall(tc));
    }
  } else {
    // user/text
    const raw = msg.normalizedText || msg.rawText || '';
    content.appendChild(renderInlineText(raw));
  }

  if (kind !== 'ai' && bubble.childNodes.length === 0) bubble.appendChild(content);
  if (role !== 'ai' || kind === 'attach') {
    // ai 以外は content をまだ append していない
    if (!bubble.contains(content)) bubble.appendChild(content);
  }

  bw.appendChild(bubble);

  // attachments
  if (Array.isArray(msg.attachments) && msg.attachments.length > 0) {
    const ats = document.createElement('div');
    ats.className = 'attachments';
    for (const a of msg.attachments) {
      const thumb = document.createElement('div');
      thumb.className = 'thumb';
      if (a.url) {
        const img = document.createElement('img');
        img.src = a.url;
        img.alt = a.filename || '';
        img.style.cursor = 'pointer';
        img.addEventListener('click', () => {
          if (!a.url || a.revoked) return;
          openLightbox(a.url);
        });
        thumb.appendChild(img);
      } else {
        const icn = document.createElement('span');
        icn.className = 'icn';
        icn.textContent = (a.kind === 'image') ? '🖼' : '📄';
        thumb.appendChild(icn);
      }
      const fname = document.createElement('span');
      fname.className = 'fname';
      fname.textContent = a.filename || '';
      thumb.appendChild(fname);
      ats.appendChild(thumb);
    }
    bw.appendChild(ats);
  }

  // C4: hover action buttons (copy / collapse) と raw link を注入
  if (typeof window !== 'undefined' && typeof window._chatC4DecorateBubble === 'function') {
    try { window._chatC4DecorateBubble(wrapEl, bw, bubble, msg); } catch (_) {}
  }

  wrapEl.appendChild(bw);
  return wrapEl;
}

export function renderToolCall(tc) {
  const wrap = document.createElement('div');
  wrap.className = 'tool-call';

  const header = document.createElement('div');
  header.className = 'tool-call-header';
  const caret = document.createElement('span');
  caret.className = 'caret';
  caret.textContent = '▶';
  header.appendChild(caret);

  const tname = document.createElement('span');
  tname.className = 'tname';
  tname.textContent = tc.name || 'Tool';
  header.appendChild(tname);

  const tdesc = document.createElement('span');
  tdesc.className = 'tdesc';
  tdesc.textContent = tc.args || '';
  header.appendChild(tdesc);

  if (tc.stat) {
    const stat = document.createElement('span');
    stat.className = 'tstat';
    stat.textContent = tc.stat;
    header.appendChild(stat);
  }

  header.addEventListener('click', () => {
    wrap.classList.toggle('open');
  });
  wrap.appendChild(header);

  const body = document.createElement('div');
  body.className = 'tool-call-body';
  body.textContent = tc.body || '';
  wrap.appendChild(body);

  return wrap;
}

export function updateChatPaneEmptyState(sid) {
  const pane = getChatPaneEl();
  if (!pane) return;
  let count = 0;
  try {
    const msgs = (typeof getMessages === 'function') ? getMessages(sid) : [];
    count = Array.isArray(msgs) ? msgs.length : 0;
  } catch (_) {}
  pane.classList.toggle('has-messages', count > 0);
}

// アクティブセッションの chat-pane を完全再構築する。
// セッション切替時 / モード切替で chat 系に入ったときに呼ぶ。
export function mountChatPaneForSession(sid) {
  const pane = getChatPaneEl();
  const timeline = getChatTimelineEl();
  if (!pane || !timeline) return;
  // タイムラインを空にして再構築
  while (timeline.firstChild) timeline.removeChild(timeline.firstChild);
  timeline.dataset.sid = sid != null ? String(sid) : '';
  _chatPaneMountedSid = (sid !== null && sid !== undefined) ? sid : null;
  _chatPaneRenderedMessageIds = new Set();
  if (sid === null || sid === undefined) {
    updateChatPaneEmptyState(sid);
    return;
  }
  if (!chatHistoryStoreRestored.has(sid) && !hasMeaningfulLocalChat(sid)) {
    restoreChatHistoryFromStore(sid).then((restored) => {
      if (restored && _chatPaneMountedSid === sid) mountChatPaneForSession(sid);
    });
  }
  let msgs = [];
  try { msgs = getMessages(sid) || []; } catch (_) {}
  const frag = document.createDocumentFragment();
  for (const m of msgs) {
    _chatPaneRenderedMessageIds.add(String(m.id));
    frag.appendChild(renderMessageBubble(sid, m));
  }
  timeline.appendChild(frag);
  updateChatPaneEmptyState(sid);
  // C4: filter/search/minimap を再構築
  if (typeof window !== 'undefined' && typeof window._chatC4OnRemount === 'function') {
    try { window._chatC4OnRemount(sid); } catch (_) {}
  }
  // 末尾追従
  scrollChatPaneToBottomSoon({ passes: 2 });
}

// display-area が chat / split 表示中かどうか（appendMessage の mount 欠落保険用）
function isChatViewModeActive() {
  const da = document.getElementById('display-area');
  return !!da && (da.classList.contains('mode-chat') || da.classList.contains('mode-split'));
}

// 1 メッセージの増分追加。subscribe コールバックから呼ばれる。
export function appendMessage(sid, msg) {
  if (sid !== _chatPaneMountedSid) {
    // bugfix 2026-06-04 保険: active session が chat/split 表示中なのに mount が
    // 欠落している場合は store から再構築する（msg は push 済みなので mount で描画される）。
    if (sid === activeSessionId && isChatViewModeActive()) {
      console.warn('[chat-history] chat-pane not mounted for active session, remounting:', sid);
      mountChatPaneForSession(sid);
    }
    return;
  }
  const timeline = getChatTimelineEl();
  if (!timeline) return;
  const wasAtBottom = chatPaneAtBottom(timeline);
  // 既に同 id がある場合は skip（重複防止）
  const msgId = String(msg.id);
  if (_chatPaneRenderedMessageIds.has(msgId)) return;
  _chatPaneRenderedMessageIds.add(msgId);
  timeline.appendChild(renderMessageBubble(sid, msg));
  updateChatPaneEmptyState(sid);
  // C4: 増分のフィルタ/検索/ミニマップ更新
  if (typeof window !== 'undefined' && typeof window._chatC4OnAppend === 'function') {
    try { window._chatC4OnAppend(sid, msg); } catch (_) {}
  }
  if (wasAtBottom) scrollChatPaneToBottomSoon({ passes: 2 });
}

// setActiveTab が chat/split に切り替わるタイミングで chat-pane の中身を保証する。
// bugfix 2026-06-04: 以前は window.setActiveTab を wrapper で差し替えていたが、
// settings.js 内の主要経路（タブクリック / applyActiveSessionViewMode）は ESM import の
// setActiveTab を直接呼ぶため wrapper を通らず、mount が欠落していた。
// monkey patch を廃止し、settings.js の setActiveTab() が DOM mode 確定後に発火する
// 'session-view-mode-changed' event を購読して mount する方式に変更。
if (typeof window !== 'undefined') {
  window.addEventListener('session-view-mode-changed', (ev) => {
    const { sid, name } = (ev && ev.detail) || {};
    if (sid === null || sid === undefined) return;
    if (sid !== activeSessionId) return;
    if (name !== 'chat' && name !== 'split') return;
    try {
      // 既に同 sid でマウント済みなら差分のみ。違う sid なら再構築。
      if (_chatPaneMountedSid !== sid) {
        mountChatPaneForSession(sid);
      } else {
        // 念のためスクロール末尾追従
        const tl = getChatTimelineEl();
        if (tl) requestAnimationFrame(() => scrollChatPaneToBottom(tl));
      }
    } catch (e) {
      console.warn('[mountChatPaneForSession] failed:', e);
    }
  });
  window.mountChatPaneForSession = mountChatPaneForSession;
  window.appendChatMessage = appendMessage;
}

// =========================================================================
// C4: チャットの補助機能 (子 plan plan_chat-history-subview_c4_extras.md)
//   - 子 C1: .path-link 右クリックメニュー (showPathPopup 流用)
//   - 子 C2: 吹き出し hover アクション (コピー / 折りたたみ / raw)
//            #btn-expand-all / #btn-collapse-all / #btn-raw-log 本実装
//   - 子 C3: 検索 (Ctrl+F) + フィルタチップ
//   - 子 C4: ミニマップ + J/K/Esc キーボード操作
// =========================================================================
(function initC4ChatExtras() {
  // ---- DOM ヘルパ ---------------------------------------------------------
  function chatPane() { return document.getElementById('chat-pane'); }
  function chatTimeline() {
    const p = chatPane();
    return p ? p.querySelector('.chat-timeline') : null;
  }
  function isChatVisible() {
    const p = chatPane();
    if (!p) return false;
    // hidden 属性 / 親 display:none を考慮
    if (p.hidden) return false;
    const rect = p.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }
  function activeMode() {
    const da = document.getElementById('display-area');
    if (!da) return null;
    if (da.classList.contains('mode-chat')) return 'chat';
    if (da.classList.contains('mode-split')) return 'split';
    if (da.classList.contains('mode-terminal')) return 'terminal';
    if (da.classList.contains('mode-files')) return 'files';
    if (da.classList.contains('mode-git')) return 'git';
    return null;
  }

  // =====================================================================
  // 子 C1: .path-link 右クリックメニュー
  // =====================================================================
  document.addEventListener('contextmenu', (e) => {
    const link = e.target.closest && e.target.closest('#chat-pane .path-link');
    if (!link) return;
    const p = link.dataset.path || link.textContent || '';
    if (!p) return;
    e.preventDefault();
    e.stopPropagation();
    // 既存ターミナルの showPathPopup を再利用
    if (typeof showPathPopup === 'function') {
      try { showPathPopup(p, e.clientX, e.clientY, activeSessionId); } catch (_) {}
    }
  }, true);

  // 左クリックでも popup を出す (ターミナル既存挙動と揃える)
  document.addEventListener('click', (e) => {
    const link = e.target.closest && e.target.closest('#chat-pane .path-link');
    if (!link) return;
    const p = link.dataset.path || link.textContent || '';
    if (!p) return;
    e.preventDefault();
    e.stopPropagation();
    if (typeof showPathPopup === 'function') {
      try { showPathPopup(p, e.clientX, e.clientY, activeSessionId); } catch (_) {}
    }
  });

  // =====================================================================
  // 子 C2: 吹き出し hover アクション (.bubble-actions)
  // =====================================================================
  // renderMessageBubble の最後で呼ばれる装飾フック
  window._chatC4DecorateBubble = function (wrapEl, bw, bubble, msg) {
    const role = (msg && msg.role) || 'system';
    if (role === 'system') return;
    // hover アクション群
    const acts = document.createElement('div');
    acts.className = 'bubble-actions';
    // copy ボタン
    const copyBtn = document.createElement('button');
    copyBtn.type = 'button';
    copyBtn.className = 'msg-action msg-action-copy';
    copyBtn.title = ti18n('chat_copy_btn', 'コピー');
    copyBtn.textContent = '📋';
    copyBtn.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      const text = (msg && (msg.normalizedText || msg.rawText)) || (bubble && bubble.textContent) || '';
      try {
        navigator.clipboard.writeText(text);
        const prev = copyBtn.textContent;
        copyBtn.textContent = '✓';
        copyBtn.classList.add('copied');
        setTimeout(() => { copyBtn.textContent = prev; copyBtn.classList.remove('copied'); }, 1000);
      } catch (_) {}
    });
    acts.appendChild(copyBtn);
    // 折りたたみボタン (bubble に .collapsed クラス)
    const collBtn = document.createElement('button');
    collBtn.type = 'button';
    collBtn.className = 'msg-action msg-action-collapse';
    collBtn.title = ti18n('chat_collapse_btn', '折りたたみ');
    collBtn.textContent = '–';
    collBtn.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      bubble.classList.toggle('collapsed');
      collBtn.textContent = bubble.classList.contains('collapsed') ? '+' : '–';
    });
    acts.appendChild(collBtn);

    // raw リンク (📄 raw)
    const rawWrap = document.createElement('div');
    rawWrap.className = 'bubble-raw-link';
    rawWrap.textContent = '📄 raw';
    rawWrap.title = ti18n('chat_raw_modal_title', '生 PTY テキスト');
    rawWrap.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      openRawModal(msg);
    });

    // actions と raw リンクを同じ行に並べる footer
    const footer = document.createElement('div');
    footer.className = 'bubble-footer';
    footer.appendChild(acts);
    footer.appendChild(rawWrap);
    bw.appendChild(footer);
  };

  // raw モーダル
  let _rawModalEl = null;
  function openRawModal(msg) {
    closeRawModal();
    const overlay = document.createElement('div');
    overlay.className = 'chat-raw-modal-overlay';
    const dlg = document.createElement('div');
    dlg.className = 'chat-raw-modal';
    const head = document.createElement('div');
    head.className = 'chat-raw-modal-head';
    const title = document.createElement('span');
    title.className = 'chat-raw-modal-title';
    title.textContent = ti18n('chat_raw_modal_title', '生 PTY テキスト') + ' ' + (msg && msg.id != null ? '#' + String(msg.id).padStart(3, '0') : '');
    head.appendChild(title);
    const copyBtn = document.createElement('button');
    copyBtn.type = 'button';
    copyBtn.className = 'chat-raw-modal-btn';
    copyBtn.textContent = '📋';
    copyBtn.title = ti18n('chat_copy_btn', 'コピー');
    const closeBtn = document.createElement('button');
    closeBtn.type = 'button';
    closeBtn.className = 'chat-raw-modal-btn chat-raw-modal-close';
    closeBtn.textContent = '✕';
    head.appendChild(copyBtn);
    head.appendChild(closeBtn);
    dlg.appendChild(head);
    const body = document.createElement('pre');
    body.className = 'chat-raw-modal-body';
    body.textContent = (msg && (msg.rawText || msg.normalizedText)) || '';
    dlg.appendChild(body);
    overlay.appendChild(dlg);
    document.body.appendChild(overlay);
    _rawModalEl = overlay;
    closeBtn.addEventListener('click', closeRawModal);
    overlay.addEventListener('click', (e) => { if (e.target === overlay) closeRawModal(); });
    copyBtn.addEventListener('click', () => {
      try {
        navigator.clipboard.writeText(body.textContent || '');
        copyBtn.textContent = '✓';
        setTimeout(() => { copyBtn.textContent = '📋'; }, 1000);
      } catch (_) {}
    });
  }
  function closeRawModal() {
    if (_rawModalEl) { try { _rawModalEl.remove(); } catch (_) {} _rawModalEl = null; }
  }

  // ---- 全展開 / 全折りたたみ / 生ログ (buildFilterBar から呼ぶ) ----------
  function expandAllTools() {
    const p = chatPane();
    if (!p) return;
    p.querySelectorAll('.tool-call').forEach(tc => tc.classList.add('open'));
  }
  function collapseAllTools() {
    const p = chatPane();
    if (!p) return;
    p.querySelectorAll('.tool-call').forEach(tc => tc.classList.remove('open'));
  }
  async function openRawLog() {
    try {
      const sess = activeSessionId != null ? sessions.get(activeSessionId) : null;
      const targetPath = sess && (sess.jsonl_path || sess.log_path || sess.JSONLPath || sess.LogPath);
      if (!targetPath) {
        showToast(ti18n('chat_raw_log_open_failed', '生ログを開けませんでした'));
        return;
      }
      const res = await fetch(`/api/open-dir?token=${encodeURIComponent(token)}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ kind: 'path', path: targetPath }),
      });
      if (!res.ok) {
        showToast(ti18n('chat_raw_log_open_failed', '生ログを開けませんでした'));
      }
    } catch (_) {
      showToast(ti18n('chat_raw_log_open_failed', '生ログを開けませんでした'));
    }
  }

  async function restoreCurrentChatFromStore() {
    if (activeSessionId == null) return;
    const restored = await restoreChatHistoryFromStore(activeSessionId, { force: true });
    if (restored) {
      mountChatPaneForSession(activeSessionId);
      showToast(ti18n('chat_db_restore_done', 'SQLite からチャットを復元しました'));
    } else {
      showToast(ti18n('chat_db_restore_empty', '復元できる履歴はありません'));
    }
  }

  function ensureGlobalSearchBox() {
    const pane = chatPane();
    if (!pane) return null;
    let box = pane.querySelector('.chat-global-search-results');
    if (!box) {
      box = document.createElement('div');
      box.className = 'chat-global-search-results';
      const bar = pane.querySelector('.chat-filter-bar');
      if (bar && bar.parentNode) bar.parentNode.insertBefore(box, bar.nextSibling);
      else pane.insertBefore(box, pane.firstChild);
    }
    return box;
  }

  async function runGlobalSearchFromBar() {
    const q = String(_searchQuery || getFilterBarInput()?.value || '').trim();
    if (!q) {
      showToast(ti18n('chat_global_search_needs_query', '検索語を入力してください'));
      return;
    }
    const box = ensureGlobalSearchBox();
    if (box) {
      box.hidden = false;
      box.innerHTML = '<div class="chat-global-search-head">検索中...</div>';
    }
    try {
      const res = await fetch(`/api/session-search?token=${encodeURIComponent(token)}&q=${encodeURIComponent(q)}&limit=30`);
      if (!res.ok) throw new Error(`status ${res.status}`);
      const data = await res.json();
      renderGlobalSearchResults(q, Array.isArray(data.results) ? data.results : []);
    } catch (err) {
      console.warn('[chatHistory] global search failed', err);
      if (box) box.innerHTML = '<div class="chat-global-search-head">横断検索に失敗しました</div>';
    }
  }

  function renderGlobalSearchResults(query, results) {
    const box = ensureGlobalSearchBox();
    if (!box) return;
    box.hidden = false;
    box.innerHTML = '';
    const head = document.createElement('div');
    head.className = 'chat-global-search-head';
    const title = document.createElement('span');
    title.textContent = `横断検索: ${query} (${results.length})`;
    const close = document.createElement('button');
    close.type = 'button';
    close.textContent = '閉じる';
    close.addEventListener('click', () => { box.hidden = true; });
    head.appendChild(title);
    head.appendChild(close);
    box.appendChild(head);

    if (results.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'chat-global-search-empty';
      empty.textContent = '一致する履歴はありません';
      box.appendChild(empty);
      return;
    }
    for (const r of results) {
      const item = document.createElement('button');
      item.type = 'button';
      item.className = 'chat-global-search-item';
      const meta = document.createElement('span');
      meta.className = 'chat-global-search-meta';
      const folder = String(r.cwd || '').replace(/\\/g, '/').split('/').filter(Boolean).pop() || r.cwd || '';
      meta.textContent = `${r.provider || 'ai'} #${r.session_id || '?'} ${folder} ${r.branch ? '[' + r.branch + ']' : ''}`;
      const body = document.createElement('span');
      body.className = 'chat-global-search-snippet';
      body.textContent = r.snippet || r.text || '';
      item.appendChild(meta);
      item.appendChild(body);
      item.addEventListener('click', async () => {
        if (r.session_id && sessions.has(r.session_id)) {
          activateSession(r.session_id);
          await restoreChatHistoryFromStore(r.session_id, { force: false });
          // bugfix 2026-06-04: 旧 window.setActiveTab('chat') は第1引数に 'chat' を渡す
          // 誤りで no-op だった。import した setActiveTab を正しい引数で呼ぶ。
          setActiveTab(r.session_id, 'chat');
          // restore 完了後の再構築を保証（event 経由 mount は restore 前の場合がある）
          mountChatPaneForSession(r.session_id);
        }
      });
      box.appendChild(item);
    }
  }

  // =====================================================================
  // 子 C3: 検索 + フィルタチップ (.chat-filter-bar 内容構築)
  // =====================================================================
  let _activeFilters = new Set(); // empty = show all
  let _searchQuery = '';
  let _searchHits = []; // [el, el, ...] 表示順
  let _searchCursor = -1;

  function buildFilterBar() {
    const pane = chatPane();
    if (!pane) return;
    const bar = pane.querySelector('.chat-filter-bar');
    if (!bar) return;
    if (bar.dataset.c4Built === '1') return;
    bar.dataset.c4Built = '1';
    bar.hidden = false;

    const chips = [
      { key: 'all',      label: ti18n('chat_filter_all', 'すべて') },
      { key: 'user',     label: '📝 ' + ti18n('chat_filter_user', '入力') },
      { key: 'ai',       label: '🤖 ' + ti18n('chat_filter_ai', 'AI出力') },
      { key: 'attach',   label: '📎 ' + ti18n('chat_filter_attach', '添付') },
      { key: 'approval', label: '⚠ ' + ti18n('chat_filter_approval', '承認') },
    ];
    for (const c of chips) {
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'filter-chip' + (c.key === 'all' ? ' active' : '');
      b.dataset.kind = c.key;
      b.innerHTML = '';
      const lab = document.createElement('span');
      lab.className = 'filter-chip-label';
      lab.textContent = c.label;
      const count = document.createElement('span');
      count.className = 'count';
      count.textContent = '0';
      b.appendChild(lab);
      b.appendChild(count);
      b.addEventListener('click', () => {
        if (c.key === 'all') {
          _activeFilters.clear();
        } else {
          if (_activeFilters.has(c.key)) _activeFilters.delete(c.key);
          else _activeFilters.add(c.key);
        }
        bar.querySelectorAll('.filter-chip').forEach(x => {
          x.classList.toggle('active', x.dataset.kind === 'all'
            ? _activeFilters.size === 0
            : _activeFilters.has(x.dataset.kind));
        });
        applyFilterAndSearch();
        const tl = chatTimeline();
        if (tl) tl.scrollTop = 0;
      });
      bar.appendChild(b);
    }

    // 全展開 / 全折りたたみ / 生ログ / 検索を同じ行グループにまとめる。
    const actionGroup = document.createElement('div');
    actionGroup.className = 'chat-filter-actions';

    // 全展開 / 全折りたたみ / 生ログ (承認チップの右隣)
    const iconBtnDefs = [
      { id: 'btn-expand-all',   icon: '⊞', label: ti18n('btn_expand_all', '全展開'),       tip: ti18n('btn_expand_all_tooltip', '全てのツール呼び出しを展開'),       fn: () => expandAllTools() },
      { id: 'btn-collapse-all', icon: '⊟', label: ti18n('btn_collapse_all', '全折りたたみ'), tip: ti18n('btn_collapse_all_tooltip', '全てのツール呼び出しを折りたたみ'), fn: () => collapseAllTools() },
      { id: 'btn-raw-log',      icon: '📄', label: ti18n('btn_raw_log', '生ログ'),           tip: ti18n('btn_raw_log_tooltip', '生ログを開く'),                       fn: () => openRawLog() },
      { id: 'btn-db-restore',   icon: '↺', label: ti18n('btn_db_restore', 'DB復元'),         tip: ti18n('btn_db_restore_tooltip', 'SQLite からチャットを復元'),      fn: () => restoreCurrentChatFromStore() },
      { id: 'btn-global-search', icon: '⌕', label: ti18n('btn_global_search', '横断'),       tip: ti18n('btn_global_search_tooltip', 'SQLite の全セッション履歴を検索'), fn: () => runGlobalSearchFromBar() },
    ];
    for (const def of iconBtnDefs) {
      const ib = document.createElement('button');
      ib.id = def.id;
      ib.type = 'button';
      ib.className = 'icon-btn';
      ib.dataset.tooltip = def.tip;
      ib.innerHTML = `<span>${def.icon}</span><span>${def.label}</span>`;
      ib.addEventListener('click', (e) => { e.preventDefault(); def.fn(); });
      actionGroup.appendChild(ib);
    }

    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'chat-search-input';
    input.placeholder = ti18n('chat_search_placeholder', '履歴を検索 (Ctrl+F)');
    input.addEventListener('input', () => {
      _searchQuery = input.value || '';
      applyFilterAndSearch();
    });
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        if (e.shiftKey) jumpSearch(-1); else jumpSearch(+1);
      } else if (e.key === 'Escape') {
        e.preventDefault();
        input.value = '';
        _searchQuery = '';
        applyFilterAndSearch();
        input.blur();
      }
    });
    actionGroup.appendChild(input);
    bar.appendChild(actionGroup);
    bar._searchInput = input;
  }

  function getFilterBarInput() {
    const pane = chatPane();
    if (!pane) return null;
    const bar = pane.querySelector('.chat-filter-bar');
    return (bar && bar._searchInput) ? bar._searchInput : null;
  }

  function classifyMsgEl(el) {
    const role = el.dataset.role || '';
    const kind = el.dataset.kind || '';
    if (role === 'system' || kind === 'approval') return 'approval';
    if (kind === 'attach') return 'attach';
    if (role === 'user') return 'user';
    if (role === 'ai') return 'ai';
    return 'other';
  }

  function applyFilterAndSearch() {
    const tl = chatTimeline();
    const pane = chatPane();
    if (!tl || !pane) return;
    const bar = pane.querySelector('.chat-filter-bar');
    const counts = { all: 0, user: 0, ai: 0, attach: 0, approval: 0 };
    const q = String(_searchQuery || '').toLowerCase();
    _searchHits = [];
    const msgs = tl.querySelectorAll('.msg');
    msgs.forEach(el => {
      const cat = classifyMsgEl(el);
      counts.all++;
      if (counts[cat] != null) counts[cat]++;
      // フィルタ
      const filterOk = _activeFilters.size === 0 || _activeFilters.has(cat);
      // 検索 (テキスト含有判定 + <mark> 化)
      // mark を毎回剥がして再適用
      unmarkInside(el);
      let searchOk = true;
      if (q) {
        const text = (el.textContent || '').toLowerCase();
        searchOk = text.indexOf(q) >= 0;
        if (searchOk) {
          highlightInside(el, q);
          _searchHits.push(el);
        }
      }
      el.classList.toggle('search-hit', !!(q && searchOk));
      el.style.display = (filterOk && searchOk) ? '' : 'none';
    });
    // counts 反映
    if (bar) {
      bar.querySelectorAll('.filter-chip').forEach(b => {
        const k = b.dataset.kind;
        const c = b.querySelector('.count');
        if (c) c.textContent = String(counts[k] != null ? counts[k] : 0);
      });
    }
    // 検索カーソルリセット
    _searchCursor = (_searchHits.length > 0) ? 0 : -1;
    if (_searchCursor >= 0) markSearchCurrent();
    // ミニマップ更新
    rebuildMinimap();
  }

  function unmarkInside(root) {
    // mark タグを外して元のテキストに戻す
    const marks = root.querySelectorAll('mark.chat-search-mark');
    marks.forEach(m => {
      const tx = document.createTextNode(m.textContent || '');
      m.parentNode.replaceChild(tx, m);
    });
    // 連続テキストを正規化
    root.normalize();
  }
  function highlightInside(root, query) {
    if (!query) return;
    const q = query.toLowerCase();
    // テキストノードのみを対象に置換 (script, style, mark 内はスキップ)
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
      acceptNode(node) {
        const p = node.parentNode;
        if (!p) return NodeFilter.FILTER_REJECT;
        if (p.nodeName === 'MARK' || p.nodeName === 'SCRIPT' || p.nodeName === 'STYLE') return NodeFilter.FILTER_REJECT;
        if (!node.nodeValue) return NodeFilter.FILTER_REJECT;
        if (node.nodeValue.toLowerCase().indexOf(q) < 0) return NodeFilter.FILTER_REJECT;
        return NodeFilter.FILTER_ACCEPT;
      },
    });
    const targets = [];
    let n;
    while ((n = walker.nextNode())) targets.push(n);
    for (const tn of targets) {
      const text = tn.nodeValue;
      const lower = text.toLowerCase();
      let idx = 0;
      const frag = document.createDocumentFragment();
      let pos = 0;
      while ((idx = lower.indexOf(q, pos)) !== -1) {
        if (idx > pos) frag.appendChild(document.createTextNode(text.slice(pos, idx)));
        const mk = document.createElement('mark');
        mk.className = 'chat-search-mark';
        mk.textContent = text.slice(idx, idx + q.length);
        frag.appendChild(mk);
        pos = idx + q.length;
      }
      if (pos < text.length) frag.appendChild(document.createTextNode(text.slice(pos)));
      tn.parentNode.replaceChild(frag, tn);
    }
  }
  function markSearchCurrent() {
    const tl = chatTimeline();
    if (!tl) return;
    tl.querySelectorAll('.msg.search-current').forEach(el => el.classList.remove('search-current'));
    if (_searchCursor >= 0 && _searchCursor < _searchHits.length) {
      const el = _searchHits[_searchCursor];
      el.classList.add('search-current');
      try { el.scrollIntoView({ behavior: 'smooth', block: 'center' }); } catch (_) {}
    }
  }
  function jumpSearch(delta) {
    if (_searchHits.length === 0) return;
    _searchCursor = (_searchCursor + delta + _searchHits.length) % _searchHits.length;
    markSearchCurrent();
  }

  // =====================================================================
  // 子 C4: ミニマップ
  // =====================================================================
  let _filterBarResizeObserver = null;
  function syncMinimapOffset() {
    const pane = chatPane();
    if (!pane) return;
    const bar = pane.querySelector('.chat-filter-bar');
    const h = (bar && !bar.hidden) ? Math.ceil(bar.getBoundingClientRect().height) : 0;
    pane.style.setProperty('--chat-filter-bar-height', h + 'px');
  }
  function observeFilterBarForMinimap() {
    const pane = chatPane();
    if (!pane) return;
    const bar = pane.querySelector('.chat-filter-bar');
    if (!bar || bar.dataset.minimapResizeObserved === '1') {
      syncMinimapOffset();
      return;
    }
    bar.dataset.minimapResizeObserved = '1';
    syncMinimapOffset();
    if (typeof ResizeObserver !== 'undefined') {
      if (!_filterBarResizeObserver) {
        _filterBarResizeObserver = new ResizeObserver(() => syncMinimapOffset());
      }
      _filterBarResizeObserver.observe(bar);
    }
  }
  function ensureMinimap() {
    const pane = chatPane();
    if (!pane) return null;
    observeFilterBarForMinimap();
    let mm = pane.querySelector('.minimap');
    if (!mm) {
      mm = document.createElement('div');
      mm.className = 'minimap';
      mm.title = ti18n('chat_minimap_title', 'メッセージへジャンプ');
      pane.appendChild(mm);
    }
    return mm;
  }
  function rebuildMinimap() {
    const mm = ensureMinimap();
    const tl = chatTimeline();
    if (!mm || !tl) return;
    while (mm.firstChild) mm.removeChild(mm.firstChild);
    const msgs = Array.from(tl.querySelectorAll('.msg')).filter(el => el.style.display !== 'none');
    for (const el of msgs) {
      const t = document.createElement('div');
      t.className = 'mm-tick';
      const cat = classifyMsgEl(el);
      if (cat === 'user') t.classList.add('mm-user');
      else if (cat === 'ai') t.classList.add('mm-ai');
      else t.classList.add('mm-system');
      const id = el.dataset.msgId || '?';
      const role = el.dataset.role || '';
      const rawText = (el.querySelector('.bubble-content')?.innerText || '').trim().replace(/\s+/g, ' ');
      const preview = rawText.length > 15 ? rawText.slice(0, 15) + '…' : rawText;
      t.dataset.label = preview || `#${String(id).padStart(3, '0')} ${role}`;
      t.addEventListener('click', () => {
        try { el.scrollIntoView({ behavior: 'smooth', block: 'start' }); } catch (_) {}
      });
      // 紐付け
      t._linkedMsg = el;
      mm.appendChild(t);
    }
    setupMinimapObserver();
    setupMinimapMagnification(mm);
  }
  let _mmObserver = null;
  function setupMinimapObserver() {
    if (_mmObserver) { try { _mmObserver.disconnect(); } catch (_) {} _mmObserver = null; }
    const tl = chatTimeline();
    const mm = ensureMinimap();
    if (!tl || !mm) return;
    const ticks = Array.from(mm.querySelectorAll('.mm-tick'));
    const map = new Map();
    for (const tk of ticks) if (tk._linkedMsg) map.set(tk._linkedMsg, tk);
    _mmObserver = new IntersectionObserver((entries) => {
      // ビューポート中央付近にあるメッセージを current に
      let bestEl = null;
      let bestRatio = 0;
      for (const ent of entries) {
        if (ent.intersectionRatio > bestRatio) {
          bestRatio = ent.intersectionRatio;
          bestEl = ent.target;
        }
      }
      // 全 tick から current 解除
      for (const tk of ticks) tk.classList.remove('mm-current');
      if (bestEl && map.has(bestEl)) map.get(bestEl).classList.add('mm-current');
    }, { root: tl, threshold: [0, 0.25, 0.5, 0.75, 1] });
    for (const el of map.keys()) _mmObserver.observe(el);
  }
  let _mmMagCleanup = null;
  function setupMinimapMagnification(mm) {
    if (_mmMagCleanup) { _mmMagCleanup(); _mmMagCleanup = null; }
    if (!mm) return;
    const MAX_ADD = 4;
    const SIGMA = 36;
    function onMove(e) {
      const rect = mm.getBoundingClientRect();
      const mouseY = e.clientY - rect.top;
      const ticks = mm.querySelectorAll('.mm-tick');
      for (const tk of ticks) {
        const tkRect = tk.getBoundingClientRect();
        const center = tkRect.top - rect.top + tkRect.height / 2;
        const dist = Math.abs(mouseY - center);
        const extra = MAX_ADD * Math.exp(-(dist * dist) / (2 * SIGMA * SIGMA));
        tk.style.flexGrow = (1 + extra).toFixed(2);
      }
    }
    function onLeave() {
      const ticks = mm.querySelectorAll('.mm-tick');
      for (const tk of ticks) {
        tk.style.flexGrow = '';
      }
    }
    mm.addEventListener('mousemove', onMove);
    mm.addEventListener('mouseleave', onLeave);
    _mmMagCleanup = () => {
      mm.removeEventListener('mousemove', onMove);
      mm.removeEventListener('mouseleave', onLeave);
    };
  }

  // =====================================================================
  // 子 C4: J / K / Esc キーボード操作
  // =====================================================================
  function getVisibleMessages() {
    const tl = chatTimeline();
    if (!tl) return [];
    return Array.from(tl.querySelectorAll('.msg')).filter(el => el.style.display !== 'none');
  }
  function getFocusedMessageIndex(list) {
    // .search-current 優先、無ければビューポート中央に最も近いもの
    if (list.length === 0) return -1;
    const idx = list.findIndex(el => el.classList.contains('msg-focus'));
    if (idx >= 0) return idx;
    const idxS = list.findIndex(el => el.classList.contains('search-current'));
    if (idxS >= 0) return idxS;
    const tl = chatTimeline();
    if (!tl) return 0;
    const center = tl.scrollTop + tl.clientHeight / 2;
    let best = 0;
    let bestDist = Infinity;
    for (let i = 0; i < list.length; i++) {
      const r = list[i];
      const mid = r.offsetTop + r.offsetHeight / 2;
      const d = Math.abs(mid - center);
      if (d < bestDist) { bestDist = d; best = i; }
    }
    return best;
  }
  function focusMessage(el) {
    const tl = chatTimeline();
    if (!tl || !el) return;
    tl.querySelectorAll('.msg.msg-focus').forEach(x => x.classList.remove('msg-focus'));
    el.classList.add('msg-focus');
    try { el.scrollIntoView({ behavior: 'smooth', block: 'center' }); } catch (_) {}
  }

  document.addEventListener('keydown', (e) => {
    // モーダル open 時の Esc
    if (e.key === 'Escape' && _rawModalEl) { e.preventDefault(); closeRawModal(); return; }

    // テキスト入力中は J/K/Ctrl+F は素通し (ただし検索 input への Ctrl+F フォーカスは許可)
    const ae = document.activeElement;
    const inSearch = !!(ae && ae.classList && ae.classList.contains('chat-search-input'));
    const inOtherInput = !!(ae && (ae.tagName === 'INPUT' || ae.tagName === 'TEXTAREA') && !inSearch);

    // Ctrl+F: chat タブ表示中なら検索 input にフォーカス
    if ((e.ctrlKey || e.metaKey) && (e.key === 'f' || e.key === 'F')) {
      if (!isChatVisible()) return;
      const mode = activeMode();
      if (mode !== 'chat' && mode !== 'split') return;
      const inp = getFilterBarInput();
      if (!inp) return;
      e.preventDefault();
      e.stopPropagation();
      try { inp.focus(); inp.select && inp.select(); } catch (_) {}
      return;
    }

    // J / K: chat タブのみ。検索 input / IME / 通常 input にフォーカスがあるときは無効
    if (e.key === 'j' || e.key === 'J' || e.key === 'k' || e.key === 'K') {
      if (e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return;
      if (inSearch || inOtherInput) return;
      if (!isChatVisible()) return;
      const mode = activeMode();
      // terminal モードは xterm 優先 (J/K を介入しない)
      if (mode === 'terminal' || mode === 'files' || mode === 'git') return;
      const list = getVisibleMessages();
      if (list.length === 0) return;
      const cur = getFocusedMessageIndex(list);
      let next;
      if (e.key === 'j' || e.key === 'J') next = Math.min(list.length - 1, cur + 1);
      else next = Math.max(0, cur - 1);
      e.preventDefault();
      e.stopPropagation();
      focusMessage(list[next]);
      return;
    }

    // Esc: 検索クリア (chat タブ表示中のみ)
    if (e.key === 'Escape') {
      if (!isChatVisible()) return;
      const mode = activeMode();
      if (mode !== 'chat' && mode !== 'split') return;
      const inp = getFilterBarInput();
      if (inp && (inp.value !== '' || _searchQuery !== '')) {
        e.preventDefault();
        inp.value = '';
        _searchQuery = '';
        applyFilterAndSearch();
        return;
      }
    }
  });

  // =====================================================================
  // ライフサイクルフック: mount / append でフィルタ・ミニマップを更新
  // =====================================================================
  window._chatC4OnRemount = function (_sid) {
    buildFilterBar();
    applyFilterAndSearch();
    rebuildMinimap();
  };
  window._chatC4OnAppend = function (_sid, _msg) {
    // 1 件追加。フィルタ・検索を最新の状態に再適用 (count・hit 配列の更新)
    applyFilterAndSearch();
  };

  // 初回マウント済みの場合に備えて初期化を試みる
  try { buildFilterBar(); } catch (_) {}
})();
