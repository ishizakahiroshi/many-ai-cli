// 引き継ぎ md のプレビュー・一覧・起動（子 plan:
// docs/local/plan_session-handoff-board_c5_handoff-md.md 内部 C1・C2・C3）。
//
// 起動そのものは既存の C4 の入口である POST /api/spawn（initial_prompt +
// handoff_from）をそのまま呼ぶ。このファイルは看板からの markdown 取得・
// プレビュー表示・provider 選択（元セッションと同じ provider は候補に出さ
// ない）までを担う。自動では 1 本も起動しない — 人がボタンを押すことが
// そのまま承認になる（親 plan 方針 B1）。
import { t } from '../i18n.js';
import type { Message } from '../types/proto.js';
import { apiFetch, escapeHtml, showToast, token } from './util.js';
import { ORCHESTRATION_CLI_OPTIONS } from './orchestration-roles.js';

function tx(key: string, fallback: string, vars: Record<string, unknown> = {}): string {
  let value = t(key, vars);
  if (value === key) {
    value = fallback;
    Object.entries(vars).forEach(([name, replacement]) => {
      value = value.replaceAll(`{${name}}`, String(replacement));
    });
  }
  return value;
}

function providerLabel(value: string): string {
  const found = ORCHESTRATION_CLI_OPTIONS.find((o) => o.value === value);
  if (!found) return value;
  return found.label || tx(found.labelKey || '', found.labelKey || value);
}

// ---- 経路 1: 残量が閾値を下回ったときの通知 --------------------------------

let notifyThresholdPercent = 10; // /api/info から上書きされるまでの保守的な既定値
const notifiedSessions = new Set<number>();

// setHandoffNotifyThresholdPercent は settings.ts の /api/info 読み込みから
// 一度だけ呼ばれる。Hub の handoff.notify_remaining_percent（既定 10）を
// そのままミラーする — 閾値の正本は 1 箇所（サーバー設定）に置く。
export function setHandoffNotifyThresholdPercent(percent: number): void {
  if (Number.isFinite(percent) && percent > 0) notifyThresholdPercent = percent;
}

function usedPercentsFromUsageStat(m: Message): number[] {
  const out: number[] = [];
  if (m.claude_5h_present && typeof m.rl_5h_pct === 'number') out.push(m.rl_5h_pct);
  if (m.claude_7d_present && typeof m.rl_7d_pct === 'number') out.push(m.rl_7d_pct);
  if (m.codex_primary_present && typeof m.codex_primary_used_pct === 'number') out.push(m.codex_primary_used_pct);
  if (m.codex_secondary_present && typeof m.codex_secondary_used_pct === 'number') out.push(m.codex_secondary_used_pct);
  return out;
}

// checkHandoffNotifyFromUsageStat は ws-client.ts の usage_stat 受信フックから
// 毎回呼ばれる。残量の実測値は Claude の statusLine / Codex の rate limit
// フィールドとして usage_stat に既に乗っている（internal/hub/usage_stat.go）
// ので、新しい WS メッセージ型を増やさずここで閾値判定だけ行う。
// 一度知らせたセッションは同じブラウザタブでは再度出さない（notifiedSessions）。
export function checkHandoffNotifyFromUsageStat(m: Message): void {
  const sessionID = Number(m.session_id || 0);
  if (!sessionID || notifiedSessions.has(sessionID)) return;
  const used = usedPercentsFromUsageStat(m);
  if (used.length === 0) return;
  const remaining = 100 - Math.max(...used);
  if (remaining > notifyThresholdPercent) return;
  notifiedSessions.add(sessionID);
  showHandoffNotifyBanner(sessionID, String(m.provider || ''));
}

const HANDOFF_NOTIFY_AUTO_DISMISS_MS = 30000;

function showHandoffNotifyBanner(sessionID: number, provider: string): void {
  const el = document.createElement('div');
  el.className = 'handoff-notify-banner';
  el.setAttribute('role', 'status');
  el.innerHTML = `<div class="handoff-notify-body">${escapeHtml(tx('handoff_notify_body', 'セッション #{id}（{provider}）の残量が少なくなっています。', { id: sessionID, provider: providerLabel(provider) }))}</div>
    <div class="handoff-notify-actions">
      <button type="button" class="handoff-notify-action">${escapeHtml(tx('handoff_notify_action', '引き継ぎを準備'))}</button>
      <button type="button" class="handoff-notify-dismiss" aria-label="${escapeHtml(tx('handoff_notify_dismiss', '閉じる'))}">×</button>
    </div>`;
  document.body.appendChild(el);
  const remove = () => { el.remove(); };
  el.querySelector('.handoff-notify-action')?.addEventListener('click', () => { remove(); void openHandoffPreviewDialog(sessionID); });
  el.querySelector('.handoff-notify-dismiss')?.addEventListener('click', remove);
  window.setTimeout(remove, HANDOFF_NOTIFY_AUTO_DISMISS_MS);
}

// ---- プレビュー + 起動ダイアログ -------------------------------------------

interface HandoffPreviewResponse {
  ok: boolean;
  session_id: number;
  exists: boolean;
  live?: boolean;
  provider?: string;
  cwd?: string;
  branch?: string;
  model?: string;
  markdown?: string;
  candidate_providers?: string[];
}

function buildDialogShell(titleKey: string, titleFallback: string, extraClass = ''): { backdrop: HTMLElement; close: () => void } {
  const backdrop = document.createElement('div');
  backdrop.className = `handoff-dialog-backdrop aac-wheel-overlay${extraClass ? ` ${extraClass}` : ''}`;
  backdrop.innerHTML = `<div class="handoff-dialog" role="dialog" aria-modal="true" aria-labelledby="handoff-dialog-title">
    <h2 id="handoff-dialog-title">${escapeHtml(tx(titleKey, titleFallback))}</h2>
    <div class="handoff-dialog-body"><p>${escapeHtml(tx('handoff_dialog_loading', '読み込み中…'))}</p></div>
    <div class="handoff-dialog-actions"><button type="button" data-handoff-cancel>${escapeHtml(tx('handoff_dialog_cancel', 'キャンセル'))}</button></div>
  </div>`;
  document.body.appendChild(backdrop);
  const onKeyDown = (event: KeyboardEvent) => { if (event.key === 'Escape') close(); };
  const close = () => {
    document.removeEventListener('keydown', onKeyDown);
    backdrop.remove();
  };
  document.addEventListener('keydown', onKeyDown);
  backdrop.addEventListener('click', (event) => { if (event.target === backdrop) close(); });
  backdrop.querySelector('[data-handoff-cancel]')?.addEventListener('click', close);
  return { backdrop, close };
}

function setDialogBody(backdrop: HTMLElement, html: string): void {
  const body = backdrop.querySelector('.handoff-dialog-body');
  if (body) body.innerHTML = html;
}

function buildProviderSelect(candidates: string[]): HTMLSelectElement {
  const select = document.createElement('select');
  select.className = 'handoff-provider-select';
  const empty = document.createElement('option');
  empty.value = '';
  empty.textContent = tx('handoff_dialog_provider_empty', 'CLI を選択…');
  select.appendChild(empty);
  candidates.forEach((value) => {
    const option = document.createElement('option');
    option.value = value;
    option.textContent = providerLabel(value);
    select.appendChild(option);
  });
  return select;
}

async function startHandoffSession(sessionID: number, provider: string, cwd: string, markdown: string, onStarted: () => void): Promise<void> {
  try {
    const response = await apiFetch(`/api/spawn?token=${encodeURIComponent(token || '')}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ provider, cwd, initial_prompt: markdown, handoff_from: sessionID }),
    });
    const data = await response.json().catch(() => ({}));
    if (!response.ok) {
      showToast(String(data?.detail || data?.error || `HTTP ${response.status}`));
      return;
    }
    onStarted();
    showToast(tx('handoff_dialog_started', '新しいセッションを起動しました'));
  } catch (error) {
    showToast(String(error instanceof Error ? error.message : error));
  }
}

function renderHandoffPreviewContent(backdrop: HTMLElement, sessionID: number, data: HandoffPreviewResponse, close: () => void): void {
  const bodyEl = backdrop.querySelector('.handoff-dialog-body') as HTMLElement | null;
  const actionsEl = backdrop.querySelector('.handoff-dialog-actions') as HTMLElement | null;
  if (!bodyEl || !actionsEl) return;

  bodyEl.innerHTML = '';
  const source = document.createElement('p');
  source.className = 'handoff-dialog-source';
  source.textContent = `${tx('handoff_dialog_source', '元セッション')}: #${sessionID} ${providerLabel(String(data.provider || ''))} — ${data.cwd || ''}`;
  bodyEl.appendChild(source);
  const pre = document.createElement('pre');
  pre.className = 'handoff-dialog-markdown';
  pre.textContent = String(data.markdown || '');
  bodyEl.appendChild(pre);

  const providerRow = document.createElement('label');
  providerRow.className = 'handoff-dialog-provider-row';
  const labelText = document.createElement('span');
  labelText.textContent = tx('handoff_dialog_provider_label', '起動する CLI');
  const select = buildProviderSelect(Array.isArray(data.candidate_providers) ? data.candidate_providers : []);
  providerRow.appendChild(labelText);
  providerRow.appendChild(select);
  bodyEl.appendChild(providerRow);

  actionsEl.innerHTML = '';
  const cancelBtn = document.createElement('button');
  cancelBtn.type = 'button';
  cancelBtn.textContent = tx('handoff_dialog_cancel', 'キャンセル');
  cancelBtn.addEventListener('click', close);
  const startBtn = document.createElement('button');
  startBtn.type = 'button';
  startBtn.className = 'primary';
  startBtn.textContent = tx('handoff_dialog_start', '起動');
  startBtn.disabled = true;
  select.addEventListener('change', () => { startBtn.disabled = !select.value; });
  startBtn.addEventListener('click', () => {
    if (!select.value || startBtn.disabled) return;
    startBtn.disabled = true;
    void startHandoffSession(sessionID, select.value, String(data.cwd || ''), String(data.markdown || ''), close)
      .finally(() => { startBtn.disabled = !select.value; });
  });
  actionsEl.appendChild(cancelBtn);
  actionsEl.appendChild(startBtn);
}

// openHandoffPreviewDialog is the shared entry point for both routes (経路 1
// の通知バナーと 経路 2 の一覧、どちらもこれを呼ぶ)。
export async function openHandoffPreviewDialog(sessionID: number): Promise<void> {
  const { backdrop, close } = buildDialogShell('handoff_dialog_title', '引き継ぎを準備');
  try {
    const response = await apiFetch(`/api/handoff/${encodeURIComponent(String(sessionID))}?token=${encodeURIComponent(token || '')}`);
    const data = await response.json() as HandoffPreviewResponse;
    if (!response.ok || !data.ok) throw new Error('handoff fetch failed');
    if (!data.exists) {
      setDialogBody(backdrop, `<p>${escapeHtml(tx('handoff_dialog_no_record', 'この記録は見つかりません'))}</p>`);
      return;
    }
    renderHandoffPreviewContent(backdrop, sessionID, data, close);
  } catch (_) {
    setDialogBody(backdrop, `<p>${escapeHtml(tx('handoff_dialog_no_record', 'この記録は見つかりません'))}</p>`);
  }
}

// ---- 経路 2: 一覧から開く（Hub 再起動後も看板ディレクトリを直接読むので働く）----

interface HandoffListEntry {
  session_id: number;
  provider?: string;
  cwd?: string;
  branch?: string;
  live?: boolean;
  started_at?: string;
  ended_at?: string;
}

export async function openHandoffListDialog(): Promise<void> {
  const { backdrop, close } = buildDialogShell('handoff_list_title', '引き継ぎできるセッション', 'handoff-list-dialog-backdrop');
  backdrop.querySelector('.handoff-dialog')?.classList.add('handoff-list-dialog');
  const body = backdrop.querySelector('.handoff-dialog-body') as HTMLElement | null;
  if (!body) return;
  try {
    const response = await apiFetch(`/api/handoff?token=${encodeURIComponent(token || '')}`);
    const data = await response.json();
    if (!response.ok || !data.ok) throw new Error('handoff list failed');
    const entries: HandoffListEntry[] = Array.isArray(data.entries) ? data.entries : [];
    if (entries.length === 0) {
      body.innerHTML = `<p>${escapeHtml(tx('handoff_list_empty', '引き継ぎの記録はまだありません'))}</p>`;
      return;
    }
    body.innerHTML = '';
    const list = document.createElement('div');
    list.className = 'handoff-list-rows';
    entries.forEach((entry) => {
      const row = document.createElement('div');
      row.className = 'handoff-list-row';
      const status = entry.live ? tx('handoff_list_live', '実行中') : tx('handoff_list_ended', '終了');
      const main = document.createElement('div');
      main.className = 'handoff-list-row-main';
      main.innerHTML = `<span class="handoff-list-row-id">#${escapeHtml(String(entry.session_id))}</span>` +
        `<span class="handoff-list-row-provider">${escapeHtml(providerLabel(String(entry.provider || '')))}</span>` +
        `<span class="handoff-list-row-status" data-live="${entry.live ? '1' : '0'}">${escapeHtml(status)}</span>` +
        `<span class="handoff-list-row-cwd" title="${escapeHtml(String(entry.cwd || ''))}">${escapeHtml(String(entry.cwd || ''))}</span>`;
      const previewBtn = document.createElement('button');
      previewBtn.type = 'button';
      previewBtn.className = 'handoff-list-row-preview';
      previewBtn.textContent = tx('handoff_list_preview', 'プレビュー');
      previewBtn.addEventListener('click', () => {
        close();
        void openHandoffPreviewDialog(entry.session_id);
      });
      row.appendChild(main);
      row.appendChild(previewBtn);
      list.appendChild(row);
    });
    body.appendChild(list);
  } catch (_) {
    body.innerHTML = `<p>${escapeHtml(tx('handoff_list_empty', '引き継ぎの記録はまだありません'))}</p>`;
  }
}

document.getElementById('handoff-list-btn')?.addEventListener('click', () => { void openHandoffListDialog(); });
