// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { showToast, token } from './util.js';
import { STORAGE_VOICE_ENGINE_KEY, STORAGE_VOICE_WHISPER_AUTO_SUBMIT_KEY, getVoiceEngine } from './user-prefs.js';
import { activeSessionId, terminals } from './state.js';
import { autoExpand, buildSendText, doSend, inputEl, set_voiceActive, set_voiceAudioActive, updateInputClearButton, updateSlashMenu, voiceAudioActive } from '../app.js';
import { isTerminalAtBottom, refitActiveTerminalAfterLayout } from './terminal.js';
import { getActiveTriggerPhrase, textEndsWithTriggerPhrase } from './settings.js';

const TARGET_SAMPLE_RATE = 16000;
const MAX_RECORD_SECONDS = 120;
const MIN_RECORD_MS = 250;
const MIN_VOICED_MS = 120;
const MIN_PEAK_RMS = 0.012;
const VAD_RMS_THRESHOLD = 0.008;
const WHISPER_HALLUCINATION_PHRASES = [
  'ご視聴ありがとうございました',
  'ご清聴ありがとうございました',
  'チャンネル登録をお願いします',
  'チャンネル登録よろしくお願いします',
  '字幕視聴ありがとうございました',
  'thanks for watching',
  'thank you for watching',
  'please subscribe',
  'like and subscribe',
  'don\'t forget to subscribe',
];

(function () {
  const btn = document.getElementById('voice-btn');
  const voiceBar = document.getElementById('voice-bar');
  const canvas = document.getElementById('voice-waveform');
  const cancelBtn = document.getElementById('voice-cancel-btn');
  const confirmBtn = document.getElementById('voice-confirm-btn');

  if (!btn || !voiceBar || !canvas || !cancelBtn || !confirmBtn || !inputEl) return;

  const supportsWhisperRecording = !!(
    navigator.mediaDevices?.getUserMedia &&
    (window.AudioContext || window.webkitAudioContext)
  );

  let isRecording = false;
  let isProcessing = false;
  let preVoiceText = '';
  let stream: MediaStream | null = null;
  let audioCtx: AudioContext | null = null;
  let sourceNode: MediaStreamAudioSourceNode | null = null;
  let workletNode: AudioWorkletNode | null = null;
  let scriptNode: ScriptProcessorNode | null = null;
  let analyserNode: AnalyserNode | null = null;
  let silentGainNode: GainNode | null = null;
  let abortController: AbortController | null = null;
  let startedAt = 0;
  let maxRecordTimer: ReturnType<typeof setTimeout> | null = null;
  let waveformRaf: number | null = null;
  let animFrame: number | null = null;
  let wavePhase = 0;
  let voiceIntensity = 0;
  let voiceIntensityTarget = 0;
  let totalSampleCount = 0;
  let voicedSampleCount = 0;
  let peakRms = 0;
  const chunks: Float32Array[] = [];

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
    showToast(byCode[normalized] || t('voice_error_detail').replace('{code}', normalized), btn);
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

  function readAudioLevel() {
    if (!analyserNode) return 0.05;
    const data = new Uint8Array(analyserNode.fftSize);
    analyserNode.getByteTimeDomainData(data);
    let sum = 0;
    for (const v of data) {
      const x = (v - 128) / 128;
      sum += x * x;
    }
    return Math.min(1, Math.sqrt(sum / Math.max(1, data.length)) * 4);
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
    voiceIntensityTarget = Math.max(0.04, readAudioLevel());
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

  function appendChunk(chunk: Float32Array) {
    observeAudioChunk(chunk);
    chunks.push(new Float32Array(chunk));
  }

  function observeAudioChunk(chunk: Float32Array) {
    if (!chunk.length) return;
    let sum = 0;
    for (const sample of chunk) sum += sample * sample;
    const rms = Math.sqrt(sum / chunk.length);
    totalSampleCount += chunk.length;
    peakRms = Math.max(peakRms, rms);
    if (rms >= VAD_RMS_THRESHOLD) voicedSampleCount += chunk.length;
  }

  function mixToMono(input: Float32Array, channels: number) {
    if (channels <= 1) return input;
    const frames = Math.floor(input.length / channels);
    const out = new Float32Array(frames);
    for (let i = 0; i < frames; i++) {
      let sum = 0;
      for (let ch = 0; ch < channels; ch++) sum += input[i * channels + ch] || 0;
      out[i] = sum / channels;
    }
    return out;
  }

  // worklet モジュールは同一オリジンの静的 JS として配信する（web/src/whisper-recorder-worklet.js）。
  // 以前は blob URL を addModule() していたが、Hub の CSP script-src 'self' が blob: を許可しないため
  // ロードがブロックされていた。CSP を緩めずに済むよう static 配信に変更した。
  const RECORDER_WORKLET_URL = '/whisper-recorder-worklet.js';

  async function attachRecorder(ctx: AudioContext, source: MediaStreamAudioSourceNode) {
    analyserNode = ctx.createAnalyser();
    analyserNode.fftSize = 256;
    source.connect(analyserNode);

    silentGainNode = ctx.createGain();
    silentGainNode.gain.value = 0;
    silentGainNode.connect(ctx.destination);

    if (ctx.audioWorklet) {
      await ctx.audioWorklet.addModule(RECORDER_WORKLET_URL);
      workletNode = new AudioWorkletNode(ctx, 'any-ai-cli-whisper-recorder');
      workletNode.port.onmessage = (event) => appendChunk(event.data);
      source.connect(workletNode);
      workletNode.connect(silentGainNode);
      return;
    }

    scriptNode = ctx.createScriptProcessor(4096, 1, 1);
    scriptNode.onaudioprocess = (event) => {
      const input = event.inputBuffer;
      const channels = input.numberOfChannels;
      if (channels <= 1) {
        appendChunk(input.getChannelData(0));
        return;
      }
      const frames = input.length;
      const interleaved = new Float32Array(frames * channels);
      for (let ch = 0; ch < channels; ch++) {
        const data = input.getChannelData(ch);
        for (let i = 0; i < frames; i++) interleaved[i * channels + ch] = data[i];
      }
      appendChunk(mixToMono(interleaved, channels));
    };
    source.connect(scriptNode);
    scriptNode.connect(silentGainNode);
  }

  function stopMediaGraph() {
    if (maxRecordTimer) clearTimeout(maxRecordTimer);
    maxRecordTimer = null;
    try { workletNode?.disconnect(); } catch (_) {}
    try { scriptNode?.disconnect(); } catch (_) {}
    try { analyserNode?.disconnect(); } catch (_) {}
    try { sourceNode?.disconnect(); } catch (_) {}
    try { silentGainNode?.disconnect(); } catch (_) {}
    stream?.getTracks().forEach((track) => {
      try { track.stop(); } catch (_) {}
    });
    if (audioCtx && audioCtx.state !== 'closed') {
      audioCtx.close().catch(() => {});
    }
    stream = null;
    audioCtx = null;
    sourceNode = null;
    workletNode = null;
    scriptNode = null;
    analyserNode = null;
    silentGainNode = null;
    setVoiceAudioActive(false);
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

  async function startWhisperRecording() {
    if (getVoiceEngine() !== 'whisper') return;
    if (!supportsWhisperRecording) {
      showVoiceError('audio_capture');
      return;
    }
    if (isProcessing) return;
    if (isRecording) {
      await finishWhisperRecording();
      return;
    }

    chunks.length = 0;
    totalSampleCount = 0;
    voicedSampleCount = 0;
    peakRms = 0;
    preVoiceText = inputEl.value;
    if (preVoiceText.length > 0 && !/\s$/.test(preVoiceText)) {
      inputEl.value = preVoiceText + ' ';
      updateInputClearButton();
    }

    try {
      const AudioContextCtor = window.AudioContext || window.webkitAudioContext;
      stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      audioCtx = new AudioContextCtor({ sampleRate: TARGET_SAMPLE_RATE });
      sourceNode = audioCtx.createMediaStreamSource(stream);
      await attachRecorder(audioCtx, sourceNode);
      if (audioCtx.state === 'suspended') await audioCtx.resume();
    } catch (err) {
      stopMediaGraph();
      inputEl.value = preVoiceText;
      autoExpand();
      const name = String(err?.name || err?.message || '').toLowerCase();
      showVoiceError(name.includes('notallowed') || name.includes('permission') ? 'permission_denied' : 'audio_capture');
      return;
    }

    isRecording = true;
    startedAt = performance.now();
    setRecordingUi(true);
    showVoiceBar();
    maxRecordTimer = setTimeout(() => {
      if (isRecording) finishWhisperRecording();
    }, MAX_RECORD_SECONDS * 1000);
  }

  function cancelWhisperRecording() {
    if (!isRecording && !isProcessing) return;
    abortController?.abort();
    abortController = null;
    isRecording = false;
    isProcessing = false;
    stopMediaGraph();
    inputEl.value = preVoiceText;
    autoExpand();
    updateSlashMenu();
    voiceBar.classList.remove('voice-processing');
    hideVoiceBar();
    setRecordingUi(false);
    setTimeout(() => inputEl.focus(), 0);
    document.dispatchEvent(new CustomEvent('voiceinput:stopped'));
  }

  async function finishWhisperRecording() {
    if (!isRecording || isProcessing) return;
    isRecording = false;
    isProcessing = true;
    const sourceRate = audioCtx?.sampleRate || TARGET_SAMPLE_RATE;
    const elapsedMs = performance.now() - startedAt;
    stopMediaGraph();
    voiceBar.classList.add('voice-processing');
    set_voiceActive(true);
    btn.classList.add('recording');

    try {
      if (!hasMeaningfulAudio(sourceRate, elapsedMs)) {
        discardWhisperResult('no_speech', {
          elapsedMs,
          totalSampleCount,
          voicedSampleCount,
          peakRms,
        });
      }
      const wav = encodeWav(chunks, sourceRate, TARGET_SAMPLE_RATE);
      abortController = new AbortController();
      const text = await transcribeWav(wav, abortController.signal);
      insertTranscribedText(text);
      voiceBar.classList.remove('voice-processing');
      hideVoiceBar();
      setRecordingUi(false);
      document.dispatchEvent(new CustomEvent('voiceinput:stopped'));
      setTimeout(() => inputEl.focus(), 0);
    } catch (err) {
      if (err?.name !== 'AbortError') showVoiceError(String(err?.message || err || 'whisper_failed'));
      inputEl.value = preVoiceText;
      autoExpand();
      voiceBar.classList.remove('voice-processing');
      hideVoiceBar();
      setRecordingUi(false);
      document.dispatchEvent(new CustomEvent('voiceinput:stopped'));
    } finally {
      abortController = null;
      isProcessing = false;
    }
  }

  function hasMeaningfulAudio(sourceRate: number, elapsedMs: number) {
    if (elapsedMs < MIN_RECORD_MS || chunks.length === 0 || totalSampleCount === 0) return false;
    const voicedMs = (voicedSampleCount / Math.max(1, sourceRate)) * 1000;
    return peakRms >= MIN_PEAK_RMS && voicedMs >= MIN_VOICED_MS;
  }

  function discardWhisperResult(reason: string, detail: any = {}) {
    console.warn('[voice-whisper] discarded transcription', { reason, ...detail });
    throw new Error(reason);
  }

  async function transcribeWav(wav: Blob, signal: AbortSignal) {
    const tk = token;
    const res = await fetch(`/api/voice/transcribe?token=${encodeURIComponent(tk || '')}`, {
      method: 'POST',
      headers: { 'Content-Type': 'audio/wav' },
      body: wav,
      signal,
    });
    let body: any = null;
    try { body = await res.json(); } catch (_) {}
    if (!res.ok) throw new Error(body?.error || 'whisper_failed');
    const text = String(body?.text || '').trim();
    if (!text) discardWhisperResult('empty_result');
    if (isKnownWhisperHallucination(text)) {
      discardWhisperResult('hallucination', { text });
    }
    return text;
  }

  function normalizeWhisperResult(text: string) {
    return String(text || '')
      .normalize('NFKC')
      .toLowerCase()
      .replace(/[\s\u3000]+/g, '')
      .replace(/[。．.!！?？、,，'"“”‘’「」『』（）()[\]{}]+$/g, '');
  }

  function isKnownWhisperHallucination(text: string) {
    const normalized = normalizeWhisperResult(text);
    return WHISPER_HALLUCINATION_PHRASES.some((phrase) => normalized === normalizeWhisperResult(phrase));
  }

  function insertTranscribedText(text: string) {
    const base = preVoiceText;
    const prefix = base && !/\s$/.test(base) ? `${base} ` : base;
    inputEl.value = `${prefix}${text.trim()} `;
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

  function flattenChunks(input: Float32Array[]) {
    let length = 0;
    for (const chunk of input) length += chunk.length;
    const out = new Float32Array(length);
    let offset = 0;
    for (const chunk of input) {
      out.set(chunk, offset);
      offset += chunk.length;
    }
    return out;
  }

  function resampleLinear(input: Float32Array, fromRate: number, toRate: number) {
    if (!input.length || fromRate === toRate) return input;
    const ratio = fromRate / toRate;
    const length = Math.max(1, Math.round(input.length / ratio));
    const out = new Float32Array(length);
    for (let i = 0; i < length; i++) {
      const pos = i * ratio;
      const idx = Math.floor(pos);
      const frac = pos - idx;
      const a = input[idx] || 0;
      const b = input[Math.min(idx + 1, input.length - 1)] || 0;
      out[i] = a + (b - a) * frac;
    }
    return out;
  }

  function encodeWav(inputChunks: Float32Array[], sourceRate: number, targetRate: number) {
    const samples = resampleLinear(flattenChunks(inputChunks), sourceRate, targetRate);
    const bytesPerSample = 2;
    const dataSize = samples.length * bytesPerSample;
    const buffer = new ArrayBuffer(44 + dataSize);
    const view = new DataView(buffer);
    writeAscii(view, 0, 'RIFF');
    view.setUint32(4, 36 + dataSize, true);
    writeAscii(view, 8, 'WAVE');
    writeAscii(view, 12, 'fmt ');
    view.setUint32(16, 16, true);
    view.setUint16(20, 1, true);
    view.setUint16(22, 1, true);
    view.setUint32(24, targetRate, true);
    view.setUint32(28, targetRate * bytesPerSample, true);
    view.setUint16(32, bytesPerSample, true);
    view.setUint16(34, 16, true);
    writeAscii(view, 36, 'data');
    view.setUint32(40, dataSize, true);
    let offset = 44;
    for (const sample of samples) {
      const clamped = Math.max(-1, Math.min(1, sample));
      view.setInt16(offset, clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff, true);
      offset += 2;
    }
    return new Blob([buffer], { type: 'audio/wav' });
  }

  function writeAscii(view: DataView, offset: number, text: string) {
    for (let i = 0; i < text.length; i++) view.setUint8(offset + i, text.charCodeAt(i));
  }

  btn.addEventListener('click', () => {
    if (getVoiceEngine() !== 'whisper') return;
    startWhisperRecording();
  });

  cancelBtn.addEventListener('click', () => {
    if (getVoiceEngine() === 'whisper') cancelWhisperRecording();
  });

  confirmBtn.addEventListener('click', () => {
    if (getVoiceEngine() === 'whisper') finishWhisperRecording();
  });

  document.addEventListener('keydown', (e) => {
    if (getVoiceEngine() !== 'whisper') return;
    if (e.key === 'Escape' && (isRecording || isProcessing)) {
      e.preventDefault();
      cancelWhisperRecording();
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
    if (isRecording || isProcessing) resizeCanvas();
  });

  updateButtonVisibility();
})();
