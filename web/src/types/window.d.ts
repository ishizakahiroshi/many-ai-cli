import type { Message, SessionSnapshot } from './proto.js';

type AnyFn = (...args: any[]) => any;

declare global {
  // TODO(ts): Replace these legacy DOM/dynamic-object compatibility shims with
  // explicit element narrowing and class fields after the migration stabilizes.
  interface Object {
    [key: string]: any;
  }

  interface EventTarget {
    closest?(selectors: string): Element | null;
    id?: string;
    dataset?: DOMStringMap;
  }

  interface Node {
    contains(other: EventTarget | null): boolean;
    classList: any;
  }

  interface Element {
    checked?: boolean;
    disabled?: boolean;
    hidden: any;
    open?: boolean;
    rel?: string;
    type?: string;
    href?: string;
    value?: string;
    files?: FileList | null;
    dataset: DOMStringMap;
    style: any;
    title: string;
    innerText: string;
    focus(options?: any): void;
    click(): void;
    _targetSession?: any;
    _searchInput?: any;
    _linkedMsg?: any;
  }

  interface Event {
    clientX: number;
    clientY: number;
    detail: any;
    waitUntil(promise: Promise<any>): void;
    notification: any;
  }

  interface Window {
    __lang?: 'ja' | 'en' | string;
    t?: (key: string, vars?: Record<string, unknown> | string) => string;
    setLang?: (lang: string) => void;
    showToast?: (message: string, anchor?: Element | null) => void;

    terminals?: Map<number, any>;
    activeSessionId?: number | null;
    orderSessions?: () => SessionSnapshot[];
    getSortedSessions?: () => SessionSnapshot[];
    getTerminalEntry?: (id: number) => any;
    sendResize?: (sessionId: number, cols: number, rows: number) => void;
    flushPendingTerminalChunks?: (id: number) => void;
    markTerminalManualScrollIntent?: () => void;
    updateScrollLockBtn?: (locked?: boolean) => void;

    multiPaneManager?: any;
    detachedGridManager?: any;
    activateSessionForMultiPane?: (id: number) => void;
    dismissSession?: (id: number) => void;
    renderSessionList?: () => void;
    setActiveTab?: AnyFn;
    syncMobileLayoutState?: () => void;
    closeMobileSessionDrawer?: () => void;
    _c5SidebarUpdating?: boolean;

    approvalParser?: any;
    approvalUiAdapter?: any;
    matchProviderApprovalTrigger?: (provider: string, line: string) => boolean;
    approvalPatternsUI?: any;

    chatHistoryAPI?: any;
    mountChatPaneForSession?: AnyFn;
    appendChatMessage?: AnyFn;
    _chatC4DecorateBubble?: AnyFn;
    _chatC4OnRemount?: AnyFn;
    _chatC4OnAppend?: AnyFn;

    GitGraphView?: any;

    __settingsSaveAll?: () => Promise<void>;
    __settingsResetAll?: () => Promise<void>;

    __anyAiCliVoiceDiagnostics?: any;
    _voiceIntentActive?: () => boolean;
    _showVoiceRecognitionError?: AnyFn;
    _wakewordSessionChanged?: () => void;
    _wakewordSessionRemoved?: (id: number) => void;
    _wakewordGlobalActive?: () => boolean;
    _wakewordSessionActive?: (id: number) => boolean;
    _stopWakewordForVoiceInput?: () => Promise<void> | void;

    webkitAudioContext?: typeof AudioContext;
    SpeechRecognition?: any;
    webkitSpeechRecognition?: any;

    [key: string]: any;
  }

  interface Navigator {
    standalone?: boolean;
    userAgentData?: {
      platform?: string;
      brands?: Array<{ brand: string; version: string }>;
      mobile?: boolean;
      getHighEntropyValues?: (hints: string[]) => Promise<Record<string, unknown>>;
    };
  }

  interface HTMLElement {
    width: number;
    height: number;
    getContext(contextId: string, options?: any): any;
  }

  interface HTMLImageElement {
    controls: boolean;
    autoplay: boolean;
    playsInline: boolean;
    pause(): void;
  }

  interface ServiceWorkerGlobalScope {
    __WB_MANIFEST?: unknown;
  }

  const self: ServiceWorkerGlobalScope & typeof globalThis;
  var module: { exports?: any } | undefined;
}

export {};
