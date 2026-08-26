// Usage dropdown: provider links remain static, while profile usage is fetched
// from the Hub only when the menu is opened. No provider auth file is read here.
import { t } from '../i18n.js';
import { apiFetch, escapeHtml } from './util.js';
import {
  remainingPercent,
  resetState,
  sortUsageWindows,
  usageSeverity,
  usedPercent,
  windowLabel,
  windowMinutes,
  type UsageWindowInput,
} from './usage-limit.js';

interface UsageWindow {
  used_percent?: number;
  remaining_percent?: number;
  window_minutes?: number;
  resets_at?: number;
}

interface ClaudeUsage {
  five_hour?: UsageWindow;
  seven_day?: UsageWindow;
}

interface CodexUsage {
  primary?: UsageWindow;
  secondary?: UsageWindow;
  plan_type?: string;
  credits?: {
    has_credits?: boolean;
    unlimited?: boolean;
    balance?: string;
  };
  credits_balance?: string;
}

interface GrokUsage {
  used_percent: number;
  remaining_percent?: number;
  period_start?: string;
  period_end?: string;
  period_type?: string;
}

interface UsageProfile {
  id: string;
  name?: string;
  plan?: string;
  retrieved_at?: string;
  auth_status?: 'ready' | 'login_required' | 'status_unknown' | string;
  probe_available?: boolean;
  probe_state?: string;
  claude?: ClaudeUsage;
  codex?: CodexUsage;
  grok?: GrokUsage;
}

interface UsageProvider {
  provider: string;
  profiles: UsageProfile[];
}

interface UsageResponse {
  providers: UsageProvider[];
}

const PROBE_CONFIRM_KEY = 'many-ai-cli-usage-probe-confirmed';
const running = new Set<string>();
const failures = new Map<string, string>();
let panelRoot: HTMLElement | null = null;
let usageData: UsageResponse | null = null;
let refreshInFlight: Promise<void> | null = null;

function tx(key: string, fallback: string, vars: Record<string, unknown> = {}): string {
  let value = t(key, vars);
  if (value === key) value = fallback;
  for (const [name, replacement] of Object.entries(vars)) {
    value = value.replaceAll(`{${name}}`, String(replacement));
  }
  return value;
}

function profileKey(profile: UsageProfile, provider: string): string {
  return `${provider}:${profile.id}`;
}

function pad(value: number): string {
  return String(value).padStart(2, '0');
}

function fixedDateTime(value: string | number | undefined): string {
  const date = typeof value === 'number' ? new Date(value * 1000) : new Date(value || '');
  if (!Number.isFinite(date.getTime())) return '';
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function retrievedText(iso: string | undefined): string {
  if (!iso) return '';
  const date = new Date(iso);
  if (!Number.isFinite(date.getTime())) return '';
  const minutes = Math.max(0, Math.floor((Date.now() - date.getTime()) / 60000));
  if (minutes < 1) return tx('usage_retrieved_just_now', 'Retrieved just now');
  if (minutes < 60) return tx('usage_retrieved_minutes', 'Retrieved {n}m ago', { n: minutes });
  return tx('usage_retrieved_at', 'Retrieved {time}', { time: fixedDateTime(iso) });
}

function relativeDuration(milliseconds: number): string {
  const minutes = Math.floor(milliseconds / 60000);
  if (minutes < 1) return tx('usage_reset_less_than_minute', '<1m');
  if (minutes < 60) return tx('usage_reset_minutes', '{n}m', { n: minutes });
  const hours = Math.floor(minutes / 60);
  const restMinutes = minutes % 60;
  if (hours < 24) {
    return restMinutes
      ? tx('usage_reset_hours_minutes', '{h}h {m}m', { h: hours, m: restMinutes })
      : tx('usage_reset_hours', '{n}h', { n: hours });
  }
  const days = Math.floor(hours / 24);
  const restHours = hours % 24;
  return restHours
    ? tx('usage_reset_days_hours', '{d}d {h}h', { d: days, h: restHours })
    : tx('usage_reset_days', '{n}d', { n: days });
}

function resetMeta(epoch: number | undefined): { text: string; stale: boolean } {
  const state = resetState(epoch);
  if (!state) return { text: '', stale: false };
  if (state.stale) return { text: tx('usage_reset_elapsed', 'Reset time passed'), stale: true };
  return {
    text: tx('usage_reset_in', 'Resets in {time} · {absolute}', {
      time: relativeDuration(state.remainingMs),
      absolute: fixedDateTime(epoch),
    }),
    stale: false,
  };
}

function windowLabelText(label: ReturnType<typeof windowLabel>): string {
  switch (label.kind) {
    case 'five_hour': return tx('usage_window_5h', '5h');
    case 'weekly': return tx('usage_window_weekly', 'Weekly');
    case 'days': return tx('usage_window_days', '{n}d', { n: label.amount });
    case 'hours': return tx('usage_window_hours', '{n}h', { n: label.amount });
    case 'minutes': return tx('usage_window_minutes', '{n}m', { n: label.amount });
    default: return tx('usage_window_generic', 'Limit');
  }
}

function meter(label: string, window: UsageWindow | undefined): string {
  if (!window) return '';
  const remaining = remainingPercent(window as UsageWindowInput);
  if (remaining === null) return '';
  const used = usedPercent(window as UsageWindowInput) ?? 0;
  const roundedRemaining = Math.round(remaining);
  const roundedUsed = Math.round(used);
  const severity = usageSeverity(remaining);
  const reset = resetMeta(window.resets_at);
  const stateText = severity === 'danger'
    ? tx('usage_remaining_danger', 'Danger: low remaining quota')
    : severity === 'warning'
      ? tx('usage_remaining_warning', 'Warning: remaining quota is low')
      : '';
  const meta = [
    reset.text,
    stateText,
  ].filter(Boolean).join(' · ');
  const aria = `${label}: ${tx('usage_remaining_aria', '{remaining}% remaining, {used}% used', { remaining: roundedRemaining, used: roundedUsed })}${stateText ? `, ${stateText}` : ''}`;
  const title = `${tx('usage_remaining_title', 'Remaining {remaining}% · Used {used}%', { remaining: roundedRemaining, used: roundedUsed })}${reset.text ? `\n${reset.text}` : ''}`;
  return `<div class="usage-meter-group${reset.stale ? ' usage-meter-group--stale' : ''}">
    <div class="usage-meter-line" data-usage-severity="${severity}">
      <span class="usage-meter-label">${escapeHtml(label)}</span>
      <span class="usage-meter" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${roundedRemaining}" aria-label="${escapeHtml(aria)}" title="${escapeHtml(title)}"><span class="usage-meter-fill" style="width:${roundedRemaining}%"></span></span>
      <span class="usage-meter-value">${escapeHtml(tx('usage_remaining_value', 'Remaining {n}%', { n: roundedRemaining }))}</span>
    </div>${meta ? `<div class="usage-meter-meta${reset.stale ? ' usage-meter-meta--stale' : ''}">${escapeHtml(meta)}</div>` : ''}
  </div>`;
}

function actionButton(className: string, label: string, attribute: string, disabled = false): string {
  return `<button type="button" class="${className}" ${attribute}${disabled ? ' disabled' : ''}>${escapeHtml(label)}</button>`;
}

function hasWindows(usage: ClaudeUsage | CodexUsage | undefined): boolean {
  return !!usage && Object.values(usage).some((window) => window && typeof window === 'object' && remainingPercent(window as UsageWindowInput) !== null);
}

function settingsAction(provider: string, id: string): string {
  return actionButton('usage-auth-action', tx('usage_auth_settings', 'Open subscription settings'), `data-open-subscription-settings="${escapeHtml(provider)}" data-subscription-id="${escapeHtml(id)}"`);
}

function authNotice(provider: string, profile: UsageProfile): string {
  const state = profile.auth_status;
  if (state === 'login_required') {
    return `<div class="usage-auth usage-auth--required" role="alert"><strong>${escapeHtml(tx('usage_auth_required', 'Re-authentication required'))}</strong><span>${escapeHtml(tx('usage_auth_required_detail', 'This provider profile is not signed in.'))}</span>${settingsAction(provider, profile.id)}</div>`;
  }
  if (state === 'status_unknown') {
    return `<div class="usage-auth usage-auth--unknown" role="status"><strong>${escapeHtml(tx('usage_auth_unknown', 'Login status could not be checked'))}</strong><span>${escapeHtml(tx('usage_auth_unknown_detail', 'This is not proof that the profile is signed out.'))}</span>${actionButton('usage-auth-retry', tx('usage_auth_retry', 'Retry'), `data-usage-auth-retry="${escapeHtml(profileKey(profile, provider))}"`)}</div>`;
  }
  return '';
}

function noWindowsText(): string {
  return `<span class="usage-not-acquired">${escapeHtml(tx('usage_no_windows', 'No usage limits are currently available'))}</span>`;
}

function profileRetrieved(profile: UsageProfile): string {
  const retrieved = retrievedText(profile.retrieved_at);
  return retrieved ? `<div class="usage-profile-meta">${escapeHtml(retrieved)}</div>` : '';
}

function creditsText(usage: CodexUsage): string {
  const credits = usage.credits;
  if (!credits) {
    return usage.credits_balance
      ? `<span>${escapeHtml(tx('usage_credits_balance', 'Remaining credits {value}', { value: usage.credits_balance }))}</span>`
      : '';
  }
  if (credits.unlimited) return `<span>${escapeHtml(tx('usage_credits_unlimited', 'Unlimited credits'))}</span>`;
  if (credits.has_credits) return `<span>${escapeHtml(tx('usage_credits_balance', 'Remaining credits {value}', { value: credits.balance ?? '' }))}</span>`;
  return `<span>${escapeHtml(tx('usage_credits_none', 'No additional credits'))}</span>`;
}

function claudeBody(provider: string, profile: UsageProfile): string {
  const key = profileKey(profile, provider);
  const usage = profile.claude;
  const hasUsage = hasWindows(usage);
  const busy = running.has(key) || profile.probe_state === 'running';
  const failure = failures.has(key) && !usage;
  let body = authNotice(provider, profile);
  if (hasUsage && usage) {
    body += meter(tx('usage_window_5h', '5h'), usage.five_hour);
    body += meter(tx('usage_window_7d', '7d'), usage.seven_day);
    body += profileRetrieved(profile);
  } else if (usage) {
    body += noWindowsText();
    body += profileRetrieved(profile);
  } else {
    body += `<div class="usage-profile-actions"><span class="usage-not-acquired">${escapeHtml(tx('usage_profile_unacquired', 'Not retrieved'))}</span></div>`;
  }
  if (busy) {
    body += `<div class="usage-profile-actions"><span class="usage-probe-state">${escapeHtml(tx('usage_probe_running', 'Retrieving'))}</span>${actionButton('usage-probe-cancel', tx('usage_probe_cancel', 'Cancel'), `data-probe-cancel="${escapeHtml(key)}"`)}</div>`;
  } else if (failure) {
    body += `<div class="usage-profile-actions"><span class="usage-probe-error">${escapeHtml(tx('usage_probe_failed', 'Could not retrieve usage'))}</span>${actionButton('usage-probe-button', tx('usage_probe_retry', 'Retry'), `data-probe-start="${escapeHtml(key)}"`)}</div>`;
  } else if (!usage && profile.probe_available) {
    body += `<div class="usage-profile-actions">${actionButton('usage-probe-button', tx('usage_probe_get', 'Retrieve'), `data-probe-start="${escapeHtml(key)}"`)}</div>`;
  }
  return body;
}

function codexBody(provider: string, profile: UsageProfile): string {
  const usage = profile.codex;
  let body = authNotice(provider, profile);
  if (!usage) return body + `<span class="usage-not-acquired">${escapeHtml(tx('usage_profile_unacquired', 'Not retrieved'))}</span>`;
  const windows = sortUsageWindows([usage.primary, usage.secondary].filter((window): window is UsageWindow => !!window && remainingPercent(window) !== null));
  if (windows.length === 0) {
    body += noWindowsText();
  } else {
    for (const window of windows) {
      body += meter(windowLabelText(windowLabel(windowMinutes(window))), window);
    }
  }
  const credits = creditsText(usage);
  if (credits) body += `<div class="usage-profile-facts">${credits}</div>`;
  body += profileRetrieved(profile);
  return body;
}

function grokBody(provider: string, profile: UsageProfile): string {
  const usage = profile.grok;
  let body = authNotice(provider, profile);
  if (!usage) return body + `<span class="usage-not-acquired">${escapeHtml(tx('usage_profile_grok_unacquired', 'Launch Grok on this subscription to see numbers'))}</span>`;
  const end = usage.period_end ? fixedDateTime(usage.period_end) : '';
  body += meter(tx('usage_window_weekly', 'Weekly'), { used_percent: usage.used_percent, remaining_percent: usage.remaining_percent });
  if (end) body += `<div class="usage-profile-meta">${escapeHtml(tx('usage_period_end', 'Billing period ends {time}', { time: end }))}</div>`;
  body += profileRetrieved(profile);
  return body;
}

function profileBody(provider: string, profile: UsageProfile): string {
  switch (provider) {
    case 'claude': return claudeBody(provider, profile);
    case 'codex': return codexBody(provider, profile);
    case 'grok': return grokBody(provider, profile);
    default: return '';
  }
}

function profilePlan(provider: string, profile: UsageProfile): string {
  const configured = profile.plan?.trim() || '';
  const observed = provider === 'codex' ? profile.codex?.plan_type?.trim() || '' : '';
  if (configured && observed && configured.toLowerCase() === observed.toLowerCase()) return configured;
  return observed || configured;
}

function renderProfiles(response: UsageResponse): void {
  if (!panelRoot) return;
  panelRoot.querySelectorAll('.usage-profile-list').forEach((el) => el.remove());
  for (const provider of response.providers || []) {
    if (!provider.profiles || provider.profiles.length === 0) continue;
    const anchor = Array.from(panelRoot.querySelectorAll<HTMLElement>('[data-usage-provider]'))
      .find((el) => el.dataset.usageProvider === provider.provider);
    if (!anchor) continue;
    const list = document.createElement('div');
    list.className = 'usage-profile-list';
    for (const profile of provider.profiles) {
      const row = document.createElement('div');
      row.className = 'usage-profile-row';
      const name = profile.name || profile.id;
      const plan = profilePlan(provider.provider, profile);
      row.innerHTML = `<div class="usage-profile-head"><span class="usage-profile-name" title="${escapeHtml(name)}">${escapeHtml(name)}</span>${plan ? `<span class="usage-profile-plan">${escapeHtml(plan)}</span>` : ''}</div><div class="usage-profile-body">${profileBody(provider.provider, profile)}</div>`;
      list.appendChild(row);
    }
    anchor.insertAdjacentElement('afterend', list);
  }
  bindProbeButtons();
}

function bindProbeButtons(): void {
  if (!panelRoot) return;
  panelRoot.querySelectorAll<HTMLButtonElement>('[data-probe-start]').forEach((button) => {
    if (button.dataset.bound === '1') return;
    button.dataset.bound = '1';
    button.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      const key = button.dataset.probeStart || '';
      void startProbe(key);
    });
  });
  panelRoot.querySelectorAll<HTMLButtonElement>('[data-probe-cancel]').forEach((button) => {
    if (button.dataset.bound === '1') return;
    button.dataset.bound = '1';
    button.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      const key = button.dataset.probeCancel || '';
      void cancelProbe(key);
    });
  });
  panelRoot.querySelectorAll<HTMLButtonElement>('[data-open-subscription-settings]').forEach((button) => {
    if (button.dataset.bound === '1') return;
    button.dataset.bound = '1';
    button.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      openSubscriptionSettings(button.dataset.openSubscriptionSettings || '', button.dataset.subscriptionId || '');
    });
  });
  panelRoot.querySelectorAll<HTMLButtonElement>('[data-usage-auth-retry]').forEach((button) => {
    if (button.dataset.bound === '1') return;
    button.dataset.bound = '1';
    button.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      button.disabled = true;
      void refreshUsagePanel(true).finally(() => { button.disabled = false; });
    });
  });
}

function openSubscriptionSettings(provider: string, id: string): void {
  const settings = document.getElementById('settings-panel') as HTMLElement | null;
  const settingsButton = document.getElementById('settings-btn');
  if (!settings) return;
  if (settings.hidden) settingsButton?.click();
  const reveal = () => {
    const section = settings.querySelector<HTMLDetailsElement>('details[data-section="subscriptions"]');
    if (section) section.open = true;
    const row = Array.from(settings.querySelectorAll<HTMLElement>('.subs-row')).find((candidate) => (
      candidate.dataset.provider === provider && candidate.dataset.id === id
    ));
    row?.scrollIntoView({ block: 'center' });
    (row?.querySelector('.subs-login') as HTMLButtonElement | null)?.focus();
  };
  requestAnimationFrame(() => {
    reveal();
    setTimeout(reveal, 50);
  });
}

function findProfile(key: string): UsageProfile | null {
  if (!usageData) return null;
  for (const provider of usageData.providers || []) {
    for (const profile of provider.profiles || []) {
      if (profileKey(profile, provider.provider) === key) return profile;
    }
  }
  return null;
}

async function confirmProbe(profile: UsageProfile): Promise<boolean> {
  try {
    if (localStorage.getItem(PROBE_CONFIRM_KEY) === '1') return true;
  } catch (_) {}
  const name = profile.name || profile.id;
  return new Promise<boolean>((resolve) => {
    const backdrop = document.createElement('div');
    backdrop.className = 'usage-probe-dialog-backdrop aac-wheel-overlay';
    backdrop.innerHTML = `<div class="usage-probe-dialog" role="dialog" aria-modal="true" aria-labelledby="usage-probe-dialog-title">
      <h2 id="usage-probe-dialog-title">${escapeHtml(tx('usage_probe_confirm_title', 'Retrieve subscription usage'))}</h2>
      <p>${escapeHtml(tx('usage_probe_confirm_profile', 'Profile: {name}', { name }))}</p>
      <p>${escapeHtml(tx('usage_probe_confirm_body', 'Claude will start briefly, read usage from its statusLine, and close.'))}</p>
      <p>${escapeHtml(tx('usage_probe_confirm_cost', 'This uses a small amount of this subscription quota.'))}</p>
      <label><input type="checkbox" data-probe-confirm-skip> ${escapeHtml(tx('usage_probe_confirm_check', 'Do not ask again'))}</label>
      <div class="usage-probe-dialog-actions"><button type="button" data-probe-dialog-cancel>${escapeHtml(tx('usage_probe_confirm_cancel', 'Cancel'))}</button><button type="button" class="primary" data-probe-dialog-start>${escapeHtml(tx('usage_probe_confirm_start', 'Retrieve'))}</button></div>
    </div>`;
    document.body.appendChild(backdrop);
    const finish = (value: boolean) => {
      const skip = backdrop.querySelector<HTMLInputElement>('[data-probe-confirm-skip]')?.checked;
      if (value && skip) {
        try { localStorage.setItem(PROBE_CONFIRM_KEY, '1'); } catch (_) {}
      }
      document.removeEventListener('keydown', onKeyDown);
      backdrop.remove();
      resolve(value);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') finish(false);
    };
    backdrop.querySelector('[data-probe-dialog-cancel]')?.addEventListener('click', () => finish(false));
    backdrop.querySelector('[data-probe-dialog-start]')?.addEventListener('click', () => finish(true));
    backdrop.addEventListener('click', (event) => {
      if (event.target === backdrop) finish(false);
    });
    document.addEventListener('keydown', onKeyDown);
    (backdrop.querySelector('[data-probe-dialog-start]') as HTMLButtonElement | null)?.focus();
  });
}

async function startProbe(key: string): Promise<void> {
  if (!key || running.has(key)) return;
  const profile = findProfile(key);
  if (!profile || !profile.probe_available) return;
  if (!await confirmProbe(profile)) return;
  running.add(key);
  failures.delete(key);
  if (usageData) renderProfiles(usageData);
  const [, id] = key.split(':');
  try {
    const response = await apiFetch('/api/subscription-usage/probe', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ provider: 'claude', id }),
    });
    if (!response.ok) throw new Error(`probe failed: ${response.status}`);
    running.delete(key);
    await refreshUsagePanel();
  } catch (_) {
    running.delete(key);
    await refreshUsagePanel();
    const latest = findProfile(key);
    if (!latest || !hasWindows(latest.claude)) {
      failures.set(key, tx('usage_probe_failed', 'Could not retrieve usage'));
      if (usageData) renderProfiles(usageData);
    }
  }
}

async function cancelProbe(key: string): Promise<void> {
  if (!key) return;
  const [, id] = key.split(':');
  try {
    const response = await apiFetch('/api/subscription-usage/probe', {
      method: 'DELETE',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ provider: 'claude', id }),
    });
    if (!response.ok) throw new Error(`cancel failed: ${response.status}`);
    running.delete(key);
    await refreshUsagePanel();
  } catch (_) {
    if (usageData) renderProfiles(usageData);
  }
}

function bindUnavailableInfo(): void {
  if (!panelRoot) return;
  panelRoot.querySelectorAll<HTMLButtonElement>('.usage-info').forEach((button) => {
    if (button.dataset.bound === '1') return;
    button.dataset.bound = '1';
    const bubble = button.parentElement?.querySelector<HTMLElement>('.usage-info-bubble');
    if (!bubble) return;
    button.setAttribute('aria-label', tx('usage_info_label', 'More information'));
    const row = button.parentElement;
    const setOpen = (open: boolean) => {
      bubble.hidden = !open;
      button.setAttribute('aria-expanded', String(open));
    };
    button.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      setOpen(bubble.hidden === true);
    });
    button.addEventListener('pointerenter', () => setOpen(true));
    button.addEventListener('pointerleave', () => setOpen(false));
    button.addEventListener('focus', () => setOpen(true));
    button.addEventListener('blur', () => setOpen(false));
    row?.addEventListener('pointerleave', () => setOpen(false));
  });
}

export function initUsagePanel(dropdown: HTMLElement): void {
  if (panelRoot === dropdown) return;
  panelRoot = dropdown;
  bindUnavailableInfo();
  document.addEventListener('pointerdown', (event) => {
    if (!panelRoot || panelRoot.contains(event.target as Node)) return;
    panelRoot.querySelectorAll<HTMLElement>('.usage-info-bubble:not([hidden])').forEach((bubble) => {
      bubble.hidden = true;
      bubble.parentElement?.querySelector('.usage-info')?.setAttribute('aria-expanded', 'false');
    });
  });
}

export async function refreshUsagePanel(forceAuth = false): Promise<void> {
  if (!panelRoot || refreshInFlight) return refreshInFlight;
  refreshInFlight = (async () => {
    try {
      const response = await apiFetch(`/api/subscription-usage${forceAuth ? '?refresh_auth=1' : ''}`);
      if (!response.ok) throw new Error(`usage request failed: ${response.status}`);
      usageData = await response.json() as UsageResponse;
      for (const provider of usageData.providers || []) {
        for (const profile of provider.profiles || []) {
          const key = profileKey(profile, provider.provider);
          if (hasWindows(profile.claude)) failures.delete(key);
          if (profile.probe_state !== 'running') running.delete(key);
        }
      }
      renderProfiles(usageData);
    } catch (_) {
      // Keep the static links and the last successful values visible. The UI
      // intentionally has no stale/expiry state; a failed refresh is not proof
      // that the last provider-reported value is invalid.
      if (usageData) renderProfiles(usageData);
    } finally {
      refreshInFlight = null;
    }
  })();
  return refreshInFlight;
}
