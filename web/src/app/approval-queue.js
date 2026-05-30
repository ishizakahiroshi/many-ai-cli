import { activeSessionId, approvalRawOptionsCache, approvalVisibleCache, multiQuestionVisibleCache, orderSessions, sessions } from './state.js';
import { activateSession } from './session-list.js';

function btn() {
  return document.getElementById('approval-queue-btn');
}

function panel() {
  return document.getElementById('approval-queue-panel');
}

function countEl() {
  return document.getElementById('approval-queue-count');
}

function pendingSessionIds() {
  return orderSessions()
    .filter(s => s && (approvalVisibleCache.get(s.id) || multiQuestionVisibleCache.get(s.id)))
    .map(s => s.id);
}

function optionPreview(options) {
  if (!Array.isArray(options) || options.length === 0) return '承認内容を検出中';
  const first = options[0];
  if (first && Array.isArray(first.options)) {
    return `${options.length} 件の質問`;
  }
  return options.slice(0, 3)
    .map(o => `${o.num}. ${o.label || o.send_text || ''}`.trim())
    .filter(Boolean)
    .join(' / ') || '承認候補あり';
}

function sessionTitle(s) {
  if (!s) return 'Session';
  const provider = s.provider ? String(s.provider) : 'ai';
  const base = s.label || s.last_message || s.first_message || (s.cwd || '').replace(/\\/g, '/').split('/').pop() || `#${s.id}`;
  return `${provider} #${s.id} ${base}`;
}

function renderQueue() {
  const b = btn();
  const p = panel();
  const c = countEl();
  if (!b || !p || !c) return;
  const ids = pendingSessionIds();
  c.textContent = String(ids.length);
  b.classList.toggle('has-pending', ids.length > 0);
  b.setAttribute('aria-expanded', p.hidden ? 'false' : 'true');
  p.innerHTML = '';

  const head = document.createElement('div');
  head.className = 'approval-queue-head';
  head.textContent = ids.length ? `承認待ち ${ids.length} 件` : '承認待ちはありません';
  p.appendChild(head);

  if (ids.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'approval-queue-empty';
    empty.textContent = '複数セッションを走らせているとき、承認待ちだけがここに集まります。';
    p.appendChild(empty);
    return;
  }

  for (const id of ids) {
    const s = sessions.get(id);
    const row = document.createElement('button');
    row.type = 'button';
    row.className = 'approval-queue-item';
    if (id === activeSessionId) row.classList.add('active');

    const main = document.createElement('span');
    main.className = 'approval-queue-item-main';
    main.textContent = sessionTitle(s);

    const sub = document.createElement('span');
    sub.className = 'approval-queue-item-sub';
    sub.textContent = optionPreview(approvalRawOptionsCache.get(id));

    row.appendChild(main);
    row.appendChild(sub);
    row.addEventListener('click', () => {
      activateSession(id);
      p.hidden = true;
      setTimeout(() => {
        const bar = document.getElementById('action-bar');
        const opts = approvalRawOptionsCache.get(id);
        if (bar && opts && window.approvalUiAdapter && typeof window.approvalUiAdapter.showOptions === 'function') {
          window.approvalUiAdapter.showOptions(bar, id, opts, false, true);
        }
      }, 60);
      renderQueue();
    });
    p.appendChild(row);
  }
}

export function updateApprovalQueue() {
  renderQueue();
}

function setupApprovalQueue() {
  const b = btn();
  const p = panel();
  if (!b || !p || b.dataset.bound === '1') return;
  b.dataset.bound = '1';
  b.addEventListener('click', (e) => {
    e.preventDefault();
    p.hidden = !p.hidden;
    renderQueue();
  });
  document.addEventListener('click', (e) => {
    if (p.hidden) return;
    if (p.contains(e.target) || b.contains(e.target)) return;
    p.hidden = true;
    renderQueue();
  });
  window.addEventListener('approval-queue-updated', renderQueue);
  renderQueue();
  setInterval(renderQueue, 1000);
}

window.updateApprovalQueue = updateApprovalQueue;
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', setupApprovalQueue);
} else {
  setupApprovalQueue();
}
