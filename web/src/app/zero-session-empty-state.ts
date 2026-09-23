import { t } from '../i18n.js';
import type { SessionSnapshot } from '../types/proto.js';
import {
  cliInstallStatuses,
  commandMissingIds,
  hasAvailableAiCli,
  type CliInstallStatus,
} from './cli-availability.js';
import { fetchInstallLinkDefaults, missingCliRowsHtml, mountCliMaintenance, type CliMaintenanceHandle } from './cli-maintenance.js';
import { loadProviderSummaries } from './provider-store.js';
import { providerIconHtml } from './session-list.js';
import { escapeHtml, showToast } from './util.js';

let root: HTMLElement | null = null;
let afterRecheckStillMissing = false;
let paintGeneration = 0;
// installed（導入済み）の行はここへ委譲する（plan_provider-cli-update_c4_list-ui.md）。
// renderUsageGuide が呼ばれるたびに作り直すので、前の画面用のポーリングを止めてから
// 新しいものを作る（root ごと innerHTML を書き換えるため、古いハンドルは残しても
// DOM は既に無い — が setInterval は残るので明示的に destroy する）。
let maintenanceHandle: CliMaintenanceHandle | null = null;

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

// renderInstallStatusList は「入っている / 入っていない」の一覧を組み立てる。
// statuses が空（provider 定義が無い等）なら見出しごと出さない。導入済みの行は
// mountCliMaintenance に委譲する（plan_provider-cli-update_c4_list-ui.md）。未導入の
// 行（インストール手順リンク）は今のまま missingCliRowsHtml が持つ。見出しは
// mountCliMaintenance 側が出すので、ここでは持たない（二重見出しを避ける）。
function renderInstallStatusList(statuses: CliInstallStatus[]): string {
  if (!statuses.length) return '';
  return `<div class="zero-session-cli-maintenance-slot" data-zero-cli-maintenance></div>${missingCliRowsHtml(statuses.filter((s) => !s.installed))}`;
}

function renderUsageGuide(statuses: CliInstallStatus[] | null): void {
  if (!root) return;
  maintenanceHandle?.destroy();
  maintenanceHandle = null;
  root.className = 'zero-session';
  // 広い画面では手順（左）と導入状況（右）を並べ、狭い画面では手順を 1 行に畳んで
  // リンク・畳みボタンの下に導入状況を出す（どちらもスクロールせずに更新一覧が見えるようにするため）。
  // 導入状況が無いときは畳まない（畳むと画面に何も残らない）。
  const collapsible = !!statuses?.length;
  root.innerHTML = `
    <section class="zero-session-card${collapsible ? ' has-status is-steps-collapsed' : ''}" aria-labelledby="zero-session-title">
      <div class="zero-session-kicker">${t('zero_session_kicker')}</div>
      <h1 id="zero-session-title">${t('zero_session_title')}</h1>
      <p class="zero-session-intro">${t('zero_session_intro')}</p>
      <div class="zero-session-body">
        ${collapsible ? `<div class="zero-session-status">${renderInstallStatusList(statuses!)}</div>` : ''}
        <div class="zero-session-guide">
          ${collapsible ? `<button type="button" class="zero-session-steps-toggle" data-zero-steps-toggle aria-expanded="false">${t('zero_session_steps_show')}</button>` : ''}
          <ol class="zero-session-steps">
            <li><div><strong>${t('zero_session_install_title')}</strong><span>${t('zero_session_install_body')}</span></div></li>
            <li><div><strong>${t('zero_session_step1_title')}</strong><span>${t('zero_session_step1_body')}</span></div></li>
            <li><div><strong>${t('zero_session_step2_title')}</strong><span>${t('zero_session_step2_body')}</span></div></li>
            <li><div><strong>${t('zero_session_step3_title')}</strong><span>${t('zero_session_step3_body')}</span></div></li>
          </ol>
        </div>
        <div class="zero-session-footer"><button type="button" data-zero-tour>${t('zero_session_tour')}</button><span>·</span><button type="button" data-zero-docs>${t('zero_session_docs')}</button><span>·</span><button type="button" data-zero-wiring>${t('zero_session_wiring')}</button></div>
      </div>
    </section>`;
  bindUsageGuideButtons();
  const slot = statuses ? root.querySelector<HTMLElement>('[data-zero-cli-maintenance]') : null;
  if (slot) maintenanceHandle = mountCliMaintenance(slot, statuses!.filter((s) => s.installed));
}

function bindUsageGuideButtons(): void {
  if (!root) return;
  root.querySelector('[data-zero-tour]')?.addEventListener('click', () => (document.getElementById('first-run-tour-btn') as HTMLButtonElement | null)?.click());
  root.querySelector('[data-zero-docs]')?.addEventListener('click', () => window.open('https://github.com/ishizakahiroshi/many-ai-cli#readme', '_blank', 'noopener'));
  root.querySelector('[data-zero-wiring]')?.addEventListener('click', () => window.open(SHARED_SKILLS_URL, '_blank', 'noopener'));
  root.querySelector<HTMLButtonElement>('[data-zero-steps-toggle]')?.addEventListener('click', (event) => {
    const button = event.currentTarget as HTMLButtonElement;
    const card = button.closest('.zero-session-card');
    const collapsed = !!card?.classList.toggle('is-steps-collapsed');
    button.setAttribute('aria-expanded', String(!collapsed));
    button.textContent = t(collapsed ? 'zero_session_steps_show' : 'zero_session_steps_hide');
  });
}

function renderMissingGuide(entries: CliInstallStatus[]): void {
  if (!root) return;
  maintenanceHandle?.destroy();
  maintenanceHandle = null;
  const items = entries.map((entry) => {
    const link = entry.installUrl
      ? `<a class="zero-session-cli-install-link" href="${escapeHtml(entry.installUrl)}" target="_blank" rel="noopener">${t('zero_session_install_link_label')}</a>`
      : '';
    return (
      `<li class="zero-session-cli-item">` +
      providerIconHtml(entry.id, 16) +
      `<span class="zero-session-cli-label">${escapeHtml(entry.displayName)}</span>` +
      `<button type="button" class="zero-session-cli-copy" data-copy="${escapeHtml(entry.id)}">${escapeHtml(entry.id)}</button>` +
      link +
      `</li>`
    );
  }).join('');
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
  const [response, installLinks] = await Promise.all([
    loadProviderSummaries({ includeDisabled: true }),
    fetchInstallLinkDefaults(),
  ]);
  if (generation !== paintGeneration) return;
  if (!root?.isConnected) return;
  if (!response) {
    afterRecheckStillMissing = false;
    renderUsageGuide(null);
    return;
  }
  const missingIds = commandMissingIds(response.diagnostics);
  const available = hasAvailableAiCli(response.providers, missingIds);
  const statuses = cliInstallStatuses(response.providers, missingIds, installLinks);
  if (available) {
    afterRecheckStillMissing = false;
    renderUsageGuide(statuses);
    return;
  }
  if (fromRecheck) afterRecheckStillMissing = true;
  renderMissingGuide(statuses);
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
    maintenanceHandle?.destroy();
    maintenanceHandle = null;
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
