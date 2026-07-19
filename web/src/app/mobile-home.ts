// mobile-home.ts
// スマホホーム画面（#mobile-home）と左ドロワーの描画モジュール。
// PC には一切副作用を与えない。全エントリーポイントは isMobileViewport() で early return する。

import { t } from '../i18n.js';
import { orderSessions, sessions, approvalVisibleCache, multiQuestionVisibleCache, activeSessionId, set_activeSessionId } from './state.js';
import { activateSession, providerIconHtml } from './session-list.js';
import { sessionTitle } from './approval-queue-tab.js';
import { openServerModal } from './server-modal.js';
import { escapeHtml } from './util.js';

const mobileMql = (typeof window !== 'undefined' && typeof window.matchMedia === 'function')
  ? window.matchMedia('(max-width: 720px)')
  : null;
function isMobileViewport(): boolean { return !!mobileMql?.matches; }

type SessionBucket = 'pending' | 'running' | 'waiting' | 'error';

let mobileHomeSearch = '';
let mobileDrawerSearch = '';
// This is deliberately not persisted: the default must remain "all sessions" every
// time a mobile monitoring home is opened.
let mobilePendingOnly = false;

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

function projectName(id: number): string {
  const cwd = sessions.get(id)?.cwd || '';
  const name = cwd.replace(/\\/g, '/').split('/').filter(Boolean).pop();
  return name || t('mobile_project_unknown');
}

function providerModelText(id: number): string {
  const s = sessions.get(id);
  const provider = String(s?.provider || 'unknown');
  const model = String(s?.model || '').trim();
  return model ? `${provider} · ${model}` : provider;
}

// P-25 monitoring home deliberately has no direct Yes/No controls. A pending
// row opens the approval sheet, leaving P-09's high-risk confirmation gate as
// the single approval path once it is introduced.
function buildMonitoringRow(id: number): HTMLElement {
  const bucket = getSessionBucket(id);
  const s = sessions.get(id);
  const row = document.createElement('button');
  row.type = 'button';
  row.className = `mh-monitor-row mh-monitor-row--${bucket}${s?.color ? ` mh-monitor-row--${s.color}` : ''}`;
  row.dataset.sessionId = String(id);
  row.classList.toggle('is-active', id === activeSessionId);
  row.setAttribute('aria-label', `${sessionTitle(s)}, ${providerModelText(id)}, ${statusChipText(bucket)}`);

  const main = document.createElement('div');
  main.className = 'mh-monitor-main';
  const title = document.createElement('div');
  title.className = 'mh-monitor-title';
  title.innerHTML = buildTitleHtml(id, 16);
  if (s?.note) {
    const note = document.createElement('span');
    note.className = 'mh-note-indicator';
    note.textContent = '•';
    note.title = s.note;
    note.setAttribute('aria-label', s.note);
    title.appendChild(note);
  }
  const meta = document.createElement('div');
  meta.className = 'mh-monitor-meta';
  meta.textContent = providerModelText(id);
  main.append(title, meta);

  const status = document.createElement('div');
  status.className = 'mh-monitor-status';
  if (bucket === 'pending') {
    const approval = document.createElement('span');
    approval.className = 'mh-approval-badge';
    approval.textContent = t('mobile_state_pending');
    status.appendChild(approval);
  }
  const chip = document.createElement('span');
  chip.className = `mh-state-chip mh-state-chip--${bucket}`;
  chip.textContent = statusChipText(bucket);
  status.appendChild(chip);
  row.append(main, status);

  row.addEventListener('click', () => {
    if (bucket === 'pending') (window as any).openMobileApprovalSheetForSession?.(id);
    else activateSession(id);
  });
  return row;
}

function buildSectionHeader(labelKey: string, count?: number): HTMLElement {
  const h = document.createElement('h3');
  h.className = 'mh-section-header';
  h.textContent = count == null ? t(labelKey) : t(labelKey, { n: count });
  return h;
}

// C7: unified-tab-bar は常時表示のバー行を廃止し(モバイルはハンバーガー/展開アイコンのみを
// フロート表示)、ビュー切り替え自体はドロワー内のこのセクションへ移した。既存の .view-tab
// ボタンをそのまま click() することで、承認バッジ等の状態管理ロジックは重複させない。
function buildViewSwitchSection(): HTMLElement | null {
  if (!document.body.classList.contains('mobile-has-session')) return null;
  const bar = document.getElementById('unified-tab-bar');
  const tabs = bar ? Array.from(bar.querySelectorAll<HTMLButtonElement>('.view-tab')) : [];
  if (tabs.length === 0) return null;

  const section = document.createElement('section');
  section.className = 'mobile-drawer-section mobile-drawer-views';
  section.appendChild(buildSectionHeader('mobile_drawer_section_views'));
  const grid = document.createElement('div');
  grid.className = 'mobile-drawer-views-grid';
  for (const tab of tabs) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'mobile-drawer-view-btn';
    if (tab.classList.contains('active')) btn.classList.add('active');
    btn.innerHTML = tab.innerHTML;
    btn.addEventListener('click', () => {
      tab.click();
      (window as any).closeMobileSessionDrawer?.();
    });
    grid.appendChild(btn);
  }
  section.appendChild(grid);
  return section;
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

function filteredOrderedIds(query: string, pendingOnly = false): number[] {
  return orderSessions()
    .filter(Boolean)
    .map(s => s.id)
    .filter(id => matchesSearch(id, query))
    .filter(id => !pendingOnly || getSessionBucket(id) === 'pending');
}

function ensurePendingOnlyToggle(parent: HTMLElement): void {
  let toggle = parent.querySelector<HTMLButtonElement>('#mobile-pending-only-toggle');
  if (!toggle) {
    toggle = document.createElement('button');
    toggle.id = 'mobile-pending-only-toggle';
    toggle.type = 'button';
    toggle.className = 'mh-pending-filter';
    toggle.addEventListener('click', () => {
      mobilePendingOnly = !mobilePendingOnly;
      renderMobileHomeResults();
    });
    parent.appendChild(toggle);
  }
  toggle.setAttribute('aria-pressed', String(mobilePendingOnly));
  toggle.textContent = t('mobile_pending_only');
  toggle.classList.toggle('is-active', mobilePendingOnly);
}

function appendProjectGroup(parent: HTMLElement, name: string, ids: number[]): void {
  const section = document.createElement('section');
  section.className = 'mh-section mh-project-group';
  const header = document.createElement('h3');
  header.className = 'mh-project-header';
  header.textContent = name;
  section.appendChild(header);
  for (const id of ids) section.appendChild(buildMonitoringRow(id));
  parent.appendChild(section);
}

export function renderMobileHome() {
  if (!isMobileViewport()) return;
  const container = document.getElementById('mobile-home');
  if (!container) return;

  ensureSearchInput(container, 'mobile-home-search', mobileHomeSearch, (value) => {
    mobileHomeSearch = value;
    renderMobileHomeResults();
  });
  ensurePendingOnlyToggle(container);
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

  const allIds = filteredOrderedIds(mobileHomeSearch, mobilePendingOnly);
  const pendingCount = filteredOrderedIds(mobileHomeSearch).filter(id => getSessionBucket(id) === 'pending').length;

  if (sessions.size === 0) {
    const empty = document.createElement('div');
    empty.className = 'mh-empty';
    empty.textContent = t('no_sessions');
    results.appendChild(empty);
    return;
  }

  const summary = document.createElement('div');
  summary.className = 'mh-monitor-summary';
  summary.textContent = t('mobile_home_section_pending_count', { n: pendingCount });
  results.appendChild(summary);

  const pinnedIds = allIds.filter(id => !!sessions.get(id)?.pinned);
  if (pinnedIds.length > 0) {
    const pinned = document.createElement('section');
    pinned.id = 'mobile-home-pinned';
    pinned.className = 'mh-section mh-section--pinned';
    pinned.appendChild(buildSectionHeader('mobile_pinned_sessions'));
    for (const id of pinnedIds) pinned.appendChild(buildMonitoringRow(id));
    results.appendChild(pinned);
  }

  const remainingIds = allIds.filter(id => !sessions.get(id)?.pinned);
  if (remainingIds.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'mh-empty mh-empty--compact';
    empty.textContent = t('mobile_home_search_empty');
    results.appendChild(empty);
  } else {
    const groups = new Map<string, number[]>();
    for (const id of remainingIds) {
      const name = projectName(id);
      const group = groups.get(name) || [];
      group.push(id);
      groups.set(name, group);
    }
    groups.forEach((ids, name) => appendProjectGroup(results, name, ids));
  }
}

export function updateMobileHomeCard(id: number) {
  if (!isMobileViewport()) return;
  const container = document.getElementById('mobile-home');
  if (!container) return;
  // A session can cross the pinned/project/filter boundaries, so a complete
  // monitoring-list refresh is safer than replacing one row in place.
  renderMobileHome();
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

  const viewSection = buildViewSwitchSection();
  if (viewSection) body.appendChild(viewSection);

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
    openServerModal();
  });
  actions.append(home, spawn, expose, shutdown, settings, server);
  body.appendChild(actions);
}

window.addEventListener('approval-queue-updated', () => {
  if (!isMobileViewport()) return;
  renderMobileHome();
  renderMobileSessionDrawer();
});

window.renderMobileHome = renderMobileHome;
window.updateMobileHomeCard = updateMobileHomeCard;
(window as any).renderMobileSessionDrawer = renderMobileSessionDrawer;
