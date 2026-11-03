// UI-only request state. A startup placeholder is never a Hub session or an input target.
export interface StartupIdentity { id: number; startedAt: string; }
export interface SessionStartup {
  requestId: string; provider: string; name: string; tabId: string; createdAt: number;
  phase: 'starting' | 'confirming' | 'failed' | 'output';
  session: StartupIdentity | null; detail: string; slow: boolean;
}
const identityKey = (session: StartupIdentity) => `${session.id}:${session.startedAt}`;
export function terminalHasVisibleContent(term: any): boolean {
  const buffer = term?.buffer?.active;
  if (!buffer) return false;
  for (let row = buffer.viewportY; row < buffer.viewportY + term.rows; row++) {
    if (buffer.getLine(row)?.translateToString(true).trim()) return true;
  }
  return false;
}
export class SessionStartupStore {
  readonly requests = new Map<string, SessionStartup>();
  private painted = new Set<string>();
  private listeners = new Set<() => void>();
  private instance = '';
  subscribe(listener: () => void): () => void { this.listeners.add(listener); return () => this.listeners.delete(listener); }
  private changed(): void { for (const listener of this.listeners) listener(); }
  begin(requestId: string, provider: string, name: string, tabId: string): SessionStartup {
    for (const [key, value] of this.requests) if (value.phase === 'output') this.requests.delete(key);
    // Match the existing bounded request tracker; stale UI cannot grow without limit.
    while (this.requests.size >= 128) this.requests.delete(this.requests.keys().next().value!);
    const value: SessionStartup = { requestId, provider, name, tabId, createdAt: Date.now(), phase: 'starting', session: null, detail: '', slow: false };
    this.requests.set(requestId, value); this.changed(); return value;
  }
  bind(requestId: string, session: StartupIdentity): void {
    const request = this.requests.get(requestId);
    if (!request || request.session || !Number.isSafeInteger(session.id) || !session.startedAt) return;
    request.session = session;
    request.phase = this.painted.has(identityKey(session)) ? 'output' : 'starting';
    this.changed();
  }
  outputPainted(session: StartupIdentity): void {
    this.painted.add(identityKey(session));
    if (this.painted.size > 256) this.painted.delete(this.painted.values().next().value!);
    let changed = false;
    for (const request of this.requests.values()) if (request.session && identityKey(request.session) === identityKey(session)) {
      request.phase = 'output'; changed = true;
    }
    if (changed) this.changed();
  }
  outcome(requestId: string, definiteFailure: boolean, detail = ''): void {
    const request = this.requests.get(requestId);
    // A proven registration wins over an HTTP failure/lost response.
    if (!request || request.session) return;
    request.phase = definiteFailure ? 'failed' : 'confirming'; request.detail = detail; this.changed();
  }
  markSlow(requestId: string): void { const value = this.requests.get(requestId); if (value && value.phase !== 'output') { value.slow = true; this.changed(); } }
  disconnected(): void {
    for (const value of this.requests.values()) if (value.phase !== 'output' && value.phase !== 'failed') value.phase = 'confirming';
    this.changed();
  }
  snapshot(instance: string, sessions: { id: number; started_at?: string; state?: string }[]): void {
    if (this.instance && instance && this.instance !== instance) { this.requests.clear(); this.painted.clear(); }
    this.instance = instance || this.instance;
    for (const request of this.requests.values()) if (request.session && request.phase !== 'output') {
      const session = sessions.find(value => value.id === request.session!.id && String(value.started_at || '') === request.session!.startedAt);
      if (!session || ['completed', 'error', 'disconnected'].includes(session.state || '')) {
        request.phase = 'failed'; request.detail = ''; request.slow = false;
      }
    }
    this.changed();
  }
  dismiss(requestId: string): void { if (this.requests.delete(requestId)) this.changed(); }
  forSession(session: StartupIdentity): SessionStartup | undefined {
    return [...this.requests.values()].find(value => value.session && identityKey(value.session) === identityKey(session));
  }
}
export const sessionStartups = new SessionStartupStore();
