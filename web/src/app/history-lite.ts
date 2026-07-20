// P-10: deliberately small persisted-session workbench. It does not revive the
// removed chat proxy; all data comes from sessionstore and the existing log API.
import { token, showToast, ti18n } from './util.js';

type Overview = Record<string, any>;
type Message = { id: number; ts: string; role: string; rawText?: string; normalizedText?: string };
let root: HTMLElement | null = null;
let all: Overview[] = [];
let selected: Overview | null = null;
let messages: Message[] = [];

const esc = (v: unknown) => String(v ?? '').replace(/[&<>"']/g, c => ({ '&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#39;' }[c] || c));
const stamp = (v: string) => { const d = new Date(v); return Number.isNaN(+d) ? v : d.toISOString().replace('T', ' ').slice(0, 16); };
const text = (m: Message) => m.normalizedText || m.rawText || '';

async function load() {
  const res = await fetch(`/api/session-history?${new URLSearchParams({ token, limit: '500' })}`);
  if (!res.ok) throw new Error(`history ${res.status}`);
  const data = await res.json();
  all = Array.isArray(data.sessions) ? data.sessions : [];
}

async function openSession(item: Overview) {
  selected = item; messages = [];
  render();
  const params = new URLSearchParams({ token, session_id: String(item.session_id), limit: '500' });
  try {
    const res = await fetch(`/api/session-chat?${params}`);
    if (!res.ok) throw new Error(`chat ${res.status}`);
    const data = await res.json();
    messages = Array.isArray(data.messages) ? data.messages : [];
  } catch (_) { showToast(ti18n('history_lite_transcript_fetch_failed', 'Could not fetch saved transcript')); }
  render();
}

function selectedText() {
  const checked = Array.from(root?.querySelectorAll<HTMLInputElement>('.history-message-check:checked') || []);
  const ids = new Set(checked.map(el => Number(el.value)));
  const picked = messages.filter(m => ids.has(Number(m.id)));
  return (picked.length ? picked : messages).map(m => `[${m.role}] ${text(m)}`).join('\n\n');
}

function download(format: 'md' | 'txt' | 'json') {
  if (!selected) return;
  const body = selectedText();
  const payload = format === 'json' ? JSON.stringify({ session: selected, messages }, null, 2)
    : format === 'md' ? `# ${selected.label || selected.title || 'Session history'}\n\n${body}` : body;
  const blob = new Blob([payload], { type: format === 'json' ? 'application/json' : 'text/plain;charset=utf-8' });
  const a = document.createElement('a'); a.href = URL.createObjectURL(blob); a.download = `many-ai-cli-history-${selected.session_id}.${format}`; a.click(); URL.revokeObjectURL(a.href);
}

function sendToSpawn() {
  if (!selected) return;
  const prompt = selectedText();
  navigator.clipboard?.writeText(prompt).catch(() => {});
  const open = (window as any).openSpawnFor;
  if (typeof open === 'function') open(selected.provider || 'claude', selected.cwd || '');
  showToast(ti18n('history_lite_spawn_copied', 'Copied selected transcript and opened new session'));
}

function render() {
  if (!root) return;
  const query = (root.querySelector<HTMLInputElement>('#history-filter')?.value || '').toLowerCase();
  const shown = all.filter(s => !query || [s.provider, s.label, s.title, s.cwd, s.first_message, s.last_message].join(' ').toLowerCase().includes(query));
  const tTitle = ti18n('history_lite_title', 'History lite');
  const tSub = ti18n('history_lite_subtitle', 'Quickly browse the last 30 days');
  const tRefresh = ti18n('history_lite_refresh', 'Refresh');
  const tPh = ti18n('history_lite_search_placeholder', 'Search history (provider / label / body)');
  const tEmptyList = ti18n('history_lite_empty_list', 'No saved sessions');
  const tEmptyDetail = ti18n('history_lite_empty_detail', 'Select a session on the left to view transcript and export.');
  const tLoading = ti18n('history_lite_transcript_loading', 'Loading transcript, or none was saved.');
  const tHint = ti18n('history_lite_select_hint', 'Only checked messages are used for export / new spawn (all if none checked).');
  const tSpawn = ti18n('history_lite_spawn_btn', 'Spawn new from selection');
  root.innerHTML = `<header class="history-lite-head"><div><strong>${esc(tTitle)}</strong><span>${esc(tSub)}</span></div><button type="button" id="history-refresh">${esc(tRefresh)}</button></header>
    <input id="history-filter" type="search" placeholder="${esc(tPh)}" value="${esc(query)}">
    <div class="history-lite-layout"><aside class="history-lite-list">${shown.slice(0, 500).map(s => `<button type="button" class="history-session${selected?.id === s.id ? ' active' : ''}" data-id="${s.id}"><b>${esc(s.label || s.title || s.cwd?.split(/[\\/]/).pop() || `Session #${s.session_id}`)}</b><small>${esc(s.provider || 'ai')} · ${stamp(s.ended_at || s.last_output_at || s.started_at || '')}${s.end_reason ? ' · DONE' : ''}</small></button>`).join('') || `<p>${esc(tEmptyList)}</p>`}</aside>
    <main class="history-lite-detail">${selected ? `<div class="history-detail-head"><strong>${esc(selected.label || selected.title || 'Session')}</strong><span>${esc(selected.provider)} · ${esc(selected.cwd)}</span><div><button data-export="md">MD</button><button data-export="txt">TXT</button><button data-export="json">JSON</button><button id="history-spawn">${esc(tSpawn)}</button></div></div><p class="history-select-hint">${esc(tHint)}</p><div class="history-transcript">${messages.length ? messages.map(m => `<label class="history-message"><input class="history-message-check" type="checkbox" value="${m.id}"><span><small>${esc(m.role)} · ${stamp(m.ts)}</small><pre>${esc(text(m))}</pre></span></label>`).join('') : `<p>${esc(tLoading)}</p>`}</div>` : `<p class="history-empty">${esc(tEmptyDetail)}</p>`}</main></div>`;
  root.querySelector('#history-filter')?.addEventListener('input', render);
  root.querySelector('#history-refresh')?.addEventListener('click', async () => { try { await load(); render(); } catch (_) { showToast(ti18n('history_lite_refresh_failed', 'Could not refresh history')); } });
  root.querySelectorAll<HTMLButtonElement>('.history-session').forEach(b => b.onclick = () => { const s = all.find(x => String(x.id) === b.dataset.id); if (s) openSession(s); });
  root.querySelectorAll<HTMLButtonElement>('[data-export]').forEach(b => b.onclick = () => download(b.dataset.export as 'md' | 'txt' | 'json'));
  root.querySelector('#history-spawn')?.addEventListener('click', sendToSpawn);
}

export function initHistoryLite() {
  root = document.getElementById('history-pane');
  if (!root) return;
  root.addEventListener('history:open' as any, () => {});
  document.getElementById('history-tab-btn')?.addEventListener('click', async () => { try { await load(); render(); } catch (_) { showToast(ti18n('history_lite_load_failed', 'Could not load history')); } });
  const tryRender = () => { try { render(); } catch (_) {} };
  if ((window as any).t) tryRender();
  else document.addEventListener('i18n-ready', tryRender, { once: true });
  tryRender();
}
