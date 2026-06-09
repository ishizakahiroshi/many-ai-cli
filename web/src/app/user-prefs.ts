// --- ESM imports (generated) ---
import { showToast, token } from './util.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- 設定パネル ----

export const STORAGE_THEME_KEY      = 'ai_cli_hub_theme';
export const STORAGE_FONTSIZE_KEY   = 'ai_cli_hub_fontsize';
export const STORAGE_LANG_KEY       = 'ai_cli_hub_lang';
export const STORAGE_FAVORITES_KEY         = 'ai_cli_hub_favorites';
export const STORAGE_ORDER_KEY             = 'ai_cli_hub_session_order';
export const STORAGE_GROUP_ORDER_KEY       = 'ai_cli_hub_group_order';
export const STORAGE_PROJECT_FAVORITES_KEY = 'ai_cli_hub_project_favorites';
export const STORAGE_SPAWN_KEY             = 'ai_cli_hub_spawn_settings';
export const STORAGE_CWD_HISTORY_KEY       = 'ai_cli_hub_cwd_history';
export const STORAGE_TRIGGER_ENABLED_KEY      = 'ai_cli_hub_trigger_enabled';
export const STORAGE_TRIGGER_PHRASE_KEY       = 'ai_cli_hub_trigger_phrase';
export const STORAGE_NOTIFY_SOUND_ENABLED_KEY = 'ai_cli_hub_notify_sound_enabled';
export const STORAGE_NOTIFY_SOUND_TYPE_KEY    = 'ai_cli_hub_notify_sound_type';
export const STORAGE_NOTIFY_SOUND_CUSTOM_KEY  = 'ai_cli_hub_notify_sound_custom';
export const STORAGE_DESKTOP_NOTIFY_ENABLED_KEY = 'ai_cli_hub_desktop_notify_enabled';
export const STORAGE_PUSH_NOTIFY_ENABLED_KEY = 'ai_cli_hub_push_notify_enabled';
export const STORAGE_APPROVAL_AUTO_SWITCH_KEY = 'ai_cli_hub_approval_auto_switch';
export const STORAGE_QUICK_CMD_1_KEY          = 'ai_cli_hub_quick_cmd_1';
export const STORAGE_QUICK_CMD_2_KEY          = 'ai_cli_hub_quick_cmd_2';
export const STORAGE_TOOLS_LEFT_KEY           = 'ai_cli_hub_tools_left';
export const STORAGE_MOBILE_INPUT_TOOLS_KEY   = 'ai_cli_hub_mobile_input_tools';
export const STORAGE_USAGE_LINK_CLAUDE_KEY    = 'ai_cli_hub_usage_link_claude';
export const STORAGE_USAGE_LINK_CODEX_KEY     = 'ai_cli_hub_usage_link_codex';
export const STORAGE_USAGE_LINK_COPILOT_KEY   = 'ai_cli_hub_usage_link_copilot';
export const STORAGE_USAGE_LINK_CURSOR_AGENT_KEY = 'ai_cli_hub_usage_link_cursor_agent';
export const STORAGE_USAGE_LINK_OLLAMA_KEY    = 'ai_cli_hub_usage_link_ollama';
export const STORAGE_USAGE_LINK_OPENCODE_KEY  = 'ai_cli_hub_usage_link_opencode';
export const STORAGE_VOICE_GRACE_KEY          = 'ai_cli_hub_voice_grace_seconds';
export const STORAGE_VOICE_INPUT_DISABLED_KEY = 'ai_cli_hub_voice_input_disabled';
export const STORAGE_VOICE_ENGINE_KEY         = 'anyai.voiceEngine';
export const STORAGE_VOICE_WHISPER_AUTO_SUBMIT_KEY = 'anyai.voiceWhisperAutoSubmit';
export const STORAGE_DISPLAY_LOCKED_MODE_KEY  = 'ai_cli_hub_display_locked_mode';
export const DEFAULT_VOICE_GRACE_SEC          = 0;
export const STORAGE_WAKE_WORD_ENABLED_KEY    = 'ai_cli_hub_wake_word_enabled';
export const STORAGE_WAKE_WORD_PHRASE_KEY     = 'ai_cli_hub_wake_word_phrase';
export const DEFAULT_WAKE_WORD_PHRASE_JA      = 'サウンドスタート';
export const DEFAULT_WAKE_WORD_PHRASE_EN      = 'SoundStart';
export const DEFAULT_TRIGGER_PHRASE_JA        = 'サウンドエンド';
export const DEFAULT_TRIGGER_PHRASE_EN        = 'SoundEnd';
export function getDefaultWakeWordPhrase() {
  const lang = (typeof localStorage !== 'undefined' && localStorage.getItem(STORAGE_LANG_KEY)) || 'ja';
  return lang === 'en' ? DEFAULT_WAKE_WORD_PHRASE_EN : DEFAULT_WAKE_WORD_PHRASE_JA;
}
export function getDefaultTriggerPhrase() {
  const lang = (typeof localStorage !== 'undefined' && localStorage.getItem(STORAGE_LANG_KEY)) || 'ja';
  return lang === 'en' ? DEFAULT_TRIGGER_PHRASE_EN : DEFAULT_TRIGGER_PHRASE_JA;
}
export const CWD_HISTORY_MAX               = 10;

export type VoiceEngine = 'off' | 'browser' | 'whisper';

export function normalizeVoiceEngine(value: any): VoiceEngine {
  if (value === 'off' || value === 'browser' || value === 'whisper') return value;
  return 'browser';
}

export function getVoiceEngine(): VoiceEngine {
  const explicit = localStorage.getItem(STORAGE_VOICE_ENGINE_KEY);
  if (explicit != null) {
    const normalized = normalizeVoiceEngine(explicit);
    if (normalized !== explicit) localStorage.setItem(STORAGE_VOICE_ENGINE_KEY, normalized);
    return normalized;
  }
  const migrated = localStorage.getItem(STORAGE_VOICE_INPUT_DISABLED_KEY) === '1' ? 'off' : 'browser';
  localStorage.setItem(STORAGE_VOICE_ENGINE_KEY, migrated);
  return migrated;
}

export function setVoiceEngine(value: any): VoiceEngine {
  const engine = normalizeVoiceEngine(value);
  localStorage.setItem(STORAGE_VOICE_ENGINE_KEY, engine);
  document.dispatchEvent(new CustomEvent('voiceengine:changed', { detail: { engine } }));
  return engine;
}

try { getVoiceEngine(); } catch (_) {}

export const DEFAULT_USAGE_LINKS = {
  claude:   'https://claude.ai/settings/usage',
  codex:    'https://chatgpt.com/codex/cloud/settings/analytics#usage',
  copilot:  'https://github.com/settings/billing',
  'cursor-agent': 'https://cursor.com/dashboard',
  ollama:   'https://ollama.com/settings',
  opencode: '',
};

export const FONTSIZE_MAP = { large: 15, medium: 13, small: 11 };

type UserPrefsObject = Record<string, any>;
type UserPrefSerializer = (value: any) => string;
type UserPrefsPathMap = Record<string, readonly [string, UserPrefSerializer]>;
type UserPrefsHttpError = Error & { phase: string; status: number };

// ---- user-prefs サーバ同期 ----
// setUserPref(path, value)
//   path: ドット区切りの user_prefs パス（例: 'voice.wake_word_phrase'）
//   value: 任意の JSON 値
//   - 対応 localStorage キーに書く
//   - サーバへ 200ms debounced PUT（取得→パス更新→PUT 全体置換）
//   - PUT 失敗時は console.warn + トースト。localStorage は保持する

export const _USER_PREFS_PATH_TO_LS: UserPrefsPathMap = {
  'trigger.enabled':           [STORAGE_TRIGGER_ENABLED_KEY,       (v) => v ? '1' : '0'],
  'trigger.phrase':            [STORAGE_TRIGGER_PHRASE_KEY,         String],
  'notify_sound.enabled':      [STORAGE_NOTIFY_SOUND_ENABLED_KEY,  (v) => v ? '1' : '0'],
  'notify_sound.type':         [STORAGE_NOTIFY_SOUND_TYPE_KEY,     String],
  'desktop_notifications.enabled': [STORAGE_DESKTOP_NOTIFY_ENABLED_KEY, (v) => v ? '1' : '0'],
  'push_notifications.enabled': [STORAGE_PUSH_NOTIFY_ENABLED_KEY, (v) => v ? '1' : '0'],
  // notify_sound.custom_file はサーバ上のファイルパスのため localStorage へはミラーしない
  // （カスタム音はサーバ API /api/user-prefs/notify-sound-custom 経由で再生する）
  'voice.grace_seconds':       [STORAGE_VOICE_GRACE_KEY,           String],
  'voice.input_disabled':      [STORAGE_VOICE_INPUT_DISABLED_KEY,  (v) => v ? '1' : '0'],
  'voice.wake_word_enabled':   [STORAGE_WAKE_WORD_ENABLED_KEY,     (v) => v ? '1' : '0'],
  'voice.wake_word_phrase':    [STORAGE_WAKE_WORD_PHRASE_KEY,      String],
  'quick_cmds.cmd1':           [STORAGE_QUICK_CMD_1_KEY,           String],
  'quick_cmds.cmd2':           [STORAGE_QUICK_CMD_2_KEY,           String],
  'usage_links.claude':        [STORAGE_USAGE_LINK_CLAUDE_KEY,     String],
  'usage_links.codex':         [STORAGE_USAGE_LINK_CODEX_KEY,      String],
  'usage_links.copilot':       [STORAGE_USAGE_LINK_COPILOT_KEY,    String],
  'usage_links.cursor-agent':  [STORAGE_USAGE_LINK_CURSOR_AGENT_KEY, String],
  'usage_links.ollama':        [STORAGE_USAGE_LINK_OLLAMA_KEY,     String],
  'usage_links.opencode':      [STORAGE_USAGE_LINK_OPENCODE_KEY,   String],
  'favorites':                 [STORAGE_FAVORITES_KEY,             JSON.stringify],
  'session_order':             [STORAGE_ORDER_KEY,                 JSON.stringify],
  'group_order':               [STORAGE_GROUP_ORDER_KEY,           JSON.stringify],
  'project_favorites':         [STORAGE_PROJECT_FAVORITES_KEY,     JSON.stringify],
  'cwd_history':               [STORAGE_CWD_HISTORY_KEY,           JSON.stringify],
  'approval.auto_switch':      [STORAGE_APPROVAL_AUTO_SWITCH_KEY,  (v) => v ? '1' : '0'],
  'mobile.input_tools_enabled': [STORAGE_MOBILE_INPUT_TOOLS_KEY,   (v) => v ? '1' : '0'],
  'spawn.defaults':            [STORAGE_SPAWN_KEY,                 JSON.stringify],
  'display.locked_mode':       [STORAGE_DISPLAY_LOCKED_MODE_KEY,   (v) => (v == null || v === '') ? '' : String(v)],
  'display.theme':             [STORAGE_THEME_KEY,                 String],
  'display.font_size':         [STORAGE_FONTSIZE_KEY,              String],
  'display.lang':              [STORAGE_LANG_KEY,                  String],
};
// Voice engine selection is intentionally absent from server-synced user prefs.
// It must stay device-local so PC can use browser recognition while iPhone uses Whisper.

export const _USER_PREFS_STRING_PATHS = new Set([
  'trigger.phrase',
  'notify_sound.type',
  'voice.wake_word_phrase',
  'quick_cmds.cmd1',
  'quick_cmds.cmd2',
  'usage_links.claude',
  'usage_links.codex',
  'usage_links.copilot',
  'usage_links.cursor-agent',
  'usage_links.ollama',
  'usage_links.opencode',
  'display.locked_mode',
  'display.theme',
  'display.font_size',
  'display.lang',
]);
export const _USER_PREFS_STRING_ARRAY_PATHS = new Set([
  'favorites',
  'session_order',
  'group_order',
  'project_favorites',
  'cwd_history',
]);

// ドット区切りパスでオブジェクトの深いフィールドを設定する
export function _setNestedValue(obj: UserPrefsObject, path: string, value: any): void {
  const keys = path.split('.');
  let cur = obj;
  for (let i = 0; i < keys.length - 1; i++) {
    if (cur[keys[i]] == null || typeof cur[keys[i]] !== 'object') cur[keys[i]] = {};
    cur = cur[keys[i]];
  }
  cur[keys[keys.length - 1]] = value;
}

export function _parseStoredUserPref(path: string, raw: string): { ok: true; value: any } | { ok: false; value?: never } {
  let parsed: any;
  try { parsed = JSON.parse(raw); } catch (_) { parsed = raw; }

  if (path.endsWith('.enabled') || path === 'voice.wake_word_enabled' || path === 'voice.input_disabled' || path === 'approval.auto_switch') {
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
    const value: Record<string, string> = {};
    for (const [k, v] of Object.entries(parsed)) {
      if (typeof k === 'string' && typeof v === 'string') value[k] = v;
    }
    return { ok: true, value };
  }
  return { ok: true, value: parsed };
}

export function _mergeStoredUserPrefs(current: UserPrefsObject): UserPrefsObject {
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
export let _userPrefsDebounceTimer: ReturnType<typeof setTimeout> | null = null;

export function _userPrefsSaveErrorMessage(err: unknown): string {
  const tfn = typeof window.t === 'function' ? window.t : (key: string) => key;
  const httpErr = err as Partial<UserPrefsHttpError> | null | undefined;
  if (typeof httpErr?.status === 'number') {
    if (httpErr.status === 401) return tfn('user_prefs_save_failed_unauthorized');
    if (httpErr.status >= 500) return tfn('user_prefs_save_failed_server', { status: String(httpErr.status) });
    return tfn('user_prefs_save_failed_http', { status: String(httpErr.status) });
  }
  return tfn('user_prefs_save_failed_network');
}

export function _userPrefsHttpError(phase: string, res: Response): UserPrefsHttpError {
  const err = new Error(`${phase} /api/user-prefs ${res.status}`) as UserPrefsHttpError;
  err.phase = phase;
  err.status = res.status;
  return err;
}

// _putUserPrefsNow は localStorage の最新値をサーバへ即時（awaited）反映する。
// 言語変更時の location.reload() 前や reset 完了時など、debounce を待てない場面で使う。
export async function _putUserPrefsNow() {
  const tk = token;
  // 現在のサーバ値を取得してからパッチ適用し全体置換
  const getRes = await fetch(`/api/user-prefs?token=${encodeURIComponent(tk || '')}`);
  if (!getRes.ok) throw _userPrefsHttpError('GET', getRes);
  const current = await getRes.json();
  // localStorage の最新値を current にマージ
  _mergeStoredUserPrefs(current);
  const putRes = await fetch(`/api/user-prefs?token=${encodeURIComponent(tk || '')}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(current),
  });
  if (!putRes.ok) throw _userPrefsHttpError('PUT', putRes);
}

export function _scheduleUserPrefsPut() {
  if (_userPrefsDebounceTimer) clearTimeout(_userPrefsDebounceTimer);
  _userPrefsDebounceTimer = setTimeout(async () => {
    try {
      await _putUserPrefsNow();
    } catch (e) {
      console.warn('[user-prefs] PUT failed:', e);
      showToast(_userPrefsSaveErrorMessage(e));
    }
  }, 200);
}

export function setUserPref(path: string, value: any): void {
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
export async function _mirrorUserPrefsFromServer() {
  const tk = token;
  try {
    const res = await fetch(`/api/user-prefs?token=${encodeURIComponent(tk || '')}`);
    if (!res.ok) return null;
    const prefs: UserPrefsObject = await res.json();
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
export async function migrateLocalstoragePrefsToServer() {
  const tk = token;
  try {
    const res = await fetch(`/api/user-prefs?token=${encodeURIComponent(tk || '')}`);
    if (!res.ok) return;
    const prefs: UserPrefsObject = await res.json();
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
    await fetch(`/api/user-prefs?token=${encodeURIComponent(tk || '')}`, {
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
  } else if (serverPrefs) {
    // 既に移行済みのユーザー向けバックフィル: 後から user_prefs に追加した
    // display.theme/font_size/lang がサーバに無く localStorage にだけある場合、
    // 一度だけサーバへ移送する（setUserPref の debounce PUT で永続化）。
    const disp: UserPrefsObject = serverPrefs.display || {};
    for (const [path, lsKey] of [
      ['display.theme',     STORAGE_THEME_KEY],
      ['display.font_size', STORAGE_FONTSIZE_KEY],
      ['display.lang',      STORAGE_LANG_KEY],
    ]) {
      const field = path.split('.')[1];
      const lsVal = localStorage.getItem(lsKey);
      if (lsVal != null && (disp[field] == null || disp[field] === '')) {
        setUserPref(path, lsVal);
      }
    }
  }
})();
