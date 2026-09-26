import { probeSpan } from '../debug/probe.js';
// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { showToast } from './util.js';
import { STORAGE_VOICE_INPUT_DISABLED_KEY, getVoiceEngine } from './user-prefs.js';
import { activeSessionId, terminals } from './state.js';
import { autoExpand, buildSendText, doSend, inputEl, set_voiceActive, set_voiceAudioActive, updateInputClearButton, updateSlashMenu, voiceAudioActive } from '../app.js';
import { isTerminalAtBottom, refitActiveTerminalAfterLayout } from './terminal.js';
import { getActiveTriggerPhrase, textEndsWithTriggerPhrase } from './settings.js';
import { diagnosticSeverity, normalizeVoiceErrorCode, shouldShowRecoveryGuide } from '../vendor/vtype-core/index.js';
import { voiceInput } from './voice-engine.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- 音声入力（ブラウザ内蔵認識）の画面側 ----
// 認識そのもの（SpeechRecognition の生成・stuck 対策の作り直し・診断モード）は vtype-core が持つ。
// ここはボタン・音声バー・波形・診断表示・トースト・ショートカットだけを扱う。
//
// ウェイクワード（2 本目の認識インスタンス）は無効化中で、旧 voice.ts でも早期 return で止めていた。
// エンジン部分は vtype-core の createHotwordListener に移してある。画面側（全体／セッション別トグル、
// 入力欄ホバー起動）の旧実装は 46d47eb の web/src/app/voice.ts 645〜1021 行にある。
(function () {
  const rec = voiceInput.recognizer;

  const btn        = document.getElementById('voice-btn');
  const voiceBar   = document.getElementById('voice-bar');
  const canvas     = document.getElementById('voice-waveform') as HTMLCanvasElement;
  const cancelBtn  = document.getElementById('voice-cancel-btn');
  const confirmBtn = document.getElementById('voice-confirm-btn');
  const diagRunBtn = document.getElementById('voice-diagnostic-run-btn') as HTMLButtonElement;
  const diagCopyBtn = document.getElementById('voice-diagnostic-copy-btn');
  const diagProfileSpecificBtn = document.getElementById('voice-diagnostic-profile-specific-btn');
  const diagStatusEl = document.getElementById('voice-diagnostic-status');
  const diagGuideEl = document.getElementById('voice-diagnostic-guide');

  if (!rec) return;
  const diagnostics = rec.diagnostics;

  function diagText(key, fallback) {
    const v = typeof window.t === 'function' ? window.t(key) : key;
    return v && v !== key ? v : fallback;
  }

  function renderRecoveryGuide(status) {
    if (!diagGuideEl) return;
    diagGuideEl.innerHTML = '';
    if (!shouldShowRecoveryGuide(status)) {
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

  // 診断状態が変わるたびに core から届く（旧 setVoiceDiagStatus の画面側）。
  function renderVoiceDiagStatus({ status, message, events }) {
    const detail = message || '';
    if (diagStatusEl) {
      const key = 'voice_diag_' + status;
      const fallback = status.replace(/_/g, ' ');
      diagStatusEl.textContent = diagText(key, fallback) + (detail ? ' ' + detail : '');
      diagStatusEl.className = 'voice-diagnostic-status ' + diagnosticSeverity(status);
    }
    renderRecoveryGuide(status);
    document.dispatchEvent(new CustomEvent('voiceinput:diagnostic', {
      detail: { status, message: detail, events },
    }));
  }
  rec.on('diagnostic', renderVoiceDiagStatus);
  function renderCurrentVoiceDiagStatus() {
    renderVoiceDiagStatus({
      status: diagnostics.getStatus(),
      message: diagnostics.getLastDetail(),
      events: diagnostics.getEvents(),
    });
  }

  async function copyVoiceDiagReport(anchor) {
    const text = diagnostics.getReportJson();
    try {
      await navigator.clipboard.writeText(text);
      showToast(diagText('voice_diag_copied', 'Voice diagnostics log copied'), anchor);
    } catch (err) {
      showToast(diagText('voice_diag_copy_failed', 'Failed to copy voice diagnostics log'), anchor);
    }
  }

  window.__anyAiCliVoiceDiagnostics = {
    getStatus: () => diagnostics.getStatus(),
    getEvents: () => diagnostics.getEvents(),
    getReport: () => diagnostics.getReport(),
    markNormalProfileSpecific: () => diagnostics.markNormalProfileSpecific(),
  };

  if (!rec.support.supported) {
    renderCurrentVoiceDiagStatus();
    if (diagRunBtn) diagRunBtn.disabled = true;
    if (diagCopyBtn) diagCopyBtn.addEventListener('click', () => copyVoiceDiagReport(diagCopyBtn));
    return;
  }
  if (!btn || !voiceBar || !canvas) return;

  // ブラウザ対応済みの印（settings.ts のトグルが「対応ブラウザでのみ再表示」判定に使う）
  btn.dataset.voiceBrowserSupported = '1';
  function updateBrowserVoiceButtonVisibility() {
    const engine = getVoiceEngine();
    if (engine === 'browser') {
      btn.dataset.voiceSupported = '1';
      btn.hidden = false;
      btn.dataset.tooltip = t('voice_tooltip');
      return;
    }
    if (engine === 'off') {
      btn.hidden = true;
      return;
    }
    if (btn.dataset.voiceWhisperSupported !== '1') btn.hidden = true;
  }
  updateBrowserVoiceButtonVisibility();
  document.addEventListener('voiceengine:changed', updateBrowserVoiceButtonVisibility);

  renderCurrentVoiceDiagStatus();

  window.__anyAiCliVoiceDiagnostics.run = () => diagnostics.run();
  window.__anyAiCliVoiceDiagnostics.copy = () => copyVoiceDiagReport(diagCopyBtn || btn);

  let interimStart = 0;
  let preVoiceText = '';

  let animFrame = null;
  let wavePhase = 0;
  let waveformRaf = null;

  let voiceIntensity = 0;
  let voiceIntensityTarget = 0;
  let lastInterimLen = 0;
  let lastKickAt = 0;

  const BAR_COUNT = 48;

  function formatVoiceError(key, code) {
    const msg = t(key);
    if (!code) return msg;
    return msg.replace('{code}', code);
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

  // ---- core のイベント → 既存の画面 ----
  // 旧コードは古いインスタンスから遅れて届くイベントも区別せずに画面へ反映していたので、
  // ここでも recognitionId / isCurrent で絞らない（確定直後の最後の言葉が消えないように）。
  rec.on('audioActive', setVoiceAudioActive);

  rec.on('start', () => {
    set_voiceActive(true);
    btn.classList.add('recording');
    btn.dataset.tooltip = t('voice_recording');
    voiceIntensityTarget = 0.15;
    showVoiceBar();
    document.dispatchEvent(new CustomEvent('voiceinput:started'));
  });

  rec.on('stop', () => {
    set_voiceActive(false);
    btn.classList.remove('recording');
    btn.dataset.tooltip = t('voice_tooltip');
    voiceBar.classList.remove('voice-processing');
    hideVoiceBar();
    setTimeout(() => inputEl.focus(), 0);
    document.dispatchEvent(new CustomEvent('voiceinput:stopped'));
  });

  rec.on('activity', ({ kind }) => {
    if (kind === 'soundstart') {
      voiceIntensityTarget = 0.55;
      lastKickAt = performance.now();
    } else if (kind === 'speechstart') {
      voiceIntensityTarget = 0.9;
      lastKickAt = performance.now();
    } else if (kind === 'speechend') {
      voiceIntensityTarget = 0.25;
    } else if (kind === 'audioend') {
      voiceIntensityTarget = 0.03;
      voiceBar.classList.add('voice-processing');
    }
  });

  rec.on('result', ({ transcript, isFinal }) => {
    const finishProbe = probeSpan('ui.freeze', () => ({ phase: 'voice.result', size: transcript.length }));
    try {
      voiceBar.classList.remove('voice-processing');
      if (transcript.length > lastInterimLen) {
        lastKickAt = performance.now();
        voiceIntensityTarget = Math.max(voiceIntensityTarget, 0.85);
      }
      lastInterimLen = isFinal ? 0 : transcript.length;
      inputEl.value = inputEl.value.slice(0, interimStart) + transcript;
      if (isFinal) {
        inputEl.value += ' ';
        interimStart = inputEl.value.length;
        const _tp = getActiveTriggerPhrase();
        if (_tp && activeSessionId !== null && textEndsWithTriggerPhrase(buildSendText(), _tp)) {
          rec.requestStop();
          doSend(activeSessionId);
          return;
        }
      }
      autoExpand();
      updateSlashMenu();
    } finally { finishProbe?.(); }
  });

  // aborted / no-speech は core が notify=false にする（旧コードも表示しなかった）。
  rec.on('error', (e) => {
    if (e.notify) showVoiceError(e.code, btn);
  });

  btn.addEventListener('click', () => {
    if (getVoiceEngine() !== 'browser') return;
    rec.recordClick();
    if (rec.isRecording()) {
      rec.abort();
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
    // start() の例外は core が error イベント（notify=true）で返す。
    rec.start();
  });

  cancelBtn.addEventListener('click', () => {
    inputEl.value = preVoiceText;
    autoExpand();
    rec.abort();
  });

  confirmBtn.addEventListener('click', () => {
    rec.stop();
  });

  if (diagRunBtn) {
    diagRunBtn.addEventListener('click', () => diagnostics.run());
  }
  if (diagCopyBtn) {
    diagCopyBtn.addEventListener('click', () => copyVoiceDiagReport(diagCopyBtn));
  }
  if (diagProfileSpecificBtn) {
    diagProfileSpecificBtn.addEventListener('click', () => {
      diagnostics.confirmNormalProfileSpecific();
    });
  }

  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && rec.isRecording()) {
      e.preventDefault();
      cancelBtn.click();
      return;
    }
    if (e.altKey && e.code === 'KeyV' && !e.ctrlKey && !e.metaKey && getVoiceEngine() === 'browser') {
      e.preventDefault();
      btn.click();
    }
  });

  window.addEventListener('resize', () => {
    if (rec.isRecording()) resizeCanvas();
  });
})();
