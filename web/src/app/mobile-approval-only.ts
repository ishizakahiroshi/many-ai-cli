// P-81: PWA shortcut destination for clearing approvals with the smallest
// possible surface. It deliberately reuses sendChoice and P-09's guard.

import { approvalQuestionContext, pendingSessionIds, sessionTitle } from './approval-queue-tab.js';
import { bindHighRiskApproval, sendChoice } from './approval.js';
import { approvalRawOptionsCache, sessions } from './state.js';

const ROUTE = '/mobile/approvals';
const MODE_KEY = 'many-ai-cli.mobile.approval-only';
const approveRe = /\b(yes|approve|allow|continue|proceed|confirm|accept|run|execute)\b/i;
const rejectRe = /\b(no|deny|reject|skip|cancel|abort|decline)\b|don't\s+allow/i;

function isApprovalRoute(): boolean {
  return window.location.pathname.replace(/\/+$/, '') === ROUTE;
}

function setMode(value: 'approval' | 'full'): void {
  try { localStorage.setItem(MODE_KEY, value); } catch (_) {}
}

function mode(): string {
  try { return localStorage.getItem(MODE_KEY) || ''; } catch (_) { return ''; }
}

function fullHubUrl(): string {
  return `${window.location.origin}/${window.location.search}${window.location.hash}`;
}

function approvalUrl(): string {
  return `${window.location.origin}${ROUTE}${window.location.search}${window.location.hash}`;
}

function selectOptions(id: number): { approve: any | null; reject: any | null } {
  const options = approvalRawOptionsCache.get(id);
  if (!Array.isArray(options) || !(options as any[]).length || (options as any[]).some((option: any) => Array.isArray(option?.options))) {
    return { approve: null, reject: null };
  }
  const usable = (options as any[]).filter((option: any) => option && typeof option.num === 'number');
  const reject = usable.find((option: any) => rejectRe.test(String(option.label || ''))) || null;
  const approve = usable.find((option: any) => approveRe.test(String(option.label || '')) && option !== reject)
    || usable.find((option: any) => option !== reject)
    || null;
  return { approve, reject };
}

function optionButton(text: string, className: string, onClick: () => void): HTMLButtonElement {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = className;
  button.textContent = text;
  button.addEventListener('click', onClick);
  return button;
}

function renderApprovalOnly(): void {
  const root = document.getElementById('mobile-approval-only');
  if (!root) return;
  root.replaceChildren();

  const heading = document.createElement('header');
  heading.className = 'mao-header';
  const title = document.createElement('div');
  title.innerHTML = '<strong>承認キュー</strong><span>外出先の詰まりを片づける</span>';
  const full = optionButton('フル Hub へ', 'mao-full-hub', () => {
    setMode('full');
    window.location.assign(fullHubUrl());
  });
  heading.append(title, full);
  root.appendChild(heading);

  const ids = pendingSessionIds();
  if (ids.length === 0) {
    const empty = document.createElement('section');
    empty.className = 'mao-empty';
    empty.innerHTML = '<strong>✓ 詰まりはありません</strong><span>すべてのセッションが進行できます。</span>';
    root.appendChild(empty);
    return;
  }

  const list = document.createElement('main');
  list.className = 'mao-list';
  for (const id of ids) {
    const options = approvalRawOptionsCache.get(id);
    const card = document.createElement('section');
    card.className = 'mao-card';
    const summary = Array.isArray(options) ? (options as any)._summary : null;
    const risk = summary?.risk === 'high' ? 'HIGH' : summary?.risk === 'low' ? 'LOW' : 'MID';
    const cardHeader = document.createElement('div');
    cardHeader.className = 'mao-card-header';
    cardHeader.innerHTML = `<span class="mao-risk mao-risk-${risk.toLowerCase()}">${risk}</span><strong>${escapeHtml(sessionTitle(sessions.get(id)))}</strong><span>#${id}</span>`;
    card.appendChild(cardHeader);
    const context = approvalQuestionContext(options);
    const prompt = document.createElement('p');
    prompt.className = 'mao-prompt';
    prompt.textContent = context.question || context.preamble || '承認待ちの内容を確認しています';
    card.appendChild(prompt);
    const selected = selectOptions(id);
    const actions = document.createElement('div');
    actions.className = 'mao-actions';
    if (selected.approve) {
      const approve = optionButton('Approve', 'mao-approve', () => sendChoice(id, selected.approve.num));
      // P-09: high-risk positive actions must be held or confirmed.
      if (summary?.risk === 'high' && !rejectRe.test(String(selected.approve.label || ''))) {
        bindHighRiskApproval(approve, id, selected.approve.num);
      }
      actions.appendChild(approve);
    }
    if (selected.reject) actions.appendChild(optionButton('Reject', 'mao-reject', () => sendChoice(id, selected.reject.num)));
    if (!selected.approve && !selected.reject) {
      const fallback = optionButton('フル Hub で回答', 'mao-full-hub', () => {
        setMode('full');
        window.location.assign(fullHubUrl());
      });
      actions.appendChild(fallback);
    }
    card.appendChild(actions);
    list.appendChild(card);
  }
  root.appendChild(list);
}

function escapeHtml(value: string): string {
  return String(value).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

export function initMobileApprovalOnly(): void {
  const route = isApprovalRoute();
  if (!route && mode() === 'approval') {
    window.location.replace(approvalUrl());
    return;
  }
  if (!route) return;
  setMode('approval');
  document.body.classList.add('mobile-approval-only-view');
  const root = document.createElement('div');
  root.id = 'mobile-approval-only';
  document.body.appendChild(root);
  renderApprovalOnly();
  window.addEventListener('approval-queue-updated', renderApprovalOnly);
}
