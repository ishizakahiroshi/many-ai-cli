import { test } from 'node:test';
import assert from 'node:assert/strict';
import { probeSpan, registerProbeSink } from '../src/debug/probe.ts';

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
