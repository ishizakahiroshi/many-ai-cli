export interface MediaStreamTrackLike {
    stop(): void;
}
export interface MediaStreamLike {
    getTracks(): MediaStreamTrackLike[];
}
export type GetUserMediaLike = (constraints: {
    audio: boolean;
}) => Promise<MediaStreamLike>;
export interface AudioNodeLike {
    connect(destination: AudioNodeLike): unknown;
    disconnect(): void;
}
export interface AnalyserNodeLike extends AudioNodeLike {
    fftSize: number;
    getByteTimeDomainData(array: Uint8Array): void;
}
export interface GainNodeLike extends AudioNodeLike {
    readonly gain: {
        value: number;
    };
}
export interface AudioBufferLike {
    readonly numberOfChannels: number;
    readonly length: number;
    getChannelData(channel: number): Float32Array;
}
export interface ScriptProcessorNodeLike extends AudioNodeLike {
    onaudioprocess: ((event: {
        readonly inputBuffer: AudioBufferLike;
    }) => void) | null;
}
export interface AudioWorkletNodeLike extends AudioNodeLike {
    readonly port: {
        onmessage: ((event: {
            readonly data: Float32Array;
        }) => void) | null;
    };
}
export interface AudioContextLike {
    readonly sampleRate: number;
    readonly state: string;
    readonly destination: AudioNodeLike;
    readonly audioWorklet?: {
        addModule(url: string): Promise<void>;
    };
    createMediaStreamSource(stream: MediaStreamLike): AudioNodeLike;
    createAnalyser(): AnalyserNodeLike;
    createGain(): GainNodeLike;
    createScriptProcessor(bufferSize: number, inputChannels: number, outputChannels: number): ScriptProcessorNodeLike;
    resume(): Promise<void>;
    close(): Promise<void>;
}
export type AudioContextConstructor = new (options: {
    sampleRate: number;
}) => AudioContextLike;
export type AudioWorkletNodeConstructor = new (context: AudioContextLike, processorName: string) => AudioWorkletNodeLike;
export interface AbortSignalLike {
    readonly aborted?: boolean;
}
export interface FetchInitLike {
    method: string;
    headers: Record<string, string>;
    body: unknown;
    signal?: AbortSignalLike;
}
export interface FetchResponseLike {
    readonly ok: boolean;
    readonly status?: number;
    json(): Promise<unknown>;
}
export type FetchLike = (url: string, init: FetchInitLike) => Promise<FetchResponseLike>;
export declare const WHISPER_TARGET_SAMPLE_RATE = 16000;
export declare const WHISPER_MAX_RECORD_SECONDS = 120;
export declare const WHISPER_MIN_RECORD_MS = 250;
export declare const WHISPER_MIN_VOICED_MS = 60;
export declare const WHISPER_MIN_PEAK_RMS = 0.012;
/**
 * RDP mic redirection or noise suppression delivers speech in short bursts, so the voiced time
 * is under-counted. A strong enough peak counts as speech regardless of voiced time.
 */
export declare const WHISPER_STRONG_PEAK_RMS = 0.05;
export declare const WHISPER_VAD_RMS_THRESHOLD = 0.004;
export declare const WHISPER_MAX_AUTO_STOP_SILENCE_SEC = 10;
/** many-ai-cli DEFAULT_VOICE_GRACE_SEC (2 s), used when no silence setting is given. */
export declare const WHISPER_DEFAULT_AUTO_STOP_SILENCE_MS = 2000;
/**
 * Error codes a whisper session can report. The first four come from the transcription server
 * (many-ai-cli Hub); `hallucination` is only listed because the source had a message for it.
 */
export declare const WHISPER_ERROR_CODES: readonly ["whisper_not_configured", "whisper_unreachable", "whisper_timeout", "whisper_failed", "no_speech", "empty_result", "hallucination", "permission_denied", "audio_capture"];
/**
 * "Silence before auto stop" setting in seconds to milliseconds (voice-whisper.ts
 * getAutoStopSilenceMs): parseInt, clamp to 0..10 s, fall back to the default on a bad value.
 */
export declare function graceSecondsToSilenceMs(raw: string | number | null | undefined, defaultSec: number): number;
/**
 * Transcription URL: `endpoint` plus `<param>=<encoded token>` when a token is configured.
 * With endpoint `/x` and token `t` this is `/x?token=t`, the shape voice-whisper.ts built.
 */
export declare function buildTranscribeUrl(endpoint: string, token?: string | null, tokenQueryParam?: string): string;
/**
 * Text before recording starts: add one space when the field has text that does not end in
 * whitespace (both engines did this in many-ai-cli before recording).
 */
export declare function prepareBaseText(value: string): string;
/** Field value after a whisper transcript (voice-whisper.ts insertTranscribedText). */
export declare function joinTranscript(base: string, text: string): string;
/** Mono 16-bit PCM WAV bytes, resampled linearly to targetRate (voice-whisper.ts encodeWav). */
export declare function encodeWavPcm16(inputChunks: Float32Array[], sourceRate: number, targetRate: number): ArrayBuffer;
type Listener<T> = (payload: T) => void;
export interface WhisperRecorderOptions {
    /** Transcription endpoint (URL string), or a function read on every request. Required. */
    endpoint: string | (() => string);
    /**
     * Token sent as `?token=` (see tokenQueryParam), or a function read on every request.
     * Omit to send no token parameter. null / '' still send an empty parameter, as many-ai-cli did.
     */
    token?: string | null | (() => string | null | undefined);
    /** Query parameter name for the token. Default `token`. */
    tokenQueryParam?: string;
    /**
     * AudioWorklet module that posts mono Float32Array chunks (many-ai-cli serves
     * `whisper-recorder-worklet.js` registering `many-ai-cli-whisper-recorder`). Without it,
     * the ScriptProcessor fallback of the source is used.
     */
    recorderWorklet?: {
        url: string;
        processorName: string;
    };
    /** Auto stop after silence once speech was heard. Default true (many-ai-cli: setting != '0'). */
    autoStop?: boolean | (() => boolean);
    /** Silence length that triggers auto stop. Default 2000 ms. See graceSecondsToSilenceMs. */
    autoStopSilenceMs?: number | (() => number);
    /** Hard recording limit. Default 120000 ms. */
    maxRecordMs?: number;
    /**
     * Called right before the microphone would be opened. Return an error code to refuse the
     * start (createVoiceInput uses this for `engine_busy`), or null to allow it.
     */
    canStart?: () => string | null;
    /** Injectable host APIs. `null` means "not available". Omit to use the host globals. */
    getUserMedia?: GetUserMediaLike | null;
    AudioContext?: AudioContextConstructor | null;
    AudioWorkletNode?: AudioWorkletNodeConstructor | null;
    fetch?: FetchLike | null;
    now?: () => number;
}
export interface WhisperRecorderState {
    readonly supported: boolean;
    /** getUserMedia / audio graph setup in progress. */
    readonly starting: boolean;
    readonly recording: boolean;
    /** Recording finished, waiting for the transcription. */
    readonly processing: boolean;
    readonly audioActive: boolean;
}
export type WhisperErrorPhase = 'start' | 'finish';
export interface WhisperRecorderError {
    /** `permission_denied`, `audio_capture`, `no_speech`, `empty_result`, a server code, or `cancelled`. */
    readonly code: string;
    readonly phase: WhisperErrorPhase;
    /** The request was aborted by cancel(); voice-whisper.ts showed "cancelled" instead of an error. */
    readonly cancelled: boolean;
    readonly message: string | null;
}
export interface WhisperRecorderEventMap {
    state: WhisperRecorderState;
    /** Recording began (voice-whisper.ts setRecordingUi(true) / `voiceinput:started`). */
    start: Record<string, never>;
    /** Recording ended, transcription requested (voice-bar `voice-processing`). */
    processing: Record<string, never>;
    /** Trimmed, non-empty transcript. */
    result: {
        readonly text: string;
    };
    error: WhisperRecorderError;
    /** start/finish called while a transcription is running (voice-whisper.ts "processing" toast). */
    notice: {
        readonly code: 'processing';
    };
    /** Session over (voice-whisper.ts `voiceinput:stopped`). `result` means the text was delivered. */
    stop: {
        readonly reason: 'cancel' | 'result' | 'error';
    };
    audioActive: boolean;
}
export interface WhisperStartOutcome {
    readonly started: boolean;
    /** toggle() finished a running recording instead of starting one. */
    readonly finished?: boolean;
    /**
     * `audio_capture` (unsupported or capture failed), `permission_denied`, `processing`,
     * `already_recording`, `already_starting`, `disposed`, or the canStart() refusal (`engine_busy`).
     */
    readonly error?: string;
}
export interface WhisperFinishOutcome {
    readonly finished: boolean;
    readonly text?: string;
    readonly reason?: 'processing' | 'not_recording' | 'cancelled' | 'error';
    readonly error?: string;
}
export interface WhisperRecorder {
    readonly support: {
        readonly supported: boolean;
    };
    /** The voice button: finish when recording, otherwise start (voice-whisper.ts startWhisperRecording). */
    toggle(): Promise<WhisperStartOutcome>;
    /** Start only; refuses with `already_recording` when recording. */
    start(): Promise<WhisperStartOutcome>;
    /** Confirm: stop recording and transcribe (voice-whisper.ts finishWhisperRecording). */
    finish(): Promise<WhisperFinishOutcome>;
    /** Cancel recording or the pending transcription (voice-whisper.ts cancelWhisperRecording). */
    cancel(): void;
    getState(): WhisperRecorderState;
    isRecording(): boolean;
    isProcessing(): boolean;
    /** starting, recording or processing. */
    isActive(): boolean;
    /** Current input level 0..1 for a waveform (voice-whisper.ts readAudioLevel; 0.05 without analyser). */
    getAudioLevel(): number;
    on<K extends keyof WhisperRecorderEventMap>(type: K, handler: Listener<WhisperRecorderEventMap[K]>): () => void;
    /** Not present in voice-whisper.ts. */
    dispose(): void;
}
export declare function createWhisperRecorder(options: WhisperRecorderOptions): WhisperRecorder;
export {};
//# sourceMappingURL=whisper.d.ts.map