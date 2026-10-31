import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

// attachments.ts の「添付の送り先」の節（実コード）を合成 DOM で動かす。Hub・ブラウザ・web/dist は使わない。
// bugfix_shell-long-paste-local-path-without-remote-target_2026-10-28.md の再現テスト。
// 送り先を選んでいないシェルのセッションで添付を送ろうとしたら、送る前に窓を出して選ばせる。
// （app.ts の doSend 側の止め方は DOM の外で、ここでは「窓を出す・記録する・決めたら知らせる」を確かめる）

class FakeEl {
  hidden = false;
  className = '';
  textContent = '';
  value = '';
  type = '';
  id = '';
  placeholder = '';
  spellcheck = true;
  autocomplete = '';
  style: Record<string, string> = {};
  children: FakeEl[] = [];
  offsetWidth = 200;
  offsetHeight = 100;
  private attrs = new Map<string, string>();
  private listeners = new Map<string, Function[]>();
  constructor(public tagName: string, private focusLog: FakeEl[] = []) {}
  setAttribute(k: string, v: string) { this.attrs.set(k, v); }
  getAttribute(k: string) { return this.attrs.get(k) ?? null; }
  removeAttribute(k: string) { this.attrs.delete(k); }
  addEventListener(type: string, fn: Function) { this.listeners.set(type, [...(this.listeners.get(type) || []), fn]); }
  append(...c: FakeEl[]) { this.children.push(...c); }
  appendChild(c: FakeEl) { this.children.push(c); return c; }
  replaceChildren(...c: FakeEl[]) { this.children = [...c]; }
  getBoundingClientRect() { return { left: 0, top: 0, width: 0, height: 0 }; }
  focus() { this.focusLog.push(this); }
  select() {}
  fire(type: string, event: any = {}) {
    const e = { preventDefault() {}, stopPropagation() {}, ...event };
    for (const fn of this.listeners.get(type) || []) fn(e);
  }
}

const SHELL = { provider: 'shell', started_at: 'S1' };

function fixture(opts: { stored?: unknown; storageBroken?: boolean } = {}) {
  const focusLog: FakeEl[] = [];
  const store = new Map<string, string>();
  if (opts.stored !== undefined) store.set('remoteAttachTargetsV1', JSON.stringify(opts.stored));
  const toasts: string[] = [];
  const body = new FakeEl('BODY', focusLog);
  const remoteBtn = new FakeEl('BUTTON', focusLog);
  const composer = new FakeEl('TEXTAREA', focusLog);
  const sessions = new Map<number, { provider: string; started_at: string }>([
    [7, { ...SHELL }],
    [8, { provider: 'claude', started_at: 'S1' }],
    [9, { ...SHELL }],
  ]);
  const ctx: any = {
    Element: FakeEl, HTMLElement: FakeEl, HTMLTextAreaElement: class {}, HTMLInputElement: class {},
    URL, File, Blob, FormData, setTimeout,
    document: {
      body,
      getElementById: (id: string) => (id === 'attach-remote-btn' ? remoteBtn : null),
      addEventListener: () => {},
      createElement: (tag: string) => new FakeEl(tag.toUpperCase(), focusLog),
    },
    window: { addEventListener: () => {}, innerWidth: 1000 },
    localStorage: {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => {
        if (opts.storageBroken) throw new Error('QuotaExceededError');
        store.set(k, v);
      },
    },
    t: (key: string) => key,
    apiFetch: async () => ({ ok: true }), showToast: (msg: string) => { toasts.push(msg); }, token: '',
    activeSessionId: 7, sessions, terminals: new Map(),
    captureComposerOwner: () => null, isComposerOwnerLive: () => false,
    appendComposerAttachment: () => false, removeComposerAttachment: () => {},
    copyPathText: async () => {}, pushMessage: () => {}, openLightbox: () => {},
    inputEl: composer, isInteractiveFocusTarget: () => false, updateInputAffordance: () => {},
    stagePastedText: () => false, basenameForPath: (p: string) => p,
  };
  const source = readFileSync(new URL('../src/app/attachments.ts', import.meta.url), 'utf8')
    .replace(/^import .*;\r?$/gm, '').replace(/^export /gm, '');
  runInNewContext(new Bun.Transpiler({ loader: 'ts' }).transformSync(source), ctx);

  const stored = () => JSON.parse(store.get('remoteAttachTargetsV1') || '[]');
  const popup = (): FakeEl => body.children[0];
  const tick = () => new Promise((resolve) => setTimeout(resolve, 5));
  const actionButtons = () => popup().children.find((c) => c.className.includes('agent-log-popup-actions'))!.children;
  const inputBox = () => popup().children.find((c) => c.tagName === 'INPUT')!;
  const findButton = (label: string) => actionButtons().find((b) => b.textContent === label)!;
  return { ctx, sessions, store, stored, toasts, remoteBtn, composer, focusLog, popup, tick, actionButtons, inputBox, findButton };
}

describe('添付の送り先: 「まだ選んでいない」と「このPCで読むと決めた」を区別して覚える', () => {
  test('秘密鍵パスを保存して再読込し、セッションごとに保持する。ローカルへ戻すと解除する', () => {
    const f = fixture();
    f.remoteBtn.fire('click');
    f.inputBox().value = 'user@example.com';
    const keyInput = f.popup().children.find((c) => c.id === 'remote-attach-identity')!;
    keyInput.value = 'D:/synthetic keys/id_ed25519';
    f.findButton('remote_attach_save').fire('click');
    const restored = fixture({ stored: f.stored() });
    expect(restored.ctx.remoteAttachIdentity(7)).toBe(keyInput.value);
    expect(restored.ctx.remoteAttachIdentity(9)).toBe('');
    restored.ctx.setRemoteAttachTarget(7, '');
    expect(restored.ctx.remoteAttachIdentity(7)).toBe('');
    expect(restored.stored()[0].identity).toBeUndefined();
  });

  test('送信と取得は保存した秘密鍵パスを API へ渡す', async () => {
    const f = fixture();
    const key = 'D:/synthetic keys/id_ed25519';
    f.ctx.setRemoteAttachTarget(7, 'user@example.com', key);
    const requests: string[] = [];
    f.ctx.apiFetch = async (url: string) => {
      requests.push(url);
      return { ok: false, status: 502, json: async () => ({ detail: 'synthetic failure' }) };
    };
    const files = [{ filename: 'a.txt', buf: new Uint8Array([1]).buffer }];
    expect(await f.ctx.flushPendingAttach(7, files)).toBeNull();
    f.ctx.isFetchableRemotePath = () => true;
    expect(await f.ctx.fetchRemoteFile(7, '/tmp/a.txt')).toBe(false);
    expect(requests).toHaveLength(2);
    for (const url of requests) {
      const params = new URL(url, 'http://localhost').searchParams;
      expect(params.get('remote_identity')).toBe(key);
      expect(params.get('remote_target')).toBe('user@example.com');
    }
    expect(files).toHaveLength(1);
  });

  test('鍵パスに改行を含むと保存せず、窓を閉じない', () => {
    const f = fixture();
    f.remoteBtn.fire('click');
    f.inputBox().value = 'user@example.com';
    f.popup().children.find((c) => c.id === 'remote-attach-identity')!.value = 'key\nnext';
    f.findButton('remote_attach_save').fire('click');
    expect(f.popup().hidden).toBe(false);
    expect(f.stored()).toEqual([]);
  });

  test('シェルのセッションは記録が無ければ未選択。シェル以外・セッション無しは未選択扱いにしない', () => {
    const f = fixture();
    expect(f.ctx.isRemoteAttachChoicePending(7)).toBe(true);
    expect(f.ctx.remoteAttachTarget(7)).toBe('');
    expect(f.ctx.isRemoteAttachChoicePending(8)).toBe(false);
    expect(f.ctx.isRemoteAttachChoicePending(null)).toBe(false);
    expect(f.ctx.isRemoteAttachChoicePending(99)).toBe(false);
  });

  test('既存の保存値（key と target だけ）はそのまま読めて、選択済みの扱いになる', () => {
    const f = fixture({ stored: [{ key: '7|S1', target: 'ubuntu@host' }] });
    expect(f.ctx.remoteAttachTarget(7)).toBe('ubuntu@host');
    expect(f.ctx.isRemoteAttachChoicePending(7)).toBe(false);
  });

  test('「このPCで読む」を選ぶと、記録を消さず local を残す。送り先は空のまま', () => {
    const f = fixture({ stored: [{ key: '9|S1', target: 'other@host' }] });
    f.ctx.setRemoteAttachTarget(7, '');
    expect(f.ctx.isRemoteAttachChoicePending(7)).toBe(false);
    expect(f.ctx.remoteAttachTarget(7)).toBe('');
    expect(f.stored()).toEqual([{ key: '9|S1', target: 'other@host' }, { key: '7|S1', target: '', local: true }]);
  });

  test('ssh 先と「このPCで読む」は 1 セッションに 1 件だけ（切り替えると前の記録は置き換わる）', () => {
    const f = fixture();
    f.ctx.setRemoteAttachTarget(7, 'a@b');
    f.ctx.setRemoteAttachTarget(7, '');
    expect(f.stored()).toEqual([{ key: '7|S1', target: '', local: true }]);
    f.ctx.setRemoteAttachTarget(7, 'c@d');
    expect(f.stored()).toEqual([{ key: '7|S1', target: 'c@d' }]);
    expect(f.ctx.remoteAttachTarget(7)).toBe('c@d');
  });

  test('Hub が再起動して番号が使い回されても（開始時刻が違えば）未選択に戻る', () => {
    const f = fixture();
    f.ctx.setRemoteAttachTarget(7, '');
    f.sessions.get(7)!.started_at = 'S2';
    expect(f.ctx.isRemoteAttachChoicePending(7)).toBe(true);
  });

  test('target も local も無い記録は、選んだことにしない', () => {
    const f = fixture({ stored: [{ key: '7|S1', target: '' }] });
    expect(f.ctx.isRemoteAttachChoicePending(7)).toBe(true);
  });

  test('「このPCで読む」の記録は、最近の送り先の候補に出ない', () => {
    const f = fixture();
    f.ctx.setRemoteAttachTarget(9, 'a@b');
    f.ctx.setRemoteAttachTarget(7, '');
    expect(f.ctx.recentRemoteTargets()).toEqual(['a@b']);
  });

  test('localStorage に書けなくても、このページを開いている間は選んだ結果を覚える（窓が出続けない）', () => {
    const f = fixture({ storageBroken: true });
    f.ctx.setRemoteAttachTarget(7, '');
    expect(f.ctx.isRemoteAttachChoicePending(7)).toBe(false);
    f.ctx.setRemoteAttachTarget(9, 'a@b');
    expect(f.ctx.remoteAttachTarget(9)).toBe('a@b');
    expect(f.ctx.isRemoteAttachChoicePending(7)).toBe(false);
  });
});

describe('添付を送る前に送り先の窓を出す（askRemoteAttachChoice）', () => {
  test('未選択のシェルでは true を返し、今のクリックが終わってから窓を開く。何も記録しない', async () => {
    const f = fixture();
    expect(f.ctx.askRemoteAttachChoice(7, () => {})).toBe(true);
    expect(f.popup()).toBeUndefined();
    await f.tick();
    expect(f.popup().hidden).toBe(false);
    // 説明（送る前に選ぶ理由）が窓に入っている
    expect(f.popup().children.map((c) => c.textContent)).toContain('remote_attach_choose_first');
    // まだ送り先の記録は作らない。ボタンの表示も「未選択」
    expect(f.stored()).toEqual([]);
    expect(f.remoteBtn.textContent).toBe('remote_attach_button_unset');
    // 戻る先が無いので、ボタンは「戻す」ではなく「このPCで読む」
    expect(f.findButton('remote_attach_read_local')).toBeDefined();
  });

  test('Esc で閉じても決めたことにならない。onDecided は呼ばれず、入力欄へ戻る', async () => {
    const f = fixture();
    let decided = 0;
    f.ctx.askRemoteAttachChoice(7, () => { decided++; });
    await f.tick();
    f.inputBox().fire('keydown', { key: 'Escape' });
    expect(f.popup().hidden).toBe(true);
    expect(decided).toBe(0);
    expect(f.ctx.isRemoteAttachChoicePending(7)).toBe(true);
    expect(f.focusLog[f.focusLog.length - 1]).toBe(f.composer);
    // 決めていないので、次に送ろうとすればまた窓が出る
    expect(f.ctx.askRemoteAttachChoice(7, () => {})).toBe(true);
  });

  test('「このPCで読む」を選ぶと onDecided が 1 回だけ呼ばれ、以後は窓を出さない', async () => {
    const f = fixture();
    let decided = 0;
    f.ctx.askRemoteAttachChoice(7, () => { decided++; });
    await f.tick();
    f.findButton('remote_attach_read_local').fire('click');
    expect(decided).toBe(1);
    expect(f.popup().hidden).toBe(true);
    expect(f.ctx.isRemoteAttachChoicePending(7)).toBe(false);
    expect(f.ctx.remoteAttachTarget(7)).toBe('');
    expect(f.stored()).toEqual([{ key: '7|S1', target: '', local: true }]);
    expect(f.remoteBtn.textContent).toBe('remote_attach_button_local');
    expect(f.ctx.askRemoteAttachChoice(7, () => { decided++; })).toBe(false);
    await f.tick();
    expect(f.popup().hidden).toBe(true);
    expect(decided).toBe(1);
  });

  test('ssh 先を保存すると、その送り先が記録されて onDecided が呼ばれる', async () => {
    const f = fixture();
    let decided = 0;
    f.ctx.askRemoteAttachChoice(7, () => { decided++; });
    await f.tick();
    f.inputBox().value = 'ubuntu@cockpit';
    f.findButton('remote_attach_save').fire('click');
    expect(decided).toBe(1);
    expect(f.ctx.remoteAttachTarget(7)).toBe('ubuntu@cockpit');
    expect(f.stored()).toEqual([{ key: '7|S1', target: 'ubuntu@cockpit' }]);
    expect(f.ctx.askRemoteAttachChoice(7, () => {})).toBe(false);
  });

  test('接続先の形が違えば決まらない（窓は開いたまま、onDecided は呼ばれない）', async () => {
    const f = fixture();
    let decided = 0;
    f.ctx.askRemoteAttachChoice(7, () => { decided++; });
    await f.tick();
    f.inputBox().value = 'not a target';
    f.findButton('remote_attach_save').fire('click');
    expect(decided).toBe(0);
    expect(f.popup().hidden).toBe(false);
    expect(f.ctx.isRemoteAttachChoicePending(7)).toBe(true);
  });

  test('シェル以外のセッションと、選択済みのセッションでは窓を出さない（これまでどおり送れる）', async () => {
    const f = fixture();
    f.ctx.activeSessionId = 8;
    expect(f.ctx.askRemoteAttachChoice(8, () => {})).toBe(false);
    f.ctx.activeSessionId = 7;
    f.ctx.setRemoteAttachTarget(7, '');
    expect(f.ctx.askRemoteAttachChoice(7, () => {})).toBe(false);
    await f.tick();
    expect(f.popup()).toBeUndefined();
  });

  test('窓を開くまでの間にセッションを切り替えたら、別のセッションの上に窓を出さない', async () => {
    const f = fixture();
    expect(f.ctx.askRemoteAttachChoice(7, () => {})).toBe(true);
    f.ctx.activeSessionId = 9;
    await f.tick();
    expect(f.popup()).toBeUndefined();
  });

  test('ボタンから開いた窓には「送る前に選ぶ」説明が出ず、決めても何も呼ばない', async () => {
    const f = fixture();
    f.remoteBtn.fire('click');
    expect(f.popup().children.map((c) => c.textContent)).not.toContain('remote_attach_choose_first');
    f.findButton('remote_attach_read_local').fire('click');
    expect(f.ctx.isRemoteAttachChoicePending(7)).toBe(false);
  });
});
