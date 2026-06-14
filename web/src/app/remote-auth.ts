// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { token, showToast } from './util.js';
import { appConfirm } from './settings.js';

// ---- リモートアクセス保護（plan_hub-remote-auth.md C3）----
//   - 全アクセス失効ボタン（キルスイッチ / B）: token + cookie secret をローテーションし全端末を弾く。
//   - 任意リモート PIN（A）: 設定/変更/解除。remote 未認証なら PIN ログインモーダルを出す。
//   - ロックアウト中は Retry-After を表示する。
// loopback（このPC）からは PIN 不要・モーダルも出さない。

function isLoopbackHost(): boolean {
  const h = location.hostname;
  return h === '127.0.0.1' || h === 'localhost' || h === '::1' || h === '[::1]';
}

function authApi(path: string, init?: RequestInit): Promise<Response> {
  const sep = path.includes('?') ? '&' : '?';
  return fetch(`${path}${sep}token=${encodeURIComponent(token || '')}`, init);
}

const jsonInit = (bodyObj: unknown): RequestInit => ({
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(bodyObj),
});

// ---- B: 全アクセス失効 ----
async function revokeAll(btn: HTMLElement): Promise<void> {
  const ok = await appConfirm({
    title: t('revoke_all_confirm_title'),
    message: t('revoke_all_confirm_message'),
    confirmText: t('revoke_all_confirm_ok'),
    cancelText: t('cancel'),
    kind: 'danger',
  });
  if (!ok) return;
  try {
    const res = await authApi('/api/auth/revoke-all', { method: 'POST' });
    if (!res.ok) {
      showToast(t('revoke_all_failed'), btn);
      return;
    }
    const data = await res.json().catch(() => ({}));
    // このPC自身も新URLでの再ログインになる。新 token 入り URL へ誘導する。
    if (data && typeof data.hub_url === 'string' && data.hub_url) {
      location.href = data.hub_url;
    } else {
      location.reload();
    }
  } catch (_) {
    showToast(t('revoke_all_failed'), btn);
  }
}

// ---- A: 任意 PIN 設定/状態 ----
async function loadPINStatus(): Promise<void> {
  const statusEl = document.getElementById('remote-pin-status');
  const clearBtn = document.getElementById('remote-pin-clear-btn') as HTMLButtonElement | null;
  try {
    const res = await authApi('/api/auth/status');
    if (!res.ok) return;
    const data = await res.json();
    const enabled = !!data.pin_enabled;
    if (statusEl) statusEl.textContent = enabled ? t('settings_remote_pin_on') : t('settings_remote_pin_off');
    if (clearBtn) clearBtn.hidden = !enabled;
  } catch (_) {}
}

async function setPIN(btn: HTMLElement): Promise<void> {
  const input = document.getElementById('remote-pin-input') as HTMLInputElement | null;
  const pin = (input?.value || '').trim();
  if (!/^[0-9]{6,}$/.test(pin)) {
    showToast(t('settings_remote_pin_format'), btn);
    return;
  }
  try {
    const res = await authApi('/api/auth/set-pin', jsonInit({ pin }));
    if (!res.ok) {
      showToast(t('settings_remote_pin_set_failed'), btn);
      return;
    }
    if (input) input.value = '';
    showToast(t('settings_remote_pin_set_done'), btn);
    void loadPINStatus();
  } catch (_) {
    showToast(t('settings_remote_pin_set_failed'), btn);
  }
}

async function clearPIN(btn: HTMLElement): Promise<void> {
  const ok = await appConfirm({
    title: t('settings_remote_pin_clear_title'),
    message: t('settings_remote_pin_clear_msg'),
    confirmText: t('settings_remote_pin_clear'),
    cancelText: t('cancel'),
    kind: 'warn',
  });
  if (!ok) return;
  try {
    const res = await authApi('/api/auth/set-pin', jsonInit({ clear: true }));
    if (!res.ok) {
      showToast(t('settings_remote_pin_clear_failed'), btn);
      return;
    }
    showToast(t('settings_remote_pin_clear_done'), btn);
    void loadPINStatus();
  } catch (_) {
    showToast(t('settings_remote_pin_clear_failed'), btn);
  }
}

// ---- A: PIN ログインモーダル ----
function showPINLogin(): void {
  const overlay = document.getElementById('pin-login-overlay');
  if (!overlay) return;
  overlay.hidden = false;
  const input = document.getElementById('pin-login-input') as HTMLInputElement | null;
  input?.focus();
}

async function submitPINLogin(): Promise<void> {
  const input = document.getElementById('pin-login-input') as HTMLInputElement | null;
  const errEl = document.getElementById('pin-login-error');
  const submitBtn = document.getElementById('pin-login-submit') as HTMLButtonElement | null;
  const pin = (input?.value || '').trim();
  if (!pin) return;
  if (errEl) errEl.hidden = true;
  if (submitBtn) submitBtn.disabled = true;
  try {
    const res = await authApi('/api/auth/login', jsonInit({ pin }));
    if (res.ok) {
      // 認証成功 → cookie が付いた状態でリロードし WS 等を張り直す。
      location.reload();
      return;
    }
    let msg = t('pin_login_wrong');
    if (res.status === 429) {
      const ra = res.headers.get('Retry-After');
      msg = ra ? t('pin_login_locked').replace('{sec}', ra) : t('pin_login_locked_generic');
    }
    if (errEl) {
      errEl.textContent = msg;
      errEl.hidden = false;
    }
    if (input) input.value = '';
  } catch (_) {
    if (errEl) {
      errEl.textContent = t('pin_login_error');
      errEl.hidden = false;
    }
  } finally {
    if (submitBtn) submitBtn.disabled = false;
    input?.focus();
  }
}

// 起動時: remote かつ PIN 有効で未認証なら PIN ログインモーダルを出す。
async function checkRemotePINGate(): Promise<void> {
  if (isLoopbackHost()) return;
  try {
    const res = await authApi('/api/auth/status');
    if (!res.ok) return;
    const data = await res.json();
    if (data.pin_enabled && data.remote && !data.authed) {
      showPINLogin();
    }
  } catch (_) {}
}

export function initRemoteAuth(): void {
  const revokeBtn = document.getElementById('revoke-all-btn');
  revokeBtn?.addEventListener('click', (e) => {
    e.stopPropagation();
    void revokeAll(revokeBtn);
  });

  const setBtn = document.getElementById('remote-pin-set-btn');
  setBtn?.addEventListener('click', (e) => {
    e.stopPropagation();
    void setPIN(setBtn);
  });

  const clearBtn = document.getElementById('remote-pin-clear-btn');
  clearBtn?.addEventListener('click', (e) => {
    e.stopPropagation();
    void clearPIN(clearBtn);
  });

  const setInput = document.getElementById('remote-pin-input');
  setInput?.addEventListener('keydown', (e) => {
    if ((e as KeyboardEvent).key === 'Enter') {
      e.preventDefault();
      if (setBtn) void setPIN(setBtn);
    }
  });

  const submitBtn = document.getElementById('pin-login-submit');
  submitBtn?.addEventListener('click', () => void submitPINLogin());
  const loginInput = document.getElementById('pin-login-input');
  loginInput?.addEventListener('keydown', (e) => {
    if ((e as KeyboardEvent).key === 'Enter') {
      e.preventDefault();
      void submitPINLogin();
    }
  });

  // 設定パネルを開いたら PIN 状態を更新。
  document.getElementById('settings-btn')?.addEventListener('click', () => void loadPINStatus());

  void checkRemotePINGate();
}
