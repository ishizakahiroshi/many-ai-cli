import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { createRequire } from 'node:module';

const esbuild = createRequire(import.meta.url)('esbuild');
const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');
const settingsSource = read('../src/app/settings.ts');
const html = read('../src/index.html');
const sessionListSource = read('../src/app/session-list.ts');
const clean = (source: string) => source.replace(/^import[\s\S]*?;\r?$/gm, '').replace(/^export /gm, '');
const run = (source: string, ctx: any) => runInNewContext(esbuild.transformSync(clean(source), {
  loader: 'ts', target: 'es2022', format: 'cjs',
}).code, ctx);
const tick = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); };
const response = (body: any, ok = true) => ({ ok, json: async () => body });
const controllerStart = settingsSource.indexOf('// AI の起動シェルは Hub の設定 API を唯一の保存元にする。');
const controllerEnd = settingsSource.indexOf('\nfunction initSettingsNav()', controllerStart);
assert.ok(controllerStart >= 0 && controllerEnd > controllerStart, 'AI launch shell controller block is present');
const controllerCode = settingsSource.slice(controllerStart, controllerEnd) + '\nglobalThis.initAiLaunchShellSettings = initAiLaunchShellSettings;';

class FakeOption {
  value = '';
  textContent = '';
  disabled = false;
  title = '';
  attrs = new Map<string, string>();
  owner: FakeSelect | null = null;
  setAttribute(key: string, value: string) { this.attrs.set(key, value); }
  getAttribute(key: string) { return this.attrs.get(key) ?? null; }
  remove() { this.owner?.removeOption(this); }
}

class FakeElement {
  hidden = false;
  disabled = false;
  textContent = '';
  listeners = new Map<string, Function[]>();
  classes = new Set<string>();
  classList = { toggle: (name: string, on: boolean) => on ? this.classes.add(name) : this.classes.delete(name) };
  addEventListener(type: string, fn: Function) { this.listeners.set(type, [...(this.listeners.get(type) || []), fn]); }
  emit(type: string) { for (const fn of this.listeners.get(type) || []) fn({ type, target: this }); }
  click() { this.emit('click'); }
}

class FakeSelect extends FakeElement {
  value = '';
  options: FakeOption[] = [];
  appendChild(option: FakeOption) { option.owner = this; this.options.push(option); return option; }
  removeOption(option: FakeOption) { this.options = this.options.filter((item) => item !== option); option.owner = null; }
  querySelectorAll(selector: string) {
    if (selector === 'option[data-ai-launch-shell-invalid]') {
      return this.options.filter((option) => option.getAttribute('data-ai-launch-shell-invalid') !== null);
    }
    return [];
  }
}

function fixture(apiFetch: Function) {
  const select = new FakeSelect();
  select.disabled = true;
  for (const [value, key, disabled] of [
    ['', 'settings_ai_launch_shell_loading', true],
    ['auto', 'settings_ai_launch_shell_auto', false],
    ['pwsh', 'settings_ai_launch_shell_pwsh', true],
    ['cmd', 'settings_ai_launch_shell_cmd', true],
  ]) {
    const option = new FakeOption(); option.value = value; option.disabled = disabled; option.setAttribute('data-i18n', key); select.appendChild(option);
  }
  const status = new FakeElement(); status.hidden = true;
  const settingsButton = new FakeElement();
  const panel = new FakeElement(); panel.hidden = true;
  const elements = new Map<string, any>([
    ['ai-launch-shell', select], ['ai-launch-shell-status', status], ['settings-btn', settingsButton], ['settings-panel', panel],
  ]);
  const events: any[] = [];
  const document = {
    getElementById: (id: string) => elements.get(id) || null,
    createElement: (tag: string) => tag === 'option' ? new FakeOption() : new FakeElement(),
    dispatchEvent: (event: any) => { events.push(event); return true; },
  };
  class TestCustomEvent { constructor(public type: string, public init: any = {}) {} }
  const ctx: any = { document, apiFetch, t: (key: string) => key, CustomEvent: TestCustomEvent };
  run(controllerCode, ctx);
  const controller = ctx.initAiLaunchShellSettings();
  return { select, status, settingsButton, panel, events, controller };
}

const allAvailable = {
  auto: { available: true },
  pwsh: { available: true },
  cmd: { available: true },
};

function openSettings(f: ReturnType<typeof fixture>) {
  f.panel.hidden = false;
  f.settingsButton.click();
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: any) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

test('Session has a localized launch-shell row beside terminal colors', () => {
  const session = html.slice(html.indexOf('data-section="session" data-group="basic"'), html.indexOf('</details>', html.indexOf('data-section="session" data-group="basic"')));
  assert.ok(session.indexOf('id="terminal-color"') < session.indexOf('id="ai-launch-shell"'));
  assert.match(session, /id="ai-launch-shell"[^>]*disabled/);
  assert.match(session, /id="ai-launch-shell-status"[^>]*role="status"[^>]*aria-live="polite"/);
  assert.match(session, /<label[^>]*for="ai-launch-shell"/);
  assert.match(session, /data-i18n="settings_ai_launch_shell_desc"/);
  assert.match(session, /<option value=""[^>]*selected disabled/);
});

test('load preserves a saved but unavailable shell and explains why', async () => {
  const f = fixture(async () => response({
    ai_launch_shell: 'pwsh',
    availability: {
      auto: { available: true },
      pwsh: { available: false, reason: 'pwsh and cmd are only supported on Windows' },
      cmd: { available: false, reason: 'pwsh.exe was not found by executable resolution' },
    },
    selected_availability: { available: false, reason: 'pwsh and cmd are only supported on Windows' },
  }));
  openSettings(f);
  await tick();
  assert.equal(f.select.value, 'pwsh');
  assert.equal(f.select.options.find((option) => option.value === 'pwsh')?.disabled, true);
  assert.equal(f.select.options.find((option) => option.value === 'cmd')?.disabled, true);
  assert.match(f.select.options.find((option) => option.value === 'cmd')?.textContent || '', /settings_ai_launch_shell_unavailable/);
  assert.match(f.status.textContent, /settings_ai_launch_shell_saved_unavailable/);
  assert.match(f.status.textContent, /settings_ai_launch_shell_unsupported/);
});

test('an unrecognized saved value remains selected and visible instead of becoming auto', async () => {
  const f = fixture(async () => response({
    ai_launch_shell: 'legacy-shell', availability: allAvailable, selected_availability: { available: false, reason: 'unsupported_platform' },
  }));
  openSettings(f);
  await tick();
  assert.equal(f.select.value, 'legacy-shell');
  assert.equal(f.select.options.find((option) => option.value === 'legacy-shell')?.disabled, true);
  assert.match(f.status.textContent, /legacy-shell/);
  assert.match(f.status.textContent, /settings_ai_launch_shell_choose_valid/);
});

test('rapid selections serialize writes, global save waits for the queue, and reset saves auto', async () => {
  const writes: Array<{ body: any; pending: ReturnType<typeof deferred<any>> }> = [];
  const f = fixture(async (_url: string, init: any = {}) => {
    if (!init.method) return response({ ai_launch_shell: 'auto', availability: allAvailable, selected_availability: { available: true } });
    const pending = deferred<any>();
    writes.push({ body: JSON.parse(init.body), pending });
    return pending.promise;
  });
  openSettings(f);
  await tick();

  f.select.value = 'pwsh'; f.select.emit('change');
  await tick();
  f.select.value = 'cmd'; f.select.emit('change');
  await tick();
  assert.deepEqual(writes.map((item) => item.body.ai_launch_shell), ['pwsh']);
  const globalSave = f.controller.save();
  await tick();
  assert.deepEqual(writes.map((item) => item.body.ai_launch_shell), ['pwsh']);

  writes[0].pending.resolve(response({ ok: true, ai_launch_shell: 'pwsh' }));
  await tick();
  assert.deepEqual(writes.map((item) => item.body.ai_launch_shell), ['pwsh', 'cmd']);
  writes[1].pending.resolve(response({ ok: true, ai_launch_shell: 'cmd' }));
  await globalSave;
  assert.equal(f.select.value, 'cmd');

  await f.controller.save();
  assert.equal(writes.length, 2, 'global save is satisfied by the confirmed queued value');

  const reset = f.controller.reset();
  await tick();
  assert.equal(f.select.value, 'auto');
  assert.equal(writes[2].body.ai_launch_shell, 'auto');
  writes[2].pending.resolve(response({ ok: true, ai_launch_shell: 'auto' }));
  await reset;
  assert.equal(f.select.value, 'auto');
});

test('a GET in flight cannot overwrite a user change and the latest value is saved', async () => {
  const pendingGet = deferred<any>();
  const writes: any[] = [];
  const f = fixture(async (_url: string, init: any = {}) => {
    if (!init.method) return pendingGet.promise;
    const body = JSON.parse(init.body); writes.push(body);
    return response({ ok: true, ai_launch_shell: body.ai_launch_shell });
  });
  openSettings(f);
  f.select.value = 'cmd';
  f.select.emit('change');
  pendingGet.resolve(response({ ai_launch_shell: 'pwsh', availability: allAvailable, selected_availability: { available: true } }));
  await tick();
  assert.equal(f.select.value, 'cmd');
  assert.deepEqual(writes, [{ ai_launch_shell: 'cmd' }]);
});

test('reopening settings waits for a pending selection write before applying GET', async () => {
  const pendingWrite = deferred<any>();
  let getCount = 0;
  let postCount = 0;
  let writeBody: any;
  const f = fixture(async (_url: string, init: any = {}) => {
    if (!init.method) {
      getCount++;
      return response({ ai_launch_shell: 'auto', availability: allAvailable, selected_availability: { available: true } });
    }
    postCount++;
    writeBody = JSON.parse(init.body);
    return pendingWrite.promise;
  });
  openSettings(f);
  await tick();
  f.select.value = 'pwsh';
  f.select.emit('change');
  await tick();
  assert.deepEqual(writeBody, { ai_launch_shell: 'pwsh' });

  f.panel.hidden = true;
  openSettings(f);
  await tick();
  assert.equal(getCount, 2);
  assert.equal(f.select.value, 'pwsh', 'the old GET result must not replace the pending user choice');

  pendingWrite.resolve(response({ ok: true, ai_launch_shell: 'pwsh' }));
  await tick();
  assert.equal(f.select.value, 'pwsh');
  await f.controller.save();
  assert.equal(postCount, 1, 'global Save must treat the completed pwsh write as current');
});

test('global save does not overwrite an unknown value after load failure; explicit reset may save auto', async () => {
  const writes: any[] = [];
  const f = fixture(async (_url: string, init: any = {}) => {
    if (!init.method) return response({ error: 'load_failed' }, false);
    const body = JSON.parse(init.body);
    writes.push(body);
    return response({ ok: true, ai_launch_shell: body.ai_launch_shell });
  });
  openSettings(f);
  await tick();
  assert.equal(f.select.disabled, true);
  assert.equal(f.select.value, '', 'an unresolved setting must not present the HTML auto choice as saved');
  await assert.rejects(f.controller.save(), /not loaded/);
  assert.deepEqual(writes, [], 'global Save must not post the HTML default');
  assert.match(f.status.textContent, /settings_ai_launch_shell_not_loaded/);

  await f.controller.reset();
  assert.deepEqual(writes, [{ ai_launch_shell: 'auto' }], 'explicit Reset intentionally saves auto');
});

test('global save does not POST while the initial value is still loading', async () => {
  const pendingGet = deferred<any>();
  const writes: any[] = [];
  const f = fixture(async (_url: string, init: any = {}) => {
    if (!init.method) return pendingGet.promise;
    const body = JSON.parse(init.body);
    writes.push(body);
    return response({ ok: true, ai_launch_shell: body.ai_launch_shell });
  });
  openSettings(f);
  await assert.rejects(f.controller.save(), /not loaded/);
  assert.deepEqual(writes, [], 'unresolved GET must not let global Save post the initial auto option');
  assert.equal(f.select.value, '');
  assert.equal(f.select.disabled, true);

  pendingGet.resolve(response({ ai_launch_shell: 'pwsh', availability: allAvailable, selected_availability: { available: true } }));
  await tick();
  assert.equal(f.select.value, 'pwsh');
});

test('failed save restores the last confirmed value and shows an error', async () => {
  let postCount = 0;
  const f = fixture(async (_url: string, init: any = {}) => {
    if (!init.method) return response({ ai_launch_shell: 'auto', availability: allAvailable, selected_availability: { available: true } });
    postCount++;
    if (postCount === 1) return response({ ok: true, ai_launch_shell: 'pwsh' });
    return response({ error: 'save_failed' }, false);
  });
  openSettings(f);
  await tick();
  f.select.value = 'pwsh'; f.select.emit('change');
  await tick();
  assert.equal(f.select.value, 'pwsh');
  f.select.value = 'cmd'; f.select.emit('change');
  await tick();
  assert.equal(f.select.value, 'pwsh');
  assert.match(f.status.textContent, /settings_ai_launch_shell_save_failed/);
});

test('direct launch badge is localized while historical shell paths remain intact', () => {
  const start = sessionListSource.indexOf('export function updateShellBadge(id) {');
  const end = sessionListSource.indexOf('\n}', start) + 2;
  assert.ok(start >= 0 && end > start);
  const source = sessionListSource.slice(start, end).replace(/^export /, '') + '\nglobalThis.updateShellBadge = updateShellBadge;';
  const badge = { textContent: '' };
  const sessions = new Map<number, any>([[1, { shell: 'direct' }], [2, { shell: 'C:\\Tools\\pwsh.exe' }]]);
  const ctx: any = {
    document: { getElementById: (id: string) => id === 'terminal-shell-info' ? badge : null },
    sessions,
    ti18n: (key: string, fallback: string) => key === 'session_shell_direct' ? '直接起動' : fallback,
  };
  run(source, ctx);
  ctx.updateShellBadge(1);
  assert.equal(badge.textContent, ' · 直接起動');
  ctx.updateShellBadge(2);
  assert.equal(badge.textContent, ' · C:\\Tools\\pwsh.exe');
});
