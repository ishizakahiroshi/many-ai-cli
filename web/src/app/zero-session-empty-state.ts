import { t } from '../i18n.js';
import type { SessionSnapshot } from '../types/proto.js';

let root: HTMLElement | null = null;

function renderGuide(): void {
  if (!root) return;
  root.className = 'zero-session';
  root.innerHTML = `
    <section class="zero-session-card" aria-labelledby="zero-session-title">
      <div class="zero-session-kicker">${t('zero_session_kicker')}</div>
      <h1 id="zero-session-title">${t('zero_session_title')}</h1>
      <p class="zero-session-intro">${t('zero_session_intro')}</p>
      <ol class="zero-session-steps">
        <li><div><strong>${t('zero_session_step1_title')}</strong><span>${t('zero_session_step1_body')}</span></div></li>
        <li><div><strong>${t('zero_session_step2_title')}</strong><span>${t('zero_session_step2_body')}</span></div></li>
        <li><div><strong>${t('zero_session_step3_title')}</strong><span>${t('zero_session_step3_body')}</span></div></li>
      </ol>
      <div class="zero-session-footer"><button type="button" data-zero-tour>${t('zero_session_tour')}</button><span>·</span><button type="button" data-zero-docs>${t('zero_session_docs')}</button></div>
    </section>`;
  root.querySelector('[data-zero-tour]')?.addEventListener('click', () => (document.getElementById('first-run-tour-btn') as HTMLButtonElement | null)?.click());
  root.querySelector('[data-zero-docs]')?.addEventListener('click', () => window.open('https://github.com/ishizakahiroshi/many-ai-cli#readme', '_blank', 'noopener'));
}

export function renderZeroSessionEmptyState(sessions: Iterable<SessionSnapshot>): void {
  const isEmpty = [...sessions].length === 0;
  const wrapper = document.getElementById('terminal-area-wrapper');
  if (!wrapper) return;
  if (!isEmpty) { root?.remove(); root = null; return; }
  if (!root) {
    root = document.createElement('div');
    root.id = 'zero-session-empty-state';
    wrapper.appendChild(root);
    renderGuide();
  }
}
