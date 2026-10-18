// 止まった git が残した .git/index.lock の知らせ
// （plan: docs/local/plan_git-invocation-index-lock.md C4）。
//
// セッションが commit の途中で終わったり、Hub が書き込みの git を時間切れで止めたり
// すると、ロックが残って次の git add / commit が "File exists" で失敗する。Hub は
// 止めてから数秒後にその repo を見て、残っていれば一覧に載せる（rust/src/files/git_lock.rs）。
//
// **Hub も画面も自分では消さない。** 消すのは利用者が「ロックを消す」を押したときだけで、
// そのときも Hub が「一覧に載せたときから変わっていない」「その repo で git が動いていない」を
// 確かめ直す。この画面は一覧を出し、押された id を送り、断られたら理由を出すだけ。
// 新しい WS メッセージは足さず、セッション終了と接続のたびに一覧を取り直す。
import { t } from '../i18n.js';
import { apiFetch, escapeHtml, showToast } from './util.js';

interface Leftover {
  id: number;
  repository: string;
  lock: string;
  cause: string;
  session: number;
  stopped_at: string;
  modified_at: string;
  size: number;
}

// Hub は止めてから 3 秒後に調べる（git_lock::SETTLE）。その後に取りに行く。
const SETTLE_MS = 3500;

let banner: HTMLElement | null = null;
let timer: number | undefined;
// ✕ で閉じたロック。このタブでは出し直さない（消えたわけではないので Hub の一覧には残る）。
const closed = new Set<number>();

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

// セッションの終了や git の失敗のあと、Hub が調べ終わるのを待ってから取り直す。
export function scheduleGitLockCheck(): void {
  if (timer !== undefined) window.clearTimeout(timer);
  timer = window.setTimeout(() => {
    timer = undefined;
    void refreshGitLockNotice();
  }, SETTLE_MS);
}

export async function refreshGitLockNotice(): Promise<void> {
  let leftovers: Leftover[];
  try {
    const res = await apiFetch('/api/git-lock-leftovers');
    if (!res.ok) return;
    const data = await res.json();
    leftovers = Array.isArray(data?.leftovers) ? data.leftovers : [];
  } catch (_) {
    return;
  }
  render(leftovers.filter((l) => !closed.has(l.id)));
}

function causeText(l: Leftover): string {
  return l.cause === 'timeout'
    ? tx('git_lock_cause_timeout', 'A Git command the Hub stopped at its time limit')
    : tx('git_lock_cause_session', 'Session #{id} ended while Git was working', { id: l.session });
}

function refusalText(code: string, detail: string): string {
  switch (code) {
    case 'git_running':
      return tx('git_lock_refused_running', 'Git is running in this repository, so the lock was kept. Try again when it finishes.');
    case 'lock_changed':
      return tx('git_lock_refused_changed', 'The lock changed after it was listed; a Git command is using it, so it was kept.');
    case 'git_unverifiable':
      return tx('git_lock_refused_unverifiable', 'Running Git commands could not be checked, so the lock was kept.');
    case 'lock_gone':
      return tx('git_lock_refused_gone', 'The lock is already gone.');
    default:
      return tx('git_lock_refused_failed', 'The lock could not be removed: {detail}', { detail });
  }
}

function render(leftovers: Leftover[]): void {
  if (leftovers.length === 0) {
    banner?.remove();
    banner = null;
    return;
  }
  if (!banner) {
    banner = document.createElement('div');
    banner.className = 'git-lock-banner';
    banner.setAttribute('role', 'status');
    document.body.appendChild(banner);
  }
  const rows = leftovers.map((l) => `
    <li class="git-lock-row" data-lock-id="${Number(l.id)}">
      <div class="git-lock-repo" title="${escapeHtml(l.lock)}">${escapeHtml(l.repository)}</div>
      <div class="git-lock-cause">${escapeHtml(causeText(l))}</div>
      <div class="git-lock-status" data-lock-status hidden></div>
      <div class="git-lock-row-actions">
        <button type="button" class="ui-button ui-button--secondary ui-button--regular git-lock-remove">${escapeHtml(tx('git_lock_remove', 'Remove lock'))}</button>
      </div>
    </li>`).join('');
  banner.innerHTML = `<div class="git-lock-head">
      <div class="git-lock-body">${escapeHtml(tx('git_lock_body', 'Git stopped mid-work and left index.lock behind. Until it is removed, git add / commit in this repository fails.'))}</div>
      <button type="button" class="ui-button ui-button--secondary ui-button--regular git-lock-dismiss" aria-label="${escapeHtml(tx('git_lock_dismiss', 'Close'))}">✕</button>
    </div>
    <ul class="git-lock-list">${rows}</ul>`;
  banner.querySelector('.git-lock-dismiss')?.addEventListener('click', () => {
    leftovers.forEach((l) => closed.add(l.id));
    banner?.remove();
    banner = null;
  });
  banner.querySelectorAll<HTMLElement>('.git-lock-row').forEach((row) => {
    const id = Number(row.dataset.lockId);
    const button = row.querySelector<HTMLButtonElement>('.git-lock-remove');
    button?.addEventListener('click', () => { void remove(id, row, button); });
  });
}

async function remove(id: number, row: HTMLElement, button: HTMLButtonElement): Promise<void> {
  button.disabled = true;
  const status = row.querySelector<HTMLElement>('[data-lock-status]');
  let code = '';
  let message = '';
  try {
    const res = await apiFetch('/api/git-lock-remove', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id }),
    });
    const data = await res.json().catch(() => ({}));
    if (res.ok && data?.ok !== false) {
      showToast(tx('git_lock_removed', 'Removed index.lock'));
    } else {
      code = String(data?.error || '');
      message = refusalText(code, String(data?.detail || `HTTP ${res.status}`));
    }
  } catch (err) {
    message = refusalText('', err instanceof Error ? err.message : String(err));
  }
  // 「もう無い」「書き換わった」は Hub が一覧から外すので、行が消える前にトーストで理由を出す。
  if (message && code !== 'lock_gone' && code !== 'lock_changed' && status) {
    status.textContent = message;
    status.hidden = false;
    button.disabled = false;
    return;
  }
  if (message) showToast(message, undefined, 4000);
  await refreshGitLockNotice();
}
