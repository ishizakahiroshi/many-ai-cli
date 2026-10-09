// Bounded, per-browser-tab correlation. These identities are not authorization or idempotency tokens.
import type { SessionSnapshot } from '../types/proto.js';
export const SPAWN_PENDING_TTL_MS = 30 * 60 * 1000;
export const SPAWN_PENDING_LIMIT = 128;
export const SPAWN_MATCH_LIMIT = 18;
export interface SpawnIdentity { id: number; startedAt: string; }
export interface PendingSpawn {
  requestId: string; tabId: string; createdAt: number; hubInstance: string; expected: number;
  matched: SpawnIdentity[];
}
export interface SpawnPendingState { version: 1; hubInstance: string; pending: PendingSpawn[]; }
export interface SpawnCorrelation { client_request_id: string; session_id: number; started_at: string; hub_instance: string; }
export const validSpawnRequestId = (value: unknown): value is string => typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
const identityKey = (value: SpawnIdentity) => `${value.id}:${value.startedAt}`;
export function normalizeSpawnPending(raw: unknown, now: number): SpawnPendingState {
  const data = raw && typeof raw === 'object' ? raw as Partial<SpawnPendingState> : {};
  const state: SpawnPendingState = { version: 1, hubInstance: typeof data.hubInstance === 'string' && data.hubInstance.length <= 128 ? data.hubInstance : '', pending: [] };
  if (data.version !== 1 || !Array.isArray(data.pending)) return state;
  for (const item of data.pending.slice(-SPAWN_PENDING_LIMIT)) {
    if (!item || !validSpawnRequestId(item.requestId) || typeof item.tabId !== 'string' || !item.tabId || item.tabId.length > 100 ||
        !Number.isFinite(item.createdAt) || item.createdAt > now || now - item.createdAt >= SPAWN_PENDING_TTL_MS ||
        state.pending.some(old => old.requestId === item.requestId)) continue;
    const matched = Array.isArray(item.matched) ? item.matched.filter(value => value && Number.isSafeInteger(value.id) && value.id > 0 &&
      typeof value.startedAt === 'string' && value.startedAt.length > 0 && value.startedAt.length <= 256).slice(0, SPAWN_MATCH_LIMIT) : [];
    state.pending.push({ requestId: item.requestId, tabId: item.tabId, createdAt: item.createdAt,
      hubInstance: typeof item.hubInstance === 'string' && item.hubInstance.length <= 128 ? item.hubInstance : '', expected: Math.max(1, Math.min(SPAWN_MATCH_LIMIT, Number(item.expected) || 1)),
      matched: [...new Map(matched.map(value => [identityKey(value), { id: value.id, startedAt: value.startedAt }])).values()] });
  }
  return state;
}
export function pruneSpawnPending(state: SpawnPendingState, now: number): void {
  state.pending = state.pending.filter(item => item.createdAt <= now && now - item.createdAt < SPAWN_PENDING_TTL_MS);
}
export function beginSpawnPending(state: SpawnPendingState, requestId: string, tabId: string, expected: number, now: number, connected: boolean): PendingSpawn | null {
  pruneSpawnPending(state, now);
  if (!validSpawnRequestId(requestId) || !tabId || state.pending.length >= SPAWN_PENDING_LIMIT || state.pending.some(item => item.requestId === requestId)) return null;
  const item: PendingSpawn = { requestId, tabId, expected: Math.max(1, Math.min(SPAWN_MATCH_LIMIT, Math.trunc(expected) || 1)),
    createdAt: now, hubInstance: connected ? state.hubInstance : '', matched: [] };
  state.pending.push(item); return item;
}
/** Compare the persisted boundary even on the FIRST snapshot after reloading the page. */
export function setSpawnHubInstance(state: SpawnPendingState, instance: string): void {
  if (!instance) return;
  state.pending = state.pending.filter(item => !item.hubInstance || item.hubInstance === instance);
  for (const item of state.pending) item.hubInstance = instance;
  state.hubInstance = instance;
}
/** Return the matching operation only after both the request and current lifecycle are proven. */
export function matchSpawnCorrelation(state: SpawnPendingState, event: SpawnCorrelation, sessions: SessionSnapshot[], now: number): PendingSpawn | null {
  pruneSpawnPending(state, now);
  if (!event.hub_instance || event.hub_instance !== state.hubInstance || !validSpawnRequestId(event.client_request_id)) return null;
  const pending = state.pending.find(item => item.requestId === event.client_request_id && item.hubInstance === event.hub_instance);
  if (!pending || pending.matched.length >= SPAWN_MATCH_LIMIT) return null;
  const session = sessions.find(item => item.id === event.session_id && item.started_at === event.started_at);
  if (!session || !session.started_at || pending.matched.some(item => item.id === session.id && item.startedAt === session.started_at)) return null;
  return pending;
}
export function recordSpawnMatch(pending: PendingSpawn, session: SpawnIdentity): void {
  if (pending.matched.length < SPAWN_MATCH_LIMIT && !pending.matched.some(item => identityKey(item) === identityKey(session))) pending.matched.push(session);
}
export function liveSpawnMatches(pending: PendingSpawn | undefined, sessions: SessionSnapshot[]): SpawnIdentity[] {
  return (pending?.matched || []).filter(item => sessions.some(session => session.id === item.id && session.started_at === item.startedAt));
}
