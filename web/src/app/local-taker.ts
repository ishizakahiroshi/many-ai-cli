// The local-claude taker: work the board's owner handed to this PC runs here
// with `claude -p` while the Hub is up. A header button (only on a Hub that has
// `local_taker` in board.json) shows whether it is working, and its dialog
// shows what it is doing, pauses it, and opens what each run did.
import { t } from '../i18n.js';
import { uiButtonClass } from './ui-components.js';
import { apiFetch } from './util.js';
import { desktopNotificationsEnabled } from './settings.js';
import { isDetachedTabView } from './detached-view-mode.js';
import { streamStepsFrom, takerNotices, takerStatusFrom } from './local-taker-model.js';
import type { StreamStep, TakerRun, TakerStatus } from './local-taker-model.js';

// A local call to this machine's Hub: cheap enough to ask often, and asked
// while the tab is hidden too, so a start or an end can be notified.
const REFRESH_MS = 15_000;
const tr = (key: string, vars?: Record<string, string>) => t('local_taker_' + key, vars);
let status: TakerStatus | null = null;
let dialog: HTMLDialogElement | null = null;
let content: HTMLElement;
let loading = false;
let saving = false;
let notice = '';
let openRun = ''; // the request id whose steps are open
let steps: StreamStep[] | null = null;
let stepsFailed = false;

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = '', className = ''): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag); el.textContent = text; el.className = className; return el;
}
function button(label: string, action: () => void, primary = false): HTMLButtonElement {
  const el = node('button', label, `${uiButtonClass(primary ? 'primary' : 'secondary')} routine-button`);
  el.type = 'button'; el.addEventListener('click', action); return el;
}
function clock(at: number | null): string {
  if (at === null) return '';
  const parsed = new Date(at); if (!Number.isFinite(parsed.valueOf())) return '';
  // Today's times without the date.
  const today = parsed.toDateString() === new Date().toDateString();
  return new Intl.DateTimeFormat(document.documentElement.lang || 'en', today ? { timeStyle: 'short' } : { dateStyle: 'short', timeStyle: 'short' }).format(parsed);
}
function minutes(from: number, to: number): string {
  return String(Math.max(0, Math.round((to - from) / 60_000)));
}

/** One line for the button's tooltip and the dialog's first line. */
function stateLine(s: TakerStatus): string {
  if (s.current) return tr('state_running', { title: s.current.title, min: minutes(s.current.started_at, Date.now()) });
  if (s.paused) return tr('state_paused');
  if (s.max_runs_per_day && s.runs_today >= s.max_runs_per_day) return tr('state_capped', { max: String(s.max_runs_per_day) });
  return s.next_look_at ? tr('state_waiting', { at: clock(s.next_look_at) }) : tr('state_starting');
}

function showButtons(): void {
  for (const el of document.querySelectorAll<HTMLElement>('[data-open-local-taker]')) {
    el.hidden = !status?.configured;
    el.dataset.takerState = !status ? '' : status.current ? 'running' : status.paused ? 'paused' : 'waiting';
    if (status?.configured) el.title = `${tr('title')}: ${stateLine(status)}`;
  }
}

function outcomeLabel(run: TakerRun): string {
  const key = ['running', 'done', 'failed', 'interrupted'].includes(run.outcome) ? run.outcome : 'interrupted';
  return tr('outcome_' + key);
}

function renderSteps(into: HTMLElement): void {
  if (stepsFailed) { into.append(node('p', tr('steps_missing'), 'routines-note')); return; }
  if (!steps) { into.append(node('p', tr('steps_loading'), 'routines-note')); return; }
  if (!steps.length) { into.append(node('p', tr('steps_empty'), 'routines-note')); return; }
  const list = node('ol', '', 'local-taker-steps');
  for (const step of steps) {
    const item = node('li', '', `local-taker-step local-taker-step--${step.kind}`);
    item.append(node('span', tr('step_' + step.kind), 'local-taker-step-kind'), node('span', step.text, 'local-taker-step-text'));
    list.append(item);
  }
  into.append(list);
}

async function toggleRun(run: TakerRun): Promise<void> {
  if (openRun === run.request_id) { openRun = ''; steps = null; render(); return; }
  openRun = run.request_id; steps = null; stepsFailed = false; render();
  try {
    const response = await apiFetch(`/api/local-taker/stream?request_id=${encodeURIComponent(run.request_id)}`);
    if (openRun !== run.request_id) return;
    if (!response.ok) throw new Error(String(response.status));
    steps = streamStepsFrom(await response.json());
  } catch (_) {
    if (openRun === run.request_id) stepsFailed = true;
  }
  render();
}

function runCard(run: TakerRun): HTMLElement {
  const card = node('article', '', 'routine-card local-taker-run');
  card.dataset.outcome = run.outcome;
  const head = node('div', '', 'local-taker-run-head');
  head.append(node('h3', run.title || run.item_id), node('span', outcomeLabel(run), 'local-taker-outcome'));
  card.append(head);
  const ended = run.ended_at ? ` – ${clock(run.ended_at)}（${tr('minutes', { min: minutes(run.started_at, run.ended_at) })}）` : '';
  card.append(node('p', `${run.project} · ${tr('request')} ${run.request_id} · ${clock(run.started_at)}${ended}`, 'routines-note'));
  if (run.summary) card.append(node('p', run.summary, 'local-taker-summary'));
  const controls = node('div', '', 'routines-actions');
  controls.append(button(openRun === run.request_id ? tr('steps_close') : tr('steps_open'), () => void toggleRun(run)));
  card.append(controls);
  if (openRun === run.request_id) renderSteps(card);
  return card;
}

function render(): void {
  if (!dialog?.open) return;
  // Redrawn every look: keep the place the person is reading.
  const scrolled = content.scrollTop;
  draw();
  content.scrollTop = scrolled;
}

function draw(): void {
  content.replaceChildren();
  if (!status?.configured) { content.append(node('p', tr('off'), 'routines-note')); return; }
  const s = status;
  const head = node('div', '', 'local-taker-state');
  head.dataset.takerState = s.current ? 'running' : s.paused ? 'paused' : 'waiting';
  head.append(node('p', stateLine(s), 'local-taker-state-line'));
  head.append(node('p', tr('counts', { runs: String(s.runs_today), max: String(s.max_runs_per_day), min: String(Math.round(s.interval_secs / 60)) }), 'routines-note'));
  const controls = node('div', '', 'routines-actions');
  const toggle = button(s.paused ? tr('resume') : tr('pause'), () => void setPaused(!s.paused), s.paused);
  toggle.disabled = saving;
  controls.append(toggle);
  head.append(controls);
  head.append(node('p', s.paused ? tr('paused_note') : tr('pause_note'), 'routines-note'));
  if (notice) head.append(node('p', notice, 'routines-note'));
  content.append(head, node('p', tr('note'), 'routines-note'));
  content.append(node('h3', tr('recent'), 'local-taker-recent-title'));
  if (!s.recent.length) content.append(node('p', tr('recent_empty'), 'routines-note'));
  for (const run of s.recent) content.append(runCard(run));
}

async function setPaused(paused: boolean): Promise<void> {
  saving = true; notice = ''; render();
  try {
    const response = await apiFetch('/api/local-taker', {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ paused }),
    });
    if (!response.ok) throw new Error(String(response.status));
    status = takerStatusFrom(await response.json()) ?? status;
  } catch (_) {
    notice = tr('save_failed');
  } finally { saving = false; }
  showButtons(); render();
}

function notify(after: TakerStatus): void {
  const notices = takerNotices(status, after);
  if (!notices.length || isDetachedTabView() || !desktopNotificationsEnabled()) return;
  if (!('Notification' in window) || Notification.permission !== 'granted') return;
  for (const n of notices) {
    const body = n.kind === 'started'
      ? tr('notify_started', { title: n.run.title })
      : tr(n.run.outcome === 'done' ? 'notify_done' : 'notify_failed', { title: n.run.title });
    try {
      const shown = new Notification(tr('title'), { body: body.slice(0, 160), tag: `many-ai-cli-local-taker-${n.run.request_id}-${n.kind}`, icon: '/icon.svg', badge: '/icon.svg' });
      shown.onclick = () => { window.focus(); open(); shown.close(); };
    } catch (_) { /* a browser that cannot show it still has the dialog */ }
  }
}

async function refresh(): Promise<void> {
  if (loading) return;
  loading = true;
  try {
    const response = await apiFetch('/api/local-taker');
    // A Hub without a board (or an older Hub) has nothing to show.
    const next = response.ok ? takerStatusFrom(await response.json()) : null;
    if (next) notify(next);
    status = next;
  } catch (_) { /* Keep what was shown; the next round asks again. */ }
  finally { loading = false; }
  showButtons(); render();
}

function ensureDialog(): HTMLDialogElement {
  if (dialog) return dialog;
  dialog = node('dialog', '', 'ui-modal ui-modal--lg routines-dialog aac-wheel-overlay'); dialog.id = 'local-taker-dialog';
  dialog.setAttribute('aria-labelledby', 'local-taker-title');
  const header = node('div', '', 'routines-header');
  const heading = node('h2', tr('title')); heading.id = 'local-taker-title';
  const dismiss = button('✕', () => dialog?.close()); dismiss.classList.add('routines-dialog-close'); dismiss.setAttribute('aria-label', t('settings_close'));
  header.append(heading, dismiss);
  content = node('div', '', 'routines-content');
  dialog.append(header, content); document.body.append(dialog);
  return dialog;
}
function open(): void {
  const modal = ensureDialog();
  if (!modal.open) modal.showModal();
  render(); void refresh();
}

if (typeof document !== 'undefined') {
  document.addEventListener('click', event => { if ((event.target as Element)?.closest?.('[data-open-local-taker]')) open(); });
  document.addEventListener('visibilitychange', () => { if (!document.hidden) void refresh(); });
  setInterval(() => void refresh(), REFRESH_MS);
  queueMicrotask(() => void refresh());
}
