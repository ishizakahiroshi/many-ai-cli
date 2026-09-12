import { t } from '../i18n.js';
import type { SessionSnapshot } from '../types/proto.js';
import { sessions } from './state.js';
import { showToast, token } from './util.js';
import { ORCHESTRATION_CLI_OPTIONS } from './orchestration-roles.js';
import { loadSubscriptions, onSubscriptionsChanged, selectableProfiles } from './subscriptions.js';
import { permissionPresetLabel } from './spawn-confirm.js';
import { PERMISSION_PRESET_SCHEMA, isPermissionPresetAvailable } from './spawn-confirm-store.js';

type RelayMode = 'worktree' | 'same-tree';
type RelayRoleKey = 'implementation' | 'implementation-strong' | 'review';
// permissionPreset は役割ごとの権限の段（空 = 既定 = orchestration.child_permission_default）。
// この画面にチェックボックスは無い: 全欄を localStorage へ自動で記憶する既存仕様に乗せる
// （子 plan: docs/local/plan_derived-session-launch_c2_permission-tiers.md 内部 C6）。
type RelayRoleValue = { provider: string; model: string; subscription?: string; permissionPreset?: string };

interface RelayDialogPrefs {
  roles?: Partial<Record<RelayRoleKey, RelayRoleValue>>;
  maxRounds?: number;
  escalateAfter?: number;
  mode?: RelayMode;
  extraImpl?: string;
  extraReview?: string;
}

interface RelayRoleDefinition {
  key: RelayRoleKey;
  labelKey: string;
  required: boolean;
}

const RELAY_ROLE_DEFS: RelayRoleDefinition[] = [
  { key: 'implementation', labelKey: 'spawn_role_implementation', required: true },
  { key: 'implementation-strong', labelKey: 'spawn_role_implementation_strong', required: false },
  { key: 'review', labelKey: 'spawn_role_review', required: true },
];
const RELAY_PREFS_KEY = 'relay_dialog_last';
const RELAY_TERMINAL_CHILD_STATES = new Set(['completed', 'done', 'error', 'disconnected', 'dismissed', 'timeout']);

const overlay = document.getElementById('relay-dialog');
const roleTableBody = document.getElementById('relay-role-table-body');
const targetEl = document.getElementById('relay-target-session');
const planInput = document.getElementById('relay-plan-path') as HTMLInputElement | null;
const planCandidates = document.getElementById('relay-plan-candidates') as HTMLSelectElement | null;
const maxRounds = document.getElementById('relay-max-rounds') as HTMLSelectElement | null;
const escalateAfter = document.getElementById('relay-escalate-after') as HTMLSelectElement | null;
const extraImpl = document.getElementById('relay-extra-impl') as HTMLTextAreaElement | null;
const extraReview = document.getElementById('relay-extra-review') as HTMLTextAreaElement | null;
const worktreeRadio = document.querySelector<HTMLInputElement>('input[name="relay-mode"][value="worktree"]');
const sameTreeRadio = document.querySelector<HTMLInputElement>('input[name="relay-mode"][value="same-tree"]');
const sameTreeWarning = document.getElementById('relay-same-tree-warning');
const remainingSlotsEl = document.getElementById('relay-remaining-slots');
const errorEl = document.getElementById('relay-dialog-error');
const cancelButton = document.getElementById('relay-cancel');
const closeButton = document.getElementById('relay-dialog-close');
const startButton = document.getElementById('relay-start') as HTMLButtonElement | null;

let currentSessionID: number | null = null;
let openGeneration = 0;
let capacityReady = false;
let capacityError = false;
let liveChildren = 0;
let maxChildren = 0;
let submitting = false;
let serverError = '';

function translated(key: string, fallback: string, vars: Record<string, unknown> = {}): string {
  let value = t(key, vars);
  if (value === key) {
    value = fallback;
    Object.entries(vars).forEach(([name, replacement]) => {
      value = value.replaceAll(`{${name}}`, String(replacement));
    });
  }
  return value;
}

function readPrefs(): RelayDialogPrefs {
  try {
    const raw = JSON.parse(localStorage.getItem(RELAY_PREFS_KEY) || '{}');
    return raw && typeof raw === 'object' && !Array.isArray(raw) ? raw as RelayDialogPrefs : {};
  } catch (_) {
    return {};
  }
}

function selectedMode(): RelayMode {
  return sameTreeRadio?.checked ? 'same-tree' : 'worktree';
}

function savePrefs(): void {
  const roles: Partial<Record<RelayRoleKey, RelayRoleValue>> = {};
  roleTableBody?.querySelectorAll<HTMLTableRowElement>('tr[data-role]').forEach((row) => {
    const key = row.dataset.role as RelayRoleKey | undefined;
    const provider = row.querySelector<HTMLSelectElement>('.relay-role-cli')?.value || '';
    const model = row.querySelector<HTMLInputElement>('.relay-role-model')?.value.trim() || '';
    const subscription = row.querySelector<HTMLSelectElement>('.relay-role-subscription');
    const permission = row.querySelector<HTMLSelectElement>('.relay-role-permission');
    if (key && provider) {
      roles[key] = {
        provider,
        model,
        subscription: subscription && !subscription.hidden ? subscription.value : '',
        permissionPreset: permission?.value || '',
      };
    }
  });
  const prefs: RelayDialogPrefs = {
    roles,
    maxRounds: Number(maxRounds?.value || 3),
    escalateAfter: Number(escalateAfter?.value || 2),
    mode: selectedMode(),
    extraImpl: extraImpl?.value || '',
    extraReview: extraReview?.value || '',
  };
  try { localStorage.setItem(RELAY_PREFS_KEY, JSON.stringify(prefs)); } catch (_) { /* private mode等は無視 */ }
}

function appendSelectOptions(select: HTMLSelectElement, selected: string): void {
  select.replaceChildren();
  ORCHESTRATION_CLI_OPTIONS.forEach((choice) => {
    const option = document.createElement('option');
    option.value = choice.value;
    option.textContent = choice.label || translated(choice.labelKey || '', choice.labelKey || choice.value);
    option.selected = choice.value === selected;
    select.appendChild(option);
  });
}

function buildRoleTable(prefs: RelayDialogPrefs): void {
  if (!roleTableBody) return;
  roleTableBody.replaceChildren();
  RELAY_ROLE_DEFS.forEach((definition) => {
    const row = document.createElement('tr');
    row.dataset.role = definition.key;

    const labelCell = document.createElement('td');
    labelCell.textContent = translated(definition.labelKey, definition.key);
    if (definition.required) {
      const required = document.createElement('span');
      required.className = 'relay-role-required';
      required.textContent = ' *';
      labelCell.appendChild(required);
    }

    const cliCell = document.createElement('td');
    const cli = document.createElement('select');
    cli.className = 'relay-role-cli';
    cli.setAttribute('aria-label', `${labelCell.textContent || definition.key} CLI`);
    appendSelectOptions(cli, prefs.roles?.[definition.key]?.provider || '');

    const modelCell = document.createElement('td');
    const model = document.createElement('input');
    model.type = 'text';
    model.className = 'relay-role-model';
    model.placeholder = translated('spawn_role_model_placeholder', 'Model name');
    model.value = prefs.roles?.[definition.key]?.model || '';
    model.disabled = !cli.value;
    model.setAttribute('aria-label', `${labelCell.textContent || definition.key} model`);

    cli.addEventListener('change', () => {
      model.disabled = !cli.value;
      if (!cli.value) model.value = '';
      updateFormState();
    });
    model.addEventListener('input', updateFormState);
    cliCell.appendChild(cli);
    modelCell.appendChild(model);
    const subscriptionCell = document.createElement('td');
    const subscription = document.createElement('select');
    subscription.className = 'relay-role-subscription';
    const refreshSubscription = (): void => {
      const profiles = selectableProfiles(cli.value);
      subscription.replaceChildren();
      const defaultOption = document.createElement('option');
      defaultOption.value = '';
      defaultOption.textContent = translated('spawn_subscription_default', 'Default');
      subscription.appendChild(defaultOption);
      profiles.forEach((profile) => {
        const option = document.createElement('option');
        option.value = profile.id;
        option.textContent = profile.name ? `${profile.name} (${profile.id})` : profile.id;
        subscription.appendChild(option);
      });
      subscription.hidden = profiles.length < 2;
      const saved = prefs.roles?.[definition.key]?.subscription || '';
      if (saved && Array.from(subscription.options).some((option) => option.value === saved)) subscription.value = saved;
    };
    refreshSubscription();
    subscription.setAttribute('aria-label', `${labelCell.textContent || definition.key} subscription`);
    cli.addEventListener('change', () => { refreshSubscription(); });
    subscriptionCell.appendChild(subscription);

    // 権限の段。空（既定）のままなら送らないので、relay の body は今までと一致する。
    // この build で選べない段は disabled で見せる（欄ごと消すより「まだ選べない」と
    // 分かる方がよい。確認ダイアログ・派生ダイアログと同じ規則）。
    const permissionCell = document.createElement('td');
    const permission = document.createElement('select');
    permission.className = 'relay-role-permission';
    const defaultPermission = document.createElement('option');
    defaultPermission.value = '';
    defaultPermission.textContent = translated('spawn_confirm_option_unset', 'Not specified');
    permission.appendChild(defaultPermission);
    PERMISSION_PRESET_SCHEMA.forEach((preset) => {
      const option = document.createElement('option');
      option.value = preset;
      option.textContent = permissionPresetLabel(preset);
      option.disabled = !isPermissionPresetAvailable(preset);
      permission.appendChild(option);
    });
    const savedPermission = prefs.roles?.[definition.key]?.permissionPreset || '';
    if (savedPermission && Array.from(permission.options).some((option) => option.value === savedPermission)) {
      permission.value = savedPermission;
    }
    permission.setAttribute('aria-label', `${labelCell.textContent || definition.key} permission tier`);
    permission.addEventListener('change', updateFormState);
    permissionCell.appendChild(permission);

    row.append(labelCell, cliCell, modelCell, subscriptionCell, permissionCell);
    roleTableBody.appendChild(row);
  });
}

function selectedRole(key: RelayRoleKey): RelayRoleValue {
  const row = Array.from(roleTableBody?.querySelectorAll<HTMLTableRowElement>('tr[data-role]') || [])
    .find((candidate) => candidate.dataset.role === key);
  return {
    provider: row?.querySelector<HTMLSelectElement>('.relay-role-cli')?.value || '',
    model: row?.querySelector<HTMLInputElement>('.relay-role-model')?.value.trim() || '',
    subscription: (() => { const select = row?.querySelector<HTMLSelectElement>('.relay-role-subscription'); return select && !select.hidden ? select.value : ''; })(),
    permissionPreset: row?.querySelector<HTMLSelectElement>('.relay-role-permission')?.value || '',
  };
}

function remainingSlots(): number | null {
  if (!capacityReady) return null;
  const strong = selectedRole('implementation-strong');
  const perRelay = strong.provider ? 3 : 2;
  return Math.max(0, Math.floor((maxChildren - liveChildren) / perRelay));
}

function setError(message: string): void {
  serverError = message;
  refreshErrorAndButton();
}

function refreshCapacityDisplay(): void {
  if (!remainingSlotsEl) return;
  if (capacityError) {
    remainingSlotsEl.textContent = translated('relay_remaining_slots_unknown', 'Relay capacity is unavailable');
    return;
  }
  const slots = remainingSlots();
  if (slots === null) {
    remainingSlotsEl.textContent = translated('relay_remaining_slots_unknown', 'Checking relay capacity…');
    return;
  }
  remainingSlotsEl.textContent = translated('relay_remaining_slots', 'You can run {n} more relay(s) at the same time', { n: slots });
}

function refreshErrorAndButton(): void {
  const implementation = selectedRole('implementation');
  const review = selectedRole('review');
  const missingRoles = !implementation.provider || !implementation.model || !review.provider || !review.model;
  const missingPlan = !planInput?.value.trim();
  const slots = remainingSlots();
  let message = '';
  if (missingRoles) {
    message = translated('relay_roles_required', 'Choose a CLI and model for implementation and review.');
  } else if (missingPlan) {
    message = translated('relay_plan_required', 'Choose or enter a plan file.');
  } else if (capacityError) {
    message = translated('relay_error_capacity', 'Relay capacity could not be read; try again.');
  } else if (slots === 0) {
    message = translated('relay_error_limit', 'The relay child limit has been reached; raise max_children_per_parent to run more.');
  } else if (serverError) {
    message = serverError;
  }
  if (errorEl) {
    errorEl.hidden = !message;
    errorEl.textContent = message;
  }
  if (startButton) {
    startButton.disabled = submitting || missingRoles || missingPlan || !capacityReady || capacityError || slots === 0 || currentSessionID === null;
  }
}

function updateFormState(): void {
  if (escalateAfter) escalateAfter.disabled = !selectedRole('implementation-strong').provider;
  const sameTree = selectedMode() === 'same-tree';
  if (sameTreeWarning) sameTreeWarning.hidden = !sameTree;
  refreshCapacityDisplay();
  refreshErrorAndButton();
}

function restorePrefs(prefs: RelayDialogPrefs): void {
  if (maxRounds) {
    const value = String(Number.isInteger(prefs.maxRounds) && (prefs.maxRounds as number) >= 1 && (prefs.maxRounds as number) <= 9 ? prefs.maxRounds : 3);
    maxRounds.value = value;
  }
  if (escalateAfter) {
    const value = String(Number.isInteger(prefs.escalateAfter) && (prefs.escalateAfter as number) >= 1 && (prefs.escalateAfter as number) <= 5 ? prefs.escalateAfter : 2);
    escalateAfter.value = value;
  }
  if (extraImpl) extraImpl.value = prefs.extraImpl || '';
  if (extraReview) extraReview.value = prefs.extraReview || '';
  const mode = prefs.mode === 'same-tree' ? 'same-tree' : 'worktree';
  if (sameTreeRadio) sameTreeRadio.checked = mode === 'same-tree';
  if (worktreeRadio) worktreeRadio.checked = mode === 'worktree';
}

function normalizePath(value: string): string {
  return value.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
}

function pathParent(value: string): string {
  const normalized = value.replace(/[\\/]+$/, '');
  const slash = Math.max(normalized.lastIndexOf('/'), normalized.lastIndexOf('\\'));
  return slash < 0 ? '' : normalized.slice(0, slash);
}

function planRootFor(session: SessionSnapshot): string {
  const cwd = String(session.cwd || '').replace(/[\\/]+$/, '');
  return `${cwd}/docs/local`;
}

async function loadPlanCandidates(session: SessionSnapshot, generation: number): Promise<void> {
  if (!planCandidates) return;
  const root = planRootFor(session);
  if (!String(session.cwd || '').trim()) return;
  try {
    const url = `/api/files-list?root=${encodeURIComponent(root)}&session=${encodeURIComponent(String(session.id))}&token=${encodeURIComponent(token || '')}`;
    const response = await fetch(url);
    if (!response.ok || generation !== openGeneration) return;
    const data = await response.json();
    const listedRoot = String(data.root || root);
    const rootKey = normalizePath(listedRoot);
    const candidates = (Array.isArray(data.items) ? data.items : [])
      .filter((item: any) => item && item.type !== 'dir' && /^plan_.*\.md$/i.test(String(item.name || '')))
      .filter((item: any) => normalizePath(pathParent(String(item.path || ''))) === rootKey)
      .sort((a: any, b: any) => String(a.name || '').localeCompare(String(b.name || '')));
    planCandidates.replaceChildren();
    const empty = document.createElement('option');
    empty.value = '';
    empty.textContent = translated('relay_plan_candidates_empty', 'Select a plan file…');
    planCandidates.appendChild(empty);
    candidates.forEach((item: any) => {
      const option = document.createElement('option');
      option.value = String(item.path || '');
      option.textContent = String(item.name || item.path || '');
      planCandidates.appendChild(option);
    });
    const current = planInput?.value.trim() || '';
    const matching = candidates.find((item: any) => normalizePath(String(item.path || '')) === normalizePath(current));
    if (matching && planCandidates) planCandidates.value = String(matching.path || '');
  } catch (_) {
    // Candidate discovery is an enhancement. The free-form path input remains usable.
  }
}

function isLiveChild(child: any): boolean {
  return !RELAY_TERMINAL_CHILD_STATES.has(String(child?.state || '').toLowerCase());
}

async function loadCapacity(sessionID: number, generation: number): Promise<void> {
  capacityReady = false;
  capacityError = false;
  liveChildren = 0;
  maxChildren = 0;
  refreshCapacityDisplay();
  refreshErrorAndButton();
  try {
    const base = `/api/sessions/${encodeURIComponent(String(sessionID))}`;
    const [childrenResponse, configResponse] = await Promise.all([
      fetch(`${base}/children?token=${encodeURIComponent(token || '')}`),
      fetch(`/api/orchestration-config?token=${encodeURIComponent(token || '')}`),
    ]);
    if (!childrenResponse.ok || !configResponse.ok) throw new Error('capacity request failed');
    const childrenData = await childrenResponse.json();
    const config = await configResponse.json();
    if (generation !== openGeneration || currentSessionID !== sessionID) return;
    const children = Array.isArray(childrenData.children) ? childrenData.children : [];
    const configuredMax = Number(config.max_children_per_parent);
    if (!Number.isFinite(configuredMax) || configuredMax <= 0) throw new Error('invalid capacity');
    liveChildren = children.filter(isLiveChild).length;
    maxChildren = Math.floor(configuredMax);
    capacityReady = true;
    updateFormState();
  } catch (_) {
    if (generation !== openGeneration) return;
    capacityError = true;
    refreshCapacityDisplay();
    refreshErrorAndButton();
  }
}

function relayErrorMessage(data: any, status: number): string {
  const error = String(data?.error || '');
  const keyByError: Record<string, string> = {
    relay_roles_missing: 'relay_error_roles_missing',
    orchestration_limit: 'relay_error_limit',
    relay_not_git: 'relay_error_not_git',
    relay_worktree_error: 'relay_error_worktree',
  };
  const key = keyByError[error];
  const detail = String(data?.detail || '').trim();
  if (key) {
    const label = translated(key, detail || error || `HTTP ${status}`);
    return detail && detail !== label ? `${label}: ${detail}` : label;
  }
  return detail || error || `HTTP ${status}`;
}

// 役割 1 件を Hub の JSON へ写す。画面側の camelCase（permissionPreset）は Hub の
// 欄名（permission_preset）へ変えて送る。段を選んでいない役割ではキーごと落とすので、
// 段の欄が無かった頃と同じ body になる。
function roleRequestBody(role: RelayRoleValue): Record<string, unknown> {
  const body: Record<string, unknown> = {
    provider: role.provider,
    model: role.model,
    subscription: role.subscription || '',
  };
  if (role.permissionPreset) body.permission_preset = role.permissionPreset;
  return body;
}

async function startRelay(): Promise<void> {
  if (currentSessionID === null || !startButton || startButton.disabled || !planInput) return;
  const implementation = selectedRole('implementation');
  const strong = selectedRole('implementation-strong');
  const review = selectedRole('review');
  const body: any = {
    plan_path: planInput.value.trim(),
    max_rounds: Number(maxRounds?.value || 3),
    escalate_after: Number(escalateAfter?.value || 2),
    mode: selectedMode(),
    roles: { implementation: roleRequestBody(implementation), review: roleRequestBody(review) },
    extra: {
      implementation: extraImpl?.value.trim() || '',
      review: extraReview?.value.trim() || '',
    },
    // Start click acknowledges full-bypass default for unattended relay children (F-AI-01 / D-12).
    acknowledge_child_full_bypass: true,
  };
  if (strong.provider) body.roles['implementation-strong'] = roleRequestBody(strong);
  savePrefs();
  submitting = true;
  updateFormState();
  try {
    const response = await fetch(`/api/sessions/${encodeURIComponent(String(currentSessionID))}/relay?token=${encodeURIComponent(token || '')}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    const data = await response.json().catch(() => ({}));
    if (!response.ok) {
      setError(relayErrorMessage(data, response.status));
      showToast(relayErrorMessage(data, response.status), startButton);
      return;
    }
    closeRelayDialog();
    showToast(translated('relay_started', 'Relay started'));
    window.renderOrchestrationDashboard?.();
    document.getElementById('orchestration-dashboard-pane')?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  } catch (error) {
    setError(String(error instanceof Error ? error.message : error));
    showToast(String(error instanceof Error ? error.message : error), startButton);
  } finally {
    submitting = false;
    refreshErrorAndButton();
  }
}

function closeRelayDialog(): void {
  savePrefs();
  openGeneration += 1;
  currentSessionID = null;
  submitting = false;
  if (overlay) overlay.hidden = true;
}

function setupListeners(): void {
  if (!overlay) return;
  closeButton?.addEventListener('click', closeRelayDialog);
  cancelButton?.addEventListener('click', closeRelayDialog);
  startButton?.addEventListener('click', () => { void startRelay(); });
  overlay.addEventListener('mousedown', (event) => {
    if (event.target === overlay) closeRelayDialog();
  });
  planCandidates?.addEventListener('change', () => {
    if (planInput && planCandidates.value) planInput.value = planCandidates.value;
    serverError = '';
    updateFormState();
  });
  planInput?.addEventListener('input', () => {
    if (planCandidates) planCandidates.value = '';
    serverError = '';
    updateFormState();
  });
  maxRounds?.addEventListener('change', updateFormState);
  escalateAfter?.addEventListener('change', updateFormState);
  extraImpl?.addEventListener('input', () => { serverError = ''; });
  extraReview?.addEventListener('input', () => { serverError = ''; });
  worktreeRadio?.addEventListener('change', updateFormState);
  sameTreeRadio?.addEventListener('change', updateFormState);
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && currentSessionID !== null && !overlay.hidden) {
      event.preventDefault();
      closeRelayDialog();
    }
  });
}

export function openRelayDialog(sessionID: number): void {
  if (!overlay) return;
  const session = sessions.get(sessionID);
  if (!session) {
    showToast(translated('relay_error_session_missing', 'The selected session is no longer available.'));
    return;
  }
  currentSessionID = sessionID;
  openGeneration += 1;
  const generation = openGeneration;
  serverError = '';
  submitting = false;
  capacityReady = false;
  capacityError = false;
  if (targetEl) targetEl.textContent = `#${session.id} ${String(session.provider || 'unknown')}`;
  if (planInput) planInput.value = '';
  if (planCandidates) {
    planCandidates.replaceChildren();
    const option = document.createElement('option');
    option.value = '';
    option.textContent = translated('relay_plan_candidates_empty', 'Select a plan file…');
    planCandidates.appendChild(option);
  }
  const prefs = readPrefs();
  buildRoleTable(prefs);
  restorePrefs(prefs);
  const gitAvailable = (session as SessionSnapshot & { git_available?: boolean }).git_available;
  if (worktreeRadio) {
    worktreeRadio.disabled = gitAvailable === false;
    if (worktreeRadio.disabled && sameTreeRadio) sameTreeRadio.checked = true;
  }
  overlay.hidden = false;
  updateFormState();
  void loadPlanCandidates(session, generation);
  void loadCapacity(sessionID, generation);
  planInput?.focus();
}

setupListeners();
onSubscriptionsChanged(() => {
  if (currentSessionID !== null) buildRoleTable(readPrefs());
});
void loadSubscriptions().then(() => {
  if (currentSessionID !== null) buildRoleTable(readPrefs());
});
