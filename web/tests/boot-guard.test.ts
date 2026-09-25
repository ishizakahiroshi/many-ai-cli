import { afterEach, beforeEach, describe, expect, test } from 'bun:test';

// boot-guard.ts は window / document / location にだけ触れるので、ここで最小の偽物を置いて試す。
// モジュールの状態（評価が届いたか・帯を出したか）はモジュールごとに 1 つなので、
// ケースごとに ?case= を変えて別のインスタンスとして読み込む。

class FakeElement {
  id = '';
  type = '';
  textContent = '';
  readonly style: Record<string, string> = {};
  readonly children: FakeElement[] = [];
  private readonly attrs = new Map<string, string>();
  private readonly listeners = new Map<string, Array<() => void>>();

  constructor(readonly tagName: string) {}

  setAttribute(name: string, value: string): void { this.attrs.set(name, value); }
  removeAttribute(name: string): void { this.attrs.delete(name); }
  getAttribute(name: string): string | null { return this.attrs.get(name) ?? null; }
  append(...els: FakeElement[]): void { this.children.push(...els); }
  appendChild(el: FakeElement): FakeElement { this.children.push(el); return el; }
  addEventListener(type: string, fn: () => void): void {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), fn]);
  }
  click(): void { for (const fn of this.listeners.get('click') ?? []) fn(); }
  allText(): string { return [this.textContent, ...this.children.map((c) => c.allText())].join(' '); }
}

const JA: Record<string, string> = {
  boot_failed_message: '画面を読み込めませんでした。再読み込みしてください。',
  boot_failed_detail: '原因はブラウザのコンソールに出しています。',
  boot_failed_reload: '再読み込み',
};

let body: FakeElement;
let summary: FakeElement;
let reloads: number;
let consoleErrors: unknown[][];
const originalConsoleError = console.error;
let caseNo = 0;

beforeEach(() => {
  body = new FakeElement('body');
  summary = new FakeElement('div');
  summary.setAttribute('data-i18n', 'loading');
  summary.textContent = '読み込み中...';
  reloads = 0;
  consoleErrors = [];
  const doc = Object.assign(new EventTarget(), {
    body,
    documentElement: new FakeElement('html'),
    getElementById: (id: string) => (id === 'summary' ? summary : null),
    createElement: (tag: string) => new FakeElement(tag),
  });
  (globalThis as any).window = new EventTarget();
  (globalThis as any).document = doc;
  (globalThis as any).location = { reload: () => { reloads++; } };
  console.error = (...args: unknown[]) => { consoleErrors.push(args); };
});

afterEach(() => {
  console.error = originalConsoleError;
  delete (globalThis as any).window;
  delete (globalThis as any).document;
  delete (globalThis as any).location;
});

async function loadGuard(): Promise<{ markAppEntryEvaluated: () => void }> {
  caseNo++;
  return import(`../src/app/boot-guard.ts?case=${caseNo}`);
}

function reportError(error: unknown): void {
  (globalThis as any).window.dispatchEvent(Object.assign(new Event('error'), { error, message: String(error) }));
}

function reportRejection(reason: unknown): void {
  (globalThis as any).window.dispatchEvent(Object.assign(new Event('unhandledrejection'), { reason }));
}

function i18nReady(): void {
  (globalThis as any).window.t = (key: string) => JA[key] ?? key;
  (globalThis as any).document.dispatchEvent(new Event('i18n-ready'));
}

const nextTask = () => new Promise((r) => setTimeout(r, 0));
const alertBar = () => body.children.find((el) => el.getAttribute('role') === 'alert');

describe('boot-guard', () => {
  test('評価が止まったら、辞書が届いてから「読み込めなかった」と再読み込みの帯を出す。例外の中身は画面に出さない', async () => {
    await loadGuard();
    const cause = new ReferenceError("Cannot access 'activeSessionId' before initialization");
    reportError(cause);
    await nextTask();
    expect(alertBar()).toBeUndefined(); // 辞書を待っている

    i18nReady();
    await nextTask();
    const bar = alertBar();
    expect(bar).toBeDefined();
    expect(bar!.allText()).toContain(JA.boot_failed_message);
    expect(bar!.allText()).toContain(JA.boot_failed_detail);
    expect(bar!.allText()).not.toContain('activeSessionId');
    expect(summary.textContent).toBe(JA.boot_failed_message);
    expect(summary.getAttribute('data-i18n')).toBeNull();
    expect(consoleErrors.some((args) => args.includes(cause))).toBe(true);

    const button = bar!.children.find((el) => el.tagName === 'button');
    expect(button?.textContent).toBe(JA.boot_failed_reload);
    button!.click();
    expect(reloads).toBe(1);
  });

  test('拒否された Promise でも出す。辞書が先に届いていれば待たない', async () => {
    await loadGuard();
    i18nReady();
    reportRejection(new Error('init failed'));
    await nextTask();
    await nextTask();
    expect(alertBar()?.allText()).toContain(JA.boot_failed_message);
  });

  test('評価の途中で例外が報告されても、評価が最後まで届いたら何も出さない', async () => {
    const guard = await loadGuard();
    reportError(new Error('listener threw while evaluating'));
    guard.markAppEntryEvaluated();
    await nextTask();
    i18nReady();
    await nextTask();
    expect(alertBar()).toBeUndefined();
    expect(summary.textContent).toBe('読み込み中...');
  });

  test('初期化の完了の後の例外では何も出さない', async () => {
    const guard = await loadGuard();
    guard.markAppEntryEvaluated();
    reportError(new Error('later'));
    reportRejection(new Error('later'));
    i18nReady();
    await nextTask();
    await nextTask();
    expect(alertBar()).toBeUndefined();
    expect(consoleErrors).toHaveLength(0);
  });
});
