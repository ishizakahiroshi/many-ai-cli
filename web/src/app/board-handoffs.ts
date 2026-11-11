// Work the board's owner handed over (Many Iris and other MCP boards): a
// sidebar button that appears only while there is some, and a list. Choosing
// one opens the ordinary new-session form with the folder and the title filled
// in; nothing starts until the person presses Launch there.
import { t } from '../i18n.js';
import { uiButtonClass } from './ui-components.js';
import { apiFetch } from './util.js';
// The filename below names the public UI module this list hands over to.
import { openSpawnPanelWith } from './spawn-panel.js'; // doxguard: allow panel
import { handedWorkFrom, handoffPrompt } from './board-handoffs-model.js';
import type { HandedWork } from './board-handoffs-model.js';

const REFRESH_MS = 60_000;
const tr = (key: string) => t('board_handoffs_' + key);
let work: HandedWork[] = [];
let dialog: HTMLDialogElement | null = null;
let content: HTMLElement;
let loading = false;

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = '', className = ''): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag); el.textContent = text; el.className = className; return el;
}
function button(label: string, action: () => void, primary = false): HTMLButtonElement {
  const el = node('button', label, `${uiButtonClass(primary ? 'primary' : 'secondary')} routine-button`);
  el.type = 'button'; el.addEventListener('click', action); return el;
}
function when(at: number | null): string {
  if (at === null) return '';
  const parsed = new Date(at); if (!Number.isFinite(parsed.valueOf())) return '';
  return new Intl.DateTimeFormat(document.documentElement.lang || 'en', { dateStyle: 'short', timeStyle: 'short' }).format(parsed);
}
function showButtons(): void {
  for (const el of document.querySelectorAll<HTMLElement>('[data-open-board-handoffs]')) {
    el.hidden = work.length === 0;
    const count = el.querySelector('[data-board-handoffs-count]');
    if (count) count.textContent = work.length ? String(work.length) : '';
  }
}
function render(): void {
  if (!dialog?.open) return;
  content.replaceChildren(node('p', tr('note'), 'routines-note'));
  if (!work.length) content.append(node('p', tr('empty'), 'routines-note'));
  for (const item of work) {
    const card = node('article', '', 'routine-card');
    card.append(node('h3', item.title || item.item_id), node('p', `${item.project} · ${item.folder}`, 'routines-note'));
    card.append(node('p', `${tr('request')} ${item.request_id}${when(item.requested_at) ? ' · ' + when(item.requested_at) : ''}`, 'routines-note'));
    const controls = node('div', '', 'routines-actions');
    controls.append(button(tr('start'), () => {
      dialog?.close();
      openSpawnPanelWith({
        cwd: item.folder, prompt: handoffPrompt(item),
        boardHandoff: { item_id: item.item_id, request_id: item.request_id, title: item.title || item.item_id },
      });
    }, true));
    card.append(controls); content.append(card);
  }
}
async function refresh(): Promise<void> {
  if (loading || document.hidden) return;
  loading = true;
  try {
    const response = await apiFetch('/api/board-handoffs');
    // A Hub without a board, or one that cannot reach it, has nothing to show.
    work = response.ok ? handedWorkFrom(await response.json()) : [];
  } catch (_) { /* Keep what was shown; the next round asks again. */ }
  finally { loading = false; }
  showButtons(); render();
}
function ensureDialog(): HTMLDialogElement {
  if (dialog) return dialog;
  dialog = node('dialog', '', 'ui-modal ui-modal--lg routines-dialog aac-wheel-overlay'); dialog.id = 'board-handoffs-dialog';
  dialog.setAttribute('aria-labelledby', 'board-handoffs-title');
  const header = node('div', '', 'routines-header');
  const heading = node('h2', tr('title')); heading.id = 'board-handoffs-title';
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
  document.addEventListener('click', event => { if ((event.target as Element)?.closest?.('[data-open-board-handoffs]')) open(); });
  document.addEventListener('visibilitychange', () => { if (!document.hidden) void refresh(); });
  // A session that took a piece of work removes it from the board's open requests.
  window.addEventListener('many-ai-cli:board-handoff-started', () => { setTimeout(() => void refresh(), 5000); });
  setInterval(() => void refresh(), REFRESH_MS);
  queueMicrotask(() => void refresh());
}