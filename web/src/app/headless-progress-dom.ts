import { t } from '../i18n.js';
import { activeSessionId, sessions } from './state.js';
import { headlessProgressText, renderHeadlessProgressValues } from './headless-progress.js';

export function createHeadlessProgressElement(id: number): HTMLElement {
  const el = document.createElement('div');
  el.className = 'headless-progress';
  el.dataset.headlessSession = String(id);
  renderHeadlessProgressElement(el, id);
  return el;
}
function renderHeadlessProgressElement(el: HTMLElement, id: number): void {
  const session = sessions.get(id);
  el.hidden = session?.execution_mode !== 'headless';
  if (el.hidden) return;
  const values = headlessProgressText(session?.headless_progress, t, performance.now(), session?.provider === "claude");
  renderHeadlessProgressValues(el, values);
}
export function refreshHeadlessProgress(): void {
  const active = document.getElementById('active-headless-progress');
  if (active) renderHeadlessProgressElement(active, activeSessionId ?? 0);
  document.querySelectorAll<HTMLElement>('[data-headless-session]').forEach(el => {
    renderHeadlessProgressElement(el, Number(el.dataset.headlessSession));
  });
}
