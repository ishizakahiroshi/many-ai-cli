import { describe, expect, test } from 'bun:test';

// 監査 N16: 保存の途中で Hub が止まった file の控えを、画面に並べる部分。
// 帯そのもの（DOM への差し込み・ボタンを押した後の通信）は実ブラウザで確かめる範囲で、
// ここでは Hub の答えの読み取りと、行の組み立てだけを動かす。

class FakeStorage {
  items = new Map<string, string>();
  getItem(key: string) { return this.items.has(key) ? this.items.get(key)! : null; }
  setItem(key: string, value: string) { this.items.set(key, value); }
  removeItem(key: string) { this.items.delete(key); }
}

async function loadNotice() {
  const g = globalThis as any;
  const names = ['location', 'navigator', 'history', 'localStorage', 'sessionStorage', 'fetch', 'setInterval'];
  const saved = names.map((name) => [name, Object.getOwnPropertyDescriptor(g, name)] as const);
  const put = (name: string, value: unknown) => {
    Object.defineProperty(g, name, { value, configurable: true, writable: true });
  };
  put('location', { hostname: 'hub.example.com', search: '', pathname: '/', hash: '' });
  put('navigator', { userAgent: 'synthetic', maxTouchPoints: 0 });
  put('history', { state: null, replaceState: () => {} });
  put('localStorage', new FakeStorage());
  put('sessionStorage', new FakeStorage());
  put('fetch', async () => new Response('{}', { status: 200 }));
  put('setInterval', () => 0);
  try {
    const notice = await import('../src/app/save-leftover-notice.ts');
    return {
      readSaveLeftovers: notice.readSaveLeftovers,
      saveLeftoverRowsHtml: (leftovers: Parameters<typeof notice.saveLeftoverRowsHtml>[0]) => {
        // 文言は辞書を読み込まずに、部品が持つ英語の既定で確かめる（t() は window.t が無ければ key を返す）。
        const had = Object.getOwnPropertyDescriptor(g, 'window');
        put('window', {});
        try {
          return notice.saveLeftoverRowsHtml(leftovers);
        } finally {
          if (had) Object.defineProperty(g, 'window', had);
          else delete g.window;
        }
      },
    };
  } finally {
    for (const [name, descriptor] of saved) {
      if (descriptor) Object.defineProperty(g, name, descriptor);
      else delete g[name];
    }
  }
}

describe('保存の途中で止まった file の控えの一覧', () => {
  test('Hub の答えから、id のある行だけを読み取る', async () => {
    const { readSaveLeftovers } = await loadNotice();
    expect(readSaveLeftovers({
      leftovers: [
        { id: 'abc123', path: '/synthetic/notes.md', saved_at: '2026-11-13T00:00:00Z', size: 12, backup: '/synthetic/tmp/abc123.bak' },
        { id: '', path: '/synthetic/no-id.md' },
        { path: '/synthetic/no-id.md' },
        null,
        { id: 'def456' },
      ],
    })).toEqual([
      { id: 'abc123', path: '/synthetic/notes.md', saved_at: '2026-11-13T00:00:00Z', size: 12, backup: '/synthetic/tmp/abc123.bak' },
      { id: 'def456', path: '', saved_at: '', size: 0, backup: '' },
    ]);
    for (const value of [null, undefined, {}, { leftovers: 'x' }, []]) {
      expect(readSaveLeftovers(value)).toEqual([]);
    }
  });

  test('どの file のものか分かる控えには「戻す」と「消す」を出し、path は文字として出す', async () => {
    const { saveLeftoverRowsHtml } = await loadNotice();
    const html = saveLeftoverRowsHtml([
      { id: 'abc123', path: '/synthetic/<img src=x onerror=1>.md', saved_at: '2026-11-13T00:00:00Z', size: 12, backup: '/synthetic/tmp/abc123.bak' },
    ]);
    expect(html).toContain('data-leftover-id="abc123"');
    expect(html).toContain('save-leftover-restore');
    expect(html).toContain('save-leftover-discard');
    expect(html).toContain('/synthetic/&lt;img src=x onerror=1&gt;.md');
    expect(html).not.toContain('<img');
    expect(html).toContain('/synthetic/tmp/abc123.bak');
  });

  test('どの file のものか記録が無い控えには「消す」だけを出す', async () => {
    const { saveLeftoverRowsHtml } = await loadNotice();
    const html = saveLeftoverRowsHtml([
      { id: 'def456', path: '', saved_at: '', size: 8, backup: '/synthetic/tmp/def456.bak' },
    ]);
    expect(html).not.toContain('save-leftover-restore');
    expect(html).toContain('save-leftover-discard');
    expect(html).toContain('/synthetic/tmp/def456.bak');
  });
});
