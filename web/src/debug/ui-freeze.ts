// One module in two realms: the page registers the sink; its dedicated worker
// keeps the bounded flight recorder and performs HTTP even if the page stalls.
// Included only with MAI_DEBUG. No speech, terminal text, URL or error message.
import { registerProbeSink } from './probe.js';

const phases = new Set(['voice.result', 'input.layout', 'terminal.fit', 'terminal.links']);
export interface FreezeEvent {
  id: number;
  phase: string;
  edge: 'begin' | 'end';
  at: number;
  size: number;
}
export interface FreezePacket {
  schema: number;
  tab: string;
  reason: 'checkpoint' | 'suspected-stall' | 'recovered' | 'worker-gap';
  at: number;
  gapMs: number;
  visible: boolean;
  dropped: number;
  events: FreezeEvent[];
  open: FreezeEvent[];
}

export class FreezeRecorder {
  private lastBeat: number;
  private lastTick: number;
  private lastWall: number;
  private lastSave = -Infinity;
  private visible: boolean;
  private stalled = false;
  private events: FreezeEvent[] = [];
  private open = new Map<number, FreezeEvent>();
  private dropped = 0;

  constructor(private tab: string, now: number, wall: number, visible: boolean) {
    this.lastBeat = this.lastTick = now;
    this.lastWall = wall;
    this.visible = visible;
  }

  heartbeat(now: number, visible: boolean): void {
    this.lastBeat = now;
    this.visible = visible;
  }

  record(event: FreezeEvent): void {
    // Explicit reconstruction keeps accidental caller fields out of the log.
    if (!phases.has(event.phase) || !['begin', 'end'].includes(event.edge)) return;
    const clean = { id: event.id, phase: event.phase, edge: event.edge, at: event.at, size: event.size };
    this.events.push(clean);
    if (this.events.length > 64) { this.events.shift(); this.dropped++; }
    if (event.edge === 'begin') {
      if (this.open.size >= 16) { this.open.delete(this.open.keys().next().value); this.dropped++; }
      this.open.set(event.id, clean);
    } else {
      this.open.delete(event.id);
    }
  }

  tick(now: number, wall: number): FreezePacket | null {
    const delta = now - this.lastTick;
    // Background throttling, PC sleep and clock jumps cannot prove a UI stall.
    const workerGap = delta > 3000 || Math.abs((wall - this.lastWall) - delta) > 3000;
    this.lastTick = now;
    this.lastWall = wall;
    const gapMs = Math.max(0, now - this.lastBeat);
    let reason: FreezePacket['reason'] = 'checkpoint';
    if (workerGap) {
      this.lastBeat = now;
      this.stalled = false;
      reason = 'worker-gap';
    } else if (this.visible && gapMs >= 5000 && !this.stalled) {
      this.stalled = true;
      reason = 'suspected-stall';
    } else if (this.stalled && gapMs < 2000) {
      this.stalled = false;
      reason = 'recovered';
    }
    if (reason === 'checkpoint' && now - this.lastSave < (this.stalled ? 30000 : 5000)) return null;
    this.lastSave = now;
    return {
      schema: 1, tab: this.tab, reason, at: wall, gapMs: Math.round(gapMs),
      visible: this.visible, dropped: this.dropped,
      events: this.events.slice(), open: [...this.open.values()],
    };
  }
}

// Single in-flight request; bounded retry state. The first stall snapshot is
// retained across failures rather than overwritten by later healthy checkpoints.
export class FreezeTransport {
  private pending: FreezePacket | null = null;
  private busy = false;
  private nextTry = 0;
  constructor(
    private send: (packet: FreezePacket) => Promise<void>,
    private status: (ok: boolean) => void,
  ) {}

  offer(packet: FreezePacket | null, now: number): void {
    if (packet && this.pending?.reason !== 'suspected-stall') this.pending = packet;
    if (this.busy || !this.pending || now < this.nextTry) return;
    const current = this.pending;
    this.pending = null;
    this.busy = true;
    let failed = false;
    void this.send(current).then(() => this.status(true), () => {
      failed = true;
      if (current.reason === 'suspected-stall' || !this.pending) this.pending = current;
      this.status(false);
    }).finally(() => {
      this.busy = false;
      // Desynchronize restored tabs after the Hub's aggregate write limit.
      this.nextTry = now + 5000 + (failed ? Math.random() * 2000 : 0);
    });
  }
}

// The worker imports this same file. Avoid DOM/module dependencies in this realm.
if (typeof self !== 'undefined' && typeof document === 'undefined') {
  let recorder: FreezeRecorder | null = null;
  const transport = new FreezeTransport(async (packet) => {
    const abort = new AbortController();
    const timeout = setTimeout(() => abort.abort(), 4000);
    try {
      const response = await fetch('/api/debug/ui-freeze', {
        method: 'POST', credentials: 'same-origin', cache: 'no-store',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(packet), signal: abort.signal,
      });
      if (!response.ok) throw new Error('ui-freeze-save-failed');
    } finally { clearTimeout(timeout); }
  }, (ok) => self.postMessage({ type: 'saved', ok }));
  self.onmessage = ({ data }) => {
    const now = performance.now();
    if (data.type === 'init' && !recorder) {
      recorder = new FreezeRecorder(data.tab, now, Date.now(), data.visible);
      self.postMessage({ type: 'ready' });
    } else if (data.type === 'heartbeat') recorder?.heartbeat(now, data.visible);
    else if (data.type === 'event') recorder?.record(data.event);
  };
  setInterval(() => {
    if (!recorder) return;
    const now = performance.now();
    transport.offer(recorder.tick(now, Date.now()), now);
  }, 1000);
} else if (typeof document !== 'undefined' && typeof Worker !== 'undefined') {
  try {
    const worker = new Worker(new URL('./ui-freeze.js', import.meta.url), { type: 'module' });
    let ready = false;
    let reportedFailure = false;
    const warn = () => {
      if (!reportedFailure) console.warn('ui-freeze recorder unavailable; capture is not confirmed');
      reportedFailure = true;
    };
    const heartbeat = () => {
      if (ready) worker.postMessage({ type: 'heartbeat', visible: document.visibilityState === 'visible' });
    };
    worker.onmessage = ({ data }) => {
      if (data.type === 'ready') { ready = true; heartbeat(); }
      if (data.type === 'saved') {
        if (!data.ok) warn();
        else reportedFailure = false;
      }
    };
    worker.onerror = warn;
    worker.postMessage({ type: 'init', tab: crypto.randomUUID(), visible: document.visibilityState === 'visible' });
    setInterval(heartbeat, 1000);
    document.addEventListener('visibilitychange', heartbeat);
    // Prevent a heartbeat timer running first after a background interval from
    // classifying that interval as an observed freeze on the next worker tick.
    window.addEventListener('pageshow', heartbeat);
    registerProbeSink('ui.freeze', (_channel, fields) => {
      if (!ready || !phases.has(String(fields.phase))) return;
      worker.postMessage({ type: 'event', event: {
        id: fields.id, phase: fields.phase, edge: fields.edge, at: Date.now(),
        size: typeof fields.size === 'number' ? Math.min(1000000000, Math.max(0, Math.floor(fields.size))) : 0,
      } });
    });
  } catch { console.warn('ui-freeze recorder could not start'); }
}
