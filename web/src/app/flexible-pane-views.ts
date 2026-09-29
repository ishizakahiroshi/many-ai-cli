// Bind the layout manager to the existing view instances. DOM ids and the
// stateful Files/Git/Review views must never be cloned into multiple panes.
import { FilesTabManager } from './files-view.js';
import { mountChatPaneForSession } from './chat-history.js';
import { activeSessionId, sessions } from './state.js';
import { setActiveTab } from './settings.js';
import { refreshHistoryLite } from './history-lite.js';
import type { PaneContent } from './flexible-pane-state.js';

type Placement =
  | { kind: 'session'; sessionId: number }
  | { kind: 'tab'; tabName: string; sessionId: number | null };

const singleRoots: Record<string, string> = {
  chat: 'chat-pane',
  approval: 'approval-pane',
  history: 'history-pane',
  orchestration: 'orchestration-dashboard-pane',
};
const originalHomes = new Map<HTMLElement, { parent: Node; next: Node | null }>();

function rootFor(tabName: string): HTMLElement | null {
  const id = singleRoots[tabName];
  return id ? document.getElementById(id) : null;
}

function rememberHome(root: HTMLElement): void {
  if (originalHomes.has(root) || !root.parentNode) return;
  originalHomes.set(root, { parent: root.parentNode, next: root.nextSibling });
}

function restoreRoot(root: HTMLElement, host: HTMLElement): void {
  if (root.parentNode !== host) return;
  const home = originalHomes.get(root);
  if (!home) return;
  if (home.next?.parentNode === home.parent) home.parent.insertBefore(root, home.next);
  else home.parent.appendChild(root);
}

function mountView(descriptor: PaneContent, host: HTMLElement): void {
  if (descriptor.kind !== 'tab') return;
  const { tabName, sessionId } = descriptor;
  if (tabName === 'files' || tabName === 'git' || tabName === 'review') {
    if (sessionId == null || !FilesTabManager.mountPaneContent(tabName, sessionId, host)) {
      host.textContent = '画面を表示できません';
    }
    return;
  }
  const root = rootFor(tabName);
  if (!root) {
    host.textContent = '画面を表示できません';
    return;
  }
  rememberHome(root);
  host.appendChild(root);
  if (tabName === 'chat' && sessionId != null) mountChatPaneForSession(sessionId);
  if (tabName === 'history') void refreshHistoryLite();
  if (tabName === 'orchestration') (window as any).renderOrchestrationDashboard?.();
}

function unmountView(descriptor: PaneContent, host: HTMLElement): void {
  if (descriptor.kind !== 'tab') return;
  if (['files', 'git', 'review'].includes(descriptor.tabName)) {
    FilesTabManager.unmountPaneContent(host);
    return;
  }
  const root = rootFor(descriptor.tabName);
  if (root) restoreRoot(root, host);
}

function currentView(): Placement | null {
  const area = document.getElementById('display-area');
  if (!area || area.hidden) return null;
  const mode = Array.from(area.classList).find(name => name.startsWith('mode-'))?.slice(5);
  if (!mode) return null;
  if (mode === 'terminal') {
    return activeSessionId != null && sessions.has(activeSessionId)
      ? { kind: 'session', sessionId: activeSessionId } : null;
  }
  if (['approval', 'history', 'orchestration'].includes(mode)) {
    return { kind: 'tab', tabName: mode, sessionId: null };
  }
  return activeSessionId != null && sessions.has(activeSessionId)
    ? { kind: 'tab', tabName: mode, sessionId: activeSessionId } : null;
}

function normalizePlacement(raw: any): Placement | null {
  if (!raw || typeof raw !== 'object') return null;
  const id = raw.sessionId == null ? null : Number(raw.sessionId);
  if (raw.kind === 'session') {
    return id != null && sessions.has(id) ? { kind: 'session', sessionId: id } : null;
  }
  if (raw.kind !== 'tab') return null;
  const name = String(raw.tabName || '');
  if (!['terminal', 'chat', 'files', 'git', 'review', 'approval', 'history', 'orchestration'].includes(name)) return null;
  if (name === 'terminal') return id != null && sessions.has(id) ? { kind: 'session', sessionId: id } : null;
  if (['approval', 'history', 'orchestration'].includes(name)) {
    return { kind: 'tab', tabName: name, sessionId: null };
  }
  return id != null && sessions.has(id) ? { kind: 'tab', tabName: name, sessionId: id } : null;
}

function commitPlacement(detail: any): void {
  const mgr = (window as any).multiPaneManager;
  if (!mgr) return;
  const multi = document.getElementById('multi-view');
  const isOpen = !!multi && !multi.hidden;
  if (detail?.kind === 'move') {
    if (isOpen && Number.isInteger(detail.fromIdx) && Number.isInteger(detail.targetIdx)) {
      mgr.moveSlot(detail.fromIdx, detail.targetIdx);
      mgr.focusSlot(detail.targetIdx);
    }
    return;
  }
  const incoming = normalizePlacement(detail);
  if (!incoming) return;
  if (incoming.kind === 'tab' && incoming.sessionId != null) {
    const owner = incoming.sessionId;
    const existing = currentView();
    const hadWorkspace = mgr.hasSessionWorkspace?.(owner);
    if (!mgr.setWorkspaceSession?.(owner)) return;
    if (!hadWorkspace) {
      const first = existing?.kind === 'tab' && existing.sessionId === owner &&
        existing.tabName !== incoming.tabName ? existing : incoming;
      if (!mgr.beginAutoSplit?.({ kind: 'session', sessionId: owner }, first)) return;
      if (first !== incoming) mgr.addTab(incoming.tabName, owner, detail?.targetIdx);
    } else {
      mgr.addTab(incoming.tabName, owner, detail?.targetIdx);
    }
    if (activeSessionId !== owner) (window as any).activateSessionForMultiPane?.(owner);
    setActiveTab(owner, 'terminal');
    mgr.picker?.hide();
    return;
  }
  if (isOpen && mgr.workspaceSession) setActiveTab(activeSessionId, 'multi');
  if (!isOpen) {
    const existing = currentView();
    if (existing && JSON.stringify(existing) === JSON.stringify(incoming)) return;
    if (existing && mgr.beginAutoSplit?.(existing, incoming)) {
      setActiveTab(activeSessionId, 'multi');
      mgr.picker?.hide();
      mgr.focusSlot(1);
      return;
    }
    setActiveTab(activeSessionId, 'multi');
    mgr.picker?.hide();
  }
  const placed = incoming.kind === 'session'
    ? mgr.addSession(incoming.sessionId, detail?.targetIdx)
    : mgr.addTab(incoming.tabName, incoming.sessionId, detail?.targetIdx);
  if (placed) mgr.focusSlot(mgr.focusedIdx);
}

export function initFlexiblePaneViews(): void {
  const hostWindow = window as any;
  hostWindow.mountFlexiblePaneView = mountView;
  hostWindow.unmountFlexiblePaneView = unmountView;
  hostWindow.currentFlexiblePaneView = currentView;
  window.addEventListener('pane-placement-commit', event => commitPlacement((event as CustomEvent).detail));
  window.addEventListener('pane-placement-request', event => {
    const detail = (event as CustomEvent).detail;
    if (detail?.source === 'display-area') commitPlacement({ ...(detail.payload || detail), targetIdx: detail.targetIdx });
  });
  window.addEventListener('flexible-pane-auto-collapse', event => {
    const remaining = (event as CustomEvent).detail?.remaining as PaneContent | undefined;
    if (!remaining) return;
    const multiView = document.getElementById('multi-view');
    if (!multiView || multiView.hidden) return;
    const mgr = hostWindow.multiPaneManager;
    if (mgr?.workspaceSession) mgr.clearSinglePaneWorkspace?.();
    const sid = remaining.sessionId;
    if (sid != null && sessions.has(sid)) (window as any).activateSessionForMultiPane?.(sid);
    setActiveTab(sid ?? activeSessionId, remaining.kind === 'session' ? 'terminal' : remaining.tabName);
  });
  const multi = document.getElementById('multi-view');
  if (multi && !multi.hidden) hostWindow.multiPaneManager?.render();
}
