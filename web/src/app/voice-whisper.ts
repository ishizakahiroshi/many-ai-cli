// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { showToast } from './util.js';
import { STORAGE_VOICE_ENGINE_KEY, STORAGE_VOICE_WHISPER_AUTO_SUBMIT_KEY, getVoiceEngine } from './user-prefs.js';
import { activeSessionId, terminals } from './state.js';
import { autoExpand, buildSendText, doSend, inputEl, set_voiceActive, set_voiceAudioActive, updateInputClearButton, updateSlashMenu, voiceAudioActive } from '../app.js';
import { isTerminalAtBottom, refitActiveTerminalAfterLayout } from './terminal.js';
import { getActiveTriggerPhrase, textEndsWithTriggerPhrase } from './settings.js';
import { joinTranscript } from '../vendor/vtype-core/index.js';
import { voiceInput } from './voice-engine.js';

// ---- 音声入力（Whisper）の画面側 ----
// 録音・無音での自動確定・WAV 化・Hub への送信は vtype-core が持つ（送り先と token は
// voice-engine.ts で渡す）。ここはボタン・音声バー・波形・トースト・ショートカットだけを扱う。
(function () {
  const wh = voiceInput.whisper;
  const btn = document.getElementById('voice-btn');
  const voiceBar = document.getElementById('voice-bar');
  const canvas = document.getElementById('voice-waveform') as HTMLCanvasElement;
  const cancelBtn = document.getElementById('voice-cancel-btn');
  const confirmBtn = document.getElementById('voice-confirm-btn');

  if (!wh || !btn || !voiceBar || !canvas || !cancelBtn || !confirmBtn || !inputEl) return;

  const supportsWhisperRecording = wh.support.supported;

  let preVoiceText = '';
  // 確定（送信）が失敗・取り消しで終わったことを error → stop の間だけ覚えておく。
  let finishFailed = false;
  let waveformRaf: number | null = null;
  let animFrame: number | null = null;
  let wavePhase = 0;
  let voiceIntensity = 0;
  let voiceIntensityTarget = 0;

  function tr(key: string, fallback: string, vars: Record<string, string> = {}) {
    let msg = t(key);
    if (!msg || msg === key) msg = fallback;
    for (const [k, v] of Object.entries(vars)) msg = msg.replace(`{${k}}`, v);
    return msg;
  }

  function updateButtonVisibility() {
    const engine = getVoiceEngine();
    if (engine === 'whisper' && supportsWhisperRecording) {
      btn.dataset.voiceWhisperSupported = '1';
      btn.dataset.voiceSupported = '1';
      btn.hidden = false;
      btn.dataset.tooltip = tr('voice_tooltip_whisper', 'Voice input (Whisper)');
      return;
    }
    if (engine === 'off') {
      btn.hidden = true;
      return;
    }
    if (engine === 'browser') {
      btn.hidden = btn.dataset.voiceBrowserSupported !== '1';
      if (btn.dataset.voiceBrowserSupported === '1') btn.dataset.voiceSupported = '1';
    }
  }

  function showVoiceError(code: string) {
    const normalized = String(code || 'unknown').trim() || 'unknown';
    const byCode: Record<string, string> = {
      whisper_not_configured: tr('voice_whisper_error_not_configured', 'Whisper server is not configured'),
      whisper_unreachable: tr('voice_whisper_error_unreachable', 'Cannot reach the Whisper server'),
      whisper_timeout: tr('voice_whisper_error_timeout', 'Whisper transcription timed out'),
      whisper_failed: tr('voice_whisper_error_failed', 'Whisper transcription failed'),
      no_speech: tr('voice_whisper_discard_silence', 'No speech detected'),
      empty_result: tr('voice_whisper_discard_empty', 'No speech was recognized'),
      hallucination: tr('voice_whisper_discard_hallucination', 'Discarded a likely hallucinated transcript'),
      permission_denied: t('voice_error_permission'),
      audio_capture: t('voice_error_audio_capture'),
    };
    showToast(byCode[normalized] || t('voice_error_detail').replace('{code}', normalized), btn, 5000);
  }

  function setVoiceAudioActive(active: boolean) {
    if (voiceAudioActive === active) return;
    set_voiceAudioActive(active);
    document.dispatchEvent(new CustomEvent('voiceinput:statechanged'));
  }

  function resizeCanvas() {
    const r = canvas.getBoundingClientRect();
    if (r.width > 0) {
      canvas.width = Math.round(r.width * devicePixelRatio);
      canvas.height = Math.round(r.height * devicePixelRatio);
    }
  }

  function drawBars() {
    const ctx2d = canvas.getContext('2d');
    if (!ctx2d) return;
    const W = canvas.width;
    const H = canvas.height;
    ctx2d.clearRect(0, 0, W, H);
    const barCount = 48;
    const barW = Math.max(2, Math.floor(W / (barCount * 1.8)));
    const gap = (W - barCount * barW) / (barCount + 1);
    voiceIntensityTarget = Math.max(0.04, wh.getAudioLevel());
    voiceIntensity += (voiceIntensityTarget - voiceIntensity) * 0.18;
    for (let i = 0; i < barCount; i++) {
      const phase = wavePhase + i * 0.42;
      const lo = Math.sin(phase) * 0.5 + 0.5;
      const hi = Math.sin(phase * 2.7 + i * 0.13) * 0.5 + 0.5;
      const rnd = (Math.sin(phase * 7.3 + i) + 1) * 0.5;
      const wave = lo * 0.4 + hi * 0.4 + rnd * 0.2;
      const v = 0.08 + wave * 0.92 * Math.min(1, voiceIntensity);
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
    wavePhase += 0.18 + voiceIntensity * 0.35;
    animFrame = requestAnimationFrame(animLoop);
  }

  function showVoiceBar() {
    const term = activeSessionId === null ? null : terminals.get(activeSessionId);
    const shouldStickToBottom = !!(term && (term.autoScroll || isTerminalAtBottom(term)));
    voiceBar.hidden = false;
    waveformRaf = requestAnimationFrame(() => {
      resizeCanvas();
      waveformRaf = null;
    });
    cancelAnimationFrame(animFrame || 0);
    wavePhase = 0;
    voiceIntensity = 0;
    voiceIntensityTarget = 0.05;
    animFrame = requestAnimationFrame(animLoop);
    refitActiveTerminalAfterLayout(shouldStickToBottom);
  }

  function hideVoiceBar() {
    const term = activeSessionId === null ? null : terminals.get(activeSessionId);
    const shouldStickToBottom = !!(term && (term.autoScroll || isTerminalAtBottom(term)));
    if (waveformRaf) cancelAnimationFrame(waveformRaf);
    waveformRaf = null;
    cancelAnimationFrame(animFrame || 0);
    animFrame = null;
    voiceBar.hidden = true;
    refitActiveTerminalAfterLayout(shouldStickToBottom);
  }

  function setRecordingUi(recording: boolean) {
    if (recording) {
      set_voiceActive(true);
      setVoiceAudioActive(true);
      btn.classList.add('recording');
      btn.dataset.tooltip = tr('voice_recording', 'Recording...');
      voiceBar.classList.remove('voice-processing');
      document.dispatchEvent(new CustomEvent('voiceinput:started'));
      return;
    }
    set_voiceActive(false);
    setVoiceAudioActive(false);
    btn.classList.remove('recording');
    btn.dataset.tooltip = tr('voice_tooltip_whisper', 'Voice input (Whisper)');
  }

  function insertTranscribedText(text: string) {
    inputEl.value = joinTranscript(preVoiceText, text);
    autoExpand();
    updateInputClearButton();
    updateSlashMenu();

    const triggerPhrase = getActiveTriggerPhrase();
    if (
      localStorage.getItem(STORAGE_VOICE_WHISPER_AUTO_SUBMIT_KEY) === '1' &&
      triggerPhrase &&
      activeSessionId !== null &&
      textEndsWithTriggerPhrase(buildSendText(), triggerPhrase)
    ) {
      doSend(activeSessionId);
    }
  }

  // ---- core のイベント → 既存の画面 ----
  wh.on('audioActive', setVoiceAudioActive);

  wh.on('start', () => {
    setRecordingUi(true);
    showVoiceBar();
  });

  // 録音を止めて文字起こしを待っている間（旧 finishWhisperRecording の前半）
  wh.on('processing', () => {
    voiceBar.classList.add('voice-processing');
    set_voiceActive(true);
    btn.classList.add('recording');
  });

  wh.on('notice', () => {
    showToast(tr('voice_whisper_processing', '認識中です…しばらくお待ちください'), btn, 3000);
  });

  wh.on('result', ({ text }) => insertTranscribedText(text));

  wh.on('error', (e) => {
    if (e.phase === 'start') {
      // マイクを開けなかった（旧 startWhisperRecording の catch）
      inputEl.value = preVoiceText;
      autoExpand();
      showVoiceError(e.code);
      return;
    }
    // 文字起こしが失敗・取り消しで終わった（旧 finishWhisperRecording の catch）
    if (e.cancelled) {
      showToast(tr('voice_whisper_cancelled', '音声認識を取り消しました'), btn, 2000);
    } else {
      showVoiceError(e.code);
    }
    inputEl.value = preVoiceText;
    autoExpand();
    finishFailed = true;
  });

  wh.on('stop', ({ reason }) => {
    if (reason === 'result') {
      voiceBar.classList.remove('voice-processing');
      hideVoiceBar();
      setRecordingUi(false);
      document.dispatchEvent(new CustomEvent('voiceinput:stopped'));
      setTimeout(() => inputEl.focus(), 0);
      return;
    }
    if (finishFailed) {
      finishFailed = false;
      voiceBar.classList.remove('voice-processing');
      hideVoiceBar();
      setRecordingUi(false);
      document.dispatchEvent(new CustomEvent('voiceinput:stopped'));
      return;
    }
    // 取り消しボタン・Escape（旧 cancelWhisperRecording）
    inputEl.value = preVoiceText;
    autoExpand();
    updateSlashMenu();
    voiceBar.classList.remove('voice-processing');
    hideVoiceBar();
    setRecordingUi(false);
    setTimeout(() => inputEl.focus(), 0);
    document.dispatchEvent(new CustomEvent('voiceinput:stopped'));
  });

  async function startWhisperRecording() {
    if (getVoiceEngine() !== 'whisper') return;
    if (!supportsWhisperRecording) {
      showVoiceError('audio_capture');
      return;
    }
    // 録音中なら確定、文字起こし中なら「認識中です」を出すだけ（core の toggle が判断する）。
    if (!wh.isRecording() && !wh.isProcessing()) {
      preVoiceText = inputEl.value;
      if (preVoiceText.length > 0 && !/\s$/.test(preVoiceText)) {
        inputEl.value = preVoiceText + ' ';
        updateInputClearButton();
      }
    }
    await wh.toggle();
  }

  btn.addEventListener('click', () => {
    if (getVoiceEngine() !== 'whisper') return;
    startWhisperRecording();
  });

  cancelBtn.addEventListener('click', () => {
    if (getVoiceEngine() === 'whisper') wh.cancel();
  });

  confirmBtn.addEventListener('click', () => {
    if (getVoiceEngine() === 'whisper') wh.finish();
  });

  document.addEventListener('keydown', (e) => {
    if (getVoiceEngine() !== 'whisper') return;
    if (e.key === 'Escape' && (wh.isRecording() || wh.isProcessing())) {
      e.preventDefault();
      wh.cancel();
      return;
    }
    if (e.altKey && e.code === 'KeyV' && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      btn.click();
    }
  });

  document.addEventListener('voiceengine:changed', updateButtonVisibility);
  window.addEventListener('storage', (event) => {
    if (event.key === STORAGE_VOICE_ENGINE_KEY) updateButtonVisibility();
  });
  window.addEventListener('resize', () => {
    if (wh.isRecording() || wh.isProcessing()) resizeCanvas();
  });

  updateButtonVisibility();
})();
