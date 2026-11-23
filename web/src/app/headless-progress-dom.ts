// 画面なし実行の進みを描く DOM の口。サイドバーのカード・開いているセッションの帯・
// ペインが同じ関数で同じセッションの値を描く（値の計算は headless-progress.ts だけが持つ）。
import { t } from '../i18n.js';
import { activeSessionId, sessions } from './state.js';
import { executionModeKind } from './session-launch-declaration.js';
import { headlessProgressText, renderHeadlessProgressValues } from './headless-progress.js';

/** 画面なしで動くセッションか（申告が正本。空・未申告は対話）。 */
export function isHeadlessSession(session: { execution_mode?: unknown } | null | undefined): boolean {
  return executionModeKind(session?.execution_mode) === 'headless';
}

export function createHeadlessProgressElement(id: number): HTMLElement {
  const el = document.createElement('div');
  el.className = 'headless-progress';
  el.dataset.headlessSession = String(id);
  renderHeadlessProgressElement(el, id);
  return el;
}

function renderHeadlessProgressElement(el: HTMLElement, id: number): void {
  const session = sessions.get(id);
  el.hidden = !isHeadlessSession(session);
  if (el.hidden) return;
  // ターン・道具・最後の発言を数えられるのは Claude の stream だけ。ほかは「未取得」のまま。
  const values = headlessProgressText(session?.headless_progress, t, performance.now(), session?.provider === 'claude');
  renderHeadlessProgressValues(el, values);
}

/** 開いているセッションの帯と、画面にある全カード・ペインの進みを描き直す。 */
export function refreshHeadlessProgress(): void {
  const active = document.getElementById('active-headless-progress');
  if (active) renderHeadlessProgressElement(active, activeSessionId ?? 0);
  document.querySelectorAll<HTMLElement>('[data-headless-session]').forEach(el => {
    renderHeadlessProgressElement(el, Number(el.dataset.headlessSession));
  });
}
