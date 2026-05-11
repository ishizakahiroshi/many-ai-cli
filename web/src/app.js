console.log('[ai-cli-hub] app.js build=2026-05-11-plain-yes-no-approval');

// ---- トースト通知 ----
let _toastTimer = null;
function showToast(msg, anchor) {
  let el = document.getElementById('toast');
  if (!el) {
    el = document.createElement('div');
    el.id = 'toast';
    document.body.appendChild(el);
  }
  el.textContent = msg;
  if (anchor) {
    const r = anchor.getBoundingClientRect();
    el.style.top = (r.top + r.height / 2) + 'px';
    el.style.bottom = 'auto';
    el.classList.add('toast--anchored');
    // 右側に置けるか確認し、はみ出す場合は anchor の左側に表示
    el.style.left = (r.right + 6) + 'px';
    const margin = 8;
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
const CWD_HISTORY_MAX               = 10;

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
  const t = (theme === 'dark' || theme === 'blue' || theme === 'light') ? theme : 'dark';
  document.documentElement.setAttribute('data-theme', t);
  const sel = document.getElementById('theme-select');
  if (sel) sel.value = t;
  try { localStorage.setItem(STORAGE_THEME_KEY, t); } catch (_) {}
}

function applyFontSize(size) {
  const s = FONTSIZE_MAP[size] ? size : 'medium';
  const px = FONTSIZE_MAP[s];
  terminals.forEach(({ term, fitAddon }, id) => {
    term.options.fontSize = px;
    requestAnimationFrame(() => {
      fitAddon.fit();
      sendResize(id, term.cols, term.rows);
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

  const panel      = document.getElementById('settings-panel');
  const btn        = document.getElementById('settings-btn');
  const themeEl    = document.getElementById('theme-select');
  const fontsizeEl = document.getElementById('fontsize-select');
  const langEl     = document.getElementById('lang-select');
  const resetBtn   = document.getElementById('settings-reset-btn');
  const saveBtn    = document.getElementById('settings-save-btn');
  const closeBtn   = document.getElementById('settings-close-btn');
  const licensesBtn = document.getElementById('settings-licenses-btn');

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
      'https://github.com/ishizakahiroshi/ai-cli-hub/blob/main/' + file,
      '_blank', 'noopener,noreferrer'
    );
  });
  licensesBtn.addEventListener('click', () => {
    panel.hidden = true;
    aboutPanel.hidden = false;
  });
  aboutCloseBtn.addEventListener('click', () => { aboutPanel.hidden = true; });
  aboutPanel.addEventListener('click', (e) => {
    if (e.target === aboutPanel) aboutPanel.hidden = true;
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
  const cols = area ? Math.floor(area.clientWidth / 7.5) || 200 : 200;
  const rows = area ? Math.floor(area.clientHeight / 16) || 50 : 50;
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
    document.getElementById('summary').textContent = t('hub_stopped') || 'Hub停止中 — ai-cli-hub serve で再起動してください';
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
      writePTYChunk(id, t.term, bytes, isActive ? () => { if (t.autoScroll) t.term.scrollToBottom(); } : undefined);
    } else {
      t.pendingChunks.push(bytes);
    }
    trackApprovalHintFromChunk(id, bytes);
    if (isActive) scheduleApprovalCheck(id);
    return;
  }

  if (m.type === 'snapshot') {
    const arr = typeof m.sessions === 'string' ? JSON.parse(m.sessions) : m.sessions;
    (arr || []).forEach(s => { sessions.set(s.id, s); addToSessionOrder(s.id); });
    document.getElementById('summary').textContent = t('connected') || '接続済み';
    renderSessionList();
    checkApprovalOnStartup();
    if (!_elapsedTimerInterval) {
      _elapsedTimerInterval = setInterval(() => renderSessionList(), 1000);
    }
  } else if (m.type === 'session_update') {
    const isNew = !sessions.has(m.session_id);
    const cur = sessions.get(m.session_id) || { id: m.session_id };
    if (m.provider)        cur.provider        = m.provider;
    if (m.display_name)    cur.display_name    = m.display_name;
    if (m.cwd)             cur.cwd             = m.cwd;
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
    const s = sessions.get(m.session_id);
    if (s) s.state = m.state || 'disconnected';
    cancelApprovalHintConfirm(m.session_id);
    approvalVisibleCache.delete(m.session_id);
    if (multiQuestionVisibleCache.delete(m.session_id) && m.session_id === activeSessionId) {
      setMultiQuestionBannerVisible(false);
    }
    removeApprovalAutoSwitchTarget(m.session_id);
    maybeAutoSwitchToNextApproval();
    const deadStates = ['completed', 'error', 'disconnected'];
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
  if (typeof Unicode11Addon !== 'undefined') {
    const u11 = new Unicode11Addon.Unicode11Addon();
    term.loadAddon(u11);
    term.unicode.activeVersion = '11';
  }
  terminals.set(id, { term, fitAddon, container: null, pendingChunks: [], pendingTextTail: '', markerFilterCarry: new Uint8Array(0), autoScroll: true, everAttached: false });
}

function attachTerminal(id) {
  const area = document.getElementById('terminal-area');
  if (!area) return;
  const t = terminals.get(id);
  if (!t) return;
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
      t.fitAddon.fit();
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
    t.fitAddon.fit();
    t.term.onScroll(() => {
      const buf = t.term.buffer.active;
      const atBottom = buf.viewportY + t.term.rows >= buf.length;
      if (atBottom) {
        t.autoScroll = true;
        if (id === activeSessionId) updateScrollLockBtn(false);
      } else if (id === activeSessionId) {
        updateScrollLockBtn(!t.autoScroll);
      }
    });
    container.addEventListener('wheel', (e) => {
      if (e.deltaY < 0) {
        t.autoScroll = false;
        if (id === activeSessionId) updateScrollLockBtn(true);
      }
    }, { passive: true });
    flushPending(id);
    t.everAttached = true;
    sendResize(id, t.term.cols, t.term.rows);
    container.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      const sel = t.term.getSelection();
      if (sel) copyCleanText(sel, container).catch(() => {});
    });
  } else {
    requestAnimationFrame(() => whenLayoutReady(id, container));
  }
}

function flushPending(id) {
  const t = terminals.get(id);
  if (!t) return;
  for (const bytes of t.pendingChunks) {
    writePTYChunk(id, t.term, bytes);
  }
  t.pendingChunks = [];
  t.term.scrollToBottom();
  scheduleApprovalCheck(id);
}

function updateScrollLockBtn(locked) {
  const topBtn = document.getElementById('scroll-to-top-btn');
  const bottomBtn = document.getElementById('scroll-to-bottom-btn');
  const t = activeSessionId === null ? null : terminals.get(activeSessionId);
  if (!t) {
    if (topBtn) topBtn.hidden = true;
    if (bottomBtn) bottomBtn.hidden = true;
    return;
  }
  const buf = t.term.buffer.active;
  const atTop = buf.viewportY <= 0;
  const atBottom = buf.viewportY + t.term.rows >= buf.length;
  if (topBtn) topBtn.hidden = atTop;
  if (bottomBtn) bottomBtn.hidden = !locked || atBottom;
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
  t.term.scrollToBottom();
  updateScrollLockBtn(false);
});

const hubMarkerBytePatterns = [
  new TextEncoder().encode('[AI-CLI-HUB]'),
  new TextEncoder().encode('[/AI-CLI-HUB]'),
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

// provider 別の承認 trigger phrase は ~/.ai-cli-hub/approval-patterns/{provider}.json に外出し。
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
  const sig = JSON.stringify(options.map(o => o.num + ':' + o.label));
  cancelApprovalHintConfirm(id);
  approvalHintConfirmTimers.set(id, setTimeout(() => {
    approvalHintConfirmTimers.delete(id);
    const cached = approvalRawOptionsCache.get(id);
    if (!cached || cached.length === 0) return;
    const cachedSig = JSON.stringify(cached.map(o => o.num + ':' + o.label));
    if (cachedSig !== sig) return;
    if (approvalVisibleCache.get(id)) return;
    approvalVisibleCache.set(id, true);
    enqueueApprovalAutoSwitch(id);
    playNotificationSound();
    ws.send(JSON.stringify({ type: 'session_hint', session_id: id, approval_visible: true }));
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
  t.pendingTextTail = (t.pendingTextTail + text).slice(-3000);

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

  // フォーマットベース検出（優先）: [AI-CLI-HUB] マーカーがあれば即確定
  const markerOpts = extractHubMarkerApproval(lines);
  if (markerOpts) {
    // doSend でテキスト送信済みの承認が Ink 再描画で再検出された場合はスキップ
    const consumed = approvalConsumedSig.get(id);
    const sig = JSON.stringify(markerOpts.map(o => o.num + ':' + o.label));
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
    const sig = JSON.stringify(plainYesNoOpts.map(o => o.num + ':' + o.label));
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
    // option 1 が pendingTextTail から欠落している場合（3000 字制限）に補完
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
    ((hasApprovalLikeLabel && (hasUserSpecifies || contextLines.some((line) => matchProviderApprovalTrigger(provider, line)))) || isHubChoice);
  const hasChoiceMenuHint = hasCursorOption && options.length > 0 && contextLines.some((line) => matchProviderApprovalTrigger(provider, line));
  const nowVisible = (options.length > 0 && approvalNear) || hasChoiceMenuHint;

  // doSend / sendChoice で消費済みの選択肢が xterm scanBuffer に残っているため
  // フォールバック検出で再抽出されるケースを抑止する（marker 検出と同じ debounce 戦略）。
  if (nowVisible) {
    const consumed = approvalConsumedSig.get(id);
    const sig = JSON.stringify(options.map(o => o.num + ':' + o.label));
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

// extractHubMarkerApproval は [AI-CLI-HUB]...[/AI-CLI-HUB] マーカーベースの承認を検出する。
// 検出した場合は options 配列を返し、検出できなければ null を返す。
function extractHubMarkerApproval(lines) {
  const yesNoRe = /\(Y:1\/N:0\)\s*$/;
  const searchStart = Math.max(0, lines.length - 40);
  let closeIdx = -1;
  let openIdx = -1;

  for (let i = lines.length - 1; i >= searchStart; i--) {
    const line = lines[i];
    // Single-line: [AI-CLI-HUB] content [/AI-CLI-HUB]
    if (/\[AI-CLI-HUB\]/.test(line) && /\[\/AI-CLI-HUB\]/.test(line)) {
      const inner = line.replace(/\[\/AI-CLI-HUB\]/g, '').replace(/\[AI-CLI-HUB\]/g, '').trim();
      return _parseHubBlock([inner], yesNoRe);
    }
    if (/\[\/AI-CLI-HUB\]/.test(line) && closeIdx === -1) { closeIdx = i; continue; }
    if (/\[AI-CLI-HUB\]/.test(line) && closeIdx !== -1) { openIdx = i; break; }
  }

  if (openIdx === -1 || closeIdx === -1) return null;
  const inner = lines.slice(openIdx + 1, closeIdx).map(l => l.trim()).filter(Boolean);
  return _parseHubBlock(inner, yesNoRe);
}

// AGENTS.md の確認フォーマットに従った素の Yes/No 質問を検出する。
// [AI-CLI-HUB] マーカーが無い場合でも `(Y:1/N:0)` が末尾にあれば Hub ボタン化する。
function extractPlainYesNoApproval(lines) {
  const yesNoRe = /\(Y:1\/N:0\)\s*$/;
  const searchStart = Math.max(0, lines.length - 20);
  for (let i = lines.length - 1; i >= searchStart; i--) {
    const line = String(lines[i] || '').trim();
    if (!line) continue;
    if (/\[AI-CLI-HUB\]|\[\/AI-CLI-HUB\]/.test(line)) return null;
    if (yesNoRe.test(line)) return _yesNoApprovalOptions();
  }
  return null;
}

function _parseHubBlock(lines, yesNoRe) {
  const text = lines.join('\n');
  if (yesNoRe.test(text)) {
    return _yesNoApprovalOptions();
  }
  const opts = lines
    .map(l => l.match(/^\s*(\d+)\.\s*(.+?)\s*$/))
    .filter(Boolean)
    .map(m => ({ num: parseInt(m[1], 10), label: m[2].trim(), isCurrent: false }));
  return opts.length > 0 ? opts : null;
}

function _yesNoApprovalOptions() {
  return [
    { num: 1, label: 'Yes (1)', isCurrent: true, preserveOrder: true },
    { num: 0, label: 'No (0)', isCurrent: false, preserveOrder: true },
  ];
}

function approvalContextLines(lines, cluster, margin = 5) {
  if (!cluster) return lines;
  return lines.slice(Math.max(0, cluster.start - margin), Math.min(lines.length, cluster.end + margin + 1));
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
  const lines = scanBuffer(id);
  const t = terminals.get(id);
  const crunchLimit = Math.max(50, (t?.term?.rows || 40) + 20);
  for (let i = lines.length - 1; i >= Math.max(0, lines.length - crunchLimit); i--) {
    // Claude Code の折りたたみパターン: "… +23 lines (ctrl+o to expand)"
    const m = lines[i].match(/[…\.]{1,3}\s*\+(\d+)\s*lines?\s*\(ctrl\+o to expand\)/i);
    if (m) return { found: true, count: parseInt(m[1]) };
  }
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

  // [AI-CLI-HUB] マーカー検出: xterm バッファではなく pendingTextTail を使う。
  // xterm バッファは回答済みの古い [AI-CLI-HUB] ブロックを保持し続けるため、
  // suppress 期間が切れると再検出・再表示されてしまう。
  // pendingTextTail は hideActionBar でクリアされるが、Ink 再描画で同一内容が
  // 再び入ることがあるため approvalConsumedSig で二重表示を防ぐ。
  const t = terminals.get(id);
  if (t) {
    const pendingLines = (t.pendingTextTail || '').split(/\r\n|\r|\n/).slice(-40).map(l => stripAnsi(l));
    const markerOpts = extractHubMarkerApproval(pendingLines);
    if (markerOpts) {
      const consumed = approvalConsumedSig.get(id);
      const sig = JSON.stringify(markerOpts.map(o => o.num + ':' + o.label));
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
      const sig = JSON.stringify(plainYesNoOpts.map(o => o.num + ':' + o.label));
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

  // pendingTextTail は 3000 字制限のため option 1 が欠落することがある。
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
    (hasUserSpecifies || contextLines.some((line) => matchProviderApprovalTrigger(provider, line)))) || isHubChoice;
  const hasApproval = options.length > 0 && approvalNear && hasCursorOption;
  const hasChoiceMenu = hasCursorOption && options.length > 0 && contextLines.some((line) => matchProviderApprovalTrigger(provider, line));
  const hasPrompt = hasApproval || hasChoiceMenu;

  // 折りたたみ（クランチ）を検出
  const crunch = detectCrunch(id);

  if (!hasPrompt && !crunch.found) {
    // 承認プロンプトが検出できない場合は確実に閉じる。
    // cached options の再表示フォールバックは、解決済み承認の残留を引き起こすため使わない。
    hideActionBar(id);
    return;
  }

  // doSend / sendChoice で消費済みの選択肢が xterm scanBuffer に残っているため
  // フォールバック検出で再抽出されるケースを抑止する（marker 検出と同じ debounce 戦略）。
  if (hasPrompt) {
    const consumed = approvalConsumedSig.get(id);
    const sig = JSON.stringify(options.map(o => o.num + ':' + o.label));
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
  actionBarFocusIdx = -1;
  if (id !== undefined) {
    cancelApprovalHintConfirm(id);
    if (approvalVisibleCache.get(id)) {
      approvalVisibleCache.set(id, false);
      removeApprovalAutoSwitchTarget(id);
      ws.send(JSON.stringify({ type: 'session_hint', session_id: id, approval_visible: false }));
    }
    // approvalVisibleCache の状態に関わらず常にクリア（race 条件で cache が false でも tail を残さない）
    approvalRawOptionsCache.delete(id);
    const t = terminals.get(id);
    if (t) t.pendingTextTail = '';
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
  bar.innerHTML = '';

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
}

function sendChoice(sessionId, targetNum) {
  // 矢印移動ではなく番号直接入力で確定する（誤選択防止）
  sendText(sessionId, `${targetNum}\r`);
  // doSend と同様に消費済み署名を記録（Ink 再描画による同一ブロックの再検出・再表示を防ぐ）
  const prevOpts = approvalRawOptionsCache.get(sessionId);
  if (prevOpts) approvalConsumedSig.set(sessionId, JSON.stringify(prevOpts.map(o => o.num + ':' + o.label)));
  hideActionBar(sessionId);
  // PTY エコーバックによる誤再表示を 2 秒間抑制
  approvalSuppressUntil.set(sessionId, Date.now() + 2000);
  // suppress 解除後に pendingTextTail を再スキャン（suppress 中に届いた [AI-CLI-HUB] ブロックを検出するため）
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
    pre.textContent = out.lines.join('\n');
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
  inputEl.style.height = 'auto';
  inputEl.style.height = Math.min(inputEl.scrollHeight, Math.floor(window.innerHeight * 0.3)) + 'px';
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
  const hadAttach = pendingAttachFiles.length > 0;
  await flushPendingAttach(sessionId);
  if (hadAttach) {
    // claude の picker が attach_file の \r を確定処理するまで待ってから text\r を送る。
    // ここで待たないと picker 確定前に text\r が届き、\r が「送信」扱いになって
    // 画像とテキストが別投稿に分裂する。
    await new Promise(r => setTimeout(r, 80));
  }
  let rawText = buildSendText();
  // トリガーフレーズを末尾から除去（PTY・AI には送らない）
  const _tp = getActiveTriggerPhrase();
  if (_tp && textEndsWithTriggerPhrase(rawText, _tp)) {
    rawText = stripTrailingTriggerPhrase(rawText, _tp);
  }
  // 改行を含む場合はブラケットペーストモードでラップ（\n が途中 Enter と解釈されるのを防ぐ）
  const textToSend = rawText.includes('\n')
    ? '\x1b[200~' + rawText + '\x1b[201~\r'
    : rawText + '\r';
  clearInput();
  hideSlashMenu();
  // テキスト送信で承認ポップアップをバイパスした場合、Ink 再描画による
  // 同一選択肢の再検出・再表示を防ぐため消費済み署名を保存する
  const prevOpts = approvalRawOptionsCache.get(sessionId);
  if (prevOpts) approvalConsumedSig.set(sessionId, JSON.stringify(prevOpts.map(o => o.num + ':' + o.label)));
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
  sendSubmittedText(sessionId, `${cmd}\r`);
}

function sendSubmittedText(sessionId, text) {
  sendText(sessionId, text);
}

function sendText(sessionId, text) {
  ws.send(JSON.stringify({ type: 'pty_input', session_id: sessionId, text }));
}

function dismissSession(id) {
  if (!sessions.has(id)) return;
  ws.send(JSON.stringify({ type: 'session_dismiss', session_id: id }));
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

let lastDevicePixelRatio = window.devicePixelRatio || 1;

function refitAllTerminals(refreshRows = false) {
  terminals.forEach(({ term, fitAddon }, id) => {
    const prevCols = term.cols;
    const prevRows = term.rows;
    fitAddon.fit();
    if (refreshRows && term.rows > 0) {
      term.refresh(0, term.rows - 1);
    }
    if (term.cols !== prevCols || term.rows !== prevRows) {
      sendResize(id, term.cols, term.rows);
    }
  });
}

const resizeObserver = new ResizeObserver(() => {
  if (activeSessionId === null) return;
  const t = terminals.get(activeSessionId);
  if (!t || !t.container) return;
  const prevCols = t.term.cols;
  const prevRows = t.term.rows;
  t.fitAddon.fit();
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
  restoreInputStateFor(id);
  ensureTerminal(id);
  attachTerminal(id);
  setMultiQuestionBannerVisible(!!multiQuestionVisibleCache.get(id));
  detectApproval(id);
  renderSessionList();
  renderToolOutputs(id);
  updateShellBadge(id);
  inputEl.focus();
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

let _sessionListClickDelegated = false;
let _sessionCardPointerDown = null;

function renderSessionList() {
  const root = document.getElementById('sessions');
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
  }
  const sidebarEl = document.getElementById('session-list');
  const isSidebarCollapsed = !!sidebarEl && sidebarEl.getBoundingClientRect().width <= SIDEBAR_COLLAPSED_WIDTH_THRESHOLD;
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

    const runningCount  = groupSessions.filter(s => s.state === 'running').length;
    const waitingCount  = groupSessions.filter(s => s.state === 'waiting').length;
    const standbyCount  = groupSessions.filter(s => (s.state || 'standby') === 'standby').length;
    const chipsEl = document.createElement('span');
    chipsEl.className = 'group-status-chips';
    const waitingBlinkClass = waitingCount > 0 ? ' status-chip--blink' : '';
    chipsEl.innerHTML =
      `<span class="status-chip status-chip--running">${runningCount}</span>` +
      `<span class="status-chip status-chip--waiting${waitingBlinkClass}">${waitingCount}</span>` +
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
      c.className = 'card' + (state === 'waiting' ? ' waiting' : '') + (s.id === activeSessionId ? ' active' : '');
      c.tabIndex = isCollapsed ? -1 : 0;
      const label = stateLabel(state);
      const lastOut = formatLastOutputAt(s.last_output_at);
      const filteredMsg = filterFirstMessage(s.last_message || s.first_message || '');
      const cwdStr = s.cwd || '';
      const lastOutHtml = lastOut
        ? `<span class="card-last-output">${isSidebarCollapsed ? 'L:' : t('last_output')}${lastOut}</span>`
        : '';
      const startedAtHtml = s.started_at ? `<span class="card-started-at">${formatStartedAt(s.started_at)}</span>` : '';
      const sessionLabel = s.label ? `<span class="card-label">[${escapeHtml(s.label)}]</span>` : '';
      const msgHtml = filteredMsg
        ? `<span class="card-msg" data-tooltip="${escapeHtml(filteredMsg)}">${escapeHtml(filteredMsg)}</span>`
        : `<span class="card-msg"></span>`;
      const metaRow = `<div class="card-meta-row"><span class="badge ${state}">${label}</span>${sessionLabel}${msgHtml}</div>`;
      c.dataset.sessionId = s.id;
      const modelBadge = s.model ? ` <span class="card-model">${escapeHtml(s.model)}</span>` : '';
      c.innerHTML =
        `<div class="card-title-row"><b>#${s.id}</b> ${providerIconHtml(s.provider)} <span class="card-display-name">${escapeHtml(s.display_name || s.provider || '')}</span>${modelBadge}${startedAtHtml}${lastOutHtml}</div>` +
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
    document.title = _titleBlinkState ? `(${_titleBlinkCount}) AI-CLI-HUB` : 'AI-CLI-HUB';
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
    document.title = 'AI-CLI-HUB';
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
    .map(([p, n]) => `<span class="summary-provider-chip">${providerIconHtml(p)}<span>${PROVIDER_LABELS[p] || p}:${n}</span></span>`)
    .join('');

  const summaryWaitingBlinkClass = stateCounts.waiting > 0 ? ' status-chip--blink' : '';
  let summary =
    `<span class="status-chip status-chip--running">${stateCounts.running}</span>` +
    `<span class="status-chip status-chip--waiting${summaryWaitingBlinkClass}">${stateCounts.waiting}</span>` +
    `<span class="status-chip status-chip--standby">${stateCounts.standby}</span>`;
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
  if (pendingAttachFiles.length === 0) return;
  const toSend = pendingAttachFiles.splice(0);
  for (const { buf, filename, wrapper } of toSend) {
    try {
      const formData = new FormData();
      formData.append('file', new Blob([buf]), filename || 'image.jpg');
      const res = await fetch(
        `/api/attach?token=${encodeURIComponent(token)}&session_id=${encodeURIComponent(sessionId)}`,
        { method: 'POST', body: formData }
      );
      if (!res.ok) showToast(`Attachment failed: HTTP ${res.status}`);
    } catch (_) {
      showToast('Attachment send failed');
    }
    if (wrapper) setTimeout(() => { wrapper.remove(); updateAttachClearBtn(); }, 1000);
  }
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
          if (data.ok && data.path) spawnCwdInput.value = data.path;
        }
      } catch (_) {}
      finally { spawnCwdBrowse.disabled = false; }
    });
  }
  spawnCwdInput.addEventListener('focus', () => renderCwdDropdown(''));
  spawnCwdInput.addEventListener('input', () => renderCwdDropdown(spawnCwdInput.value.trim()));
  spawnCwdInput.addEventListener('blur',  () => setTimeout(() => { cwdDropdown.hidden = true; }, 150));
  spawnCwdInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter')  { cwdDropdown.hidden = true; spawnSession(); }
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
    if (e.key === 'Enter' && idx >= 0) { spawnCwdInput.value = items[idx].dataset.value; cwdDropdown.hidden = true; spawnCwdInput.focus(); }
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
    const KEY = 'ai-cli-hub.settings-section-state';
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
  };

  window.__settingsResetAll = async () => {
    applyTheme('dark');
    applyFontSize('medium');
    window.setLang('ja');

    try {
      localStorage.setItem(STORAGE_TRIGGER_ENABLED_KEY, '0');
      localStorage.setItem(STORAGE_TRIGGER_PHRASE_KEY, '');
      localStorage.setItem(STORAGE_NOTIFY_SOUND_ENABLED_KEY, '0');
      localStorage.setItem(STORAGE_NOTIFY_SOUND_TYPE_KEY, 'default');
      localStorage.removeItem(STORAGE_NOTIFY_SOUND_CUSTOM_KEY);
      localStorage.setItem(STORAGE_APPROVAL_AUTO_SWITCH_KEY, '0');
      localStorage.setItem(STORAGE_QUICK_CMD_1_KEY, DEFAULT_QUICK_CMD_1);
      localStorage.setItem(STORAGE_QUICK_CMD_2_KEY, DEFAULT_QUICK_CMD_2);
    } catch (_) {}

    const triggerEnabled = document.getElementById('trigger-enabled');
    const triggerPhrase  = document.getElementById('trigger-phrase-input');
    const triggerRow     = document.getElementById('trigger-phrase-row');
    if (triggerEnabled) triggerEnabled.checked = false;
    if (triggerPhrase) triggerPhrase.value = '';
    if (triggerRow) triggerRow.hidden = true;

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
    terminals.forEach(({ term, fitAddon }, id) => {
      fitAddon.fit();
      sendResize(id, term.cols, term.rows);
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
  let interimStart = 0;
  let preVoiceText = '';

  let animFrame = null;
  let wavePhase = 0;
  let waveformRaf = null;

  const BAR_COUNT = 22;

  function getLang() {
    const lang = localStorage.getItem(STORAGE_LANG_KEY) || 'ja';
    return lang === 'ja' ? 'ja-JP' : 'en-US';
  }

  function resizeCanvas() {
    const r = canvas.getBoundingClientRect();
    if (r.width > 0) {
      canvas.width  = Math.round(r.width  * devicePixelRatio);
      canvas.height = Math.round(r.height * devicePixelRatio);
    }
  }

  function drawBars(freqData) {
    const ctx2d = canvas.getContext('2d');
    if (!ctx2d) return;
    const W = canvas.width;
    const H = canvas.height;
    ctx2d.clearRect(0, 0, W, H);
    const barW = Math.max(3, Math.floor(W / (BAR_COUNT * 2.2)));
    const gap  = (W - BAR_COUNT * barW) / (BAR_COUNT + 1);
    for (let i = 0; i < BAR_COUNT; i++) {
      let v;
      if (freqData) {
        v = freqData[Math.floor(i * freqData.length / BAR_COUNT / 3)] / 255;
      } else {
        v = Math.sin(wavePhase + i * 0.55) * 0.15 + 0.2;
      }
      const barH = Math.max(barW, v * H * 0.88);
      const x = gap + i * (barW + gap);
      const y = (H - barH) / 2;
      ctx2d.fillStyle = `rgba(59,130,246,${Math.min(1, 0.3 + v * 0.9)})`;
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
    drawBars(null);
    wavePhase += 0.28;
    animFrame = requestAnimationFrame(animLoop);
  }

  function startWaveform() {
    resizeCanvas();
    cancelAnimationFrame(animFrame);
    wavePhase = 0;
    animFrame = requestAnimationFrame(animLoop);
  }

  function stopWaveform() {
    cancelAnimationFrame(animFrame);
    animFrame = null;
  }

  function showVoiceBar() {
    voiceBar.hidden = false;
    waveformRaf = requestAnimationFrame(() => {
      resizeCanvas();
      waveformRaf = null;
    });
    startWaveform();
  }

  function hideVoiceBar() {
    if (waveformRaf) {
      cancelAnimationFrame(waveformRaf);
      waveformRaf = null;
    }
    stopWaveform();
    voiceBar.hidden = true;
  }

  let forceCleanupTimer = null;
  function forceCleanup() {
    isRecording = false;
    voiceActive = false;
    btn.classList.remove('recording');
    btn.dataset.tooltip = t('voice_tooltip');
    hideVoiceBar();
    setTimeout(() => inputEl.focus(), 0);
  }
  function scheduleForceCleanup() {
    clearTimeout(forceCleanupTimer);
    forceCleanupTimer = setTimeout(() => {
      if (isRecording) forceCleanup();
    }, 1500);
  }
  function cancelForceCleanup() {
    clearTimeout(forceCleanupTimer);
    forceCleanupTimer = null;
  }

  btn.addEventListener('click', () => {
    if (isRecording) {
      try { recognition.abort(); } catch (_) {}
      scheduleForceCleanup();
      return;
    }
    recognition.lang = getLang();
    preVoiceText = inputEl.value;
    interimStart = inputEl.value.length;
    try {
      recognition.start();
    } catch (err) {
      forceCleanup();
      showToast(t('voice_error'));
      console.error('SpeechRecognition start failed:', err);
    }
  });

  cancelBtn.addEventListener('click', () => {
    inputEl.value = preVoiceText;
    autoExpand();
    try { recognition.abort(); } catch (_) {}
    scheduleForceCleanup();
  });

  confirmBtn.addEventListener('click', () => {
    try { recognition.stop(); } catch (_) {}
    scheduleForceCleanup();
  });

  recognition.addEventListener('start', () => {
    isRecording = true;
    voiceActive = true;
    btn.classList.add('recording');
    btn.dataset.tooltip = t('voice_recording');
    showVoiceBar();
  });

  recognition.addEventListener('result', (e) => {
    const result = e.results[e.resultIndex];
    if (!result) return;
    const transcript = result[0].transcript;
    inputEl.value = inputEl.value.slice(0, interimStart) + transcript;
    if (result.isFinal) {
      inputEl.value += ' ';
      interimStart = inputEl.value.length;
      const _tp = getActiveTriggerPhrase();
      if (_tp && activeSessionId !== null && textEndsWithTriggerPhrase(buildSendText(), _tp)) {
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
    isRecording = false;
    voiceActive = false;
    btn.classList.remove('recording');
    btn.dataset.tooltip = t('voice_tooltip');
    hideVoiceBar();
    setTimeout(() => inputEl.focus(), 0);
  });

  recognition.addEventListener('error', (e) => {
    console.warn('SpeechRecognition error:', e.error, e.message || '');
    cancelForceCleanup();
    isRecording = false;
    voiceActive = false;
    btn.classList.remove('recording');
    btn.dataset.tooltip = t('voice_tooltip');
    hideVoiceBar();
    if (e.error === 'not-allowed') showToast(t('voice_error_permission'));
    else if (e.error === 'audio-capture') showToast(t('voice_error_audio_capture'));
    else if (e.error === 'network') showToast(t('voice_error_network'));
    else if (e.error === 'service-not-allowed') showToast(t('voice_error_service'));
    else if (e.error === 'language-not-supported') showToast(t('voice_error_language'));
    else if (e.error !== 'no-speech' && e.error !== 'aborted') showToast(t('voice_error'));
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
