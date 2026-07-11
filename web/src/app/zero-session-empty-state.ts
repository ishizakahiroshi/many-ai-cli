import { t } from '../i18n.js';
import { apiFetch, showToast } from './util.js';
import type { SessionSnapshot } from '../types/proto.js';

type RestoreSpec = Pick<SessionSnapshot, 'provider' | 'cwd' | 'label' | 'note'>;

const LAST_SET_KEY = 'many-ai-cli:last-session-set:v1';
let activeSet = new Map<number, RestoreSpec>();
let hadEmptyState = true;
let root: HTMLElement | null = null;
let pendingRestoreNotes: RestoreSpec[] = [];
let restoredSessionIDs = new Set<number>();

function readLastSet(): RestoreSpec[] {
  try {
    const value = JSON.parse(localStorage.getItem(LAST_SET_KEY) || '[]');
    return Array.isArray(value) ? value.filter((item): item is RestoreSpec =>
      item && typeof item.provider === 'string' && typeof item.cwd === 'string') : [];
  } catch (_) {
    return [];
  }
}

function writeLastSet(): void {
  const set = [...activeSet.values()];
  if (set.length === 0) return;
  try { localStorage.setItem(LAST_SET_KEY, JSON.stringify(set)); } catch (_) {}
}

function syncLastSet(sessions: Iterable<SessionSnapshot>): boolean {
  const current = [...sessions];
  if (current.length === 0) {
    if (!hadEmptyState) writeLastSet();
    activeSet.clear();
    hadEmptyState = true;
    return true;
  }
  if (hadEmptyState) activeSet.clear();
  hadEmptyState = false;
  applyPendingRestoreNotes(current);
  for (const session of current) {
    if (!session.provider || !session.cwd) continue;
    activeSet.set(session.id, {
      provider: session.provider,
      cwd: session.cwd,
      label: session.label || '',
      note: session.note || '',
    });
  }
  return false;
}

function applyPendingRestoreNotes(current: SessionSnapshot[]): void {
  if (pendingRestoreNotes.length === 0) return;
  const newSessions = current.filter((session) => !restoredSessionIDs.has(session.id))
    .sort((a, b) => a.id - b.id);
  for (const session of newSessions) {
    const spec = pendingRestoreNotes.shift();
    if (!spec) break;
    restoredSessionIDs.add(session.id);
    if (!spec.note) continue;
    void apiFetch(`/api/sessions/${encodeURIComponent(String(session.id))}/meta`, {
      method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ note: spec.note }),
    });
  }
}

async function startClaude(): Promise<void> {
  const info = await apiFetch('/api/info');
  const { cwd } = info.ok ? await info.json() : {};
  if (!cwd) throw new Error('cwd unavailable');
  const response = await apiFetch('/api/spawn', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ provider: 'claude', cwd, label: 'First session' }),
  });
  if (!response.ok) throw new Error(`spawn ${response.status}`);
}

async function restoreLastSet(button: HTMLButtonElement): Promise<void> {
  const specs = readLastSet();
  if (specs.length === 0) return;
  button.disabled = true;
  try {
    for (const spec of specs) {
      const response = await apiFetch('/api/spawn', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ provider: spec.provider, cwd: spec.cwd, label: spec.label || '' }),
      });
      if (!response.ok) throw new Error(`spawn ${response.status}`);
    }
    pendingRestoreNotes = specs.slice();
    restoredSessionIDs.clear();
    showToast(t('zero_session_restore_started'));
  } catch (_) {
    showToast(t('zero_session_restore_failed'));
    button.disabled = false;
  }
}

function showDemo(): void {
  if (!root) return;
  root.classList.add('zero-session--demo');
  root.innerHTML = `
    <section class="zero-session-demo" aria-live="polite">
      <div class="zero-session-kicker">${t('zero_session_demo_kicker')}</div>
      <h2>${t('zero_session_demo_title')}</h2>
      <p>${t('zero_session_demo_body')}</p>
      <div class="zero-session-approval">
        <span class="zero-session-approval-dot" aria-hidden="true"></span>
        <div><strong>${t('zero_session_demo_approval_title')}</strong><small>${t('zero_session_demo_approval_body')}</small></div>
        <button type="button" class="zero-session-primary" data-demo-approve>${t('zero_session_demo_approve')}</button>
      </div>
      <button type="button" class="zero-session-link" data-demo-close>${t('zero_session_demo_close')}</button>
    </section>`;
  root.querySelector<HTMLButtonElement>('[data-demo-approve]')?.addEventListener('click', (event) => {
    const approve = event.currentTarget as HTMLButtonElement;
    approve.disabled = true;
    approve.textContent = t('zero_session_demo_approved');
    window.setTimeout(() => (document.getElementById('new-session-btn') as HTMLButtonElement | null)?.click(), 550);
  });
  root.querySelector('[data-demo-close]')?.addEventListener('click', renderEmptyState);
}

async function updateClaudeAvailability(button: HTMLButtonElement, hint: HTMLElement): Promise<void> {
  try {
    const response = await apiFetch('/api/doctor');
    if (!response.ok) return;
    const report = await response.json();
    const provider = Array.isArray(report.checks) ? report.checks.find((check: { name?: string }) => check.name === 'provider') : null;
    if (provider?.level === 'FAIL') {
      button.disabled = true;
      hint.textContent = t('zero_session_claude_missing');
      hint.hidden = false;
      hint.addEventListener('click', () => (document.getElementById('settings-btn') as HTMLButtonElement | null)?.click());
    }
  } catch (_) {}
}

function renderEmptyState(): void {
  if (!root) return;
  const hasLastSet = readLastSet().length > 0;
  root.className = 'zero-session';
  root.innerHTML = `
    <section class="zero-session-card" aria-labelledby="zero-session-title">
      <div class="zero-session-kicker">${t('zero_session_kicker')}</div>
      <h1 id="zero-session-title">${t('zero_session_title')}</h1>
      <p class="zero-session-intro">${t('zero_session_intro')}</p>
      <div class="zero-session-actions">
        <button type="button" class="zero-session-action zero-session-action--primary" data-zero-claude>
          <strong>${t('zero_session_claude')}</strong><span>${t('zero_session_claude_desc')}</span>
        </button>
        <button type="button" class="zero-session-action" data-zero-restore ${hasLastSet ? '' : 'disabled'}>
          <strong>${t('zero_session_restore')}</strong><span>${hasLastSet ? t('zero_session_restore_desc') : t('zero_session_restore_empty')}</span>
        </button>
        <button type="button" class="zero-session-action" data-zero-demo>
          <strong>${t('zero_session_demo')}</strong><span>${t('zero_session_demo_desc')}</span>
        </button>
      </div>
      <button type="button" class="zero-session-doctor" data-zero-doctor hidden></button>
      <div class="zero-session-footer"><button type="button" data-zero-tour>${t('zero_session_tour')}</button><span>·</span><button type="button" data-zero-docs>${t('zero_session_docs')}</button></div>
    </section>`;
  const claude = root.querySelector<HTMLButtonElement>('[data-zero-claude]')!;
  const doctorHint = root.querySelector<HTMLElement>('[data-zero-doctor]')!;
  void updateClaudeAvailability(claude, doctorHint);
  claude.addEventListener('click', async () => {
    claude.disabled = true;
    try { await startClaude(); showToast(t('zero_session_claude_started')); }
    catch (_) { showToast(t('zero_session_claude_failed')); claude.disabled = false; }
  });
  root.querySelector<HTMLButtonElement>('[data-zero-restore]')?.addEventListener('click', (event) => void restoreLastSet(event.currentTarget as HTMLButtonElement));
  root.querySelector('[data-zero-demo]')?.addEventListener('click', showDemo);
  root.querySelector('[data-zero-tour]')?.addEventListener('click', () => (document.getElementById('first-run-tour-btn') as HTMLButtonElement | null)?.click());
  root.querySelector('[data-zero-docs]')?.addEventListener('click', () => window.open('https://github.com/ishizakahiroshi/many-ai-cli#readme', '_blank', 'noopener'));
}

export function renderZeroSessionEmptyState(sessions: Iterable<SessionSnapshot>): void {
  const isEmpty = syncLastSet(sessions);
  const wrapper = document.getElementById('terminal-area-wrapper');
  if (!wrapper) return;
  if (!isEmpty) { root?.remove(); root = null; return; }
  if (!root) {
    root = document.createElement('div');
    root.id = 'zero-session-empty-state';
    wrapper.appendChild(root);
  }
  renderEmptyState();
}
