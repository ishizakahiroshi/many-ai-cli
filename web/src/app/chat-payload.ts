// chat-payload.ts — 内蔵 API プロキシ経由で捕捉した payload ベースのチャット履歴。
//
// PTY スクレイプによる従来のチャット履歴と並列に動作する（既存実装は破壊しない）。
// チャットペイン上部のトグルで「Payload表示 (β)」を ON にすると、payload ベースの
// クリーンなターン一覧を表示する。
//
// 受信メッセージ:
//   chat_turn           — 1 ターン追記（プロキシが新規捕捉した）
//   chat_turns_snapshot — UI 接続時の既存履歴一括（最大 chatHistoryRingSize 件）
//
// 各ターン右上に [Raw] トグルがあり、押下で生 JSON を <pre> で開閉する。

export interface ChatTurn {
  id: number;
  provider: string; // "anthropic" | "openai"
  endpoint: string;
  received_at: string;
  duration_ms?: number;
  status_code?: number;
  is_stream?: boolean;
  truncated?: boolean;
  model?: string;
  message_count?: number;
  tool_count?: number;
  has_system?: boolean;
  tokens_in?: number;
  tokens_out?: number;
  request_json?: string;
  response_json?: string;
  error_text?: string;
}

// session ID → turns[]（id 昇順）
const turnsBySession = new Map<number, ChatTurn[]>();

// 現在表示中のセッション
let mountedSessionId: number | null = null;

// Payload 表示モード ON/OFF
let payloadMode = false;

function getTimelineEl(): HTMLElement | null {
  return document.querySelector('#chat-pane .chat-payload-timeline') as HTMLElement | null;
}

function getLegacyTimelineEl(): HTMLElement | null {
  return document.querySelector('#chat-pane .chat-timeline') as HTMLElement | null;
}

function getBadgeEl(): HTMLElement | null {
  return document.getElementById('chat-source-badge');
}

function getEmptyEl(): HTMLElement | null {
  return document.querySelector('#chat-pane .chat-pane-empty') as HTMLElement | null;
}

function setPayloadModeUI(on: boolean) {
  const payloadEl = getTimelineEl();
  const legacyEl = getLegacyTimelineEl();
  const badgeEl = getBadgeEl();
  if (payloadEl) payloadEl.hidden = !on;
  if (legacyEl) legacyEl.hidden = on;
  if (badgeEl) {
    badgeEl.hidden = !on;
    badgeEl.textContent = on ? 'payload' : 'pty-scrape';
  }
}

export function setActiveSessionForPayload(sessionId: number | null) {
  mountedSessionId = sessionId;
  renderAll();
}

function renderAll() {
  const el = getTimelineEl();
  if (!el) return;
  el.innerHTML = '';
  if (mountedSessionId == null) return;
  const turns = turnsBySession.get(mountedSessionId) || [];
  el.dataset.sid = String(mountedSessionId);
  for (const t of turns) {
    el.appendChild(renderTurn(t));
  }
  if (payloadMode) {
    const emptyEl = getEmptyEl();
    if (emptyEl) emptyEl.hidden = turns.length > 0;
  }
}

function appendTurn(sessionId: number, turn: ChatTurn) {
  let arr = turnsBySession.get(sessionId);
  if (!arr) {
    arr = [];
    turnsBySession.set(sessionId, arr);
  }
  // dedupe by id
  if (arr.some(t => t.id === turn.id)) return;
  arr.push(turn);
  arr.sort((a, b) => a.id - b.id);
  // 上限カット（Hub 側 ring と同じ 50）
  if (arr.length > 50) arr.splice(0, arr.length - 50);
  if (sessionId === mountedSessionId) {
    const el = getTimelineEl();
    if (el) {
      el.appendChild(renderTurn(turn));
      if (payloadMode) {
        const emptyEl = getEmptyEl();
        if (emptyEl) emptyEl.hidden = true;
      }
    }
  }
}

function setSnapshot(sessionId: number, turns: ChatTurn[]) {
  turnsBySession.set(sessionId, [...turns].sort((a, b) => a.id - b.id));
  if (sessionId === mountedSessionId) renderAll();
}

export function handleChatTurnMessage(m: any) {
  if (m && m.type === 'chat_turn' && m.session_id && m.chat_turn) {
    appendTurn(m.session_id, m.chat_turn as ChatTurn);
  } else if (m && m.type === 'chat_turns_snapshot' && m.session_id && Array.isArray(m.chat_turns)) {
    setSnapshot(m.session_id, m.chat_turns as ChatTurn[]);
  }
}

export function clearChatPayloadForSession(sessionId: number) {
  turnsBySession.delete(sessionId);
  if (sessionId === mountedSessionId) renderAll();
}

export function purgeAllChatPayload() {
  turnsBySession.clear();
  renderAll();
}

function renderTurn(t: ChatTurn): HTMLElement {
  const card = document.createElement('div');
  card.className = 'chat-payload-turn';
  card.dataset.turnId = String(t.id);

  // ヘッダー
  const header = document.createElement('div');
  header.className = 'chat-payload-turn-header';
  const time = new Date(t.received_at).toLocaleTimeString();
  const status = t.status_code || 0;
  const statusClass = status >= 400 || status === 0 ? 'err' : 'ok';
  header.innerHTML = `
    <span class="cp-provider cp-prov-${escapeHtml(t.provider)}">${escapeHtml(t.provider)}</span>
    <span class="cp-endpoint">${escapeHtml(t.endpoint)}</span>
    <span class="cp-model">${escapeHtml(t.model || '')}</span>
    <span class="cp-time">${escapeHtml(time)}</span>
    <span class="cp-status cp-status-${statusClass}">${status || '—'}</span>
    ${t.is_stream ? '<span class="cp-stream">stream</span>' : ''}
    ${t.truncated ? '<span class="cp-truncated">truncated</span>' : ''}
  `;
  card.appendChild(header);

  // ボディ: messages / assistant content を payload からパースして表示
  const body = document.createElement('div');
  body.className = 'chat-payload-turn-body';
  body.appendChild(renderTurnBody(t));
  card.appendChild(body);

  // usage + Raw トグル
  const footer = document.createElement('div');
  footer.className = 'chat-payload-turn-footer';
  const usage = (t.tokens_in || t.tokens_out)
    ? `in ${t.tokens_in || 0} / out ${t.tokens_out || 0} tok`
    : '';
  footer.innerHTML = `<span class="cp-usage">${escapeHtml(usage)}</span>`;
  const rawBtn = document.createElement('button');
  rawBtn.type = 'button';
  rawBtn.className = 'cp-raw-toggle';
  rawBtn.textContent = 'Raw';
  const rawBox = document.createElement('div');
  rawBox.className = 'cp-raw-box';
  rawBox.hidden = true;
  rawBtn.addEventListener('click', () => {
    if (rawBox.hidden) {
      rawBox.innerHTML = '';
      const reqPre = document.createElement('pre');
      reqPre.className = 'cp-raw-req';
      reqPre.textContent = '# request\n' + prettyJSON(t.request_json || '');
      const respPre = document.createElement('pre');
      respPre.className = 'cp-raw-resp';
      respPre.textContent = '# response\n' + prettyJSON(t.response_json || '');
      rawBox.appendChild(reqPre);
      rawBox.appendChild(respPre);
      rawBox.hidden = false;
      rawBtn.textContent = 'Raw ▾';
    } else {
      rawBox.hidden = true;
      rawBtn.textContent = 'Raw';
    }
  });
  footer.appendChild(rawBtn);
  card.appendChild(footer);
  card.appendChild(rawBox);

  if (t.error_text) {
    const err = document.createElement('div');
    err.className = 'cp-error';
    err.textContent = t.error_text;
    card.appendChild(err);
  }

  return card;
}

function renderTurnBody(t: ChatTurn): HTMLElement {
  const box = document.createElement('div');
  box.className = 'cp-body';
  try {
    if (t.provider === 'anthropic') {
      box.appendChild(renderAnthropic(t));
    } else if (t.provider === 'openai') {
      box.appendChild(renderOpenAI(t));
    } else {
      box.appendChild(textBlock('(unknown provider)', 'cp-meta-line'));
    }
  } catch (e) {
    box.appendChild(textBlock('(render error: ' + (e as Error).message + ')', 'cp-error'));
  }
  return box;
}

function renderAnthropic(t: ChatTurn): HTMLElement {
  const wrap = document.createElement('div');
  let req: any = null;
  try { req = JSON.parse(t.request_json || 'null'); } catch {}
  if (req && Array.isArray(req.messages)) {
    if (req.system) {
      wrap.appendChild(collapsible('system', stringifyContent(req.system)));
    }
    if (req.tools && Array.isArray(req.tools) && req.tools.length) {
      wrap.appendChild(collapsible(`tools (${req.tools.length})`, JSON.stringify(req.tools, null, 2)));
    }
    for (const msg of req.messages) {
      wrap.appendChild(renderAnthropicMessage(msg));
    }
  }
  // assistant 応答（SSE 結合 or 単一 JSON）
  const respText = extractAnthropicAssistantText(t.response_json || '');
  if (respText) {
    const block = roleBlock('assistant', respText);
    wrap.appendChild(block);
  }
  return wrap;
}

function renderAnthropicMessage(msg: any): HTMLElement {
  const role = String(msg.role || 'unknown');
  if (typeof msg.content === 'string') {
    return roleBlock(role, msg.content);
  }
  if (Array.isArray(msg.content)) {
    const wrap = document.createElement('div');
    for (const part of msg.content) {
      if (part && part.type === 'text') {
        wrap.appendChild(roleBlock(role, String(part.text || '')));
      } else if (part && part.type === 'tool_use') {
        wrap.appendChild(toolBlock('tool_use', part.name, part.input));
      } else if (part && part.type === 'tool_result') {
        wrap.appendChild(toolBlock('tool_result', part.tool_use_id || '', part.content));
      } else if (part && part.type === 'image') {
        wrap.appendChild(roleBlock(role, '[image]'));
      } else {
        wrap.appendChild(roleBlock(role, JSON.stringify(part)));
      }
    }
    return wrap;
  }
  return roleBlock(role, JSON.stringify(msg));
}

function extractAnthropicAssistantText(respJSON: string): string {
  if (!respJSON) return '';
  // SSE 結合: 1 JSON / 行 → content_block_delta の text を順に連結
  let parts: string[] = [];
  let singleObj: any = null;
  try { singleObj = JSON.parse(respJSON); } catch {}
  if (singleObj && singleObj.content && Array.isArray(singleObj.content)) {
    return singleObj.content
      .filter((c: any) => c && c.type === 'text')
      .map((c: any) => c.text || '')
      .join('');
  }
  for (const line of respJSON.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    try {
      const ev = JSON.parse(trimmed);
      if (ev && ev.type === 'content_block_delta' && ev.delta && ev.delta.type === 'text_delta') {
        parts.push(String(ev.delta.text || ''));
      } else if (ev && ev.type === 'message_start' && ev.message && Array.isArray(ev.message.content)) {
        for (const c of ev.message.content) {
          if (c && c.type === 'text') parts.push(c.text || '');
        }
      }
    } catch { /* skip non-JSON */ }
  }
  return parts.join('');
}

function renderOpenAI(t: ChatTurn): HTMLElement {
  const wrap = document.createElement('div');
  let req: any = null;
  try { req = JSON.parse(t.request_json || 'null'); } catch {}
  if (req && Array.isArray(req.messages)) {
    if (req.tools && Array.isArray(req.tools) && req.tools.length) {
      wrap.appendChild(collapsible(`tools (${req.tools.length})`, JSON.stringify(req.tools, null, 2)));
    }
    for (const msg of req.messages) {
      wrap.appendChild(renderOpenAIMessage(msg));
    }
  }
  const respText = extractOpenAIAssistantText(t.response_json || '');
  if (respText) {
    wrap.appendChild(roleBlock('assistant', respText));
  }
  return wrap;
}

function renderOpenAIMessage(msg: any): HTMLElement {
  const role = String(msg.role || 'unknown');
  if (typeof msg.content === 'string') {
    return roleBlock(role, msg.content);
  }
  if (Array.isArray(msg.content)) {
    const wrap = document.createElement('div');
    for (const part of msg.content) {
      if (part && part.type === 'text') {
        wrap.appendChild(roleBlock(role, String(part.text || '')));
      } else if (part && part.type === 'image_url') {
        wrap.appendChild(roleBlock(role, '[image]'));
      } else {
        wrap.appendChild(roleBlock(role, JSON.stringify(part)));
      }
    }
    return wrap;
  }
  if (msg.tool_calls) {
    return toolBlock('tool_use', '(tool_calls)', msg.tool_calls);
  }
  return roleBlock(role, JSON.stringify(msg));
}

function extractOpenAIAssistantText(respJSON: string): string {
  if (!respJSON) return '';
  let single: any = null;
  try { single = JSON.parse(respJSON); } catch {}
  if (single && Array.isArray(single.choices)) {
    return single.choices.map((c: any) => c?.message?.content || '').join('');
  }
  let parts: string[] = [];
  for (const line of respJSON.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    try {
      const ev = JSON.parse(trimmed);
      if (ev && Array.isArray(ev.choices)) {
        for (const c of ev.choices) {
          if (c.delta && typeof c.delta.content === 'string') parts.push(c.delta.content);
        }
      }
    } catch { /* skip */ }
  }
  return parts.join('');
}

function roleBlock(role: string, text: string): HTMLElement {
  const div = document.createElement('div');
  div.className = `cp-msg cp-role-${escapeHtml(role)}`;
  const label = document.createElement('div');
  label.className = 'cp-msg-role';
  label.textContent = role;
  const body = document.createElement('div');
  body.className = 'cp-msg-body';
  body.textContent = text;
  div.appendChild(label);
  div.appendChild(body);
  return div;
}

function toolBlock(kind: string, name: string, payload: any): HTMLElement {
  const div = document.createElement('div');
  div.className = `cp-tool cp-tool-${escapeHtml(kind)}`;
  const summary = document.createElement('div');
  summary.className = 'cp-tool-summary';
  summary.textContent = `${kind}: ${name}`;
  const pre = document.createElement('pre');
  pre.className = 'cp-tool-payload';
  pre.textContent = typeof payload === 'string' ? payload : JSON.stringify(payload, null, 2);
  pre.hidden = true;
  summary.addEventListener('click', () => { pre.hidden = !pre.hidden; });
  div.appendChild(summary);
  div.appendChild(pre);
  return div;
}

function collapsible(label: string, content: string): HTMLElement {
  const wrap = document.createElement('details');
  wrap.className = 'cp-collapsible';
  const sum = document.createElement('summary');
  sum.textContent = label;
  wrap.appendChild(sum);
  const pre = document.createElement('pre');
  pre.textContent = content;
  wrap.appendChild(pre);
  return wrap;
}

function textBlock(text: string, cls: string): HTMLElement {
  const div = document.createElement('div');
  div.className = cls;
  div.textContent = text;
  return div;
}

function stringifyContent(v: any): string {
  if (typeof v === 'string') return v;
  return JSON.stringify(v, null, 2);
}

function prettyJSON(s: string): string {
  if (!s) return '';
  // SSE 結合は 1 JSON / 行なので、行ごとに parse して整形する。失敗したら原文。
  const lines = s.split('\n');
  if (lines.length === 1) {
    try { return JSON.stringify(JSON.parse(s), null, 2); } catch { return s; }
  }
  return lines.map(line => {
    const t = line.trim();
    if (!t) return '';
    try { return JSON.stringify(JSON.parse(t), null, 2); } catch { return line; }
  }).filter(Boolean).join('\n\n');
}

function escapeHtml(s: string): string {
  return String(s).replace(/[&<>"']/g, ch => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  }[ch] as string));
}

// トグル UI 初期化
export function initChatPayloadUI() {
  const toggle = document.getElementById('chat-source-payload-toggle') as HTMLInputElement | null;
  if (toggle) {
    toggle.addEventListener('change', () => {
      payloadMode = toggle.checked;
      setPayloadModeUI(payloadMode);
      if (payloadMode) renderAll();
    });
  }
}

// 自動初期化（DOM ready 時）
if (typeof window !== 'undefined') {
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initChatPayloadUI);
  } else {
    initChatPayloadUI();
  }
}
