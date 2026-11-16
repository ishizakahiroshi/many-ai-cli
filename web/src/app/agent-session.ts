// 「エージェントセッション」画面。PTY（端末）ではなく、人が直接操作する会話（Hub の /api/acp/*）。
//
// 守ること（plan_provider-declarative-capabilities.md C12）:
//   - 境界の保証は Hub の受け口（C10・C11。画面の証明が無い呼び出しは 403）。この画面のボタンの有無は保証ではない
//   - 自動で答える・自動で再試行する・自動で続ける処理を持たない。許可のボタンは Hub が返した
//     pending_permissions からだけ作り、押した人の操作だけを送る
//   - 出来事の取得は長いポーリング（events?after=&wait_ms=）。失敗したら止まり、人が「再取得」を押す
//   - 利用量は、人が「利用量」を押したときだけ見せる。こちらから問い合わせない・定期取得しない
//   - 認証が要るときは案内を出すだけ。独自の認証の中継はしない
// 文字は textContent だけで入れる（innerHTML を使わない）。

import { t } from '../i18n.js';
import { uiButtonClass } from './ui-components.js';
import { apiFetch } from './util.js';
import { activeSessionId, sessions } from './state.js';
import {
  declaredProviderIds, onProviderDeclarationsChanged, providerIsAcp, providerLongLabel,
} from './provider-declarations.js';
import {
  applyEventsResponse, diffLines, emptyState, permissionAnswer,
} from './agent-session-model.js';
import type { AgentEvent, PendingPermission, SessionState, TimelineItem } from './agent-session-model.js';

type SessionView = {
  id: number; provider: string; cwd: string; agent_session_id: string; started_at: string;
  phase: 'idle' | 'running' | 'ended'; ended_reason?: string | null; log_file?: string | null;
  can_list_agent_sessions: boolean; can_resume: boolean; can_close: boolean; accepts_images: boolean;
};
type SavedSession = { file: string; provider: string; cwd: string; agent_session_id: string; started_at: string };
type AgentConversation = { agent_session_id: string; cwd?: string; title?: string; updated_at?: string };

class ApiError extends Error {
  constructor(message: string, readonly status: number, readonly code: string, readonly authMethods: string[]) { super(message); }
}

const WAIT_MS = 20000;
const tr = (key: string) => t('agent_session_' + key);

let dialog: HTMLDialogElement | null = null;
let content: HTMLElement;
let feedback: HTMLElement;
let opener: HTMLElement | null = null;
// 画面を切り替えるたびに進める。古い非同期の結果が新しい画面に混ざらないようにする。
let generation = 0;

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = '', className = ''): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  el.textContent = text;
  if (tag === 'button') el.className = `${uiButtonClass('secondary', 'compact')} ${className}`.trim();
  else if (tag === 'select' || tag === 'textarea' || tag === 'input') el.className = `ui-input ${className}`.trim();
  else el.className = className;
  return el;
}
function button(label: string, action: () => void, variant: 'primary' | 'secondary' | 'danger' | 'quiet' | 'warn' = 'secondary'): HTMLButtonElement {
  const el = document.createElement('button');
  el.type = 'button';
  el.textContent = label;
  el.className = uiButtonClass(variant, 'compact');
  el.addEventListener('click', action);
  return el;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await apiFetch(path, init);
  if (!response.ok) {
    let detail = '';
    let code = '';
    let methods: string[] = [];
    try {
      const body = await response.json();
      if (typeof body.detail === 'string') detail = body.detail.slice(0, 350);
      if (typeof body.error === 'string') code = body.error;
      if (Array.isArray(body.auth_methods)) methods = body.auth_methods.map((m: { name?: string; id?: string }) => String(m.name || m.id || ''));
    } catch (_) { /* 本文が JSON でなければ状態コードだけ出す */ }
    throw new ApiError(`${tr('request_failed')} (${response.status})${detail ? ' ' + detail : ''}`, response.status, code, methods);
  }
  return await response.json() as T;
}
const post = (data?: unknown): RequestInit => ({ method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(data ?? {}) });

function showError(error: unknown): void {
  if (error instanceof ApiError && error.code === 'auth_required') {
    // 案内だけ。認証は公式ランタイムの流れで本人が行う。
    const methods = error.authMethods.filter(Boolean).join(', ');
    feedback.textContent = `${tr('auth_required')}${methods ? ' (' + methods + ')' : ''}\n${tr('auth_note')}`;
    return;
  }
  feedback.textContent = error instanceof Error ? error.message : tr('request_failed');
}

function page(): number {
  generation++;
  content.replaceChildren();
  feedback.textContent = '';
  return generation;
}
function current(epoch: number): boolean { return !!dialog?.open && epoch === generation; }
function close(): void { dialog?.close(); }

function ensureDialog(): HTMLDialogElement {
  if (dialog) return dialog;
  dialog = node('dialog', '', 'ui-modal ui-modal--lg agent-session-dialog aac-wheel-overlay');
  dialog.id = 'agent-session-dialog';
  dialog.setAttribute('aria-labelledby', 'agent-session-title');
  const header = node('div', '', 'agent-session-header');
  const heading = node('h2', tr('title'));
  heading.id = 'agent-session-title';
  const dismiss = button('✕', close, 'quiet');
  dismiss.setAttribute('aria-label', t('settings_close'));
  header.append(heading, dismiss);
  feedback = node('p', '', 'agent-session-feedback');
  feedback.setAttribute('role', 'status');
  content = node('div', '', 'agent-session-content');
  dialog.append(header, feedback, content);
  document.body.append(dialog);
  // 閉じると取得の流れが止まる（Hub 側のセッションはそのまま。再び開けば続きから見られる）。
  dialog.addEventListener('close', () => { generation++; opener?.focus({ preventScroll: true }); });
  return dialog;
}

export function openAgentSessions(): void {
  const modal = ensureDialog();
  if (!modal.open) {
    opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    modal.showModal();
  }
  void showList();
}

function acpProviderIds(): string[] {
  return declaredProviderIds({ includeAcp: true }).filter((id) => providerIsAcp(id));
}

function when(value?: string): string {
  if (!value) return '';
  const parsed = new Date(value);
  if (!Number.isFinite(parsed.valueOf())) return value;
  return new Intl.DateTimeFormat(document.documentElement.lang || 'en', { dateStyle: 'short', timeStyle: 'short' }).format(parsed);
}

// ---- 一覧（開始・再開・開いている会話） ----

async function showList(): Promise<void> {
  const epoch = page();
  content.append(node('p', tr('human_only_note'), 'agent-session-note'));

  const form = node('form', '', 'agent-session-start');
  const providerLabel = node('label', tr('provider'));
  const provider = node('select');
  provider.name = 'provider';
  const fillProviders = () => {
    const keep = provider.value;
    provider.replaceChildren();
    for (const id of acpProviderIds()) {
      const option = node('option', providerLongLabel(id));
      option.value = id;
      provider.append(option);
    }
    if (keep) provider.value = keep;
    start.disabled = provider.options.length === 0;
    none.hidden = provider.options.length > 0;
  };
  providerLabel.append(provider);
  const cwdLabel = node('label', tr('cwd'));
  const cwd = node('input');
  cwd.name = 'cwd';
  cwd.required = true;
  cwd.value = (activeSessionId === null ? undefined : sessions.get(activeSessionId)?.cwd) || '';
  cwdLabel.append(cwd);
  const start = node('button', tr('start'));
  start.type = 'submit';
  start.className = uiButtonClass('primary', 'compact');
  const none = node('p', tr('no_provider'), 'agent-session-note');
  form.append(providerLabel, cwdLabel, start, none);
  content.append(form);
  fillProviders();
  const unsubscribe = onProviderDeclarationsChanged(() => { if (current(epoch)) fillProviders(); else unsubscribe(); });
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    if (start.disabled) return;
    start.disabled = true;
    feedback.textContent = tr('starting');
    try {
      const view = await request<SessionView>('/api/acp/sessions', post({ provider: provider.value, cwd: cwd.value.trim() }));
      if (current(epoch)) void showSession(view);
    } catch (error) {
      if (current(epoch)) { showError(error); start.disabled = false; }
    }
  });

  const live = node('section', '', 'agent-session-group');
  live.append(node('h3', tr('live')), node('p', tr('loading'), 'agent-session-note'));
  const saved = node('section', '', 'agent-session-group');
  saved.append(node('h3', tr('saved')));
  content.append(live, saved);
  try {
    const list = await request<{ sessions: SessionView[]; saved: SavedSession[] }>('/api/acp/sessions');
    if (!current(epoch)) return;
    live.lastElementChild?.remove();
    if (!list.sessions.length) live.append(node('p', tr('empty_live'), 'agent-session-note'));
    for (const view of list.sessions) {
      const row = node('div', '', 'agent-session-row');
      row.append(
        node('span', `${providerLongLabel(view.provider)} · ${view.cwd}`, 'agent-session-row-main'),
        node('span', tr('phase_' + view.phase), 'agent-session-badge'),
        button(tr('open'), () => void showSession(view), 'primary'),
      );
      live.append(row);
    }
    if (!list.saved.length) saved.append(node('p', tr('empty_saved'), 'agent-session-note'));
    for (const entry of list.saved) {
      const row = node('div', '', 'agent-session-row');
      const resume = button(tr('resume'), async () => {
        if (resume.disabled) return;
        resume.disabled = true;
        feedback.textContent = tr('starting');
        try {
          const view = await request<SessionView>('/api/acp/sessions', post({
            provider: entry.provider, cwd: entry.cwd, resume_agent_session_id: entry.agent_session_id,
          }));
          if (current(epoch)) void showSession(view);
        } catch (error) {
          if (current(epoch)) { showError(error); resume.disabled = false; }
        }
      }, 'secondary');
      row.append(node('span', `${providerLongLabel(entry.provider)} · ${entry.cwd} · ${when(entry.started_at)}`, 'agent-session-row-main'), resume);
      saved.append(row);
    }
  } catch (error) {
    if (!current(epoch)) return;
    live.lastElementChild?.remove();
    showError(error);
    live.append(button(tr('retry'), () => void showList()));
  }
}

// ---- 会話 ----

function toolStatusLabel(status: string): string { return tr('tool_' + status); }

function renderDiff(path: string, oldText: string, newText: string): HTMLElement {
  const box = node('div', '', 'agent-session-diff');
  box.append(node('div', path, 'agent-session-diff-path'));
  const pre = node('pre', '', 'agent-session-diff-body');
  for (const line of diffLines(oldText, newText)) {
    const row = node('span', line.op === '…' ? '…' : `${line.op} ${line.text}`, `agent-session-diff-line agent-session-diff-${line.op === '+' ? 'add' : line.op === '-' ? 'del' : 'ctx'}`);
    pre.append(row, document.createTextNode('\n'));
  }
  box.append(pre);
  return box;
}

function renderItem(item: TimelineItem): HTMLElement {
  const replay = 'replay' in item && item.replay ? ' agent-session-replay' : '';
  switch (item.type) {
    case 'user': return node('div', item.text, 'agent-session-msg agent-session-user' + replay);
    case 'agent': return node('div', item.text, 'agent-session-msg agent-session-agent' + replay);
    case 'thought': {
      const details = node('details', '', 'agent-session-thought' + replay);
      details.append(node('summary', tr('thought')), node('div', item.text));
      return details;
    }
    case 'tool': {
      const box = node('div', '', `agent-session-tool agent-session-tool-${item.status}${replay}`);
      box.append(node('div', `${item.title} · ${toolStatusLabel(item.status)}`, 'agent-session-tool-head'));
      if (item.locations.length) box.append(node('div', item.locations.join(', '), 'agent-session-note'));
      for (const diff of item.diffs) box.append(renderDiff(diff.path, diff.oldText, diff.newText));
      for (const text of item.texts) box.append(node('pre', text, 'agent-session-tool-text'));
      return box;
    }
    case 'plan': {
      const box = node('div', '', 'agent-session-plan' + replay);
      box.append(node('div', tr('plan'), 'agent-session-tool-head'));
      const list = node('ul');
      for (const entry of item.entries) list.append(node('li', `${entry.content}${entry.status ? ' · ' + entry.status : ''}`));
      box.append(list);
      return box;
    }
    case 'permission': {
      const label = item.state === 'open' ? tr('permission_open')
        : item.state === 'answered' ? `${tr('permission_answered')}${item.chosen ? ': ' + item.chosen : ''}`
          : `${tr('permission_closed')}${item.reason ? ': ' + item.reason : ''}`;
      return node('div', `${tr('permission')}: ${item.title} · ${label}`, 'agent-session-notice');
    }
    case 'turn_end': return node('div', `${tr('turn_end')}: ${item.stopReason}`, 'agent-session-notice');
    case 'error': return node('div', item.message, 'agent-session-notice agent-session-error');
    case 'notice': return node('div', item.text, 'agent-session-notice');
    case 'ended': return node('div', `${tr('ended')}${item.reason ? ': ' + item.reason : ''}`, 'agent-session-notice');
  }
}

function showSession(initial: SessionView): void {
  const epoch = page();
  let view = initial;
  let state: SessionState = emptyState();
  let polling = false;
  let showUsage = false;

  const top = node('div', '', 'agent-session-top');
  const back = button('← ' + tr('title'), () => void showList(), 'quiet');
  const phase = node('span', '', 'agent-session-badge');
  const title = node('strong', `${providerLongLabel(view.provider)} · ${view.cwd}`, 'agent-session-row-main');
  top.append(back, title, phase);
  const meta = node('p', '', 'agent-session-note');
  const timeline = node('div', '', 'agent-session-timeline');
  timeline.setAttribute('aria-live', 'polite');
  const permissions = node('div', '', 'agent-session-permissions');
  const retry = button(tr('retry'), () => { retry.hidden = true; void poll(0); }, 'primary');
  retry.hidden = true;

  const composer = node('form', '', 'agent-session-composer');
  const prompt = node('textarea');
  prompt.rows = 3;
  prompt.placeholder = tr('prompt_placeholder');
  const send = node('button', tr('send'));
  send.type = 'submit';
  send.className = uiButtonClass('primary', 'compact');
  const stop = button(tr('cancel_turn'), async () => {
    stop.disabled = true;
    try { await request('/api/acp/sessions/' + view.id + '/cancel', post()); } catch (error) { if (current(epoch)) showError(error); }
    stop.disabled = false;
  }, 'warn');
  composer.append(prompt, send, stop);
  composer.addEventListener('submit', async (event) => {
    event.preventDefault();
    const text = prompt.value.trim();
    if (!text || send.disabled) return;
    send.disabled = true;
    feedback.textContent = '';
    try {
      await request('/api/acp/sessions/' + view.id + '/prompt', post({ text }));
      prompt.value = '';
    } catch (error) {
      if (current(epoch)) showError(error);
    } finally {
      if (current(epoch)) render();
    }
  });
  prompt.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !event.isComposing) { event.preventDefault(); composer.requestSubmit(); }
  });

  const tools = node('div', '', 'agent-session-tools');
  const usageBtn = button(tr('usage'), () => { showUsage = !showUsage; render(); }, 'quiet');
  const usageBox = node('p', '', 'agent-session-note');
  const conversationsBtn = button(tr('agent_conversations'), () => void listConversations(), 'quiet');
  const endBtn = button(tr('end'), () => confirmEnd(), 'danger');
  const endConfirm = node('div', '', 'agent-session-confirm');
  endConfirm.hidden = true;
  const conversations = node('div', '', 'agent-session-group');
  conversations.hidden = true;
  tools.append(usageBtn, conversationsBtn, endBtn);
  content.append(top, meta, timeline, permissions, retry, composer, tools, usageBox, endConfirm, conversations);

  function confirmEnd(): void {
    endConfirm.hidden = false;
    endConfirm.replaceChildren(
      node('p', tr('end_confirm'), 'agent-session-note'),
      button(tr('end_no'), () => { endConfirm.hidden = true; endConfirm.replaceChildren(); }, 'secondary'),
      button(tr('end_yes'), async () => {
        endConfirm.hidden = true;
        try { view = await request<SessionView>('/api/acp/sessions/' + view.id + '/end', post()); } catch (error) { if (current(epoch)) showError(error); }
        if (current(epoch)) render();
      }, 'danger'),
    );
  }

  async function listConversations(): Promise<void> {
    conversations.hidden = false;
    conversations.replaceChildren(node('p', tr('loading'), 'agent-session-note'));
    try {
      const list = await request<{ sessions: AgentConversation[] }>('/api/acp/sessions/' + view.id + '/agent-sessions');
      if (!current(epoch)) return;
      conversations.replaceChildren(node('h3', tr('agent_conversations')));
      if (!list.sessions.length) conversations.append(node('p', tr('no_agent_conversations'), 'agent-session-note'));
      for (const entry of list.sessions) {
        const row = node('div', '', 'agent-session-row');
        const resume = button(tr('resume'), async () => {
          if (resume.disabled) return;
          resume.disabled = true;
          try {
            const next = await request<SessionView>('/api/acp/sessions', post({
              provider: view.provider, cwd: entry.cwd || view.cwd, resume_agent_session_id: entry.agent_session_id,
            }));
            if (current(epoch)) showSession(next);
          } catch (error) {
            if (current(epoch)) { showError(error); resume.disabled = false; }
          }
        }, 'secondary');
        row.append(node('span', `${entry.title || entry.agent_session_id} · ${when(entry.updated_at)}`, 'agent-session-row-main'), resume);
        conversations.append(row);
      }
    } catch (error) {
      if (current(epoch)) { conversations.hidden = true; showError(error); }
    }
  }

  function permissionBlock(entry: PendingPermission): HTMLElement {
    const box = node('div', '', 'agent-session-permission');
    box.setAttribute('role', 'group');
    box.append(node('div', `${tr('permission')}: ${String(entry.tool_call.title || entry.tool_call.toolCallId || '')}`, 'agent-session-tool-head'));
    const row = node('div', '', 'agent-session-actions');
    const buttons: HTMLButtonElement[] = [];
    const answer = async (body: Record<string, unknown>) => {
      for (const b of buttons) b.disabled = true;
      try {
        await request('/api/acp/sessions/' + view.id + '/permission', post(body));
      } catch (error) {
        if (current(epoch)) { showError(error); for (const b of buttons) b.disabled = false; }
      }
    };
    // 選択肢の見た目は Hub が返した kind だけで決める。ボタンの文言は選択肢の名前そのまま。
    for (const option of entry.options) {
      const variant = option.kind.startsWith('reject') ? 'secondary' : 'primary';
      buttons.push(button(option.name || option.option_id, () => void answer(permissionAnswer(entry.request_key, { optionId: option.option_id })), variant));
    }
    buttons.push(button(tr('permission_cancel'), () => void answer(permissionAnswer(entry.request_key, { cancelled: true })), 'quiet'));
    row.append(...buttons);
    box.append(row);
    return box;
  }

  function render(): void {
    const nearBottom = timeline.scrollHeight - timeline.scrollTop - timeline.clientHeight < 40;
    const phaseNow = view.phase === 'ended' ? 'ended' : state.phase;
    phase.textContent = tr('phase_' + phaseNow);
    const parts: string[] = [];
    if (state.model) parts.push(`${tr('model')}: ${state.model}`);
    if (state.gap) parts.push(tr('gap'));
    meta.textContent = parts.join(' · ');
    meta.hidden = parts.length === 0;
    timeline.replaceChildren(...state.items.map(renderItem));
    if (nearBottom) timeline.scrollTop = timeline.scrollHeight;
    // 許可のボタンは Hub の保留中の記録（pending_permissions）だけから作る。
    permissions.replaceChildren(...state.pending.map(permissionBlock));
    const ended = phaseNow === 'ended';
    composer.hidden = ended;
    send.disabled = state.phase === 'running';
    stop.hidden = state.phase !== 'running';
    endBtn.hidden = ended;
    conversationsBtn.hidden = !view.can_list_agent_sessions || ended;
    usageBtn.textContent = showUsage ? tr('usage_hide') : tr('usage');
    usageBox.hidden = !showUsage;
    if (showUsage) {
      const usage = state.usage;
      usageBox.textContent = usage
        ? [usage.used !== undefined ? `${usage.used}${usage.size !== undefined ? ' / ' + usage.size : ''}` : '', usage.cost || ''].filter(Boolean).join(' · ')
        : tr('usage_none');
    }
  }

  // 長いポーリング。失敗したら止まり、人が「再取得」を押すまで再開しない（自動の再試行はしない）。
  async function poll(wait: number): Promise<void> {
    if (polling) return;
    polling = true;
    try {
      let first = true;
      while (current(epoch)) {
        const after = state.lastSeq;
        const started = Date.now();
        const response = await request<{ session: SessionView; first_seq: number; events: AgentEvent[]; pending_permissions: PendingPermission[] }>(
          `/api/acp/sessions/${view.id}/events?after=${after}&wait_ms=${first ? wait : WAIT_MS}`,
        );
        if (!current(epoch)) return;
        first = false;
        view = response.session;
        state = applyEventsResponse(state, response, after);
        render();
        if (state.phase === 'ended') return;
        // Hub が待たずに返した（出来事が無い）ときに、休まず回り続けない。
        if (!response.events.length && Date.now() - started < 500) await new Promise((resolve) => setTimeout(resolve, 1000));
      }
    } catch (error) {
      if (current(epoch)) { showError(error); retry.hidden = false; }
    } finally {
      polling = false;
    }
  }

  render();
  void poll(0);
}

document.addEventListener('click', (event) => {
  if ((event.target as Element)?.closest?.('[data-open-agent-sessions]')) openAgentSessions();
});
