import { t } from '../i18n.js';
import { isBatchOptions, isMultiSelectOptions } from './approval-parser.js';
import { sessionTitle, approvalQuestionContext } from './approval-queue-tab.js';
import {
  BATCH_FREE,
  getSingleFreeText,
  sendBatchChoices,
  sendChoice,
  sendMultiSelectChoices,
  sendSingleFreeText,
  setSingleFreeText,
} from './approval.js';
import {
  activeSessionId,
  approvalRawOptionsCache,
  approvalVisibleCache,
  batchActiveQ,
  batchFreeText,
  batchSelections,
  multiQuestionVisibleCache,
  multiSelectSelections,
  sessions,
} from './state.js';
import { showToast } from './util.js';

const mobileMql = (typeof window !== 'undefined' && typeof window.matchMedia === 'function')
  ? window.matchMedia('(max-width: 720px)')
  : null;

let sheetEl: HTMLElement | null = null;
let backdropEl: HTMLElement | null = null;
let contentEl: HTMLElement | null = null;
let openSessionId: number | null = null;
let suppressedActiveApproval = false;
let touchStartY: number | null = null;

function isMobileViewport(): boolean {
  return !!mobileMql?.matches;
}

function isApprovalPending(sessionId: number | null): boolean {
  if (sessionId === null) return false;
  return !!(approvalVisibleCache.get(sessionId) || multiQuestionVisibleCache.get(sessionId));
}

function isComposerBusy(): boolean {
  const input = document.getElementById('input') as HTMLTextAreaElement | null;
  const active = document.activeElement;
  return !!(input && (active === input || input.value.trim().length > 0));
}

function hasComposerDraft(): boolean {
  const input = document.getElementById('input') as HTMLTextAreaElement | null;
  return !!input?.value.trim();
}

export function mobileApprovalActiveBadgeCount(): number {
  if (!isMobileViewport()) return 0;
  if (openSessionId !== null) return 0;
  return activeSessionId !== null && isApprovalPending(activeSessionId) ? 1 : 0;
}

function syncMobileBadgeSoon(): void {
  requestAnimationFrame(() => {
    try { (window as any).syncMobileLayoutState?.(); } catch (_) {}
  });
}

function ensureSheet(): void {
  if (sheetEl && backdropEl && contentEl) return;

  backdropEl = document.createElement('div');
  backdropEl.id = 'mobile-approval-sheet-backdrop';
  backdropEl.hidden = true;

  sheetEl = document.createElement('section');
  sheetEl.id = 'mobile-approval-sheet';
  sheetEl.setAttribute('aria-label', t('mobile_approval_sheet_title'));
  sheetEl.hidden = true;

  const grab = document.createElement('button');
  grab.type = 'button';
  grab.className = 'mas-grab';
  grab.setAttribute('aria-label', t('mobile_approval_sheet_close'));
  grab.addEventListener('click', () => closeApprovalSheet());

  contentEl = document.createElement('div');
  contentEl.className = 'mas-content';

  sheetEl.append(grab, contentEl);
  document.body.append(backdropEl, sheetEl);

  backdropEl.addEventListener('click', () => closeApprovalSheet());
  sheetEl.addEventListener('touchstart', (ev) => {
    if (!isMobileViewport()) return;
    touchStartY = ev.touches[0]?.clientY ?? null;
  }, { passive: true });
  sheetEl.addEventListener('touchend', (ev) => {
    if (touchStartY === null) return;
    const endY = ev.changedTouches[0]?.clientY ?? touchStartY;
    if (endY - touchStartY > 56) closeApprovalSheet();
    touchStartY = null;
  }, { passive: true });
}

function setSheetOpen(open: boolean): void {
  ensureSheet();
  if (!sheetEl || !backdropEl) return;
  sheetEl.hidden = !open;
  backdropEl.hidden = !open;
  document.body.classList.toggle('mobile-approval-sheet-open', open);
  if (!open) openSessionId = null;
  syncMobileBadgeSoon();
}

export function closeApprovalSheet(): void {
  setSheetOpen(false);
}

export function openApprovalSheet(sessionId: number | null = activeSessionId): void {
  if (!isMobileViewport()) return;
  if (sessionId === null || !isApprovalPending(sessionId)) return;
  openSessionId = sessionId;
  suppressedActiveApproval = false;
  renderSheet();
  setSheetOpen(true);
}

function renderHeader(parent: HTMLElement, sessionId: number, titleText: string): void {
  const session = sessions.get(sessionId);
  const header = document.createElement('div');
  header.className = 'mas-header';

  const meta = document.createElement('div');
  meta.className = 'mas-meta';
  meta.textContent = t('mobile_approval_sheet_title');

  const title = document.createElement('div');
  title.className = 'mas-title';
  title.textContent = titleText || sessionTitle(session);

  const close = document.createElement('button');
  close.type = 'button';
  close.className = 'mas-close';
  close.textContent = '×';
  close.setAttribute('aria-label', t('mobile_approval_sheet_close'));
  close.addEventListener('click', () => closeApprovalSheet());

  const text = document.createElement('div');
  text.className = 'mas-header-text';
  text.append(meta, title);
  header.append(text, close);
  parent.appendChild(header);
}

function appendPreamble(parent: HTMLElement, text: unknown): void {
  const value = String(text || '').trim();
  if (!value) return;
  const el = document.createElement('div');
  el.className = 'mas-preamble';
  el.textContent = value;
  parent.appendChild(el);
}

function appendQuestion(parent: HTMLElement, text: unknown): void {
  const value = String(text || '').trim();
  if (!value) return;
  const el = document.createElement('div');
  el.className = 'mas-question';
  el.textContent = value;
  parent.appendChild(el);
}

function optionButton(label: string, num: string | number): HTMLButtonElement {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'mas-option';
  const n = document.createElement('span');
  n.className = 'mas-option-num';
  n.textContent = String(num);
  const l = document.createElement('span');
  l.className = 'mas-option-label';
  l.textContent = label;
  btn.append(n, l);
  return btn;
}

function isRecommendedOption(opt: any, options: any[]): boolean {
  const isSessionAllowLabel = (s: unknown) => /during this session|allow.*session|yes.*allow/i.test(String(s || ''));
  const hasSessionAllow = options.some(o => isSessionAllowLabel(o.label));
  return hasSessionAllow ? isSessionAllowLabel(opt.label) : !!opt.isCurrent;
}

function renderSingle(sessionId: number, options: any[]): void {
  if (!contentEl) return;
  const { preamble, question } = approvalQuestionContext(options);
  renderHeader(contentEl, sessionId, question || t('chat_system_approval_title'));
  appendPreamble(contentEl, preamble);
  appendQuestion(contentEl, question);

  const list = document.createElement('div');
  list.className = 'mas-options';
  for (const opt of options) {
    const label = String(opt.label || opt.send_text || opt.num);
    const btn = optionButton(label + (isRecommendedOption(opt, options) ? ` (${t('approval_recommended')})` : ''), opt.num);
    if (isRecommendedOption(opt, options)) btn.classList.add('is-current');
    btn.addEventListener('click', () => {
      btn.disabled = true;
      sendChoice(sessionId, opt.num);
      closeApprovalSheet();
    });
    list.appendChild(btn);
  }
  contentEl.appendChild(list);

  if ((options as any)._freeInput) {
    const row = document.createElement('div');
    row.className = 'mas-free-row';
    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'mas-free-input';
    input.placeholder = t('approval_free_input_placeholder');
    input.value = getSingleFreeText(sessionId);
    input.addEventListener('input', () => setSingleFreeText(sessionId, input.value));
    input.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter' && !ev.shiftKey && !(ev as any).isComposing) {
        ev.preventDefault();
        sendSingleFreeText(sessionId);
        closeApprovalSheet();
      }
    });
    const send = document.createElement('button');
    send.type = 'button';
    send.className = 'mas-primary';
    send.textContent = t('send');
    send.addEventListener('click', () => {
      sendSingleFreeText(sessionId);
      closeApprovalSheet();
    });
    row.append(input, send);
    contentEl.appendChild(row);
  }
}

function ensureBatchState(sessionId: number, sections: any[]): void {
  const n = sections.length;
  const selections = batchSelections.get(sessionId);
  if (!selections || selections.length !== n) batchSelections.set(sessionId, new Array(n).fill(null));
  const free = batchFreeText.get(sessionId);
  if (!free || free.length !== n) batchFreeText.set(sessionId, new Array(n).fill(''));
  const active = batchActiveQ.get(sessionId);
  if (active == null || active < 0 || active >= n) batchActiveQ.set(sessionId, 0);
}

function isBatchAnswered(sessionId: number, idx: number): boolean {
  const sel = batchSelections.get(sessionId)?.[idx];
  if (sel == null) return false;
  if (sel !== BATCH_FREE) return true;
  return !!batchFreeText.get(sessionId)?.[idx]?.trim();
}

function renderBatch(sessionId: number, sections: any[]): void {
  if (!contentEl) return;
  ensureBatchState(sessionId, sections);
  const idx = batchActiveQ.get(sessionId) || 0;
  const section = sections[idx];
  renderHeader(contentEl, sessionId, t('approval_batch_label', { n: sections.length }));
  appendPreamble(contentEl, (sections as any)._preamble || section?._preamble);

  const progress = document.createElement('div');
  progress.className = 'mas-progress';
  progress.textContent = `Q${idx + 1} / ${sections.length}`;
  contentEl.appendChild(progress);
  appendQuestion(contentEl, section?.title || section?._question);

  const list = document.createElement('div');
  list.className = 'mas-options';
  for (const opt of section.options || []) {
    const btn = optionButton(String(opt.label || opt.num), opt.num);
    if (batchSelections.get(sessionId)?.[idx] === opt.num) btn.classList.add('is-current');
    btn.addEventListener('click', () => {
      const selections = batchSelections.get(sessionId);
      if (!selections) return;
      selections[idx] = opt.num;
      if (idx < sections.length - 1) batchActiveQ.set(sessionId, idx + 1);
      renderSheet();
    });
    list.appendChild(btn);
  }
  if (section._freeInput) {
    const freeBtn = optionButton(t('approval_free_input_full'), 'N');
    freeBtn.addEventListener('click', () => {
      const selections = batchSelections.get(sessionId);
      if (!selections) return;
      selections[idx] = BATCH_FREE;
      renderSheet();
    });
    list.appendChild(freeBtn);
  }
  contentEl.appendChild(list);

  if (batchSelections.get(sessionId)?.[idx] === BATCH_FREE) {
    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'mas-free-input';
    input.placeholder = t('approval_free_input_placeholder');
    input.value = batchFreeText.get(sessionId)?.[idx] || '';
    input.addEventListener('input', () => {
      const free = batchFreeText.get(sessionId);
      if (free) free[idx] = input.value;
    });
    contentEl.appendChild(input);
    setTimeout(() => input.focus(), 0);
  }

  const nav = document.createElement('div');
  nav.className = 'mas-actions';
  const back = document.createElement('button');
  back.type = 'button';
  back.className = 'mas-secondary';
  back.textContent = t('approval_confirm_back');
  back.disabled = idx === 0;
  back.addEventListener('click', () => {
    batchActiveQ.set(sessionId, Math.max(0, idx - 1));
    renderSheet();
  });
  const primary = document.createElement('button');
  primary.type = 'button';
  primary.className = 'mas-primary';
  const allAnswered = sections.every((_: any, i: number) => isBatchAnswered(sessionId, i));
  primary.textContent = idx < sections.length - 1 ? t('mobile_approval_next') : t('approval_batch_submit');
  primary.disabled = idx < sections.length - 1 ? !isBatchAnswered(sessionId, idx) : !allAnswered;
  primary.addEventListener('click', () => {
    if (idx < sections.length - 1) {
      batchActiveQ.set(sessionId, idx + 1);
      renderSheet();
      return;
    }
    sendBatchChoices(sessionId);
    closeApprovalSheet();
  });
  nav.append(back, primary);
  contentEl.appendChild(nav);
}

function renderMulti(sessionId: number, options: any[]): void {
  if (!contentEl) return;
  let selected = multiSelectSelections.get(sessionId);
  if (!selected) {
    selected = new Set<number>();
    multiSelectSelections.set(sessionId, selected);
  }
  const question = String(options[0]?._question || '').trim();
  renderHeader(contentEl, sessionId, question || t('approval_multi_label'));
  appendPreamble(contentEl, (options as any)._preamble);
  appendQuestion(contentEl, question);

  const list = document.createElement('div');
  list.className = 'mas-options';
  for (const opt of options) {
    const checked = selected.has(opt.num);
    const btn = optionButton(String(opt.label || opt.num), checked ? '✓' : opt.num);
    btn.classList.toggle('is-current', checked);
    btn.addEventListener('click', () => {
      const s = multiSelectSelections.get(sessionId) || new Set<number>();
      if (s.has(opt.num)) s.delete(opt.num);
      else s.add(opt.num);
      multiSelectSelections.set(sessionId, s);
      renderSheet();
    });
    list.appendChild(btn);
  }
  contentEl.appendChild(list);

  const nav = document.createElement('div');
  nav.className = 'mas-actions';
  const clear = document.createElement('button');
  clear.type = 'button';
  clear.className = 'mas-secondary';
  clear.textContent = t('approval_batch_clear');
  clear.addEventListener('click', () => {
    multiSelectSelections.set(sessionId, new Set<number>());
    renderSheet();
  });
  const submit = document.createElement('button');
  submit.type = 'button';
  submit.className = 'mas-primary';
  submit.textContent = t('approval_batch_submit');
  submit.disabled = selected.size === 0;
  submit.addEventListener('click', () => {
    sendMultiSelectChoices(sessionId);
    closeApprovalSheet();
  });
  nav.append(clear, submit);
  contentEl.appendChild(nav);
}

function renderFallback(sessionId: number): void {
  if (!contentEl) return;
  renderHeader(contentEl, sessionId, t('chat_system_approval_title'));
  const msg = document.createElement('div');
  msg.className = 'mas-empty';
  msg.textContent = t('approval_tab_detecting');
  contentEl.appendChild(msg);
}

function renderSheet(): void {
  ensureSheet();
  if (!contentEl || openSessionId === null) return;
  contentEl.innerHTML = '';
  const options = approvalRawOptionsCache.get(openSessionId);
  if (Array.isArray(options) && isBatchOptions(options)) renderBatch(openSessionId, options);
  else if (Array.isArray(options) && isMultiSelectOptions(options)) renderMulti(openSessionId, options);
  else if (Array.isArray(options) && options.length > 0) renderSingle(openSessionId, options);
  else renderFallback(openSessionId);
}

function maybeAutoOpen(): void {
  if (!isMobileViewport()) {
    closeApprovalSheet();
    return;
  }
  if (openSessionId !== null && !isApprovalPending(openSessionId)) {
    closeApprovalSheet();
    showToast(t('mobile_approval_resolved_toast'));
    return;
  }
  if (openSessionId !== null) {
    renderSheet();
    return;
  }
  if (activeSessionId === null || !isApprovalPending(activeSessionId)) {
    suppressedActiveApproval = false;
    syncMobileBadgeSoon();
    return;
  }
  if (isComposerBusy()) {
    suppressedActiveApproval = true;
    syncMobileBadgeSoon();
    return;
  }
  openApprovalSheet(activeSessionId);
}

function retrySuppressedOpen(): void {
  if (!suppressedActiveApproval) return;
  if (activeSessionId === null || !isApprovalPending(activeSessionId)) {
    suppressedActiveApproval = false;
    syncMobileBadgeSoon();
    return;
  }
  if (!isComposerBusy()) openApprovalSheet(activeSessionId);
  else syncMobileBadgeSoon();
}

function openAfterComposerIdle(): void {
  if (!suppressedActiveApproval) return;
  if (activeSessionId === null || !isApprovalPending(activeSessionId)) return;
  if (!hasComposerDraft()) openApprovalSheet(activeSessionId);
}

window.addEventListener('approval-queue-updated', maybeAutoOpen);
mobileMql?.addEventListener('change', maybeAutoOpen);
document.addEventListener('input', (ev) => {
  if ((ev.target as HTMLElement | null)?.id === 'input') setTimeout(retrySuppressedOpen, 0);
});
document.addEventListener('focusout', (ev) => {
  if ((ev.target as HTMLElement | null)?.id === 'input') setTimeout(retrySuppressedOpen, 80);
});
window.addEventListener('mobile-composer-idle', () => setTimeout(openAfterComposerIdle, 0));

export function openMobileApprovalSheetForSession(sessionId: number): void {
  openApprovalSheet(sessionId);
}

(window as any).openMobileApprovalSheetForSession = openMobileApprovalSheetForSession;
