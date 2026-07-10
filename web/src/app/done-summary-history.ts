import { activateSession } from './session-list.js';
import type { DoneSummary } from '../types/proto.js';

const STORAGE_KEY = 'many-ai-cli.done-summary-history.v1';
const MAX_ITEMS = 30;
const MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;

let items: DoneSummary[] = [];
let panel: HTMLElement | null = null;
let list: HTMLElement | null = null;

function load(): DoneSummary[] {
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) || '[]');
    const cutoff = Date.now() - MAX_AGE_MS;
    return Array.isArray(raw) ? raw.filter((v): v is DoneSummary => !!v && typeof v.text === 'string' && typeof v.session_id === 'number' && Date.parse(v.at || '') >= cutoff).slice(0, MAX_ITEMS) : [];
  } catch (_) { return []; }
}
function save(): void { try { localStorage.setItem(STORAGE_KEY, JSON.stringify(items)); } catch (_) {} }
function label(kind: string): string { return ({ failure: '失敗', aborted: '中断', needs_action: '要判断' } as Record<string, string>)[kind] || '成功'; }
function displayTime(at: string): string { const d = new Date(at); return Number.isNaN(d.getTime()) ? '' : `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`; }

function render(): void {
  if (!list) return;
  list.textContent = '';
  if (!items.length) { const empty = document.createElement('p'); empty.className = 'done-summary-empty'; empty.textContent = '完了サマリーはまだありません。'; list.appendChild(empty); return; }
  for (const item of items) {
    const row = document.createElement('article'); row.className = `done-summary-item done-summary-${item.kind || 'success'}`;
    const head = document.createElement('div'); head.className = 'done-summary-item-head';
    const state = document.createElement('span'); state.className = 'done-summary-kind'; state.textContent = label(item.kind);
    const meta = document.createElement('span'); meta.textContent = `${displayTime(item.at)} ${item.title || item.provider || `#${item.session_id}`}`;
    head.append(state, meta);
    const text = document.createElement('p'); text.textContent = item.text;
    const actions = document.createElement('div'); actions.className = 'done-summary-actions';
    const open = document.createElement('button'); open.type = 'button'; open.textContent = 'セッションを開く'; open.addEventListener('click', () => { activateSession(item.session_id); panel?.setAttribute('hidden', ''); });
    const edit = document.createElement('button'); edit.type = 'button'; edit.textContent = '編集'; edit.addEventListener('click', () => { const next = window.prompt('完了サマリーを編集', item.text); if (next === null) return; item.text = next.trim().slice(0, 320); save(); render(); });
    actions.append(open, edit); row.append(head, text, actions); list.appendChild(row);
  }
}
function add(item: DoneSummary): void { if (!item?.text) return; items = [item, ...items.filter(v => !(v.session_id === item.session_id && v.at === item.at))].slice(0, MAX_ITEMS); save(); render(); }

export function initDoneSummaryHistory(): void {
  items = load();
  const anchor = document.getElementById('summary')?.parentElement || document.body;
  const trigger = document.createElement('button'); trigger.type = 'button'; trigger.className = 'done-summary-trigger'; trigger.textContent = '完了履歴'; trigger.title = '完了サマリー履歴';
  panel = document.createElement('aside'); panel.className = 'done-summary-panel'; panel.setAttribute('hidden', ''); panel.innerHTML = '<div class="done-summary-panel-header"><strong>完了サマリー</strong><button type="button" aria-label="閉じる">×</button></div>';
  list = document.createElement('div'); list.className = 'done-summary-list'; panel.appendChild(list);
  panel.querySelector('button')?.addEventListener('click', () => panel?.setAttribute('hidden', ''));
  trigger.addEventListener('click', () => panel?.toggleAttribute('hidden'));
  anchor.appendChild(trigger); document.body.appendChild(panel);
  window.addEventListener('many-done-summary', (event: Event) => add((event as CustomEvent<DoneSummary>).detail));
  render();
}
