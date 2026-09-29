import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

// attachments.ts の実コード（window の paste 処理）を合成 DOM で動かす。Hub・ブラウザ・
// web/dist は使わない。bugfix_memo-paste-goes-to-composer_2026-09-30.md の再現テスト。
class FakeElement {
  isContentEditable = false;
  constructor(public tagName: string) {}
}
class FakeHTMLElement extends FakeElement {}
class FakeTextArea extends FakeHTMLElement { constructor() { super('TEXTAREA'); } }
class FakeInput extends FakeHTMLElement { constructor(public type = 'text') { super('INPUT'); } }

const FIVE_LINES = ['1 行目', '2 行目', '3 行目', '4 行目', '5 行目'].join('\n');

function fixture() {
  const windowListeners = new Map<string, Function[]>();
  const stagedTexts: string[] = [];
  const composer = new FakeTextArea();
  const body = new FakeHTMLElement('BODY');
  const ctx: any = {
    Element: FakeElement, HTMLElement: FakeHTMLElement,
    HTMLTextAreaElement: FakeTextArea, HTMLInputElement: FakeInput,
    URL, File, Blob, Promise, Math, Error,
    document: { getElementById: () => null, addEventListener: () => {}, createElement: () => ({}) },
    window: { addEventListener: (type: string, fn: Function) => windowListeners.set(type, [...(windowListeners.get(type) || []), fn]) },
    t: (key: string) => key, apiFetch: async () => ({ ok: true }), showToast: () => {}, token: '',
    activeSessionId: 7, sessions: new Map(), terminals: new Map(),
    copyPathText: async () => {}, pushMessage: () => {}, openLightbox: () => {},
    inputEl: composer, isInteractiveFocusTarget: () => false, updateInputAffordance: () => {},
    // 本物は 5 行以上か 300 字超でチップにして true を返す（app.ts の stagePastedText）。
    stagePastedText: (text: string) => { stagedTexts.push(text); return true; },
  };
  const source = readFileSync(new URL('../src/app/attachments.ts', import.meta.url), 'utf8')
    .replace(/^import .*;\r?$/gm, '').replace(/^export /gm, '');
  runInNewContext(new Bun.Transpiler({ loader: 'ts' }).transformSync(source) + '\nglobalThis.probe={pendingAttachFiles};', ctx);

  async function paste(target: FakeElement, data: { text?: string; image?: boolean }) {
    const items = data.image
      ? [{ kind: 'file', getAsFile: () => new File([new Uint8Array([1, 2, 3])], 'image.png', { type: 'image/png' }) }]
      : [{ kind: 'string', getAsFile: () => null }];
    const event = {
      target,
      defaultPrevented: false,
      clipboardData: { items, getData: () => data.text ?? '' },
      preventDefault() { this.defaultPrevented = true; },
    };
    for (const fn of windowListeners.get('paste') || []) fn(event);
    for (let i = 0; i < 5; i++) await new Promise(resolve => setTimeout(resolve, 0));
    return event;
  }
  return { composer, body, stagedTexts, pending: ctx.probe.pendingAttachFiles as unknown[], paste };
}

describe('window の paste 処理は、下の入力欄以外の文字入力欄への貼り付けを横取りしない', () => {
  test('作業メモ欄（別の textarea）に 5 行貼っても、下の入力欄のチップにしない', async () => {
    const f = fixture();
    const memoInput = new FakeTextArea();
    const event = await f.paste(memoInput, { text: FIVE_LINES });
    expect(f.stagedTexts).toEqual([]);
    expect(event.defaultPrevented).toBe(false);
  });

  test('作業メモ欄に画像を貼っても、下の入力欄の添付に積まない', async () => {
    const f = fixture();
    await f.paste(new FakeTextArea(), { image: true });
    expect(f.pending.length).toBe(0);
  });

  test('文字入力の input と contenteditable も同じく横取りしない', async () => {
    const f = fixture();
    const editable = new FakeHTMLElement('DIV');
    editable.isContentEditable = true;
    await f.paste(new FakeInput('text'), { text: FIVE_LINES });
    await f.paste(editable, { text: FIVE_LINES });
    expect(f.stagedTexts).toEqual([]);
  });

  test('対照: 下の入力欄に 5 行貼ったら、今までどおりチップにする', async () => {
    const f = fixture();
    const event = await f.paste(f.composer, { text: FIVE_LINES });
    expect(f.stagedTexts).toEqual([FIVE_LINES]);
    expect(event.defaultPrevented).toBe(true);
  });

  test('対照: 文字入力欄にフォーカスが無いときの画像は、今までどおり下の入力欄の添付に積む', async () => {
    const f = fixture();
    await f.paste(f.body, { image: true });
    expect(f.pending.length).toBe(1);
  });

  test('対照: チェックボックスにフォーカスがあるときの長文は、今までどおり下の入力欄へ回す', async () => {
    const f = fixture();
    await f.paste(new FakeInput('checkbox'), { text: FIVE_LINES });
    expect(f.stagedTexts).toEqual([FIVE_LINES]);
  });
});
