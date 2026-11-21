import { describe, expect, test } from 'bun:test';
import { streamStepsFrom, takerNotices, takerStatusFrom } from '../src/app/local-taker-model';
import type { TakerStatus } from '../src/app/local-taker-model';

const run = (request_id: string, outcome: string, ended_at: number | null = null) => ({
  request_id, item_id: 'backlog/1', project: 'demo', title: `Tidy ${request_id}`, started_at: 1_700_000_000_000, ended_at, outcome, summary: '',
});
const status = (over: Partial<TakerStatus> = {}): TakerStatus => ({
  configured: true, paused: false, current: null, next_look_at: 1_700_000_300_000, runs_today: 0, max_runs_per_day: 6, interval_secs: 300, recent: [], ...over,
});

describe("this PC's taker", () => {
  test('reads the state the Hub sent', () => {
    const sent = status({ current: run('REQ-1', 'running'), recent: [run('REQ-1', 'running'), run('REQ-0', 'done', 1_700_000_060_000)] });
    expect(takerStatusFrom({ ok: true, taker: sent })).toEqual(sent);
  });
  test('a reply of another shape is nothing, and bad rows are dropped', () => {
    for (const payload of [null, 'x', {}, { taker: 'x' }]) expect(takerStatusFrom(payload)).toBeNull();
    const odd = takerStatusFrom({ taker: { configured: 'yes', runs_today: -1, recent: [null, { request_id: 'REQ-2' }, run('REQ-3', 'done')] } });
    expect(odd?.configured).toBe(false);
    expect(odd?.runs_today).toBe(0);
    expect(odd?.recent.map(r => r.request_id)).toEqual(['REQ-3']);
  });
  test('keeps only the steps it knows how to show', () => {
    const entries = [{ kind: 'text', text: 'Looking' }, { kind: 'tool', text: 'Bash: ls' }, { kind: 'script', text: 'x' }, { kind: 'result', text: '' }, null];
    expect(streamStepsFrom({ entries })).toEqual([{ kind: 'text', text: 'Looking' }, { kind: 'tool', text: 'Bash: ls' }]);
    expect(streamStepsFrom({ entries: 'x' })).toEqual([]);
  });
});

describe('notifications of a run', () => {
  test('the first look notifies nothing (old runs are not replayed)', () => {
    expect(takerNotices(null, status({ current: run('REQ-1', 'running'), recent: [run('REQ-0', 'done', 1)] }))).toEqual([]);
  });
  test('a run that starts, then ends', () => {
    const idle = status({ recent: [run('REQ-0', 'done', 1)] });
    const going = status({ current: run('REQ-1', 'running'), recent: [run('REQ-1', 'running'), run('REQ-0', 'done', 1)] });
    const ended = status({ recent: [run('REQ-1', 'done', 2), run('REQ-0', 'done', 1)] });
    expect(takerNotices(idle, going).map(n => [n.kind, n.run.request_id])).toEqual([['started', 'REQ-1']]);
    expect(takerNotices(going, going)).toEqual([]);
    expect(takerNotices(going, ended).map(n => [n.kind, n.run.request_id])).toEqual([['finished', 'REQ-1']]);
    expect(takerNotices(ended, ended)).toEqual([]);
  });
  test('a run that started and failed between two looks is told once, as ended', () => {
    const idle = status({ recent: [run('REQ-0', 'done', 1)] });
    const failed = status({ recent: [run('REQ-1', 'failed', 2), run('REQ-0', 'done', 1)] });
    expect(takerNotices(idle, failed).map(n => [n.kind, n.run.outcome])).toEqual([['finished', 'failed']]);
  });
  test('a run cut off by a Hub restart is not told as finished', () => {
    const going = status({ current: run('REQ-1', 'running'), recent: [run('REQ-1', 'running')] });
    expect(takerNotices(going, status({ recent: [run('REQ-1', 'interrupted')] }))).toEqual([]);
  });
});
