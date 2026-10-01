// 音声入力エンジン（vtype-core）の生成。ブラウザ内蔵認識と Whisper の 2 エンジンを 1 か所で作り、
// マイクの取り合い（SpeechRecognition と getUserMedia の併用）を core 側の排他に任せる。
// 画面（ボタン・バー・波形・トースト・ショートカット）は voice.ts / voice-whisper.ts が持つ。
//
// vtype-core は src/vendor/vtype-core/ に置いた写しを読む（scripts/sync-vtype-core.mjs を参照）。
import { appLangToRecognitionLang, createVoiceInput, graceSecondsToSilenceMs } from '../vendor/vtype-core/index.js';
import type { FetchLike } from '../vendor/vtype-core/index.js';
import { apiFetch } from './util.js';

// vtype-core's FetchInitLike.body is unknown; bridge to apiFetch (Cookie + Bearer).
const whisperApiFetch: FetchLike = (url, init) =>
  apiFetch(url, {
    method: init.method,
    headers: init.headers,
    body: init.body as BodyInit,
    signal: init.signal as AbortSignal | undefined,
  });
import {
  DEFAULT_VOICE_GRACE_SEC,
  STORAGE_LANG_KEY,
  STORAGE_VOICE_GRACE_KEY,
  STORAGE_VOICE_WHISPER_AUTO_STOP_KEY,
  getVoiceEngine,
} from './user-prefs.js';

// worklet モジュールは同一オリジンの静的 JS として配信する（web/src/whisper-recorder-worklet.js）。
// blob URL を addModule() すると Hub の CSP script-src 'self' に止められるため static 配信にしている。
const RECORDER_WORKLET_URL = '/whisper-recorder-worklet.js';

export const voiceInput = createVoiceInput({
  engine: () => getVoiceEngine(),
  recognition: {
    lang: () => appLangToRecognitionLang(localStorage.getItem(STORAGE_LANG_KEY)),
    getAppVersion: () => document.querySelector('.settings-app-version')?.textContent || null,
  },
  whisper: {
    endpoint: '/api/voice/transcribe',
    // Cookie / Bearer via apiFetch — do not put Hub token in ?token= (F-WEB-07).
    fetch: whisperApiFetch,
    recorderWorklet: { url: RECORDER_WORKLET_URL, processorName: 'many-ai-cli-whisper-recorder' },
    autoStop: () => localStorage.getItem(STORAGE_VOICE_WHISPER_AUTO_STOP_KEY) !== '0',
    autoStopSilenceMs: () => graceSecondsToSilenceMs(localStorage.getItem(STORAGE_VOICE_GRACE_KEY), DEFAULT_VOICE_GRACE_SEC),
  },
  // ウェイクワードは無効化中のため作らない（旧 voice.ts でも早期 return で止めていた）。
});
