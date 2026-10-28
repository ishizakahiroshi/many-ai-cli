import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

// remote-file-view.ts（ssh 先のファイルを読み取り専用で開く画面）と、それが使う files-view.ts の
// Markdown 変換（renderMarkdown の remote モード）の実コードを、合成 DOM / 合成 fetch で動かす。
// Hub・ブラウザ・web/dist は使わない。DOM が無いので、DOMPurify の実際の除去と scrubRemoteMarkdownDom の
// 属性の走査はここでは動かせない（その分は isAllowedRemoteAttribute の表と、renderMarkdown が DOMPurify へ渡す
// 設定で確かめる。jsdom を使った実物の確認は plan の検証欄に別に書く）。
const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const transpile = (code: string) => new Bun.Transpiler({ loader: 'ts' }).transformSync(code);
const between = (source: string, from: string, to: string) => {
  const start = source.indexOf(from);
  const end = source.indexOf(to, start);
  if (start < 0 || end < 0) throw new Error(`cannot slice ${from} .. ${to}`);
  return source.slice(start, end);
};

class FakeNode {
  children: FakeNode[] = [];
  className = ''; title = ''; src = ''; alt = ''; type = ''; textContent = '';
  dataset: Record<string, string> = {};
  attrs = new Map<string, string>();
  listeners = new Map<string, Function[]>();
  constructor(public tagName: string) {}
  appendChild(node: FakeNode) { this.children.push(node); return node; }
  append(...nodes: FakeNode[]) { this.children.push(...nodes); }
  replaceChildren(...nodes: FakeNode[]) { this.children = [...nodes]; this.textContent = ''; }
  setAttribute(key: string, value: string) { this.attrs.set(key, value); }
  addEventListener(name: string, fn: Function) { this.listeners.set(name, [...(this.listeners.get(name) || []), fn]); }
  querySelectorAll() { return []; }
  removeAttribute() {}
  all(): FakeNode[] { return [this, ...this.children.flatMap((child) => child.all())]; }
}

const encode = (text: string) => new TextEncoder().encode(text);

function fixture() {
  const toasts: Array<{ message: string; ms?: number }> = [];
  const fetches: string[] = [];
  const revoked: string[] = [];
  const rendered: Array<{ text: string; options: any }> = [];
  const state: { onClose: (() => void) | null; cancelled: number; closeCalls: number; body: FakeNode | null; signal: AbortSignal | null } =
    { onClose: null, cancelled: 0, closeCalls: 0, body: null, signal: null };
  let respond: () => Promise<any> = async () => ({ ok: false, status: 500, headers: { get: () => null }, json: async () => ({}) });

  const ctx: any = {
    t: (key: string, vars?: Record<string, unknown>) => (vars ? `${key}|${JSON.stringify(vars)}` : key),
    showToast: (message: string, _anchor?: unknown, ms?: number) => toasts.push({ message, ms }),
    apiFetch: async (url: string, init?: { signal: AbortSignal }) => { fetches.push(url); state.signal = init?.signal || null; return respond(); },
    basenameForPath: (p: string) => String(p).split('/').pop() || '',
    copyPathText: async () => {},
    closeFileModal: () => { state.closeCalls++; const fn = state.onClose; state.onClose = null; if (fn) fn(); },
    openFileModalShell: (onClose: () => void) => {
      state.onClose = onClose;
      state.body = new FakeNode('div');
      return { dialogEl: new FakeNode('div'), body: state.body };
    },
    FilesPreview: { renderMarkdown: (text: string, _base: string, _cb: unknown, options: any) => { rendered.push({ text, options }); return '<p>md</p>'; } },
    bindUrlLinksIn: () => {}, linkifyUrlsInElement: () => {}, openLightbox: () => {},
    document: { createElement: (tag: string) => new FakeNode(tag), importNode: (node: FakeNode) => node, querySelectorAll: () => [] },
    DOMParser: class { parseFromString() { return { body: { firstElementChild: new FakeNode('div') } }; } },
    URL: { createObjectURL: () => 'blob:synthetic-image', revokeObjectURL: (url: string) => revoked.push(url) },
    Blob, TextDecoder, AbortController, encodeURIComponent, Promise, Array, Number, String, JSON, Math,
  };
  ctx.isImagePath = undefined; ctx.isTextPath = undefined;
  // 種類の判定は path-links.ts の実物を使う（拡張子の表を写さない）。
  const links = read('../src/app/path-links.ts');
  runInNewContext(
    transpile(between(links, 'export function isImagePath', '// MANY-AI-CLI 内蔵プレビュー').replace(/^export /gm, ''))
      + '\nglobalThis.isImagePath = isImagePath; globalThis.isTextPath = isTextPath;',
    ctx,
  );
  const source = read('../src/app/remote-file-view.ts').replace(/^import .*;\r?$/gm, '').replace(/^export /gm, '');
  runInNewContext(
    transpile(source) + '\nglobalThis.api = { remoteViewKind, isRemoteViewablePath, isFetchableRemotePath, isAllowedRemoteAttribute, openRemoteFileView, REMOTE_VIEW_MAX_TEXT_BYTES, REMOTE_VIEW_MAX_IMAGE_BYTES };',
    ctx,
  );
  const response = (opts: { ok?: boolean; status?: number; body?: Uint8Array; headers?: Record<string, string>; json?: unknown } = {}) => {
    const bytes = opts.body || new Uint8Array();
    return {
      ok: opts.ok ?? true,
      status: opts.status ?? 200,
      headers: { get: (key: string) => opts.headers?.[key] ?? null },
      json: async () => opts.json ?? {},
      arrayBuffer: async () => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength),
      body: { cancel: async () => { state.cancelled++; } },
    };
  };
  return {
    api: ctx.api, toasts, fetches, revoked, rendered, state, response,
    setResponse(fn: () => Promise<any>) { respond = fn; },
    texts() { return state.body ? state.body.all().map((node) => node.textContent).filter(Boolean) : []; },
    find(predicate: (node: FakeNode) => boolean) { return state.body?.all().find(predicate); },
  };
}

describe('remote-file-view の種類の判定', () => {
  test('PC のプレビューと同じ分け方（.md は Markdown、文字は等幅、画像は画像、それ以外は開かない）', () => {
    const { api } = fixture();
    expect(api.remoteViewKind('/srv/a/report.md')).toBe('markdown');
    expect(api.remoteViewKind('/srv/a/REPORT.MD')).toBe('markdown');
    expect(api.remoteViewKind('/srv/a/notes.txt')).toBe('text');
    expect(api.remoteViewKind('/srv/a/Dockerfile')).toBe('text');
    expect(api.remoteViewKind('/srv/a/out.PNG')).toBe('image');
    expect(api.remoteViewKind('/srv/a/photo.jpeg')).toBe('image');
    expect(api.remoteViewKind('/srv/a/movie.mp4')).toBeNull();
    expect(api.remoteViewKind('/srv/a/tool.exe')).toBeNull();
    expect(api.remoteViewKind('/etc/hosts')).toBeNull();
    expect(api.isRemoteViewablePath('/srv/a/report.md')).toBe(true);
    expect(api.isRemoteViewablePath('/srv/a/movie.mp4')).toBe(false);
  });
});

describe('送り先のパスの規則（「取ってくる」と「開く」で共有）', () => {
  test('/ か ~/ で始まるファイルのパスだけ受ける', () => {
    const { api } = fixture();
    for (const ok of ['/srv/a.md', '~/out/a.png', '/a', '~/a']) expect(api.isFetchableRemotePath(ok)).toBe(true);
    for (const ng of ['', 'docs/a.md', './a.md', '/', '~/', '~', '/srv/dir/', 'C:\\a.txt', '/srv/a\u0000.md', '/srv/a\n.md', `/${'a'.repeat(4096)}`]) {
      expect(api.isFetchableRemotePath(ng)).toBe(false);
    }
  });
});

describe('端末のパスのメニュー（attachments.ts の remoteFetchMenuItems）', () => {
  // attachments.ts の実コード（開く・取ってくるの項目を作る部分）だけを取り出して動かす。
  function menu() {
    const f = fixture();
    const calls: Array<{ fn: string; args: unknown[] }> = [];
    const ctx: any = {
      t: (key: string, vars?: Record<string, unknown>) => (vars ? `${key}|${JSON.stringify(vars)}` : key),
      remoteAttachTarget: (id: number) => (id === 7 ? 'dev@host' : ''),
      isFetchableRemotePath: f.api.isFetchableRemotePath,
      isRemoteViewablePath: f.api.isRemoteViewablePath,
      openRemoteFileView: async (...args: unknown[]) => { calls.push({ fn: 'open', args }); return true; },
      fetchRemoteFile: async (...args: unknown[]) => { calls.push({ fn: 'fetch', args }); return true; },
      Promise,
    };
    const attachments = read('../src/app/attachments.ts');
    const code = between(attachments, '/**\n * ssh 先のファイルを、ダウンロードせずに', 'function openRemoteFetchPopup').replace(/^export /gm, '');
    runInNewContext(`${transpile(code)}\nglobalThis.remoteFetchMenuItems = remoteFetchMenuItems;`, ctx);
    return { items: ctx.remoteFetchMenuItems as (id: number | null, path: string) => any[], calls };
  }

  test('開ける種類なら「開く」を先に、「取ってくる」を後に出す。開くは onFetch 付きで画面を呼ぶ', () => {
    const { items, calls } = menu();
    const list = items(7, '/srv/a.md');
    expect(list.map((item) => item.label.split('|')[0])).toEqual(['remote_view_menu', 'remote_fetch_menu']);
    list[0].action();
    expect(calls[0].fn).toBe('open');
    expect(calls[0].args.slice(0, 3)).toEqual([7, 'dev@host', '/srv/a.md']);
    (calls[0].args[3] as { onFetch: () => void }).onFetch();
    expect(calls[1]).toEqual({ fn: 'fetch', args: [7, '/srv/a.md'] });
    list[1].action();
    expect(calls[2]).toEqual({ fn: 'fetch', args: [7, '/srv/a.md'] });
  });

  test('開けない種類は「取ってくる」だけ。送り先が無い・相対パス・セッションなしは何も出さない', () => {
    const { items } = menu();
    expect(items(7, '/srv/data.zip').map((item) => item.label.split('|')[0])).toEqual(['remote_fetch_menu']);
    expect(items(8, '/srv/a.md')).toEqual([]);
    expect(items(7, 'docs/a.md')).toEqual([]);
    expect(items(null, '/srv/a.md')).toEqual([]);
  });
});

describe('ssh 先の Markdown に残してよい属性', () => {
  test('PC のファイルとして開かれうる URL・押すと PC のパスを開く印・on* は残さない', () => {
    const { api } = fixture();
    const ok = (tag: string, name: string, value: string) => api.isAllowedRemoteAttribute(tag, name, value);
    // 残してよいもの
    expect(ok('a', 'href', 'https://example.com/a')).toBe(true);
    expect(ok('a', 'href', 'HTTP://example.com/a')).toBe(true);
    expect(ok('img', 'src', 'data:image/png;base64,iVBORw0KGgo=')).toBe(true);
    expect(ok('p', 'class', 'x')).toBe(true);
    expect(ok('img', 'alt', '/api/files-asset?path=x')).toBe(true); // alt は文字でしかない
    // 残さないもの
    for (const value of ['docs/other.md', './other.md', '../x', '/etc/passwd', '/api/files-content?path=C:/s.txt', 'C:\\work\\s.txt', 'file:///C:/x', '#intro', 'javascript:alert(1)', 'mailto:a@example.com', '']) {
      expect(ok('a', 'href', value)).toBe(false);
    }
    expect(ok('img', 'src', '/api/files-asset?path=C%3A%2Fs.png')).toBe(false);
    expect(ok('img', 'src', 'images/a.png')).toBe(false);
    expect(ok('img', 'src', 'https://example.com/a.png')).toBe(false);
    expect(ok('img', 'src', 'data:image/svg+xml;base64,PHN2Zz4=')).toBe(false);
    expect(ok('img', 'srcset', 'https://example.com/a.png 2x')).toBe(false);
    expect(ok('video', 'src', 'https://example.com/a.mp4')).toBe(false);
    expect(ok('video', 'poster', 'https://example.com/a.png')).toBe(false);
    expect(ok('a', 'data-files-path-link', 'C:\\secret.txt')).toBe(false);
    expect(ok('a', 'data-files-rel-link', 'other.md')).toBe(false);
    expect(ok('a', 'onclick', 'x()')).toBe(false);
    expect(ok('use', 'xlink:href', '/api/x#a')).toBe(false);
    expect(ok('A', 'HREF', '/etc/passwd')).toBe(false);
  });
});

describe('FilesPreview.renderMarkdown の remote モード', () => {
  // files-view.ts の renderMarkdown の実コードを、本物の marked と DOMPurify の代役（渡された設定を記録）で動かす。
  function renderer() {
    const files = read('../src/app/files-view.ts');
    const code = transpile(between(files, '  let markedBaseConfigured = false;', '  /**\n   * 各 <pre> ブロックを wrapper'));
    const sanitizeConfigs: any[] = [];
    const ctx: any = {
      escapeHtml: (s: unknown) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;'),
      findUrlCandidates: () => [],
      DOMPurify: { isSupported: true, sanitize: (html: string, config: any) => { sanitizeConfigs.push(config); return html; } },
      hljs: undefined, Array, Object, String, RegExp, JSON,
    };
    runInNewContext(read('../src/vendor/marked.min.js'), ctx); // UMD: ctx.marked を定義する
    runInNewContext(`${code}\nglobalThis.renderMarkdown = renderMarkdown;`, ctx);
    return { render: ctx.renderMarkdown as (...args: unknown[]) => string, sanitizeConfigs };
  }
  const markdown = [
    '[web](https://example.com/a)',
    '[rel](docs/other.md)',
    '[abs](/etc/passwd)',
    '[win](C:\\work\\secret.txt)',
    '[file](file:///C:/work/secret.txt)',
    '[hub](/api/files-content?path=C:/secret.txt)',
    '![rel img](images/a.png)',
    '![hub img](/api/files-asset?path=C%3A%2Fsecret.png)',
    '![data img](data:image/png;base64,iVBORw0KGgo=)',
  ].join('\n\n');

  test('http(s) 以外のリンク・画像は、PC のファイルを開く形（<a data-files-*> / <img src>）にならない', () => {
    const { render } = renderer();
    const html = render(markdown, '', null, { remote: true });
    expect(html).toContain('<a href="https://example.com/a"');
    expect(html).not.toContain('data-files-rel-link');
    expect(html).not.toContain('data-files-path-link');
    expect(html).not.toMatch(/<a [^>]*href="(?!https?:\/\/)/); // http(s) 以外を指す <a> は無い
    expect(html).not.toMatch(/<img [^>]*src="(?!data:image\/)/); // data:image 以外の <img> は無い
    expect(html).not.toMatch(/(?:src|href)="\/api\//); // PC の Hub の API を指す属性は無い（title の文字として出るのは URL ではない）
    expect(html).toContain('<img src="data:image/png;base64,iVBORw0KGgo=" alt="data img">');
    // 文字は残る（読めなくならない）
    expect(html).toContain('>rel</span>');
    expect(html).toContain('rel img');
  });

  test('DOMPurify には http(s) だけを通す設定を渡し、動画・埋め込みなども禁止する', () => {
    const { render, sanitizeConfigs } = renderer();
    render('x', '', null, { remote: true });
    const config = sanitizeConfigs[0];
    const allowed = (url: string) => config.ALLOWED_URI_REGEXP.test(url);
    expect(allowed('https://example.com/')).toBe(true);
    for (const url of ['/api/files-content?path=x', 'docs/a.md', 'C:\\x', 'file:///C:/x', '#a', 'javascript:alert(1)']) expect(allowed(url)).toBe(false);
    for (const tag of ['video', 'audio', 'source', 'iframe', 'object', 'embed', 'svg', 'form', 'style']) expect(config.FORBID_TAGS).toContain(tag);
    for (const attr of ['srcset', 'poster', 'style']) expect(config.FORBID_ATTR).toContain(attr);
    expect(config.ADD_ATTR).not.toContain('data-files-path-link');
    expect(config.ADD_ATTR).not.toContain('data-files-rel-link');
  });

  test('通常のモード（PC のファイル）は今までと同じ: 相対リンクは data-files-rel-link、絶対パスは data-files-path-link', () => {
    const { render, sanitizeConfigs } = renderer();
    const html = render('[rel](a.md) [abs](/x/y.png) [web](https://example.com/)\n\n![i](images/a.png)', '', null);
    expect(html).toContain('<a href="#" data-files-rel-link="a.md" class="files-md-link">rel</a>');
    expect(html).toContain('<a href="#" data-files-path-link="/x/y.png" class="files-md-link">abs</a>');
    expect(html).toContain('<img src="images/a.png" alt="i">');
    expect(sanitizeConfigs[0].ALLOWED_URI_REGEXP).toBeUndefined();
    expect(sanitizeConfigs[0].ADD_ATTR).toContain('data-files-path-link');
  });
});

describe('openRemoteFileView', () => {
  test('送り先が無い・パスが / か ~/ で始まらない・開けない種類のときは、通信もモーダルも出さない', async () => {
    const f = fixture();
    expect(await f.api.openRemoteFileView(7, '', '/srv/a.md')).toBe(false);
    expect(await f.api.openRemoteFileView(7, 'dev@host', 'docs/a.md')).toBe(false);
    expect(await f.api.openRemoteFileView(7, 'dev@host', '/srv/tool.exe')).toBe(false);
    expect(f.toasts.map((toast) => toast.message.split('|')[0])).toEqual(['remote_fetch_no_target', 'remote_fetch_invalid', 'remote_view_unsupported']);
    expect(f.fetches).toEqual([]);
    expect(f.state.body).toBeNull();
  });

  test('Hub が失敗を返したら detail をトーストに出してモーダルを閉じる。PC のファイルを読む API は叩かない', async () => {
    const f = fixture();
    f.setResponse(async () => f.response({ ok: false, status: 404, json: { detail: 'no such file: /srv/a.md' } }));
    expect(await f.api.openRemoteFileView(7, 'dev@host', '/srv/a.md')).toBe(false);
    expect(f.fetches).toEqual(['/api/files-download?session=7&remote_target=dev%40host&path=%2Fsrv%2Fa.md']);
    expect(f.toasts).toHaveLength(1);
    expect(f.toasts[0].message).toBe('remote_view_failed|{"target":"dev@host","detail":"no such file: /srv/a.md"}');
    expect(f.state.closeCalls).toBe(1);
  });

  test('文字のファイルは等幅のまま文字として入れる（HTML として解釈しない）', async () => {
    const f = fixture();
    const text = '<script>alert(1)</script>\n[x](/etc/passwd)';
    f.setResponse(async () => f.response({ body: encode(text) }));
    expect(await f.api.openRemoteFileView(7, 'dev@host', '/srv/log.txt')).toBe(true);
    const pre = f.find((node) => node.tagName === 'pre');
    expect(pre?.textContent).toBe(text);
    expect(f.rendered).toEqual([]); // Markdown の変換は通らない
    expect(f.find((node) => node.tagName === 'a')).toBeUndefined();
  });

  test('.md は remote モードの変換を通す', async () => {
    const f = fixture();
    f.setResponse(async () => f.response({ body: encode('[abs](/etc/passwd)') }));
    expect(await f.api.openRemoteFileView(7, 'dev@host', '/srv/README.md')).toBe(true);
    expect(f.rendered).toEqual([{ text: '[abs](/etc/passwd)', options: { remote: true } }]);
  });

  test('大きすぎるものは描かず「取ってくる」を案内する。Content-Length で分かれば本文は読まない', async () => {
    const f = fixture();
    f.setResponse(async () => f.response({ headers: { 'Content-Length': String(f.api.REMOTE_VIEW_MAX_TEXT_BYTES + 1) } }));
    expect(await f.api.openRemoteFileView(7, 'dev@host', '/srv/big.log')).toBe(false);
    expect(f.state.cancelled).toBe(1);
    expect(f.texts().join('\n')).toContain('remote_view_too_large');
    expect(f.state.closeCalls).toBe(0);
  });

  test('文字として読めない中身（UTF-8 でない・NUL を含む）は描かず案内する', async () => {
    const f = fixture();
    f.setResponse(async () => f.response({ body: new Uint8Array([0x83, 0x65, 0x83, 0x58, 0x83, 0x67]) })); // Shift_JIS の「テスト」
    expect(await f.api.openRemoteFileView(7, 'dev@host', '/srv/sjis.txt')).toBe(false);
    expect(f.texts().join('\n')).toContain('remote_view_unreadable');
    f.setResponse(async () => f.response({ body: encode('abc\u0000def') }));
    expect(await f.api.openRemoteFileView(7, 'dev@host', '/srv/bin.txt')).toBe(false);
    expect(f.texts().join('\n')).toContain('remote_view_unreadable');
  });

  test('読み込み中に閉じたら通信を止め、あとから届いた中身は描かない', async () => {
    const f = fixture();
    let release: (value: unknown) => void = () => {};
    f.setResponse(() => new Promise((resolve) => { release = resolve; }));
    const opened = f.api.openRemoteFileView(7, 'dev@host', '/srv/a.md');
    await Promise.resolve();
    f.state.onClose?.(); // Esc・背景クリック・ほかのモーダルが開いたとき
    expect(f.state.signal?.aborted).toBe(true);
    release(f.response({ body: encode('late') }));
    expect(await opened).toBe(false);
    expect(f.rendered).toEqual([]);
  });

  test('画像は Blob URL で出し、閉じたら解放する', async () => {
    const f = fixture();
    f.setResponse(async () => f.response({ body: new Uint8Array([0x89, 0x50, 0x4e, 0x47]) }));
    expect(await f.api.openRemoteFileView(7, 'dev@host', '/srv/out.png')).toBe(true);
    const image = f.find((node) => node.tagName === 'img');
    expect(image?.src).toBe('blob:synthetic-image');
    expect(f.fetches[0]).not.toContain('files-asset');
    f.state.onClose?.();
    expect(f.revoked).toEqual(['blob:synthetic-image']);
  });
});
