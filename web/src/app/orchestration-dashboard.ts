import { escapeHtml } from './util.js';
import { sessions } from './state.js';

// C1 (P-18): conductor とその子の現在状態を、既存の session snapshot だけで
// まとめて見せる軽量ダッシュボード。board 本文と集計は C2/C3 で追加する。
let selectedConductorID: number | null = null;

function stateLabel(state: string): string {
  const labels: Record<string, string> = {
    running: '実行中', waiting: '待機', standby: '待機中', completed: '完了',
    error: 'エラー', disconnected: '切断',
  };
  return labels[state] || state || '待機中';
}

function conductorSessions(): any[] {
  return Array.from(sessions.values())
    .filter((session: any) => session.orchestration_id && !session.parent_session_id)
    .sort((a: any, b: any) => Number(b.id) - Number(a.id));
}

function doneFor(child: any): boolean {
  // session_end 経路で Hub が completed にするため、marker の有無に依存しない。
  return child.state === 'completed';
}

function worktreeLabel(child: any): string {
  return String(child.worktree_branch || child.cwd || '既定の作業ディレクトリ');
}

export function renderOrchestrationDashboard(): void {
  const pane = document.getElementById('orchestration-dashboard-pane');
  if (!pane) return;
  const conductors = conductorSessions();
  if (!conductors.some((session: any) => session.id === selectedConductorID)) {
    selectedConductorID = conductors[0]?.id ?? null;
  }

  pane.replaceChildren();
  const heading = document.createElement('div');
  heading.className = 'orchestration-dashboard-heading';
  heading.innerHTML = '<div><h2>Orchestration</h2><p>指揮者と子セッションの現在地</p></div>';
  pane.appendChild(heading);

  if (conductors.length === 0 || selectedConductorID === null) {
    const empty = document.createElement('div');
    empty.className = 'orchestration-dashboard-empty';
    empty.textContent = '実行中または履歴中の orchestration conductor はありません。';
    pane.appendChild(empty);
    return;
  }

  const tabs = document.createElement('div');
  tabs.className = 'orchestration-conductor-tabs';
  conductors.forEach((conductor: any) => {
    const tab = document.createElement('button');
    tab.type = 'button';
    tab.className = 'orchestration-conductor-tab' + (conductor.id === selectedConductorID ? ' active' : '');
    tab.textContent = `#${conductor.id} ${conductor.label || conductor.auto_title || 'conductor'}`;
    tab.title = conductor.cwd || '';
    tab.addEventListener('click', () => { selectedConductorID = conductor.id; renderOrchestrationDashboard(); });
    tabs.appendChild(tab);
  });
  pane.appendChild(tabs);

  const conductor = conductors.find((session: any) => session.id === selectedConductorID);
  if (!conductor) return;
  const detail = document.createElement('div');
  detail.className = 'orchestration-conductor-detail';
  detail.innerHTML = `<span class="orchestration-detail-role">conductor #${conductor.id}</span><span>${escapeHtml(String(conductor.provider || 'unknown'))}</span><span>${escapeHtml(stateLabel(String(conductor.state || 'standby')))}</span><span title="${escapeHtml(String(conductor.board_path || ''))}">${escapeHtml(String(conductor.board_path || 'board 未作成'))}</span>`;
  pane.appendChild(detail);

  const children = Array.from(sessions.values())
    .filter((session: any) => session.parent_session_id === conductor.id)
    .sort((a: any, b: any) => Number(a.id) - Number(b.id));
  const grid = document.createElement('div');
  grid.className = 'orchestration-child-grid';
  children.forEach((child: any) => {
    const done = doneFor(child);
    const card = document.createElement('button');
    card.type = 'button';
    card.className = `orchestration-child-card state-${escapeHtml(String(child.state || 'standby'))}${done ? ' done' : ''}`;
    card.innerHTML = `<div class="orchestration-child-card-head"><strong>${escapeHtml(String(child.role || 'child'))}</strong><span class="orchestration-done-badge ${done ? 'done' : 'pending'}">${done ? 'DONE' : '作業中'}</span></div><dl><div><dt>状態</dt><dd>${escapeHtml(stateLabel(String(child.state || 'standby')))}</dd></div><div><dt>Provider</dt><dd>${escapeHtml(String(child.provider || 'unknown'))}</dd></div><div><dt>Worktree</dt><dd title="${escapeHtml(worktreeLabel(child))}">${escapeHtml(worktreeLabel(child))}</dd></div></dl>`;
    card.addEventListener('click', () => {
      window.dispatchEvent(new CustomEvent('orchestration-dashboard-open-session', { detail: { sessionID: child.id } }));
    });
    grid.appendChild(card);
  });
  if (children.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'orchestration-dashboard-empty';
    empty.textContent = 'この conductor はまだ子セッションを開始していません。';
    grid.appendChild(empty);
  }
  pane.appendChild(grid);
}

window.renderOrchestrationDashboard = renderOrchestrationDashboard;
