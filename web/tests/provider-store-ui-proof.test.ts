import { afterEach, describe, expect, test } from 'bun:test';
import {
  recoverProvider,
  removeProvider,
  resetProviderOverride,
  restoreProviderBackup,
  restoreProviderRevision,
  saveProviderDefinition,
  loadProviderDetail,
} from '../src/app/provider-store';
import { providerErrorMessage } from '../src/app/provider-manager-view.ts';

// 同梱の AI CLI の定義を変える要求は、画面の印（GET / で出し直される cookie）が要る。印は 24 時間で
// 切れるので、開いたままの画面で 403 ui_required が返ったら、GET / を 1 回読んで同じ要求を 1 回だけ
// 送り直す。2 回目も断られたら「ページを開き直して」を出す。

type Call = { url: string; method: string };

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
}

const refused = () => json(403, { error: 'ui_required', detail: 'refused' });

function stubFetch(answers: Array<(call: Call) => Response>): Call[] {
  const calls: Call[] = [];
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const call = { url: String(input), method: (init?.method || 'GET').toUpperCase() };
    calls.push(call);
    const answer = answers.shift();
    if (!answer) throw new Error(`unexpected request ${call.method} ${call.url}`);
    return answer(call);
  }) as unknown as typeof fetch;
  return calls;
}

describe('changing a built-in after the dashboard proof expired', () => {
  const realFetch = globalThis.fetch;
  const realSecure = Object.getOwnPropertyDescriptor(globalThis, 'isSecureContext');
  afterEach(() => {
    globalThis.fetch = realFetch;
    if (realSecure) Object.defineProperty(globalThis, 'isSecureContext', realSecure);
    else delete (globalThis as { isSecureContext?: boolean }).isSecureContext;
  });

  test('a save renews the proof from the page once and is sent again', async () => {
    const calls = stubFetch([refused, () => new Response('<html></html>', { status: 200 }), () => json(200, { revision: 'r2', diagnostics: [] })]);
    const result = await saveProviderDefinition('claude', 'r1', { id: 'claude' });
    expect(result).toEqual({ ok: true, revision: 'r2', diagnostics: [] });
    expect(calls).toEqual([
      { url: '/api/providers/claude', method: 'PATCH' },
      { url: '/', method: 'GET' },
      { url: '/api/providers/claude', method: 'PATCH' },
    ]);
  });

  test('every request that changes a built-in takes the same retry', async () => {
    const changes: Array<[() => Promise<{ ok: boolean }>, string, string]> = [
      [() => removeProvider('claude', 'r1'), '/api/providers/claude?expected_revision=r1', 'DELETE'],
      [() => resetProviderOverride('claude', 'r1'), '/api/providers/claude/reset', 'POST'],
      [() => restoreProviderRevision('claude', 'r0', 'r1'), '/api/providers/claude/restore', 'POST'],
      [() => restoreProviderBackup('claude', 'b1', 'r1'), '/api/providers/claude/backups/b1/restore', 'POST'],
      [() => recoverProvider('claude', 'r0'), '/api/providers/claude/recovery', 'POST'],
    ];
    for (const [change, url, method] of changes) {
      const calls = stubFetch([refused, () => new Response('', { status: 200 }), () => json(200, { revision: 'r2' })]);
      expect((await change()).ok).toBe(true);
      expect(calls).toEqual([{ url, method }, { url: '/', method: 'GET' }, { url, method }]);
    }
  });

  test('a second refusal is not retried again and asks to reload the page', async () => {
    const calls = stubFetch([refused, () => new Response('', { status: 200 }), refused]);
    const result = await saveProviderDefinition('claude', 'r1', { id: 'claude' });
    expect(calls.length).toBe(3);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result).toMatchObject({ kind: 'http', status: 403, code: 'ui_required' });
    expect(providerErrorMessage(result).key).toBe('settings_ai_providers_reload_page');
  });

  test('a page opened over plain http from another computer is told to use a forward or HTTPS', () => {
    Object.defineProperty(globalThis, 'isSecureContext', { value: false, configurable: true });
    const message = providerErrorMessage({ kind: 'http', status: 403, code: 'ui_required' });
    expect(message.key).toBe('settings_ai_providers_needs_secure_page');
  });

  test('when the page cannot be read, the first refusal is returned without a second send', async () => {
    const calls = stubFetch([refused, () => new Response('', { status: 503 })]);
    const result = await saveProviderDefinition('claude', 'r1', { id: 'claude' });
    expect(calls.map((call) => call.method)).toEqual(['PATCH', 'GET']);
    expect(result).toMatchObject({ ok: false, kind: 'http', code: 'ui_required' });
  });

  test('cancelling while the page is being read returns aborted, not the first refusal', async () => {
    const controller = new AbortController();
    const calls = stubFetch([
      refused,
      () => {
        controller.abort();
        throw new DOMException('The operation was aborted.', 'AbortError');
      },
    ]);
    const result = await saveProviderDefinition('claude', 'r1', { id: 'claude' }, { signal: controller.signal });
    expect(calls.map((call) => call.method)).toEqual(['PATCH', 'GET']);
    expect(result).toEqual({ ok: false, kind: 'aborted' });
  });

  test('cancelling after the page was read does not send again', async () => {
    const controller = new AbortController();
    const calls = stubFetch([
      refused,
      () => {
        controller.abort();
        return new Response('', { status: 200 });
      },
    ]);
    const result = await saveProviderDefinition('claude', 'r1', { id: 'claude' }, { signal: controller.signal });
    expect(calls.map((call) => call.method)).toEqual(['PATCH', 'GET']);
    expect(result).toEqual({ ok: false, kind: 'aborted' });
  });

  test('a page that is not a secure context does not renew or send again', async () => {
    Object.defineProperty(globalThis, 'isSecureContext', { value: false, configurable: true });
    const calls = stubFetch([refused]);
    const result = await saveProviderDefinition('claude', 'r1', { id: 'claude' });
    expect(calls).toEqual([{ url: '/api/providers/claude', method: 'PATCH' }]);
    expect(result).toMatchObject({ ok: false, kind: 'http', status: 403, code: 'ui_required' });
    if (result.ok) return;
    expect(providerErrorMessage(result).key).toBe('settings_ai_providers_needs_secure_page');
  });

  test('other refusals and reads are not retried', async () => {
    let calls = stubFetch([() => json(403, { error: 'forbidden' })]);
    expect((await saveProviderDefinition('claude', 'r1', {})).ok).toBe(false);
    expect(calls.length).toBe(1);
    calls = stubFetch([refused]);
    expect((await loadProviderDetail('claude')).ok).toBe(false);
    expect(calls.length).toBe(1);
  });
});
