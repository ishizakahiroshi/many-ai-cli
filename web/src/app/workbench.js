import { escapeHtml, showToast, token } from './util.js';
import { activeSessionId, sessions } from './state.js';
import { autoExpand, inputEl, sendSubmittedText } from '../app.js';

const pane = document.getElementById('workbench-pane');

const wb = {
  loaded: false,
  sessions: [],
  templates: [],
  palette: [],
  policies: [],
  tasks: [],
};

function api(path, opts = {}) {
  const sep = path.includes('?') ? '&' : '?';
  return fetch(`${path}${sep}token=${encodeURIComponent(token || '')}`, opts);
}

function jsonHeaders(body) {
  return {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  };
}

function selectedSessionId() {
  const raw = pane?.querySelector('#wb-session-select')?.value || '';
  const n = parseInt(raw, 10);
  if (Number.isFinite(n) && n > 0) return n;
  return activeSessionId || 0;
}

function sessionLabel(s) {
  if (!s) return '';
  const title = s.title || s.last_message || s.first_message || s.display_name || s.cwd || '';
  const short = title ? ` - ${title}` : '';
  return `#${s.session_id || s.id} ${s.provider || ''}${short}`.slice(0, 120);
}

function insertIntoInput(text) {
  if (!inputEl || !text) return;
  inputEl.value = inputEl.value ? `${inputEl.value}\n\n${text}` : text;
  autoExpand();
  inputEl.focus();
}

function renderWorkbench() {
  if (!pane) return;
  pane.innerHTML = `
    <div class="wb-shell">
      <div class="wb-head">
        <div>
          <h2>Workbench</h2>
          <p>残りの機能案を一つの作業面に集約。セッション履歴、比較、Files/Git、承認、安全、タスク、利用量をここから扱う。</p>
        </div>
        <div class="wb-head-actions">
          <select id="wb-session-select"></select>
          <button type="button" data-wb-action="refresh">更新</button>
        </div>
      </div>

      <div class="wb-grid">
        ${card('timeline', 'Session replay / timeline', '過去イベント、要約、タイトル、タグ、export。', `
          <div class="wb-fields">
            <input id="wb-meta-title" placeholder="session title">
            <input id="wb-meta-tags" placeholder="tags: review, bugfix">
          </div>
          <textarea id="wb-meta-summary" rows="5" placeholder="summary"></textarea>
          <div class="wb-actions">
            <button type="button" data-wb-action="timeline">Timeline</button>
            <button type="button" data-wb-action="summarize">自動要約</button>
            <button type="button" data-wb-action="save-meta">保存</button>
            <button type="button" data-wb-action="export">Export zip</button>
          </div>
          <div id="wb-timeline" class="wb-list"></div>
        `)}

        ${card('compare', '同一プロンプト比較', '同じ依頼を複数セッションへ同時送信。結果比較の入口にする。', `
          <textarea id="wb-compare-prompt" rows="5" placeholder="比較したい依頼文"></textarea>
          <div id="wb-compare-targets" class="wb-checks"></div>
          <div class="wb-actions">
            <button type="button" data-wb-action="compare-send">選択セッションへ送信</button>
            <button type="button" data-wb-action="compare-save">比較メモをタスク化</button>
          </div>
          <div id="wb-compare-result" class="wb-note"></div>
        `)}

        ${card('workspace', 'Hub ワークスペース復元', '選択セッション、タブ、比較対象などを保存して再利用。', `
          <div class="wb-actions">
            <button type="button" data-wb-action="workspace-save">現在状態を保存</button>
            <button type="button" data-wb-action="workspace-load">保存状態を表示</button>
          </div>
          <pre id="wb-workspace-state" class="wb-pre"></pre>
        `)}

        ${card('templates', 'Templates / Prompt palette', '定型プロンプト、playbook、quick command を保存して入力へ投入。', `
          <input id="wb-item-title" placeholder="title">
          <textarea id="wb-item-body" rows="4" placeholder="prompt or playbook body"></textarea>
          <div class="wb-actions">
            <button type="button" data-wb-action="template-save">Template 保存</button>
            <button type="button" data-wb-action="palette-save">Palette 保存</button>
          </div>
          <div class="wb-two-lists">
            <div><h4>Templates</h4><div id="wb-templates" class="wb-list"></div></div>
            <div><h4>Palette</h4><div id="wb-palette" class="wb-list"></div></div>
          </div>
        `)}

        ${card('files', 'Files からコンテキスト投入', '絶対パスを指定して安全範囲内のテキストを prompt snippet 化。', `
          <textarea id="wb-files-paths" rows="5" placeholder="C:\\\\dev\\\\any-ai-cli\\\\internal\\\\hub\\\\server.go"></textarea>
          <div class="wb-actions">
            <button type="button" data-wb-action="files-context">Context 生成</button>
            <button type="button" data-wb-action="files-insert">入力へ追加</button>
          </div>
          <textarea id="wb-files-output" rows="8" readonly></textarea>
          <div id="wb-files-result" class="wb-list"></div>
        `)}

        ${card('git', 'Git review / watcher / worktree', '変更レビュー、ファイル watcher、worktree launcher。', `
          <div class="wb-actions">
            <button type="button" data-wb-action="git-review">Git review</button>
            <button type="button" data-wb-action="file-watch">File watcher</button>
            <button type="button" data-wb-action="worktrees">Worktrees</button>
          </div>
          <div class="wb-fields">
            <input id="wb-worktree-branch" placeholder="new branch">
            <input id="wb-worktree-path" placeholder="absolute worktree path">
          </div>
          <div class="wb-actions">
            <button type="button" data-wb-action="worktree-create">Worktree 作成</button>
          </div>
          <pre id="wb-git-output" class="wb-pre"></pre>
        `)}

        ${card('safety', '安全 / 承認 / 診断 / 通知', 'redaction preview、approval simulator、policy profiles、provider health。', `
          <textarea id="wb-redaction-input" rows="4" placeholder="mask preview text"></textarea>
          <div class="wb-actions">
            <button type="button" data-wb-action="redact">Mask preview</button>
            <button type="button" data-wb-action="approval-sim">Approval simulate</button>
            <button type="button" data-wb-action="diagnostics">Diagnostics</button>
            <button type="button" data-wb-action="notify-enable">通知許可</button>
          </div>
          <pre id="wb-safety-output" class="wb-pre"></pre>
          <h4>Policy profiles</h4>
          <div id="wb-policies" class="wb-list"></div>
        `)}

        ${card('tasks', 'Task board / usage / stale cleanup', 'セッションの手動タスク管理、利用量集計、放置セッション確認。', `
          <div class="wb-fields">
            <input id="wb-task-title" placeholder="task title">
            <select id="wb-task-status">
              <option value="todo">todo</option>
              <option value="doing">doing</option>
              <option value="waiting">waiting</option>
              <option value="done">done</option>
            </select>
          </div>
          <textarea id="wb-task-body" rows="3" placeholder="task note"></textarea>
          <div class="wb-actions">
            <button type="button" data-wb-action="task-save">Task 保存</button>
            <button type="button" data-wb-action="usage">Usage</button>
            <button type="button" data-wb-action="stale">Stale sessions</button>
            <button type="button" data-wb-action="test-results">Test watcher</button>
          </div>
          <div id="wb-tasks" class="wb-board"></div>
          <pre id="wb-ops-output" class="wb-pre"></pre>
        `)}
      </div>
    </div>
  `;
}

function card(kind, title, desc, body) {
  return `
    <section class="wb-card wb-card-${kind}">
      <div class="wb-card-head">
        <h3>${escapeHtml(title)}</h3>
      </div>
      <p>${escapeHtml(desc)}</p>
      ${body}
    </section>
  `;
}

async function refreshAll() {
  await Promise.all([
    loadSessions(),
    loadItems('templates'),
    loadItems('palette'),
    loadItems('policies'),
    loadItems('tasks'),
  ]);
  loadMeta().catch(() => {});
}

async function loadSessions() {
  const res = await api('/api/workbench/sessions?limit=120&archived=1');
  const data = res.ok ? await res.json() : { sessions: [] };
  const live = Array.from(sessions.values()).map(s => ({ ...s, session_id: s.id }));
  const byId = new Map();
  [...(data.sessions || []), ...live].forEach(s => byId.set(Number(s.session_id || s.id), s));
  wb.sessions = Array.from(byId.values()).sort((a, b) => Number(b.session_id || b.id) - Number(a.session_id || a.id));
  renderSessionSelect();
  renderCompareTargets();
}

function renderSessionSelect() {
  const sel = pane.querySelector('#wb-session-select');
  if (!sel) return;
  sel.innerHTML = wb.sessions.map(s => {
    const id = s.session_id || s.id;
    return `<option value="${id}" ${id === activeSessionId ? 'selected' : ''}>${escapeHtml(sessionLabel(s))}</option>`;
  }).join('');
}

function renderCompareTargets() {
  const root = pane.querySelector('#wb-compare-targets');
  if (!root) return;
  const live = Array.from(sessions.values());
  if (live.length === 0) {
    root.innerHTML = '<span class="wb-muted">ライブセッションがありません</span>';
    return;
  }
  root.innerHTML = live.map(s => `
    <label><input type="checkbox" value="${s.id}" ${s.id === activeSessionId ? 'checked' : ''}> #${s.id} ${escapeHtml(s.provider || '')} ${escapeHtml(s.model || '')}</label>
  `).join('');
}

async function loadMeta() {
  const id = selectedSessionId();
  if (!id) return;
  const res = await api(`/api/workbench/session-meta?session_id=${id}`);
  if (!res.ok) return;
  const data = await res.json();
  const s = data.session || {};
  pane.querySelector('#wb-meta-title').value = s.title || '';
  pane.querySelector('#wb-meta-tags').value = (s.tags || []).join(', ');
  pane.querySelector('#wb-meta-summary').value = s.summary || '';
}

async function saveMeta() {
  const id = selectedSessionId();
  const body = {
    session_id: id,
    title: pane.querySelector('#wb-meta-title').value,
    tags: pane.querySelector('#wb-meta-tags').value.split(',').map(s => s.trim()).filter(Boolean),
    summary: pane.querySelector('#wb-meta-summary').value,
  };
  const res = await api('/api/workbench/session-meta', jsonHeaders(body));
  if (!res.ok) throw new Error('save failed');
  showToast('session meta saved');
  await loadSessions();
}

async function loadTimeline() {
  const id = selectedSessionId();
  const res = await api(`/api/workbench/session-timeline?session_id=${id}&limit=120`);
  const data = res.ok ? await res.json() : { events: [] };
  const root = pane.querySelector('#wb-timeline');
  root.innerHTML = (data.events || []).slice(-80).map(ev => `
    <div class="wb-row">
      <b>${escapeHtml(ev.type || 'event')}</b>
      <span>${escapeHtml(ev.ts || '')}</span>
      <code>${escapeHtml(JSON.stringify(ev.payload || {}).slice(0, 180))}</code>
    </div>
  `).join('') || '<div class="wb-empty">timeline はまだありません</div>';
}

async function summarizeSession() {
  const id = selectedSessionId();
  const res = await api('/api/workbench/session-summary', jsonHeaders({ session_id: id }));
  if (!res.ok) throw new Error('summary failed');
  const data = await res.json();
  pane.querySelector('#wb-meta-title').value = data.title || '';
  pane.querySelector('#wb-meta-tags').value = (data.tags || []).join(', ');
  pane.querySelector('#wb-meta-summary').value = data.summary || '';
  showToast('summary updated');
}

function exportSession() {
  const id = selectedSessionId();
  if (!id) return;
  location.href = `/api/workbench/session-export?session_id=${id}&redact=1&token=${encodeURIComponent(token || '')}`;
}

async function compareSend() {
  const text = pane.querySelector('#wb-compare-prompt').value.trim();
  if (!text) return;
  const targets = Array.from(pane.querySelectorAll('#wb-compare-targets input:checked')).map(el => parseInt(el.value, 10));
  targets.forEach(id => sendSubmittedText(id, `${text}\r`));
  pane.querySelector('#wb-compare-result').textContent = `${targets.length} sessions sent`;
}

async function saveCompareTask() {
  const text = pane.querySelector('#wb-compare-prompt').value.trim();
  if (!text) return;
  await saveItem('tasks', {
    title: `Compare: ${text.slice(0, 48)}`,
    body: text,
    status: 'doing',
    tags: ['compare'],
  });
}

async function workspaceSave() {
  const state = {
    active_session_id: activeSessionId,
    selected_session_id: selectedSessionId(),
    tab: 'workbench',
    compare_targets: Array.from(pane.querySelectorAll('#wb-compare-targets input:checked')).map(el => el.value),
    saved_at: new Date().toISOString(),
  };
  const res = await api('/api/workbench/state', jsonHeaders(state));
  if (!res.ok) throw new Error('workspace save failed');
  pane.querySelector('#wb-workspace-state').textContent = JSON.stringify(state, null, 2);
  showToast('workspace saved');
}

async function workspaceLoad() {
  const res = await api('/api/workbench/state');
  const data = res.ok ? await res.json() : {};
  pane.querySelector('#wb-workspace-state').textContent = JSON.stringify(data.state || {}, null, 2);
}

async function loadItems(kind) {
  const res = await api(`/api/workbench/${kind}`);
  const data = res.ok ? await res.json() : { items: [] };
  wb[kind] = data.items || [];
  renderItems(kind);
}

async function saveItem(kind, item) {
  const res = await api(`/api/workbench/${kind}`, jsonHeaders(item));
  if (!res.ok) throw new Error(`${kind} save failed`);
  const data = await res.json();
  wb[kind] = data.items || [];
  renderItems(kind);
  showToast(`${kind} saved`);
}

function renderItems(kind) {
  const idMap = { templates: 'wb-templates', palette: 'wb-palette', policies: 'wb-policies' };
  if (kind === 'tasks') return renderTasks();
  const root = pane.querySelector(`#${idMap[kind]}`);
  if (!root) return;
  root.innerHTML = (wb[kind] || []).map(item => `
    <button type="button" class="wb-item" data-wb-insert="${escapeHtml(item.body || '')}">
      <strong>${escapeHtml(item.title || '(untitled)')}</strong>
      <span>${escapeHtml((item.tags || []).join(', '))}</span>
    </button>
  `).join('') || '<div class="wb-empty">empty</div>';
}

function renderTasks() {
  const root = pane.querySelector('#wb-tasks');
  if (!root) return;
  const groups = ['todo', 'doing', 'waiting', 'done'];
  root.innerHTML = groups.map(status => {
    const items = (wb.tasks || []).filter(item => (item.status || 'todo') === status);
    return `<div class="wb-lane"><h4>${status}</h4>${items.map(item => `
      <div class="wb-task"><strong>${escapeHtml(item.title || '(untitled)')}</strong><span>${escapeHtml(item.body || '')}</span></div>
    `).join('')}</div>`;
  }).join('');
}

async function saveTemplate(kind) {
  const title = pane.querySelector('#wb-item-title').value.trim();
  const body = pane.querySelector('#wb-item-body').value.trim();
  if (!title || !body) return;
  await saveItem(kind, { title, body, tags: [kind] });
}

async function filesContext() {
  const paths = pane.querySelector('#wb-files-paths').value.split(/\r?\n/).map(s => s.trim()).filter(Boolean);
  const res = await api('/api/workbench/files-context', jsonHeaders({ session_id: selectedSessionId(), paths }));
  const data = res.ok ? await res.json() : { items: [], prompt: '' };
  pane.querySelector('#wb-files-output').value = data.prompt || '';
  pane.querySelector('#wb-files-result').innerHTML = (data.items || []).map(item => `
    <div class="wb-row ${item.ok ? '' : 'warn'}"><b>${item.ok ? 'OK' : 'NG'}</b><span>${escapeHtml(item.rel || item.path || '')}</span><code>${escapeHtml(item.error || `${item.size || 0} bytes`)}</code></div>
  `).join('');
}

async function gitAction(action) {
  const id = selectedSessionId();
  let path = `/api/workbench/${action}?session=${id}`;
  const res = await api(path);
  const data = res.ok ? await res.json() : await res.json().catch(() => ({ error: 'failed' }));
  pane.querySelector('#wb-git-output').textContent = JSON.stringify(data, null, 2);
}

async function createWorktree() {
  const body = {
    session_id: selectedSessionId(),
    branch: pane.querySelector('#wb-worktree-branch').value.trim(),
    path: pane.querySelector('#wb-worktree-path').value.trim(),
  };
  const res = await api('/api/workbench/worktrees', jsonHeaders(body));
  const data = res.ok ? await res.json() : await res.json().catch(() => ({ error: 'failed' }));
  pane.querySelector('#wb-git-output').textContent = JSON.stringify(data, null, 2);
}

async function safetyAction(action) {
  const out = pane.querySelector('#wb-safety-output');
  if (action === 'redact') {
    const text = pane.querySelector('#wb-redaction-input').value;
    const res = await api('/api/workbench/redaction-preview', jsonHeaders({ text }));
    out.textContent = JSON.stringify(await res.json(), null, 2);
  } else if (action === 'approval-sim') {
    const text = pane.querySelector('#wb-redaction-input').value;
    const provider = (sessions.get(selectedSessionId()) || {}).provider || 'codex';
    const res = await api('/api/workbench/approval-simulate', jsonHeaders({ provider, text }));
    out.textContent = JSON.stringify(await res.json(), null, 2);
  } else if (action === 'diagnostics') {
    const res = await api('/api/workbench/diagnostics');
    out.textContent = JSON.stringify(await res.json(), null, 2);
  } else if (action === 'notify-enable') {
    if ('Notification' in window) {
      const perm = await Notification.requestPermission();
      out.textContent = `Notification permission: ${perm}`;
    } else {
      out.textContent = 'Notification API is not available';
    }
  }
}

async function saveTask() {
  const title = pane.querySelector('#wb-task-title').value.trim();
  if (!title) return;
  await saveItem('tasks', {
    title,
    body: pane.querySelector('#wb-task-body').value.trim(),
    status: pane.querySelector('#wb-task-status').value,
    tags: ['task'],
  });
}

async function opsAction(action) {
  const map = {
    usage: '/api/workbench/usage',
    stale: '/api/workbench/stale-sessions?hours=24',
    'test-results': '/api/workbench/test-results',
  };
  const res = await api(map[action]);
  pane.querySelector('#wb-ops-output').textContent = JSON.stringify(await res.json(), null, 2);
}

function wireWorkbench() {
  if (!pane) return;
  pane.addEventListener('change', (e) => {
    if (e.target && e.target.id === 'wb-session-select') loadMeta().catch(() => {});
  });
  pane.addEventListener('click', async (e) => {
    const insert = e.target.closest('[data-wb-insert]');
    if (insert) {
      insertIntoInput(insert.dataset.wbInsert || '');
      return;
    }
    const btn = e.target.closest('[data-wb-action]');
    if (!btn) return;
    const action = btn.dataset.wbAction;
    btn.disabled = true;
    try {
      if (action === 'refresh') await refreshAll();
      else if (action === 'timeline') await loadTimeline();
      else if (action === 'summarize') await summarizeSession();
      else if (action === 'save-meta') await saveMeta();
      else if (action === 'export') exportSession();
      else if (action === 'compare-send') await compareSend();
      else if (action === 'compare-save') await saveCompareTask();
      else if (action === 'workspace-save') await workspaceSave();
      else if (action === 'workspace-load') await workspaceLoad();
      else if (action === 'template-save') await saveTemplate('templates');
      else if (action === 'palette-save') await saveTemplate('palette');
      else if (action === 'files-context') await filesContext();
      else if (action === 'files-insert') insertIntoInput(pane.querySelector('#wb-files-output').value);
      else if (action === 'git-review') await gitAction('git-review');
      else if (action === 'file-watch') await gitAction('file-watch');
      else if (action === 'worktrees') await gitAction('worktrees');
      else if (action === 'worktree-create') await createWorktree();
      else if (['redact', 'approval-sim', 'diagnostics', 'notify-enable'].includes(action)) await safetyAction(action);
      else if (action === 'task-save') await saveTask();
      else if (['usage', 'stale', 'test-results'].includes(action)) await opsAction(action);
    } catch (err) {
      showToast(err && err.message ? err.message : 'workbench action failed', btn);
    } finally {
      btn.disabled = false;
    }
  });
}

function initWorkbench() {
  if (!pane || wb.loaded) return;
  wb.loaded = true;
  renderWorkbench();
  wireWorkbench();
  refreshAll().catch(() => {});
}

window.addEventListener('workbench-opened', initWorkbench);
if (pane) {
  initWorkbench();
}
