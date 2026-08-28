import { t } from '../i18n.js';
import type { RelayEvent, RelayStatus } from '../types/proto.js';
import { dirnameForPath } from './path-links.js';
import { FilesTabManager } from './files-view.js';
import { sessions } from './state.js';
import { escapeHtml, showToast, token } from './util.js';
import { openRelayDialog } from './relay-dialog.js';

// C1 (P-18): conductor とその子の現在状態を、既存の session snapshot だけで
// まとめて見せる軽量ダッシュボード。relay は C4 で API の時系列を加えている。
let selectedConductorID: number | null = null;
let relayTimelineCache = new Map<string, RelayEvent[]>();
let relayTimelineFetchKey = '';
let relayTimelineRequestID = 0;
const relayTimelineErrors = new Map<number, string>();

const RELAY_ACTIVE_STATES = new Set(['implementing', 'reviewing', 'fixing']);
const RELAY_RESUMABLE_REASONS = new Set(['hub_restart', 'child_exited', 'timeout']);

function stateLabel(state: string): string {
  const labels: Record<string, string> = {
    running: '実行中', waiting: '待機', standby: '待機中', completed: '完了',
    error: 'エラー', disconnected: '切断',
  };
  return labels[state] || state || '待機中';
}

function i18n(key: string, fallback: string): string {
  const value = t(key);
  return value === key ? fallback : value;
}

function conductorSessions(): any[] {
  return Array.from(sessions.values())
    .filter((session: any) => session.orchestration_id && !session.parent_session_id)
    .sort((a: any, b: any) => Number(b.id) - Number(a.id));
}

function doneFor(child: any): boolean {
  // session_end 経路で Hub が completed にするため、marker の有無に依存しない。
  return child.state === 'completed';
}

function worktreeLabel(child: any): string {
  return String(child.worktree_branch || child.cwd || '既定の作業ディレクトリ');
}

function relayShortID(relay: RelayStatus): string {
  const id = String(relay.orchestration_id || 'unknown');
  return id.length > 6 ? id.slice(-6) : id;
}

function basename(filePath: string): string {
  const normalized = String(filePath || '').replace(/[\\/]+$/, '');
  return normalized.split(/[\\/]/).pop() || normalized;
}

function relayStateClass(state: string): string {
  const safe = state.toLowerCase().replace(/[^a-z0-9-]/g, '');
  return safe || 'unknown';
}

function relayReasonParts(reason: string): { key: string; detail: string } {
  const raw = String(reason || '');
  const separator = raw.indexOf(':');
  if (separator < 0) return { key: raw, detail: '' };
  return { key: raw.slice(0, separator).trim(), detail: raw.slice(separator + 1).trim() };
}

function appendField(parent: HTMLElement, label: string, value: string, title = ''): HTMLElement {
  const item = document.createElement('div');
  const dt = document.createElement('dt');
  dt.textContent = label;
  const dd = document.createElement('dd');
  dd.textContent = value;
  if (title) dd.title = title;
  item.append(dt, dd);
  parent.appendChild(item);
  return dd;
}

function openRelayFile(conductor: any, filePath: string): void {
  if (!filePath) return;
  const root = dirnameForPath(filePath) || filePath;
  FilesTabManager.openFilesTabAtFile(
    Number(conductor.id),
    String(conductor.orchestration_id || conductor.cwd || ''),
    root,
    root,
    filePath,
  );
}

function openRelaySession(sessionID: number): void {
  if (!Number.isInteger(sessionID) || sessionID <= 0) return;
  window.dispatchEvent(new CustomEvent('orchestration-dashboard-open-session', { detail: { sessionID } }));
}

async function copyRelayText(text: string, anchor: HTMLElement, messageKey = 'copied_to_clipboard'): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    showToast(i18n(messageKey, messageKey), anchor);
  } catch (_) {
    showToast(i18n('relay_action_failed', 'Relay action failed'), anchor);
  }
}

function appendSessionRole(container: HTMLElement, role: string, labelKey: string, sessionID: number, active: boolean): void {
  const item = document.createElement('div');
  item.className = 'orchestration-relay-role';
  const label = document.createElement('span');
  label.textContent = i18n(labelKey, role);
  item.appendChild(label);
  if (sessionID > 0) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = `orchestration-relay-session${active ? ' active' : ''}`;
    button.textContent = `#${sessionID}`;
    button.title = i18n('relay_active_implementer', 'Active implementer');
    button.addEventListener('click', (event) => {
      event.stopPropagation();
      openRelaySession(sessionID);
    });
    item.appendChild(button);
  } else if (role === 'implementation-strong') {
    const pending = document.createElement('span');
    pending.className = 'orchestration-relay-session relay-role-not-started';
    pending.textContent = i18n('relay_strong_not_started', 'not started');
    item.appendChild(pending);
  }
  container.appendChild(item);
}

function appendRelayActionButton(container: HTMLElement, conductor: any, relay: RelayStatus, action: 'relay-stop' | 'relay-resume' | 'relay-cleanup', labelKey: string): void {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'orchestration-relay-action';
  button.textContent = i18n(labelKey, action);
  button.addEventListener('click', (event) => {
    event.stopPropagation();
    button.disabled = true;
    void sendRelayAction(conductor, relay, action, button);
  });
  container.appendChild(button);
}

async function sendRelayAction(conductor: any, relay: RelayStatus, action: string, button: HTMLButtonElement): Promise<void> {
  const sessionID = Number(conductor.id);
  const relayID = String(relay.orchestration_id || '');
  try {
    const response = await fetch(`/api/sessions/${encodeURIComponent(String(sessionID))}/${action}?token=${encodeURIComponent(token || '')}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ orchestration_id: relayID }),
    });
    const data = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(String(data.detail || data.error || `HTTP ${response.status}`));
    if (data.relay && typeof data.relay === 'object') Object.assign(relay, data.relay);
    relayTimelineFetchKey = '';
    relayTimelineErrors.delete(sessionID);
    renderOrchestrationDashboard();
  } catch (error) {
    showToast(`${i18n('relay_action_failed', 'Relay action failed')}: ${String(error instanceof Error ? error.message : error)}`, button);
    button.disabled = false;
  }
}

function relayModeLabel(mode: string): string {
  return mode === 'same-tree'
    ? i18n('relay_mode_same_tree', 'same-tree')
    : i18n('relay_mode_worktree', 'worktree');
}

function relayStateLabel(state: string): string {
  return i18n(`relay_state_${state}`, state || 'unknown');
}

function renderRelayCard(conductor: any, relay: RelayStatus): HTMLElement {
  const state = String(relay.state || 'unknown');
  const card = document.createElement('article');
  card.className = `orchestration-relay-card orchestration-relay-state-${relayStateClass(state)}`;

  const header = document.createElement('div');
  header.className = 'orchestration-relay-card-head';
  const title = document.createElement('strong');
  title.textContent = `${i18n('relay_card_title', 'Relay')} #${relayShortID(relay)}`;
  const stateBadge = document.createElement('span');
  stateBadge.className = 'orchestration-relay-state-badge';
  stateBadge.textContent = relayStateLabel(state);
  header.append(title, stateBadge);
  card.appendChild(header);

  const meta = document.createElement('dl');
  meta.className = 'orchestration-relay-meta';
  const planPath = String(relay.plan_path || '');
  appendField(meta, i18n('relay_plan', 'Plan'), basename(planPath), planPath);
  appendField(meta, i18n('relay_mode', 'Mode'), relayModeLabel(String(relay.mode || 'worktree')));
  appendField(meta, i18n('relay_completed_cs', 'Completed C'), String(relay.completed_cs ?? 0));
  appendField(meta, i18n('relay_round', 'Round'), `${relay.round ?? 0} / ${relay.max_rounds ?? 0}`);
  if (state === 'stopped' && relay.reason) {
    const reason = relayReasonParts(String(relay.reason));
    const reasonText = `${i18n(`relay_reason_${reason.key}`, reason.key)}${reason.detail ? `: ${reason.detail}` : ''}`;
    appendField(meta, i18n('relay_reason', 'Reason'), reasonText);
  }
  if (relay.branch) {
    const branchCell = appendField(meta, i18n('relay_branch', 'Branch'), '');
    const branchButton = document.createElement('button');
    branchButton.type = 'button';
    branchButton.className = 'orchestration-relay-branch';
    branchButton.textContent = String(relay.branch);
    branchButton.title = `git merge ${String(relay.branch)}`;
    branchButton.addEventListener('click', (event) => {
      event.stopPropagation();
      void copyRelayText(`git merge ${String(relay.branch)}`, branchButton, 'relay_branch_copied');
    });
    branchCell.replaceChildren(branchButton);
  }
  card.appendChild(meta);

  const roles = document.createElement('div');
  roles.className = 'orchestration-relay-roles';
  appendSessionRole(roles, 'implementation', 'relay_role_implementation', Number(relay.implementation_session_id || 0), relay.active_implementer === 'implementation');
  appendSessionRole(roles, 'implementation-strong', 'relay_role_strong', Number(relay.strong_session_id || 0), relay.active_implementer === 'implementation-strong');
  appendSessionRole(roles, 'review', 'relay_role_review', Number(relay.review_session_id || 0), relay.active_implementer === 'review');
  if (relay.active_implementer) {
    const active = document.createElement('span');
    active.className = 'orchestration-relay-active-label';
    active.textContent = `${i18n('relay_active_implementer', 'Active implementer')}: ${i18n(`relay_role_${relay.active_implementer === 'implementation-strong' ? 'strong' : relay.active_implementer}`, String(relay.active_implementer))}`;
    roles.appendChild(active);
  }
  card.appendChild(roles);

  if (relay.review_path) {
    const review = document.createElement('button');
    review.type = 'button';
    review.className = 'orchestration-relay-review-link';
    review.textContent = `${i18n('relay_open_review', 'Open latest review')}: ${basename(String(relay.review_path))}`;
    review.title = String(relay.review_path);
    review.addEventListener('click', (event) => {
      event.stopPropagation();
      openRelayFile(conductor, String(relay.review_path));
    });
    card.appendChild(review);
  }

  const actions = document.createElement('div');
  actions.className = 'orchestration-relay-actions';
  if (RELAY_ACTIVE_STATES.has(state)) {
    appendRelayActionButton(actions, conductor, relay, 'relay-stop', 'relay_stop');
  }
  if (state === 'stopped' && RELAY_RESUMABLE_REASONS.has(relayReasonParts(String(relay.reason || '')).key)) {
    appendRelayActionButton(actions, conductor, relay, 'relay-resume', 'relay_resume');
  }
  if ((state === 'completed' || state === 'stopped') && relay.mode === 'worktree' && relay.worktree_path) {
    appendRelayActionButton(actions, conductor, relay, 'relay-cleanup', 'relay_cleanup_worktree');
  }
  if (actions.childElementCount > 0) card.appendChild(actions);

  const events = relayTimelineCache.get(String(relay.orchestration_id || '')) || [];
  card.appendChild(renderRelayTimeline(conductor, relay, events));
  return card;
}

export function renderRelayCards(conductor: any): HTMLElement {
  const wrapper = document.createElement('section');
  wrapper.className = 'orchestration-relay-cards';
  const heading = document.createElement('h3');
  heading.textContent = i18n('relay_card_title', 'Relay');
  wrapper.appendChild(heading);
  const relays: RelayStatus[] = Array.isArray(conductor.relays) ? conductor.relays.slice() : [];
  relays.sort((a, b) => {
    const activeA = RELAY_ACTIVE_STATES.has(String(a.state || '')) ? 0 : 1;
    const activeB = RELAY_ACTIVE_STATES.has(String(b.state || '')) ? 0 : 1;
    return activeA - activeB;
  });
  const cards = document.createElement('div');
  cards.className = 'orchestration-relay-card-list';
  relays.forEach((relay) => cards.appendChild(renderRelayCard(conductor, relay)));
  wrapper.appendChild(cards);
  return wrapper;
}

function formatRelayEventTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value || '—';
  return date.toLocaleString(undefined, {
    year: 'numeric', month: '2-digit', day: '2-digit',
    hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false,
  });
}

function appendRelayEventField(row: HTMLElement, value: string, className: string): HTMLElement {
  const span = document.createElement('span');
  span.className = className;
  span.textContent = value;
  row.appendChild(span);
  return span;
}

function renderRelayEventRow(conductor: any, event: RelayEvent): HTMLElement {
  const row = document.createElement('div');
  row.className = 'orchestration-relay-timeline-row';
  appendRelayEventField(row, formatRelayEventTime(String(event.at || '')), 'relay-event-time');
  appendRelayEventField(row, i18n(`relay_event_${event.kind || 'unknown'}`, String(event.kind || 'event')), 'relay-event-kind');
  if (event.c !== undefined) appendRelayEventField(row, `C${event.c}`, 'relay-event-c');
  if (event.round !== undefined) appendRelayEventField(row, `r${event.round}`, 'relay-event-round');
  if (event.review_path) {
    const review = document.createElement('button');
    review.type = 'button';
    review.className = 'relay-event-review';
    review.textContent = basename(String(event.review_path));
    review.title = String(event.review_path);
    review.addEventListener('click', (clickEvent) => {
      clickEvent.stopPropagation();
      openRelayFile(conductor, String(event.review_path));
    });
    row.appendChild(review);
  }
  if (event.commit) {
    const commit = document.createElement('button');
    commit.type = 'button';
    commit.className = 'relay-event-commit';
    commit.textContent = String(event.commit).slice(0, 7);
    commit.title = String(event.commit);
    commit.addEventListener('click', (clickEvent) => {
      clickEvent.stopPropagation();
      void copyRelayText(String(event.commit), commit);
    });
    row.appendChild(commit);
  }
  if (event.files_changed !== undefined) appendRelayEventField(row, `files:${event.files_changed}`, 'relay-event-files');
  if (event.text) appendRelayEventField(row, String(event.text), 'relay-event-text');
  return row;
}

export function renderRelayTimeline(conductor: any, relay: RelayStatus, events: RelayEvent[] = []): HTMLElement {
  const details = document.createElement('details');
  details.className = 'orchestration-relay-timeline';
  const state = String(relay.state || '');
  details.open = RELAY_ACTIVE_STATES.has(state);
  const summary = document.createElement('summary');
  summary.textContent = `${i18n('relay_timeline_title', 'Timeline')} (${events.length})`;
  details.appendChild(summary);
  const rows = document.createElement('div');
  rows.className = 'orchestration-relay-timeline-rows';
  const ordered = events.slice().reverse();
  const renderRows = (showAll: boolean) => {
    rows.replaceChildren(...ordered.slice(0, showAll ? ordered.length : 20).map((event) => renderRelayEventRow(conductor, event)));
  };
  renderRows(false);
  details.appendChild(rows);
  if (ordered.length > 20) {
    const showAll = document.createElement('button');
    showAll.type = 'button';
    showAll.className = 'orchestration-relay-timeline-more';
    showAll.textContent = i18n('relay_timeline_show_all', 'Show all');
    let expanded = false;
    showAll.addEventListener('click', (event) => {
      event.stopPropagation();
      expanded = !expanded;
      renderRows(expanded);
      showAll.textContent = expanded ? i18n('relay_timeline_show_less', 'Show less') : i18n('relay_timeline_show_all', 'Show all');
    });
    details.appendChild(showAll);
  }
  return details;
}

function appendChildRelayBadges(head: HTMLElement, child: any, relays: RelayStatus[]): void {
  relays.forEach((relay) => {
    const childID = Number(child.id);
    const matches: Array<{ id: number; key: string }> = [
      { id: Number(relay.implementation_session_id || 0), key: 'relay_role_implementation' },
      { id: Number(relay.strong_session_id || 0), key: 'relay_role_strong' },
      { id: Number(relay.review_session_id || 0), key: 'relay_role_review' },
    ];
    matches.filter((match) => match.id > 0 && match.id === childID).forEach((match) => {
      const badge = document.createElement('span');
      badge.className = 'orchestration-relay-role-badge';
      badge.textContent = `${i18n(match.key, 'relay')} #${relayShortID(relay)}`;
      head.appendChild(badge);
    });
  });
}

async function loadRelayTimeline(conductorID: number, relays: RelayStatus[]): Promise<void> {
  const relayIDs = relays.map((relay) => [
    relay.orchestration_id || '',
    relay.updated_at || '',
    relay.state || '',
    relay.completed_cs || 0,
    relay.round || 0,
    relay.reason || '',
    relay.review_path || '',
  ].join(':')).join('|');
  const key = `${conductorID}:${relayIDs}`;
  if (!relayIDs || key === relayTimelineFetchKey) return;
  relayTimelineFetchKey = key;
  const requestID = ++relayTimelineRequestID;
  try {
    const response = await fetch(`/api/sessions/${encodeURIComponent(String(conductorID))}/relay?token=${encodeURIComponent(token || '')}`);
    const data = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(String(data.detail || data.error || `HTTP ${response.status}`));
    const next = new Map<string, RelayEvent[]>();
    if (Array.isArray(data.relays)) {
      data.relays.forEach((entry: any) => {
        const relayID = String(entry?.relay?.orchestration_id || '');
        if (relayID) next.set(relayID, Array.isArray(entry.events) ? entry.events : []);
      });
    }
    relayTimelineCache = next;
    relayTimelineErrors.delete(conductorID);
    if (requestID === relayTimelineRequestID && selectedConductorID === conductorID) renderOrchestrationDashboard(false);
  } catch (error) {
    relayTimelineErrors.set(conductorID, String(error instanceof Error ? error.message : error));
    if (requestID === relayTimelineRequestID && selectedConductorID === conductorID) renderOrchestrationDashboard(false);
  }
}

export function renderOrchestrationDashboard(fetchTimeline = true): void {
  const pane = document.getElementById('orchestration-dashboard-pane');
  if (!pane) return;
  const conductors = conductorSessions();
  if (!conductors.some((session: any) => session.id === selectedConductorID)) {
    selectedConductorID = conductors[0]?.id ?? null;
  }

  pane.replaceChildren();
  const heading = document.createElement('div');
  heading.className = 'orchestration-dashboard-heading';
  heading.innerHTML = '<div><h2>Orchestration</h2><p>指揮者と子セッションの現在地</p></div>';
  pane.appendChild(heading);

  if (conductors.length === 0 || selectedConductorID === null) {
    relayTimelineFetchKey = '';
    relayTimelineCache.clear();
    const empty = document.createElement('div');
    empty.className = 'orchestration-dashboard-empty';
    empty.textContent = '実行中または履歴中の orchestration conductor はありません。';
    pane.appendChild(empty);
    return;
  }

  const tabs = document.createElement('div');
  tabs.className = 'orchestration-conductor-tabs';
  conductors.forEach((conductor: any) => {
    const tab = document.createElement('button');
    tab.type = 'button';
    tab.className = 'orchestration-conductor-tab' + (conductor.id === selectedConductorID ? ' active' : '');
    tab.textContent = `#${conductor.id} ${conductor.label || conductor.auto_title || 'conductor'}`;
    tab.title = conductor.cwd || '';
    tab.addEventListener('click', () => { selectedConductorID = conductor.id; relayTimelineFetchKey = ''; renderOrchestrationDashboard(); });
    tabs.appendChild(tab);
  });
  pane.appendChild(tabs);

  const conductor = conductors.find((session: any) => session.id === selectedConductorID);
  if (!conductor) return;
  const detail = document.createElement('div');
  detail.className = 'orchestration-conductor-detail';
  detail.innerHTML = `<span class="orchestration-detail-role">conductor #${escapeHtml(String(conductor.id))}</span><span>${escapeHtml(String(conductor.provider || 'unknown'))}</span><span>${escapeHtml(stateLabel(String(conductor.state || 'standby')))}</span><span title="${escapeHtml(String(conductor.board_path || ''))}">${escapeHtml(String(conductor.board_path || 'board 未作成'))}</span>`;
  const startRelayButton = document.createElement('button');
  startRelayButton.type = 'button';
  startRelayButton.className = 'relay-start-button';
  startRelayButton.textContent = i18n('relay_start', 'Start relay');
  startRelayButton.addEventListener('click', () => openRelayDialog(Number(conductor.id)));
  detail.appendChild(startRelayButton);
  pane.appendChild(detail);

  const relays: RelayStatus[] = Array.isArray(conductor.relays) ? conductor.relays : [];
  if (relays.length > 0) {
    pane.appendChild(renderRelayCards(conductor));
    if (fetchTimeline) void loadRelayTimeline(Number(conductor.id), relays);
    const timelineError = relayTimelineErrors.get(Number(conductor.id));
    if (timelineError) {
      const error = document.createElement('p');
      error.className = 'orchestration-relay-timeline-error';
      error.textContent = `${i18n('relay_action_failed', 'Relay action failed')}: ${timelineError}`;
      pane.appendChild(error);
    }
  }

  const children = Array.from(sessions.values())
    .filter((session: any) => session.parent_session_id === conductor.id)
    .sort((a: any, b: any) => Number(a.id) - Number(b.id));
  const grid = document.createElement('div');
  grid.className = 'orchestration-child-grid';
  children.forEach((child: any) => {
    const done = doneFor(child);
    const card = document.createElement('button');
    card.type = 'button';
    card.className = `orchestration-child-card state-${relayStateClass(String(child.state || 'standby'))}${done ? ' done' : ''}`;
    card.innerHTML = `<div class="orchestration-child-card-head"><strong>${escapeHtml(String(child.role || 'child'))}</strong><span class="orchestration-done-badge ${done ? 'done' : 'pending'}">${done ? 'DONE' : '作業中'}</span></div><dl><div><dt>状態</dt><dd>${escapeHtml(stateLabel(String(child.state || 'standby')))}</dd></div><div><dt>Provider</dt><dd>${escapeHtml(String(child.provider || 'unknown'))}</dd></div><div><dt>Worktree</dt><dd title="${escapeHtml(worktreeLabel(child))}">${escapeHtml(worktreeLabel(child))}</dd></div></dl>`;
    const childHead = card.querySelector('.orchestration-child-card-head');
    if (childHead instanceof HTMLElement) appendChildRelayBadges(childHead, child, relays);
    card.addEventListener('click', () => openRelaySession(Number(child.id)));
    grid.appendChild(card);
  });
  if (children.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'orchestration-dashboard-empty';
    empty.textContent = 'この conductor はまだ子セッションを開始していません。';
    grid.appendChild(empty);
  }
  pane.appendChild(grid);
}

window.renderOrchestrationDashboard = renderOrchestrationDashboard;
