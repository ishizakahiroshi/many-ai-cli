// 「エージェントセッション」画面（PTY ではない、人が直接操作する会話）の、画面に依存しない部分。
//
// Hub の /api/acp/sessions/{id}/events が返す「連番つきの出来事」を、会話の並び（タイムライン）に畳む。
// ここは DOM も fetch も持たない。
//
// 守ること（plan_provider-declarative-capabilities.md C12）:
//   - 許可のボタンは Hub が返す pending_permissions だけから作る。出来事の並び（permission_request）からは作らない
//   - 自動で答える・再試行する・続行する処理を持たない。答えを作るのは画面の人の操作だけ
//   - 利用量は、エージェントが自分で送ってきた最後の値を持つだけ。画面は人が求めたときだけ見せる（こちらからは問い合わせない）

export type AgentEvent = { seq: number; at: string; kind: string; data: Record<string, any> };

export type PermissionOption = { option_id: string; name: string; kind: string };

// Hub の「保留中の記録」。許可のボタンはこれだけから出す。
export type PendingPermission = {
  request_key: number;
  tool_call: Record<string, any>;
  options: PermissionOption[];
  at: string;
};

export type DiffBlock = { path: string; oldText: string; newText: string };

export type ToolStatus = 'pending' | 'in_progress' | 'completed' | 'failed' | 'unknown';

export type TimelineItem =
  | { type: 'user'; seq: number; text: string; replay: boolean }
  | { type: 'agent'; seq: number; text: string; replay: boolean }
  | { type: 'thought'; seq: number; text: string; replay: boolean }
  | {
      type: 'tool'; seq: number; id: string; title: string; kind: string; status: ToolStatus;
      diffs: DiffBlock[]; texts: string[]; locations: string[]; replay: boolean;
    }
  | { type: 'plan'; seq: number; entries: { content: string; status: string }[]; replay: boolean }
  | { type: 'permission'; seq: number; requestKey: number; title: string; state: 'open' | 'answered' | 'closed'; chosen?: string; reason?: string }
  | { type: 'turn_end'; seq: number; stopReason: string }
  | { type: 'error'; seq: number; message: string }
  | { type: 'notice'; seq: number; text: string }
  | { type: 'ended'; seq: number; reason: string };

export type SessionState = {
  items: TimelineItem[];
  lastSeq: number;
  // メモリから古い出来事が落ちていて、欠けがあるか（first_seq が after の先にある）。
  gap: boolean;
  phase: 'idle' | 'running' | 'ended';
  // Hub の応答の pending_permissions そのもの（許可ボタンの唯一の材料）。
  pending: PendingPermission[];
  // エージェントが報告した現在のモデル（表示だけ。画面から変える口は Hub に無い）。
  model: string;
  // エージェントが自分で送ってきた最後の利用量。画面は人が「利用量」を押したときだけ見せる。
  usage: { used?: number; size?: number; cost?: string } | null;
};

export function emptyState(): SessionState {
  return { items: [], lastSeq: 0, gap: false, phase: 'idle', pending: [], model: '', usage: null };
}

const str = (value: unknown): string => (typeof value === 'string' ? value : '');
const obj = (value: unknown): Record<string, any> => (value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, any>) : {});

function chunkText(update: Record<string, any>): string {
  const content = obj(update.content);
  if (content.type === 'text') return str(content.text);
  if (content.type === 'resource_link') return str(content.name) || str(content.uri);
  if (content.type === 'image') return '[image]';
  return '';
}

function toolStatus(value: unknown): ToolStatus {
  return value === 'pending' || value === 'in_progress' || value === 'completed' || value === 'failed' ? value : 'unknown';
}

function toolContent(content: unknown): { diffs: DiffBlock[]; texts: string[] } {
  const diffs: DiffBlock[] = [];
  const texts: string[] = [];
  if (Array.isArray(content)) {
    for (const entry of content) {
      const item = obj(entry);
      if (item.type === 'diff') {
        diffs.push({ path: str(item.path), oldText: str(item.oldText), newText: str(item.newText) });
      } else if (item.type === 'content') {
        const inner = obj(item.content);
        if (inner.type === 'text' && str(inner.text)) texts.push(str(inner.text));
      }
    }
  }
  return { diffs, texts };
}

function toolLocations(locations: unknown): string[] {
  if (!Array.isArray(locations)) return [];
  const paths: string[] = [];
  for (const entry of locations) {
    const path = str(obj(entry).path);
    if (path) paths.push(path);
  }
  return paths;
}

function pushText(items: TimelineItem[], type: 'agent' | 'user' | 'thought', seq: number, text: string, replay: boolean): void {
  if (!text) return;
  const last = items[items.length - 1];
  // 続けて届く同じ種類の断片は 1 つの吹き出しにつなげる（ストリーミング）。
  if (last && last.type === type && last.replay === replay) {
    last.text += text;
    return;
  }
  items.push({ type, seq, text, replay });
}

function findTool(items: TimelineItem[], id: string): Extract<TimelineItem, { type: 'tool' }> | undefined {
  for (let index = items.length - 1; index >= 0; index--) {
    const item = items[index];
    if (item.type === 'tool' && item.id === id) return item;
  }
  return undefined;
}

function applyUpdate(state: SessionState, seq: number, data: Record<string, any>): void {
  const update = obj(data.update);
  const replay = data.replay === true;
  switch (update.sessionUpdate) {
    case 'agent_message_chunk':
      pushText(state.items, 'agent', seq, chunkText(update), replay);
      break;
    case 'user_message_chunk':
      pushText(state.items, 'user', seq, chunkText(update), replay);
      break;
    case 'agent_thought_chunk':
      pushText(state.items, 'thought', seq, chunkText(update), replay);
      break;
    case 'tool_call': {
      const id = str(update.toolCallId) || `seq-${seq}`;
      const { diffs, texts } = toolContent(update.content);
      const existing = findTool(state.items, id);
      if (existing) {
        existing.title = str(update.title) || existing.title;
        existing.status = toolStatus(update.status);
        existing.diffs = diffs;
        existing.texts = texts;
        existing.locations = toolLocations(update.locations);
        break;
      }
      state.items.push({
        type: 'tool', seq, id, title: str(update.title) || id, kind: str(update.kind) || 'other',
        status: toolStatus(update.status), diffs, texts, locations: toolLocations(update.locations), replay,
      });
      break;
    }
    case 'tool_call_update': {
      const id = str(update.toolCallId);
      const tool = id ? findTool(state.items, id) : undefined;
      if (!tool) break;
      if (str(update.title)) tool.title = str(update.title);
      if (str(update.kind)) tool.kind = str(update.kind);
      if (update.status !== undefined) tool.status = toolStatus(update.status);
      if (update.content !== undefined && update.content !== null) {
        const { diffs, texts } = toolContent(update.content);
        tool.diffs = diffs;
        tool.texts = texts;
      }
      if (update.locations !== undefined && update.locations !== null) tool.locations = toolLocations(update.locations);
      break;
    }
    case 'plan': {
      const entries = Array.isArray(update.entries)
        ? update.entries.map((entry: unknown) => ({ content: str(obj(entry).content), status: str(obj(entry).status) }))
        : [];
      const last = state.items[state.items.length - 1];
      if (last && last.type === 'plan') last.entries = entries;
      else state.items.push({ type: 'plan', seq, entries, replay });
      break;
    }
    case 'config_option_update': {
      const options = Array.isArray(update.configOptions) ? update.configOptions : [];
      for (const raw of options) {
        const option = obj(raw);
        if (option.category === 'model' || option.id === 'model') {
          const value = str(option.currentValue);
          if (value) state.model = value;
        }
      }
      break;
    }
    case 'usage_update': {
      const used = typeof update.used === 'number' ? update.used : undefined;
      const size = typeof update.size === 'number' ? update.size : undefined;
      const cost = obj(update.cost);
      state.usage = { used, size, cost: typeof cost.amount === 'number' ? `${cost.amount} ${str(cost.currency)}`.trim() : undefined };
      break;
    }
    default:
      // available_commands_update・current_mode_update・session_info_update などは出さない。
      break;
  }
}

function permissionTitle(toolCall: Record<string, any>): string {
  return str(toolCall.title) || str(toolCall.toolCallId) || '';
}

function applyOne(state: SessionState, event: AgentEvent): void {
  const data = obj(event.data);
  switch (event.kind) {
    case 'user_prompt':
      pushText(state.items, 'user', event.seq, str(data.text), false);
      // 続く応答は別の吹き出しにする。
      break;
    case 'update':
      applyUpdate(state, event.seq, data);
      break;
    case 'permission_request':
      state.items.push({
        type: 'permission', seq: event.seq, requestKey: Number(data.request_key),
        title: permissionTitle(obj(data.tool_call)), state: 'open',
      });
      break;
    case 'permission_answered':
    case 'permission_closed': {
      const key = Number(data.request_key);
      for (const item of state.items) {
        if (item.type === 'permission' && item.requestKey === key) {
          item.state = event.kind === 'permission_answered' ? 'answered' : 'closed';
          const answer = obj(data.answer);
          item.chosen = str(answer.option_id) || (answer.cancelled === true ? 'cancelled' : undefined);
          item.reason = str(data.reason) || undefined;
        }
      }
      break;
    }
    case 'turn_end':
      state.items.push({ type: 'turn_end', seq: event.seq, stopReason: str(data.stop_reason) });
      break;
    case 'turn_error':
      state.items.push({ type: 'error', seq: event.seq, message: str(data.message) });
      break;
    case 'notice':
      state.items.push({ type: 'notice', seq: event.seq, text: str(data.message) });
      break;
    case 'ended':
      state.items.push({ type: 'ended', seq: event.seq, reason: str(data.reason) });
      break;
    default:
      // meta・notification など。会話には出さない。
      break;
  }
}

// events の応答（session・first_seq・last_seq・events・pending_permissions）を取り込む。
// すでに取り込んだ連番（seq <= lastSeq）は読み飛ばす（同じ応答を 2 回受けても増えない）。
export function applyEventsResponse(
  state: SessionState,
  response: { session?: { phase?: string }; first_seq?: number; events?: AgentEvent[]; pending_permissions?: PendingPermission[] },
  requestedAfter: number,
): SessionState {
  const next: SessionState = { ...state, items: state.items.slice() };
  // 追加で書き換えるタイムラインの要素は複製しておく（前の状態を壊さない）。
  next.items = next.items.map((item) => ({ ...item }) as TimelineItem);
  if (typeof response.first_seq === 'number' && response.first_seq > requestedAfter + 1 && response.first_seq > 1) {
    next.gap = true;
  }
  for (const event of Array.isArray(response.events) ? response.events : []) {
    if (!event || typeof event.seq !== 'number' || event.seq <= next.lastSeq) continue;
    applyOne(next, event);
    next.lastSeq = event.seq;
  }
  const phase = response.session?.phase;
  if (phase === 'idle' || phase === 'running' || phase === 'ended') next.phase = phase;
  // 許可のボタンの材料は Hub の保留中の記録そのもの。出来事の並びからは作らない。
  next.pending = Array.isArray(response.pending_permissions) ? response.pending_permissions.map(clonePending) : [];
  return next;
}

function clonePending(entry: PendingPermission): PendingPermission {
  return {
    request_key: Number(entry.request_key),
    tool_call: obj(entry.tool_call),
    options: Array.isArray(entry.options)
      ? entry.options.map((option) => ({ option_id: str(option.option_id), name: str(option.name), kind: str(option.kind) }))
      : [],
    at: str(entry.at),
  };
}

// 許可の返答の本文。選んだ選択肢か、取りやめだけ。ほかの形は作らない。
export function permissionAnswer(requestKey: number, choice: { optionId: string } | { cancelled: true }): Record<string, unknown> {
  if ('cancelled' in choice) return { request_key: requestKey, cancelled: true };
  return { request_key: requestKey, option_id: choice.optionId };
}

// 差分の見せ方: 先頭と末尾の同じ行を畳み、変わった部分の前後に少しだけ文脈を付ける。
export type DiffLine = { op: ' ' | '-' | '+' | '…'; text: string };

export function diffLines(oldText: string, newText: string, context = 3): DiffLine[] {
  const before = oldText === '' ? [] : oldText.split('\n');
  const after = newText === '' ? [] : newText.split('\n');
  let head = 0;
  while (head < before.length && head < after.length && before[head] === after[head]) head++;
  let tail = 0;
  while (
    tail < before.length - head && tail < after.length - head
    && before[before.length - 1 - tail] === after[after.length - 1 - tail]
  ) tail++;
  const lines: DiffLine[] = [];
  const headStart = Math.max(0, head - context);
  if (headStart > 0) lines.push({ op: '…', text: '' });
  for (let index = headStart; index < head; index++) lines.push({ op: ' ', text: before[index] });
  for (let index = head; index < before.length - tail; index++) lines.push({ op: '-', text: before[index] });
  for (let index = head; index < after.length - tail; index++) lines.push({ op: '+', text: after[index] });
  const tailShown = Math.min(tail, context);
  for (let index = 0; index < tailShown; index++) lines.push({ op: ' ', text: after[after.length - tail + index] });
  if (tail > tailShown) lines.push({ op: '…', text: '' });
  return lines;
}
