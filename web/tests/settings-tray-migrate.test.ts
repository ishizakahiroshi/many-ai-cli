import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { createRequire } from 'node:module';
const ts = createRequire(import.meta.url)('typescript');

// The tray-migration row, its i18n surface and the two wirings that show and
// fire it are exercised against the real sources with a synthetic DOM/HTTP.
// No Hub, product build, browser, network or new dependency is used.
const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');
const html = read('../src/index.html');
const settings = read('../src/app/settings.ts');
const app = read('../src/app.ts');
const locales = {
  ja: JSON.parse(read('../src/i18n/ja.json')),
  en: JSON.parse(read('../src/i18n/en.json')),
  vi: JSON.parse(read('../src/i18n/vi.json')),
};
const run = (source: string, ctx: any) =>
  runInNewContext(
    ts.transpileModule(source, {
      compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
    }).outputText,
    ctx,
  );
const tick = async () => {
  for (let i = 0; i < 15; i++) await Promise.resolve();
};

test('tray migration offer remains inside the Hub section without a separate shutdown action', () => {
  const hub = html.slice(
    html.indexOf('id="settings-hub-actions"'),
    html.indexOf('</details>', html.indexOf('id="settings-hub-actions"')),
  );
  assert.match(hub, /<div id="tray-migrate-offer" hidden>/);
  assert.match(hub, /id="tray-migrate-btn"/);
  assert.match(hub, /data-i18n="tray_migrate_note"/);
  assert.match(hub, /data-i18n="tray_migrate_btn"/);
  assert.doesNotMatch(hub, /id="shutdown-btn"/);
});

test('tray migration i18n keys exist in all locales', () => {
  const keys = [
    'tray_migrate_btn',
    'tray_migrate_note',
    'tray_migrate_confirm_title',
    'tray_migrate_confirm',
    'tray_migrate_confirm_expose',
    'tray_migrate_status_failed',
    'tray_migrate_expose_failed',
    'tray_migrate_failed',
  ];
  for (const [lang, dict] of Object.entries(locales)) {
    for (const key of keys) {
      assert.equal(typeof dict[key], 'string', `${lang} missing ${key}`);
      assert.notEqual(dict[key], '', `${lang} has empty ${key}`);
    }
  }
});

test('launch-state wiring only unhides the offer for a real migration offer', async () => {
  // Execute the actual /api/info block's launch-state branch with a stubbed
  // apiFetch and a synthetic element.
  const start = settings.indexOf('// Hub の起動形態');
  const end = settings.indexOf('} catch (_) {}', start) + '} catch (_) {}'.length;
  assert.ok(start > 0 && end > start, 'launch-state block not found in settings.ts');
  const block = settings.slice(start, end);

  for (const [offerFlag, expected] of [
    [true, false],
    [false, true],
  ]) {
    const offer = { hidden: true };
    const ctx = {
      apiFetch: async (path: string) => {
        assert.equal(path, '/api/hub/launch-state');
        return { ok: true, json: async () => ({ show_tray_migration_offer: offerFlag }) };
      },
      document: { getElementById: (id: string) => (id === 'tray-migrate-offer' ? offer : null) },
    };
    run(`(async () => { ${block} })()`, ctx);
    await tick();
    assert.equal(offer.hidden, expected, `offer=${offerFlag}`);
  }

  // A failed or non-OK probe never shows the row.
  for (const response of [
    async () => ({ ok: false, json: async () => ({}) }),
    async () => { throw new Error('network down'); },
  ]) {
    const offer = { hidden: true };
    run(`(async () => { ${block} })()`, {
      apiFetch: response,
      document: { getElementById: () => offer },
    });
    await tick();
    assert.equal(offer.hidden, true);
  }
});

function migrateSlice(): string {
  const marker = "const migrateBtn = document.getElementById('tray-migrate-btn');";
  const migrateAt = app.indexOf(marker);
  assert.ok(migrateAt > 0, 'tray-migrate-btn listener not found in app.ts');
  const start = app.lastIndexOf('(function () {', migrateAt);
  const end = app.indexOf('})();', migrateAt) + '})();'.length;
  assert.ok(start >= 0 && end > start, 'tray migrate IIFE not found');
  return app.slice(start, end);
}

function migrateDom() {
  const button: {
    disabled: boolean;
    listeners: Record<string, () => Promise<void>>;
    addEventListener: (type: string, fn: () => Promise<void>) => void;
  } = {
    disabled: false,
    listeners: {},
    addEventListener(type, fn) { this.listeners[type] = fn; },
  };
  const offer = { hidden: false };
  return {
    button,
    offer,
    document: {
      getElementById: (id: string) => {
        if (id === 'tray-migrate-btn') return button;
        if (id === 'tray-migrate-offer') return offer;
        return null;
      },
    },
  };
}

test('migrate click stops when exposure cannot be checked or closed, and hides only after a 2xx POST', async () => {
  const slice = migrateSlice();
  const cases: Array<{
    name: string;
    status: () => Promise<{ ok: boolean; status?: { state: string } }>;
    confirm?: () => Promise<boolean>;
    disable?: () => Promise<{ ok: boolean }>;
    post?: () => Promise<{ ok: boolean }>;
    posts: number;
    toast: string | null;
    hidden: boolean;
    log: string[];
  }> = [
    {
      name: 'status failed',
      status: async () => ({ ok: false }),
      posts: 0,
      toast: 'tray_migrate_status_failed',
      hidden: false,
      log: [],
    },
    {
      name: 'cancelled',
      status: async () => ({ ok: true, status: { state: 'ready' } }),
      confirm: async () => false,
      posts: 0,
      toast: null,
      hidden: false,
      log: ['confirm'],
    },
    {
      name: 'expose close failed',
      status: async () => ({ ok: true, status: { state: 'ready' } }),
      disable: async () => ({ ok: false }),
      posts: 0,
      toast: 'tray_migrate_expose_failed',
      hidden: false,
      log: ['confirm', 'disable'],
    },
    {
      name: 'post refused',
      status: async () => ({ ok: true, status: { state: 'ready' } }),
      disable: async () => ({ ok: true }),
      post: async () => ({ ok: false }),
      posts: 1,
      toast: 'tray_migrate_failed',
      hidden: false,
      log: ['confirm', 'disable', 'post'],
    },
    {
      name: 'post threw',
      status: async () => ({ ok: true, status: { state: 'idle' } }),
      post: async () => { throw new Error('down'); },
      posts: 1,
      toast: 'tray_migrate_failed',
      hidden: false,
      log: ['confirm', 'post'],
    },
    {
      name: 'moved',
      status: async () => ({ ok: true, status: { state: 'ready' } }),
      disable: async () => ({ ok: true }),
      post: async () => ({ ok: true }),
      posts: 1,
      toast: null,
      hidden: true,
      log: ['confirm', 'disable', 'post'],
    },
  ];
  for (const item of cases) {
    const dom = migrateDom();
    const log: string[] = [];
    const toasts: string[] = [];
    let posts = 0;
    run(slice, {
      document: dom.document,
      fetchExposeStatus: async (force: boolean) => {
        assert.equal(force, true, item.name);
        return item.status();
      },
      appConfirm: async () => {
        log.push('confirm');
        return item.confirm ? item.confirm() : true;
      },
      disableExpose: async () => {
        log.push('disable');
        if (!item.disable) throw new Error(`${item.name} disabled exposure`);
        return item.disable();
      },
      apiFetch: async (path: string, init: { method?: string }) => {
        assert.equal(path, '/api/tray-migrate', item.name);
        assert.equal(init.method, 'POST', item.name);
        log.push('post');
        posts += 1;
        if (!item.post) throw new Error(`${item.name} posted`);
        return item.post();
      },
      showToast: (message: string) => { toasts.push(message); },
      t: (key: string) => key,
    });
    await dom.button.listeners.click();
    await tick();
    assert.equal(posts, item.posts, item.name);
    assert.deepEqual(log, item.log, item.name);
    assert.deepEqual(toasts, item.toast ? [item.toast] : [], item.name);
    assert.equal(dom.offer.hidden, item.hidden, item.name);
    assert.equal(dom.button.disabled, false, item.name);
  }
});
