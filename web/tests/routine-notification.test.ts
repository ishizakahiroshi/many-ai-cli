import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const transpiler = new Bun.Transpiler({ loader: 'ts' });
function worker(clients: any[]) {
  const listeners = new Map<string, Function>();
  const opened: string[] = [];
  const self = { location: { origin: 'https://hub.example' }, addEventListener: (name: string, fn: Function) => listeners.set(name, fn), clients: {
    matchAll: async () => clients, openWindow: async (url: string) => { opened.push(url); },
  } };
  const code = transpiler.transformSync(readFileSync(new URL('../src/sw.ts', import.meta.url), 'utf8'));
  runInNewContext(code, { self, URL, caches: { open: async () => ({ match: async () => undefined }) }, Date });
  return { opened, async click(data: object) {
    let task: Promise<void> | undefined;
    listeners.get('notificationclick')!({ notification: { data, close() {} }, waitUntil(value: Promise<void>) { task = value; } });
    await task;
  } };
}

describe('routine notification routing', () => {
  test('an existing Hub receives the immutable run, not the reusable live session id', async () => {
    const messages: any[] = []; let focused = false;
    const sw = worker([{ postMessage: (message: any) => messages.push(message), focus: async () => { focused = true; } }]);
    await sw.click({ session_id: 4, url: '/?routine_run=run-original' });
    expect(messages).toEqual([{ type: 'many-ai-cli-open-routine', run_id: 'run-original' }]);
    expect(focused).toBe(true); expect(sw.opened).toEqual([]);
  });
  test('a new Hub opens the exact run without also activating an old session', async () => {
    const sw = worker([]);
    await sw.click({ session_id: 4, url: '/?routine_run=run-original&session_id=4&token=synthetic' });
    const url = new URL(sw.opened[0]);
    expect(url.searchParams.get('routine_run')).toBe('run-original');
    expect(url.searchParams.has('session_id')).toBe(false);
    expect(url.searchParams.get('token')).toBe('synthetic');
  });
  test('ordinary session notifications preserve their existing path', async () => {
    const messages: any[] = [];
    const sw = worker([{ postMessage: (message: any) => messages.push(message), focus: async () => {} }]);
    await sw.click({ session_id: 8, url: '/?session_id=8' });
    expect(messages).toEqual([{ type: 'many-ai-cli-open-session', session_id: 8 }]);
  });
  test('external URL cannot become a routine target or receive a token', async () => {
    const sw = worker([]);
    await sw.click({ session_id: 8, url: 'https://outside.example/?routine_run=run-other&token=synthetic' });
    const url = new URL(sw.opened[0]);
    expect(url.origin).toBe('https://hub.example');expect(url.searchParams.has('routine_run')).toBe(false);expect(url.searchParams.has('token')).toBe(false);
  });
  test('PWA bridges a validated run message without opening the session', () => {
    const listeners = new Map<string, Function>(); const events: any[] = []; const activated: number[] = [];
    const source = readFileSync(new URL('../src/app/pwa.ts', import.meta.url), 'utf8').replace(/^import .*;\r?$/gm, '').replace(/^export /gm, '');
    runInNewContext(transpiler.transformSync(source), {
      token: '', apiFetch: () => {}, activateSession: (id: number) => activated.push(id),
      navigator: { serviceWorker: { addEventListener: (name: string, fn: Function) => listeners.set(name, fn) } },
      window: { addEventListener() {}, dispatchEvent: (event: any) => events.push(event) },
      CustomEvent: class { constructor(public type: string, public options: any) {} },
    });
    listeners.get('message')!({ data: { type: 'many-ai-cli-open-routine', run_id: 'run-original' } });
    listeners.get('message')!({ data: { type: 'many-ai-cli-open-routine', run_id: '../bad' } });
    expect(events).toHaveLength(1);expect(events[0].type).toBe('many-ai-cli:open-routine');expect(events[0].options.detail.runID).toBe('run-original');expect(activated).toEqual([]);
  });
});
