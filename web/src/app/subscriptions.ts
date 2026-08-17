// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { escapeHtml, showToast, token } from './util.js';
import { appConfirm } from './settings.js';

// ---- Settings > Subscriptions（plan_multi-subscription-pool C4）----
//
// 1 つの AI CLI に複数のログインを持たせ、セッション起動時にどれを使うか選ぶための
// 管理画面。ここで扱うのは「profile が何個あって、どれが有効か」だけで、
// **token / 認証ファイルの中身は Hub API から返ってこないし、ここにも出さない。**

export interface SubscriptionProfileEntry {
  provider: string;
  id: string;
  name?: string;
  plan?: string;
  enabled: boolean;
  profile_dir?: string;
  exists: boolean;
  issue?: string;
}

export interface SubscriptionProviderEntry {
  provider: string;
  supported: boolean;
  env_var?: string;
  profiles: SubscriptionProfileEntry[];
}

const PROVIDER_LABELS: Record<string, string> = {
  claude: 'Claude Code',
  codex: 'Codex CLI',
  copilot: 'GitHub Copilot',
  'cursor-agent': 'Cursor Agent',
  opencode: 'OpenCode',
  grok: 'Grok Build',
};

let cachedProviders: SubscriptionProviderEntry[] = [];
let loadPromise: Promise<SubscriptionProviderEntry[]> | null = null;
const changeListeners = new Set<() => void>();

function providerLabel(provider: string): string {
  return PROVIDER_LABELS[provider] || provider;
}

function subsApi(path: string, init?: RequestInit): Promise<Response> {
  const sep = path.includes('?') ? '&' : '?';
  return fetch(`${path}${sep}token=${encodeURIComponent(token || '')}`, init);
}

const jsonInit = (bodyObj: unknown): RequestInit => ({
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(bodyObj),
});

/** 直近に取得した provider 一覧（spawn パネルが同期的に参照する）。 */
export function getSubscriptionProviders(): SubscriptionProviderEntry[] {
  return cachedProviders;
}

/** provider の「選択できる」profile（有効かつ設定が壊れていないもの）だけを返す。 */
export function selectableProfiles(provider: string): SubscriptionProfileEntry[] {
  const entry = cachedProviders.find((p) => p.provider === provider);
  if (!entry || !entry.supported) return [];
  return entry.profiles.filter((p) => p.enabled && !p.issue);
}

export function onSubscriptionsChanged(cb: () => void): void {
  changeListeners.add(cb);
}

function notifyChanged(): void {
  changeListeners.forEach((cb) => {
    try { cb(); } catch (_) {}
  });
}

function applyPayload(data: unknown): SubscriptionProviderEntry[] {
  const providers = (data as { providers?: SubscriptionProviderEntry[] })?.providers;
  cachedProviders = Array.isArray(providers) ? providers : [];
  notifyChanged();
  return cachedProviders;
}

/** 一覧を取り直す。失敗しても例外を投げず、空配列にフォールバックする。 */
export async function loadSubscriptions(force = false): Promise<SubscriptionProviderEntry[]> {
  if (!force && loadPromise) return loadPromise;
  loadPromise = (async () => {
    try {
      const res = await subsApi('/api/subscriptions');
      if (!res.ok) return cachedProviders;
      return applyPayload(await res.json());
    } catch (_) {
      return cachedProviders;
    }
  })();
  const result = await loadPromise;
  loadPromise = null;
  return result;
}

function setError(message: string): void {
  const el = document.getElementById('subs-error');
  if (!el) return;
  el.textContent = message;
  el.hidden = !message;
}

function statusText(p: SubscriptionProfileEntry): string {
  if (p.issue) return p.issue;
  if (!p.enabled) return t('subs_status_disabled');
  if (!p.exists) return t('subs_status_needs_login');
  return p.plan ? t('subs_status_ready_plan', { plan: p.plan }) : t('subs_status_ready');
}

function renderList(): void {
  const host = document.getElementById('subs-list');
  if (!host) return;
  const supported = cachedProviders.filter((entry) => entry.supported || entry.profiles.length > 0);
  if (supported.length === 0) {
    host.innerHTML = `<div class="settings-note">${escapeHtml(t('subs_none_supported'))}</div>`;
    updateSummary();
    return;
  }
  host.innerHTML = supported.map((entry) => {
    const rows = entry.profiles.map((p) => {
      const disabledCls = p.enabled ? '' : ' subs-row--off';
      const issueCls = p.issue ? ' subs-row--issue' : '';
      return (
        `<div class="subs-row${disabledCls}${issueCls}" data-provider="${escapeHtml(entry.provider)}" data-id="${escapeHtml(p.id)}">` +
        `<input class="subs-name settings-input-text" type="text" value="${escapeHtml(p.name || '')}" ` +
        `placeholder="${escapeHtml(t('subs_name_placeholder'))}" aria-label="${escapeHtml(t('subs_name_placeholder'))}">` +
        `<span class="subs-id">${escapeHtml(p.id)}</span>` +
        `<span class="subs-status">${escapeHtml(statusText(p))}</span>` +
        `<div class="subs-actions">` +
        `<button type="button" class="settings-inline-btn subs-login">${escapeHtml(t('subs_login'))}</button>` +
        `<button type="button" class="settings-inline-btn subs-test">${escapeHtml(t('subs_test'))}</button>` +
        `<button type="button" class="settings-inline-btn subs-toggle">${escapeHtml(p.enabled ? t('subs_disable') : t('subs_enable'))}</button>` +
        `<button type="button" class="settings-inline-btn danger subs-remove">${escapeHtml(t('subs_remove'))}</button>` +
        `</div></div>`
      );
    }).join('');
    const envNote = entry.env_var
      ? `<span class="subs-envvar" data-tooltip="${escapeHtml(t('subs_env_var_tooltip'))}">${escapeHtml(entry.env_var)}</span>`
      : '';
    const unsupported = entry.supported
      ? ''
      : `<div class="settings-note settings-note-warn">${escapeHtml(t('subs_provider_unsupported'))}</div>`;
    const addRow = entry.supported
      ? `<div class="subs-add" data-provider="${escapeHtml(entry.provider)}">` +
        `<input class="subs-add-name settings-input-text" type="text" placeholder="${escapeHtml(t('subs_add_placeholder'))}" ` +
        `aria-label="${escapeHtml(t('subs_add_placeholder'))}">` +
        `<button type="button" class="settings-inline-btn subs-add-btn">${escapeHtml(t('subs_add'))}</button>` +
        `</div>`
      : '';
    return (
      `<div class="subs-provider" data-provider="${escapeHtml(entry.provider)}">` +
      `<div class="subs-provider-head"><span class="subs-provider-name">${escapeHtml(providerLabel(entry.provider))}</span>${envNote}</div>` +
      unsupported + rows + addRow +
      `</div>`
    );
  }).join('');
  updateSummary();
}

function updateSummary(): void {
  const el = document.querySelector('.settings-section-current[data-section="subscriptions"]') as HTMLElement | null;
  if (!el) return;
  const count = cachedProviders.reduce((n, entry) => n + entry.profiles.length, 0);
  el.textContent = count === 0 ? t('subs_summary_none') : t('subs_summary_count', { count });
}

function rowContext(el: Element): { provider: string; id: string } | null {
  const row = el.closest('.subs-row') as HTMLElement | null;
  if (!row) return null;
  const provider = row.dataset.provider || '';
  const id = row.dataset.id || '';
  if (!provider || !id) return null;
  return { provider, id };
}

async function postAndRefresh(path: string, body: unknown, btn: HTMLElement, failKey: string): Promise<boolean> {
  try {
    const res = await subsApi(path, jsonInit(body));
    if (!res.ok) {
      const detail = await res.json().catch(() => ({} as { detail?: string }));
      const message = (detail && typeof detail.detail === 'string' && detail.detail) ? detail.detail : t(failKey);
      setError(message);
      showToast(t(failKey), btn);
      return false;
    }
    setError('');
    applyPayload(await res.json().catch(() => ({})));
    renderList();
    return true;
  } catch (_) {
    showToast(t(failKey), btn);
    return false;
  }
}

async function addProfile(btn: HTMLElement): Promise<void> {
  const box = btn.closest('.subs-add') as HTMLElement | null;
  const provider = box?.dataset.provider || '';
  const input = box?.querySelector('.subs-add-name') as HTMLInputElement | null;
  const name = (input?.value || '').trim();
  if (!provider) return;
  if (!name) {
    showToast(t('subs_add_name_required'), btn);
    input?.focus();
    return;
  }
  const ok = await postAndRefresh('/api/subscriptions', { provider, name }, btn, 'subs_add_failed');
  if (ok && input) input.value = '';
}

async function renameProfile(input: HTMLInputElement): Promise<void> {
  const ctx = rowContext(input);
  if (!ctx) return;
  await postAndRefresh('/api/subscriptions/update', { ...ctx, name: input.value.trim() }, input, 'subs_rename_failed');
}

async function toggleProfile(btn: HTMLElement): Promise<void> {
  const ctx = rowContext(btn);
  if (!ctx) return;
  const entry = cachedProviders.find((p) => p.provider === ctx.provider);
  const profile = entry?.profiles.find((p) => p.id === ctx.id);
  if (!profile) return;
  await postAndRefresh('/api/subscriptions/update', { ...ctx, enabled: !profile.enabled }, btn, 'subs_update_failed');
}

async function removeProfile(btn: HTMLElement): Promise<void> {
  const ctx = rowContext(btn);
  if (!ctx) return;
  // 既定は many-ai-cli の登録解除のみ。vendor CLI の認証ファイル削除は
  // 2 段目の確認を通ったときだけ行う（誤操作で再ログインを強いられないように）。
  const ok = await appConfirm({
    title: t('subs_remove_confirm_title'),
    message: t('subs_remove_confirm_message'),
    confirmText: t('subs_remove'),
    cancelText: t('cancel'),
    kind: 'warn',
  });
  if (!ok) return;
  const alsoDelete = await appConfirm({
    title: t('subs_remove_credentials_title'),
    message: t('subs_remove_credentials_message'),
    confirmText: t('subs_remove_credentials_confirm'),
    cancelText: t('subs_remove_credentials_keep'),
    kind: 'danger',
  });
  await postAndRefresh('/api/subscriptions/remove', { ...ctx, delete_credentials: alsoDelete }, btn, 'subs_remove_failed');
}

async function testProfile(btn: HTMLButtonElement): Promise<void> {
  const ctx = rowContext(btn);
  if (!ctx) return;
  btn.disabled = true;
  const row = btn.closest('.subs-row');
  const statusEl = row?.querySelector('.subs-status') as HTMLElement | null;
  if (statusEl) statusEl.textContent = t('subs_status_checking');
  try {
    const res = await subsApi('/api/subscriptions/test', jsonInit(ctx));
    const data = await res.json().catch(() => ({} as Record<string, unknown>));
    if (!res.ok) {
      const detail = typeof data.detail === 'string' ? data.detail : t('subs_test_failed');
      if (statusEl) statusEl.textContent = detail;
      setError(detail);
      return;
    }
    setError('');
    if (statusEl) {
      statusEl.textContent = data.logged_in
        ? (data.plan ? t('subs_status_signed_in_plan', { plan: String(data.plan) }) : t('subs_status_signed_in'))
        : t('subs_status_needs_login');
    }
    // plan が返った場合 Hub 側が config に書き戻すので一覧を取り直す。
    await loadSubscriptions(true);
    renderList();
  } catch (_) {
    if (statusEl) statusEl.textContent = t('subs_test_failed');
  } finally {
    btn.disabled = false;
  }
}

async function loginProfile(btn: HTMLButtonElement): Promise<void> {
  const ctx = rowContext(btn);
  if (!ctx) return;
  btn.disabled = true;
  try {
    const res = await subsApi('/api/subscriptions/login', jsonInit(ctx));
    if (!res.ok) {
      const detail = await res.json().catch(() => ({} as { detail?: string }));
      const message = (detail && typeof detail.detail === 'string' && detail.detail) ? detail.detail : t('subs_login_failed');
      setError(message);
      showToast(t('subs_login_failed'), btn);
      return;
    }
    setError('');
    showToast(t('subs_login_started'), btn);
  } catch (_) {
    showToast(t('subs_login_failed'), btn);
  } finally {
    btn.disabled = false;
  }
}

export function initSubscriptions(): void {
  const host = document.getElementById('subs-list');
  if (!host) return;

  host.addEventListener('click', (e) => {
    const target = e.target as HTMLElement | null;
    if (!target) return;
    e.stopPropagation();
    if (target.classList.contains('subs-add-btn')) { void addProfile(target); return; }
    if (target.classList.contains('subs-test')) { void testProfile(target as HTMLButtonElement); return; }
    if (target.classList.contains('subs-login')) { void loginProfile(target as HTMLButtonElement); return; }
    if (target.classList.contains('subs-toggle')) { void toggleProfile(target); return; }
    if (target.classList.contains('subs-remove')) { void removeProfile(target); return; }
  });

  host.addEventListener('change', (e) => {
    const target = e.target as HTMLElement | null;
    if (target?.classList.contains('subs-name')) void renameProfile(target as HTMLInputElement);
  });

  host.addEventListener('keydown', (e) => {
    const ev = e as KeyboardEvent;
    const target = ev.target as HTMLElement | null;
    if (ev.key !== 'Enter' || !target) return;
    if (target.classList.contains('subs-add-name')) {
      ev.preventDefault();
      const btn = target.parentElement?.querySelector('.subs-add-btn') as HTMLElement | null;
      if (btn) void addProfile(btn);
    } else if (target.classList.contains('subs-name')) {
      ev.preventDefault();
      (target as HTMLInputElement).blur();
    }
  });

  // 設定パネルを開いたときに取り直す。spawn パネル側も起動時に 1 回読む。
  document.getElementById('settings-btn')?.addEventListener('click', () => {
    void loadSubscriptions(true).then(renderList);
  });

  // 初回描画が i18n 辞書のロードより先に走ると、ラベルが翻訳キーのまま出る。
  // 辞書が届いた時点で 1 回描き直す。
  document.addEventListener('i18n-ready', () => renderList(), { once: true });
  void loadSubscriptions(true).then(renderList);
}
