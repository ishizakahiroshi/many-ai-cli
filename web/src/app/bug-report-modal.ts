import { t } from '../i18n.js';
import { activeSessionId } from './state.js';
import { apiFetch, showToast } from './util.js';

interface PreviewResponse {
  markdown: string;
  environment_markdown: string;
  warnings: string[];
  gh_available: boolean;
  session_log_recorded: boolean;
  log_attachment_available: boolean;
  log_markdown?: string;
  log_saved_path?: string;
  log_preview_token?: string;
}

interface FinalizeResponse {
  markdown: string;
  url?: string;
  saved_path?: string;
  warnings: string[];
}

let overlay: HTMLDivElement | null = null;
let symptomEl: HTMLTextAreaElement | null = null;
let reproductionEl: HTMLTextAreaElement | null = null;
let environmentEl: HTMLTextAreaElement | null = null;
let fullPreviewEl: HTMLPreElement | null = null;
let submitBtn: HTMLButtonElement | null = null;
let statusEl: HTMLDivElement | null = null;
let logCheckboxEl: HTMLInputElement | null = null;
let logHintEl: HTMLParagraphElement | null = null;
let logPreviewWrapEl: HTMLDivElement | null = null;
let logPreviewEl: HTMLPreElement | null = null;
let logPathEl: HTMLParagraphElement | null = null;
let modalSessionId: number | null = null;
let previewedLogMarkdown = '';
let logPreviewToken = '';
let logPreviewReady = false;
let logPreviewLoading = false;

function handleModalKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape' && overlay) closeModal();
}

function locale(): string {
  return document.documentElement.lang || window.__lang || 'ja';
}

function buildVisiblePreview(): string {
  const ja = locale().toLowerCase().startsWith('ja');
  const symptom = symptomEl?.value.trim() || '';
  const reproduction = reproductionEl?.value.trim() || (ja ? '未記入' : 'Not provided');
  const environment = environmentEl?.value.trim() || '';
  return `## ${ja ? '症状' : 'Symptom'}\n\n${symptom}\n\n` +
    `## ${ja ? '再現手順（任意）' : 'Steps to reproduce (optional)'}\n\n${reproduction}\n\n` +
    `## ${ja ? '環境情報' : 'Environment'}\n\n${environment}\n`;
}

function updateSubmitState(): void {
  if (submitBtn) {
    submitBtn.disabled = !symptomEl?.value.trim() || logPreviewLoading || (!!logCheckboxEl?.checked && !logPreviewReady);
  }
}

function refreshFullPreview(): void {
  if (fullPreviewEl) fullPreviewEl.textContent = buildVisiblePreview();
  updateSubmitState();
  if (statusEl) {
    statusEl.hidden = true;
    statusEl.textContent = '';
  }
}

function closeModal(): void {
  document.removeEventListener('keydown', handleModalKeydown);
  overlay?.remove();
  overlay = null;
  symptomEl = null;
  reproductionEl = null;
  environmentEl = null;
  fullPreviewEl = null;
  submitBtn = null;
  statusEl = null;
  logCheckboxEl = null;
  logHintEl = null;
  logPreviewWrapEl = null;
  logPreviewEl = null;
  logPathEl = null;
  modalSessionId = null;
  previewedLogMarkdown = '';
  logPreviewToken = '';
  logPreviewReady = false;
  logPreviewLoading = false;
  document.body.classList.remove('bug-report-open');
}

async function loadLogPreview(): Promise<void> {
  if (!logCheckboxEl?.checked || modalSessionId == null) return;
  logPreviewLoading = true;
  logPreviewReady = false;
  previewedLogMarkdown = '';
  logPreviewToken = '';
  if (logPreviewWrapEl) logPreviewWrapEl.hidden = false;
  if (logPreviewEl) logPreviewEl.textContent = t('bug_report_log_loading');
  if (logPathEl) logPathEl.textContent = '';
  refreshFullPreview();
  try {
    const res = await apiFetch('/api/bug-report/preview', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        session_id: modalSessionId,
        include_recent_log_lines: 200,
        locale: locale(),
        user_agent: navigator.userAgent,
      }),
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json() as PreviewResponse;
    if (!logCheckboxEl?.checked || !overlay) return;
    if (!data.log_markdown || !data.log_saved_path || !data.log_preview_token) throw new Error('missing redacted log preview');
    previewedLogMarkdown = data.log_markdown;
    logPreviewToken = data.log_preview_token;
    logPreviewReady = true;
    if (logPreviewEl) logPreviewEl.textContent = data.log_markdown;
    if (logPathEl) logPathEl.textContent = t('bug_report_log_saved', { path: data.log_saved_path });
  } catch (_) {
    if (logCheckboxEl) logCheckboxEl.checked = false;
    if (logPreviewEl) logPreviewEl.textContent = t('bug_report_log_preview_failed');
    if (statusEl) {
      statusEl.hidden = false;
      statusEl.textContent = t('bug_report_log_preview_failed');
    }
  } finally {
    logPreviewLoading = false;
    updateSubmitState();
  }
}

function handleLogCheckboxChange(): void {
  if (!logCheckboxEl?.checked) {
    previewedLogMarkdown = '';
    logPreviewToken = '';
    logPreviewReady = false;
    logPreviewLoading = false;
    if (logPreviewWrapEl) logPreviewWrapEl.hidden = true;
    if (logPreviewEl) logPreviewEl.textContent = '';
    if (logPathEl) logPathEl.textContent = '';
    refreshFullPreview();
    return;
  }
  void loadLogPreview();
}

function field(labelText: string, required = false): { wrapper: HTMLDivElement; textarea: HTMLTextAreaElement } {
  const wrapper = document.createElement('div');
  wrapper.className = 'bug-report-field';
  const label = document.createElement('label');
  label.textContent = labelText;
  const textarea = document.createElement('textarea');
  textarea.required = required;
  textarea.addEventListener('input', refreshFullPreview);
  label.appendChild(textarea);
  wrapper.appendChild(label);
  return { wrapper, textarea };
}

async function finalizeReport(): Promise<void> {
  const symptom = symptomEl?.value.trim() || '';
  if (!symptom) {
    symptomEl?.focus();
    return;
  }

  // Open a dedicated blank tab while handling the user gesture. After the Hub
  // has performed the final server-side scrub, navigate this tab to GitHub.
  // `noopener` in the feature string makes window.open return null in some
  // browsers, which would prevent the async finalize result from navigating
  // this tab. Detach it immediately while retaining our WindowProxy handle.
  const issueTab = window.open('about:blank', '_blank');
  if (!issueTab) {
    showToast(t('bug_report_popup_blocked'));
    if (statusEl) {
      statusEl.hidden = false;
      statusEl.textContent = t('bug_report_popup_blocked');
    }
    return;
  }
  issueTab.opener = null;
  if (submitBtn) submitBtn.disabled = true;
  if (statusEl) {
    statusEl.hidden = false;
    statusEl.textContent = t('bug_report_finalizing');
  }

  try {
    const res = await apiFetch('/api/bug-report/finalize', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        symptom,
        reproduction: reproductionEl?.value || '',
        environment_markdown: environmentEl?.value || '',
        locale: locale(),
        include_session_log: !!logCheckboxEl?.checked && logPreviewReady,
        log_markdown: logCheckboxEl?.checked ? previewedLogMarkdown : '',
        log_preview_token: logCheckboxEl?.checked ? logPreviewToken : '',
      }),
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json() as FinalizeResponse;
    if (data.saved_path) {
      if (data.url) issueTab.location.replace(data.url);
      else issueTab.close();
      if (statusEl) {
        statusEl.hidden = false;
        statusEl.textContent = data.warnings.includes('issue_url_too_long')
          ? t('bug_report_saved_fallback', { path: data.saved_path })
          : t('bug_report_gist_failed_fallback', { path: data.saved_path });
      }
      return;
    }
    if (!data.url) throw new Error('missing issue URL');
    issueTab.location.replace(data.url);
    closeModal();
  } catch (_) {
    issueTab.close();
    if (statusEl) {
      statusEl.hidden = false;
      statusEl.textContent = t('bug_report_failed');
    }
  } finally {
    if (submitBtn) submitBtn.disabled = !symptomEl?.value.trim();
  }
}

async function openModal(): Promise<void> {
  if (overlay) return;
  modalSessionId = activeSessionId;
  overlay = document.createElement('div');
  overlay.className = 'bug-report-overlay';
  overlay.setAttribute('role', 'presentation');

  const dialog = document.createElement('section');
  dialog.className = 'bug-report-dialog';
  dialog.setAttribute('role', 'dialog');
  dialog.setAttribute('aria-modal', 'true');
  dialog.setAttribute('aria-labelledby', 'bug-report-title');

  const header = document.createElement('header');
  header.className = 'bug-report-header';
  const title = document.createElement('h2');
  title.id = 'bug-report-title';
  title.textContent = t('bug_report_title');
  const close = document.createElement('button');
  close.type = 'button';
  close.className = 'bug-report-close';
  close.textContent = '×';
  close.setAttribute('aria-label', t('bug_report_cancel'));
  close.addEventListener('click', closeModal);
  header.append(title, close);

  const warning = document.createElement('p');
  warning.className = 'bug-report-warning';
  warning.textContent = t('bug_report_send_warning');

  const symptom = field(t('bug_report_symptom'), true);
  symptom.textarea.placeholder = t('bug_report_symptom_placeholder');
  symptomEl = symptom.textarea;
  const reproduction = field(t('bug_report_reproduction'));
  reproduction.textarea.placeholder = t('bug_report_reproduction_placeholder');
  reproductionEl = reproduction.textarea;

  const details = document.createElement('details');
  details.className = 'bug-report-environment';
  details.open = true;
  const summary = document.createElement('summary');
  summary.textContent = t('bug_report_environment');
  const environment = field(t('bug_report_environment_editable'));
  environmentEl = environment.textarea;
  environment.textarea.className = 'bug-report-environment-text';
  environment.textarea.readOnly = true;
  environment.textarea.placeholder = t('bug_report_loading');
  details.append(summary, environment.wrapper);

  const logAttachment = document.createElement('section');
  logAttachment.className = 'bug-report-log-attachment';
  const logLabel = document.createElement('label');
  logLabel.className = 'bug-report-log-label';
  logCheckboxEl = document.createElement('input');
  logCheckboxEl.type = 'checkbox';
  logCheckboxEl.checked = false;
  logCheckboxEl.disabled = true;
  logCheckboxEl.addEventListener('change', handleLogCheckboxChange);
  const logLabelText = document.createElement('span');
  logLabelText.textContent = t('bug_report_log_attach_label');
  logLabel.append(logCheckboxEl, logLabelText);
  logHintEl = document.createElement('p');
  logHintEl.className = 'bug-report-log-hint';
  logHintEl.textContent = t('bug_report_log_checking');
  const gistWarning = document.createElement('p');
  gistWarning.className = 'bug-report-log-warning';
  gistWarning.textContent = t('bug_report_log_gist_warning');
  logPreviewWrapEl = document.createElement('div');
  logPreviewWrapEl.className = 'bug-report-log-preview-wrap';
  logPreviewWrapEl.hidden = true;
  const logPreviewTitle = document.createElement('h3');
  logPreviewTitle.textContent = t('bug_report_log_preview_title');
  logPreviewEl = document.createElement('pre');
  logPreviewEl.className = 'bug-report-log-preview';
  logPathEl = document.createElement('p');
  logPathEl.className = 'bug-report-log-path';
  logPreviewWrapEl.append(logPreviewTitle, logPreviewEl, logPathEl);
  logAttachment.append(logLabel, logHintEl, gistWarning, logPreviewWrapEl);

  const previewLabel = document.createElement('h3');
  previewLabel.className = 'bug-report-preview-title';
  previewLabel.textContent = t('bug_report_full_preview');
  fullPreviewEl = document.createElement('pre');
  fullPreviewEl.className = 'bug-report-full-preview';

  const screenshot = document.createElement('p');
  screenshot.className = 'bug-report-screenshot-guide';
  screenshot.textContent = t('bug_report_screenshot_guide');

  statusEl = document.createElement('div');
  statusEl.className = 'bug-report-status';
  statusEl.hidden = true;

  const actions = document.createElement('div');
  actions.className = 'bug-report-actions';
  const cancel = document.createElement('button');
  cancel.type = 'button';
  cancel.className = 'bug-report-cancel';
  cancel.textContent = t('bug_report_cancel');
  cancel.addEventListener('click', closeModal);
  submitBtn = document.createElement('button');
  submitBtn.type = 'button';
  submitBtn.className = 'bug-report-submit';
  submitBtn.textContent = t('bug_report_open_github');
  submitBtn.disabled = true;
  submitBtn.addEventListener('click', () => void finalizeReport());
  actions.append(cancel, submitBtn);

  dialog.append(header, warning, symptom.wrapper, reproduction.wrapper, details, logAttachment,
    previewLabel, fullPreviewEl, screenshot, statusEl, actions);
  overlay.appendChild(dialog);
  overlay.addEventListener('click', event => { if (event.target === overlay) closeModal(); });
  document.body.appendChild(overlay);
  document.body.classList.add('bug-report-open');
  document.addEventListener('keydown', handleModalKeydown);
  refreshFullPreview();

  try {
    const res = await apiFetch('/api/bug-report/preview', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        session_id: modalSessionId ?? undefined,
        include_recent_log_lines: 0,
        locale: locale(),
        user_agent: navigator.userAgent,
      }),
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json() as PreviewResponse;
    if (!environmentEl || !overlay) return;
    environmentEl.value = data.environment_markdown;
    environmentEl.readOnly = false;
    if (logCheckboxEl && logHintEl) {
      logCheckboxEl.disabled = !data.log_attachment_available;
      if (data.log_attachment_available) {
        logHintEl.textContent = t('bug_report_log_ready');
      } else if (modalSessionId == null) {
        logHintEl.textContent = t('bug_report_log_requires_session');
      } else if (!data.gh_available) {
        logHintEl.textContent = t('bug_report_log_requires_gh');
      } else if (!data.session_log_recorded) {
        logHintEl.textContent = t('bug_report_log_not_recorded');
      } else {
        logHintEl.textContent = t('bug_report_log_unavailable');
      }
    }
    refreshFullPreview();
    environmentEl.style.height = `${Math.min(environmentEl.scrollHeight + 4, 320)}px`;
  } catch (_) {
    if (statusEl) {
      statusEl.hidden = false;
      statusEl.textContent = t('bug_report_preview_failed');
    }
  }
  symptomEl?.focus();
}

export function initBugReportModal(): void {
  document.getElementById('bug-report-btn')?.addEventListener('click', () => void openModal());
}
