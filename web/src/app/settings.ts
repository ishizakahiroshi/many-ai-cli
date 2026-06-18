// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { escapeHtml, showToast, ti18n, token } from './util.js';
import { DEFAULT_USAGE_LINKS, DEFAULT_VOICE_GRACE_SEC, FONTSIZE_MAP, STORAGE_DESKTOP_NOTIFY_ENABLED_KEY, STORAGE_DISPLAY_LOCKED_MODE_KEY, STORAGE_FONTSIZE_KEY, STORAGE_LANG_KEY, STORAGE_MOBILE_INPUT_TOOLS_KEY, STORAGE_PC_INPUT_TOOLS_KEY, STORAGE_NOTIFY_SOUND_CUSTOM_KEY, STORAGE_NOTIFY_SOUND_ENABLED_KEY, STORAGE_NOTIFY_SOUND_TYPE_KEY, STORAGE_PUSH_NOTIFY_ENABLED_KEY, STORAGE_QUICK_CMD_1_KEY, STORAGE_QUICK_CMD_2_KEY, STORAGE_QUICK_CMD_3_KEY, STORAGE_QUICK_CMD_4_KEY, STORAGE_QUICK_CMD_5_KEY, STORAGE_QUICK_CMD_1_SHOW_KEY, STORAGE_QUICK_CMD_2_SHOW_KEY, STORAGE_QUICK_CMD_3_SHOW_KEY, STORAGE_QUICK_CMD_4_SHOW_KEY, STORAGE_QUICK_CMD_5_SHOW_KEY, STORAGE_THEME_KEY, STORAGE_TRIGGER_ENABLED_KEY, STORAGE_TRIGGER_PHRASE_KEY, STORAGE_USAGE_LINK_CLAUDE_KEY, STORAGE_USAGE_LINK_CODEX_KEY, STORAGE_USAGE_LINK_COPILOT_KEY, STORAGE_USAGE_LINK_CURSOR_AGENT_KEY, STORAGE_USAGE_LINK_OLLAMA_KEY, STORAGE_USAGE_LINK_OPENCODE_KEY, STORAGE_VOICE_GRACE_KEY, STORAGE_VOICE_WHISPER_AUTO_STOP_KEY,  STORAGE_VOICE_WHISPER_AUTO_SUBMIT_KEY, STORAGE_WAKE_WORD_ENABLED_KEY, STORAGE_WAKE_WORD_PHRASE_KEY, _putUserPrefsNow, _setNestedValue, getDefaultTriggerPhrase, getDefaultWakeWordPhrase, getVoiceEngine, setUserPref, setVoiceEngine } from './user-prefs.js';
import { activeSessionId, deriveProjectKeyFromCwd, maybeAutoSwitchToNextApproval, sessions, terminals } from './state.js';
import { _userAvatarUrl, _userDisplayName, inputEl, set__userAvatarUrl, set__userDisplayName } from '../app.js';
import { activateSession, openDetachedGridForSessions, providerDisplayName, providerIconHtml, render, renderSessionList, safeClassToken, sessionProjectKey, setFaviconEnvBadge, stateLabel } from './session-list.js';
import { pathPopupEl } from './path-links.js';
import { TERMINAL_SCROLLBACK_LINES, attachTerminal, fitTerminalPreservingBottom, refitActiveTerminalAfterLayout, sendResize } from './terminal.js';
import { providerApprovalTriggers } from './approval.js';
import { MULTI_SCROLLBACK, getMessages } from './chat-history.js';
import { FilesTabManager } from './files-view.js';
import { fetchPushStatus, getPushSubscription, isLikelyIOSBrowserTabWithoutStandalone, pushNotificationsSupported, subscribeWebPush, unsubscribeWebPush } from './pwa.js';
import { setStatusbarEnabled, isStatusbarEnabled } from './token-statusbar.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- Hub 承認ボタン機能 オプトイントースト ----
export let _approvalAlertChecked = false;

export async function checkApprovalOnStartup() {
  if (_approvalAlertChecked) return;
  _approvalAlertChecked = true;
  try {
    const res = await fetch(`/api/approval/status?token=${token}`);
    if (!res.ok) return;
    const data = await res.json();
    if (!data.enabled && !data.first_launch_shown) {
      showApprovalToast();
    }
  } catch (_) {}
}

export function showApprovalToast() {
  if (document.getElementById('approval-toast')) return;
  const el = document.createElement('div');
  el.id = 'approval-toast';

  const dialog = document.createElement('div');
  dialog.className = 'approval-toast-dialog';

  const msg = document.createElement('p');
  msg.className = 'approval-toast-msg';
  msg.textContent = t('approval_toast_msg');

  const actions = document.createElement('div');
  actions.className = 'approval-toast-actions';

  const yesBtn = document.createElement('button');
  yesBtn.className = 'approval-toast-btn';
  yesBtn.textContent = t('approval_toast_yes');
  yesBtn.addEventListener('click', async (e) => {
    e.stopPropagation();
    yesBtn.disabled = true;
    yesBtn.textContent = t('approval_toast_enabling');
    try {
      await fetch(`/api/approval/enable?token=${token}`, { method: 'POST' });
    } catch (_) {}
    el.remove();
    updateApprovalToggle(true);
  });

  const noBtn = document.createElement('button');
  noBtn.className = 'approval-toast-btn-dismiss';
  noBtn.textContent = t('approval_toast_later');
  noBtn.addEventListener('click', async (e) => {
    e.stopPropagation();
    try {
      await fetch(`/api/approval/dismiss?token=${token}`, { method: 'POST' });
    } catch (_) {}
    el.remove();
  });

  actions.appendChild(yesBtn);
  actions.appendChild(noBtn);
  dialog.appendChild(msg);
  dialog.appendChild(actions);
  el.appendChild(dialog);
  document.body.appendChild(el);
  yesBtn.focus();
}

export function updateApprovalToggle(enabled) {
  const toggle = document.getElementById('approval-toggle-input');
  if (toggle) toggle.checked = enabled;
}

export async function loadApprovalSettings() {
  try {
    const res = await fetch(`/api/approval/status?token=${token}`);
    if (!res.ok) return;
    const data = await res.json();
    updateApprovalToggle(data.enabled);
  } catch (_) {}
}

// ---- トークン/コスト ステータスバー トグル ----

export function updateTokenStatusbarToggle(enabled: boolean): void {
  const toggle = document.getElementById('token-statusbar-toggle-input') as HTMLInputElement | null;
  if (toggle) toggle.checked = enabled;
  setStatusbarEnabled(enabled);
}

// 設定パネルのトグルが DOM に存在する時点（パネル open 時）に一度だけ配線する。
let _tokenStatusbarToggleAttached = false;
export function attachTokenStatusbarToggle(): void {
  if (_tokenStatusbarToggleAttached) return;
  const toggle = document.getElementById('token-statusbar-toggle-input') as HTMLInputElement | null;
  if (!toggle) return;
  _tokenStatusbarToggleAttached = true;
  // 現在値を反映
  toggle.checked = isStatusbarEnabled();
  toggle.addEventListener('change', async () => {
    const enabled = toggle.checked;
    setStatusbarEnabled(enabled);
    // /api/user-prefs に保存
    try {
      await fetch(`/api/user-prefs?token=${encodeURIComponent(token || '')}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ token_statusbar: { enabled } }),
      });
    } catch (_) {}
  });
}

// ---- タスク完了サマリー通知トグル ----

export function updateDoneSummaryNotifyToggle(enabled: boolean): void {
  const toggle = document.getElementById('done-summary-notify-toggle') as HTMLInputElement | null;
  if (toggle) toggle.checked = enabled;
}

let _doneSummaryNotifyToggleAttached = false;
export function attachDoneSummaryNotifyToggle(): void {
  if (_doneSummaryNotifyToggleAttached) return;
  const toggle = document.getElementById('done-summary-notify-toggle') as HTMLInputElement | null;
  if (!toggle) return;
  _doneSummaryNotifyToggleAttached = true;
  // 現在値をサーバから読んで反映
  (async () => {
    try {
      const res = await fetch(`/api/user-prefs?token=${encodeURIComponent(token || '')}`);
      if (res.ok) {
        const data = await res.json();
        toggle.checked = !!(data?.done_summary_notify?.enabled);
      }
    } catch (_) {}
  })();
  toggle.addEventListener('change', async () => {
    const enabled = toggle.checked;
    try {
      await fetch(`/api/user-prefs?token=${encodeURIComponent(token || '')}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ done_summary_notify: { enabled } }),
      });
    } catch (_) {}
  });
}

// ---- グローバルカスタムツールチップ（遅延なし） ----
(function () {
  const tip = document.createElement('div');
  tip.id = 'hub-tooltip';
  document.body.appendChild(tip);

  function pos(x, y) {
    const GAP = 13;
    tip.style.left = (x + GAP) + 'px';
    tip.style.top  = (y + GAP) + 'px';
    const r = tip.getBoundingClientRect();
    if (r.right  > window.innerWidth  - 6) tip.style.left = (x - r.width  - GAP) + 'px';
    if (r.bottom > window.innerHeight - 6) tip.style.top  = (y - r.height - GAP) + 'px';
  }

  let cur = null;
  document.addEventListener('mouseover', e => {
    const t = e.target.closest('[data-tooltip]');
    if (!t || !t.dataset.tooltip) { tip.style.display = 'none'; cur = null; return; }
    cur = t;
    tip.textContent = t.dataset.tooltip;
    tip.style.display = 'block';
    pos(e.clientX, e.clientY);
  });
  document.addEventListener('mousemove', e => {
    if (tip.style.display === 'block') pos(e.clientX, e.clientY);
  });
  document.addEventListener('mouseout', e => {
    if (cur && !cur.contains(e.relatedTarget)) { tip.style.display = 'none'; cur = null; }
  });
  document.addEventListener('click',  () => { tip.style.display = 'none'; cur = null; });
  document.addEventListener('scroll', () => { tip.style.display = 'none'; cur = null; }, true);
})();

// ---- 通知音 ----
export let _audioCtx = null;
export function _getAudioCtx() {
  if (!_audioCtx) _audioCtx = new (window.AudioContext || window.webkitAudioContext)();
  return _audioCtx;
}

export function playNotificationSound() {
  if (localStorage.getItem(STORAGE_NOTIFY_SOUND_ENABLED_KEY) !== '1') return;
  const type = localStorage.getItem(STORAGE_NOTIFY_SOUND_TYPE_KEY) || 'default';
  if (type === 'custom') {
    const tk = token;
    const customUrl = `/api/user-prefs/notify-sound-custom?token=${encodeURIComponent(tk || '')}`;
    if (customUrl) {
      try { new Audio(customUrl).play().catch(() => {}); return; } catch (_) {}
    }
  }
  try {
    const ctx = _getAudioCtx();
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.connect(gain);
    gain.connect(ctx.destination);
    osc.type = 'sine';
    osc.frequency.setValueAtTime(880, ctx.currentTime);
    gain.gain.setValueAtTime(0.3, ctx.currentTime);
    gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + 0.3);
    osc.start(ctx.currentTime);
    osc.stop(ctx.currentTime + 0.3);
  } catch (_) {}
}

export function desktopNotificationsEnabled() {
  return localStorage.getItem(STORAGE_DESKTOP_NOTIFY_ENABLED_KEY) === '1';
}

export function desktopNotificationPermission() {
  if (!('Notification' in window)) return 'unsupported';
  return Notification.permission || 'default';
}

export async function requestDesktopNotificationPermission() {
  if (!('Notification' in window)) return 'unsupported';
  try {
    return await Notification.requestPermission();
  } catch (_) {
    return desktopNotificationPermission();
  }
}

export function showDesktopApprovalNotification(sessionId) {
  if (!desktopNotificationsEnabled()) return;
  if (!('Notification' in window) || Notification.permission !== 'granted') return;
  const sess = sessions.get(sessionId);
  if (!sess) return;
  const isCurrentVisible =
    document.visibilityState === 'visible' &&
    sessionId === activeSessionId &&
    !document.hidden;
  if (isCurrentVisible) return;
  const provider = providerDisplayName(sess.provider) || sess.provider || '';
  const title = sess.label ? `${provider} #${sessionId} [${sess.label}]` : `${provider} #${sessionId}`;
  const bodySource = sess.last_message || sess.first_message || sess.cwd || '';
  const body = bodySource.replace(/\s+/g, ' ').trim().slice(0, 160) || t('desktop_notification_body');
  try {
    const n = new Notification(title, {
      body,
      tag: `many-ai-cli-approval-${sessionId}`,
      icon: '/icon.svg',
      badge: '/icon.svg',
      requireInteraction: false,
    });
    n.onclick = () => {
      window.focus();
      activateSession(sessionId);
      n.close();
    };
  } catch (_) {}
}

export function getActiveTriggerPhrase() {
  if (localStorage.getItem(STORAGE_TRIGGER_ENABLED_KEY) !== '1') return '';
  return (localStorage.getItem(STORAGE_TRIGGER_PHRASE_KEY) ?? getDefaultTriggerPhrase()).trim();
}

export function normalizeTriggerMatchText(text) {
  return String(text || '')
    .normalize('NFKC')
    .toLowerCase()
    .replace(/[\s\u3000]+/g, '')
    .replace(/[。．.!！?？、,，]+$/g, '');
}

export function textEndsWithTriggerPhrase(text, triggerPhrase) {
  const tp = normalizeTriggerMatchText(triggerPhrase);
  if (!tp) return false;
  return normalizeTriggerMatchText(text).endsWith(tp);
}

export function stripTrailingTriggerPhrase(text, triggerPhrase) {
  const original = String(text || '');
  const tp = normalizeTriggerMatchText(triggerPhrase);
  if (!tp) return original;
  if (!normalizeTriggerMatchText(original).endsWith(tp)) return original;
  for (let i = 0; i <= original.length; i++) {
    if (normalizeTriggerMatchText(original.slice(i)) === tp) {
      return original.slice(0, i).replace(/[\s\u3000]+$/g, '');
    }
  }
  return original;
}

export function normalizeHttpUrl(value, fallback = '') {
  const raw = String(value || '').trim();
  if (!raw) return fallback;
  try {
    const url = new URL(raw);
    if (url.protocol === 'https:' || url.protocol === 'http:') return url.href;
  } catch (_) {}
  return fallback;
}

export function getUsageLinkUrl(provider) {
  const keyMap = {
    claude:   STORAGE_USAGE_LINK_CLAUDE_KEY,
    codex:    STORAGE_USAGE_LINK_CODEX_KEY,
    copilot:  STORAGE_USAGE_LINK_COPILOT_KEY,
    'cursor-agent': STORAGE_USAGE_LINK_CURSOR_AGENT_KEY,
    ollama:   STORAGE_USAGE_LINK_OLLAMA_KEY,
    opencode: STORAGE_USAGE_LINK_OPENCODE_KEY,
  };
  const key = keyMap[provider];
  if (!key) return DEFAULT_USAGE_LINKS[provider] || '#';
  return normalizeHttpUrl(localStorage.getItem(key), DEFAULT_USAGE_LINKS[provider] || '') || '#';
}

export function applyUsageLinks() {
  for (const p of ['claude', 'codex', 'copilot', 'cursor-agent', 'ollama', 'opencode']) {
    const el = document.getElementById(`usage-link-${p}`);
    if (el) el.href = getUsageLinkUrl(p);
  }
}

export function loadUsageLinkSettings() {
  const keyMap = {
    claude:   STORAGE_USAGE_LINK_CLAUDE_KEY,
    codex:    STORAGE_USAGE_LINK_CODEX_KEY,
    copilot:  STORAGE_USAGE_LINK_COPILOT_KEY,
    'cursor-agent': STORAGE_USAGE_LINK_CURSOR_AGENT_KEY,
    ollama:   STORAGE_USAGE_LINK_OLLAMA_KEY,
    opencode: STORAGE_USAGE_LINK_OPENCODE_KEY,
  };
  for (const [p, k] of Object.entries(keyMap)) {
    const el = document.getElementById(`usage-link-${p}-url`);
    if (el) el.value = localStorage.getItem(k) || '';
  }
  applyUsageLinks();
}

export function saveUsageLinkSettings() {
  const pairs = [
    ['claude',   'usage_links.claude',   STORAGE_USAGE_LINK_CLAUDE_KEY],
    ['codex',    'usage_links.codex',    STORAGE_USAGE_LINK_CODEX_KEY],
    ['copilot',  'usage_links.copilot',  STORAGE_USAGE_LINK_COPILOT_KEY],
    ['cursor-agent', 'usage_links.cursor-agent', STORAGE_USAGE_LINK_CURSOR_AGENT_KEY],
    ['ollama',   'usage_links.ollama',   STORAGE_USAGE_LINK_OLLAMA_KEY],
    ['opencode', 'usage_links.opencode', STORAGE_USAGE_LINK_OPENCODE_KEY],
  ];
  for (const [p, prefPath, key] of pairs) {
    const input = document.getElementById(`usage-link-${p}-url`);
    if (!input) continue;
    const raw = input.value.trim();
    const normalized = normalizeHttpUrl(raw, '');
    if (normalized) {
      setUserPref(prefPath, normalized);
      input.value = normalized;
    } else {
      try { localStorage.removeItem(key); } catch (_) {}
      setUserPref(prefPath, '');
      input.value = '';
    }
  }
  applyUsageLinks();
}

export function initUsageDropdown() {
  const btn      = document.getElementById('usage-menu-btn');
  const dropdown = document.getElementById('usage-dropdown');
  if (!btn || !dropdown) return;

  // header に backdrop-filter があり新しい stacking context が作られるため、
  // dropdown が body 配下にないと z-index が効かず本体側に隠れる。body 直下へ移す。
  if (dropdown.parentElement !== document.body) {
    document.body.appendChild(dropdown);
  }

  const positionDropdown = () => {
    const rect = btn.getBoundingClientRect();
    const margin = 6;
    dropdown.style.top = `${Math.min(rect.bottom + 4, window.innerHeight - margin)}px`;
    dropdown.style.right = `${Math.max(margin, window.innerWidth - rect.right)}px`;
  };

  const closeDropdown = () => {
    dropdown.hidden = true;
    btn.setAttribute('aria-expanded', 'false');
  };

  btn.addEventListener('click', (e) => {
    e.stopPropagation();
    const isOpen = !dropdown.hidden;
    if (isOpen) {
      closeDropdown();
      return;
    }
    positionDropdown();
    dropdown.hidden = false;
    btn.setAttribute('aria-expanded', 'true');
  });

  // xterm 等で stopPropagation されると bubble phase の document リスナーまで届かないため、
   // capture phase の mousedown / touchstart で拾って確実に閉じる。
   const onOutsidePointer = (e) => {
     if (dropdown.hidden) return;
     if (btn.contains(e.target) || dropdown.contains(e.target)) return;
     closeDropdown();
   };
   document.addEventListener('mousedown',  onOutsidePointer, true);
   document.addEventListener('touchstart', onOutsidePointer, true);

  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !dropdown.hidden) {
      closeDropdown();
    }
  });

  window.addEventListener('resize', () => {
    if (!dropdown.hidden) positionDropdown();
  });
}

export function appConfirm({ title, message, confirmText, cancelText, kind = 'default' }): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    const overlay = document.getElementById('model-picker-overlay');
    if (!overlay) { resolve(window.confirm(message)); return; }

    const close = (value) => {
      overlay.removeEventListener('click', onOverlayClick);
      document.removeEventListener('keydown', onKeyDown);
      overlay.hidden = true;
      overlay.innerHTML = '';
      resolve(value);
    };
    const onOverlayClick = (e) => {
      // この確認ダイアログは設定パネルの外（model-picker-overlay）に出るため、
      // ここで止めないとクリックが document まで伝播し、設定パネルの
      // 「パネル外クリックで閉じる」ハンドラに拾われてパネルごと閉じてしまう。
      e.stopPropagation();
      if (e.target === overlay) close(false);
    };
    const onKeyDown = (e) => {
      if (e.key === 'Escape') close(false);
      if (e.key === 'Enter') close(true);
    };

    overlay.innerHTML = '';
    overlay.hidden = false;

    const dialog = document.createElement('div');
    dialog.className = `confirm-dialog confirm-dialog--${kind}`;
    dialog.innerHTML = `
      <div class="confirm-icon" aria-hidden="true">!</div>
      <div class="confirm-body">
        <div class="confirm-title">${escapeHtml(title)}</div>
        <div class="confirm-message">${escapeHtml(message)}</div>
      </div>
      <div class="confirm-actions">
        <button class="confirm-btn" id="app-confirm-cancel">${escapeHtml(cancelText || t('confirm_cancel'))}</button>
        <button class="confirm-btn primary" id="app-confirm-ok">${escapeHtml(confirmText || t('confirm_ok'))}</button>
      </div>
    `;
    overlay.appendChild(dialog);

    // ボタン経路では close() が onOverlayClick を先に解除するため、overlay 側の
    // stopPropagation では止められない。各ボタンで直接伝播を止めて、document の
    // 「パネル外クリックで閉じる」ハンドラに届かないようにする。
    document.getElementById('app-confirm-cancel').addEventListener('click', (e) => { e.stopPropagation(); close(false); });
    document.getElementById('app-confirm-ok').addEventListener('click', (e) => { e.stopPropagation(); close(true); });
    overlay.addEventListener('click', onOverlayClick);
    document.addEventListener('keydown', onKeyDown);
    document.getElementById('app-confirm-ok').focus();
  });
}

// 新バージョン移行時の「データリセット & ログ設定」案内（チェックボックス複数選択 + 実行）
// 戻り値: { deleteLogs, deleteAttachments, enableLogging } / null（閉じる・Escape）
export function appLegacyResetNotice(): Promise<{ deleteLogs: boolean; deleteAttachments: boolean; enableLogging: boolean } | null> {
  return new Promise((resolve) => {
    const overlay = document.getElementById('model-picker-overlay');
    if (!overlay) { resolve(null); return; }

    // このモーダルは閉じる手段を持たない（✕・キャンセル・Escape・オーバーレイクリック
    // すべて不可）。必ずチェック内容を確認して「実行」を押させてから先へ進ませる。
    const blockEsc = (e) => { if (e.key === 'Escape') { e.stopPropagation(); e.preventDefault(); } };
    const close = (value) => {
      document.removeEventListener('keydown', blockEsc, true);
      overlay.hidden = true;
      overlay.innerHTML = '';
      resolve(value);
    };

    overlay.innerHTML = '';
    overlay.hidden = false;

    const rowStyle = 'display:flex;align-items:center;gap:8px;margin-top:10px;text-align:left;cursor:pointer';
    const dialog = document.createElement('div');
    dialog.className = 'confirm-dialog confirm-dialog--danger';
    dialog.innerHTML = `
      <div class="confirm-icon" aria-hidden="true">!</div>
      <div class="confirm-body">
        <div class="confirm-title">${escapeHtml(t('legacy_logs_notice_title'))}</div>
        <div class="confirm-message">${escapeHtml(t('legacy_logs_notice_message'))}</div>
        <label style="${rowStyle}"><input type="checkbox" id="legacy-del-logs" checked> <span>${escapeHtml(t('legacy_logs_notice_opt_logs'))}</span></label>
        <label style="${rowStyle}"><input type="checkbox" id="legacy-del-attach" checked> <span>${escapeHtml(t('legacy_logs_notice_opt_attach'))}</span></label>
        <label style="${rowStyle}"><input type="checkbox" id="legacy-enable-logging"> <span>${escapeHtml(t('legacy_logs_notice_opt_enable'))}</span></label>
      </div>
      <div class="confirm-actions">
        <button class="confirm-btn primary" id="legacy-exec">${escapeHtml(t('legacy_logs_notice_exec'))}</button>
      </div>
    `;
    overlay.appendChild(dialog);

    document.getElementById('legacy-exec').addEventListener('click', () => close({
      deleteLogs:        (document.getElementById('legacy-del-logs') as HTMLInputElement).checked,
      deleteAttachments: (document.getElementById('legacy-del-attach') as HTMLInputElement).checked,
      enableLogging:     (document.getElementById('legacy-enable-logging') as HTMLInputElement).checked,
    }));
    // Escape を捕捉段階で握りつぶし、他のハンドラにも閉じさせない。
    document.addEventListener('keydown', blockEsc, true);
    document.getElementById('legacy-exec').focus();
  });
}

// Ollama エンコーディング警告ダイアログ（3 択）
// 戻り値: 'utf8' | 'continue' | null（キャンセル）
export function appConfirmOllamaEncoding() {
  return new Promise((resolve) => {
    const overlay = document.getElementById('model-picker-overlay');
    if (!overlay) {
      // フォールバック: confirm で代替（プレーン環境）
      if (window.confirm(t('ollama_encoding_warn_message'))) {
        resolve('utf8');
      } else {
        resolve('continue');
      }
      return;
    }

    const close = (value) => {
      overlay.removeEventListener('click', onOverlayClick);
      document.removeEventListener('keydown', onKeyDown);
      overlay.hidden = true;
      overlay.innerHTML = '';
      resolve(value);
    };
    const onOverlayClick = (e) => {
      if (e.target === overlay) close(null);
    };
    const onKeyDown = (e) => {
      if (e.key === 'Escape') close(null);
    };

    overlay.innerHTML = '';
    overlay.hidden = false;

    const dialog = document.createElement('div');
    dialog.className = 'confirm-dialog confirm-dialog--warn';
    dialog.innerHTML = `
      <div class="confirm-icon" aria-hidden="true">⚠</div>
      <div class="confirm-body">
        <div class="confirm-title">${escapeHtml(t('ollama_encoding_warn_title'))}</div>
        <div class="confirm-message">${escapeHtml(t('ollama_encoding_warn_message'))}</div>
      </div>
      <div class="confirm-actions ollama-encoding-actions">
        <button class="confirm-btn" id="ollama-enc-continue">${escapeHtml(t('ollama_encoding_continue'))}</button>
        <button class="confirm-btn primary" id="ollama-enc-utf8">${escapeHtml(t('ollama_encoding_utf8_session'))}</button>
      </div>
    `;
    overlay.appendChild(dialog);

    document.getElementById('ollama-enc-continue').addEventListener('click', () => close('continue'));
    document.getElementById('ollama-enc-utf8').addEventListener('click',    () => close('utf8'));
    overlay.addEventListener('click', onOverlayClick);
    document.addEventListener('keydown', onKeyDown);
    document.getElementById('ollama-enc-utf8').focus();
  });
}

// Hub 停止確認ダイアログ。
// 戻り値: null（キャンセル）/ { action: 'shutdown'|'sessions', stopExpose: boolean }。
// opts.exposeActive=true（tailscale serve が ready）のとき「外部公開も停止する」
// チェックボックス（既定 ON）を表示し、その状態を stopExpose で返す。
// serve は --bg で tailscaled 側に残るため、Hub 停止だけでは幽霊公開状態になる（既定で一緒に停止）。
export function appConfirmShutdown(opts) {
  const exposeActive = !!(opts && opts.exposeActive);
  return new Promise<{ action: 'shutdown' | 'sessions'; stopExpose: boolean } | null>((resolve) => {
    const overlay = document.getElementById('model-picker-overlay');
    if (!overlay) {
      const ok = window.confirm(t('shutdown_confirm'));
      resolve(ok ? { action: 'shutdown', stopExpose: exposeActive } : null);
      return;
    }

    const readStopExpose = () => {
      if (!exposeActive) return false;
      const cb = document.getElementById('app-confirm-stop-expose');
      return cb ? !!cb.checked : true;
    };
    const close = (action) => {
      const stopExpose = readStopExpose();
      overlay.removeEventListener('click', onOverlayClick);
      document.removeEventListener('keydown', onKeyDown);
      overlay.hidden = true;
      overlay.innerHTML = '';
      resolve(action ? { action, stopExpose } : null);
    };
    const onOverlayClick = (e) => { if (e.target === overlay) close(null); };
    const onKeyDown = (e) => {
      if (e.key === 'Escape') close(null);
      if (e.key === 'Enter') close('shutdown');
    };

    overlay.innerHTML = '';
    overlay.hidden = false;

    const exposeRow = exposeActive ? `
        <label class="confirm-check">
          <input type="checkbox" id="app-confirm-stop-expose" checked>
          <span>${escapeHtml(t('shutdown_stop_expose'))}</span>
        </label>` : '';

    const dialog = document.createElement('div');
    dialog.className = 'confirm-dialog confirm-dialog--danger';
    dialog.innerHTML = `
      <div class="confirm-icon" aria-hidden="true">!</div>
      <div class="confirm-body">
        <div class="confirm-title">${escapeHtml(t('shutdown_confirm_title'))}</div>
        <div class="confirm-message">${escapeHtml(t('shutdown_confirm'))}</div>${exposeRow}
      </div>
      <div class="confirm-actions">
        <button class="confirm-btn" id="app-confirm-cancel">${escapeHtml(t('spawn_cancel'))}</button>
        <button class="confirm-btn primary" id="app-confirm-sessions">${escapeHtml(t('shutdown_sessions_only'))}</button>
        <button class="confirm-btn primary" id="app-confirm-ok">${escapeHtml(t('shutdown_confirm_run'))}</button>
      </div>
    `;
    overlay.appendChild(dialog);

    document.getElementById('app-confirm-cancel').addEventListener('click', () => close(null));
    document.getElementById('app-confirm-sessions').addEventListener('click', () => close('sessions'));
    document.getElementById('app-confirm-ok').addEventListener('click', () => close('shutdown'));
    overlay.addEventListener('click', onOverlayClick);
    document.addEventListener('keydown', onKeyDown);
    document.getElementById('app-confirm-ok').focus();
  });
}

// VT スクレイプ用の堅牢な ANSI/制御文字除去。旧版は CSI を終端 [A-Za-z] のみ・
// OSC 本体や生制御文字（BEL 等）を残し、承認/質問ポップアップにゴミ文字が混入していた。
// chat-history.ts の stripAnsiBasic と同等のカバレッジ（CSI中間バイト・OSC・DCS/PM/APC・
// charset 指定・制御文字）に統一する。
export function stripAnsi(str) {
  if (!str) return '';
  return String(str)
    // CSI: ESC [ パラメータ 中間バイト 終端（@-~ の任意終端まで）
    .replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, '')
    // OSC: ESC ] ... BEL もしくは ESC \
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, '')
    // DCS / PM / APC / SOS: ESC P|X|^|_ ... ESC \
    .replace(/\x1b[PX^_][^\x1b]*\x1b\\/g, '')
    // charset 指定: ESC ( | ) + 1 文字
    .replace(/\x1b[()][0-9A-Za-z]/g, '')
    // ESC = / ESC >
    .replace(/\x1b[=>]/g, '')
    // 取りこぼした単独 ESC + 1 文字
    .replace(/\x1b[^[]/g, '')
    // 生の制御文字（BEL 等。改行・タブは残す）
    .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, '');
}

// 初回（未設定）は空欄にして入力欄のプレースホルダーで自由入力を誘導する。
// 空のスロットはボタン自体を非表示にする（送るものが無いため）。
// クイックコマンドのスロット数（1..QUICK_CMD_SLOTS）。
export const QUICK_CMD_SLOTS = 5;
// スロット番号 → action-bar 上のボタン要素 id。1,2 は歴史的経緯で別名のまま。
const QUICK_CMD_BTN_IDS = {
  1: 'quick-clear-btn',
  2: 'quick-model-btn',
  3: 'quick-cmd-btn-3',
  4: 'quick-cmd-btn-4',
  5: 'quick-cmd-btn-5',
};
const QUICK_CMD_KEYS = {
  1: STORAGE_QUICK_CMD_1_KEY,
  2: STORAGE_QUICK_CMD_2_KEY,
  3: STORAGE_QUICK_CMD_3_KEY,
  4: STORAGE_QUICK_CMD_4_KEY,
  5: STORAGE_QUICK_CMD_5_KEY,
};
const QUICK_CMD_SHOW_KEYS = {
  1: STORAGE_QUICK_CMD_1_SHOW_KEY,
  2: STORAGE_QUICK_CMD_2_SHOW_KEY,
  3: STORAGE_QUICK_CMD_3_SHOW_KEY,
  4: STORAGE_QUICK_CMD_4_SHOW_KEY,
  5: STORAGE_QUICK_CMD_5_SHOW_KEY,
};
// 未設定(null)スロットの初期コマンド。スロット1だけ見本として 1 個入れておき、
// 「便利そう」と思った人が設定で増やせるよう促す。ユーザーが空にしたら空のまま。
const QUICK_CMD_DEFAULTS = {
  1: '/clear',
};
// スロット番号からボタン要素 id を引く（app.ts のイベント登録でも使用）。
export function quickCommandButtonId(slot) {
  return QUICK_CMD_BTN_IDS[slot] || null;
}

// スロットの初期コマンド（リセット時の復元に使用）。無ければ空。
export function quickCommandDefault(slot) {
  return QUICK_CMD_DEFAULTS[slot] || '';
}
// datalist の入力候補（自由入力可。スラッシュコマンドだけでなく任意テキストも登録できる）
export const QUICK_COMMAND_PRESETS = [
  '/clear', '/model', '/help', '/status', '/usage', '/review', '/compact', '/config',
];
// PTY へ送る文字列の最大長。改行は \r 連結時の複数行送信を防ぐため除去する。
export const QUICK_CMD_MAXLEN = 200;
// action-bar 上のボタンに表示する先頭文字数（残りは … で省略、全文は tooltip 表示）。
export const QUICK_CMD_LABEL_LEN = 5;

// 自由入力テキストをクイックコマンドとして受け入れ可能な形に整える。
// 旧実装の whitelist 制限は撤廃（任意テキストを登録・送信できる）。
export function sanitizeQuickCommand(cmd, fallback) {
  if (typeof cmd !== 'string') return fallback;
  const cleaned = cmd
    .replace(/[\r\n]+/g, ' ')        // 改行は空白化（複数行送信化を防ぐ）
    .replace(/[\x00-\x1f\x7f]/g, '') // 制御文字を除去
    .trim();
  if (!cleaned) return fallback;
  return cleaned.slice(0, QUICK_CMD_MAXLEN);
}

// ボタン表示用の短縮ラベル（先頭 QUICK_CMD_LABEL_LEN 文字 + …）。
// コードポイント単位で数えて絵文字・CJK が途中で割れないようにする。
export function quickCommandLabel(cmd) {
  const chars = [...(cmd || '')];
  if (chars.length <= QUICK_CMD_LABEL_LEN) return chars.join('');
  return chars.slice(0, QUICK_CMD_LABEL_LEN).join('') + '…';
}

export function getQuickCommand(slot) {
  // 未設定(null)は既定値（無ければ空欄）。空文字は「ユーザーが意図的に空にした」
  // 未登録スロットを意味するため、その場合は既定値へ戻さず空のままにする。
  const key = QUICK_CMD_KEYS[slot];
  if (!key) return '';
  const raw = localStorage.getItem(key);
  const fallback = QUICK_CMD_DEFAULTS[slot] || '';
  if (raw === null) return sanitizeQuickCommand(fallback, '');
  return sanitizeQuickCommand(raw, '');
}

// クイックコマンドボタンの表示状態（既定: 表示）。未設定(null)は表示扱い、'0' のみ非表示。
export function getQuickCommandVisible(slot) {
  const key = QUICK_CMD_SHOW_KEYS[slot];
  if (!key) return false;
  return localStorage.getItem(key) !== '0';
}

export function refreshQuickCommandButtons() {
  for (let slot = 1; slot <= QUICK_CMD_SLOTS; slot++) {
    const btn = document.getElementById(QUICK_CMD_BTN_IDS[slot]);
    if (!btn) continue;
    const cmd = getQuickCommand(slot);
    btn.textContent = quickCommandLabel(cmd);
    btn.dataset.tooltip = cmd;
    // 表示トグル ON かつコマンドが空でないときだけボタンを出す。
    btn.hidden = !(getQuickCommandVisible(slot) && cmd !== '');
  }
}

// @attachment と先頭スラッシュコマンドを除外してカード表示用テキストを返す
export function filterFirstMessage(text) {
  if (!text) return '';
  const cleaned = text
    // CSI エスケープシーケンス（ESC [ ... 終端）を除去。ブラケットペースト ESC[200~ / ESC[201~ もここで消える
    .replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, '')
    // OSC / その他 ESC + 1 文字シーケンス（ESC D など）を除去
    .replace(/\x1b[\]P^_X][\s\S]*?(?:\x07|\x1b\\)/g, '')
    .replace(/\x1b./g, '')
    // ESC が剥がれてマーカーの数値部だけ残ったケース（[200~ / [201~）の残骸を除去
    .replace(/\[20[01]~/g, '')
    // 残存する C0 制御文字を除去
    .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, '');
  const trimmed = cleaned.trim();
  if (trimmed.startsWith('/')) return '';
  return trimmed.replace(/@\S+/g, '').replace(/\s+/g, ' ').trim();
}

export function applyTheme(theme) {
  // 'blue' は廃止済み。既存設定が 'blue' の場合は既定テーマにフォールバック
  const t = (theme === 'dark' || theme === 'light') ? theme : 'light';
  document.documentElement.setAttribute('data-theme', t);
  const sel = document.getElementById('theme-select');
  if (sel) sel.value = t;
  try { localStorage.setItem(STORAGE_THEME_KEY, t); } catch (_) {}
}

export function applyFontSize(size) {
  const s = FONTSIZE_MAP[size] ? size : 'medium';
  const px = FONTSIZE_MAP[s];
  document.documentElement.style.setProperty('--chat-font-size', px + 'px');
  // terminals は const なので初期 IIFE 呼び出し時点では TDZ。
  // TDZ では typeof も ReferenceError を投げるため try/catch で守る。初期呼び出し時は terminals 自体未初期化なので何もしないで OK。
  try {
    terminals.forEach((t, id) => {
      t.term.options.fontSize = px;
      requestAnimationFrame(() => {
        fitTerminalPreservingBottom(t, id);
        sendResize(id, t.term.cols, t.term.rows);
      });
    });
  } catch (_) {}
  const sel = document.getElementById('fontsize-select');
  if (sel) sel.value = s;
  try { localStorage.setItem(STORAGE_FONTSIZE_KEY, s); } catch (_) {}
}

export function applyLang(lang) {
  const l = (lang === 'ja' || lang === 'en') ? lang : 'ja';
  const sel = document.getElementById('lang-select');
  if (sel) sel.value = l;
}

(function () {
  applyTheme(localStorage.getItem(STORAGE_THEME_KEY) || 'light');
  applyFontSize(localStorage.getItem(STORAGE_FONTSIZE_KEY) || 'medium');
  applyLang(localStorage.getItem(STORAGE_LANG_KEY) || 'ja');
  applyUsageLinks();
  initUsageDropdown();

  const panel      = document.getElementById('settings-panel');
  const btn        = document.getElementById('settings-btn');
  const themeEl    = document.getElementById('theme-select');
  const fontsizeEl = document.getElementById('fontsize-select');
  const langEl     = document.getElementById('lang-select');
  const resetBtn   = document.getElementById('settings-reset-btn');
  const saveBtn    = document.getElementById('settings-save-btn');
  const closeBtn   = document.getElementById('settings-close-btn');
  const licensesBtn = document.getElementById('settings-licenses-btn');
  const usageLinksResetBtn = document.getElementById('usage-links-reset-btn');

  fontsizeEl.value = localStorage.getItem(STORAGE_FONTSIZE_KEY) || 'medium';

  btn.addEventListener('click', (e) => {
    e.stopPropagation();
    panel.hidden = !panel.hidden;
    if (panel.hidden) maybeAutoSwitchToNextApproval();
  });
  if (closeBtn) {
    closeBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      panel.hidden = true;
      maybeAutoSwitchToNextApproval();
    });
  }

  document.addEventListener('click', (e) => {
    if (!panel.hidden && !panel.contains(e.target)) {
      panel.hidden = true;
      maybeAutoSwitchToNextApproval();
    }
    if (pathPopupEl && !pathPopupEl.hidden && !pathPopupEl.contains(e.target)) {
      pathPopupEl.hidden = true;
    }
  });

  themeEl.addEventListener('change',    () => { applyTheme(themeEl.value); setUserPref('display.theme', themeEl.value); });
  fontsizeEl.addEventListener('change', () => { applyFontSize(fontsizeEl.value); setUserPref('display.font_size', fontsizeEl.value); });
  langEl.addEventListener('change',     async () => {
    // setLang は即 location.reload() するため、debounce を待たず同期 PUT で確実に永続化する。
    // 永続化しないとリロード後に mirror が旧サーバ値で localStorage を上書きし、言語が巻き戻る。
    setUserPref('display.lang', langEl.value);
    try { await _putUserPrefsNow(); } catch (_) {}
    window.setLang(langEl.value);
  });
  if (saveBtn) {
    saveBtn.addEventListener('click', async (e) => {
      e.stopPropagation();
      try {
        if (window.__settingsSaveAll) await window.__settingsSaveAll();
        showToast(t('settings_saved'), saveBtn);
      } catch (_) {}
    });
  }
  if (resetBtn) {
    resetBtn.addEventListener('click', async (e) => {
      e.stopPropagation();
      const ok = await appConfirm({
        title: t('settings_reset_confirm_title'),
        message: t('settings_reset_confirm_message'),
        confirmText: t('settings_reset'),
        cancelText: t('cancel'),
        kind: 'warn',
      });
      if (!ok) return;
      try {
        if (window.__settingsResetAll) await window.__settingsResetAll();
        showToast(t('settings_reset_done'), resetBtn);
      } catch (_) {}
    });
  }

  const aboutPanel = document.getElementById('about-panel');
  const aboutCloseBtn = document.getElementById('about-close-btn');

  document.getElementById('settings-readme-btn').addEventListener('click', () => {
    const file = window.__lang === 'ja' ? 'README.ja.md' : 'README.md';
    window.open(
      'https://github.com/ishizakahiroshi/many-ai-cli/blob/main/' + file,
      '_blank', 'noopener,noreferrer'
    );
  });
  licensesBtn.addEventListener('click', () => {
    panel.hidden = true;
    aboutPanel.hidden = false;
  });
  if (usageLinksResetBtn) {
    usageLinksResetBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      setUserPref('usage_links.claude', '');
      setUserPref('usage_links.codex', '');
      setUserPref('usage_links.copilot', '');
      setUserPref('usage_links.cursor-agent', '');
      setUserPref('usage_links.ollama', '');
      setUserPref('usage_links.opencode', '');
      loadUsageLinkSettings();
      showToast(t('settings_usage_links_reset_done'), usageLinksResetBtn);
    });
  }
  aboutCloseBtn.addEventListener('click', () => { aboutPanel.hidden = true; });
  aboutPanel.addEventListener('click', (e) => {
    if (e.target === aboutPanel) aboutPanel.hidden = true;
  });
})();

// ---- C5: 表示モード固定 (soft lock) 設定 ----
(function () {
  const sel = document.getElementById('locked-mode-select');
  if (!sel) return;
  // 初期値読み込み (localStorage → '' / 'terminal' / 'chat' / 'split')
  const initial = (typeof getDisplayLockedMode === 'function') ? getDisplayLockedMode() : '';
  sel.value = initial || '';
  sel.addEventListener('change', () => {
    const v = sel.value;
    // 空文字 = 自由切替 (lock 解除)。STORAGE 側は '' で正常に削除される
    setUserPref('display.locked_mode', v === '' ? '' : v);
    // 🔒 アイコンの即時反映 (現在開いているセッションのモードは触らない / L2 soft lock)
    if (typeof refreshLockedModeTabClasses === 'function') refreshLockedModeTabClasses();
  });
})();

// ---- 音声入力 エンジン設定 ----
(function () {
  const group = document.getElementById('voice-engine-segment');
  if (!group) return;
  const buttons = Array.from(group.querySelectorAll('[data-voice-engine]'));
  const descEl = document.getElementById('voice-engine-desc');
  const browserUnavailableEl = document.getElementById('voice-browser-unavailable-note');
  const whisperBlock = document.getElementById('voice-whisper-settings');
  const whisperAutoSubmitEl = document.getElementById('voice-whisper-auto-submit');
  const whisperAutoStopEl = document.getElementById('voice-whisper-auto-stop');
  const whisperManagedPanel = document.getElementById('voice-whisper-managed-panel');
  const whisperManagedStateEl = document.getElementById('voice-whisper-managed-state');
  const whisperModelSelect = document.getElementById('voice-whisper-model-select');
  const whisperInstallProgress = document.getElementById('voice-whisper-install-progress');
  const whisperInstallProgressBar = document.getElementById('voice-whisper-install-progress-bar');
  const whisperManagedNote = document.getElementById('voice-whisper-managed-note');
  const whisperInstallBtn = document.getElementById('voice-whisper-install-btn');
  const whisperStartBtn = document.getElementById('voice-whisper-start-btn');
  const whisperStopBtn = document.getElementById('voice-whisper-stop-btn');
  const whisperUninstallBtn = document.getElementById('voice-whisper-uninstall-btn');
  const graceRow = document.getElementById('voice-grace-row');
  const graceDesc = document.getElementById('voice-grace-desc');
  const diagnosticsPanel = document.getElementById('voice-diagnostics-panel');
  let whisperPollTimer = null;
  let lastWhisperStatus = null;
  let whisperUserModelChoice = null;

  function browserRecognitionSupported() {
    const SpeechRecognition = window.SpeechRecognition || window.webkitSpeechRecognition;
    const isChromium = navigator.userAgentData?.brands?.some(b => /Chromium/.test(b.brand))
      ?? /Chrome\//.test(navigator.userAgent);
    return !!SpeechRecognition && !!isChromium;
  }

  function descriptionKey(engine) {
    if (engine === 'off') return 'settings_voice_engine_desc_off';
    if (engine === 'whisper') return 'settings_voice_engine_desc_whisper';
    return 'settings_voice_engine_desc_browser';
  }

  function renderVoiceEngineSettings() {
    const engine = getVoiceEngine();
    const browserSupported = browserRecognitionSupported();
    buttons.forEach((button) => {
      const value = button.dataset.voiceEngine;
      button.classList.toggle('active', value === engine);
      button.setAttribute('aria-checked', value === engine ? 'true' : 'false');
      if (value === 'browser') {
        button.disabled = !browserSupported;
        button.dataset.tooltip = browserSupported ? '' : t('settings_voice_browser_unavailable');
      }
    });
    if (descEl) {
      const key = descriptionKey(engine);
      descEl.dataset.i18n = key;
      descEl.textContent = t(key);
    }
    if (browserUnavailableEl) browserUnavailableEl.hidden = browserSupported;
    if (whisperBlock) whisperBlock.hidden = engine !== 'whisper';
    // 「終了検知の待ち時間」は browser / whisper の両方で表示する（OFF のときだけ隠す）。
    // 値（秒）は両エンジン共通の voice.grace_seconds。Whisper では無音→自動確定までの待ち時間として実効、
    // Chrome 内蔵では Web Speech API の制約からブラウザ側の終了判定が優先される。
    if (graceRow) graceRow.hidden = engine === 'off';
    if (graceDesc) graceDesc.hidden = engine === 'off';
    if (diagnosticsPanel) diagnosticsPanel.hidden = engine !== 'browser';
    if (whisperAutoSubmitEl) {
      whisperAutoSubmitEl.checked = localStorage.getItem(STORAGE_VOICE_WHISPER_AUTO_SUBMIT_KEY) === '1';
    }
    if (whisperAutoStopEl) {
      whisperAutoStopEl.checked = localStorage.getItem(STORAGE_VOICE_WHISPER_AUTO_STOP_KEY) !== '0';
    }
    if (engine === 'whisper') {
      refreshWhisperStatus();
    } else {
      setWhisperPolling(false);
    }
  }

  function formatWhisperBytes(bytes) {
    const n = Number(bytes || 0);
    if (!Number.isFinite(n) || n <= 0) return '';
    if (n >= 1024 * 1024 * 1024) return `${(n / 1024 / 1024 / 1024).toFixed(1)} GB`;
    if (n >= 1024 * 1024) return `${Math.round(n / 1024 / 1024)} MB`;
    return `${Math.round(n / 1024)} KB`;
  }

  function setWhisperPolling(active) {
    if (whisperPollTimer) {
      clearInterval(whisperPollTimer);
      whisperPollTimer = null;
    }
    if (active) {
      whisperPollTimer = setInterval(refreshWhisperStatus, 2000);
    }
  }

  function renderWhisperStatus(data) {
    lastWhisperStatus = data;
    if (!data || !whisperManagedPanel) return;
    whisperManagedPanel.hidden = false;
    if (whisperModelSelect && Array.isArray(data.models)) {
      const current = whisperModelSelect.value || data.model || '';
      whisperModelSelect.innerHTML = '';
      for (const model of data.models) {
        const opt = document.createElement('option');
        opt.value = model.id;
        const size = formatWhisperBytes(model.size_bytes);
        opt.textContent = size ? `${model.label} (${size})` : model.label;
        opt.title = model.quality || '';
        whisperModelSelect.appendChild(opt);
      }
      if (whisperUserModelChoice && whisperUserModelChoice === data.model) {
        whisperUserModelChoice = null;
      }
      whisperModelSelect.value = (whisperUserModelChoice && !data.running)
        ? whisperUserModelChoice
        : (data.model || current);
    }
    const install = data.install || {};
    const installing = !!install.installing;
    const percent = Math.round((Number(install.progress || 0) * 100));
    if (whisperInstallProgress) whisperInstallProgress.hidden = !installing;
    if (whisperInstallProgressBar) whisperInstallProgressBar.style.width = `${Math.max(0, Math.min(100, percent))}%`;

    let stateText = '';
    if (!data.supported) {
      stateText = t('settings_voice_whisper_status_unsupported');
    } else if (installing) {
      stateText = t('settings_voice_whisper_status_installing')
        .replace('{phase}', install.phase || '')
        .replace('{percent}', String(percent));
    } else if (install.error) {
      stateText = t('settings_voice_whisper_status_error').replace('{error}', install.error);
    } else if (data.running) {
      stateText = t('settings_voice_whisper_status_running');
    } else if (data.installed) {
      stateText = t('settings_voice_whisper_status_installed');
    } else {
      stateText = t('settings_voice_whisper_status_not_installed');
    }
    if (whisperManagedStateEl) whisperManagedStateEl.textContent = stateText;
    if (document.getElementById('voice-whisper-status')) {
      const statusEl = document.getElementById('voice-whisper-status');
      statusEl.textContent = data.server_url || stateText;
      statusEl.dataset.i18n = '';
    }
    const supported = !!data.supported;
    if (whisperInstallBtn) whisperInstallBtn.disabled = !supported || installing;
    if (whisperStartBtn) whisperStartBtn.disabled = !supported || installing || !data.installed || data.running;
    if (whisperStopBtn) whisperStopBtn.disabled = !supported || installing || !data.running;
    if (whisperUninstallBtn) whisperUninstallBtn.disabled = !supported || installing || (!data.installed && !data.managed);
    if (whisperModelSelect) whisperModelSelect.disabled = !supported || installing || data.running;
    if (whisperManagedNote) {
      if (!supported) {
        whisperManagedNote.textContent = data.manual_only_message || t('settings_voice_whisper_status_unsupported');
      } else if (data.installed) {
        whisperManagedNote.textContent = t('settings_voice_whisper_note_ready').replace('{path}', data.install_dir || '');
      } else {
        whisperManagedNote.textContent = t('settings_voice_whisper_note_download');
      }
    }
    setWhisperPolling(installing && getVoiceEngine() === 'whisper');
  }

  async function refreshWhisperStatus() {
    if (!whisperManagedPanel) return;
    try {
      const res = await fetch(`/api/whisper/status?token=${encodeURIComponent(token || '')}`);
      if (!res.ok) return;
      renderWhisperStatus(await res.json());
    } catch (_) {}
  }

  async function postWhisperAction(action, body = null) {
    const init: any = { method: 'POST' };
    if (body) {
      init.headers = { 'Content-Type': 'application/json' };
      init.body = JSON.stringify(body);
    }
    const res = await fetch(`/api/whisper/${action}?token=${encodeURIComponent(token || '')}`, init);
    let data = null;
    try { data = await res.json(); } catch (_) {}
    if (!res.ok) {
      const detail = data?.detail || data?.error || `${action} failed`;
      throw new Error(detail);
    }
    renderWhisperStatus(data);
    return data;
  }

  buttons.forEach((button) => {
    button.addEventListener('click', () => {
      const engine = button.dataset.voiceEngine;
      if (!engine || button.disabled) return;
      const voiceBtn = document.getElementById('voice-btn');
      if (voiceBtn?.classList.contains('recording')) {
        document.getElementById('voice-cancel-btn')?.click();
      }
      setVoiceEngine(engine);
      renderVoiceEngineSettings();
    });
  });

  whisperModelSelect?.addEventListener('change', () => {
    whisperUserModelChoice = whisperModelSelect.value || null;
  });
  whisperAutoSubmitEl?.addEventListener('change', () => {
    localStorage.setItem(STORAGE_VOICE_WHISPER_AUTO_SUBMIT_KEY, whisperAutoSubmitEl.checked ? '1' : '0');
  });
  whisperAutoStopEl?.addEventListener('change', () => {
    localStorage.setItem(STORAGE_VOICE_WHISPER_AUTO_STOP_KEY, whisperAutoStopEl.checked ? '1' : '0');
  });
  whisperInstallBtn?.addEventListener('click', async () => {
    try {
      await postWhisperAction('install', { model: whisperModelSelect?.value || lastWhisperStatus?.model || 'small' });
      showToast(t('settings_voice_whisper_action_done'), whisperInstallBtn);
    } catch (err) {
      showToast(String(err?.message || err), whisperInstallBtn);
    }
  });
  whisperStartBtn?.addEventListener('click', async () => {
    try {
      const model = whisperModelSelect?.value || '';
      await postWhisperAction('start', model ? { model } : null);
      showToast(t('settings_voice_whisper_action_done'), whisperStartBtn);
    } catch (err) {
      showToast(String(err?.message || err), whisperStartBtn);
    }
  });
  whisperStopBtn?.addEventListener('click', async () => {
    try {
      await postWhisperAction('stop');
      showToast(t('settings_voice_whisper_action_done'), whisperStopBtn);
    } catch (err) {
      showToast(String(err?.message || err), whisperStopBtn);
    }
  });
  whisperUninstallBtn?.addEventListener('click', async () => {
    if (!window.confirm(t('settings_voice_whisper_uninstall'))) return;
    try {
      await postWhisperAction('uninstall');
      showToast(t('settings_voice_whisper_action_done'), whisperUninstallBtn);
    } catch (err) {
      showToast(String(err?.message || err), whisperUninstallBtn);
    }
  });

  document.addEventListener('voiceengine:changed', renderVoiceEngineSettings);
  renderVoiceEngineSettings();
})();

// ---- 音声入力 終了検知 待ち時間 設定 ----
(function () {
  const sel = document.getElementById('voice-grace-select');
  if (!sel) return;
  // 旧来は「2 を既定値リセット対象として消す」移行処理があったが、
  // この設定が Whisper の無音→自動確定にも実効するようになり 2 秒が標準的な既定値になったため撤去。
  const saved = localStorage.getItem(STORAGE_VOICE_GRACE_KEY);
  const v = saved == null ? DEFAULT_VOICE_GRACE_SEC : parseInt(saved, 10);
  const clamped = Number.isFinite(v) ? Math.max(0, Math.min(5, v)) : DEFAULT_VOICE_GRACE_SEC;
  sel.value = String(clamped);
  sel.addEventListener('change', () => {
    setUserPref('voice.grace_seconds', parseInt(sel.value, 10) || 0);
  });
})();

// ---- 入力補助ツール表示トグル（PC / スマホ 独立）----
// PC（広い画面）は既定 ON: OFF にした時だけ body.pc-input-tools-off で隠す。
// スマホ（狭い画面 max-width:720px / pointer:coarse）は既定 OFF:
//   ON にした時だけ body.mobile-input-tools-on で表示する。
// 2 つは別キーで、画面幅ごとに独立して効く（CSS 側でメディアクエリ分岐）。
(function () {
  // PC: 既定 ON（localStorage 未設定 or '0' 以外を ON とみなす）
  const pcEl = document.getElementById('pc-input-tools-enabled') as HTMLInputElement | null;
  if (pcEl) {
    const applyPc = (on: boolean) => {
      document.body.classList.toggle('pc-input-tools-off', !on);
    };
    pcEl.addEventListener('change', () => {
      applyPc(pcEl.checked);
      setUserPref('pc.input_tools_enabled', pcEl.checked);
    });
    pcEl.checked = localStorage.getItem(STORAGE_PC_INPUT_TOOLS_KEY) !== '0';
    applyPc(pcEl.checked);
  }

  // スマホ: 既定 OFF（'1' の時だけ ON）
  const mobileEl = document.getElementById('mobile-input-tools-enabled') as HTMLInputElement | null;
  if (mobileEl) {
    const applyMobile = (on: boolean) => {
      document.body.classList.toggle('mobile-input-tools-on', on);
    };
    mobileEl.addEventListener('change', () => {
      applyMobile(mobileEl.checked);
      setUserPref('mobile.input_tools_enabled', mobileEl.checked);
    });
    mobileEl.checked = localStorage.getItem(STORAGE_MOBILE_INPUT_TOOLS_KEY) === '1';
    applyMobile(mobileEl.checked);
  }
})();

// ---- トリガーフレーズ設定 ----
(function () {
  const enabledEl    = document.getElementById('trigger-enabled');
  const phraseRow    = document.getElementById('trigger-phrase-row');
  const phraseInputEl = document.getElementById('trigger-phrase-input');
  if (!enabledEl) return;

  enabledEl.addEventListener('change', () => {
    phraseRow.hidden = !enabledEl.checked;
    setUserPref('trigger.enabled', enabledEl.checked);
  });
  phraseInputEl.addEventListener('input', () => {
    setUserPref('trigger.phrase', phraseInputEl.value);
  });

  enabledEl.checked = localStorage.getItem(STORAGE_TRIGGER_ENABLED_KEY) === '1';
  phraseRow.hidden = !enabledEl.checked;
  phraseInputEl.value = localStorage.getItem(STORAGE_TRIGGER_PHRASE_KEY) ?? getDefaultTriggerPhrase();
})();

// ---- ウェイクワード設定 ----
(function () {
  const enabledEl = document.getElementById('wakeword-enabled');
  const phraseRow = document.getElementById('wakeword-phrase-row');
  const phraseEl  = document.getElementById('wakeword-phrase-input');
  if (!enabledEl) return;

  enabledEl.addEventListener('change', () => {
    phraseRow.hidden = !enabledEl.checked;
    setUserPref('voice.wake_word_enabled', enabledEl.checked);
    document.dispatchEvent(new CustomEvent('wakewordsettings:changed'));
  });
  phraseEl.addEventListener('input', () => {
    setUserPref('voice.wake_word_phrase', phraseEl.value);
  });

  enabledEl.checked = localStorage.getItem(STORAGE_WAKE_WORD_ENABLED_KEY) === '1';
  phraseRow.hidden = !enabledEl.checked;
  phraseEl.value = localStorage.getItem(STORAGE_WAKE_WORD_PHRASE_KEY) ?? getDefaultWakeWordPhrase();
})();

// ---- デスクトップ通知設定 ----
(function () {
  const enabledEl = document.getElementById('desktop-notify-enabled');
  const statusEl = document.getElementById('desktop-notify-status');
  if (!enabledEl) return;

  function renderStatus() {
    const perm = desktopNotificationPermission();
    if (statusEl) {
      const key = perm === 'granted' ? 'desktop_notification_granted'
        : perm === 'denied' ? 'desktop_notification_denied'
        : perm === 'unsupported' ? 'desktop_notification_unsupported'
        : 'desktop_notification_default';
      statusEl.textContent = t(key);
    }
    enabledEl.checked = desktopNotificationsEnabled() && perm === 'granted';
    enabledEl.disabled = perm === 'unsupported' || perm === 'denied';
  }

  enabledEl.addEventListener('change', async () => {
    if (!enabledEl.checked) {
      setUserPref('desktop_notifications.enabled', false);
      renderStatus();
      return;
    }
    const perm = await requestDesktopNotificationPermission();
    if (perm === 'granted') {
      setUserPref('desktop_notifications.enabled', true);
    } else {
      setUserPref('desktop_notifications.enabled', false);
    }
    renderStatus();
  });

  renderStatus();
})();

// ---- プッシュ通知設定 ----
(function () {
  const enabledEl = document.getElementById('push-notify-enabled');
  const statusEl = document.getElementById('push-notify-status');
  if (!enabledEl) return;

  async function renderStatus() {
    const supported = pushNotificationsSupported();
    const iosPwaRequired = isLikelyIOSBrowserTabWithoutStandalone();
    const permission = ('Notification' in window) ? (Notification.permission || 'default') : 'unsupported';
    let subscribed = false;
    let serverSupported = false;
    let key = 'push_notification_unsupported';

    if (iosPwaRequired) {
      key = 'push_notification_pwa_required';
    } else if (supported) {
      const [subscription, status] = await Promise.all([
        getPushSubscription().catch(() => null),
        fetchPushStatus().catch(() => ({ supported: false })),
      ]);
      subscribed = !!subscription;
      serverSupported = !!(status && status.supported);
      key = !serverSupported ? 'push_notification_unavailable'
        : subscribed ? 'push_notification_enabled'
        : permission === 'denied' ? 'push_notification_denied'
        : permission === 'granted' ? 'push_notification_ready'
        : 'push_notification_default';
    }

    if (statusEl) statusEl.textContent = t(key);
    enabledEl.checked =
      localStorage.getItem(STORAGE_PUSH_NOTIFY_ENABLED_KEY) === '1' &&
      subscribed &&
      permission === 'granted';
    enabledEl.disabled = iosPwaRequired || !supported || !serverSupported || permission === 'denied';
  }

  enabledEl.addEventListener('change', async () => {
    enabledEl.disabled = true;
    try {
      if (!enabledEl.checked) {
        await unsubscribeWebPush();
        setUserPref('push_notifications.enabled', false);
      } else if (isLikelyIOSBrowserTabWithoutStandalone()) {
        setUserPref('push_notifications.enabled', false);
        showToast(t('push_notification_pwa_required'));
      } else {
        await subscribeWebPush();
        setUserPref('push_notifications.enabled', true);
      }
    } catch (_) {
      setUserPref('push_notifications.enabled', false);
      showToast(t('push_notification_enable_failed'));
    } finally {
      await renderStatus();
    }
  });

  renderStatus();
})();

// ---- 通知音設定 ----
(function () {
  const soundEnabledEl  = document.getElementById('notify-sound-enabled');
  const soundTypeEl     = document.getElementById('notify-sound-type');
  const soundTypeRow    = document.getElementById('notify-sound-type-row');
  const soundCustomRow  = document.getElementById('notify-sound-custom-row');
  const soundFileEl     = document.getElementById('notify-sound-file');
  const soundBrowseBtn  = document.getElementById('notify-sound-browse-btn');
  const soundFilenameEl = document.getElementById('notify-sound-filename');
  const soundTestBtn    = document.getElementById('notify-sound-test-btn');
  if (!soundEnabledEl) return;

  function updateSoundVisibility() {
    soundTypeRow.hidden    = !soundEnabledEl.checked;
    soundCustomRow.hidden  = !soundEnabledEl.checked || soundTypeEl.value !== 'custom';
  }

  soundEnabledEl.addEventListener('change', () => {
    setUserPref('notify_sound.enabled', soundEnabledEl.checked);
    updateSoundVisibility();
  });

  soundTypeEl.addEventListener('change', () => {
    setUserPref('notify_sound.type', soundTypeEl.value);
    updateSoundVisibility();
  });

  soundBrowseBtn.addEventListener('click', () => soundFileEl.click());

  soundFileEl.addEventListener('change', () => {
    const file = soundFileEl.files[0];
    if (!file) return;
    // バイナリをサーバに PUT（dataURL ではなく binary 送信）
    const uploadCustomSound = async (f) => {
      const tk = token;
      try {
        const buf = await f.arrayBuffer();
        const putRes = await fetch(`/api/user-prefs/notify-sound-custom?token=${tk}`, {
          method: 'PUT',
          headers: { 'Content-Type': f.type || 'application/octet-stream' },
          body: buf,
        });
        if (!putRes.ok) throw new Error(`PUT notify-sound-custom ${putRes.status}`);
        // custom_file フラグをサーバ側で設定済みのため type を custom にする
        setUserPref('notify_sound.type', 'custom');
        soundTypeEl.value = 'custom';
        soundFilenameEl.textContent = f.name;
        // localStorage にはファイル名だけ記録（dataURL は不要）
        try { localStorage.setItem(STORAGE_NOTIFY_SOUND_CUSTOM_KEY, f.name); } catch (_) {}
        showToast(typeof window.t === 'function' ? t('user_prefs_notify_sound_set') : 'Custom sound set.');
        updateSoundVisibility();
      } catch (e) {
        console.warn('[user-prefs] notify-sound upload failed:', e);
        showToast(typeof window.t === 'function' ? t('user_prefs_notify_sound_upload_failed') : 'Custom sound upload failed.');
      }
    };
    uploadCustomSound(file);
  });

  soundTestBtn.addEventListener('click', () => {
    const prev = localStorage.getItem(STORAGE_NOTIFY_SOUND_ENABLED_KEY);
    localStorage.setItem(STORAGE_NOTIFY_SOUND_ENABLED_KEY, '1');
    playNotificationSound();
    localStorage.setItem(STORAGE_NOTIFY_SOUND_ENABLED_KEY, prev || '0');
  });

  soundEnabledEl.checked = localStorage.getItem(STORAGE_NOTIFY_SOUND_ENABLED_KEY) === '1';
  soundTypeEl.value      = localStorage.getItem(STORAGE_NOTIFY_SOUND_TYPE_KEY) || 'default';
  const savedCustom = localStorage.getItem(STORAGE_NOTIFY_SOUND_CUSTOM_KEY);
  if (savedCustom) soundFilenameEl.textContent = savedCustom.startsWith('data:') ? t('notify_sound_file_set') : savedCustom;
  updateSoundVisibility();
})();

// ---- クイックコマンド設定 ----
(function () {
  // スロット1の入力欄が無ければ設定パネル未描画とみなして何もしない。
  if (!document.getElementById('quick-cmd-1')) return;

  refreshQuickCommandButtons();

  // 入力中はボタンのラベル・表示を即時プレビュー（保存はフォーカス確定時の change）。
  const bind = (el, btnId, prefKey, slot) => {
    const btn = document.getElementById(btnId);
    el.addEventListener('input', () => {
      if (!btn) return;
      const v = el.value.trim();
      btn.textContent = quickCommandLabel(v);
      btn.dataset.tooltip = v;
      btn.hidden = !(getQuickCommandVisible(slot) && v !== '');
    });
    el.addEventListener('change', () => {
      const value = sanitizeQuickCommand(el.value, '');
      el.value = value;
      setUserPref(prefKey, value);
      refreshQuickCommandButtons();
    });
  };
  const bindShow = (el, prefKey, slot) => {
    if (!el) return;
    el.checked = getQuickCommandVisible(slot);
    el.addEventListener('change', () => {
      setUserPref(prefKey, el.checked);
      refreshQuickCommandButtons();
    });
  };

  for (let slot = 1; slot <= QUICK_CMD_SLOTS; slot++) {
    const cmdEl = document.getElementById(`quick-cmd-${slot}`);
    if (cmdEl) {
      cmdEl.value = getQuickCommand(slot);
      bind(cmdEl, quickCommandButtonId(slot), `quick_cmds.cmd${slot}`, slot);
    }
    const showEl = document.getElementById(`quick-cmd-${slot}-show`);
    bindShow(showEl, `quick_cmds.show${slot}`, slot);
  }
})();

// ---- アバター・表示名設定 ----
(function () {
  const previewEl   = document.getElementById('settings-avatar-preview');
  const statusEl    = document.getElementById('settings-avatar-status');
  const nameInputEl = document.getElementById('display-name-input');
  const urlInputEl  = document.getElementById('avatar-url-input');
  const applyBtn    = document.getElementById('avatar-url-apply-btn');
  const fileBtn     = document.getElementById('avatar-file-btn');
  const fileInputEl = document.getElementById('avatar-file-input');
  const clearBtn    = document.getElementById('avatar-clear-btn');
  if (!previewEl) return;

  function getInitialLetter(name) {
    if (name) return [...name][0].toUpperCase();
    return (document.documentElement.lang || '').startsWith('ja') ? 'あ' : 'Y';
  }

  function updatePreview(avatarUrl, displayName) {
    previewEl.innerHTML = '';
    if (avatarUrl) {
      const img = document.createElement('img');
      img.src = avatarUrl;
      img.alt = getInitialLetter(displayName);
      img.onload = () => {
        statusEl.textContent = typeof window.t === 'function' ? t('settings_avatar_loaded') : '画像を読み込み済み';
        statusEl.className = 'settings-avatar-status ok';
      };
      img.onerror = () => {
        img.remove();
        previewEl.textContent = getInitialLetter(displayName);
        statusEl.textContent = typeof window.t === 'function' ? t('settings_avatar_load_failed') : '読み込み失敗';
        statusEl.className = 'settings-avatar-status err';
      };
      previewEl.appendChild(img);
      statusEl.textContent = '';
      statusEl.className = 'settings-avatar-status';
    } else {
      previewEl.textContent = getInitialLetter(displayName);
      statusEl.textContent = '';
      statusEl.className = 'settings-avatar-status';
    }
  }

  async function patchServerPref(path, value) {
    const tk = token;
    try {
      const getRes = await fetch(`/api/user-prefs?token=${encodeURIComponent(tk || '')}`);
      if (!getRes.ok) throw new Error(`GET ${getRes.status}`);
      const prefs = await getRes.json();
      _setNestedValue(prefs, path, value);
      const putRes = await fetch(`/api/user-prefs?token=${encodeURIComponent(tk || '')}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(prefs),
      });
      if (!putRes.ok) throw new Error(`PUT ${putRes.status}`);
    } catch (e) {
      console.warn('[user-prefs] patch failed:', e);
      showToast(typeof window.t === 'function' ? t('user_prefs_save_failed_network') : '設定の保存に失敗しました');
    }
  }

  function initFromGlobals() {
    nameInputEl.value = _userDisplayName || '';
    const isLocalFile = _userAvatarUrl && _userAvatarUrl.startsWith('/api/avatar');
    urlInputEl.value = isLocalFile ? '' : (_userAvatarUrl || '');
    updatePreview(_userAvatarUrl, _userDisplayName);
  }

  document.addEventListener('user-info-ready', initFromGlobals, { once: true });
  const sectionEl = document.querySelector('.settings-section[data-section="appearance"]');
  if (sectionEl) {
    sectionEl.addEventListener('toggle', () => { if (sectionEl.open) initFromGlobals(); });
  }

  let _nameDebounce = null;
  nameInputEl.addEventListener('input', () => {
    clearTimeout(_nameDebounce);
    _nameDebounce = setTimeout(async () => {
      const val = nameInputEl.value;
      set__userDisplayName(val);
      updatePreview(_userAvatarUrl, _userDisplayName);
      await patchServerPref('display_name', val);
    }, 500);
  });

  applyBtn.addEventListener('click', async () => {
    const url = urlInputEl.value.trim();
    set__userAvatarUrl(url);
    updatePreview(url, _userDisplayName);
    await patchServerPref('avatar', url);
  });

  fileBtn.addEventListener('click', () => fileInputEl.click());

  fileInputEl.addEventListener('change', async () => {
    const file = fileInputEl.files[0];
    if (!file) return;
    const tk = token;
    try {
      const buf = await file.arrayBuffer();
      const res = await fetch(`/api/user-prefs/avatar?token=${tk}`, {
        method: 'PUT',
        headers: { 'Content-Type': file.type || 'application/octet-stream' },
        body: buf,
      });
      if (!res.ok) throw new Error(`PUT avatar ${res.status}`);
      // キャッシュバスター付きで更新
      set__userAvatarUrl(`/api/avatar?token=${tk}`);
      urlInputEl.value = '';
      updatePreview(`/api/avatar?token=${tk}&t=${Date.now()}`, _userDisplayName);
      showToast(typeof window.t === 'function' ? t('settings_avatar_file_set') : 'アイコン画像を設定しました');
    } catch (e) {
      console.warn('[user-prefs] avatar upload failed:', e);
      showToast(typeof window.t === 'function' ? t('settings_avatar_upload_failed') : '画像のアップロードに失敗しました');
    }
    fileInputEl.value = '';
  });

  clearBtn.addEventListener('click', async () => {
    urlInputEl.value = '';
    set__userAvatarUrl('');
    updatePreview('', _userDisplayName);
    await patchServerPref('avatar', '');
  });
})();

// token moved to app/util.js (ESM shared export)

// usage-link デフォルトをリモート（GitHub バック）から取得して更新。
// 失敗時は DEFAULT_USAGE_LINKS のハードコード値をそのまま使う。
(async () => {
  try {
    const res = await fetch(`/api/usage-link-defaults?token=${encodeURIComponent(token || '')}`);
    if (!res.ok) return;
    const d = await res.json();
    for (const k of ['claude', 'codex', 'copilot', 'cursor-agent', 'ollama', 'opencode']) {
      // 空文字は無視（GitHub 側が古くキーを欠く場合に空で返るため、
      // ローカルの正しいデフォルト値を潰さない）
      if (typeof d[k] === 'string' && d[k] !== '') DEFAULT_USAGE_LINKS[k] = d[k];
    }
    applyUsageLinks();
  } catch (_) {}
})();

// renderStaleBinaryBanner は #stale-binary-banner を stale フラグに応じて出し分ける。
// 常設・dismissible。multi-question-banner と同じ -text / -close クラスを流用する。
function renderStaleBinaryBanner(stale: boolean): void {
  const banner = document.getElementById('stale-binary-banner');
  if (!banner) return;
  const tr = (key: string, fallback: string): string => {
    if (typeof window.t !== 'function') return fallback;
    const v = window.t(key);
    return v && v !== key ? v : fallback;
  };
  if (!stale) {
    banner.hidden = true;
    banner.innerHTML = '';
    return;
  }
  banner.innerHTML = '';
  const msg = document.createElement('span');
  msg.className = 'multi-question-banner-text';
  msg.textContent = tr('stale_binary_banner', 'This Hub is running an old build; restart it to apply your rebuild.');
  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.className = 'multi-question-banner-close';
  closeBtn.textContent = '×';
  closeBtn.title = tr('stale_binary_banner_close_tooltip', 'Dismiss');
  closeBtn.addEventListener('click', () => { banner.hidden = true; });
  banner.appendChild(msg);
  banner.appendChild(closeBtn);
  banner.hidden = false;
}

// ---- Hub 情報表示（single source: main.version / runtime → /api/info → ここ） ----
(async () => {
  try {
    const res = await fetch(`/api/info?token=${token}`);
    if (!res.ok) return;
    const info = await res.json();
    set__userAvatarUrl(info.userAvatar || '');
    set__userDisplayName(info.userDisplayName || '');
    document.dispatchEvent(new CustomEvent('user-info-ready'));
    // 稼働中 Hub が古いバイナリ（起動後にディスクの exe が差し替わった）なら
    // 常設バナーで再起動を促す。multi-question-banner と同じ構造・クラスを流用。
    renderStaleBinaryBanner(!!info.binary_stale);
    const ver = 'v' + (info.version || 'dev');
    const runtimeMode = info.runtime_mode || '';
    const runtimeLabel = () => {
      if (typeof window.t !== 'function') return info.runtime_label || runtimeMode;
      const key = `runtime_${String(runtimeMode).replace(/-/g, '_')}`;
      const translated = window.t(key);
      return translated && translated !== key ? translated : (info.runtime_label || runtimeMode);
    };
    const envFallbacks: Record<string, { label: string; short: string; color: string; title: string }> = {
      local: { label: 'Local', short: 'L', color: '#8b5cf6', title: 'L MANY-AI-CLI' },
      wsl: { label: 'WSL', short: 'W', color: '#3b82f6', title: 'W MANY-AI-CLI' },
      remote: { label: 'Remote server', short: 'R', color: '#f97316', title: 'R MANY-AI-CLI' },
      'remote-tunnel': { label: 'Remote server (tunnel)', short: 'T', color: '#ef4444', title: 'T MANY-AI-CLI' },
    };
    const normalizeEnvKind = (value) => {
      const raw = String(value || '').trim().toLowerCase().replace(/_/g, '-');
      if (raw === 'local' || raw === 'wsl' || raw === 'remote' || raw === 'remote-tunnel') return raw;
      if (raw === 'remotetunnel') return 'remote-tunnel';
      return '';
    };
    const sanitizeEnvColor = (value, fallback) => {
      const color = String(value || '').trim();
      return /^#[0-9a-fA-F]{6}$/.test(color) ? color : fallback;
    };
    // SSH セッション経由なら runtime バッジに SSH のみ常時表示し、IP/host は
    // title とクリック時の一時展開にだけ出す。スクリーンショットへの映り込みを避けるため。
    // launcher の SSH tunnel モードでは既存 Hub が SSH 経由かどうかを自己判定できない
    // （SSH_CONNECTION 等を持たず、NIC からはコンテナ内部 IP しか見えない）ため、
    // launcher が URL クエリ（via=ssh / host_label=<接続先 host>）で渡すヒントを最優先する。
    // リロード・タブ内遷移でクエリが落ちても保持できるよう sessionStorage に退避する。
    const urlParams = new URLSearchParams(location.search);
    if (urlParams.get('via') === 'ssh') sessionStorage.setItem('netHint.ssh', '1');
    if (urlParams.get('host_label')) sessionStorage.setItem('netHint.hostLabel', urlParams.get('host_label'));
    if (urlParams.get('env_kind')) sessionStorage.setItem('netHint.envKind', urlParams.get('env_kind'));
    const hintSSH = sessionStorage.getItem('netHint.ssh') === '1';
    const hintHost = sessionStorage.getItem('netHint.hostLabel') || '';
    const hintEnvKind = normalizeEnvKind(sessionStorage.getItem('netHint.envKind'));
    const showSSH = hintSSH || info.ssh;
    const showHost = hintHost || info.host_ip || '';
    const serverEnvKind = normalizeEnvKind(info.env_kind);
    const envKind = serverEnvKind || hintEnvKind || (hintSSH ? 'remote-tunnel' : 'local');
    const envBase = envFallbacks[envKind] || envFallbacks.local;
    const envLabelRaw = info.env_label || envBase.label;
    const envShort = info.env_short || envBase.short;
    const envColor = sanitizeEnvColor(info.env_color, envBase.color);
    const envTitle = info.env_title || envBase.title || `${envShort} MANY-AI-CLI`;
    const connectionSuffix = showSSH ? ' SSH' : '';
    // SSH 経由の Hub ではフォルダ選択ダイアログがリモート側で開いてしまい使えないため、
    // spawn パネルのフォルダ参照ボタンを非表示にする。
    // WSL（ランチャー経由）は powershell.exe interop で Windows ダイアログが開けるので残す。
    if (showSSH) {
      const browseBtn = document.getElementById('spawn-cwd-browse');
      if (browseBtn) browseBtn.hidden = true;
    }
    const apply = () => {
      const runtime = runtimeLabel();
      const envKey = `env_${String(envKind).replace(/-/g, '_')}`;
      const translatedEnv = (typeof window.t === 'function') ? window.t(envKey) : '';
      const envLabel = translatedEnv && translatedEnv !== envKey ? translatedEnv : envLabelRaw;
      document.title = 'MANY-AI-CLI';
      // CLI ロゴをベースに、環境略号（L/W/V/T）を左下隅の小バッジとして重ねる
      // （drawFavicon 側で合成）。旧実装は全面バッジ化でロゴが消えていた。
      setFaviconEnvBadge(envShort, envColor);
      const appleTitle = document.getElementById('apple-web-app-title');
      if (appleTitle) appleTitle.setAttribute('content', 'MANY-AI-CLI');
      const envBadgeEl = document.getElementById('env-badge');
      if (envBadgeEl) {
        envBadgeEl.textContent = envLabel.toUpperCase();
        envBadgeEl.hidden = false;
        envBadgeEl.dataset.envKind = envKind;
        envBadgeEl.style.setProperty('--env-color', envColor);
        envBadgeEl.title = info.env_host_label || showHost || envLabel;
      }
      const badgeEl = document.getElementById('runtime-badge');
      if (badgeEl && runtime) {
        const baseText = runtime + connectionSuffix;
        badgeEl.textContent = baseText;
        badgeEl.hidden = false;
        badgeEl.dataset.mode = runtimeMode;
        badgeEl.dataset.baseText = baseText;
        badgeEl.dataset.hostExpanded = '0';
        if (showHost) {
          badgeEl.title = showHost;
          badgeEl.dataset.hostLabel = showHost;
        } else {
          badgeEl.removeAttribute('title');
          delete badgeEl.dataset.hostLabel;
        }
        if (!badgeEl.dataset.hostToggleAttached) {
          badgeEl.dataset.hostToggleAttached = '1';
          badgeEl.addEventListener('click', () => {
            const host = badgeEl.dataset.hostLabel || '';
            const base = badgeEl.dataset.baseText || badgeEl.textContent || '';
            if (!host || !base) return;
            const expanded = badgeEl.dataset.hostExpanded === '1';
            badgeEl.dataset.hostExpanded = expanded ? '0' : '1';
            badgeEl.textContent = expanded ? base : `${base} ${host}`;
          });
        }
      }
      const settingsEl = document.querySelector('.settings-app-version');
      if (settingsEl) settingsEl.textContent = runtime ? `${ver} [Hub UI] - ${envLabel} - ${runtime}` : `${ver} [Hub UI] - ${envLabel}`;
      const aboutEl = document.querySelector('.about-version');
      if (aboutEl) {
        aboutEl.textContent = (typeof window.t === 'function')
          ? window.t('about_version', { version: ver })
          : ver + ' [Hub UI]';
      }
      const aboutRuntimeEl = document.querySelector('.about-runtime');
      if (aboutRuntimeEl) {
        aboutRuntimeEl.textContent = runtime
          ? (typeof window.t === 'function' ? window.t('about_runtime_env', { env: envLabel, runtime }) : `Environment: ${envLabel} / Runtime: ${runtime}`)
          : (typeof window.t === 'function' ? window.t('about_env', { env: envLabel }) : `Environment: ${envLabel}`);
      }
    };
    if (typeof window.t === 'function') {
      apply();
    } else {
      document.addEventListener('i18n-ready', apply, { once: true });
    }
  } catch (_) {}
})();

// ---- 承認検出パターン編集 UI ----
window.approvalPatternsUI = (function () {
  const providerEl = document.getElementById('approval-patterns-provider');
  const profileEl = document.getElementById('approval-patterns-profile');
  const listEl = document.getElementById('approval-patterns-list');
  const inputEl = document.getElementById('approval-patterns-input');
  const addBtn = document.getElementById('approval-patterns-add-btn');
  const copyOfficialBtn = document.getElementById('approval-patterns-copy-official-btn');
  const readonlyNote = document.getElementById('approval-patterns-readonly-note');
  if (!providerEl || !profileEl || !listEl || !inputEl || !addBtn || !copyOfficialBtn) return null;

  // プロファイル別のキャッシュ。{ claude: { official: [], custom: [] }, ... }
  const cache = {
    claude: { official: [], custom: [] },
    codex:  { official: [], custom: [] },
    copilot: { official: [], custom: [] },
    'cursor-agent': { official: [], custom: [] },
    common: { official: [], custom: [] },
  };
  // アクティブプロファイル設定（サーバ側 ApprovalProfiles と同期）
  let activeProfiles = { claude: 'official', codex: 'official', copilot: 'official', 'cursor-agent': 'official', common: 'official' };

  function currentProvider() { return providerEl.value; }
  function currentProfile() { return profileEl.value; }
  function isReadonly() { return currentProfile() === 'official'; }

  async function loadActive() {
    try {
      const profRes = await fetch(`/api/approval-patterns/profile?token=${token}`);
      if (profRes.ok) {
        const p = await profRes.json();
        activeProfiles = {
          claude: p.claude || 'official',
          codex:  p.codex  || 'official',
          copilot: p.copilot || 'official',
          'cursor-agent': p['cursor-agent'] || 'official',
          common: p.common || 'official',
        };
      }
    } catch (e) {
      console.warn('approval profiles load failed', e);
    }
    try {
      const res = await fetch(`/api/approval-patterns?token=${token}`);
      if (res.ok) {
        const data = await res.json();
        const norm = arr => (Array.isArray(arr) ? arr : []).map(s => String(s).toLowerCase()).filter(Boolean);
        providerApprovalTriggers.claude = norm(data.claude);
        providerApprovalTriggers.codex  = norm(data.codex);
        providerApprovalTriggers.copilot = norm(data.copilot);
        providerApprovalTriggers['cursor-agent'] = norm(data['cursor-agent']);
        providerApprovalTriggers.common = norm(data.common);
      }
    } catch (e) {
      console.warn('approval patterns load failed', e);
    }
  }

  async function fetchProfileList(provider, profile) {
    try {
      const res = await fetch(`approval-patterns/${encodeURIComponent(provider)}.${encodeURIComponent(profile)}.json?token=${encodeURIComponent(token || '')}`);
      if (!res.ok) return [];
      return await res.json();
    } catch (_) {
      return [];
    }
  }

  async function loadProvider(provider) {
    const [official, custom] = await Promise.all([
      fetchProfileList(provider, 'official'),
      fetchProfileList(provider, 'custom'),
    ]);
    cache[provider].official = Array.isArray(official) ? official : [];
    cache[provider].custom = Array.isArray(custom) ? custom : [];
  }

  async function loadAll() {
    try {
      await loadActive();
      await Promise.all(Object.keys(cache).map(loadProvider));
      profileEl.value = activeProfiles[currentProvider()] || 'official';
      render();
    } catch (e) {
      console.warn('approval patterns init failed', e);
      showToast(t('settings_approval_patterns_load_failed'));
    }
  }

  function render() {
    const provider = currentProvider();
    const profile = currentProfile();
    const list = (cache[provider] && cache[provider][profile]) || [];
    const readonly = isReadonly();

    listEl.innerHTML = '';
    listEl.classList.toggle('is-readonly', readonly);
    for (let i = 0; i < list.length; i++) {
      const li = document.createElement('li');
      const span = document.createElement('span');
      span.className = 'pattern-text';
      span.textContent = list[i];
      li.appendChild(span);
      if (!readonly) {
        const rm = document.createElement('button');
        rm.className = 'pattern-remove';
        rm.textContent = '✕';
        rm.title = t('settings_approval_patterns_remove');
        rm.addEventListener('click', () => removeAt(i));
        li.appendChild(rm);
      }
      listEl.appendChild(li);
    }

    inputEl.disabled = readonly;
    addBtn.disabled = readonly;
    if (readonlyNote) readonlyNote.classList.toggle('is-visible', readonly);
    copyOfficialBtn.style.display = readonly ? 'none' : '';
  }

  async function saveCustom(provider) {
    try {
      const res = await fetch(`/api/approval-patterns/${encodeURIComponent(provider)}?token=${encodeURIComponent(token || '')}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(cache[provider].custom),
      });
      if (!res.ok) throw new Error('http ' + res.status);
      if (activeProfiles[provider] === 'custom') {
        const norm = arr => arr.map(s => String(s).toLowerCase()).filter(Boolean);
        providerApprovalTriggers[provider] = norm(cache[provider].custom);
      }
      showToast(t('settings_approval_patterns_saved'));
    } catch (e) {
      console.warn('approval patterns save failed', e);
      showToast(t('settings_approval_patterns_save_failed'));
      await loadAll();
    }
  }

  function addPattern() {
    if (isReadonly()) return;
    const text = inputEl.value.trim();
    if (!text) return;
    const provider = currentProvider();
    if (cache[provider].custom.includes(text)) {
      inputEl.value = '';
      return;
    }
    cache[provider].custom = [...cache[provider].custom, text];
    inputEl.value = '';
    render();
    saveCustom(provider);
  }

  function removeAt(idx) {
    if (isReadonly()) return;
    const provider = currentProvider();
    cache[provider].custom = cache[provider].custom.filter((_, i) => i !== idx);
    render();
    saveCustom(provider);
  }

  async function switchProfile() {
    const provider = currentProvider();
    const profile = currentProfile();
    if (activeProfiles[provider] === profile) {
      render();
      return;
    }
    try {
      const res = await fetch(`/api/approval-patterns/profile?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ provider, profile }),
      });
      if (!res.ok) throw new Error('http ' + res.status);
      activeProfiles[provider] = profile;
      const list = (cache[provider] && cache[provider][profile]) || [];
      const norm = arr => arr.map(s => String(s).toLowerCase()).filter(Boolean);
      providerApprovalTriggers[provider] = norm(list);
      render();
    } catch (e) {
      console.warn('approval profile switch failed', e);
      showToast(t('settings_approval_patterns_save_failed'));
      profileEl.value = activeProfiles[provider] || 'official';
    }
  }

  async function copyFromOfficial() {
    if (isReadonly()) return;
    const provider = currentProvider();
    try {
      const res = await fetch(`/api/approval-patterns/copy-official?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ provider }),
      });
      if (!res.ok) throw new Error('http ' + res.status);
      await loadProvider(provider);
      render();
      showToast(t('settings_approval_patterns_saved'));
    } catch (e) {
      console.warn('approval copy-official failed', e);
      showToast(t('settings_approval_patterns_save_failed'));
    }
  }

  providerEl.addEventListener('change', () => {
    profileEl.value = activeProfiles[currentProvider()] || 'official';
    render();
  });
  profileEl.addEventListener('change', switchProfile);
  addBtn.addEventListener('click', addPattern);
  inputEl.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); addPattern(); } });
  copyOfficialBtn.addEventListener('click', copyFromOfficial);

  loadAll();

  return {
    async onOfficialUpdated(providers) {
      if (!Array.isArray(providers) || providers.length === 0) return;
      await Promise.all(providers.map(p => cache[p] ? loadProvider(p) : Promise.resolve()));
      for (const p of providers) {
        if (activeProfiles[p] === 'official') {
          const norm = arr => arr.map(s => String(s).toLowerCase()).filter(Boolean);
          providerApprovalTriggers[p] = norm(cache[p].official);
        }
      }
      render();
    },
  };
})();

// ---- スラッシュコマンドソース設定 ----
export async function loadSlashCmdSources() {
  const claudeEl = document.getElementById('slash-src-claude');
  const codexEl  = document.getElementById('slash-src-codex');
  const copilotEl = document.getElementById('slash-src-copilot');
  const cursorAgentEl = document.getElementById('slash-src-cursor-agent');
  if (!claudeEl || !codexEl || !copilotEl) return;
  try {
    const resp = await fetch(`/api/slash-cmd-sources?token=${token}`);
    if (!resp.ok) return;
    const data = await resp.json();
    claudeEl.value = data.claude || '';
    codexEl.value  = data.codex  || '';
    copilotEl.value = data.copilot || '';
    if (cursorAgentEl) cursorAgentEl.value = data['cursor-agent'] || '';
  } catch (_) {}
}

(function () {
  const saveBtn = document.getElementById('slash-src-save-btn');
  if (!saveBtn) return;
  saveBtn.addEventListener('click', async () => {
    const body = {
      claude: (document.getElementById('slash-src-claude')?.value || '').trim(),
      codex:  (document.getElementById('slash-src-codex')?.value  || '').trim(),
      copilot: (document.getElementById('slash-src-copilot')?.value || '').trim(),
      'cursor-agent': (document.getElementById('slash-src-cursor-agent')?.value || '').trim(),
    };
    try {
      const resp = await fetch(`/api/slash-cmd-sources?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (resp.ok) showToast(t('settings_slash_src_saved'), saveBtn);
    } catch (_) {}
  });
})();

// moved to /app/files-view.js

// ─── C2: 統合タブバー (setActiveTab) ───────────────────────────────────
// セッション毎の表示モード (D13: in-memory, リロードで初期化)
export const sessionViewMode = new Map(); // sid -> 'terminal' | 'chat' | 'split' | 'files' | 'git' | 'workbench'
// Files/Git の遅延ロード状態 (sid -> Set<'files'|'git'>)
export const sessionLazyLoaded = new Map();

export const VALID_TAB_NAMES = new Set(['terminal', 'chat', 'split', 'files', 'git', 'workbench', 'multi', 'approval']);
// C5: lock の対象モード (Files/Git は lock 対象外: D10 の lazy 読み込みと相性が悪い)
export const LOCKABLE_MODES = new Set(['terminal', 'chat', 'split']);
export const RESPONSIVE_WIDE_MODE_MIN = 1001;

export function normalizeResponsiveTabName(name) {
  if ((name === 'split' || name === 'multi') && window.innerWidth < RESPONSIVE_WIDE_MODE_MIN) {
    return 'terminal';
  }
  return name;
}

// C5: 「表示モードを固定」設定値の取得 ('' / 'terminal' / 'chat' / 'split')
export function getDisplayLockedMode() {
  try {
    const raw = localStorage.getItem(STORAGE_DISPLAY_LOCKED_MODE_KEY);
    if (!raw) return '';
    if (LOCKABLE_MODES.has(raw)) return raw;
  } catch (_) {}
  return '';
}

export function getSessionViewMode(sid) {
  if (sid === null || sid === undefined) return 'terminal';
  // C5: セッション未登録 (新規 spawn / リロード後の初回) は lock 値を初期モードとして適用
  if (!sessionViewMode.has(sid)) {
    const lock = getDisplayLockedMode();
    if (lock && LOCKABLE_MODES.has(lock)) return lock;
    return 'terminal';
  }
  return sessionViewMode.get(sid) || 'terminal';
}

export function isTabLazyLoaded(sid, name) {
  const set = sessionLazyLoaded.get(sid);
  return !!(set && set.has(name));
}

export function markTabLazyLoaded(sid, name) {
  if (!sessionLazyLoaded.has(sid)) sessionLazyLoaded.set(sid, new Set());
  sessionLazyLoaded.get(sid).add(name);
}

// C5: lock 値に対応するタブにのみ .locked-mode を付与 (🔒 表示)
export function refreshLockedModeTabClasses() {
  const lock = getDisplayLockedMode();
  document.querySelectorAll('#unified-tab-bar .view-tab').forEach(btn => {
    const isLocked = !!lock && btn.dataset.tab === lock;
    btn.classList.toggle('locked-mode', isLocked);
  });
}

// C5: lock 中にユーザーが別タブへ切替えた時、セッションごとに 5 分クールダウン付きでトースト
export const _lockedModeToastLastTs = new Map(); // sid -> ts(ms)
export const LOCKED_MODE_TOAST_COOLDOWN_MS = 5 * 60 * 1000;

export function maybeFireLockedModeToast(sid, requestedMode) {
  const lock = getDisplayLockedMode();
  if (!lock) return;
  if (!LOCKABLE_MODES.has(requestedMode)) return;
  if (requestedMode === lock) return;
  if (sid === null || sid === undefined) return;
  const now = Date.now();
  const last = _lockedModeToastLastTs.get(sid) || 0;
  if (now - last < LOCKED_MODE_TOAST_COOLDOWN_MS) return;
  _lockedModeToastLastTs.set(sid, now);
  const tfn = (typeof window.t === 'function') ? window.t : ((k) => k);
  const modeLabel = tfn('settings_locked_mode_' + lock);
  showToast(tfn('toast_locked_mode_switched', { mode: modeLabel }));
}

// Files/Git のうち、現セッションでまだ未取得のものは .lazy クラスを付け直す
export function refreshLazyTabClasses(sid) {
  const filesBtn = document.querySelector('#unified-tab-bar .view-tab[data-tab="files"]');
  const gitBtn   = document.querySelector('#unified-tab-bar .view-tab[data-tab="git"]');
  if (filesBtn) {
    const loaded = isTabLazyLoaded(sid, 'files');
    filesBtn.classList.toggle('lazy', !loaded);
    filesBtn.classList.toggle('loaded', loaded);
  }
  if (gitBtn) {
    const loaded = isTabLazyLoaded(sid, 'git');
    gitBtn.classList.toggle('lazy', !loaded);
    gitBtn.classList.toggle('loaded', loaded);
  }
}

// D11: タブバー左端のセッション情報チップを描画 (セッションカード相当)
export function renderSessionInfoChip() {
  const chip = document.getElementById('session-info-chip');
  if (!chip) return;
  const sid = activeSessionId;
  const s = (sid !== null && sid !== undefined) ? sessions.get(sid) : null;
  if (!s) {
    chip.hidden = true;
    chip.innerHTML = '';
    return;
  }
  chip.hidden = false;
  const providerName = providerDisplayName(s.provider);
  const providerChipHtml = providerName
    ? `<span class="card-provider-chip ${safeClassToken(s.provider)}">${escapeHtml(providerName)}</span>`
    : '';
  const isOllamaBackedSess = (s.route === 'ollama');
  let modelBadge = '';
  if (s.model) {
    const badgeProviderKey = isOllamaBackedSess ? 'ollama' : (s.provider || '');
    const badgeProviderLabel = isOllamaBackedSess ? 'Ollama' : providerName;
    const tip = badgeProviderLabel ? `${badgeProviderLabel} · ${s.model}` : s.model;
    modelBadge = ` <span class="card-model card-model--with-icon" data-tooltip="${escapeHtml(tip)}">${providerIconHtml(badgeProviderKey)}<span class="card-model-text">${escapeHtml(s.model)}</span></span>`;
  }
  const state = s.state || 'standby';
  const stateLbl = (typeof stateLabel === 'function') ? stateLabel(state) : state;
  // 状態 pill はステータスバー（token-statusbar）と表示順・フォント・色を揃える:
  //   #N → 状態pill(●ドット付き) → providerアイコン+チップ+モデル
  // disconnected はステータスバーと同様に error 配色へ寄せる。
  const pillCls = (state === 'running') ? 'running'
    : (state === 'waiting') ? 'waiting'
    : (state === 'error' || state === 'disconnected') ? 'error'
    : 'standby';
  const statePill = `<span class="tsb-pill ${pillCls}"><span class="tsb-pdot"></span>${escapeHtml(stateLbl)}</span>`;
  chip.innerHTML =
    `<span class="sid">#${s.id}</span>` +
    ` ${statePill} ` +
    `${providerIconHtml(s.provider)} ${providerChipHtml}${modelBadge}`;
}

// D12: チャット件数バッジ更新
export function updateChatCountBadge() {
  const badge = document.getElementById('chat-count-badge');
  if (!badge) return;
  const sid = activeSessionId;
  let n = 0;
  if (sid !== null && sid !== undefined) {
    try {
      const msgs = (typeof getMessages === 'function') ? getMessages(sid) : [];
      n = Array.isArray(msgs) ? msgs.length : 0;
    } catch (_) {}
  }
  badge.textContent = String(n);
  badge.hidden = (n === 0);
}

// C2 公開 API: タブを切り替える
export let _setActiveTabRecursion = false;
export function setActiveTab(sid, name) {
  if (!VALID_TAB_NAMES.has(name)) return;
  name = normalizeResponsiveTabName(name);

  // マルチタブはセッション非依存のビュー: セッションなしでも動作させる
  if (name === 'multi') {
    const area = document.getElementById('display-area');
    if (!area) return;
    const multiView = document.getElementById('multi-view');
    const mgr = window.multiPaneManager;
    area.hidden = true;
    if (multiView) multiView.hidden = false;
    // scrollback を縮小（全セッション）
    const scrollbackMulti = MULTI_SCROLLBACK();
    sessions.forEach(s => {
      const t = terminals.get(s.id);
      if (t && t.term) { try { t.term.options.scrollback = scrollbackMulti; } catch (_) {} }
    });
    // render() → attachToSlot() で xterm をペインにアタッチ
    if (mgr) mgr.render();
    document.querySelectorAll('#unified-tab-bar .view-tab').forEach(b => {
      b.classList.toggle('active', b.dataset.tab === 'multi');
    });
    if (mgr && mgr.picker) mgr.picker.toggle();
    // C4: 最後にフォーカスしていたスロットを復元 → activeSessionId を同期
    if (mgr) {
      const restoreIdx = (mgr.focusedIdx >= 0 && mgr.focusedIdx < mgr.slots.length)
        ? mgr.focusedIdx : 0;
      mgr.focusSlot(restoreIdx);
    }
    if (typeof refreshLockedModeTabClasses === 'function') refreshLockedModeTabClasses();
    // C3: "Detach current grid" ボタンをタブバーに挿入（初回のみ生成）
    _ensureMultiDetachBtn();
    return;
  }

  if (name === 'workbench') {
    const area = document.getElementById('display-area');
    if (!area) return;
    const multiView = document.getElementById('multi-view');
    const mgr = window.multiPaneManager;
    const prevMultiOpen = (multiView && !multiView.hidden);
    if (multiView) multiView.hidden = true;
    if (mgr && mgr.picker) mgr.picker.hide();
    if (prevMultiOpen && mgr) {
      mgr.teardown();
      sessions.forEach(s => {
        const t = terminals.get(s.id);
        if (t && t.term) {
          try { t.term.options.scrollback = TERMINAL_SCROLLBACK_LINES; } catch (_) {}
        }
      });
      if (activeSessionId !== null && activeSessionId !== undefined) {
        attachTerminal(activeSessionId);
      }
    }
    area.hidden = false;
    area.classList.remove('mode-terminal', 'mode-chat', 'mode-split', 'mode-files', 'mode-git', 'mode-workbench', 'mode-approval');
    area.classList.add('mode-workbench');
    document.querySelectorAll('#unified-tab-bar .view-tab').forEach(b => {
      b.classList.toggle('active', b.dataset.tab === 'workbench');
    });
    if (activeSessionId !== null && activeSessionId !== undefined) {
      sessionViewMode.set(activeSessionId, name);
    }
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent('workbench-opened'));
    }
    return;
  }

  // 承認タブ: セッション非依存のビュー（workbench と同様の扱い）
  if (name === 'approval') {
    const area = document.getElementById('display-area');
    if (!area) return;
    const multiView = document.getElementById('multi-view');
    const mgr = window.multiPaneManager;
    const prevMultiOpen = (multiView && !multiView.hidden);
    if (multiView) multiView.hidden = true;
    if (mgr && mgr.picker) mgr.picker.hide();
    if (prevMultiOpen && mgr) {
      mgr.teardown();
      sessions.forEach(s => {
        const t = terminals.get(s.id);
        if (t && t.term) {
          try { t.term.options.scrollback = TERMINAL_SCROLLBACK_LINES; } catch (_) {}
        }
      });
      if (activeSessionId !== null && activeSessionId !== undefined) {
        attachTerminal(activeSessionId);
      }
    }
    area.hidden = false;
    area.classList.remove('mode-terminal', 'mode-chat', 'mode-split', 'mode-files', 'mode-git', 'mode-workbench', 'mode-approval');
    area.classList.add('mode-approval');
    document.querySelectorAll('#unified-tab-bar .view-tab').forEach(b => {
      b.classList.toggle('active', b.dataset.tab === 'approval');
    });
    if (typeof refreshLockedModeTabClasses === 'function') refreshLockedModeTabClasses();
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent('session-view-mode-changed', {
        detail: { sid: activeSessionId, name },
      }));
    }
    return;
  }

  const targetSid = (sid !== null && sid !== undefined) ? sid : activeSessionId;
  if (targetSid === null || targetSid === undefined) return;

  // セッション毎モードを保存 (D13)
  sessionViewMode.set(targetSid, name);

  // 現セッションへの切替でなければ DOM 反映は不要 (アクティブ化時に反映される)
  if (targetSid !== activeSessionId) return;

  const area = document.getElementById('display-area');
  if (!area) return;
  const multiView = document.getElementById('multi-view');
  const mgr = window.multiPaneManager;

  // ── 他タブへ切替: マルチビューを閉じる ──
  const prevMultiOpen = (multiView && !multiView.hidden);
  if (multiView) multiView.hidden = true;
  if (mgr && mgr.picker) mgr.picker.hide();
  // C3: マルチを離れたとき全スロットを detach し、アクティブセッションを #terminal-area に戻す
  if (prevMultiOpen) {
    // 全スロットを detach（xterm の container を宙ぶらりんに）
    if (mgr) mgr.teardown();
    // scrollback を標準値に戻す
    sessions.forEach(s => {
      const t = terminals.get(s.id);
      if (t && t.term) {
        try { t.term.options.scrollback = TERMINAL_SCROLLBACK_LINES; } catch (_) {}
      }
    });
    // アクティブセッションの xterm を #terminal-area に再アタッチ
    if (activeSessionId !== null && activeSessionId !== undefined) {
      attachTerminal(activeSessionId);
    }
  }
  area.hidden = false;

  area.classList.remove('mode-terminal', 'mode-chat', 'mode-split', 'mode-files', 'mode-git', 'mode-workbench', 'mode-approval');
  area.classList.add('mode-' + name);

  // タブボタンの active 切替
  document.querySelectorAll('#unified-tab-bar .view-tab').forEach(b => {
    b.classList.toggle('active', b.dataset.tab === name);
  });

  // D10: Files/Git は初回クリックで FilesTabManager に開かせる
  // FilesTabManager.setActive が再帰的に setActiveTab を呼ぶため再帰防止フラグで守る
  if ((name === 'files' || name === 'git') && !_setActiveTabRecursion) {
    _setActiveTabRecursion = true;
    try { handleLazyTabOpen(targetSid, name); }
    finally { _setActiveTabRecursion = false; }
  }

  // xterm のリサイズ (D6): terminal/split に切り替えたときは refit
  if (name === 'terminal' || name === 'split') {
    if (typeof refitActiveTerminalAfterLayout === 'function') {
      refitActiveTerminalAfterLayout(true);
    }
  }

  // FilesTabManager は内部状態として terminalWrapper.style.display を弄っていたため、
  // 統合バー導入後はインラインスタイルを除去して CSS (display-area mode) に任せる
  const termWrap = document.getElementById('terminal-wrapper');
  if (termWrap) termWrap.style.display = '';

  // C5: lock 値に対応するタブに 🔒 を付け直す (DOM 更新後)
  if (typeof refreshLockedModeTabClasses === 'function') refreshLockedModeTabClasses();

  // bugfix 2026-06-04: DOM mode 確定後に view mode change event を発火する。
  // chat-history.js はこれを購読して chat/split 切替時の chat-pane mount を保証する。
  // ESM import binding は window.setActiveTab の上書きでは差し替わらないため、
  // monkey patch ではなく event で副作用を流す（タブクリック / applyActiveSessionViewMode
  // など全呼び出し経路がここを通る）。
  if (typeof window !== 'undefined') {
    window.dispatchEvent(new CustomEvent('session-view-mode-changed', {
      detail: { sid: targetSid, name },
    }));
  }
}

// D10: Files/Git タブを初回クリックで開く (および既ロード時の再アクティブ化)
// openFilesTab / openGitTab は idempotent (既存タブがあれば再利用) なので、
// セッション切替で .active が外れた files/git pane の再表示にも兼用する。
export function handleLazyTabOpen(sid, name) {
  const sess = sessions.get(sid);
  if (!sess) return;
  const gr = sess.git_root || sess.cwd || '';
  if (!gr) {
    // git_root / cwd が無いので開けない。lazy のまま
    return;
  }
  try {
    if (name === 'files') {
      const pk = sess.project || deriveProjectKeyFromCwd(gr);
      FilesTabManager.openFilesTab(sid, pk, gr, gr);
    } else if (name === 'git') {
      FilesTabManager.openGitTab(sid, gr, sess.branch || '');
    }
    markTabLazyLoaded(sid, name);
    refreshLazyTabClasses(sid);
  } catch (e) {
    console.warn('[setActiveTab] lazy open failed:', name, e);
  }
}

// 既存 switchToSessionView() (FilesTabManager 内) は #main-tab-bar の DOM 操作を行う。
// 統合バー導入後は setActiveTab(sid, 'terminal') への薄いラッパーとして使う。
export function switchToTerminalView() {
  if (activeSessionId !== null && activeSessionId !== undefined) {
    setActiveTab(activeSessionId, 'terminal');
  }
}

// ─── タブボタンのクリックハンドラ配線 ───
(function wireUnifiedTabBar() {
  const bar = document.getElementById('unified-tab-bar');
  if (!bar) return;
  bar.querySelectorAll('.view-tab').forEach(btn => {
    btn.addEventListener('click', () => {
      const name = btn.dataset.tab;
      if (!VALID_TAB_NAMES.has(name)) return;
      setActiveTab(activeSessionId, name);
      // C5: lock 中に lock 値以外へ切替えたら、セッションごと 5 分クールダウンでトースト
      if (typeof maybeFireLockedModeToast === 'function') {
        maybeFireLockedModeToast(activeSessionId, name);
      }
    });
  });
  // C5: 初期 🔒 反映 (起動時に lock 値が設定済みなら該当タブへ付与)
  if (typeof refreshLockedModeTabClasses === 'function') refreshLockedModeTabClasses();
})();

// 新規セッション/アクティブ切替時に display-area のモードを復元
export function applyActiveSessionViewMode() {
  if (activeSessionId === null || activeSessionId === undefined) return;
  const mode = normalizeResponsiveTabName(getSessionViewMode(activeSessionId));
  refreshLazyTabClasses(activeSessionId);
  setActiveTab(activeSessionId, mode);
}

window.addEventListener('resize', () => {
  if (window.innerWidth >= RESPONSIVE_WIDE_MODE_MIN) return;
  const multiView = document.getElementById('multi-view');
  const area = document.getElementById('display-area');
  if ((multiView && !multiView.hidden) || (area && area.classList.contains('mode-split'))) {
    setActiveTab(activeSessionId, 'terminal');
  }
});


// ─── C3: Multi tab "Detach current grid" ボタン ───────────────────────────
/**
 * Multi タブが開いているとき、unified-tab-bar に "Detach current grid" ボタンを追加する。
 * 初回呼び出しでボタンを生成し、以降は既存要素を再利用する。
 */
function _ensureMultiDetachBtn(): void {
  const bar = document.getElementById('unified-tab-bar');
  if (!bar) return;
  let btn = document.getElementById('multi-detach-btn');
  if (btn) return; // 既存
  btn = document.createElement('button');
  btn.id = 'multi-detach-btn';
  btn.className = 'multi-detach-btn view-tab-util';
  btn.type = 'button';
  btn.textContent = '⊞↗';
  btn.title = ti18n('multi_detach_current_grid', 'Detach current grid');
  btn.setAttribute('aria-label', ti18n('multi_detach_current_grid', 'Detach current grid'));
  btn.addEventListener('click', () => {
    const mgr = window.multiPaneManager;
    if (!mgr) return;
    // 現在 Multi にマウントされているセッション id を収集する
    const slotIds: number[] = mgr.slots
      .filter((s: any) => s && s.session && s.session.id != null)
      .map((s: any) => s.session.id);
    if (slotIds.length === 0) return;
    openDetachedGridForSessions(slotIds);
  });
  // タブバー末尾に追加
  bar.appendChild(btn);
}

// ─── カード右クリックメニュー (Open Git / Files / Activate / Copy ID) ───
export let _cardCtxMenuEl = null;
export let _cardCtxSid    = null;
export function openCardCtxMenu(x, y, sid) {
  closeCardCtxMenu();
  _cardCtxSid = sid;
  const menu = document.createElement('div');
  menu.className = 'card-ctx-menu open';
  menu.id = 'card-ctx-menu';
  const labelOpenGit        = ti18n('ctx_open_git',              'Open Git View');
  const labelOpenFiles      = ti18n('ctx_open_files',            'Open Files Tab');
  const labelActivate       = ti18n('ctx_activate',              'Activate Session');
  const labelCopyId         = ti18n('ctx_copy_id',               'Copy session ID');
  const labelOpenInGrid     = ti18n('ctx_open_in_grid',          'Open in detached grid');
  const labelOpenProjectGrid = ti18n('ctx_open_project_in_grid', 'Open project in detached grid');
  menu.innerHTML =
    `<button type="button" data-action="open-git"><span class="ico">⎇</span><span>${escapeHtml(labelOpenGit)}</span><span class="kbd">Ctrl+Shift+G</span></button>` +
    `<button type="button" data-action="open-files"><span class="ico">📁</span><span>${escapeHtml(labelOpenFiles)}</span><span class="kbd">Ctrl+Shift+F</span></button>` +
    `<div class="card-ctx-sep"></div>` +
    `<button type="button" data-action="open-in-grid"><span class="ico">⊞</span><span>${escapeHtml(labelOpenInGrid)}</span></button>` +
    `<button type="button" data-action="open-project-grid"><span class="ico">⊞</span><span>${escapeHtml(labelOpenProjectGrid)}</span></button>` +
    `<div class="card-ctx-sep"></div>` +
    `<button type="button" data-action="activate"><span class="ico">→</span><span>${escapeHtml(labelActivate)}</span></button>` +
    `<button type="button" data-action="copy-id"><span class="ico">#</span><span>${escapeHtml(labelCopyId)}</span></button>`;
  document.body.appendChild(menu);
  const r = menu.getBoundingClientRect();
  const px = Math.min(x, window.innerWidth  - r.width  - 4);
  const py = Math.min(y, window.innerHeight - r.height - 4);
  menu.style.left = Math.max(0, px) + 'px';
  menu.style.top  = Math.max(0, py) + 'px';
  _cardCtxMenuEl = menu;
  menu.querySelectorAll('button').forEach(b => {
    b.addEventListener('click', () => {
      const action = b.dataset.action;
      const id = _cardCtxSid;
      closeCardCtxMenu();
      const sess = sessions.get(id);
      if (!sess) return;
      if (action === 'open-git') {
        const gr = String(sess.git_root || sess.cwd || '');
        if (!gr) return;
        FilesTabManager.openGitTab(id, gr, sess.branch || '');
      } else if (action === 'open-files') {
        const gr = String(sess.git_root || sess.cwd || '');
        const pk = sess.project || (gr ? gr.split(/[\\/]/).filter(Boolean).pop() : '__no_project__');
        if (!gr) return;
        FilesTabManager.openFilesTab(id, pk, gr, gr);
      } else if (action === 'open-in-grid') {
        // C3: このセッション単体を別窓 grid で開く
        openDetachedGridForSessions([id]);
      } else if (action === 'open-project-grid') {
        // C3: 同一 project group の全セッションを別窓 grid で開く
        const projKey = sessionProjectKey(sess);
        const projectSessionIds: number[] = [];
        sessions.forEach((s, sid2) => {
          if (sessionProjectKey(s) === projKey) projectSessionIds.push(sid2);
        });
        openDetachedGridForSessions(projectSessionIds);
      } else if (action === 'activate') {
        activateSession(id);
        FilesTabManager.switchToSessionView();
      } else if (action === 'copy-id') {
        try { navigator.clipboard && navigator.clipboard.writeText(String(id)); } catch (_) {}
      }
    });
  });
}
export function closeCardCtxMenu() {
  if (_cardCtxMenuEl) { try { _cardCtxMenuEl.remove(); } catch (_) {} _cardCtxMenuEl = null; }
  _cardCtxSid = null;
}
document.addEventListener('mousedown', (e) => {
  if (_cardCtxMenuEl && !e.target.closest('#card-ctx-menu')) {
    closeCardCtxMenu();
  }
});
window.addEventListener('blur', () => closeCardCtxMenu());
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && _cardCtxMenuEl) closeCardCtxMenu();
});
document.addEventListener('scroll', () => closeCardCtxMenu(), true);

// ─── Ctrl+Shift+G / Ctrl+Shift+F グローバルショートカット ──────────────
document.addEventListener('keydown', (e) => {
  // IME 中や input/textarea にフォーカス中は素通しはしない（Ctrl+Shift 系は通常衝突しないため発火可）
  if (!e.ctrlKey || !e.shiftKey || e.altKey || e.metaKey) return;
  const k = e.key;
  if (k === 'G' || k === 'g') {
    const sid = activeSessionId;
    if (sid == null) return;
    const sess = sessions.get(sid);
    if (!sess) return;
    const gr = String(sess.git_root || sess.cwd || '');
    if (!gr) return;
    e.preventDefault();
    e.stopPropagation();
    FilesTabManager.openGitTab(sid, gr, sess.branch || '');
  } else if (k === 'F' || k === 'f') {
    const sid = activeSessionId;
    if (sid == null) return;
    const sess = sessions.get(sid);
    if (!sess) return;
    const gr = String(sess.git_root || sess.cwd || '');
    if (!gr) return;
    const pk = sess.project || (gr ? gr.split(/[\\/]/).filter(Boolean).pop() : '__no_project__');
    e.preventDefault();
    e.stopPropagation();
    FilesTabManager.openFilesTab(sid, pk, gr, gr);
  }
});

// ─── タブ状態変化でカード再描画 (open マーカー反映) ────────────────────
window.addEventListener('files-tab-state-changed', () => {
  try {
    if (typeof renderSessionList === 'function') renderSessionList();
  } catch (_) {}
});


// ---- ntfy / webhook 通知設定 ----
(function () {
  const toggleBtn = document.getElementById('ntfy-settings-toggle-btn');
  const block = document.getElementById('ntfy-settings-block') as HTMLDivElement | null;
  const backendsList = document.getElementById('ntfy-backends-list');
  const addNtfyBtn = document.getElementById('ntfy-add-ntfy-btn');
  const addWebhookBtn = document.getElementById('ntfy-add-webhook-btn');
  const eventApprovalEl = document.getElementById('ntfy-event-approval') as HTMLInputElement | null;
  const eventDoneEl = document.getElementById('ntfy-event-done') as HTMLInputElement | null;
  const saveBtn = document.getElementById('ntfy-save-btn');
  const saveStatus = document.getElementById('ntfy-save-status');
  if (!toggleBtn || !block || !backendsList || !saveBtn) return;

  type Backend = { type: string; url: string; topic: string };
  let backends: Backend[] = [];

  toggleBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    block.hidden = !block.hidden;
    if (!block.hidden) loadNotifyConfig();
  });

  async function loadNotifyConfig() {
    try {
      const res = await fetch(`/api/notify-config?token=${encodeURIComponent(token || '')}`);
      if (!res.ok) return;
      const data = await res.json();
      backends = Array.isArray(data.backends) ? data.backends.map((b: any) => ({
        type: b.type || 'ntfy',
        url: b.url || '',
        topic: b.topic || '',
      })) : [];
      if (eventApprovalEl || eventDoneEl) {
        const events: string[] = Array.isArray(data.events) ? data.events : [];
        if (eventApprovalEl) {
          eventApprovalEl.checked = events.length === 0 || events.some((e: string) => e === 'approval');
        }
        if (eventDoneEl) {
          eventDoneEl.checked = events.some((e: string) => e === 'done');
        }
      }
      renderBackends();
    } catch (_) {}
  }

  function renderBackends() {
    backendsList.innerHTML = '';
    backends.forEach((b, i) => {
      const row = document.createElement('div');
      row.className = 'ntfy-backend-row';
      row.style.cssText = 'display:flex;flex-direction:column;gap:4px;border:1px solid var(--border-color,#ccc);border-radius:4px;padding:6px 8px;margin-bottom:6px';

      const header = document.createElement('div');
      header.style.cssText = 'display:flex;align-items:center;gap:6px;';
      const typeBadge = document.createElement('span');
      typeBadge.textContent = b.type.toUpperCase();
      typeBadge.style.cssText = 'font-size:0.75em;background:var(--accent-bg,#e0e0e0);padding:1px 6px;border-radius:3px;';
      const removeBtn = document.createElement('button');
      removeBtn.className = 'settings-link-btn';
      removeBtn.textContent = '✕';
      removeBtn.style.cssText = 'margin-left:auto;color:var(--danger-color,#c00);';
      removeBtn.title = t('settings_ntfy_remove_backend') || 'Remove';
      removeBtn.addEventListener('click', () => { backends.splice(i, 1); renderBackends(); });
      header.appendChild(typeBadge);
      header.appendChild(removeBtn);

      const urlRow = document.createElement('div');
      urlRow.style.cssText = 'display:flex;align-items:center;gap:6px;';
      const urlLabel = document.createElement('span');
      urlLabel.textContent = 'URL';
      urlLabel.style.cssText = 'font-size:0.82em;min-width:52px;';
      const urlInput = document.createElement('input');
      urlInput.type = 'url';
      urlInput.value = b.url;
      urlInput.placeholder = b.type === 'ntfy' ? 'https://ntfy.sh' : 'https://...';
      urlInput.className = 'settings-input-url';
      urlInput.style.cssText = 'flex:1;font-size:0.85em;';
      urlInput.addEventListener('input', () => { backends[i].url = urlInput.value; });
      urlRow.appendChild(urlLabel);
      urlRow.appendChild(urlInput);

      row.appendChild(header);
      row.appendChild(urlRow);

      if (b.type === 'ntfy') {
        const topicRow = document.createElement('div');
        topicRow.style.cssText = 'display:flex;align-items:center;gap:6px;';
        const topicLabel = document.createElement('span');
        topicLabel.textContent = t('settings_ntfy_topic') || 'Topic';
        topicLabel.style.cssText = 'font-size:0.82em;min-width:52px;';
        const topicInput = document.createElement('input');
        topicInput.type = 'text';
        topicInput.value = b.topic || '';
        topicInput.placeholder = 'anyaicli-xxxx';
        topicInput.className = 'settings-input-url';
        topicInput.style.cssText = 'flex:1;font-size:0.85em;';
        topicInput.addEventListener('input', () => { backends[i].topic = topicInput.value; });
        const genBtn = document.createElement('button');
        genBtn.className = 'settings-link-btn';
        genBtn.textContent = t('settings_ntfy_generate_topic') || '自動生成';
        genBtn.addEventListener('click', async () => {
          try {
            const res = await fetch(`/api/notify-generate-topic?token=${encodeURIComponent(token || '')}`, { method: 'POST' });
            if (!res.ok) return;
            const data = await res.json();
            backends[i].topic = data.topic || '';
            topicInput.value = backends[i].topic;
          } catch (_) {}
        });
        topicRow.appendChild(topicLabel);
        topicRow.appendChild(topicInput);
        topicRow.appendChild(genBtn);
        row.appendChild(topicRow);
      }

      const testRow = document.createElement('div');
      testRow.style.cssText = 'display:flex;align-items:center;gap:6px;margin-top:2px;';
      const testBtn = document.createElement('button');
      testBtn.className = 'settings-link-btn';
      testBtn.textContent = t('settings_ntfy_test_send') || 'テスト送信';
      testBtn.addEventListener('click', async () => {
        testBtn.disabled = true;
        try {
          const res = await fetch(`/api/notify-test?token=${encodeURIComponent(token || '')}`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ backend: backends[i] }),
          });
          const data = await res.json().catch(() => ({}));
          if (res.ok && data.ok) {
            showToast(t('settings_ntfy_test_ok') || 'テスト送信しました', testBtn);
          } else {
            showToast((data.detail || data.error || t('settings_ntfy_test_failed') || '送信失敗'), testBtn);
          }
        } catch (_) {
          showToast(t('settings_ntfy_test_failed') || '送信失敗', testBtn);
        } finally {
          testBtn.disabled = false;
        }
      });
      testRow.appendChild(testBtn);
      row.appendChild(testRow);

      backendsList.appendChild(row);
    });
  }

  addNtfyBtn?.addEventListener('click', () => {
    backends.push({ type: 'ntfy', url: 'https://ntfy.sh', topic: '' });
    renderBackends();
  });
  addWebhookBtn?.addEventListener('click', () => {
    backends.push({ type: 'webhook', url: '', topic: '' });
    renderBackends();
  });

  saveBtn.addEventListener('click', async () => {
    saveBtn.disabled = true;
    try {
      const events: string[] = [];
      if (eventApprovalEl?.checked) events.push('approval');
      if (eventDoneEl?.checked) events.push('done');
      const res = await fetch(`/api/notify-config?token=${encodeURIComponent(token || '')}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ backends, events }),
      });
      if (res.ok) {
        if (saveStatus) { saveStatus.textContent = t('settings_ntfy_saved') || '保存しました'; setTimeout(() => { if (saveStatus) saveStatus.textContent = ''; }, 2500); }
      } else {
        if (saveStatus) { saveStatus.textContent = t('settings_ntfy_save_failed') || '保存失敗'; }
      }
    } catch (_) {
      if (saveStatus) { saveStatus.textContent = t('settings_ntfy_save_failed') || '保存失敗'; }
    } finally {
      saveBtn.disabled = false;
    }
  });
})();

// ---- ファイルリンク設定 ----
(function () {
  const terminalAppEl     = document.getElementById('settings-terminal-app');
  const terminalBrowseBtn = document.getElementById('settings-terminal-app-browse');
  const terminalEffectiveEl = document.getElementById('settings-terminal-app-effective');
  if (!terminalAppEl) return;

  function renderEffectiveCommand(el, value) {
    if (!el) return;
    el.textContent = value ? `${t('settings_effective_command')}: ${value}` : '';
  }

  async function loadTerminalApp() {
    try {
      const res = await fetch(`/api/terminal-app?token=${token}`);
      if (!res.ok) return;
      const cfg = await res.json();
      terminalAppEl.value = cfg.terminal_app || '';
      renderEffectiveCommand(terminalEffectiveEl, cfg.effective_terminal_app);
    } catch (_) {}
  }

  async function saveTerminalApp() {
    try {
      const res = await fetch(`/api/terminal-app?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ terminal_app: terminalAppEl.value.trim() }),
      });
      if (res.ok) {
        const cfg = await res.json();
        renderEffectiveCommand(terminalEffectiveEl, cfg.effective_terminal_app);
      }
    } catch (_) {}
  }

  if (terminalBrowseBtn) {
    terminalBrowseBtn.addEventListener('click', async () => {
      terminalBrowseBtn.disabled = true;
      try {
        const res = await fetch(`/api/pick-file?filter=exe&token=${token}`, { method: 'POST' });
        if (res.ok) {
          const data = await res.json();
          if (data.ok && data.path) terminalAppEl.value = data.path;
        }
      } catch (_) {}
      finally { terminalBrowseBtn.disabled = false; }
    });
  }

  // __settingsSaveAll フックにチェーンで登録
  const origSave = window.__settingsSaveAll;
  window.__settingsSaveAll = async () => {
    if (origSave) await origSave();
    await saveTerminalApp();
  };

  // 設定パネルを開いたときのロード
  const settingsBtn = document.getElementById('settings-btn');
  if (settingsBtn) {
    settingsBtn.addEventListener('click', () => {
      if (!document.getElementById('settings-panel').hidden) {
        loadTerminalApp();
      }
    });
  }
})();

