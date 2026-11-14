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
// 「隠す」は、この画面（ブラウザ）だけの操作。看板側の依頼はそのまま残る。
const HIDDEN_KEY = 'many-ai-cli:board-handoffs-hidden';
let hiddenIds = new Set<string>();
let hiddenCount = 0;
let showHidden = false;
let notice = ''; // 取り下げに失敗したときの一言。次に取り下げを始めるまで出す
try { hiddenIds = new Set<string>(JSON.parse(localStorage.getItem(HIDDEN_KEY) || '[]')); } catch (_) { /* 読めなければ何も隠さない */ }
function saveHidden(): void {
  try { localStorage.setItem(HIDDEN_KEY, JSON.stringify([...hiddenIds])); } catch (_) { /* 保存できなくても今回は隠れる */ }
}

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
    const count = el.querySelector<HTMLElement>('[data-board-handoffs-count]');
    if (count) { count.textContent = work.length ? String(work.length) : ''; count.hidden = work.length === 0; }
  }
}
function render(): void {
  if (!dialog?.open) return;
  content.replaceChildren(node('p', tr('note'), 'routines-note'));
  if (notice) content.append(node('p', notice, 'routines-note'));
  if (!work.length) content.append(node('p', tr('empty'), 'routines-note'));
  if (hiddenCount) {
    const toggle = button(showHidden ? tr('hide_hidden') : t('board_handoffs_show_hidden', { n: String(hiddenCount) }), () => { showHidden = !showHidden; void refresh(); });
    content.append(toggle);
  }
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
    if (!hiddenIds.has(item.request_id)) {
      controls.append(button(tr('hide'), () => { hiddenIds.add(item.request_id); saveHidden(); void refresh(); }));
    }
    controls.append(button(tr('withdraw'), () => {
      // 看板側の依頼を取り下げる（取り戻せない）。確かめの行を同じカードの中に出す。
      controls.replaceChildren(node('p', tr('withdraw_confirm'), 'routines-note'));
      const cancel = button(tr('cancel'), () => void render());
      const doit = button(tr('withdraw_do'), async () => {
        doit.disabled = true; cancel.disabled = true; notice = '';
        try {
          const response = await apiFetch('/api/board-handoffs', {
            method: 'POST', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ item_id: item.item_id, request_id: item.request_id }),
          });
          // 409 は「もう開いていない」（誰かが受けた・すでに取り下げた）。どちらも一覧から外れるので再取得に任せる。
          if (!response.ok && response.status !== 409) throw new Error(String(response.status));
          await refresh();
          // 断られたのに依頼が残っている（古い看板など）なら、黙らず失敗を伝える。
          if (work.some(w => w.request_id === item.request_id)) { notice = tr('withdraw_failed'); render(); }
        } catch (_) {
          notice = tr('withdraw_failed'); render();
        }
      }, true);
      controls.append(cancel, doit);
    }));
    card.append(controls); content.append(card);
  }
}
async function refresh(): Promise<void> {
  if (loading || document.hidden) return;
  loading = true;
  try {
    const response = await apiFetch('/api/board-handoffs');
    // A Hub without a board, or one that cannot reach it, has nothing to show.
    const all = response.ok ? handedWorkFrom(await response.json()) : [];
    // 看板から消えた依頼の印は捨てる（溜め込まない）。
    const live = new Set(all.map(w => w.request_id));
    const kept = [...hiddenIds].filter(id => live.has(id));
    if (kept.length !== hiddenIds.size) { hiddenIds = new Set(kept); saveHidden(); }
    hiddenCount = all.filter(w => hiddenIds.has(w.request_id)).length;
    work = showHidden ? all : all.filter(w => !hiddenIds.has(w.request_id));
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