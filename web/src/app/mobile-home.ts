// mobile-home.ts
// スマホホーム画面（#mobile-home）と左ドロワーの描画モジュール。
// PC には一切副作用を与えない。全エントリーポイントは isMobileViewport() で early return する。

import { t } from '../i18n.js';
import { orderSessions, sessions, approvalVisibleCache, multiQuestionVisibleCache, approvalRawOptionsCache, activeSessionId, set_activeSessionId } from './state.js';
import { activateSession, providerIconHtml } from './session-list.js';
import { filterFirstMessage } from './settings.js';
import { isBatchOptions, isMultiSelectOptions } from './approval-parser.js';
import { sessionTitle, approvalQuestionContext, renderOptionButtons } from './approval-queue-tab.js';
import { getSingleFreeText, setSingleFreeText, sendSingleFreeText } from './approval.js';

const mobileMql = (typeof window !== 'undefined' && typeof window.matchMedia === 'function')
  ? window.matchMedia('(max-width: 720px)')
  : null;
function isMobileViewport(): boolean { return !!mobileMql?.matches; }

type SessionBucket = 'pending' | 'running' | 'waiting' | 'error';

let mobileHomeSearch = '';
let mobileDrawerSearch = '';

function getSessionBucket(id: number): SessionBucket {
  const s = sessions.get(id);
  if (approvalVisibleCache.get(id) || multiQuestionVisibleCache.get(id)) return 'pending';
  const state = s?.state || 'standby';
  if (state === 'error' || state === 'disconnected') return 'error';
  if (state === 'running') return 'running';
  return 'waiting';
}

function sessionSearchText(id: number): string {
  const s = sessions.get(id);
  return [
    String(id),
    sessionTitle(s),
    s?.label || '',
    s?.cwd || '',
    String(s?.cwd || '').replace(/\\/g, '/').split('/').filter(Boolean).pop() || '',
  ].join(' ').toLowerCase();
}

function matchesSearch(id: number, query: string): boolean {
  const q = query.trim().toLowerCase();
  return !q || sessionSearchText(id).includes(q);
}

function statusChipText(bucket: SessionBucket): string {
  if (bucket === 'pending') return t('mobile_state_pending');
  if (bucket === 'running') return t('mobile_state_running');
  if (bucket === 'error') return t('mobile_state_disconnected');
  return t('mobile_state_idle');
}

function progressText(bucket: SessionBucket): string {
  if (bucket === 'pending') return t('mobile_status_waiting_approval');
  if (bucket === 'running') return t('mobile_status_running', { min: 0 });
  if (bucket === 'error') return t('mobile_status_disconnected');
  return t('mobile_status_idle');
}

function buildTitleHtml(id: number, iconSize = 20): string {
  const s = sessions.get(id);
  const iconHtml = s ? providerIconHtml(s.provider, iconSize) : '';
  return `${iconHtml}<span class="mh-session-id">#${id}</span><span class="mh-session-name">${escapeHtml(sessionTitle(s))}</span>`;
}

function appendCardHeader(parent: HTMLElement, id: number, bucket: SessionBucket): void {
  const header = document.createElement('div');
  header.className = 'mh-card-header';

  const titleEl = document.createElement('div');
  titleEl.className = 'mh-card-title';
  titleEl.innerHTML = buildTitleHtml(id);

  const stateChip = document.createElement('span');
  stateChip.className = `mh-state-chip mh-state-chip--${bucket}`;
  stateChip.textContent = statusChipText(bucket);

  header.append(titleEl, stateChip);
  parent.appendChild(header);
}

function appendProgressLine(parent: HTMLElement, id: number, bucket: SessionBucket): void {
  const s = sessions.get(id);
  const line = document.createElement('div');
  line.className = 'mh-progress-line';
  const snippet = filterFirstMessage(s?.last_message || s?.first_message || '');
  line.textContent = snippet || progressText(bucket);
  parent.appendChild(line);
}

function buildSessionRow(id: number, compact = false): HTMLElement {
  const bucket = getSessionBucket(id);
  const row = document.createElement('button');
  row.type = 'button';
  row.className = `mh-session-row mh-session-row--${bucket}${compact ? ' mh-session-row--compact' : ''}`;
  row.dataset.sessionId = String(id);
  row.classList.toggle('is-active', id === activeSessionId);

  const main = document.createElement('div');
  main.className = 'mh-row-main';

  const title = document.createElement('div');
  title.className = 'mh-row-title';
  title.innerHTML = buildTitleHtml(id, compact ? 16 : 18);

  const progress = document.createElement('div');
  progress.className = 'mh-row-progress';
  progress.textContent = progressText(bucket);

  main.append(title, progress);

  const chip = document.createElement('span');
  chip.className = `mh-state-chip mh-state-chip--${bucket}`;
  chip.textContent = statusChipText(bucket);

  row.append(main, chip);
  row.addEventListener('click', () => {
    activateSession(id);
    (window as any).closeMobileSessionDrawer?.();
  });
  return row;
}

function buildCard(id: number): HTMLElement {
  const bucket = getSessionBucket(id);
  if (bucket !== 'pending') return buildSessionRow(id);

  const options = approvalRawOptionsCache.get(id);
  const isMultiQ = !!multiQuestionVisibleCache.get(id);
  const isBatch = Array.isArray(options) && isBatchOptions(options);
  const isMultiSel = Array.isArray(options) && isMultiSelectOptions?.(options);

  const card = document.createElement('div');
  card.className = 'mh-card mh-card--pending';
  card.dataset.sessionId = String(id);

  appendCardHeader(card, id, bucket);

  if (isMultiQ || isBatch || isMultiSel) {
    const fallback = document.createElement('button');
    fallback.type = 'button';
    fallback.className = 'mh-approval-fallback';
    fallback.textContent = t('mobile_approval_open_sheet');
    fallback.addEventListener('click', (e) => {
      e.stopPropagation();
      (window as any).openMobileApprovalSheetForSession?.(id);
    });
    card.appendChild(fallback);
  } else if (Array.isArray(options) && options.length > 0) {
    const { preamble, question } = approvalQuestionContext(options);
    if (preamble) {
      const preEl = document.createElement('div');
      preEl.className = 'mh-card-preamble';
      preEl.textContent = preamble;
      card.appendChild(preEl);
    }
    if (question) {
      const qEl = document.createElement('div');
      qEl.className = 'mh-card-question';
      qEl.textContent = question;
      card.appendChild(qEl);
    }

    const optContainer = document.createElement('div');
    optContainer.className = 'mh-options';
    renderOptionButtons(optContainer, id, options);
    card.appendChild(optContainer);

    if ((options as any)._freeInput) {
      const freeWrap = document.createElement('div');
      freeWrap.className = 'mh-free-input';

      const inp = document.createElement('input');
      inp.type = 'text';
      inp.className = 'mh-free-input-field';
      inp.placeholder = t('approval_free_input_placeholder');
      inp.value = getSingleFreeText(id);
      inp.addEventListener('click', (e) => e.stopPropagation());
      inp.addEventListener('input', () => setSingleFreeText(id, inp.value));
      inp.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' && !(e as any).isComposing && !e.shiftKey) {
          e.preventDefault();
          e.stopPropagation();
          sendSingleFreeText(id);
        }
      });

      const sendBtn = document.createElement('button');
      sendBtn.type = 'button';
      sendBtn.className = 'mh-free-input-send';
      sendBtn.textContent = t('send');
      sendBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        sendSingleFreeText(id);
      });

      freeWrap.append(inp, sendBtn);
      card.appendChild(freeWrap);
    }
  } else {
    appendProgressLine(card, id, bucket);
  }

  card.addEventListener('click', (e) => {
    if ((e.target as HTMLElement).closest('button, input')) return;
    activateSession(id);
  });

  return card;
}

function buildSectionHeader(labelKey: string, count?: number): HTMLElement {
  const h = document.createElement('h3');
  h.className = 'mh-section-header';
  h.textContent = count == null ? t(labelKey) : t(labelKey, { n: count });
  return h;
}

function buildSearchInput(value: string, onChange: (value: string) => void, placeholderKey = 'mobile_home_search_placeholder'): HTMLElement {
  const wrap = document.createElement('label');
  wrap.className = 'mh-search';
  const icon = document.createElement('span');
  icon.className = 'mh-search-icon';
  icon.setAttribute('aria-hidden', 'true');
  icon.textContent = '⌕';
  const input = document.createElement('input');
  input.type = 'search';
  input.value = value;
  input.placeholder = t(placeholderKey);
  input.addEventListener('input', () => onChange(input.value));
  wrap.append(icon, input);
  return wrap;
}

function ensureSearchInput(parent: HTMLElement, id: string, value: string, onChange: (value: string) => void, placeholderKey = 'mobile_home_search_placeholder'): void {
  let search = parent.querySelector<HTMLElement>(`#${id}`);
  if (search) return;
  search = buildSearchInput(value, onChange, placeholderKey);
  search.id = id;
  parent.prepend(search);
}

function filteredOrderedIds(query: string): number[] {
  return orderSessions().filter(Boolean).map(s => s.id).filter(id => matchesSearch(id, query));
}

export function renderMobileHome() {
  if (!isMobileViewport()) return;
  const container = document.getElementById('mobile-home');
  if (!container) return;

  ensureSearchInput(container, 'mobile-home-search', mobileHomeSearch, (value) => {
    mobileHomeSearch = value;
    renderMobileHomeResults();
  });
  renderMobileHomeResults();
}

function renderMobileHomeResults(): void {
  if (!isMobileViewport()) return;
  const container = document.getElementById('mobile-home');
  if (!container) return;
  let results = container.querySelector<HTMLElement>('.mh-results');
  if (!results) {
    results = document.createElement('div');
    results.className = 'mh-results';
    container.appendChild(results);
  }
  results.innerHTML = '';

  const allIds = filteredOrderedIds(mobileHomeSearch);
  const pendingIds = allIds.filter(id => getSessionBucket(id) === 'pending');
  const sessionIds = allIds.filter(id => getSessionBucket(id) !== 'pending');

  if (sessions.size === 0) {
    const empty = document.createElement('div');
    empty.className = 'mh-empty';
    empty.textContent = t('no_sessions');
    results.appendChild(empty);
    return;
  }

  const pinnedSection = document.createElement('section');
  pinnedSection.id = 'mobile-home-pinned';
  pinnedSection.className = 'mh-section mh-section--pinned';
  pinnedSection.appendChild(buildSectionHeader('mobile_home_section_pending_count', pendingIds.length));
  if (pendingIds.length === 0) {
    const emptyPending = document.createElement('div');
    emptyPending.className = 'mh-pending-empty';
    emptyPending.textContent = t('approval_tab_empty');
    pinnedSection.appendChild(emptyPending);
  } else {
    for (const id of pendingIds) pinnedSection.appendChild(buildCard(id));
  }
  results.appendChild(pinnedSection);

  const sessionsSection = document.createElement('section');
  sessionsSection.className = 'mh-section mh-section--sessions';
  sessionsSection.appendChild(buildSectionHeader('mobile_home_section_sessions'));
  if (sessionIds.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'mh-empty mh-empty--compact';
    empty.textContent = t('mobile_home_search_empty');
    sessionsSection.appendChild(empty);
  } else {
    for (const id of sessionIds) sessionsSection.appendChild(buildCard(id));
  }
  results.appendChild(sessionsSection);
}

export function updateMobileHomeCard(id: number) {
  if (!isMobileViewport()) return;
  const container = document.getElementById('mobile-home');
  if (!container) return;
  const existing = container.querySelector<HTMLElement>(`[data-session-id="${id}"]`);
  if (!existing || !sessions.has(id) || !matchesSearch(id, mobileHomeSearch)) {
    renderMobileHome();
    return;
  }
  const newBucket = getSessionBucket(id);
  const existingBucket = Array.from(existing.classList)
    .find(c => c.startsWith('mh-card--') || c.startsWith('mh-session-row--'))
    ?.replace('mh-card--', '')
    ?.replace('mh-session-row--', '') as SessionBucket | undefined;
  if (existingBucket !== newBucket) {
    renderMobileHome();
    return;
  }
  existing.replaceWith(buildCard(id));
}

export function renderMobileSessionDrawer() {
  if (!isMobileViewport()) return;
  const root = document.getElementById('mobile-drawer-content');
  if (!root) return;
  root.hidden = false;
  ensureSearchInput(root, 'mobile-drawer-search', mobileDrawerSearch, (value) => {
    mobileDrawerSearch = value;
    renderMobileDrawerResults();
  }, 'mobile_drawer_search_placeholder');
  renderMobileDrawerResults();
}

function renderMobileDrawerResults(): void {
  if (!isMobileViewport()) return;
  const root = document.getElementById('mobile-drawer-content');
  if (!root) return;
  let body = root.querySelector<HTMLElement>('.mobile-drawer-body');
  if (!body) {
    body = document.createElement('div');
    body.className = 'mobile-drawer-body';
    root.appendChild(body);
  }
  body.innerHTML = '';

  const ids = filteredOrderedIds(mobileDrawerSearch);
  const pendingIds = ids.filter(id => getSessionBucket(id) === 'pending');
  const sessionIds = ids.filter(id => getSessionBucket(id) !== 'pending');

  const pending = document.createElement('section');
  pending.className = 'mobile-drawer-section';
  pending.appendChild(buildSectionHeader('mobile_home_section_pending_count', pendingIds.length));
  for (const id of pendingIds) pending.appendChild(buildSessionRow(id, true));
  body.appendChild(pending);

  const list = document.createElement('section');
  list.className = 'mobile-drawer-section';
  list.appendChild(buildSectionHeader('mobile_home_section_sessions'));
  if (ids.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'mh-empty mh-empty--compact';
    empty.textContent = t('mobile_home_search_empty');
    list.appendChild(empty);
  } else {
    for (const id of sessionIds) list.appendChild(buildSessionRow(id, true));
  }
  body.appendChild(list);

  const actions = document.createElement('section');
  actions.className = 'mobile-drawer-section mobile-drawer-actions';
  const home = document.createElement('button');
  home.type = 'button';
  home.className = 'mobile-drawer-action';
  home.textContent = t('mobile_back');
  home.addEventListener('click', () => {
    set_activeSessionId(null);
    (window as any).closeMobileSessionDrawer?.();
    (window as any).syncMobileLayoutState?.();
    renderMobileHome();
  });
  const spawn = document.createElement('button');
  spawn.type = 'button';
  spawn.className = 'mobile-drawer-action';
  spawn.textContent = t('mobile_new_session');
  spawn.addEventListener('click', () => {
    document.getElementById('new-session-btn')?.click();
  });
  const expose = document.createElement('button');
  expose.type = 'button';
  expose.className = 'mobile-drawer-action';
  expose.textContent = `🌐 ${t('expose_btn')}`;
  expose.addEventListener('click', () => {
    (window as any).closeMobileSessionDrawer?.();
    document.getElementById('expose-btn')?.click();
  });
  const shutdown = document.createElement('button');
  shutdown.type = 'button';
  shutdown.className = 'mobile-drawer-action';
  shutdown.textContent = `⏻ ${t('shutdown_tooltip')}`;
  shutdown.addEventListener('click', () => {
    (window as any).closeMobileSessionDrawer?.();
    document.getElementById('shutdown-btn')?.click();
  });
  const settings = document.createElement('button');
  settings.type = 'button';
  settings.className = 'mobile-drawer-action';
  settings.textContent = t('mobile_drawer_settings_server');
  settings.addEventListener('click', () => {
    (window as any).closeMobileSessionDrawer?.();
    document.getElementById('settings-btn')?.click();
  });
  const server = document.createElement('button');
  server.type = 'button';
  server.className = 'mobile-drawer-action';
  server.textContent = t('server_btn');
  server.addEventListener('click', () => {
    (window as any).closeMobileSessionDrawer?.();
    document.getElementById('server-btn')?.click();
  });
  actions.append(home, spawn, expose, shutdown, settings, server);
  body.appendChild(actions);
}

function escapeHtml(str: string): string {
  return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

window.addEventListener('approval-queue-updated', () => {
  if (!isMobileViewport()) return;
  renderMobileHome();
  renderMobileSessionDrawer();
});

window.renderMobileHome = renderMobileHome;
window.updateMobileHomeCard = updateMobileHomeCard;
(window as any).renderMobileSessionDrawer = renderMobileSessionDrawer;
