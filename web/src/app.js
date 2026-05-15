console.log('[any-ai-cli] app.js build=2026-05-15-voice-hw-end-await');

// ---- トースト通知 ----
let _toastTimer = null;
function getToastAnchorRect(anchor) {
  if (!anchor) return null;
  if (typeof anchor.getBoundingClientRect === 'function') {
    return anchor.getBoundingClientRect();
  }
  if (typeof anchor.clientX === 'number' && typeof anchor.clientY === 'number') {
    return {
      left: anchor.clientX,
      right: anchor.clientX,
      top: anchor.clientY,
      height: 0,
    };
  }
  return null;
}

function showToast(msg, anchor) {
  let el = document.getElementById('toast');
  if (!el) {
    el = document.createElement('div');
    el.id = 'toast';
    document.body.appendChild(el);
  }
  el.textContent = msg;
  const r = getToastAnchorRect(anchor);
  if (r) {
    const margin = 8;
    const y = Math.min(Math.max(r.top + r.height / 2, margin), window.innerHeight - margin);
    el.style.top = y + 'px';
    el.style.bottom = 'auto';
    el.classList.add('toast--anchored');
    // 右側に置けるか確認し、はみ出す場合は anchor の左側に表示
    el.style.left = (r.right + 6) + 'px';
    const w = el.offsetWidth;
    if (r.right + 6 + w > window.innerWidth - margin) {
      el.style.left = Math.max(margin, r.left - 6 - w) + 'px';
    }
  } else {
    el.style.left = '';
    el.style.top = '';
    el.style.bottom = '';
    el.classList.remove('toast--anchored');
  }
  el.classList.add('show');
  clearTimeout(_toastTimer);
  _toastTimer = setTimeout(() => { el.classList.remove('show'); el.classList.remove('toast--anchored'); }, 1800);
}

// i18n ロード前のフォールバック（i18n.js が window.t を上書きするまでキーをそのまま返す）
if (typeof window.t !== 'function') window.t = (key) => key;

// ---- Hub 承認ボタン機能 オプトイントースト ----
let _approvalAlertChecked = false;

async function checkApprovalOnStartup() {
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

function showApprovalToast() {
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

function updateApprovalToggle(enabled) {
  const toggle = document.getElementById('approval-toggle-input');
  if (toggle) toggle.checked = enabled;
}

async function loadApprovalSettings() {
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

// ---- 設定パネル ----

const STORAGE_THEME_KEY      = 'ai_cli_hub_theme';
const STORAGE_FONTSIZE_KEY   = 'ai_cli_hub_fontsize';
const STORAGE_LANG_KEY       = 'ai_cli_hub_lang';
const STORAGE_FAVORITES_KEY         = 'ai_cli_hub_favorites';
const STORAGE_ORDER_KEY             = 'ai_cli_hub_session_order';
const STORAGE_GROUP_ORDER_KEY       = 'ai_cli_hub_group_order';
const STORAGE_PROJECT_FAVORITES_KEY = 'ai_cli_hub_project_favorites';
const STORAGE_SPAWN_KEY             = 'ai_cli_hub_spawn_settings';
const STORAGE_CWD_HISTORY_KEY       = 'ai_cli_hub_cwd_history';
const STORAGE_TRIGGER_ENABLED_KEY      = 'ai_cli_hub_trigger_enabled';
const STORAGE_TRIGGER_PHRASE_KEY       = 'ai_cli_hub_trigger_phrase';
const STORAGE_NOTIFY_SOUND_ENABLED_KEY = 'ai_cli_hub_notify_sound_enabled';
const STORAGE_NOTIFY_SOUND_TYPE_KEY    = 'ai_cli_hub_notify_sound_type';
const STORAGE_NOTIFY_SOUND_CUSTOM_KEY  = 'ai_cli_hub_notify_sound_custom';
const STORAGE_APPROVAL_AUTO_SWITCH_KEY = 'ai_cli_hub_approval_auto_switch';
const STORAGE_QUICK_CMD_1_KEY          = 'ai_cli_hub_quick_cmd_1';
const STORAGE_QUICK_CMD_2_KEY          = 'ai_cli_hub_quick_cmd_2';
const STORAGE_USAGE_LINK_CLAUDE_KEY    = 'ai_cli_hub_usage_link_claude';
const STORAGE_USAGE_LINK_CODEX_KEY     = 'ai_cli_hub_usage_link_codex';
const STORAGE_VOICE_GRACE_KEY          = 'ai_cli_hub_voice_grace_seconds';
const DEFAULT_VOICE_GRACE_SEC          = 2;
const STORAGE_WAKE_WORD_ENABLED_KEY    = 'ai_cli_hub_wake_word_enabled';
const STORAGE_WAKE_WORD_PHRASE_KEY     = 'ai_cli_hub_wake_word_phrase';
const DEFAULT_WAKE_WORD_PHRASE         = '音声入力実施';
const CWD_HISTORY_MAX               = 10;

const DEFAULT_USAGE_LINKS = {
  claude: 'https://claude.ai/settings/usage',
  codex: 'https://chatgpt.com/codex/cloud/settings/analytics#usage',
};

const FONTSIZE_MAP = { large: 15, medium: 13, small: 11 };

// ---- 通知音 ----
let _audioCtx = null;
function _getAudioCtx() {
  if (!_audioCtx) _audioCtx = new (window.AudioContext || window.webkitAudioContext)();
  return _audioCtx;
}

function playNotificationSound() {
  if (localStorage.getItem(STORAGE_NOTIFY_SOUND_ENABLED_KEY) !== '1') return;
  const type = localStorage.getItem(STORAGE_NOTIFY_SOUND_TYPE_KEY) || 'default';
  if (type === 'custom') {
    const dataUrl = localStorage.getItem(STORAGE_NOTIFY_SOUND_CUSTOM_KEY);
    if (dataUrl) {
      try { new Audio(dataUrl).play().catch(() => {}); return; } catch (_) {}
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

function getActiveTriggerPhrase() {
  if (localStorage.getItem(STORAGE_TRIGGER_ENABLED_KEY) !== '1') return '';
  return (localStorage.getItem(STORAGE_TRIGGER_PHRASE_KEY) || '').trim();
}

function normalizeTriggerMatchText(text) {
  return String(text || '')
    .normalize('NFKC')
    .replace(/[\s\u3000]+$/g, '')
    .replace(/[。．.!！?？、,，]+$/g, '')
    .replace(/[\s\u3000]+$/g, '');
}

function textEndsWithTriggerPhrase(text, triggerPhrase) {
  const tp = normalizeTriggerMatchText(triggerPhrase);
  if (!tp) return false;
  return normalizeTriggerMatchText(text).endsWith(tp);
}

function stripTrailingTriggerPhrase(text, triggerPhrase) {
  const original = String(text || '');
  const tp = normalizeTriggerMatchText(triggerPhrase);
  if (!tp) return original;
  let normalized = normalizeTriggerMatchText(original);
  if (!normalized.endsWith(tp)) return original;
  normalized = normalized.slice(0, normalized.length - tp.length);
  return normalized.trimEnd();
}

function escapeHtml(str) {
  return String(str).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function normalizeHttpUrl(value, fallback = '') {
  const raw = String(value || '').trim();
  if (!raw) return fallback;
  try {
    const url = new URL(raw);
    if (url.protocol === 'https:' || url.protocol === 'http:') return url.href;
  } catch (_) {}
  return fallback;
}

function getUsageLinkUrl(provider) {
  const key = provider === 'claude' ? STORAGE_USAGE_LINK_CLAUDE_KEY : STORAGE_USAGE_LINK_CODEX_KEY;
  return normalizeHttpUrl(localStorage.getItem(key), DEFAULT_USAGE_LINKS[provider]);
}

function applyUsageLinks() {
  const claudeLink = document.getElementById('usage-link-claude');
  const codexLink = document.getElementById('usage-link-codex');
  if (claudeLink) claudeLink.href = getUsageLinkUrl('claude');
  if (codexLink) codexLink.href = getUsageLinkUrl('codex');
}

function loadUsageLinkSettings() {
  const claudeInput = document.getElementById('usage-link-claude-url');
  const codexInput = document.getElementById('usage-link-codex-url');
  if (claudeInput) claudeInput.value = localStorage.getItem(STORAGE_USAGE_LINK_CLAUDE_KEY) || '';
  if (codexInput) codexInput.value = localStorage.getItem(STORAGE_USAGE_LINK_CODEX_KEY) || '';
  applyUsageLinks();
}

function saveUsageLinkSettings() {
  const claudeInput = document.getElementById('usage-link-claude-url');
  const codexInput = document.getElementById('usage-link-codex-url');
  const pairs = [
    [claudeInput, STORAGE_USAGE_LINK_CLAUDE_KEY],
    [codexInput, STORAGE_USAGE_LINK_CODEX_KEY],
  ];
  for (const [input, key] of pairs) {
    if (!input) continue;
    const raw = input.value.trim();
    const normalized = normalizeHttpUrl(raw, '');
    try {
      if (normalized) {
        localStorage.setItem(key, normalized);
        input.value = normalized;
      } else {
        localStorage.removeItem(key);
        input.value = '';
      }
    } catch (_) {}
  }
  applyUsageLinks();
}

function appConfirm({ title, message, confirmText, cancelText, kind = 'default' }) {
  return new Promise((resolve) => {
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

function appConfirmShutdown() {
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

function stripAnsi(str) {
  return str.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, '').replace(/\x1b[^[]/g, '');
}

const DEFAULT_QUICK_CMD_1 = '/clear';
const DEFAULT_QUICK_CMD_2 = '/model';
const ALLOWED_QUICK_COMMANDS = new Set([
  '/clear', '/model', '/help', '/status', '/usage', '/review', '/compact', '/config',
]);

function sanitizeQuickCommand(cmd, fallback) {
  return ALLOWED_QUICK_COMMANDS.has(cmd) ? cmd : fallback;
}

function getQuickCommand(slot) {
  if (slot === 1) {
    const saved = localStorage.getItem(STORAGE_QUICK_CMD_1_KEY) || DEFAULT_QUICK_CMD_1;
    return sanitizeQuickCommand(saved, DEFAULT_QUICK_CMD_1);
  }
  const saved = localStorage.getItem(STORAGE_QUICK_CMD_2_KEY) || DEFAULT_QUICK_CMD_2;
  return sanitizeQuickCommand(saved, DEFAULT_QUICK_CMD_2);
}

function refreshQuickCommandButtons() {
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
function filterFirstMessage(text) {
  if (!text) return '';
  const trimmed = text.trim();
  if (trimmed.startsWith('/')) return '';
  return trimmed.replace(/@\S+/g, '').replace(/\s+/g, ' ').trim();
}

function applyTheme(theme) {
  // 'blue' は廃止済み。既存設定が 'blue' の場合は 'dark' にフォールバック
  const t = (theme === 'dark' || theme === 'light') ? theme : 'dark';
  document.documentElement.setAttribute('data-theme', t);
  const sel = document.getElementById('theme-select');
  if (sel) sel.value = t;
  try { localStorage.setItem(STORAGE_THEME_KEY, t); } catch (_) {}
}

function applyFontSize(size) {
  const s = FONTSIZE_MAP[size] ? size : 'medium';
  const px = FONTSIZE_MAP[s];
  terminals.forEach((t, id) => {
    t.term.options.fontSize = px;
    requestAnimationFrame(() => {
      fitTerminalPreservingBottom(t, id);
      sendResize(id, t.term.cols, t.term.rows);
    });
  });
  const sel = document.getElementById('fontsize-select');
  if (sel) sel.value = s;
  try { localStorage.setItem(STORAGE_FONTSIZE_KEY, s); } catch (_) {}
}

function applyLang(lang) {
  const l = (lang === 'ja' || lang === 'en') ? lang : 'ja';
  const sel = document.getElementById('lang-select');
  if (sel) sel.value = l;
}

(function () {
  applyTheme(localStorage.getItem(STORAGE_THEME_KEY) || 'dark');
  applyLang(localStorage.getItem(STORAGE_LANG_KEY) || 'ja');
  applyUsageLinks();

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

  themeEl.addEventListener('change',    () => applyTheme(themeEl.value));
  fontsizeEl.addEventListener('change', () => applyFontSize(fontsizeEl.value));
  langEl.addEventListener('change',     () => window.setLang(langEl.value));
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
      try {
        localStorage.removeItem(STORAGE_USAGE_LINK_CLAUDE_KEY);
        localStorage.removeItem(STORAGE_USAGE_LINK_CODEX_KEY);
      } catch (_) {}
      loadUsageLinkSettings();
      showToast(t('settings_usage_links_reset_done'), usageLinksResetBtn);
    });
  }
  aboutCloseBtn.addEventListener('click', () => { aboutPanel.hidden = true; });
  aboutPanel.addEventListener('click', (e) => {
    if (e.target === aboutPanel) aboutPanel.hidden = true;
  });
})();

// ---- 音声入力 終了検知 待ち時間 設定 ----
(function () {
  const sel = document.getElementById('voice-grace-select');
  if (!sel) return;
  const saved = localStorage.getItem(STORAGE_VOICE_GRACE_KEY);
  const v = saved == null ? DEFAULT_VOICE_GRACE_SEC : parseInt(saved, 10);
  const clamped = Number.isFinite(v) ? Math.max(0, Math.min(5, v)) : DEFAULT_VOICE_GRACE_SEC;
  sel.value = String(clamped);
  sel.addEventListener('change', () => {
    try { localStorage.setItem(STORAGE_VOICE_GRACE_KEY, sel.value); } catch (_) {}
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
    try { localStorage.setItem(STORAGE_TRIGGER_ENABLED_KEY, enabledEl.checked ? '1' : '0'); } catch (_) {}
  });
  phraseInputEl.addEventListener('input', () => {
    try { localStorage.setItem(STORAGE_TRIGGER_PHRASE_KEY, phraseInputEl.value); } catch (_) {}
  });

  enabledEl.checked = localStorage.getItem(STORAGE_TRIGGER_ENABLED_KEY) === '1';
  phraseRow.hidden = !enabledEl.checked;
  phraseInputEl.value = localStorage.getItem(STORAGE_TRIGGER_PHRASE_KEY) || '';
})();

// ---- ウェイクワード設定 ----
(function () {
  const enabledEl = document.getElementById('wakeword-enabled');
  const phraseRow = document.getElementById('wakeword-phrase-row');
  const phraseEl  = document.getElementById('wakeword-phrase-input');
  if (!enabledEl) return;

  enabledEl.addEventListener('change', () => {
    phraseRow.hidden = !enabledEl.checked;
    try { localStorage.setItem(STORAGE_WAKE_WORD_ENABLED_KEY, enabledEl.checked ? '1' : '0'); } catch (_) {}
    document.dispatchEvent(new CustomEvent('wakewordsettings:changed'));
  });
  phraseEl.addEventListener('input', () => {
    try { localStorage.setItem(STORAGE_WAKE_WORD_PHRASE_KEY, phraseEl.value); } catch (_) {}
  });

  enabledEl.checked = localStorage.getItem(STORAGE_WAKE_WORD_ENABLED_KEY) === '1';
  phraseRow.hidden = !enabledEl.checked;
  phraseEl.value = localStorage.getItem(STORAGE_WAKE_WORD_PHRASE_KEY) ?? DEFAULT_WAKE_WORD_PHRASE;
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
    try { localStorage.setItem(STORAGE_NOTIFY_SOUND_ENABLED_KEY, soundEnabledEl.checked ? '1' : '0'); } catch (_) {}
    updateSoundVisibility();
  });

  soundTypeEl.addEventListener('change', () => {
    try { localStorage.setItem(STORAGE_NOTIFY_SOUND_TYPE_KEY, soundTypeEl.value); } catch (_) {}
    updateSoundVisibility();
  });

  soundBrowseBtn.addEventListener('click', () => soundFileEl.click());

  soundFileEl.addEventListener('change', () => {
    const file = soundFileEl.files[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = (e) => {
      try {
        localStorage.setItem(STORAGE_NOTIFY_SOUND_CUSTOM_KEY, e.target.result);
        soundFilenameEl.textContent = file.name;
      } catch (_) { showToast(t('notify_sound_storage_error')); }
    };
    reader.readAsDataURL(file);
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
  if (savedCustom) soundFilenameEl.textContent = t('notify_sound_file_set');
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
    try { localStorage.setItem(STORAGE_QUICK_CMD_1_KEY, value); } catch (_) {}
    refreshQuickCommandButtons();
  });

  quickCmd2El.addEventListener('change', () => {
    const value = sanitizeQuickCommand(quickCmd2El.value, DEFAULT_QUICK_CMD_2);
    quickCmd2El.value = value;
    try { localStorage.setItem(STORAGE_QUICK_CMD_2_KEY, value); } catch (_) {}
    refreshQuickCommandButtons();
  });
})();

const token = new URLSearchParams(location.search).get('token');

// ---- バージョン表示（single source: main.version → /api/info → ここ） ----
(async () => {
  try {
    const res = await fetch(`/api/info?token=${token}`);
    if (!res.ok) return;
    const info = await res.json();
    const ver = 'v' + (info.version || 'dev');
    const apply = () => {
      const settingsEl = document.querySelector('.settings-app-version');
      if (settingsEl) settingsEl.textContent = ver + ' [Hub UI]';
      const aboutEl = document.querySelector('.about-version');
      if (aboutEl) {
        aboutEl.textContent = (typeof window.t === 'function')
          ? window.t('about_version', { version: ver })
          : ver + ' [Hub UI]';
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
(function () {
  const providerEl = document.getElementById('approval-patterns-provider');
  const listEl = document.getElementById('approval-patterns-list');
  const inputEl = document.getElementById('approval-patterns-input');
  const addBtn = document.getElementById('approval-patterns-add-btn');
  const resetBtn = document.getElementById('approval-patterns-reset-btn');
  if (!providerEl || !listEl || !inputEl || !addBtn || !resetBtn) return;

  const cache = { claude: [], codex: [], common: [] };

  async function loadAll() {
    try {
      const res = await fetch(`/api/approval-patterns?token=${token}`);
      if (!res.ok) throw new Error('http ' + res.status);
      const data = await res.json();
      for (const k of Object.keys(cache)) cache[k] = Array.isArray(data[k]) ? data[k] : [];
      render();
      // フロント側のキャッシュも更新（検出ロジックが即時に新パターンを使えるように）
      const norm = arr => arr.map(s => String(s).toLowerCase()).filter(Boolean);
      providerApprovalTriggers.claude = norm(cache.claude);
      providerApprovalTriggers.codex  = norm(cache.codex);
      providerApprovalTriggers.common = norm(cache.common);
    } catch (e) {
      console.warn('approval patterns load failed', e);
      showToast(t('settings_approval_patterns_load_failed'));
    }
  }

  function render() {
    const provider = providerEl.value;
    const list = cache[provider] || [];
    listEl.innerHTML = '';
    for (let i = 0; i < list.length; i++) {
      const li = document.createElement('li');
      const span = document.createElement('span');
      span.className = 'pattern-text';
      span.textContent = list[i];
      const rm = document.createElement('button');
      rm.className = 'pattern-remove';
      rm.textContent = '✕';
      rm.title = t('settings_approval_patterns_remove');
      rm.addEventListener('click', () => removeAt(i));
      li.appendChild(span);
      li.appendChild(rm);
      listEl.appendChild(li);
    }
  }

  async function save(provider) {
    try {
      const res = await fetch(`/api/approval-patterns/${provider}?token=${token}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(cache[provider]),
      });
      if (!res.ok) throw new Error('http ' + res.status);
      const norm = arr => arr.map(s => String(s).toLowerCase()).filter(Boolean);
      providerApprovalTriggers[provider] = norm(cache[provider]);
      showToast(t('settings_approval_patterns_saved'));
    } catch (e) {
      console.warn('approval patterns save failed', e);
      showToast(t('settings_approval_patterns_save_failed'));
      // 失敗時は再ロードして整合性を回復
      await loadAll();
    }
  }

  function addPattern() {
    const text = inputEl.value.trim();
    if (!text) return;
    const provider = providerEl.value;
    if (cache[provider].includes(text)) {
      inputEl.value = '';
      return;
    }
    cache[provider] = [...cache[provider], text];
    inputEl.value = '';
    render();
    save(provider);
  }

  function removeAt(idx) {
    const provider = providerEl.value;
    cache[provider] = cache[provider].filter((_, i) => i !== idx);
    render();
    save(provider);
  }

  async function resetCurrent() {
    const provider = providerEl.value;
    const ok = await appConfirm({
      title: t('settings_approval_patterns_reset'),
      message: t('settings_approval_patterns_reset_confirm'),
      confirmText: t('settings_approval_patterns_reset'),
      kind: 'warn',
    });
    if (!ok) return;
    try {
      const res = await fetch(`/api/approval-patterns/${provider}/reset?token=${token}`, { method: 'POST' });
      if (!res.ok) throw new Error('http ' + res.status);
      showToast(t('settings_approval_patterns_reset_done'));
      await loadAll();
    } catch (e) {
      console.warn('approval patterns reset failed', e);
      showToast(t('settings_approval_patterns_save_failed'));
    }
  }

  providerEl.addEventListener('change', render);
  addBtn.addEventListener('click', addPattern);
  inputEl.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); addPattern(); } });
  resetBtn.addEventListener('click', resetCurrent);

  loadAll();
})();

const sessions = new Map();
const terminals = new Map(); // sessionId -> { term, fitAddon, container, pendingChunks, pendingTextTail, markerFilterCarry }
const approvalVisibleCache = new Map();
const multiQuestionVisibleCache = new Map(); // sessionId → bool（Claude Code AskUserQuestion 等の複数質問 UI が画面に出ているか）
const approvalRawOptionsCache = new Map(); // sessionId → [{num, label, isCurrent}]
const approvalConsumedSig = new Map(); // sessionId → 消費済み承認の署名（doSend でテキスト送信した場合の再表示防止）
const approvalConsumedSigDeleteTimer = new Map(); // sessionId → timer（sig を debounce 型で削除するためのタイマー）
const APPROVAL_PENDING_TEXT_TAIL_LIMIT = 12000;

// 承認選択肢の sig を計算。Ink の再描画やスクロールバック残骸による
// label の微妙な差異（前後空白、空白の重複、truncate 位置）を吸収するため normalize する。
// (Y:1/N:0) Yes/No プロンプトはどれも同じ label を持つため、_ctx に質問文ハッシュを
// 載せて区別する（連続する別質問が同一 sig で誤抑制されないように）。
function approvalSig(options) {
  return JSON.stringify((options || []).map(o => {
    const lbl = String(o.label || '').trim().replace(/\s+/g, ' ').slice(0, 80);
    const ctx = o && o._ctx ? `|${o._ctx}` : '';
    return `${o.num}:${lbl}${ctx}`;
  }));
}

// シンプルな文字列ハッシュ (djb2)。承認質問文の同一性判定に使う。
function _approvalCtxHash(s) {
  const text = String(s || '').replace(/\s+/g, ' ').trim();
  let h = 5381;
  for (let i = 0; i < text.length; i++) {
    h = (((h << 5) + h) + text.charCodeAt(i)) | 0;
  }
  return (h >>> 0).toString(36);
}
const approvalHintConfirmTimers = new Map(); // sessionId → timer（生バイト検出を短時間 debounce してチカチカを防ぐ）
const toolOutputs = new Map(); // sessionId → [{uid, lines, ts}]
const sessionInputState = new Map(); // sessionId → { inputValue, pastedTextsData, pendingAttachFiles, thumbsFragment }
const autoDismissTimers = new Map(); // sessionId → timer
const approvalSuppressUntil = new Map(); // sessionId → timestamp (sendChoice 後の誤再表示を抑制)
const approvalAutoSwitchQueue = [];
const utf8Decoder = new TextDecoder('utf-8');

let activeSessionId = null;
let isComposing = false;       // IMEコンポジション状態
let pendingSend = false;       // IME確定後に送信するフラグ
let composeEndSendTimer = null; // compositionend が doSend をスケジュール済みの場合のタイマーID
let composeEndSent = false;    // compositionend タイマーが既に doSend を実行済みの場合のフラグ
const SIDEBAR_COLLAPSED_WIDTH_THRESHOLD = 180;
let expandCapturePending = false;
// action-bar の点滅防止用
// - lastActionBarRender: 前回描画した内容のシグネチャ（同一なら DOM 再構築をスキップ）
// - crunchLatch: detectCrunch の振動吸収用ヒステリシス（一度 found=true を観測したら CRUNCH_LATCH_MS の間は維持）
const lastActionBarRender = { sessionId: null, sig: null };
const crunchLatch = new Map();
const CRUNCH_LATCH_MS = 800;
let _elapsedTimerInterval = null;
let dragSrcId = null;
let dragSrcGroupKey = null;
let pendingAutoSwitch = false;
let actionBarFocusIdx = -1;
let approvalAutoSwitchInProgress = false;

let favorites = JSON.parse(localStorage.getItem(STORAGE_FAVORITES_KEY) || '[]');
let projectFavorites = JSON.parse(localStorage.getItem(STORAGE_PROJECT_FAVORITES_KEY) || '[]');
let sessionOrder = JSON.parse(localStorage.getItem(STORAGE_ORDER_KEY) || '[]');
let groupOrder = JSON.parse(localStorage.getItem(STORAGE_GROUP_ORDER_KEY) || '[]');
const collapsedGroups = new Set();

function saveFavorites() {
  localStorage.setItem(STORAGE_FAVORITES_KEY, JSON.stringify(favorites));
}

function saveProjectFavorites() {
  localStorage.setItem(STORAGE_PROJECT_FAVORITES_KEY, JSON.stringify(projectFavorites));
}

function saveGroupOrder() {
  localStorage.setItem(STORAGE_GROUP_ORDER_KEY, JSON.stringify(groupOrder));
}

function saveSessionOrder() {
  localStorage.setItem(STORAGE_ORDER_KEY, JSON.stringify(sessionOrder));
}

function addToSessionOrder(id, forceToFront = false) {
  const idx = sessionOrder.indexOf(id);
  if (idx !== -1) {
    if (forceToFront) { sessionOrder.splice(idx, 1); sessionOrder.unshift(id); }
  } else {
    sessionOrder.unshift(id);
  }
}

function removeFromSessionOrder(id) {
  const idx = sessionOrder.indexOf(id);
  if (idx !== -1) { sessionOrder.splice(idx, 1); saveSessionOrder(); }
}

function isApprovalAutoSwitchEnabled() {
  return localStorage.getItem(STORAGE_APPROVAL_AUTO_SWITCH_KEY) === '1';
}

function isCurrentSessionHoldingApprovalFocus() {
  if (activeSessionId === null) return false;
  return !!approvalVisibleCache.get(activeSessionId);
}

function removeApprovalAutoSwitchTarget(sessionId) {
  for (let i = approvalAutoSwitchQueue.length - 1; i >= 0; i--) {
    if (approvalAutoSwitchQueue[i] === sessionId) approvalAutoSwitchQueue.splice(i, 1);
  }
}

function maybeAutoSwitchToNextApproval() {
  if (!isApprovalAutoSwitchEnabled()) return;
  if (approvalAutoSwitchInProgress) return;
  if (activeSessionId === null) return;
  if (isCurrentSessionHoldingApprovalFocus()) return;
  const bar = document.getElementById('action-bar');
  if (bar && bar.classList.contains('visible')) return;
  const panel = document.getElementById('settings-panel');
  if (panel && !panel.hidden) return;

  while (approvalAutoSwitchQueue.length > 0) {
    const nextId = approvalAutoSwitchQueue[0];
    if (!sessions.has(nextId) || !approvalVisibleCache.get(nextId) || nextId === activeSessionId) {
      approvalAutoSwitchQueue.shift();
      continue;
    }
    approvalAutoSwitchQueue.shift();
    approvalAutoSwitchInProgress = true;
    try {
      activateSession(nextId);
    } finally {
      approvalAutoSwitchInProgress = false;
    }
    return;
  }
}

function enqueueApprovalAutoSwitch(sessionId) {
  if (!isApprovalAutoSwitchEnabled()) return;
  if (sessionId === activeSessionId) return;
  if (!sessions.has(sessionId)) return;
  if (!approvalAutoSwitchQueue.includes(sessionId)) {
    approvalAutoSwitchQueue.push(sessionId);
  }
  maybeAutoSwitchToNextApproval();
}

document.addEventListener('i18n-ready', () => {
  document.getElementById('summary').textContent = t('registering');
  renderSessionList();
});

const ws = new WebSocket(`ws://${location.host}/ws`);
ws.onerror = () => { document.getElementById('summary').textContent = t('ws_error'); };
ws.onclose = (e) => {
  document.getElementById('summary').textContent = t('ws_close', { code: e.code });
  const nsBtn = document.getElementById('new-session-btn');
  if (nsBtn) { nsBtn.disabled = true; document.getElementById('new-session-panel').hidden = true; }
  document.getElementById('reconnect-btn').hidden = false;
  sessions.clear();
  autoDismissTimers.forEach(t => clearTimeout(t));
  autoDismissTimers.clear();
  activeSessionId = null;
  updateShellBadge(null);
  updateTabNotification(0);
  const area = document.getElementById('terminal-area');
  if (area) area.innerHTML = '';
  hideActionBar(undefined);
  renderSessionList();
};

ws.onopen = () => {
  document.getElementById('summary').textContent = t('registering');
  const nsBtn = document.getElementById('new-session-btn');
  if (nsBtn) nsBtn.disabled = false;
  document.getElementById('reconnect-btn').hidden = true;
  const area = document.getElementById('terminal-area');
  // clientWidth/Height が「ゼロではないが極小（レイアウト未確定 / 親が
  // display:none 直後など）」のときに `0 || 200` のフォールバックが効かず、
  // cols=1〜13 のような不正値が Hub の lastUICols として記録される。
  // その値で spawn された新セッションは極狭幅 PTY で起動し、内部で強制改行
  // された出力を吐く。xterm 側で再 fit しても過去行はリフローされず、
  // 1 行 5〜7 文字のような折り返しが残り続ける症状になる。
  // 安全しきい値未満は既定値（200×50）にフォールバック。
  // 実寸は attach 後 whenLayoutReady → sendResize で送り直される。
  const cw = area ? area.clientWidth : 0;
  const ch = area ? area.clientHeight : 0;
  const cols = cw >= 120 ? Math.floor(cw / 7.5) : 200;
  const rows = ch >= 80  ? Math.floor(ch / 16)  : 50;
  ws.send(JSON.stringify({ type: 'register', role: 'ui', token, cols, rows }));
};

document.getElementById('reconnect-btn').addEventListener('click', async () => {
  const btn = document.getElementById('reconnect-btn');
  btn.disabled = true;
  btn.textContent = t('reconnect_checking') || '確認中...';
  try {
    // トークン不要の疎通確認（401でも「Hub起動中」と判断）
    await fetch('/', { signal: AbortSignal.timeout(2000) });
    location.reload();
  } catch (_) {
    btn.textContent = '↺ ' + (t('reconnect') || '再接続');
    btn.disabled = false;
    document.getElementById('summary').textContent = t('hub_stopped') || 'Hub停止中 — any-ai-cli serve で再起動してください';
  }
});

ws.onmessage = (ev) => {
  const m = JSON.parse(ev.data);

  if (m.type === 'pty_data') {
    const id = m.session_id;
    ensureTerminal(id);
    const t = terminals.get(id);
    const binary = atob(m.data);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    const isActive = id === activeSessionId;
    // 一度 attach 済みのセッションでは、非アクティブ中も xterm へライブ書き込みする。
    // pendingChunks に貯めるだけだと scanBuffer が古いままで、
    // 承認 UI（カーソル位置制御による再描画）が他カード閲覧中に検出できず
    // sidebar の状態が "保留中" に追従しない。
    if (isActive || t.everAttached) {
      writePTYChunk(id, t.term, bytes, isActive ? () => {
        // ユーザー wheel up 直後（ガード期間中）は autoScroll が一時 true に再セットされていても
        // スクロールアップ意図を尊重して最下部へ吸わない。
        if (t.autoScroll && !isWithinUserScrollUpGuard(t)) t.term.scrollToBottom();
      } : undefined);
    } else {
      t.pendingChunks.push(bytes);
    }
    trackApprovalHintFromChunk(id, bytes);
    if (isActive) scheduleApprovalCheck(id);
    return;
  }

  if (m.type === 'snapshot') {
    const arr = typeof m.sessions === 'string' ? JSON.parse(m.sessions) : m.sessions;
    (arr || []).forEach(s => {
      if (s.state === 'completed') {
        requestSessionDismiss(s.id);
        return;
      }
      sessions.set(s.id, s);
      addToSessionOrder(s.id);
    });
    document.getElementById('summary').textContent = t('connected') || '接続済み';
    renderSessionList();
    checkApprovalOnStartup();
    if (!_elapsedTimerInterval) {
      _elapsedTimerInterval = setInterval(() => renderSessionList(), 1000);
    }
  } else if (m.type === 'session_update') {
    if (m.state === 'completed') {
      requestSessionDismiss(m.session_id);
      removeLocalSession(m.session_id);
      return;
    }
    const isNew = !sessions.has(m.session_id);
    const cur = sessions.get(m.session_id) || { id: m.session_id };
    if (m.provider)        cur.provider        = m.provider;
    if (m.display_name)    cur.display_name    = m.display_name;
    if (m.cwd)             cur.cwd             = m.cwd;
    if (m.branch !== undefined) cur.branch      = m.branch;
    if (m.label !== undefined) cur.label       = m.label;
    if (m.shell)           cur.shell           = m.shell;
    if (m.state)           cur.state           = m.state;
    if (m.last_output_at)  cur.last_output_at  = m.last_output_at;
    if (m.started_at)      cur.started_at      = m.started_at;
    if (m.first_message)   cur.first_message   = m.first_message;
    if (m.last_message)    cur.last_message    = m.last_message;
    if (m.model !== undefined) cur.model       = m.model;
    sessions.set(m.session_id, cur);
    addToSessionOrder(m.session_id, isNew);
    if (isNew && pendingAutoSwitch) {
      pendingAutoSwitch = false;
      activateSession(m.session_id);
    }
  } else if (m.type === 'session_end') {
    if ((m.state || 'disconnected') === 'completed') {
      requestSessionDismiss(m.session_id);
      removeLocalSession(m.session_id);
      return;
    }
    const s = sessions.get(m.session_id);
    if (s) {
      s.state = m.state || 'disconnected';
      if (m.reason) s.end_reason = m.reason;
    }
    cancelApprovalHintConfirm(m.session_id);
    approvalVisibleCache.delete(m.session_id);
    if (multiQuestionVisibleCache.delete(m.session_id) && m.session_id === activeSessionId) {
      setMultiQuestionBannerVisible(false);
    }
    removeApprovalAutoSwitchTarget(m.session_id);
    maybeAutoSwitchToNextApproval();
    const deadStates = ['error', 'disconnected'];
    if (deadStates.includes(m.state) && !autoDismissTimers.has(m.session_id)) {
      const timer = setTimeout(() => {
        autoDismissTimers.delete(m.session_id);
        dismissSession(m.session_id);
      }, 5000);
      autoDismissTimers.set(m.session_id, timer);
    }
  } else if (m.type === 'session_removed') {
    removeLocalSession(m.session_id);
  }

  render();
};

// ---- ターミナルパスリンクポップアップ ----

let pathPopupEl = null;
let pathPopupHideTimer = null;

function getOrCreatePathPopup() {
  if (pathPopupEl) return pathPopupEl;
  pathPopupEl = document.createElement('div');
  pathPopupEl.id = 'path-link-popup';
  pathPopupEl.className = 'path-link-popup';
  pathPopupEl.addEventListener('mouseenter', () => {
    if (pathPopupHideTimer) { clearTimeout(pathPopupHideTimer); pathPopupHideTimer = null; }
  });
  pathPopupEl.addEventListener('mouseleave', () => { scheduleHidePathPopup(); });
  document.body.appendChild(pathPopupEl);
  return pathPopupEl;
}

function scheduleHidePathPopup() {
  if (pathPopupHideTimer) clearTimeout(pathPopupHideTimer);
  pathPopupHideTimer = setTimeout(() => {
    if (pathPopupEl) pathPopupEl.hidden = true;
    pathPopupHideTimer = null;
  }, 300);
}

async function copyPathText(text, anchor) {
  if (!text) return;
  await navigator.clipboard.writeText(text);
  showToast(t('copied_to_clipboard'), anchor);
}

function isImagePath(filePath) {
  return /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(String(filePath || '').trim());
}

function isTextPath(filePath) {
  const path = String(filePath || '').trim();
  if (/(^|[\\/])(Dockerfile|Makefile|README|LICENSE|CHANGELOG|NOTICE)$/i.test(path)) return true;
  return /\.(txt|md|markdown|rst|log|json|jsonl|yaml|yml|toml|ini|cfg|conf|env|csv|tsv|xml|html?|css|scss|sass|less|js|mjs|cjs|jsx|ts|tsx|vue|go|rs|py|rb|php|java|kt|kts|c|cc|cpp|cxx|h|hh|hpp|cs|sh|bash|zsh|fish|ps1|psm1|bat|cmd|sql|graphql|gql|proto|diff|patch|gitignore|gitattributes|editorconfig)$/i.test(path);
}

// ANY-AI-CLI 内蔵プレビューが扱える拡張子（バックエンド /api/files-content の許可リストと一致させること）
function isAnyAiCliPreviewable(filePath) {
  return isTextPath(filePath);
}

function getPathOpenItem(filePath) {
  if (isImagePath(filePath)) {
    return { icon: '🖼️', key: 'link_open_image', action: () => callOpenApi('/api/open-default-file', filePath, 'link_open_default_error') };
  }
  if (isTextPath(filePath)) {
    return { icon: '📝', key: 'link_open_text', action: () => callOpenApi('/api/open-file', filePath) };
  }
  return { icon: '📄', key: 'link_open_file', action: () => callOpenApi('/api/open-default-file', filePath, 'link_open_default_error') };
}

function showPathPopup(filePath, clientX, clientY, sessionId) {
  const popup = getOrCreatePathPopup();
  popup.innerHTML = '';
  popup.hidden = false;

  const items = [];
  if (isAnyAiCliPreviewable(filePath)) {
    items.push({
      icon: '📖',
      key: 'link_open_any_ai_cli',
      action: () => {
        const ses = sessions.get(sessionId);
        const cwd = ses?.cwd;
        if (!cwd) { showToast(t('link_open_error')); return; }
        const projectKey = cwd.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || cwd;
        FilesTabManager.openFilesTabAtFile(sessionId, projectKey, cwd, cwd, filePath);
      },
    });
  }
  items.push(
    getPathOpenItem(filePath),
    { icon: '📁', key: 'link_open_folder', action: () => callOpenApi('/api/open-folder', filePath) },
    { icon: '💻', key: 'link_open_terminal', action: () => {
      const dir = dirnameForPath(filePath) || sessions.get(sessionId)?.cwd || filePath;
      callOpenApi('/api/open-terminal', dir);
    }},
    { icon: '📋', key: 'link_copy_path', action: (anchor) => {
      return copyPathText(filePath, anchor).catch(() => {});
    }},
    { icon: '📋', key: 'link_copy_rel_path', action: (anchor) => {
      const ses = sessions.get(sessionId);
      const cwd = ses?.cwd || '';
      const rel = cwd ? computeRelPath(cwd, filePath) : filePath;
      return copyPathText(rel, anchor).catch(() => {});
    }},
  );

  for (const item of items) {
    const btn = document.createElement('button');
    btn.className = 'path-link-popup-item';
    btn.textContent = item.icon + ' ' + t(item.key);
    btn.addEventListener('click', () => {
      Promise.resolve(item.action(btn)).finally(() => {
        popup.hidden = true;
      });
    });
    popup.appendChild(btn);
  }

  // 位置調整: 画面端からはみ出さないようにする
  popup.style.left = '0';
  popup.style.top = '0';
  document.body.appendChild(popup);
  const rect = popup.getBoundingClientRect();
  const vw = window.innerWidth, vh = window.innerHeight;
  let left = clientX + 8;
  let top = clientY + 8;
  if (left + rect.width > vw - 8) left = clientX - rect.width - 8;
  if (top + rect.height > vh - 8) top = clientY - rect.height - 8;
  popup.style.left = Math.max(4, left) + 'px';
  popup.style.top = Math.max(4, top) + 'px';
}

async function callOpenApi(endpoint, path, errorKey = 'link_open_error') {
  try {
    const res = await fetch(`${endpoint}?token=${encodeURIComponent(token)}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path }),
    });
    if (res.ok) {
      const data = await res.json();
      if (!data.ok) showToast(data.error ? `${t(errorKey)}: ${data.error}` : t(errorKey));
    } else {
      showToast(t(errorKey));
    }
  } catch (_) { showToast(t(errorKey)); }
}

function dirnameForPath(filePath) {
  const normalized = String(filePath || '').replace(/[\\/]+$/, '');
  const slash = Math.max(normalized.lastIndexOf('/'), normalized.lastIndexOf('\\'));
  if (slash <= 0) return '';
  if (/^[A-Za-z]:[\\/]?[^\\/]*$/.test(normalized)) return normalized.slice(0, slash + 1);
  return normalized.slice(0, slash);
}

function computeRelPath(from, to) {
  // OS セパレータを / に正規化
  const sep = to.includes('\\') ? '\\' : '/';
  const fromParts = from.replace(/\\/g, '/').split('/').filter(Boolean);
  const toParts = to.replace(/\\/g, '/').split('/').filter(Boolean);
  let common = 0;
  while (common < fromParts.length && common < toParts.length && fromParts[common] === toParts[common]) common++;
  const ups = fromParts.length - common;
  const rel = [...Array(ups).fill('..'), ...toParts.slice(common)].join(sep);
  return rel || '.';
}

function trimTerminalPathCandidate(path) {
  let text = String(path || '').trim().replace(/[,;'"<>\])}]+$/, '');
  if (/^[A-Za-z]:[\\/]/.test(text)) text = trimWindowsPathCandidate(text);
  text = stripTerminalLineSuffix(text);
  return text;
}

function trimWindowsPathCandidate(path) {
  let text = String(path || '');
  text = text.replace(/([\\/])\s+.*$/, '$1');
  text = text.replace(/(\.[a-zA-Z0-9]{1,15})\s*[\u3040-\u30ff\u3400-\u9fff\uff00-\uffef\u4e00-\u9fff].*$/u, '$1');
  text = text.replace(/\s+[\u3040-\u30ff\u3400-\u9fff\uff00-\uffef].*$/u, '');
  text = text.replace(/\s+[A-Za-z]$/, '');
  return text.replace(/[,;'"<>\])}]+$/, '');
}

function stripTerminalLineSuffix(path) {
  const text = String(path || '');
  return text.replace(/(\.[A-Za-z0-9]{1,15}):\d+(?::\d+)?$/, '$1');
}

function isAbsolutePath(path) {
  return /^[A-Za-z]:[\\/]/.test(path) || path.startsWith('/');
}

function joinPath(base, rel) {
  if (!base || !rel) return rel || base || '';
  const sep = base.includes('\\') ? '\\' : '/';
  const baseNorm = base.replace(/[\\/]+$/, '');
  return normalizePathSegments(baseNorm + sep + rel.replace(/^[\\/]+/, '').replace(/[\\/]/g, sep), sep);
}

function normalizePathSegments(path, sep) {
  const drive = /^[A-Za-z]:/.test(path) ? path.slice(0, 2) : '';
  const rest = drive ? path.slice(2) : path;
  const rooted = rest.startsWith(sep);
  const parts = rest.split(/[\\/]+/);
  const out = [];
  for (const part of parts) {
    if (!part || part === '.') continue;
    if (part === '..' && out.length > 0 && out[out.length - 1] !== '..') out.pop();
    else if (part !== '..' || !rooted) out.push(part);
  }
  return drive + (rooted ? sep : '') + out.join(sep);
}

function resolveTerminalPathCandidate(path, sessionId) {
  const cleaned = trimTerminalPathCandidate(path);
  if (!cleaned) return '';
  if (isAbsolutePath(cleaned)) return cleaned;
  const cwd = sessions.get(sessionId)?.cwd || '';
  if (!cwd) return cleaned;
  return joinPath(cwd, cleaned);
}

const ABS_WIN_PATH_RE = /([A-Za-z]:\\(?:(?!\s+[A-Za-z]:\\)[^\x00-\x1f<>:"|?*])+)/g;
// 空白を挟んだ説明文中の区切り（例: "hljs / highlight / prism"）を
// Unix 絶対パスとして誤検出しないよう、セグメント内の空白は許可しない。
const ABS_UNIX_PATH_RE = /(\/[^\s\/\x00-\x1f"'<>`|]+(?:\/[^\s\/\x00-\x1f"'<>`|]*)*)/g;
const REL_PATH_RE = /(^|[\s([{"'`])((?:\.{1,2}[\\/]|[A-Za-z0-9_.-]+[\\/])(?:[^\s\x00-\x1f"'<>`|]+[\\/])*[^\s\x00-\x1f"'<>`|]+)/g;

function isTerminalPathStartBoundary(text, start) {
  if (start <= 0) return true;
  return /[\s([{"'`]/.test(text[start - 1] || '');
}

function findPathCandidates(text) {
  const candidates = [];
  for (const re of [ABS_WIN_PATH_RE, ABS_UNIX_PATH_RE]) {
    re.lastIndex = 0;
    let m;
    while ((m = re.exec(text)) !== null) {
      if (re === ABS_UNIX_PATH_RE && !isTerminalPathStartBoundary(text, m.index)) continue;
      const pathStr = trimTerminalPathCandidate(m[1]);
      if (pathStr.length >= 3) candidates.push({ start: m.index, end: m.index + pathStr.length, text: pathStr });
    }
  }
  REL_PATH_RE.lastIndex = 0;
  let m;
  while ((m = REL_PATH_RE.exec(text)) !== null) {
    const pathStr = trimTerminalPathCandidate(m[2]);
    if (pathStr.length >= 3) candidates.push({ start: m.index + m[1].length, end: m.index + m[1].length + pathStr.length, text: pathStr });
  }
  candidates.sort((a, b) => a.start - b.start || b.end - a.end);
  const out = [];
  for (const c of candidates) {
    if (out.some(x => c.start < x.end && c.end > x.start)) continue;
    out.push(c);
  }
  return out;
}

function appendLinkedText(container, text, sessionId) {
  const candidates = findPathCandidates(text);
  if (candidates.length === 0) {
    container.textContent = text;
    return;
  }
  container.textContent = '';
  let pos = 0;
  for (const c of candidates) {
    if (c.start > pos) container.appendChild(document.createTextNode(text.slice(pos, c.start)));
    const resolvedPath = resolveTerminalPathCandidate(c.text, sessionId);
    const link = document.createElement('span');
    link.className = 'tool-output-path-link';
    link.textContent = c.text;
    link.tabIndex = 0;
    link.addEventListener('mouseenter', (e) => showPathPopup(resolvedPath, e.clientX, e.clientY, sessionId));
    link.addEventListener('mouseleave', () => scheduleHidePathPopup());
    link.addEventListener('click', (e) => showPathPopup(resolvedPath, e.clientX, e.clientY, sessionId));
    link.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        const r = link.getBoundingClientRect();
        showPathPopup(resolvedPath, r.left, r.bottom, sessionId);
      }
    });
    container.appendChild(link);
    pos = c.end;
  }
  if (pos < text.length) container.appendChild(document.createTextNode(text.slice(pos)));
}

// ---- xterm.js 管理 ----

function ensureTerminal(id) {
  if (terminals.has(id)) return;
  const provider = sessions.get(id)?.provider;
  const term = new Terminal({
    cursorBlink: false,
    scrollback: 5000,
    // xterm はセル幅ベースで描画するため、絵文字フォント混在でグリフが巨大化/崩れする環境がある。
    // 端末領域は等幅フォントのみを使い、見た目の安定性を優先する。
    fontFamily: '"Cascadia Mono", "Cascadia Code", "BIZ UDゴシック", "MS Gothic", Consolas, "Courier New", monospace',
    fontSize: FONTSIZE_MAP[localStorage.getItem(STORAGE_FONTSIZE_KEY)] || 13,
    // 一部フォントで大文字上端がクリップされるため、行高を少し広げて回避する。
    lineHeight: 1.25,
    windowsPty: { backend: 'conpty' },
    // cursor を背景色と同色にしてブロックカーソルを不可視化する。
    // 'transparent' はブラウザ実装によって輪郭線だけ □ として描画されることがある。
    theme: { background: '#0d1117', cursor: '#0d1117', cursorAccent: '#e6edf3' },
    disableStdin: true,
    allowProposedApi: true,
  });
  const fitAddon = new FitAddon.FitAddon();
  term.loadAddon(fitAddon);
  if (typeof WebLinksAddon !== 'undefined') {
    const webLinks = new WebLinksAddon.WebLinksAddon((event, uri) => {
      event.preventDefault();
      window.open(uri, '_blank', 'noopener');
    });
    term.loadAddon(webLinks);
  }
  term.registerLinkProvider({
    provideLinks(y, callback) {
      const line = term.buffer.active.getLine(y - 1);
      if (!line) { callback([]); return; }
      const text = line.translateToString(true);

      // charIndex → cellX マッピング（全角文字対応）
      const cellMap = [];
      for (let x = 0; x < line.length; x++) {
        const cell = line.getCell(x);
        if (cell && cell.getWidth() !== 0) cellMap.push(x);
      }
      const toX = (ci) => (cellMap[ci] ?? ci) + 1;
      const toEndX = (ci) => (cellMap[ci] ?? ci) + 1;

      const links = [];
      const occupiedRanges = [];
      const overlapsExistingLink = (start, end) => occupiedRanges.some(r => start <= r.end && end >= r.start);
      const addPathLink = (rawPath, startCI) => {
        const pathStr = trimTerminalPathCandidate(rawPath);
        if (pathStr.length < 3) return;
        const endCI = startCI + pathStr.length - 1;
        if (overlapsExistingLink(startCI, endCI)) return;
        occupiedRanges.push({ start: startCI, end: endCI });
        const capturedPath = resolveTerminalPathCandidate(pathStr, id);
        links.push({
          range: { start: { x: toX(startCI), y }, end: { x: toEndX(endCI), y } },
          text: pathStr,
          hover(event, _text) {
            showPathPopup(capturedPath, event.clientX, event.clientY, id);
          },
          leave() {
            scheduleHidePathPopup();
          },
          activate(_event, _text) {
            // クリックでポップアップが閉じていた場合は再表示
            showPathPopup(capturedPath, _event.clientX, _event.clientY, id);
          }
        });
      };

      for (const re of [ABS_WIN_PATH_RE, ABS_UNIX_PATH_RE]) {
        re.lastIndex = 0;
        let m;
        while ((m = re.exec(text)) !== null) {
          if (re === ABS_UNIX_PATH_RE && !isTerminalPathStartBoundary(text, m.index)) continue;
          addPathLink(m[1], m.index);
        }
      }
      let m;
      REL_PATH_RE.lastIndex = 0;
      while ((m = REL_PATH_RE.exec(text)) !== null) {
        const rawPath = m[2];
        const startCI = m.index + m[1].length;
        addPathLink(rawPath, startCI);
      }
      callback(links);
    }
  });
  if (typeof Unicode11Addon !== 'undefined') {
    const u11 = new Unicode11Addon.Unicode11Addon();
    term.loadAddon(u11);
    term.unicode.activeVersion = '11';
  }
  terminals.set(id, { term, fitAddon, container: null, pendingChunks: [], pendingTextTail: '', markerFilterCarry: new Uint8Array(0), autoScroll: true, stickToBottomOnNextFit: false, everAttached: false, userScrolledUpAt: 0 });
}

function attachTerminal(id) {
  const area = document.getElementById('terminal-area');
  if (!area) return;
  const t = terminals.get(id);
  if (!t) return;
  // xterm container の wheel リスナは container 内（≒キャンバス上）でしか発火しない。
  // セッション切替直後は inputEl.focus() でマウス視線が入力欄付近に移り、その上で wheel
  // しても #terminal-area-wrapper 外のため何も起きず「画面が固定」と感じる事象が起きる。
  // よって最上位 #terminal-wrapper に wheel を仕込み、xterm container 外の wheel は
  // term.scrollLines で xterm 本体を直接スクロールさせる。
  const termWrapper = document.getElementById('terminal-wrapper');
  const inputElForWheel = document.getElementById('input');
  if (termWrapper && !termWrapper._wheelBound) {
    termWrapper._wheelBound = true;
    termWrapper.addEventListener('wheel', (e) => {
      if (activeSessionId === null) return;
      const tActive = terminals.get(activeSessionId);
      if (!tActive || !tActive.term) return;
      // xterm container 内は xterm 自身が wheel を処理する（二重スクロール防止）。
      // autoScroll の切替は line 1584 の container 上リスナで行われる。
      if (tActive.container && tActive.container.contains(e.target)) return;
      // textarea が複数行で自前スクロール可能なら textarea を優先。
      if (inputElForWheel && inputElForWheel.contains(e.target)
          && inputElForWheel.scrollHeight > inputElForWheel.clientHeight + 1) {
        return;
      }
      // alternate screen buffer（Codex の TUI 等）では scrollLines が効かないため、
      // PgUp/PgDn を PTY へ送って TUI 側にスクロールを委ねる。
      if (forwardWheelToAltBuffer(activeSessionId, tActive, e.deltaY)) return;
      const lineHeight = 24;
      const lines = Math.sign(e.deltaY) * Math.max(1, Math.round(Math.abs(e.deltaY) / lineHeight));
      try { tActive.term.scrollLines(lines); } catch (_) {}
      if (e.deltaY < 0) {
        tActive.autoScroll = false;
        // forceTerminalToBottom / onFlush の "無条件再追従" によって直後に巻き戻されるのを抑止する。
        // セッション切替→inputEl.focus() 直後の 2 段 RAF（forceTerminalToBottomAfterLayout）と
        // 継続中の PTY ストリームの両方をガードする。
        tActive.userScrolledUpAt = performance.now();
        updateScrollLockBtn(true);
      }
      // 下方向 wheel が最下部に達した場合は onScroll で autoScroll=true へ復帰する。
    }, { passive: true });
  }
  if (t.container) {
    area.innerHTML = '';
    area.appendChild(t.container);
    t.autoScroll = true;
    updateScrollLockBtn(false);
    requestAnimationFrame(() => {
      if (!terminals.has(id)) return;
      // 非アクティブ中の chunks は、その間 PTY が使っていた旧 cols/rows のまま先に反映する。
      // 先に fit すると、TUI の古い幅の再描画フレームが別幅で解釈されて上部に残像が出る。
      flushPending(id);
      const prevCols = t.term.cols;
      const prevRows = t.term.rows;
      fitTerminalPreservingBottom(t, id);
      // 寸法が実際に変わった場合のみ送信（不要な SIGWINCH → 再描画 → 空白行挿入を防ぐ）
      if (t.term.cols !== prevCols || t.term.rows !== prevRows) {
        sendResize(id, t.term.cols, t.term.rows);
      }
    });
    return;
  }
  const container = document.createElement('div');
  container.style.width = '100%';
  container.style.height = '100%';
  t.container = container;
  area.innerHTML = '';
  area.appendChild(container);
  whenLayoutReady(id, container);
}

function whenLayoutReady(id, container) {
  const t = terminals.get(id);
  if (!t) return;
  if (container.clientWidth > 0 && container.clientHeight > 0) {
    t.term.open(container);
    fitTerminalPreservingBottom(t, id);
    t.term.onScroll(() => {
      const buf = t.term.buffer.active;
      const atBottom = buf.viewportY + t.term.rows >= buf.length;
      if (atBottom) {
        t.autoScroll = true;
        t.userScrolledUpAt = 0;
        if (id === activeSessionId) updateScrollLockBtn(false);
      } else {
        t.autoScroll = false;
        if (id === activeSessionId) updateScrollLockBtn(true);
      }
    });
    container.addEventListener('wheel', (e) => {
      // alt buffer 中は xterm 内 wheel も scrollLines が no-op になるため、
      // ここで PgUp/PgDn を PTY 側へ転送し、autoScroll 操作は skip する。
      if (forwardWheelToAltBuffer(id, t, e.deltaY)) return;
      if (e.deltaY < 0) {
        t.autoScroll = false;
        t.userScrolledUpAt = performance.now();
        if (id === activeSessionId) updateScrollLockBtn(true);
      }
    }, { passive: true });
    flushPending(id);
    t.everAttached = true;
    sendResize(id, t.term.cols, t.term.rows);
    container.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      const sel = t.term.getSelection();
      if (sel) copyCleanText(sel, e).catch(() => {});
    });
  } else {
    requestAnimationFrame(() => whenLayoutReady(id, container));
  }
}

function flushPending(id) {
  const t = terminals.get(id);
  if (!t) return;
  const chunks = t.pendingChunks;
  t.pendingChunks = [];
  if (chunks.length === 0) {
    if (!isWithinUserScrollUpGuard(t)) t.term.scrollToBottom();
    scheduleApprovalCheck(id);
    return;
  }
  // writeUtf8 の onFlush は xterm 内部キューが drain した後に発火する。
  // 同期で scrollToBottom してしまうと未反映の行ぶん viewport が上に取り残されるため、
  // 最後の chunk の onFlush 内で最下部固定する。
  for (let i = 0; i < chunks.length; i++) {
    const isLast = i === chunks.length - 1;
    writePTYChunk(id, t.term, chunks[i], isLast ? () => {
      if (!isWithinUserScrollUpGuard(t)) t.term.scrollToBottom();
      scheduleApprovalCheck(id);
    } : undefined);
  }
}

function isTerminalAtBottom(t) {
  if (!t || !t.term || !t.term.buffer) return true;
  const buf = t.term.buffer.active;
  return buf.viewportY + t.term.rows >= buf.length;
}

function fitTerminalPreservingBottom(t, id) {
  if (!canFitTerminal(t)) return;
  const shouldStickToBottom = !!(t && (t.autoScroll || t.stickToBottomOnNextFit || isTerminalAtBottom(t)));
  if (t) t.stickToBottomOnNextFit = false;
  t.fitAddon.fit();
  if (shouldStickToBottom && !isWithinUserScrollUpGuard(t)) {
    t.autoScroll = true;
    t.term.scrollToBottom();
    if (id === activeSessionId) updateScrollLockBtn(false);
  }
}

// 直前にユーザーが wheel up したセッションでは "無条件追従" を一時的に止める。
// セッションカード切替の forceTerminalToBottomAfterLayout（2段RAF）や、
// 継続する PTY ストリームの onFlush による snap-back を抑止するためのガード。
const USER_SCROLL_UP_GUARD_MS = 800;
function isWithinUserScrollUpGuard(t) {
  if (!t || !t.userScrolledUpAt) return false;
  return (performance.now() - t.userScrolledUpAt) < USER_SCROLL_UP_GUARD_MS;
}

// xterm が alternate screen buffer（TUI モード, Codex 等）に居るかを判定。
// alt buffer は scrollback を持たないため term.scrollLines は no-op となり、
// 修正①〜⑥の wheel 経路はすべてこのバッファでは履歴を遡れない。
function isAlternateBuffer(t) {
  if (!t || !t.term || !t.term.buffer) return false;
  const active = t.term.buffer.active;
  return !!(active && active.type === 'alternate');
}

// alt buffer 中の wheel は PTY 側アプリ（Codex の TUI 等）に PgUp/PgDn として転送する。
// 戻り値: 転送した場合 true（呼び元はそれ以降の autoScroll 操作等をスキップ）。
// mouse tracking が ON の場合は xterm が wheel を mouse escape として送るため二重送信を避ける。
function forwardWheelToAltBuffer(sessionId, t, deltaY) {
  if (!isAlternateBuffer(t)) return false;
  try {
    const mode = t.term.modes && t.term.modes.mouseTrackingMode;
    if (mode && mode !== 'none') return true; // xterm 自身が送るのでこちらは何もしない（が転送扱いで他処理を抑止）
  } catch (_) {}
  const key = deltaY < 0 ? '\x1b[5~' : '\x1b[6~';
  try { sendText(sessionId, key); } catch (_) {}
  return true;
}

// セッションカード切替直後はマウスポインタが `#session-list` 上に残ったままになりがちで、
// その状態で wheel すると `#session-list` と `#terminal-wrapper` は兄弟要素のため
// `#terminal-wrapper` のリスナにバブリングせずターミナルが固まって見える。
// 切替時刻を記録し、グレース期間中の session-list 上 wheel をアクティブターミナルへ転送する。
const POST_SWITCH_WHEEL_GRACE_MS = 1500;
let _activeSessionSwitchedAt = 0;

function forceTerminalToBottom(id) {
  const t = terminals.get(id);
  if (!t || !t.term) return;
  if (isWithinUserScrollUpGuard(t)) return;
  t.autoScroll = true;
  t.term.scrollToBottom();
  if (id === activeSessionId) updateScrollLockBtn(false);
}

function forceTerminalToBottomAfterLayout(id) {
  forceTerminalToBottom(id);
  requestAnimationFrame(() => {
    forceTerminalToBottom(id);
    requestAnimationFrame(() => forceTerminalToBottom(id));
  });
}

function refitActiveTerminalAfterLayout(stickToBottom) {
  if (activeSessionId === null) return;
  const id = activeSessionId;
  const t = terminals.get(id);
  if (!canFitTerminal(t)) return;
  if (stickToBottom) {
    t.autoScroll = true;
    t.stickToBottomOnNextFit = true;
  }
  requestAnimationFrame(() => {
    if (activeSessionId !== id || !canFitTerminal(t)) return;
    const prevCols = t.term.cols;
    const prevRows = t.term.rows;
    fitTerminalPreservingBottom(t, id);
    if (t.term.cols !== prevCols || t.term.rows !== prevRows) {
      sendResize(id, t.term.cols, t.term.rows);
    }
    if (stickToBottom) forceTerminalToBottomAfterLayout(id);
  });
}

function updateScrollLockBtn(_locked) {
  // ボタンはアクティブセッションがある間は常時表示する。
  // 以前は viewportY で「最上部/最下部判定して hidden」していたが、
  // xterm の onScroll は viewportY が動いた時しか発火せず、
  // PTY 出力でバッファが伸びた場合などにボタン表示が更新されない事象があった。
  // 常時表示なら更新タイミングに依存しないし、すでに端に居る時に再度押しても無害。
  const topBtn = document.getElementById('scroll-to-top-btn');
  const bottomBtn = document.getElementById('scroll-to-bottom-btn');
  const hasSession = activeSessionId !== null && terminals.has(activeSessionId);
  if (topBtn) topBtn.hidden = !hasSession;
  if (bottomBtn) bottomBtn.hidden = !hasSession;
}

document.getElementById('scroll-to-top-btn')?.addEventListener('click', () => {
  if (activeSessionId === null) return;
  const t = terminals.get(activeSessionId);
  if (!t) return;
  t.autoScroll = false;
  t.term.scrollToTop();
  updateScrollLockBtn(true);
});

document.getElementById('scroll-to-bottom-btn')?.addEventListener('click', () => {
  if (activeSessionId === null) return;
  const t = terminals.get(activeSessionId);
  if (!t) return;
  t.autoScroll = true;
  t.userScrolledUpAt = 0;
  t.term.scrollToBottom();
  updateScrollLockBtn(false);
});

const hubMarkerBytePatterns = [
  new TextEncoder().encode('[ANY-AI-CLI]'),
  new TextEncoder().encode('[/ANY-AI-CLI]'),
];
const hubMarkerEndBytes = hubMarkerBytePatterns[1];
const eraseDisplayBelowBytes = new TextEncoder().encode('\x1b[J');

function bytesStartWith(bytes, offset, pattern) {
  if (offset + pattern.length > bytes.length) return false;
  for (let i = 0; i < pattern.length; i++) {
    if (bytes[offset + i] !== pattern[i]) return false;
  }
  return true;
}

function isPossibleMarkerPrefix(bytes, offset) {
  const remaining = bytes.length - offset;
  return hubMarkerBytePatterns.some((pattern) => {
    if (remaining >= pattern.length) return false;
    for (let i = 0; i < remaining; i++) {
      if (bytes[offset + i] !== pattern[i]) return false;
    }
    return true;
  });
}

function filterHubMarkersForDisplay(id, bytes) {
  const t = terminals.get(id);
  if (!t) return bytes;
  const carry = t.markerFilterCarry || new Uint8Array(0);
  const combined = new Uint8Array(carry.length + bytes.length);
  combined.set(carry, 0);
  combined.set(bytes, carry.length);

  const out = [];
  let i = 0;
  while (i < combined.length) {
    const marker = hubMarkerBytePatterns.find(pattern => bytesStartWith(combined, i, pattern));
    if (marker) {
      i += marker.length;
      if (marker === hubMarkerEndBytes) {
        for (const b of eraseDisplayBelowBytes) out.push(b);
      }
      continue;
    }
    if (isPossibleMarkerPrefix(combined, i)) break;
    out.push(combined[i]);
    i++;
  }

  t.markerFilterCarry = combined.slice(i);
  return new Uint8Array(out);
}

function writePTYChunk(id, term, bytes, onFlush) {
  const displayBytes = filterHubMarkersForDisplay(id, bytes);
  if (displayBytes.length === 0) {
    if (onFlush) onFlush();
    return;
  }
  if (typeof term.writeUtf8 === 'function') {
    term.writeUtf8(displayBytes, onFlush);
    return;
  }
  term.write(utf8Decoder.decode(displayBytes, { stream: true }), onFlush);
}

// ---- バッファスキャン共通 ----

function scanBuffer(id) {
  const t = terminals.get(id);
  if (!t || !t.term.buffer) return [];
  const buf = t.term.buffer.active;
  const lines = [];
  for (let i = 0; i < buf.length; i++) {
    lines.push(buf.getLine(i)?.translateToString(true) || '');
  }
  return lines;
}

// ---- 複数質問 UI（Claude Code AskUserQuestion 等）検出 ----
// タブで質問を切替えながら複数項目を回答するタイプの TUI は、
// Hub の単一質問 action-bar では駆動できないため、検出して action-bar を抑制し、
// ターミナル直接操作を促すバナーを出す。

const reviewAnswersRe = /Review your answers/i;
const readyToSubmitAnswersRe = /Ready to submit your answers/i;
const tabBoxMarkerRe = /[◻□☐✓☑]/; // ◻ □ ☐ ✓ ☑

function isMultiQuestionPrompt(lines) {
  for (const line of lines) {
    if (!line) continue;
    if (reviewAnswersRe.test(line)) return true;
    if (readyToSubmitAnswersRe.test(line)) return true;
    // タブストリップ: 同一行に ← と → の両方があり、タブマーカーまたは Submit を含む
    if (line.indexOf('←') !== -1 && line.indexOf('→') !== -1 &&
        (tabBoxMarkerRe.test(line) || /\bSubmit\b/.test(line))) {
      return true;
    }
  }
  return false;
}

function setMultiQuestionBannerVisible(visible) {
  const banner = document.getElementById('multi-question-banner');
  if (!banner) return;
  if (visible) {
    banner.textContent = t('multi_question_banner');
    banner.hidden = false;
  } else {
    banner.hidden = true;
  }
}

// ---- 承認検出 (xterm.js バッファスキャン) ----

// provider 別の承認 trigger phrase は ~/.any-ai-cli/approval-patterns/{provider}.json に外出し。
// Hub 起動時にデフォルトをユーザー設定ディレクトリに展開（既存ファイルは尊重）し、
// HTTP 経由で配信する。ユーザーが直接編集して文言を追加・調整できる。
// claude / codex は英語固定（Anthropic/OpenAI が国際化していない）、common は多言語混在。
const providerApprovalTriggers = { claude: [], codex: [], common: [] };

(async function loadApprovalPatterns() {
  const fetchJson = async (name) => {
    try {
      const res = await fetch(`approval-patterns/${name}.json`);
      if (!res.ok) return [];
      return await res.json();
    } catch (e) {
      console.warn(`approval-patterns/${name}.json load failed`, e);
      return [];
    }
  };
  const [claude, codex, common] = await Promise.all([
    fetchJson('claude'), fetchJson('codex'), fetchJson('common'),
  ]);
  const norm = arr => (Array.isArray(arr) ? arr : []).map(s => String(s).toLowerCase()).filter(Boolean);
  providerApprovalTriggers.claude = norm(claude);
  providerApprovalTriggers.codex  = norm(codex);
  providerApprovalTriggers.common = norm(common);
})();

function matchProviderApprovalTrigger(provider, line) {
  if (!line) return false;
  const lower = String(line).toLowerCase();
  const list = providerApprovalTriggers[provider] || [];
  for (const s of list) if (lower.includes(s)) return true;
  for (const s of providerApprovalTriggers.common) if (lower.includes(s)) return true;
  return false;
}

const userSpecifiesRe = /user specifies|その他指定/i;
const hubChoiceQuestionRe = /どれで進めますか|どれで進める|どちらで進め|どの選択肢|選択してください|how would you like to proceed|which option/i;
const recommendedChoiceRe = /\(recommended\)|（recommended）|推奨/i;
let approvalCheckTimer = null;

function cancelApprovalHintConfirm(id) {
  const timer = approvalHintConfirmTimers.get(id);
  if (timer) {
    clearTimeout(timer);
    approvalHintConfirmTimers.delete(id);
  }
}

function scheduleApprovalHintConfirm(id, options) {
  if (!options || options.length === 0) return;
  const sig = approvalSig(options);
  cancelApprovalHintConfirm(id);
  approvalHintConfirmTimers.set(id, setTimeout(() => {
    approvalHintConfirmTimers.delete(id);
    const cached = approvalRawOptionsCache.get(id);
    if (!cached || cached.length === 0) return;
    const cachedSig = approvalSig(cached);
    if (cachedSig !== sig) return;
    const wasVisible = approvalVisibleCache.get(id);
    if (!wasVisible) {
      approvalVisibleCache.set(id, true);
      enqueueApprovalAutoSwitch(id);
      playNotificationSound();
      ws.send(JSON.stringify({ type: 'session_hint', session_id: id, approval_visible: true }));
    }
    // 連続して [ANY-AI-CLI] ブロックが来た場合 (例: 1質問目を回答せず 2質問目が来た) は
    // 既に approvalVisible=true でも action-bar を最新オプションに張り替える。
    if (id === activeSessionId) {
      const bar = document.getElementById('action-bar');
      if (bar) showActionBar(bar, id, cached, false);
    }
  }, 350));
}

function isHubChoicePrompt(contextLines, options) {
  if (!options.length) return false;
  const hasPrompt = contextLines.some(line => hubChoiceQuestionRe.test(line));
  const hasChoiceMarker = contextLines.some(line => userSpecifiesRe.test(line)) ||
    options.some(opt => userSpecifiesRe.test(opt.label) || recommendedChoiceRe.test(opt.label));
  return hasPrompt && hasChoiceMarker;
}

function markHubChoiceDefault(options, contextLines) {
  if (options.some(o => o.isCurrent) || !isHubChoicePrompt(contextLines, options)) return;
  const recommended = options.find(o => recommendedChoiceRe.test(o.label)) || options.find(o => o.num === 1) || options[0];
  if (recommended) recommended.isCurrent = true;
}

function trackApprovalHintFromChunk(id, bytes) {
  const t = terminals.get(id);
  if (!t) return;
  const provider = sessions.get(id)?.provider;
  const text = new TextDecoder('utf-8').decode(bytes);
  t.pendingTextTail = (t.pendingTextTail + text).slice(-APPROVAL_PENDING_TEXT_TAIL_LIMIT);

  // sendChoice 直後の誤再表示を抑制
  const suppressUntil = approvalSuppressUntil.get(id);
  if (suppressUntil && Date.now() < suppressUntil) return;
  approvalSuppressUntil.delete(id);

  const rawLines = t.pendingTextTail.split(/\r\n|\r|\n/).slice(-40);
  const lines = rawLines.map(l => stripAnsi(l));

  // 複数質問 UI を最優先で判定 — Hub の action-bar では正しく駆動できないので
  // 検出したら通常の承認検出をスキップする。スクロールバック残骸での誤検出を避けるため
  // ターミナル末尾 40 行に限定する（AskUserQuestion UI は通常 ~20 行以内に収まる）。
  let multiQContext = lines;
  if (t && t.everAttached) {
    multiQContext = lines.concat(scanBuffer(id).slice(-40));
  }
  if (isMultiQuestionPrompt(multiQContext)) {
    cancelApprovalHintConfirm(id);
    approvalRawOptionsCache.delete(id);
    if (!multiQuestionVisibleCache.get(id)) {
      multiQuestionVisibleCache.set(id, true);
      if (id === activeSessionId) setMultiQuestionBannerVisible(true);
      // 待機通知を Hub と同期（auto-switch とサウンドは action-bar と同等の扱い）
      if (!approvalVisibleCache.get(id)) {
        approvalVisibleCache.set(id, true);
        enqueueApprovalAutoSwitch(id);
        playNotificationSound();
        ws.send(JSON.stringify({ type: 'session_hint', session_id: id, approval_visible: true }));
      }
    } else if (id === activeSessionId) {
      setMultiQuestionBannerVisible(true);
    }
    return;
  }

  // フォーマットベース検出（優先）: [ANY-AI-CLI] マーカーがあれば即確定
  const markerOpts = extractHubMarkerApproval(lines);
  if (markerOpts) {
    // doSend でテキスト送信済みの承認が Ink 再描画で再検出された場合はスキップ
    const consumed = approvalConsumedSig.get(id);
    const sig = approvalSig(markerOpts);
    if (consumed === sig) {
      // Ink 再描画で同一ブロックが再送されている — タイマーをリセットして
      // ブロックが届かなくなるまで sig を保持し続ける（debounce 型削除）
      const prev = approvalConsumedSigDeleteTimer.get(id);
      if (prev) clearTimeout(prev);
      approvalConsumedSigDeleteTimer.set(id, setTimeout(() => {
        approvalConsumedSig.delete(id);
        approvalConsumedSigDeleteTimer.delete(id);
      }, 5000));
      return;
    }
    // 異なる選択肢 → 新しい質問なのでリセット
    const prevTimer = approvalConsumedSigDeleteTimer.get(id);
    if (prevTimer) { clearTimeout(prevTimer); approvalConsumedSigDeleteTimer.delete(id); }
    approvalConsumedSig.delete(id);
    approvalRawOptionsCache.set(id, markerOpts);
    scheduleApprovalHintConfirm(id, markerOpts);
    return;
  }

  const plainYesNoOpts = extractPlainYesNoApproval(lines);
  if (plainYesNoOpts) {
    const consumed = approvalConsumedSig.get(id);
    const sig = approvalSig(plainYesNoOpts);
    if (consumed === sig) {
      const prev = approvalConsumedSigDeleteTimer.get(id);
      if (prev) clearTimeout(prev);
      approvalConsumedSigDeleteTimer.set(id, setTimeout(() => {
        approvalConsumedSig.delete(id);
        approvalConsumedSigDeleteTimer.delete(id);
      }, 5000));
      return;
    }
    const prevTimer = approvalConsumedSigDeleteTimer.get(id);
    if (prevTimer) { clearTimeout(prevTimer); approvalConsumedSigDeleteTimer.delete(id); }
    approvalConsumedSig.delete(id);
    approvalRawOptionsCache.set(id, plainYesNoOpts);
    scheduleApprovalHintConfirm(id, plainYesNoOpts);
    return;
  }

  // フォールバック検出（既存）
  let extraction = extractApprovalOptions(lines);
  const options = extraction.options;
  let contextSourceLines = lines;
  let contextCluster = extraction.cluster;

  // 非アクティブセッション含めて xterm の解釈済みバッファ (scanBuffer) も参照する。
  // Ink/Codex のカーソル位置制御による再描画は pendingTextTail を行分割しても
  // カーソル付き選択肢を取り出せないため、xterm 解釈済みのバッファのほうが
  // より正確に行構造とカーソル位置を保持している。
  if (t && t.everAttached) {
    const visibleRows = t.term?.rows || 40;
    const bufferTail = scanBuffer(id).slice(-Math.max(120, visibleRows + 60));
    const bufExtraction = extractApprovalOptions(bufferTail);
    const bufOpts = bufExtraction.options;
    const pendingHasCursor = options.some(o => o.isCurrent);
    const bufHasCursor = bufOpts.some(o => o.isCurrent);
    if (bufOpts.length > options.length || (!pendingHasCursor && bufHasCursor && bufOpts.length >= options.length)) {
      options.length = 0;
      options.push(...bufOpts);
      contextSourceLines = bufferTail;
      contextCluster = bufExtraction.cluster;
    }
    // option 1 が pendingTextTail から欠落している場合（保持上限）に補完
    if (options.length >= 1 && !options.some(o => o.num === 1)) {
      const maxNum = Math.max(...options.map(o => o.num));
      if (bufOpts.length > 0 && Math.max(...bufOpts.map(o => o.num)) === maxNum) {
        for (const bo of bufOpts) {
          if (!options.some(o => o.num === bo.num)) options.push(bo);
        }
        options.sort((a, b) => a.num - b.num);
        contextSourceLines = bufferTail;
        contextCluster = bufExtraction.cluster;
      }
    }
  }
  const contextLines = approvalContextLines(contextSourceLines, contextCluster);

  markHubChoiceDefault(options, contextLines);
  const lastOpt = options[options.length - 1];
  const hasUserSpecifies = (lastOpt && userSpecifiesRe.test(lastOpt.label)) || contextLines.some(line => userSpecifiesRe.test(line));
  // Ink UI は常に選択中の項目に > / ❯ カーソルを付ける（isCurrent: true）。
  // カーソル付き選択肢がない場合は AI の通常応答の箇条書きとみなして無視する。
  const hasCursorOption = options.some(o => o.isCurrent);
  // 実際のCLI承認プロンプトは yes/no/allow/deny/proceed 等を含む。
  // Claude の通常回答（「パネル幅拡大...」等）との誤検出を防ぐため、
  // ラベル内容が承認系のときのみ approvalNear を評価する。
  const approvalLabelRe = /\b(yes|no|allow|deny|proceed|abort|don[''']t ask|cancel)\b/i;
  const hasApprovalLikeLabel = options.some((opt) => approvalLabelRe.test(opt.label));
  const isHubChoice = isHubChoicePrompt(contextLines, options);
  const approvalNear = hasCursorOption &&
    ((hasApprovalLikeLabel && (hasUserSpecifies || contextLines.some((line) => matchProviderApprovalTrigger(provider, line) || matchNativeApprovalTrigger(line)))) || isHubChoice);
  const hasChoiceMenuHint = hasCursorOption && options.length > 0 && contextLines.some((line) => matchProviderApprovalTrigger(provider, line) || matchNativeApprovalTrigger(line));
  const nowVisible = (options.length > 0 && approvalNear) || hasChoiceMenuHint;

  // doSend / sendChoice で消費済みの選択肢が xterm scanBuffer に残っているため
  // フォールバック検出で再抽出されるケースを抑止する（marker 検出と同じ debounce 戦略）。
  if (nowVisible) {
    const consumed = approvalConsumedSig.get(id);
    const sig = approvalSig(options);
    if (consumed === sig) {
      const prev = approvalConsumedSigDeleteTimer.get(id);
      if (prev) clearTimeout(prev);
      approvalConsumedSigDeleteTimer.set(id, setTimeout(() => {
        approvalConsumedSig.delete(id);
        approvalConsumedSigDeleteTimer.delete(id);
      }, 5000));
      return;
    }
  }

  if (nowVisible) {
    approvalRawOptionsCache.set(id, options);
  } else {
    cancelApprovalHintConfirm(id);
    approvalRawOptionsCache.delete(id);
  }

  // true への遷移は短時間 debounce してから送信する。PTY 生バイト上だけの一瞬の誤検出で
  // サイドバーが waiting/running を往復するのを防ぐ。
  if (nowVisible && !approvalVisibleCache.get(id)) {
    scheduleApprovalHintConfirm(id, options);
  }
  // 非アクティブセッションでは false を送らない。
  // PTY の断片的な再描画で nowVisible が一時的に false になると、
  // waiting -> running/standby のチラつきが起きるため、保留状態を維持する。
  // false への遷移はアクティブ時の detectApproval/hideActionBar で確定させる。
}

function scheduleApprovalCheck(id) {
  if (approvalCheckTimer) clearTimeout(approvalCheckTimer);
  approvalCheckTimer = setTimeout(() => detectApproval(id), 300);
}

// extractHubMarkerApproval は [ANY-AI-CLI]...[/ANY-AI-CLI] マーカーベースの承認を検出する。
// 検出した場合は options 配列を返し、検出できなければ null を返す。
function hasYesNoApprovalMarker(text) {
  return /\(Y:1\/N:0\)/.test(String(text || ''));
}

function looksLikeYesNoQuestion(text) {
  const s = String(text || '');
  if (!hasYesNoApprovalMarker(s)) return false;
  const before = s.slice(0, s.lastIndexOf('(Y:1/N:0)'));
  return /[?？]\s*$/.test(before.trim()) || /[?？]/.test(before.slice(-120));
}

// (Y:1/N:0) 直前の質問本文を sig 用 ctx として抽出する。
// 同一プロンプト再描画時に hash が変わらないよう、Ink ノイズが乗りにくい部分を選ぶ。
function _yesNoCtxFromText(text) {
  const s = String(text || '');
  const idx = s.lastIndexOf('(Y:1/N:0)');
  const before = idx >= 0 ? s.slice(0, idx) : s;
  return before.slice(-200);
}

function extractHubMarkerApproval(lines) {
  const searchStart = Math.max(0, lines.length - 40);
  const recentText = lines.slice(searchStart).join('\n');
  const blockRe = /\[ANY-AI-CLI\]([\s\S]*?)\[\/ANY-AI-CLI\]/g;
  let match;
  let lastBlock = null;
  while ((match = blockRe.exec(recentText)) !== null) {
    lastBlock = match[1];
  }
  if (lastBlock !== null) {
    const inner = lastBlock.split('\n').map(l => l.trim()).filter(Boolean);
    return _parseHubBlock(inner);
  }

  let closeIdx = -1;
  let openIdx = -1;

  for (let i = lines.length - 1; i >= searchStart; i--) {
    const line = lines[i];
    // Single-line: [ANY-AI-CLI] content [/ANY-AI-CLI]
    if (/\[ANY-AI-CLI\]/.test(line) && /\[\/ANY-AI-CLI\]/.test(line)) {
      const inner = line.replace(/^[\s\S]*?\[ANY-AI-CLI\]/, '').replace(/\[\/ANY-AI-CLI\][\s\S]*$/, '').trim();
      return _parseHubBlock([inner]);
    }
    if (/\[\/ANY-AI-CLI\]/.test(line) && closeIdx === -1) { closeIdx = i; continue; }
    if (/\[ANY-AI-CLI\]/.test(line) && closeIdx !== -1) { openIdx = i; break; }
  }

  if (openIdx === -1 || closeIdx === -1) return null;
  const inner = lines.slice(openIdx + 1, closeIdx).map(l => l.trim()).filter(Boolean);
  return _parseHubBlock(inner);
}

// AGENTS.md の確認フォーマットに従った素の Yes/No 質問を検出する。
// [ANY-AI-CLI] マーカーが無い場合でも、質問文中に `(Y:1/N:0)` があれば Hub ボタン化する。
function extractPlainYesNoApproval(lines) {
  const searchStart = Math.max(0, lines.length - 20);
  const recentLines = lines.slice(searchStart).map(line => String(line || '').trim()).filter(Boolean);
  for (let i = lines.length - 1; i >= searchStart; i--) {
    const line = String(lines[i] || '').trim();
    if (!line) continue;
    if (/\[ANY-AI-CLI\]|\[\/ANY-AI-CLI\]/.test(line)) continue;
    // TUI redraw/status text can be appended after the marker on the same logical line.
    if (looksLikeYesNoQuestion(line)) return _yesNoApprovalOptions(_yesNoCtxFromText(line));
  }
  const recentText = recentLines.join('\n');
  if (!/\[ANY-AI-CLI\]|\[\/ANY-AI-CLI\]/.test(recentText) && looksLikeYesNoQuestion(recentText)) {
    return _yesNoApprovalOptions(_yesNoCtxFromText(recentText));
  }
  return null;
}

function _parseHubBlock(lines) {
  const text = lines.join('\n');
  if (hasYesNoApprovalMarker(text)) {
    return _yesNoApprovalOptions(_yesNoCtxFromText(text));
  }
  const opts = lines
    .map(l => l.match(/^\s*(\d+)\.\s*(.+?)\s*$/))
    .filter(Boolean)
    .map(m => ({ num: parseInt(m[1], 10), label: m[2].trim(), isCurrent: false }));
  return opts.length > 0 ? opts : null;
}

function _yesNoApprovalOptions(ctxText) {
  const ctx = ctxText ? _approvalCtxHash(ctxText) : '';
  return [
    { num: 1, label: 'Yes (1)', isCurrent: true, preserveOrder: true, _ctx: ctx },
    { num: 0, label: 'No (0)', isCurrent: false, preserveOrder: true, _ctx: ctx },
  ];
}

function approvalContextLines(lines, cluster, margin = 10) {
  if (!cluster) return lines;
  return lines.slice(Math.max(0, cluster.start - margin), Math.min(lines.length, cluster.end + margin + 1));
}

function matchNativeApprovalTrigger(line) {
  if (!line) return false;
  const lower = String(line).toLowerCase();
  return lower.includes('requires approval') ||
    lower.includes('would you like to run the following command') ||
    lower.includes('would you like to run') ||
    lower.includes('do you want to proceed?') ||
    lower.includes('this command requires approval') ||
    lower.includes('press enter to confirm');
}

function extractApprovalOptions(tail) {
  const options = [];
  let clusterStart = -1;
  let clusterEnd = -1;
  let lastOptionIdx = -1;
  const maxGap = 4;
  // ドット必須にして `462           const m = ...` のような差分行番号を誤検出しないようにする。
  // `2.Yes` 形式（ドット直後にスペース無し）は `\s*` が 0 個マッチでカバーする。
  // 番号上限は 1〜99（実用的な承認メニューは ≤ 9 個。Codex 等で 2 桁が出てもカバー）。
  for (let i = tail.length - 1; i >= 0; i--) {
    const line = tail[i];
    const cm = line.match(/^\s*[>❯›❱]\s*(\d{1,2})\.\s*(.+?)\s*$/);
    if (cm) {
      if (lastOptionIdx !== -1 && lastOptionIdx - i > maxGap) break;
      options.unshift({ num: parseInt(cm[1], 10), label: cm[2].trim().replace(/\s{2,}.*$/, '').replace(/\s*\d+\.\s*[A-Za-z].*$/, '').trim(), isCurrent: true });
      if (clusterEnd === -1) clusterEnd = i;
      clusterStart = i;
      lastOptionIdx = i;
      continue;
    }
    const om = line.match(/^\s*(\d{1,2})\.\s*(.+?)\s*$/);
    if (om) {
      if (lastOptionIdx !== -1 && lastOptionIdx - i > maxGap) break;
      options.unshift({ num: parseInt(om[1], 10), label: om[2].trim().replace(/\s{2,}.*$/, '').replace(/\s*\d+\.\s*[A-Za-z].*$/, '').trim(), isCurrent: false });
      if (clusterEnd === -1) clusterEnd = i;
      clusterStart = i;
      lastOptionIdx = i;
      continue;
    }
    if (lastOptionIdx !== -1 && lastOptionIdx - i > maxGap) break;
  }
  if (options.length === 0) return { options: [], cluster: null };
  const nums = options.map(opt => opt.num);
  const numMin = Math.min(...nums);
  const numMax = Math.max(...nums);
  if (numMax > 20 || numMax - numMin > 15 || options.length > 12) {
    return { options: [], cluster: null };
  }
  // 重複行を除去（折り返し/再描画ノイズ対策）
  const seen = new Set();
  const uniqueOptions = options.filter(opt => {
    const key = `${opt.num}:${opt.label}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  return {
    options: uniqueOptions,
    cluster: { start: clusterStart, end: clusterEnd },
  };
}

// ---- クランチ（折りたたみ）検出 ----

function detectCrunch(id) {
  // ライブ TUI 描画領域（baseY 〜 baseY+rows-1）のみを対象にする。
  // scanBuffer はスクロールバック全体を返すため、`/clear` 後も古い
  // "(ctrl+o to expand)" 行を拾って展開ボタンが残るバグの原因になっていた。
  const t = terminals.get(id);
  if (!t || !t.term || !t.term.buffer) return { found: false, count: 0 };
  const buf = t.term.buffer.active;
  const rows = t.term.rows || 40;
  const startY = Math.max(0, buf.baseY);
  const endY = Math.min(buf.length, startY + rows);
  for (let y = endY - 1; y >= startY; y--) {
    const line = buf.getLine(y)?.translateToString(true) || '';
    // Claude Code の折りたたみパターン: "… +23 lines (ctrl+o to expand)"
    const m = line.match(/[…\.]{1,3}\s*\+(\d+)\s*lines?\s*\(ctrl\+o to expand\)/i);
    if (m) {
      const count = parseInt(m[1]);
      crunchLatch.set(id, { until: Date.now() + CRUNCH_LATCH_MS, count });
      return { found: true, count };
    }
  }
  // ストリーミング中の上書きで一時的に行が消える瞬間を吸収する（点滅防止）。
  // 直前に found=true を観測してから CRUNCH_LATCH_MS 以内ならその状態を維持する。
  const latched = crunchLatch.get(id);
  if (latched && Date.now() < latched.until) {
    return { found: true, count: latched.count };
  }
  if (latched) crunchLatch.delete(id);
  return { found: false, count: 0 };
}

function detectApproval(id) {
  // sendChoice 直後の誤再表示を抑制
  const suppressUntil = approvalSuppressUntil.get(id);
  if (suppressUntil && Date.now() < suppressUntil) return;
  approvalSuppressUntil.delete(id);

  const provider = sessions.get(id)?.provider;
  const bar = document.getElementById('action-bar');
  if (!bar) return;

  const tEarly = terminals.get(id);
  // 複数質問 UI（AskUserQuestion 等）の判定を最優先。末尾 40 行に限定して scrollback 残骸を除外。
  const mqLines = (tEarly?.pendingTextTail || '').split(/\r\n|\r|\n/).slice(-40).map(l => stripAnsi(l))
    .concat(scanBuffer(id).slice(-40));
  if (isMultiQuestionPrompt(mqLines)) {
    // action-bar は出さない（誤誘導防止）
    bar.classList.remove('visible');
    bar.innerHTML = '';
    actionBarFocusIdx = -1;
    if (!multiQuestionVisibleCache.get(id)) {
      multiQuestionVisibleCache.set(id, true);
      if (!approvalVisibleCache.get(id)) {
        approvalVisibleCache.set(id, true);
        enqueueApprovalAutoSwitch(id);
        ws.send(JSON.stringify({ type: 'session_hint', session_id: id, approval_visible: true }));
      }
    }
    if (id === activeSessionId) setMultiQuestionBannerVisible(true);
    return;
  }
  if (multiQuestionVisibleCache.get(id)) {
    multiQuestionVisibleCache.delete(id);
    if (id === activeSessionId) setMultiQuestionBannerVisible(false);
    if (approvalVisibleCache.get(id)) {
      approvalVisibleCache.set(id, false);
      removeApprovalAutoSwitchTarget(id);
      ws.send(JSON.stringify({ type: 'session_hint', session_id: id, approval_visible: false }));
    }
  }

  // [ANY-AI-CLI] マーカー検出: xterm バッファではなく pendingTextTail を使う。
  // xterm バッファは回答済みの古い [ANY-AI-CLI] ブロックを保持し続けるため、
  // suppress 期間が切れると再検出・再表示されてしまう。
  // pendingTextTail は hideActionBar でクリアされるが、Ink 再描画で同一内容が
  // 再び入ることがあるため approvalConsumedSig で二重表示を防ぐ。
  const t = terminals.get(id);
  if (t) {
    const pendingLines = (t.pendingTextTail || '').split(/\r\n|\r|\n/).slice(-40).map(l => stripAnsi(l));
    const markerOpts = extractHubMarkerApproval(pendingLines);
    if (markerOpts) {
      const consumed = approvalConsumedSig.get(id);
      const sig = approvalSig(markerOpts);
      if (consumed === sig) return; // 消費済み承認の再表示をスキップ（タイマーは trackApprovalHintFromChunk 側で管理）
      const prevTimer2 = approvalConsumedSigDeleteTimer.get(id);
      if (prevTimer2) { clearTimeout(prevTimer2); approvalConsumedSigDeleteTimer.delete(id); }
      approvalConsumedSig.delete(id);
      showActionBar(bar, id, markerOpts, false);
      if (!approvalVisibleCache.get(id)) {
        cancelApprovalHintConfirm(id);
        approvalVisibleCache.set(id, true);
        enqueueApprovalAutoSwitch(id);
        ws.send(JSON.stringify({ type: 'session_hint', session_id: id, approval_visible: true }));
      }
      return;
    }

    const plainYesNoOpts = extractPlainYesNoApproval(pendingLines);
    if (plainYesNoOpts) {
      const consumed = approvalConsumedSig.get(id);
      const sig = approvalSig(plainYesNoOpts);
      if (consumed === sig) return;
      const prevTimer2 = approvalConsumedSigDeleteTimer.get(id);
      if (prevTimer2) { clearTimeout(prevTimer2); approvalConsumedSigDeleteTimer.delete(id); }
      approvalConsumedSig.delete(id);
      showActionBar(bar, id, plainYesNoOpts, false);
      if (!approvalVisibleCache.get(id)) {
        cancelApprovalHintConfirm(id);
        approvalVisibleCache.set(id, true);
        enqueueApprovalAutoSwitch(id);
        ws.send(JSON.stringify({ type: 'session_hint', session_id: id, approval_visible: true }));
      }
      return;
    }
  }

  // フォールバック検出: pendingTextTail を使う（scanBuffer は履歴を保持するため
  // hideActionBar 後も古い選択肢を再検出してしまう）
  const tail = (t ? t.pendingTextTail || '' : '').split(/\r\n|\r|\n/).slice(-120).map(l => stripAnsi(l));

  // フォールバック検出（既存）
  let extraction = extractApprovalOptions(tail);
  const options = extraction.options;
  let contextSourceLines = tail;
  let contextCluster = extraction.cluster;
  // ターミナル高さぶんの空白行 + 余裕60行を確保（ダイアログが画面上部にある場合に備える）
  const visibleRows = t?.term?.rows || 40;
  const bufferTail = scanBuffer(id).slice(-Math.max(120, visibleRows + 60));

  // Ink.js（Claude Code）はカーソル位置制御シーケンスで各行を描画するため \r\n がなく、
  // pendingTextTail の split で全テキストが1行に連結されて options が空になるか、
  // "Yes2.No" のように選択肢が連結されて1行に結合されることがある（concat artifact）。
  // xterm バッファ（行分割済み）がより多くの選択肢を持つ場合はそちらを優先する。
  if (t && t.pendingTextTail) {
    const bufExtraction = extractApprovalOptions(bufferTail);
    const bufOpts = bufExtraction.options;
    const pendingHasCursor = options.some(o => o.isCurrent);
    const bufHasCursor = bufOpts.some(o => o.isCurrent);
    if (bufOpts.length > options.length || (!pendingHasCursor && bufHasCursor && bufOpts.length >= options.length)) {
      options.length = 0;
      options.push(...bufOpts);
      options.sort((a, b) => a.num - b.num);
      contextSourceLines = bufferTail;
      contextCluster = bufExtraction.cluster;
    }
  }

  // pendingTextTail は保持上限があるため option 1 が欠落することがある。
  // 2択プロンプト（Yes/No）では option 2 だけ pendingTextTail に入り options.length=1 のまま
  // 補完が発動しないケースも含め、option 1 が未検出なら常に xterm バッファで補完する。
  if (t && t.pendingTextTail && options.length >= 1 && !options.some(o => o.num === 1)) {
    const maxNum = Math.max(...options.map(o => o.num));
    const bufExtraction = extractApprovalOptions(bufferTail);
    const bufOpts = bufExtraction.options;
    if (bufOpts.length > 0 && Math.max(...bufOpts.map(o => o.num)) === maxNum) {
      for (const bo of bufOpts) {
        if (!options.some(o => o.num === bo.num)) options.push(bo);
      }
      options.sort((a, b) => a.num - b.num);
      contextSourceLines = bufferTail;
      contextCluster = bufExtraction.cluster;
    }
  }

  const contextLines = approvalContextLines(contextSourceLines, contextCluster);
  markHubChoiceDefault(options, contextLines);
  const lastOpt = options[options.length - 1];
  const hasUserSpecifies = (lastOpt && userSpecifiesRe.test(lastOpt.label)) || contextLines.some(line => userSpecifiesRe.test(line));
  const hasCursorOption = options.some(o => o.isCurrent);
  const approvalLabelRe = /\b(yes|no|allow|deny|proceed|abort|don[''']t ask|cancel)\b/i;
  const hasApprovalLikeLabel = options.some((opt) => approvalLabelRe.test(opt.label));
  const isHubChoice = isHubChoicePrompt(contextLines, options);
  const approvalNear = (hasApprovalLikeLabel &&
    (hasUserSpecifies || contextLines.some((line) => matchProviderApprovalTrigger(provider, line) || matchNativeApprovalTrigger(line)))) || isHubChoice;
  const hasApproval = options.length > 0 && approvalNear && hasCursorOption;
  const hasChoiceMenu = hasCursorOption && options.length > 0 && contextLines.some((line) => matchProviderApprovalTrigger(provider, line) || matchNativeApprovalTrigger(line));
  const hasPrompt = hasApproval || hasChoiceMenu;

  // 折りたたみ（クランチ）を検出
  const crunch = detectCrunch(id);

  if (!hasPrompt && !crunch.found) {
    // 承認プロンプトが検出できない場合は確実に閉じる。
    // ただし、approvalVisibleCache=true かつ cache が残っている場合は、
    // pendingTextTail のローテート（長考時に [ANY-AI-CLI] マーカーが押し出される）や
    // 一時的なフォールバック検出失敗で action-bar を誤って消さないよう、
    // cache から action-bar を復元する（H9: 非対称スタック対策 — plan_action-bar-not-showing.md §7.1）。
    // sendChoice / doSend / closeBtn は hideActionBar を直接呼ぶため、ここの復元経路は通らない。
    // 解決済み承認の残留は approvalConsumedSig（sendChoice/doSend で sig 保存）で抑止される。
    if (approvalVisibleCache.get(id)) {
      const cached = approvalRawOptionsCache.get(id);
      if (cached && cached.length > 0) {
        showActionBar(bar, id, cached, false);
        return;
      }
    }
    hideActionBar(id);
    return;
  }

  // doSend / sendChoice で消費済みの選択肢が xterm scanBuffer に残っているため
  // フォールバック検出で再抽出されるケースを抑止する（marker 検出と同じ debounce 戦略）。
  if (hasPrompt) {
    const consumed = approvalConsumedSig.get(id);
    const sig = approvalSig(options);
    if (consumed === sig) {
      const prev = approvalConsumedSigDeleteTimer.get(id);
      if (prev) clearTimeout(prev);
      approvalConsumedSigDeleteTimer.set(id, setTimeout(() => {
        approvalConsumedSig.delete(id);
        approvalConsumedSigDeleteTimer.delete(id);
      }, 5000));
      hideActionBar(id);
      return;
    }
  }

  // 承認プロンプト表示中は展開ボタンを出さない（ctrl+o が承認 UI に届いて誤動作するのを防ぐ）
  showActionBar(bar, id, hasPrompt ? options : [], crunch.found && !hasPrompt);

  // session_hint: 承認 UI の可視状態を Hub に通知
  const nowVisible = hasPrompt;
  if (nowVisible !== !!approvalVisibleCache.get(id)) {
    if (nowVisible) cancelApprovalHintConfirm(id);
    approvalVisibleCache.set(id, nowVisible);
    if (nowVisible) enqueueApprovalAutoSwitch(id);
    else removeApprovalAutoSwitchTarget(id);
    ws.send(JSON.stringify({ type: 'session_hint', session_id: id, approval_visible: nowVisible }));
  }
}

function getActionBarButtons() {
  const bar = document.getElementById('action-bar');
  if (!bar) return [];
  // expand-btn は除外: クランチ展開のみで action-bar が visible のとき Enter/←→ が
  // 展開クリックに化けて TUI の選択確定（\r）が PTY に届かなくなるのを防ぐ
  return Array.from(bar.querySelectorAll('.action-btn:not(.expand-btn)'));
}

function setActionBarFocus(idx) {
  actionBarFocusIdx = idx;
  getActionBarButtons().forEach((btn, i) => btn.classList.toggle('kbd-focus', i === idx));
}

function hideActionBar(id) {
  const bar = document.getElementById('action-bar');
  if (bar) { bar.classList.remove('visible'); bar.innerHTML = ''; }
  // 差分スキップ用キャッシュをリセット（次回 showActionBar が同一シグネチャでも再描画されるように）
  lastActionBarRender.sessionId = null;
  lastActionBarRender.sig = null;
  actionBarFocusIdx = -1;
  if (id !== undefined) {
    cancelApprovalHintConfirm(id);
    const wasVisible = !!approvalVisibleCache.get(id);
    if (wasVisible) {
      approvalVisibleCache.set(id, false);
      removeApprovalAutoSwitchTarget(id);
      ws.send(JSON.stringify({ type: 'session_hint', session_id: id, approval_visible: false }));
    }
    // approvalRawOptionsCache / pendingTextTail のクリアは、approvalVisibleCache が
    // true → false へ実際に遷移する時（= sendChoice / doSend / closeBtn / detectApproval が
    // cache 復元できず最終的に閉じた時）のみ実行する。
    // wasVisible=false で呼ばれた場合は race 条件 or 重複呼び出しなので、cache を保護する。
    // （H9: 非対称スタック対策 — plan_action-bar-not-showing.md §7.1）
    if (wasVisible) {
      approvalRawOptionsCache.delete(id);
      const t = terminals.get(id);
      if (t) t.pendingTextTail = '';
    }
    // approvalConsumedSig は debounce 型タイマーで削除する。
    // trackApprovalHintFromChunk が同一ブロックを再検出するたびにタイマーをリセットし、
    // ブロックが届かなくなってから 5 秒後に削除する。
    // ここでは Ink が再描画を開始する前の初期タイマーを 10 秒で設定する（フォールバック）。
    const prevTimer = approvalConsumedSigDeleteTimer.get(id);
    if (prevTimer) clearTimeout(prevTimer);
    approvalConsumedSigDeleteTimer.set(id, setTimeout(() => {
      approvalConsumedSig.delete(id);
      approvalConsumedSigDeleteTimer.delete(id);
    }, 10000));
    // action-bar 消失でターミナル領域の高さが拡張されるため、追従中なら最下部へ再スナップする。
    // showActionBar が plan_approval-bar-scroll-resnap.md で同等の処理を持つので、その対称ケース。
    // wheel up ガード中は forceTerminalToBottom 内でガードが効くため、ユーザー意図を上書きしない。
    if (wasVisible && id === activeSessionId) {
      const term = terminals.get(id);
      const shouldStickToBottom = !!(term && (term.autoScroll || isTerminalAtBottom(term)));
      if (shouldStickToBottom) forceTerminalToBottomAfterLayout(id);
    }
    maybeAutoSwitchToNextApproval();
  }
}

function normalizeActionOptions(options) {
  const byNum = new Map();
  for (const opt of options || []) {
    if (!opt || typeof opt.num !== 'number') continue;
    const prev = byNum.get(opt.num);
    if (!prev) {
      byNum.set(opt.num, { ...opt });
      continue;
    }
    // 同一番号が再描画ノイズで重複した場合は、現在選択中フラグと長いラベルを優先する。
    byNum.set(opt.num, {
      ...prev,
      isCurrent: !!(prev.isCurrent || opt.isCurrent),
      preserveOrder: !!(prev.preserveOrder || opt.preserveOrder),
      label: (opt.label && opt.label.length > (prev.label || '').length) ? opt.label : prev.label,
    });
  }
  const normalized = Array.from(byNum.values());
  if (normalized.some(opt => opt.preserveOrder)) return normalized;
  return normalized.sort((a, b) => a.num - b.num);
}

function showActionBar(bar, sessionId, options, showExpand) {
  options = normalizeActionOptions(options);
  // 注意: 局所変数名 `t` は window.t（i18n 翻訳関数）と衝突するため使わない。
  // `term` にすることで本関数末尾の t('expand_btn') 等が正しく i18n を参照できる。
  const term = sessionId === activeSessionId ? terminals.get(sessionId) : null;
  const shouldStickToBottom = !!(term && (term.autoScroll || isTerminalAtBottom(term)));

  // 差分スキップ: 前回描画と同一シグネチャなら DOM を再構築しない（点滅防止）。
  // detectApproval は scheduleApprovalCheck 経由で 300ms ごとに走るため、
  // 内容が変わらない場合に bar.innerHTML を毎回作り直すと expand-btn 等が点滅する。
  // kbd-focus は外部の setActionBarFocus が触るので、ここでは options/showExpand のみを sig に含める。
  const sig = JSON.stringify({
    s: sessionId,
    opts: options.map(o => ({ n: o.num, l: o.label, c: !!o.isCurrent, p: !!o.preserveOrder })),
    x: !!showExpand,
    v: bar.classList.contains('visible'),
  });
  if (lastActionBarRender.sessionId === sessionId && lastActionBarRender.sig === sig) {
    // 承認検出は PTY write / ResizeObserver / セッション自動切替と同時に走ることがある。
    // DOM 再描画をスキップする場合でも、追従中なら最下部への再スナップは省略しない。
    if (shouldStickToBottom) forceTerminalToBottomAfterLayout(sessionId);
    return;
  }
  lastActionBarRender.sessionId = sessionId;
  lastActionBarRender.sig = sig;
  bar.innerHTML = '';

  // "⚠ Approval needed" ラベル
  if (options.length > 0) {
    const label = document.createElement('span');
    label.className = 'action-bar-label';
    label.textContent = '⚠ Approval needed';
    bar.appendChild(label);
  }

  // "Yes, and" 系（セッション全体許可）が存在する場合はそちらを推奨（橙色）にする
  const hasSessionAllow = options.some(o => /during this session|allow.*session|yes.*allow/i.test(o.label));

  // 選択肢ボタン（左側）
  for (const opt of options) {
    const btn = document.createElement('button');
    const isPermanent = /don[''']t ask again/i.test(opt.label);
    const isSessionAllow = /during this session|allow.*session|yes.*allow/i.test(opt.label);
    const isRecommended = hasSessionAllow ? isSessionAllow : opt.isCurrent;
    let cls = 'action-btn';
    if (isSessionAllow) cls += ' session-allow';
    else if (isRecommended) cls += ' current';
    if (isPermanent) cls += ' permanent';
    btn.className = cls;
    btn.textContent = `${opt.num}. ${opt.label}`;
    btn.title = `${opt.num}. ${opt.label}`;
    btn.onclick = () => sendChoice(sessionId, opt.num);
    bar.appendChild(btn);
  }

  // 展開ボタン（クランチ検出時・選択肢の右側）
  if (showExpand) {
    const btn = document.createElement('button');
    btn.className = 'action-btn expand-btn';
    btn.textContent = t('expand_btn');
    btn.title = t('expand_title');
    btn.onclick = () => handleExpandClick(sessionId);
    bar.appendChild(btn);
  }

  // 手動閉じボタン（誤検出時に消すため）— action-bar の右端に表示
  const closeBtn = document.createElement('button');
  closeBtn.className = 'action-dismiss-btn';
  closeBtn.textContent = '✕';
  closeBtn.title = t('dismiss_title');
  closeBtn.onclick = (e) => {
    e.stopPropagation();
    hideActionBar(sessionId);
    approvalSuppressUntil.set(sessionId, Date.now() + 60000);
  };
  bar.appendChild(closeBtn);

  bar.classList.add('visible');
  if (shouldStickToBottom) {
    forceTerminalToBottomAfterLayout(sessionId);
  }
}

function sendChoice(sessionId, targetNum) {
  // 矢印移動ではなく番号直接入力で確定する（誤選択防止）
  sendText(sessionId, `${targetNum}\r`);
  // doSend と同様に消費済み署名を記録（Ink 再描画による同一ブロックの再検出・再表示を防ぐ）
  const prevOpts = approvalRawOptionsCache.get(sessionId);
  if (prevOpts) approvalConsumedSig.set(sessionId, approvalSig(prevOpts));
  hideActionBar(sessionId);
  // PTY エコーバックによる誤再表示を 2 秒間抑制
  approvalSuppressUntil.set(sessionId, Date.now() + 2000);
  // suppress 解除後に pendingTextTail を再スキャン（suppress 中に届いた [ANY-AI-CLI] ブロックを検出するため）
  setTimeout(() => {
    detectApproval(sessionId);
    maybeAutoSwitchToNextApproval();
  }, 2050);
  setTimeout(() => inputEl.focus(), 0);
}

// ---- 展開キャプチャ ----

function handleExpandClick(id) {
  if (expandCapturePending) return;
  expandCapturePending = true;

  // ctrl+o 送信前のバッファをスナップショット
  const beforeSet = new Set(scanBuffer(id));
  sendText(id, '\x0f'); // ctrl+o（detailed transcript へ切替）

  // 800ms 後にバッファ差分を取得して保存し、ctrl+o を再送して元のコンパクト表示へ戻す
  // （戻さないと Claude Code が「Showing detailed transcript」モードに張り付き、
  //  入力プロンプトが見えなくなって「セッション切れ？」と誤認される）
  setTimeout(() => {
    const afterLines = scanBuffer(id);
    const expanded = afterLines.filter(l => l.trim() && !beforeSet.has(l));

    sendText(id, '\x0f'); // ctrl+o（コンパクト表示へ戻す）
    expandCapturePending = false;

    if (expanded.length === 0) return;

    if (!toolOutputs.has(id)) toolOutputs.set(id, []);
    const outputs = toolOutputs.get(id);
    const now = new Date();
    outputs.unshift({ uid: now.getTime(), lines: expanded, ts: formatDateTime(now) });
    if (outputs.length > 10) outputs.length = 10; // 最大10件保持

    renderToolOutputs(id);
    // パネル表示後に承認プロンプトが来ていた場合を検出するため再評価
    scheduleApprovalCheck(id);
  }, 800);
}

function formatDateTime(d) {
  const p = n => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth()+1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}


function formatStartedAt(isoStr) {
  if (!isoStr) return '';
  const d = new Date(isoStr);
  if (isNaN(d.getTime())) return '';
  const elapsed = Math.max(0, Math.floor((Date.now() - d.getTime()) / 1000));
  const p = n => String(n).padStart(2, '0');
  const s = elapsed % 60;
  const m = Math.floor(elapsed / 60) % 60;
  const h = Math.floor(elapsed / 3600);
  return h > 0 ? `[${h}:${p(m)}:${p(s)}]` : `[${p(m)}:${p(s)}]`;
}

function formatLastOutputAt(isoStr) {
  if (!isoStr) return '';
  const d = new Date(isoStr);
  if (isNaN(d.getTime())) return '';
  const p = n => String(n).padStart(2, '0');
  const now = new Date();
  const sameDay = d.getFullYear() === now.getFullYear() &&
                  d.getMonth() === now.getMonth() &&
                  d.getDate() === now.getDate();
  const time = `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
  if (sameDay) return time;
  const DOW = JSON.parse(t('dow'));
  return `${d.getFullYear()}-${p(d.getMonth()+1)}-${p(d.getDate())}(${DOW[d.getDay()]}) ${time}`;
}

function cleanCopiedText(linesOrText) {
  const lines = Array.isArray(linesOrText)
    ? linesOrText.slice()
    : String(linesOrText || '').replace(/\r\n?/g, '\n').split('\n');
  const cleaned = lines.map(line => String(line || '').replace(/[ \t]+$/g, ''));

  while (cleaned.length > 0 && cleaned[0].trim() === '') cleaned.shift();
  while (cleaned.length > 0 && cleaned[cleaned.length - 1].trim() === '') cleaned.pop();

  const indents = cleaned
    .filter(line => line.trim() !== '')
    .map(line => (line.match(/^[ \t]*/) || [''])[0].length);
  const minIndent = indents.length > 0 ? Math.min(...indents) : 0;
  if (minIndent <= 0) return cleaned.join('\n');

  return cleaned
    .map(line => line.trim() === '' ? '' : line.slice(minIndent))
    .join('\n');
}

async function copyCleanText(linesOrText, anchor) {
  const text = cleanCopiedText(linesOrText);
  if (!text) return;
  await navigator.clipboard.writeText(text);
  showToast(t('copied_to_clipboard'), anchor);
}

function renderToolOutputs(id) {
  const panel = document.getElementById('tool-outputs-panel');
  const list = document.getElementById('tool-outputs-list');
  const countEl = document.getElementById('tool-outputs-count');
  if (!panel || !list) return;

  const outputs = toolOutputs.get(id) || [];
  if (outputs.length === 0) {
    panel.hidden = true;
    return;
  }

  panel.hidden = false;
  if (countEl) countEl.textContent = t('tool_outputs_count', { n: outputs.length });

  list.innerHTML = '';
  outputs.forEach((out, idx) => {
    const item = document.createElement('details');
    item.className = 'tool-output-item';
    if (idx === 0) item.open = true; // 最新は展開して表示

    const summary = document.createElement('summary');
    summary.textContent = t('tool_output_summary', { ts: out.ts, n: out.lines.length });
    item.appendChild(summary);

    const body = document.createElement('div');
    body.className = 'tool-output-body';

    const copyBtn = document.createElement('button');
    copyBtn.type = 'button';
    copyBtn.className = 'tool-output-copy';
    copyBtn.textContent = '⧉';
    copyBtn.title = t('copy_to_clipboard');
    copyBtn.setAttribute('aria-label', t('copy_to_clipboard'));
    copyBtn.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      copyCleanText(out.lines, copyBtn).catch(() => {});
    });
    body.appendChild(copyBtn);

    const pre = document.createElement('pre');
    pre.className = 'tool-output-content';
    appendLinkedText(pre, out.lines.join('\n'), id);
    body.appendChild(pre);
    item.appendChild(body);

    list.appendChild(item);
  });
}

// ---- 入力バー ----

const inputEl = document.getElementById('input');
const pasteChipsEl = document.getElementById('paste-chips');

// ペースト折りたたみ状態
const pastedTexts = []; // [{id, text, lineCount}]
let pasteCounter = 0;

function autoExpand() {
  const t = activeSessionId === null ? null : terminals.get(activeSessionId);
  const shouldStickToBottom = !!(t && (t.autoScroll || isTerminalAtBottom(t)));
  inputEl.style.height = 'auto';
  inputEl.style.height = Math.min(inputEl.scrollHeight, Math.floor(window.innerHeight * 0.3)) + 'px';
  refitActiveTerminalAfterLayout(shouldStickToBottom);
}

function renderPasteChips() {
  if (!pasteChipsEl) return;
  pasteChipsEl.innerHTML = '';
  pastedTexts.forEach((pt, idx) => {
    const chip = document.createElement('div');
    chip.className = 'paste-chip';

    const label = document.createElement('span');
    label.className = 'paste-chip-label';
    label.textContent = t('paste_chip_label', { id: pt.id, n: pt.lineCount });

    const expandBtn = document.createElement('button');
    expandBtn.className = 'paste-chip-expand';
    expandBtn.textContent = t('expand');
    expandBtn.title = t('expand_title_btn');
    expandBtn.addEventListener('click', () => expandPasteChip(idx));

    const removeBtn = document.createElement('button');
    removeBtn.className = 'paste-chip-remove';
    removeBtn.textContent = t('remove');
    removeBtn.title = t('remove_paste');
    removeBtn.addEventListener('click', () => removePasteChip(idx));

    chip.appendChild(label);
    chip.appendChild(expandBtn);
    chip.appendChild(removeBtn);
    pasteChipsEl.appendChild(chip);
  });
}

function expandPasteChip(idx) {
  const pt = pastedTexts[idx];
  if (!pt) return;
  inputEl.value = pt.text + (inputEl.value ? '\n' + inputEl.value : '');
  pastedTexts.splice(idx, 1);
  renderPasteChips();
  autoExpand();
  inputEl.focus();
}

function removePasteChip(idx) {
  pastedTexts.splice(idx, 1);
  renderPasteChips();
}

function clearAllPastes() {
  pastedTexts.length = 0;
  renderPasteChips();
}

function buildSendText() {
  const parts = pastedTexts.map(pt => pt.text);
  if (inputEl.value) parts.push(inputEl.value);
  return parts.join('\n');
}

function clearInput() {
  inputEl.value = '';
  inputEl.style.height = 'auto';
  clearAllPastes();
}

async function doSend(sessionId) {
  const injects = await flushPendingAttach(sessionId);
  const injectPrefix = injects.join('');
  let rawText = buildSendText();
  // トリガーフレーズを末尾から除去（PTY・AI には送らない）
  const _tp = getActiveTriggerPhrase();
  if (_tp && textEndsWithTriggerPhrase(rawText, _tp)) {
    rawText = stripTrailingTriggerPhrase(rawText, _tp);
  }
  // 改行を含む場合はブラケットペーストモードでラップ（\n が途中 Enter と解釈されるのを防ぐ）
  // ブラケットペーストはテキスト部分のみに適用し、injectPrefix は前置する
  let textPart;
  if (rawText === '' && injectPrefix !== '') {
    // 画像のみ（テキストなし）: inject 末尾の \r or スペースで確定済み → 追加の \r で送信
    textPart = '\r';
  } else if (rawText.includes('\n')) {
    textPart = '\x1b[200~' + rawText + '\x1b[201~\r';
  } else {
    textPart = rawText + '\r';
  }
  const textToSend = injectPrefix + textPart;
  clearInput();
  hideSlashMenu();
  // テキスト送信で承認ポップアップをバイパスした場合、Ink 再描画による
  // 同一選択肢の再検出・再表示を防ぐため消費済み署名を保存する
  const prevOpts = approvalRawOptionsCache.get(sessionId);
  if (prevOpts) approvalConsumedSig.set(sessionId, approvalSig(prevOpts));
  hideActionBar(sessionId);
  // PTY エコーバックによる誤再表示を抑制（sendChoice と同様）
  approvalSuppressUntil.set(sessionId, Date.now() + 2000);
  setTimeout(() => {
    detectApproval(sessionId);
    maybeAutoSwitchToNextApproval();
  }, 2050);
  sendSubmittedText(sessionId, textToSend);
}

function saveInputStateFor(id) {
  if (id === null) return;
  const frag = document.createDocumentFragment();
  if (attachThumbnails) {
    while (attachThumbnails.firstChild) frag.appendChild(attachThumbnails.firstChild);
  }
  sessionInputState.set(id, {
    inputValue: inputEl.value,
    pastedTextsData: [...pastedTexts],
    pendingAttachFiles: [...pendingAttachFiles],
    thumbsFragment: frag,
  });
  inputEl.value = '';
  inputEl.style.height = 'auto';
  pastedTexts.length = 0;
  pendingAttachFiles.length = 0;
}

function restoreInputStateFor(id) {
  const state = sessionInputState.get(id);
  if (state) {
    inputEl.value = state.inputValue;
    pastedTexts.length = 0;
    pastedTexts.push(...state.pastedTextsData);
    pendingAttachFiles.length = 0;
    pendingAttachFiles.push(...state.pendingAttachFiles);
    if (attachThumbnails) {
      attachThumbnails.innerHTML = '';
      if (state.thumbsFragment) attachThumbnails.appendChild(state.thumbsFragment);
    }
  } else {
    inputEl.value = '';
    inputEl.style.height = 'auto';
    pastedTexts.length = 0;
    pendingAttachFiles.length = 0;
    if (attachThumbnails) attachThumbnails.innerHTML = '';
  }
  autoExpand();
  renderPasteChips();
  updateAttachClearBtn();
}

function cleanupSessionInputState(id) {
  const state = sessionInputState.get(id);
  if (!state) return;
  if (state.thumbsFragment) {
    state.thumbsFragment.querySelectorAll('img').forEach(img => {
      if (img.src.startsWith('blob:')) URL.revokeObjectURL(img.src);
    });
  }
  sessionInputState.delete(id);
}

const specialKeys = {
  'ArrowUp':    '\x1b[A',
  'ArrowDown':  '\x1b[B',
  'ArrowRight': '\x1b[C',
  'ArrowLeft':  '\x1b[D',
  'Escape':     '\x1b',
};

// ---- スラッシュコマンドメニュー ----

function getSlashCommands() {
  return [
    { cmd: '/clear',    desc: t('slash_clear') },
    { cmd: '/compact',  desc: t('slash_compact') },
    { cmd: '/config',   desc: t('slash_config') },
    { cmd: '/cost',     desc: t('slash_cost') },
    { cmd: '/doctor',   desc: t('slash_doctor') },
    { cmd: '/help',     desc: t('slash_help') },
    { cmd: '/init',     desc: t('slash_init') },
    { cmd: '/login',    desc: t('slash_login') },
    { cmd: '/logout',   desc: t('slash_logout') },
    { cmd: '/model',    desc: t('slash_model') },
    { cmd: '/review',   desc: t('slash_review') },
    { cmd: '/resume',   desc: t('slash_resume') },
    { cmd: '/status',   desc: t('slash_status') },
    { cmd: '/usage',    desc: t('slash_usage') },
    { cmd: '/vim',      desc: t('slash_vim') },
  ];
}

const slashMenuEl = document.getElementById('slash-menu');
let slashItems = [];
let slashIndex = -1;

function updateSlashMenu() {
  const val = inputEl.value;
  if (!val.startsWith('/')) { hideSlashMenu(); return; }
  const filtered = getSlashCommands().filter(c => c.cmd.startsWith(val));
  if (filtered.length === 0) { hideSlashMenu(); return; }
  slashItems = filtered;
  if (slashIndex >= slashItems.length) slashIndex = 0;
  if (slashIndex < 0) slashIndex = 0;
  renderSlashMenu();
}

function renderSlashMenu() {
  slashMenuEl.innerHTML = '';
  slashItems.forEach((item, i) => {
    const div = document.createElement('div');
    div.className = 'slash-item' + (i === slashIndex ? ' selected' : '');
    div.innerHTML = `<span class="slash-cmd">${item.cmd}</span><span class="slash-desc">${item.desc}</span>`;
    div.addEventListener('mousedown', (e) => { e.preventDefault(); selectSlashItem(i); });
    slashMenuEl.appendChild(div);
  });
  slashMenuEl.hidden = false;
  scrollSlashIntoView();
}

function hideSlashMenu() {
  slashMenuEl.hidden = true;
  slashItems = [];
  slashIndex = -1;
}

function selectSlashItem(i) {
  if (i < 0 || i >= slashItems.length) return;
  const cmd = slashItems[i].cmd;
  // /clear と /model はクイック実行用途のため、候補選択時に即送信する
  if (activeSessionId !== null && (cmd === '/clear' || cmd === '/model')) {
    sendQuickCommand(activeSessionId, cmd);
    clearInput();
    hideSlashMenu();
    inputEl.focus();
    return;
  }
  inputEl.value = cmd + ' ';
  hideSlashMenu();
  autoExpand();
  inputEl.focus();
}

function scrollSlashIntoView() {
  const items = slashMenuEl.querySelectorAll('.slash-item');
  if (items[slashIndex]) items[slashIndex].scrollIntoView({ block: 'nearest' });
}

inputEl.addEventListener('input', () => {
  autoExpand(); updateSlashMenu(); composeEndSent = false;
  if (!isComposing) {
    const _tp = getActiveTriggerPhrase();
    if (_tp && activeSessionId !== null && textEndsWithTriggerPhrase(buildSendText(), _tp)) {
      doSend(activeSessionId);
    }
  }
});
inputEl.addEventListener('blur', () => setTimeout(hideSlashMenu, 150));
inputEl.addEventListener('compositionstart', () => { isComposing = true; composeEndSent = false; });
inputEl.addEventListener('compositionend', () => {
  isComposing = false;
  // 自動送信トリガー: input イベントは IME 環境/ブラウザによって compositionend より
  // 前または最中にしか発火せず、isComposing=true で autosend がスキップされてしまう。
  // ここで末尾チェックして発火させる。input ハンドラ側でもチェックされるが、
  // doSend 後は inputEl.value='' になるので二重送信しない。
  const _tp = getActiveTriggerPhrase();
  if (_tp && activeSessionId !== null && textEndsWithTriggerPhrase(buildSendText(), _tp)) {
    pendingSend = false;
    if (composeEndSendTimer !== null) {
      clearTimeout(composeEndSendTimer);
      composeEndSendTimer = null;
    }
    composeEndSent = true;
    doSend(activeSessionId);
    return;
  }
  if (pendingSend) {
    pendingSend = false;
    composeEndSendTimer = setTimeout(() => {
      composeEndSendTimer = null;
      if (activeSessionId === null) return;
      composeEndSent = true;
      doSend(activeSessionId);
    }, 0);
  }
});

inputEl.addEventListener('keydown', (e) => {
  // スラッシュメニューが開いているときはメニュー操作を優先
  if (!slashMenuEl.hidden && slashItems.length > 0) {
    if (e.key === 'ArrowUp') {
      slashIndex = (slashIndex - 1 + slashItems.length) % slashItems.length;
      renderSlashMenu();
      e.preventDefault(); return;
    }
    if (e.key === 'ArrowDown') {
      slashIndex = (slashIndex + 1) % slashItems.length;
      renderSlashMenu();
      e.preventDefault(); return;
    }
    if (e.key === 'Enter' || e.key === 'Tab') {
      selectSlashItem(slashIndex);
      e.preventDefault(); return;
    }
    if (e.key === 'Escape') {
      hideSlashMenu();
      e.preventDefault(); return;
    }
  }

  if (activeSessionId === null) return;

  // Tab でセッション切り替え（スラッシュメニューが閉じているとき）
  if (e.key === 'Tab' && !e.isComposing && slashMenuEl.hidden) {
    switchSessionByTab(e.shiftKey);
    e.preventDefault(); return;
  }

  // action-bar 表示中 + 入力なし → ←→ キーでボタン間移動
  if ((e.key === 'ArrowLeft' || e.key === 'ArrowRight') && inputEl.value === '') {
    const bar = document.getElementById('action-bar');
    if (bar && bar.classList.contains('visible')) {
      const btns = getActionBarButtons();
      if (btns.length > 0) {
        if (actionBarFocusIdx < 0) actionBarFocusIdx = 0;
        const delta = e.key === 'ArrowRight' ? 1 : -1;
        setActionBarFocus((actionBarFocusIdx + delta + btns.length) % btns.length);
        e.preventDefault(); return;
      }
    }
  }

  if (specialKeys[e.key]) {
    // 入力テキストあり + 矢印キーはブラウザのカーソル移動に委譲する
    if (inputEl.value !== '' && (e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'ArrowUp' || e.key === 'ArrowDown')) return;
    sendText(activeSessionId, specialKeys[e.key]);
    inputEl.value = ''; // TUI 操作中の誤入力を流さないようにクリア
    e.preventDefault(); return;
  }
  if (e.ctrlKey && e.key === 'c') {
    // xterm.js 選択中はクリップボードにコピーして SIGINT を送らない
    const xt = terminals.get(activeSessionId);
    if (xt?.term.hasSelection()) {
      navigator.clipboard.writeText(xt.term.getSelection()).catch(() => {});
      e.preventDefault(); return;
    }
    // ブラウザ側の通常テキスト選択中もコピーに委譲
    if (window.getSelection()?.toString().length > 0) return;
    sendText(activeSessionId, '\x03'); e.preventDefault(); return;
  }
  if (e.ctrlKey && e.key === 'd') { sendText(activeSessionId, '\x04'); e.preventDefault(); return; }
  // ctrl+o: Claude Code の折りたたみ展開（ターミナル直接操作と同等）
  if (e.ctrlKey && e.key === 'o') { sendText(activeSessionId, '\x0f'); e.preventDefault(); return; }
  if (e.key === 'Enter') {
    if (e.isComposing || isComposing) { pendingSend = true; return; } // IME確定後に送信
    if (e.shiftKey) { autoExpand(); return; } // Shift+Enter: 改行
    // action-bar 表示中かつ入力が空 → フォーカス中ボタン（未指定なら先頭）を実行
    const bar = document.getElementById('action-bar');
    if (bar && bar.classList.contains('visible') && inputEl.value.trim() === '') {
      const btns = getActionBarButtons();
      const targetBtn = actionBarFocusIdx >= 0 ? btns[actionBarFocusIdx] : btns[0];
      if (targetBtn) { targetBtn.click(); e.preventDefault(); return; }
    }
    // compositionend が既に doSend をスケジュール済みの場合はキャンセル（二重送信防止）
    if (composeEndSendTimer !== null) {
      clearTimeout(composeEndSendTimer);
      composeEndSendTimer = null;
    }
    pendingSend = false;
    composeEndSent = false;
    doSend(activeSessionId);
    e.preventDefault();
  }
});

document.getElementById('send-btn').addEventListener('mousedown', () => {
  // クリック時に IME が確定中の場合、compositionend 後に送信するよう予約
  if (isComposing) pendingSend = true;
});
document.getElementById('send-btn').addEventListener('click', () => {
  if (activeSessionId === null) return;
  if (isComposing) return; // compositionend 側で処理する
  // compositionend タイマーが既に発火して doSend を実行済みの場合はスキップ（二重送信防止）
  if (composeEndSent) { composeEndSent = false; return; }
  if (composeEndSendTimer !== null) {
    // compositionend が既に doSend をスケジュール済み → タイマーキャンセルして直接実行（二重送信防止）
    clearTimeout(composeEndSendTimer);
    composeEndSendTimer = null;
  }
  pendingSend = false;
  doSend(activeSessionId);
});

document.getElementById('quick-clear-btn').addEventListener('click', () => {
  if (activeSessionId === null) return;
  sendQuickCommand(activeSessionId, getQuickCommand(1));
});

document.getElementById('quick-model-btn').addEventListener('click', () => {
  if (activeSessionId === null) return;
  sendQuickCommand(activeSessionId, getQuickCommand(2));
});

function sendQuickCommand(sessionId, cmd) {
  // doSend / sendChoice と同様に承認 UI 状態を Hub と同期する。
  // /clear 等で画面がリセットされた後も approvalVisibleCache=true が残ると、
  // セッションカードの "Pending" バッジが消えなくなる。
  const prevOpts = approvalRawOptionsCache.get(sessionId);
  if (prevOpts) approvalConsumedSig.set(sessionId, approvalSig(prevOpts));
  hideActionBar(sessionId);
  approvalSuppressUntil.set(sessionId, Date.now() + 2000);
  setTimeout(() => {
    detectApproval(sessionId);
    maybeAutoSwitchToNextApproval();
  }, 2050);
  sendSubmittedText(sessionId, `${cmd}\r`);
}

function sendSubmittedText(sessionId, text) {
  // 送信操作は最新出力を見たい意図なので、スクロールアップ中でも最下部へ戻して追従を再開する
  const t = terminals.get(sessionId);
  if (t) {
    // wheel up ガード（USER_SCROLL_UP_GUARD_MS = 800ms）が立ったまま送信されると、
    // forceTerminalToBottom / onFlush / fitTerminalPreservingBottom が no-op になり、
    // 送信直後のレイアウト変化（clearInput の height='auto' / hideActionBar）で
    // viewport が一瞬上にジャンプしたまま戻らない事象が起きる。
    // 送信操作は「最新を見たい」意図なのでガードを明示解除する。
    t.userScrolledUpAt = 0;
    t.autoScroll = true;
    try { t.term.scrollToBottom(); } catch (_) {}
    if (sessionId === activeSessionId) updateScrollLockBtn(false);
    // hideActionBar / clearInput によるレイアウト変化（action-bar 消失・入力欄縮小）の後に
    // ResizeObserver が fit() を呼ぶタイミング次第で最下部判定を取りこぼし、最上部に寄って
    // 見えることがあるため、showActionBar と対称に 2 段 RAF で再スナップする。
    if (sessionId === activeSessionId) forceTerminalToBottomAfterLayout(sessionId);
  }
  sendText(sessionId, text);
}

function sendText(sessionId, text) {
  ws.send(JSON.stringify({ type: 'pty_input', session_id: sessionId, text }));
}

function requestSessionDismiss(id) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ type: 'session_dismiss', session_id: id }));
  }
}

function dismissSession(id) {
  if (!sessions.has(id)) return;
  requestSessionDismiss(id);
}

function removeLocalSession(id) {
  const timer = autoDismissTimers.get(id);
  if (timer) { clearTimeout(timer); autoDismissTimers.delete(id); }
  sessions.delete(id);
  removeFromSessionOrder(id);
  const t = terminals.get(id);
  if (t) { try { t.term.dispose(); } catch (_) {} terminals.delete(id); }
  toolOutputs.delete(id);
  approvalVisibleCache.delete(id);
  if (multiQuestionVisibleCache.delete(id) && id === activeSessionId) {
    setMultiQuestionBannerVisible(false);
  }
  removeApprovalAutoSwitchTarget(id);
  approvalRawOptionsCache.delete(id);
  approvalConsumedSig.delete(id);
  cancelApprovalHintConfirm(id);
  approvalSuppressUntil.delete(id);
  cleanupSessionInputState(id);
  if (activeSessionId === id) {
    activeSessionId = null;
    const area = document.getElementById('terminal-area');
    if (area) area.innerHTML = '';
    hideActionBar(undefined);
    setMultiQuestionBannerVisible(false);
    if (sessions.size > 0) {
      activateSession(sessions.keys().next().value);
      return;
    }
  }
  maybeAutoSwitchToNextApproval();
  render();
}

// ---- resize ----

function sendResize(sessionId, cols, rows) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ type: 'pty_resize', session_id: sessionId, cols, rows }));
  }
}

function canFitTerminal(t) {
  if (!t || !t.container || !t.container.isConnected) return false;
  if (!t.term || !t.term.element || !t.term.element.isConnected) return false;
  // display:none などで非表示状態だと offsetParent が null になり、幅も 0 になる。
  // この状態で fitAddon.fit() を呼ぶと cols が 1 桁台に潰れ、表示復帰後も narrow なまま残るので除外。
  if (t.container.offsetWidth <= 0 || t.container.offsetHeight <= 0) return false;
  return true;
}

let lastDevicePixelRatio = window.devicePixelRatio || 1;

function refitAllTerminals(refreshRows = false) {
  terminals.forEach((t, id) => {
    if (!canFitTerminal(t)) return;
    const prevCols = t.term.cols;
    const prevRows = t.term.rows;
    fitTerminalPreservingBottom(t, id);
    if (refreshRows && t.term.rows > 0) {
      t.term.refresh(0, t.term.rows - 1);
    }
    if (t.term.cols !== prevCols || t.term.rows !== prevRows) {
      sendResize(id, t.term.cols, t.term.rows);
    }
  });
}

const resizeObserver = new ResizeObserver(() => {
  if (activeSessionId === null) return;
  const t = terminals.get(activeSessionId);
  if (!canFitTerminal(t)) return;
  const prevCols = t.term.cols;
  const prevRows = t.term.rows;
  fitTerminalPreservingBottom(t, activeSessionId);
  if (t.term.cols !== prevCols || t.term.rows !== prevRows) {
    sendResize(activeSessionId, t.term.cols, t.term.rows);
  }
});

const termArea = document.getElementById('terminal-area');
if (termArea) resizeObserver.observe(termArea);

window.addEventListener('resize', () => {
  const dpr = window.devicePixelRatio || 1;
  if (Math.abs(dpr - lastDevicePixelRatio) < 0.001) return;
  lastDevicePixelRatio = dpr;
  refitAllTerminals(true);
});

// セッション選択中は inputEl からフォーカスが外れたら即座に戻す
// ただし設定パネルが開いている間、またはテキスト選択操作中はフォーカスを奪わない
let suppressFocusReclaim = false;
let voiceActive = false;
let voiceAudioActive = false;
function isInteractiveFocusTarget(target) {
  if (!(target instanceof Element)) return false;
  return !!target.closest([
    'button',
    'input',
    'textarea',
    'select',
    'a',
    'label',
    '[contenteditable="true"]',
    '[role="button"]',
    '#input-bar',
    '#action-bar',
    '#attach-panel',
    '#slash-picker',
    '#settings-panel',
    '#new-session-panel',
    '#model-picker-overlay',
    '#about-panel',
    '#session-list',
    '#topbar'
  ].join(','));
}
document.addEventListener('mousedown', () => { suppressFocusReclaim = true; });
document.addEventListener('mouseup',   () => { setTimeout(() => { suppressFocusReclaim = false; }, 300); });

inputEl.addEventListener('blur', (e) => {
  if (isInteractiveFocusTarget(e.relatedTarget)) return;
  if (suppressFocusReclaim || voiceActive) return;
  if (activeSessionId !== null && document.getElementById('settings-panel').hidden) {
    setTimeout(() => inputEl.focus(), 0);
  }
});

// ---- ツール出力パネル 折りたたみ / 閉じるボタン ----

const collapseOutputsBtn = document.getElementById('tool-outputs-collapse');
if (collapseOutputsBtn) {
  collapseOutputsBtn.addEventListener('click', () => {
    const panel = document.getElementById('tool-outputs-panel');
    if (!panel) return;
    const collapsed = panel.classList.toggle('collapsed');
    collapseOutputsBtn.textContent = collapsed ? '▶' : '▼';
  });
}

const clearOutputsBtn = document.getElementById('tool-outputs-clear');
if (clearOutputsBtn) {
  clearOutputsBtn.addEventListener('click', () => {
    if (activeSessionId !== null) {
      toolOutputs.delete(activeSessionId);
      renderToolOutputs(activeSessionId);
    }
  });
}

// ---- セッション管理 ----

function activateSession(id) {
  if (activeSessionId !== null && activeSessionId !== id) {
    saveInputStateFor(activeSessionId);
  }
  activeSessionId = id;
  _activeSessionSwitchedAt = performance.now();
  // 新セッションを必ず最下部追従で開く。fitTerminalPreservingBottom や
  // refitActiveTerminalAfterLayout の評価より前にリセットする必要がある。
  // 同時に userScrolledUpAt ガードもクリア（カード明示切替は「最新を見たい」意図）。
  const tNext = terminals.get(id);
  if (tNext) {
    tNext.autoScroll = true;
    tNext.userScrolledUpAt = 0;
  }
  restoreInputStateFor(id);
  ensureTerminal(id);
  attachTerminal(id);
  updateScrollLockBtn();
  setMultiQuestionBannerVisible(!!multiQuestionVisibleCache.get(id));
  detectApproval(id);
  renderSessionList();
  renderToolOutputs(id);
  updateShellBadge(id);
  inputEl.focus();
  if (typeof window._wakewordSessionChanged === 'function') window._wakewordSessionChanged();
  // v2: セッション切替時にターミナルビューへ戻す
  FilesTabManager.switchToSessionView();
  const sessionInfo = sessions.get(id);
  if (sessionInfo) {
    const label = sessionInfo.label
      ? `[${sessionInfo.label}] #${id}`
      : `#${id}`;
    FilesTabManager.updateSessionTabLabel(label);
  }
}

function updateShellBadge(id) {
  const el = document.getElementById('terminal-shell-info');
  if (!el) return;
  const s = id !== null ? sessions.get(id) : null;
  const shell = s?.shell || '';
  el.textContent = shell ? ' · ' + shell : '';
}

function stateLabel(state) {
  return t('state_' + state) || state;
}

function providerIconHtml(provider) {
  if (provider === 'claude') {
    return `<svg class="card-provider-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><circle cx="8" cy="8" r="6" fill="#FFF7ED" stroke="#F97316" stroke-width="2"/></svg>`;
  }
  if (provider === 'codex') {
    return `<svg class="card-provider-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><circle cx="8" cy="8" r="6" fill="#EFF6FF" stroke="#3B82F6" stroke-width="2"/></svg>`;
  }
  return '';
}

function getOrderedSessions() {
  const ordered = sessionOrder.filter(id => sessions.has(id)).map(id => sessions.get(id));
  sessions.forEach((s) => { if (!sessionOrder.includes(s.id)) ordered.push(s); });
  const starred = favorites.filter(id => sessions.has(id)).map(id => sessions.get(id));
  const unstarred = ordered.filter(s => !favorites.includes(s.id));
  return [...starred, ...unstarred];
}

function switchSessionByTab(shift) {
  if (sessions.size <= 1) return;
  const all = getOrderedSessions();
  const currentIdx = all.findIndex(s => s.id === activeSessionId);
  if (currentIdx === -1) return;
  const nextIdx = shift
    ? (currentIdx - 1 + all.length) % all.length
    : (currentIdx + 1) % all.length;
  activateSession(all[nextIdx].id);
  const bar = document.getElementById('action-bar');
  if (bar && bar.classList.contains('visible')) {
    setActionBarFocus(0);
  } else {
    actionBarFocusIdx = -1;
  }
}

function jumpToSessionByIndex(n) {
  const all = getOrderedSessions();
  const target = all[n - 1];
  if (!target) return;
  activateSession(target.id);
  requestAnimationFrame(() => {
    const card = document.querySelector(`.card[data-session-id="${target.id}"]`);
    card?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  });
}

document.addEventListener('keydown', (e) => {
  if (!e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
  const n = parseInt(e.key, 10);
  if (!(n >= 1 && n <= 9)) return;
  const active = document.activeElement;
  const tag = active?.tagName;
  const isOtherInput = (tag === 'INPUT' || tag === 'TEXTAREA') && active.id !== 'input-el';
  if (isOtherInput) return;
  e.preventDefault();
  jumpToSessionByIndex(n);
});

let _sessionListClickDelegated = false;
let _sessionCardPointerDown = null;

function renderSessionList() {
  const root = document.getElementById('sessions');
  const scrollEl = document.getElementById('session-list');
  // innerHTML クリアで scrollTop が 0 に戻るのを防ぐ。経過時間更新の 1Hz 再描画や
  // session_update 受信のたびにユーザのスクロール位置がトップへ吹き飛ぶのを避ける。
  const prevScrollTop = scrollEl ? scrollEl.scrollTop : 0;
  if (!_sessionListClickDelegated) {
    _sessionListClickDelegated = true;
    root.addEventListener('pointerdown', (e) => {
      if (e.button !== 0) return;
      const card = e.target.closest('.card');
      if (!card || e.target.closest('.card-actions')) {
        _sessionCardPointerDown = null;
        return;
      }
      _sessionCardPointerDown = {
        id: parseInt(card.dataset.sessionId, 10),
        x: e.clientX,
        y: e.clientY,
      };
    });
    root.addEventListener('pointerup', (e) => {
      const down = _sessionCardPointerDown;
      _sessionCardPointerDown = null;
      if (!down || isNaN(down.id)) return;
      const card = e.target.closest('.card');
      if (!card || e.target.closest('.card-actions')) return;
      const id = parseInt(card.dataset.sessionId, 10);
      if (id !== down.id) return;
      const moved = Math.hypot(e.clientX - down.x, e.clientY - down.y);
      if (moved <= 8) activateSession(id);
    });
    root.addEventListener('click', (e) => {
      const card = e.target.closest('.card');
      if (!card || e.target.closest('.card-actions')) return;
      const id = parseInt(card.dataset.sessionId, 10);
      if (!isNaN(id)) activateSession(id);
    });
    // セッション切替直後（グレース期間中）、`#session-list` 上の wheel をアクティブターミナルへ転送する。
    // 兄弟要素のため `#terminal-wrapper` の wheel リスナにバブリングしない問題への対策。
    const sessionListEl = document.getElementById('session-list');
    if (sessionListEl && !sessionListEl._wheelForwardBound) {
      sessionListEl._wheelForwardBound = true;
      sessionListEl.addEventListener('wheel', (e) => {
        const withinGrace = _activeSessionSwitchedAt
          && (performance.now() - _activeSessionSwitchedAt) < POST_SWITCH_WHEEL_GRACE_MS;
        if (!withinGrace) return;
        if (activeSessionId === null) return;
        const tActive = terminals.get(activeSessionId);
        if (!tActive || !tActive.term) return;
        // alt buffer なら PgUp/PgDn を PTY 側へ転送し session-list 自身の縦スクロールを抑止。
        if (forwardWheelToAltBuffer(activeSessionId, tActive, e.deltaY)) {
          e.preventDefault();
          return;
        }
        const lineHeight = 24;
        const lines = Math.sign(e.deltaY) * Math.max(1, Math.round(Math.abs(e.deltaY) / lineHeight));
        try { tActive.term.scrollLines(lines); } catch (_) {}
        if (e.deltaY < 0) {
          tActive.autoScroll = false;
          tActive.userScrolledUpAt = performance.now();
          updateScrollLockBtn(true);
        }
        // session-list 自身の縦スクロールが同時に動かないよう既定動作を抑止する。
        e.preventDefault();
      }, { passive: false });
    }
  }
  root.innerHTML = '';
  if (sessions.size === 0) {
    const p = document.createElement('div');
    p.className = 'no-sessions';
    p.textContent = t('no_sessions');
    root.appendChild(p);
    return;
  }

  // セッションをプロジェクト別にグループ化
  const groups = new Map();
  getOrderedSessions().forEach(s => {
    const cwdStr = s.cwd || '';
    const name = cwdStr
      ? cwdStr.replace(/\\/g, '/').split('/').filter(p => p.length > 0).pop() || ''
      : '';
    const key = name || '__no_project__';
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(s);
  });

  // ピン留め優先、その中で groupOrder に従ってソート（未登録キーは末尾）
  const sortedGroupKeys = [...groups.keys()].sort((a, b) => {
    const aPin = projectFavorites.includes(a);
    const bPin = projectFavorites.includes(b);
    if (aPin !== bPin) return aPin ? -1 : 1;
    if (aPin && bPin) return projectFavorites.indexOf(a) - projectFavorites.indexOf(b);
    const ai = groupOrder.indexOf(a);
    const bi = groupOrder.indexOf(b);
    if (ai === -1 && bi === -1) return 0;
    if (ai === -1) return 1;
    if (bi === -1) return -1;
    return ai - bi;
  });

  sortedGroupKeys.forEach(key => {
    const groupSessions = groups.get(key);
    const projectDisplayName = key === '__no_project__' ? t('no_project') : key;
    const isCollapsed = collapsedGroups.has(key);
    const groupEl = document.createElement('div');
    groupEl.className = 'project-group' + (projectFavorites.includes(key) ? ' project-group--pinned' : '');

    const header = document.createElement('div');
    header.className = 'project-group-header' + (isCollapsed ? ' project-group-header--collapsed' : '');
    header.dataset.project = key;

    const chevron = document.createElement('span');
    chevron.className = 'project-group-chevron';
    chevron.textContent = '▼';
    header.appendChild(chevron);

    const nameSpan = document.createElement('span');
    nameSpan.textContent = '📁 ' + projectDisplayName;
    header.appendChild(nameSpan);

    // v2: files ボタン（__no_project__ 以外のプロジェクトにのみ表示）
    if (key !== '__no_project__') {
      const filesBtn = document.createElement('button');
      filesBtn.className = 'project-group-files-btn';
      filesBtn.textContent = '📁 files';
      filesBtn.title = t('files_group_btn_tooltip');
      filesBtn.addEventListener('click', async (e) => {
        e.stopPropagation();
        // セッション ID（グループの最初のアクティブセッション）
        const firstSession = groupSessions[0];
        const sessionId = firstSession ? firstSession.id : null;
        // セッションの cwd（= UI 上のプロジェクト直下）を直接開く。
        // /api/files-roots の gitRoot は cwd の親方向探索結果なので、
        // 親側に別の .git があるとプロジェクト外を指してしまう。ここでは使わない。
        const rootToOpen = firstSession ? firstSession.cwd : null;
        if (rootToOpen) {
          FilesTabManager.openFilesTab(sessionId, key, rootToOpen, rootToOpen);
        }
      });
      header.appendChild(filesBtn);
    }

    const runningCount  = groupSessions.filter(s => s.state === 'running').length;
    const waitingCount  = groupSessions.filter(s => s.state === 'waiting').length;
    const standbyCount  = groupSessions.filter(s => (s.state || 'standby') === 'standby').length;
    const chipsEl = document.createElement('span');
    chipsEl.className = 'group-status-chips';
    chipsEl.innerHTML =
      `<span class="status-chip status-chip--running">${runningCount}</span>` +
      `<span class="status-chip status-chip--waiting">${waitingCount}</span>` +
      `<span class="status-chip status-chip--standby">${standbyCount}</span>`;
    header.appendChild(chipsEl);

    // プロジェクト ☆/✕ ボタン
    const projActions = document.createElement('div');
    projActions.className = 'project-group-actions';

    const projStarBtn = document.createElement('button');
    const isProjFav = projectFavorites.includes(key);
    projStarBtn.className = 'star-btn' + (isProjFav ? ' starred' : '');
    projStarBtn.textContent = isProjFav ? '★' : '☆';
    projStarBtn.title = isProjFav ? t('project_favorite_remove') : t('project_favorite_add');
    projStarBtn.onclick = (e) => {
      e.stopPropagation();
      const idx = projectFavorites.indexOf(key);
      if (idx !== -1) { projectFavorites.splice(idx, 1); } else { projectFavorites.push(key); }
      saveProjectFavorites();
      renderSessionList();
    };
    projActions.appendChild(projStarBtn);

    const projXBtn = document.createElement('button');
    projXBtn.className = 'dismiss-btn';
    projXBtn.textContent = t('remove');
    projXBtn.title = t('project_dismiss');
    projXBtn.onclick = (e) => {
      e.stopPropagation();
      groupSessions.forEach(s => dismissSession(s.id));
    };
    projActions.appendChild(projXBtn);

    header.appendChild(projActions);

    header.addEventListener('click', () => {
      if (dragSrcGroupKey) return;
      if (collapsedGroups.has(key)) {
        collapsedGroups.delete(key);
      } else {
        collapsedGroups.add(key);
      }
      renderSessionList();
    });

    // グループD&D
    header.draggable = true;
    header.addEventListener('dragstart', (e) => {
      dragSrcGroupKey = key;
      dragSrcId = null;
      groupEl.classList.add('dragging');
      e.dataTransfer.effectAllowed = 'move';
      e.stopPropagation();
    });
    header.addEventListener('dragend', () => {
      dragSrcGroupKey = null;
      root.querySelectorAll('.project-group').forEach(el => el.classList.remove('dragging', 'drag-over'));
    });
    groupEl.addEventListener('dragover', (e) => {
      if (!dragSrcGroupKey || dragSrcGroupKey === key) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = 'move';
      root.querySelectorAll('.project-group').forEach(el => el.classList.remove('drag-over'));
      groupEl.classList.add('drag-over');
    });
    groupEl.addEventListener('dragleave', (e) => {
      if (!groupEl.contains(e.relatedTarget)) groupEl.classList.remove('drag-over');
    });
    groupEl.addEventListener('drop', (e) => {
      e.preventDefault();
      groupEl.classList.remove('drag-over');
      if (!dragSrcGroupKey || dragSrcGroupKey === key) return;
      const srcKey = dragSrcGroupKey;
      dragSrcGroupKey = null;
      if (!groupOrder.length) groupOrder = [...sortedGroupKeys];
      sortedGroupKeys.forEach(k => { if (!groupOrder.includes(k)) groupOrder.push(k); });
      const srcIdx = groupOrder.indexOf(srcKey);
      const dstIdx = groupOrder.indexOf(key);
      if (srcIdx !== -1) groupOrder.splice(srcIdx, 1);
      groupOrder.splice(dstIdx, 0, srcKey);
      saveGroupOrder();
      renderSessionList();
    });

    groupEl.appendChild(header);

    const body = document.createElement('div');
    body.className = 'project-group-body' + (isCollapsed ? ' hidden' : '');

    groupSessions.forEach(s => {
      const c = document.createElement('div');
      const state = s.state || 'standby';
      const stateClass = (state === 'running' || state === 'waiting') ? ` ${state}` : '';
      c.className = 'card' + stateClass + (s.id === activeSessionId ? ' active' : '');
      c.tabIndex = isCollapsed ? -1 : 0;
      const label = stateLabel(state);
      const lastOut = formatLastOutputAt(s.last_output_at);
      const filteredMsg = filterFirstMessage(s.last_message || s.first_message || '');
      const cwdStr = s.cwd || '';
      const lastOutHtml = lastOut ? `<span class="card-last-output">L[${lastOut}]</span>` : '';
      const startedAtHtml = s.started_at ? `<span class="card-started-at">T${formatStartedAt(s.started_at)}</span>` : '';
      const sessionLabel = s.label ? `<span class="card-label">[${escapeHtml(s.label)}]</span>` : '';
      const msgHtml = filteredMsg
        ? `<span class="card-msg" data-tooltip="${escapeHtml(filteredMsg)}">${escapeHtml(filteredMsg)}</span>`
        : `<span class="card-msg"></span>`;
      const providerName = s.provider === 'claude' ? 'Claude' : s.provider === 'codex' ? 'Codex' : (s.provider || '');
      const providerChipHtml = providerName ? `<span class="card-provider-chip ${s.provider || ''}">${providerName}</span>` : '';
      const isDeadState = state === 'error' || state === 'disconnected';
      let reasonText = '';
      if (isDeadState && s.end_reason) {
        const key = 'end_reason_' + s.end_reason;
        const translated = window.t(key, { provider: providerName || s.provider || '' });
        // 未知の reason コードは window.t がキー文字列をそのまま返すため、その場合は非表示にする。
        if (translated !== key) reasonText = translated;
      }
      const reasonHtml = reasonText
        ? `<span class="card-end-reason" data-tooltip="${escapeHtml(reasonText)}">${escapeHtml(reasonText)}</span>`
        : '';
      const metaRow = `<div class="card-meta-row"><span class="badge ${state}">${label}</span>${reasonHtml}${sessionLabel}${msgHtml}</div>`;
      c.dataset.sessionId = s.id;
      const modelBadge = s.model ? ` <span class="card-model">${escapeHtml(s.model)}</span>` : '';
      const branchBadge = s.branch ? ` <span class="card-branch" data-tooltip="${escapeHtml(s.branch)}">${escapeHtml(s.branch)}</span>` : '';
      c.innerHTML =
        `<div class="card-title-row"><b>#${s.id}</b> ${providerIconHtml(s.provider)} ${providerChipHtml}${modelBadge}${branchBadge}${startedAtHtml}${lastOutHtml}</div>` +
        metaRow;

      const actions = document.createElement('div');
      actions.className = 'card-actions';
      c.appendChild(actions);

      // ☆/★ボタン
      const starBtn = document.createElement('button');
      const isFav = favorites.includes(s.id);
      starBtn.className = 'star-btn' + (isFav ? ' starred' : '');
      starBtn.textContent = isFav ? '★' : '☆';
      starBtn.title = isFav ? t('favorite_remove') : t('favorite_add');
      starBtn.onclick = (e) => {
        e.stopPropagation();
        const idx = favorites.indexOf(s.id);
        if (idx !== -1) { favorites.splice(idx, 1); } else { favorites.push(s.id); }
        saveFavorites();
        renderSessionList();
      };
      actions.appendChild(starBtn);


      const xBtn = document.createElement('button');
      xBtn.className = 'dismiss-btn';
      xBtn.textContent = t('remove');
      xBtn.title = t('dismiss_session');
      xBtn.onclick = (e) => { e.stopPropagation(); dismissSession(s.id); };
      actions.appendChild(xBtn);

      // D&Dドラッグ順序変更
      c.draggable = true;
      c.addEventListener('dragstart', (e) => {
        dragSrcId = s.id;
        c.classList.add('dragging');
        e.dataTransfer.effectAllowed = 'move';
      });
      c.addEventListener('dragend', () => {
        c.classList.remove('dragging');
        root.querySelectorAll('.card').forEach(el => el.classList.remove('drag-over'));
        setTimeout(() => inputEl.focus(), 0);
      });
      c.addEventListener('dragover', (e) => {
        if (dragSrcGroupKey) { e.dataTransfer.dropEffect = 'none'; return; }
        if (!dragSrcId || dragSrcId === s.id) return;
        // グループ跨ぎは禁止（★→☆ / ☆→★）
        if (favorites.includes(dragSrcId) !== favorites.includes(s.id)) { e.dataTransfer.dropEffect = 'none'; return; }
        e.preventDefault();
        e.dataTransfer.dropEffect = 'move';
        root.querySelectorAll('.card').forEach(el => el.classList.remove('drag-over'));
        c.classList.add('drag-over');
      });
      c.addEventListener('dragleave', () => { c.classList.remove('drag-over'); });
      c.addEventListener('drop', (e) => {
        e.preventDefault();
        c.classList.remove('drag-over');
        if (!dragSrcId || dragSrcId === s.id) return;
        if (favorites.includes(dragSrcId) !== favorites.includes(s.id)) return;
        const srcId = dragSrcId;
        dragSrcId = null;
        if (favorites.includes(srcId)) {
          const srcIdx = favorites.indexOf(srcId);
          const dstIdx = favorites.indexOf(s.id);
          favorites.splice(srcIdx, 1);
          favorites.splice(dstIdx, 0, srcId);
          saveFavorites();
        } else {
          if (!sessionOrder.includes(srcId)) sessionOrder.push(srcId);
          if (!sessionOrder.includes(s.id)) sessionOrder.push(s.id);
          const srcIdx = sessionOrder.indexOf(srcId);
          const dstIdx = sessionOrder.indexOf(s.id);
          sessionOrder.splice(srcIdx, 1);
          sessionOrder.splice(dstIdx, 0, srcId);
          saveSessionOrder();
        }
        renderSessionList();
      });

      body.appendChild(c);
    });

    groupEl.appendChild(body);
    root.appendChild(groupEl);
  });

  if (scrollEl) {
    const max = scrollEl.scrollHeight - scrollEl.clientHeight;
    scrollEl.scrollTop = Math.max(0, Math.min(prevScrollTop, max));
  }
}

// ---- タブ通知（保留バッジ） ----

let _faviconCanvas = null;
let _faviconCtx = null;
let _faviconBaseImg = null;
let _faviconBaseLoaded = false;
let _faviconPendingCount = 0;

let _titleBlinkInterval = null;
let _titleBlinkState = false;
let _titleBlinkCount = 0;

function startTitleBlink(pendingCount) {
  if (_titleBlinkInterval && _titleBlinkCount === pendingCount) return;
  stopTitleBlink();
  _titleBlinkCount = pendingCount;
  _titleBlinkState = true;
  _titleBlinkInterval = setInterval(() => {
    _titleBlinkState = !_titleBlinkState;
    document.title = _titleBlinkState ? `(${_titleBlinkCount}) ANY-AI-CLI` : 'ANY-AI-CLI';
  }, 800);
}

function stopTitleBlink() {
  if (_titleBlinkInterval) {
    clearInterval(_titleBlinkInterval);
    _titleBlinkInterval = null;
  }
}

function initFaviconCanvas() {
  if (_faviconCanvas) return;
  _faviconCanvas = document.createElement('canvas');
  _faviconCanvas.width = 32;
  _faviconCanvas.height = 32;
  _faviconCtx = _faviconCanvas.getContext('2d');
  _faviconBaseImg = new Image();
  _faviconBaseImg.onload = () => {
    _faviconBaseLoaded = true;
    drawFavicon(_faviconPendingCount);
  };
  _faviconBaseImg.src = '/icon.svg';
}

function drawFavicon(pendingCount) {
  initFaviconCanvas();
  const ctx = _faviconCtx;
  const SIZE = 32;
  ctx.clearRect(0, 0, SIZE, SIZE);
  if (_faviconBaseLoaded) ctx.drawImage(_faviconBaseImg, 0, 0, SIZE, SIZE);

  if (pendingCount > 0) {
    const R = 9;
    const cx = SIZE - R, cy = R;
    ctx.beginPath();
    ctx.arc(cx, cy, R, 0, Math.PI * 2);
    ctx.fillStyle = '#ef4444';
    ctx.fill();
    ctx.fillStyle = '#ffffff';
    ctx.font = `bold ${pendingCount > 9 ? 10 : 12}px sans-serif`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(pendingCount > 9 ? '9+' : String(pendingCount), cx, cy);
  }

  let link = document.querySelector("link[rel~='icon']");
  if (!link) { link = document.createElement('link'); link.rel = 'icon'; document.head.appendChild(link); }
  link.type = 'image/png';
  link.href = _faviconCanvas.toDataURL('image/png');
}

function updateTabNotification(pendingCount) {
  _faviconPendingCount = pendingCount;

  if (pendingCount > 0) {
    startTitleBlink(pendingCount);
  } else {
    stopTitleBlink();
    document.title = 'ANY-AI-CLI';
  }

  drawFavicon(pendingCount);
}

function render() {
  const stateCounts = { running: 0, waiting: 0, standby: 0 };
  const connByProvider = {};
  sessions.forEach(s => {
    const p = s.provider || 'unknown';
    connByProvider[p] = (connByProvider[p] || 0) + 1;
    const st = s.state || 'standby';
    if (st === 'running') stateCounts.running++;
    else if (st === 'waiting') stateCounts.waiting++;
    else stateCounts.standby++;
  });
  const totalWaiting = stateCounts.waiting;

  const PROVIDER_LABELS = { claude: 'Claude', codex: 'Codex' };
  const providerParts = Object.entries(connByProvider)
    .map(([p, n]) => `<span class="summary-provider-chip">${providerIconHtml(p)}<span class="summary-provider-name ${p}">${PROVIDER_LABELS[p] || p}</span><span class="summary-provider-count">: ${n}</span></span>`)
    .join('');

  let summary = '';
  if (stateCounts.running > 0) {
    summary += `<span class="session-chip running"><span class="chip-dot"></span>${stateCounts.running} running</span>`;
  }
  if (stateCounts.waiting > 0) {
    summary += `<span class="session-chip waiting"><span class="chip-dot"></span>${stateCounts.waiting} waiting</span>`;
  }
  if (stateCounts.standby > 0) {
    summary += `<span class="session-chip standby"><span class="chip-dot"></span>${stateCounts.standby} standby</span>`;
  }
  if (providerParts) summary += `<span class="summary-sep">|</span>${providerParts}`;
  document.getElementById('summary').innerHTML = summary;
  updateTabNotification(totalWaiting);

  if (activeSessionId === null && sessions.size > 0) {
    activateSession(sessions.keys().next().value);
    return;
  }
  renderSessionList();
}

// ---- ファイル転送 (attach) ----

const attachDropZone = document.getElementById('attach-drop-zone');
const attachFileInput = document.getElementById('attach-file-input');
const attachThumbnails = document.getElementById('attach-thumbnails');
const attachClearBtn = document.getElementById('attach-clear-btn');
const pendingAttachFiles = []; // {buf, filename, entry, wrapper} — ステージング済み未送信ファイル
const MAX_ATTACH_BYTES = 8 * 1024 * 1024;

function isImageFile(file) {
  return file.type.startsWith('image/');
}

function updateAttachClearBtn() {
  if (!attachClearBtn || !attachThumbnails) return;
  attachClearBtn.hidden = attachThumbnails.querySelectorAll('.attach-thumb-wrapper').length === 0;
}

if (attachClearBtn) {
  attachClearBtn.addEventListener('click', () => {
    if (!attachThumbnails) return;
    pendingAttachFiles.length = 0;
    attachThumbnails.querySelectorAll('.attach-thumb-wrapper').forEach(wrapper => {
      const img = wrapper.querySelector('img');
      if (img) URL.revokeObjectURL(img.src);
      wrapper.remove();
    });
    updateAttachClearBtn();
  });
}

window.addEventListener('paste', (e) => {
  if (activeSessionId === null) return;
  const items = e.clipboardData?.items;
  if (!items) return;

  // ファイルを優先（画像 or その他ファイル）
  let hasFile = false;
  for (const item of items) {
    if (item.kind !== 'file') continue;
    const file = item.getAsFile();
    if (!file) continue;
    hasFile = true;
    if (isImageFile(file)) stageAttach(file);
    else stageFileAttach(file);
  }
  if (hasFile) return;

  // 長いテキストはチップに折りたたむ
  const text = e.clipboardData?.getData('text');
  if (text) {
    const lines = text.split('\n');
    if (lines.length > 4 || text.length > 300) {
      e.preventDefault();
      if (pastedTexts.length >= 3) pastedTexts.shift();
      pasteCounter++;
      pastedTexts.push({ id: pasteCounter, text, lineCount: lines.length });
      renderPasteChips();
    }
  }
});

if (attachDropZone) {
  attachDropZone.addEventListener('dragover', (e) => {
    e.preventDefault();
    attachDropZone.classList.add('dragover');
  });
  attachDropZone.addEventListener('dragleave', () => attachDropZone.classList.remove('dragover'));
  attachDropZone.addEventListener('drop', (e) => {
    e.preventDefault();
    attachDropZone.classList.remove('dragover');
    if (activeSessionId === null) return;
    for (const file of e.dataTransfer?.files ?? []) {
      if (isImageFile(file)) stageAttach(file);
      else stageFileAttach(file);
    }
  });
  attachDropZone.addEventListener('click', () => attachFileInput?.click());
}

if (attachFileInput) {
  attachFileInput.addEventListener('change', () => {
    for (const file of attachFileInput.files ?? []) {
      if (isImageFile(file)) stageAttach(file);
      else stageFileAttach(file);
    }
    attachFileInput.value = '';
  });
}

// セッション内どこでもD&D（terminal-wrapper全体）
const terminalWrapper = document.getElementById('terminal-wrapper');
if (terminalWrapper) {
  terminalWrapper.addEventListener('click', (e) => {
    if (activeSessionId === null) return;
    if (isInteractiveFocusTarget(e.target)) return;
    const xt = terminals.get(activeSessionId);
    if (!xt?.term.hasSelection()) inputEl.focus();
  });

  // xterm.js canvas が click イベントを止める場合のフォールバック:
  // mouseup は canvas からもバブルするため、こちらで確実にフォーカスを戻す
  document.getElementById('terminal-area-wrapper')?.addEventListener('mouseup', () => {
    if (activeSessionId === null) return;
    const xt = terminals.get(activeSessionId);
    // 50ms 待って xterm の選択状態が確定してから判定
    setTimeout(() => { if (!xt?.term.hasSelection()) inputEl.focus(); }, 50);
  });

  let dragCounter = 0;
  terminalWrapper.addEventListener('dragenter', (e) => {
    if (!e.dataTransfer?.types.includes('Files')) return;
    e.preventDefault();
    if (++dragCounter === 1) terminalWrapper.classList.add('drag-active');
  });
  terminalWrapper.addEventListener('dragleave', () => {
    if (--dragCounter <= 0) {
      dragCounter = 0;
      terminalWrapper.classList.remove('drag-active');
    }
  });
  terminalWrapper.addEventListener('dragover', (e) => {
    if (e.dataTransfer?.types.includes('Files')) e.preventDefault();
  });
  terminalWrapper.addEventListener('drop', (e) => {
    e.preventDefault();
    dragCounter = 0;
    terminalWrapper.classList.remove('drag-active');
    if (activeSessionId === null) return;
    for (const file of e.dataTransfer?.files ?? []) {
      if (isImageFile(file)) stageAttach(file);
      else stageFileAttach(file);
    }
  });
}

async function stageAttach(file) {
  const normalized = await normalizeAttachImage(file);
  const buf = await normalized.arrayBuffer();
  if (buf.byteLength > MAX_ATTACH_BYTES) {
    showToast(`Attachment too large: ${(buf.byteLength / (1024 * 1024)).toFixed(1)}MB (max 8MB)`);
    return;
  }
  const entry = {};
  const wrapper = addAttachThumbnail(normalized, () => {
    const idx = pendingAttachFiles.findIndex(p => p.entry === entry);
    if (idx !== -1) pendingAttachFiles.splice(idx, 1);
  });
  entry.wrapper = wrapper;
  pendingAttachFiles.push({ buf, filename: normalized.name || '', entry, wrapper });
}

async function stageFileAttach(file) {
  const buf = await file.arrayBuffer();
  if (buf.byteLength > MAX_ATTACH_BYTES) {
    showToast(`Attachment too large: ${(buf.byteLength / (1024 * 1024)).toFixed(1)}MB (max 8MB)`);
    return;
  }
  const entry = {};
  const wrapper = addFileChip(file, () => {
    const idx = pendingAttachFiles.findIndex(p => p.entry === entry);
    if (idx !== -1) pendingAttachFiles.splice(idx, 1);
  });
  entry.wrapper = wrapper;
  pendingAttachFiles.push({ buf, filename: file.name || '', entry, wrapper });
}

// Claude 側の画像処理失敗を避けるため、長辺を抑えて標準JPEGへ再エンコードする。
// 変換に失敗した場合は元ファイルをそのまま使う。
async function normalizeAttachImage(file) {
  try {
    const maxEdge = 1568;
    const bmp = await createImageBitmap(file);
    const w = bmp.width;
    const h = bmp.height;
    const scale = Math.min(1, maxEdge / Math.max(w, h));
    const outW = Math.max(1, Math.round(w * scale));
    const outH = Math.max(1, Math.round(h * scale));

    const canvas = document.createElement('canvas');
    canvas.width = outW;
    canvas.height = outH;
    const ctx = canvas.getContext('2d');
    if (!ctx) {
      bmp.close();
      return file;
    }
    ctx.drawImage(bmp, 0, 0, outW, outH);
    bmp.close();

    const blob = await new Promise((resolve) => {
      canvas.toBlob(resolve, 'image/jpeg', 0.92);
    });
    if (!blob) return file;

    const base = (file.name || 'image').replace(/\.[^.]+$/, '');
    return new File([blob], `${base}.jpg`, { type: 'image/jpeg' });
  } catch (_) {
    return file;
  }
}

function arrayBufferToBase64(buf) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const s = String(reader.result || '');
      const comma = s.indexOf(',');
      resolve(comma >= 0 ? s.slice(comma + 1) : s);
    };
    reader.onerror = () => reject(reader.error || new Error('base64 encode failed'));
    reader.readAsDataURL(new Blob([buf]));
  });
}

async function flushPendingAttach(sessionId) {
  if (pendingAttachFiles.length === 0) return [];
  const toSend = pendingAttachFiles.splice(0);
  const injects = [];
  for (const { buf, filename, wrapper } of toSend) {
    try {
      const formData = new FormData();
      formData.append('file', new Blob([buf]), filename || 'image.jpg');
      const res = await fetch(
        `/api/attach?token=${encodeURIComponent(token)}&session_id=${encodeURIComponent(sessionId)}`,
        { method: 'POST', body: formData }
      );
      if (!res.ok) {
        showToast(`Attachment failed: HTTP ${res.status}`);
      } else {
        try {
          const data = await res.json();
          if (data && data.inject) injects.push(data.inject);
        } catch (_) {
          showToast('Attachment response parse failed');
        }
      }
    } catch (_) {
      showToast('Attachment send failed');
    }
    if (wrapper) setTimeout(() => { wrapper.remove(); updateAttachClearBtn(); }, 1000);
  }
  return injects;
}

function addAttachThumbnail(file, onRemove) {
  if (!attachThumbnails) return;
  const url = URL.createObjectURL(file);

  const wrapper = document.createElement('div');
  wrapper.className = 'attach-thumb-wrapper';

  const img = document.createElement('img');
  img.src = url;
  img.className = 'attach-thumb';
  img.title = (file.name || 'image') + t('expand_image');
  img.addEventListener('click', () => openLightbox(img.src));

  const removeBtn = document.createElement('button');
  removeBtn.className = 'attach-thumb-remove';
  removeBtn.textContent = t('remove');
  removeBtn.title = t('delete_attach');
  removeBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    URL.revokeObjectURL(url);
    wrapper.remove();
    updateAttachClearBtn();
    onRemove?.();
  });

  wrapper.appendChild(img);
  wrapper.appendChild(removeBtn);
  attachThumbnails.prepend(wrapper);

  const wrappers = attachThumbnails.querySelectorAll('.attach-thumb-wrapper');
  for (let i = 10; i < wrappers.length; i++) {
    URL.revokeObjectURL(wrappers[i].querySelector('img').src);
    wrappers[i].remove();
  }
  updateAttachClearBtn();
  return wrapper;
}

function addFileChip(file, onRemove) {
  if (!attachThumbnails) return;
  const wrapper = document.createElement('div');
  wrapper.className = 'attach-thumb-wrapper attach-file-chip';

  const label = document.createElement('span');
  label.className = 'attach-file-name';
  label.textContent = t('file_chip_label', { name: file.name || 'file' });
  label.title = file.name || 'file';

  const removeBtn = document.createElement('button');
  removeBtn.className = 'attach-thumb-remove';
  removeBtn.textContent = t('remove');
  removeBtn.title = t('delete_attach');
  removeBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    wrapper.remove();
    updateAttachClearBtn();
    onRemove?.();
  });

  wrapper.appendChild(label);
  wrapper.appendChild(removeBtn);
  attachThumbnails.prepend(wrapper);
  updateAttachClearBtn();
  return wrapper;
}

function openLightbox(src) {
  const overlay = document.createElement('div');
  overlay.id = 'image-lightbox';
  const img = document.createElement('img');
  img.src = src;
  overlay.appendChild(img);
  document.body.appendChild(overlay);
  const close = () => {
    overlay.remove();
    document.removeEventListener('keydown', onKey);
  };
  const onKey = (e) => { if (e.key === 'Escape') close(); };
  overlay.addEventListener('click', close);
  document.addEventListener('keydown', onKey);
}

(function () {
  const newSessionBtn   = document.getElementById('new-session-btn');
  const newSessionPanel = document.getElementById('new-session-panel');
  const spawnCwdInput   = document.getElementById('spawn-cwd');
  const spawnCwdBrowse  = document.getElementById('spawn-cwd-browse');
  const cwdDropdown     = document.getElementById('spawn-cwd-dropdown');
  const spawnCancelBtn  = document.getElementById('spawn-cancel-btn');
  const spawnLaunchBtn  = document.getElementById('spawn-launch-btn');
  const spawnProviderEl = document.getElementById('spawn-provider');
  const spawnProviderIcon = document.getElementById('spawn-provider-icon');
  const spawnCodexModelBtn = document.getElementById('spawn-codex-model-btn');
  const spawnClaudeModelBtn = document.getElementById('spawn-claude-model-btn');
  const spawnModelInput = document.getElementById('spawn-model');
  let codexModelSelection = null;
  let claudeModelSelection = null;

  function updateSpawnProviderIcon() {
    if (spawnProviderIcon) spawnProviderIcon.innerHTML = providerIconHtml(spawnProviderEl.value);
  }
  spawnProviderEl.addEventListener('change', () => {
    updateSpawnProviderIcon();
    const p = spawnProviderEl.value;
    document.getElementById('spawn-claude-opts').hidden = (p !== 'claude');
    document.getElementById('spawn-codex-opts').hidden  = (p !== 'codex');
    if (p !== 'codex')  codexModelSelection  = null;
    if (p !== 'claude') claudeModelSelection = null;
  });
  updateSpawnProviderIcon();

  function loadSpawnSettings() {
    try {
      const s = JSON.parse(localStorage.getItem(STORAGE_SPAWN_KEY) || '{}');
      if (s.provider) {
        spawnProviderEl.value = s.provider;
        const p = s.provider;
        document.getElementById('spawn-claude-opts').hidden = (p !== 'claude');
        document.getElementById('spawn-codex-opts').hidden  = (p !== 'codex');
      }
      if (s.cwd)              spawnCwdInput.value = s.cwd;
      if (s.model !== undefined) document.getElementById('spawn-model').value = s.model;
      if (s.permission_mode)  document.getElementById('spawn-permission-mode').value = s.permission_mode;
      if (s.sandbox)          document.getElementById('spawn-sandbox').value = s.sandbox;
      if (s.ask_for_approval) document.getElementById('spawn-ask-approval').value = s.ask_for_approval;
      return !!s.cwd;
    } catch (_) { return false; }
  }

  function saveSpawnSettings(obj) {
    try { localStorage.setItem(STORAGE_SPAWN_KEY, JSON.stringify(obj)); } catch (_) {}
  }

  function loadCwdHistory() {
    try { return JSON.parse(localStorage.getItem(STORAGE_CWD_HISTORY_KEY) || '[]'); } catch (_) { return []; }
  }

  function saveCwdHistory(cwd) {
    if (!cwd) return;
    const hist = loadCwdHistory().filter(v => v !== cwd);
    hist.unshift(cwd);
    if (hist.length > CWD_HISTORY_MAX) hist.length = CWD_HISTORY_MAX;
    try { localStorage.setItem(STORAGE_CWD_HISTORY_KEY, JSON.stringify(hist)); } catch (_) {}
  }

  function deleteCwdHistoryItem(cwd) {
    const hist = loadCwdHistory().filter(v => v !== cwd);
    try { localStorage.setItem(STORAGE_CWD_HISTORY_KEY, JSON.stringify(hist)); } catch (_) {}
  }

  function renderCwdDropdown(filter) {
    const hist = loadCwdHistory();
    const items = filter
      ? hist.filter(v => v.toLowerCase().includes(filter.toLowerCase()))
      : hist;
    if (items.length === 0) { cwdDropdown.hidden = true; return; }
    cwdDropdown.innerHTML = items.map(v =>
      `<li class="cwd-dropdown-item" tabindex="-1" data-value="${escapeHtml(v)}">` +
      `<span class="cwd-dropdown-label">${escapeHtml(v)}</span>` +
      `<button class="cwd-dropdown-del" tabindex="-1" data-value="${escapeHtml(v)}">×</button>` +
      `</li>`
    ).join('');
    cwdDropdown.hidden = false;
    applyDropdownMissingStatus();
    checkPathsExist(items).then(applyDropdownMissingStatus);
  }

  // path existence: 作業ディレクトリが実在しないと Cmd.Dir の chdir が
  // Windows で ERROR_DIRECTORY を返して spawn が失敗する。事前に弾いて
  // 起動ボタンを抑止し、ホバーで原因を出す。
  const pathExistsCache = new Map();
  let pathCheckDebounce = null;

  async function checkPathsExist(paths) {
    const unknown = [...new Set(paths)].filter(p => p && !pathExistsCache.has(p));
    if (unknown.length === 0) return;
    try {
      const res = await fetch(`/api/path-exists?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ paths: unknown }),
      });
      if (!res.ok) return;
      const data = await res.json();
      for (const [p, exists] of Object.entries(data.results || {})) {
        pathExistsCache.set(p, !!exists);
      }
    } catch (_) { /* 通信失敗時はキャッシュ更新せず楽観扱い */ }
  }

  function isPathMissing(p) {
    return pathExistsCache.has(p) && pathExistsCache.get(p) === false;
  }

  function applyCwdInputStatus() {
    const v = spawnCwdInput.value.trim();
    if (!v) {
      spawnCwdInput.classList.remove('is-missing');
      spawnCwdInput.removeAttribute('title');
      spawnLaunchBtn.disabled = true;
      spawnLaunchBtn.removeAttribute('title');
      return;
    }
    if (isPathMissing(v)) {
      spawnCwdInput.classList.add('is-missing');
      spawnCwdInput.title = t('spawn_cwd_missing', { path: v });
      spawnLaunchBtn.disabled = true;
      spawnLaunchBtn.title = t('spawn_cwd_missing_btn');
    } else {
      spawnCwdInput.classList.remove('is-missing');
      spawnCwdInput.removeAttribute('title');
      spawnLaunchBtn.disabled = false;
      spawnLaunchBtn.removeAttribute('title');
    }
  }

  async function refreshCwdInputStatus() {
    const v = spawnCwdInput.value.trim();
    if (v) await checkPathsExist([v]);
    applyCwdInputStatus();
  }

  function scheduleCwdInputCheck() {
    if (pathCheckDebounce) clearTimeout(pathCheckDebounce);
    pathCheckDebounce = setTimeout(refreshCwdInputStatus, 200);
  }

  function applyDropdownMissingStatus() {
    cwdDropdown.querySelectorAll('.cwd-dropdown-item').forEach(el => {
      const v = el.dataset.value;
      if (isPathMissing(v)) {
        el.classList.add('is-missing');
        el.title = t('spawn_cwd_missing', { path: v });
      } else {
        el.classList.remove('is-missing');
        el.removeAttribute('title');
      }
    });
  }

  // ボタン押下: パネル表示 + 保存設定を復元 / 未保存時は /api/info から CWD を取得
  newSessionBtn.addEventListener('click', async () => {
    if (!newSessionPanel.hidden) { newSessionPanel.hidden = true; return; }
    const hasSavedCwd = loadSpawnSettings();
    if (!hasSavedCwd) {
      try {
        const res = await fetch(`/api/info?token=${token}`);
        if (res.ok) spawnCwdInput.value = (await res.json()).cwd || '';
      } catch (_) {}
    }
    newSessionPanel.hidden = false;
    updateSpawnProviderIcon();
    spawnCwdInput.focus();
    refreshCwdInputStatus();
  });

  spawnCancelBtn.addEventListener('click', () => { newSessionPanel.hidden = true; });
  spawnLaunchBtn.addEventListener('click', spawnSession);
  if (spawnCwdBrowse) {
    spawnCwdBrowse.addEventListener('click', async () => {
      spawnCwdBrowse.disabled = true;
      try {
        const res = await fetch(`/api/pick-directory?token=${token}`, { method: 'POST' });
        if (res.ok) {
          const data = await res.json();
          if (data.ok && data.path) {
            spawnCwdInput.value = data.path;
            refreshCwdInputStatus();
          }
        }
      } catch (_) {}
      finally { spawnCwdBrowse.disabled = false; }
    });
  }
  spawnCwdInput.addEventListener('focus', () => { renderCwdDropdown(''); refreshCwdInputStatus(); });
  spawnCwdInput.addEventListener('input', () => { renderCwdDropdown(spawnCwdInput.value.trim()); scheduleCwdInputCheck(); });
  spawnCwdInput.addEventListener('blur',  () => setTimeout(() => { cwdDropdown.hidden = true; }, 150));
  spawnCwdInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter')  { cwdDropdown.hidden = true; if (!spawnLaunchBtn.disabled) spawnSession(); }
    if (e.key === 'Escape') { cwdDropdown.hidden = true; newSessionPanel.hidden = true; }
    if (e.key === 'ArrowDown' && !cwdDropdown.hidden) {
      e.preventDefault();
      const first = cwdDropdown.querySelector('.cwd-dropdown-item');
      if (first) first.focus();
    }
  });
  cwdDropdown.addEventListener('keydown', (e) => {
    const items = [...cwdDropdown.querySelectorAll('.cwd-dropdown-item')];
    const idx = items.indexOf(document.activeElement);
    if (e.key === 'ArrowDown') { e.preventDefault(); items[idx + 1]?.focus(); }
    if (e.key === 'ArrowUp')   { e.preventDefault(); idx > 0 ? items[idx - 1].focus() : spawnCwdInput.focus(); }
    if (e.key === 'Enter' && idx >= 0) { spawnCwdInput.value = items[idx].dataset.value; cwdDropdown.hidden = true; spawnCwdInput.focus(); refreshCwdInputStatus(); }
    if (e.key === 'Escape') { cwdDropdown.hidden = true; spawnCwdInput.focus(); }
  });
  cwdDropdown.addEventListener('mousedown', (e) => {
    e.preventDefault();
    const delBtn = e.target.closest('.cwd-dropdown-del');
    if (delBtn) {
      deleteCwdHistoryItem(delBtn.dataset.value);
      renderCwdDropdown(spawnCwdInput.value.trim());
      spawnCwdInput.focus();
      return;
    }
    const item = e.target.closest('.cwd-dropdown-item');
    if (!item) return;
    spawnCwdInput.value = item.dataset.value;
    cwdDropdown.hidden = true;
    spawnCwdInput.focus();
    refreshCwdInputStatus();
  });

  function isCodexHighRisk(currentModel, nextModel, sandbox, approval) {
    const current = (currentModel || '').trim();
    const next = (nextModel || '').trim();
    const modelChanged = !!next && next !== current;
    const permissionHigh = sandbox === 'danger-full-access' || approval === 'never';
    return modelChanged || permissionHigh;
  }

  function isClaudeHighRisk(currentModel, nextModel, permissionMode) {
    const current = (currentModel || '').trim();
    const next = (nextModel || '').trim();
    const modelChanged = !!next && next !== current;
    const permissionHigh = permissionMode === 'bypassPermissions';
    return modelChanged || permissionHigh;
  }

  // provider共通のモデル選択モーダル
  // isHighRiskFn(candidateModel) → bool
  // opts: { titleKey, permSummaryKey }
  function openModelModal(currentModel, isHighRiskFn, opts) {
    return new Promise((resolve) => {
      const overlay = document.getElementById('model-picker-overlay');
      if (!overlay) { resolve(null); return; }
      const display = (currentModel || '').trim() || '(none)';
      overlay.innerHTML = '';
      overlay.hidden = false;

      const dialog = document.createElement('div');
      dialog.className = 'model-picker-dialog';
      dialog.innerHTML = `
        <div class="model-picker-title">${escapeHtml(t(opts.titleKey))}</div>
        <div class="model-picker-current">${escapeHtml(t('model_current', { model: display }))}</div>
        <label class="model-picker-note">${escapeHtml(t('model_candidate'))}</label>
        <input class="model-picker-input" id="model-candidate-input" type="text" value="${escapeHtml(currentModel || '')}">
        <div class="model-picker-note">${escapeHtml(t('model_summary'))}</div>
        <div class="model-picker-note">- ${escapeHtml(t('model_summary_cost'))}</div>
        <div class="model-picker-note">- ${escapeHtml(t(opts.permSummaryKey))}</div>
        <div class="model-picker-note">- ${escapeHtml(t('model_summary_compat'))}</div>
        <label class="model-picker-check" id="model-risk-check-wrap" hidden>
          <input id="model-risk-check" type="checkbox">
          <span>${escapeHtml(t('model_require_confirm'))}</span>
        </label>
        <div class="model-picker-actions">
          <button class="model-picker-btn" id="model-cancel-btn">${escapeHtml(t('model_cancel'))}</button>
          <button class="model-picker-btn primary" id="model-apply-btn">${escapeHtml(t('model_apply'))}</button>
        </div>
      `;
      overlay.appendChild(dialog);

      const input = document.getElementById('model-candidate-input');
      const riskWrap = document.getElementById('model-risk-check-wrap');
      const riskCheck = document.getElementById('model-risk-check');
      const applyBtn = document.getElementById('model-apply-btn');
      const cancelBtn = document.getElementById('model-cancel-btn');

      function refreshRisk() {
        const highRisk = isHighRiskFn(input.value);
        riskWrap.hidden = !highRisk;
        applyBtn.disabled = highRisk && !riskCheck.checked;
      }
      function close(v) {
        overlay.removeEventListener('click', onOverlayClick);
        overlay.hidden = true;
        overlay.innerHTML = '';
        resolve(v);
      }
      function onOverlayClick(e) {
        if (e.target === overlay) close(null);
      }

      input.addEventListener('input', refreshRisk);
      riskCheck.addEventListener('change', refreshRisk);
      cancelBtn.addEventListener('click', () => close(null));
      applyBtn.addEventListener('click', () => {
        const candidate = input.value.trim();
        if (!candidate) {
          alert(t('model_model_required'));
          return;
        }
        const highRisk = isHighRiskFn(candidate);
        close({
          model: candidate,
          mode: highRisk ? 'required' : 'explicit',
          risk_confirmed: highRisk ? !!riskCheck.checked : false,
        });
      });
      overlay.addEventListener('click', onOverlayClick);
      input.focus();
      refreshRisk();
    });
  }

  function openCodexModelModal() {
    const currentModel = (spawnModelInput.value || '').trim();
    const sandbox = document.getElementById('spawn-sandbox').value;
    const approval = document.getElementById('spawn-ask-approval').value;
    return openModelModal(
      currentModel,
      (candidate) => isCodexHighRisk(spawnModelInput.value, candidate, sandbox, approval),
      { titleKey: 'codex_model_title', permSummaryKey: 'codex_model_summary_permission' }
    );
  }

  function openClaudeModelModal() {
    const currentModel = (spawnModelInput.value || '').trim();
    const permMode = document.getElementById('spawn-permission-mode').value;
    return openModelModal(
      currentModel,
      (candidate) => isClaudeHighRisk(spawnModelInput.value, candidate, permMode),
      { titleKey: 'claude_model_title', permSummaryKey: 'claude_model_summary_permission' }
    );
  }

  if (spawnCodexModelBtn) {
    spawnCodexModelBtn.addEventListener('click', async () => {
      const picked = await openCodexModelModal();
      if (!picked) return;
      spawnModelInput.value = picked.model;
      codexModelSelection = picked;
    });
  }

  if (spawnClaudeModelBtn) {
    spawnClaudeModelBtn.addEventListener('click', async () => {
      const picked = await openClaudeModelModal();
      if (!picked) return;
      spawnModelInput.value = picked.model;
      claudeModelSelection = picked;
    });
  }

  async function spawnSession() {
    const provider = document.getElementById('spawn-provider').value;
    const cwd = spawnCwdInput.value.trim();
    spawnLaunchBtn.disabled = true;
    try {
      const model = spawnModelInput.value.trim();
      const label = document.getElementById('spawn-label').value.trim();
      const bodyObj = { provider, cwd, model, label };
      if (provider === 'claude') {
        const picked = claudeModelSelection;
        const permMode = document.getElementById('spawn-permission-mode').value;
        const highRisk = isClaudeHighRisk('', model, permMode);
        const pickedConfirmed = !!picked?.risk_confirmed;
        let riskConfirmed = pickedConfirmed;

        if (highRisk && !riskConfirmed) {
          riskConfirmed = await appConfirm({
            title: t('claude_model_confirm_title'),
            message: t('claude_model_confirm_message'),
            confirmText: t('claude_model_confirm_run'),
            cancelText: t('spawn_cancel'),
            kind: 'danger',
          });
          if (!riskConfirmed) {
            spawnLaunchBtn.disabled = false;
            return;
          }
        }

        bodyObj.permission_mode = permMode;
        bodyObj.model_selection_mode = picked ? picked.mode : 'auto';
        bodyObj.risk_confirmed = riskConfirmed;
      } else if (provider === 'codex') {
        const picked = codexModelSelection;
        const sandbox = document.getElementById('spawn-sandbox').value;
        const approval = document.getElementById('spawn-ask-approval').value;
        const highRisk = isCodexHighRisk('', model, sandbox, approval);
        const pickedConfirmed = !!picked?.risk_confirmed;
        let riskConfirmed = pickedConfirmed;

        if (highRisk && !riskConfirmed) {
          riskConfirmed = await appConfirm({
            title: t('codex_model_confirm_title'),
            message: t('codex_model_confirm_message'),
            confirmText: t('codex_model_confirm_run'),
            cancelText: t('spawn_cancel'),
            kind: 'danger',
          });
          if (!riskConfirmed) {
            spawnLaunchBtn.disabled = false;
            return;
          }
        }

        bodyObj.model_selection_mode = highRisk ? 'required' : (picked?.mode || 'auto');
        bodyObj.risk_confirmed = riskConfirmed;
        bodyObj.sandbox = sandbox;
        bodyObj.ask_for_approval = approval;
      }
      const res = await fetch(`/api/spawn?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(bodyObj),
      });
      if (res.ok) {
        saveCwdHistory(cwd);
        saveSpawnSettings({
          provider,
          cwd,
          model,
          ...(provider === 'claude' ? { permission_mode: bodyObj.permission_mode } : {}),
          ...(provider === 'codex'  ? { sandbox: bodyObj.sandbox, ask_for_approval: bodyObj.ask_for_approval } : {}),
        });
        document.getElementById('spawn-label').value = '';
        codexModelSelection  = null;
        claudeModelSelection = null;
        newSessionPanel.hidden = true;
        pendingAutoSwitch = true;
      } else {
        alert(t('spawn_failed') + await res.text());
      }
    } catch (e) {
      alert(t('spawn_failed') + e.message);
    } finally {
      spawnLaunchBtn.disabled = false;
    }
  }
})();

(function () {
  const killAllBtn = document.getElementById('kill-all-btn');
  if (!killAllBtn) return;

  killAllBtn.addEventListener('click', async () => {
    const ok = await appConfirm({
      title: t('kill_all_confirm_title'),
      message: t('kill_all_confirm'),
      confirmText: t('kill_all_confirm_run'),
      cancelText: t('spawn_cancel'),
      kind: 'danger',
    });
    if (!ok) return;
    killAllBtn.disabled = true;
    try {
      await fetch(`/api/kill-all?token=${token}`, { method: 'POST' });
    } catch (_) {}
    killAllBtn.disabled = false;
  });
})();

(function () {
  const shutdownBtn = document.getElementById('shutdown-btn');
  if (!shutdownBtn) return;

  shutdownBtn.addEventListener('click', async () => {
    const result = await appConfirmShutdown();
    if (!result) return;
    shutdownBtn.disabled = true;
    if (result === 'sessions') {
      try { await fetch(`/api/kill-all?token=${token}`, { method: 'POST' }); } catch (_) {}
      try { await fetch(`/api/shutdown?token=${token}`, { method: 'POST' }); } catch (_) {}
      window.close();
    } else {
      try {
        await fetch(`/api/shutdown?token=${token}`, { method: 'POST' });
      } catch (_) {}
      window.close();
    }
  });
})();

(function () {
  const idleTimeoutEl     = document.getElementById('idle-timeout-min');
  const reconnectGraceEl  = document.getElementById('reconnect-grace-min');
  const logEnabledEl      = document.getElementById('log-enabled');
  const logMaxSizeEl      = document.getElementById('log-max-size');
  const logMaxBackupsEl   = document.getElementById('log-max-backups');

  async function loadIdleTimeout() {
    if (!idleTimeoutEl) return;
    try {
      const res = await fetch(`/api/idle-timeout?token=${token}`);
      if (!res.ok) return;
      const cfg = await res.json();
      idleTimeoutEl.value = cfg.idle_timeout_min;
    } catch (_) {}
  }

  async function saveIdleTimeout() {
    if (!idleTimeoutEl) return;
    const raw = parseInt(idleTimeoutEl.value, 10);
    const min = Number.isFinite(raw) ? Math.max(0, Math.min(1440, raw)) : 60;
    idleTimeoutEl.value = String(min);
    try {
      await fetch(`/api/idle-timeout?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ idle_timeout_min: min }),
      });
    } catch (_) {}
  }

  async function loadReconnectGrace() {
    if (!reconnectGraceEl) return;
    try {
      const res = await fetch(`/api/reconnect-grace?token=${token}`);
      if (!res.ok) return;
      const cfg = await res.json();
      const sec = Number(cfg.wrapper_reconnect_grace_sec) || 0;
      reconnectGraceEl.value = String(Math.round(sec / 60));
    } catch (_) {}
  }

  async function saveReconnectGrace() {
    if (!reconnectGraceEl) return;
    const raw = parseInt(reconnectGraceEl.value, 10);
    const min = Number.isFinite(raw) ? Math.max(0, Math.min(1440, raw)) : 60;
    reconnectGraceEl.value = String(min);
    try {
      await fetch(`/api/reconnect-grace?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ wrapper_reconnect_grace_sec: min * 60 }),
      });
    } catch (_) {}
  }

  async function loadLogConfig() {
    try {
      const res = await fetch(`/api/log-config?token=${token}`);
      if (!res.ok) return;
      const cfg = await res.json();
      logEnabledEl.checked  = cfg.enabled;
      logMaxSizeEl.value    = cfg.max_size_mb;
      logMaxBackupsEl.value = cfg.max_backups;
      const logDirBtn = document.getElementById('log-dir-btn');
      if (logDirBtn && cfg.log_dir) {
        logDirBtn.dataset.tooltip = cfg.log_dir;
      }
      const attachDirBtn = document.getElementById('attach-dir-btn');
      if (attachDirBtn && cfg.attach_dir) {
        attachDirBtn.dataset.tooltip = cfg.attach_dir;
      }
    } catch (_) {}
  }

  async function openDirOrCopy(btn, kind) {
    const path = btn.dataset.tooltip;
    if (!path || path === t('loading')) return;
    try {
      const res = await fetch(`/api/open-dir?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ kind }),
      });
      if (res.ok) return;
    } catch (_) {}
    try {
      await navigator.clipboard.writeText(path);
      const prev = btn.dataset.tooltip;
      btn.dataset.tooltip = t('copied_to_clipboard');
      setTimeout(() => { btn.dataset.tooltip = prev; }, 1500);
    } catch (_) {}
  }

  const logDirBtn = document.getElementById('log-dir-btn');
  if (logDirBtn) {
    logDirBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      openDirOrCopy(logDirBtn, 'log');
    });
  }

  const attachDirBtn = document.getElementById('attach-dir-btn');
  if (attachDirBtn) {
    attachDirBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      openDirOrCopy(attachDirBtn, 'attach');
    });
  }

  (function () {
    const KEY = 'any-ai-cli.settings-section-state';
    let state = {};
    try { state = JSON.parse(localStorage.getItem(KEY) || '{}') || {}; } catch (_) { state = {}; }
    document.querySelectorAll('.settings-section[data-section]').forEach((el) => {
      const id = el.dataset.section;
      if (state[id]) el.open = true;
      el.addEventListener('toggle', () => {
        state[id] = el.open;
        try { localStorage.setItem(KEY, JSON.stringify(state)); } catch (_) {}
      });
    });
  })();

  const approvalToggleInput = document.getElementById('approval-toggle-input');
  if (approvalToggleInput) {
    approvalToggleInput.addEventListener('change', async () => {
      const endpoint = approvalToggleInput.checked ? 'enable' : 'disable';
      try {
        await fetch(`/api/approval/${endpoint}?token=${token}`, { method: 'POST' });
      } catch (_) {}
    });
  }
  const approvalAutoSwitchInput = document.getElementById('approval-auto-switch-input');
  if (approvalAutoSwitchInput) {
    approvalAutoSwitchInput.checked = localStorage.getItem(STORAGE_APPROVAL_AUTO_SWITCH_KEY) === '1';
    approvalAutoSwitchInput.addEventListener('change', () => {
      try { localStorage.setItem(STORAGE_APPROVAL_AUTO_SWITCH_KEY, approvalAutoSwitchInput.checked ? '1' : '0'); } catch (_) {}
      if (approvalAutoSwitchInput.checked) maybeAutoSwitchToNextApproval();
    });
  }

  async function saveLogConfig() {
    try {
      await fetch(`/api/log-config?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          enabled:      logEnabledEl.checked,
          max_size_mb:  parseInt(logMaxSizeEl.value, 10) || 10,
          max_backups:  parseInt(logMaxBackupsEl.value, 10) || 3,
          compress:     false,
        }),
      });
    } catch (_) {}
  }

  async function saveSlashCmdSources() {
    const body = {
      claude: (document.getElementById('slash-src-claude')?.value || '').trim(),
      codex:  (document.getElementById('slash-src-codex')?.value  || '').trim(),
    };
    try {
      await fetch(`/api/slash-cmd-sources?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
    } catch (_) {}
  }

  window.__settingsSaveAll = async () => {
    await saveIdleTimeout();
    await saveReconnectGrace();
    await saveLogConfig();
    await saveSlashCmdSources();
    saveUsageLinkSettings();
  };

  window.__settingsResetAll = async () => {
    applyTheme('dark');
    applyFontSize('medium');
    window.setLang('ja');

    try {
      localStorage.setItem(STORAGE_TRIGGER_ENABLED_KEY, '0');
      localStorage.setItem(STORAGE_TRIGGER_PHRASE_KEY, '');
      localStorage.setItem(STORAGE_WAKE_WORD_ENABLED_KEY, '0');
      localStorage.setItem(STORAGE_WAKE_WORD_PHRASE_KEY, DEFAULT_WAKE_WORD_PHRASE);
      localStorage.setItem(STORAGE_NOTIFY_SOUND_ENABLED_KEY, '0');
      localStorage.setItem(STORAGE_NOTIFY_SOUND_TYPE_KEY, 'default');
      localStorage.removeItem(STORAGE_NOTIFY_SOUND_CUSTOM_KEY);
      localStorage.setItem(STORAGE_APPROVAL_AUTO_SWITCH_KEY, '0');
      localStorage.setItem(STORAGE_QUICK_CMD_1_KEY, DEFAULT_QUICK_CMD_1);
      localStorage.setItem(STORAGE_QUICK_CMD_2_KEY, DEFAULT_QUICK_CMD_2);
      localStorage.removeItem(STORAGE_USAGE_LINK_CLAUDE_KEY);
      localStorage.removeItem(STORAGE_USAGE_LINK_CODEX_KEY);
      localStorage.setItem(STORAGE_VOICE_GRACE_KEY, String(DEFAULT_VOICE_GRACE_SEC));
    } catch (_) {}

    const triggerEnabled = document.getElementById('trigger-enabled');
    const triggerPhrase  = document.getElementById('trigger-phrase-input');
    const triggerRow     = document.getElementById('trigger-phrase-row');
    if (triggerEnabled) triggerEnabled.checked = false;
    if (triggerPhrase) triggerPhrase.value = '';
    if (triggerRow) triggerRow.hidden = true;

    const wakeWordEnabled = document.getElementById('wakeword-enabled');
    const wakeWordPhrase  = document.getElementById('wakeword-phrase-input');
    const wakeWordRow     = document.getElementById('wakeword-phrase-row');
    if (wakeWordEnabled) wakeWordEnabled.checked = false;
    if (wakeWordPhrase) wakeWordPhrase.value = DEFAULT_WAKE_WORD_PHRASE;
    if (wakeWordRow) wakeWordRow.hidden = true;
    document.dispatchEvent(new CustomEvent('wakewordsettings:changed'));

    const soundEnabledEl  = document.getElementById('notify-sound-enabled');
    const soundTypeEl     = document.getElementById('notify-sound-type');
    const soundTypeRow    = document.getElementById('notify-sound-type-row');
    const soundCustomRow  = document.getElementById('notify-sound-custom-row');
    const soundFilenameEl = document.getElementById('notify-sound-filename');
    const soundFileEl     = document.getElementById('notify-sound-file');
    if (soundEnabledEl) soundEnabledEl.checked = false;
    if (soundTypeEl) soundTypeEl.value = 'default';
    if (soundTypeRow) soundTypeRow.hidden = true;
    if (soundCustomRow) soundCustomRow.hidden = true;
    if (soundFilenameEl) soundFilenameEl.textContent = '';
    if (soundFileEl) soundFileEl.value = '';

    const quickCmd1El = document.getElementById('quick-cmd-1');
    const quickCmd2El = document.getElementById('quick-cmd-2');
    if (quickCmd1El) quickCmd1El.value = DEFAULT_QUICK_CMD_1;
    if (quickCmd2El) quickCmd2El.value = DEFAULT_QUICK_CMD_2;

    const voiceGraceEl = document.getElementById('voice-grace-select');
    if (voiceGraceEl) {
      voiceGraceEl.value = String(DEFAULT_VOICE_GRACE_SEC);
      try { localStorage.setItem(STORAGE_VOICE_GRACE_KEY, String(DEFAULT_VOICE_GRACE_SEC)); } catch (_) {}
    }

    const approvalAutoSwitchInput = document.getElementById('approval-auto-switch-input');
    if (approvalAutoSwitchInput) approvalAutoSwitchInput.checked = false;

    const idleTimeoutEl = document.getElementById('idle-timeout-min');
    const reconnectGraceEl = document.getElementById('reconnect-grace-min');
    const logEnabledEl = document.getElementById('log-enabled');
    const logMaxSizeEl = document.getElementById('log-max-size');
    const logMaxBackupsEl = document.getElementById('log-max-backups');
    if (idleTimeoutEl) idleTimeoutEl.value = '60';
    if (reconnectGraceEl) reconnectGraceEl.value = '60';
    if (logEnabledEl) logEnabledEl.checked = true;
    if (logMaxSizeEl) logMaxSizeEl.value = '10';
    if (logMaxBackupsEl) logMaxBackupsEl.value = '3';

    const approvalToggleInput = document.getElementById('approval-toggle-input');
    if (approvalToggleInput) {
      approvalToggleInput.checked = false;
      try { await fetch(`/api/approval/disable?token=${token}`, { method: 'POST' }); } catch (_) {}
    }

    const slashClaudeEl = document.getElementById('slash-src-claude');
    const slashCodexEl = document.getElementById('slash-src-codex');
    if (slashClaudeEl) slashClaudeEl.value = '';
    if (slashCodexEl) slashCodexEl.value = '';
    loadUsageLinkSettings();

    const foAppEl = document.getElementById('settings-file-open-app');
    const termAppEl = document.getElementById('settings-terminal-app');
    if (foAppEl) foAppEl.value = '';
    if (termAppEl) termAppEl.value = '';

    await window.__settingsSaveAll();
    await loadApprovalSettings();
  };

  // 設定パネルが開かれたときにログ設定を読み込む
  document.getElementById('settings-btn').addEventListener('click', () => {
    if (!document.getElementById('settings-panel').hidden) {
      loadIdleTimeout();
      loadReconnectGrace();
      loadLogConfig();
      loadApprovalSettings();
      loadSlashCmdSources();
      loadUsageLinkSettings();
    }
  });

  if (idleTimeoutEl) idleTimeoutEl.addEventListener('change', saveIdleTimeout);
  if (reconnectGraceEl) reconnectGraceEl.addEventListener('change', saveReconnectGrace);
  logEnabledEl.addEventListener('change', saveLogConfig);
  logMaxSizeEl.addEventListener('change', saveLogConfig);
  logMaxBackupsEl.addEventListener('change', saveLogConfig);
})();

(function () {
  const resizer  = document.getElementById('sidebar-resizer');
  const sidebar  = document.getElementById('session-list');
  if (!resizer || !sidebar) return;

  const STORAGE_KEY = 'ai_cli_hub_sidebar_width';
  const MIN = 160, MAX = 520;

  const saved = parseInt(localStorage.getItem(STORAGE_KEY), 10);
  if (saved >= MIN && saved <= MAX) sidebar.style.width = saved + 'px';

  let startX = 0, startW = 0;

  function onMove(e) {
    const dx = (e.clientX || (e.touches && e.touches[0].clientX) || 0) - startX;
    const w = Math.min(MAX, Math.max(MIN, startW + dx));
    sidebar.style.width = w + 'px';
    try { localStorage.setItem(STORAGE_KEY, w); } catch (_) {}
    renderSessionList();
    // ターミナルの幅変化に追従
    terminals.forEach((t, id) => {
      if (!canFitTerminal(t)) return;
      const prevCols = t.term.cols;
      const prevRows = t.term.rows;
      fitTerminalPreservingBottom(t, id);
      if (t.term.cols !== prevCols || t.term.rows !== prevRows) {
        sendResize(id, t.term.cols, t.term.rows);
      }
    });
  }

  function onUp() {
    resizer.classList.remove('dragging');
    document.removeEventListener('mousemove', onMove);
    document.removeEventListener('mouseup', onUp);
    document.body.style.cursor = '';
    document.body.style.userSelect = '';
  }

  resizer.addEventListener('mousedown', (e) => {
    e.preventDefault();
    startX = e.clientX;
    startW = sidebar.getBoundingClientRect().width;
    resizer.classList.add('dragging');
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  });
})();

// ---- 音声入力 ----
(function () {
  const SpeechRecognition = window.SpeechRecognition || window.webkitSpeechRecognition;
  if (!SpeechRecognition) return;

  // Chrome / Edge (Chromium) のみ表示。Safari の webkitSpeechRecognition は動作不安定のため除外。
  const isChromium = navigator.userAgentData?.brands?.some(b => /Chromium/.test(b.brand))
    ?? /Chrome\//.test(navigator.userAgent);
  if (!isChromium) return;

  const btn        = document.getElementById('voice-btn');
  const voiceBar   = document.getElementById('voice-bar');
  const canvas     = document.getElementById('voice-waveform');
  const cancelBtn  = document.getElementById('voice-cancel-btn');
  const confirmBtn = document.getElementById('voice-confirm-btn');
  if (!btn || !voiceBar || !canvas) return;

  btn.hidden = false;
  btn.dataset.tooltip = t('voice_tooltip');

  const recognition = new SpeechRecognition();
  recognition.interimResults = true;
  recognition.continuous = false;
  recognition.maxAlternatives = 1;

  let isRecording  = false;
  let isStarting   = false;
  // ウェイクワード IIFE への排他フラグ。
  // btn.click 直後（hw.abort() 前）に true にして、`hw.end` で走る再起動タイマーを抑止する。
  // forceCleanup() で false に戻す（録音失敗・正常終了・キャンセル共通の出口）。
  let voiceIntent  = false;
  let interimStart = 0;
  let preVoiceText = '';

  let animFrame = null;
  let wavePhase = 0;
  let waveformRaf = null;

  // 発話強度 (0..1)。soundstart/speechstart で上がり、無音で下がる。
  // result イベントで一時的にキックすることで「話している瞬間」が視覚的に伝わる。
  let voiceIntensity = 0;
  let voiceIntensityTarget = 0;
  let lastInterimLen = 0;
  let lastKickAt = 0;

  // 自動 restart 用フラグ群
  // 設定値 grace 秒以内に最後の result が出ていれば、Chrome の auto-end 後に recognition.start() を再呼び出しして発話を継続させる。
  let userIntendedStop = false;   // ✓/✕/Esc/トリガーフレーズ/致命エラー等で停止 (auto-restart を抑止)
  let restartTimer = null;
  let isAutoRestarting = false;   // 直後の 'start' イベントが自動 restart 由来かを判定
  let lastResultAt = 0;           // 最終 result イベント時刻 (performance.now())
  let silenceStopTimer = null;

  function getVoiceGraceSec() {
    const raw = localStorage.getItem(STORAGE_VOICE_GRACE_KEY);
    const v = raw == null ? DEFAULT_VOICE_GRACE_SEC : parseInt(raw, 10);
    return Number.isFinite(v) ? Math.max(0, Math.min(5, v)) : DEFAULT_VOICE_GRACE_SEC;
  }
  function clearRestartTimer() {
    if (restartTimer) {
      clearTimeout(restartTimer);
      restartTimer = null;
    }
  }
  function clearSilenceTimer() {
    clearTimeout(silenceStopTimer);
    silenceStopTimer = null;
  }
  function resetSilenceTimer() {
    clearSilenceTimer();
    const grace = getVoiceGraceSec();
    if (grace <= 0 || !isRecording || userIntendedStop) return;
    silenceStopTimer = setTimeout(() => {
      silenceStopTimer = null;
      if (!isRecording || userIntendedStop) return;
      userIntendedStop = true;
      clearRestartTimer();
      try { recognition.abort(); } catch (_) {}
      scheduleForceCleanup();
    }, grace * 1000);
  }

  const BAR_COUNT = 48;

  function getLang() {
    const lang = localStorage.getItem(STORAGE_LANG_KEY) || 'ja';
    return lang === 'ja' ? 'ja-JP' : 'en-US';
  }

  function formatVoiceError(key, code) {
    const msg = t(key);
    if (!code) return msg;
    return msg.replace('{code}', code);
  }

  function normalizeVoiceErrorCode(error) {
    const raw = typeof error === 'string' ? error : (error?.error || error?.name || error?.message || '');
    return String(raw || 'unknown').trim() || 'unknown';
  }

  function showVoiceError(error, anchor) {
    const code = normalizeVoiceErrorCode(error);
    if (code === 'not-allowed' || code === 'permission-denied') {
      showToast(t('voice_error_permission'), anchor);
    } else if (code === 'audio-capture') {
      showToast(t('voice_error_audio_capture'), anchor);
    } else if (code === 'network') {
      showToast(t('voice_error_network'), anchor);
    } else if (code === 'service-not-allowed') {
      showToast(t('voice_error_service'), anchor);
    } else if (code === 'language-not-supported') {
      showToast(t('voice_error_language'), anchor);
    } else {
      showToast(formatVoiceError('voice_error_detail', code), anchor);
    }
  }
  window._showVoiceRecognitionError = showVoiceError;
  window._voiceIntentActive = () => voiceIntent;

  function resizeCanvas() {
    const r = canvas.getBoundingClientRect();
    if (r.width > 0) {
      canvas.width  = Math.round(r.width  * devicePixelRatio);
      canvas.height = Math.round(r.height * devicePixelRatio);
    }
  }

  function drawBars() {
    const ctx2d = canvas.getContext('2d');
    if (!ctx2d) return;
    const W = canvas.width;
    const H = canvas.height;
    ctx2d.clearRect(0, 0, W, H);
    const barW = Math.max(2, Math.floor(W / (BAR_COUNT * 1.8)));
    const gap  = (W - BAR_COUNT * barW) / (BAR_COUNT + 1);
    // 強度を滑らかに追従させる (1フレームあたり線形補間)
    voiceIntensity += (voiceIntensityTarget - voiceIntensity) * 0.18;
    // 発話キックの減衰
    const sinceKick = (performance.now() - lastKickAt) / 1000;
    const kick = Math.max(0, 1 - sinceKick * 3);
    const active = Math.min(1, voiceIntensity + kick * 0.6);

    for (let i = 0; i < BAR_COUNT; i++) {
      // 複数の正弦波 + 擬似ノイズで「波形っぽい」分布を作る
      const phase = wavePhase + i * 0.42;
      const lo = Math.sin(phase) * 0.5 + 0.5;
      const hi = Math.sin(phase * 2.7 + i * 0.13) * 0.5 + 0.5;
      const rnd = (Math.sin(phase * 7.3 + i) + 1) * 0.5;
      const wave = (lo * 0.4 + hi * 0.4 + rnd * 0.2);
      // active が低いときは静止に近い小振幅、active が高いほど振幅・コントラスト増
      const baseAmp = 0.08;
      const dynAmp  = 0.92 * active;
      const v = baseAmp + wave * dynAmp;

      const barH = Math.max(barW, v * H * 0.92);
      const x = gap + i * (barW + gap);
      const y = (H - barH) / 2;
      ctx2d.fillStyle = `rgba(59,130,246,${Math.min(1, 0.35 + v * 0.85)})`;
      if (ctx2d.roundRect) {
        ctx2d.beginPath();
        ctx2d.roundRect(x, y, barW, barH, barW / 2);
        ctx2d.fill();
      } else {
        ctx2d.fillRect(x, y, barW, barH);
      }
    }
  }

  function animLoop() {
    drawBars();
    // active が高いほど波が早く動く (見た目の躍動感)
    wavePhase += 0.18 + voiceIntensity * 0.35;
    animFrame = requestAnimationFrame(animLoop);
  }

  function startWaveform() {
    resizeCanvas();
    cancelAnimationFrame(animFrame);
    wavePhase = 0;
    voiceIntensity = 0;
    voiceIntensityTarget = 0.05;
    lastInterimLen = 0;
    lastKickAt = 0;
    animFrame = requestAnimationFrame(animLoop);
  }

  function stopWaveform() {
    cancelAnimationFrame(animFrame);
    animFrame = null;
  }

  function showVoiceBar() {
    const t = activeSessionId === null ? null : terminals.get(activeSessionId);
    const shouldStickToBottom = !!(t && (t.autoScroll || isTerminalAtBottom(t)));
    voiceBar.hidden = false;
    waveformRaf = requestAnimationFrame(() => {
      resizeCanvas();
      waveformRaf = null;
    });
    // NOTE: getUserMedia({audio:true}) を併用すると Chrome の SpeechRecognition と
    // マイクを奪い合い、波形は出るのに result イベントが届かなくなる (0d4f787 で一度修正済)。
    // 波形は SpeechRecognition の audiostart/soundstart/speechstart/result から
    // voiceIntensityTarget を駆動するルートだけで賄う。
    startWaveform();
    refitActiveTerminalAfterLayout(shouldStickToBottom);
  }

  function hideVoiceBar() {
    const t = activeSessionId === null ? null : terminals.get(activeSessionId);
    const shouldStickToBottom = !!(t && (t.autoScroll || isTerminalAtBottom(t)));
    if (waveformRaf) {
      cancelAnimationFrame(waveformRaf);
      waveformRaf = null;
    }
    stopWaveform();
    voiceBar.hidden = true;
    refitActiveTerminalAfterLayout(shouldStickToBottom);
  }

  function setVoiceAudioActive(active) {
    if (voiceAudioActive === active) return;
    voiceAudioActive = active;
    document.dispatchEvent(new CustomEvent('voiceinput:statechanged'));
  }

  let forceCleanupTimer = null;
  function forceCleanup() {
    userIntendedStop = true;
    voiceIntent = false;
    clearSilenceTimer();
    clearRestartTimer();
    clearBeginRetryTimer();
    isStarting = false;
    isRecording = false;
    voiceActive = false;
    setVoiceAudioActive(false);
    btn.classList.remove('recording');
    btn.dataset.tooltip = t('voice_tooltip');
    hideVoiceBar();
    setTimeout(() => inputEl.focus(), 0);
    document.dispatchEvent(new CustomEvent('voiceinput:stopped'));
  }
  function scheduleForceCleanup() {
    clearTimeout(forceCleanupTimer);
    forceCleanupTimer = setTimeout(() => {
      // voiceIntent も判定対象に入れる: retry 中（recognition.start が未成功）に
      // マイクボタン 2 度押しで停止扱いになった場合、他フラグは全て false でも
      // voiceIntent だけ true で残り、wakeword IIFE が再起動できなくなる。
      if (isRecording || isStarting || voiceActive || voiceIntent) forceCleanup();
    }, 1500);
  }
  function cancelForceCleanup() {
    clearTimeout(forceCleanupTimer);
    forceCleanupTimer = null;
  }

  let beginRetryTimer = null;
  function clearBeginRetryTimer() {
    if (beginRetryTimer) {
      clearTimeout(beginRetryTimer);
      beginRetryTimer = null;
    }
  }

  function beginVoiceRecognition(retryCount = 0) {
    clearBeginRetryTimer();
    userIntendedStop = false;
    if (retryCount === 0) {
      clearSilenceTimer();
      isAutoRestarting = false;
      // result が一度も来ていない状態を表すセンチネル。
      // クリック直後に Chrome が end を返すケース（権限プロンプト直後・無音タイムアウト等）で
      // 誤って auto-restart ループに入らないよう、ここでは 0 にしておき end ハンドラで判別する。
      lastResultAt = 0;
      preVoiceText = inputEl.value;
      interimStart = inputEl.value.length;
    }
    recognition.lang = getLang();
    isStarting = true;
    try {
      recognition.start();
    } catch (err) {
      isStarting = false;
      // InvalidStateError: 直前のセッション (voice 本体 / wakeword hw) が
      // まだ完全に終了しておらず Chrome の認識サービスが transient 状態。
      // abort で確実に止めてから短いバックオフで再試行する。
      const isInvalidState = err && (err.name === 'InvalidStateError' || /already started/i.test(err.message || ''));
      if (isInvalidState && retryCount < 4) {
        try { recognition.abort(); } catch (_) {}
        const delay = 200 + retryCount * 200;
        beginRetryTimer = setTimeout(() => {
          beginRetryTimer = null;
          if (userIntendedStop || isRecording) return;
          beginVoiceRecognition(retryCount + 1);
        }, delay);
        return;
      }
      forceCleanup();
      showVoiceError(err, btn);
      console.error('SpeechRecognition start failed:', err);
    }
  }

  btn.addEventListener('click', () => {
    if (isRecording || isStarting || beginRetryTimer) {
      userIntendedStop = true;
      clearSilenceTimer();
      clearRestartTimer();
      clearBeginRetryTimer();
      try { recognition.abort(); } catch (_) {}
      scheduleForceCleanup();
      return;
    }
    // hw.abort() で hw.end が遅延発火しても再起動を抑止するため、abort 前にフラグを立てる
    voiceIntent = true;
    let stopPromise = null;
    if (typeof window._stopWakewordForVoiceInput === 'function') {
      const ret = window._stopWakewordForVoiceInput();
      if (ret && typeof ret.then === 'function') stopPromise = ret;
    }
    if (stopPromise) {
      // hw のマイク解放後に start することで、Chrome のマイク取り合いで result イベントが
      // 届かなくなる症状（波形は出るが入力欄にテキストが入らない）を回避する。
      stopPromise.then(() => {
        if (!voiceIntent) return; // 既にキャンセルされた場合はスキップ
        beginVoiceRecognition();
      });
    } else {
      beginVoiceRecognition();
    }
  });

  cancelBtn.addEventListener('click', () => {
    userIntendedStop = true;
    clearSilenceTimer();
    clearRestartTimer();
    clearBeginRetryTimer();
    inputEl.value = preVoiceText;
    autoExpand();
    try { recognition.abort(); } catch (_) {}
    scheduleForceCleanup();
  });

  confirmBtn.addEventListener('click', () => {
    userIntendedStop = true;
    clearSilenceTimer();
    clearRestartTimer();
    clearBeginRetryTimer();
    try { recognition.stop(); } catch (_) {}
    scheduleForceCleanup();
  });

  recognition.addEventListener('start', () => {
    isStarting = false;
    isRecording = true;
    voiceActive = true;
    btn.classList.add('recording');
    btn.dataset.tooltip = t('voice_recording');
    document.dispatchEvent(new CustomEvent('voiceinput:started'));
    // 自動 restart 時は既にバーが表示中で intensity リセットも不要
    if (!isAutoRestarting) {
      showVoiceBar();
    }
    isAutoRestarting = false;
    resetSilenceTimer();
  });

  recognition.addEventListener('audiostart', () => {
    setVoiceAudioActive(true);
    voiceIntensityTarget = 0.15;
  });
  recognition.addEventListener('soundstart', () => { voiceIntensityTarget = 0.55; lastKickAt = performance.now(); });
  recognition.addEventListener('speechstart', () => { voiceIntensityTarget = 0.9;  lastKickAt = performance.now(); });
  recognition.addEventListener('speechend',   () => { voiceIntensityTarget = 0.25; });
  recognition.addEventListener('soundend',    () => { voiceIntensityTarget = 0.08; });
  recognition.addEventListener('audioend',    () => {
    setVoiceAudioActive(false);
    voiceIntensityTarget = 0.03;
  });

  recognition.addEventListener('result', (e) => {
    const result = e.results[e.resultIndex];
    if (!result) return;
    const transcript = result[0].transcript;
    // interim 結果が伸びた = 今まさに発話中、を視覚化するキック
    if (transcript.length > lastInterimLen) {
      lastKickAt = performance.now();
      voiceIntensityTarget = Math.max(voiceIntensityTarget, 0.85);
    }
    lastInterimLen = result.isFinal ? 0 : transcript.length;
    lastResultAt = performance.now();
    resetSilenceTimer();
    inputEl.value = inputEl.value.slice(0, interimStart) + transcript;
    if (result.isFinal) {
      inputEl.value += ' ';
      interimStart = inputEl.value.length;
      const _tp = getActiveTriggerPhrase();
      if (_tp && activeSessionId !== null && textEndsWithTriggerPhrase(buildSendText(), _tp)) {
        userIntendedStop = true;
        clearSilenceTimer();
        clearRestartTimer();
        recognition.stop();
        doSend(activeSessionId);
        return;
      }
    }
    autoExpand();
    updateSlashMenu();
  });

  recognition.addEventListener('end', () => {
    cancelForceCleanup();
    isStarting = false;
    setVoiceAudioActive(false);
    const grace = getVoiceGraceSec();
    const hasResult = lastResultAt > 0;
    const sinceLastResult = hasResult ? (performance.now() - lastResultAt) / 1000 : Infinity;
    if (!userIntendedStop && grace > 0 && isRecording && hasResult && sinceLastResult < grace) {
      // 直近の発話から grace 秒以内なので、短い間隔で recognition を再開して継続させる
      clearRestartTimer();
      restartTimer = setTimeout(() => {
        restartTimer = null;
        if (!isRecording || userIntendedStop) return;
        isAutoRestarting = true;
        try {
          recognition.start();
        } catch (err) {
          isAutoRestarting = false;
          console.warn('SpeechRecognition auto-restart failed:', err);
          forceCleanup();
        }
      }, 120);
      return;
    }
    forceCleanup();
  });

  recognition.addEventListener('error', (e) => {
    console.warn('SpeechRecognition error:', e.error, e.message || '');
    isStarting = false;
    setVoiceAudioActive(false);
    // 致命系エラーは auto-restart 抑止 (繰り返してもまた失敗するため)。
    // 'no-speech' と 'aborted' はソフトエラーで、'end' イベントが判定する。
    const fatalErrors = ['not-allowed', 'permission-denied', 'audio-capture', 'network', 'service-not-allowed', 'language-not-supported'];
    if (fatalErrors.includes(e.error)) {
      userIntendedStop = true;
      clearSilenceTimer();
      clearRestartTimer();
    }
    if (e.error !== 'no-speech' && e.error !== 'aborted') showVoiceError(e, btn);
    // クリーンアップは 'end' イベントに任せる (auto-restart 判定の単一窓口にするため)
  });

  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && isRecording) {
      e.preventDefault();
      cancelBtn.click();
      return;
    }
    if (e.altKey && e.code === 'KeyV' && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      btn.click();
    }
  });
  window.addEventListener('resize', () => {
    if (isRecording) resizeCanvas();
  });
})();

// ---- ウェイクワード検出（グローバルトグル＋セッション個別トグル＋入力欄ホバー起動） ----
(function () {
  const SpeechRecognition = window.SpeechRecognition || window.webkitSpeechRecognition;
  if (!SpeechRecognition) return;
  const isChromium = navigator.userAgentData?.brands?.some(b => /Chromium/.test(b.brand))
    ?? /Chrome\//.test(navigator.userAgent);
  if (!isChromium) return;

  const globalBtn  = document.getElementById('global-wakeword-btn');
  const sessionBtn = document.getElementById('voice-wakeword-btn');
  const voiceBtn   = document.getElementById('voice-btn');
  const inputBar   = document.getElementById('input-bar');
  if (!globalBtn || !sessionBtn || !voiceBtn || !inputBar) return;

  globalBtn.hidden  = false;
  sessionBtn.hidden = false;
  globalBtn.dataset.tooltip  = t('voice_wakeword_tooltip');
  sessionBtn.dataset.tooltip = t('voice_wakeword_session_tooltip');

  // セッション個別の ON/OFF 状態 (sessionId -> boolean)
  const sessionWakeMap = new Map();

  let isGlobalActive = false;  // ヘッダーボタンの状態
  let isHovered      = false;  // マウスが #input-bar 上にあるか
  let isListening    = false;
  let isStarting     = false;
  let restartTimer   = null;

  function sessionActive() {
    return activeSessionId !== null && (sessionWakeMap.get(activeSessionId) || false);
  }

  function getWakePhrase() {
    if (localStorage.getItem(STORAGE_WAKE_WORD_ENABLED_KEY) !== '1') return '';
    return (localStorage.getItem(STORAGE_WAKE_WORD_PHRASE_KEY) ?? DEFAULT_WAKE_WORD_PHRASE).trim();
  }

  function isWakewordEnabled() {
    return localStorage.getItem(STORAGE_WAKE_WORD_ENABLED_KEY) === '1';
  }

  function getLang() {
    const lang = localStorage.getItem(STORAGE_LANG_KEY) || 'ja';
    return lang === 'ja' ? 'ja-JP' : 'en-US';
  }

  const hw = new SpeechRecognition();
  hw.interimResults = true;
  hw.continuous = false;
  hw.maxAlternatives = 1;

  // グローバル ON または 当該セッション個別 ON、かつ入力欄ホバー中
  // voiceIntent: 音声入力ボタン押下直後〜recognition.start 成功までの「これから録音」の意思表示。
  // Chrome は同一ページの SpeechRecognition を並行起動できないため、この間 hw を再起動するとマイクを奪い合って InvalidStateError になる。
  function canListen() {
    const voiceBusy = voiceActive || (typeof window._voiceIntentActive === 'function' && window._voiceIntentActive());
    return isWakewordEnabled() && (isGlobalActive || sessionActive()) && isHovered && !voiceBusy;
  }

  function startHotword() {
    if (!canListen() || isListening || isStarting) return;
    isStarting = true;
    hw.lang = getLang();
    try { hw.start(); } catch (err) {
      isStarting = false;
      console.warn('Wake word recognition start failed:', err);
    }
  }

  function stopHotword() {
    clearTimeout(restartTimer);
    restartTimer = null;
    try { hw.abort(); } catch (_) {}
  }

  // 戻り値: Promise<boolean>（true = 直前まで hw が active だった）
  // hw が active だった場合は hw.end イベント＋短い余裕（マイクキャプチャ解放待ち）を経てから resolve する。
  // 過去事例: hw.abort() は非同期で、直後に同期で recognition.start() を呼ぶと
  // Chrome のマイクが半分掴まれた状態で start し、audiostart は発火するが result が届かない
  // 「波形は出るがテキストが入らない」症状になる（voice_input_text_not_inserted_2026-05-14.md 系）。
  function stopHotwordForVoiceInput() {
    const wasActive = isListening || isStarting;
    if (!wasActive) {
      stopHotword();
      isListening = false;
      isStarting = false;
      updateMicChip();
      return Promise.resolve(false);
    }
    return new Promise((resolve) => {
      let settled = false;
      const finish = () => {
        if (settled) return;
        settled = true;
        hw.removeEventListener('end', onEnd);
        isListening = false;
        isStarting = false;
        updateMicChip();
        // Chrome のマイクキャプチャ解放を待つ短い余裕（経験則: 50ms）。
        setTimeout(() => resolve(true), 50);
      };
      const onEnd = () => finish();
      hw.addEventListener('end', onEnd);
      stopHotword();
      // セーフティ: 500ms 経っても end が来なければ強制 resolve（壊れた状態でブロックしない）。
      setTimeout(finish, 500);
    });
  }

  function updateMicChip() {
    const chip = document.getElementById('mic-status-chip');
    if (!chip) return;
    const hasVoiceRecordingClass = voiceBtn.classList.contains('recording');
    const isVoiceRecording = voiceActive && voiceAudioActive && hasVoiceRecordingClass;
    if (voiceActive && !hasVoiceRecordingClass) voiceActive = false;
    const wakeEnabled = isWakewordEnabled();
    const wakeActive = wakeEnabled && (isGlobalActive || sessionActive());
    if (isVoiceRecording) {
      chip.hidden = false;
      chip.className = 'status-chip status-chip--running status-chip--blink';
      chip.textContent = t('mic_chip_recording');
    } else if (wakeActive && isListening) {
      chip.hidden = false;
      chip.className = 'status-chip status-chip--running status-chip--blink';
      chip.textContent = t('mic_chip_listening');
    } else if (wakeActive) {
      chip.hidden = false;
      chip.className = 'status-chip status-chip--standby';
      chip.textContent = t('mic_chip_standby');
    } else {
      chip.hidden = true;
      chip.textContent = '';
    }
  }

  function updateGlobalBtn() {
    if (!isWakewordEnabled()) {
      globalBtn.classList.remove('standby', 'listening');
      globalBtn.dataset.tooltip = t('voice_wakeword_tooltip');
    } else if (!isGlobalActive) {
      globalBtn.classList.remove('standby', 'listening');
      globalBtn.dataset.tooltip = t('voice_wakeword_tooltip');
    } else if (isHovered) {
      globalBtn.classList.add('standby', 'listening');
      globalBtn.dataset.tooltip = t('voice_wakeword_listening');
    } else {
      globalBtn.classList.add('standby');
      globalBtn.classList.remove('listening');
      globalBtn.dataset.tooltip = t('voice_wakeword_armed');
    }
    updateMicChip();
    updateSessionBtn();
    renderSessionList();
  }

  function updateSessionBtn() {
    const on = sessionActive();
    const effectiveOn = isWakewordEnabled() && (on || isGlobalActive);
    if (effectiveOn && isHovered) {
      sessionBtn.classList.add('standby', 'listening');
      sessionBtn.dataset.tooltip = t('voice_wakeword_listening');
    } else if (effectiveOn) {
      sessionBtn.classList.add('standby');
      sessionBtn.classList.remove('listening');
      sessionBtn.dataset.tooltip = t(on ? 'voice_wakeword_session_armed' : 'voice_wakeword_armed');
    } else {
      sessionBtn.classList.remove('standby', 'listening');
      sessionBtn.dataset.tooltip = t('voice_wakeword_session_tooltip');
    }
    updateMicChip();
  }

  hw.addEventListener('start', () => {
    isStarting = false;
    isListening = true;
    updateMicChip();
  });

  hw.addEventListener('result', (e) => {
    const phrase = normalizeTriggerMatchText(getWakePhrase());
    if (!phrase) return;
    for (let i = e.resultIndex; i < e.results.length; i++) {
      const raw = e.results[i][0].transcript;
      if (normalizeTriggerMatchText(raw).includes(phrase)) {
        stopHotwordForVoiceInput();
        if (!voiceActive) voiceBtn.click();
        return;
      }
    }
  });

  hw.addEventListener('end', () => {
    isListening = false;
    isStarting = false;
    updateMicChip();
    if (!canListen()) return;
    clearTimeout(restartTimer);
    restartTimer = setTimeout(() => {
      restartTimer = null;
      if (canListen()) startHotword();
    }, 250);
  });

  hw.addEventListener('error', (e) => {
    isStarting = false;
    const fatal = ['not-allowed', 'permission-denied', 'audio-capture', 'network', 'service-not-allowed', 'language-not-supported'];
    if (fatal.includes(e.error)) {
      isGlobalActive = false;
      if (activeSessionId !== null) sessionWakeMap.set(activeSessionId, false);
      updateGlobalBtn();
      updateSessionBtn();
      updateMicChip();
      if (typeof window._showVoiceRecognitionError === 'function') {
        window._showVoiceRecognitionError(e, globalBtn);
      } else {
        showToast(t('voice_error_detail').replace('{code}', e.error || 'unknown'), globalBtn);
      }
    }
  });

  // メイン録音終了後にホットワード監視を再アーム（hover 中かつ ON のセッションのみ）
  document.addEventListener('voiceinput:stopped', () => {
    updateMicChip();
    if (!canListen() || isListening || isStarting) return;
    clearTimeout(restartTimer);
    restartTimer = setTimeout(() => {
      restartTimer = null;
      if (canListen()) startHotword();
    }, 300);
  });

  document.addEventListener('voiceinput:started', () => { updateMicChip(); });
  document.addEventListener('voiceinput:statechanged', () => { updateMicChip(); });

  // マウスが入力欄に入ったら認識開始、出たら停止
  inputBar.addEventListener('mouseenter', () => {
    isHovered = true;
    updateGlobalBtn();
    updateSessionBtn();
    if (canListen() && !voiceActive) startHotword();
  });

  inputBar.addEventListener('mouseleave', () => {
    isHovered = false;
    updateGlobalBtn();
    updateSessionBtn();
    stopHotword();
  });

  // ヘッダーのグローバルボタン
  globalBtn.addEventListener('click', () => {
    if (!isWakewordEnabled()) {
      isGlobalActive = false;
      sessionWakeMap.clear();
      updateGlobalBtn();
      stopHotword();
      return;
    }
    isGlobalActive = !isGlobalActive;
    updateGlobalBtn();
    if (isGlobalActive && isHovered && !voiceActive) startHotword();
    if (!isGlobalActive && !sessionActive()) stopHotword();
  });

  // 入力バーのセッション個別ボタン
  sessionBtn.addEventListener('click', () => {
    if (activeSessionId === null) return;
    if (!isWakewordEnabled()) {
      sessionWakeMap.set(activeSessionId, false);
      updateSessionBtn();
      stopHotword();
      return;
    }
    const cur = sessionWakeMap.get(activeSessionId) || false;
    sessionWakeMap.set(activeSessionId, !cur);
    updateGlobalBtn();
    if (!cur && isHovered && !voiceActive) startHotword();
    if (cur && !isGlobalActive) stopHotword();
  });

  // セッション切り替え時にセッションボタンの状態を反映（activateSession から呼ばれる）
  window._wakewordSessionChanged = () => {
    updateGlobalBtn();
    if (isHovered) {
      if (canListen() && !isListening && !isStarting) startHotword();
      else if (!canListen()) stopHotword();
    }
  };

  document.addEventListener('wakewordsettings:changed', () => {
    if (!isWakewordEnabled()) {
      isGlobalActive = false;
      sessionWakeMap.clear();
      stopHotword();
    }
    updateGlobalBtn();
    updateSessionBtn();
    updateMicChip();
  });

  updateGlobalBtn();

  document.addEventListener('keydown', (e) => {
    if (e.altKey && e.code === 'KeyW' && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      globalBtn.click();
    }
  });

  window._wakewordGlobalActive = () => isGlobalActive;
  window._wakewordSessionActive = (id) => sessionWakeMap.get(id) || false;
  window._stopWakewordForVoiceInput = stopHotwordForVoiceInput;
})();

// ---- スラッシュコマンドピッカー ----
(function () {
  const pickerEl       = document.getElementById('slash-picker');
  const titleEl        = document.getElementById('slash-picker-title');
  const timeEl         = document.getElementById('slash-picker-time');
  const searchEl       = document.getElementById('slash-picker-search');
  const listEl         = document.getElementById('slash-picker-list');
  const refreshBtn     = document.getElementById('slash-picker-refresh');
  const closeBtn       = document.getElementById('slash-picker-close');
  const pickerBtn      = document.getElementById('slash-picker-btn');
  if (!pickerEl || !pickerBtn) return;

  let pickerProvider = null;
  let pickerData     = null; // { cmds, fetched_at, source_url }

  pickerBtn.addEventListener('click', async () => {
    if (!pickerEl.hidden) { hidePicker(); return; }
    const sess = sessions.get(activeSessionId);
    const provider = sess?.provider || 'claude';
    await openPicker(provider, false);
  });

  refreshBtn.addEventListener('click', async (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (pickerProvider) await openPicker(pickerProvider, true);
  });

  closeBtn.addEventListener('click', (e) => {
    e.preventDefault();
    e.stopPropagation();
    hidePicker();
  });

  searchEl.addEventListener('input', () => renderList(searchEl.value));

  searchEl.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') { hidePicker(); e.preventDefault(); }
  });

  document.addEventListener('mousedown', (e) => {
    if (!pickerEl.hidden && !pickerEl.contains(e.target) && e.target !== pickerBtn) {
      hidePicker();
    }
  });

  async function openPicker(provider, forceRefresh) {
    pickerProvider = provider;
    pickerEl.hidden = false;
    titleEl.textContent = provider === 'claude' ? 'Claude Code' : 'Codex CLI';
    timeEl.textContent  = '';
    listEl.innerHTML = `<div class="slash-picker-status">${t('slash_picker_loading')}</div>`;
    searchEl.value = '';
    try {
      const method = forceRefresh ? 'POST' : 'GET';
      const resp = await fetch(`/api/slash-commands?provider=${provider}&token=${token}`, { method });
      if (!resp.ok) {
        const txt = await resp.text();
        if (resp.status === 404) {
          listEl.innerHTML = `<div class="slash-picker-status slash-picker-status--warn">${t('slash_picker_not_configured')}</div>`;
        } else {
          listEl.innerHTML = `<div class="slash-picker-status slash-picker-status--error">${t('slash_picker_error')}</div>`;
        }
        return;
      }
      pickerData = await resp.json();
      timeEl.textContent = formatAge(pickerData.fetched_at);
      renderList('');
      setTimeout(() => searchEl.focus(), 0);
    } catch (_) {
      listEl.innerHTML = `<div class="slash-picker-status slash-picker-status--error">${t('slash_picker_error')}</div>`;
    }
  }

  function renderList(filter) {
    if (!pickerData) return;
    const cmds = pickerData.cmds || [];
    const q = filter.trim().toLowerCase();
    const filtered = q
      ? cmds.filter(c => c.cmd.includes(q) || (c.desc || '').toLowerCase().includes(q))
      : cmds;
    if (filtered.length === 0) {
      listEl.innerHTML = `<div class="slash-picker-status">${t('slash_picker_empty')}</div>`;
      return;
    }
    listEl.innerHTML = '';
    for (const item of filtered) {
      const div = document.createElement('div');
      div.className = 'slash-picker-item';
      const cmdSpan = document.createElement('span');
      cmdSpan.className = 'slash-picker-cmd';
      cmdSpan.textContent = item.cmd;
      const descSpan = document.createElement('span');
      descSpan.className = 'slash-picker-desc';
      descSpan.textContent = item.desc || '';
      if (item.desc) descSpan.title = item.desc;
      div.appendChild(cmdSpan);
      div.appendChild(descSpan);
      div.addEventListener('mousedown', (e) => {
        e.preventDefault();
        if (activeSessionId !== null) sendQuickCommand(activeSessionId, item.cmd);
        hidePicker();
      });
      listEl.appendChild(div);
    }
  }

  function hidePicker() {
    pickerEl.hidden = true;
    pickerData = null;
  }

  function formatAge(iso) {
    if (!iso) return '';
    const diffMs = Date.now() - new Date(iso).getTime();
    const m = Math.floor(diffMs / 60000);
    if (m < 1)  return t('slash_picker_just_now');
    if (m < 60) return t('slash_picker_ago_min').replace('{n}', m);
    const h = Math.floor(m / 60);
    if (h < 24) return t('slash_picker_ago_hour').replace('{n}', h);
    return t('slash_picker_ago_day').replace('{n}', Math.floor(h / 24));
  }
})();

// ---- スラッシュコマンドソース設定 ----
async function loadSlashCmdSources() {
  const claudeEl = document.getElementById('slash-src-claude');
  const codexEl  = document.getElementById('slash-src-codex');
  if (!claudeEl || !codexEl) return;
  try {
    const resp = await fetch(`/api/slash-cmd-sources?token=${token}`);
    if (!resp.ok) return;
    const data = await resp.json();
    claudeEl.value = data.claude || '';
    codexEl.value  = data.codex  || '';
  } catch (_) {}
}

(function () {
  const saveBtn = document.getElementById('slash-src-save-btn');
  if (!saveBtn) return;
  saveBtn.addEventListener('click', async () => {
    const body = {
      claude: (document.getElementById('slash-src-claude')?.value || '').trim(),
      codex:  (document.getElementById('slash-src-codex')?.value  || '').trim(),
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

// ---- v2: Files タブシステム ----

// localStorage キー移行（旧 docs キー → 新 files キー、1 回限り）
(function migrateDocsToFilesLS() {
  const moves = [
    ['any-ai-cli.docs.tabs',        'any-ai-cli.files.tabs'],
    ['any_ai_cli_docs_tree_width',  'any_ai_cli_files_tree_width'],
  ];
  for (const [oldK, newK] of moves) {
    try {
      const v = localStorage.getItem(oldK);
      if (v != null && localStorage.getItem(newK) == null) {
        localStorage.setItem(newK, v);
      }
      if (v != null) localStorage.removeItem(oldK);
    } catch (_) {}
  }
})();

/**
 * FilesTabManager — メインエリアのタブ管理
 *
 * - openFilesTab(sessionId, projectKey, filesRoot) → tabId
 * - closeFilesTab(tabId)
 * - switchToSessionView()   ← セッションカード切替時に呼ぶ
 * - updateSessionTabLabel(label) ← セッション切替時にセッションタブの表示名を更新
 * - restoreFromLocalStorage()  ← 起動時に呼ぶ
 */
const FilesTabManager = (function () {
  const LS_KEY = 'any-ai-cli.files.tabs';
  const tabList = document.getElementById('main-tab-list');
  const tabBar  = document.getElementById('main-tab-bar');
  const terminalWrapper = document.getElementById('terminal-wrapper');
  const filesContents   = document.getElementById('files-tab-contents');

  if (!tabList || !tabBar || !terminalWrapper || !filesContents) {
    return {
      openFilesTab: () => null,
      closeFilesTab: () => {},
      switchToSessionView: () => {},
      updateSessionTabLabel: () => {},
      restoreFromLocalStorage: () => {},
    };
  }

  // タブデータ構造: { id, type: 'session'|'files', label, sessionId?, filesRoot?, gitRoot?, el, contentEl }
  let tabs = [];
  let activeTabId = null;
  let sessionTabEl = null;  // セッションタブ（常に1枚）

  // ─── LS 読み書き ───────────────────────────────────────────────────────
  function lsLoad() {
    try { return JSON.parse(localStorage.getItem(LS_KEY) || '{}'); } catch (_) { return {}; }
  }
  function lsSave(data) {
    try { localStorage.setItem(LS_KEY, JSON.stringify(data)); } catch (_) {}
  }
  function lsAddTab(gitRoot, filesRoot) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!data[gitRoot]) data[gitRoot] = [];
    // 同じ root が既にある場合は重複しない
    if (!data[gitRoot].some(e => e.root === filesRoot)) {
      data[gitRoot].push({ root: filesRoot, openedFile: null });
    }
    lsSave(data);
  }
  function lsRemoveTab(gitRoot, filesRoot) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!data[gitRoot]) return;
    data[gitRoot] = data[gitRoot].filter(e => e.root !== filesRoot);
    if (data[gitRoot].length === 0) delete data[gitRoot];
    lsSave(data);
  }
  /** ファイル選択時に呼ぶ用の公開関数 */
  function lsUpdateOpenedFile(gitRoot, filesRoot, filePath) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!data[gitRoot]) return;
    const entry = data[gitRoot].find(e => e.root === filesRoot);
    if (entry) { entry.openedFile = filePath; lsSave(data); }
  }

  // ─── DOM ──────────────────────────────────────────────────────────────
  function makeid() { return 'dtab-' + Math.random().toString(36).slice(2, 9); }

  function ensureSessionTab() {
    if (sessionTabEl) return;
    sessionTabEl = document.createElement('button');
    sessionTabEl.className = 'main-tab main-tab-session active';
    sessionTabEl.dataset.tabId = 'session';
    sessionTabEl.textContent = t('files_tab_session_label');
    sessionTabEl.addEventListener('click', () => switchToSessionView());
    tabList.insertBefore(sessionTabEl, tabList.firstChild);
    activeTabId = 'session';
  }

  function showTabBar() {
    tabBar.style.display = '';
  }

  function setActive(tabId) {
    activeTabId = tabId;
    // タブボタン
    tabList.querySelectorAll('.main-tab').forEach(el => {
      el.classList.toggle('active', el.dataset.tabId === tabId);
    });
    // コンテンツ
    const isSession = (tabId === 'session');
    if (isSession) {
      terminalWrapper.style.display = '';
      filesContents.classList.remove('visible');
      filesContents.querySelectorAll('.files-tab-content').forEach(el => el.classList.remove('active'));
      // display:none で隠れていた間に ResizeObserver が 0 幅で fit() を呼んでいる可能性があるため、
      // 表示復帰後にレイアウト確定を待って refit する。これをしないと xterm の cols が極小のまま残り、
      // 文字が縦に細く折り返される（depth padding 修正と同系統の "MD で出した narrow 表示" 事象）。
      if (typeof refitActiveTerminalAfterLayout === 'function') {
        refitActiveTerminalAfterLayout(true);
      }
    } else {
      terminalWrapper.style.display = 'none';
      filesContents.classList.add('visible');
      filesContents.querySelectorAll('.files-tab-content').forEach(el => {
        el.classList.toggle('active', el.dataset.tabId === tabId);
      });
    }
  }

  // ─── 公開 API ─────────────────────────────────────────────────────────

  function openFilesTab(sessionId, projectKey, filesRoot, gitRoot) {
    ensureSessionTab();
    showTabBar();

    // 同じ root のタブが既にあればそちらをアクティブに
    const existing = tabs.find(t => t.type === 'files' && t.filesRoot === filesRoot);
    if (existing) {
      setActive(existing.id);
      return existing.id;
    }

    const id = makeid();
    const displayName = projectKey && projectKey !== '__no_project__' ? projectKey : filesRoot;
    // タブラベルは "📁 <projectName>/<開いたディレクトリの basename>" 形式。
    const rootBase = (filesRoot || '').replace(/[\\/]+$/, '').split(/[\\/]/).pop() || filesRoot || '';
    const label = '📁 ' + displayName + '/' + rootBase;

    // タブボタン DOM
    const tabBtn = document.createElement('button');
    tabBtn.className = 'main-tab';
    tabBtn.dataset.tabId = id;

    const labelSpan = document.createElement('span');
    labelSpan.textContent = label;
    tabBtn.appendChild(labelSpan);

    const closeBtn = document.createElement('button');
    closeBtn.className = 'main-tab-close';
    closeBtn.textContent = '×';
    closeBtn.title = t('files_tab_close_tooltip');
    closeBtn.addEventListener('click', (e) => { e.stopPropagation(); closeFilesTab(id); });
    tabBtn.appendChild(closeBtn);

    tabBtn.addEventListener('click', () => setActive(id));
    tabList.appendChild(tabBtn);

    // コンテンツ DOM（2ペインスケルトン）
    const contentEl = document.createElement('div');
    contentEl.className = 'files-tab-content';
    contentEl.dataset.tabId = id;
    contentEl.dataset.filesRoot = filesRoot;
    contentEl.dataset.sessionId = sessionId || '';
    contentEl.dataset.gitRoot = gitRoot || '';
    contentEl.dataset.projectKey = projectKey || '';
    contentEl.innerHTML = `
      <div class="files-tab-placeholder">
        <div class="files-tab-toolbar" data-files-toolbar="${id}"></div>
        <div class="files-tab-panes">
          <div class="files-tab-tree-pane" data-files-tree="${id}">${escapeHtml(t('files_tab_loading'))}</div>
          <div class="files-tab-tree-resizer" data-files-tree-resizer="${id}"></div>
          <div class="files-tab-preview-pane" data-files-preview="${id}">${escapeHtml(t('files_tab_loading'))}</div>
        </div>
      </div>
    `;
    filesContents.appendChild(contentEl);

    // ツリーペイン幅をリストア + リサイザー配線
    setupFilesTreeResizer(contentEl);

    const tabObj = { id, type: 'files', label, sessionId, filesRoot, gitRoot, projectKey, el: tabBtn, contentEl };
    tabs.push(tabObj);

    // localStorage に保存
    if (gitRoot) lsAddTab(gitRoot, filesRoot);

    setActive(id);
    return id;
  }

  const FILES_TREE_WIDTH_KEY = 'any_ai_cli_files_tree_width';
  const FILES_TREE_MIN = 140;
  const FILES_TREE_MAX = 640;

  function getFilesTreeSavedWidth() {
    const v = parseInt(localStorage.getItem(FILES_TREE_WIDTH_KEY), 10);
    if (!isFinite(v)) return null;
    if (v < FILES_TREE_MIN || v > FILES_TREE_MAX) return null;
    return v;
  }

  function setupFilesTreeResizer(contentEl) {
    const pane = contentEl.querySelector('.files-tab-tree-pane');
    const resizer = contentEl.querySelector('.files-tab-tree-resizer');
    if (!pane || !resizer) return;
    const saved = getFilesTreeSavedWidth();
    if (saved != null) pane.style.width = saved + 'px';

    let startX = 0, startW = 0;

    function onMove(e) {
      const cx = e.clientX || (e.touches && e.touches[0] && e.touches[0].clientX) || 0;
      const dx = cx - startX;
      const w = Math.min(FILES_TREE_MAX, Math.max(FILES_TREE_MIN, startW + dx));
      // 全ての files タブのツリーペインを同期
      document.querySelectorAll('.files-tab-tree-pane').forEach(el => { el.style.width = w + 'px'; });
      try { localStorage.setItem(FILES_TREE_WIDTH_KEY, String(w)); } catch (_) {}
    }
    function onUp() {
      resizer.classList.remove('dragging');
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    }
    resizer.addEventListener('mousedown', (e) => {
      e.preventDefault();
      startX = e.clientX;
      startW = pane.getBoundingClientRect().width;
      resizer.classList.add('dragging');
      document.body.style.cursor = 'col-resize';
      document.body.style.userSelect = 'none';
      document.addEventListener('mousemove', onMove);
      document.addEventListener('mouseup', onUp);
    });
  }

  function closeFilesTab(id) {
    const idx = tabs.findIndex(t => t.id === id);
    if (idx === -1) return;
    const tabObj = tabs[idx];
    // DOM 削除
    tabObj.el.remove();
    tabObj.contentEl.remove();
    tabs.splice(idx, 1);
    // LS から削除
    if (tabObj.gitRoot) lsRemoveTab(tabObj.gitRoot, tabObj.filesRoot);
    // アクティブだったら session ビューに戻る
    if (activeTabId === id) {
      switchToSessionView();
    }
    // タブが Files タブだけゼロになったらタブバーを隠す（セッションタブのみ）
    if (tabs.length === 0) {
      // セッションタブも不要なら消す
      if (sessionTabEl) { sessionTabEl.remove(); sessionTabEl = null; }
      tabBar.style.display = 'none';
    }
  }

  function switchToSessionView() {
    ensureSessionTab();
    showTabBar();
    setActive('session');
  }

  function updateSessionTabLabel(label) {
    if (sessionTabEl) {
      sessionTabEl.textContent = label || t('files_tab_session_label');
    }
  }

  async function restoreFromLocalStorage() {
    // /api/info からセッション一覧を取得して gitRoot → projectKey のマッピングを試みる
    let sessions = [];
    try {
      const res = await fetch(`/api/info?token=${encodeURIComponent(token)}`);
      if (res.ok) { const d = await res.json(); sessions = d.sessions || []; }
    } catch (err) {
      console.warn('[FilesTabManager] /api/info fetch error:', err);
    }

    const data = lsLoad();
    for (const [gitRoot, entries] of Object.entries(data)) {
      if (!Array.isArray(entries)) continue;
      // gitRoot に対応するセッションを探す（cwd が gitRoot で始まるもの）
      const matchedSession = sessions.find(s => s.cwd && (s.cwd === gitRoot || s.cwd.startsWith(gitRoot)));
      const sessionId  = matchedSession ? matchedSession.id : null;
      const projectKey = matchedSession ? matchedSession.project : null;

      for (const entry of entries) {
        if (!entry.root) continue;
        // 現在の Hub の許可ルート外なら復元しない（ゾンビタブ防止）。
        // localStorage は残すので、対象プロジェクトで Hub を起動し直せば次回自動復活する。
        try {
          const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
          const probeUrl = `/api/files-list?root=${encodeURIComponent(entry.root)}&token=${encodeURIComponent(token)}${sessionQs}`;
          const probeRes = await fetch(probeUrl);
          if (!probeRes.ok) {
            console.info('[FilesTabManager] skip restoring tab (not accessible by current Hub):', entry.root, probeRes.status);
            continue;
          }
          const probeData = await probeRes.json();
          if (!probeData.exists) {
            console.info('[FilesTabManager] skip restoring tab (root not found):', entry.root);
            continue;
          }
        } catch (err) {
          console.warn('[FilesTabManager] probe error for', entry.root, err);
          continue;
        }
        openFilesTab(sessionId, projectKey || gitRoot, entry.root, gitRoot);
      }
    }
  }

  /**
   * 指定ファイルを Files タブで開く（タブが無ければ作る、あればアクティブ化）。
   * バインド完了を待ってからプレビュー読み込みとツリー選択を呼ぶ。
   * `fileAbsPath` は許可ルート配下の絶対パスを想定。
   */
  function openFilesTabAtFile(sessionId, projectKey, filesRoot, gitRoot, fileAbsPath) {
    const id = openFilesTab(sessionId, projectKey, filesRoot, gitRoot);
    if (!id || !fileAbsPath) return id;
    if (gitRoot) lsUpdateOpenedFile(gitRoot, filesRoot, fileAbsPath);
    const tabObj = tabs.find(t => t.id === id);
    if (!tabObj) return id;

    const relPath = computeRelPath(filesRoot, fileAbsPath);
    let attempt = 0;
    const tryActivate = () => {
      const previewPane = tabObj.contentEl.querySelector('[data-files-preview]');
      const treePane = tabObj.contentEl.querySelector('[data-files-tree]');
      if (previewPane && previewPane._filesPreview) {
        previewPane._filesPreview.loadFile(fileAbsPath, relPath);
        if (treePane && treePane._filesTree && treePane._filesTree.selectFile) {
          treePane._filesTree.selectFile(fileAbsPath);
        }
        // ツリー描画完了後にスクロールイントゥ
        const scrollIntoView = (n = 0) => {
          const fileEl = treePane && treePane.querySelector(`.files-tree-item[data-abs-path="${CSS.escape(fileAbsPath)}"]`);
          if (fileEl) {
            try { fileEl.scrollIntoView({ block: 'center' }); } catch (_) {}
          } else if (n < 6) {
            setTimeout(() => scrollIntoView(n + 1), 250);
          }
        };
        setTimeout(scrollIntoView, 100);
        return;
      }
      if (attempt++ < 20) setTimeout(tryActivate, 100);
    };
    tryActivate();
    return id;
  }

  return {
    openFilesTab,
    openFilesTabAtFile,
    closeFilesTab,
    switchToSessionView,
    updateSessionTabLabel,
    restoreFromLocalStorage,
    lsUpdateOpenedFile,
  };
})();

// Hub 起動時に localStorage からタブを復元
FilesTabManager.restoreFromLocalStorage();

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

// ---- v2: Files ビュー本体 ----

/**
 * FilesTreeView — 左ペイン
 * bind(containerEl, { filesRoot, sessionId, gitRoot, onFileSelect })
 * unbind(containerEl)
 */
const FilesTreeView = (function () {

  // ディレクトリの開閉状態を localStorage に保存する。
  // 既定は「全て折りたたみ」。展開済みのキー（filesRoot + "::" + relPath）のみを Set に持つ。
  const EXPANDED_STORAGE_KEY = 'any_ai_cli_files_tree_expanded';
  function loadExpandedSet() {
    try {
      const raw = localStorage.getItem(EXPANDED_STORAGE_KEY);
      if (!raw) return new Set();
      const arr = JSON.parse(raw);
      return new Set(Array.isArray(arr) ? arr : []);
    } catch (_) { return new Set(); }
  }
  function saveExpandedSet(set) {
    try { localStorage.setItem(EXPANDED_STORAGE_KEY, JSON.stringify(Array.from(set))); } catch (_) {}
  }
  const expandedSet = loadExpandedSet();

  /** items[] を { name, relPath, absPath, type:'file'|'dir', children:[] } ツリーに変換 */
  // API (/api/files-list) は { path, rel, name, size, mtime, summary } のフラットなファイル一覧を返す。
  // ディレクトリ要素は含まれないので、ここで rel パスから階層を再構築する。
  // ディレクトリの absPath は、最初の子ファイルの absPath から逆算して埋める（D&D の移動先解決に使用）。
  function buildTree(items) {
    const root = { children: [] };
    const nodes = {};
    // rel（相対パス）でアルファベット昇順ソート
    const sorted = [...items].sort((a, b) => {
      const aRel = a.rel || '';
      const bRel = b.rel || '';
      return aRel.localeCompare(bRel);
    });
    for (const item of sorted) {
      const relPath = item.rel || '';
      const absPath = item.path || '';
      if (!relPath) continue;
      const parts = relPath.replace(/\\/g, '/').split('/');
      // OS のパス区切り文字を absPath から推定（Windows なら \\、それ以外は /）
      const sep = absPath.indexOf('\\') >= 0 ? '\\' : '/';
      const absParts = absPath.split(/[\\/]/);
      let parent = root;
      for (let i = 0; i < parts.length - 1; i++) {
        const key = parts.slice(0, i + 1).join('/');
        if (!nodes[key]) {
          // dir は file より (parts.length - 1 - i) 段浅い。absParts の末尾をその分だけ削れば dir の abs が得られる。
          const dropTail = (parts.length - 1) - i;
          const dirAbsParts = absParts.slice(0, absParts.length - dropTail);
          const dirAbs = dirAbsParts.length > 0 ? dirAbsParts.join(sep) : '';
          const dir = { name: parts[i], relPath: key, absPath: dirAbs, type: 'dir', children: [] };
          nodes[key] = dir;
          parent.children.push(dir);
        }
        parent = nodes[key];
      }
      const node = { name: parts[parts.length - 1], relPath, absPath, type: 'file', children: [] };
      nodes[relPath] = node;
      parent.children.push(node);
    }
    // 各階層でディレクトリ先行・名前順に並べ替え
    function sortChildren(n) {
      n.children.sort((a, b) => {
        if (a.type !== b.type) return a.type === 'dir' ? -1 : 1;
        return a.name.localeCompare(b.name);
      });
      for (const c of n.children) {
        if (c.type === 'dir') sortChildren(c);
      }
    }
    sortChildren(root);
    return root;
  }

  function isPreviewable(name) {
    return /\.(md|txt)$/i.test(name);
  }

  function renderTree(treeRoot, opts) {
    const { onFileSelect, onContextMenu, filterText, filesRoot } = opts;
    const filterLower = (filterText || '').toLowerCase();
    const rootKey = filesRoot || '';

    function nodeMatchesFilter(node) {
      if (!filterLower) return true;
      if (node.name.toLowerCase().includes(filterLower)) return true;
      if (node.type === 'dir' && node.children) {
        return node.children.some(c => nodeMatchesFilter(c));
      }
      return false;
    }

    function makeNode(node, depth) {
      if (filterLower && !nodeMatchesFilter(node)) return null;

      const item = document.createElement('div');
      item.className = 'files-tree-item';
      item.dataset.type = node.type;
      item.dataset.relPath = node.relPath;
      if (node.absPath) item.dataset.absPath = node.absPath;
      // D&D: absPath が解決できているアイテムのみドラッグ可
      if (node.absPath) item.draggable = true;
      item.style.paddingLeft = (depth * 5 + 2) + 'px';

      const label = document.createElement('span');
      label.className = 'files-tree-label';

      if (node.type === 'dir') {
        item.classList.add('files-tree-dir');
        const arrow = document.createElement('span');
        arrow.className = 'files-tree-arrow';
        arrow.textContent = '▼';
        label.appendChild(arrow);
        const nameSpan = document.createElement('span');
        nameSpan.className = 'files-tree-name';
        nameSpan.textContent = node.name;
        label.appendChild(nameSpan);
        item.appendChild(label);

        const childrenEl = document.createElement('div');
        childrenEl.className = 'files-tree-children';
        // 既定は折りたたみ。localStorage に保存された展開状態を尊重する。
        // 検索フィルタ中はヒット確認のため強制展開（保存状態は変更しない）。
        const expandKey = rootKey + '::' + node.relPath;
        const forceExpand = !!filterLower;
        let expanded = forceExpand || expandedSet.has(expandKey);
        arrow.textContent = expanded ? '▼' : '▶';
        childrenEl.style.display = expanded ? '' : 'none';

        const toggle = () => {
          expanded = !expanded;
          arrow.textContent = expanded ? '▼' : '▶';
          childrenEl.style.display = expanded ? '' : 'none';
          if (!forceExpand) {
            if (expanded) expandedSet.add(expandKey); else expandedSet.delete(expandKey);
            saveExpandedSet(expandedSet);
          }
        };

        item.addEventListener('click', (e) => { e.stopPropagation(); toggle(); });
        item.addEventListener('contextmenu', (e) => { e.preventDefault(); e.stopPropagation(); if (onContextMenu) onContextMenu(e, node); });

        const childList = filterLower ? node.children.filter(nodeMatchesFilter) : node.children;
        for (const child of childList) {
          const childEl = makeNode(child, depth + 1);
          if (childEl) childrenEl.appendChild(childEl);
        }
        item.appendChild(childrenEl);
        return item;
      } else {
        // file
        const icon = document.createElement('span');
        icon.className = 'files-tree-file-icon';
        icon.textContent = isPreviewable(node.name) ? '📄' : '📃';
        label.appendChild(icon);
        const nameSpan = document.createElement('span');
        nameSpan.className = 'files-tree-name';
        nameSpan.textContent = node.name;
        label.appendChild(nameSpan);
        item.appendChild(label);

        if (isPreviewable(node.name)) {
          item.classList.add('files-tree-file--previewable');
          item.addEventListener('click', (e) => { e.stopPropagation(); if (onFileSelect) onFileSelect(node); });
        } else {
          item.classList.add('files-tree-file--other');
        }
        item.addEventListener('contextmenu', (e) => { e.preventDefault(); e.stopPropagation(); if (onContextMenu) onContextMenu(e, node); });
        return item;
      }
    }

    const container = document.createElement('div');
    container.className = 'files-tree-root';
    for (const child of treeRoot.children) {
      const el = makeNode(child, 0);
      if (el) container.appendChild(el);
    }
    return container;
  }

  /** containerEl 内に左ペインを構築 */
  function bind(containerEl, opts) {
    const { filesRoot, sessionId, gitRoot, onFileSelect } = opts;
    containerEl.innerHTML = '';
    containerEl.style.removeProperty('align-items');
    containerEl.style.removeProperty('justify-content');

    // ──── ツールバー ────
    const toolbar = document.createElement('div');
    toolbar.className = 'files-tree-toolbar';

    const reloadBtn = document.createElement('button');
    reloadBtn.className = 'files-tree-toolbar-btn';
    reloadBtn.title = t('files_tree_reload_tooltip') || 'Reload';
    reloadBtn.textContent = '🔄';

    const openFolderBtn = document.createElement('button');
    openFolderBtn.className = 'files-tree-toolbar-btn';
    openFolderBtn.title = t('files_tree_open_folder_tooltip') || 'Open folder in OS';
    openFolderBtn.textContent = '⛶';

    const searchBtn = document.createElement('button');
    searchBtn.className = 'files-tree-toolbar-btn';
    searchBtn.title = t('files_tree_search_tooltip') || 'Search';
    searchBtn.textContent = '🔍';

    const closeTabBtn = document.createElement('button');
    closeTabBtn.className = 'files-tree-toolbar-btn';
    closeTabBtn.title = t('files_tree_close_tab_tooltip') || 'Close tab';
    closeTabBtn.textContent = '×';

    toolbar.appendChild(reloadBtn);
    toolbar.appendChild(openFolderBtn);
    toolbar.appendChild(searchBtn);

    // 検索インプット（初期非表示）
    const searchWrap = document.createElement('div');
    searchWrap.className = 'files-tree-search-wrap';
    searchWrap.hidden = true;
    const searchInput = document.createElement('input');
    searchInput.type = 'text';
    searchInput.className = 'files-tree-search-input';
    searchInput.placeholder = t('files_tree_search_placeholder') || 'Filter files…';
    const searchClearBtn = document.createElement('button');
    searchClearBtn.className = 'files-tree-toolbar-btn';
    searchClearBtn.textContent = '×';
    searchWrap.appendChild(searchInput);
    searchWrap.appendChild(searchClearBtn);
    toolbar.appendChild(searchWrap);

    // spacer
    const spacer = document.createElement('span');
    spacer.style.flex = '1';
    toolbar.appendChild(spacer);
    toolbar.appendChild(closeTabBtn);

    containerEl.appendChild(toolbar);

    // ──── ツリー本体 ────
    const treeArea = document.createElement('div');
    treeArea.className = 'files-tree-area';
    containerEl.appendChild(treeArea);

    let currentTree = null;
    let selectedAbsPath = null;
    let filterText = '';
    // D&D: ドラッグ中のソース（{ absPath, type }）と現在のドロップ強調先 element
    let draggingSrc = null;
    let hoverDropEl = null;

    function highlightSelected(absPath) {
      const all = treeArea.querySelectorAll('.files-tree-item');
      all.forEach(el => el.classList.remove('files-tree-item--selected'));
      if (absPath) {
        const found = treeArea.querySelector(`.files-tree-item[data-abs-path="${CSS.escape(absPath)}"]`);
        if (found) found.classList.add('files-tree-item--selected');
      }
    }

    // ──── D&D ヘルパー ────
    function clearDropHighlight() {
      if (!hoverDropEl) return;
      if (hoverDropEl === treeArea) {
        treeArea.classList.remove('files-tree-area--drop-target');
      } else {
        hoverDropEl.classList.remove('files-tree-item--drop-target');
      }
      hoverDropEl = null;
    }
    function setDropHighlight(el) {
      if (hoverDropEl === el) return;
      clearDropHighlight();
      if (el === treeArea) {
        treeArea.classList.add('files-tree-area--drop-target');
      } else if (el) {
        el.classList.add('files-tree-item--drop-target');
      }
      hoverDropEl = el || null;
    }
    // パス比較（OS 由来の \ / の差を吸収しつつ、Windows のドライブレターは大文字小文字無視）
    function normalizePath(p) {
      return (p || '').replace(/[\\/]+$/, '');
    }
    function pathsEqualCI(a, b) {
      const na = normalizePath(a);
      const nb = normalizePath(b);
      return na === nb || na.toLowerCase() === nb.toLowerCase();
    }
    function isUnderPath(child, parent) {
      const nc = normalizePath(child);
      const np = normalizePath(parent);
      if (!nc || !np) return false;
      const sep = nc.indexOf('\\') >= 0 || np.indexOf('\\') >= 0 ? '\\' : '/';
      return nc.toLowerCase().startsWith(np.toLowerCase() + sep);
    }
    function dirnameOf(p) {
      const n = normalizePath(p);
      const idx = Math.max(n.lastIndexOf('\\'), n.lastIndexOf('/'));
      return idx >= 0 ? n.slice(0, idx) : n;
    }
    // 与えられた dragover/drop イベントから「ドロップ先ディレクトリの絶対パス」と該当 element を解決する。
    // 戻り値: { el, dirAbs } or null（無効なドロップ先）
    function resolveDropTarget(e) {
      if (!draggingSrc) return null;
      const dirItem = e.target.closest && e.target.closest('.files-tree-dir');
      if (dirItem) {
        const dirAbs = dirItem.dataset.absPath || '';
        if (!dirAbs) return null;
        // 自身への移動禁止
        if (pathsEqualCI(dirAbs, draggingSrc.absPath)) return null;
        // dir をその子孫に入れる移動禁止
        if (draggingSrc.type === 'dir' && isUnderPath(dirAbs, draggingSrc.absPath)) return null;
        // 同一親ディレクトリへの移動は no-op
        if (pathsEqualCI(dirAbs, dirnameOf(draggingSrc.absPath))) return null;
        return { el: dirItem, dirAbs };
      }
      // ディレクトリ要素外（ツリーのルート領域）に落とした場合は filesRoot へ
      if (!filesRoot) return null;
      if (pathsEqualCI(filesRoot, dirnameOf(draggingSrc.absPath))) return null;
      if (draggingSrc.type === 'dir' && (pathsEqualCI(filesRoot, draggingSrc.absPath) || isUnderPath(filesRoot, draggingSrc.absPath))) return null;
      return { el: treeArea, dirAbs: filesRoot };
    }

    // ──── D&D ハンドラ（イベント委譲） ────
    treeArea.addEventListener('dragstart', (e) => {
      const item = e.target.closest && e.target.closest('.files-tree-item');
      if (!item || !item.dataset.absPath) { return; }
      draggingSrc = { absPath: item.dataset.absPath, type: item.dataset.type || 'file' };
      item.classList.add('files-tree-item--dragging');
      try {
        e.dataTransfer.effectAllowed = 'move';
        e.dataTransfer.setData('text/plain', draggingSrc.absPath);
      } catch (_) {}
    });
    treeArea.addEventListener('dragend', (e) => {
      const item = e.target.closest && e.target.closest('.files-tree-item');
      if (item) item.classList.remove('files-tree-item--dragging');
      draggingSrc = null;
      clearDropHighlight();
    });
    treeArea.addEventListener('dragover', (e) => {
      const target = resolveDropTarget(e);
      if (!target) { clearDropHighlight(); return; }
      e.preventDefault();
      try { e.dataTransfer.dropEffect = 'move'; } catch (_) {}
      setDropHighlight(target.el);
    });
    treeArea.addEventListener('dragleave', (e) => {
      // ツリー全体の外に出たら強調を消す
      if (e.target === treeArea && !treeArea.contains(e.relatedTarget)) {
        clearDropHighlight();
      }
    });
    treeArea.addEventListener('drop', async (e) => {
      const target = resolveDropTarget(e);
      clearDropHighlight();
      if (!target || !draggingSrc) return;
      e.preventDefault();
      const src = draggingSrc.absPath;
      const dstDir = target.dirAbs;
      draggingSrc = null;
      try {
        const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
        const url = `/api/files-move?token=${encodeURIComponent(token)}${sessionQs}`;
        const res = await fetch(url, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ src, dstDir }),
        });
        const data = await res.json().catch(() => ({}));
        if (!res.ok || !data.ok) {
          const msg = (data && data.error) ? data.error : ('HTTP ' + res.status);
          alert((t('files_tree_move_failed') || 'Move failed') + ': ' + msg);
          return;
        }
        // 移動後にプレビュー中のファイルが追従できるよう selectedAbsPath を更新
        if (selectedAbsPath && pathsEqualCI(selectedAbsPath, src) && data.newAbs) {
          selectedAbsPath = data.newAbs;
        }
        await loadTree();
      } catch (err) {
        alert((t('files_tree_move_failed') || 'Move failed') + ': ' + String(err));
      }
    });

    function renderAndMount(tree, filter) {
      const el = renderTree(tree, {
        filterText: filter,
        filesRoot,
        onFileSelect: (node) => {
          selectedAbsPath = node.absPath;
          highlightSelected(selectedAbsPath);
          if (onFileSelect) onFileSelect(node);
        },
        onContextMenu: (e, node) => {
          if (node.absPath) showPathPopup(node.absPath, e.clientX, e.clientY, sessionId || '');
        },
      });
      treeArea.innerHTML = '';
      treeArea.appendChild(el);
      if (selectedAbsPath) highlightSelected(selectedAbsPath);
    }

    async function loadTree() {
      treeArea.innerHTML = `<div class="files-tree-loading">${escapeHtml(t('files_tab_loading') || 'Loading…')}</div>`;
      try {
        const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
        const url = `/api/files-list?root=${encodeURIComponent(filesRoot)}&token=${encodeURIComponent(token)}${sessionQs}`;
        const res = await fetch(url);
        if (!res.ok) { treeArea.innerHTML = `<div class="files-tree-error">${escapeHtml('HTTP ' + res.status)}</div>`; return; }
        const data = await res.json();
        if (!data.exists) { treeArea.innerHTML = `<div class="files-tree-error">${escapeHtml(t('files_tree_not_found') || 'Directory not found')}</div>`; return; }
        currentTree = buildTree(data.items || []);
        renderAndMount(currentTree, filterText);
      } catch (err) {
        treeArea.innerHTML = `<div class="files-tree-error">${escapeHtml(String(err))}</div>`;
      }
    }

    reloadBtn.addEventListener('click', () => loadTree());

    openFolderBtn.addEventListener('click', () => {
      callOpenApi('/api/open-folder', filesRoot);
    });

    searchBtn.addEventListener('click', () => {
      searchWrap.hidden = !searchWrap.hidden;
      if (!searchWrap.hidden) { searchInput.focus(); }
      else { searchInput.value = ''; filterText = ''; if (currentTree) renderAndMount(currentTree, ''); }
    });

    searchInput.addEventListener('input', () => {
      filterText = searchInput.value.trim();
      if (currentTree) renderAndMount(currentTree, filterText);
    });

    searchClearBtn.addEventListener('click', () => {
      searchInput.value = '';
      filterText = '';
      searchWrap.hidden = true;
      if (currentTree) renderAndMount(currentTree, '');
    });

    // タブを閉じる — タブIDは contentEl の data-tab-id から
    closeTabBtn.addEventListener('click', () => {
      const contentEl = containerEl.closest('.files-tab-content');
      if (contentEl && contentEl.dataset.tabId) {
        FilesTabManager.closeFilesTab(contentEl.dataset.tabId);
      }
    });

    // 初回ロード
    loadTree();

    // 外部から "ファイルを選択済み状態にする" 用
    containerEl._filesTree = {
      selectFile: (absPath) => {
        selectedAbsPath = absPath;
        highlightSelected(absPath);
      },
    };
  }

  function unbind(containerEl) {
    containerEl.innerHTML = '';
    delete containerEl._filesTree;
  }

  return { bind, unbind };
})();

/**
 * FilesPreview — 右ペイン
 * bind(containerEl, { sessionId, gitRoot, filesRoot })
 * showFile(containerEl, { absPath, relPath })
 * unbind(containerEl)
 */
const FilesPreview = (function () {

  /** marked で Markdown → HTML 変換し、DOMPurify でサニタイズ */
  function renderMarkdown(content, baseDir, onLinkClick) {
    if (typeof marked === 'undefined') return `<pre>${escapeHtml(content)}</pre>`;

    // marked の renderer をカスタマイズしてリンク処理を差し替え
    const renderer = new marked.Renderer();
    renderer.link = ({ href, title, tokens }) => {
      const text = tokens ? tokens.map(t => t.raw || '').join('') : (href || '');
      const safeHref = escapeHtml(href || '');
      const safeText = escapeHtml(text);
      const safeTitle = title ? ` title="${escapeHtml(title)}"` : '';

      if (/^https?:\/\//i.test(href)) {
        return `<a href="${safeHref}" target="_blank" rel="noopener noreferrer"${safeTitle}>${safeText}</a>`;
      }
      if (/\.(md|txt)$/i.test(href) && !/^https?:\/\//i.test(href) && !href.startsWith('/')) {
        // 相対 md リンク → data 属性で処理
        return `<a href="#" data-files-rel-link="${safeHref}" class="files-md-link"${safeTitle}>${safeText}</a>`;
      }
      // その他（絶対OSパス等）→ data 属性
      return `<a href="#" data-files-path-link="${safeHref}" class="files-md-link"${safeTitle}>${safeText}</a>`;
    };

    // ```lang ... ``` → highlight.js で色付け（hljs 未ロード時はプレーン表示にフォールバック）
    renderer.code = function (codeObj, infoArg) {
      let code = '';
      let lang = '';
      if (codeObj && typeof codeObj === 'object') {
        code = codeObj.text != null ? codeObj.text : '';
        lang = (codeObj.lang || '').trim().split(/\s+/)[0] || '';
      } else {
        code = codeObj || '';
        lang = (infoArg || '').trim().split(/\s+/)[0] || '';
      }
      let body = '';
      let langClass = '';
      if (typeof hljs !== 'undefined') {
        try {
          if (lang && hljs.getLanguage(lang)) {
            body = hljs.highlight(code, { language: lang, ignoreIllegals: true }).value;
            langClass = ' language-' + lang;
          } else if (lang) {
            // 未知の言語名 → エスケープのみ
            body = escapeHtml(code);
            langClass = ' language-' + lang;
          } else {
            // 言語指定なし → 自動判定（短いスニペットでは外しがちなので保険）
            const auto = hljs.highlightAuto(code);
            body = auto.value;
            if (auto.language) langClass = ' language-' + auto.language;
          }
        } catch (_) {
          body = escapeHtml(code);
        }
      } else {
        body = escapeHtml(code);
      }
      return `<pre><code class="hljs${langClass}">${body}</code></pre>`;
    };

    marked.use({
      renderer,
      gfm: true,
      breaks: false,
    });

    let html;
    try { html = marked.parse(content); } catch (_) { return `<pre>${escapeHtml(content)}</pre>`; }
    // DOMPurify でサニタイズ（href/src スキームは markdown プレビューで開く相対リンク/data-* も許容したいので
    // 既定の許可リストで十分。許可外スキームの a[href] は DOMPurify が落とす）
    if (typeof DOMPurify !== 'undefined') {
      html = DOMPurify.sanitize(html, {
        ADD_ATTR: ['target', 'data-files-rel-link', 'data-files-path-link', 'data-files-skip-search'],
      });
    }
    return html;
  }

  /**
   * 各 <pre> ブロックを wrapper で囲み、右上にコピー用ボタンを追加する。
   * 検索ハイライト walker の対象から除外できるよう、ボタンには `data-files-skip-search` を付ける。
   */
  function addCodeCopyButtons(el) {
    el.querySelectorAll('pre').forEach(pre => {
      if (pre.parentElement && pre.parentElement.classList.contains('files-preview-code-wrapper')) return;
      const wrapper = document.createElement('div');
      wrapper.className = 'files-preview-code-wrapper';
      pre.parentNode.insertBefore(wrapper, pre);
      wrapper.appendChild(pre);

      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'files-preview-code-copy-btn';
      btn.dataset.filesSkipSearch = '1';
      const defaultLabel = t('files_preview_code_copy_label') || 'Copy';
      const copiedLabel = t('files_preview_code_copied_label') || 'Copied';
      btn.textContent = defaultLabel;
      btn.title = t('files_preview_code_copy_tooltip') || 'Copy code';
      btn.addEventListener('click', async (e) => {
        e.preventDefault();
        e.stopPropagation();
        const code = pre.querySelector('code');
        const text = (code ? code.innerText : pre.innerText) || '';
        try {
          await navigator.clipboard.writeText(text);
          btn.textContent = copiedLabel;
          btn.classList.add('copied');
          setTimeout(() => {
            btn.textContent = defaultLabel;
            btn.classList.remove('copied');
          }, 1200);
        } catch (_) {
          showToast(t('copied_to_clipboard') || 'Copied', btn);
        }
      });
      wrapper.appendChild(btn);
    });
  }

  /**
   * <table> を wrapper で囲み、右上に「表をコピー」ボタンを追加する。
   * クリップボードには Markdown テーブル形式 (text/plain) と HTML テーブル (text/html) の両方を書き込む。
   */
  function addTableCopyButtons(el) {
    el.querySelectorAll('table').forEach(table => {
      if (table.parentElement && table.parentElement.classList.contains('files-preview-table-wrapper')) return;
      const wrapper = document.createElement('div');
      wrapper.className = 'files-preview-table-wrapper';
      table.parentNode.insertBefore(wrapper, table);
      wrapper.appendChild(table);

      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'files-preview-table-copy-btn';
      btn.dataset.filesSkipSearch = '1';
      const defaultLabel = t('files_preview_table_copy_label') || 'Copy table';
      const copiedLabel = t('files_preview_table_copied_label') || 'Copied';
      btn.textContent = defaultLabel;
      btn.title = t('files_preview_table_copy_tooltip') || 'Copy table';
      btn.addEventListener('click', async (e) => {
        e.preventDefault();
        e.stopPropagation();
        const md = tableToMarkdown(table);
        const htmlText = table.outerHTML;
        try {
          if (window.ClipboardItem && navigator.clipboard.write) {
            const item = new ClipboardItem({
              'text/plain': new Blob([md], { type: 'text/plain' }),
              'text/html': new Blob([htmlText], { type: 'text/html' }),
            });
            await navigator.clipboard.write([item]);
          } else {
            await navigator.clipboard.writeText(md);
          }
          btn.textContent = copiedLabel;
          btn.classList.add('copied');
          setTimeout(() => {
            btn.textContent = defaultLabel;
            btn.classList.remove('copied');
          }, 1200);
        } catch (_) {
          try { await navigator.clipboard.writeText(md); } catch (_) {}
          showToast(t('copied_to_clipboard') || 'Copied', btn);
        }
      });
      wrapper.appendChild(btn);
    });
  }

  /** <table> を GFM Markdown テーブル文字列に変換 */
  function tableToMarkdown(table) {
    const rowsFromSection = (section) => {
      if (!section) return [];
      return Array.from(section.rows).map(tr =>
        Array.from(tr.cells).map(cell => {
          // セル内の改行・パイプを Markdown 互換にエスケープ
          const text = (cell.innerText || '').replace(/\r?\n/g, ' ').replace(/\|/g, '\\|').trim();
          return text;
        })
      );
    };

    const headRows = rowsFromSection(table.tHead);
    const bodyRows = [];
    Array.from(table.tBodies).forEach(tb => bodyRows.push(...rowsFromSection(tb)));

    // tHead がない場合は最初の行をヘッダ扱いにする
    let header = headRows[0];
    let body = bodyRows;
    if (!header && bodyRows.length > 0) {
      header = bodyRows[0];
      body = bodyRows.slice(1);
    }
    if (!header) return '';

    const colCount = header.length;
    const padRow = (row) => {
      const r = row.slice(0, colCount);
      while (r.length < colCount) r.push('');
      return r;
    };

    const lines = [];
    lines.push('| ' + padRow(header).join(' | ') + ' |');
    lines.push('| ' + Array(colCount).fill('---').join(' | ') + ' |');
    body.forEach(row => {
      lines.push('| ' + padRow(row).join(' | ') + ' |');
    });
    return lines.join('\n');
  }

  /** containerEl 内に右ペインを構築 */
  function bind(containerEl, opts) {
    const { sessionId, gitRoot, filesRoot } = opts;
    containerEl.innerHTML = '';
    containerEl.style.removeProperty('align-items');
    containerEl.style.removeProperty('justify-content');
    containerEl.style.removeProperty('color');

    // ──── ツールバー ────
    const toolbar = document.createElement('div');
    toolbar.className = 'files-preview-toolbar';

    const breadcrumb = document.createElement('span');
    breadcrumb.className = 'files-preview-breadcrumb';
    const breadcrumbDir = document.createElement('span');
    breadcrumbDir.className = 'files-preview-breadcrumb-dir';
    const breadcrumbFile = document.createElement('span');
    breadcrumbFile.className = 'files-preview-breadcrumb-file';
    breadcrumb.appendChild(breadcrumbDir);
    breadcrumb.appendChild(breadcrumbFile);
    toolbar.appendChild(breadcrumb);

    const spacer = document.createElement('span');
    spacer.style.flex = '0 0 auto';
    toolbar.appendChild(spacer);

    // ツールバーボタン群（ファイル選択後に有効化）
    const openEditorBtn = document.createElement('button');
    openEditorBtn.className = 'files-preview-toolbar-btn';
    openEditorBtn.title = t('files_preview_open_editor_tooltip') || 'Open in editor';
    openEditorBtn.textContent = '📝';
    openEditorBtn.disabled = true;

    const openFolderBtn = document.createElement('button');
    openFolderBtn.className = 'files-preview-toolbar-btn';
    openFolderBtn.title = t('files_preview_open_folder_tooltip') || 'Open folder';
    openFolderBtn.textContent = '📁';
    openFolderBtn.disabled = true;

    const copyPathBtn = document.createElement('button');
    copyPathBtn.className = 'files-preview-toolbar-btn';
    copyPathBtn.title = t('files_preview_copy_path_tooltip') || 'Copy path';
    copyPathBtn.textContent = '🔗';
    copyPathBtn.disabled = true;

    const searchBtn = document.createElement('button');
    searchBtn.className = 'files-preview-toolbar-btn';
    searchBtn.title = t('files_preview_search_tooltip') || 'Search in page';
    searchBtn.textContent = '🔎';
    searchBtn.disabled = true;

    const reloadBtn = document.createElement('button');
    reloadBtn.className = 'files-preview-toolbar-btn';
    reloadBtn.title = t('files_preview_reload_tooltip') || 'Reload';
    reloadBtn.textContent = '🔄';
    reloadBtn.disabled = true;

    const closeBtn = document.createElement('button');
    closeBtn.className = 'files-preview-toolbar-btn';
    closeBtn.title = t('files_preview_close_tooltip') || 'Close preview';
    closeBtn.textContent = '×';
    closeBtn.disabled = true;

    toolbar.appendChild(openEditorBtn);
    toolbar.appendChild(openFolderBtn);
    toolbar.appendChild(copyPathBtn);
    toolbar.appendChild(searchBtn);
    toolbar.appendChild(reloadBtn);
    toolbar.appendChild(closeBtn);

    containerEl.appendChild(toolbar);

    // ──── ページ内検索バー ────
    const searchBar = document.createElement('div');
    searchBar.className = 'files-preview-search-bar';
    searchBar.hidden = true;
    const searchInput = document.createElement('input');
    searchInput.type = 'text';
    searchInput.className = 'files-preview-search-input';
    searchInput.placeholder = t('files_preview_search_placeholder') || 'Search…';
    const searchPrevBtn = document.createElement('button');
    searchPrevBtn.className = 'files-preview-search-nav-btn';
    searchPrevBtn.textContent = '▲';
    searchPrevBtn.title = t('files_preview_search_prev') || 'Previous';
    const searchNextBtn = document.createElement('button');
    searchNextBtn.className = 'files-preview-search-nav-btn';
    searchNextBtn.textContent = '▼';
    searchNextBtn.title = t('files_preview_search_next') || 'Next';
    const searchCountEl = document.createElement('span');
    searchCountEl.className = 'files-preview-search-count';
    const searchCloseBtn = document.createElement('button');
    searchCloseBtn.className = 'files-preview-search-nav-btn';
    searchCloseBtn.textContent = '×';
    searchCloseBtn.title = t('files_preview_search_close') || 'Close search';
    searchBar.appendChild(searchInput);
    searchBar.appendChild(searchPrevBtn);
    searchBar.appendChild(searchNextBtn);
    searchBar.appendChild(searchCountEl);
    searchBar.appendChild(searchCloseBtn);
    containerEl.appendChild(searchBar);

    // ──── 本文エリア ────
    const bodyEl = document.createElement('div');
    bodyEl.className = 'files-preview-body';
    const contentEl = document.createElement('div');
    contentEl.className = 'files-preview-markdown';
    bodyEl.appendChild(contentEl);
    containerEl.appendChild(bodyEl);

    let currentAbsPath = null;
    let currentRelPath = null;
    let highlightMatches = [];
    let highlightIndex = 0;

    function setBreadcrumb(pathText, fullPath) {
      const text = pathText || (t('files_preview_no_file') || 'No file selected');
      const normalized = String(text).replace(/\\/g, '/');
      const slashIdx = normalized.lastIndexOf('/');
      if (slashIdx >= 0) {
        breadcrumbDir.textContent = normalized.slice(0, slashIdx + 1);
        breadcrumbFile.textContent = normalized.slice(slashIdx + 1);
      } else {
        breadcrumbDir.textContent = '';
        breadcrumbFile.textContent = normalized;
      }
      breadcrumb.title = fullPath || text;
    }

    setBreadcrumb('');

    // ──── ページ内検索 ────
    function clearHighlights() {
      contentEl.querySelectorAll('.files-search-highlight').forEach(el => {
        el.replaceWith(el.firstChild || document.createTextNode(''));
      });
      contentEl.normalize();
      highlightMatches = [];
      highlightIndex = 0;
      searchCountEl.textContent = '';
    }

    function applySearch(query) {
      clearHighlights();
      if (!query) return;
      const walker = document.createTreeWalker(contentEl, NodeFilter.SHOW_TEXT, {
        acceptNode(n) {
          // コピーボタンの中の文字列は検索対象から除外
          if (n.parentNode && n.parentNode.closest && n.parentNode.closest('[data-files-skip-search]')) {
            return NodeFilter.FILTER_REJECT;
          }
          return NodeFilter.FILTER_ACCEPT;
        },
      });
      const textNodes = [];
      let node;
      while ((node = walker.nextNode())) textNodes.push(node);
      const lq = query.toLowerCase();
      for (const tn of textNodes) {
        const val = tn.nodeValue;
        const idx = val.toLowerCase().indexOf(lq);
        if (idx === -1) continue;
        const before = document.createTextNode(val.slice(0, idx));
        const mark = document.createElement('mark');
        mark.className = 'files-search-highlight';
        mark.textContent = val.slice(idx, idx + query.length);
        const after = document.createTextNode(val.slice(idx + query.length));
        const parent = tn.parentNode;
        parent.insertBefore(before, tn);
        parent.insertBefore(mark, tn);
        parent.insertBefore(after, tn);
        parent.removeChild(tn);
        highlightMatches.push(mark);
      }
      searchCountEl.textContent = highlightMatches.length > 0
        ? `${highlightIndex + 1}/${highlightMatches.length}`
        : t('files_search_no_results') || '0 results';
      if (highlightMatches.length > 0) scrollToMatch(0);
    }

    function scrollToMatch(idx) {
      highlightMatches.forEach((el, i) => el.classList.toggle('files-search-highlight--active', i === idx));
      if (highlightMatches[idx]) highlightMatches[idx].scrollIntoView({ block: 'center' });
      searchCountEl.textContent = `${idx + 1}/${highlightMatches.length}`;
    }

    searchInput.addEventListener('input', () => applySearch(searchInput.value));
    searchPrevBtn.addEventListener('click', () => {
      if (!highlightMatches.length) return;
      highlightIndex = (highlightIndex - 1 + highlightMatches.length) % highlightMatches.length;
      scrollToMatch(highlightIndex);
    });
    searchNextBtn.addEventListener('click', () => {
      if (!highlightMatches.length) return;
      highlightIndex = (highlightIndex + 1) % highlightMatches.length;
      scrollToMatch(highlightIndex);
    });
    searchInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        if (e.shiftKey) searchPrevBtn.click();
        else searchNextBtn.click();
      } else if (e.key === 'Escape') {
        searchCloseBtn.click();
      }
    });
    searchCloseBtn.addEventListener('click', () => {
      clearHighlights();
      searchInput.value = '';
      searchBar.hidden = true;
      searchBtn.classList.remove('active');
    });

    searchBtn.addEventListener('click', () => {
      if (!currentAbsPath) return;
      const show = searchBar.hidden;
      searchBar.hidden = !show;
      if (show) { searchInput.focus(); }
      else { clearHighlights(); searchInput.value = ''; }
    });

    // ──── ファイルロード ────
    async function loadFile(absPath, relPath) {
      contentEl.innerHTML = `<div class="files-preview-loading">${escapeHtml(t('files_tab_loading') || 'Loading…')}</div>`;
      clearHighlights();
      searchBar.hidden = true;
      searchInput.value = '';

      try {
        const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
        const url = `/api/files-content?path=${encodeURIComponent(absPath)}&token=${encodeURIComponent(token)}${sessionQs}`;
        const res = await fetch(url);
        if (!res.ok) {
          contentEl.innerHTML = `<div class="files-preview-error">${escapeHtml('HTTP ' + res.status)}</div>`;
          return;
        }
        const data = await res.json();
        const content = data.content || '';
        const isMd = /\.md$/i.test(absPath);

        if (data.truncated) {
          const warn = document.createElement('div');
          warn.className = 'files-preview-truncated-warn';
          warn.textContent = t('files_preview_truncated_warn') || '⚠ File truncated (>1MiB). Showing partial content.';
          contentEl.innerHTML = '';
          contentEl.appendChild(warn);
          const pre = document.createElement('pre');
          pre.className = 'files-preview-raw';
          pre.textContent = content;
          contentEl.appendChild(pre);
          addCodeCopyButtons(contentEl);
        } else if (isMd) {
          const html = renderMarkdown(content, absPath, null);
          contentEl.innerHTML = `<div class="files-preview-md-body">${html}</div>`;
          addCodeCopyButtons(contentEl);
          addTableCopyButtons(contentEl);

          // 相対リンク処理
          contentEl.querySelectorAll('a[data-files-rel-link]').forEach(a => {
            a.addEventListener('click', (e) => {
              e.preventDefault();
              const rel = a.dataset.filesRelLink;
              if (!rel) return;
              // 現在のファイルのディレクトリから絶対パスを計算
              const dir = currentAbsPath ? currentAbsPath.replace(/[/\\][^/\\]*$/, '') : filesRoot;
              const sep = dir.includes('\\') ? '\\' : '/';
              const target = dir + sep + rel.replace(/\//g, sep);
              containerEl._filesPreview && containerEl._filesPreview.loadFile(target, rel);
            });
          });
          contentEl.querySelectorAll('a[data-files-path-link]').forEach(a => {
            a.addEventListener('click', (e) => {
              e.preventDefault();
              const p = a.dataset.filesPathLink;
              if (p) showPathPopup(p, e.clientX, e.clientY, sessionId || '');
            });
          });
        } else {
          // .txt など
          const pre = document.createElement('pre');
          pre.className = 'files-preview-raw';
          pre.textContent = content;
          contentEl.innerHTML = '';
          contentEl.appendChild(pre);
          addCodeCopyButtons(contentEl);
        }
        bodyEl.scrollTop = 0;
      } catch (err) {
        contentEl.innerHTML = `<div class="files-preview-error">${escapeHtml(String(err))}</div>`;
      }
    }

    // ──── ボタン Wire-up ────
    openEditorBtn.addEventListener('click', () => {
      if (currentAbsPath) callOpenApi('/api/open-file', currentAbsPath);
    });
    openFolderBtn.addEventListener('click', () => {
      if (currentAbsPath) callOpenApi('/api/open-folder', currentAbsPath);
    });
    copyPathBtn.addEventListener('click', (e) => {
      if (currentAbsPath) copyPathText(currentAbsPath, e.currentTarget).catch(() => {});
    });
    reloadBtn.addEventListener('click', () => {
      if (currentAbsPath) loadFile(currentAbsPath, currentRelPath);
    });
    closeBtn.addEventListener('click', () => {
      currentAbsPath = null;
      currentRelPath = null;
      setBreadcrumb('');
      contentEl.innerHTML = '';
      clearHighlights();
      searchBar.hidden = true;
      [openEditorBtn, openFolderBtn, copyPathBtn, searchBtn, reloadBtn, closeBtn].forEach(b => { b.disabled = true; });
    });

    // 外部 API
    containerEl._filesPreview = {
      loadFile: (absPath, relPath) => {
        currentAbsPath = absPath;
        currentRelPath = relPath;
        const dispPath = relPath || absPath;
        setBreadcrumb(dispPath, absPath);
        [openEditorBtn, openFolderBtn, copyPathBtn, searchBtn, reloadBtn, closeBtn].forEach(b => { b.disabled = false; });
        loadFile(absPath, relPath);
      },
    };
  }

  function unbind(containerEl) {
    containerEl.innerHTML = '';
    delete containerEl._filesPreview;
  }

  return { bind, unbind };
})();

/**
 * FilesViewMounter — タブ生成後に FilesTreeView + FilesPreview を該当 DOM に bind する
 *
 * FilesTabManager が openFilesTab() でスケルトンを作り contentEl を filesContents に追加しているので、
 * MutationObserver で検知してすぐ bind する。
 */
(function () {
  const filesContents = document.getElementById('files-tab-contents');
  if (!filesContents) return;

  function mountTab(contentEl) {
    if (contentEl._filesMounted) return;
    contentEl._filesMounted = true;

    const filesRoot  = contentEl.dataset.filesRoot  || '';
    const sessionId  = contentEl.dataset.sessionId  || '';
    const gitRoot    = contentEl.dataset.gitRoot    || '';
    const projectKey = contentEl.dataset.projectKey || '';
    const tabId      = contentEl.dataset.tabId      || '';

    const treePaneEl    = contentEl.querySelector('[data-files-tree]');
    const previewPaneEl = contentEl.querySelector('[data-files-preview]');
    if (!treePaneEl || !previewPaneEl) return;

    // ペインのローディングテキストを消してスタイルリセット
    treePaneEl.textContent    = '';
    previewPaneEl.textContent = '';

    // ─── FilesPreview を先に bind（onFileSelect から参照するため）───
    FilesPreview.bind(previewPaneEl, { sessionId, gitRoot, filesRoot });

    // ─── FilesTreeView を bind ───
    FilesTreeView.bind(treePaneEl, {
      filesRoot,
      sessionId,
      gitRoot,
      onFileSelect: (node) => {
        if (!previewPaneEl._filesPreview) return;
        const relPath = node.relPath;
        previewPaneEl._filesPreview.loadFile(node.absPath, relPath);
        // localStorage 更新
        FilesTabManager.lsUpdateOpenedFile(gitRoot, filesRoot, node.absPath);
      },
    });

    // ─── localStorage 復元: 最後に開いていたファイルを自動選択 ───
    try {
      const lsKey = 'any-ai-cli.files.tabs';
      const data = JSON.parse(localStorage.getItem(lsKey) || '{}');
      const entries = data[gitRoot] || [];
      const entry = entries.find(e => e.root === filesRoot);
      if (entry && entry.openedFile) {
        const openedFile = entry.openedFile;
        // ツリーが API 取得後に描画されるまで少し待つ
        const trySelect = () => {
          const fileEl = treePaneEl.querySelector(`.files-tree-item[data-abs-path="${CSS.escape(openedFile)}"]`);
          if (fileEl) {
            fileEl.click();
          } else {
            // ツリーがまだ描画されていない → 少し後に再試行（最大3回）
            if (!trySelect._count) trySelect._count = 0;
            if (trySelect._count++ < 6) setTimeout(trySelect, 300);
          }
        };
        setTimeout(trySelect, 400);
      }
    } catch (_) {}
  }

  // 既存 contentEl のマウント（restoreFromLocalStorage で追加済みのものを処理）
  filesContents.querySelectorAll('.files-tab-content').forEach(mountTab);

  // 新規追加を検知
  const observer = new MutationObserver(mutations => {
    for (const m of mutations) {
      m.addedNodes.forEach(node => {
        if (node.nodeType === 1 && node.classList.contains('files-tab-content')) {
          mountTab(node);
        }
      });
    }
  });
  observer.observe(filesContents, { childList: true });
})();
