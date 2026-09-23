// P-11: Ctrl+K で開くセッション横断検索パレット。
// 既存 /api/session-search を UI の主導線に引き上げ、検索条件・履歴は端末ローカルに
// 保存する。検索本文そのものは Hub 外へ送らない。
//
// C3（plan_ux-notify-palette-review_c3_palette.md）: Ctrl+K をコマンドパレットとしても
// 使えるようにした。結果の上に「次の承認待ちへ」等のコマンド区画を描き、↑↓ + Enter で
// コマンド・検索結果のどちらも選べる。コマンドの定義・絞り込み・選択位置の計算は
// command-palette-store.ts（DOM なし）に切り出している。
import { t } from '../i18n.js';
import { token, showToast } from './util.js';
import { activeSessionId, orderSessions, sessions } from './state.js';
import { activateSession } from './session-list.js';
import { scrollTerminalToSearchMatch } from './terminal.js';
import { openHistoryViewer } from './history-viewer.js';
import { pendingSessionIds } from './approval-queue-tab.js';
import { openSpawnPanelIfClosed } from './spawn-panel.js';
import { openSettingsSection, setActiveTab } from './settings.js';
import { openShortcutHelp } from './shortcut-help.js';
import {
  type CommandPaletteCommandId,
  type CommandPaletteContext,
  filterCommandPaletteItems,
  listCommandPaletteCommands,
  nextCommandPaletteSelectionIndex,
  pickNextPendingApprovalId,
  pickNextStandbySessionId,
} from './command-palette-store.js';

const HISTORY_KEY = 'many-ai-cli.session-search.history';
const PINNED_KEY = 'many-ai-cli.session-search.pinned';
const DEBOUNCE_MS = 300;
const PAGE_SIZE = 20;
const RESULT_LIMIT = 100;

type SearchResult = {
  session_id?: number;
  provider?: string;
  cwd?: string;
  branch?: string;
  state?: string;
  started_at?: string;
  ts?: string;
  text?: string;
  snippet?: string;
  recent?: boolean;
};

interface RenderableCommand {
  id: CommandPaletteCommandId;
  label: string;
  keywords: readonly string[];
  enabled: boolean;
  disabledReasonKey: string | null;
}

let root: HTMLElement | null = null;
let input: HTMLInputElement | null = null;
let commandsEl: HTMLElement | null = null;
let resultsEl: HTMLElement | null = null;
let contextEl: HTMLElement | null = null;
let providerEl: HTMLSelectElement | null = null;
let labelEl: HTMLSelectElement | null = null;
let periodEl: HTMLSelectElement | null = null;
let statusEl: HTMLElement | null = null;
let debounceTimer: number | null = null;
let requestID = 0;
let page = 0;
let rawResults: SearchResult[] = [];
let currentCommandItems: RenderableCommand[] = [];
let selectedIndex = 0;

function readQueries(key: string): string[] {
  try {
    const value = JSON.parse(localStorage.getItem(key) || '[]');
    return Array.isArray(value) ? value.filter((item) => typeof item === 'string' && item.trim()) : [];
  } catch (_) {
    return [];
  }
}

function writeQueries(key: string, values: string[]) {
  try { localStorage.setItem(key, JSON.stringify(values.slice(0, 12))); } catch (_) {}
}

function saveRecentQuery(query: string) {
  const normalized = query.trim();
  if (!normalized) return;
  writeQueries(HISTORY_KEY, [normalized, ...readQueries(HISTORY_KEY).filter((item) => item !== normalized)]);
}

function isPinned(query: string): boolean {
  return readQueries(PINNED_KEY).includes(query);
}

function togglePinned(query: string) {
  const values = readQueries(PINNED_KEY);
  writeQueries(PINNED_KEY, values.includes(query) ? values.filter((item) => item !== query) : [query, ...values]);
  renderSuggestions();
}

function sessionLabel(id: number | undefined): string {
  const session: any = id ? sessions.get(id) : undefined;
  return String(session?.label || session?.auto_title || '');
}

function sessionTitle(result: SearchResult): string {
  const label = sessionLabel(result.session_id);
  if (label) return label;
  const cwd = String(result.cwd || '').replace(/\\/g, '/');
  return cwd.split('/').filter(Boolean).pop() || `Session #${result.session_id || '?'}`;
}

function formatTime(value: string | undefined): string {
  const at = Date.parse(String(value || ''));
  if (Number.isNaN(at)) return '';
  const date = new Date(at);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function makeRecentResults(): SearchResult[] {
  return Array.from(sessions.values())
    .sort((a: any, b: any) => Date.parse(String(b.started_at || '')) - Date.parse(String(a.started_at || '')))
    .slice(0, RESULT_LIMIT)
    .map((session: any) => ({
      session_id: session.id,
      provider: session.provider,
      cwd: session.cwd,
      branch: session.branch,
      state: session.state,
      started_at: session.started_at,
      text: session.label || session.auto_title || '',
      recent: true,
    }));
}

function refreshFilterOptions() {
  if (!providerEl || !labelEl) return;
  const selectedProvider = providerEl.value;
  const selectedLabel = labelEl.value;
  const providers = new Set<string>();
  const labels = new Set<string>();
  sessions.forEach((session: any) => {
    if (session.provider) providers.add(String(session.provider));
    const label = String(session.label || session.auto_title || '');
    if (label) labels.add(label);
  });
  providerEl.replaceChildren(new Option(t('session_search_provider_all'), ''), ...Array.from(providers).sort().map((value) => new Option(value, value)));
  labelEl.replaceChildren(new Option(t('session_search_label_all'), ''), ...Array.from(labels).sort().map((value) => new Option(value, value)));
  providerEl.value = providers.has(selectedProvider) ? selectedProvider : '';
  labelEl.value = labels.has(selectedLabel) ? selectedLabel : '';
}

function periodCutoff(): number {
  const days = Number(periodEl?.value || '0');
  return days > 0 ? Date.now() - days * 24 * 60 * 60 * 1000 : 0;
}

function filteredResults(): SearchResult[] {
  const provider = providerEl?.value || '';
  const label = labelEl?.value || '';
  const cutoff = periodCutoff();
  return rawResults.filter((result) => {
    if (provider && result.provider !== provider) return false;
    if (label && sessionLabel(result.session_id) !== label) return false;
    if (cutoff) {
      const timestamp = Date.parse(String(result.ts || result.started_at || ''));
      if (Number.isNaN(timestamp) || timestamp < cutoff) return false;
    }
    return true;
  });
}

function setStatus(message: string) {
  if (statusEl) statusEl.textContent = message;
}

// ── コマンドパレット: コンテキストの組み立て・絞り込み・選択位置 ───────────

function buildContext(): CommandPaletteContext {
  const active = activeSessionId !== null ? sessions.get(activeSessionId) : undefined;
  const hasWorkdir = !!(active && (active.git_root || active.cwd));
  return {
    pendingApprovalIds: pendingSessionIds(),
    standbySessionIds: orderSessions().filter((s: any) => (s.state || 'standby') === 'standby').map((s: any) => s.id),
    activeSessionId,
    activeSessionHasWorkdir: hasWorkdir,
  };
}

function buildCommandItems(query: string): RenderableCommand[] {
  const ctx = buildContext();
  const items: RenderableCommand[] = listCommandPaletteCommands(ctx).map(({ def, enabled, disabledReasonKey }) => ({
    id: def.id,
    label: t(def.labelKey),
    keywords: def.keywords,
    enabled,
    disabledReasonKey,
  }));
  return filterCommandPaletteItems(items, query);
}

// コマンド・検索結果の両方を横断した「今選ばれている要素」の一覧。DOM を直接見るので
// commandsEl / resultsEl のどちらを再描画した後でも呼び直すだけで整合する。
function selectableEls(): HTMLElement[] {
  const cmds = commandsEl ? Array.from(commandsEl.querySelectorAll<HTMLElement>('.command-palette-command')) : [];
  const results = resultsEl ? Array.from(resultsEl.querySelectorAll<HTMLElement>('.session-search-result')) : [];
  return [...cmds, ...results];
}

function updateSelectionClasses() {
  const els = selectableEls();
  if (els.length === 0) { selectedIndex = 0; return; }
  selectedIndex = ((selectedIndex % els.length) + els.length) % els.length;
  els.forEach((el, idx) => {
    const isSelected = idx === selectedIndex;
    el.classList.toggle('is-selected', isSelected);
    el.setAttribute('aria-selected', String(isSelected));
    if (isSelected) el.scrollIntoView({ block: 'nearest' });
  });
}

function moveSelection(direction: 1 | -1) {
  const count = selectableEls().length;
  if (count === 0) return;
  selectedIndex = nextCommandPaletteSelectionIndex(count, selectedIndex, direction);
  updateSelectionClasses();
}

function activateSessionAndShowTerminal(id: number) {
  activateSession(id);
  const termTab = document.querySelector('#unified-tab-bar .view-tab[data-tab="terminal"]') as HTMLButtonElement | null;
  termTab?.click();
}

function runCommand(item: RenderableCommand) {
  if (!item.enabled) return;
  const ctx = buildContext();
  const orderedIds = orderSessions().map((s: any) => s.id);
  switch (item.id) {
    case 'next-pending-approval': {
      const nextId = pickNextPendingApprovalId(orderedIds, ctx.pendingApprovalIds, ctx.activeSessionId);
      if (nextId == null) return;
      closePalette();
      activateSessionAndShowTerminal(nextId);
      return;
    }
    case 'next-standby': {
      const nextId = pickNextStandbySessionId(orderedIds, ctx.standbySessionIds, ctx.activeSessionId);
      if (nextId == null) return;
      closePalette();
      activateSessionAndShowTerminal(nextId);
      return;
    }
    case 'new-session':
      closePalette();
      openSpawnPanelIfClosed();
      return;
    case 'open-review':
      if (ctx.activeSessionId == null) return;
      closePalette();
      setActiveTab(ctx.activeSessionId, 'review');
      return;
    case 'open-files':
      if (ctx.activeSessionId == null) return;
      closePalette();
      setActiveTab(ctx.activeSessionId, 'files');
      return;
    case 'open-git':
      if (ctx.activeSessionId == null) return;
      closePalette();
      setActiveTab(ctx.activeSessionId, 'git');
      return;
    case 'open-settings-notify':
      closePalette();
      openSettingsSection('notify-sound');
      return;
    case 'open-settings-approval':
      closePalette();
      openSettingsSection('approval-hub');
      return;
    case 'show-shortcuts':
      closePalette();
      openShortcutHelp();
      return;
  }
}

function renderCommands() {
  if (!commandsEl) return;
  currentCommandItems = buildCommandItems(String(input?.value || ''));
  commandsEl.replaceChildren();
  commandsEl.hidden = currentCommandItems.length === 0;
  for (const item of currentCommandItems) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'command-palette-command';
    btn.setAttribute('role', 'option');
    btn.setAttribute('aria-selected', 'false');
    if (!item.enabled) btn.classList.add('command-palette-command--disabled');
    const label = document.createElement('span');
    label.className = 'command-palette-command-label';
    label.textContent = item.label;
    btn.appendChild(label);
    if (!item.enabled && item.disabledReasonKey) {
      const reason = document.createElement('span');
      reason.className = 'command-palette-command-reason';
      reason.textContent = t(item.disabledReasonKey);
      btn.appendChild(reason);
    }
    btn.addEventListener('mouseenter', () => {
      const idx = selectableEls().indexOf(btn);
      if (idx !== -1) { selectedIndex = idx; updateSelectionClasses(); }
    });
    btn.addEventListener('click', () => runCommand(item));
    commandsEl.appendChild(btn);
  }
  updateSelectionClasses();
}

/** ↑↓ + Enter で選ばれている項目を実行する。実行できたら true。 */
function runSelected(): boolean {
  const els = selectableEls();
  if (els.length === 0) return false;
  selectedIndex = ((selectedIndex % els.length) + els.length) % els.length;
  if (selectedIndex < currentCommandItems.length) {
    runCommand(currentCommandItems[selectedIndex]);
    return true;
  }
  const shown = filteredResults().slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE);
  const result = shown[selectedIndex - currentCommandItems.length];
  if (!result) return false;
  openResult(result);
  return true;
}

function renderResults() {
  if (!resultsEl) return;
  selectedIndex = 0;
  const results = filteredResults();
  const pageCount = Math.max(1, Math.ceil(results.length / PAGE_SIZE));
  page = Math.min(page, pageCount - 1);
  resultsEl.replaceChildren();
  if (results.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'session-search-empty';
    empty.textContent = rawResults.length ? t('session_search_empty_filtered') : t('session_search_empty_none');
    resultsEl.appendChild(empty);
    updateSelectionClasses();
    return;
  }
  for (const result of results.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE)) {
    const item = document.createElement('button');
    item.type = 'button';
    item.className = 'session-search-result';
    item.setAttribute('role', 'option');
    item.setAttribute('aria-selected', 'false');
    const meta = document.createElement('span');
    meta.className = 'session-search-result-meta';
    meta.textContent = `${result.provider || 'ai'} · ${sessionTitle(result)}${result.branch ? ` · ${result.branch}` : ''}${result.state ? ` · ${result.state}` : ''}`;
    const body = document.createElement('span');
    body.className = 'session-search-result-body';
    body.textContent = result.recent ? (result.text || t('session_search_recent_session')) : (result.snippet || result.text || t('session_search_no_body'));
    const time = document.createElement('span');
    time.className = 'session-search-result-time';
    time.textContent = formatTime(result.ts || result.started_at);
    item.append(meta, body, time);
    item.addEventListener('mouseenter', () => {
      const idx = selectableEls().indexOf(item);
      if (idx !== -1) { selectedIndex = idx; updateSelectionClasses(); }
    });
    item.addEventListener('click', () => openResult(result));
    resultsEl.appendChild(item);
  }
  if (pageCount > 1) {
    const pager = document.createElement('div');
    pager.className = 'session-search-pager';
    const previous = document.createElement('button');
    previous.type = 'button';
    previous.textContent = t('session_search_prev_page');
    previous.disabled = page === 0;
    previous.onclick = () => { page--; renderResults(); };
    const label = document.createElement('span');
    label.textContent = t('session_search_page_indicator', { page: page + 1, pageCount, limit: RESULT_LIMIT });
    const next = document.createElement('button');
    next.type = 'button';
    next.textContent = t('session_search_next_page');
    next.disabled = page >= pageCount - 1;
    next.onclick = () => { page++; renderResults(); };
    pager.append(previous, label, next);
    resultsEl.appendChild(pager);
  }
  updateSelectionClasses();
}

function showContext(result: SearchResult, note: string) {
  if (!contextEl) return;
  contextEl.hidden = false;
  contextEl.replaceChildren();
  const title = document.createElement('strong');
  title.textContent = `${result.provider || 'ai'} · ${sessionTitle(result)}`;
  const text = document.createElement('pre');
  text.textContent = result.snippet || result.text || t('session_search_no_saved_body');
  const hint = document.createElement('div');
  hint.className = 'session-search-context-hint';
  hint.textContent = note;
  contextEl.append(title, text, hint);
}

function openResult(result: SearchResult) {
  const sid = Number(result.session_id || 0);
  if (!sid || !sessions.has(sid)) {
    showContext(result, t('session_search_result_disconnected_hint'));
    return;
  }
  activateSession(sid);
  const query = String(input?.value || '').trim();
  requestAnimationFrame(() => {
    if (query && scrollTerminalToSearchMatch(sid, query)) {
      showToast(t('session_search_scrolled_toast'));
      closePalette();
      return;
    }
    openHistoryViewer(sid, { offset: -1 });
    showContext(result, t('session_search_scrollback_hint'));
  });
}

async function search() {
  const query = String(input?.value || '').trim();
  page = 0;
  contextEl && (contextEl.hidden = true);
  if (query.length < 3) {
    rawResults = makeRecentResults();
    setStatus(query ? t('session_search_status_too_short') : t('session_search_status_idle'));
    renderCommands();
    renderResults();
    renderSuggestions();
    return;
  }
  const currentRequest = ++requestID;
  setStatus(t('session_search_status_searching'));
  try {
    const params = new URLSearchParams({ token, q: query, limit: String(RESULT_LIMIT) });
    const response = await fetch(`/api/session-search?${params.toString()}`);
    if (!response.ok) throw new Error(`session-search ${response.status}`);
    const data = await response.json();
    if (currentRequest !== requestID) return;
    rawResults = Array.isArray(data.results) ? data.results : [];
    saveRecentQuery(query);
    setStatus(t('session_search_status_results_count', { count: rawResults.length, limit: RESULT_LIMIT }));
    renderCommands();
    renderResults();
    renderSuggestions();
  } catch (error) {
    if (currentRequest !== requestID) return;
    rawResults = [];
    setStatus(t('session_search_status_failed'));
    renderCommands();
    renderResults();
    console.warn('[session-search-palette] search failed', error);
  }
}

function renderSuggestions() {
  const host = root?.querySelector<HTMLElement>('.session-search-suggestions');
  if (!host) return;
  host.replaceChildren();
  const groups: Array<[string, string[]]> = [
    [t('session_search_pinned_group'), readQueries(PINNED_KEY)],
    [t('session_search_recent_group'), readQueries(HISTORY_KEY)],
  ];
  for (const [title, queries] of groups) {
    if (!queries.length) continue;
    const row = document.createElement('div');
    row.className = 'session-search-suggestion-group';
    const heading = document.createElement('span');
    heading.textContent = title;
    row.appendChild(heading);
    for (const query of queries) {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'session-search-query';
      button.textContent = query;
      button.onclick = () => {
        if (!input) return;
        input.value = query;
        search();
      };
      row.appendChild(button);
    }
    host.appendChild(row);
  }
}

function closePalette() {
  if (!root) return;
  root.hidden = true;
  selectedIndex = 0;
  if (debounceTimer) window.clearTimeout(debounceTimer);
}

function ensurePalette(): HTMLElement | null {
  if (root) return root;
  const overlay = document.createElement('div');
  overlay.id = 'session-search-palette';
  overlay.classList.add('aac-wheel-overlay');
  overlay.hidden = true;
  overlay.setAttribute('role', 'dialog');
  overlay.setAttribute('aria-modal', 'true');
  overlay.setAttribute('aria-label', t('session_search_aria_label'));
  overlay.addEventListener('mousedown', (event) => { if (event.target === overlay) closePalette(); });

  const dialog = document.createElement('section');
  dialog.className = 'session-search-dialog';
  const header = document.createElement('div');
  header.className = 'session-search-header';
  input = document.createElement('input');
  input.type = 'search';
  input.placeholder = t('session_search_placeholder');
  input.autocomplete = 'off';
  input.addEventListener('input', () => {
    if (debounceTimer) window.clearTimeout(debounceTimer);
    debounceTimer = window.setTimeout(search, DEBOUNCE_MS);
  });
  input.addEventListener('keydown', (event) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      moveSelection(event.key === 'ArrowDown' ? 1 : -1);
      return;
    }
    if (event.key !== 'Enter' || !input) return;
    event.preventDefault();
    if (runSelected()) return;
    search();
  });
  const pin = document.createElement('button');
  pin.type = 'button';
  pin.className = 'session-search-pin';
  pin.title = t('session_search_pin_title');
  pin.textContent = '☆';
  pin.onclick = () => {
    const query = String(input?.value || '').trim();
    if (!query) return;
    togglePinned(query);
    pin.textContent = isPinned(query) ? '★' : '☆';
  };
  const close = document.createElement('button');
  close.type = 'button';
  close.className = 'session-search-close';
  close.textContent = 'Esc';
  close.onclick = closePalette;
  header.append(input, pin, close);

  const filters = document.createElement('div');
  filters.className = 'session-search-filters';
  providerEl = document.createElement('select');
  labelEl = document.createElement('select');
  periodEl = document.createElement('select');
  periodEl.append(
    new Option(t('session_search_period_all'), '0'),
    new Option(t('session_search_period_24h'), '1'),
    new Option(t('session_search_period_7d'), '7'),
    new Option(t('session_search_period_30d'), '30'),
  );
  for (const filter of [providerEl, labelEl, periodEl]) filter.addEventListener('change', () => { page = 0; renderResults(); });
  filters.append(providerEl, labelEl, periodEl);

  const suggestions = document.createElement('div');
  suggestions.className = 'session-search-suggestions';
  commandsEl = document.createElement('div');
  commandsEl.className = 'command-palette-commands';
  statusEl = document.createElement('div');
  statusEl.className = 'session-search-status';
  resultsEl = document.createElement('div');
  resultsEl.className = 'session-search-results';
  contextEl = document.createElement('aside');
  contextEl.className = 'session-search-context';
  contextEl.hidden = true;
  dialog.append(header, filters, suggestions, commandsEl, statusEl, resultsEl, contextEl);
  overlay.appendChild(dialog);
  document.body.appendChild(overlay);
  root = overlay;
  return root;
}

export function openSessionSearchPalette() {
  const palette = ensurePalette();
  if (!palette) return;
  palette.hidden = false;
  refreshFilterOptions();
  rawResults = makeRecentResults();
  page = 0;
  selectedIndex = 0;
  setStatus(t('session_search_status_idle'));
  renderSuggestions();
  renderCommands();
  renderResults();
  requestAnimationFrame(() => input?.focus());
}

export function initSessionSearchPalette() {
  document.addEventListener('keydown', (event) => {
    const paletteOpen = !!root && !root.hidden;
    if (event.key === 'Escape' && paletteOpen) {
      event.preventDefault();
      closePalette();
      return;
    }
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      openSessionSearchPalette();
    }
  }, true);
}
