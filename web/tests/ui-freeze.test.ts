import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Worker } from 'node:worker_threads';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { FreezeRecorder, FreezeTransport } from '../src/debug/ui-freeze.ts';
import { probeSpan, registerProbeSink } from '../src/debug/probe.ts';

const tab = '12345678-1234-1234-1234-123456789abc';
const event = (id: number, edge: 'begin' | 'end' = 'begin') => ({
  id, phase: 'voice.result', edge, at: 1000, size: 12,
});

test('foreground silence captures unfinished nested work and recovery', () => {
  const r = new FreezeRecorder(tab, 0, 1000, true);
  r.record(event(1));
  r.record({ ...event(2), phase: 'input.layout' });
  let packet;
  for (let now = 1000; now <= 5000; now += 1000) packet = r.tick(now, now + 1000) || packet;
  assert.equal(packet.reason, 'suspected-stall');
  assert.deepEqual(packet.open.map(e => e.phase), ['voice.result', 'input.layout']);
  r.record(event(2, 'end'));
  r.record(event(1, 'end'));
  r.heartbeat(6000, true);
  assert.equal(r.tick(6000, 7000)?.reason, 'recovered');
});

test('background tabs and suspended workers do not prove a foreground stall', () => {
  const hidden = new FreezeRecorder(tab, 0, 1000, false);
  for (let now = 1000; now <= 10000; now += 1000) {
    assert.notEqual(hidden.tick(now, now + 1000)?.reason, 'suspected-stall');
  }
  const asleep = new FreezeRecorder(tab, 0, 1000, true);
  assert.equal(asleep.tick(60000, 61000)?.reason, 'worker-gap');
  assert.notEqual(asleep.tick(61000, 62000)?.reason, 'suspected-stall');
  const wallJump = new FreezeRecorder(tab, 0, 1000, true);
  assert.equal(wallJump.tick(1000, 61000)?.reason, 'worker-gap');
});

test('ring and unfinished work are bounded; arbitrary caller text is discarded', () => {
  const r = new FreezeRecorder(tab, 0, 1000, true);
  for (let id = 1; id <= 1000; id++) r.record({ ...event(id), transcript: 'never store this' } as any);
  const packet = r.tick(1000, 2000)!;
  assert.equal(packet.events.length, 64);
  assert.equal(packet.open.length, 16);
  assert.ok(packet.dropped > 0);
  assert.ok(!JSON.stringify(packet).includes('never store this'));
});

test('failed stall save survives later checkpoints and only one request is in flight', async () => {
  const r = new FreezeRecorder(tab, 0, 1000, true);
  const checkpoint = r.tick(1000, 2000)!;
  const stall = { ...checkpoint, reason: 'suspected-stall' as const };
  let rejectFirst: (e: Error) => void;
  const sent: string[] = [];
  const transport = new FreezeTransport(packet => {
    sent.push(packet.reason);
    if (sent.length === 1) return new Promise((_resolve, reject) => { rejectFirst = reject; });
    return Promise.resolve();
  }, () => {});
  transport.offer(stall, 0);
  transport.offer(checkpoint, 1000);
  assert.equal(sent.length, 1);
  rejectFirst!(new Error('offline'));
  await new Promise(resolve => setImmediate(resolve));
  transport.offer(checkpoint, 2000);
  assert.equal(sent.length, 1);
  transport.offer(checkpoint, 8000);
  assert.deepEqual(sent, ['suspected-stall', 'suspected-stall']);
});

test('span hook is inert when disabled and cannot break the caller on sink failure', () => {
  (globalThis as any).__MAI_DEBUG__ = false;
  assert.equal(probeSpan('span-test', () => { throw new Error('must not run'); }), undefined);
  (globalThis as any).__MAI_DEBUG__ = true;
  const events: any[] = [];
  registerProbeSink('span-test', (_channel, fields) => { events.push(fields); });
  const finish = probeSpan('span-test', () => ({ phase: 'test' }));
  finish?.();
  assert.deepEqual(events.map(e => e.edge), ['begin', 'end']);
  assert.equal(events[0].id, events[1].id);
  registerProbeSink('span-test', () => { throw new Error('recorder failure'); });
  assert.doesNotThrow(() => probeSpan('span-test', () => ({})));
  delete (globalThis as any).__MAI_DEBUG__;
});

test('actual recorder worker saves during a blocked parent event loop', { timeout: 15000 }, async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ui-freeze-test-'));
  const file = join(dir, 'captures.jsonl');
  // Run the real browser worker entry with a worker_threads message adapter.
  // fetch writes to a temporary collector: HTTP auth/persistence is tested in Go.
  const worker = new Worker(`
    const {parentPort, workerData} = require('node:worker_threads');
    const {appendFileSync} = require('node:fs');
    globalThis.self = {postMessage: message => parentPort.postMessage(message)};
    globalThis.fetch = async (_url, options) => {
      appendFileSync(workerData.file, options.body + '\\n');
      return {ok: true};
    };
    import(workerData.module).then(() => {
      parentPort.on('message', data => self.onmessage({data}));
      parentPort.postMessage({type: 'adapter-ready'});
    }).catch(error => { throw error; });
  `, { eval: true, workerData: { file, module: new URL('../src/debug/ui-freeze.ts', import.meta.url).href } });
  try {
    await new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('worker startup timeout')), 5000);
      worker.on('error', reject);
      worker.on('message', data => {
        if (data.type === 'adapter-ready') worker.postMessage({ type: 'init', tab, visible: true });
        if (data.type === 'ready') {
          worker.postMessage({ type: 'event', event: event(1) });
        }
        if (data.type === 'saved') { clearTimeout(timeout); resolve(); }
      });
    });
    const blockedAt = Date.now();
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 6500);
    const resumedAt = Date.now();
    const packets = readFileSync(file, 'utf8').trim().split('\n').map(line => JSON.parse(line));
    const stall = packets.find(packet => packet.reason === 'suspected-stall');
    assert.ok(stall, 'worker must save without parent event-loop progress');
    assert.ok(stall.at >= blockedAt && stall.at < resumedAt);
    assert.equal(stall.open[0].phase, 'voice.result');
    assert.ok(!stall.events.some(e => e.edge === 'end'));
  } finally {
    await worker.terminate();
    rmSync(dir, { recursive: true, force: true });
  }
});
