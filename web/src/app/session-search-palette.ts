// P-11: Ctrl+K で開くセッション横断検索パレット。
// 既存 /api/session-search を UI の主導線に引き上げ、検索条件・履歴は端末ローカルに
// 保存する。検索本文そのものは Hub 外へ送らない。
import { token, showToast } from './util.js';
import { sessions } from './state.js';
import { activateSession } from './session-list.js';
import { scrollTerminalToSearchMatch } from './terminal.js';
import { openHistoryViewer } from './history-viewer.js';

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

let root: HTMLElement | null = null;
let input: HTMLInputElement | null = null;
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
  providerEl.replaceChildren(new Option('すべての provider', ''), ...Array.from(providers).sort().map((value) => new Option(value, value)));
  labelEl.replaceChildren(new Option('すべてのラベル', ''), ...Array.from(labels).sort().map((value) => new Option(value, value)));
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

function renderResults() {
  if (!resultsEl) return;
  const results = filteredResults();
  const pageCount = Math.max(1, Math.ceil(results.length / PAGE_SIZE));
  page = Math.min(page, pageCount - 1);
  resultsEl.replaceChildren();
  if (results.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'session-search-empty';
    empty.textContent = rawResults.length ? '現在のフィルタ条件では一致しません' : '一致するセッションはありません';
    resultsEl.appendChild(empty);
    return;
  }
  for (const result of results.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE)) {
    const item = document.createElement('button');
    item.type = 'button';
    item.className = 'session-search-result';
    const meta = document.createElement('span');
    meta.className = 'session-search-result-meta';
    meta.textContent = `${result.provider || 'ai'} · ${sessionTitle(result)}${result.branch ? ` · ${result.branch}` : ''}${result.state ? ` · ${result.state}` : ''}`;
    const body = document.createElement('span');
    body.className = 'session-search-result-body';
    body.textContent = result.recent ? (result.text || '最近のセッション') : (result.snippet || result.text || '本文なし');
    const time = document.createElement('span');
    time.className = 'session-search-result-time';
    time.textContent = formatTime(result.ts || result.started_at);
    item.append(meta, body, time);
    item.addEventListener('click', () => openResult(result));
    resultsEl.appendChild(item);
  }
  if (pageCount > 1) {
    const pager = document.createElement('div');
    pager.className = 'session-search-pager';
    const previous = document.createElement('button');
    previous.type = 'button';
    previous.textContent = '← 前';
    previous.disabled = page === 0;
    previous.onclick = () => { page--; renderResults(); };
    const label = document.createElement('span');
    label.textContent = `${page + 1} / ${pageCount}（最大 ${RESULT_LIMIT} 件）`;
    const next = document.createElement('button');
    next.type = 'button';
    next.textContent = '次 →';
    next.disabled = page >= pageCount - 1;
    next.onclick = () => { page++; renderResults(); };
    pager.append(previous, label, next);
    resultsEl.appendChild(pager);
  }
}

function showContext(result: SearchResult, note: string) {
  if (!contextEl) return;
  contextEl.hidden = false;
  contextEl.replaceChildren();
  const title = document.createElement('strong');
  title.textContent = `${result.provider || 'ai'} · ${sessionTitle(result)}`;
  const text = document.createElement('pre');
  text.textContent = result.snippet || result.text || '保存された本文はありません';
  const hint = document.createElement('div');
  hint.className = 'session-search-context-hint';
  hint.textContent = note;
  contextEl.append(title, text, hint);
}

function openResult(result: SearchResult) {
  const sid = Number(result.session_id || 0);
  if (!sid || !sessions.has(sid)) {
    showContext(result, 'このセッションは現在 Hub に接続されていません。検索時点の文脈をここに表示しています。');
    return;
  }
  activateSession(sid);
  const query = String(input?.value || '').trim();
  requestAnimationFrame(() => {
    if (query && scrollTerminalToSearchMatch(sid, query)) {
      showToast('端末内の一致行へ移動しました');
      closePalette();
      return;
    }
    openHistoryViewer(sid, { offset: -1 });
    showContext(result, '現在の scrollback 外のため、末尾付近の過去ログを開きました。');
  });
}

async function search() {
  const query = String(input?.value || '').trim();
  page = 0;
  contextEl && (contextEl.hidden = true);
  if (query.length < 3) {
    rawResults = makeRecentResults();
    setStatus(query ? '3 文字以上で横断検索します。最近のセッションを表示中です。' : '最近のセッション。3 文字以上で横断検索します。');
    renderResults();
    renderSuggestions();
    return;
  }
  const currentRequest = ++requestID;
  setStatus('検索中…');
  try {
    const params = new URLSearchParams({ token, q: query, limit: String(RESULT_LIMIT) });
    const response = await fetch(`/api/session-search?${params.toString()}`);
    if (!response.ok) throw new Error(`session-search ${response.status}`);
    const data = await response.json();
    if (currentRequest !== requestID) return;
    rawResults = Array.isArray(data.results) ? data.results : [];
    saveRecentQuery(query);
    setStatus(`${rawResults.length} 件（最大 ${RESULT_LIMIT} 件）`);
    renderResults();
    renderSuggestions();
  } catch (error) {
    if (currentRequest !== requestID) return;
    rawResults = [];
    setStatus('検索に失敗しました');
    renderResults();
    console.warn('[session-search-palette] search failed', error);
  }
}

function renderSuggestions() {
  const host = root?.querySelector<HTMLElement>('.session-search-suggestions');
  if (!host) return;
  host.replaceChildren();
  const groups: Array<[string, string[]]> = [
    ['ピン留め', readQueries(PINNED_KEY)],
    ['最近の検索', readQueries(HISTORY_KEY)],
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
  if (debounceTimer) window.clearTimeout(debounceTimer);
}

function ensurePalette(): HTMLElement | null {
  if (root) return root;
  const overlay = document.createElement('div');
  overlay.id = 'session-search-palette';
  overlay.hidden = true;
  overlay.setAttribute('role', 'dialog');
  overlay.setAttribute('aria-modal', 'true');
  overlay.setAttribute('aria-label', 'セッション横断検索');
  overlay.addEventListener('mousedown', (event) => { if (event.target === overlay) closePalette(); });

  const dialog = document.createElement('section');
  dialog.className = 'session-search-dialog';
  const header = document.createElement('div');
  header.className = 'session-search-header';
  input = document.createElement('input');
  input.type = 'search';
  input.placeholder = 'セッション横断検索（3文字以上）';
  input.autocomplete = 'off';
  input.addEventListener('input', () => {
    if (debounceTimer) window.clearTimeout(debounceTimer);
    debounceTimer = window.setTimeout(search, DEBOUNCE_MS);
  });
  input.addEventListener('keydown', (event) => {
    if (event.key !== 'Enter' || !input) return;
    event.preventDefault();
    search();
  });
  const pin = document.createElement('button');
  pin.type = 'button';
  pin.className = 'session-search-pin';
  pin.title = '現在の検索語をピン留め';
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
  periodEl.append(new Option('すべての期間', '0'), new Option('24 時間', '1'), new Option('7 日', '7'), new Option('30 日', '30'));
  for (const filter of [providerEl, labelEl, periodEl]) filter.addEventListener('change', () => { page = 0; renderResults(); });
  filters.append(providerEl, labelEl, periodEl);

  const suggestions = document.createElement('div');
  suggestions.className = 'session-search-suggestions';
  statusEl = document.createElement('div');
  statusEl.className = 'session-search-status';
  resultsEl = document.createElement('div');
  resultsEl.className = 'session-search-results';
  contextEl = document.createElement('aside');
  contextEl.className = 'session-search-context';
  contextEl.hidden = true;
  dialog.append(header, filters, suggestions, statusEl, resultsEl, contextEl);
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
  setStatus('最近のセッション。3 文字以上で横断検索します。');
  renderSuggestions();
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
