import { loadProviderSummaries, validateProviderDefinition, type ProviderSummary } from './provider-store.js';
import { token } from './util.js';

type ProviderFormState = { id: string; revision: string } | null;

function initProviderManager(): void {
  const list = document.getElementById('provider-manager-list');
  const addButton = document.getElementById('provider-add-btn') as HTMLButtonElement | null;
  const status = document.getElementById('provider-manager-status');
  const form = document.getElementById('provider-enrollment-form') as HTMLFormElement | null;
  const idInput = document.getElementById('provider-enrollment-id') as HTMLInputElement | null;
  const labelInput = document.getElementById('provider-enrollment-label') as HTMLInputElement | null;
  const executableInput = document.getElementById('provider-enrollment-executable') as HTMLInputElement | null;
  const validateButton = document.getElementById('provider-enrollment-validate');
  const cancelButton = document.getElementById('provider-enrollment-cancel');
  const result = document.getElementById('provider-enrollment-result');
  if (!list || !addButton || !status || !form || !idInput || !labelInput || !executableInput || !validateButton || !cancelButton || !result) return;

  let editing: ProviderFormState = null;
  let currentRevision = '';

  const setStatus = (message: string, warning = false): void => {
    status.textContent = message;
    status.classList.toggle('settings-note-warn', warning);
  };

  const definitionFromForm = (): Record<string, unknown> => ({
    schema_version: 1,
    id: idInput.value.trim(),
    display_name: labelInput.value.trim(),
    launch: { executable: executableInput.value.trim() },
  });

  const showForm = (value: ProviderSummary | null = null): void => {
    editing = value ? { id: value.id, revision: currentRevision } : null;
    idInput.value = value?.id || '';
    idInput.disabled = !!value;
    labelInput.value = value?.display_name || '';
    executableInput.value = '';
    result.textContent = '';
    form.hidden = false;
    (value ? labelInput : idInput).focus();
  };

  const hideForm = (): void => {
    editing = null;
    form.hidden = true;
    idInput.disabled = false;
    result.textContent = '';
  };

  const render = (providers: ProviderSummary[]): void => {
    list.innerHTML = '';
    for (const provider of providers) {
      const row = document.createElement('div');
      row.className = 'provider-manager-row';
      const details = document.createElement('div');
      details.className = 'provider-manager-details';
      const name = document.createElement('strong');
      name.textContent = provider.display_name || provider.id;
      const meta = document.createElement('span');
      meta.className = 'provider-manager-meta';
      meta.textContent = `${provider.id} · ${provider.origin}`;
      details.append(name, meta);
      const actions = document.createElement('div');
      actions.className = 'provider-manager-actions';
      const history = document.createElement('button');
      history.className = 'settings-inline-btn';
      history.type = 'button';
      history.textContent = 'History';
      history.addEventListener('click', () => void showHistory(provider));
      actions.append(history);
      if (provider.origin !== 'embedded') {
        const edit = document.createElement('button');
        edit.className = 'settings-inline-btn';
        edit.type = 'button';
        edit.textContent = 'Edit';
        edit.addEventListener('click', () => void editProvider(provider));
        const remove = document.createElement('button');
        remove.className = 'settings-inline-btn danger';
        remove.type = 'button';
        remove.textContent = 'Delete';
        remove.addEventListener('click', () => void removeProvider(provider));
        actions.append(edit, remove);
      } else {
        const standard = document.createElement('span');
        standard.className = 'provider-manager-meta';
        standard.textContent = 'Built-in';
        actions.append(standard);
      }
      row.append(details, actions);
      list.append(row);
    }
  };

  const load = async (): Promise<void> => {
    const response = await loadProviderSummaries();
    if (!response) {
      setStatus('Provider list could not be loaded.', true);
      return;
    }
    currentRevision = response.revision;
    render(response.providers);
    setStatus('');
  };

  const editProvider = async (provider: ProviderSummary): Promise<void> => {
    showForm(provider);
    try {
      const response = await fetch(`/api/providers/${encodeURIComponent(provider.id)}?token=${encodeURIComponent(token || '')}`);
      if (!response.ok) return;
      const body = await response.json();
      const executable = body?.provider?.launch?.executable;
      if (typeof executable === 'string') executableInput.value = executable;
    } catch (_) {
      result.textContent = 'Provider details could not be loaded.';
      result.classList.add('settings-note-warn');
    }
  };

  const showHistory = async (provider: ProviderSummary): Promise<void> => {
    try {
      const response = await fetch(`/api/providers/${encodeURIComponent(provider.id)}/history?token=${encodeURIComponent(token || '')}`);
      if (!response.ok) {
        setStatus(`History unavailable (${response.status}).`, true);
        return;
      }
      const body = await response.json();
      const revisions = Array.isArray(body?.revisions) ? body.revisions : [];
      setStatus(revisions.length === 0
        ? `${provider.display_name || provider.id}: no local revisions`
        : `${provider.display_name || provider.id}: ${revisions.length} local revision(s)`);
    } catch (_) {
      setStatus('History request failed.', true);
    }
  };

  const validate = async (): Promise<boolean> => {
    const response = await validateProviderDefinition(definitionFromForm());
    if (!response) {
      result.textContent = 'Validation request failed.';
      return false;
    }
    const errors = response.diagnostics.filter((diagnostic: any) => diagnostic?.severity === 'error');
    result.textContent = response.valid
      ? 'Definition is valid.'
      : errors.map((diagnostic: any) => String(diagnostic.message || 'Invalid definition')).join(' ');
    result.classList.toggle('settings-note-warn', !response.valid);
    return response.valid;
  };

  const save = async (): Promise<void> => {
    if (!(await validate())) return;
    const url = editing
      ? `/api/providers/${encodeURIComponent(editing.id)}?token=${encodeURIComponent(token || '')}`
      : `/api/providers?token=${encodeURIComponent(token || '')}`;
    const body = editing
      ? { expected_revision: editing.revision, definition: definitionFromForm() }
      : definitionFromForm();
    const response = await fetch(url, {
      method: editing ? 'PATCH' : 'POST',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
      body: JSON.stringify(body),
    });
    if (!response.ok) {
      result.textContent = `Save failed (${response.status}). Your input was kept.`;
      result.classList.add('settings-note-warn');
      return;
    }
    hideForm();
    await load();
  };

  const removeProvider = async (provider: ProviderSummary): Promise<void> => {
    const response = await fetch(`/api/providers/${encodeURIComponent(provider.id)}?token=${encodeURIComponent(token || '')}&expected_revision=${encodeURIComponent(currentRevision)}`, { method: 'DELETE' });
    if (!response.ok) {
      setStatus(`Delete failed (${response.status}). Reload and try again.`, true);
      return;
    }
    await load();
  };

  addButton.addEventListener('click', () => showForm());
  cancelButton.addEventListener('click', hideForm);
  validateButton.addEventListener('click', () => void validate());
  form.addEventListener('submit', (event) => {
    event.preventDefault();
    void save();
  });
  document.addEventListener('provider-enrollment-open', () => showForm());
  void load();
}

if (typeof document !== 'undefined') {
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', initProviderManager, { once: true });
  else initProviderManager();
}
