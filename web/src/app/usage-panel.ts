// Usage dropdown: provider links remain static, while profile usage is fetched
// from the Hub only when the menu is opened. No provider auth file is read here.
import { t } from '../i18n.js';
import { apiFetch, escapeHtml } from './util.js';

interface UsageWindow {
  used_percent: number;
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
  credits_balance?: string;
}

interface GrokUsage {
  used_percent: number;
  period_start?: string;
  period_end?: string;
  period_type?: string;
}

interface UsageProfile {
  id: string;
  name?: string;
  plan?: string;
  retrieved_at?: string;
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

function clampPercent(value: unknown): number {
  const n = typeof value === 'number' && Number.isFinite(value) ? value : 0;
  return Math.max(0, Math.min(100, n));
}

function pad(value: number): string {
  return String(value).padStart(2, '0');
}

function fixedDateTime(value: string | number | undefined): string {
  const date = typeof value === 'number' ? new Date(value * 1000) : new Date(value || '');
  if (!Number.isFinite(date.getTime())) return '';
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function resetText(epoch: number | undefined): string {
  if (!epoch || epoch <= 0) return '';
  return tx('usage_reset_at', 'Resets {time}', { time: fixedDateTime(epoch) });
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

function meter(label: string, window: UsageWindow | undefined): string {
  if (!window) return '';
  const percent = clampPercent(window.used_percent);
  return `<div class="usage-meter-line">
    <span class="usage-meter-label">${escapeHtml(label)}</span>
    <span class="usage-meter" role="img" aria-label="${escapeHtml(`${label} ${Math.round(percent)}%`)}"><span class="usage-meter-fill" style="width:${percent}%"></span></span>
    <span class="usage-meter-value">${Math.round(percent)}%</span>
  </div>`;
}

function actionButton(className: string, label: string, attribute: string, disabled = false): string {
  return `<button type="button" class="${className}" ${attribute}${disabled ? ' disabled' : ''}>${escapeHtml(label)}</button>`;
}

function hasClaudeUsage(profile: UsageProfile): boolean {
  return !!profile.claude;
}

function claudeBody(provider: string, profile: UsageProfile): string {
  const key = profileKey(profile, provider);
  const usage = hasClaudeUsage(profile);
  const busy = running.has(key) || profile.probe_state === 'running';
  const failure = failures.has(key) && !usage;
  let body = '';
  if (usage && profile.claude) {
    body += meter(tx('usage_window_5h', '5h'), profile.claude.five_hour);
    body += meter(tx('usage_window_7d', '7d'), profile.claude.seven_day);
    const retrieved = retrievedText(profile.retrieved_at);
    if (retrieved) body += `<div class="usage-profile-meta">${escapeHtml(retrieved)}</div>`;
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

function codexBody(profile: UsageProfile): string {
  const usage = profile.codex;
  if (!usage || (!usage.primary && !usage.secondary)) {
    return `<span class="usage-not-acquired">${escapeHtml(tx('usage_profile_unacquired', 'Not retrieved'))}</span>`;
  }
  const window = usage.primary || usage.secondary;
  return `${meter(tx('usage_window_weekly', 'Weekly'), window)}${resetText(window?.resets_at) ? `<div class="usage-profile-meta">${escapeHtml(resetText(window?.resets_at))}</div>` : ''}`;
}

function grokBody(profile: UsageProfile): string {
  const usage = profile.grok;
  if (!usage) {
    return `<span class="usage-not-acquired">${escapeHtml(tx('usage_profile_grok_unacquired', 'Launch Grok on this subscription to see numbers'))}</span>`;
  }
  const end = usage.period_end ? fixedDateTime(usage.period_end) : '';
  const retrieved = retrievedText(profile.retrieved_at);
  let body = meter(tx('usage_window_weekly', 'Weekly'), { used_percent: usage.used_percent });
  if (end) body += `<div class="usage-profile-meta">${escapeHtml(tx('usage_period_end', 'Billing period ends {time}', { time: end }))}</div>`;
  if (retrieved) body += `<div class="usage-profile-meta">${escapeHtml(retrieved)}</div>`;
  return body;
}

function profileBody(provider: string, profile: UsageProfile): string {
  switch (provider) {
    case 'claude': return claudeBody(provider, profile);
    case 'codex': return codexBody(profile);
    case 'grok': return grokBody(profile);
    default: return '';
  }
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
      row.innerHTML = `<div class="usage-profile-head"><span class="usage-profile-name" title="${escapeHtml(name)}">${escapeHtml(name)}</span>${profile.plan ? `<span class="usage-profile-plan">${escapeHtml(profile.plan)}</span>` : ''}</div><div class="usage-profile-body">${profileBody(provider.provider, profile)}</div>`;
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
    backdrop.className = 'usage-probe-dialog-backdrop';
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
    if (!latest || !hasClaudeUsage(latest)) {
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

export async function refreshUsagePanel(): Promise<void> {
  if (!panelRoot || refreshInFlight) return refreshInFlight;
  refreshInFlight = (async () => {
    try {
      const response = await apiFetch('/api/subscription-usage');
      if (!response.ok) throw new Error(`usage request failed: ${response.status}`);
      usageData = await response.json() as UsageResponse;
      for (const provider of usageData.providers || []) {
        for (const profile of provider.profiles || []) {
          const key = profileKey(profile, provider.provider);
          if (hasClaudeUsage(profile)) failures.delete(key);
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
