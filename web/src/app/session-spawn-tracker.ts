import { t } from '../i18n.js';
import { showToast } from './util.js';
import type { SessionSnapshot } from '../types/proto.js';
import { assignCorrelatedSessionListSpawn, captureSessionListTab } from './session-list-tabs.js';
import { beginSpawnPending, liveSpawnMatches, matchSpawnCorrelation, normalizeSpawnPending, pruneSpawnPending,
  recordSpawnMatch, setSpawnHubInstance, SPAWN_PENDING_TTL_MS, type SpawnCorrelation, type SpawnPendingState } from './session-spawn-state.js';
const STORAGE_KEY = 'many-ai-cli-pending-spawns-v1'; // sessionStorage: another browser must not consume our operations.
let state: SpawnPendingState | null = null;
let connected = false;
let storageWarned = false;
let live: SessionSnapshot[] = [];
const candidates = new Map<string, SpawnCorrelation>();
const listeners = new Set<() => void>();
function pendingState(): SpawnPendingState {
  if (!state) {
    try { state = normalizeSpawnPending(JSON.parse(sessionStorage.getItem(STORAGE_KEY) || 'null'), Date.now()); }
    catch { state = normalizeSpawnPending(null, Date.now()); }
  }
  return state;
}
function save(): void {
  try { sessionStorage.setItem(STORAGE_KEY, JSON.stringify(pendingState())); }
  catch {
    if (!storageWarned && pendingState().pending.length) {
      storageWarned = true; showToast(t('spawn_correlation_storage'));
    }
  }
}
export function captureSpawnTab(): string { return captureSessionListTab(); }
export function beginTrackedSpawn(expected = 1, tabId = captureSpawnTab()): string {
  const id = `spawn-${Array.from(crypto.getRandomValues(new Uint32Array(4)), value => value.toString(16).padStart(8, '0')).join('')}`;
  if (!beginSpawnPending(pendingState(), id, tabId, expected, Date.now(), connected)) throw new Error(t('spawn_correlation_capacity'));
  save(); return id;
}
export function noteSpawnDisconnected(): void { connected = false; live = []; candidates.clear(); }
export function noteSpawnSnapshot(instance: string, sessions: SessionSnapshot[]): void {
  const old = pendingState().hubInstance;
  setSpawnHubInstance(pendingState(), instance);
  if (old && instance && old !== instance) candidates.clear();
  connected = !!instance;
  save();
  // The caller has populated the entire map before this function resolves families.
  live = sessions;
  for (const session of sessions) {
    if (typeof session.client_request_id === 'string' && session.client_request_id) {
      addCandidate({ client_request_id: session.client_request_id, session_id: session.id, started_at: String(session.started_at || ''), hub_instance: instance });
    }
  }
  reconcile();
}
export function noteSpawnSessions(sessions: SessionSnapshot[]): void { live = sessions; reconcile(); }
export function noteSpawnCorrelation(event: SpawnCorrelation, sessions: SessionSnapshot[]): void {
  live = sessions; addCandidate(event); reconcile();
}
function addCandidate(event: SpawnCorrelation): void {
  // Only our bounded operations may reserve candidate state; arbitrary broadcast nonces cannot grow it.
  if (!pendingState().pending.some(item => item.requestId === event.client_request_id) ||
      !event.started_at || event.started_at.length > 256 || !Number.isSafeInteger(event.session_id) || event.session_id <= 0) return;
  if (candidates.size >= 128 * 18) return;
  candidates.set(`${event.session_id}:${event.started_at}`, event);
}
function reconcile(): void {
  const current = pendingState(); const before = current.pending.length;
  pruneSpawnPending(current, Date.now());
  let changed = before !== current.pending.length;
  for (const [key, event] of candidates) {
    const pending = matchSpawnCorrelation(current, event, live, Date.now());
    if (pending && assignCorrelatedSessionListSpawn(event.session_id, event.started_at, pending.tabId, live)) {
      recordSpawnMatch(pending, { id: event.session_id, startedAt: event.started_at });
      candidates.delete(key); changed = true;
    } else if (!current.pending.some(item => item.requestId === event.client_request_id) ||
        current.pending.some(item => item.requestId === event.client_request_id && item.matched.some(match => match.id === event.session_id && match.startedAt === event.started_at))) candidates.delete(key);
  }
  if (changed) save();
  for (const listener of listeners) listener();
}
/** No HTTP outcome removes an accepted request: partial starts and late registration remain attributable. */
export function waitForTrackedSpawn(requestId: string, timeoutMs = 15000): Promise<number[]> {
  return new Promise(resolve => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const finish = () => {
      if (timer !== null) clearTimeout(timer);
      listeners.delete(check);
      resolve(liveSpawnMatches(pendingState().pending.find(item => item.requestId === requestId), live).map(item => item.id));
    };
    const check = () => {
      const pending = pendingState().pending.find(item => item.requestId === requestId);
      if (!pending || liveSpawnMatches(pending, live).length >= pending.expected) finish();
    };
    listeners.add(check); timer = setTimeout(finish, Math.min(SPAWN_PENDING_TTL_MS, timeoutMs)); check();
  });
}
