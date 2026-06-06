// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { showToast } from './util.js';
import { STORAGE_LANG_KEY, STORAGE_VOICE_INPUT_DISABLED_KEY, STORAGE_WAKE_WORD_ENABLED_KEY, STORAGE_WAKE_WORD_PHRASE_KEY, getDefaultWakeWordPhrase } from './user-prefs.js';
import { activeSessionId, terminals } from './state.js';
import { autoExpand, buildSendText, doSend, inputEl, set_voiceActive, set_voiceAudioActive, updateInputClearButton, updateSlashMenu, voiceActive, voiceAudioActive } from '../app.js';
import { renderSessionList } from './session-list.js';
import { isTerminalAtBottom, refitActiveTerminalAfterLayout } from './terminal.js';
import { getActiveTriggerPhrase, normalizeTriggerMatchText, textEndsWithTriggerPhrase } from './settings.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- 音声入力 / ウェイクワード ----
// ---- 音声入力 ----
(function () {
  const SpeechRecognition = window.SpeechRecognition || window.webkitSpeechRecognition;
  const isChromium = navigator.userAgentData?.brands?.some(b => /Chromium/.test(b.brand))
    ?? /Chrome\//.test(navigator.userAgent);

  const btn        = document.getElementById('voice-btn');
  const voiceBar   = document.getElementById('voice-bar');
  const canvas     = document.getElementById('voice-waveform');
  const cancelBtn  = document.getElementById('voice-cancel-btn');
  const confirmBtn = document.getElementById('voice-confirm-btn');
  const diagRunBtn = document.getElementById('voice-diagnostic-run-btn');
  const diagCopyBtn = document.getElementById('voice-diagnostic-copy-btn');
  const diagProfileSpecificBtn = document.getElementById('voice-diagnostic-profile-specific-btn');
  const diagStatusEl = document.getElementById('voice-diagnostic-status');
  const diagGuideEl = document.getElementById('voice-diagnostic-guide');

  const VOICE_DIAG_EVENT_LIMIT = 80;
  const VOICE_DIAG_STUCK_MS = 20000;
  const voiceDiagEvents = [];
  let voiceDiagStatus = SpeechRecognition && isChromium ? 'idle' : 'unsupported';
  let voiceDiagLastDetail = '';
  let voiceDiagSeq = 0;

  function diagText(key, fallback) {
    const v = typeof window.t === 'function' ? window.t(key) : key;
    return v && v !== key ? v : fallback;
  }

  function pushVoiceDiagEvent(recognitionId, event, detail: any = {}) {
    const item = {
      timestamp: new Date().toISOString(),
      recognitionId: recognitionId == null ? null : recognitionId,
      event,
      error: detail?.error || null,
      message: detail?.message || null,
      hasResult: !!detail?.hasResult,
      transcriptLength: Number.isFinite(detail?.transcriptLength) ? detail.transcriptLength : 0,
    };
    voiceDiagEvents.push(item);
    while (voiceDiagEvents.length > VOICE_DIAG_EVENT_LIMIT) voiceDiagEvents.shift();
    return item;
  }

  function voiceDiagClass(status) {
    if (status === 'healthy') return 'ok';
    if (status === 'permission_denied' || status === 'audio_capture_failed' || status === 'speech_service_failed') return 'err';
    if (status === 'profile_or_stt_stuck_suspected' || status === 'normal_profile_specific' || status === 'no_result') return 'warn';
    return '';
  }

  function shouldShowRecoveryGuide(status) {
    return status === 'profile_or_stt_stuck_suspected' || status === 'normal_profile_specific';
  }

  function renderRecoveryGuide() {
    if (!diagGuideEl) return;
    diagGuideEl.innerHTML = '';
    if (!shouldShowRecoveryGuide(voiceDiagStatus)) {
      diagGuideEl.hidden = true;
      return;
    }
    const title = document.createElement('div');
    title.className = 'voice-diagnostic-guide-title';
    title.textContent = diagText('voice_diag_guide_title', 'Recovery guide');
    const list = document.createElement('ol');
    for (let i = 1; i <= 6; i++) {
      const li = document.createElement('li');
      const text = diagText('voice_diag_guide_' + i, '');
      if (text.includes('chrome://settings/content/all?searchSubpage=127.0.0.1')) {
        const [before, after] = text.split('chrome://settings/content/all?searchSubpage=127.0.0.1');
        li.appendChild(document.createTextNode(before));
        const code = document.createElement('code');
        code.textContent = 'chrome://settings/content/all?searchSubpage=127.0.0.1';
        li.appendChild(code);
        li.appendChild(document.createTextNode(after || ''));
      } else {
        li.textContent = text;
      }
      list.appendChild(li);
    }
    diagGuideEl.appendChild(title);
    diagGuideEl.appendChild(list);
    diagGuideEl.hidden = false;
  }

  function setVoiceDiagStatus(status, detail = null) {
    voiceDiagStatus = status;
    voiceDiagLastDetail = detail || '';
    if (diagStatusEl) {
      const key = 'voice_diag_' + status;
      const fallback = status.replace(/_/g, ' ');
      diagStatusEl.textContent = diagText(key, fallback) + (detail ? ' ' + detail : '');
      diagStatusEl.className = 'voice-diagnostic-status ' + voiceDiagClass(status);
    }
    renderRecoveryGuide();
    document.dispatchEvent(new CustomEvent('voiceinput:diagnostic', {
      detail: { status, message: voiceDiagLastDetail, events: voiceDiagEvents.slice() },
    }));
  }

  function classifyVoiceError(error) {
    if (error === 'not-allowed' || error === 'permission-denied') return 'permission_denied';
    if (error === 'audio-capture') return 'audio_capture_failed';
    if (error === 'network' || error === 'service-not-allowed') return 'speech_service_failed';
    if (error === 'no-speech') return 'no_result';
    return 'speech_service_failed';
  }

  function createVoiceDiagReport() {
    const uaData = navigator.userAgentData ? {
      brands: navigator.userAgentData.brands,
      mobile: navigator.userAgentData.mobile,
      platform: navigator.userAgentData.platform,
    } : null;
    return {
      generatedAt: new Date().toISOString(),
      appVersion: document.querySelector('.settings-app-version')?.textContent || null,
      userAgent: navigator.userAgent,
      userAgentData: uaData,
      origin: location.origin,
      isLocalOrigin: /^https?:\/\/127\.0\.0\.1(?::\d+)?$/.test(location.origin),
      speechRecognitionSupported: !!SpeechRecognition,
      chromiumDetected: !!isChromium,
      status: voiceDiagStatus,
      message: voiceDiagLastDetail,
      events: voiceDiagEvents.slice(),
    };
  }

  async function copyVoiceDiagReport(anchor) {
    const text = JSON.stringify(createVoiceDiagReport(), null, 2);
    try {
      await navigator.clipboard.writeText(text);
      showToast(diagText('voice_diag_copied', 'Voice diagnostics log copied'), anchor);
    } catch (err) {
      showToast(diagText('voice_diag_copy_failed', 'Failed to copy voice diagnostics log'), anchor);
    }
  }

  window.__anyAiCliVoiceDiagnostics = {
    getStatus: () => voiceDiagStatus,
    getEvents: () => voiceDiagEvents.slice(),
    getReport: createVoiceDiagReport,
    markNormalProfileSpecific: () => setVoiceDiagStatus('normal_profile_specific'),
  };

  if (!SpeechRecognition || !isChromium) {
    setVoiceDiagStatus('unsupported');
    if (diagRunBtn) diagRunBtn.disabled = true;
    if (diagCopyBtn) diagCopyBtn.addEventListener('click', () => copyVoiceDiagReport(diagCopyBtn));
    return;
  }
  if (!btn || !voiceBar || !canvas) return;

  // ブラウザ対応済みの印（settings.ts のトグルが「対応ブラウザでのみ再表示」判定に使う）
  btn.dataset.voiceSupported = '1';
  btn.hidden = localStorage.getItem(STORAGE_VOICE_INPUT_DISABLED_KEY) === '1';
  btn.dataset.tooltip = t('voice_tooltip');

  let dbgSeq = 0;
  let recognition = new SpeechRecognition();
  recognition._dbgId = ++dbgSeq;
  function configureRecognition() {
    recognition.interimResults = true;
    recognition.continuous = false;
    recognition.maxAlternatives = 1;
    recognition.lang = getLang();
  }
  configureRecognition();
  setVoiceDiagStatus('idle');

  function configureDiagnosticRecognition(rec) {
    rec.interimResults = true;
    rec.continuous = false;
    rec.maxAlternatives = 1;
    rec.lang = getLang();
  }

  function runVoiceDiagnostic() {
    if (!SpeechRecognition) {
      setVoiceDiagStatus('unsupported');
      return;
    }
    if (isRecording) {
      try { recognition.abort(); } catch (_) {}
      stopVoice();
    }
    const diag = new SpeechRecognition();
    const diagId = 'diag-' + (++voiceDiagSeq);
    let settled = false;
    let sawResult = false;
    let sawError = false;
    let stuckTimer = null;
    let hardTimer = null;
    configureDiagnosticRecognition(diag);

    function clearTimers() {
      clearTimeout(stuckTimer);
      clearTimeout(hardTimer);
      stuckTimer = null;
      hardTimer = null;
    }
    function finish(status, detail = '') {
      if (settled) return;
      settled = true;
      clearTimers();
      setVoiceAudioActive(false);
      setVoiceDiagStatus(status, detail);
      try { diag.abort(); } catch (_) {}
    }
    function event(name, detail = {}) {
      pushVoiceDiagEvent(diagId, name, detail || {});
    }

    setVoiceDiagStatus('running');
    event('click');
    hardTimer = setTimeout(() => {
      event('diagnostic-timeout', { message: 'no terminal event within hard timeout' });
      finish('profile_or_stt_stuck_suspected');
    }, VOICE_DIAG_STUCK_MS + 10000);

    diag.onstart = () => {
      event('start');
      setVoiceAudioActive(true);
    };
    diag.onaudiostart = () => event('audiostart');
    diag.onsoundstart = () => event('soundstart');
    diag.onspeechstart = () => event('speechstart');
    diag.onspeechend = () => event('speechend');
    diag.onsoundend = () => event('soundend');
    diag.onaudioend = () => {
      event('audioend');
      setVoiceAudioActive(false);
      stuckTimer = setTimeout(() => {
        event('stuck-timeout', { message: 'audioend without result/end/error' });
        finish('profile_or_stt_stuck_suspected');
      }, VOICE_DIAG_STUCK_MS);
    };
    diag.onresult = (e) => {
      const result = e.results[e.resultIndex];
      const text = result?.[0]?.transcript || '';
      sawResult = true;
      event('result', { hasResult: true, transcriptLength: text.length });
      finish('healthy');
    };
    diag.onnomatch = () => event('nomatch');
    diag.onerror = (e) => {
      sawError = true;
      const error = normalizeVoiceErrorCode(e);
      event('error', { error, message: e.message || null });
      finish(classifyVoiceError(error), e.message || '');
    };
    diag.onend = () => {
      event('end');
      if (!sawResult && !sawError) finish('no_result');
    };

    try {
      diag.start();
    } catch (err) {
      const error = normalizeVoiceErrorCode(err);
      event('start-error', { error, message: err?.message || null });
      finish(classifyVoiceError(error), err?.message || '');
    }
  }
  window.__anyAiCliVoiceDiagnostics.run = runVoiceDiagnostic;
  window.__anyAiCliVoiceDiagnostics.copy = () => copyVoiceDiagReport(diagCopyBtn || btn);

  let isRecording  = false;
  let interimStart = 0;
  let preVoiceText = '';
  let audioendStuckTimer = null;

  let animFrame = null;
  let wavePhase = 0;
  let waveformRaf = null;

  let voiceIntensity = 0;
  let voiceIntensityTarget = 0;
  let lastInterimLen = 0;
  let lastKickAt = 0;

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
    set_voiceAudioActive(active);
    document.dispatchEvent(new CustomEvent('voiceinput:statechanged'));
  }

  function stopVoice() {
    pushVoiceDiagEvent(recognition._dbgId, 'stopVoice', { message: 'manual or terminal stop' });
    clearTimeout(audioendStuckTimer);
    audioendStuckTimer = null;
    if (!isRecording) return;
    isRecording = false;
    set_voiceActive(false);
    setVoiceAudioActive(false);
    btn.classList.remove('recording');
    btn.dataset.tooltip = t('voice_tooltip');
    voiceBar.classList.remove('voice-processing');
    hideVoiceBar();
    setTimeout(() => inputEl.focus(), 0);
    document.dispatchEvent(new CustomEvent('voiceinput:stopped'));
    // 次回のためにインスタンス作り直し（Chrome stuck 対策）
    recognition = new SpeechRecognition();
    recognition._dbgId = ++dbgSeq;
    configureRecognition();
    attachHandlers();
  }

  function attachHandlers() {
    const myId = recognition._dbgId;
    recognition.onstart = () => {
      pushVoiceDiagEvent(myId, 'start');
      isRecording = true;
      set_voiceActive(true);
      setVoiceAudioActive(true);
      btn.classList.add('recording');
      btn.dataset.tooltip = t('voice_recording');
      voiceIntensityTarget = 0.15;
      showVoiceBar();
      document.dispatchEvent(new CustomEvent('voiceinput:started'));
    };

    recognition.onaudiostart = () => {
      pushVoiceDiagEvent(myId, 'audiostart');
    };

    recognition.onsoundstart = () => {
      pushVoiceDiagEvent(myId, 'soundstart');
      voiceIntensityTarget = 0.55;
      lastKickAt = performance.now();
    };

    recognition.onspeechstart = () => {
      pushVoiceDiagEvent(myId, 'speechstart');
      voiceIntensityTarget = 0.9;
      lastKickAt = performance.now();
    };

    recognition.onspeechend = () => {
      pushVoiceDiagEvent(myId, 'speechend');
      voiceIntensityTarget = 0.25;
    };

    recognition.onsoundend = () => {
      pushVoiceDiagEvent(myId, 'soundend');
    };

    recognition.onaudioend = () => {
      pushVoiceDiagEvent(myId, 'audioend');
      setVoiceAudioActive(false);
      voiceIntensityTarget = 0.03;
      voiceBar.classList.add('voice-processing');
      clearTimeout(audioendStuckTimer);
      audioendStuckTimer = setTimeout(() => {
        pushVoiceDiagEvent(myId, 'stuck-timeout', { message: 'audioend without result/end/error' });
        setVoiceDiagStatus('profile_or_stt_stuck_suspected');
      }, VOICE_DIAG_STUCK_MS);
    };

    recognition.onresult = (e) => {
      clearTimeout(audioendStuckTimer);
      audioendStuckTimer = null;
      voiceBar.classList.remove('voice-processing');
      const result = e.results[e.resultIndex];
      if (!result) return;
      const transcript = result[0].transcript;
      pushVoiceDiagEvent(myId, 'result', { hasResult: true, transcriptLength: transcript.length });
      if (transcript.length > lastInterimLen) {
        lastKickAt = performance.now();
        voiceIntensityTarget = Math.max(voiceIntensityTarget, 0.85);
      }
      lastInterimLen = result.isFinal ? 0 : transcript.length;
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
    };

    recognition.onnomatch = () => {
      pushVoiceDiagEvent(myId, 'nomatch');
    };

    recognition.onend = () => {
      clearTimeout(audioendStuckTimer);
      audioendStuckTimer = null;
      pushVoiceDiagEvent(myId, 'end');
      stopVoice();
    };

    recognition.onerror = (e) => {
      clearTimeout(audioendStuckTimer);
      audioendStuckTimer = null;
      pushVoiceDiagEvent(myId, 'error', { error: e.error || null, message: e.message || null });
      if (e.error !== 'no-speech' && e.error !== 'aborted') {
        showVoiceError(e, btn);
      }
      stopVoice();
    };
  }
  attachHandlers();

  btn.addEventListener('click', () => {
    pushVoiceDiagEvent(recognition._dbgId, 'click');
    if (isRecording) {
      try { recognition.abort(); } catch (_) {}
      stopVoice();
      return;
    }
    // 設定で無効化されている場合は録音を開始しない（Alt+V 経由の click も含む）
    if (localStorage.getItem(STORAGE_VOICE_INPUT_DISABLED_KEY) === '1') return;
    preVoiceText = inputEl.value;
    if (preVoiceText.length > 0 && !/\s$/.test(preVoiceText)) {
      inputEl.value = preVoiceText + ' ';
      updateInputClearButton();
    }
    interimStart = inputEl.value.length;
    recognition.lang = getLang();
    try {
      recognition.start();
    } catch (err) {
      showVoiceError(err, btn);
    }
  });

  cancelBtn.addEventListener('click', () => {
    inputEl.value = preVoiceText;
    autoExpand();
    try { recognition.abort(); } catch (_) {}
    stopVoice();
  });

  confirmBtn.addEventListener('click', () => {
    try { recognition.stop(); } catch (_) {}
    stopVoice();
  });

  if (diagRunBtn) {
    diagRunBtn.addEventListener('click', () => runVoiceDiagnostic());
  }
  if (diagCopyBtn) {
    diagCopyBtn.addEventListener('click', () => copyVoiceDiagReport(diagCopyBtn));
  }
  if (diagProfileSpecificBtn) {
    diagProfileSpecificBtn.addEventListener('click', () => {
      pushVoiceDiagEvent(null, 'normal-profile-specific-confirmed', {
        message: 'user confirmed Incognito or a new profile works',
      });
      setVoiceDiagStatus('normal_profile_specific');
    });
  }

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
    if (voiceActive && !hasVoiceRecordingClass) set_voiceActive(false);
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
  window._wakewordSessionRemoved = (id) => {
    const wasSessionActive = sessionWakeMap.get(id) || false;
    sessionWakeMap.delete(id);
    updateGlobalBtn();
    updateSessionBtn();
    updateMicChip();
    if (activeSessionId === id && wasSessionActive && !isGlobalActive) stopHotword();
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
  window._stopWakewordForVoiceInput = async () => { await stopHotwordForVoiceInput(); };
})();
