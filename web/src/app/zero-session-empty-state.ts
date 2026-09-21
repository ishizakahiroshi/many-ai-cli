import { t } from '../i18n.js';
import type { SessionSnapshot } from '../types/proto.js';
import {
  aiCliGuideEntries,
  commandMissingIds,
  hasAvailableAiCli,
  type CliGuideEntry,
} from './cli-availability.js';
import { loadProviderSummaries } from './provider-store.js';
import { escapeHtml, showToast } from './util.js';

let root: HTMLElement | null = null;
let afterRecheckStillMissing = false;
let paintGeneration = 0;

/**
 * Procedure for sharing one skills shelf and one canonical rule file across CLIs.
 *
 * Deliberately a repository file rather than the article it links to: this URL
 * ships inside the bundle, so a link that moves would stay broken for anyone who
 * does not upgrade, while a path on main can be corrected at any time. Same URL
 * for every locale — the document is English and there is no translated copy to
 * drift out of sync with it.
 */
const SHARED_SKILLS_URL =
  'https://github.com/ishizakahiroshi/many-ai-cli/blob/main/docs/manual_shared-skills-and-rules.md';

function renderUsageGuide(): void {
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
      <div class="zero-session-footer"><button type="button" data-zero-tour>${t('zero_session_tour')}</button><span>·</span><button type="button" data-zero-docs>${t('zero_session_docs')}</button><span>·</span><button type="button" data-zero-wiring>${t('zero_session_wiring')}</button></div>
    </section>`;
  bindUsageGuideButtons();
}

function bindUsageGuideButtons(): void {
  if (!root) return;
  root.querySelector('[data-zero-tour]')?.addEventListener('click', () => (document.getElementById('first-run-tour-btn') as HTMLButtonElement | null)?.click());
  root.querySelector('[data-zero-docs]')?.addEventListener('click', () => window.open('https://github.com/ishizakahiroshi/many-ai-cli#readme', '_blank', 'noopener'));
  root.querySelector('[data-zero-wiring]')?.addEventListener('click', () => window.open(SHARED_SKILLS_URL, '_blank', 'noopener'));
}

function renderMissingGuide(entries: CliGuideEntry[]): void {
  if (!root) return;
  const items = entries.map((entry) => (
    `<li class="zero-session-cli-item">` +
    `<span class="zero-session-cli-label">${escapeHtml(entry.displayName)}</span>` +
    `<button type="button" class="zero-session-cli-copy" data-copy="${escapeHtml(entry.id)}">${escapeHtml(entry.id)}</button>` +
    `</li>`
  )).join('');
  const still = afterRecheckStillMissing
    ? `<p class="zero-session-missing-still">${t('zero_session_missing_recheck_still')}</p>`
    : '';
  root.className = 'zero-session';
  root.innerHTML = `
    <section class="zero-session-card" aria-labelledby="zero-session-title">
      <div class="zero-session-kicker">${t('zero_session_missing_kicker')}</div>
      <h1 id="zero-session-title">${t('zero_session_missing_title')}</h1>
      <p class="zero-session-intro">${t('zero_session_missing_intro')}</p>
      <p class="zero-session-cli-heading">${t('zero_session_missing_list_label')}</p>
      <ul class="zero-session-cli-list">${items}</ul>
      <p class="zero-session-cli-hint">${t('zero_session_missing_copy_hint')}</p>
      ${still}
      <div class="zero-session-footer">
        <button type="button" class="zero-session-recheck" data-zero-recheck>${t('zero_session_missing_recheck')}</button>
        <span>·</span>
        <button type="button" data-zero-docs>${t('zero_session_docs')}</button>
      </div>
    </section>`;
  root.querySelector('[data-zero-docs]')?.addEventListener('click', () => window.open('https://github.com/ishizakahiroshi/many-ai-cli#readme', '_blank', 'noopener'));
  root.querySelector('[data-zero-recheck]')?.addEventListener('click', () => { void recheckCliAvailability(); });
  root.querySelectorAll<HTMLButtonElement>('[data-copy]').forEach((button) => {
    button.addEventListener('click', () => { void copyCommandName(button); });
  });
}

async function copyCommandName(button: HTMLButtonElement): Promise<void> {
  const value = button.dataset.copy || '';
  if (!value) return;
  try {
    await navigator.clipboard.writeText(value);
    showToast(t('copied_to_clipboard'), button);
  } catch (_) {
    showToast(t('settings_doctor_copy_failed'), button);
  }
}

async function paintEmptyState(fromRecheck = false): Promise<void> {
  const generation = ++paintGeneration;
  const response = await loadProviderSummaries({ includeDisabled: true });
  if (generation !== paintGeneration) return;
  if (!root?.isConnected) return;
  if (!response) {
    afterRecheckStillMissing = false;
    renderUsageGuide();
    return;
  }
  const missingIds = commandMissingIds(response.diagnostics);
  const available = hasAvailableAiCli(response.providers, missingIds);
  if (available) {
    afterRecheckStillMissing = false;
    renderUsageGuide();
    return;
  }
  if (fromRecheck) afterRecheckStillMissing = true;
  renderMissingGuide(aiCliGuideEntries(response.providers));
}

async function recheckCliAvailability(): Promise<void> {
  const button = root?.querySelector<HTMLButtonElement>('[data-zero-recheck]');
  if (button) button.disabled = true;
  try {
    await paintEmptyState(true);
  } finally {
    const next = root?.querySelector<HTMLButtonElement>('[data-zero-recheck]');
    if (next) next.disabled = false;
  }
}

export function renderZeroSessionEmptyState(sessions: Iterable<SessionSnapshot>): void {
  const isEmpty = [...sessions].length === 0;
  const wrapper = document.getElementById('terminal-area-wrapper');
  if (!wrapper) return;
  if (!isEmpty) {
    root?.remove();
    root = null;
    afterRecheckStillMissing = false;
    return;
  }
  if (!root) {
    root = document.createElement('div');
    root.id = 'zero-session-empty-state';
    wrapper.appendChild(root);
    void paintEmptyState();
  }
}

if (typeof document !== 'undefined') {
  document.addEventListener('i18n-ready', () => {
    if (root?.isConnected) void paintEmptyState();
  });
}
