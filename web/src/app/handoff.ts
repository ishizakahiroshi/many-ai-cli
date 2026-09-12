// 引き継ぎ md の残量通知と一覧（子 plan:
// docs/local/plan_session-handoff-board_c5_handoff-md.md 内部 C2・C3）。
//
// **起動はこのファイルでは行わない。** 帯の「引き継ぎを準備」も ↪ 一覧の行も、
// 派生ダイアログ（derive-dialog.ts）を種別 = 引き継ぎで開くだけになった
// （子 plan: docs/local/plan_derived-session-launch_c3_derive-launch.md 内部 C3）。
// 看板 markdown の取得・プレビュー・provider 選択・POST /api/spawn は全部そちらに
// あり、同じことを 2 通り書かない。ここに `/api/spawn` を呼ぶコードを戻さないこと。
//
// 自動では 1 本も起動しない — 人が起動ボタンを押すことがそのまま承認になる
// （親 plan 方針 B1）。apiFetch は Cookie 主体で query token を付けない（F-WEB-05）。
import { t } from '../i18n.js';
import type { Message } from '../types/proto.js';
import { apiFetch, escapeHtml, showToast } from './util.js';
import { ORCHESTRATION_CLI_OPTIONS } from './orchestration-roles.js';
import { openDeriveDialog } from './derive-dialog.js';
import {
  handoffNoteActionFor,
  normalizeHandoffNoteMode,
  remainingPercentFromUsageStat,
  type HandoffNoteMode,
} from './handoff-store.js';

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

// 引き継ぎメモの扱い（ask / auto / off）。閾値と同じく /api/info の値をミラーする
// だけで、既定は保守的な ask（人が押したときだけ前任のトークンを使う）。
let noteMode: HandoffNoteMode = 'ask';

export function setHandoffNoteMode(value: unknown): void {
  noteMode = normalizeHandoffNoteMode(value);
}

// checkHandoffNotifyFromUsageStat は ws-client.ts の usage_stat 受信フックから
// 毎回呼ばれる。残量の実測値は Claude の statusLine / Codex の rate limit
// フィールドとして usage_stat に既に乗っている（internal/hub/usage_stat.go）
// ので、新しい WS メッセージ型を増やさずここで閾値判定だけ行う。
// 一度知らせたセッションは同じブラウザタブでは再度出さない（notifiedSessions）。
export function checkHandoffNotifyFromUsageStat(m: Message): void {
  const sessionID = Number(m.session_id || 0);
  if (!sessionID || notifiedSessions.has(sessionID)) return;
  const remaining = remainingPercentFromUsageStat(m);
  if (remaining === null || remaining > notifyThresholdPercent) return;
  notifiedSessions.add(sessionID);
  showHandoffNotifyBanner(sessionID, String(m.provider || ''));
}

const HANDOFF_NOTIFY_AUTO_DISMISS_MS = 30000;

// 帯は 30 秒で消えるが、メモの結果（WS の handoff_note）はその後に届くことが多い。
// 生きている帯があればそこへ、無ければトーストへ出す。
const noteBanners = new Map<number, HTMLElement>();

function showHandoffNotifyBanner(sessionID: number, provider: string): void {
  const action = handoffNoteActionFor(noteMode);
  const el = document.createElement('div');
  el.className = 'handoff-notify-banner';
  el.setAttribute('role', 'status');
  const noteButton = action === 'button'
    ? `<button type="button" class="handoff-notify-note">${escapeHtml(tx('handoff_notify_note', '引き継ぎメモを書かせる'))}</button>`
    : '';
  el.innerHTML = `<div class="handoff-notify-body">${escapeHtml(tx('handoff_notify_body', 'セッション #{id}（{provider}）の残量が少なくなっています。', { id: sessionID, provider: providerLabel(provider) }))}</div>
    <div class="handoff-notify-status" data-handoff-note-status hidden></div>
    <div class="handoff-notify-actions">
      <button type="button" class="handoff-notify-action">${escapeHtml(tx('handoff_notify_action', '引き継ぎを準備'))}</button>
      ${noteButton}
      <button type="button" class="handoff-notify-dismiss" aria-label="${escapeHtml(tx('handoff_notify_dismiss', '閉じる'))}">×</button>
    </div>`;
  document.body.appendChild(el);
  noteBanners.set(sessionID, el);
  const remove = () => { el.remove(); if (noteBanners.get(sessionID) === el) noteBanners.delete(sessionID); };
  el.querySelector('.handoff-notify-action')?.addEventListener('click', () => { remove(); void openDeriveDialog(sessionID, { kind: 'handoff' }); });
  const noteBtn = el.querySelector('.handoff-notify-note') as HTMLButtonElement | null;
  noteBtn?.addEventListener('click', () => {
    noteBtn.disabled = true;
    setBannerStatus(sessionID, tx('handoff_note_asked', '引き継ぎメモを依頼しました…'));
    void requestHandoffNote(sessionID);
  });
  el.querySelector('.handoff-notify-dismiss')?.addEventListener('click', remove);
  window.setTimeout(remove, HANDOFF_NOTIFY_AUTO_DISMISS_MS);
  // auto は人を待たずに 1 回だけ依頼する。押す先が無いのでボタンは出していない。
  if (action === 'auto') {
    setBannerStatus(sessionID, tx('handoff_note_asked', '引き継ぎメモを依頼しました…'));
    void requestHandoffNote(sessionID);
  }
}

function setBannerStatus(sessionID: number, text: string): void {
  const status = noteBanners.get(sessionID)?.querySelector('[data-handoff-note-status]') as HTMLElement | null;
  if (!status) return;
  status.textContent = text;
  status.hidden = false;
}

// メモの結果は帯が残っていれば帯へ、消えていればトーストへ。どちらにも出せない
// ときは黙る（この通知のために画面を奪わない）。
function reportNoteResult(sessionID: number, text: string): void {
  if (noteBanners.has(sessionID)) {
    setBannerStatus(sessionID, text);
    return;
  }
  showToast(text);
}

// POST /api/handoff/<id>/note。Hub は注入したところで返し、書けたかどうかは後から
// WS の handoff_note で届く（前任が 1 ターン使って書くため）。
async function requestHandoffNote(sessionID: number): Promise<void> {
  try {
    const res = await apiFetch(`/api/handoff/${encodeURIComponent(String(sessionID))}/note`, { method: 'POST' });
    if (!res.ok) reportNoteResult(sessionID, tx('handoff_note_failed', 'メモは書かれませんでした'));
  } catch (_) {
    reportNoteResult(sessionID, tx('handoff_note_failed', 'メモは書かれませんでした'));
  }
}

// handleHandoffNoteMessage は ws-client.ts の handoff_note 受信フックから呼ばれる。
export function handleHandoffNoteMessage(m: Message): void {
  const sessionID = Number(m.session_id || 0);
  if (!sessionID) return;
  const path = String(m.note_path || '');
  reportNoteResult(sessionID, m.note_ok && path
    ? tx('handoff_note_written', 'メモを書きました: {path}', { path })
    : tx('handoff_note_failed', 'メモは書かれませんでした'));
}

// ---- 一覧ダイアログの外枠 ---------------------------------------------------

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

// ---- 経路 2: 一覧から開く（Hub 再起動後も看板ディレクトリを直接読むので働く）----

interface HandoffListEntry {
  session_id: number;
  provider?: string;
  cwd?: string;
  branch?: string;
  live?: boolean;
  started_at?: string;
  ended_at?: string;
  /** このセッションを引き継いだ後継（Hub 側の逆引き。無ければ 0 / 未送）。 */
  handoff_to?: number;
}

export async function openHandoffListDialog(): Promise<void> {
  const { backdrop, close } = buildDialogShell('handoff_list_title', '引き継ぎできるセッション', 'handoff-list-dialog-backdrop');
  backdrop.querySelector('.handoff-dialog')?.classList.add('handoff-list-dialog');
  const body = backdrop.querySelector('.handoff-dialog-body') as HTMLElement | null;
  if (!body) return;
  try {
    const response = await apiFetch('/api/handoff');
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
      // 後継がいる行にはその番号を出す（子 plan:
      // plan_derived-session-launch_c3_derive-launch.md 内部 C4）。前任の看板には
      // 何も書かず、Hub 側が後継の handoff_from から逆引きした値をそのまま見せる。
      const successor = Number(entry.handoff_to || 0);
      const successorHtml = successor
        ? `<span class="handoff-list-row-successor">${escapeHtml(tx('card_handoff_to', `Successor #${successor}`, { id: successor }))}</span>`
        : '';
      main.innerHTML = `<span class="handoff-list-row-id">#${escapeHtml(String(entry.session_id))}</span>` +
        `<span class="handoff-list-row-provider">${escapeHtml(providerLabel(String(entry.provider || '')))}</span>` +
        `<span class="handoff-list-row-status" data-live="${entry.live ? '1' : '0'}">${escapeHtml(status)}</span>` +
        successorHtml +
        `<span class="handoff-list-row-cwd" title="${escapeHtml(String(entry.cwd || ''))}">${escapeHtml(String(entry.cwd || ''))}</span>`;
      const previewBtn = document.createElement('button');
      previewBtn.type = 'button';
      previewBtn.className = 'handoff-list-row-preview';
      previewBtn.textContent = tx('handoff_list_preview', 'プレビュー');
      previewBtn.addEventListener('click', () => {
        close();
        // 一覧の行からも派生ダイアログ（種別 = 引き継ぎ）を開く。看板 markdown は
        // そちらが自分で取りに行くので、ここでプレビューを組み立てない。
        void openDeriveDialog(entry.session_id, { kind: 'handoff' });
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
