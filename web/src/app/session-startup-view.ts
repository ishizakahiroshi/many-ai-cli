import { t } from '../i18n.js';
import { escapeHtml, showToast } from './util.js';
import { sessionStartups, terminalHasVisibleContent, type SessionStartup } from './session-startup.js';
import { terminals } from './state.js';
import { providerIconHtml } from './provider-icon.js';
import { uiButtonClass } from './ui-components.js';
import { captureSessionListTab } from './session-list-tabs.js';

const slowTimers = new Map<string, ReturnType<typeof setTimeout>>();
const reviewActions = new Map<string, () => void>();
export function startupViewHtml(value: SessionStartup): string {
  const title = value.phase === 'failed' ? t('startup_failed') : t('startup_starting', { provider: value.name });
  const message = value.phase === 'failed' ? t('startup_failed_hint') : value.phase === 'confirming'
    ? t('startup_confirming') : value.slow ? t('startup_slow') : t('startup_other_panes');
  return `<div class="session-startup-content"><span class="session-startup-icon" aria-hidden="true">${providerIconHtml(value.provider)}</span>` +
    `<span class="session-startup-spinner" aria-hidden="true"${value.phase === 'failed' ? ' hidden' : ''}></span>` +
    `<strong>${escapeHtml(title)}</strong><p>${escapeHtml(message)}</p>` +
    (value.detail ? `<details><summary>${escapeHtml(t('startup_details'))}</summary><p>${escapeHtml(value.detail)}</p></details>` : '') +
    (value.phase === 'failed' ? `<button type="button" class="${uiButtonClass('secondary', 'compact')}" data-startup-review>${escapeHtml(t('startup_review'))}</button>` : '') + '</div>';
}
export function mountStartupView(host: HTMLElement, requestId: string): HTMLElement | null {
  const value = sessionStartups.requests.get(requestId);
  if (!value || value.phase === 'output') return null;
  const root = document.createElement('div');
  root.className = 'session-startup'; root.dataset.startupRequest = requestId;
  root.setAttribute('role', 'status'); root.setAttribute('aria-live', 'polite');
  host.appendChild(root); updateRoot(root, value); return root;
}
function updateRoot(root: HTMLElement, value: SessionStartup): void {
  const renderKey = JSON.stringify([value.phase, value.detail, value.slow, value.name, value.provider]);
  if (root.dataset.startupRenderKey === renderKey) return;
  root.dataset.startupRenderKey = renderKey;
  root.dataset.phase = value.phase; root.innerHTML = startupViewHtml(value);
  root.querySelector('[data-startup-review]')?.addEventListener('click', () => {
    reviewActions.get(value.requestId)?.();
  });
}
export function mountTerminalStartup(host: HTMLElement, id: number, startedAt: string): void {
  const value = sessionStartups.forSession({ id, startedAt });
  if (!value || value.phase === 'output' || host.querySelector('.session-startup')) return;
  // Output may have been parsed before the metadata/correlation or DOM attach.
  if (startedAt && terminalHasVisibleContent(terminals.get(id)?.term)) {
    sessionStartups.outputPainted({ id, startedAt }); return;
  }
  host.classList.add('session-startup-host'); mountStartupView(host, value.requestId);
}
/** Where a launch's feedback went. The caller decides from this what to do with the form and with the new session. */
export type StartupPlacement =
  /** A startup pane in the open multi grid (or the user left this launch's box): close the form now, do not open the session later. */
  | 'pane'
  /** No pane: the new session's own terminal carries the feedback once it is opened (single view, per-session workspace). */
  | 'terminal'
  /** The open multi grid has no empty pane: say so, then open the session as usual. */
  | 'no-space';
/** The multi grid the user is working in. The single view and the per-session workspace (also a visible multi view) are not it. */
function multiGridOpen(): boolean {
  const multi = document.getElementById('multi-view');
  return !!multi && !multi.hidden && multi.dataset.paneMode !== 'workspace';
}
export function beginSessionStartup(requestId: string, provider: string, name: string, tabId: string, review?: () => void): StartupPlacement {
  if (review) reviewActions.set(requestId, review);
  sessionStartups.begin(requestId, provider, name, tabId);
  slowTimers.set(requestId, setTimeout(() => { slowTimers.delete(requestId); sessionStartups.markSlow(requestId); }, 10000));
  // A confirmation may have stayed open while the user selected another box.
  // Keep its launch correlated to the captured tab, without taking over the new view.
  if (captureSessionListTab() !== tabId) return 'pane';
  // Only a multi grid the user already has open grows a pane. From the single view the launch stays there: splitting the
  // terminal being looked at into a multi grid leaves the new session beside the old one in a view the user did not choose
  // (bugfix_spawn-opens-multi-view_2026-11-04.md). The caller opens the session once registered, as before the startup pane existed.
  if (!multiGridOpen()) return 'terminal';
  if (!window.multiPaneManager?.addStartup?.(requestId)) { sessionStartups.dismiss(requestId); return 'no-space'; }
  return 'pane';
}
/** Forget a launch that never got a visible place and never produced a session, so no stale request stays in the store. */
export function dropSessionStartup(requestId: string): void { sessionStartups.dismiss(requestId); }
export function noteSessionStartupOutcome(requestId: string, definiteFailure: boolean, detail = ''): void {
  const request = sessionStartups.requests.get(requestId);
  sessionStartups.outcome(requestId, definiteFailure, detail);
  const visible = Array.from(document.querySelectorAll<HTMLElement>('.session-startup')).some(root =>
    root.dataset.startupRequest === requestId && root.getClientRects().length > 0);
  // A changed tab or a dismissed placeholder must not turn rejection into silence.
  if (!visible && !request?.session) showToast(definiteFailure ? `${t('startup_failed')} ${detail}`.trim() : t('startup_confirming'));
}
sessionStartups.subscribe(() => {
  const manager = window.multiPaneManager;
  manager?.reconcileStartups?.(sessionStartups.requests);
  document.querySelectorAll<HTMLElement>('.session-startup').forEach(root => {
    const value = sessionStartups.requests.get(root.dataset.startupRequest || '');
    if (!value || value.phase === 'output') { root.remove(); return; }
    updateRoot(root, value);
  });
  for (const [id, timer] of slowTimers) {
    const value = sessionStartups.requests.get(id);
    if (!value || value.phase === 'output' || value.phase === 'failed') { clearTimeout(timer); slowTimers.delete(id); }
  }
  for (const id of reviewActions.keys()) {
    const value = sessionStartups.requests.get(id);
    if (!value || value.phase === 'output') reviewActions.delete(id);
  }
});
