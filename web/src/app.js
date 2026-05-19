console.log('[any-ai-cli] app.js build=2026-05-18-voice-dbg');

let _userAvatarUrl = '';
let _userDisplayName = '';

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

// i18n フォールバックヘルパ: t() がキー文字列をそのまま返した（未登録）場合に
// fallback を返す。t()自体は key を確実に返す仕様なので、簡易判定で問題ない。
function ti18n(key, fallback, vars) {
  const v = window.t ? window.t(key, vars) : key;
  return (v === key && fallback != null) ? fallback : v;
}

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
const STORAGE_TOOLS_LEFT_KEY           = 'ai_cli_hub_tools_left';
const STORAGE_USAGE_LINK_CLAUDE_KEY    = 'ai_cli_hub_usage_link_claude';
const STORAGE_USAGE_LINK_CODEX_KEY     = 'ai_cli_hub_usage_link_codex';
const STORAGE_USAGE_LINK_OLLAMA_KEY    = 'ai_cli_hub_usage_link_ollama';
const STORAGE_USAGE_LINK_OPENCODE_KEY  = 'ai_cli_hub_usage_link_opencode';
const STORAGE_VOICE_GRACE_KEY          = 'ai_cli_hub_voice_grace_seconds';
const STORAGE_DISPLAY_LOCKED_MODE_KEY  = 'ai_cli_hub_display_locked_mode';
const DEFAULT_VOICE_GRACE_SEC          = 0;
const STORAGE_WAKE_WORD_ENABLED_KEY    = 'ai_cli_hub_wake_word_enabled';
const STORAGE_WAKE_WORD_PHRASE_KEY     = 'ai_cli_hub_wake_word_phrase';
const DEFAULT_WAKE_WORD_PHRASE_JA      = 'サウンドスタート';
const DEFAULT_WAKE_WORD_PHRASE_EN      = 'SoundStart';
const DEFAULT_TRIGGER_PHRASE_JA        = 'サウンドエンド';
const DEFAULT_TRIGGER_PHRASE_EN        = 'SoundEnd';
function getDefaultWakeWordPhrase() {
  const lang = (typeof localStorage !== 'undefined' && localStorage.getItem(STORAGE_LANG_KEY)) || 'ja';
  return lang === 'en' ? DEFAULT_WAKE_WORD_PHRASE_EN : DEFAULT_WAKE_WORD_PHRASE_JA;
}
function getDefaultTriggerPhrase() {
  const lang = (typeof localStorage !== 'undefined' && localStorage.getItem(STORAGE_LANG_KEY)) || 'ja';
  return lang === 'en' ? DEFAULT_TRIGGER_PHRASE_EN : DEFAULT_TRIGGER_PHRASE_JA;
}
const CWD_HISTORY_MAX               = 10;

const DEFAULT_USAGE_LINKS = {
  claude:   'https://claude.ai/settings/usage',
  codex:    'https://chatgpt.com/codex/cloud/settings/analytics#usage',
  ollama:   'https://ollama.com/settings',
  opencode: '',
};

const FONTSIZE_MAP = { large: 15, medium: 13, small: 11 };

// ---- user-prefs サーバ同期 ----
// setUserPref(path, value)
//   path: ドット区切りの user_prefs パス（例: 'voice.wake_word_phrase'）
//   value: 任意の JSON 値
//   - 対応 localStorage キーに書く
//   - サーバへ 200ms debounced PUT（取得→パス更新→PUT 全体置換）
//   - PUT 失敗時は console.warn + トースト。localStorage は保持する

const _USER_PREFS_PATH_TO_LS = {
  'trigger.enabled':           [STORAGE_TRIGGER_ENABLED_KEY,       (v) => v ? '1' : '0'],
  'trigger.phrase':            [STORAGE_TRIGGER_PHRASE_KEY,         String],
  'notify_sound.enabled':      [STORAGE_NOTIFY_SOUND_ENABLED_KEY,  (v) => v ? '1' : '0'],
  'notify_sound.type':         [STORAGE_NOTIFY_SOUND_TYPE_KEY,     String],
  // notify_sound.custom_file はサーバ上のファイルパスのため localStorage へはミラーしない
  // （カスタム音はサーバ API /api/user-prefs/notify-sound-custom 経由で再生する）
  'voice.grace_seconds':       [STORAGE_VOICE_GRACE_KEY,           String],
  'voice.wake_word_enabled':   [STORAGE_WAKE_WORD_ENABLED_KEY,     (v) => v ? '1' : '0'],
  'voice.wake_word_phrase':    [STORAGE_WAKE_WORD_PHRASE_KEY,      String],
  'quick_cmds.cmd1':           [STORAGE_QUICK_CMD_1_KEY,           String],
  'quick_cmds.cmd2':           [STORAGE_QUICK_CMD_2_KEY,           String],
  'usage_links.claude':        [STORAGE_USAGE_LINK_CLAUDE_KEY,     String],
  'usage_links.codex':         [STORAGE_USAGE_LINK_CODEX_KEY,      String],
  'usage_links.ollama':        [STORAGE_USAGE_LINK_OLLAMA_KEY,     String],
  'usage_links.opencode':      [STORAGE_USAGE_LINK_OPENCODE_KEY,   String],
  'favorites':                 [STORAGE_FAVORITES_KEY,             JSON.stringify],
  'session_order':             [STORAGE_ORDER_KEY,                 JSON.stringify],
  'group_order':               [STORAGE_GROUP_ORDER_KEY,           JSON.stringify],
  'project_favorites':         [STORAGE_PROJECT_FAVORITES_KEY,     JSON.stringify],
  'cwd_history':               [STORAGE_CWD_HISTORY_KEY,           JSON.stringify],
  'approval.auto_switch':      [STORAGE_APPROVAL_AUTO_SWITCH_KEY,  (v) => v ? '1' : '0'],
  'spawn.defaults':            [STORAGE_SPAWN_KEY,                 JSON.stringify],
  'display.locked_mode':       [STORAGE_DISPLAY_LOCKED_MODE_KEY,   (v) => (v == null || v === '') ? '' : String(v)],
};

const _USER_PREFS_STRING_PATHS = new Set([
  'trigger.phrase',
  'notify_sound.type',
  'voice.wake_word_phrase',
  'quick_cmds.cmd1',
  'quick_cmds.cmd2',
  'usage_links.claude',
  'usage_links.codex',
  'usage_links.ollama',
  'usage_links.opencode',
  'display.locked_mode',
]);
const _USER_PREFS_STRING_ARRAY_PATHS = new Set([
  'favorites',
  'session_order',
  'group_order',
  'project_favorites',
  'cwd_history',
]);

// ドット区切りパスでオブジェクトの深いフィールドを設定する
function _setNestedValue(obj, path, value) {
  const keys = path.split('.');
  let cur = obj;
  for (let i = 0; i < keys.length - 1; i++) {
    if (cur[keys[i]] == null || typeof cur[keys[i]] !== 'object') cur[keys[i]] = {};
    cur = cur[keys[i]];
  }
  cur[keys[keys.length - 1]] = value;
}

function _parseStoredUserPref(path, raw) {
  let parsed;
  try { parsed = JSON.parse(raw); } catch (_) { parsed = raw; }

  if (path.endsWith('.enabled') || path === 'voice.wake_word_enabled' || path === 'approval.auto_switch') {
    return { ok: true, value: raw === '1' || raw === 'true' || parsed === true };
  }
  if (path === 'voice.grace_seconds') {
    const n = parseInt(String(parsed), 10);
    return { ok: true, value: Number.isFinite(n) ? Math.max(0, n) : 0 };
  }
  if (_USER_PREFS_STRING_PATHS.has(path)) {
    return { ok: true, value: parsed == null ? '' : String(parsed) };
  }
  if (_USER_PREFS_STRING_ARRAY_PATHS.has(path)) {
    if (!Array.isArray(parsed)) return { ok: false };
    return { ok: true, value: parsed.filter((v) => typeof v === 'string') };
  }
  if (path === 'spawn.defaults') {
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return { ok: false };
    const value = {};
    for (const [k, v] of Object.entries(parsed)) {
      if (typeof k === 'string' && typeof v === 'string') value[k] = v;
    }
    return { ok: true, value };
  }
  return { ok: true, value: parsed };
}

function _mergeStoredUserPrefs(current) {
  for (const [path, [lsKey, _]] of Object.entries(_USER_PREFS_PATH_TO_LS)) {
    const raw = localStorage.getItem(lsKey);
    if (raw == null) continue;
    const parsed = _parseStoredUserPref(path, raw);
    if (!parsed.ok) continue;
    _setNestedValue(current, path, parsed.value);
  }
  return current;
}

// PUT debounce タイマー
let _userPrefsDebounceTimer = null;

function _userPrefsSaveErrorMessage(err) {
  const tfn = typeof window.t === 'function' ? window.t : (key) => key;
  if (err && typeof err.status === 'number') {
    if (err.status === 401) return tfn('user_prefs_save_failed_unauthorized');
    if (err.status >= 500) return tfn('user_prefs_save_failed_server', { status: String(err.status) });
    return tfn('user_prefs_save_failed_http', { status: String(err.status) });
  }
  return tfn('user_prefs_save_failed_network');
}

function _userPrefsHttpError(phase, res) {
  const err = new Error(`${phase} /api/user-prefs ${res.status}`);
  err.phase = phase;
  err.status = res.status;
  return err;
}

function _scheduleUserPrefsPut() {
  clearTimeout(_userPrefsDebounceTimer);
  _userPrefsDebounceTimer = setTimeout(async () => {
    const tk = new URLSearchParams(location.search).get('token');
    if (!tk) return;
    try {
      // 現在のサーバ値を取得してからパッチ適用し全体置換
      const getRes = await fetch(`/api/user-prefs?token=${tk}`);
      if (!getRes.ok) throw _userPrefsHttpError('GET', getRes);
      const current = await getRes.json();
      // localStorage の最新値を current にマージ
      _mergeStoredUserPrefs(current);
      const putRes = await fetch(`/api/user-prefs?token=${tk}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(current),
      });
      if (!putRes.ok) throw _userPrefsHttpError('PUT', putRes);
    } catch (e) {
      console.warn('[user-prefs] PUT failed:', e);
      showToast(_userPrefsSaveErrorMessage(e));
    }
  }, 200);
}

function setUserPref(path, value) {
  // 1. localStorage に書く
  const entry = _USER_PREFS_PATH_TO_LS[path];
  if (entry) {
    const [lsKey, serialize] = entry;
    try {
      const serialized = serialize(value);
      if (serialized == null || serialized === 'null') {
        localStorage.removeItem(lsKey);
      } else {
        localStorage.setItem(lsKey, serialized);
      }
    } catch (_) {}
  }
  // 2. サーバへ debounced PUT
  _scheduleUserPrefsPut();
}

// サーバから user_prefs を取得して localStorage にミラーする（起動時 1 回）
async function _mirrorUserPrefsFromServer() {
  const tk = new URLSearchParams(location.search).get('token');
  if (!tk) return null;
  try {
    const res = await fetch(`/api/user-prefs?token=${tk}`);
    if (!res.ok) return null;
    const prefs = await res.json();
    // 各フィールドを localStorage にミラー（既存値を上書き）
    for (const [path, [lsKey, serialize]] of Object.entries(_USER_PREFS_PATH_TO_LS)) {
      const keys = path.split('.');
      let val = prefs;
      for (const k of keys) { if (val == null) break; val = val[k]; }
      if (val == null) continue;
      try {
        const serialized = serialize(val);
        if (serialized != null && serialized !== 'null') {
          localStorage.setItem(lsKey, serialized);
        }
      } catch (_) {}
    }
    return prefs;
  } catch (_) {
    return null;
  }
}

// 移行: localStorage 既存値 → サーバ（初回のみ）
async function migrateLocalstoragePrefsToServer() {
  const tk = new URLSearchParams(location.search).get('token');
  if (!tk) return;
  try {
    const res = await fetch(`/api/user-prefs?token=${tk}`);
    if (!res.ok) return;
    const prefs = await res.json();
    if (prefs.migrated_from_localstorage) return; // 移行済み
    // localStorage に何か値があれば移行
    let hasAny = false;
    const merged = prefs;
    for (const [path, [lsKey, _]] of Object.entries(_USER_PREFS_PATH_TO_LS)) {
      const raw = localStorage.getItem(lsKey);
      if (raw == null) continue;
      hasAny = true;
      const parsed = _parseStoredUserPref(path, raw);
      if (!parsed.ok) continue;
      _setNestedValue(merged, path, parsed.value);
    }
    if (!hasAny) return;
    merged.migrated_from_localstorage = true;
    await fetch(`/api/user-prefs?token=${tk}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(merged),
    });
    console.info('[user-prefs] migrated from localStorage to server');
  } catch (e) {
    console.warn('[user-prefs] migration failed:', e);
  }
}

// 起動時実行: サーバからミラー → 移行チェック
(async () => {
  const serverPrefs = await _mirrorUserPrefsFromServer();
  if (serverPrefs && !serverPrefs.migrated_from_localstorage) {
    await migrateLocalstoragePrefsToServer();
  }
})();

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
    const tk = new URLSearchParams(location.search).get('token');
    const customUrl = tk ? `/api/user-prefs/notify-sound-custom?token=${tk}` : null;
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

function getActiveTriggerPhrase() {
  if (localStorage.getItem(STORAGE_TRIGGER_ENABLED_KEY) !== '1') return '';
  return (localStorage.getItem(STORAGE_TRIGGER_PHRASE_KEY) ?? getDefaultTriggerPhrase()).trim();
}

function normalizeTriggerMatchText(text) {
  return String(text || '')
    .normalize('NFKC')
    .toLowerCase()
    .replace(/[\s\u3000]+/g, '')
    .replace(/[。．.!！?？、,，]+$/g, '');
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
  if (!normalizeTriggerMatchText(original).endsWith(tp)) return original;
  for (let i = 0; i <= original.length; i++) {
    if (normalizeTriggerMatchText(original.slice(i)) === tp) {
      return original.slice(0, i).replace(/[\s\u3000]+$/g, '');
    }
  }
  return original;
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
  const keyMap = {
    claude:   STORAGE_USAGE_LINK_CLAUDE_KEY,
    codex:    STORAGE_USAGE_LINK_CODEX_KEY,
    ollama:   STORAGE_USAGE_LINK_OLLAMA_KEY,
    opencode: STORAGE_USAGE_LINK_OPENCODE_KEY,
  };
  const key = keyMap[provider];
  if (!key) return DEFAULT_USAGE_LINKS[provider] || '#';
  return normalizeHttpUrl(localStorage.getItem(key), DEFAULT_USAGE_LINKS[provider] || '') || '#';
}

function applyUsageLinks() {
  for (const p of ['claude', 'codex', 'ollama', 'opencode']) {
    const el = document.getElementById(`usage-link-${p}`);
    if (el) el.href = getUsageLinkUrl(p);
  }
}

function loadUsageLinkSettings() {
  const keyMap = {
    claude:   STORAGE_USAGE_LINK_CLAUDE_KEY,
    codex:    STORAGE_USAGE_LINK_CODEX_KEY,
    ollama:   STORAGE_USAGE_LINK_OLLAMA_KEY,
    opencode: STORAGE_USAGE_LINK_OPENCODE_KEY,
  };
  for (const [p, k] of Object.entries(keyMap)) {
    const el = document.getElementById(`usage-link-${p}-url`);
    if (el) el.value = localStorage.getItem(k) || '';
  }
  applyUsageLinks();
}

function saveUsageLinkSettings() {
  const pairs = [
    ['claude',   'usage_links.claude',   STORAGE_USAGE_LINK_CLAUDE_KEY],
    ['codex',    'usage_links.codex',    STORAGE_USAGE_LINK_CODEX_KEY],
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

function initUsageDropdown() {
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

// Ollama エンコーディング警告ダイアログ（3 択）
// 戻り値: 'utf8' | 'continue' | null（キャンセル）
function appConfirmOllamaEncoding() {
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

function applyLang(lang) {
  const l = (lang === 'ja' || lang === 'en') ? lang : 'ja';
  const sel = document.getElementById('lang-select');
  if (sel) sel.value = l;
}

(function () {
  applyTheme(localStorage.getItem(STORAGE_THEME_KEY) || 'dark');
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
      setUserPref('usage_links.claude', '');
      setUserPref('usage_links.codex', '');
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
      const tk = new URLSearchParams(location.search).get('token');
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
    const tk = new URLSearchParams(location.search).get('token');
    if (!tk) return;
    try {
      const getRes = await fetch(`/api/user-prefs?token=${tk}`);
      if (!getRes.ok) throw new Error(`GET ${getRes.status}`);
      const prefs = await getRes.json();
      _setNestedValue(prefs, path, value);
      const putRes = await fetch(`/api/user-prefs?token=${tk}`, {
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
      _userDisplayName = val;
      updatePreview(_userAvatarUrl, _userDisplayName);
      await patchServerPref('display_name', val);
    }, 500);
  });

  applyBtn.addEventListener('click', async () => {
    const url = urlInputEl.value.trim();
    _userAvatarUrl = url;
    updatePreview(url, _userDisplayName);
    await patchServerPref('avatar', url);
  });

  fileBtn.addEventListener('click', () => fileInputEl.click());

  fileInputEl.addEventListener('change', async () => {
    const file = fileInputEl.files[0];
    if (!file) return;
    const tk = new URLSearchParams(location.search).get('token');
    try {
      const buf = await file.arrayBuffer();
      const res = await fetch(`/api/user-prefs/avatar?token=${tk}`, {
        method: 'PUT',
        headers: { 'Content-Type': file.type || 'application/octet-stream' },
        body: buf,
      });
      if (!res.ok) throw new Error(`PUT avatar ${res.status}`);
      // キャッシュバスター付きで更新
      _userAvatarUrl = `/api/avatar?token=${tk}`;
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
    _userAvatarUrl = '';
    updatePreview('', _userDisplayName);
    await patchServerPref('avatar', '');
  });
})();

const token = new URLSearchParams(location.search).get('token');

// usage-link デフォルトをリモート（GitHub バック）から取得して更新。
// 失敗時は DEFAULT_USAGE_LINKS のハードコード値をそのまま使う。
(async () => {
  if (!token) return;
  try {
    const res = await fetch(`/api/usage-link-defaults?token=${token}`);
    if (!res.ok) return;
    const d = await res.json();
    for (const k of ['claude', 'codex', 'ollama', 'opencode']) {
      if (typeof d[k] === 'string') DEFAULT_USAGE_LINKS[k] = d[k];
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
    _userAvatarUrl = info.userAvatar || '';
    _userDisplayName = info.userDisplayName || '';
    document.dispatchEvent(new CustomEvent('user-info-ready'));
    const ver = 'v' + (info.version || 'dev');
    const runtimeMode = info.runtime_mode || '';
    const runtimeLabel = () => {
      if (typeof window.t !== 'function') return info.runtime_label || runtimeMode;
      const key = `runtime_${String(runtimeMode).replace(/-/g, '_')}`;
      const translated = window.t(key);
      return translated && translated !== key ? translated : (info.runtime_label || runtimeMode);
    };
    const apply = () => {
      const runtime = runtimeLabel();
      const badgeEl = document.getElementById('runtime-badge');
      if (badgeEl && runtime) {
        badgeEl.textContent = runtime;
        badgeEl.hidden = false;
        badgeEl.dataset.mode = runtimeMode;
      }
      const settingsEl = document.querySelector('.settings-app-version');
      if (settingsEl) settingsEl.textContent = runtime ? `${ver} [Hub UI] - ${runtime}` : ver + ' [Hub UI]';
      const aboutEl = document.querySelector('.about-version');
      if (aboutEl) {
        aboutEl.textContent = (typeof window.t === 'function')
          ? window.t('about_version', { version: ver })
          : ver + ' [Hub UI]';
      }
      const aboutRuntimeEl = document.querySelector('.about-runtime');
      if (aboutRuntimeEl) {
        aboutRuntimeEl.textContent = runtime
          ? (typeof window.t === 'function' ? window.t('about_runtime', { runtime }) : `Runtime: ${runtime}`)
          : '';
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
    common: { official: [], custom: [] },
  };
  // アクティブプロファイル設定（サーバ側 ApprovalProfiles と同期）
  let activeProfiles = { claude: 'official', codex: 'official', common: 'official' };

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
        providerApprovalTriggers.common = norm(data.common);
      }
    } catch (e) {
      console.warn('approval patterns load failed', e);
    }
  }

  async function fetchProfileList(provider, profile) {
    try {
      const res = await fetch(`approval-patterns/${provider}.${profile}.json`);
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
      const res = await fetch(`/api/approval-patterns/${provider}?token=${token}`, {
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

const sessions = new Map();
const terminals = new Map(); // sessionId -> { term, fitAddon, container, pendingChunks, pendingTextTail, markerFilterCarry }
const approvalVisibleCache = new Map();
const multiQuestionVisibleCache = new Map(); // sessionId → bool（Claude Code AskUserQuestion 等の複数質問 UI が画面に出ているか）
const multiQuestionDismissedCache = new Map(); // sessionId → bool（banner の ✕ ボタンで誤検出を手動 dismiss した状態。次の PTY 送信でクリア）
const sequentialChoiceCache = new Map(); // sessionId → { sig, prompts, answers, index }
const approvalRawOptionsCache = new Map(); // sessionId → [{num, label, isCurrent}] または [{num, title, options}, ...]（バッチ承認）
const approvalConsumedSig = new Map(); // sessionId → 消費済み承認の署名（doSend でテキスト送信した場合の再表示防止）
const batchSelections = new Map(); // sessionId → number[]（セクションごとの選択番号、未選択は null）
let batchFocusIdx = -1; // 現在フォーカス中のバッチセクション index（-1: 未フォーカス / 範囲外）
const approvalConsumedSigDeleteTimer = new Map(); // sessionId → timer（sig を debounce 型で削除するためのタイマー）
const APPROVAL_PENDING_TEXT_TAIL_LIMIT = 12000;

// 承認選択肢の sig を計算。Ink の再描画やスクロールバック残骸による
// label の微妙な差異（前後空白、空白の重複、truncate 位置）を吸収するため normalize する。
// (Y:1/N:0) Yes/No プロンプトはどれも同じ label を持つため、_ctx に質問文ハッシュを
// 載せて区別する（連続する別質問が同一 sig で誤抑制されないように）。
function approvalSig(options) {
  if (isBatchOptions(options)) {
    return JSON.stringify(options.map(s => ({
      n: s.num,
      t: String(s.title || '').replace(/\s+/g, ' ').slice(0, 80),
      o: (s.options || []).map(o => `${o.num}:${String(o.label || '').trim().replace(/\s+/g, ' ').slice(0, 80)}`),
    })));
  }
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

function sequentialChoiceSig(prompts) {
  return _approvalCtxHash((prompts || []).map(p => `${p.key}:${p.question}:${p.options.map(o => `${o.num}.${o.label}`).join('|')}`).join('\n'));
}
const approvalHintConfirmTimers = new Map(); // sessionId → timer（生バイト検出を短時間 debounce してチカチカを防ぐ）
const toolOutputs = new Map(); // sessionId → [{uid, lines, ts}]
const sessionInputState = new Map(); // sessionId → { inputValue, pastedTextsData, pendingAttachFiles, thumbsFragment }

// =========================================================================
// chatHistory store (plan_chat-history-subview.md §C1)
//
// セッションごとのチャット履歴を保持する in-memory store。
// C2 (タブ切替) / C3 (吹き出しレンダラ) が購読して描画する。
//
// メッセージ shape:
//   { id, ts, role, kind, rawText, normalizedText, attachments, tool, meta }
//   role  : 'user' | 'ai' | 'system'
//   kind  : 'text' | 'attach' | 'approval' | 'tool'
//   rawText        : 生 PTY テキスト（StripANSI 適用済み / D16: raw 切替用）
//   normalizedText : 軽い正規化を適用したレンダリング用テキスト (D15)
//   attachments    : kind='attach' のとき [{path?, filename?, kind:'image'|'file'}]
//   tool           : kind='tool' のとき { name, args, ... }（C3 で扱う）
//   meta           : 任意の付随情報（approval の question/answer など）
//
// API:
//   pushMessage(sid, msg)    : メッセージを追加し subscriber 通知
//   getMessages(sid)         : メッセージ配列の浅いコピーを取得
//   subscribe(sid, cb)       : 変化通知購読 (unsubscribe 関数を返す)
//   onSessionRemoved(sid)    : ストア + subscriber を破棄
// =========================================================================

const chatHistory = new Map();              // sid → Message[]
const chatHistorySubs = new Map();          // sid → Set<callback>
const chatHistoryIdSeq = new Map();         // sid → 次に振る連番 (1 始まり)
const chatHistoryOutputBuffers = new Map(); // sid → { rawChunks:[], lastTs }
const chatHistoryAutoCommitTimers = new Map(); // sid → timerId
// Go 側 chatHistoryUserTurnMarker と一致させること
const CHAT_HISTORY_USER_TURN_MARKER = "\x1b]47777;user-turn\x07";

// ANSI エスケープシーケンスを取り除く軽量ヘルパ。
// 完全な StripANSI 実装ではなく、表示用 normalized 生成と raw 用の最低限の整形に使う。
// 制御文字や CSI / OSC を概ね除去できれば C3 に渡す材料としては十分。
function stripAnsiBasic(s) {
  if (!s) return '';
  // CSI: ESC [ ... letter
  // OSC: ESC ] ... BEL or ESC \\
  // その他の ESC + 1 文字
  return s
    .replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, '')
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, '')
    .replace(/\x1b[PX^_][^\x1b]*\x1b\\/g, '')
    .replace(/\x1b[()][0-9A-Za-z]/g, '')
    .replace(/\x1b[=>]/g, '')
    .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, '');
}

// D15: 軽い正規化（行頭末 trim / prompt prefix 除去 / 連続空行圧縮）。
// コードブロック内（```...```）の trim はあえてしない。
function normalizeChatText(raw) {
  if (!raw) return '';
  // [ANY-AI-CLI]...[/ANY-AI-CLI] ブロックを除去（承認マーカーはチャット履歴に表示しない）
  let s = raw.replace(/\[ANY-AI-CLI\][\s\S]*?\[\/ANY-AI-CLI\]/g, '');
  const lines = s.split(/\r?\n/);
  const out = [];
  let inFence = false;
  let blankRun = 0;
  for (let line of lines) {
    if (/^\s*```/.test(line)) inFence = !inFence;
    if (!inFence) {
      // prompt prefix 除去（行頭のみ）
      line = line.replace(/^[\s]*[▌>$#]\s?/, '');
      line = line.replace(/\s+$/, '');
    }
    if (!inFence && line.trim() === '') {
      blankRun++;
      if (blankRun <= 1) out.push('');
    } else {
      blankRun = 0;
      out.push(line);
    }
  }
  return out.join('\n').replace(/^\n+|\n+$/g, '');
}

function chatHistoryNextId(sid) {
  const next = (chatHistoryIdSeq.get(sid) || 0) + 1;
  chatHistoryIdSeq.set(sid, next);
  return next;
}

function chatHistoryNotify(sid, msg) {
  const subs = chatHistorySubs.get(sid);
  if (!subs || subs.size === 0) return;
  for (const cb of subs) {
    try { cb(msg, getMessages(sid)); }
    catch (err) { console.warn('[chatHistory] subscriber error', err); }
  }
}

function pushMessage(sid, msg) {
  if (sid === null || sid === undefined) return null;
  if (!chatHistory.has(sid)) chatHistory.set(sid, []);
  const arr = chatHistory.get(sid);
  const raw = msg.rawText != null ? msg.rawText : (msg.text != null ? msg.text : '');
  const normalized = msg.normalizedText != null
    ? msg.normalizedText
    : normalizeChatText(raw);
  const entry = {
    id: chatHistoryNextId(sid),
    ts: msg.ts || Date.now(),
    role: msg.role || 'system',
    kind: msg.kind || 'text',
    rawText: raw,
    normalizedText: normalized,
    attachments: Array.isArray(msg.attachments) ? msg.attachments.slice() : null,
    tool: msg.tool || null,
    meta: msg.meta || null,
  };
  arr.push(entry);
  chatHistoryNotify(sid, entry);
  return entry;
}

function getMessages(sid) {
  const arr = chatHistory.get(sid);
  return arr ? arr.slice() : [];
}

function subscribeChatHistory(sid, cb) {
  if (typeof cb !== 'function') return () => {};
  if (!chatHistorySubs.has(sid)) chatHistorySubs.set(sid, new Set());
  const subs = chatHistorySubs.get(sid);
  subs.add(cb);
  return () => {
    const cur = chatHistorySubs.get(sid);
    if (cur) {
      cur.delete(cb);
      if (cur.size === 0) chatHistorySubs.delete(sid);
    }
  };
}

function chatHistoryAppendOutput(sid, raw) {
  if (sid === null || sid === undefined || !raw) return;
  let buf = chatHistoryOutputBuffers.get(sid);
  if (!buf) {
    buf = { rawChunks: [], lastTs: 0 };
    chatHistoryOutputBuffers.set(sid, buf);
  }
  buf.rawChunks.push(raw);
  buf.lastTs = Date.now();
  // PTY 出力が 1.5 秒止まったら自動コミット（AI 返答完了後に手動で次の入力を待たずに表示）
  const existing = chatHistoryAutoCommitTimers.get(sid);
  if (existing) clearTimeout(existing);
  chatHistoryAutoCommitTimers.set(sid, setTimeout(() => {
    chatHistoryAutoCommitTimers.delete(sid);
    chatHistoryCommitOutput(sid);
  }, 1500));
}

function chatHistoryCommitOutput(sid) {
  const t = chatHistoryAutoCommitTimers.get(sid);
  if (t) { clearTimeout(t); chatHistoryAutoCommitTimers.delete(sid); }
  const buf = chatHistoryOutputBuffers.get(sid);
  if (!buf) return;
  if (buf.rawChunks.length === 0) return;
  const raw = buf.rawChunks.join('');
  buf.rawChunks.length = 0;
  // 純粋な制御文字のみのチャンク（プロンプト再描画等）は無視
  const stripped = stripAnsiBasic(raw);
  if (!stripped.trim()) return;
  // spinner / progress 文字のみのチャンクは無視（例: Codex が出力する "•"）
  // stripped.trim() で前後の空白・タブを除いた上で判定（"• " など末尾スペース付きを取りこぼさないため）
  if (/^[•◦⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏\r\n]+$/.test(stripped.trim())) return;
  // 起動バナー混入防止: 何もインタラクションがない状態（メッセージ0件）の間は AI 出力を記録しない。
  // role:'user' 限定では承認ボタン回答（role:'system'）後も AI 出力が記録されないため msgs.length で判定する。
  const msgs = chatHistory.get(sid);
  if (!msgs || msgs.length === 0) return;
  pushMessage(sid, {
    role: 'ai',
    kind: 'text',
    rawText: stripped,
    // normalizedText は pushMessage 側で生成
  });
}

// マーカー検出専用の commit。msgs.length === 0 の場合はリプレイの先頭マーカーとみなし、
// 起動バナーをバッファから捨てつつ空の user エントリを1件積んで以降の commit を解放する。
function chatHistoryCommitOutputOrSeed(sid) {
  const msgs = chatHistory.get(sid);
  if (!msgs || msgs.length === 0) {
    const buf = chatHistoryOutputBuffers.get(sid);
    if (buf) buf.rawChunks.length = 0;
    pushMessage(sid, { role: 'user', kind: 'text', rawText: '' });
    return;
  }
  chatHistoryCommitOutput(sid);
}

function onChatHistorySessionRemoved(sid) {
  const t = chatHistoryAutoCommitTimers.get(sid);
  if (t) { clearTimeout(t); chatHistoryAutoCommitTimers.delete(sid); }
  chatHistoryOutputBuffers.delete(sid);
  chatHistory.delete(sid);
  chatHistorySubs.delete(sid);
  chatHistoryIdSeq.delete(sid);
}

// C2/C3 から（および将来の拡張用に）window 経由でも触れるよう公開
if (typeof window !== 'undefined') {
  window.chatHistoryAPI = {
    getMessages,
    subscribe: subscribeChatHistory,
    push: pushMessage,
  };
}

const autoDismissTimers = new Map(); // sessionId → timer
const approvalSuppressUntil = new Map(); // sessionId → timestamp (sendChoice 後の誤再表示を抑制)
const approvalAutoSwitchQueue = [];
const utf8Decoder = new TextDecoder('utf-8');
const utf8Encoder = new TextEncoder();

let activeSessionId = null;
let isComposing = false;       // IMEコンポジション状態
let pendingSend = false;       // IME確定後に送信するフラグ
let composeEndSendTimer = null; // compositionend が doSend をスケジュール済みの場合のタイマーID
let lastDoSendAt = 0;          // 直前の doSend 実行時刻（二重送信防止の短時間ガード用）
const DOUBLE_SEND_GUARD_MS = 100;
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
const actionBarShownAt = new Map(); // sessionId -> timestamp(ms), Enter即確定ガード用

let favorites = JSON.parse(localStorage.getItem(STORAGE_FAVORITES_KEY) || '[]');
let projectFavorites = JSON.parse(localStorage.getItem(STORAGE_PROJECT_FAVORITES_KEY) || '[]');
let sessionOrder = JSON.parse(localStorage.getItem(STORAGE_ORDER_KEY) || '[]');
let groupOrder = JSON.parse(localStorage.getItem(STORAGE_GROUP_ORDER_KEY) || '[]');
const collapsedGroups = new Set();

function saveFavorites() {
  setUserPref('favorites', favorites);
}

function saveProjectFavorites() {
  setUserPref('project_favorites', projectFavorites);
}

function saveGroupOrder() {
  setUserPref('group_order', groupOrder);
}

function saveSessionOrder() {
  setUserPref('session_order', sessionOrder);
}

// cwd からプロジェクトキーを派生する。
// renderSessionList のグループ化（末尾セグメント）と同じ規則で揃え、
// FilesTabManager の可視性判定（curSess.project 参照）が一貫して機能するようにする。
function deriveProjectKeyFromCwd(cwd) {
  if (!cwd) return '';
  const name = String(cwd).replace(/\\/g, '/').split('/').filter(p => p.length > 0).pop() || '';
  return name;
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
  ws.send(JSON.stringify({ type: 'register', role: 'ui', token, cols, rows, ui_active_session_id: activeSessionId || 0 }));
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

    // マーカーを先にデコードして検出し、xterm.js / スキャナにはマーカー除去済みバイトを渡す
    let textChunk = '';
    let xtermBytes = bytes;
    let hasMarker = false;
    try {
      textChunk = utf8Decoder.decode(bytes, { stream: true });
      if (textChunk.includes(CHAT_HISTORY_USER_TURN_MARKER)) {
        hasMarker = true;
        xtermBytes = utf8Encoder.encode(textChunk.split(CHAT_HISTORY_USER_TURN_MARKER).join(''));
      }
    } catch (_) {}

    if (isActive) {
      writePTYChunk(id, t.term, xtermBytes, () => {
        if (t.autoScroll) t.term.scrollToBottom();
      });
    } else {
      // 非アクティブセッションは everAttached に関わらず pendingChunks に溜める。
      // 承認検出は scanBuffer ではなく pendingTextTail ベースで行うため、
      // xterm のライブ書き込みを非アクティブ中は止めてよい。
      // セッション切替時に attachTerminal → flushPending で一括 xterm 書き込みする。
      t.pendingChunks.push(xtermBytes);
    }
    trackApprovalHintFromChunk(id, xtermBytes);
    if (isActive) scheduleApprovalCheck(id);

    // chatHistory: マーカー検出でターン境界を確定し AI 出力を commit する
    if (hasMarker) {
      const parts = textChunk.split(CHAT_HISTORY_USER_TURN_MARKER);
      for (let i = 0; i < parts.length; i++) {
        if (parts[i]) chatHistoryAppendOutput(id, parts[i]);
        if (i < parts.length - 1) chatHistoryCommitOutputOrSeed(id);
      }
    } else if (textChunk) {
      chatHistoryAppendOutput(id, textChunk);
    }
    return;
  }

  if (m.type === 'approval_patterns_updated') {
    showToast(t('toast_approval_patterns_updated'));
    if (window.approvalPatternsUI && typeof window.approvalPatternsUI.onOfficialUpdated === 'function') {
      window.approvalPatternsUI.onOfficialUpdated(Array.isArray(m.providers) ? m.providers : []);
    }
    return;
  }

  if (m.type === 'snapshot') {
    const arr = typeof m.sessions === 'string' ? JSON.parse(m.sessions) : m.sessions;
    (arr || []).forEach(s => {
      if (s.state === 'completed') {
        requestSessionDismiss(s.id);
        return;
      }
      s.project = deriveProjectKeyFromCwd(s.cwd);
      if (s.LogPath && !s.log_path) s.log_path = s.LogPath;
      if (s.JSONLPath && !s.jsonl_path) s.jsonl_path = s.JSONLPath;
      sessions.set(s.id, s);
      addToSessionOrder(s.id);
    });
    document.getElementById('summary').textContent = t('connected') || '接続済み';
    renderSessionList();
    checkApprovalOnStartup();
    if (!_elapsedTimerInterval) {
      _elapsedTimerInterval = setInterval(() => updateMainTabStatus(), 1000);
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
    if (m.cwd)            { cur.cwd = m.cwd; cur.project = deriveProjectKeyFromCwd(m.cwd); }
    if (m.branch !== undefined) cur.branch      = m.branch;
    if (m.label !== undefined) cur.label       = m.label;
    if (m.shell)           cur.shell           = m.shell;
    if (m.state)           cur.state           = m.state;
    if (m.last_output_at)  cur.last_output_at  = m.last_output_at;
    if (m.started_at)      cur.started_at      = m.started_at;
    if (m.first_message)   cur.first_message   = m.first_message;
    if (m.last_message)    cur.last_message    = m.last_message;
    if (m.log_path)        cur.log_path        = m.log_path;
    if (m.jsonl_path)      cur.jsonl_path      = m.jsonl_path;
    if (m.model !== undefined) cur.model       = m.model;
    if (m.route !== undefined) cur.route       = m.route;
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

function isVideoPath(filePath) {
  return /\.(mp4|webm|ogv|mov|m4v)$/i.test(String(filePath || '').trim());
}

function isMediaPath(filePath) {
  return isImagePath(filePath) || isVideoPath(filePath);
}

function isTextPath(filePath) {
  const path = String(filePath || '').trim();
  if (/(^|[\\/])(Dockerfile|Makefile|README|LICENSE|CHANGELOG|NOTICE)$/i.test(path)) return true;
  return /\.(txt|md|markdown|rst|log|json|jsonl|yaml|yml|toml|ini|cfg|conf|env|csv|tsv|xml|html?|css|scss|sass|less|js|mjs|cjs|jsx|ts|tsx|vue|go|rs|py|rb|php|java|kt|kts|c|cc|cpp|cxx|h|hh|hpp|cs|sh|bash|zsh|fish|ps1|psm1|bat|cmd|sql|graphql|gql|proto|diff|patch|gitignore|gitattributes|editorconfig)$/i.test(path);
}

// ANY-AI-CLI 内蔵プレビューが扱える拡張子（バックエンド /api/files-content の許可リストと一致させること）
function isAnyAiCliPreviewable(filePath) {
  return isTextPath(filePath) || isMediaPath(filePath);
}

function getFilesAssetUrl(absPath, sessionId) {
  const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
  return `/api/files-asset?path=${encodeURIComponent(absPath)}&token=${encodeURIComponent(token)}${sessionQs}`;
}

function getPathOpenItem(filePath) {
  if (isVideoPath(filePath)) {
    return { icon: '🎞️', key: 'link_open_file', action: () => callOpenApi('/api/open-default-file', filePath, 'link_open_default_error') };
  }
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
    { icon: '✏️', key: 'link_rename', action: () => renameFileViaApi(filePath, sessionId) },
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

function basenameForPath(filePath) {
  const normalized = String(filePath || '').replace(/[\\/]+$/, '');
  const slash = Math.max(normalized.lastIndexOf('/'), normalized.lastIndexOf('\\'));
  return slash >= 0 ? normalized.slice(slash + 1) : normalized;
}

async function renameFileViaApi(filePath, sessionId) {
  const current = basenameForPath(filePath);
  const input = window.prompt(t('link_rename_prompt') || 'Enter new file name', current);
  if (input == null) return;
  const newName = input.trim();
  if (!newName || newName === current) return;
  try {
    const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
    const url = `/api/files-rename?token=${encodeURIComponent(token)}${sessionQs}`;
    const res = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ src: filePath, newName }),
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok || !data.ok) {
      const msg = (data && data.error) ? data.error : ('HTTP ' + res.status);
      showToast(`${t('link_rename_failed') || 'Failed to rename'}: ${msg}`);
      return;
    }
    window.dispatchEvent(new CustomEvent('any-ai-cli:files-changed', {
      detail: { kind: 'rename', oldAbs: filePath, newAbs: data.newAbs },
    }));
  } catch (err) {
    showToast(`${t('link_rename_failed') || 'Failed to rename'}: ${String(err)}`);
  }
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
  // 異なるファイルシステム間（Windows ドライブ ↔ Unix ルート、ドライブ違い、UNC 等）は
  // 相対化不能なので絶対パスをそのまま返す。
  const fromDrive = /^([A-Za-z]:)[\\/]/.exec(from);
  const toDrive = /^([A-Za-z]:)[\\/]/.exec(to);
  const fromUnix = from.startsWith('/');
  const toUnix = to.startsWith('/');
  if (!!fromDrive !== !!toDrive) return to;
  if (fromDrive && toDrive && fromDrive[1].toUpperCase() !== toDrive[1].toUpperCase()) return to;
  if (fromUnix !== toUnix && !fromDrive && !toDrive) return to;
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
  let text = String(path || '').trim().replace(/(?:\s*[,;:'"<>\])}]+)+$/, '');
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
  return text.replace(/(?:\s*[,;:'"<>\])}]+)+$/, '');
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

// Windows drive paths can appear with either backslashes or forward slashes
// in terminal output, e.g. C:\dev\app.go or C:/Users/me/.claude/CLAUDE.md.
const ABS_WIN_PATH_RE = /([A-Za-z]:[\\/](?:(?!\s+[A-Za-z]:[\\/])[^\x00-\x1f<>:"|?*])+)/g;
// 空白を挟んだ説明文中の区切り（例: "hljs / highlight / prism"）を
// Unix 絶対パスとして誤検出しないよう、セグメント内の空白は許可しない。
const ABS_UNIX_PATH_RE = /(\/[^\s\/\x00-\x1f"'<>`|]+(?:\/[^\s\/\x00-\x1f"'<>`|]*)*)/g;
const REL_PATH_RE = /(^|[\s([{"'`])((?:\.{1,2}[\\/]|[A-Za-z0-9_.-]+[\\/])(?:[^\s\x00-\x1f"'<>`|]+[\\/])*[^\s\x00-\x1f"'<>`|]+)/g;

// `Y/N` / `1/2` / `bash/zsh` 等を誤検出しないための post-filter。
// 受理条件: `./` `../` 始まり、またはセパレータ 2 個以上、または末尾拡張子あり。
function isLikelyRelPath(path) {
  if (!path) return false;
  if (/^\.{1,2}[\\/]/.test(path)) return true;
  const sepCount = (path.match(/[\\/]/g) || []).length;
  if (sepCount >= 2) return true;
  if (/\.[a-zA-Z0-9]{1,15}$/.test(path)) return true;
  return false;
}

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
    if (pathStr.length < 3) continue;
    if (!isLikelyRelPath(pathStr)) continue;
    candidates.push({ start: m.index + m[1].length, end: m.index + m[1].length + pathStr.length, text: pathStr });
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
      const buf = term.buffer.active;
      const thisLine = buf.getLine(y - 1);
      if (!thisLine) { callback([]); return; }

      // wrapped 継続行は先頭行側で処理済みなので空を返す
      if (thisLine.isWrapped) { callback([]); return; }

      // 論理行を構成する物理行（この行 + 後続の wrapped 継続行）を収集する
      const buildCellMap = (line) => {
        const cm = [];
        for (let x = 0; x < line.length; x++) {
          const cell = line.getCell(x);
          if (cell && cell.getWidth() !== 0) cm.push(x);
        }
        return cm;
      };
      const physRows = [{ y1: y, text: thisLine.translateToString(true), cellMap: buildCellMap(thisLine) }];
      let peek = y; // getLine は 0-based。peek=y は 1-based の y+1 行目
      while (true) {
        const next = buf.getLine(peek);
        if (!next || !next.isWrapped) break;
        physRows.push({ y1: peek + 1, text: next.translateToString(true), cellMap: buildCellMap(next) });
        peek++;
      }

      // 行テキストを結合し、各行の開始オフセットを記録する
      const rowOffsets = [];
      let off = 0;
      for (const r of physRows) { rowOffsets.push(off); off += r.text.length; }
      const combined = physRows.map(r => r.text).join('');

      // combined 上の charIndex → xterm の { x (1-based), y (1-based) }
      const ciToXY = (ci) => {
        let ri = physRows.length - 1;
        for (let i = 0; i < physRows.length - 1; i++) {
          if (ci < rowOffsets[i + 1]) { ri = i; break; }
        }
        const r = physRows[ri];
        const charInRow = ci - rowOffsets[ri];
        return { x: (r.cellMap[charInRow] ?? charInRow) + 1, y: r.y1 };
      };

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
        const startPos = ciToXY(startCI);
        const endPos = ciToXY(endCI);
        links.push({
          range: { start: startPos, end: endPos },
          text: pathStr,
          hover() {
            // ホバーではポップアップを開かない（クリック起動のみ）。
            // xterm のリンク下線表示は維持される。
          },
          leave() {
            scheduleHidePathPopup();
          },
          activate(_event, _text) {
            showPathPopup(capturedPath, _event.clientX, _event.clientY, id);
          }
        });
      };

      for (const re of [ABS_WIN_PATH_RE, ABS_UNIX_PATH_RE]) {
        re.lastIndex = 0;
        let m;
        while ((m = re.exec(combined)) !== null) {
          if (re === ABS_UNIX_PATH_RE && !isTerminalPathStartBoundary(combined, m.index)) continue;
          addPathLink(m[1], m.index);
        }
      }
      let m;
      REL_PATH_RE.lastIndex = 0;
      while ((m = REL_PATH_RE.exec(combined)) !== null) {
        const rawPath = m[2];
        const trimmed = trimTerminalPathCandidate(rawPath);
        if (!isLikelyRelPath(trimmed)) continue;
        addPathLink(rawPath, m.index + m[1].length);
      }
      callback(links);
    }
  });
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
      const atBottom = isTerminalAtBottom(t);
      t.autoScroll = atBottom;
      if (id === activeSessionId) updateScrollLockBtn(!atBottom);
    });
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
    if (t.autoScroll) t.term.scrollToBottom();
    scheduleApprovalCheck(id);
    return;
  }
  // writeUtf8 の onFlush は xterm 内部キューが drain した後に発火する。
  // 同期で scrollToBottom してしまうと未反映の行ぶん viewport が上に取り残されるため、
  // 最後の chunk の onFlush 内で最下部固定する。
  for (let i = 0; i < chunks.length; i++) {
    const isLast = i === chunks.length - 1;
    writePTYChunk(id, t.term, chunks[i], isLast ? () => {
      if (t.autoScroll) t.term.scrollToBottom();
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
  const wasAtBottom = isTerminalAtBottom(t);
  t.fitAddon.fit();
  if (wasAtBottom) {
    t.autoScroll = true;
    t.term.scrollToBottom();
    if (id === activeSessionId) updateScrollLockBtn(false);
  }
}

// xterm が alternate screen buffer（TUI モード, Codex 等）に居るかを判定。
// alt buffer は scrollback を持たないため term.scrollLines は no-op となり、
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

function isWheelTargetExcluded(target) {
  if (!(target instanceof Element)) return true;
  if (document.body && !document.body.contains(target)) return true;
  const input = document.getElementById('input');
  if (input && input.contains(target) && input.scrollHeight > input.clientHeight + 1) {
    return true;
  }
  if (target.closest('.card-actions')) return true;
  if (target.closest('[data-wheel-native]')) return true;
  if (target.closest('#settings-panel')) return true;
  // ファイルタブ（ツリー / プレビュー）はネイティブの wheel スクロールを使う。
  // これを除外しないと document レベルのリスナーがターミナルへ転送して preventDefault してしまい、
  // プレビューのスクロールが効かなくなる。
  if (target.closest('#files-tab-contents')) return true;
  return false;
}

function routeWheelToOpenSettingsPanel(e) {
  const panel = document.getElementById('settings-panel');
  if (!panel || panel.hidden) return false;
  const body = panel.querySelector('.settings-body');
  if (!body) return false;

  if (e.target instanceof Element && body.contains(e.target)) {
    return false;
  }

  const unit = e.deltaMode === WheelEvent.DOM_DELTA_LINE
    ? 24
    : e.deltaMode === WheelEvent.DOM_DELTA_PAGE
      ? Math.max(1, body.clientHeight)
      : 1;
  body.scrollTop += e.deltaY * unit;
  e.preventDefault();
  e.stopPropagation();
  return true;
}

// xterm のネイティブ wheel は viewport.scrollTop を直接書き換える → scroll イベント →
// scrollLines という非同期チェーンを経て初めて BufferService.isUserScrolling=true になる。
// この間に PTY 出力が来ると、内部 scroll() が `isUserScrolling || (ydisp = ybase)` で
// 強制的に最下部に戻してしまう（= AI 実行中に wheel up しても戻されるバグの正体）。
// → capture phase で wheel を奪い、scrollLines() を同期で呼んで isUserScrolling を
//    確実に立ててから xterm に伝播させない。マウス位置による分岐は不要。
document.addEventListener('wheel', (e) => {
  if (routeWheelToOpenSettingsPanel(e)) return;

  // マウスがチャット履歴ペイン上にある場合: 最近傍のスクロール可能要素へ明示スクロール
  if (e.target instanceof Element && e.target.closest('#chat-pane')) {
    let scrollEl = null;
    let el = e.target;
    while (el && el.id !== 'chat-pane') {
      const ov = window.getComputedStyle(el).overflowY;
      if ((ov === 'auto' || ov === 'scroll') && el.scrollHeight > el.clientHeight) {
        scrollEl = el;
        break;
      }
      el = el.parentElement;
    }
    if (!scrollEl) scrollEl = document.querySelector('#chat-pane .chat-timeline');
    if (scrollEl) {
      const unit = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? 24
        : e.deltaMode === WheelEvent.DOM_DELTA_PAGE ? Math.max(1, scrollEl.clientHeight) : 1;
      scrollEl.scrollTop += e.deltaY * unit;
      e.preventDefault();
      e.stopPropagation();
    }
    return;
  }

  if (activeSessionId === null) return;
  const t = terminals.get(activeSessionId);
  if (!t || !t.term) return;
  if (isWheelTargetExcluded(e.target)) return;

  if (forwardWheelToAltBuffer(activeSessionId, t, e.deltaY)) {
    markTerminalManualScrollIntent();
    e.preventDefault();
    e.stopPropagation();
    return;
  }

  markTerminalManualScrollIntent();
  const lineHeight = 24;
  const lines = Math.sign(e.deltaY) * Math.max(1, Math.round(Math.abs(e.deltaY) / lineHeight));
  try { t.term.scrollLines(lines); } catch (_) {}
  t.autoScroll = isTerminalAtBottom(t);
  if (activeSessionId !== null) updateScrollLockBtn(!t.autoScroll);
  e.preventDefault();
  e.stopPropagation();
}, { passive: false, capture: true });

let lastTerminalManualScrollAt = 0;

function markTerminalManualScrollIntent() {
  lastTerminalManualScrollAt = Date.now();
}

function scrollTerminalToBottomSoon(id, opts = {}) {
  const t = terminals.get(id);
  if (!t || !t.term) return;
  const force = !!opts.force;
  const passes = Math.max(1, opts.passes || 1);
  const startedAt = opts.startedAt || Date.now();

  const snap = () => {
    if (!terminals.has(id)) return;
    const tNext = terminals.get(id);
    if (!tNext || !tNext.term) return;
    if (force && lastTerminalManualScrollAt > startedAt) return;
    if (!force && !tNext.autoScroll) return;
    tNext.autoScroll = true;
    tNext.term.scrollToBottom();
    if (id === activeSessionId) updateScrollLockBtn(false);
  };

  snap();
  let remaining = passes;
  const scheduleNext = () => {
    if (remaining <= 0) return;
    remaining--;
    requestAnimationFrame(() => {
      snap();
      scheduleNext();
    });
  };
  scheduleNext();
}

function refitAndStickTerminalToBottomSoon(id, opts = {}) {
  if (id !== activeSessionId) return;
  const passes = Math.max(1, opts.passes || 4);
  const force = !!opts.force;
  const startedAt = opts.startedAt || Date.now();

  const run = () => {
    if (id !== activeSessionId) return;
    if (force && lastTerminalManualScrollAt > startedAt) return;
    const t = terminals.get(id);
    if (!canFitTerminal(t)) return;
    const prevCols = t.term.cols;
    const prevRows = t.term.rows;
    t.autoScroll = true;
    fitTerminalPreservingBottom(t, id);
    if (t.term.cols !== prevCols || t.term.rows !== prevRows) {
      sendResize(id, t.term.cols, t.term.rows);
    }
    scrollTerminalToBottomSoon(id, { force, passes: 1, startedAt });
  };

  requestAnimationFrame(() => {
    run();
    let remaining = passes - 1;
    const next = () => {
      if (remaining <= 0) return;
      remaining--;
      requestAnimationFrame(() => {
        run();
        next();
      });
    };
    next();
  });
}

function refitAndStickTerminalToBottomAfterLayoutSettles(id, opts = {}) {
  const startedAt = opts.startedAt || Date.now();
  const force = !!opts.force;
  const passes = opts.passes || 4;
  const delays = opts.delays || [0, 80, 220];

  for (const delay of delays) {
    setTimeout(() => {
      if (activeSessionId !== id) return;
      if (force && lastTerminalManualScrollAt > startedAt) return;
      refitAndStickTerminalToBottomSoon(id, { force, passes, startedAt });
    }, delay);
  }
}

function refitActiveTerminalAfterLayout(stickToBottom) {
  if (activeSessionId === null) return;
  const id = activeSessionId;
  const t = terminals.get(id);
  if (!canFitTerminal(t)) return;
  if (stickToBottom) {
    t.autoScroll = true;
  }
  requestAnimationFrame(() => {
    if (activeSessionId !== id || !canFitTerminal(t)) return;
    const prevCols = t.term.cols;
    const prevRows = t.term.rows;
    fitTerminalPreservingBottom(t, id);
    if (t.term.cols !== prevCols || t.term.rows !== prevRows) {
      sendResize(id, t.term.cols, t.term.rows);
    }
    if (stickToBottom) scrollTerminalToBottomSoon(id);
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
  markTerminalManualScrollIntent();
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

function asciiBytes(str) {
  const bytes = new Uint8Array(str.length);
  for (let i = 0; i < str.length; i++) bytes[i] = str.charCodeAt(i) & 0xff;
  return bytes;
}

function filterReverseVideoForDisplay(id, bytes) {
  const t = terminals.get(id);
  if (!t) return bytes;
  const carry = t.reverseVideoFilterCarry || new Uint8Array(0);
  const combined = new Uint8Array(carry.length + bytes.length);
  combined.set(carry, 0);
  combined.set(bytes, carry.length);

  const out = [];
  let i = 0;
  while (i < combined.length) {
    if (combined[i] !== 0x1b) {
      out.push(combined[i]);
      i++;
      continue;
    }
    if (i + 1 >= combined.length) break;
    if (combined[i + 1] !== 0x5b) {
      out.push(combined[i]);
      i++;
      continue;
    }

    let j = i + 2;
    while (j < combined.length && !(combined[j] >= 0x40 && combined[j] <= 0x7e)) j++;
    if (j >= combined.length) break;
    if (combined[j] !== 0x6d) {
      for (let k = i; k <= j; k++) out.push(combined[k]);
      i = j + 1;
      continue;
    }

    const params = Array.from(combined.slice(i + 2, j), b => String.fromCharCode(b)).join('');
    const parts = params.split(';');
    const hasReverse = parts.includes('7');
    const hasReverseOff = parts.includes('27');
    const filtered = parts.filter(p => p !== '7' && p !== '27');
    if (hasReverse) filtered.push('48', '5', '238');
    if (hasReverseOff) filtered.push('49');
    if (filtered.length > 0) {
      for (const b of asciiBytes(`\x1b[${filtered.join(';')}m`)) out.push(b);
    }
    i = j + 1;
  }

  t.reverseVideoFilterCarry = combined.slice(i);
  return new Uint8Array(out);
}

function writePTYChunk(id, term, bytes, onFlush) {
  const displayBytes = filterReverseVideoForDisplay(id, filterHubMarkersForDisplay(id, bytes));
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

function scanBuffer(id, limit) {
  const t = terminals.get(id);
  if (!t || !t.term.buffer) return [];
  const buf = t.term.buffer.active;
  const start = (limit != null) ? Math.max(0, buf.length - limit) : 0;
  const lines = [];
  for (let i = start; i < buf.length; i++) {
    lines.push(buf.getLine(i)?.translateToString(true) || '');
  }
  return lines;
}

// moved to /app/approval.js


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
const inputClearBtn = document.getElementById('input-clear-btn');
const pasteChipsEl = document.getElementById('paste-chips');

// ペースト折りたたみ状態
const pastedTexts = []; // [{id, text, lineCount}]
let pasteCounter = 0;

function autoExpand() {
  const t = activeSessionId === null ? null : terminals.get(activeSessionId);
  const shouldStickToBottom = !!(t && (t.autoScroll || isTerminalAtBottom(t)));
  inputEl.style.height = 'auto';
  inputEl.style.height = Math.min(inputEl.scrollHeight, Math.floor(window.innerHeight * 0.3)) + 'px';
  updateInputClearButton();
  refitActiveTerminalAfterLayout(shouldStickToBottom);
}

function updateInputClearButton() {
  inputClearBtn?.classList.toggle('has-text', inputEl.value.length > 0);
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

// Ollama route で起動したセッションでは Claude Code / Codex 側の /model コマンドが
// spawn 時固定の env (ANTHROPIC_BASE_URL=http://localhost:11434 等) と整合しないため
// 純正モデルに切替えるとエラーになる。行頭 /model 入力は送信前にブロックする。
function isOllamaModelCommandBlocked(sessionId, text) {
  const s = sessions.get(sessionId);
  if (!s || s.route !== 'ollama') return false;
  const trimmed = String(text || '').replace(/^[\s\x00-\x1f]+/, '');
  return /^\/model(\b|\s|$)/i.test(trimmed);
}

function clearInput() {
  inputEl.value = '';
  inputEl.style.height = 'auto';
  updateInputClearButton();
  clearAllPastes();
}

async function doSend(sessionId) {
  // Ollama route セッションで /model 始まりはブロック（spawn 時固定 env と不整合のため）
  if (isOllamaModelCommandBlocked(sessionId, buildSendText())) {
    showToast(t('toast_model_blocked_on_ollama'));
    return;
  }
  lastDoSendAt = Date.now();
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
  // 送信したら次のプロンプトは別物の可能性があるため dismiss フラグをクリア
  multiQuestionDismissedCache.delete(sessionId);
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
  // chatHistory: ユーザー送信は AI ターンの境界。
  // まず蓄積中の AI 出力チャンクを即 commit してから user 入力を push する。
  chatHistoryCommitOutput(sessionId);
  if (rawText && rawText !== '') {
    pushMessage(sessionId, { role: 'user', kind: 'text', rawText });
  }
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
  updateInputClearButton();
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
  autoExpand(); updateSlashMenu();
  updateInputClearButton();
  if (!isComposing) {
    const _tp = getActiveTriggerPhrase();
    if (_tp && activeSessionId !== null && textEndsWithTriggerPhrase(buildSendText(), _tp)) {
      doSend(activeSessionId);
    }
  }
});
inputEl.addEventListener('blur', () => setTimeout(hideSlashMenu, 150));
inputEl.addEventListener('compositionstart', () => { isComposing = true; });
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
    doSend(activeSessionId);
    return;
  }
  if (pendingSend) {
    pendingSend = false;
    composeEndSendTimer = setTimeout(() => {
      composeEndSendTimer = null;
      if (activeSessionId === null) return;
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

  // バッチ承認モード（複数質問の一括回答）の専用キー処理。
  // 入力が空のときのみ作動し、通常の文字入力・IME と競合しないようにする。
  if (inputEl.value === '' && !e.isComposing && isBatchActionBarVisible()) {
    if (e.key === 'Tab' && slashMenuEl.hidden) {
      moveBatchFocus(e.shiftKey ? -1 : 1);
      e.preventDefault(); return;
    }
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      moveBatchFocus(e.key === 'ArrowRight' ? 1 : -1);
      e.preventDefault(); return;
    }
    if (e.key === ' ') {
      moveBatchFocus(1);
      e.preventDefault(); return;
    }
    if (/^[0-9]$/.test(e.key) && !e.ctrlKey && !e.metaKey && !e.altKey) {
      if (handleBatchNumberKey(activeSessionId, parseInt(e.key, 10))) {
        e.preventDefault(); return;
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      sendBatchChoices(activeSessionId);
      e.preventDefault(); return;
    }
  }

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
    updateInputClearButton();
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
      const shownAt = actionBarShownAt.get(activeSessionId) || 0;
      // /model 送信直後の Enter が action-bar 初期選択を即確定してしまう事故を防ぐ。
      if (Date.now() - shownAt < 300) { e.preventDefault(); return; }
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
    doSend(activeSessionId);
    e.preventDefault();
  }
});

// ツール群 左右切替ボタン
(function initToolsFlip() {
  const wrap = document.getElementById('input-wrap');
  const btn = document.getElementById('tools-flip-btn');
  const inputArea = document.getElementById('input-area');
  const inputTools = document.getElementById('input-tools');
  if (!wrap || !btn || !inputArea || !inputTools) return;

  const applyToolsPosition = (isLeft) => {
    wrap.classList.toggle('tools-left', isLeft);
    const voiceBtn = document.getElementById('voice-btn');
    const sendBtn = document.getElementById('send-btn');
    if (isLeft) {
      wrap.append(btn, inputTools);
      if (voiceBtn) wrap.append(voiceBtn);
      if (sendBtn) wrap.append(sendBtn);
      if (inputClearBtn) wrap.append(inputClearBtn);
      wrap.append(inputArea);
    } else {
      wrap.append(inputArea);
      if (inputClearBtn) wrap.append(inputClearBtn);
      if (sendBtn) wrap.append(sendBtn);
      if (voiceBtn) wrap.append(voiceBtn);
      wrap.append(inputTools, btn);
    }
  };

  applyToolsPosition(localStorage.getItem(STORAGE_TOOLS_LEFT_KEY) === '1');
  btn.addEventListener('click', () => {
    const isLeft = !wrap.classList.contains('tools-left');
    applyToolsPosition(isLeft);
    localStorage.setItem(STORAGE_TOOLS_LEFT_KEY, isLeft ? '1' : '0');
  });
})();

inputClearBtn?.addEventListener('click', () => {
  inputEl.value = '';
  autoExpand();
  updateInputClearButton();
  inputEl.focus();
});

document.getElementById('send-btn').addEventListener('mousedown', () => {
  // クリック時に IME が確定中の場合、compositionend 後に送信するよう予約
  if (isComposing) pendingSend = true;
});
document.getElementById('send-btn').addEventListener('click', () => {
  if (activeSessionId === null) return;
  if (isComposing) return; // compositionend 側で処理する
  // 直前 (DOUBLE_SEND_GUARD_MS) に doSend 済み → autosend 等の直後 click を取り込む二重送信防止
  if (Date.now() - lastDoSendAt < DOUBLE_SEND_GUARD_MS) return;
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
  // Ollama route セッションで /model 始まりはブロック（quick-model-btn 経由含む）
  if (isOllamaModelCommandBlocked(sessionId, cmd)) {
    showToast(t('toast_model_blocked_on_ollama'));
    return;
  }
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
  focusInputForTerminalKeys();
}

function focusInputForTerminalKeys() {
  if (activeSessionId === null || document.activeElement === inputEl) return;
  try {
    inputEl.focus({ preventScroll: true });
  } catch (_) {
    inputEl.focus();
  }
}

function sendSubmittedText(sessionId, text) {
  // 送信操作は最新出力を見たい意図なので、スクロールアップ中でも最下部へ戻して追従を再開する
  const t = terminals.get(sessionId);
  if (t) {
    t.autoScroll = true;
    try { t.term.scrollToBottom(); } catch (_) {}
    if (sessionId === activeSessionId) {
      updateScrollLockBtn(false);
      // 承認バー表示中に送信すると、hideActionBar による action-bar 消失 + clearInput +
      // PTY echo back + Codex TUI の再描画が連続で走る。単発 RAF の scrollTerminalToBottomSoon
      // では onScroll で autoScroll=false に倒れた後の再描画フレームで viewport が上へズレ、
      // 最悪スクロールバック先頭まで戻る。force + 複数 delay の fit+snap で
      // レイアウト確定後（~220ms 内）まで最下部に張り付かせる。
      const startedAt = Date.now();
      scrollTerminalToBottomSoon(sessionId, { force: true, passes: 4, startedAt });
      refitAndStickTerminalToBottomAfterLayoutSettles(sessionId, { force: true, startedAt });
    }
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
  // sessions.delete より前に git/files タブの付け替えを試みる
  // （onSessionRemoved 内で sessionsRef を引いて代替を探すため、削除前の方が探しやすい）
  try { FilesTabManager.onSessionRemoved(id); } catch (_) {}
  // C1/C2: チャット履歴 store とビューモード state をクリーンアップ
  try { if (typeof onChatHistorySessionRemoved === 'function') onChatHistorySessionRemoved(id); } catch (_) {}
  try { if (typeof sessionViewMode !== 'undefined') sessionViewMode.delete(id); } catch (_) {}
  try { if (typeof sessionLazyLoaded !== 'undefined') sessionLazyLoaded.delete(id); } catch (_) {}
  sessions.delete(id);
  removeFromSessionOrder(id);
  const t = terminals.get(id);
  if (t) { try { t.term.dispose(); } catch (_) {} terminals.delete(id); }
  toolOutputs.delete(id);
  approvalVisibleCache.delete(id);
  if (multiQuestionVisibleCache.delete(id) && id === activeSessionId) {
    setMultiQuestionBannerVisible(false);
  }
  multiQuestionDismissedCache.delete(id);
  removeApprovalAutoSwitchTarget(id);
  approvalRawOptionsCache.delete(id);
  approvalConsumedSig.delete(id);
  batchSelections.delete(id);
  clearSequentialChoiceState(id);
  cancelApprovalHintConfirm(id);
  approvalSuppressUntil.delete(id);
  cleanupSessionInputState(id);
  onChatHistorySessionRemoved(id);
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

function updateSessionListActiveCard(id) {
  const root = document.getElementById('sessions');
  if (!root) return;
  root.querySelectorAll('.card').forEach(card => {
    const cid = parseInt(card.dataset.sessionId, 10);
    card.classList.toggle('active', cid === id);
  });
}

function activateSession(id) {
  if (activeSessionId !== null && activeSessionId !== id) {
    saveInputStateFor(activeSessionId);
  }
  activeSessionId = id;
  restoreInputStateFor(id);
  // files/git 表示からセッションカードへ戻る場合、先にターミナルを表示してから
  // attach/fit/detect しないと、承認 UI 検出と最下部スナップが hidden レイアウトを基準に走る。
  FilesTabManager.switchToSessionView();
  ensureTerminal(id);
  attachTerminal(id);
  updateScrollLockBtn();
  setMultiQuestionBannerVisible(!!multiQuestionVisibleCache.get(id));
  detectApproval(id);
  updateSessionListActiveCard(id);
  renderToolOutputs(id);
  updateShellBadge(id);
  updateQuickCmdButtons(id);
  // C2: D11 セッション情報チップ更新 + D13 セッション毎モード復元 + チャット件数バッジ購読
  if (typeof renderSessionInfoChip === 'function') renderSessionInfoChip();
  if (typeof applyActiveSessionViewMode === 'function') applyActiveSessionViewMode();
  if (typeof rewireChatHistorySub === 'function') rewireChatHistorySub(id);
  inputEl.focus();
  if (typeof window._wakewordSessionChanged === 'function') window._wakewordSessionChanged();
  const sessionInfo = sessions.get(id);
  if (sessionInfo) {
    const label = sessionInfo.label
      ? `[${sessionInfo.label}] #${id}`
      : `#${id}`;
    FilesTabManager.updateSessionTabLabel(label);
  }
  const switchStartedAt = Date.now();
  scrollTerminalToBottomSoon(id, { force: true, passes: 4, startedAt: switchStartedAt });
  requestAnimationFrame(() => {
    if (activeSessionId !== id) return;
    detectApproval(id);
    refitAndStickTerminalToBottomSoon(id, { force: true, passes: 4, startedAt: switchStartedAt });
  });
  refitAndStickTerminalToBottomAfterLayoutSettles(id, {
    force: true,
    passes: 4,
    startedAt: switchStartedAt,
  });
}

function updateShellBadge(id) {
  const el = document.getElementById('terminal-shell-info');
  if (!el) return;
  const s = id !== null ? sessions.get(id) : null;
  const shell = s?.shell || '';
  el.textContent = shell ? ' · ' + shell : '';
}

// provider 別の quick コマンドボタン制御。
// Ollama REPL は `/model` を持たず（近いのは `/load`）、Hub 側のスラッシュピッカーも
// Claude/Codex 専用候補で構成されているため、Ollama セッションでは両方を非活性化する。
// `/clear` は Ollama REPL にも実在し意味も一致するため活性のまま残す。
function updateQuickCmdButtons(id) {
  const s = id !== null ? sessions.get(id) : null;
  const provider = s?.provider || '';
  const isOllama = provider === 'ollama';
  const modelBtn  = document.getElementById('quick-model-btn');
  const pickerBtn = document.getElementById('slash-picker-btn');
  for (const btn of [modelBtn, pickerBtn]) {
    if (!btn) continue;
    btn.disabled = isOllama;
    if (isOllama) btn.setAttribute('aria-disabled', 'true');
    else btn.removeAttribute('aria-disabled');
  }
}

function stateLabel(state) {
  return t('state_' + state) || state;
}

function providerIconHtml(provider, size = 16) {
  const base = `class="card-provider-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="${size}" height="${size}" aria-hidden="true"`;
  const txt  = `text-anchor="middle" dominant-baseline="central" font-size="7.5" font-weight="bold" font-family="sans-serif"`;
  if (provider === 'claude') {
    return `<svg ${base}><circle cx="8" cy="8" r="6" fill="#FFF7ED" stroke="#F97316" stroke-width="2"/><text x="8" y="8" ${txt} fill="#F97316">C</text></svg>`;
  }
  if (provider === 'codex') {
    return `<svg ${base}><circle cx="8" cy="8" r="6" fill="#EFF6FF" stroke="#3B82F6" stroke-width="2"/><text x="8" y="8" ${txt} fill="#3B82F6">X</text></svg>`;
  }
  if (provider === 'ollama') {
    return `<svg ${base}><rect x="1" y="1" width="14" height="14" rx="3" fill="#FDF6E3" stroke="#C4973A" stroke-width="2"/><text x="8" y="8" ${txt} fill="#C4973A">O</text></svg>`;
  }
  if (provider === 'opencode') {
    return `<svg ${base}><rect x="1" y="1" width="14" height="14" rx="3" fill="#FAF5FF" stroke="#A855F7" stroke-width="2"/><text x="8" y="8" ${txt} fill="#A855F7">O</text></svg>`;
  }
  const letter = (provider || '?')[0].toUpperCase();
  return `<svg ${base}><circle cx="8" cy="8" r="6" fill="#F3F4F6" stroke="#6B7280" stroke-width="2"/><text x="8" y="8" ${txt} fill="#6B7280">${letter}</text></svg>`;
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
  if (bar && bar.classList.contains('visible') && !bar.classList.contains('batch')) {
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
      if (!card || e.target.closest('.card-actions') || e.target.closest('.card-branch')) {
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
      if (!card || e.target.closest('.card-actions') || e.target.closest('.card-branch')) return;
      const id = parseInt(card.dataset.sessionId, 10);
      if (id !== down.id) return;
      const moved = Math.hypot(e.clientX - down.x, e.clientY - down.y);
      if (moved <= 8) activateSession(id);
    });
    root.addEventListener('click', (e) => {
      const card = e.target.closest('.card');
      if (!card || e.target.closest('.card-actions')) return;
      // branch バッジクリックは git タブ open（stopPropagation 役）
      const branchEl = e.target.closest('.card-branch');
      if (branchEl) {
        e.stopPropagation();
        if (branchEl.getAttribute('data-disabled') === 'true') return;
        const sid = parseInt(branchEl.dataset.sid, 10);
        if (isNaN(sid)) return;
        const sess = sessions.get(sid);
        if (!sess) return;
        const gr = sess.git_root || sess.cwd || '';
        if (!gr) return;
        activateSession(sid);
        FilesTabManager.openGitTab(sid, gr, sess.branch || '');
        return;
      }
      const id = parseInt(card.dataset.sessionId, 10);
      if (!isNaN(id)) activateSession(id);
    });
    // branch バッジのキーボード操作 (Enter / Space)
    root.addEventListener('keydown', (e) => {
      const branchEl = e.target.closest && e.target.closest('.card-branch');
      if (!branchEl) return;
      if (e.key !== 'Enter' && e.key !== ' ' && e.key !== 'Spacebar') return;
      e.preventDefault();
      e.stopPropagation();
      if (branchEl.getAttribute('data-disabled') === 'true') return;
      const sid = parseInt(branchEl.dataset.sid, 10);
      if (isNaN(sid)) return;
      const sess = sessions.get(sid);
      if (!sess) return;
      const gr = sess.git_root || sess.cwd || '';
      if (!gr) return;
      activateSession(sid);
      FilesTabManager.openGitTab(sid, gr, sess.branch || '');
    });
    // カード右クリック context menu
    root.addEventListener('contextmenu', (e) => {
      const card = e.target.closest('.card');
      if (!card) return;
      const id = parseInt(card.dataset.sessionId, 10);
      if (isNaN(id)) return;
      e.preventDefault();
      openCardCtxMenu(e.clientX, e.clientY, id);
    });
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
    nameSpan.textContent = projectDisplayName;
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
      const filteredMsg = filterFirstMessage(s.last_message || s.first_message || '');
      const cwdStr = s.cwd || '';
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
      const isOllamaBackedSess = (s.route === 'ollama');
      let modelBadge = '';
      if (s.model) {
        const badgeProviderKey = isOllamaBackedSess ? 'ollama' : (s.provider || '');
        const badgeProviderLabel = isOllamaBackedSess ? 'Ollama' : providerName;
        const tip = badgeProviderLabel ? `${badgeProviderLabel} · ${s.model}` : s.model;
        modelBadge = ` <span class="card-model card-model--with-icon" data-tooltip="${escapeHtml(tip)}">${providerIconHtml(badgeProviderKey)}<span class="card-model-text">${escapeHtml(s.model)}</span></span>`;
      }
      const branchStr = s.branch || '';
      const branchTip = branchStr
        ? ti18n('card_branch_tooltip', `Open Git view (${branchStr})`, { branch: branchStr })
        : ti18n('card_branch_disabled_tooltip', 'No git repository');
      const branchDisabledAttr = branchStr ? '' : ' data-disabled="true"';
      // 空 branch でも常に span を表示（git 外であることが分かるよう "(no git)" を表示）
      const branchLabel = branchStr || ti18n('card_branch_no_git', '(no git)');
      const branchBadge = ` <span class="card-branch" role="button" tabindex="0" data-sid="${s.id}"${branchDisabledAttr} data-tooltip="${escapeHtml(branchTip)}" aria-label="${escapeHtml(branchTip)}">${escapeHtml(branchLabel)}</span>`;
      c.innerHTML =
        `<div class="card-title-row"><b>#${s.id}</b> ${providerIconHtml(s.provider)} ${providerChipHtml}${modelBadge}${branchBadge}</div>` +
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

  updateMainTabStatus();
}

function updateMainTabStatus() {
  // D11: セッション情報チップも同タイミングで更新 (state badge を反映)
  if (typeof renderSessionInfoChip === 'function') renderSessionInfoChip();
  const wrap = document.getElementById('main-tab-status');
  if (!wrap) return;
  const sess = sessions.get(activeSessionId);
  if (!sess) {
    wrap.hidden = true;
    return;
  }
  const runtimeStr = sess.started_at ? formatStartedAt(sess.started_at) : '';
  const lastStr = sess.last_output_at ? formatLastOutputAt(sess.last_output_at) : '';
  if (!runtimeStr && !lastStr) {
    wrap.hidden = true;
    return;
  }
  const runtimeLabel = (typeof window.t === 'function') ? window.t('main_tab_runtime_label') : 'Runtime';
  const lastLabel = (typeof window.t === 'function') ? window.t('main_tab_last_label') : 'Last';
  const runtimeEl = wrap.querySelector('.main-tab-status-runtime');
  const lastEl = wrap.querySelector('.main-tab-status-last');
  if (runtimeEl) {
    runtimeEl.textContent = runtimeStr ? `${runtimeLabel} ${runtimeStr}` : '';
    runtimeEl.hidden = !runtimeStr;
  }
  if (lastEl) {
    lastEl.textContent = lastStr ? `${lastLabel} [${lastStr}]` : '';
    lastEl.hidden = !lastStr;
  }
  wrap.hidden = false;
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

let _summaryResizeObserver = null;
function ensureSummaryResizeObserver() {
  if (_summaryResizeObserver) return;
  const el = document.getElementById('summary');
  if (!el) return;
  _summaryResizeObserver = new ResizeObserver(() => updateSummaryCompactMode());
  _summaryResizeObserver.observe(el);
  const parent = el.parentElement;
  if (parent) _summaryResizeObserver.observe(parent);
}

function updateSummaryCompactMode() {
  const el = document.getElementById('summary');
  if (!el) return;
  // scrollWidth > clientWidth は #summary が flex-wrap:nowrap + overflow:hidden の前提でのみ意味を持つ。
  el.classList.remove('summary--compact');
  const overflow = el.scrollWidth > el.clientWidth + 1;
  if (overflow) el.classList.add('summary--compact');
}

function render() {
  const stateCounts = { running: 0, waiting: 0, standby: 0 };
  // groupKey -> { provider, model, isOllamaBacked, count }
  // Ollama backend sessions are split per-model so each model gets its own chip.
  const providerGroups = new Map();
  sessions.forEach(s => {
    const provider = s.provider || 'unknown';
    const route = s.route || '';
    const model = s.model || '';
    const isOllamaBacked = route === 'ollama';
    const key = isOllamaBacked ? `ollama::${model}` : provider;
    const g = providerGroups.get(key);
    if (g) g.count++;
    else providerGroups.set(key, { provider, model, isOllamaBacked, count: 1 });
    const st = s.state || 'standby';
    if (st === 'running') stateCounts.running++;
    else if (st === 'waiting') stateCounts.waiting++;
    else stateCounts.standby++;
  });
  const totalWaiting = stateCounts.waiting;

  const PROVIDER_LABELS = { claude: 'Claude', codex: 'Codex', ollama: 'Ollama', opencode: 'OpenCode' };
  const PROVIDER_ORDER = { claude: 0, ollama: 1, codex: 2, opencode: 3 };
  const sortedGroups = Array.from(providerGroups.values()).sort((a, b) => {
    const ka = a.isOllamaBacked ? 'ollama' : a.provider;
    const kb = b.isOllamaBacked ? 'ollama' : b.provider;
    const oa = ka in PROVIDER_ORDER ? PROVIDER_ORDER[ka] : 99;
    const ob = kb in PROVIDER_ORDER ? PROVIDER_ORDER[kb] : 99;
    if (oa !== ob) return oa - ob;
    return (a.model || '').localeCompare(b.model || '');
  });
  const providerParts = sortedGroups.map(g => {
    if (g.isOllamaBacked) {
      const label = PROVIDER_LABELS.ollama;
      const modelHtml = g.model ? `<span class="summary-ollama-model">${escapeHtml(g.model)}</span>` : '';
      const tip = g.model ? `Ollama · ${g.model} : ${g.count}` : `Ollama : ${g.count}`;
      return `<span class="summary-provider-chip" data-tooltip="${escapeHtml(tip)}">${providerIconHtml('ollama')}<span class="compact-hide"><span class="summary-provider-name ollama">${label}</span>${modelHtml}<span class="summary-provider-count">: ${g.count}</span></span><span class="compact-count">${g.count}</span></span>`;
    }
    const provider = g.provider;
    const label = PROVIDER_LABELS[provider] || provider;
    const tip = `${label} : ${g.count}`;
    return `<span class="summary-provider-chip" data-tooltip="${escapeHtml(tip)}">${providerIconHtml(provider)}<span class="compact-hide"><span class="summary-provider-name ${provider}">${label}</span><span class="summary-provider-count">: ${g.count}</span></span><span class="compact-count">${g.count}</span></span>`;
  }).join('');

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
  ensureSummaryResizeObserver();
  updateSummaryCompactMode();
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

  terminalWrapper.addEventListener('dragenter', (e) => {
    if (!e.dataTransfer?.types.includes('Files')) return;
    e.preventDefault();
    terminalWrapper.classList.add('drag-active');
  });
  terminalWrapper.addEventListener('dragleave', (e) => {
    if (!terminalWrapper.contains(e.relatedTarget)) {
      terminalWrapper.classList.remove('drag-active');
    }
  });
  terminalWrapper.addEventListener('dragover', (e) => {
    if (e.dataTransfer?.types.includes('Files')) e.preventDefault();
  });
  terminalWrapper.addEventListener('drop', (e) => {
    e.preventDefault();
    terminalWrapper.classList.remove('drag-active');
    if (activeSessionId === null) return;
    for (const file of e.dataTransfer?.files ?? []) {
      if (isImageFile(file)) stageAttach(file);
      else stageFileAttach(file);
    }
  });
}

// チャット履歴ペインへの D&D
const chatPane = document.getElementById('chat-pane');
if (chatPane) {
  chatPane.addEventListener('dragenter', (e) => {
    if (!e.dataTransfer?.types.includes('Files')) return;
    e.preventDefault();
    chatPane.classList.add('drag-active');
  });
  chatPane.addEventListener('dragleave', (e) => {
    if (!chatPane.contains(e.relatedTarget)) {
      chatPane.classList.remove('drag-active');
    }
  });
  chatPane.addEventListener('dragover', (e) => {
    if (e.dataTransfer?.types.includes('Files')) e.preventDefault();
  });
  chatPane.addEventListener('drop', (e) => {
    e.preventDefault();
    chatPane.classList.remove('drag-active');
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
  // chatHistory 用: 送信に成功した添付の情報を集める
  const historyAttachments = [];
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
          const attachKind = /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(filename || '') ? 'image' : 'file';
          const blob = new Blob([buf]);
          historyAttachments.push({
            filename: filename || '',
            byteLength: (buf && buf.byteLength) || 0,
            kind: attachKind,
            path: data && data.saved_path ? data.saved_path : null,
            url: attachKind === 'image' ? URL.createObjectURL(blob) : null,
          });
        } catch (_) {
          showToast('Attachment response parse failed');
        }
      }
    } catch (_) {
      showToast('Attachment send failed');
    }
    if (wrapper) setTimeout(() => { wrapper.remove(); updateAttachClearBtn(); }, 1000);
  }
  // chatHistory: attach を user/attach として 1 メッセージにまとめて push
  if (historyAttachments.length > 0) {
    pushMessage(sessionId, {
      role: 'user',
      kind: 'attach',
      attachments: historyAttachments,
      rawText: '',
    });
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

function openLightbox(src, opts = {}) {
  const overlay = document.createElement('div');
  overlay.id = 'image-lightbox';
  const isVideo = opts.type === 'video';
  const media = document.createElement(isVideo ? 'video' : 'img');
  if (isVideo) {
    media.controls = true;
    media.autoplay = true;
    media.playsInline = true;
  }
  media.src = src;
  overlay.appendChild(media);
  document.body.appendChild(overlay);
  const close = () => {
    if (isVideo) {
      try { media.pause(); } catch (_) {}
    }
    overlay.remove();
    document.removeEventListener('keydown', onKey);
  };
  const onKey = (e) => { if (e.key === 'Escape') close(); };
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) close();
  });
  document.addEventListener('keydown', onKey);
}

// moved to /app/spawn-panel.js

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
      setUserPref('approval.auto_switch', approvalAutoSwitchInput.checked);
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

    setUserPref('trigger.enabled', false);
    setUserPref('trigger.phrase', getDefaultTriggerPhrase());
    setUserPref('voice.wake_word_enabled', false);
    setUserPref('voice.wake_word_phrase', getDefaultWakeWordPhrase());
    setUserPref('notify_sound.enabled', false);
    setUserPref('notify_sound.type', 'default');
    try { localStorage.removeItem(STORAGE_NOTIFY_SOUND_CUSTOM_KEY); } catch (_) {}
    setUserPref('approval.auto_switch', false);
    setUserPref('quick_cmds.cmd1', DEFAULT_QUICK_CMD_1);
    setUserPref('quick_cmds.cmd2', DEFAULT_QUICK_CMD_2);
    setUserPref('usage_links.claude', '');
    setUserPref('usage_links.codex', '');
    setUserPref('usage_links.ollama', '');
    setUserPref('usage_links.opencode', '');
    setUserPref('voice.grace_seconds', DEFAULT_VOICE_GRACE_SEC);

    const triggerEnabled = document.getElementById('trigger-enabled');
    const triggerPhrase  = document.getElementById('trigger-phrase-input');
    const triggerRow     = document.getElementById('trigger-phrase-row');
    if (triggerEnabled) triggerEnabled.checked = false;
    if (triggerPhrase) triggerPhrase.value = getDefaultTriggerPhrase();
    if (triggerRow) triggerRow.hidden = true;

    const wakeWordEnabled = document.getElementById('wakeword-enabled');
    const wakeWordPhrase  = document.getElementById('wakeword-phrase-input');
    const wakeWordRow     = document.getElementById('wakeword-phrase-row');
    if (wakeWordEnabled) wakeWordEnabled.checked = false;
    if (wakeWordPhrase) wakeWordPhrase.value = getDefaultWakeWordPhrase();
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

// moved to /app/voice.js

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

// moved to /app/files-view.js

// ─── C2: 統合タブバー (setActiveTab) ───────────────────────────────────
// セッション毎の表示モード (D13: in-memory, リロードで初期化)
const sessionViewMode = new Map(); // sid -> 'terminal' | 'chat' | 'split' | 'files' | 'git'
// Files/Git の遅延ロード状態 (sid -> Set<'files'|'git'>)
const sessionLazyLoaded = new Map();

const VALID_TAB_NAMES = new Set(['terminal', 'chat', 'split', 'files', 'git']);
// C5: lock の対象モード (Files/Git は lock 対象外: D10 の lazy 読み込みと相性が悪い)
const LOCKABLE_MODES = new Set(['terminal', 'chat', 'split']);

// C5: 「表示モードを固定」設定値の取得 ('' / 'terminal' / 'chat' / 'split')
function getDisplayLockedMode() {
  try {
    const raw = localStorage.getItem(STORAGE_DISPLAY_LOCKED_MODE_KEY);
    if (!raw) return '';
    if (LOCKABLE_MODES.has(raw)) return raw;
  } catch (_) {}
  return '';
}

function getSessionViewMode(sid) {
  if (sid === null || sid === undefined) return 'terminal';
  // C5: セッション未登録 (新規 spawn / リロード後の初回) は lock 値を初期モードとして適用
  if (!sessionViewMode.has(sid)) {
    const lock = getDisplayLockedMode();
    if (lock && LOCKABLE_MODES.has(lock)) return lock;
    return 'terminal';
  }
  return sessionViewMode.get(sid) || 'terminal';
}

function isTabLazyLoaded(sid, name) {
  const set = sessionLazyLoaded.get(sid);
  return !!(set && set.has(name));
}

function markTabLazyLoaded(sid, name) {
  if (!sessionLazyLoaded.has(sid)) sessionLazyLoaded.set(sid, new Set());
  sessionLazyLoaded.get(sid).add(name);
}

// C5: lock 値に対応するタブにのみ .locked-mode を付与 (🔒 表示)
function refreshLockedModeTabClasses() {
  const lock = getDisplayLockedMode();
  document.querySelectorAll('#unified-tab-bar .view-tab').forEach(btn => {
    const isLocked = !!lock && btn.dataset.tab === lock;
    btn.classList.toggle('locked-mode', isLocked);
  });
}

// C5: lock 中にユーザーが別タブへ切替えた時、セッションごとに 5 分クールダウン付きでトースト
const _lockedModeToastLastTs = new Map(); // sid -> ts(ms)
const LOCKED_MODE_TOAST_COOLDOWN_MS = 5 * 60 * 1000;

function maybeFireLockedModeToast(sid, requestedMode) {
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
function refreshLazyTabClasses(sid) {
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
function renderSessionInfoChip() {
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
  const providerName = s.provider === 'claude' ? 'Claude'
                     : s.provider === 'codex'  ? 'Codex'
                     : s.provider === 'ollama' ? 'Ollama'
                     : s.provider === 'opencode' ? 'OpenCode'
                     : (s.provider || '');
  const providerChipHtml = providerName
    ? `<span class="card-provider-chip ${s.provider || ''}">${escapeHtml(providerName)}</span>`
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
    ` <span class="badge ${state}">${escapeHtml(stateLbl)}</span>`;
}

// D12: チャット件数バッジ更新
function updateChatCountBadge() {
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
let _setActiveTabRecursion = false;
function setActiveTab(sid, name) {
  if (!VALID_TAB_NAMES.has(name)) return;
  const targetSid = (sid !== null && sid !== undefined) ? sid : activeSessionId;
  if (targetSid === null || targetSid === undefined) return;

  // セッション毎モードを保存 (D13)
  sessionViewMode.set(targetSid, name);

  // 現セッションへの切替でなければ DOM 反映は不要 (アクティブ化時に反映される)
  if (targetSid !== activeSessionId) return;

  const area = document.getElementById('display-area');
  if (!area) return;
  area.classList.remove('mode-terminal', 'mode-chat', 'mode-split', 'mode-files', 'mode-git');
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
}

// D10: Files/Git タブを初回クリックで開く (および既ロード時の再アクティブ化)
// openFilesTab / openGitTab は idempotent (既存タブがあれば再利用) なので、
// セッション切替で .active が外れた files/git pane の再表示にも兼用する。
function handleLazyTabOpen(sid, name) {
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
function switchToTerminalView() {
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
function applyActiveSessionViewMode() {
  if (activeSessionId === null || activeSessionId === undefined) return;
  const mode = getSessionViewMode(activeSessionId);
  refreshLazyTabClasses(activeSessionId);
  setActiveTab(activeSessionId, mode);
}

// チャット履歴の購読: バッジ更新用 (アクティブセッションだけ)
let _activeChatSubUnsub = null;
function rewireChatHistorySub(sid) {
  if (_activeChatSubUnsub) { try { _activeChatSubUnsub(); } catch (_) {} _activeChatSubUnsub = null; }
  if (sid === null || sid === undefined) return;
  if (typeof window === 'undefined' || !window.chatHistoryAPI || typeof window.chatHistoryAPI.subscribe !== 'function') return;
  _activeChatSubUnsub = window.chatHistoryAPI.subscribe(sid, (msg) => {
    if (sid !== activeSessionId) return;
    updateChatCountBadge();
    // C3: chat-pane が現在マウント中のセッションなら増分追加
    if (_chatPaneMountedSid === sid && msg) {
      appendMessage(sid, msg);
    }
  });
  updateChatCountBadge();
}

if (typeof window !== 'undefined') {
  window.setActiveTab = setActiveTab;
}

// =========================================================================
// C3: チャット履歴メッセージレンダリング本体
// docs/local/plan_chat-history-subview.md §C3
//
// 主要関数:
//   mountChatPaneForSession(sid)     — chat-pane を再構築
//   appendMessage(sid, msg)           — 1 件 append (新規メッセージのみ)
//   renderMessageBubble(msg, opts)    — DOM 要素を返す
//   renderInlineText(text)            — path / URL / inline-code → DOM 変換
//   parseToolCallsFromOutput(text, provider) — provider 別ツール呼び出し抽出
// =========================================================================

let _chatPaneMountedSid = null;

function getChatPaneEl() {
  return document.getElementById('chat-pane');
}

function getChatTimelineEl() {
  const pane = getChatPaneEl();
  if (!pane) return null;
  return pane.querySelector('.chat-timeline');
}

function chatPaneAtBottom(timeline) {
  if (!timeline) return true;
  const remain = timeline.scrollHeight - timeline.scrollTop - timeline.clientHeight;
  return remain < 60;
}

function scrollChatPaneToBottom(timeline) {
  if (!timeline) return;
  timeline.scrollTop = timeline.scrollHeight;
}

function getAiDisplayName(provider) {
  switch (provider) {
    case 'claude':   return ti18n('chat_ai_name_claude', 'Claude');
    case 'codex':    return ti18n('chat_ai_name_codex', 'Codex');
    case 'ollama':   return ti18n('chat_ai_name_ollama', 'Ollama');
    case 'opencode': return ti18n('chat_ai_name_opencode', 'OpenCode');
    default: return provider ? String(provider) : 'AI';
  }
}

function getAiAvatarLetter(provider) {
  switch (provider) {
    case 'claude':   return 'C';
    case 'codex':    return 'X';
    case 'ollama':   return 'O';
    case 'opencode': return 'P';
    default: return 'A';
  }
}

function getUserAvatarLetter() {
  if (_userDisplayName) {
    return [..._userDisplayName][0].toUpperCase();
  }
  const lang = (document.documentElement.lang || '').toLowerCase();
  return lang.startsWith('ja') ? 'あ' : 'Y';
}

function formatTimestamp(ts) {
  const d = (ts instanceof Date) ? ts : new Date(ts);
  if (Number.isNaN(d.getTime())) return '';
  const pad = (n) => String(n).padStart(2, '0');
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

function formatMsgNumber(id) {
  const n = Math.max(1, Number(id) || 1);
  return '#' + String(n).padStart(3, '0');
}

// テキスト中の URL / ファイルパス / インラインコード を DOM に変換する。
// 戻り値は DocumentFragment（または Node 配列を返さず単体 Node にする）。
function renderInlineText(text) {
  const frag = document.createDocumentFragment();
  if (!text) return frag;
  const src = String(text);

  // 全体を順次トークナイズ:
  //   - ``` で囲まれたコードブロックは行内ではここでは扱わず、上位 (renderMessageBody) で処理する
  //   - インラインコード `...` を最優先で切り出す
  //   - URL (http(s)://)
  //   - ファイルパス候補（拡張子付き、または絶対パス、または ./ ../ で始まるもの）
  //
  // 単一 regex で全パターン候補を OR して、マッチ位置順に処理する。
  //
  // 注: 簡易実装。括弧やカンマ等の終端で誤検出する可能性はあるが、C4 で
  // 右クリック時にユーザーが手動で paste 修正可能。

  // インラインコードを最初に抽出（バッククォート内は path/URL 検出をしない）
  const codeRe = /`([^`\n]+?)`/g;
  const tokens = []; // {kind:'code'|'plain', text, raw?}
  let idx = 0;
  let m;
  while ((m = codeRe.exec(src)) !== null) {
    if (m.index > idx) tokens.push({ kind: 'plain', text: src.slice(idx, m.index) });
    tokens.push({ kind: 'code', text: m[1] });
    idx = m.index + m[0].length;
  }
  if (idx < src.length) tokens.push({ kind: 'plain', text: src.slice(idx) });

  for (const tk of tokens) {
    if (tk.kind === 'code') {
      const el = document.createElement('span');
      el.className = 'code-inline';
      el.textContent = tk.text;
      frag.appendChild(el);
      continue;
    }
    // plain: URL / path を抽出
    _appendPlainWithLinks(frag, tk.text);
  }
  return frag;
}

// プレーンテキストから URL とファイルパスを抽出し、frag に追加する。
function _appendPlainWithLinks(frag, text) {
  if (!text) return;
  // URL: http(s)://...
  // path 候補:
  //   - 絶対パス Unix: /usr/... (ただしコードブロック外)
  //   - 絶対パス Windows: C:\... or C:/...
  //   - 相対パス: ./xxx ../xxx
  //   - 拡張子付き相対: foo/bar.ext または bar.ext (拡張子に絞る)
  //
  // 安全側: 末尾の句読点 ,.;:!?) を除外する。

  // 単一の包括 regex
  const re = /(https?:\/\/[^\s<>"'`)\]]+)|((?:[a-zA-Z]:[\\/]|[.]{1,2}[\\/]|\/)[^\s<>"'`)\]]+)|([\w][\w\-/\\.]*\.[a-zA-Z]{1,8}\b)/g;
  let last = 0;
  let m;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) frag.appendChild(document.createTextNode(text.slice(last, m.index)));
    let token = m[0];
    // 末尾の句読点を分離
    let trail = '';
    while (token.length > 0 && /[.,;:!?)\]\}>]/.test(token[token.length - 1])) {
      trail = token[token.length - 1] + trail;
      token = token.slice(0, -1);
    }
    if (!token) {
      frag.appendChild(document.createTextNode(m[0]));
      last = m.index + m[0].length;
      continue;
    }
    if (m[1]) {
      // URL
      const a = document.createElement('a');
      a.className = 'url-link';
      a.href = token;
      a.target = '_blank';
      a.rel = 'noopener noreferrer';
      a.textContent = token;
      frag.appendChild(a);
    } else {
      // path
      const span = document.createElement('span');
      span.className = 'path-link';
      span.dataset.path = token;
      span.textContent = token;
      frag.appendChild(span);
    }
    if (trail) frag.appendChild(document.createTextNode(trail));
    last = m.index + m[0].length;
  }
  if (last < text.length) frag.appendChild(document.createTextNode(text.slice(last)));
}

// PTY output からツール呼び出しを抽出する（provider 別の簡易版）。
// Claude フォーマット: 行頭の `● ToolName(args)` を検出。
// Codex / 他: 同様パターンを暫定で適用（誤マッチは v2 で改善）。
//
// 戻り値: [{ name, args, body }, ...]  body は次のツール呼び出しまでの本文（任意）。
function parseToolCallsFromOutput(text, _provider) {
  const calls = [];
  if (!text) return calls;
  const lines = String(text).split(/\r?\n/);
  const re = /^[\s•●○●○]*●\s+([A-Z][A-Za-z0-9_]*)\s*\(([^\n]*)\)\s*$/;
  let cur = null;
  for (const line of lines) {
    const m = line.match(re);
    if (m) {
      if (cur) calls.push(cur);
      cur = { name: m[1], args: m[2] || '', body: '' };
    } else if (cur) {
      // body 候補: " ⎿  ..." のような Claude のツール結果行を取り込む
      // ただし不確実なので最大 12 行までに抑える
      const bodyLines = cur.body ? cur.body.split('\n') : [];
      if (bodyLines.length < 12) {
        cur.body = cur.body ? cur.body + '\n' + line : line;
      }
    }
  }
  if (cur) calls.push(cur);
  return calls;
}

// AI メッセージ本文からツール呼び出し行を取り除いた残りテキストを返す。
function stripToolCallLines(text) {
  if (!text) return '';
  const lines = String(text).split(/\r?\n/);
  const out = [];
  const re = /^[\s•●○●○]*●\s+([A-Z][A-Za-z0-9_]*)\s*\([^\n]*\)\s*$/;
  let skipping = false;
  for (const line of lines) {
    if (re.test(line)) { skipping = true; continue; }
    if (skipping) {
      // ⎿ で始まる結果行はツール呼び出しの一部とみなしてスキップ
      if (/^\s*⎿/.test(line)) continue;
      skipping = false;
    }
    out.push(line);
  }
  return out.join('\n').replace(/\n{3,}/g, '\n\n').trim();
}

// 1 メッセージの DOM を構築する。
function renderMessageBubble(sid, msg) {
  const sess = (typeof sessions !== 'undefined' && sessions) ? sessions.get(sid) : null;
  const provider = (sess && sess.provider) || 'claude';
  const role = msg.role || 'system';
  const kind = msg.kind || 'text';

  const wrapEl = document.createElement('div');
  wrapEl.className = 'msg ' + role;
  wrapEl.dataset.msgId = String(msg.id);
  wrapEl.dataset.role = role;
  wrapEl.dataset.kind = kind;

  // メッセージ番号
  const numEl = document.createElement('span');
  numEl.className = 'msg-number';
  numEl.textContent = formatMsgNumber(msg.id);
  wrapEl.appendChild(numEl);

  if (role === 'system') {
    // system/approval: 中央寄せのバブル単体
    const bubble = document.createElement('div');
    bubble.className = 'bubble approval';
    const ttl = document.createElement('div');
    ttl.className = 'ttl';
    const icon = document.createElement('span');
    icon.textContent = '⚠';
    ttl.appendChild(icon);
    const title = document.createElement('span');
    title.textContent = ti18n('chat_system_approval_title', '承認待ち');
    ttl.appendChild(title);
    bubble.appendChild(ttl);

    // 質問本文（meta.kind === 'batch' なら複数質問、'single' なら単問）
    const meta = msg.meta || {};
    if (meta.kind === 'batch' && Array.isArray(meta.answers)) {
      for (const ans of meta.answers) {
        const line = document.createElement('div');
        line.appendChild(renderInlineText(String(ans.question || '')));
        bubble.appendChild(line);
      }
    } else if (msg.rawText) {
      const line = document.createElement('div');
      line.appendChild(renderInlineText(msg.normalizedText || msg.rawText));
      bubble.appendChild(line);
    }

    // 回答ライン
    const ans = document.createElement('div');
    ans.className = 'ans';
    let answerStr = '';
    if (meta.kind === 'single') {
      answerStr = meta.label ? `${meta.answer}. ${meta.label}` : String(meta.answer || msg.rawText || '');
    } else if (meta.kind === 'batch' && Array.isArray(meta.answers)) {
      answerStr = meta.answers.map(a => `${a.key}: ${a.answer}`).join(', ');
    } else {
      answerStr = msg.normalizedText || msg.rawText || '';
    }
    const timeStr = formatTimestamp(msg.ts);
    ans.textContent = ti18n('chat_approval_answered_by', `→ ${answerStr} (${timeStr} にあなたが承認)`, {
      answer: answerStr,
      time: timeStr,
    });
    bubble.appendChild(ans);

    wrapEl.appendChild(bubble);
    return wrapEl;
  }

  // user / ai 共通: avatar + bubble-wrap
  const avatar = document.createElement('div');
  avatar.className = 'avatar ' + (role === 'user' ? 'user' : ('ai ' + provider));
  if (role === 'user') {
    if (_userAvatarUrl) {
      const img = document.createElement('img');
      img.src = _userAvatarUrl;
      img.alt = getUserAvatarLetter();
      img.onerror = () => {
        img.remove();
        avatar.textContent = getUserAvatarLetter();
      };
      avatar.appendChild(img);
    } else {
      avatar.textContent = getUserAvatarLetter();
    }
  } else {
    avatar.innerHTML = providerIconHtml(provider, 30);
  }
  wrapEl.appendChild(avatar);

  const bw = document.createElement('div');
  bw.className = 'bubble-wrap';

  // meta line
  const metaEl = document.createElement('div');
  metaEl.className = 'bubble-meta';
  const ts = formatTimestamp(msg.ts);
  if (role === 'user') {
    metaEl.textContent = `${ts} · ${ti18n('chat_user', 'あなた')}`;
  } else {
    const nameSpan = document.createElement('span');
    nameSpan.className = 'name';
    nameSpan.textContent = getAiDisplayName(provider);
    metaEl.appendChild(nameSpan);
    metaEl.appendChild(document.createTextNode(' · ' + ts));
    // AI メッセージのトークン・所要時間（meta 経由 or tool 経由）
    const tool = msg.tool || {};
    const m = msg.meta || {};
    const elapsed = (typeof m.elapsed_ms === 'number') ? m.elapsed_ms
                  : (typeof tool.elapsed_ms === 'number' ? tool.elapsed_ms : null);
    if (elapsed != null) {
      const s = document.createElement('span');
      s.className = 'stat';
      const sec = (elapsed / 1000).toFixed(1) + 's';
      s.innerHTML = `⏱ <b>${escapeHtml(sec)}</b>`;
      metaEl.appendChild(s);
    }
    const tokIn  = (m.tokens_in  != null) ? m.tokens_in  : (tool.tokens_in  != null ? tool.tokens_in  : null);
    const tokOut = (m.tokens_out != null) ? m.tokens_out : (tool.tokens_out != null ? tool.tokens_out : null);
    if (tokIn != null || tokOut != null) {
      const s = document.createElement('span');
      s.className = 'stat';
      const inStr  = tokIn  != null ? Number(tokIn).toLocaleString()  : '–';
      const outStr = tokOut != null ? Number(tokOut).toLocaleString() : '–';
      s.innerHTML = `🪙 in <b>${escapeHtml(inStr)}</b> / out <b>${escapeHtml(outStr)}</b>`;
      metaEl.appendChild(s);
    }
  }
  bw.appendChild(metaEl);

  // bubble 本体
  const bubble = document.createElement('div');
  bubble.className = 'bubble ' + (role === 'user' ? 'user-text' : 'ai-text');

  const content = document.createElement('div');
  content.className = 'bubble-content';

  if (kind === 'attach') {
    // attach メッセージはテキスト無し or 短い rawText のみ
    const txt = msg.normalizedText || msg.rawText || '';
    if (txt) {
      content.appendChild(renderInlineText(txt));
    } else {
      // 添付のみの場合、placeholder テキスト
      const n = Array.isArray(msg.attachments) ? msg.attachments.length : 0;
      content.textContent = ti18n('chat_attachment_count', `${n} 件の添付`, { n });
    }
  } else if (role === 'ai') {
    // AI: ツール呼び出しを抽出してから本文を表示
    const raw = msg.normalizedText || msg.rawText || '';
    const cleanText = stripToolCallLines(raw);
    if (cleanText) content.appendChild(renderInlineText(cleanText));
    bubble.appendChild(content);
    const toolCalls = parseToolCallsFromOutput(raw, provider);
    for (const tc of toolCalls) {
      bubble.appendChild(renderToolCall(tc));
    }
  } else {
    // user/text
    const raw = msg.normalizedText || msg.rawText || '';
    content.appendChild(renderInlineText(raw));
  }

  if (kind !== 'ai' && bubble.childNodes.length === 0) bubble.appendChild(content);
  if (role !== 'ai' || kind === 'attach') {
    // ai 以外は content をまだ append していない
    if (!bubble.contains(content)) bubble.appendChild(content);
  }

  bw.appendChild(bubble);

  // attachments
  if (Array.isArray(msg.attachments) && msg.attachments.length > 0) {
    const ats = document.createElement('div');
    ats.className = 'attachments';
    for (const a of msg.attachments) {
      const thumb = document.createElement('div');
      thumb.className = 'thumb';
      if (a.url) {
        const img = document.createElement('img');
        img.src = a.url;
        img.alt = a.filename || '';
        img.style.cursor = 'pointer';
        img.addEventListener('click', () => openLightbox(img.src));
        thumb.appendChild(img);
      } else {
        const icn = document.createElement('span');
        icn.className = 'icn';
        icn.textContent = (a.kind === 'image') ? '🖼' : '📄';
        thumb.appendChild(icn);
      }
      const fname = document.createElement('span');
      fname.className = 'fname';
      fname.textContent = a.filename || '';
      thumb.appendChild(fname);
      ats.appendChild(thumb);
    }
    bw.appendChild(ats);
  }

  // C4: hover action buttons (copy / collapse) と raw link を注入
  if (typeof window !== 'undefined' && typeof window._chatC4DecorateBubble === 'function') {
    try { window._chatC4DecorateBubble(wrapEl, bw, bubble, msg); } catch (_) {}
  }

  wrapEl.appendChild(bw);
  return wrapEl;
}

function renderToolCall(tc) {
  const wrap = document.createElement('div');
  wrap.className = 'tool-call';

  const header = document.createElement('div');
  header.className = 'tool-call-header';
  const caret = document.createElement('span');
  caret.className = 'caret';
  caret.textContent = '▶';
  header.appendChild(caret);

  const tname = document.createElement('span');
  tname.className = 'tname';
  tname.textContent = tc.name || 'Tool';
  header.appendChild(tname);

  const tdesc = document.createElement('span');
  tdesc.className = 'tdesc';
  tdesc.textContent = tc.args || '';
  header.appendChild(tdesc);

  if (tc.stat) {
    const stat = document.createElement('span');
    stat.className = 'tstat';
    stat.textContent = tc.stat;
    header.appendChild(stat);
  }

  header.addEventListener('click', () => {
    wrap.classList.toggle('open');
  });
  wrap.appendChild(header);

  const body = document.createElement('div');
  body.className = 'tool-call-body';
  body.textContent = tc.body || '';
  wrap.appendChild(body);

  return wrap;
}

function updateChatPaneEmptyState(sid) {
  const pane = getChatPaneEl();
  if (!pane) return;
  let count = 0;
  try {
    const msgs = (typeof getMessages === 'function') ? getMessages(sid) : [];
    count = Array.isArray(msgs) ? msgs.length : 0;
  } catch (_) {}
  pane.classList.toggle('has-messages', count > 0);
}

// アクティブセッションの chat-pane を完全再構築する。
// セッション切替時 / モード切替で chat 系に入ったときに呼ぶ。
function mountChatPaneForSession(sid) {
  const pane = getChatPaneEl();
  const timeline = getChatTimelineEl();
  if (!pane || !timeline) return;
  // タイムラインを空にして再構築
  while (timeline.firstChild) timeline.removeChild(timeline.firstChild);
  timeline.dataset.sid = sid != null ? String(sid) : '';
  _chatPaneMountedSid = (sid !== null && sid !== undefined) ? sid : null;
  if (sid === null || sid === undefined) {
    updateChatPaneEmptyState(sid);
    return;
  }
  let msgs = [];
  try { msgs = getMessages(sid) || []; } catch (_) {}
  const frag = document.createDocumentFragment();
  for (const m of msgs) {
    frag.appendChild(renderMessageBubble(sid, m));
  }
  timeline.appendChild(frag);
  updateChatPaneEmptyState(sid);
  // C4: filter/search/minimap を再構築
  if (typeof window !== 'undefined' && typeof window._chatC4OnRemount === 'function') {
    try { window._chatC4OnRemount(sid); } catch (_) {}
  }
  // 末尾追従
  requestAnimationFrame(() => scrollChatPaneToBottom(timeline));
}

// 1 メッセージの増分追加。subscribe コールバックから呼ばれる。
function appendMessage(sid, msg) {
  if (sid !== _chatPaneMountedSid) return;
  const timeline = getChatTimelineEl();
  if (!timeline) return;
  const wasAtBottom = chatPaneAtBottom(timeline);
  // 既に同 id がある場合は skip（重複防止）
  if (timeline.querySelector(`.msg[data-msg-id="${CSS.escape(String(msg.id))}"]`)) return;
  timeline.appendChild(renderMessageBubble(sid, msg));
  updateChatPaneEmptyState(sid);
  // C4: 増分のフィルタ/検索/ミニマップ更新
  if (typeof window !== 'undefined' && typeof window._chatC4OnAppend === 'function') {
    try { window._chatC4OnAppend(sid, msg); } catch (_) {}
  }
  if (wasAtBottom) requestAnimationFrame(() => scrollChatPaneToBottom(timeline));
}

// setActiveTab が chat/split に切り替わるタイミングで chat-pane の中身を保証する。
// 既存 setActiveTab を wrap せず、毎回 mountChatPaneForSession を呼ぶ。
// setActiveTab 自体は C2 のままに保ち、本ファイル末尾で chat/split に切り替わったときの
// 副作用としてマウントを行う薄いラッパーを追加する。
const _originalSetActiveTab_C3 = setActiveTab;
setActiveTab = function (sid, name) {
  const ret = _originalSetActiveTab_C3.call(this, sid, name);
  try {
    const targetSid = (sid !== null && sid !== undefined) ? sid : activeSessionId;
    if (targetSid !== null && targetSid !== undefined &&
        targetSid === activeSessionId &&
        (name === 'chat' || name === 'split')) {
      // 既に同 sid でマウント済みなら差分のみ。違う sid なら再構築。
      if (_chatPaneMountedSid !== targetSid) {
        mountChatPaneForSession(targetSid);
      } else {
        // 念のためスクロール末尾追従
        const tl = getChatTimelineEl();
        if (tl) requestAnimationFrame(() => scrollChatPaneToBottom(tl));
      }
    }
  } catch (e) {
    console.warn('[mountChatPaneForSession] failed:', e);
  }
  return ret;
};
if (typeof window !== 'undefined') {
  window.setActiveTab = setActiveTab;
  window.mountChatPaneForSession = mountChatPaneForSession;
  window.appendChatMessage = appendMessage;
}

// =========================================================================
// C4: チャット履歴の補助機能 (子 plan plan_chat-history-subview_c4_extras.md)
//   - 子 C1: .path-link 右クリックメニュー (showPathPopup 流用)
//   - 子 C2: 吹き出し hover アクション (コピー / 折りたたみ / raw)
//            #btn-expand-all / #btn-collapse-all / #btn-raw-log 本実装
//   - 子 C3: 検索 (Ctrl+F) + フィルタチップ
//   - 子 C4: ミニマップ + J/K/Esc キーボード操作
// =========================================================================
(function initC4ChatExtras() {
  // ---- DOM ヘルパ ---------------------------------------------------------
  function chatPane() { return document.getElementById('chat-pane'); }
  function chatTimeline() {
    const p = chatPane();
    return p ? p.querySelector('.chat-timeline') : null;
  }
  function isChatVisible() {
    const p = chatPane();
    if (!p) return false;
    // hidden 属性 / 親 display:none を考慮
    if (p.hidden) return false;
    const rect = p.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }
  function activeMode() {
    const da = document.getElementById('display-area');
    if (!da) return null;
    if (da.classList.contains('mode-chat')) return 'chat';
    if (da.classList.contains('mode-split')) return 'split';
    if (da.classList.contains('mode-terminal')) return 'terminal';
    if (da.classList.contains('mode-files')) return 'files';
    if (da.classList.contains('mode-git')) return 'git';
    return null;
  }

  // =====================================================================
  // 子 C1: .path-link 右クリックメニュー
  // =====================================================================
  document.addEventListener('contextmenu', (e) => {
    const link = e.target.closest && e.target.closest('#chat-pane .path-link');
    if (!link) return;
    const p = link.dataset.path || link.textContent || '';
    if (!p) return;
    e.preventDefault();
    e.stopPropagation();
    // 既存ターミナルの showPathPopup を再利用
    if (typeof showPathPopup === 'function') {
      try { showPathPopup(p, e.clientX, e.clientY, activeSessionId); } catch (_) {}
    }
  }, true);

  // 左クリックでも popup を出す (ターミナル既存挙動と揃える)
  document.addEventListener('click', (e) => {
    const link = e.target.closest && e.target.closest('#chat-pane .path-link');
    if (!link) return;
    const p = link.dataset.path || link.textContent || '';
    if (!p) return;
    e.preventDefault();
    e.stopPropagation();
    if (typeof showPathPopup === 'function') {
      try { showPathPopup(p, e.clientX, e.clientY, activeSessionId); } catch (_) {}
    }
  });

  // =====================================================================
  // 子 C2: 吹き出し hover アクション (.bubble-actions)
  // =====================================================================
  // renderMessageBubble の最後で呼ばれる装飾フック
  window._chatC4DecorateBubble = function (wrapEl, bw, bubble, msg) {
    const role = (msg && msg.role) || 'system';
    if (role === 'system') return;
    // hover アクション群
    const acts = document.createElement('div');
    acts.className = 'bubble-actions';
    // copy ボタン
    const copyBtn = document.createElement('button');
    copyBtn.type = 'button';
    copyBtn.className = 'msg-action msg-action-copy';
    copyBtn.title = ti18n('chat_copy_btn', 'コピー');
    copyBtn.textContent = '📋';
    copyBtn.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      const text = (msg && (msg.normalizedText || msg.rawText)) || (bubble && bubble.textContent) || '';
      try {
        navigator.clipboard.writeText(text);
        const prev = copyBtn.textContent;
        copyBtn.textContent = '✓';
        copyBtn.classList.add('copied');
        setTimeout(() => { copyBtn.textContent = prev; copyBtn.classList.remove('copied'); }, 1000);
      } catch (_) {}
    });
    acts.appendChild(copyBtn);
    // 折りたたみボタン (bubble に .collapsed クラス)
    const collBtn = document.createElement('button');
    collBtn.type = 'button';
    collBtn.className = 'msg-action msg-action-collapse';
    collBtn.title = ti18n('chat_collapse_btn', '折りたたみ');
    collBtn.textContent = '–';
    collBtn.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      bubble.classList.toggle('collapsed');
      collBtn.textContent = bubble.classList.contains('collapsed') ? '+' : '–';
    });
    acts.appendChild(collBtn);

    // raw リンク (📄 raw)
    const rawWrap = document.createElement('div');
    rawWrap.className = 'bubble-raw-link';
    rawWrap.textContent = '📄 raw';
    rawWrap.title = ti18n('chat_raw_modal_title', '生 PTY テキスト');
    rawWrap.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      openRawModal(msg);
    });

    // actions と raw リンクを同じ行に並べる footer
    const footer = document.createElement('div');
    footer.className = 'bubble-footer';
    footer.appendChild(acts);
    footer.appendChild(rawWrap);
    bw.appendChild(footer);
  };

  // raw モーダル
  let _rawModalEl = null;
  function openRawModal(msg) {
    closeRawModal();
    const overlay = document.createElement('div');
    overlay.className = 'chat-raw-modal-overlay';
    const dlg = document.createElement('div');
    dlg.className = 'chat-raw-modal';
    const head = document.createElement('div');
    head.className = 'chat-raw-modal-head';
    const title = document.createElement('span');
    title.className = 'chat-raw-modal-title';
    title.textContent = ti18n('chat_raw_modal_title', '生 PTY テキスト') + ' ' + (msg && msg.id != null ? '#' + String(msg.id).padStart(3, '0') : '');
    head.appendChild(title);
    const copyBtn = document.createElement('button');
    copyBtn.type = 'button';
    copyBtn.className = 'chat-raw-modal-btn';
    copyBtn.textContent = '📋';
    copyBtn.title = ti18n('chat_copy_btn', 'コピー');
    const closeBtn = document.createElement('button');
    closeBtn.type = 'button';
    closeBtn.className = 'chat-raw-modal-btn chat-raw-modal-close';
    closeBtn.textContent = '✕';
    head.appendChild(copyBtn);
    head.appendChild(closeBtn);
    dlg.appendChild(head);
    const body = document.createElement('pre');
    body.className = 'chat-raw-modal-body';
    body.textContent = (msg && (msg.rawText || msg.normalizedText)) || '';
    dlg.appendChild(body);
    overlay.appendChild(dlg);
    document.body.appendChild(overlay);
    _rawModalEl = overlay;
    closeBtn.addEventListener('click', closeRawModal);
    overlay.addEventListener('click', (e) => { if (e.target === overlay) closeRawModal(); });
    copyBtn.addEventListener('click', () => {
      try {
        navigator.clipboard.writeText(body.textContent || '');
        copyBtn.textContent = '✓';
        setTimeout(() => { copyBtn.textContent = '📋'; }, 1000);
      } catch (_) {}
    });
  }
  function closeRawModal() {
    if (_rawModalEl) { try { _rawModalEl.remove(); } catch (_) {} _rawModalEl = null; }
  }

  // ---- 全展開 / 全折りたたみ / 生ログ (buildFilterBar から呼ぶ) ----------
  function expandAllTools() {
    const p = chatPane();
    if (!p) return;
    p.querySelectorAll('.tool-call').forEach(tc => tc.classList.add('open'));
  }
  function collapseAllTools() {
    const p = chatPane();
    if (!p) return;
    p.querySelectorAll('.tool-call').forEach(tc => tc.classList.remove('open'));
  }
  async function openRawLog() {
    try {
      const sess = activeSessionId != null ? sessions.get(activeSessionId) : null;
      const targetPath = sess && (sess.jsonl_path || sess.log_path || sess.JSONLPath || sess.LogPath);
      if (!targetPath) {
        showToast(ti18n('chat_raw_log_open_failed', '生ログを開けませんでした'));
        return;
      }
      const res = await fetch(`/api/open-dir?token=${encodeURIComponent(token)}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ kind: 'path', path: targetPath }),
      });
      if (!res.ok) {
        showToast(ti18n('chat_raw_log_open_failed', '生ログを開けませんでした'));
      }
    } catch (_) {
      showToast(ti18n('chat_raw_log_open_failed', '生ログを開けませんでした'));
    }
  }

  // =====================================================================
  // 子 C3: 検索 + フィルタチップ (.chat-filter-bar 内容構築)
  // =====================================================================
  let _activeFilters = new Set(); // empty = show all
  let _searchQuery = '';
  let _searchHits = []; // [el, el, ...] 表示順
  let _searchCursor = -1;

  function buildFilterBar() {
    const pane = chatPane();
    if (!pane) return;
    const bar = pane.querySelector('.chat-filter-bar');
    if (!bar) return;
    if (bar.dataset.c4Built === '1') return;
    bar.dataset.c4Built = '1';
    bar.hidden = false;

    const chips = [
      { key: 'all',      label: ti18n('chat_filter_all', 'すべて') },
      { key: 'user',     label: '📝 ' + ti18n('chat_filter_user', '入力') },
      { key: 'ai',       label: '🤖 ' + ti18n('chat_filter_ai', 'AI出力') },
      { key: 'attach',   label: '📎 ' + ti18n('chat_filter_attach', '添付') },
      { key: 'approval', label: '⚠ ' + ti18n('chat_filter_approval', '承認') },
    ];
    for (const c of chips) {
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'filter-chip' + (c.key === 'all' ? ' active' : '');
      b.dataset.kind = c.key;
      b.innerHTML = '';
      const lab = document.createElement('span');
      lab.className = 'filter-chip-label';
      lab.textContent = c.label;
      const count = document.createElement('span');
      count.className = 'count';
      count.textContent = '0';
      b.appendChild(lab);
      b.appendChild(count);
      b.addEventListener('click', () => {
        if (c.key === 'all') {
          _activeFilters.clear();
        } else {
          if (_activeFilters.has(c.key)) _activeFilters.delete(c.key);
          else _activeFilters.add(c.key);
        }
        bar.querySelectorAll('.filter-chip').forEach(x => {
          x.classList.toggle('active', x.dataset.kind === 'all'
            ? _activeFilters.size === 0
            : _activeFilters.has(x.dataset.kind));
        });
        applyFilterAndSearch();
        const tl = chatTimeline();
        if (tl) tl.scrollTop = 0;
      });
      bar.appendChild(b);
    }

    // 全展開 / 全折りたたみ / 生ログ / 検索を同じ行グループにまとめる。
    const actionGroup = document.createElement('div');
    actionGroup.className = 'chat-filter-actions';

    // 全展開 / 全折りたたみ / 生ログ (承認チップの右隣)
    const iconBtnDefs = [
      { id: 'btn-expand-all',   icon: '⊞', label: ti18n('btn_expand_all', '全展開'),       tip: ti18n('btn_expand_all_tooltip', '全てのツール呼び出しを展開'),       fn: () => expandAllTools() },
      { id: 'btn-collapse-all', icon: '⊟', label: ti18n('btn_collapse_all', '全折りたたみ'), tip: ti18n('btn_collapse_all_tooltip', '全てのツール呼び出しを折りたたみ'), fn: () => collapseAllTools() },
      { id: 'btn-raw-log',      icon: '📄', label: ti18n('btn_raw_log', '生ログ'),           tip: ti18n('btn_raw_log_tooltip', '生ログを開く'),                       fn: () => openRawLog() },
    ];
    for (const def of iconBtnDefs) {
      const ib = document.createElement('button');
      ib.id = def.id;
      ib.type = 'button';
      ib.className = 'icon-btn';
      ib.dataset.tooltip = def.tip;
      ib.innerHTML = `<span>${def.icon}</span><span>${def.label}</span>`;
      ib.addEventListener('click', (e) => { e.preventDefault(); def.fn(); });
      actionGroup.appendChild(ib);
    }

    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'chat-search-input';
    input.placeholder = ti18n('chat_search_placeholder', '履歴を検索 (Ctrl+F)');
    input.addEventListener('input', () => {
      _searchQuery = input.value || '';
      applyFilterAndSearch();
    });
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        if (e.shiftKey) jumpSearch(-1); else jumpSearch(+1);
      } else if (e.key === 'Escape') {
        e.preventDefault();
        input.value = '';
        _searchQuery = '';
        applyFilterAndSearch();
        input.blur();
      }
    });
    actionGroup.appendChild(input);
    bar.appendChild(actionGroup);
    bar._searchInput = input;
  }

  function getFilterBarInput() {
    const pane = chatPane();
    if (!pane) return null;
    const bar = pane.querySelector('.chat-filter-bar');
    return (bar && bar._searchInput) ? bar._searchInput : null;
  }

  function classifyMsgEl(el) {
    const role = el.dataset.role || '';
    const kind = el.dataset.kind || '';
    if (role === 'system' || kind === 'approval') return 'approval';
    if (kind === 'attach') return 'attach';
    if (role === 'user') return 'user';
    if (role === 'ai') return 'ai';
    return 'other';
  }

  function applyFilterAndSearch() {
    const tl = chatTimeline();
    const pane = chatPane();
    if (!tl || !pane) return;
    const bar = pane.querySelector('.chat-filter-bar');
    const counts = { all: 0, user: 0, ai: 0, image: 0, approval: 0 };
    const q = String(_searchQuery || '').toLowerCase();
    _searchHits = [];
    const msgs = tl.querySelectorAll('.msg');
    msgs.forEach(el => {
      const cat = classifyMsgEl(el);
      counts.all++;
      if (counts[cat] != null) counts[cat]++;
      // フィルタ
      const filterOk = _activeFilters.size === 0 || _activeFilters.has(cat);
      // 検索 (テキスト含有判定 + <mark> 化)
      // mark を毎回剥がして再適用
      unmarkInside(el);
      let searchOk = true;
      if (q) {
        const text = (el.textContent || '').toLowerCase();
        searchOk = text.indexOf(q) >= 0;
        if (searchOk) {
          highlightInside(el, q);
          _searchHits.push(el);
        }
      }
      el.classList.toggle('search-hit', !!(q && searchOk));
      el.style.display = (filterOk && searchOk) ? '' : 'none';
    });
    // counts 反映
    if (bar) {
      bar.querySelectorAll('.filter-chip').forEach(b => {
        const k = b.dataset.kind;
        const c = b.querySelector('.count');
        if (c) c.textContent = String(counts[k] != null ? counts[k] : 0);
      });
    }
    // 検索カーソルリセット
    _searchCursor = (_searchHits.length > 0) ? 0 : -1;
    if (_searchCursor >= 0) markSearchCurrent();
    // ミニマップ更新
    rebuildMinimap();
  }

  function unmarkInside(root) {
    // mark タグを外して元のテキストに戻す
    const marks = root.querySelectorAll('mark.chat-search-mark');
    marks.forEach(m => {
      const tx = document.createTextNode(m.textContent || '');
      m.parentNode.replaceChild(tx, m);
    });
    // 連続テキストを正規化
    root.normalize();
  }
  function highlightInside(root, query) {
    if (!query) return;
    const q = query.toLowerCase();
    // テキストノードのみを対象に置換 (script, style, mark 内はスキップ)
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
      acceptNode(node) {
        const p = node.parentNode;
        if (!p) return NodeFilter.FILTER_REJECT;
        if (p.nodeName === 'MARK' || p.nodeName === 'SCRIPT' || p.nodeName === 'STYLE') return NodeFilter.FILTER_REJECT;
        if (!node.nodeValue) return NodeFilter.FILTER_REJECT;
        if (node.nodeValue.toLowerCase().indexOf(q) < 0) return NodeFilter.FILTER_REJECT;
        return NodeFilter.FILTER_ACCEPT;
      },
    });
    const targets = [];
    let n;
    while ((n = walker.nextNode())) targets.push(n);
    for (const tn of targets) {
      const text = tn.nodeValue;
      const lower = text.toLowerCase();
      let idx = 0;
      const frag = document.createDocumentFragment();
      let pos = 0;
      while ((idx = lower.indexOf(q, pos)) !== -1) {
        if (idx > pos) frag.appendChild(document.createTextNode(text.slice(pos, idx)));
        const mk = document.createElement('mark');
        mk.className = 'chat-search-mark';
        mk.textContent = text.slice(idx, idx + q.length);
        frag.appendChild(mk);
        pos = idx + q.length;
      }
      if (pos < text.length) frag.appendChild(document.createTextNode(text.slice(pos)));
      tn.parentNode.replaceChild(frag, tn);
    }
  }
  function markSearchCurrent() {
    const tl = chatTimeline();
    if (!tl) return;
    tl.querySelectorAll('.msg.search-current').forEach(el => el.classList.remove('search-current'));
    if (_searchCursor >= 0 && _searchCursor < _searchHits.length) {
      const el = _searchHits[_searchCursor];
      el.classList.add('search-current');
      try { el.scrollIntoView({ behavior: 'smooth', block: 'center' }); } catch (_) {}
    }
  }
  function jumpSearch(delta) {
    if (_searchHits.length === 0) return;
    _searchCursor = (_searchCursor + delta + _searchHits.length) % _searchHits.length;
    markSearchCurrent();
  }

  // =====================================================================
  // 子 C4: ミニマップ
  // =====================================================================
  function ensureMinimap() {
    const pane = chatPane();
    if (!pane) return null;
    let mm = pane.querySelector('.minimap');
    if (!mm) {
      mm = document.createElement('div');
      mm.className = 'minimap';
      mm.title = ti18n('chat_minimap_title', 'メッセージへジャンプ');
      pane.appendChild(mm);
    }
    return mm;
  }
  function rebuildMinimap() {
    const mm = ensureMinimap();
    const tl = chatTimeline();
    if (!mm || !tl) return;
    while (mm.firstChild) mm.removeChild(mm.firstChild);
    const msgs = Array.from(tl.querySelectorAll('.msg')).filter(el => el.style.display !== 'none');
    for (const el of msgs) {
      const t = document.createElement('div');
      t.className = 'mm-tick';
      const cat = classifyMsgEl(el);
      if (cat === 'user') t.classList.add('mm-user');
      else if (cat === 'ai') t.classList.add('mm-ai');
      else t.classList.add('mm-system');
      const id = el.dataset.msgId || '?';
      const role = el.dataset.role || '';
      const rawText = (el.querySelector('.bubble-content')?.innerText || '').trim().replace(/\s+/g, ' ');
      const preview = rawText.length > 15 ? rawText.slice(0, 15) + '…' : rawText;
      t.dataset.label = preview || `#${String(id).padStart(3, '0')} ${role}`;
      t.addEventListener('click', () => {
        try { el.scrollIntoView({ behavior: 'smooth', block: 'start' }); } catch (_) {}
      });
      // 紐付け
      t._linkedMsg = el;
      mm.appendChild(t);
    }
    setupMinimapObserver();
    setupMinimapMagnification(mm);
  }
  let _mmObserver = null;
  function setupMinimapObserver() {
    if (_mmObserver) { try { _mmObserver.disconnect(); } catch (_) {} _mmObserver = null; }
    const tl = chatTimeline();
    const mm = ensureMinimap();
    if (!tl || !mm) return;
    const ticks = Array.from(mm.querySelectorAll('.mm-tick'));
    const map = new Map();
    for (const tk of ticks) if (tk._linkedMsg) map.set(tk._linkedMsg, tk);
    _mmObserver = new IntersectionObserver((entries) => {
      // ビューポート中央付近にあるメッセージを current に
      let bestEl = null;
      let bestRatio = 0;
      for (const ent of entries) {
        if (ent.intersectionRatio > bestRatio) {
          bestRatio = ent.intersectionRatio;
          bestEl = ent.target;
        }
      }
      // 全 tick から current 解除
      for (const tk of ticks) tk.classList.remove('mm-current');
      if (bestEl && map.has(bestEl)) map.get(bestEl).classList.add('mm-current');
    }, { root: tl, threshold: [0, 0.25, 0.5, 0.75, 1] });
    for (const el of map.keys()) _mmObserver.observe(el);
  }
  let _mmMagCleanup = null;
  function setupMinimapMagnification(mm) {
    if (_mmMagCleanup) { _mmMagCleanup(); _mmMagCleanup = null; }
    if (!mm) return;
    const MAX_ADD = 4;
    const SIGMA = 36;
    function onMove(e) {
      const rect = mm.getBoundingClientRect();
      const mouseY = e.clientY - rect.top;
      const ticks = mm.querySelectorAll('.mm-tick');
      for (const tk of ticks) {
        const tkRect = tk.getBoundingClientRect();
        const center = tkRect.top - rect.top + tkRect.height / 2;
        const dist = Math.abs(mouseY - center);
        const extra = MAX_ADD * Math.exp(-(dist * dist) / (2 * SIGMA * SIGMA));
        tk.style.flexGrow = (1 + extra).toFixed(2);
      }
    }
    function onLeave() {
      const ticks = mm.querySelectorAll('.mm-tick');
      for (const tk of ticks) {
        tk.style.flexGrow = '';
      }
    }
    mm.addEventListener('mousemove', onMove);
    mm.addEventListener('mouseleave', onLeave);
    _mmMagCleanup = () => {
      mm.removeEventListener('mousemove', onMove);
      mm.removeEventListener('mouseleave', onLeave);
    };
  }

  // =====================================================================
  // 子 C4: J / K / Esc キーボード操作
  // =====================================================================
  function getVisibleMessages() {
    const tl = chatTimeline();
    if (!tl) return [];
    return Array.from(tl.querySelectorAll('.msg')).filter(el => el.style.display !== 'none');
  }
  function getFocusedMessageIndex(list) {
    // .search-current 優先、無ければビューポート中央に最も近いもの
    if (list.length === 0) return -1;
    const idx = list.findIndex(el => el.classList.contains('msg-focus'));
    if (idx >= 0) return idx;
    const idxS = list.findIndex(el => el.classList.contains('search-current'));
    if (idxS >= 0) return idxS;
    const tl = chatTimeline();
    if (!tl) return 0;
    const center = tl.scrollTop + tl.clientHeight / 2;
    let best = 0;
    let bestDist = Infinity;
    for (let i = 0; i < list.length; i++) {
      const r = list[i];
      const mid = r.offsetTop + r.offsetHeight / 2;
      const d = Math.abs(mid - center);
      if (d < bestDist) { bestDist = d; best = i; }
    }
    return best;
  }
  function focusMessage(el) {
    const tl = chatTimeline();
    if (!tl || !el) return;
    tl.querySelectorAll('.msg.msg-focus').forEach(x => x.classList.remove('msg-focus'));
    el.classList.add('msg-focus');
    try { el.scrollIntoView({ behavior: 'smooth', block: 'center' }); } catch (_) {}
  }

  document.addEventListener('keydown', (e) => {
    // モーダル open 時の Esc
    if (e.key === 'Escape' && _rawModalEl) { e.preventDefault(); closeRawModal(); return; }

    // テキスト入力中は J/K/Ctrl+F は素通し (ただし検索 input への Ctrl+F フォーカスは許可)
    const ae = document.activeElement;
    const inSearch = !!(ae && ae.classList && ae.classList.contains('chat-search-input'));
    const inOtherInput = !!(ae && (ae.tagName === 'INPUT' || ae.tagName === 'TEXTAREA') && !inSearch);

    // Ctrl+F: chat タブ表示中なら検索 input にフォーカス
    if ((e.ctrlKey || e.metaKey) && (e.key === 'f' || e.key === 'F')) {
      if (!isChatVisible()) return;
      const mode = activeMode();
      if (mode !== 'chat' && mode !== 'split') return;
      const inp = getFilterBarInput();
      if (!inp) return;
      e.preventDefault();
      e.stopPropagation();
      try { inp.focus(); inp.select && inp.select(); } catch (_) {}
      return;
    }

    // J / K: chat タブのみ。検索 input / IME / 通常 input にフォーカスがあるときは無効
    if (e.key === 'j' || e.key === 'J' || e.key === 'k' || e.key === 'K') {
      if (e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return;
      if (inSearch || inOtherInput) return;
      if (!isChatVisible()) return;
      const mode = activeMode();
      // terminal モードは xterm 優先 (J/K を介入しない)
      if (mode === 'terminal' || mode === 'files' || mode === 'git') return;
      const list = getVisibleMessages();
      if (list.length === 0) return;
      const cur = getFocusedMessageIndex(list);
      let next;
      if (e.key === 'j' || e.key === 'J') next = Math.min(list.length - 1, cur + 1);
      else next = Math.max(0, cur - 1);
      e.preventDefault();
      e.stopPropagation();
      focusMessage(list[next]);
      return;
    }

    // Esc: 検索クリア (chat タブ表示中のみ)
    if (e.key === 'Escape') {
      if (!isChatVisible()) return;
      const mode = activeMode();
      if (mode !== 'chat' && mode !== 'split') return;
      const inp = getFilterBarInput();
      if (inp && (inp.value !== '' || _searchQuery !== '')) {
        e.preventDefault();
        inp.value = '';
        _searchQuery = '';
        applyFilterAndSearch();
        return;
      }
    }
  });

  // =====================================================================
  // ライフサイクルフック: mount / append でフィルタ・ミニマップを更新
  // =====================================================================
  window._chatC4OnRemount = function (_sid) {
    buildFilterBar();
    applyFilterAndSearch();
    rebuildMinimap();
  };
  window._chatC4OnAppend = function (_sid, _msg) {
    // 1 件追加。フィルタ・検索を最新の状態に再適用 (count・hit 配列の更新)
    applyFilterAndSearch();
  };

  // 初回マウント済みの場合に備えて初期化を試みる
  try { buildFilterBar(); } catch (_) {}
})();

// ─── カード右クリックメニュー (Open Git / Files / Activate / Copy ID) ───
let _cardCtxMenuEl = null;
let _cardCtxSid    = null;
function openCardCtxMenu(x, y, sid) {
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
        const gr = sess.git_root || sess.cwd || '';
        if (!gr) return;
        FilesTabManager.openGitTab(id, gr, sess.branch || '');
      } else if (action === 'open-files') {
        const gr = sess.git_root || sess.cwd || '';
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
function closeCardCtxMenu() {
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
    const gr = sess.git_root || sess.cwd || '';
    if (!gr) return;
    e.preventDefault();
    e.stopPropagation();
    FilesTabManager.openGitTab(sid, gr, sess.branch || '');
  } else if (k === 'F' || k === 'f') {
    const sid = activeSessionId;
    if (sid == null) return;
    const sess = sessions.get(sid);
    if (!sess) return;
    const gr = sess.git_root || sess.cwd || '';
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

// moved to /app/files-view.js

// moved to /app/git-view.js

