console.log('[any-ai-cli] app.js build=2026-05-16-wakeword-disabled');

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
const autoDismissTimers = new Map(); // sessionId → timer
const approvalSuppressUntil = new Map(); // sessionId → timestamp (sendChoice 後の誤再表示を抑制)
const approvalAutoSwitchQueue = [];
const utf8Decoder = new TextDecoder('utf-8');

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
        if (t.autoScroll) t.term.scrollToBottom();
      } : undefined);
    } else {
      t.pendingChunks.push(bytes);
    }
    trackApprovalHintFromChunk(id, bytes);
    if (isActive) scheduleApprovalCheck(id);
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
    if (m.cwd)            { cur.cwd = m.cwd; cur.project = deriveProjectKeyFromCwd(m.cwd); }
    if (m.branch !== undefined) cur.branch      = m.branch;
    if (m.label !== undefined) cur.label       = m.label;
    if (m.shell)           cur.shell           = m.shell;
    if (m.state)           cur.state           = m.state;
    if (m.last_output_at)  cur.last_output_at  = m.last_output_at;
    if (m.started_at)      cur.started_at      = m.started_at;
    if (m.first_message)   cur.first_message   = m.first_message;
    if (m.last_message)    cur.last_message    = m.last_message;
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
        while ((m = re.exec(text)) !== null) {
          if (re === ABS_UNIX_PATH_RE && !isTerminalPathStartBoundary(text, m.index)) continue;
          addPathLink(m[1], m.index);
        }
      }
      let m;
      REL_PATH_RE.lastIndex = 0;
      while ((m = REL_PATH_RE.exec(text)) !== null) {
        const rawPath = m[2];
        const trimmed = trimTerminalPathCandidate(rawPath);
        if (!isLikelyRelPath(trimmed)) continue;
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
const sequentialQuestionHeaderRe = /^\s*([A-Z]{1,3}\d{1,3}|Q\d{1,3}|問\d{1,3})\s*[:：]\s*(.+?)\s*$/i;

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

function extractSequentialChoicePrompts(lines) {
  const prompts = [];
  let current = null;
  const recent = (lines || []).slice(-80).map(line => String(line || '').trimEnd());
  for (const rawLine of recent) {
    const line = rawLine.trim();
    if (!line) continue;
    if (/\[ANY-AI-CLI\]|\[\/ANY-AI-CLI\]/.test(line)) return null;

    const hm = line.match(sequentialQuestionHeaderRe);
    if (hm) {
      if (current && current.options.length >= 2) prompts.push(current);
      current = {
        key: hm[1].trim(),
        question: hm[2].trim(),
        options: [],
      };
      continue;
    }

    if (!current) continue;
    const om = line.match(/^\s*(\d{1,2})\.\s*(.+?)\s*$/);
    if (om) {
      current.options.push({
        num: parseInt(om[1], 10),
        label: om[2].trim(),
        isCurrent: current.options.length === 0,
      });
      continue;
    }
    if (/^\s*N\.\s*(User specifies|その他指定)/i.test(line)) continue;

    // 選択肢群の後に説明文へ戻ったら、この質問ブロックはいったん閉じる。
    if (current.options.length > 0 && !/^\s{2,}/.test(rawLine)) {
      if (current.options.length >= 2) prompts.push(current);
      current = null;
    }
  }
  if (current && current.options.length >= 2) prompts.push(current);

  const unique = [];
  const seen = new Set();
  for (const prompt of prompts) {
    const key = `${prompt.key}:${prompt.question}`;
    if (seen.has(key)) continue;
    seen.add(key);
    prompt.options.sort((a, b) => a.num - b.num);
    unique.push(prompt);
  }
  return unique.length >= 2 ? unique : null;
}

function getSequentialChoiceState(id, prompts) {
  if (!prompts || prompts.length < 2) return null;
  const sig = sequentialChoiceSig(prompts);
  let state = sequentialChoiceCache.get(id);
  if (!state || state.sig !== sig) {
    state = { sig, prompts, answers: new Map(), index: 0 };
    sequentialChoiceCache.set(id, state);
  } else {
    state.prompts = prompts;
  }
  while (state.index < state.prompts.length && state.answers.has(state.prompts[state.index].key)) {
    state.index++;
  }
  return state.index < state.prompts.length ? state : null;
}

function sequentialChoiceOptionsForState(state) {
  if (!state) return [];
  const prompt = state.prompts[state.index];
  const question = `${prompt.key}: ${prompt.question}`;
  return prompt.options.map((opt, idx) => ({
    ...opt,
    isCurrent: idx === 0,
    _sequentialChoice: true,
    _sequentialKey: prompt.key,
    _sequentialQuestion: question,
  }));
}

function clearSequentialChoiceState(id) {
  sequentialChoiceCache.delete(id);
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
      if (bar) showActionBar(bar, id, cached, false, !wasVisible);
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

  const seqPrompts = extractSequentialChoicePrompts(multiQContext);
  const seqState = getSequentialChoiceState(id, seqPrompts);
  if (seqState) {
    const seqOpts = sequentialChoiceOptionsForState(seqState);
    approvalRawOptionsCache.set(id, seqOpts);
    scheduleApprovalHintConfirm(id, seqOpts);
    return;
  }
  if (!seqPrompts) clearSequentialChoiceState(id);

  // フォールバック検出（既存）
  let extraction = extractApprovalOptions(lines);
  const options = extraction.options;
  let contextSourceLines = lines;
  let contextCluster = extraction.cluster;

  // 非アクティブセッション含めて xterm の解釈済みバッファ (scanBuffer) も参照する。
  // Ink/Codex のカーソル位置制御による再描画は pendingTextTail を行分割しても
  // カーソル付き選択肢を取り出せないため、xterm 解釈済みのバッファのほうが
  // より正確に行構造とカーソル位置を保持している。
  // ただし scanBuffer は履歴を保持し続けるため、承認解決後の応答チャンクで
  // 古い選択肢が再検出される（approvalConsumedSig は label 差異で抑止が外れる）。
  // pendingTextTail に承認系の手がかりが無く、かつ既に visible でも無い場合は scanBuffer を見ない。
  const pendingHasApprovalHint = lines.some(line =>
    userSpecifiesRe.test(line) ||
    recommendedChoiceRe.test(line) ||
    matchProviderApprovalTrigger(provider, line) ||
    matchNativeApprovalTrigger(line));
  const visibleRows = t?.term?.rows || 40;
  const bufferTail = t && t.everAttached ? scanBuffer(id).slice(-Math.max(120, visibleRows + 60)) : [];
  const bufferHasApprovalHint = bufferTail.some(line =>
    userSpecifiesRe.test(line) ||
    recommendedChoiceRe.test(line) ||
    matchProviderApprovalTrigger(provider, line) ||
    matchNativeApprovalTrigger(line));
  const allowBufferFallback = approvalVisibleCache.get(id) || options.length > 0 || pendingHasApprovalHint || bufferHasApprovalHint;
  if (t && t.everAttached && allowBufferFallback) {
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
  const hasNativePromptHint = contextLines.some((line) => matchProviderApprovalTrigger(provider, line) || matchNativeApprovalTrigger(line));
  const isCodexShortcutMenu = provider === 'codex' && options.some(o => o._sendText) && hasNativePromptHint;
  const approvalNear = (hasCursorOption || isCodexShortcutMenu) &&
    ((hasApprovalLikeLabel && (hasUserSpecifies || contextLines.some((line) => matchProviderApprovalTrigger(provider, line) || matchNativeApprovalTrigger(line)))) || isHubChoice);
  const hasChoiceMenuHint = (hasCursorOption || isCodexShortcutMenu) && options.length > 0 && hasNativePromptHint;
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
    // approvalVisibleCache=true の間（= Hub marker / plainYesNo / フォールバックいずれかで
    // 既に承認 UI を表示中）は cache を保護する。Claude Code の thinking スピナー
    // ("Worked for Xs") 等で pendingTextTail がローテートし `(Y:1/N:0)` 行が末尾 20-40 行
    // から押し出されると、フォールバック検出が「実行内容: 1. ... 2. ..." 等の番号付き本文を
    // 拾うが承認ラベルが無いため nowVisible=false になる。ここで cache を削除すると
    // detectApproval の復元経路 (action-bar 消失防止) が動かず、action-bar が
    // 表示・非表示を高頻度で繰り返す（画面チカチカ）症状になる。
    // sendChoice / doSend / hideActionBar の経路では確実に cache.delete されるため
    // この保護は安全（解決済み承認の残留は起きない）。
    if (!approvalVisibleCache.get(id)) {
      approvalRawOptionsCache.delete(id);
    }
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

// バッチ承認の戻り値判定: 要素が { options: [...] } を持つ場合は複数質問形式。
function isBatchOptions(value) {
  return Array.isArray(value) && value.length > 0 &&
    value[0] && Array.isArray(value[0].options);
}

function _parseHubBlock(lines) {
  const text = lines.join('\n');
  if (hasYesNoApprovalMarker(text)) {
    return _yesNoApprovalOptions(_yesNoCtxFromText(text));
  }
  // セクション分割: 行頭が `<数字><空白><テキスト>` の行を見出し（質問）、
  // `<数字>.<空白><テキスト>` の行を選択肢として解釈する。
  // 見出しは「数字直後にピリオドが無い」点で選択肢と区別される。
  const sections = [];
  const looseOpts = [];
  let cur = null;
  const optionRe = /^(\d+)\.\s*(.+?)\s*$/;
  const headingRe = /^(\d+)\s+(.+?)\s*$/;
  for (const raw of lines) {
    const line = String(raw || '').trim();
    if (!line) continue;
    const om = line.match(optionRe);
    if (om) {
      const opt = { num: parseInt(om[1], 10), label: om[2].trim(), isCurrent: false };
      if (cur) cur.options.push(opt);
      else looseOpts.push(opt);
      continue;
    }
    const hm = line.match(headingRe);
    if (hm) {
      cur = { num: parseInt(hm[1], 10), title: hm[2].trim(), options: [] };
      sections.push(cur);
      continue;
    }
  }
  const filledSections = sections.filter(s => s.options.length > 0);
  if (filledSections.length >= 2) return filledSections;
  if (filledSections.length === 1) return filledSections[0].options;
  return looseOpts.length > 0 ? looseOpts : null;
}

function _yesNoApprovalOptions(ctxText) {
  const ctx = ctxText ? _approvalCtxHash(ctxText) : '';
  return [
    { num: 1, label: 'Yes (1)', isCurrent: true, preserveOrder: true, _ctx: ctx },
    { num: 0, label: 'No (0)', isCurrent: false, preserveOrder: true, _ctx: ctx },
  ];
}

function codexShortcutSendText(label) {
  const m = String(label || '').match(/\((y|p|n|esc|escape)\)\s*$/i);
  if (!m) return null;
  const key = m[1].toLowerCase();
  if (key === 'esc' || key === 'escape') return '\x1b';
  return key;
}

function buildApprovalOption(numText, labelText, isCurrent) {
  const label = String(labelText || '').trim()
    .replace(/\s{2,}.*$/, '')
    .replace(/\s*\d+\.\s*[A-Za-z].*$/, '')
    .trim();
  const opt = { num: parseInt(numText, 10), label, isCurrent: !!isCurrent };
  const sendText = codexShortcutSendText(label);
  if (sendText) opt._sendText = sendText;
  return opt;
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
    lower.includes('press enter to confirm') ||
    lower.includes('enter to select') ||
    lower.includes('↑/↓ to navigate') ||
    lower.includes('esc to cancel');
}

function extractApprovalOptions(tail) {
  const options = [];
  let clusterStart = -1;
  let clusterEnd = -1;
  let seenOption = false;
  let blankGap = 0;
  // Codex の長い選択肢ラベルがターミナル幅で wrap した場合、継続行は番号で始まらない
  // インデント付き非空行として残る。末尾走査でこれに当たると以前は break して
  // option 2 以降を取りこぼしていた（例: "2. Yes, and don't ask again ...yyyy-MM- / dd"), ...(p)"）。
  // 末尾走査で番号行より先に現れる継続行を pendingContinuation に積み、直後の番号行 label に結合する。
  let pendingContinuation = [];
  // 空行は Codex の選択肢間に挟まる Ink 再描画ノイズ対策で最大 4 行まで許容する。
  // 番号付きでもインデント継続行でもない非空行に当たったら即終端（無関係な箇条書き / 過去出力 /
  // コードブロック内の `1.` を吸い込んで approvalSig が揺らぎ、approvalConsumedSig の抑止が外れて
  // action-bar がチラつく事故を防ぐ）。
  const maxBlankGap = 4;
  // ドット必須にして `462           const m = ...` のような差分行番号を誤検出しないようにする。
  // `2.Yes` 形式（ドット直後にスペース無し）は `\s*` が 0 個マッチでカバーする。
  // 番号上限は 1〜99（実用的な承認メニューは ≤ 9 個。Codex 等で 2 桁が出てもカバー）。
  const consumeContinuations = (label) => {
    if (pendingContinuation.length === 0) return label;
    const suffix = pendingContinuation.slice().reverse().join(' ');
    pendingContinuation = [];
    return `${label} ${suffix}`.replace(/\s+/g, ' ').trim();
  };
  for (let i = tail.length - 1; i >= 0; i--) {
    const line = tail[i];
    const cm = line.match(/^\s*[>❯›❱]\s*(\d{1,2})\.\s*(.+?)\s*$/);
    if (cm) {
      options.unshift(buildApprovalOption(cm[1], consumeContinuations(cm[2]), true));
      if (clusterEnd === -1) clusterEnd = i;
      clusterStart = i;
      seenOption = true;
      blankGap = 0;
      continue;
    }
    const om = line.match(/^\s*(\d{1,2})\.\s*(.+?)\s*$/);
    if (om) {
      options.unshift(buildApprovalOption(om[1], consumeContinuations(om[2]), false));
      if (clusterEnd === -1) clusterEnd = i;
      clusterStart = i;
      seenOption = true;
      blankGap = 0;
      continue;
    }
    if (!seenOption) continue;
    if (!String(line || '').trim()) {
      blankGap++;
      if (blankGap > maxBlankGap) break;
      // 空行を挟んだら継続行扱いを切る（別クラスタに飛ばないようにする）。
      pendingContinuation = [];
      continue;
    }
    // 番号で始まらない非空行: 先頭インデントがあれば直前 option の wrap continuation とみなす。
    // インデント無しの行は無関係な過去出力なので break で打ち切る。
    if (blankGap === 0 && /^\s+\S/.test(line)) {
      pendingContinuation.push(line.trim());
      continue;
    }
    break;
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
      approvalRawOptionsCache.set(id, markerOpts);
      const wasVisible = !!approvalVisibleCache.get(id);
      showActionBar(bar, id, markerOpts, false, !wasVisible);
      if (!wasVisible) {
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
      approvalRawOptionsCache.set(id, plainYesNoOpts);
      const wasVisible = !!approvalVisibleCache.get(id);
      showActionBar(bar, id, plainYesNoOpts, false, !wasVisible);
      if (!wasVisible) {
        cancelApprovalHintConfirm(id);
        approvalVisibleCache.set(id, true);
        enqueueApprovalAutoSwitch(id);
        ws.send(JSON.stringify({ type: 'session_hint', session_id: id, approval_visible: true }));
      }
      return;
    }
  }

  const seqLines = (t?.pendingTextTail || '').split(/\r\n|\r|\n/).slice(-80).map(l => stripAnsi(l))
    .concat(scanBuffer(id).slice(-80));
  const seqPrompts = extractSequentialChoicePrompts(seqLines);
  const seqState = getSequentialChoiceState(id, seqPrompts);
  if (seqState) {
    const seqOpts = sequentialChoiceOptionsForState(seqState);
    approvalRawOptionsCache.set(id, seqOpts);
    const wasVisible = !!approvalVisibleCache.get(id);
    showActionBar(bar, id, seqOpts, false, !wasVisible);
    if (!wasVisible) {
      cancelApprovalHintConfirm(id);
      approvalVisibleCache.set(id, true);
      enqueueApprovalAutoSwitch(id);
      ws.send(JSON.stringify({ type: 'session_hint', session_id: id, approval_visible: true }));
    }
    return;
  }
  if (!seqPrompts) clearSequentialChoiceState(id);

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
  // ただし scanBuffer は履歴を保持し続けるため、承認解決後の応答チャンクで
  // 古い選択肢が再検出される（approvalConsumedSig は label 差異で抑止が外れる）。
  // pendingTextTail に承認系の手がかりが無く、かつ既に visible でも無い場合は scanBuffer を見ない。
  const pendingHasApprovalHint = tail.some(line =>
    userSpecifiesRe.test(line) ||
    recommendedChoiceRe.test(line) ||
    matchProviderApprovalTrigger(provider, line) ||
    matchNativeApprovalTrigger(line));
  const bufferHasApprovalHint = bufferTail.some(line =>
    userSpecifiesRe.test(line) ||
    recommendedChoiceRe.test(line) ||
    matchProviderApprovalTrigger(provider, line) ||
    matchNativeApprovalTrigger(line));
  const allowBufferFallback = approvalVisibleCache.get(id) || options.length > 0 || pendingHasApprovalHint || bufferHasApprovalHint;
  if (t && allowBufferFallback) {
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
  if (t && options.length >= 1 && !options.some(o => o.num === 1)) {
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
  const hasNativePromptHint = contextLines.some((line) => matchProviderApprovalTrigger(provider, line) || matchNativeApprovalTrigger(line));
  const isCodexShortcutMenu = provider === 'codex' && options.some(o => o._sendText) && hasNativePromptHint;
  const approvalNear = (hasApprovalLikeLabel &&
    (hasUserSpecifies || hasNativePromptHint)) || isHubChoice || isCodexShortcutMenu;
  const hasApproval = options.length > 0 && approvalNear && (hasCursorOption || isCodexShortcutMenu);
  const hasChoiceMenu = (hasCursorOption || isCodexShortcutMenu) && options.length > 0 && hasNativePromptHint;
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
  const wasVisibleBeforeShow = !!approvalVisibleCache.get(id);
  showActionBar(bar, id, hasPrompt ? options : [], crunch.found && !hasPrompt, hasPrompt && !wasVisibleBeforeShow);

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
  if (bar) { bar.classList.remove('visible', 'batch'); bar.innerHTML = ''; }
  // 差分スキップ用キャッシュをリセット（次回 showActionBar が同一シグネチャでも再描画されるように）
  lastActionBarRender.sessionId = null;
  lastActionBarRender.sig = null;
  actionBarFocusIdx = -1;
  batchFocusIdx = -1;
  if (id !== undefined) batchSelections.delete(id);
  if (id !== undefined) {
    cancelApprovalHintConfirm(id);
    clearSequentialChoiceState(id);
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
    if (wasVisible && id === activeSessionId) {
      const term = terminals.get(id);
      const shouldStickToBottom = !!(term && (term.autoScroll || isTerminalAtBottom(term)));
      if (shouldStickToBottom) scrollTerminalToBottomSoon(id);
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

function showActionBar(bar, sessionId, options, showExpand, forceStickToBottom = false) {
  if (isBatchOptions(options)) {
    showBatchActionBar(bar, sessionId, options, forceStickToBottom);
    return;
  }
  options = normalizeActionOptions(options);
  // 注意: 局所変数名 `t` は window.t（i18n 翻訳関数）と衝突するため使わない。
  // `term` にすることで本関数末尾の t('expand_btn') 等が正しく i18n を参照できる。
  const term = sessionId === activeSessionId ? terminals.get(sessionId) : null;
  const shouldStickToBottom = !!(term && (forceStickToBottom || term.autoScroll || isTerminalAtBottom(term)));

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
    if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
    return;
  }
  lastActionBarRender.sessionId = sessionId;
  lastActionBarRender.sig = sig;
  bar.innerHTML = '';
  // バッチ→単一質問の遷移で残留する .batch クラスと選択状態を取り除く（縦スタック CSS の誤適用と
  // 後続バッチへの古いセレクション持ち越しを防ぐ）。
  bar.classList.remove('batch');
  batchSelections.delete(sessionId);

  // "⚠ Approval needed" ラベル
  if (options.length > 0) {
    const label = document.createElement('span');
    label.className = 'action-bar-label';
    const sequentialQuestion = options.find(o => o && o._sequentialQuestion)?._sequentialQuestion;
    label.textContent = sequentialQuestion ? `⚠ ${sequentialQuestion}` : '⚠ Approval needed';
    if (sequentialQuestion) {
      label.classList.add('sequential-question-label');
      label.title = sequentialQuestion;
    }
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
    refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
  }
}

function showBatchActionBar(bar, sessionId, sections, forceStickToBottom = false) {
  const term = sessionId === activeSessionId ? terminals.get(sessionId) : null;
  const shouldStickToBottom = !!(term && (forceStickToBottom || term.autoScroll || isTerminalAtBottom(term)));

  // セクション数が変わったら選択状態をリセット（前回の sectionA→sectionB セレクションが残らないように）
  let selections = batchSelections.get(sessionId);
  if (!selections || selections.length !== sections.length) {
    selections = new Array(sections.length).fill(null);
    batchSelections.set(sessionId, selections);
    if (batchFocusIdx < 0 || batchFocusIdx >= sections.length) batchFocusIdx = 0;
  }

  const sig = JSON.stringify({
    s: sessionId,
    mode: 'batch',
    sects: sections.map(sec => ({
      n: sec.num,
      t: sec.title,
      o: (sec.options || []).map(o => ({ n: o.num, l: o.label, c: !!o.isCurrent })),
    })),
    sel: selections,
    f: batchFocusIdx,
    v: bar.classList.contains('visible'),
  });
  if (lastActionBarRender.sessionId === sessionId && lastActionBarRender.sig === sig) {
    if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
    return;
  }
  lastActionBarRender.sessionId = sessionId;
  lastActionBarRender.sig = sig;
  bar.innerHTML = '';
  bar.classList.add('batch');

  const label = document.createElement('span');
  label.className = 'action-bar-label';
  label.textContent = t('approval_batch_label', { n: sections.length });
  bar.appendChild(label);

  sections.forEach((sec, idx) => {
    const sectionEl = document.createElement('div');
    sectionEl.className = 'action-section';
    if (idx === batchFocusIdx) sectionEl.classList.add('focused');
    sectionEl.dataset.idx = String(idx);

    const titleEl = document.createElement('div');
    titleEl.className = 'action-section-title';
    titleEl.textContent = `${sec.num}. ${sec.title}`;
    titleEl.title = sec.title;
    sectionEl.appendChild(titleEl);

    const btnRow = document.createElement('div');
    btnRow.className = 'action-section-buttons';
    (sec.options || []).forEach((opt) => {
      const btn = document.createElement('button');
      let cls = 'action-btn batch-option';
      if (opt.isCurrent) cls += ' current';
      if (selections[idx] === opt.num) cls += ' selected';
      btn.className = cls;
      btn.textContent = `${opt.num}. ${opt.label}`;
      btn.title = `${opt.num}. ${opt.label}`;
      btn.onclick = (e) => {
        e.stopPropagation();
        selectBatchOption(sessionId, idx, opt.num);
      };
      btnRow.appendChild(btn);
    });
    sectionEl.appendChild(btnRow);
    bar.appendChild(sectionEl);
  });

  const footer = document.createElement('div');
  footer.className = 'action-bar-footer';

  const progress = document.createElement('span');
  progress.className = 'action-bar-progress';
  const done = selections.filter(v => v != null).length;
  progress.textContent = t('approval_batch_progress', { done, total: sections.length });
  footer.appendChild(progress);

  const submitBtn = document.createElement('button');
  submitBtn.className = 'action-submit-btn';
  submitBtn.textContent = t('approval_batch_submit');
  submitBtn.disabled = !selections.every(v => v != null);
  submitBtn.onclick = (e) => {
    e.stopPropagation();
    sendBatchChoices(sessionId);
  };
  footer.appendChild(submitBtn);

  const clearBtn = document.createElement('button');
  clearBtn.className = 'action-clear-btn';
  clearBtn.textContent = t('approval_batch_clear');
  clearBtn.onclick = (e) => {
    e.stopPropagation();
    clearBatchSelections(sessionId);
  };
  footer.appendChild(clearBtn);

  const closeBatchBtn = document.createElement('button');
  closeBatchBtn.className = 'action-dismiss-btn';
  closeBatchBtn.textContent = '✕';
  closeBatchBtn.title = t('dismiss_title');
  closeBatchBtn.onclick = (e) => {
    e.stopPropagation();
    hideActionBar(sessionId);
    approvalSuppressUntil.set(sessionId, Date.now() + 60000);
  };
  footer.appendChild(closeBatchBtn);

  bar.appendChild(footer);
  bar.classList.add('visible');
  if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
}

function selectBatchOption(sessionId, sectionIdx, optionNum) {
  const selections = batchSelections.get(sessionId);
  if (!selections) return;
  selections[sectionIdx] = optionNum;
  const cached = approvalRawOptionsCache.get(sessionId);
  if (isBatchOptions(cached)) {
    // 自動前進: 末尾セクションを選んだら -1（無効化）して Enter で送信可能にする
    batchFocusIdx = sectionIdx + 1 < cached.length ? sectionIdx + 1 : -1;
    const bar = document.getElementById('action-bar');
    if (bar) showBatchActionBar(bar, sessionId, cached);
  }
  setTimeout(() => inputEl.focus(), 0);
}

function clearBatchSelections(sessionId) {
  const cached = approvalRawOptionsCache.get(sessionId);
  if (!isBatchOptions(cached)) return;
  batchSelections.set(sessionId, new Array(cached.length).fill(null));
  batchFocusIdx = 0;
  const bar = document.getElementById('action-bar');
  if (bar) showBatchActionBar(bar, sessionId, cached);
  setTimeout(() => inputEl.focus(), 0);
}

function sendBatchChoices(sessionId) {
  const selections = batchSelections.get(sessionId);
  if (!selections || selections.length === 0 || selections.some(v => v == null)) return;
  const text = selections.join(' ');
  const prevOpts = approvalRawOptionsCache.get(sessionId);
  if (prevOpts) approvalConsumedSig.set(sessionId, approvalSig(prevOpts));
  sendText(sessionId, `${text}\r`);
  hideActionBar(sessionId);
  approvalSuppressUntil.set(sessionId, Date.now() + 2000);
  batchSelections.delete(sessionId);
  setTimeout(() => {
    detectApproval(sessionId);
    maybeAutoSwitchToNextApproval();
  }, 2050);
  setTimeout(() => inputEl.focus(), 0);
}

function isBatchActionBarVisible() {
  const bar = document.getElementById('action-bar');
  return !!(bar && bar.classList.contains('visible') && bar.classList.contains('batch'));
}

function moveBatchFocus(delta) {
  if (activeSessionId === null) return false;
  const cached = approvalRawOptionsCache.get(activeSessionId);
  if (!isBatchOptions(cached) || cached.length === 0) return false;
  const n = cached.length;
  const start = batchFocusIdx < 0 ? (delta > 0 ? -1 : n) : batchFocusIdx;
  batchFocusIdx = ((start + delta) % n + n) % n;
  const bar = document.getElementById('action-bar');
  if (bar) showBatchActionBar(bar, activeSessionId, cached);
  return true;
}

function handleBatchNumberKey(sessionId, num) {
  const cached = approvalRawOptionsCache.get(sessionId);
  if (!isBatchOptions(cached)) return false;
  if (batchFocusIdx < 0 || batchFocusIdx >= cached.length) return false;
  const section = cached[batchFocusIdx];
  if (!section) return false;
  const opt = (section.options || []).find(o => o.num === num);
  if (!opt) return false;
  selectBatchOption(sessionId, batchFocusIdx, num);
  return true;
}

function sendChoice(sessionId, targetNum) {
  const seqState = sequentialChoiceCache.get(sessionId);
  if (seqState && seqState.index < seqState.prompts.length) {
    const prompt = seqState.prompts[seqState.index];
    seqState.answers.set(prompt.key, targetNum);
    seqState.index++;
    while (seqState.index < seqState.prompts.length && seqState.answers.has(seqState.prompts[seqState.index].key)) {
      seqState.index++;
    }

    const bar = document.getElementById('action-bar');
    if (seqState.index < seqState.prompts.length && bar) {
      const nextOpts = sequentialChoiceOptionsForState(seqState);
      approvalRawOptionsCache.set(sessionId, nextOpts);
      showActionBar(bar, sessionId, nextOpts, false);
      setTimeout(() => inputEl.focus(), 0);
      return;
    }

    const response = seqState.prompts
      .map(p => `${p.key}: ${seqState.answers.get(p.key)}`)
      .join('\n') + '\r';
    clearSequentialChoiceState(sessionId);
    const prevOpts = approvalRawOptionsCache.get(sessionId);
    if (prevOpts) approvalConsumedSig.set(sessionId, approvalSig(prevOpts));
    sendText(sessionId, response);
    hideActionBar(sessionId);
    approvalSuppressUntil.set(sessionId, Date.now() + 2000);
    setTimeout(() => {
      detectApproval(sessionId);
      maybeAutoSwitchToNextApproval();
    }, 2050);
    setTimeout(() => inputEl.focus(), 0);
    return;
  }

  // 矢印移動ではなく番号直接入力で確定する（誤選択防止）
  const cachedOpts = approvalRawOptionsCache.get(sessionId);
  const targetOpt = Array.isArray(cachedOpts) && !isBatchOptions(cachedOpts)
    ? cachedOpts.find(o => o && o.num === targetNum)
    : null;
  const choiceText = targetOpt && targetOpt._sendText ? targetOpt._sendText : `${targetNum}\r`;
  sendText(sessionId, choiceText);
  // doSend と同様に消費済み署名を記録（Ink 再描画による同一ブロックの再検出・再表示を防ぐ）
  const prevOpts = cachedOpts;
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
  autoExpand(); updateSlashMenu();
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
    if (isLeft) {
      wrap.append(btn, inputTools);
      const voiceBtn = document.getElementById('voice-btn');
      const sendBtn = document.getElementById('send-btn');
      if (voiceBtn) wrap.append(voiceBtn);
      if (sendBtn) wrap.append(sendBtn);
      wrap.append(inputArea);
    } else {
      const voiceBtn = document.getElementById('voice-btn');
      const sendBtn = document.getElementById('send-btn');
      wrap.append(inputArea);
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
  batchSelections.delete(id);
  clearSequentialChoiceState(id);
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
  restoreInputStateFor(id);
  // files/git 表示からセッションカードへ戻る場合、先にターミナルを表示してから
  // attach/fit/detect しないと、承認 UI 検出と最下部スナップが hidden レイアウトを基準に走る。
  FilesTabManager.switchToSessionView();
  ensureTerminal(id);
  attachTerminal(id);
  updateScrollLockBtn();
  setMultiQuestionBannerVisible(!!multiQuestionVisibleCache.get(id));
  detectApproval(id);
  renderSessionList();
  renderToolOutputs(id);
  updateShellBadge(id);
  updateQuickCmdButtons(id);
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

function providerIconHtml(provider) {
  const base = `class="card-provider-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"`;
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
  const spawnModelDatalist = document.getElementById('spawn-model-datalist');
  const spawnModelClearBtn = document.getElementById('spawn-model-clear');
  const spawnModelRefreshBtn = document.getElementById('spawn-model-refresh');
  let codexModelSelection = null;
  let claudeModelSelection = null;

  // /api/models から取得した groups の最新キャッシュ。
  // populateModelDatalist と resolveRoute で共有する。
  let spawnModelGroups = null;
  // model id → route の即時参照 Map。
  const spawnModelRouteMap = new Map();
  let spawnModelFetchInFlight = null;

  function rebuildModelRouteMap(groups) {
    spawnModelRouteMap.clear();
    if (!Array.isArray(groups)) return;
    for (const g of groups) {
      if (!g || !Array.isArray(g.models)) continue;
      for (const m of g.models) {
        if (m && m.id) spawnModelRouteMap.set(m.id, g.route || '');
      }
    }
  }

  function getModelGroupsForProvider(provider) {
    if (!Array.isArray(spawnModelGroups)) return [];
    const groups = spawnModelGroups.filter(g => g && Array.isArray(g.models) && (!g.provider || g.provider === provider));
    groups.sort((a, b) => {
      const rank = (g) => {
        if (g.provider === provider) return 0;
        if (g.label === 'Ollama Cloud') return 1;
        if (g.label === 'Ollama Local') return 2;
        return 3;
      };
      return rank(a) - rank(b);
    });
    return groups;
  }

  function groupHasModel(group, model) {
    return !!group?.models?.some(m => m && m.id === model);
  }

  function isModelCompatibleWithProvider(provider, model) {
    const m = (model || '').trim();
    if (!m || !Array.isArray(spawnModelGroups)) return true;
    let known = false;
    for (const g of spawnModelGroups) {
      if (!groupHasModel(g, m)) continue;
      known = true;
      if (!g.provider || g.provider === provider) return true;
    }
    return !known;
  }

  function clearModelSelectionState() {
    codexModelSelection = null;
    claudeModelSelection = null;
  }

  function syncModelClearButton() {
    if (spawnModelClearBtn) spawnModelClearBtn.hidden = !spawnModelInput.value.trim();
  }

  function setSpawnModelValue(value) {
    spawnModelInput.value = value || '';
    syncModelClearButton();
  }

  function clearIncompatibleModelForProvider(provider) {
    if (!isModelCompatibleWithProvider(provider, spawnModelInput.value)) {
      setSpawnModelValue('');
      clearModelSelectionState();
    }
  }

  // dialog open 時、復元された model が Ollama route なら空にする。
  // 残しておくとそのまま spawn 実行で env 焼き付け → /model blocked の罠を踏むため、
  // Ollama は毎回明示的に選び直す運用に倒す（saveSpawnSettings 側でも保存しない）。
  function clearOllamaModelDefault() {
    const m = spawnModelInput.value.trim();
    if (!m) return;
    if (resolveRoute(spawnProviderEl.value, m) === 'ollama') {
      setSpawnModelValue('');
      clearModelSelectionState();
    }
  }

  function populateModelDatalist() {
    if (!spawnModelDatalist) return;
    spawnModelDatalist.innerHTML = '';
    if (!Array.isArray(spawnModelGroups)) return;
    const currentProvider = spawnProviderEl.value;
    // 並び順: 同 provider 専用 → Ollama Cloud → Ollama Local。
    // 他 provider 専用は非表示。Ollama 系は provider="" で両 provider に表示する。
    for (const g of getModelGroupsForProvider(currentProvider)) {
      for (const m of g.models) {
        const opt = document.createElement('option');
        opt.value = m.id;
        // <option label> はブラウザ実装差があるため、フォールバックとして
        // text content にも同じ表記を入れる。
        const label = `[${g.label}] ${m.label || m.id}`;
        opt.setAttribute('label', label);
        opt.textContent = label;
        opt.dataset.route = g.route || '';
        spawnModelDatalist.appendChild(opt);
      }
    }
  }

  function resolveRoute(provider, model) {
    const m = (model || '').trim();
    if (!m) return '';
    for (const g of getModelGroupsForProvider(provider)) {
      if (groupHasModel(g, m)) return g.route || '';
    }
    if (spawnModelRouteMap.has(m) && isModelCompatibleWithProvider(provider, m)) return spawnModelRouteMap.get(m);
    if (m.includes(':cloud')) return 'ollama';
    if (provider === 'claude') return 'anthropic';
    if (provider === 'codex')  return 'openai';
    return '';
  }

  async function fetchModelGroups(force) {
    if (spawnModelFetchInFlight) return spawnModelFetchInFlight;
    const method = force ? 'POST' : 'GET';
    const url = `/api/models?token=${token}`;
    const p = (async () => {
      try {
        const res = await fetch(url, { method });
        if (!res.ok) throw new Error('HTTP ' + res.status);
        const data = await res.json();
        spawnModelGroups = Array.isArray(data.groups) ? data.groups : [];
        rebuildModelRouteMap(spawnModelGroups);
        populateModelDatalist();
        clearIncompatibleModelForProvider(spawnProviderEl.value);
        clearOllamaModelDefault();
        return data;
      } finally {
        spawnModelFetchInFlight = null;
      }
    })();
    spawnModelFetchInFlight = p;
    return p;
  }

  if (spawnModelRefreshBtn) {
    spawnModelRefreshBtn.addEventListener('click', async () => {
      spawnModelRefreshBtn.classList.add('is-loading');
      try {
        await fetchModelGroups(true);
      } catch (_) {
        alert(t('spawn_model_fetch_failed'));
      } finally {
        spawnModelRefreshBtn.classList.remove('is-loading');
      }
    });
  }

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
    populateModelDatalist();
    clearIncompatibleModelForProvider(p);
  });
  updateSpawnProviderIcon();

  // フォーカス時に入力値を一時クリアして datalist の全候補を表示し、
  // 未選択のまま離れたら元の値を復元する。
  let _savedModelValue = '';
  let _modelInputDirty = false;
  spawnModelInput.addEventListener('focus', () => {
    _savedModelValue = spawnModelInput.value;
    _modelInputDirty = false;
    spawnModelInput.value = '';
  });
  spawnModelInput.addEventListener('input', () => {
    _modelInputDirty = true;
    clearModelSelectionState();
    syncModelClearButton();
  });
  spawnModelInput.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      _modelInputDirty = false;
      setSpawnModelValue(_savedModelValue);
      spawnModelInput.blur();
    }
  });
  spawnModelInput.addEventListener('blur', () => {
    if (!_modelInputDirty) {
      setSpawnModelValue(_savedModelValue);
    }
  });
  if (spawnModelClearBtn) {
    spawnModelClearBtn.addEventListener('click', () => {
      setSpawnModelValue('');
      clearModelSelectionState();
      spawnModelInput.focus();
    });
  }

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
      if (s.model !== undefined) setSpawnModelValue(s.model);
      if (s.permission_mode)  document.getElementById('spawn-permission-mode').value = s.permission_mode;
      if (s.sandbox)          document.getElementById('spawn-sandbox').value = s.sandbox;
      if (s.ask_for_approval) document.getElementById('spawn-ask-approval').value = s.ask_for_approval;
      return !!s.cwd;
    } catch (_) { return false; }
  }

  function saveSpawnSettings(obj) {
    setUserPref('spawn.defaults', obj);
  }

  function loadCwdHistory() {
    try { return JSON.parse(localStorage.getItem(STORAGE_CWD_HISTORY_KEY) || '[]'); } catch (_) { return []; }
  }

  function saveCwdHistory(cwd) {
    if (!cwd) return;
    const hist = loadCwdHistory().filter(v => v !== cwd);
    hist.unshift(cwd);
    if (hist.length > CWD_HISTORY_MAX) hist.length = CWD_HISTORY_MAX;
    setUserPref('cwd_history', hist);
  }

  function deleteCwdHistoryItem(cwd) {
    const hist = loadCwdHistory().filter(v => v !== cwd);
    setUserPref('cwd_history', hist);
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
    // モデル一覧を初回または stale なら裏で取得して datalist を埋める。
    // 失敗しても UI 起動はブロックしない（手入力で従来通り）。
    if (!spawnModelGroups) {
      fetchModelGroups(false).catch(() => {});
    } else {
      populateModelDatalist();
      clearOllamaModelDefault();
    }
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
        } else {
          // Non-2xx (typically 500 when no native folder picker is available,
          // e.g. Linux without zenity/kdialog). Surface the server message so
          // the click isn't silently ignored.
          let msg = '';
          try { msg = (await res.text()).trim(); } catch (_) {}
          showToast(msg ? `${t('link_open_error')}: ${msg}` : t('link_open_error'));
        }
      } catch (_) { showToast(t('link_open_error')); }
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
        <input class="model-picker-input" id="model-candidate-input" type="text" list="spawn-model-datalist" value="${escapeHtml(currentModel || '')}">
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
    populateModelDatalist();
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
    populateModelDatalist();
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
      setSpawnModelValue(picked.model);
      codexModelSelection = picked;
    });
  }

  if (spawnClaudeModelBtn) {
    spawnClaudeModelBtn.addEventListener('click', async () => {
      const picked = await openClaudeModelModal();
      if (!picked) return;
      setSpawnModelValue(picked.model);
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
      if (model && !isModelCompatibleWithProvider(provider, model)) {
        setSpawnModelValue('');
        clearModelSelectionState();
        showToast(t('spawn_model_provider_mismatch'));
        spawnLaunchBtn.disabled = false;
        return;
      }
      const route = resolveRoute(provider, model);
      const bodyObj = { provider, cwd, model, label };
      if (route) bodyObj.route = route;
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
        // Ollama route のモデルは default として保存しない。
        // 残すと次回 spawn dialog で Ollama モデルが pre-fill されたまま起動 →
        // spawn 時 env (ANTHROPIC_BASE_URL=localhost:11434 等) が焼き付き、
        // そのセッション内で /model が blocked になる罠を踏むため。
        // Claude/Codex の純正モデル選択は引き続き sticky に残す。
        const persistedModel = route === 'ollama' ? '' : model;
        saveSpawnSettings({
          provider,
          cwd,
          model: persistedModel,
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

  // Chrome の SpeechRecognition は 'aborted' などで内部 stuck 状態に陥ると、
  // 同じインスタンスへの .start() / .abort() / .stop() では復旧不可能になる。
  // 復旧手段は new SpeechRecognition() で作り直すしかないため、
  // リスナーを配列に控えておいて再アタッチできるようにしておく。
  let recognition;
  const _recognitionListeners = [];
  function _onRecognition(eventName, handler) {
    _recognitionListeners.push([eventName, handler]);
    recognition.addEventListener(eventName, handler);
  }
  function _configureRecognition(rec) {
    rec.interimResults = true;
    rec.continuous = false;
    rec.maxAlternatives = 1;
    rec.lang = getLang();
  }
  function _recreateRecognition() {
    const oldRecognition = recognition;
    recognition = new SpeechRecognition();
    _configureRecognition(recognition);
    for (const [name, fn] of _recognitionListeners) {
      recognition.addEventListener(name, fn);
    }
    try { oldRecognition.abort(); } catch (_) {}
  }
  function _isCurrentRecognitionEvent(e) {
    return !e || !e.currentTarget || e.currentTarget === recognition;
  }
  recognition = new SpeechRecognition();
  _configureRecognition(recognition);

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

  // ---- no-result watchdog ----
  // audiostart/soundstart/speechstart は届くのに result イベントが一向に来ない
  // (= hw 等の別 SpeechRecognition と mic を奪い合って分配済み) 状態を検出して
  // 自動復旧する。recreate-5 まで「次回のため hw を fresh に保つ」対策は入れたが、
  // 今まさに stuck な録音セッションを救う経路が無く、ユーザーが ✕/✓ を押すまで
  // 波形だけが残る状況が再発していた。
  const NO_RESULT_WATCHDOG_MS = 4500;
  let noResultWatchdog = null;
  function armNoResultWatchdog() {
    clearTimeout(noResultWatchdog);
    noResultWatchdog = setTimeout(() => {
      noResultWatchdog = null;
      console.warn('SpeechRecognition: audio active but no result for', NO_RESULT_WATCHDOG_MS, 'ms — forcing recovery');
      userIntendedStop = true;
      clearSilenceTimer();
      clearRestartTimer();
      _recreateRecognition();
      forceCleanup();
      showToast(t('voice_error_no_result'), btn);
    }, NO_RESULT_WATCHDOG_MS);
  }
  function clearNoResultWatchdog() {
    if (noResultWatchdog) {
      clearTimeout(noResultWatchdog);
      noResultWatchdog = null;
    }
  }

  let forceCleanupTimer = null;
  function forceCleanup() {
    clearTimeout(forceCleanupTimer);
    forceCleanupTimer = null;
    userIntendedStop = true;
    voiceIntent = false;
    clearSilenceTimer();
    clearRestartTimer();
    clearBeginRetryTimer();
    clearNoResultWatchdog();
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
      // 既存テキストの末尾が空白/改行でない場合、認識結果との連結を防ぐため
      // 半角スペースを 1 つ挟む。preVoiceText 自体は変えずキャンセル復元に使う。
      if (preVoiceText.length > 0 && !/\s$/.test(preVoiceText)) {
        inputEl.value = preVoiceText + ' ';
      }
      interimStart = inputEl.value.length;
    }
    recognition.lang = getLang();
    isStarting = true;
    try {
      recognition.start();
    } catch (err) {
      isStarting = false;
      // InvalidStateError: SpeechRecognition インスタンスが内部 stuck 状態
      // （直前セッションの 'aborted' で Chrome 側の state が "started" のまま固着、等）。
      // 過去は abort+バックオフで凌ごうとしていたが、stuck 状態の inst には abort が効かず
      // 何度リトライしても同じエラーになる。インスタンス作り直しが唯一の復旧手段。
      const isInvalidState = err && (err.name === 'InvalidStateError' || /already started/i.test(err.message || ''));
      if (isInvalidState && retryCount < 2) {
        _recreateRecognition();
        const delay = 150;
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
      // stuck 状態だと recognition.abort() が無視されて 'end' イベントも来ないので、
      // scheduleForceCleanup (1500ms 後) ではユーザーから「ボタンが効かない」ように見える。
      // 即時 forceCleanup + 次回起動時の保険として _recreateRecognition を呼ぶ。
      try { recognition.abort(); } catch (_) {}
      _recreateRecognition();
      forceCleanup();
      return;
    }
    // hw.abort() で hw.end が遅延発火しても再起動を抑止するため、abort 前にフラグを立てる
    voiceIntent = true;
    // 録音開始のたびに recognition を fresh インスタンスに差し替える。
    // recreate-5 までは hw 側だけ毎回作り直していたが、recognition 側を使い回していたため
    // Chrome 内部で半固着した状態 (=「.start() は通るが result が届かない」) を持ち越し、
    // 「波形は出るがテキストが入らない」症状の根本原因として残っていた。
    // hw 側と同じく毎回 new SpeechRecognition() で開始する。
    _recreateRecognition();
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
    inputEl.value = preVoiceText;
    autoExpand();
    // stuck 状態だと abort が無視されて 'end' イベントが届かず、
    // scheduleForceCleanup の 1500ms 後の条件チェックでしか voice-bar が消えない (= UX 上「ボタン無反応」に見える)。
    // インスタンスを作り直して stuck を持ち越さず、即時に forceCleanup で voice-bar を畳む。
    try { recognition.abort(); } catch (_) {}
    _recreateRecognition();
    forceCleanup();
  });

  confirmBtn.addEventListener('click', () => {
    // stuck 状態では stop() も無視されるため、即時 forceCleanup で voice-bar を畳む。
    // 入力欄には interim 結果がそのまま残るので「確定」相当の挙動になる。
    try { recognition.stop(); } catch (_) {}
    _recreateRecognition();
    forceCleanup();
  });

  _onRecognition('start', (e) => {
    if (!_isCurrentRecognitionEvent(e)) return;
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

  _onRecognition('audiostart', (e) => {
    if (!_isCurrentRecognitionEvent(e)) return;
    setVoiceAudioActive(true);
    voiceIntensityTarget = 0.15;
    // audiostart は無音テストでも発火するため、ここで watchdog を arm すると
    // 「黙っていただけ」でも 4.5 秒後にマイク取り合い疑いトーストが誤発火する。
    // soundstart / speechstart（実際に音や発話が検出されたとき）でだけ arm する。
  });
  _onRecognition('soundstart', (e) => {
    if (!_isCurrentRecognitionEvent(e)) return;
    voiceIntensityTarget = 0.55;
    lastKickAt = performance.now();
    armNoResultWatchdog();
  });
  _onRecognition('speechstart', (e) => {
    if (!_isCurrentRecognitionEvent(e)) return;
    voiceIntensityTarget = 0.9;
    lastKickAt = performance.now();
    armNoResultWatchdog();
  });
  _onRecognition('speechend', (e) => {
    if (!_isCurrentRecognitionEvent(e)) return;
    voiceIntensityTarget = 0.25;
  });
  _onRecognition('soundend', (e) => {
    if (!_isCurrentRecognitionEvent(e)) return;
    voiceIntensityTarget = 0.08;
  });
  _onRecognition('audioend', (e) => {
    if (!_isCurrentRecognitionEvent(e)) return;
    setVoiceAudioActive(false);
    voiceIntensityTarget = 0.03;
    clearNoResultWatchdog();
  });

  _onRecognition('result', (e) => {
    if (!_isCurrentRecognitionEvent(e)) return;
    clearNoResultWatchdog();
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

  _onRecognition('end', (e) => {
    if (!_isCurrentRecognitionEvent(e)) return;
    cancelForceCleanup();
    clearNoResultWatchdog();
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

  _onRecognition('error', (e) => {
    if (!_isCurrentRecognitionEvent(e)) return;
    console.warn('SpeechRecognition error:', e.error, e.message || '');
    clearNoResultWatchdog();
    isStarting = false;
    setVoiceAudioActive(false);
    // 'aborted' は Chrome が 'end' を返さないことがあるため、その場で終了扱いにする。
    // 同時にインスタンスを作り直し、次回起動へ stuck 状態を持ち越さない。
    if (e.error === 'aborted') {
      userIntendedStop = true;
      clearSilenceTimer();
      clearRestartTimer();
      _recreateRecognition();
      forceCleanup();
      return;
    }
    // 致命系エラーは auto-restart 抑止 (繰り返してもまた失敗するため)。
    // 'no-speech' はソフトエラーで、'end' イベントが判定する。
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
  // ウェイクワード機能は無効化中（UI も非表示）。
  // recreate-1〜7 で繰り返した stuck の発火点はほぼ全て hw (このウェイクワード SR インスタンス) 側だった。
  // 復活が必要なら本 return と index.html / isWakewordEnabled() の改修を併せて戻す。
  return;
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
    return (localStorage.getItem(STORAGE_WAKE_WORD_PHRASE_KEY) ?? getDefaultWakeWordPhrase()).trim();
  }

  function isWakewordEnabled() {
    return localStorage.getItem(STORAGE_WAKE_WORD_ENABLED_KEY) === '1';
  }

  function getLang() {
    const lang = localStorage.getItem(STORAGE_LANG_KEY) || 'ja';
    return lang === 'ja' ? 'ja-JP' : 'en-US';
  }

  // hw も recognition と同じく `'aborted'` 後に内部 state が "started" のまま固着し、
  // abort() / stop() が無視される stuck 状態に陥ることがある。stuck だと hw.end が
  // 来ない → stopHotwordForVoiceInput が 500ms セーフティで強制 resolve → そのまま
  // recognition.start() が走り、hw がマイクを掴んだままなので audiostart は来るが
  // result が届かない（「波形は動くがテキストが入らない」二次再発の根本原因）。
  // recognition 側と同じく差し替え可能にし、stuck を疑う経路で破棄する。
  let hw;
  const _hotwordListeners = [];
  function _onHotword(eventName, handler) {
    _hotwordListeners.push([eventName, handler]);
    hw.addEventListener(eventName, handler);
  }
  function _configureHotword(rec) {
    rec.interimResults = true;
    rec.continuous = false;
    rec.maxAlternatives = 1;
  }
  function _recreateHotword() {
    const oldHotword = hw;
    hw = new SpeechRecognition();
    _configureHotword(hw);
    for (const [name, fn] of _hotwordListeners) {
      hw.addEventListener(name, fn);
    }
    try { oldHotword.abort(); } catch (_) {}
  }
  function _isCurrentHotwordEvent(e) {
    return !e || !e.currentTarget || e.currentTarget === hw;
  }
  hw = new SpeechRecognition();
  _configureHotword(hw);

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
      // 早期 return 経路でも hw を必ず作り直す。
      // hw.error: 'aborted' などで isListening/isStarting が false に戻った直後でも、
      // Chrome 内部の SpeechRecognition は state="started" のまま固着している可能性があり、
      // その状態で recognition.start() しても audiostart は通るが result が届かない。
      _recreateHotword();
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
        // end が来たケースでも hw を作り直す。end が届いた = mic 解放完了 とは限らず、
        // Chrome 内部で半解放状態のまま次の recognition.start() に持ち越すと
        // 「波形は出るがテキストが入らない」症状が再発するため、毎回 fresh な hw を用意する。
        _recreateHotword();
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

  _onHotword('start', (e) => {
    if (!_isCurrentHotwordEvent(e)) return;
    isStarting = false;
    isListening = true;
    updateMicChip();
  });

  _onHotword('result', (e) => {
    if (!_isCurrentHotwordEvent(e)) return;
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

  _onHotword('end', (e) => {
    if (!_isCurrentHotwordEvent(e)) return;
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

  _onHotword('error', (e) => {
    if (!_isCurrentHotwordEvent(e)) return;
    isStarting = false;
    // 'aborted' は Chrome 側で SpeechRecognition が stuck になる代表的な起点。
    // abort() / stop() 後の追い掛けや、recognition.start() が mic を奪った結果として発生し、
    // この後 'end' が届かないケースがある。stuck な hw が mic を掴んだままになると
    // 直後の recognition.start() で「波形は出るが result が届かない」症状になるため、
    // 即座にインスタンスを捨てる。次回 startHotword() は fresh な hw で開始される。
    if (e.error === 'aborted') {
      isListening = false;
      _recreateHotword();
      updateMicChip();
      return;
    }
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
      openGitTab: () => null,
      closeFilesTab: () => {},
      closeMainTab: () => {},
      switchToSessionView: () => {},
      updateSessionTabLabel: () => {},
      restoreFromLocalStorage: () => {},
      hasGitTabForRoot: () => false,
      hasFilesTabForRoot: () => false,
      onSessionRemoved: () => {},
      getSessionsRef: () => null,
      setSessionsRef: () => {},
    };
  }

  // タブデータ構造: { id, type/kind: 'session'|'files'|'git', label, sessionId?, filesRoot?, gitRoot?, viewRef?, projectName?, el, contentEl }
  // - kind は新エイリアス（'session'|'files'|'git'）。type は後方互換で残置（'files' のみ既存ロジック互換に使う）。
  let tabs = [];
  let activeTabId = null;
  let sessionTabEl = null;  // セッションタブ（常に1枚）

  // 外部から渡される sessions(Map) への参照（restore / 付け替えで使う）。
  // app.js 上部の `let sessions = new Map()` を直接参照できないので setter で受け取る。
  let sessionsRef = null;
  function setSessionsRef(map) { sessionsRef = map; }
  function getSessionsRef() { return sessionsRef; }

  // ─── LS 読み書き（schema v2: { v:2, files: {gitRoot:[...]}, git: [...] }） ─────
  function lsLoadRaw() {
    try { return JSON.parse(localStorage.getItem(LS_KEY) || '{}'); } catch (_) { return {}; }
  }
  function lsLoad() {
    const raw = lsLoadRaw();
    // v1（旧形式: gitRoot→entries[] のフラット map）→ v2 への一度きり migration
    if (!raw || typeof raw !== 'object') return { v: 2, files: {}, git: [] };
    if (raw.v === 2 && raw.files && Array.isArray(raw.git)) return raw;
    // v1 とみなす（{gitRoot: [{root, openedFile}, ...], ...} 直下に gitRoot キー）
    const files = {};
    for (const [k, v] of Object.entries(raw)) {
      if (k === 'v' || k === 'files' || k === 'git') continue;
      if (Array.isArray(v)) files[k] = v;
    }
    const migrated = { v: 2, files, git: [] };
    try { localStorage.setItem(LS_KEY, JSON.stringify(migrated)); } catch (_) {}
    return migrated;
  }
  function lsSave(data) {
    try { localStorage.setItem(LS_KEY, JSON.stringify(data)); } catch (_) {}
  }
  function lsAddTab(gitRoot, filesRoot) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!data.files[gitRoot]) data.files[gitRoot] = [];
    // 同じ root が既にある場合は重複しない
    if (!data.files[gitRoot].some(e => e.root === filesRoot)) {
      data.files[gitRoot].push({ root: filesRoot, openedFile: null });
    }
    lsSave(data);
  }
  function lsRemoveTab(gitRoot, filesRoot) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!data.files[gitRoot]) return;
    data.files[gitRoot] = data.files[gitRoot].filter(e => e.root !== filesRoot);
    if (data.files[gitRoot].length === 0) delete data.files[gitRoot];
    lsSave(data);
  }
  /** ファイル選択時に呼ぶ用の公開関数 */
  function lsUpdateOpenedFile(gitRoot, filesRoot, filePath) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!data.files[gitRoot]) return;
    const entry = data.files[gitRoot].find(e => e.root === filesRoot);
    if (entry) { entry.openedFile = filePath; lsSave(data); }
  }

  // ─── git タブ用 LS ─────────────────────────────────────────────────────
  function lsAddGitTab(gitRoot, projectName, viewRef) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!Array.isArray(data.git)) data.git = [];
    const existing = data.git.find(e => e.gitRoot === gitRoot);
    if (existing) {
      existing.projectName = projectName || existing.projectName;
      existing.viewRef = viewRef || existing.viewRef || '';
    } else {
      data.git.push({ gitRoot, projectName: projectName || '', viewRef: viewRef || '' });
    }
    lsSave(data);
  }
  function lsRemoveGitTab(gitRoot) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!Array.isArray(data.git)) return;
    data.git = data.git.filter(e => e.gitRoot !== gitRoot);
    lsSave(data);
  }
  function lsUpdateGitViewRef(gitRoot, viewRef) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!Array.isArray(data.git)) return;
    const entry = data.git.find(e => e.gitRoot === gitRoot);
    if (entry) { entry.viewRef = viewRef || ''; lsSave(data); }
  }

  // ─── DOM ──────────────────────────────────────────────────────────────
  function makeid() { return 'dtab-' + Math.random().toString(36).slice(2, 9); }
  function currentSessionId() {
    return (typeof activeSessionId !== 'undefined') ? activeSessionId : null;
  }
  function sameSessionId(a, b) {
    if (a == null || b == null) return a == null && b == null;
    return String(a) === String(b);
  }
  function sessionTabPrefix(sessionId) {
    return sessionId != null ? `#${sessionId} ` : '';
  }
  function updateTabLabelPrefix(tab, newSid) {
    if (!tab || !tab.labelEl) return;
    const cur = tab.labelEl.textContent || '';
    const stripped = cur.replace(/^#\d+\s+/, '');
    tab.labelEl.textContent = sessionTabPrefix(newSid) + stripped;
  }
  // タブの可視性判定:
  // - 同一セッションのタブは常に可視
  // - プロジェクトキーが取れる場合は、現セッションと同じプロジェクトに属するタブも可視
  //   （プロジェクト単位で files/git タブを共有するため）
  // - プロジェクトが '__no_project__' / 空 のセッション同士はプロジェクト共有しない
  function isVisibleInCurrentSession(tab) {
    const curSid = currentSessionId();
    if (sameSessionId(tab.sessionId, curSid)) return true;
    const tabPk = tab.projectKey;
    if (!tabPk || tabPk === '__no_project__') return false;
    if (curSid == null || !sessionsRef) return false;
    const curSess = sessionsRef.get(curSid);
    if (!curSess) return false;
    const curPk = curSess.project || '';
    if (!curPk || curPk === '__no_project__') return false;
    return tabPk === curPk;
  }
  function refreshVisibleTabs() {
    tabs.forEach(tab => {
      const visible = isVisibleInCurrentSession(tab);
      if (tab.el) tab.el.hidden = !visible;
      if (tab.contentEl && !visible) tab.contentEl.classList.remove('active');
    });
    const active = tabs.find(tab => tab.id === activeTabId);
    if (active && !isVisibleInCurrentSession(active)) {
      activeTabId = 'session';
    }
  }

  function ensureSessionTab() {
    if (sessionTabEl) return;
    sessionTabEl = document.createElement('button');
    sessionTabEl.className = 'main-tab main-tab-session active';
    sessionTabEl.dataset.tabId = 'session';
    sessionTabEl.textContent = t('files_tab_session_label');
    sessionTabEl.addEventListener('click', () => switchToSessionView());
    tabList.insertBefore(sessionTabEl, tabList.firstChild);
    ensureAddTabButton();
    activeTabId = 'session';
  }

  function placeAddTabButtonAfterSessionTab() {
    if (!addTabBtn || !sessionTabEl || sessionTabEl.parentNode !== tabList) return;
    if (addTabBtn.parentNode !== tabList || addTabBtn.previousSibling !== sessionTabEl) {
      tabList.insertBefore(addTabBtn, sessionTabEl.nextSibling);
    }
  }

  function showTabBar() {
    tabBar.style.display = '';
  }

  function setActive(tabId) {
    refreshVisibleTabs();
    const targetTab = tabs.find(tab => tab.id === tabId);
    if (targetTab && !isVisibleInCurrentSession(targetTab)) {
      tabId = 'session';
    }
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
      filesContents.querySelectorAll('.files-tab-content, .git-tab-content').forEach(el => el.classList.remove('active'));
      // display:none で隠れていた間に ResizeObserver が 0 幅で fit() を呼んでいる可能性があるため、
      // 表示復帰後にレイアウト確定を待って refit する。これをしないと xterm の cols が極小のまま残り、
      // 文字が縦に細く折り返される（depth padding 修正と同系統の "MD で出した narrow 表示" 事象）。
      if (typeof refitActiveTerminalAfterLayout === 'function') {
        refitActiveTerminalAfterLayout(true);
      }
    } else {
      terminalWrapper.style.display = 'none';
      filesContents.classList.add('visible');
      filesContents.querySelectorAll('.files-tab-content, .git-tab-content').forEach(el => {
        el.classList.toggle('active', el.dataset.tabId === tabId);
      });
    }
  }

  // ─── 公開 API ─────────────────────────────────────────────────────────

  function openFilesTab(sessionId, projectKey, filesRoot, gitRoot) {
    ensureSessionTab();
    showTabBar();
    ensureAddTabButton();

    // 同じ root のタブが既にあればそちらをアクティブに。
    // プロジェクト共有: 同じ projectKey（__no_project__ 除く）& 同じ filesRoot を再利用。
    // プロジェクト不明セッションは従来通り sessionId 一致のみ。
    const hasProject = projectKey && projectKey !== '__no_project__';
    const existing = tabs.find(t => {
      if ((t.kind || t.type) !== 'files') return false;
      if (t.filesRoot !== filesRoot) return false;
      if (hasProject) return t.projectKey === projectKey;
      return sameSessionId(t.sessionId, sessionId);
    });
    if (existing) {
      // 別セッションで作られたタブをアクティブセッションへ付け替え（API 呼び出しが現セッションに紐づくよう）
      if (sessionId != null && !sameSessionId(existing.sessionId, sessionId)) {
        existing.sessionId = sessionId;
        if (existing.contentEl) existing.contentEl.dataset.sessionId = String(sessionId);
        updateTabLabelPrefix(existing, sessionId);
      }
      setActive(existing.id);
      return existing.id;
    }

    const id = makeid();
    const displayName = projectKey && projectKey !== '__no_project__' ? projectKey : filesRoot;
    // タブラベルは通常 "📁 <projectName>/<開いたディレクトリの basename>"。
    // プロジェクト直下を開いた場合は projectName と basename が同じなので片方だけ表示する。
    const rootBase = (filesRoot || '').replace(/[\\/]+$/, '').split(/[\\/]/).pop() || filesRoot || '';
    const sameDisplay = String(displayName).toLowerCase() === String(rootBase).toLowerCase();
    const label = sessionTabPrefix(sessionId) + 'Files: ' + (sameDisplay ? displayName : displayName + '/' + rootBase);

    // タブボタン DOM
    const tabBtn = document.createElement('button');
    tabBtn.className = 'main-tab';
    tabBtn.dataset.tabId = id;

    const labelSpan = document.createElement('span');
    labelSpan.dataset.tabLabel = '1';
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

    const tabObj = { id, kind: 'files', type: 'files', label, sessionId, filesRoot, gitRoot, projectKey, el: tabBtn, contentEl, labelEl: labelSpan };
    tabs.push(tabObj);

    // localStorage に保存
    if (gitRoot) lsAddTab(gitRoot, filesRoot);

    setActive(id);
    notifyTabStateChanged();
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
    // backward-compat alias. kind 別の cleanup は closeMainTab に統合済み。
    return closeMainTab(id);
  }

  function closeMainTab(id) {
    const idx = tabs.findIndex(t => t.id === id);
    if (idx === -1) return;
    const tabObj = tabs[idx];
    // DOM 削除
    try { tabObj.el.remove(); } catch (_) {}
    try { tabObj.contentEl.remove(); } catch (_) {}
    tabs.splice(idx, 1);
    // kind 別の cleanup
    const kind = tabObj.kind || tabObj.type;
    if (kind === 'files') {
      if (tabObj.gitRoot) lsRemoveTab(tabObj.gitRoot, tabObj.filesRoot);
    } else if (kind === 'git') {
      if (tabObj.gitRoot) lsRemoveGitTab(tabObj.gitRoot);
      // GitGraphView インスタンスがあれば dispose
      try {
        if (tabObj.gitView && typeof tabObj.gitView.dispose === 'function') {
          tabObj.gitView.dispose();
        }
      } catch (_) {}
    }
    // アクティブだったら session ビューに戻る
    if (activeTabId === id) {
      switchToSessionView();
    }
    // Files/Git タブが 0 になってもセッションタブと + ボタンは残す
    // （+ ボタンから次の Files/Git タブを開けるようにするため）
    // カードの open マーカー再描画
    notifyTabStateChanged();
  }

  function switchToSessionView() {
    ensureSessionTab();
    showTabBar();
    refreshVisibleTabs();
    setActive('session');
  }

  // ─── + ボタン (タブバー右端) ───────────────────────────────────────────
  let addTabBtn = null;
  function ensureAddTabButton() {
    if (addTabBtn) return;
    addTabBtn = document.createElement('button');
    addTabBtn.className = 'main-tab-add-btn';
    addTabBtn.type = 'button';
    addTabBtn.textContent = '+';
    addTabBtn.title = ti18n('main_tab_add_tooltip', 'Add tab');
    addTabBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      openAddTabMenu(addTabBtn.getBoundingClientRect());
    });
    placeAddTabButtonAfterSessionTab();
  }
  function removeAddTabButton() {
    if (addTabBtn) { try { addTabBtn.remove(); } catch (_) {} addTabBtn = null; }
  }

  // ─── + ボタン → 簡易ドロップダウンメニュー ───────────────────────────
  let addTabMenuEl = null;
  function openAddTabMenu(anchorRect) {
    closeAddTabMenu();
    const menu = document.createElement('div');
    menu.className = 'main-tab-add-menu open';
    const sessId = (typeof activeSessionId !== 'undefined') ? activeSessionId : null;
    menu.innerHTML =
      `<button data-act="add-git" type="button"><span class="ico">⎇</span><span>${escapeHtml(ti18n('add_git_tab', 'Add Git tab'))}</span></button>` +
      `<button data-act="add-files" type="button"><span class="ico">📁</span><span>${escapeHtml(ti18n('add_files_tab', 'Add Files tab'))}</span></button>`;
    document.body.appendChild(menu);
    const r = menu.getBoundingClientRect();
    const x = Math.min(anchorRect.left, window.innerWidth - r.width - 4);
    const y = Math.min(anchorRect.bottom + 2, window.innerHeight - r.height - 4);
    menu.style.left = x + 'px';
    menu.style.top  = y + 'px';
    addTabMenuEl = menu;
    menu.querySelectorAll('button').forEach(b => {
      b.addEventListener('click', (e) => {
        e.stopPropagation();
        const act = b.dataset.act;
        closeAddTabMenu();
        if (sessId == null) return;
        const sess = sessionsRef ? sessionsRef.get(sessId) : null;
        if (!sess) return;
        if (act === 'add-git') {
          const gr = sess.git_root || sess.cwd || '';
          if (!gr) return;
          openGitTab(sessId, gr, sess.branch || '');
        } else if (act === 'add-files') {
          const gr = sess.git_root || sess.cwd || '';
          const pk = sess.project || (gr ? gr.split(/[\\/]/).filter(Boolean).pop() : '__no_project__');
          if (!gr) return;
          openFilesTab(sessId, pk, gr, gr);
        }
      });
    });
  }
  function closeAddTabMenu() {
    if (addTabMenuEl) { try { addTabMenuEl.remove(); } catch (_) {} addTabMenuEl = null; }
  }
  document.addEventListener('mousedown', (e) => {
    if (addTabMenuEl && !e.target.closest('.main-tab-add-menu') && !e.target.closest('.main-tab-add-btn')) {
      closeAddTabMenu();
    }
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && addTabMenuEl) closeAddTabMenu();
  });

  // ─── git タブを開く ────────────────────────────────────────────────────
  function openGitTab(sessionId, gitRoot, branch) {
    if (!gitRoot) return null;
    ensureSessionTab();
    showTabBar();
    ensureAddTabButton();

    // 渡された sessionId からプロジェクトキーを引き、プロジェクト単位でタブを共有する。
    // プロジェクト不明（__no_project__/空）の場合は従来通り sessionId 一致のみ。
    let projectKey = '';
    if (sessionsRef && sessionId != null) {
      const s = sessionsRef.get(sessionId);
      if (s) projectKey = s.project || '';
    }
    const hasProject = projectKey && projectKey !== '__no_project__';

    // 同 gitRoot の git タブがあれば activate + view ref 更新
    const existing = tabs.find(t => {
      if ((t.kind || t.type) !== 'git') return false;
      if (t.gitRoot !== gitRoot) return false;
      if (hasProject) return t.projectKey === projectKey;
      return sameSessionId(t.sessionId, sessionId);
    });
    if (existing) {
      const newRef = branch || existing.viewRef || '';
      existing.viewRef = newRef;
      // セッションが渡されていればそちらに付け替え（ラベル prefix も追従）
      if (sessionId != null && !sameSessionId(existing.sessionId, sessionId)) {
        existing.sessionId = sessionId;
        if (existing.contentEl) existing.contentEl.dataset.sessionId = String(sessionId);
        updateTabLabelPrefix(existing, sessionId);
        try {
          if (existing.gitView && typeof existing.gitView.setSessionId === 'function') {
            existing.gitView.setSessionId(sessionId);
          }
        } catch (_) {}
      } else if (sessionId != null) {
        existing.sessionId = sessionId;
      }
      // タブラベルの ref 部分を更新
      try {
        const refSpan = existing.el.querySelector('.ref');
        if (refSpan) refSpan.textContent = newRef ? `(${newRef})` : '';
      } catch (_) {}
      if (existing.gitRoot) lsUpdateGitViewRef(existing.gitRoot, newRef);
      try {
        if (existing.gitView && typeof existing.gitView.setViewRef === 'function' && newRef) {
          existing.gitView.setViewRef(newRef);
        }
      } catch (_) {}
      setActive(existing.id);
      notifyTabStateChanged();
      return existing.id;
    }

    // 新規作成
    const id = makeid();
    // projectName 推定: project キーがあればそれを使う。無ければ gitRoot の basename
    let projectName = hasProject ? projectKey : '';
    if (!projectName) {
      projectName = (gitRoot || '').replace(/[\\/]+$/, '').split(/[\\/]/).pop() || gitRoot;
    }

    const tabBtn = document.createElement('button');
    tabBtn.className = 'main-tab main-tab-git';
    tabBtn.dataset.tabId = id;
    tabBtn.type = 'button';

    const iconSpan = document.createElement('span');
    iconSpan.className = 'icon';
    iconSpan.textContent = '⎇';
    tabBtn.appendChild(iconSpan);

    const labelSpan = document.createElement('span');
    labelSpan.dataset.tabLabel = '1';
    labelSpan.textContent = sessionTabPrefix(sessionId) + 'Git: ' + projectName + ' ';
    tabBtn.appendChild(labelSpan);

    const refSpan = document.createElement('span');
    refSpan.className = 'ref';
    refSpan.textContent = branch ? `(${branch})` : '';
    tabBtn.appendChild(refSpan);

    const closeBtn = document.createElement('button');
    closeBtn.className = 'main-tab-close';
    closeBtn.type = 'button';
    closeBtn.textContent = '×';
    closeBtn.title = (typeof t === 'function' ? t('files_tab_close_tooltip') : 'Close');
    closeBtn.addEventListener('click', (e) => { e.stopPropagation(); closeMainTab(id); });
    tabBtn.appendChild(closeBtn);

    tabBtn.addEventListener('click', () => setActive(id));

    tabList.appendChild(tabBtn);

    // コンテンツ DOM
    const contentEl = document.createElement('div');
    contentEl.className = 'git-tab-content';
    contentEl.dataset.tabId = id;
    contentEl.dataset.gitRoot = gitRoot;
    contentEl.dataset.sessionId = sessionId != null ? String(sessionId) : '';
    // session 不明時の警告ヘッダ
    const warnHtml = (sessionId == null)
      ? `<div class="git-tab-placeholder-warning" data-git-no-session>${escapeHtml(ti18n('git_tab_no_session_warning', 'session: なし (元セッション削除済)'))}</div>`
      : '';
    contentEl.innerHTML = warnHtml +
      `<div class="git-tab-placeholder-body" data-git-placeholder-body>${escapeHtml(ti18n('git_tab_loading', 'Loading Git view...'))}</div>`;
    filesContents.appendChild(contentEl);

    const tabObj = {
      id, kind: 'git', type: 'git',
      label: 'git: ' + projectName,
      sessionId: sessionId != null ? sessionId : null,
      gitRoot, viewRef: branch || '', projectName,
      projectKey: hasProject ? projectKey : '',
      el: tabBtn, contentEl, labelEl: labelSpan,
      gitView: null,
    };
    tabs.push(tabObj);

    // GitGraphView インスタンス化（クラスが存在する場合のみ）
    try {
      if (typeof window.GitGraphView === 'function' && sessionId != null) {
        // placeholder を消してから GitGraphView をマウント
        const body = contentEl.querySelector('[data-git-placeholder-body]');
        if (body) body.remove();
        tabObj.gitView = new window.GitGraphView(contentEl, {
          sessionId,
          gitRoot,
          viewRef: branch || '',
        });
      }
    } catch (err) {
      console.warn('[FilesTabManager] GitGraphView mount failed:', err);
    }

    // LS 保存
    lsAddGitTab(gitRoot, projectName, branch || '');

    setActive(id);
    notifyTabStateChanged();
    return id;
  }

  // ─── カード open マーカー再描画通知 ───────────────────────────────────
  // 外部で listen する用。renderSessionList を直接呼ぶと循環するため
  // setTimeout で非同期化。
  function notifyTabStateChanged() {
    try {
      if (typeof window.dispatchEvent === 'function') {
        window.dispatchEvent(new CustomEvent('files-tab-state-changed'));
      }
    } catch (_) {}
  }

  // ─── 同 gitRoot のタブ存在チェック（カードマーカー用） ───────────────
  function hasGitTabForRoot(gitRoot) {
    if (!gitRoot) return false;
    const sessionId = arguments.length >= 2 ? arguments[1] : undefined;
    return tabs.some(t => (t.kind || t.type) === 'git' && t.gitRoot === gitRoot && (sessionId === undefined || sameSessionId(t.sessionId, sessionId)));
  }
  function hasFilesTabForRoot(gitRoot) {
    if (!gitRoot) return false;
    const sessionId = arguments.length >= 2 ? arguments[1] : undefined;
    return tabs.some(t => (t.kind || t.type) === 'files' && t.gitRoot === gitRoot && (sessionId === undefined || sameSessionId(t.sessionId, sessionId)));
  }

  // ─── セッション削除イベント: 紐づきタブを別セッションに付け替え ────
  function onSessionRemoved(removedSessionId) {
    if (!sessionsRef) return;
    let mutated = false;
    for (const tab of tabs) {
      if (tab.sessionId !== removedSessionId) continue;
      const kind = tab.kind || tab.type;
      if (kind !== 'git' && kind !== 'files') continue;
      // 同 gitRoot の別セッションを探す
      let candidate = null;
      if (tab.gitRoot && sessionsRef) {
        for (const s of sessionsRef.values()) {
          if (s.id === removedSessionId) continue;
          const gr = s.git_root || s.cwd || '';
          if (gr === tab.gitRoot || (gr && tab.gitRoot && gr.startsWith(tab.gitRoot))) {
            candidate = s;
            break;
          }
        }
      }
      if (candidate) {
        tab.sessionId = candidate.id;
        if (tab.contentEl) tab.contentEl.dataset.sessionId = String(candidate.id);
        updateTabLabelPrefix(tab, candidate.id);
        mutated = true;
        // git タブの警告ヘッダがあれば除去 + GitGraphView の sessionId 同期
        if (kind === 'git') {
          const warn = tab.contentEl.querySelector('[data-git-no-session]');
          if (warn) warn.remove();
          try {
            if (tab.gitView && typeof tab.gitView.setSessionId === 'function') {
              tab.gitView.setSessionId(candidate.id);
            }
          } catch (_) {}
        }
      } else {
        // 付け替え不可
        if (kind === 'git') {
          tab.sessionId = null;
          updateTabLabelPrefix(tab, null);
          if (tab.contentEl) {
            tab.contentEl.dataset.sessionId = '';
            // 警告ヘッダがなければ追加
            if (!tab.contentEl.querySelector('[data-git-no-session]')) {
              const warn = document.createElement('div');
              warn.className = 'git-tab-placeholder-warning';
              warn.setAttribute('data-git-no-session', '');
              warn.textContent = ti18n('git_tab_no_session_warning', 'session: なし (元セッション削除済)');
              tab.contentEl.insertBefore(warn, tab.contentEl.firstChild);
            }
          }
          mutated = true;
        } else if (kind === 'files') {
          // files タブはセッションがないと操作不能なので閉じる
          closeMainTab(tab.id);
        }
      }
    }
    if (mutated) notifyTabStateChanged();
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
    // ─ git タブ復元（先に開いておくと files タブと順序が安定する）─────
    if (Array.isArray(data.git)) {
      const restorableGitTabs = [];
      for (const entry of data.git) {
        if (!entry || !entry.gitRoot) continue;
        const matched = sessions.find(s => {
          const gr = s.git_root || s.cwd || '';
          return gr === entry.gitRoot || (gr && gr.startsWith(entry.gitRoot));
        });
        if (matched) {
          restorableGitTabs.push(entry);
          openGitTab(matched.id, entry.gitRoot, entry.viewRef || matched.branch || '');
        }
      }
      if (restorableGitTabs.length !== data.git.length) {
        data.git = restorableGitTabs;
        lsSave(data);
      }
    }
    // ─ files タブ復元（既存ロジック維持）─────────────────────────
    const filesMap = data.files || {};
    for (const [gitRoot, entries] of Object.entries(filesMap)) {
      if (!Array.isArray(entries)) continue;
      // gitRoot に対応するセッションを探す（cwd が gitRoot で始まるもの）
      const matchedSession = sessions.find(s => s.cwd && (s.cwd === gitRoot || s.cwd.startsWith(gitRoot)));
      // 対応するセッションが無い場合は復元しない。
      // セッションが無いまま Hub の cwd と localStorage の root が重なると、
      // probe が偶然通って `..` だけのゴーストタブが復元されてしまうため。
      // localStorage は残すので、対象セッションを spawn し直した次回起動で自動復活する。
      if (!matchedSession) {
        console.info('[FilesTabManager] skip restoring tabs for', gitRoot, '(no matching session)');
        continue;
      }
      const sessionId  = matchedSession.id;
      const projectKey = matchedSession.project;

      for (const entry of entries) {
        if (!entry.root) continue;
        // 現在の Hub の許可ルート外なら復元しない（ゾンビタブ防止）。
        // また items が空（.md が一切無い）の場合も復元しない（"死骸タブ" 防止）。
        // 加えて全エントリの rel が `..` で始まる場合（filesRoot がセッション cwd の外側）も復元しない。
        // → tree が `..` 1 個だけで中身を確認できないため。
        try {
          const sessionQs = `&session=${encodeURIComponent(sessionId)}`;
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
          if (!Array.isArray(probeData.items) || probeData.items.length === 0) {
            console.info('[FilesTabManager] skip restoring tab (no files under root):', entry.root);
            continue;
          }
          const allOutsideCwd = probeData.items.every(it => {
            const rel = String(it && it.rel || '').replace(/\\/g, '/');
            return rel.startsWith('../') || rel === '..';
          });
          if (allOutsideCwd) {
            console.info('[FilesTabManager] skip restoring tab (root outside session cwd):', entry.root);
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
    openGitTab,
    closeFilesTab,
    closeMainTab,
    switchToSessionView,
    updateSessionTabLabel,
    restoreFromLocalStorage,
    lsUpdateOpenedFile,
    hasGitTabForRoot,
    hasFilesTabForRoot,
    onSessionRemoved,
    setSessionsRef,
    getSessionsRef,
  };
})();

// Hub 起動時に localStorage からタブを復元
FilesTabManager.setSessionsRef(sessions);
FilesTabManager.restoreFromLocalStorage();

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
  // API (/api/files-list) は { path, rel, name, type, size, mtime, summary } のフラットな一覧を返す。
  // 古いレスポンスやファイル由来の親ディレクトリも扱えるよう、存在しない親 dir はここで補完する。
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
      const rawRel = item.rel || '';
      const absPath = item.path || '';
      if (!rawRel) continue;
      // Windows の API レスポンスは '\' 区切り。nodes[] キーを全て '/' 区切りに揃えることで
      // 親 dir 補完キー（'/' 区切り）と自エントリ登録キーの不整合による重複ノードを防ぐ。
      const relPath = rawRel.replace(/\\/g, '/');
      const parts = relPath.split('/');
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
      const itemType = item.type === 'dir' ? 'dir' : 'file';
      if (itemType === 'dir') {
        if (!nodes[relPath]) {
          const node = { name: parts[parts.length - 1], relPath, absPath, type: 'dir', children: [] };
          nodes[relPath] = node;
          parent.children.push(node);
        } else if (!nodes[relPath].absPath && absPath) {
          nodes[relPath].absPath = absPath;
        }
      } else {
        const node = { name: parts[parts.length - 1], relPath, absPath, type: 'file', children: [] };
        nodes[relPath] = node;
        parent.children.push(node);
      }
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
    // /api/files-content の許可リストと一致する isTextPath を再利用。
    return typeof isTextPath === 'function' ? isTextPath(name) : /\.(md|txt)$/i.test(name);
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

    // ancestorMatch=true なら、祖先 dir 名がフィルタにヒット済みなので
    // 自身および配下は無条件で表示する（dir 名検索時の期待動作）。
    function makeNode(node, depth, ancestorMatch) {
      if (filterLower && !ancestorMatch && !nodeMatchesFilter(node)) return null;

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

        const dirNameMatches = !!filterLower && node.name.toLowerCase().includes(filterLower);
        const childAncestorMatch = ancestorMatch || dirNameMatches;
        const childList = (filterLower && !childAncestorMatch)
          ? node.children.filter(nodeMatchesFilter)
          : node.children;
        for (const child of childList) {
          const childEl = makeNode(child, depth + 1, childAncestorMatch);
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
      const el = makeNode(child, 0, false);
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
        if (!res.ok) { treeArea.innerHTML = `<div class="files-tree-error">${escapeHtml('HTTP ' + res.status)} — ${escapeHtml(filesRoot)}</div>`; return; }
        const data = await res.json();
        if (!data.exists) { treeArea.innerHTML = `<div class="files-tree-error">${escapeHtml(t('files_tree_not_found') || 'Directory not found')} — ${escapeHtml(filesRoot)}</div>`; return; }
        const items = data.items || [];
        if (items.length === 0) {
          treeArea.innerHTML = `<div class="files-tree-error">${escapeHtml(t('files_tree_empty') || 'No files found')}<br><small>${escapeHtml(filesRoot)}</small></div>`;
          currentTree = buildTree(items);
          return;
        }
        currentTree = buildTree(items);
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

    // ファイル変更（rename 等）を受けてツリーを再読み込み
    const filesChangedHandler = (e) => {
      const detail = (e && e.detail) || {};
      if (detail.kind === 'rename' && selectedAbsPath && detail.oldAbs && detail.newAbs
          && pathsEqualCI(selectedAbsPath, detail.oldAbs)) {
        selectedAbsPath = detail.newAbs;
      }
      loadTree();
    };
    window.addEventListener('any-ai-cli:files-changed', filesChangedHandler);

    // 初回ロード
    loadTree();

    // 外部から "ファイルを選択済み状態にする" 用
    containerEl._filesTree = {
      selectFile: (absPath) => {
        selectedAbsPath = absPath;
        highlightSelected(absPath);
      },
    };
    containerEl._filesTreeCleanup = () => {
      window.removeEventListener('any-ai-cli:files-changed', filesChangedHandler);
    };
  }

  function unbind(containerEl) {
    if (typeof containerEl._filesTreeCleanup === 'function') {
      try { containerEl._filesTreeCleanup(); } catch (_) {}
    }
    delete containerEl._filesTreeCleanup;
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

  /** ファイル拡張子 → highlight.js 言語名のマップ（小文字で照合） */
  function detectHljsLangFromPath(absPath) {
    const m = /\.([A-Za-z0-9_+-]+)$/.exec(absPath || '');
    if (!m) return '';
    const ext = m[1].toLowerCase();
    const map = {
      js: 'javascript', mjs: 'javascript', cjs: 'javascript',
      ts: 'typescript', tsx: 'typescript', jsx: 'javascript',
      go: 'go', py: 'python', rb: 'ruby', rs: 'rust',
      html: 'xml', htm: 'xml', xml: 'xml', svg: 'xml',
      css: 'css', scss: 'scss', less: 'less',
      json: 'json', yaml: 'yaml', yml: 'yaml', toml: 'ini', ini: 'ini',
      sh: 'bash', bash: 'bash', zsh: 'bash', ps1: 'powershell',
      c: 'c', h: 'c', cpp: 'cpp', hpp: 'cpp', cc: 'cpp', cs: 'csharp',
      java: 'java', kt: 'kotlin', swift: 'swift',
      sql: 'sql', dockerfile: 'dockerfile', makefile: 'makefile',
      md: 'markdown',
    };
    return map[ext] || '';
  }

  /**
   * ソースコード文字列を hljs で色付けして <pre><code class="hljs"> を返す。
   * - 拡張子 → 言語マップで明示判定し、未対応なら highlightAuto にフォールバック
   * - 巨大ファイル（>=200KB）は重いので自動判定をスキップしプレーン表示
   * - hljs 未ロード時 / 例外時もプレーンで安全に表示する
   */
  function renderSourceToPre(content, absPath) {
    const pre = document.createElement('pre');
    pre.className = 'files-preview-raw';
    const code = document.createElement('code');
    const lang = detectHljsLangFromPath(absPath);
    let html = '';
    if (typeof hljs !== 'undefined') {
      try {
        if (lang && hljs.getLanguage(lang)) {
          html = hljs.highlight(content, { language: lang, ignoreIllegals: true }).value;
          code.className = 'hljs language-' + lang;
        } else if (content.length < 200000) {
          const auto = hljs.highlightAuto(content);
          html = auto.value;
          code.className = 'hljs' + (auto.language ? ' language-' + auto.language : '');
        } else {
          html = escapeHtml(content);
          code.className = 'hljs';
        }
      } catch (_) {
        html = escapeHtml(content);
        code.className = 'hljs';
      }
    } else {
      html = escapeHtml(content);
    }
    code.innerHTML = html;
    pre.appendChild(code);
    return pre;
  }

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
          contentEl.appendChild(renderSourceToPre(content, absPath));
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
          // .txt など — hljs.highlightAuto で自動判定（巨大ファイルはプレーン）
          contentEl.innerHTML = '';
          contentEl.appendChild(renderSourceToPre(content, absPath));
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

// ============================================================
// GitGraphView (C3) — git タブ contentEl 配下に SVG ブランチグラフ /
// コミット一覧 / 上下 split 詳細パネル / ref ドロップダウン /
// Copy 右クリックメニュー を描画する。
//
// 親 plan : docs/local/plan_git_graph_view.md
// 子 plan : docs/local/plan_git_graph_view_c3_graph_view.md
// mock   : docs/local/mockup-git-graph.html
//
// API:
//   GET /api/git-log?session&token&ref&limit&skip
//   GET /api/git-show?session&token&hash
//   GET /api/git-refs?session&token
// ============================================================
(function setupGitGraphView() {
  const LANE_W = 16;
  const LANE_X0 = 12;
  const ROW_H  = 30;
  const DOT_R  = 4.5;
  const LANE_COLORS = ['lane-1', 'lane-2', 'lane-3', 'lane-4'];
  const PAGE_LIMIT = 100;

  function _esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, c => ({
      '&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'
    }[c]));
  }
  function _shortHash(h) { return (h || '').slice(0, 8); }
  function _laneX(idx) { return LANE_X0 + idx * LANE_W; }
  function _laneColor(idx) {
    return LANE_COLORS[((idx % LANE_COLORS.length) + LANE_COLORS.length) % LANE_COLORS.length];
  }
  function _gt(key, fallback) {
    if (typeof window.t === 'function') {
      const v = window.t(key);
      if (v && v !== key) return v;
    }
    return fallback != null ? fallback : key;
  }
  function _toast(title, body) {
    if (typeof window.showToast === 'function') {
      const msg = body ? `${title}: ${body}` : title;
      const one = msg.replace(/\n/g, ' ↵ ');
      window.showToast(one.length > 120 ? one.slice(0, 120) + '…' : one);
    }
  }
  function _formatDate(iso) {
    if (!iso) return '';
    try {
      const d = new Date(iso);
      if (isNaN(d.getTime())) return iso;
      const pad = (n) => String(n).padStart(2, '0');
      return `${d.getFullYear()}/${pad(d.getMonth()+1)}/${pad(d.getDate())} ` +
             `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
    } catch (_) { return iso; }
  }
  function _copyText(text) {
    try {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).catch(() => {});
        return;
      }
    } catch (_) {}
    try {
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.style.position = 'fixed'; ta.style.opacity = '0';
      document.body.appendChild(ta); ta.select();
      document.execCommand('copy');
      ta.remove();
    } catch (_) {}
  }

  // ─────────────────────────────────────────────────────────
  // lane allocation
  //   activeLanes[i] = "次に出現したら閉じたい hash" もしくは null
  //   行を上から順に走査:
  //     - その行の hash がアクティブな lane で待たれていればそこに dot
  //     - 待たれていなければ新規 lane に割当
  //     - parents[0] はその lane を継続。parents[1..] は別 lane を新規確保
  // ─────────────────────────────────────────────────────────
  function computeGraph(commits) {
    const activeLanes = [];
    const laneColors = [];

    function allocLane(color) {
      for (let i = 0; i < activeLanes.length; i++) {
        if (activeLanes[i] == null) {
          activeLanes[i] = '__pending__';
          laneColors[i] = color || laneColors[i] || _laneColor(i);
          return i;
        }
      }
      activeLanes.push('__pending__');
      laneColors.push(color || _laneColor(activeLanes.length - 1));
      return activeLanes.length - 1;
    }

    const rows = [];
    for (const c of commits) {
      const hash = c.hash;
      let myLane = -1;
      const incoming = []; // 自分を待っていた他 lane (merge 入線元)
      for (let i = 0; i < activeLanes.length; i++) {
        if (activeLanes[i] === hash) {
          if (myLane < 0) myLane = i;
          else {
            incoming.push({ x: i, color: laneColors[i] || _laneColor(i) });
            activeLanes[i] = null;
          }
        }
      }
      if (myLane < 0) {
        myLane = allocLane();
      }

      const parents = c.parents || [];
      const isMerge = parents.length >= 2;

      // 描画用 lanes (現在の activeLanes スナップショット)
      const drawLanes = [];
      for (let i = 0; i < activeLanes.length; i++) {
        if (i === myLane) {
          drawLanes.push({
            x: i,
            type: isMerge ? 'merge' : 'dot',
            color: laneColors[i] || _laneColor(i),
            inFrom: isMerge ? incoming.slice() : undefined,
          });
        } else if (activeLanes[i] != null && activeLanes[i] !== '__pending__') {
          drawLanes.push({
            x: i,
            type: 'line',
            color: laneColors[i] || _laneColor(i),
          });
        }
      }

      // parents 展開
      if (parents.length === 0) {
        activeLanes[myLane] = null;
      } else {
        activeLanes[myLane] = parents[0];
        const baseColor = laneColors[myLane] || _laneColor(myLane);
        const mergeExtra = [];
        for (let pi = 1; pi < parents.length; pi++) {
          const ln = allocLane();
          // 追加 parent lane の色: 元 lane と区別するため新色
          const cc = _laneColor(ln);
          laneColors[ln] = cc;
          activeLanes[ln] = parents[pi];
          mergeExtra.push({ x: ln, color: cc });
          drawLanes.push({ x: ln, type: 'line', color: cc });
        }
        // merge dot に追加 parent の入線も付与（mock の inFrom と同等）
        if (isMerge) {
          const me = drawLanes.find(l => l.x === myLane);
          if (me) {
            const fromArr = (me.inFrom || []).slice();
            me.inFrom = fromArr.concat(mergeExtra);
          }
          // base color を残す
          if (laneColors[myLane] == null) laneColors[myLane] = baseColor;
        }
      }

      rows.push({ hashLane: myLane, lanes: drawLanes });
    }
    return rows;
  }

  function renderGraphSvg(row) {
    if (!row) return '';
    const lanes = row.lanes || [];
    const maxX = lanes.reduce((m, l) => Math.max(m, l.x), 0);
    const w = Math.max(110, _laneX(maxX + 1) + 6);
    const h = ROW_H;
    const parts = [];
    for (const lane of lanes) {
      const x = _laneX(lane.x);
      const color = `var(--${lane.color})`;
      if (lane.type === 'line') {
        parts.push(`<line x1="${x}" y1="0" x2="${x}" y2="${h}" stroke="${color}" stroke-width="2"/>`);
      } else if (lane.type === 'dot') {
        parts.push(`<line x1="${x}" y1="0" x2="${x}" y2="${h}" stroke="${color}" stroke-width="2"/>`);
        parts.push(`<circle cx="${x}" cy="${h/2}" r="${DOT_R}" fill="${color}" stroke="var(--bg)" stroke-width="1.5"/>`);
      } else if (lane.type === 'merge') {
        parts.push(`<line x1="${x}" y1="0" x2="${x}" y2="${h}" stroke="${color}" stroke-width="2"/>`);
        for (const inc of (lane.inFrom || [])) {
          const xi = _laneX(inc.x);
          const ic = `var(--${inc.color})`;
          parts.push(`<path d="M ${xi} 0 C ${xi} ${h*0.55}, ${x} ${h*0.45}, ${x} ${h/2}" stroke="${ic}" stroke-width="2" fill="none"/>`);
        }
        const s = DOT_R + 0.5;
        parts.push(
          `<rect x="${x - s}" y="${h/2 - s}" width="${s*2}" height="${s*2}" ` +
          `transform="rotate(45 ${x} ${h/2})" ` +
          `fill="${color}" stroke="var(--bg)" stroke-width="1.5"/>`
        );
      }
    }
    return `<svg width="${w}" height="${h}">${parts.join('')}</svg>`;
  }

  function renderRefsInline(refs, headHash, commitHash) {
    if (!refs || !refs.length) return '';
    return refs.map(r => {
      const kind = (r.kind || 'local').toLowerCase();
      const name = _esc(r.name || '');
      const isHead = (headHash && commitHash && headHash === commitHash &&
                      (kind === 'local' || kind === 'head'));
      if (isHead) return `<span class="ref-chip head">${name}</span>`;
      if (kind === 'remote') return `<span class="ref-chip remote">${name}</span>`;
      if (kind === 'tag')    return `<span class="ref-chip tag">${name}</span>`;
      return `<span class="ref-chip local">${name}</span>`;
    }).join('');
  }

  function splitSubject(subject) {
    const m = (subject || '').match(/^([a-zA-Z0-9_-]+:)\s+(.*)$/);
    if (m) return { prefix: m[1], rest: m[2] };
    return { prefix: '', rest: subject || '' };
  }
  function statusClass(status) {
    const s = String(status || 'M').slice(0, 1).toUpperCase();
    return /^[A-Z]$/.test(s) ? s : 'U';
  }

  // ─── Copy 右クリックメニュー (全 git タブ共有) ─────────────
  let _ctxMenuEl = null;
  let _ctxTarget = null;
  let _ctxTargetEl = null;
  let _ctxGithubBase = '';

  function _ensureCtxMenu() {
    if (_ctxMenuEl) return _ctxMenuEl;
    const m = document.createElement('div');
    m.className = 'git-ctx-menu';
    m.innerHTML = `
      <div class="ctx-header" data-ctx-header>commit</div>
      <button data-action="copy-short"><span class="ctx-icon">#</span><span class="ctx-label">${_esc(_gt('git_ctx_copy_short', 'Copy short hash'))}</span><span class="ctx-hint" data-hint-short></span></button>
      <button data-action="copy-full"><span class="ctx-icon">⎘</span><span class="ctx-label">${_esc(_gt('git_ctx_copy_full', 'Copy full hash'))}</span><span class="ctx-hint" data-hint-full></span></button>
      <div class="ctx-sep"></div>
      <button data-action="copy-subject"><span class="ctx-icon">✎</span><span class="ctx-label">${_esc(_gt('git_ctx_copy_subject', 'Copy subject'))}</span></button>
      <button data-action="copy-message"><span class="ctx-icon">¶</span><span class="ctx-label">${_esc(_gt('git_ctx_copy_message', 'Copy message (subject + body)'))}</span></button>
      <button data-action="copy-hash-subject"><span class="ctx-icon">⇋</span><span class="ctx-label">${_esc(_gt('git_ctx_copy_hash_subject', 'Copy hash + subject'))}</span></button>
      <div class="ctx-sep"></div>
      <button data-action="copy-github-url" data-ctx-gh><span class="ctx-icon">↗</span><span class="ctx-label">${_esc(_gt('git_ctx_copy_github', 'Copy GitHub link'))}</span><span class="ctx-hint" data-hint-gh></span></button>
    `;
    document.body.appendChild(m);
    m.addEventListener('click', (e) => {
      const btn = e.target.closest('button[data-action]');
      if (!btn || btn.disabled || !_ctxTarget) return;
      _ctxCopy(btn.dataset.action, _ctxTarget);
      _closeCtxMenu();
    });
    document.addEventListener('click', (e) => {
      if (!_ctxMenuEl) return;
      if (!_ctxMenuEl.contains(e.target)) _closeCtxMenu();
    });
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') _closeCtxMenu();
    });
    window.addEventListener('blur', _closeCtxMenu);
    _ctxMenuEl = m;
    return m;
  }

  function _openCtxMenu(x, y, commit, rowEl, githubBase) {
    const m = _ensureCtxMenu();
    _ctxTarget = commit;
    _ctxGithubBase = githubBase || '';
    if (_ctxTargetEl) _ctxTargetEl.classList.remove('context');
    _ctxTargetEl = rowEl;
    if (rowEl) rowEl.classList.add('context');

    m.querySelector('[data-ctx-header]').textContent =
      `${_shortHash(commit.hash)}  ${commit.author_name || ''}`;
    m.querySelector('[data-hint-short]').textContent = _shortHash(commit.hash);
    m.querySelector('[data-hint-full]').textContent  = (commit.hash || '').slice(0, 14) + '…';
    const ghBtn  = m.querySelector('[data-ctx-gh]');
    const ghHint = m.querySelector('[data-hint-gh]');
    if (_ctxGithubBase) {
      ghBtn.disabled = false;
      ghHint.textContent = 'github.com/…';
    } else {
      ghBtn.disabled = true;
      ghHint.textContent = '';
    }
    m.classList.add('open');
    const r = m.getBoundingClientRect();
    const maxX = window.innerWidth  - r.width  - 4;
    const maxY = window.innerHeight - r.height - 4;
    m.style.left = Math.max(0, Math.min(x, maxX)) + 'px';
    m.style.top  = Math.max(0, Math.min(y, maxY)) + 'px';
  }
  function _closeCtxMenu() {
    if (!_ctxMenuEl) return;
    _ctxMenuEl.classList.remove('open');
    if (_ctxTargetEl) { _ctxTargetEl.classList.remove('context'); _ctxTargetEl = null; }
    _ctxTarget = null;
  }
  function _ctxCopy(action, c) {
    const sub = splitSubject(c.subject);
    const subjectFull = (sub.prefix ? sub.prefix + ' ' : '') + sub.rest;
    let value = ''; let title = '';
    switch (action) {
      case 'copy-short':
        value = _shortHash(c.hash); title = _gt('git_ctx_copy_short', 'Copy short hash'); break;
      case 'copy-full':
        value = c.hash || ''; title = _gt('git_ctx_copy_full', 'Copy full hash'); break;
      case 'copy-subject':
        value = subjectFull; title = _gt('git_ctx_copy_subject', 'Copy subject'); break;
      case 'copy-message':
        value = subjectFull + (c.body ? '\n\n' + c.body : '');
        title = _gt('git_ctx_copy_message', 'Copy message'); break;
      case 'copy-hash-subject':
        value = `${_shortHash(c.hash)} ${subjectFull}`;
        title = _gt('git_ctx_copy_hash_subject', 'Copy hash + subject'); break;
      case 'copy-github-url':
        if (!_ctxGithubBase) return;
        value = `${_ctxGithubBase.replace(/\/$/, '')}/commit/${c.hash}`;
        title = _gt('git_ctx_copy_github', 'Copy GitHub link'); break;
    }
    _copyText(value);
    _toast(title, value);
  }

  // ───────────────────────────────────────────────────────
  // GitGraphView クラス本体
  // ───────────────────────────────────────────────────────
  class GitGraphView {
    constructor(containerEl, opts) {
      this.container = containerEl;
      this.opts = opts || {};
      this.sessionId = this.opts.sessionId;
      this.gitRoot   = this.opts.gitRoot || '';
      this.viewRef   = this.opts.viewRef || 'HEAD';

      this.token = new URLSearchParams(location.search).get('token') || '';

      this.commits = [];
      this.filteredCommits = [];
      this.graphRows = [];
      this.skip = 0;
      this.hasMore = false;
      this.headHash = '';
      this.sessionBranch = '';
      this.refs = [];
      this.githubUrl = '';
      this.selectedHash = null;
      this.selectedShow = null;
      this.activeTab = 'info';
      this.panelHeight = 340;
      this.filterText = '';
      this.loading = false;
      this.workingTree = null;
      this.commitModalState = {
        open: false,
        reviewed: false,
        busy: false,
        generating: false,
        error: '',
        subject: '',
        body: '',
      };

      this.els = {};
      this._renderShell();
      this._refDropdownOpen = false;

      this._docClickHandler = (e) => {
        if (!this._refDropdownOpen) return;
        if (this.els.refDropdown && this.els.refDropdown.contains(e.target)) return;
        if (this.els.viewRefBtn && this.els.viewRefBtn.contains(e.target)) return;
        this._closeRefDropdown();
      };
      this._docKeyHandler = (e) => {
        if (e.key === 'Escape' && this._refDropdownOpen) this._closeRefDropdown();
      };
      document.addEventListener('click', this._docClickHandler);
      document.addEventListener('keydown', this._docKeyHandler);

      this.load().catch(err => this._showError(err && err.message ? err.message : String(err)));
    }

    _renderShell() {
      const c = this.container;
      const root = document.createElement('div');
      root.className = 'git-graph-root';
      root.innerHTML = `
        <div class="git-graph-header">
          <div class="git-graph-title">⎇ Git</div>
          <div class="git-graph-repo" data-repo>${_esc(this.gitRoot)}</div>
          <button class="view-ref-btn" data-view-ref-btn>
            <span data-view-ref-label>${_esc(this.viewRef || 'HEAD')}</span>
            <span class="chev">▾</span>
          </button>
          <div class="session-head-chip" data-session-head-chip style="display:none">session HEAD: <span data-session-head-label></span></div>
          <div class="git-graph-spacer"></div>
          <div class="git-working-count" data-working-count>—</div>
          <div class="git-graph-count" data-count>${_esc(_gt('git_view_loading', 'loading...'))}</div>
          <button class="git-icon-btn" data-refresh-btn title="${_esc(_gt('git_view_refresh', 'Refresh'))}">↻</button>
          <button class="git-icon-btn" data-loadmore-btn title="${_esc(_gt('git_view_load_more', 'Load 100 more'))}">+${PAGE_LIMIT}</button>
          <button class="git-commit-all-btn" data-commit-all-btn disabled>${_esc(_gt('git_commit_all', 'Commit all'))}</button>
        </div>

        <div class="git-graph-toolbar">
          <input type="search" data-filter placeholder="${_esc(_gt('git_view_filter_placeholder', 'subject / author / hash filter'))}">
          <div class="filter-group">
            <button class="git-icon-btn" data-toggle="HEAD" title="HEAD">HEAD</button>
            <button class="git-icon-btn" data-toggle="--all" title="--all">all</button>
          </div>
        </div>

        <div class="git-working-preview" data-working-preview></div>

        <div class="git-graph-split">
          <div class="git-graph-log-table" data-log-table>
            <div class="git-graph-loading">${_esc(_gt('git_view_loading', 'Loading...'))}</div>
          </div>
          <div class="split-divider" data-divider title="${_esc(_gt('git_view_divider_tip', 'Drag to resize'))}"></div>
          <div class="detail-panel" data-detail-panel>
            <div class="detail-tabbar">
              <button class="detail-tab active" data-tab="info">${_esc(_gt('git_view_tab_info', 'INFORMATION'))}</button>
              <button class="detail-tab" data-tab="changes">${_esc(_gt('git_view_tab_changes', 'CHANGES'))}</button>
              <button class="detail-tab" data-tab="files">${_esc(_gt('git_view_tab_files', 'FILES'))}</button>
              <div class="detail-tab-spacer"></div>
              <div class="detail-tab-meta" data-detail-meta>—</div>
              <button class="detail-close" data-detail-close title="${_esc(_gt('git_view_detail_close', 'Close'))}">✕</button>
            </div>
            <div class="detail-content" data-detail-content>
              <div class="detail-empty">${_esc(_gt('git_view_select_row_hint', 'Click a row to see commit detail'))}</div>
            </div>
          </div>
        </div>

        <div class="ref-dropdown" data-ref-dropdown>
          <div class="ref-dropdown-search">
            <input type="search" data-ref-filter placeholder="${_esc(_gt('git_view_ref_filter_placeholder', 'Search branches / tags...'))}">
          </div>
          <div class="ref-list" data-ref-list></div>
        </div>

        <div class="git-commit-modal-backdrop" data-commit-modal hidden>
          <div class="git-commit-modal" role="dialog" aria-modal="true">
            <div class="git-commit-modal-head">
              <div>
                <div class="git-commit-modal-title">${_esc(_gt('git_commit_all', 'Commit all'))}</div>
                <div class="git-commit-modal-sub" data-commit-summary>—</div>
              </div>
              <button class="git-commit-close" data-commit-close title="${_esc(_gt('git_view_detail_close', 'Close'))}">✕</button>
            </div>
            <div class="git-commit-warning">${_esc(_gt('git_commit_no_push_warning', 'Only git add -A and git commit are run. Push is not run.'))}</div>
            <div class="git-commit-generate-note">${_esc(_gt('git_commit_generate_note', 'Generate checks the diff with the current AI agent and fills in a draft commit message. Review it before committing.'))}</div>
            <label class="git-commit-field">
              <span>${_esc(_gt('git_commit_subject', 'Commit message subject'))}</span>
              <input type="text" data-commit-subject maxlength="200">
            </label>
            <label class="git-commit-field">
              <span>${_esc(_gt('git_commit_body', 'Optional body'))}</span>
              <textarea data-commit-body rows="6"></textarea>
            </label>
            <div class="git-commit-review" data-commit-review-box hidden>
              ${_esc(_gt('git_commit_review_ready', 'Review complete. Commit will include all current working tree changes.'))}
            </div>
            <div class="git-commit-hint" data-commit-hint hidden></div>
            <div class="git-commit-error" data-commit-error hidden></div>
            <div class="git-commit-actions">
              <button class="git-secondary-btn" data-commit-generate>${_esc(_gt('git_commit_generate_message', 'Generate'))}</button>
              <div class="git-commit-action-spacer"></div>
              <button class="git-secondary-btn" data-commit-cancel>${_esc(_gt('confirm_cancel', 'Cancel'))}</button>
              <button class="git-secondary-btn" data-commit-review>${_esc(_gt('git_commit_review', 'Review'))}</button>
              <button class="git-commit-run-btn" data-commit-run disabled>${_esc(_gt('git_commit_commit', 'Commit'))}</button>
            </div>
          </div>
        </div>
      `;
      c.appendChild(root);

      this.els.root          = root;
      this.els.repo          = root.querySelector('[data-repo]');
      this.els.viewRefBtn    = root.querySelector('[data-view-ref-btn]');
      this.els.viewRefLabel  = root.querySelector('[data-view-ref-label]');
      this.els.sessionHeadChip  = root.querySelector('[data-session-head-chip]');
      this.els.sessionHeadLabel = root.querySelector('[data-session-head-label]');
      this.els.count         = root.querySelector('[data-count]');
      this.els.workingCount  = root.querySelector('[data-working-count]');
      this.els.refreshBtn    = root.querySelector('[data-refresh-btn]');
      this.els.loadmoreBtn   = root.querySelector('[data-loadmore-btn]');
      this.els.commitAllBtn  = root.querySelector('[data-commit-all-btn]');
      this.els.filter        = root.querySelector('[data-filter]');
      this.els.workingPreview = root.querySelector('[data-working-preview]');
      this.els.toggles       = root.querySelectorAll('.filter-group [data-toggle]');
      this.els.logTable      = root.querySelector('[data-log-table]');
      this.els.divider       = root.querySelector('[data-divider]');
      this.els.detailPanel   = root.querySelector('[data-detail-panel]');
      this.els.detailMeta    = root.querySelector('[data-detail-meta]');
      this.els.detailContent = root.querySelector('[data-detail-content]');
      this.els.detailClose   = root.querySelector('[data-detail-close]');
      this.els.detailTabs    = root.querySelectorAll('.detail-tab[data-tab]');
      this.els.refDropdown   = root.querySelector('[data-ref-dropdown]');
      this.els.refFilter     = root.querySelector('[data-ref-filter]');
      this.els.refList       = root.querySelector('[data-ref-list]');
      this.els.commitModal   = root.querySelector('[data-commit-modal]');
      this.els.commitSummary = root.querySelector('[data-commit-summary]');
      this.els.commitClose   = root.querySelector('[data-commit-close]');
      this.els.commitCancel  = root.querySelector('[data-commit-cancel]');
      this.els.commitSubject = root.querySelector('[data-commit-subject]');
      this.els.commitBody    = root.querySelector('[data-commit-body]');
      this.els.commitGenerate = root.querySelector('[data-commit-generate]');
      this.els.commitReview  = root.querySelector('[data-commit-review]');
      this.els.commitRun     = root.querySelector('[data-commit-run]');
      this.els.commitReviewBox = root.querySelector('[data-commit-review-box]');
      this.els.commitHint    = root.querySelector('[data-commit-hint]');
      this.els.commitError   = root.querySelector('[data-commit-error]');

      this.els.refreshBtn.addEventListener('click', () => this.refresh());
      this.els.loadmoreBtn.addEventListener('click', () => this.loadMore());
      this.els.commitAllBtn.addEventListener('click', () => this._openCommitModal());
      this.els.filter.addEventListener('input', (e) => {
        this.filterText = e.target.value || '';
        this._renderLogTable();
      });
      this.els.toggles.forEach(btn => {
        btn.addEventListener('click', () => this.setViewRef(btn.dataset.toggle));
      });
      this.els.viewRefBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        if (this._refDropdownOpen) this._closeRefDropdown();
        else this._openRefDropdown();
      });
      this.els.refFilter.addEventListener('input', (e) => {
        this._renderRefList(e.target.value || '');
      });
      this.els.detailClose.addEventListener('click', () => this._toggleDetailPanel());
      this.els.commitClose.addEventListener('click', () => this._closeCommitModal());
      this.els.commitCancel.addEventListener('click', () => this._closeCommitModal());
      this.els.commitModal.addEventListener('click', (e) => {
        if (e.target === this.els.commitModal) this._closeCommitModal();
      });
      this.els.commitSubject.addEventListener('input', () => {
        this.commitModalState.subject = this.els.commitSubject.value;
        this.commitModalState.reviewed = false;
        this._renderCommitModalState();
      });
      this.els.commitBody.addEventListener('input', () => {
        this.commitModalState.body = this.els.commitBody.value;
        this.commitModalState.reviewed = false;
        this._renderCommitModalState();
      });
      this.els.commitGenerate.addEventListener('click', () => this._generateCommitMessage());
      this.els.commitReview.addEventListener('click', () => this._reviewCommitMessage());
      this.els.commitRun.addEventListener('click', () => this._commitAll());
      this.els.detailTabs.forEach(tabBtn => {
        tabBtn.addEventListener('click', () => {
          this.activeTab = tabBtn.dataset.tab;
          this._syncDetailTabs();
          if (this.selectedShow) this._renderDetailContent();
        });
      });

      this._setupDividerDrag();
      this._syncToggleButtons();
      this._updateViewRefHeader();
    }

    async load() {
      this.skip = 0;
      this.commits = [];
      this._showLoading();
      const refsPromise = this._fetchRefs();
      const statusPromise = this._fetchStatus();
      await this._fetchLog({ append: false });
      await refsPromise;
      await statusPromise;
      this._renderLogTable();
      this._renderWorkingTreePreview();
      if (!this.selectedHash && this.commits.length) {
        const head = this.commits.find(c => c.hash === this.headHash) || this.commits[0];
        if (head) this.selectCommit(head.hash).catch(() => {});
      }
    }

    async loadMore() {
      if (this.loading || !this.hasMore) return;
      this.skip = this.commits.length;
      await this._fetchLog({ append: true });
      this._renderLogTable();
    }

    async refresh() {
      this.skip = 0;
      this.commits = [];
      this.selectedHash = null;
      this.selectedShow = null;
      this._showLoading();
      await Promise.all([this._fetchLog({ append: false }), this._fetchRefs(), this._fetchStatus()]);
      this._renderLogTable();
      this._renderWorkingTreePreview();
      this._renderDetailEmpty();
    }

    async selectCommit(hash) {
      if (!hash) return;
      this.selectedHash = hash;
      this._highlightSelected();
      try {
        const data = await this._fetchShow(hash);
        if (!data || data.ok === false) {
          this._renderDetailError(data && data.detail ? data.detail : 'git-show failed');
          return;
        }
        this.selectedShow = data;
        const panel = this.els.detailPanel;
        if (panel.classList.contains('collapsed')) {
          panel.classList.remove('collapsed');
          panel.style.height = (this.panelHeight || 340) + 'px';
        }
        this._renderDetailContent();
      } catch (err) {
        this._renderDetailError(err && err.message ? err.message : String(err));
      }
    }

    setViewRef(ref) {
      if (!ref) return;
      this.viewRef = ref;
      this._updateViewRefHeader();
      this._syncToggleButtons();
      this.refresh().catch(err => this._showError(err && err.message ? err.message : String(err)));
    }

    setSessionId(newSid) {
      if (newSid == null) return;
      if (String(this.sessionId) === String(newSid)) return;
      this.sessionId = newSid;
      this.selectedHash = null;
      this.selectedShow = null;
      this.load().catch(err => this._showError(err && err.message ? err.message : String(err)));
    }

    dispose() {
      try { document.removeEventListener('click', this._docClickHandler); } catch (_) {}
      try { document.removeEventListener('keydown', this._docKeyHandler); } catch (_) {}
      try { this.container.innerHTML = ''; } catch (_) {}
    }

    async _fetchLog({ append }) {
      this.loading = true;
      try {
        const params = new URLSearchParams({
          session: String(this.sessionId),
          token: this.token,
          ref: this.viewRef || 'HEAD',
          limit: String(PAGE_LIMIT),
          skip: String(this.skip),
        });
        const res = await fetch(`/api/git-log?${params.toString()}`);
        const data = await res.json().catch(() => ({}));
        if (!res.ok || data.ok === false) {
          throw new Error(data && data.detail ? data.detail : `HTTP ${res.status}`);
        }
        if (!append) this.commits = [];
        const got = Array.isArray(data.commits) ? data.commits : [];
        this.commits = this.commits.concat(got);
        this.headHash      = data.head_hash || '';
        this.sessionBranch = data.branch || '';
        this.hasMore       = !!data.has_more;
        if (data.git_root && !this.gitRoot) {
          this.gitRoot = data.git_root;
          if (this.els.repo) this.els.repo.textContent = this.gitRoot;
        }
      } catch (err) {
        if (!append) this._showError(err.message || String(err));
        throw err;
      } finally {
        this.loading = false;
      }
    }

    async _fetchRefs() {
      try {
        const params = new URLSearchParams({
          session: String(this.sessionId),
          token: this.token,
        });
        const res = await fetch(`/api/git-refs?${params.toString()}`);
        const data = await res.json().catch(() => ({}));
        if (!res.ok || data.ok === false) return;
        this.refs = Array.isArray(data.refs) ? data.refs : [];
        this.githubUrl = data.github_url || '';
        if (data.head) this.sessionBranch = data.head;
        this._updateViewRefHeader();
      } catch (_) { /* noop */ }
    }

    async _fetchStatus() {
      try {
        const params = new URLSearchParams({
          session: String(this.sessionId),
          token: this.token,
        });
        const res = await fetch(`/api/git-status?${params.toString()}`);
        const data = await res.json().catch(() => ({}));
        if (!res.ok || data.ok === false) {
          this.workingTree = null;
          this._renderWorkingTreePreview();
          return;
        }
        this.workingTree = data;
        if (data.git_root && !this.gitRoot) {
          this.gitRoot = data.git_root;
          if (this.els.repo) this.els.repo.textContent = this.gitRoot;
        }
      } catch (_) {
        this.workingTree = null;
      } finally {
        this._renderWorkingTreePreview();
      }
    }

    async _fetchShow(hash) {
      const params = new URLSearchParams({
        session: String(this.sessionId),
        token: this.token,
        hash,
      });
      const res = await fetch(`/api/git-show?${params.toString()}`);
      const data = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(data && data.detail ? data.detail : `HTTP ${res.status}`);
      return data;
    }

    _renderWorkingTreePreview() {
      const wt = this.workingTree;
      const files = wt && Array.isArray(wt.files) ? wt.files : [];
      const changed = wt && wt.summary ? wt.summary.files_changed || files.length : files.length;
      if (this.els.workingCount) {
        this.els.workingCount.textContent = changed
          ? _gt('git_commit_changed_count', '{n} files changed').replace('{n}', String(changed))
          : _gt('git_commit_no_changes', 'No changes');
      }
      if (this.els.commitAllBtn) {
        this.els.commitAllBtn.disabled = !changed;
      }
      if (!this.els.workingPreview) return;
      if (!changed) {
        this.els.workingPreview.innerHTML = `
          <div class="git-working-preview-empty">${_esc(_gt('git_commit_no_changes', 'No changes'))}</div>
        `;
        return;
      }
      const summary = wt.summary || {};
      const max = 8;
      const rows = files.slice(0, max).map(f => {
        const added = f.added == null ? '' : `<span class="file-stat-add">+${f.added || 0}</span>`;
        const removed = f.removed == null ? '' : `<span class="file-stat-del">-${f.removed || 0}</span>`;
        return `
          <div class="git-working-file-row">
            <span class="file-status ${_esc(statusClass(f.status))}">${_esc(f.status || 'M')}</span>
            <span class="file-path">${_esc(f.path || '')}</span>
            ${added}${removed}
          </div>
        `;
      }).join('');
      const more = files.length > max
        ? `<div class="git-working-more">+${files.length - max} ${_esc(_gt('git_commit_more_files', 'more files'))}</div>`
        : '';
      this.els.workingPreview.innerHTML = `
        <div class="git-working-preview-head">
          <span>${_esc(_gt('git_commit_working_tree_preview', 'Working tree preview'))}</span>
          <span class="git-working-summary">${changed} files · +${summary.added || 0} -${summary.removed || 0}</span>
        </div>
        <div class="git-working-files">${rows}${more}</div>
      `;
    }

    _openCommitModal() {
      if (!this.workingTree || !this.workingTree.has_changes) return;
      this.commitModalState = {
        open: true,
        reviewed: false,
        busy: false,
        generating: false,
        error: '',
        subject: '',
        body: '',
      };
      this._renderCommitModalState();
      this.els.commitModal.hidden = false;
      _toast(
        _gt('git_commit_all', 'Commit all'),
        _gt('git_commit_open_toast', 'Commit all will stage all current working tree changes with git add -A, then commit them. Push is not run.')
      );
      setTimeout(() => { try { this.els.commitSubject.focus(); } catch (_) {} }, 0);
    }

    _closeCommitModal() {
      this.commitModalState.open = false;
      this.els.commitModal.hidden = true;
    }

    _renderCommitModalState() {
      const st = this.commitModalState;
      const wt = this.workingTree || {};
      const summary = wt.summary || {};
      if (this.els.commitSummary) {
        const repo = wt.repo_name || this.gitRoot || '';
        const branch = wt.branch || this.sessionBranch || 'HEAD';
        this.els.commitSummary.textContent =
          `${repo} · ${branch} · ${summary.files_changed || 0} files`;
      }
      if (this.els.commitSubject && this.els.commitSubject.value !== st.subject) {
        this.els.commitSubject.value = st.subject;
      }
      if (this.els.commitBody && this.els.commitBody.value !== st.body) {
        this.els.commitBody.value = st.body;
      }
      const hasSubject = (st.subject || '').trim() !== '';
      this.els.commitReview.disabled = !hasSubject || st.busy || st.generating;
      this.els.commitRun.disabled = !hasSubject || !st.reviewed || st.busy || st.generating;
      this.els.commitRun.title = !hasSubject
        ? _gt('git_commit_disabled_no_subject', 'Enter a subject first.')
        : (!st.reviewed ? _gt('git_commit_disabled_needs_review', 'Press Review to enable Commit.') : '');
      this.els.commitReview.classList.toggle('primary', hasSubject && !st.reviewed && !st.busy && !st.generating);
      this.els.commitGenerate.disabled = st.busy || st.generating;
      this.els.commitSubject.disabled = st.busy;
      this.els.commitBody.disabled = st.busy;
      this.els.commitReviewBox.hidden = !st.reviewed;
      if (this.els.commitHint) {
        const showHint = hasSubject && !st.reviewed && !st.busy && !st.generating;
        this.els.commitHint.hidden = !showHint;
        this.els.commitHint.textContent = showHint
          ? _gt('git_commit_review_required_hint', 'Press Review to enable Commit.')
          : '';
      }
      if (st.error) {
        this.els.commitError.hidden = false;
        this.els.commitError.textContent = st.error;
      } else {
        this.els.commitError.hidden = true;
        this.els.commitError.textContent = '';
      }
      this.els.commitGenerate.textContent = st.generating
        ? _gt('git_commit_generating', 'Generating...')
        : _gt('git_commit_generate_message', 'Generate');
      this.els.commitRun.textContent = st.busy
        ? _gt('git_commit_committing', 'Committing...')
        : _gt('git_commit_commit', 'Commit');
    }

    async _generateCommitMessage() {
      const st = this.commitModalState;
      st.generating = true;
      st.error = '';
      this._renderCommitModalState();
      try {
        const res = await fetch('/api/git-commit-message', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            session: this.sessionId,
            token: this.token,
            mode: 'generate',
            language: (localStorage.getItem(STORAGE_LANG_KEY) || 'ja').startsWith('en') ? 'en' : 'ja',
          }),
        });
        const data = await res.json().catch(() => ({}));
        if (!res.ok || data.ok === false) {
          throw new Error(data && data.detail ? data.detail : `HTTP ${res.status}`);
        }
        st.subject = data.subject || '';
        st.body = data.body || '';
        st.reviewed = false;
      } catch (err) {
        st.error = err && err.message ? err.message : String(err);
      } finally {
        st.generating = false;
        this._renderCommitModalState();
      }
    }

    _reviewCommitMessage() {
      const st = this.commitModalState;
      st.subject = this.els.commitSubject.value || '';
      st.body = this.els.commitBody.value || '';
      st.error = '';
      st.reviewed = st.subject.trim() !== '';
      this._renderCommitModalState();
    }

    async _commitAll() {
      const st = this.commitModalState;
      if (!st.reviewed || !(st.subject || '').trim()) return;
      st.busy = true;
      st.error = '';
      this._renderCommitModalState();
      try {
        const res = await fetch('/api/git-commit-all', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            session: this.sessionId,
            token: this.token,
            subject: st.subject,
            body: st.body,
          }),
        });
        const data = await res.json().catch(() => ({}));
        if (!res.ok || data.ok === false) {
          const msg = data && data.error === 'commit_identity_missing'
            ? _gt('git_commit_identity_missing', 'Git user.name / user.email is not configured.')
            : (data && data.detail ? data.detail : `HTTP ${res.status}`);
          throw new Error(msg);
        }
        this._closeCommitModal();
        _toast(_gt('git_commit_created', 'Commit created'), `${data.short_hash || ''} ${data.subject || ''}`.trim());
        await this.refresh();
      } catch (err) {
        st.error = err && err.message ? err.message : String(err);
      } finally {
        st.busy = false;
        this._renderCommitModalState();
      }
    }

    _showLoading() {
      this.els.logTable.innerHTML =
        `<div class="git-graph-loading">${_esc(_gt('git_view_loading', 'Loading...'))}</div>`;
      if (this.els.count) this.els.count.textContent = _gt('git_view_loading', 'Loading...');
    }
    _showError(msg) {
      this.els.logTable.innerHTML =
        `<div class="git-graph-error">${_esc(msg)}</div>`;
      if (this.els.count) this.els.count.textContent = '—';
    }

    _filterCommits() {
      const f = (this.filterText || '').trim().toLowerCase();
      if (!f) return this.commits.slice();
      return this.commits.filter(c =>
        (c.subject || '').toLowerCase().includes(f)
        || (c.author_name || '').toLowerCase().includes(f)
        || (c.author_email || '').toLowerCase().includes(f)
        || (c.hash || '').toLowerCase().includes(f)
        || (c.short_hash || '').toLowerCase().includes(f)
      );
    }

    _renderLogTable() {
      const list = this._filterCommits();
      this.filteredCommits = list;
      this.graphRows = computeGraph(list);

      if (!list.length) {
        this.els.logTable.innerHTML =
          `<div class="git-graph-loading">${_esc(_gt('git_view_no_commits', 'No commits'))}</div>`;
      } else {
        const html = list.map((c, i) => {
          const row = this.graphRows[i] || { lanes: [] };
          const sub = splitSubject(c.subject);
          const isHead = (this.headHash && c.hash === this.headHash);
          const refsHtml = renderRefsInline(c.refs, this.headHash, c.hash);
          return `
            <div class="log-row${isHead ? ' head' : ''}${this.selectedHash === c.hash ? ' selected' : ''}" data-hash="${_esc(c.hash)}">
              <div class="col-graph">${renderGraphSvg(row)}</div>
              <div class="col-refs">${refsHtml}</div>
              <div class="col-subject">${sub.prefix ? `<span class="prefix">${_esc(sub.prefix)}</span>` : ''}${_esc(sub.rest)}</div>
              <div class="col-author">${_esc(c.author_name || '')}</div>
              <div class="col-hash-wrap"><span class="col-hash">${_esc(c.short_hash || _shortHash(c.hash))}</span></div>
              <div class="col-date">${_esc(_formatDate(c.author_date))}</div>
            </div>
          `;
        }).join('');
        this.els.logTable.innerHTML = html;
        this.els.logTable.querySelectorAll('.log-row').forEach(el => {
          el.addEventListener('click', () => {
            const h = el.dataset.hash;
            this.selectCommit(h).catch(() => {});
          });
          el.addEventListener('contextmenu', (e) => {
            e.preventDefault();
            const h = el.dataset.hash;
            const c = this.commits.find(x => x.hash === h);
            if (c) _openCtxMenu(e.clientX, e.clientY, c, el, this.githubUrl);
          });
        });
        this.els.logTable.onscroll = () => _closeCtxMenu();
      }

      const total = this.commits.length;
      const fcount = list.length;
      const filtered = (this.filterText || '').trim() !== '';
      if (this.els.count) {
        if (filtered) {
          this.els.count.textContent = `${fcount}/${total} commits (filtered)`;
        } else {
          this.els.count.textContent = `${total} commits${this.hasMore ? ' · has more' : ''}`;
        }
      }
      if (this.els.loadmoreBtn) this.els.loadmoreBtn.disabled = !this.hasMore || filtered;
    }

    _highlightSelected() {
      if (!this.els.logTable) return;
      this.els.logTable.querySelectorAll('.log-row.selected').forEach(el => el.classList.remove('selected'));
      if (this.selectedHash) {
        const sel = this.els.logTable.querySelector(`.log-row[data-hash="${CSS.escape(this.selectedHash)}"]`);
        if (sel) sel.classList.add('selected');
      }
    }

    _renderDetailEmpty() {
      this.els.detailMeta.textContent = '—';
      this.els.detailContent.innerHTML =
        `<div class="detail-empty">${_esc(_gt('git_view_select_row_hint', 'Click a row to see commit detail'))}</div>`;
    }
    _renderDetailError(msg) {
      this.els.detailContent.innerHTML =
        `<div class="git-graph-error">${_esc(msg)}</div>`;
    }
    _toggleDetailPanel() {
      const p = this.els.detailPanel;
      if (p.classList.contains('collapsed')) {
        p.classList.remove('collapsed');
        p.style.height = (this.panelHeight || 340) + 'px';
      } else {
        const h = p.getBoundingClientRect().height;
        if (h > 60) this.panelHeight = h;
        p.classList.add('collapsed');
      }
    }
    _syncDetailTabs() {
      this.els.detailTabs.forEach(t => {
        t.classList.toggle('active', t.dataset.tab === this.activeTab);
      });
    }
    _renderDetailContent() {
      const c = this.selectedShow;
      if (!c) { this._renderDetailEmpty(); return; }
      this.els.detailMeta.textContent =
        `${_shortHash(c.hash)} · ${c.author_name || ''} · ${_formatDate(c.author_date)}`;
      if (this.activeTab === 'info')         this.els.detailContent.innerHTML = this._renderInfoTab(c);
      else if (this.activeTab === 'changes') this.els.detailContent.innerHTML = this._renderChangesTab(c);
      else if (this.activeTab === 'files')   this.els.detailContent.innerHTML = this._renderFilesTab(c);
      this._wireDetailLinks(c);
    }

    _renderInfoTab(c) {
      const sub = splitSubject(c.subject);
      const subjectFull = (sub.prefix ? sub.prefix + ' ' : '') + sub.rest;
      const initial = ((c.author_name || '?')[0] || '?').toUpperCase();
      const parents = Array.isArray(c.parents) ? c.parents : [];
      const refs = c.refs || [];
      const refsHtml = refs.length
        ? renderRefsInline(refs, this.headHash, c.hash)
        : `<span style="color:var(--muted);font-size:11.5px">—</span>`;
      const files = Array.isArray(c.files) ? c.files : [];
      const filesSection = files.length
        ? `<div class="info-label">FILES</div><div class="info-value">${files.map(f => `
            <div class="file-row" data-file-action="changes" data-path="${_esc(f.path)}">
              <span class="file-status ${_esc(f.status || 'M')}">${_esc(f.status || 'M')}</span>
              <span class="file-path">${_esc(f.path || '')}</span>
              <span class="file-stat-add">+${f.added || 0}</span>
              <span class="file-stat-del">-${f.removed || 0}</span>
            </div>`).join('')}</div>`
        : `<div class="info-label">FILES</div><div class="info-value" style="color:var(--muted);font-size:11.5px">—</div>`;

      const ghDisabled = this.githubUrl ? '' : 'disabled';
      const bodyHtml = c.body
        ? _esc(c.body)
        : `<span style="color:var(--muted);font-size:11.5px">${_esc(_gt('git_view_no_body', '(no body)'))}</span>`;

      return `
        <div class="info-grid">
          <div class="info-label">AUTHOR</div>
          <div class="info-value info-author">
            <div class="info-avatar">${_esc(initial)}</div>
            <div>
              <div><span class="info-author-name">${_esc(c.author_name || '')}</span><span class="info-author-email">${_esc(c.author_email || '')}</span></div>
              <div class="info-author-date">${_esc(_formatDate(c.author_date))}</div>
            </div>
          </div>

          <div class="info-label">SHA</div>
          <div class="info-value info-sha">
            <span class="info-sha-value">${_esc(c.hash || '')}</span>
            <button class="info-mini-btn" data-action="copy-full">⎘ Copy</button>
            <button class="info-mini-btn" data-action="copy-github-url" ${ghDisabled}>↗ GH</button>
          </div>

          <div class="info-label">PARENTS</div>
          <div class="info-value">${parents.length
            ? parents.map(p => `<span class="info-parent-link" data-parent="${_esc(p)}">${_esc(_shortHash(p))}</span>`).join('')
            : `<span style="color:var(--muted);font-size:11.5px">(root commit)</span>`}</div>

          <div class="info-label">REFS</div>
          <div class="info-value">${refsHtml}</div>

          <div class="info-label">MESSAGE</div>
          <div class="info-value"><div class="info-message"><span class="subject">${_esc(subjectFull)}</span>${bodyHtml}</div></div>

          ${filesSection}
        </div>
      `;
    }

    _renderFilesTab(c) {
      const files = Array.isArray(c.files) ? c.files : [];
      if (!files.length) return `<div class="detail-empty">${_esc(_gt('git_view_no_files', 'No file changes'))}</div>`;
      return files.map(f => `
        <div class="file-row" data-file-action="changes" data-path="${_esc(f.path)}">
          <span class="file-status ${_esc(f.status || 'M')}">${_esc(f.status || 'M')}</span>
          <span class="file-path">${_esc(f.path || '')}</span>
          <span class="file-stat-add">+${f.added || 0}</span>
          <span class="file-stat-del">-${f.removed || 0}</span>
        </div>
      `).join('');
    }

    _renderChangesTab(c) {
      const files = Array.isArray(c.files) ? c.files : [];
      if (!files.length) return `<div class="detail-empty">${_esc(_gt('git_view_no_diff', 'No diff'))}</div>`;
      return files.map(f => {
        const diff = (f.diff || '').split('\n').map(line => {
          let cls = 'ctx';
          if (line.startsWith('@@'))      cls = 'hunk';
          else if (line.startsWith('+++') || line.startsWith('---')) cls = 'ctx';
          else if (line.startsWith('+'))  cls = 'add';
          else if (line.startsWith('-'))  cls = 'del';
          return `<span class="diff-line ${cls}">${_esc(line) || ' '}</span>`;
        }).join('');
        return `
          <div class="diff-file">
            <div class="diff-file-header">
              <span class="file-status ${_esc(f.status || 'M')}" style="width:16px;height:16px;font-size:10px">${_esc(f.status || 'M')}</span>
              <span style="flex:1">${_esc(f.path || '')}</span>
              <span class="file-stat-add">+${f.added || 0}</span>
              <span class="file-stat-del">-${f.removed || 0}</span>
            </div>
            <div class="diff-body">${diff}</div>
          </div>
        `;
      }).join('');
    }

    _wireDetailLinks(c) {
      const content = this.els.detailContent;
      const ghBase = this.githubUrl;
      content.querySelectorAll('[data-action]').forEach(btn => {
        btn.addEventListener('click', () => {
          _ctxGithubBase = ghBase;
          _ctxCopy(btn.dataset.action, c);
        });
      });
      content.querySelectorAll('[data-parent]').forEach(link => {
        link.addEventListener('click', () => this._jumpToCommit(link.dataset.parent));
      });
      content.querySelectorAll("[data-file-action='changes']").forEach(row => {
        row.addEventListener('click', () => {
          this.activeTab = 'changes';
          this._syncDetailTabs();
          this._renderDetailContent();
        });
      });
    }

    _jumpToCommit(hash) {
      const target = this.commits.find(x => x.hash === hash);
      if (!target) {
        _toast(_gt('git_view_parent_not_in_list', 'Parent commit not in list'),
               `${_shortHash(hash)}`);
        return;
      }
      const el = this.els.logTable.querySelector(`.log-row[data-hash="${CSS.escape(hash)}"]`);
      if (el) {
        this.selectCommit(hash).catch(() => {});
        try { el.scrollIntoView({ block: 'center', behavior: 'smooth' }); } catch (_) {}
      }
    }

    _setupDividerDrag() {
      const divider = this.els.divider;
      const panel   = this.els.detailPanel;
      let startY = 0, startH = 0, dragging = false;
      const onMove = (e) => {
        if (!dragging) return;
        const dy = startY - e.clientY;
        const newH = Math.max(0, Math.min(window.innerHeight - 180, startH + dy));
        panel.style.height = newH + 'px';
        if (newH > 60) {
          panel.classList.remove('collapsed');
          this.panelHeight = newH;
        }
      };
      const onUp = () => {
        if (!dragging) return;
        dragging = false;
        divider.classList.remove('dragging');
        document.body.style.cursor = '';
        if (parseInt(panel.style.height || '0', 10) < 40) {
          panel.classList.add('collapsed');
        }
      };
      divider.addEventListener('mousedown', (e) => {
        dragging = true;
        startY = e.clientY;
        startH = panel.getBoundingClientRect().height;
        divider.classList.add('dragging');
        document.body.style.cursor = 'row-resize';
        e.preventDefault();
      });
      document.addEventListener('mousemove', onMove);
      document.addEventListener('mouseup', onUp);
    }

    _syncToggleButtons() {
      const v = this.viewRef;
      this.els.toggles.forEach(b => {
        const target = b.dataset.toggle;
        b.classList.toggle('active', target === v);
      });
    }

    _openRefDropdown() {
      const dd = this.els.refDropdown;
      const btn = this.els.viewRefBtn;
      const r = btn.getBoundingClientRect();
      dd.style.left = r.left + 'px';
      dd.style.top  = (r.bottom + 4) + 'px';
      dd.classList.add('open');
      this._refDropdownOpen = true;
      this._renderRefList('');
      if (this.els.refFilter) {
        this.els.refFilter.value = '';
        setTimeout(() => { try { this.els.refFilter.focus(); } catch (_) {} }, 0);
      }
    }
    _closeRefDropdown() {
      this.els.refDropdown.classList.remove('open');
      this._refDropdownOpen = false;
    }
    _renderRefList(filter) {
      const f = (filter || '').trim().toLowerCase();
      const KIND_ICON = { local: '⎇', remote: '↑', tag: '🏷', special: '★' };
      const special = [
        { kind: 'special', name: 'HEAD',  hash: '' },
        { kind: 'special', name: '--all', hash: '' },
      ];
      const grouped = { local: [], remote: [], tag: [] };
      for (const r of (this.refs || [])) {
        const k = (r.kind || 'local').toLowerCase();
        if (k === 'head') continue;
        if (grouped[k]) grouped[k].push(r);
      }
      const sections = [
        { kind: 'special', label: _gt('git_view_ref_section_special', 'Special'), items: special },
        { kind: 'local',   label: _gt('git_view_ref_section_local',   'Local branches'), items: grouped.local },
        { kind: 'remote',  label: _gt('git_view_ref_section_remote',  'Remote branches'), items: grouped.remote },
        { kind: 'tag',     label: _gt('git_view_ref_section_tag',     'Tags'), items: grouped.tag },
      ];
      const matches = (r) => !f || (r.name || '').toLowerCase().includes(f);

      const html = sections.map(sec => {
        const items = sec.items.filter(matches);
        if (!items.length) return '';
        return `<div class="ref-section">${_esc(sec.label)}</div>` + items.map(r => {
          const isActive = (r.name === this.viewRef);
          return `
            <div class="ref-item${isActive ? ' active' : ''}" data-ref="${_esc(r.name)}">
              <span class="ref-check">${isActive ? '✓' : ''}</span>
              <span class="ref-name ${_esc(r.kind || 'local')}"><span class="ico">${_esc(KIND_ICON[r.kind] || '?')}</span>${_esc(r.name)}</span>
              ${r.hash ? `<span class="ref-hash">${_esc(_shortHash(r.hash))}</span>` : '<span></span>'}
            </div>
          `;
        }).join('');
      }).join('');

      this.els.refList.innerHTML = html ||
        `<div class="ref-section" style="opacity:0.6">${_esc(_gt('git_view_ref_no_match', 'No match'))}</div>`;
      this.els.refList.querySelectorAll('.ref-item').forEach(el => {
        el.addEventListener('click', () => {
          this._closeRefDropdown();
          this.setViewRef(el.dataset.ref);
        });
      });
    }

    _updateViewRefHeader() {
      const label = this.els.viewRefLabel;
      const btn   = this.els.viewRefBtn;
      const chip  = this.els.sessionHeadChip;
      const chipLabel = this.els.sessionHeadLabel;
      if (!label || !btn || !chip) return;

      const displayName = (this.viewRef === 'HEAD') ? (this.sessionBranch || 'HEAD') : this.viewRef;
      label.textContent = displayName;

      const sessionHead = this.sessionBranch || '';
      const diverged = (this.viewRef !== 'HEAD' &&
                        this.viewRef !== sessionHead &&
                        this.viewRef !== '--all' &&
                        sessionHead !== '');
      btn.classList.toggle('diverged', diverged);
      chip.style.display = diverged ? '' : 'none';
      if (chipLabel) chipLabel.textContent = sessionHead;
    }
  }

  window.GitGraphView = GitGraphView;
})();
