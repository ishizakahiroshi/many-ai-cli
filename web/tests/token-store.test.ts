import { afterEach, describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import {
  ASSET_TICKET_STORAGE_KEY, TOKEN_STORAGE_KEY, isHandheld, isLoopbackHost, keepsTokenInTabOnly,
  loadToken, ticketStore,
} from '../src/app/token-store.ts';

// 監査 N24: PC の手元の画面（127.0.0.1 / localhost を PC のブラウザで開いたもの）では、Hub の
// token をタブの外に残さない。置き場の決め方（token-store.ts）と、同じ判定を自分で持つ
// 2 か所（サインインの画面 reauth.html、Service Worker sw.ts）、それを使う画面の起動
// （util.ts）を、偽物の置き場で実際に動かして確かめる。token の値はどれも合成。

class FakeStorage {
  readonly items = new Map<string, string>();
  getItem(key: string): string | null { return this.items.get(key) ?? null; }
  setItem(key: string, value: string): void { this.items.set(key, String(value)); }
  removeItem(key: string): void { this.items.delete(key); }
}

class BrokenStorage {
  getItem(): string | null { throw new Error('denied'); }
  setItem(): void { throw new Error('denied'); }
  removeItem(): void { throw new Error('denied'); }
}

function stores() {
  return { lasting: new FakeStorage(), tab: new FakeStorage() };
}

// ブラウザの名乗り（合成。形だけ実物に合わせてある）。
const PC = { userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/130.0 Safari/537.36', platform: 'Win32', maxTouchPoints: 0 };
const TOUCH_PC = { ...PC, maxTouchPoints: 10 };
const MAC = { userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Safari/605.1.15', platform: 'MacIntel', maxTouchPoints: 0 };
const IPHONE = { userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) Safari/604.1', platform: 'iPhone', maxTouchPoints: 5 };
const IPAD = { ...MAC, maxTouchPoints: 5 };
const ANDROID = { userAgent: 'Mozilla/5.0 (Linux; Android 15) Chrome/130.0 Mobile Safari/537.36', platform: 'Linux armv81', maxTouchPoints: 5 };

describe('どの画面を PC の手元の画面と見なすか', () => {
  test('127.0.0.0/8・localhost・::1 は手元の origin', () => {
    for (const host of ['127.0.0.1', '127.1.2.3', 'localhost', 'LOCALHOST.', 'hub.localhost', '[::1]', '::1']) {
      expect(isLoopbackHost(host)).toBe(true);
    }
  });

  test('ほかのアドレス・名前の付いたホスト・似た形の名前は手元の origin ではない', () => {
    for (const host of [
      '203.0.113.5', '198.51.100.7', 'example.com', '127.0.0.1.example.com', '1270.0.0.1',
      '127.0.0.256', '127.0.1', 'localhost.example.com', '',
    ]) {
      expect(isLoopbackHost(host)).toBe(false);
    }
  });

  test('スマホ・タブレットを見分ける（Mac と名乗る iPad を含む。タッチ付きの PC は PC）', () => {
    for (const browser of [IPHONE, IPAD, ANDROID, { userAgentData: { mobile: true } }]) {
      expect(isHandheld(browser)).toBe(true);
    }
    for (const browser of [PC, TOUCH_PC, MAC, {}, null, undefined]) {
      expect(isHandheld(browser)).toBe(false);
    }
  });

  test('タブの中だけに置くのは、手元の origin を PC のブラウザで開いたときだけ', () => {
    expect(keepsTokenInTabOnly('127.0.0.1', PC)).toBe(true);
    expect(keepsTokenInTabOnly('localhost', MAC)).toBe(true);
    // README の手順（SSH の転送でスマホ自身の 127.0.0.1 を開く）は今までどおり。
    expect(keepsTokenInTabOnly('127.0.0.1', IPHONE)).toBe(false);
    expect(keepsTokenInTabOnly('127.0.0.1', IPAD)).toBe(false);
    expect(keepsTokenInTabOnly('127.0.0.1', ANDROID)).toBe(false);
    // 外からの画面は、PC でも今までどおり。
    expect(keepsTokenInTabOnly('hub.example.com', PC)).toBe(false);
    expect(keepsTokenInTabOnly('203.0.113.5', PC)).toBe(false);
  });
});

describe('PC の手元の画面の token', () => {
  test('URL の token はタブの中にだけ置き、ブラウザを閉じても残る置き場には置かない', () => {
    const s = stores();
    expect(loadToken('synthetic-token', true, s)).toBe('synthetic-token');
    expect(s.tab.getItem(TOKEN_STORAGE_KEY)).toBe('synthetic-token');
    expect(s.lasting.items.size).toBe(0);
  });

  test('同じタブの再読み込みでは、タブの中の token をそのまま使う', () => {
    const s = stores();
    s.tab.setItem(TOKEN_STORAGE_KEY, 'synthetic-token');
    expect(loadToken('', true, s)).toBe('synthetic-token');
    expect(s.lasting.items.size).toBe(0);
  });

  test('新しいタブ（どこにも無い）は token を持たない', () => {
    expect(loadToken('', true, stores())).toBe('');
  });

  test('前の版が残した token と券は、1 回だけ使ってタブの中へ移し、残る置き場から消す', () => {
    const s = stores();
    s.lasting.setItem(TOKEN_STORAGE_KEY, 'synthetic-old-token');
    s.lasting.setItem(ASSET_TICKET_STORAGE_KEY, '9999999999.synthetic-ticket');
    s.lasting.setItem('many-ai-cli-theme', 'dark');
    expect(loadToken('', true, s)).toBe('synthetic-old-token');
    expect(s.tab.getItem(TOKEN_STORAGE_KEY)).toBe('synthetic-old-token');
    expect(s.lasting.getItem(TOKEN_STORAGE_KEY)).toBeNull();
    expect(s.lasting.getItem(ASSET_TICKET_STORAGE_KEY)).toBeNull();
    // ほかの設定には触れない。
    expect(s.lasting.getItem('many-ai-cli-theme')).toBe('dark');
    // 次に開く新しいタブには、もう残っていない。
    expect(loadToken('', true, { lasting: s.lasting, tab: new FakeStorage() })).toBe('');
  });

  test('読み取り専用の券もタブの中に置く', () => {
    const s = stores();
    expect(ticketStore(true, s)).toBe(s.tab);
    expect(ticketStore(false, s)).toBe(s.lasting);
  });

  test('置き場が使えなくても落ちず、URL の token はこのページの間だけ使える', () => {
    const broken = { lasting: new BrokenStorage(), tab: new BrokenStorage() };
    expect(loadToken('synthetic-token', true, broken)).toBe('synthetic-token');
    expect(loadToken('', true, broken)).toBe('');
    expect(loadToken('', true, { lasting: null, tab: null })).toBe('');
  });
});

describe('スマホや外からの画面の token は今までどおり', () => {
  test('URL の token を残る置き場に置き、token 無しで開き直せる', () => {
    const s = stores();
    expect(loadToken('synthetic-token', false, s)).toBe('synthetic-token');
    expect(s.lasting.getItem(TOKEN_STORAGE_KEY)).toBe('synthetic-token');
    expect(loadToken('', false, { lasting: s.lasting, tab: new FakeStorage() })).toBe('synthetic-token');
  });

  test('残る置き場に無ければ、サインインの画面がタブの中に置いた分を使う', () => {
    const s = stores();
    s.tab.setItem(TOKEN_STORAGE_KEY, 'synthetic-token');
    expect(loadToken('', false, s)).toBe('synthetic-token');
  });
});

// ---- 画面の起動（web/src/app/util.ts）を実際に読み込む ----

let utilCase = 0;

async function bootDashboard(
  hostname: string, browser: object, search: string, lasting: FakeStorage, tab: FakeStorage,
) {
  const g = globalThis as any;
  const names = ['location', 'navigator', 'history', 'localStorage', 'sessionStorage', 'fetch', 'setInterval'];
  const saved = names.map((name) => [name, Object.getOwnPropertyDescriptor(g, name)] as const);
  const bar: string[] = [];
  const sent: Array<{ url: string; authorization: string }> = [];
  const put = (name: string, value: unknown) => {
    Object.defineProperty(g, name, { value, configurable: true, writable: true });
  };
  put('location', { hostname, search, pathname: '/', hash: '' });
  put('navigator', browser);
  put('history', { state: null, replaceState: (_state: unknown, _title: string, url: string) => { bar.push(url); } });
  put('localStorage', lasting);
  put('sessionStorage', tab);
  put('fetch', async (url: string, init: RequestInit = {}) => {
    sent.push({ url, authorization: new Headers(init.headers).get('Authorization') || '' });
    return new Response(JSON.stringify({ ticket: '9999999999.synthetic-ticket' }), { status: 200 });
  });
  put('setInterval', () => 0);
  try {
    const util = await import(`../src/app/util.ts?case=${++utilCase}`);
    for (let i = 0; i < 20; i++) await Promise.resolve();
    return { token: util.token as string, tabOnly: util.tokenTabOnly as boolean, bar, sent };
  } finally {
    for (const [name, descriptor] of saved) {
      if (descriptor) Object.defineProperty(g, name, descriptor);
      else delete g[name];
    }
  }
}

describe('画面の起動', () => {
  test('PC の手元の画面は URL の token をアドレスバーから消し、token と券をタブの中にだけ置く', async () => {
    const lasting = new FakeStorage();
    const tab = new FakeStorage();
    const boot = await bootDashboard('127.0.0.1', PC, '?token=synthetic-token&via=ssh', lasting, tab);
    expect(boot.token).toBe('synthetic-token');
    expect(boot.tabOnly).toBe(true);
    expect(boot.bar).toEqual(['/?via=ssh']);
    expect(boot.sent).toEqual([{ url: '/api/asset-ticket', authorization: 'Bearer synthetic-token' }]);
    expect(tab.getItem(TOKEN_STORAGE_KEY)).toBe('synthetic-token');
    expect(tab.getItem(ASSET_TICKET_STORAGE_KEY)).toBe('9999999999.synthetic-ticket');
    expect(lasting.items.size).toBe(0);
    // 同じタブの再読み込みは、URL に token が無くても動く。
    const again = await bootDashboard('127.0.0.1', PC, '', lasting, tab);
    expect(again.token).toBe('synthetic-token');
    // 新しいタブ（ブックマーク・PWA）は token を持たない。
    const fresh = await bootDashboard('127.0.0.1', PC, '', lasting, new FakeStorage());
    expect(fresh.token).toBe('');
    expect(fresh.sent.map((request) => request.url)).toEqual(['/api/auth/status']);
  });

  test('スマホが自分の 127.0.0.1 で開いた画面は今までどおり、残る置き場に置いて開き直せる', async () => {
    const lasting = new FakeStorage();
    const boot = await bootDashboard('127.0.0.1', IPHONE, '?token=synthetic-token', lasting, new FakeStorage());
    expect(boot.token).toBe('synthetic-token');
    expect(boot.tabOnly).toBe(false);
    expect(lasting.getItem(TOKEN_STORAGE_KEY)).toBe('synthetic-token');
    // ホーム画面のアイコンから開き直す（新しいタブ・URL に token 無し）。
    const reopened = await bootDashboard('127.0.0.1', IPHONE, '', lasting, new FakeStorage());
    expect(reopened.token).toBe('synthetic-token');
  });

  test('外からの画面は PC でも今までどおり、残る置き場に置いて開き直せる', async () => {
    const lasting = new FakeStorage();
    const boot = await bootDashboard('hub.example.com', PC, '?token=synthetic-token', lasting, new FakeStorage());
    expect(boot.token).toBe('synthetic-token');
    expect(lasting.getItem(TOKEN_STORAGE_KEY)).toBe('synthetic-token');
    expect(lasting.getItem(ASSET_TICKET_STORAGE_KEY)).toBe('9999999999.synthetic-ticket');
    const reopened = await bootDashboard('hub.example.com', PC, '', lasting, new FakeStorage());
    expect(reopened.token).toBe('synthetic-token');
  });
});

// ---- サインインの画面（rust/src/hub/reauth.html）の script を実際に動かす ----
// 動かすのはこの repository の reauth.html に書いてある script そのもの（外から来た文字列は混ぜない）。

function reauthScript(): string {
  const html = readFileSync(new URL('../../rust/src/hub/reauth.html', import.meta.url), 'utf8');
  const match = /<script nonce="__NONCE__">([\s\S]*?)<\/script>/.exec(html);
  if (!match) throw new Error('reauth.html の script が見つからない');
  return match[1].replaceAll('__TOKEN_KEY__', TOKEN_STORAGE_KEY);
}

async function runReauth(hostname: string, browser: object, launchOk: boolean, lasting = new FakeStorage()) {
  const tab = new FakeStorage();
  const shown: Record<string, string> = { working: '', failed: 'hide', 'failed-here': 'hide' };
  const elements = new Map(Object.keys(shown).map((id) => [id, {
    get className() { return shown[id]; },
    set className(value: string) { shown[id] = value; },
  }]));
  const replaced: string[] = [];
  const location = {
    hostname, search: '?launch=0123&via=ssh', pathname: '/', hash: '',
    replace: (to: string) => { replaced.push(to); },
  };
  const fetched: Array<{ url: string; body: string }> = [];
  const fetch = async (url: string, init: { body: string }) => {
    fetched.push({ url, body: init.body });
    return { ok: launchOk, json: async () => ({ token: 'synthetic-token' }) };
  };
  const run = new Function(
    'location', 'navigator', 'localStorage', 'sessionStorage', 'document', 'history', 'fetch',
    reauthScript(),
  );
  run(
    location, browser, lasting, tab, { getElementById: (id: string) => elements.get(id) },
    { replaceState: () => {} }, fetch,
  );
  for (let i = 0; i < 10; i++) await Promise.resolve();
  return { lasting, tab, shown, replaced, fetched };
}

describe('サインインの画面（起動の番号を token に替える）', () => {
  test('PC の手元の画面では token をタブの中にだけ置き、前の版が残した値を消す', async () => {
    const old = new FakeStorage();
    old.setItem(TOKEN_STORAGE_KEY, 'synthetic-old-token');
    const run = await runReauth('127.0.0.1', PC, true, old);
    expect(run.fetched).toEqual([{ url: '/api/auth/launch', body: JSON.stringify({ code: '0123' }) }]);
    expect(run.tab.getItem(TOKEN_STORAGE_KEY)).toBe('synthetic-token');
    expect(run.lasting.items.size).toBe(0);
    expect(run.replaced).toEqual(['/?via=ssh']);
  });

  test('スマホと外からの画面では今までどおり残る置き場に置く', async () => {
    for (const [hostname, browser] of [['127.0.0.1', IPAD], ['127.0.0.1', ANDROID], ['hub.example.com', PC]] as const) {
      const run = await runReauth(hostname, browser, true);
      expect(run.lasting.getItem(TOKEN_STORAGE_KEY)).toBe('synthetic-token');
      expect(run.tab.items.size).toBe(0);
      expect(run.replaced).toEqual(['/?via=ssh']);
    }
  });

  test('番号が使えなかったときは何も置かず、PC の手元の画面には手元向けの案内を出す', async () => {
    const here = await runReauth('localhost', MAC, false);
    expect(here.tab.items.size + here.lasting.items.size).toBe(0);
    expect(here.replaced).toEqual([]);
    expect(here.shown).toEqual({ working: 'hide', failed: 'hide', 'failed-here': '' });
    const away = await runReauth('hub.example.com', PC, false);
    expect(away.shown).toEqual({ working: 'hide', failed: '', 'failed-here': 'hide' });
  });
});

// ---- Service Worker（web/src/sw.ts）を実際に動かす ----

const TOKEN_CACHE = 'many-ai-cli-token-v1';
let swCase = 0;

class FakeCaches {
  readonly stores = new Map<string, Map<string, Response>>();
  async keys(): Promise<string[]> { return [...this.stores.keys()]; }
  async delete(name: string): Promise<boolean> { return this.stores.delete(name); }
  async open(name: string) {
    if (!this.stores.has(name)) this.stores.set(name, new Map());
    const store = this.stores.get(name)!;
    return {
      put: async (url: string, response: Response) => { store.set(url, response); },
      match: async (url: string) => store.get(url)?.clone(),
    };
  }
}

type Handler = (event: any) => void;

async function loadServiceWorker(hostname: string, browser: object, caches: FakeCaches) {
  const handlers = new Map<string, Handler>();
  const opened: string[] = [];
  const g = globalThis as any;
  const saved = { self: g.self, caches: g.caches };
  g.caches = caches;
  g.self = {
    location: { hostname, origin: `http://${hostname}:47777` },
    navigator: browser,
    addEventListener: (type: string, handler: Handler) => { handlers.set(type, handler); },
    skipWaiting: async () => {},
    clients: {
      claim: async () => {},
      matchAll: async () => [],
      openWindow: async (url: string) => { opened.push(url); },
    },
  };
  await import(`../src/sw.ts?case=${++swCase}`);
  const fire = async (type: string, event: Record<string, unknown> = {}) => {
    const waits: Array<Promise<unknown>> = [];
    handlers.get(type)!({ ...event, waitUntil: (promise: Promise<unknown>) => { waits.push(promise); } });
    await Promise.all(waits);
  };
  const restore = () => { g.self = saved.self; g.caches = saved.caches; };
  return { fire, opened, restore };
}

const openClick = {
  notification: { close: () => {}, data: { session_id: 3, url: '' } }, action: 'open',
};

describe('Service Worker が預かる token', () => {
  let restore = () => {};
  afterEach(() => { restore(); });

  async function withOldToken(): Promise<FakeCaches> {
    const caches = new FakeCaches();
    const cache = await caches.open(TOKEN_CACHE);
    await cache.put('/__many-ai-cli-token__', new Response(JSON.stringify({
      token: 'synthetic-old-token', expires: Date.now() + 60_000,
    })));
    return caches;
  }

  test('PC の手元の画面では預からず、前の版が預かった分を消し、通知から開く URL に token を付けない', async () => {
    const caches = await withOldToken();
    const sw = await loadServiceWorker('127.0.0.1', PC, caches);
    restore = sw.restore;
    await sw.fire('activate');
    expect(caches.stores.has(TOKEN_CACHE)).toBe(false);
    await sw.fire('message', { data: { type: 'many-ai-cli-token', token: 'synthetic-token' } });
    expect(caches.stores.has(TOKEN_CACHE)).toBe(false);
    await sw.fire('notificationclick', openClick);
    expect(sw.opened).toEqual(['http://127.0.0.1:47777/?session_id=3']);
  });

  test('前の版が預かったまま動き続けていても、画面からの知らせが届けば消す', async () => {
    const caches = await withOldToken();
    const sw = await loadServiceWorker('localhost', MAC, caches);
    restore = sw.restore;
    await sw.fire('message', { data: { type: 'many-ai-cli-token', token: '' } });
    expect(caches.stores.has(TOKEN_CACHE)).toBe(false);
  });

  test('スマホと外からの画面では今までどおり預かり、通知から開く URL に付ける', async () => {
    for (const [hostname, browser] of [['127.0.0.1', ANDROID], ['127.0.0.1', IPHONE], ['hub.example.com', PC]] as const) {
      const caches = new FakeCaches();
      const sw = await loadServiceWorker(hostname, browser, caches);
      restore = sw.restore;
      await sw.fire('activate');
      await sw.fire('message', { data: { type: 'many-ai-cli-token', token: 'synthetic-token' } });
      expect(caches.stores.has(TOKEN_CACHE)).toBe(true);
      await sw.fire('notificationclick', openClick);
      expect(sw.opened).toEqual([`http://${hostname}:47777/?token=synthetic-token&session_id=3`]);
      sw.restore();
    }
  });
});
