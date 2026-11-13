// 音声入力エンジン（vtype-core）の生成。ブラウザ内蔵認識（SpeechRecognition）だけを作る。
// 画面（ボタン・バー・波形・トースト・ショートカット）は voice.ts が持つ。
//
// vtype-core は src/vendor/vtype-core/ に置いた写しを読む（scripts/sync-vtype-core.mjs を参照）。
import { appLangToRecognitionLang, createVoiceInput } from '../vendor/vtype-core/index.js';
import { STORAGE_LANG_KEY, getVoiceEngine } from './user-prefs.js';

export const voiceInput = createVoiceInput({
  engine: () => getVoiceEngine(),
  recognition: {
    lang: () => appLangToRecognitionLang(localStorage.getItem(STORAGE_LANG_KEY)),
    getAppVersion: () => document.querySelector('.settings-app-version')?.textContent || null,
  },
  // ウェイクワードは無効化中のため作らない（旧 voice.ts でも早期 return で止めていた）。
});
