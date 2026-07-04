import { t } from '../i18n.js';
import { approvalVisibleCache, sessions, terminals } from './state.js';
import { scanBuffer } from './terminal.js';

export type MobileTranscriptRole = 'user' | 'ai';

export interface MobileTranscriptMessage {
  id: number;
  role: MobileTranscriptRole;
  text: string;
  at: number;
  pending?: boolean;
}

interface MobileTranscriptState {
  messages: MobileTranscriptMessage[];
  nextId: number;
  lastBufferText: string;
  pendingEchoText: string;
  lastUserSubmittedAt: number;
}

const MAX_MESSAGES_PER_SESSION = 200;
const MAX_SCAN_LINES = 800;
const MAX_OVERLAP_CHARS = 2000;

const transcriptBySession = new Map<number, MobileTranscriptState>();

const mobileTranscriptMql = (typeof window !== 'undefined' && typeof window.matchMedia === 'function')
  ? window.matchMedia('(max-width: 720px)') : null;

function isMobileViewport(): boolean {
  return !!mobileTranscriptMql?.matches;
}

const RULE_RE = /^[\s_\-─=*]+$/;

function isRuleLine(s: string): boolean {
  const trimmed = s.trim();
  return trimmed.length >= 3 && RULE_RE.test(trimmed);
}

export function collapseDecorations(lines: string[]): string[] {
  const out: string[] = [];
  let blankRun = 0;
  let prevRule = false;
  for (const raw of lines) {
    const isBlank = raw.trim() === '';
    if (isBlank) {
      blankRun++;
      if (blankRun === 1) out.push(raw);
      prevRule = false;
      continue;
    }
    blankRun = 0;
    if (isRuleLine(raw)) {
      if (prevRule) continue;
      prevRule = true;
      out.push(raw);
      continue;
    }
    prevRule = false;
    out.push(raw);
  }
  return out;
}

function stateFor(sessionId: number): MobileTranscriptState {
  let state = transcriptBySession.get(sessionId);
  if (!state) {
    state = { messages: [], nextId: 1, lastBufferText: '', pendingEchoText: '', lastUserSubmittedAt: 0 };
    transcriptBySession.set(sessionId, state);
  }
  return state;
}

function cleanBufferText(sessionId: number): string {
  const lines = scanBuffer(sessionId, MAX_SCAN_LINES);
  while (lines.length > 0 && lines[lines.length - 1].trim() === '') lines.pop();
  return collapseDecorations(lines).join('\n').trim();
}

function trimPendingEcho(state: MobileTranscriptState, text: string): string {
  const echo = state.pendingEchoText.trim();
  if (!echo) return text;
  const trimmed = text.replace(/^\s+/, '');
  if (trimmed.startsWith(echo)) {
    state.pendingEchoText = '';
    return trimmed.slice(echo.length).replace(/^\s+/, '');
  }
  return text;
}

function suffixPrefixOverlap(prev: string, next: string): number {
  const max = Math.min(prev.length, next.length, MAX_OVERLAP_CHARS);
  if (max <= 0) return 0;
  const nextHead = next.slice(0, max);
  const prevTail = prev.slice(prev.length - max);
  const combined = `${nextHead}\0${prevTail}`;
  const table = new Array<number>(combined.length).fill(0);
  for (let i = 1; i < combined.length; i++) {
    let j = table[i - 1];
    while (j > 0 && combined[i] !== combined[j]) j = table[j - 1];
    if (combined[i] === combined[j]) j++;
    table[i] = j;
  }
  return Math.min(table[combined.length - 1] || 0, max);
}

function existingTranscriptText(state: MobileTranscriptState): string {
  return state.messages.map((msg) => msg.text).join('\n').trim();
}

function trimExistingTranscriptPrefix(state: MobileTranscriptState, text: string): string {
  const existing = existingTranscriptText(state);
  if (!existing) return text;
  const overlap = suffixPrefixOverlap(existing, text);
  return overlap > 0 ? text.slice(overlap) : text;
}

function appendMessage(state: MobileTranscriptState, role: MobileTranscriptRole, text: string, pending = false): void {
  const clean = text.trim();
  if (!clean) return;
  const last = state.messages[state.messages.length - 1];
  if (role === 'ai' && last && last.role === 'ai' && last.pending) {
    last.text = clean;
    last.pending = pending;
    last.at = Date.now();
  } else {
    state.messages.push({ id: state.nextId++, role, text: clean, at: Date.now(), pending });
  }
  while (state.messages.length > MAX_MESSAGES_PER_SESSION) state.messages.shift();
}

function sanitizeSubmittedText(text: string): string {
  let out = String(text || '');
  out = out.replace(/\x1b\[200~/g, '').replace(/\x1b\[201~/g, '');
  out = out.replace(/[\x00-\x09\x0b-\x1f\x7f]/g, '');
  out = out.replace(/\r/g, '\n').trim();
  return out;
}

export function recordMobileTranscriptUserSubmission(sessionId: number, text: string): void {
  if (!isMobileViewport()) return;
  const submitted = sanitizeSubmittedText(text);
  if (!submitted) return;
  const state = stateFor(sessionId);
  syncMobileTranscriptFromBuffer(sessionId, { commitPending: true });
  appendMessage(state, 'user', submitted, false);
  state.pendingEchoText = submitted;
  state.lastUserSubmittedAt = Date.now();
  state.lastBufferText = cleanBufferText(sessionId);
}

export function syncMobileTranscriptFromBuffer(sessionId: number, opts: { commitPending?: boolean } = {}): MobileTranscriptMessage[] {
  if (!isMobileViewport()) return transcriptBySession.get(sessionId)?.messages || [];
  const state = stateFor(sessionId);
  const current = cleanBufferText(sessionId);
  if (!state.lastBufferText) {
    state.lastBufferText = current;
    if (state.messages.length === 0 && current) {
      appendMessage(state, 'ai', current, !opts.commitPending);
    }
    return state.messages;
  }
  if (current === state.lastBufferText) {
    if (opts.commitPending) {
      const last = state.messages[state.messages.length - 1];
      if (last && last.role === 'ai') last.pending = false;
    }
    return state.messages;
  }

  let delta = '';
  if (current.startsWith(state.lastBufferText)) {
    delta = current.slice(state.lastBufferText.length);
  } else {
    const overlap = suffixPrefixOverlap(state.lastBufferText, current);
    delta = overlap > 0 ? current.slice(overlap) : current;
    delta = trimExistingTranscriptPrefix(state, delta);
  }
  delta = trimPendingEcho(state, delta);
  appendMessage(state, 'ai', delta, !opts.commitPending);
  state.lastBufferText = current;
  return state.messages;
}

export function getMobileTranscriptMessages(sessionId: number): MobileTranscriptMessage[] {
  return transcriptBySession.get(sessionId)?.messages || [];
}

export function clearMobileTranscriptSession(sessionId: number): void {
  transcriptBySession.delete(sessionId);
}

export function mobileTranscriptStatusText(sessionId: number): string {
  const session = sessions.get(sessionId);
  const state = String(session?.state || 'standby');
  if (approvalVisibleCache.get(sessionId) || state === 'waiting') {
    return t('mobile_status_waiting_approval');
  }
  if (state === 'disconnected' || !terminals.get(sessionId)) {
    return t('mobile_status_disconnected');
  }
  if (state === 'running') {
    const transcriptState = transcriptBySession.get(sessionId);
    const fallbackStarted = session?.started_at ? Date.parse(String(session.started_at)) : NaN;
    const since = transcriptState?.lastUserSubmittedAt || (Number.isFinite(fallbackStarted) ? fallbackStarted : Date.now());
    const minutes = Math.max(0, Math.floor((Date.now() - since) / 60000));
    return t('mobile_status_running', { min: String(minutes) });
  }
  return t('live_status_idle');
}
