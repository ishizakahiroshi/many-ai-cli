import { type HotwordListener, type HotwordListenerOptions, type SpeechRecognizer, type SpeechRecognizerOptions } from './recognition.js';
import { type WhisperFinishOutcome, type WhisperRecorder, type WhisperRecorderOptions } from './whisper.js';
export { createSpeechRecognizer, createHotwordListener, detectSupport, getSpeechRecognitionConstructor, isChromiumBrowser, normalizeVoiceErrorCode, classifyVoiceError, classifyVoiceErrorForDisplay, isSilentRecognitionError, diagnosticSeverity, shouldShowRecoveryGuide, appLangToRecognitionLang, VOICE_DIAG_EVENT_LIMIT, VOICE_DIAG_STUCK_MS, VOICE_DIAG_HARD_TIMEOUT_EXTRA_MS, HOTWORD_RESTART_DELAY_MS, HOTWORD_REARM_DELAY_MS, HOTWORD_MIC_RELEASE_WAIT_MS, HOTWORD_STOP_SAFETY_MS, HOTWORD_FATAL_ERRORS, } from './recognition.js';
export type { SpeechRecognitionAlternativeLike, SpeechRecognitionResultLike, SpeechRecognitionResultListLike, SpeechRecognitionEventLike, SpeechRecognitionErrorEventLike, SpeechRecognitionLike, SpeechRecognitionConstructor, NavigatorUADataLike, NavigatorLike, RecognitionSupport, SupportOptions, DiagnosticStatus, DiagnosticSeverity, DiagnosticSnapshot, VoiceErrorKind, RecognitionId, VoiceDiagEvent, VoiceDiagEventDetail, VoiceDiagReport, VoiceDiagnostics, SpeechRecognizerOptions, SpeechRecognizerState, SpeechRecognizerResult, SpeechRecognizerError, SpeechRecognizerEventMap, RecognitionActivity, StartOutcome, SpeechRecognizer, HotwordListenerOptions, HotwordState, HotwordListener, } from './recognition.js';
export { createWhisperRecorder, buildTranscribeUrl, graceSecondsToSilenceMs, prepareBaseText, joinTranscript, encodeWavPcm16, WHISPER_TARGET_SAMPLE_RATE, WHISPER_MAX_RECORD_SECONDS, WHISPER_MIN_RECORD_MS, WHISPER_MIN_VOICED_MS, WHISPER_MIN_PEAK_RMS, WHISPER_STRONG_PEAK_RMS, WHISPER_VAD_RMS_THRESHOLD, WHISPER_MAX_AUTO_STOP_SILENCE_SEC, WHISPER_DEFAULT_AUTO_STOP_SILENCE_MS, WHISPER_ERROR_CODES, } from './whisper.js';
export type { MediaStreamTrackLike, MediaStreamLike, GetUserMediaLike, AudioNodeLike, AnalyserNodeLike, GainNodeLike, AudioBufferLike, ScriptProcessorNodeLike, AudioWorkletNodeLike, AudioContextLike, AudioContextConstructor, AudioWorkletNodeConstructor, AbortSignalLike, FetchInitLike, FetchResponseLike, FetchLike, WhisperRecorderOptions, WhisperRecorderState, WhisperErrorPhase, WhisperRecorderError, WhisperRecorderEventMap, WhisperStartOutcome, WhisperFinishOutcome, WhisperRecorder, } from './whisper.js';
/** Engine names as many-ai-cli stores them. */
export type VoiceEngineName = 'browser' | 'whisper' | 'off';
/** Refusal code when the other engine holds (or is opening) the microphone. */
export declare const ENGINE_BUSY = "engine_busy";
export interface VoiceInputOptions {
    /**
     * Current engine, or a function read on every toggle/confirm/cancel. Default: `browser` when
     * `recognition` is given, else `whisper` when `whisper` is given, else `off`.
     */
    engine?: VoiceEngineName | (() => VoiceEngineName);
    /** Browser SpeechRecognition engine. Omit to have none. */
    recognition?: SpeechRecognizerOptions;
    /** Whisper engine. `canStart` is owned by createVoiceInput. Omit to have none. */
    whisper?: Omit<WhisperRecorderOptions, 'canStart'>;
    /**
     * Wake word listener. Its isVoiceBusy is combined with "browser recording or whisper active".
     * many-ai-cli keeps this disabled today; leave it out to keep that behaviour.
     */
    hotword?: HotwordListenerOptions;
}
export interface VoiceToggleOutcome {
    readonly engine: VoiceEngineName;
    /** A recording was started. */
    readonly started: boolean;
    /** A running recording was stopped or finished instead. */
    readonly stopped: boolean;
    /** `off`, `unavailable`, `engine_busy`, or the engine's own start error. */
    readonly error?: string;
}
export interface VoiceInput {
    /** Browser engine; start() and diagnostics.run() refuse with engine_busy while whisper is active. */
    readonly recognizer: SpeechRecognizer | null;
    /** Whisper engine; starting refuses with engine_busy while the recognizer or hotword is active. */
    readonly whisper: WhisperRecorder | null;
    readonly hotword: HotwordListener | null;
    getEngine(): VoiceEngineName;
    /** Which engine holds the microphone right now, if any. */
    getActiveEngine(): 'browser' | 'whisper' | null;
    isActive(): boolean;
    /** The voice button for the current engine. */
    toggle(): Promise<VoiceToggleOutcome>;
    /** The confirm button for the current engine. */
    confirm(): Promise<WhisperFinishOutcome | void>;
    /** The cancel button for the current engine. */
    cancel(): void;
    dispose(): void;
}
export declare function createVoiceInput(options: VoiceInputOptions): VoiceInput;
//# sourceMappingURL=index.d.ts.map