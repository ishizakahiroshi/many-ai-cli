export interface SpeechRecognitionAlternativeLike {
    readonly transcript: string;
    readonly confidence?: number;
}
export interface SpeechRecognitionResultLike {
    readonly isFinal: boolean;
    readonly length: number;
    readonly [index: number]: SpeechRecognitionAlternativeLike;
}
export interface SpeechRecognitionResultListLike {
    readonly length: number;
    readonly [index: number]: SpeechRecognitionResultLike;
}
export interface SpeechRecognitionEventLike {
    readonly resultIndex: number;
    readonly results: SpeechRecognitionResultListLike;
    readonly currentTarget?: unknown;
}
export interface SpeechRecognitionErrorEventLike {
    readonly error?: string;
    readonly message?: string;
    readonly currentTarget?: unknown;
}
export interface SpeechRecognitionLike {
    lang: string;
    continuous: boolean;
    interimResults: boolean;
    maxAlternatives: number;
    start(): void;
    stop(): void;
    abort(): void;
    onstart: ((ev: unknown) => void) | null;
    onaudiostart: ((ev: unknown) => void) | null;
    onsoundstart: ((ev: unknown) => void) | null;
    onspeechstart: ((ev: unknown) => void) | null;
    onspeechend: ((ev: unknown) => void) | null;
    onsoundend: ((ev: unknown) => void) | null;
    onaudioend: ((ev: unknown) => void) | null;
    onresult: ((ev: SpeechRecognitionEventLike) => void) | null;
    onnomatch: ((ev: unknown) => void) | null;
    onerror: ((ev: SpeechRecognitionErrorEventLike) => void) | null;
    onend: ((ev: unknown) => void) | null;
    addEventListener(type: string, listener: (ev: never) => void): void;
    removeEventListener(type: string, listener: (ev: never) => void): void;
}
export type SpeechRecognitionConstructor = new () => SpeechRecognitionLike;
export interface NavigatorUADataLike {
    readonly brands?: ReadonlyArray<{
        readonly brand: string;
        readonly version?: string;
    }>;
    readonly mobile?: boolean;
    readonly platform?: string;
}
export interface NavigatorLike {
    readonly userAgent?: string;
    readonly userAgentData?: NavigatorUADataLike;
}
/** Max entries kept in the diagnostic event log; older ones are dropped (voice.ts VOICE_DIAG_EVENT_LIMIT). */
export declare const VOICE_DIAG_EVENT_LIMIT = 80;
/** audioend without result/end/error for this long means "stuck" (voice.ts VOICE_DIAG_STUCK_MS). */
export declare const VOICE_DIAG_STUCK_MS = 20000;
/** Extra time on top of the stuck timeout before the diagnostic run gives up. */
export declare const VOICE_DIAG_HARD_TIMEOUT_EXTRA_MS = 10000;
/** Hotword: delay before re-listening after its own `end`. */
export declare const HOTWORD_RESTART_DELAY_MS = 250;
/** Hotword: delay before re-arming after the main voice input stopped. */
export declare const HOTWORD_REARM_DELAY_MS = 300;
/** Hotword: grace after `end` for Chrome to release the mic capture (empirical). */
export declare const HOTWORD_MIC_RELEASE_WAIT_MS = 50;
/** Hotword: resolve stopForVoiceInput() even if `end` never arrives. */
export declare const HOTWORD_STOP_SAFETY_MS = 500;
/** Hotword errors after which listening must be turned off by the host. */
export declare const HOTWORD_FATAL_ERRORS: readonly string[];
export interface RecognitionSupport {
    /** SpeechRecognition exists and the browser is Chromium. Otherwise the engine is `unsupported`. */
    readonly supported: boolean;
    readonly speechRecognitionSupported: boolean;
    readonly chromiumDetected: boolean;
}
export interface SupportOptions {
    /**
     * Constructor to use instead of `window.SpeechRecognition || window.webkitSpeechRecognition`.
     * `null` means "treat as missing". Omit to read the host global.
     */
    SpeechRecognition?: SpeechRecognitionConstructor | null;
    /** Navigator to inspect instead of the host `navigator`. */
    navigator?: NavigatorLike;
    /** Override the Chromium check (for tests or hosts that already know). */
    isChromium?: boolean;
}
/** `window.SpeechRecognition || window.webkitSpeechRecognition`, or null. */
export declare function getSpeechRecognitionConstructor(): SpeechRecognitionConstructor | null;
/**
 * Same test as voice.ts: userAgentData brands contain "Chromium"; only when brands are
 * unavailable, fall back to `Chrome/` in the user agent string.
 */
export declare function isChromiumBrowser(nav?: NavigatorLike | undefined): boolean;
/** Whether browser speech recognition can be used here (voice.ts `SpeechRecognition && isChromium`). */
export declare function detectSupport(options?: SupportOptions): RecognitionSupport;
/** Error code from a SpeechRecognitionErrorEvent, a DOMException, an Error or a string. */
export declare function normalizeVoiceErrorCode(error: unknown): string;
export type DiagnosticStatus = 'idle' | 'unsupported' | 'running' | 'healthy' | 'permission_denied' | 'audio_capture_failed' | 'speech_service_failed' | 'no_result' | 'profile_or_stt_stuck_suspected' | 'normal_profile_specific';
/** Map an error code to the diagnostic status it implies (voice.ts classifyVoiceError). */
export declare function classifyVoiceError(error: string): DiagnosticStatus;
export type VoiceErrorKind = 'permission' | 'audio_capture' | 'network' | 'service' | 'language' | 'other';
/**
 * Which message the host should show for an error (the branches of voice.ts showVoiceError).
 * The host owns the wording; `other` corresponds to the generic "error: {code}" message.
 */
export declare function classifyVoiceErrorForDisplay(error: unknown): VoiceErrorKind;
/** Errors the original never showed to the user during voice input. */
export declare function isSilentRecognitionError(error: string | null | undefined): boolean;
export type DiagnosticSeverity = 'ok' | 'err' | 'warn' | '';
/** Severity of a diagnostic status (voice.ts voiceDiagClass; the host maps it to its own styling). */
export declare function diagnosticSeverity(status: DiagnosticStatus): DiagnosticSeverity;
/** Whether the host should show its recovery guide for this status. */
export declare function shouldShowRecoveryGuide(status: DiagnosticStatus): boolean;
/**
 * many-ai-cli's UI language to recognition language (voice.ts getLang). `null`/empty means
 * the many-ai-cli default `ja`.
 */
export declare function appLangToRecognitionLang(appLang: string | null | undefined): string;
type Listener<T> = (payload: T) => void;
export type RecognitionId = number | string | null;
export interface VoiceDiagEvent {
    readonly timestamp: string;
    readonly recognitionId: RecognitionId;
    readonly event: string;
    readonly error: string | null;
    readonly message: string | null;
    readonly hasResult: boolean;
    readonly transcriptLength: number;
}
export interface VoiceDiagEventDetail {
    error?: string | null;
    message?: string | null;
    hasResult?: boolean;
    transcriptLength?: number;
}
export interface DiagnosticSnapshot {
    readonly status: DiagnosticStatus;
    /** Extra detail (usually the browser's error message). Empty string when none. */
    readonly message: string;
    readonly events: VoiceDiagEvent[];
}
export interface VoiceDiagReport {
    readonly generatedAt: string;
    readonly appVersion: string | null;
    readonly userAgent: string | undefined;
    readonly userAgentData: {
        brands: NavigatorUADataLike['brands'];
        mobile: boolean | undefined;
        platform: string | undefined;
    } | null;
    readonly origin: string | null;
    readonly isLocalOrigin: boolean;
    readonly speechRecognitionSupported: boolean;
    readonly chromiumDetected: boolean;
    readonly status: DiagnosticStatus;
    readonly message: string;
    readonly events: VoiceDiagEvent[];
}
export interface SpeechRecognizerOptions extends SupportOptions {
    /**
     * Recognition language (BCP 47), or a function read right before each start. voice.ts read
     * it at construction, on every instance re-creation and right before start().
     */
    lang: string | (() => string);
    /** Defaults to VOICE_DIAG_STUCK_MS (20000). */
    stuckTimeoutMs?: number;
    /** Defaults to VOICE_DIAG_EVENT_LIMIT (80). */
    diagnosticEventLimit?: number;
    /** Supplies `appVersion` for the diagnostic report (voice.ts read `.settings-app-version`). */
    getAppVersion?: () => string | null | undefined;
}
export interface SpeechRecognizerState {
    readonly supported: boolean;
    /** Between the native `start` event and the stop (many-ai-cli `voiceActive` / `recording`). */
    readonly recording: boolean;
    /**
     * start() accepted but the native `start` event has not arrived yet. voice.ts did not track
     * this; it is here so other engines can refuse to open the microphone meanwhile.
     */
    readonly starting: boolean;
    /** Microphone audio is being captured (many-ai-cli `voiceAudioActive`). */
    readonly audioActive: boolean;
    /** `audioend` seen, waiting for the result (many-ai-cli `voice-processing`). */
    readonly processing: boolean;
    /** A diagnostic run holds its own recognition instance. */
    readonly diagnosticRunning: boolean;
    /** Id of the current recognition instance. A new id is issued after every stop. */
    readonly recognitionId: number;
}
export type RecognitionActivity = 'audiostart' | 'soundstart' | 'speechstart' | 'speechend' | 'soundend' | 'audioend' | 'nomatch';
export interface SpeechRecognizerResult {
    readonly recognitionId: number;
    /**
     * False when the result came from an instance that was already replaced (for example the
     * final result that Chrome delivers after stop()). voice.ts applied those results too.
     */
    readonly isCurrent: boolean;
    /** `results[resultIndex][0].transcript`, passed through as is (may be an empty string). */
    readonly transcript: string;
    readonly isFinal: boolean;
    readonly resultIndex: number;
}
export interface SpeechRecognizerError {
    readonly recognitionId: number;
    /** Normalized code (`not-allowed`, `aborted`, `InvalidStateError`, ...). */
    readonly code: string;
    /** Raw `event.error` of an onerror event; null for a start() exception. */
    readonly error: string | null;
    readonly message: string | null;
    /** `recognition`: onerror event. `start`: start() threw. */
    readonly source: 'recognition' | 'start';
    /** voice.ts showed a toast for this error (everything except no-speech / aborted). */
    readonly notify: boolean;
}
export interface SpeechRecognizerEventMap {
    /** Any field of the state snapshot changed. */
    state: SpeechRecognizerState;
    /** Native `start` fired (voice.ts dispatched `voiceinput:started`). */
    start: {
        readonly recognitionId: number;
    };
    /** Recording ended for any reason (voice.ts dispatched `voiceinput:stopped`). */
    stop: {
        readonly recognitionId: number;
    };
    result: SpeechRecognizerResult;
    error: SpeechRecognizerError;
    /** audioActive changed (voice.ts dispatched `voiceinput:statechanged`). */
    audioActive: boolean;
    /** Lower-level recognition events, e.g. to drive a waveform. */
    activity: {
        readonly recognitionId: number;
        readonly kind: RecognitionActivity;
    };
    /** Diagnostic status changed (voice.ts dispatched `voiceinput:diagnostic`). */
    diagnostic: DiagnosticSnapshot;
}
export interface StartOutcome {
    readonly started: boolean;
    /** Error code when not started: `unsupported`, `disposed`, or the normalized start() error. */
    readonly error?: string;
}
export interface VoiceDiagnostics {
    getStatus(): DiagnosticStatus;
    getLastDetail(): string;
    getEvents(): VoiceDiagEvent[];
    getReport(): VoiceDiagReport;
    /** `JSON.stringify(getReport(), null, 2)`: the text voice.ts copied to the clipboard. */
    getReportJson(): string;
    /** Run the diagnostic with its own recognition instance (voice.ts runVoiceDiagnostic). */
    run(): void;
    /** Set status `normal_profile_specific` (window.__anyAiCliVoiceDiagnostics.markNormalProfileSpecific). */
    markNormalProfileSpecific(): void;
    /** Log the user's confirmation and set `normal_profile_specific` (the diagnostic "profile specific" button). */
    confirmNormalProfileSpecific(): void;
    pushEvent(recognitionId: RecognitionId, event: string, detail?: VoiceDiagEventDetail): VoiceDiagEvent;
}
export interface SpeechRecognizer {
    readonly support: RecognitionSupport;
    readonly diagnostics: VoiceDiagnostics;
    /**
     * Start listening (the non-recording branch of the voice button). Does not check whether
     * a recording is already running, like voice.ts; use isRecording() for the toggle.
     */
    start(): StartOutcome;
    /** Confirm: native stop() then finish immediately (voice.ts confirm button). */
    stop(): void;
    /** Cancel: native abort() then finish immediately (voice.ts cancel button / toggle while recording). */
    abort(): void;
    /**
     * Native stop() only; the finish happens on the `end` event (voice.ts trigger-phrase path).
     * Exceptions from the native call are not caught, as in voice.ts.
     */
    requestStop(): void;
    /** Log a `click` diagnostic event for the current instance (voice.ts button handler). */
    recordClick(): void;
    getState(): SpeechRecognizerState;
    isRecording(): boolean;
    isAudioActive(): boolean;
    /** recording, starting or a diagnostic run holds the microphone. */
    isActive(): boolean;
    getRecognitionId(): number;
    on<K extends keyof SpeechRecognizerEventMap>(type: K, handler: Listener<SpeechRecognizerEventMap[K]>): () => void;
    /** Abort, clear timers and drop all listeners. Not present in voice.ts (it lived for the page). */
    dispose(): void;
}
export declare function createSpeechRecognizer(options: SpeechRecognizerOptions): SpeechRecognizer;
export interface HotwordListenerOptions extends SupportOptions {
    lang: string | (() => string);
    /**
     * The wake phrase, or '' when wake word is disabled (voice.ts getWakePhrase: '' unless the
     * setting is on, then the trimmed phrase).
     */
    getPhrase: () => string;
    /** Text normalization applied to both the phrase and transcripts (voice.ts normalizeTriggerMatchText). Defaults to identity. */
    normalize?: (text: string) => string;
    /** Host conditions for listening: enabled, armed (global or session) and hovered. */
    canListen: () => boolean;
    /** Main voice input is running or about to (voice.ts `voiceActive || _voiceIntentActive()`). */
    isVoiceBusy?: () => boolean;
    /** Phrase detected. stopForVoiceInput() has already been started (not awaited, as in voice.ts). */
    onWake?: () => void;
    /** A fatal error: the host should disarm wake word and show the error. */
    onFatalError?: (error: {
        code: string;
        error: string;
        message: string | null;
    }) => void;
    /** Listening state changed (the places voice.ts called updateMicChip). */
    onChange?: (state: HotwordState) => void;
}
export interface HotwordState {
    readonly listening: boolean;
    readonly starting: boolean;
}
export interface HotwordListener {
    readonly support: RecognitionSupport;
    /** Start listening if canListen() and not already listening/starting (voice.ts startHotword). */
    start(): void;
    /** Abort listening and cancel a pending restart (voice.ts stopHotword). */
    stop(): void;
    /**
     * Release the microphone before the main recognizer starts (voice.ts stopHotwordForVoiceInput).
     * Resolves true if the hotword was active; always replaces the instance.
     */
    stopForVoiceInput(): Promise<boolean>;
    /** Call when the main voice input stopped: re-arms after a short delay (voice.ts `voiceinput:stopped` listener). */
    notifyVoiceStopped(): void;
    canListen(): boolean;
    getState(): HotwordState;
    /** listening or starting: the hotword holds (or is about to hold) the microphone. */
    isActive(): boolean;
    /** Not present in voice.ts. */
    dispose(): void;
}
export declare function createHotwordListener(options: HotwordListenerOptions): HotwordListener;
export {};
//# sourceMappingURL=recognition.d.ts.map