// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { escapeHtml, showToast, ti18n, token } from './util.js';
import { DEFAULT_USAGE_LINKS, DEFAULT_VOICE_GRACE_SEC, FONTSIZE_MAP, STORAGE_DESKTOP_NOTIFY_ENABLED_KEY, STORAGE_DISPLAY_LOCKED_MODE_KEY, STORAGE_FONTSIZE_KEY, STORAGE_LANG_KEY, STORAGE_NOTIFY_SOUND_CUSTOM_KEY, STORAGE_NOTIFY_SOUND_ENABLED_KEY, STORAGE_NOTIFY_SOUND_TYPE_KEY, STORAGE_PUSH_NOTIFY_ENABLED_KEY, STORAGE_QUICK_CMD_1_KEY, STORAGE_QUICK_CMD_2_KEY, STORAGE_THEME_KEY, STORAGE_TRIGGER_ENABLED_KEY, STORAGE_TRIGGER_PHRASE_KEY, STORAGE_USAGE_LINK_CLAUDE_KEY, STORAGE_USAGE_LINK_CODEX_KEY, STORAGE_USAGE_LINK_COPILOT_KEY, STORAGE_USAGE_LINK_CURSOR_AGENT_KEY, STORAGE_USAGE_LINK_OLLAMA_KEY, STORAGE_USAGE_LINK_OPENCODE_KEY, STORAGE_VOICE_GRACE_KEY, STORAGE_VOICE_WHISPER_AUTO_SUBMIT_KEY, STORAGE_WAKE_WORD_ENABLED_KEY, STORAGE_WAKE_WORD_PHRASE_KEY, _putUserPrefsNow, _setNestedValue, getDefaultTriggerPhrase, getDefaultWakeWordPhrase, getVoiceEngine, setUserPref, setVoiceEngine } from './user-prefs.js';
import { activeSessionId, deriveProjectKeyFromCwd, maybeAutoSwitchToNextApproval, sessions, terminals } from './state.js';
import { _userAvatarUrl, _userDisplayName, inputEl, set__userAvatarUrl, set__userDisplayName } from '../app.js';
import { activateSession, providerDisplayName, providerIconHtml, render, renderSessionList, safeClassToken, stateLabel } from './session-list.js';
import { pathPopupEl } from './path-links.js';
import { TERMINAL_SCROLLBACK_LINES, attachTerminal, fitTerminalPreservingBottom, refitActiveTerminalAfterLayout, sendResize } from './terminal.js';
import { providerApprovalTriggers } from './approval.js';
import { MULTI_SCROLLBACK, getMessages } from './chat-history.js';
import { FilesTabManager } from './files-view.js';
import { fetchPushStatus, getPushSubscription, isLikelyIOSBrowserTabWithoutStandalone, pushNotificationsSupported, subscribeWebPush, unsubscribeWebPush } from './pwa.js';

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
      tag: `any-ai-cli-approval-${sessionId}`,
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

    document.getElementById('app-confirm-cancel').addEventListener('click', () => close(false));
    document.getElementById('app-confirm-ok').addEventListener('click', () => close(true));
    overlay.addEventListener('click', onOverlayClick);
    document.addEventListener('keydown', onKeyDown);
    document.getElementById('app-confirm-ok').focus();
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

export function appConfirmShutdown() {
  return new Promise((resolve) => {
    const overlay = document.getElementById('model-picker-overlay');
    if (!overlay) { resolve(window.confirm(t('shutdown_confirm')) ? 'shutdown' : null); return; }

    const close = (value) => {
      overlay.removeEventListener('click', onOverlayClick);
      document.removeEventListener('keydown', onKeyDown);
      overlay.hidden = true;
      overlay.innerHTML = '';
      resolve(value);
    };
    const onOverlayClick = (e) => { if (e.target === overlay) close(null); };
    const onKeyDown = (e) => {
      if (e.key === 'Escape') close(null);
      if (e.key === 'Enter') close('shutdown');
    };

    overlay.innerHTML = '';
    overlay.hidden = false;

    const dialog = document.createElement('div');
    dialog.className = 'confirm-dialog confirm-dialog--danger';
    dialog.innerHTML = `
      <div class="confirm-icon" aria-hidden="true">!</div>
      <div class="confirm-body">
        <div class="confirm-title">${escapeHtml(t('shutdown_confirm_title'))}</div>
        <div class="confirm-message">${escapeHtml(t('shutdown_confirm'))}</div>
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

export function stripAnsi(str) {
  return str.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, '').replace(/\x1b[^[]/g, '');
}

export const DEFAULT_QUICK_CMD_1 = '/clear';
export const DEFAULT_QUICK_CMD_2 = '/model';
export const ALLOWED_QUICK_COMMANDS = new Set([
  '/clear', '/model', '/help', '/status', '/usage', '/review', '/compact', '/config',
]);

export function sanitizeQuickCommand(cmd, fallback) {
  return ALLOWED_QUICK_COMMANDS.has(cmd) ? cmd : fallback;
}

export function getQuickCommand(slot) {
  if (slot === 1) {
    const saved = localStorage.getItem(STORAGE_QUICK_CMD_1_KEY) || DEFAULT_QUICK_CMD_1;
    return sanitizeQuickCommand(saved, DEFAULT_QUICK_CMD_1);
  }
  const saved = localStorage.getItem(STORAGE_QUICK_CMD_2_KEY) || DEFAULT_QUICK_CMD_2;
  return sanitizeQuickCommand(saved, DEFAULT_QUICK_CMD_2);
}

export function refreshQuickCommandButtons() {
  const btn1 = document.getElementById('quick-clear-btn');
  const btn2 = document.getElementById('quick-model-btn');
  if (!btn1 || !btn2) return;
  const cmd1 = getQuickCommand(1);
  const cmd2 = getQuickCommand(2);
  btn1.textContent = cmd1;
  btn2.textContent = cmd2;
  btn1.dataset.tooltip = cmd1;
  btn2.dataset.tooltip = cmd2;
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
      'https://github.com/ishizakahiroshi/any-ai-cli/blob/main/' + file,
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
    if (graceRow) graceRow.hidden = engine !== 'browser';
    if (graceDesc) graceDesc.hidden = engine !== 'browser';
    if (diagnosticsPanel) diagnosticsPanel.hidden = engine !== 'browser';
    if (whisperAutoSubmitEl) {
      whisperAutoSubmitEl.checked = localStorage.getItem(STORAGE_VOICE_WHISPER_AUTO_SUBMIT_KEY) === '1';
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
      whisperModelSelect.value = data.model || current;
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

  whisperAutoSubmitEl?.addEventListener('change', () => {
    localStorage.setItem(STORAGE_VOICE_WHISPER_AUTO_SUBMIT_KEY, whisperAutoSubmitEl.checked ? '1' : '0');
  });
  whisperInstallBtn?.addEventListener('click', async () => {
    try {
      await postWhisperAction('install', { model: whisperModelSelect?.value || lastWhisperStatus?.model || 'large-v3-turbo-q5_0' });
      showToast(t('settings_voice_whisper_action_done'), whisperInstallBtn);
    } catch (err) {
      showToast(String(err?.message || err), whisperInstallBtn);
    }
  });
  whisperStartBtn?.addEventListener('click', async () => {
    try {
      await postWhisperAction('start');
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
  try {
    if (localStorage.getItem(STORAGE_VOICE_GRACE_KEY) === '2') {
      localStorage.removeItem(STORAGE_VOICE_GRACE_KEY);
    }
  } catch (_) {}
  const saved = localStorage.getItem(STORAGE_VOICE_GRACE_KEY);
  const v = saved == null ? DEFAULT_VOICE_GRACE_SEC : parseInt(saved, 10);
  const clamped = Number.isFinite(v) ? Math.max(0, Math.min(5, v)) : DEFAULT_VOICE_GRACE_SEC;
  sel.value = String(clamped);
  sel.addEventListener('change', () => {
    setUserPref('voice.grace_seconds', parseInt(sel.value, 10) || 0);
  });
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
  const quickCmd1El = document.getElementById('quick-cmd-1');
  const quickCmd2El = document.getElementById('quick-cmd-2');
  if (!quickCmd1El || !quickCmd2El) return;

  quickCmd1El.value = getQuickCommand(1);
  quickCmd2El.value = getQuickCommand(2);
  refreshQuickCommandButtons();

  quickCmd1El.addEventListener('change', () => {
    const value = sanitizeQuickCommand(quickCmd1El.value, DEFAULT_QUICK_CMD_1);
    quickCmd1El.value = value;
    setUserPref('quick_cmds.cmd1', value);
    refreshQuickCommandButtons();
  });

  quickCmd2El.addEventListener('change', () => {
    const value = sanitizeQuickCommand(quickCmd2El.value, DEFAULT_QUICK_CMD_2);
    quickCmd2El.value = value;
    setUserPref('quick_cmds.cmd2', value);
    refreshQuickCommandButtons();
  });
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

// ---- Hub 情報表示（single source: main.version / runtime → /api/info → ここ） ----
(async () => {
  try {
    const res = await fetch(`/api/info?token=${token}`);
    if (!res.ok) return;
    const info = await res.json();
    set__userAvatarUrl(info.userAvatar || '');
    set__userDisplayName(info.userDisplayName || '');
    document.dispatchEvent(new CustomEvent('user-info-ready'));
    const ver = 'v' + (info.version || 'dev');
    const runtimeMode = info.runtime_mode || '';
    const runtimeLabel = () => {
      if (typeof window.t !== 'function') return info.runtime_label || runtimeMode;
      const key = `runtime_${String(runtimeMode).replace(/-/g, '_')}`;
      const translated = window.t(key);
      return translated && translated !== key ? translated : (info.runtime_label || runtimeMode);
    };
    const envFallbacks: Record<string, { label: string; short: string; color: string; title: string }> = {
      local: { label: 'Local', short: 'L', color: '#22c55e', title: 'ANY-AI-CLI Local' },
      wsl: { label: 'WSL', short: 'W', color: '#3b82f6', title: 'ANY-AI-CLI WSL' },
      vps: { label: 'VPS', short: 'V', color: '#f97316', title: 'ANY-AI-CLI VPS' },
      'vps-tunnel': { label: 'VPS Tunnel', short: 'T', color: '#ef4444', title: 'ANY-AI-CLI VPS Tunnel' },
    };
    const normalizeEnvKind = (value) => {
      const raw = String(value || '').trim().toLowerCase().replace(/_/g, '-');
      if (raw === 'local' || raw === 'wsl' || raw === 'vps' || raw === 'vps-tunnel') return raw;
      if (raw === 'vpstunnel') return 'vps-tunnel';
      return '';
    };
    const sanitizeEnvColor = (value, fallback) => {
      const color = String(value || '').trim();
      return /^#[0-9a-fA-F]{6}$/.test(color) ? color : fallback;
    };
    const escapeSvgText = (value) => String(value || '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
    const envFaviconUrl = (short, color) => {
      const text = escapeSvgText(String(short || 'L').trim().slice(0, 1).toUpperCase() || 'L');
      const fill = sanitizeEnvColor(color, '#22c55e');
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="${fill}"/><text x="32" y="35" text-anchor="middle" dominant-baseline="middle" font-family="Segoe UI, Arial, sans-serif" font-size="36" font-weight="700" fill="#fff">${text}</text></svg>`;
      return `data:image/svg+xml,${encodeURIComponent(svg)}`;
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
    const envKind = serverEnvKind || hintEnvKind || (hintSSH ? 'vps-tunnel' : 'local');
    const envBase = envFallbacks[envKind] || envFallbacks.local;
    const envLabelRaw = info.env_label || envBase.label;
    const envShort = info.env_short || envBase.short;
    const envColor = sanitizeEnvColor(info.env_color, envBase.color);
    const envTitle = info.env_title || envBase.title || `ANY-AI-CLI ${envLabelRaw}`;
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
      document.title = envTitle;
      const appleTitle = document.getElementById('apple-web-app-title');
      if (appleTitle) appleTitle.setAttribute('content', envTitle);
      const faviconEl = document.getElementById('app-favicon') || document.querySelector('link[rel~="icon"]');
      if (faviconEl) {
        faviconEl.setAttribute('href', envFaviconUrl(envShort, envColor));
        faviconEl.setAttribute('type', 'image/svg+xml');
      }
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

export const VALID_TAB_NAMES = new Set(['terminal', 'chat', 'split', 'files', 'git', 'workbench', 'multi']);
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
  chip.innerHTML =
    `<span class="sid">#${s.id}</span>` +
    `${providerIconHtml(s.provider)} ${providerChipHtml}${modelBadge}` +
    ` <span class="badge ${safeClassToken(state)}">${escapeHtml(stateLbl)}</span>`;
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
    area.classList.remove('mode-terminal', 'mode-chat', 'mode-split', 'mode-files', 'mode-git', 'mode-workbench');
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

  area.classList.remove('mode-terminal', 'mode-chat', 'mode-split', 'mode-files', 'mode-git', 'mode-workbench');
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


// ─── カード右クリックメニュー (Open Git / Files / Activate / Copy ID) ───
export let _cardCtxMenuEl = null;
export let _cardCtxSid    = null;
export function openCardCtxMenu(x, y, sid) {
  closeCardCtxMenu();
  _cardCtxSid = sid;
  const menu = document.createElement('div');
  menu.className = 'card-ctx-menu open';
  menu.id = 'card-ctx-menu';
  const labelOpenGit   = ti18n('ctx_open_git',   'Open Git View');
  const labelOpenFiles = ti18n('ctx_open_files', 'Open Files Tab');
  const labelActivate  = ti18n('ctx_activate',   'Activate Session');
  const labelCopyId    = ti18n('ctx_copy_id',    'Copy session ID');
  menu.innerHTML =
    `<button type="button" data-action="open-git"><span class="ico">⎇</span><span>${escapeHtml(labelOpenGit)}</span><span class="kbd">Ctrl+Shift+G</span></button>` +
    `<button type="button" data-action="open-files"><span class="ico">📁</span><span>${escapeHtml(labelOpenFiles)}</span><span class="kbd">Ctrl+Shift+F</span></button>` +
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


// ---- ファイルリンク設定 ----
(function () {
  const fileOpenAppEl     = document.getElementById('settings-file-open-app');
  const fileOpenBrowseBtn = document.getElementById('settings-file-open-app-browse');
  const fileOpenEffectiveEl = document.getElementById('settings-file-open-app-effective');
  const terminalAppEl     = document.getElementById('settings-terminal-app');
  const terminalBrowseBtn = document.getElementById('settings-terminal-app-browse');
  const terminalEffectiveEl = document.getElementById('settings-terminal-app-effective');
  if (!fileOpenAppEl) return;

  function renderEffectiveCommand(el, value) {
    if (!el) return;
    el.textContent = value ? `${t('settings_effective_command')}: ${value}` : '';
  }

  async function loadFileOpenApp() {
    try {
      const res = await fetch(`/api/file-open-app?token=${token}`);
      if (!res.ok) return;
      const cfg = await res.json();
      fileOpenAppEl.value = cfg.file_open_app || '';
      renderEffectiveCommand(fileOpenEffectiveEl, cfg.effective_file_open_app);
    } catch (_) {}
  }

  async function saveFileOpenApp() {
    try {
      const res = await fetch(`/api/file-open-app?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ file_open_app: fileOpenAppEl.value.trim() }),
      });
      if (res.ok) {
        const cfg = await res.json();
        renderEffectiveCommand(fileOpenEffectiveEl, cfg.effective_file_open_app);
      }
    } catch (_) {}
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

  if (fileOpenBrowseBtn) {
    fileOpenBrowseBtn.addEventListener('click', async () => {
      fileOpenBrowseBtn.disabled = true;
      try {
        const res = await fetch(`/api/pick-file?filter=exe&token=${token}`, { method: 'POST' });
        if (res.ok) {
          const data = await res.json();
          if (data.ok && data.path) fileOpenAppEl.value = data.path;
        }
      } catch (_) {}
      finally { fileOpenBrowseBtn.disabled = false; }
    });
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
    await saveFileOpenApp();
    await saveTerminalApp();
  };

  // 設定パネルを開いたときのロード
  const settingsBtn = document.getElementById('settings-btn');
  if (settingsBtn) {
    settingsBtn.addEventListener('click', () => {
      if (!document.getElementById('settings-panel').hidden) {
        loadFileOpenApp();
        loadTerminalApp();
      }
    });
  }
})();

