// vtype-core: Whisper path (record with WebAudio, encode WAV, POST to a transcription endpoint).
//
// Ported from many-ai-cli `web/src/app/voice-whisper.ts`. As with recognition.ts, nothing is
// rendered: no DOM lookups, no class names, no toasts, no storage. The transcription endpoint
// and its token are passed in by the caller; the core holds no URL of its own.
//
// Recording uses getUserMedia + AudioContext (AudioWorklet when the host provides a worklet
// module, ScriptProcessor otherwise), not MediaRecorder, exactly like the source: MediaRecorder
// produces AAC on iOS and would need ffmpeg on the server.
//
// getUserMedia, AudioContext, AudioWorkletNode, fetch and the clock are injectable so tests
// run without a browser and an extension offscreen document can supply its own.
function host() {
    return globalThis;
}
// ---------------------------------------------------------------------------
// Constants (values unchanged from voice-whisper.ts)
// ---------------------------------------------------------------------------
export const WHISPER_TARGET_SAMPLE_RATE = 16000;
export const WHISPER_MAX_RECORD_SECONDS = 120;
export const WHISPER_MIN_RECORD_MS = 250;
export const WHISPER_MIN_VOICED_MS = 60;
export const WHISPER_MIN_PEAK_RMS = 0.012;
/**
 * RDP mic redirection or noise suppression delivers speech in short bursts, so the voiced time
 * is under-counted. A strong enough peak counts as speech regardless of voiced time.
 */
export const WHISPER_STRONG_PEAK_RMS = 0.05;
export const WHISPER_VAD_RMS_THRESHOLD = 0.004;
export const WHISPER_MAX_AUTO_STOP_SILENCE_SEC = 10;
/** many-ai-cli DEFAULT_VOICE_GRACE_SEC (2 s), used when no silence setting is given. */
export const WHISPER_DEFAULT_AUTO_STOP_SILENCE_MS = 2000;
/**
 * Error codes a whisper session can report. The first four come from the transcription server
 * (many-ai-cli Hub); `hallucination` is only listed because the source had a message for it.
 */
export const WHISPER_ERROR_CODES = [
    'whisper_not_configured',
    'whisper_unreachable',
    'whisper_timeout',
    'whisper_failed',
    'no_speech',
    'empty_result',
    'hallucination',
    'permission_denied',
    'audio_capture',
];
// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------
/**
 * "Silence before auto stop" setting in seconds to milliseconds (voice-whisper.ts
 * getAutoStopSilenceMs): parseInt, clamp to 0..10 s, fall back to the default on a bad value.
 */
export function graceSecondsToSilenceMs(raw, defaultSec) {
    const sec = raw == null ? defaultSec : parseInt(String(raw), 10);
    const clamped = Number.isFinite(sec) ? Math.max(0, Math.min(WHISPER_MAX_AUTO_STOP_SILENCE_SEC, sec)) : defaultSec;
    return clamped * 1000;
}
/**
 * Transcription URL: `endpoint` plus `<param>=<encoded token>` when a token is configured.
 * With endpoint `/x` and token `t` this is `/x?token=t`, the shape voice-whisper.ts built.
 */
export function buildTranscribeUrl(endpoint, token, tokenQueryParam = 'token') {
    if (token === undefined)
        return endpoint;
    const sep = endpoint.includes('?') ? '&' : '?';
    return `${endpoint}${sep}${tokenQueryParam}=${encodeURIComponent(token || '')}`;
}
/**
 * Text before recording starts: add one space when the field has text that does not end in
 * whitespace (both engines did this in many-ai-cli before recording).
 */
export function prepareBaseText(value) {
    return value.length > 0 && !/\s$/.test(value) ? value + ' ' : value;
}
/** Field value after a whisper transcript (voice-whisper.ts insertTranscribedText). */
export function joinTranscript(base, text) {
    const prefix = base && !/\s$/.test(base) ? `${base} ` : base;
    return `${prefix}${text.trim()} `;
}
function mixToMono(input, channels) {
    if (channels <= 1)
        return input;
    const frames = Math.floor(input.length / channels);
    const out = new Float32Array(frames);
    for (let i = 0; i < frames; i++) {
        let sum = 0;
        for (let ch = 0; ch < channels; ch++)
            sum += input[i * channels + ch] || 0;
        out[i] = sum / channels;
    }
    return out;
}
function flattenChunks(input) {
    let length = 0;
    for (const chunk of input)
        length += chunk.length;
    const out = new Float32Array(length);
    let offset = 0;
    for (const chunk of input) {
        out.set(chunk, offset);
        offset += chunk.length;
    }
    return out;
}
function resampleLinear(input, fromRate, toRate) {
    if (!input.length || fromRate === toRate)
        return input;
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
function writeAscii(view, offset, text) {
    for (let i = 0; i < text.length; i++)
        view.setUint8(offset + i, text.charCodeAt(i));
}
/** Mono 16-bit PCM WAV bytes, resampled linearly to targetRate (voice-whisper.ts encodeWav). */
export function encodeWavPcm16(inputChunks, sourceRate, targetRate) {
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
    return buffer;
}
function errorField(err, field) {
    if (err && typeof err === 'object' && field in err) {
        const v = err[field];
        return typeof v === 'string' ? v : '';
    }
    return '';
}
class Emitter {
    listeners = new Map();
    on(type, fn) {
        let set = this.listeners.get(type);
        if (!set) {
            set = new Set();
            this.listeners.set(type, set);
        }
        set.add(fn);
        return () => {
            set.delete(fn);
        };
    }
    emit(type, payload) {
        const set = this.listeners.get(type);
        if (!set)
            return;
        for (const fn of Array.from(set)) {
            try {
                fn(payload);
            }
            catch (err) {
                host().console?.error?.('vtype-core listener failed:', err);
            }
        }
    }
    clear() {
        this.listeners.clear();
    }
}
function readOption(value) {
    return typeof value === 'function' ? value() : value;
}
export function createWhisperRecorder(options) {
    const emitter = new Emitter();
    const g = host();
    const mediaDevices = g.navigator?.mediaDevices;
    const getUserMedia = options.getUserMedia !== undefined
        ? options.getUserMedia
        : (mediaDevices?.getUserMedia ? (c) => mediaDevices.getUserMedia(c) : null);
    const AudioContextCtor = options.AudioContext !== undefined
        ? options.AudioContext
        : (g.AudioContext || g.webkitAudioContext || null);
    const WorkletNodeCtor = options.AudioWorkletNode !== undefined
        ? options.AudioWorkletNode
        : (g.AudioWorkletNode || null);
    const supported = !!(getUserMedia && AudioContextCtor);
    const now = options.now ?? (() => (g.performance ? g.performance.now() : Date.now()));
    const maxRecordMs = options.maxRecordMs ?? WHISPER_MAX_RECORD_SECONDS * 1000;
    let isRecording = false;
    let isProcessing = false;
    let isStarting = false;
    let audioActive = false;
    let disposed = false;
    let stream = null;
    let audioCtx = null;
    let sourceNode = null;
    let workletNode = null;
    let scriptNode = null;
    let analyserNode = null;
    let silentGainNode = null;
    let abortController = null;
    let startedAt = 0;
    let maxRecordTimer = null;
    let totalSampleCount = 0;
    let voicedSampleCount = 0;
    let peakRms = 0;
    let silentSampleCount = 0;
    const chunks = [];
    let lastState = null;
    function getState() {
        return { supported, starting: isStarting, recording: isRecording, processing: isProcessing, audioActive };
    }
    function emitState() {
        const next = getState();
        const prev = lastState;
        if (prev
            && prev.starting === next.starting
            && prev.recording === next.recording
            && prev.processing === next.processing
            && prev.audioActive === next.audioActive)
            return;
        lastState = next;
        emitter.emit('state', next);
    }
    function setAudioActive(active) {
        if (audioActive === active)
            return;
        audioActive = active;
        emitter.emit('audioActive', active);
    }
    function getAudioLevel() {
        if (!analyserNode)
            return 0.05;
        const data = new Uint8Array(analyserNode.fftSize);
        analyserNode.getByteTimeDomainData(data);
        let sum = 0;
        for (const v of data) {
            const x = (v - 128) / 128;
            sum += x * x;
        }
        return Math.min(1, Math.sqrt(sum / Math.max(1, data.length)) * 4);
    }
    function appendChunk(chunk) {
        observeAudioChunk(chunk);
        chunks.push(new Float32Array(chunk));
    }
    function observeAudioChunk(chunk) {
        if (!chunk.length)
            return;
        let sum = 0;
        for (const sample of chunk)
            sum += sample * sample;
        const rms = Math.sqrt(sum / chunk.length);
        totalSampleCount += chunk.length;
        peakRms = Math.max(peakRms, rms);
        if (rms >= WHISPER_VAD_RMS_THRESHOLD) {
            voicedSampleCount += chunk.length;
            silentSampleCount = 0;
        }
        else {
            silentSampleCount += chunk.length;
        }
        maybeAutoStopOnSilence();
    }
    function autoStopEnabled() {
        return options.autoStop === undefined ? true : readOption(options.autoStop);
    }
    function autoStopSilenceMs() {
        return options.autoStopSilenceMs === undefined
            ? WHISPER_DEFAULT_AUTO_STOP_SILENCE_MS
            : readOption(options.autoStopSilenceMs);
    }
    function maybeAutoStopOnSilence() {
        if (!isRecording || isProcessing || !autoStopEnabled())
            return;
        // 発話前の無音では切らない（発話とみなす条件は hasMeaningfulAudio と同じピーク基準）
        if (peakRms < WHISPER_MIN_PEAK_RMS)
            return;
        const sampleRate = audioCtx?.sampleRate || WHISPER_TARGET_SAMPLE_RATE;
        const silentMs = (silentSampleCount / Math.max(1, sampleRate)) * 1000;
        if (silentMs >= autoStopSilenceMs()) {
            void finish();
        }
    }
    async function attachRecorder(ctx, source) {
        analyserNode = ctx.createAnalyser();
        analyserNode.fftSize = 256;
        source.connect(analyserNode);
        silentGainNode = ctx.createGain();
        silentGainNode.gain.value = 0;
        silentGainNode.connect(ctx.destination);
        const worklet = options.recorderWorklet;
        if (ctx.audioWorklet && worklet && WorkletNodeCtor) {
            await ctx.audioWorklet.addModule(worklet.url);
            workletNode = new WorkletNodeCtor(ctx, worklet.processorName);
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
                for (let i = 0; i < frames; i++)
                    interleaved[i * channels + ch] = data[i];
            }
            appendChunk(mixToMono(interleaved, channels));
        };
        source.connect(scriptNode);
        scriptNode.connect(silentGainNode);
    }
    function stopMediaGraph() {
        if (maxRecordTimer != null)
            g.clearTimeout(maxRecordTimer);
        maxRecordTimer = null;
        try {
            workletNode?.disconnect();
        }
        catch (_) { /* ignore */ }
        try {
            scriptNode?.disconnect();
        }
        catch (_) { /* ignore */ }
        try {
            analyserNode?.disconnect();
        }
        catch (_) { /* ignore */ }
        try {
            sourceNode?.disconnect();
        }
        catch (_) { /* ignore */ }
        try {
            silentGainNode?.disconnect();
        }
        catch (_) { /* ignore */ }
        stream?.getTracks().forEach((track) => {
            try {
                track.stop();
            }
            catch (_) { /* ignore */ }
        });
        if (audioCtx && audioCtx.state !== 'closed') {
            audioCtx.close().catch(() => { });
        }
        stream = null;
        audioCtx = null;
        sourceNode = null;
        workletNode = null;
        scriptNode = null;
        analyserNode = null;
        silentGainNode = null;
        setAudioActive(false);
    }
    function emitError(error) {
        emitter.emit('error', error);
    }
    async function beginRecording() {
        chunks.length = 0;
        totalSampleCount = 0;
        voicedSampleCount = 0;
        peakRms = 0;
        silentSampleCount = 0;
        isStarting = true;
        emitState();
        try {
            stream = await getUserMedia({ audio: true });
            audioCtx = new AudioContextCtor({ sampleRate: WHISPER_TARGET_SAMPLE_RATE });
            sourceNode = audioCtx.createMediaStreamSource(stream);
            await attachRecorder(audioCtx, sourceNode);
            if (audioCtx.state === 'suspended')
                await audioCtx.resume();
        }
        catch (err) {
            isStarting = false;
            stopMediaGraph();
            const name = String(errorField(err, 'name') || errorField(err, 'message') || '').toLowerCase();
            const code = name.includes('notallowed') || name.includes('permission') ? 'permission_denied' : 'audio_capture';
            emitError({ code, phase: 'start', cancelled: false, message: errorField(err, 'message') || null });
            emitState();
            return { started: false, error: code };
        }
        isStarting = false;
        if (disposed) {
            stopMediaGraph();
            emitState();
            return { started: false, error: 'disposed' };
        }
        isRecording = true;
        startedAt = now();
        setAudioActive(true);
        emitter.emit('start', {});
        maxRecordTimer = g.setTimeout(() => {
            if (isRecording)
                void finish();
        }, maxRecordMs);
        emitState();
        return { started: true };
    }
    function precheck() {
        if (disposed)
            return { started: false, error: 'disposed' };
        if (!supported) {
            emitError({ code: 'audio_capture', phase: 'start', cancelled: false, message: null });
            return { started: false, error: 'audio_capture' };
        }
        if (isProcessing) {
            emitter.emit('notice', { code: 'processing' });
            return { started: false, error: 'processing' };
        }
        return null;
    }
    function gateNewRecording() {
        // Not in voice-whisper.ts: a second start while getUserMedia is pending would open a
        // second stream and orphan the first one (the microphone would stay on).
        if (isStarting)
            return { started: false, error: 'already_starting' };
        const refusal = options.canStart?.() ?? null;
        if (refusal)
            return { started: false, error: refusal };
        return null;
    }
    async function toggle() {
        const pre = precheck();
        if (pre)
            return pre;
        if (isRecording) {
            await finish();
            return { started: false, finished: true };
        }
        const gate = gateNewRecording();
        if (gate)
            return gate;
        return beginRecording();
    }
    async function start() {
        const pre = precheck();
        if (pre)
            return pre;
        if (isRecording)
            return { started: false, error: 'already_recording' };
        const gate = gateNewRecording();
        if (gate)
            return gate;
        return beginRecording();
    }
    function cancel() {
        if (!isRecording && !isProcessing)
            return;
        abortController?.abort();
        abortController = null;
        isRecording = false;
        isProcessing = false;
        stopMediaGraph();
        emitter.emit('stop', { reason: 'cancel' });
        emitState();
    }
    function hasMeaningfulAudio(sourceRate, elapsedMs) {
        if (elapsedMs < WHISPER_MIN_RECORD_MS || chunks.length === 0 || totalSampleCount === 0)
            return false;
        const voicedMs = (voicedSampleCount / Math.max(1, sourceRate)) * 1000;
        if (peakRms >= WHISPER_STRONG_PEAK_RMS)
            return true;
        return peakRms >= WHISPER_MIN_PEAK_RMS && voicedMs >= WHISPER_MIN_VOICED_MS;
    }
    function discardWhisperResult(reason, detail = {}) {
        g.console?.warn?.('[voice-whisper] discarded transcription', { reason, ...detail });
        throw new Error(reason);
    }
    async function transcribeWav(body, signal) {
        const doFetch = options.fetch !== undefined
            ? options.fetch
            : (g.fetch ? (url, init) => g.fetch(url, init) : null);
        if (!doFetch)
            throw new Error('whisper_failed');
        const token = options.token === undefined ? undefined : readOption(options.token) ?? null;
        const url = buildTranscribeUrl(readOption(options.endpoint), token, options.tokenQueryParam);
        const init = {
            method: 'POST',
            headers: { 'Content-Type': 'audio/wav' },
            body,
        };
        if (signal)
            init.signal = signal;
        const res = await doFetch(url, init);
        let json = null;
        try {
            json = await res.json();
        }
        catch (_) { /* ignore */ }
        const b = json && typeof json === 'object' ? json : null;
        if (!res.ok)
            throw new Error(String(b?.error || 'whisper_failed'));
        const text = String(b?.text || '').trim();
        if (!text)
            discardWhisperResult('empty_result');
        return text;
    }
    async function finish() {
        if (isProcessing) {
            emitter.emit('notice', { code: 'processing' });
            return { finished: false, reason: 'processing' };
        }
        if (!isRecording)
            return { finished: false, reason: 'not_recording' };
        isRecording = false;
        isProcessing = true;
        const sourceRate = audioCtx?.sampleRate || WHISPER_TARGET_SAMPLE_RATE;
        const elapsedMs = now() - startedAt;
        stopMediaGraph();
        emitter.emit('processing', {});
        emitState();
        try {
            if (!hasMeaningfulAudio(sourceRate, elapsedMs)) {
                discardWhisperResult('no_speech', {
                    elapsedMs,
                    totalSampleCount,
                    voicedSampleCount,
                    peakRms,
                });
            }
            const wav = encodeWavPcm16(chunks, sourceRate, WHISPER_TARGET_SAMPLE_RATE);
            const BlobCtor = g.Blob;
            const body = BlobCtor ? new BlobCtor([wav], { type: 'audio/wav' }) : wav;
            abortController = g.AbortController ? new g.AbortController() : null;
            const text = await transcribeWav(body, abortController?.signal);
            // Cleared before notifying so a `stop` listener can start the other engine right away.
            abortController = null;
            isProcessing = false;
            emitter.emit('result', { text });
            emitter.emit('stop', { reason: 'result' });
            emitState();
            return { finished: true, text };
        }
        catch (err) {
            const cancelled = errorField(err, 'name') === 'AbortError';
            const code = cancelled ? 'cancelled' : String(errorField(err, 'message') || err || 'whisper_failed');
            abortController = null;
            isProcessing = false;
            emitError({ code, phase: 'finish', cancelled, message: errorField(err, 'message') || null });
            emitter.emit('stop', { reason: cancelled ? 'cancel' : 'error' });
            emitState();
            return { finished: false, reason: cancelled ? 'cancelled' : 'error', error: code };
        }
        finally {
            abortController = null;
            isProcessing = false;
        }
    }
    function dispose() {
        if (disposed)
            return;
        cancel();
        disposed = true;
        stopMediaGraph();
        emitter.clear();
    }
    lastState = getState();
    return {
        support: { supported },
        toggle,
        start,
        finish,
        cancel,
        getState,
        isRecording: () => isRecording,
        isProcessing: () => isProcessing,
        isActive: () => isStarting || isRecording || isProcessing,
        getAudioLevel,
        on: (type, handler) => emitter.on(type, handler),
        dispose,
    };
}
