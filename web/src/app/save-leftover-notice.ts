// 保存の途中で Hub が止まった file の知らせ（監査 N16）。
//
// ファイルタブの保存は、書き込む前に元の内容の控えを Hub の私用フォルダへ取り、書き終えたら
// 消す。書き込みの途中で Hub が止まる（強制終了・電源断）と、file は書きかけのまま、控えだけが
// 残る。次に起動した Hub はその控えを一覧に載せる（rust/src/files/save_backup.rs）。
//
// **Hub も画面も自分では戻さない・消さない。** 利用者が「元の内容へ戻す」か「控えを消す」を
// 選んだときだけ動く。一覧に載せてから 7 日たった控えは Hub が消す。
// 新しい WS メッセージは足さず、接続のたびに一覧を取り直す。
import { t } from '../i18n.js';
import { apiFetch, escapeHtml, showToast } from './util.js';

export interface SaveLeftover {
  id: string;
  path: string;
  saved_at: string;
  size: number;
  backup: string;
}

let banner: HTMLElement | null = null;
// ✕ で閉じた控え。このタブでは出し直さない（消えたわけではないので Hub の一覧には残る）。
const closed = new Set<string>();

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

// Hub の答えから、画面に出せる行だけを取り出す。
export function readSaveLeftovers(data: unknown): SaveLeftover[] {
  const list = (data as { leftovers?: unknown } | null)?.leftovers;
  if (!Array.isArray(list)) return [];
  return list
    .filter((l) => l && typeof l.id === 'string' && l.id !== '')
    .map((l) => ({
      id: String(l.id),
      path: typeof l.path === 'string' ? l.path : '',
      saved_at: typeof l.saved_at === 'string' ? l.saved_at : '',
      size: Number.isFinite(Number(l.size)) ? Number(l.size) : 0,
      backup: typeof l.backup === 'string' ? l.backup : '',
    }));
}

export async function refreshSaveLeftoverNotice(): Promise<void> {
  let leftovers: SaveLeftover[];
  try {
    const res = await apiFetch('/api/files-save-leftovers');
    if (!res.ok) return;
    leftovers = readSaveLeftovers(await res.json());
  } catch (_) {
    return;
  }
  render(leftovers.filter((l) => !closed.has(l.id)));
}

function whenText(l: SaveLeftover): string {
  const at = new Date(l.saved_at);
  const time = l.saved_at && !Number.isNaN(at.getTime()) ? at.toLocaleString() : l.saved_at;
  return tx('save_leftover_when', 'Save started {time} · earlier content {size} bytes', { time, size: l.size });
}

function refusalText(code: string, detail: string): string {
  switch (code) {
    case 'leftover_gone':
      return tx('save_leftover_refused_gone', 'The copy is already gone.');
    case 'save_running':
      return tx('save_leftover_refused_running', 'A save is using this copy right now.');
    case 'no_record':
      return tx('save_leftover_refused_no_record', 'This copy has no record of its file, so it cannot be put back from here.');
    default:
      return tx('save_leftover_refused_failed', 'It could not be done: {detail}', { detail });
  }
}

function render(leftovers: SaveLeftover[]): void {
  if (leftovers.length === 0) {
    banner?.remove();
    banner = null;
    return;
  }
  if (!banner) {
    banner = document.createElement('div');
    banner.className = 'git-lock-banner save-leftover-banner';
    banner.setAttribute('role', 'status');
    document.body.appendChild(banner);
  }
  banner.innerHTML = `<div class="git-lock-head">
      <div class="git-lock-body">${escapeHtml(tx('save_leftover_body', 'The Hub stopped while it was saving these files, so they may be half written. A copy of each file\'s earlier content was kept. Put it back, or delete the copy.'))}</div>
      <button type="button" class="ui-button ui-button--secondary ui-button--regular git-lock-dismiss" aria-label="${escapeHtml(tx('git_lock_dismiss', 'Close'))}">✕</button>
    </div>
    <ul class="git-lock-list">${saveLeftoverRowsHtml(leftovers)}</ul>`;
  banner.querySelector('.git-lock-dismiss')?.addEventListener('click', () => {
    leftovers.forEach((l) => closed.add(l.id));
    banner?.remove();
    banner = null;
  });
  banner.querySelectorAll<HTMLElement>('.git-lock-row').forEach((row) => {
    const leftover = leftovers.find((l) => l.id === row.dataset.leftoverId);
    if (!leftover) return;
    row.querySelector<HTMLButtonElement>('.save-leftover-restore')
      ?.addEventListener('click', () => { void act('restore', leftover, row); });
    row.querySelector<HTMLButtonElement>('.save-leftover-discard')
      ?.addEventListener('click', () => { void act('discard', leftover, row); });
  });
}

// 一覧の行。どの file のものか分かる控えにだけ「元の内容へ戻す」を出す。
export function saveLeftoverRowsHtml(leftovers: SaveLeftover[]): string {
  return leftovers.map((l) => {
    const name = l.path || tx('save_leftover_unknown_file', 'A copy whose file is not recorded');
    const restore = l.path
      ? `<button type="button" class="ui-button ui-button--secondary ui-button--regular save-leftover-restore">${escapeHtml(tx('save_leftover_restore', 'Put earlier content back'))}</button>`
      : '';
    return `
    <li class="git-lock-row" data-leftover-id="${escapeHtml(l.id)}">
      <div class="git-lock-repo">${escapeHtml(name)}</div>
      <div class="git-lock-cause">${escapeHtml(whenText(l))}</div>
      <div class="git-lock-cause">${escapeHtml(tx('save_leftover_kept_at', 'Copy: {backup}', { backup: l.backup }))}</div>
      <div class="git-lock-status" data-leftover-status hidden></div>
      <div class="git-lock-row-actions save-leftover-actions">
        <button type="button" class="ui-button ui-button--secondary ui-button--regular save-leftover-discard">${escapeHtml(tx('save_leftover_discard', 'Delete the copy'))}</button>
        ${restore}
      </div>
    </li>`;
  }).join('');
}

async function act(what: 'restore' | 'discard', l: SaveLeftover, row: HTMLElement): Promise<void> {
  // 確認の部品は設定の画面と同じもの。押されたときに読み込む（この知らせは接続の直後に動くので、
  // 読み込みの順に縛られないようにする）。
  const { appConfirm } = await import('./settings.js');
  const confirmed = what === 'restore'
    ? await appConfirm({ action: 'restore', message: tx('save_leftover_restore_confirm', 'The current content of {path} is replaced with its content from before the interrupted save.', { path: l.path }) })
    : await appConfirm({ action: 'delete', message: tx('save_leftover_discard_confirm', 'The copy of the earlier content of {path} is deleted. The file itself is not changed.', { path: l.path || l.backup }) });
  if (!confirmed) return;
  const buttons = row.querySelectorAll<HTMLButtonElement>('button');
  buttons.forEach((b) => { b.disabled = true; });
  const status = row.querySelector<HTMLElement>('[data-leftover-status]');
  let code = '';
  let message = '';
  try {
    const res = await apiFetch(what === 'restore' ? '/api/files-save-leftover-restore' : '/api/files-save-leftover-discard', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: l.id }),
    });
    const data = await res.json().catch(() => ({}));
    if (res.ok && data?.ok !== false) {
      showToast(what === 'restore'
        ? tx('save_leftover_restored', 'Put the earlier content back')
        : tx('save_leftover_discarded', 'Deleted the copy'));
    } else {
      code = String(data?.error || '');
      message = refusalText(code, String(data?.detail || `HTTP ${res.status}`));
    }
  } catch (err) {
    message = refusalText('', err instanceof Error ? err.message : String(err));
  }
  // 「もう無い」は Hub の一覧からも消えているので、行が消える前にトーストで理由を出す。
  if (message && code !== 'leftover_gone' && status) {
    status.textContent = message;
    status.hidden = false;
    buttons.forEach((b) => { b.disabled = false; });
    return;
  }
  if (message) showToast(message, undefined, 4000);
  await refreshSaveLeftoverNotice();
}
