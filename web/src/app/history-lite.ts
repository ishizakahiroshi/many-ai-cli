// P-10: deliberately small persisted-session workbench. It does not revive the
// removed chat proxy; all data comes from sessionstore and the existing log API.
import { token, showToast } from './util.js';

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
  const params = new URLSearchParams({ token, session_db_id: String(item.id), limit: '500' });
  try {
    const res = await fetch(`/api/session-chat?${params}`);
    if (!res.ok) throw new Error(`chat ${res.status}`);
    const data = await res.json();
    messages = Array.isArray(data.messages) ? data.messages : [];
  } catch (_) { showToast('保存済み transcript を取得できませんでした'); }
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
  showToast('選択 transcript をクリップボードへコピーし、新規セッションを開きました');
}

function render() {
  if (!root) return;
  const query = (root.querySelector<HTMLInputElement>('#history-filter')?.value || '').toLowerCase();
  const shown = all.filter(s => !query || [s.provider, s.label, s.title, s.cwd, s.first_message, s.last_message].join(' ').toLowerCase().includes(query));
  root.innerHTML = `<header class="history-lite-head"><div><strong>履歴ライト</strong><span>直近 30 日をすばやく辿る</span></div><button type="button" id="history-refresh">更新</button></header>
    <input id="history-filter" type="search" placeholder="履歴を検索（provider / ラベル / 本文）" value="${esc(query)}">
    <div class="history-lite-layout"><aside class="history-lite-list">${shown.slice(0, 500).map(s => `<button type="button" class="history-session${selected?.id === s.id ? ' active' : ''}" data-id="${s.id}"><b>${esc(s.label || s.title || s.cwd?.split(/[\\/]/).pop() || `Session #${s.session_id}`)}</b><small>${esc(s.provider || 'ai')} · ${stamp(s.ended_at || s.last_output_at || s.started_at || '')}${s.end_reason ? ' · DONE' : ''}</small></button>`).join('') || '<p>保存済みセッションはありません</p>'}</aside>
    <main class="history-lite-detail">${selected ? `<div class="history-detail-head"><strong>${esc(selected.label || selected.title || 'Session')}</strong><span>${esc(selected.provider)} · ${esc(selected.cwd)}</span><div><button data-export="md">MD</button><button data-export="txt">TXT</button><button data-export="json">JSON</button><button id="history-spawn">選択を新規 spawn へ</button></div></div><p class="history-select-hint">チェックした発言だけを export / 新規 spawn に渡せます（未選択時は全件）。</p><div class="history-transcript">${messages.length ? messages.map(m => `<label class="history-message"><input class="history-message-check" type="checkbox" value="${m.id}"><span><small>${esc(m.role)} · ${stamp(m.ts)}</small><pre>${esc(text(m))}</pre></span></label>`).join('') : '<p>transcript を読み込み中、または保存されていません。</p>'}</div>` : '<p class="history-empty">左からセッションを選ぶと transcript と export を表示します。</p>'}</main></div>`;
  root.querySelector('#history-filter')?.addEventListener('input', render);
  root.querySelector('#history-refresh')?.addEventListener('click', async () => { try { await load(); render(); } catch (_) { showToast('履歴を更新できませんでした'); } });
  root.querySelectorAll<HTMLButtonElement>('.history-session').forEach(b => b.onclick = () => { const s = all.find(x => String(x.id) === b.dataset.id); if (s) openSession(s); });
  root.querySelectorAll<HTMLButtonElement>('[data-export]').forEach(b => b.onclick = () => download(b.dataset.export as 'md' | 'txt' | 'json'));
  root.querySelector('#history-spawn')?.addEventListener('click', sendToSpawn);
}

export function initHistoryLite() {
  root = document.getElementById('history-pane');
  if (!root) return;
  root.addEventListener('history:open' as any, () => {});
  document.getElementById('history-tab-btn')?.addEventListener('click', async () => { try { await load(); render(); } catch (_) { showToast('履歴を読み込めませんでした'); } });
  render();
}
