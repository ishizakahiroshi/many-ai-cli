import { t } from '../i18n.js';
import {
  createRequestGuard,
  cycleDialogFocus,
  duplicateProviderDraft,
  formatDiagnosticMessages,
  isBuiltinProviderID,
  originLabelKey,
  providerErrorMessage,
  providerStatusKind,
  statusLabelKey,
  suggestProviderId,
} from './provider-manager-view.js';
import {
  createProvider,
  loadProviderBackups,
  loadProviderDetail,
  loadProviderHistory,
  loadProviderRecovery,
  loadProviderRevisionDiff,
  loadProviderSummaries,
  recoverProvider,
  removeProvider as requestRemoveProvider,
  resetProviderOverride,
  restoreProviderBackup,
  restoreProviderRevision,
  saveProviderDefinition,
  validateProviderDefinition,
  verifyProviderBackup,
  type ProviderEffectiveDefinition,
  type ProviderMutationResult,
  type ProviderRecoveryCandidate,
  type ProviderRevisionRecord,
  type ProviderSummary,
} from './provider-store.js';
import { appConfirm } from './settings.js';

type ProviderFormState = { id: string; revision: string; isBuiltin: boolean; pending: boolean } | null;

function tx(key: string, fallback: string, vars: Record<string, unknown> = {}): string {
  let value = t(key, vars);
  if (value === key) {
    value = fallback;
    Object.entries(vars).forEach(([name, replacement]) => {
      value = value.replaceAll(`{${name}}`, String(replacement));
    });
  }
  return value;
}

// The advanced JSON editor prefill drops fields the server computes and never
// reads back (source/effective_source/revision/capabilities_summary) and the
// identity fields that already have dedicated required inputs above it.
function definitionForAdvancedPreview(definition: ProviderEffectiveDefinition): string {
  const rest: Record<string, unknown> = { ...(definition as unknown as Record<string, unknown>) };
  delete rest.schema_version;
  delete rest.id;
  delete rest.display_name;
  delete rest.source;
  delete rest.effective_source;
  delete rest.revision;
  delete rest.field_origins;
  delete rest.capabilities_summary;
  if (Object.keys(rest).length === 0) return '';
  return JSON.stringify(rest, null, 2);
}

function initProviderManager(): void {
  const list = document.getElementById('provider-manager-list');
  const addButton = document.getElementById('provider-add-btn') as HTMLButtonElement | null;
  const status = document.getElementById('provider-manager-status');
  const overlay = document.getElementById('provider-enrollment-overlay');
  const form = document.getElementById('provider-enrollment-form') as HTMLFormElement | null;
  const title = document.getElementById('provider-enrollment-title');
  const idInput = document.getElementById('provider-enrollment-id') as HTMLInputElement | null;
  const labelInput = document.getElementById('provider-enrollment-label') as HTMLInputElement | null;
  const executableInput = document.getElementById('provider-enrollment-executable') as HTMLInputElement | null;
  const advancedInput = document.getElementById('provider-enrollment-advanced') as HTMLTextAreaElement | null;
  const validateButton = document.getElementById('provider-enrollment-validate') as HTMLButtonElement | null;
  const saveButton = document.getElementById('provider-enrollment-save') as HTMLButtonElement | null;
  const cancelButton = document.getElementById('provider-enrollment-cancel');
  const closeButton = document.getElementById('provider-enrollment-close');
  const result = document.getElementById('provider-enrollment-result') as HTMLElement | null;
  const historyOverlay = document.getElementById('provider-history-overlay');
  const historyPanel = document.getElementById('provider-history-panel') as HTMLElement | null;
  const historyCloseButton = document.getElementById('provider-history-close');
  const historyStatus = document.getElementById('provider-history-status') as HTMLElement | null;
  const historyResetButton = document.getElementById('provider-history-reset') as HTMLButtonElement | null;
  const historyRevisionsList = document.getElementById('provider-history-revisions');
  const historyBackupsList = document.getElementById('provider-history-backups');
  if (!list || !addButton || !status || !overlay || !form || !title || !idInput || !labelInput || !executableInput || !advancedInput || !validateButton || !saveButton || !cancelButton || !closeButton || !result
    || !historyOverlay || !historyPanel || !historyCloseButton || !historyStatus || !historyResetButton || !historyRevisionsList || !historyBackupsList) return;

  // historyRecovery has no counterpart in index.html: it is created here and
  // inserted right above the revisions list so it only appears in the DOM
  // (and only leaves the "state === 'ok'" screen unchanged) when a provider
  // actually needs recovery.
  const historyRecovery = document.createElement('div');
  historyRecovery.className = 'provider-history-recovery';
  historyRecovery.hidden = true;
  const historyRevisionsSection = historyRevisionsList.parentElement;
  if (historyRevisionsSection) {
    historyRevisionsSection.before(historyRecovery);
  } else {
    historyPanel.insertBefore(historyRecovery, historyRevisionsList);
  }

  let editing: ProviderFormState = null;
  let currentRevision = '';
  let idTouched = false;
  let renderedIds = new Set<string>();
  let previousFocus: HTMLElement | null = null;
  let keyHandler: ((event: KeyboardEvent) => void) | null = null;
  let detailAbort: AbortController | null = null;
  let saving = false;
  const detailGuard = createRequestGuard();
  const listGuard = createRequestGuard();
  let historyPreviousFocus: HTMLElement | null = null;
  let historyKeyHandler: ((event: KeyboardEvent) => void) | null = null;
  let historyAbort: AbortController | null = null;
  let historyProvider: ProviderSummary | null = null;
  const historyGuard = createRequestGuard();

  const setStatus = (message: string, warning = false): void => {
    status.textContent = message;
    status.classList.toggle('settings-note-warn', warning);
    status.dataset.state = warning ? 'error' : (message ? 'ok' : '');
  };

  const setResult = (message: string, state: 'ok' | 'error' | ''): void => {
    result.textContent = message;
    result.classList.toggle('settings-note-warn', state === 'error');
    result.dataset.state = state;
    if (state === 'error' && message) result.focus();
  };

  // providerFailureText only formats the message; callers still branch on
  // `kind === 'aborted'` themselves before calling it. This project's
  // tsconfig runs with strictNullChecks off, under which TypeScript cannot
  // narrow a discriminated union through `!x.ok` or an else branch (only the
  // positive `x.ok === false` check narrows) — every call site below follows
  // that shape rather than routing through a shared branch-and-report helper.
  const providerFailureText = (failure: { kind: 'network' | 'aborted' | 'http'; status?: number }): string => {
    const message = providerErrorMessage(failure);
    return tx(message.key, message.fallback, message.vars);
  };

  const setFormBusy = (busy: boolean): void => {
    validateButton.disabled = busy;
    saveButton.disabled = busy;
    (cancelButton as HTMLButtonElement).disabled = busy;
    (closeButton as HTMLButtonElement).disabled = busy;
  };

  // definitionFromForm merges the required basic fields with the optional
  // advanced JSON: the basic id/display_name/launch.executable always win
  // (they are the only fields a user can edit without opening the advanced
  // textarea, so nothing hidden there can silently change what those inputs
  // show), while every other field the JSON supplies (models, capabilities,
  // adapters, presentation, launch.args/model_args/effort_*/headless, ...)
  // passes through untouched.
  const definitionFromForm = (): { ok: true; definition: Record<string, unknown> } | { ok: false; message: string } => {
    const executable = executableInput.value.trim();
    const base: Record<string, unknown> = {
      schema_version: 1,
      id: idInput.value.trim(),
      display_name: labelInput.value.trim(),
      launch: executable ? { executable } : {},
    };
    const advancedRaw = advancedInput.value.trim();
    if (!advancedRaw) return { ok: true, definition: base };
    let parsed: unknown;
    try {
      parsed = JSON.parse(advancedRaw);
    } catch (_) {
      return { ok: false, message: tx('settings_ai_providers_advanced_invalid_json', 'Advanced definition is not valid JSON.') };
    }
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      return { ok: false, message: tx('settings_ai_providers_advanced_invalid_json', 'Advanced definition is not valid JSON.') };
    }
    const parsedObject = parsed as Record<string, unknown>;
    const parsedLaunch = (parsedObject.launch && typeof parsedObject.launch === 'object' && !Array.isArray(parsedObject.launch))
      ? parsedObject.launch as Record<string, unknown>
      : {};
    const baseLaunch = base.launch as Record<string, unknown>;
    return {
      ok: true,
      definition: {
        ...parsedObject,
        ...base,
        launch: { ...parsedLaunch, ...baseLaunch },
      },
    };
  };

  const currentDefinitionOrShowError = (): Record<string, unknown> | null => {
    const parsed = definitionFromForm();
    if (parsed.ok === false) {
      setResult(parsed.message, 'error');
      return null;
    }
    return parsed.definition;
  };

  const onKeyDown = (event: KeyboardEvent): void => {
    if (overlay.hidden) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      hideForm();
      return;
    }
    cycleDialogFocus(form, event);
  };

  const showForm = (value: ProviderSummary | null = null): void => {
    detailGuard.begin();
    detailAbort?.abort();
    detailAbort = null;
    const isBuiltin = value ? isBuiltinProviderID(value.id) : false;
    editing = value ? { id: value.id, revision: '', isBuiltin, pending: true } : null;
    idTouched = !!value;
    idInput.value = value?.id || '';
    idInput.disabled = !!value;
    labelInput.value = value?.display_name || '';
    executableInput.value = '';
    executableInput.required = true;
    advancedInput.value = '';
    setFormBusy(!!editing?.pending);
    setResult('', '');
    title.textContent = value
      ? tx('settings_ai_providers_title_edit', 'Edit AI CLI')
      : tx('settings_ai_providers_title_add', 'Add AI CLI');
    closeButton.setAttribute('aria-label', tx('settings_close', 'Close'));
    previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    overlay.hidden = false;
    (value ? labelInput : idInput).focus();
    if (!keyHandler) {
      keyHandler = onKeyDown;
      document.addEventListener('keydown', keyHandler, true);
    }
  };

  const hideForm = (): void => {
    if (saving) return;
    detailGuard.begin();
    detailAbort?.abort();
    detailAbort = null;
    editing = null;
    idTouched = false;
    overlay.hidden = true;
    idInput.disabled = false;
    setFormBusy(false);
    setResult('', '');
    if (keyHandler) {
      document.removeEventListener('keydown', keyHandler, true);
      keyHandler = null;
    }
    previousFocus?.focus();
    previousFocus = null;
  };

  const render = (providers: ProviderSummary[], errorIds: Set<string>, missingIds: Set<string>): void => {
    list.innerHTML = '';
    renderedIds = new Set(providers.map((p) => p.id));
    if (providers.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'provider-manager-empty';
      empty.textContent = tx('settings_ai_providers_list_empty', 'No AI CLI is registered yet.');
      list.append(empty);
      return;
    }
    for (const provider of providers) {
      const row = document.createElement('div');
      row.className = 'provider-manager-row';
      const details = document.createElement('div');
      details.className = 'provider-manager-details';
      const name = document.createElement('strong');
      name.textContent = provider.display_name || provider.id;
      const meta = document.createElement('span');
      meta.className = 'provider-manager-meta';
      const kind = providerStatusKind({
        enabled: provider.enabled,
        hasError: errorIds.has(provider.id),
        commandMissing: missingIds.has(provider.id),
      });
      const statusBadge = document.createElement('span');
      statusBadge.className = 'provider-manager-status';
      statusBadge.dataset.kind = kind;
      statusBadge.textContent = tx(statusLabelKey(kind), kind);
      meta.textContent = `${provider.id} · ${tx(originLabelKey(provider.origin), provider.origin)}`;
      details.append(name, meta, statusBadge);
      const actions = document.createElement('div');
      actions.className = 'provider-manager-actions';
      const history = document.createElement('button');
      history.className = 'settings-inline-btn';
      history.type = 'button';
      history.textContent = tx('settings_ai_providers_history', 'History');
      history.setAttribute('aria-label', `${history.textContent}: ${name.textContent}`);
      history.addEventListener('click', () => void showHistory(provider));
      actions.append(history);
      const duplicate = document.createElement('button');
      duplicate.className = 'settings-inline-btn';
      duplicate.type = 'button';
      duplicate.textContent = tx('settings_ai_providers_duplicate', 'Duplicate');
      duplicate.setAttribute('aria-label', `${duplicate.textContent}: ${name.textContent}`);
      duplicate.addEventListener('click', () => void duplicateProvider(provider));
      actions.append(duplicate);
      if (isBuiltinProviderID(provider.id)) {
        const edit = document.createElement('button');
        edit.className = 'settings-inline-btn';
        edit.type = 'button';
        edit.textContent = tx('settings_ai_providers_edit', 'Edit');
        edit.setAttribute('aria-label', `${edit.textContent}: ${name.textContent}`);
        edit.addEventListener('click', () => void editProvider(provider));
        const toggle = document.createElement('button');
        toggle.className = provider.enabled ? 'settings-inline-btn danger' : 'settings-inline-btn';
        toggle.type = 'button';
        toggle.textContent = provider.enabled
          ? tx('settings_ai_providers_disable', 'Disable')
          : tx('settings_ai_providers_enable', 'Enable');
        toggle.setAttribute('aria-label', `${toggle.textContent}: ${name.textContent}`);
        toggle.addEventListener('click', () => void toggleBuiltinEnabled(provider));
        actions.append(edit, toggle);
      } else {
        const edit = document.createElement('button');
        edit.className = 'settings-inline-btn';
        edit.type = 'button';
        edit.textContent = tx('settings_ai_providers_edit', 'Edit');
        edit.setAttribute('aria-label', `${edit.textContent}: ${name.textContent}`);
        edit.addEventListener('click', () => void editProvider(provider));
        const remove = document.createElement('button');
        remove.className = 'settings-inline-btn danger';
        remove.type = 'button';
        remove.textContent = tx('settings_ai_providers_delete', 'Delete');
        remove.setAttribute('aria-label', `${remove.textContent}: ${name.textContent}`);
        remove.addEventListener('click', () => void removeProvider(provider));
        actions.append(edit, remove);
      }
      row.append(details, actions);
      list.append(row);
    }
  };

  const load = async (): Promise<void> => {
    const requestToken = listGuard.begin();
    const response = await loadProviderSummaries({ includeDisabled: true });
    if (!listGuard.isCurrent(requestToken)) return;
    if (!response) {
      setStatus(tx('settings_ai_providers_list_failed', 'Provider list could not be loaded.'), true);
      return;
    }
    currentRevision = response.revision;
    const errorIds = new Set<string>();
    const missingIds = new Set<string>();
    for (const diagnostic of response.diagnostics) {
      const field = String(diagnostic?.field || '');
      const owner = response.providers.find((entry) => field === entry.id || field.startsWith(`${entry.id}.`));
      if (!owner) continue;
      if (diagnostic.severity === 'error') errorIds.add(owner.id);
      if (diagnostic.code === 'command_missing') missingIds.add(owner.id);
    }
    render(response.providers, errorIds, missingIds);
    const formatted = formatDiagnosticMessages(response.diagnostics);
    if (formatted.hasError) {
      setStatus(formatted.text, true);
      return;
    }
    setStatus(currentRevision
      ? tx('settings_ai_providers_revision', 'Revision {revision}', { revision: currentRevision })
      : '');
  };

  // editProvider pins the C2 fix: opening B's edit while A's detail fetch is
  // still in flight (or aborting it outright) must never let A's response
  // land in B's form. detailGuard.begin()/isCurrent() catches every ordering
  // even if the AbortController's cancellation itself doesn't stop a response
  // that already started downloading.
  const editProvider = async (provider: ProviderSummary): Promise<void> => {
    showForm(provider);
    const isBuiltin = isBuiltinProviderID(provider.id);
    const controller = new AbortController();
    detailAbort = controller;
    const requestToken = detailGuard.begin();
    const detail = await loadProviderDetail(provider.id, { signal: controller.signal });
    if (!detailGuard.isCurrent(requestToken)) return;
    if (detail.ok === false) {
      if (detail.kind !== 'aborted') setResult(providerFailureText(detail), 'error');
      return;
    }
    const definition = detail.provider;
    const launch = (definition.launch && typeof definition.launch === 'object') ? definition.launch as Record<string, unknown> : {};
    executableInput.value = typeof launch.executable === 'string' ? launch.executable : '';
    const candidates = Array.isArray(launch.executable_candidates) ? launch.executable_candidates : [];
    executableInput.required = !(isBuiltin && !executableInput.value && candidates.length > 0);
    advancedInput.value = definitionForAdvancedPreview(definition);
    editing = {
      id: provider.id,
      revision: isBuiltin ? (definition.effective_source?.revision || '') : detail.revision,
      isBuiltin,
      pending: false,
    };
    setFormBusy(false);
  };

  // duplicateProvider seeds a fresh "add" form from an existing provider's
  // definition so a variant can be registered without hand-typing JSON.
  // showForm(null) leaves `editing` at null, so saving goes through the
  // normal createProvider path (this never mutates the source provider's
  // definition or history).
  const duplicateProvider = async (provider: ProviderSummary): Promise<void> => {
    showForm(null);
    const controller = new AbortController();
    detailAbort = controller;
    const requestToken = detailGuard.begin();
    const detail = await loadProviderDetail(provider.id, { signal: controller.signal });
    if (!detailGuard.isCurrent(requestToken)) return;
    if (detail.ok === false) {
      if (detail.kind !== 'aborted') setResult(providerFailureText(detail), 'error');
      return;
    }
    const definition = detail.provider;
    const draft = duplicateProviderDraft({ id: provider.id, display_name: provider.display_name }, renderedIds);
    const launch = (definition.launch && typeof definition.launch === 'object') ? definition.launch as Record<string, unknown> : {};
    idInput.value = draft.id;
    labelInput.value = `${draft.displayName}${tx('settings_ai_providers_copy_suffix', ' (copy)')}`;
    executableInput.value = typeof launch.executable === 'string' ? launch.executable : '';
    advancedInput.value = definitionForAdvancedPreview(definition);
    idTouched = true;
    setFormBusy(false);
  };

  const hideHistoryOverlay = (): void => {
    historyAbort?.abort();
    historyAbort = null;
    historyOverlay.hidden = true;
    if (historyKeyHandler) {
      document.removeEventListener('keydown', historyKeyHandler, true);
      historyKeyHandler = null;
    }
    historyPreviousFocus?.focus();
    historyPreviousFocus = null;
  };

  const onHistoryKeyDown = (event: KeyboardEvent): void => {
    if (historyOverlay.hidden) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      hideHistoryOverlay();
      return;
    }
    cycleDialogFocus(historyPanel, event);
  };

  const setHistoryStatus = (message: string, state: 'ok' | 'error' | ''): void => {
    historyStatus.textContent = message;
    historyStatus.dataset.state = state;
  };

  const verifyBackupAction = async (providerId: string, backupId: string): Promise<void> => {
    const result = await verifyProviderBackup(providerId, backupId);
    if (result.ok === false) {
      if (result.kind !== 'aborted') setHistoryStatus(providerFailureText(result), 'error');
      return;
    }
    setHistoryStatus(tx('settings_ai_providers_history_verified', 'Verified.'), 'ok');
  };

  const showRevisionDiff = async (providerId: string, revision: string): Promise<void> => {
    const diff = await loadProviderRevisionDiff(providerId, revision);
    if (diff.ok === false) {
      if (diff.kind !== 'aborted') setHistoryStatus(providerFailureText(diff), 'error');
      return;
    }
    setHistoryStatus(JSON.stringify(diff.diff, null, 2), 'ok');
  };

  const resetHistoryProvider = async (): Promise<void> => {
    const provider = historyProvider;
    if (!provider || !isBuiltinProviderID(provider.id)) return;
    const detail = await loadProviderDetail(provider.id);
    if (detail.ok === false) {
      if (detail.kind !== 'aborted') setHistoryStatus(providerFailureText(detail), 'error');
      return;
    }
    const reset = await resetProviderOverride(provider.id, detail.provider.effective_source?.revision || '');
    if (reset.ok === false) {
      if (reset.kind !== 'aborted') setHistoryStatus(providerFailureText(reset), 'error');
      return;
    }
    setHistoryStatus(tx('settings_ai_providers_history_reset_done', 'Reset to distributed default.'), 'ok');
    await load();
    await loadHistoryContent(provider.id, provider.display_name || provider.id);
  };

  const restoreAction = async (
    kind: 'revision' | 'backup',
    providerId: string,
    providerName: string,
    targetRevision: string,
  ): Promise<void> => {
    const confirmed = await appConfirm({
      title: tx('settings_ai_providers_history_restore', 'Restore'),
      message: tx('settings_ai_providers_history_restore_confirm', 'Restore {name} to this revision?', { name: providerName }),
      confirmText: tx('settings_ai_providers_history_restore', 'Restore'),
      cancelText: tx('settings_ai_providers_cancel', 'Cancel'),
      kind: 'danger',
    });
    if (!confirmed) return;
    // Restoring, like every other builtin write, needs the *current*
    // per-provider revision as expected_revision — never one captured
    // earlier, since anything else that touched this provider in the
    // meantime would make a stale value here a spurious 409.
    const detail = await loadProviderDetail(providerId);
    if (detail.ok === false) {
      if (detail.kind !== 'aborted') setHistoryStatus(providerFailureText(detail), 'error');
      return;
    }
    const expectedRevision = detail.provider.effective_source?.revision || '';
    const result = kind === 'revision'
      ? await restoreProviderRevision(providerId, targetRevision, expectedRevision)
      : await restoreProviderBackup(providerId, targetRevision, expectedRevision);
    if (result.ok === false) {
      if (result.kind !== 'aborted') setHistoryStatus(providerFailureText(result), 'error');
      return;
    }
    setHistoryStatus(tx('settings_ai_providers_history_restored', 'Restored.'), 'ok');
    await load();
    await loadHistoryContent(providerId, providerName);
  };

  const recoveryCandidateLabel = (candidate: ProviderRecoveryCandidate): string =>
    candidate.kind === 'user_revision'
      ? tx(
          'settings_ai_providers_recovery_user_revision',
          'Last readable version of your own config ({created_at})',
          { created_at: candidate.created_at },
        )
      : tx('settings_ai_providers_recovery_base', 'State with your changes removed');

  // recoverAction is deliberately not `kind: 'danger'` in its confirm dialog:
  // unlike restoreAction (which overwrites a normally-working config), this
  // runs only while the provider is already in the recovery_required state,
  // and the file it replaces stays safe in quarantine either way.
  const recoverAction = async (
    providerId: string,
    providerName: string,
    candidate: ProviderRecoveryCandidate,
  ): Promise<void> => {
    const confirmed = await appConfirm({
      title: tx('settings_ai_providers_recovery_apply', 'Restore to this state'),
      message: recoveryCandidateLabel(candidate),
      confirmText: tx('settings_ai_providers_recovery_apply', 'Restore to this state'),
      cancelText: tx('settings_ai_providers_cancel', 'Cancel'),
    });
    if (!confirmed) return;
    const result = await recoverProvider(providerId, candidate.kind === 'user_revision' ? candidate.revision : '');
    if (result.ok === false) {
      if (result.kind !== 'aborted') setHistoryStatus(providerFailureText(result), 'error');
      return;
    }
    setHistoryStatus(tx('settings_ai_providers_history_restored', 'Restored.'), 'ok');
    await load();
    await loadHistoryContent(providerId, providerName);
  };

  const renderRecovery = (
    providerId: string,
    providerName: string,
    state: 'ok' | 'recovery_required',
    candidates: ProviderRecoveryCandidate[],
  ): void => {
    historyRecovery.innerHTML = '';
    if (state !== 'recovery_required') {
      historyRecovery.hidden = true;
      return;
    }
    historyRecovery.hidden = false;
    const notice = document.createElement('div');
    notice.className = 'provider-history-recovery-notice settings-note settings-note-warn';
    notice.textContent = tx(
      'settings_ai_providers_recovery_required',
      "This CLI's config file is broken. The broken file has been preserved. Choose what to restore to.",
    );
    historyRecovery.append(notice);
    for (const candidate of candidates) {
      const row = document.createElement('div');
      row.className = 'provider-history-row';
      const meta = document.createElement('div');
      meta.className = 'provider-history-row-meta';
      const label = document.createElement('strong');
      label.textContent = recoveryCandidateLabel(candidate);
      meta.append(label);
      const actions = document.createElement('div');
      actions.className = 'provider-history-row-actions';
      const applyButton = document.createElement('button');
      applyButton.className = 'settings-inline-btn';
      applyButton.type = 'button';
      applyButton.textContent = tx('settings_ai_providers_recovery_apply', 'Restore to this state');
      applyButton.addEventListener('click', () => void recoverAction(providerId, providerName, candidate));
      actions.append(applyButton);
      row.append(meta, actions);
      historyRecovery.append(row);
    }
  };

  const renderHistoryList = (
    container: HTMLElement,
    records: ProviderRevisionRecord[],
    kind: 'revision' | 'backup',
    providerId: string,
    providerName: string,
    currentRevisionId: string,
  ): void => {
    container.innerHTML = '';
    if (records.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'provider-history-empty';
      empty.textContent = tx('settings_ai_providers_history_list_empty', 'None yet.');
      container.append(empty);
      return;
    }
    for (const record of records) {
      const row = document.createElement('div');
      row.className = 'provider-history-row';
      const meta = document.createElement('div');
      meta.className = 'provider-history-row-meta';
      const isCurrent = kind === 'revision' && record.revision === currentRevisionId;
      const label = document.createElement('strong');
      label.textContent = [record.reason, record.created_at].filter(Boolean).join(' ')
        + (isCurrent ? ` ${tx('settings_ai_providers_history_current', '(current)')}` : '');
      const digest = document.createElement('span');
      digest.textContent = record.revision;
      meta.append(label, digest);
      const actions = document.createElement('div');
      actions.className = 'provider-history-row-actions';
      if (kind === 'backup') {
        const verifyButton = document.createElement('button');
        verifyButton.className = 'settings-inline-btn';
        verifyButton.type = 'button';
        verifyButton.textContent = tx('settings_ai_providers_history_verify', 'Verify');
        verifyButton.addEventListener('click', () => void verifyBackupAction(providerId, record.revision));
        actions.append(verifyButton);
      }
      if (kind === 'revision') {
        const diffButton = document.createElement('button');
        diffButton.className = 'settings-inline-btn';
        diffButton.type = 'button';
        diffButton.textContent = tx('settings_ai_providers_history_diff', 'Diff');
        diffButton.addEventListener('click', () => void showRevisionDiff(providerId, record.revision));
        actions.append(diffButton);
      }
      if (!isCurrent) {
        const restoreButton = document.createElement('button');
        restoreButton.className = 'settings-inline-btn';
        restoreButton.type = 'button';
        restoreButton.textContent = tx('settings_ai_providers_history_restore', 'Restore');
        restoreButton.addEventListener('click', () => void restoreAction(kind, providerId, providerName, record.revision));
        actions.append(restoreButton);
      }
      row.append(meta, actions);
      container.append(row);
    }
  };

  // loadHistoryContent guards its three parallel/sequential fetches with the
  // same request-generation pattern as editProvider: opening history for
  // provider A, then quickly reopening it for B, must not let A's late
  // responses render into B's (still-open, now-for-B) panel.
  const loadHistoryContent = async (providerId: string, providerName: string): Promise<void> => {
    const controller = new AbortController();
    historyAbort = controller;
    const requestToken = historyGuard.begin();
    const [historyResult, backupsResult, detail, recoveryResult] = await Promise.all([
      loadProviderHistory(providerId, { signal: controller.signal }),
      loadProviderBackups(providerId, { signal: controller.signal }),
      loadProviderDetail(providerId, { signal: controller.signal }),
      loadProviderRecovery(providerId, { signal: controller.signal }),
    ]);
    if (!historyGuard.isCurrent(requestToken)) return;
    const currentRevisionId = detail.ok ? (detail.provider.effective_source?.revision || '') : '';
    if (historyResult.ok === false) {
      if (historyResult.kind !== 'aborted') setHistoryStatus(providerFailureText(historyResult), 'error');
    } else {
      renderHistoryList(historyRevisionsList, historyResult.revisions, 'revision', providerId, providerName, currentRevisionId);
    }
    if (backupsResult.ok === false) {
      if (backupsResult.kind !== 'aborted') setHistoryStatus(providerFailureText(backupsResult), 'error');
    } else {
      renderHistoryList(historyBackupsList, backupsResult.backups, 'backup', providerId, providerName, currentRevisionId);
    }
    if (recoveryResult.ok === false) {
      // A failed recovery-status lookup must not be mistaken for "no recovery
      // needed": leave the panel hidden (its state === 'ok' default) rather
      // than claim ok, and only surface the error if it wasn't just this
      // request being superseded/aborted.
      renderRecovery(providerId, providerName, 'ok', []);
      if (recoveryResult.kind !== 'aborted') setHistoryStatus(providerFailureText(recoveryResult), 'error');
    } else {
      renderRecovery(providerId, providerName, recoveryResult.state, recoveryResult.candidates);
    }
  };

  const showHistory = async (provider: ProviderSummary): Promise<void> => {
    historyAbort?.abort();
    setHistoryStatus('', '');
    historyRecovery.innerHTML = '';
    historyRecovery.hidden = true;
    historyRevisionsList.innerHTML = '';
    historyBackupsList.innerHTML = '';
    historyPreviousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    historyProvider = provider;
    historyResetButton.hidden = !isBuiltinProviderID(provider.id);
    historyOverlay.hidden = false;
    if (!historyKeyHandler) {
      historyKeyHandler = onHistoryKeyDown;
      document.addEventListener('keydown', historyKeyHandler, true);
    }
    historyCloseButton.focus();
    await loadHistoryContent(provider.id, provider.display_name || provider.id);
  };

  const validate = async (capturedDefinition?: Record<string, unknown>): Promise<boolean> => {
    const definition = capturedDefinition || currentDefinitionOrShowError();
    if (!definition) return false;
    const response = await validateProviderDefinition(definition);
    if (!response) {
      setResult(tx('settings_ai_providers_validate_failed', 'Validation request failed.'), 'error');
      return false;
    }
    const formatted = formatDiagnosticMessages(response.diagnostics);
    setResult(
      response.valid
        ? tx('settings_ai_providers_valid', 'Definition is valid.')
        : (formatted.text || tx('settings_ai_providers_invalid', 'Invalid definition')),
      response.valid ? 'ok' : 'error',
    );
    return response.valid;
  };

  const save = async (): Promise<void> => {
    const definition = currentDefinitionOrShowError();
    if (!definition) return;
    if (editing?.pending || saving) return;
    const submittedEditing = editing ? { ...editing } : null;
    const requestToken = detailGuard.begin();
    saving = true;
    setFormBusy(true);
    if (!(await validate(definition)) || !detailGuard.isCurrent(requestToken)) {
      saving = false;
      setFormBusy(false);
      return;
    }
    const savedId = String(definition.id || '');

    const mutation: ProviderMutationResult = submittedEditing
      ? await saveProviderDefinition(submittedEditing.id, submittedEditing.revision, definition)
      : await createProvider(definition);
    if (!detailGuard.isCurrent(requestToken)) {
      saving = false;
      setFormBusy(false);
      return;
    }
    saving = false;
    setFormBusy(false);
    if (mutation.ok === false) {
      if (mutation.kind === 'aborted') return;
      if (!submittedEditing && mutation.kind === 'http' && mutation.status === 409) {
        // Create-time 409 is "this id already exists", a different situation
        // from the revision-conflict 409 an edit gets, so it keeps its own
        // specific wording instead of the generic providerErrorMessage copy.
        setResult(tx('settings_ai_providers_conflict', 'This ID is already used. Your input was kept.'), 'error');
        return;
      }
      setResult(providerFailureText(mutation), 'error');
      return;
    }
    hideForm();
    document.dispatchEvent(new CustomEvent('provider-enrollment-saved', { detail: { id: savedId } }));
    await load();
  };

  const removeProvider = async (provider: ProviderSummary): Promise<void> => {
    const name = provider.display_name || provider.id;
    const confirmed = await appConfirm({
      title: tx('settings_ai_providers_delete', 'Delete'),
      message: tx('settings_ai_providers_delete_confirm', 'Delete {name}? This does not uninstall the CLI.', { name }),
      confirmText: tx('settings_ai_providers_delete', 'Delete'),
      cancelText: tx('settings_ai_providers_cancel', 'Cancel'),
      kind: 'danger',
    });
    if (!confirmed) return;
    const result = await requestRemoveProvider(provider.id, currentRevision);
    if (result.ok === false) {
      if (result.kind !== 'aborted') setStatus(providerFailureText(result), true);
      return;
    }
    // Lets the spawn provider selector drop the option immediately instead of
    // leaving a stale entry until some unrelated refresh happens to run.
    document.dispatchEvent(new CustomEvent('provider-enrollment-deleted', { detail: { id: provider.id } }));
    await load();
  };

  // toggleBuiltinEnabled always re-fetches the provider's current per-provider
  // history revision immediately before acting (never reuses a revision
  // captured at the last list load), because unlike custom providers a
  // built-in's expected_revision is not registry.Revision() and can go stale
  // from any other override happening in between.
  const toggleBuiltinEnabled = async (provider: ProviderSummary): Promise<void> => {
    if (provider.enabled) {
      const name = provider.display_name || provider.id;
      const confirmed = await appConfirm({
        title: tx('settings_ai_providers_disable', 'Disable'),
        message: tx('settings_ai_providers_disable_confirm', 'Disable {name}? You can turn it back on any time.', { name }),
        confirmText: tx('settings_ai_providers_disable', 'Disable'),
        cancelText: tx('settings_ai_providers_cancel', 'Cancel'),
        kind: 'danger',
      });
      if (!confirmed) return;
    }
    const detail = await loadProviderDetail(provider.id);
    if (detail.ok === false) {
      if (detail.kind !== 'aborted') setStatus(providerFailureText(detail), true);
      return;
    }
    const expectedRevision = detail.provider.effective_source?.revision || '';
    // A sparse { id, enabled } payload, not the whole fetched definition:
    // the Hub merges this onto whatever is already overridden and validates
    // the result against the current embedded/distribution base, so
    // re-enabling never freezes launch/model/adapter fields the user never
    // touched (see internal/provider/history.go SaveOverride).
    const result = provider.enabled
      ? await requestRemoveProvider(provider.id, expectedRevision)
      : await saveProviderDefinition(provider.id, expectedRevision, { schema_version: 1, id: provider.id, enabled: true });
    if (result.ok === false) {
      if (result.kind !== 'aborted') setStatus(providerFailureText(result), true);
      return;
    }
    await load();
  };

  addButton.addEventListener('click', () => showForm());
  cancelButton.addEventListener('click', hideForm);
  closeButton.addEventListener('click', hideForm);
  overlay.addEventListener('click', (event) => {
    if (event.target === overlay) hideForm();
  });
  historyCloseButton.addEventListener('click', hideHistoryOverlay);
  historyResetButton.addEventListener('click', () => void resetHistoryProvider());
  historyOverlay.addEventListener('click', (event) => {
    if (event.target === historyOverlay) hideHistoryOverlay();
  });
  validateButton.addEventListener('click', () => void validate());
  form.addEventListener('submit', (event) => {
    event.preventDefault();
    void save();
  });
  labelInput.addEventListener('input', () => {
    if (editing || idTouched) return;
    idInput.value = suggestProviderId(labelInput.value);
  });
  idInput.addEventListener('input', () => {
    idTouched = true;
  });
  document.addEventListener('provider-enrollment-open', () => showForm());
  document.addEventListener('i18n-ready', () => { void load(); });
  void load();
}

if (typeof document !== 'undefined') {
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', initProviderManager, { once: true });
  else initProviderManager();
}
