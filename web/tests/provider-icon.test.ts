import { afterEach, describe, expect, test } from 'bun:test';
import {
  applyProviderPresentation,
  ensureProviderIconSprite,
  fillProviderIconSlots,
  isValidIconColor,
  isValidIconText,
  providerDefaultIconColor,
  providerDisplayName,
  providerIconHtml,
  providerIconImageUrl,
  providerIconLoadedImageHref,
  providerIconPreviewHtml,
  providerIconSymbolId,
  safeClassToken,
  setProviderIconImageProbe,
} from '../src/app/provider-icon';

const BUNDLED = [
  'claude', 'codex', 'copilot', 'cursor-agent', 'ollama', 'lm-studio', 'opencode', 'grok', 'command-code',
];

// 最小の DOM もどき。スプライトの作成と symbol の追加だけを見る（実ブラウザの描画は見ない）。
function installFakeDocument() {
  const state = { spriteHtml: '', appended: [] as string[], connected: true, inserts: 0 };
  const sprite: any = {
    get isConnected() { return state.connected; },
    insertAdjacentHTML(_pos: string, html: string) { state.appended.push(html); },
  };
  const body: any = {
    insertAdjacentHTML(_pos: string, html: string) { state.spriteHtml = html; state.inserts++; },
  };
  const slots: any[] = [];
  // :root の CSS 変数（--prov-*）を模す。inline style で上書きした値と、styles.css の既定値を分けて持つ。
  const cssDefaults: Record<string, string> = { '--prov-claude': '#fb923c', '--prov-codex': '#60a5fa' };
  const props = new Map<string, string>();
  const style = {
    props,
    setProperty(name: string, value: string) { props.set(name, value); },
    removeProperty(name: string) { props.delete(name); },
  };
  const symbolEls = new Map<string, { innerHTML: string }>();
  const doc: any = {
    body,
    documentElement: { style },
    getElementById(id: string) {
      if (!state.spriteHtml) return null;
      if (id === 'aac-provider-icon-sprite') return sprite;
      const marker = `<symbol id="${id}"`;
      if (!state.spriteHtml.includes(marker) && !state.appended.some((html) => html.includes(marker))) return null;
      if (!symbolEls.has(id)) symbolEls.set(id, { innerHTML: '' });
      return symbolEls.get(id);
    },
    querySelectorAll(sel: string) {
      return { forEach(fn: (el: any) => void) { if (sel === '[data-provider-icon]') slots.forEach(fn); } };
    },
  };
  (globalThis as any).document = doc;
  (globalThis as any).getComputedStyle = () => ({
    getPropertyValue(name: string) { return props.get(name) ?? cssDefaults[name] ?? ''; },
  });
  current = state;
  return { state, slots, doc, style, symbolEls };
}

// モジュールはスプライト要素を覚えている。テストが終わったら切り離し扱いにして、次のテストで作り直させる。
let current: { connected: boolean } | null = null;
afterEach(() => {
  // 利用者設定はモジュールが覚えているので、document が残っているうちに全部外す。
  applyProviderPresentation([]);
  setProviderIconImageProbe(null);
  if (current) current.connected = false;
  current = null;
  delete (globalThis as any).document;
  delete (globalThis as any).getComputedStyle;
});

describe('safeClassToken', () => {
  test('lowercases and replaces unsafe characters', () => {
    expect(safeClassToken('Claude')).toBe('claude');
    expect(safeClassToken('my ai/../x')).toBe('my-ai-x');
    expect(safeClassToken('"><script>')).toBe('script');
  });

  test('empty input becomes unknown and length is capped', () => {
    expect(safeClassToken('')).toBe('unknown');
    expect(safeClassToken(null)).toBe('unknown');
    expect(safeClassToken('***')).toBe('unknown');
    expect(safeClassToken('a'.repeat(100))).toHaveLength(64);
  });
});

describe('providerDisplayName', () => {
  test('bundled ids map to the same labels as the session card', () => {
    expect(providerDisplayName('claude')).toBe('Claude');
    expect(providerDisplayName('Grok')).toBe('Grok Build');
    expect(providerDisplayName('cursor-agent')).toBe('Cursor Agent');
    expect(providerDisplayName('lm-studio')).toBe('LM Studio');
  });

  test('unknown ids are returned as given and empty input is empty', () => {
    expect(providerDisplayName('my-ai')).toBe('my-ai');
    expect(providerDisplayName('constructor')).toBe('constructor');
    expect(providerDisplayName(undefined)).toBe('');
  });
});

describe('providerIconHtml', () => {
  test('bundled providers reference their own symbol', () => {
    for (const id of BUNDLED) {
      const html = providerIconHtml(id);
      expect(html).toContain(`<use href="#aac-prov-${id}"/>`);
      expect(html).toContain('class="card-provider-icon"');
      expect(html).toContain('viewBox="0 0 16 16"');
      expect(providerIconSymbolId(id)).toBe(`aac-prov-${id}`);
    }
  });

  test('id case does not matter', () => {
    expect(providerIconHtml('Claude')).toBe(providerIconHtml('claude'));
  });

  test('unknown and hostile ids become a safe token', () => {
    const html = providerIconHtml('"><img src=x onerror=alert(1)>');
    expect(html).not.toContain('<img');
    expect(html).not.toContain('onerror="');
    expect(html).toContain('<use href="#aac-prov-img-src-x-onerror-alert-1"/>');
    expect(providerIconHtml('')).toContain('<use href="#aac-prov-unknown"/>');
    expect(providerIconHtml(undefined)).toContain('<use href="#aac-prov-unknown"/>');
  });

  test('size is rounded into 1..64 and falls back to 16', () => {
    expect(providerIconHtml('claude', 14)).toContain('width="14" height="14"');
    expect(providerIconHtml('claude', 13.9)).toContain('width="13" height="13"');
    expect(providerIconHtml('claude', 500)).toContain('width="64" height="64"');
    expect(providerIconHtml('claude', 0)).toContain('width="16" height="16"');
    expect(providerIconHtml('claude', -3)).toContain('width="16" height="16"');
    expect(providerIconHtml('claude', Number.NaN)).toContain('width="16" height="16"');
    expect(providerIconHtml('claude')).toContain('width="16" height="16"');
  });

  test('does not touch document when there is none', () => {
    expect(() => providerIconHtml('claude')).not.toThrow();
    expect(() => ensureProviderIconSprite()).not.toThrow();
    expect(() => fillProviderIconSlots()).not.toThrow();
  });
});

describe('sprite', () => {
  test('is created once with a symbol for every bundled provider', () => {
    const { state } = installFakeDocument();
    providerIconHtml('claude');
    providerIconHtml('codex');
    ensureProviderIconSprite();
    expect(state.inserts).toBe(1);
    for (const id of BUNDLED) {
      expect(state.spriteHtml).toContain(`<symbol id="aac-prov-${id}"`);
    }
    // 頭文字・図形・色は今までと同じ値（見た目を変えない）
    expect(state.spriteHtml).toMatch(/<symbol id="aac-prov-claude"[^>]*><circle[^>]*r="6"[^>]*\/><text[^>]*>C<\/text>/);
    expect(state.spriteHtml).toMatch(/<symbol id="aac-prov-codex"[^>]*><circle[\s\S]*?>X<\/text>/);
    expect(state.spriteHtml).toMatch(/<symbol id="aac-prov-cursor-agent"[^>]*><circle[\s\S]*?>r<\/text>/);
    expect(state.spriteHtml).toMatch(/<symbol id="aac-prov-ollama"[^>]*><rect[^>]*rx="3"[^>]*\/><text[^>]*>O<\/text>/);
    expect(state.spriteHtml).toMatch(/<symbol id="aac-prov-lm-studio"[^>]*><rect[\s\S]*?>L<\/text>/);
    expect(state.spriteHtml).toContain('var(--prov-claude,#6b7280)');
    expect(state.spriteHtml).toContain('class="prov-shape claude"');
    expect(state.spriteHtml).toContain('class="prov-letter claude"');
    // display:none にはしない
    expect(state.spriteHtml).not.toContain('display:none');
    expect(state.spriteHtml).toContain('aria-hidden="true"');
  });

  test('unknown provider gets a symbol with its first letter, added once', () => {
    const { state } = installFakeDocument();
    providerIconHtml('claude');
    providerIconHtml('zeta-ai');
    providerIconHtml('zeta-ai');
    expect(state.appended).toHaveLength(1);
    expect(state.appended[0]).toContain('<symbol id="aac-prov-zeta-ai"');
    expect(state.appended[0]).toContain('>Z</text>');
    // 同梱の頭文字クラスは付けない
    expect(state.appended[0]).toContain('class="prov-shape"');
    expect(state.appended[0]).toContain('var(--prov-zeta-ai,#6b7280)');
  });

  test('a letter that needs escaping is escaped', () => {
    const { state } = installFakeDocument();
    providerIconHtml('claude');
    providerIconHtml('<b>');
    // token は b になるが、頭文字は raw の先頭（<）を HTML エスケープして入れる
    expect(state.appended[0]).toContain('&lt;');
    expect(state.appended[0]).not.toContain('><</text>');
  });
});

describe('fillProviderIconSlots', () => {
  test('fills every slot with the matching icon and is idempotent', () => {
    const { slots } = installFakeDocument();
    slots.push({ dataset: { providerIcon: 'claude' }, innerHTML: '' });
    slots.push({ dataset: { providerIcon: 'lm-studio', providerIconSize: '12' }, innerHTML: '' });
    slots.push({ dataset: {}, innerHTML: 'keep' });
    fillProviderIconSlots();
    fillProviderIconSlots();
    expect(slots[0].innerHTML).toBe(providerIconHtml('claude', 16));
    expect(slots[1].innerHTML).toBe(providerIconHtml('lm-studio', 12));
    expect(slots[2].innerHTML).toBe('keep');
  });
});

describe('isValidIconColor / isValidIconText', () => {
  test('color accepts only #RRGGBB', () => {
    for (const ok of ['#000000', '#FFFFFF', '#1e90ff', '#1E90Ff']) expect(isValidIconColor(ok)).toBe(true);
    for (const bad of ['', 'red', '#fff', '#12345g', '#1234567', 'red;}', '#123456;x', ' #123456', 'rgb(0,0,0)', 'var(--x)', null, undefined, 12]) {
      expect(isValidIconColor(bad)).toBe(false);
    }
  });

  test('letters are 1 or 2 visible characters with no surrounding whitespace or control characters', () => {
    for (const ok of ['A', 'AB', 'あ', 'Gé', 'e\u0301', '\u{1F1EF}\u{1F1F5}', '\u{1F468}\u200D\u{1F469}\u200D\u{1F467}']) {
      expect(isValidIconText(ok)).toBe(true);
    }
    for (const bad of ['', ' ', ' A', 'A ', 'ABC', 'A\nB', 'A\u0000', '\u0007', 'x'.repeat(65), null, undefined, 7]) {
      expect(isValidIconText(bad)).toBe(false);
    }
  });
});

describe('applyProviderPresentation', () => {
  test('a saved color and letters change the shared CSS variable and symbol without touching the markup users hold', () => {
    const { style, symbolEls, state } = installFakeDocument();
    const before = providerIconHtml('claude', 14);
    applyProviderPresentation([{ id: 'claude', presentation: { icon_text: 'A', color: '#1E90FF' } }]);
    expect(style.props.get('--prov-claude')).toBe('#1E90FF');
    expect(symbolEls.get('aac-prov-claude')?.innerHTML).toContain('>A</text>');
    expect(symbolEls.get('aac-prov-claude')?.innerHTML).toContain('var(--prov-claude,#6b7280)');
    // 描画済みの <use> は symbol の参照のまま。再描画しなくても変わる作りであることを固定する。
    expect(providerIconHtml('claude', 14)).toBe(before);
    expect(state.inserts).toBe(1);
  });

  test('applying the same list again changes nothing more', () => {
    const { style, symbolEls } = installFakeDocument();
    providerIconHtml('claude');
    const list = [{ id: 'claude', presentation: { icon_text: 'A', color: '#1E90FF' } }];
    applyProviderPresentation(list);
    const firstHtml = symbolEls.get('aac-prov-claude')?.innerHTML;
    applyProviderPresentation(list);
    expect(symbolEls.get('aac-prov-claude')?.innerHTML).toBe(firstHtml);
    expect(style.props.get('--prov-claude')).toBe('#1E90FF');
  });

  test('dropping the presentation puts the shipped letter and color back', () => {
    const { style, symbolEls, state } = installFakeDocument();
    providerIconHtml('claude');
    applyProviderPresentation([{ id: 'claude', presentation: { icon_text: 'A', color: '#1E90FF' } }]);
    applyProviderPresentation([{ id: 'claude' }]);
    expect(style.props.has('--prov-claude')).toBe(false);
    expect(symbolEls.get('aac-prov-claude')?.innerHTML).toContain('>C</text>');
    expect(state.spriteHtml).toContain('>C</text>');
  });

  test('a provider that disappears from the list goes back to the default too', () => {
    const { style } = installFakeDocument();
    applyProviderPresentation([{ id: 'zeta', presentation: { color: '#00FF00' } }]);
    expect(style.props.get('--prov-zeta')).toBe('#00FF00');
    applyProviderPresentation([{ id: 'claude' }]);
    expect(style.props.has('--prov-zeta')).toBe(false);
  });

  test('values that fail the check are ignored, so no CSS reaches the page', () => {
    const { style, symbolEls } = installFakeDocument();
    providerIconHtml('claude');
    applyProviderPresentation([
      { id: 'claude', presentation: { icon_text: 'ABC', color: 'red;}' } },
      { id: 'codex', presentation: { icon_text: ' X', color: 'url(x)' } },
      { id: 'grok', presentation: null },
      { id: 42, presentation: { color: '#000000' } },
      null as any,
    ]);
    expect(style.props.size).toBe(0);
    expect(symbolEls.get('aac-prov-claude')).toBeUndefined();
  });

  test('letters set before a custom provider is first drawn are used when its symbol is created', () => {
    const { state } = installFakeDocument();
    providerIconHtml('claude');
    applyProviderPresentation([{ id: 'omega-ai', presentation: { icon_text: 'ZZ' } }]);
    providerIconHtml('omega-ai');
    expect(state.appended).toHaveLength(1);
    expect(state.appended[0]).toContain('<symbol id="aac-prov-omega-ai"');
    expect(state.appended[0]).toContain('>ZZ</text>');
    expect(state.appended[0]).toContain('font-size="6"');
    expect(state.appended[0]).toContain('class="prov-shape"');
  });

  test('the shape never changes: a bundled rounded provider stays a rect, a circle stays a circle', () => {
    const { symbolEls } = installFakeDocument();
    providerIconHtml('ollama');
    providerIconHtml('claude');
    applyProviderPresentation([
      { id: 'ollama', presentation: { icon_text: 'Q', color: '#010203' } },
      { id: 'claude', presentation: { icon_text: 'Q', color: '#010203' } },
    ]);
    expect(symbolEls.get('aac-prov-ollama')?.innerHTML).toContain('<rect');
    expect(symbolEls.get('aac-prov-claude')?.innerHTML).toContain('<circle');
  });

  test('does nothing harmful without a document', () => {
    expect(() => applyProviderPresentation([{ id: 'claude', presentation: { color: '#1E90FF', icon_text: 'A' } }])).not.toThrow();
    applyProviderPresentation([]);
  });
});

describe('providerDefaultIconColor / providerIconPreviewHtml', () => {
  test('the default color is the stylesheet value even after a user color is applied', () => {
    installFakeDocument();
    expect(providerDefaultIconColor('claude')).toBe('#fb923c');
    applyProviderPresentation([{ id: 'claude', presentation: { color: '#1E90FF' } }]);
    expect(providerDefaultIconColor('claude')).toBe('#fb923c');
    applyProviderPresentation([]);
    expect(providerDefaultIconColor('claude')).toBe('#fb923c');
    // 既定の色を持たない AI は灰色
    expect(providerDefaultIconColor('zeta')).toBe('#6b7280');
  });

  test('the preview draws the typed letters and color directly, not through the shared symbol', () => {
    installFakeDocument();
    const html = providerIconPreviewHtml('claude', { icon_text: 'AB', color: '#1E90FF' }, 32);
    expect(html).not.toContain('<use');
    expect(html).toContain('width="32" height="32"');
    expect(html).toContain('>AB</text>');
    expect(html).toContain('#1E90FF');
    expect(html).not.toContain('var(--prov');
    expect(html).toContain('<circle');
  });

  test('missing or invalid draft values fall back to the shipped icon; a hostile letter is escaped', () => {
    installFakeDocument();
    const fallback = providerIconPreviewHtml('claude', { icon_text: 'ABC', color: 'red;}' });
    expect(fallback).toContain('>C</text>');
    expect(fallback).toContain('#fb923c');
    expect(fallback).not.toContain('red;}');
    const escaped = providerIconPreviewHtml('claude', { icon_text: '<' });
    expect(escaped).toContain('&lt;');
    expect(escaped).not.toContain('><</text>');
    expect(providerIconPreviewHtml('ollama', {})).toContain('<rect');
  });
});

// plan_provider-icon-single-source.md C4: a picture chosen as the icon.
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

function deferredProbe() {
  const calls: { href: string; settle: (ok: boolean) => void }[] = [];
  const probe = (href: string) => new Promise<boolean>((settle) => { calls.push({ href, settle }); });
  return { calls, probe };
}

describe('providerIconImageUrl', () => {
  test('only an id the Hub can store and a plain version token become a URL', () => {
    expect(providerIconImageUrl('claude', '1a2b-3c')).toBe('/api/provider-icons/claude?v=1a2b-3c');
    expect(providerIconImageUrl('cursor-agent', 'A_b-9')).toBe('/api/provider-icons/cursor-agent?v=A_b-9');
    for (const bad of ['', 'Claude', 'a/b', '../x', 'my.ai', 'a b', 'x'.repeat(65), null, undefined, 7]) {
      expect(providerIconImageUrl(bad, '1')).toBe('');
    }
    for (const bad of ['', 'a b', '1"><script>', '1&x', '../1', 'v'.repeat(65), null, undefined, 7]) {
      expect(providerIconImageUrl('claude', bad)).toBe('');
    }
  });
});

describe('icon image in the shared symbol', () => {
  test('after the image loads, the symbol shows it clipped to the shape with the ring kept, and no letter', async () => {
    const { symbolEls } = installFakeDocument();
    providerIconHtml('claude');
    setProviderIconImageProbe(async () => true);
    applyProviderPresentation([{ id: 'claude', icon_image_version: '1a-2b', presentation: { color: '#1E90FF', icon_text: 'A' } }]);
    await flush();
    const inner = symbolEls.get('aac-prov-claude')?.innerHTML ?? '';
    expect(inner).toContain('<clipPath id="aac-provclip-claude"><circle cx="8" cy="8" r="6"/></clipPath>');
    expect(inner).toContain('<image href="/api/provider-icons/claude?v=1a-2b"');
    expect(inner).toContain('clip-path="url(#aac-provclip-claude)"');
    expect(inner).toContain('preserveAspectRatio="xMidYMid slice"');
    // ring in the user's color stays; the letter is not drawn while the image is
    expect(inner).toContain('fill:none;stroke:var(--prov-claude,#6b7280)');
    expect(inner).not.toContain('<text');
    expect(providerIconLoadedImageHref('claude')).toBe('/api/provider-icons/claude?v=1a-2b');
    // the markup handed out to screens is still just a <use>, so nothing needs redrawing
    expect(providerIconHtml('claude')).toContain('<use href="#aac-prov-claude"/>');
  });

  test('a rounded provider keeps its rounded clip', async () => {
    const { symbolEls } = installFakeDocument();
    providerIconHtml('ollama');
    setProviderIconImageProbe(async () => true);
    applyProviderPresentation([{ id: 'ollama', icon_image_version: '1' }]);
    await flush();
    const inner = symbolEls.get('aac-prov-ollama')?.innerHTML ?? '';
    expect(inner).toContain('<clipPath id="aac-provclip-ollama"><rect');
    expect(inner).toContain('rx="3"');
    expect(inner).not.toContain('<circle');
  });

  test('until the image has loaded the letters stay; if it cannot be loaded they stay for good', async () => {
    const { symbolEls } = installFakeDocument();
    providerIconHtml('claude');
    const { calls, probe } = deferredProbe();
    setProviderIconImageProbe(probe);
    applyProviderPresentation([{ id: 'claude', icon_image_version: '1' }]);
    expect(calls).toHaveLength(1);
    expect(symbolEls.get('aac-prov-claude')).toBeUndefined();
    calls[0].settle(false);
    await flush();
    expect(providerIconLoadedImageHref('claude')).toBe('');
    expect(symbolEls.get('aac-prov-claude')).toBeUndefined();
    // the same list again does not retry a version that already failed
    applyProviderPresentation([{ id: 'claude', icon_image_version: '1' }]);
    expect(calls).toHaveLength(1);
  });

  test('taking the image away puts the letters and the user color back', async () => {
    const { symbolEls, style } = installFakeDocument();
    providerIconHtml('claude');
    setProviderIconImageProbe(async () => true);
    applyProviderPresentation([{ id: 'claude', icon_image_version: '1', presentation: { icon_text: 'A', color: '#1E90FF' } }]);
    await flush();
    expect(symbolEls.get('aac-prov-claude')?.innerHTML).toContain('<image');
    applyProviderPresentation([{ id: 'claude', presentation: { icon_text: 'A', color: '#1E90FF' } }]);
    const inner = symbolEls.get('aac-prov-claude')?.innerHTML ?? '';
    expect(inner).not.toContain('<image');
    expect(inner).toContain('>A</text>');
    expect(style.props.get('--prov-claude')).toBe('#1E90FF');
    expect(providerIconLoadedImageHref('claude')).toBe('');
  });

  test('a provider that leaves the list loses its image too', async () => {
    const { symbolEls } = installFakeDocument();
    providerIconHtml('claude');
    setProviderIconImageProbe(async () => true);
    applyProviderPresentation([{ id: 'claude', icon_image_version: '1' }]);
    await flush();
    applyProviderPresentation([{ id: 'codex' }]);
    expect(symbolEls.get('aac-prov-claude')?.innerHTML).toContain('>C</text>');
  });

  test('a new version is probed and shown only when it loads; a late answer for an old version is dropped', async () => {
    const { symbolEls } = installFakeDocument();
    providerIconHtml('claude');
    const { calls, probe } = deferredProbe();
    setProviderIconImageProbe(probe);
    applyProviderPresentation([{ id: 'claude', icon_image_version: '1' }]);
    applyProviderPresentation([{ id: 'claude', icon_image_version: '2' }]);
    expect(calls.map((call) => call.href)).toEqual(['/api/provider-icons/claude?v=1', '/api/provider-icons/claude?v=2']);
    calls[1].settle(true);
    await flush();
    expect(symbolEls.get('aac-prov-claude')?.innerHTML).toContain('?v=2');
    calls[0].settle(true);
    await flush();
    expect(symbolEls.get('aac-prov-claude')?.innerHTML).toContain('?v=2');
    expect(providerIconLoadedImageHref('claude')).toBe('/api/provider-icons/claude?v=2');
  });

  test('a replaced image that fails to load falls back to the letters instead of keeping the old picture', async () => {
    const { symbolEls } = installFakeDocument();
    providerIconHtml('claude');
    setProviderIconImageProbe(async (href) => href.endsWith('v=1'));
    applyProviderPresentation([{ id: 'claude', icon_image_version: '1' }]);
    await flush();
    expect(symbolEls.get('aac-prov-claude')?.innerHTML).toContain('<image');
    applyProviderPresentation([{ id: 'claude', icon_image_version: '2' }]);
    await flush();
    expect(symbolEls.get('aac-prov-claude')?.innerHTML).toContain('>C</text>');
  });

  test('an id or version that is not a plain token never reaches the probe or the page', async () => {
    const { symbolEls } = installFakeDocument();
    providerIconHtml('claude');
    const { calls, probe } = deferredProbe();
    setProviderIconImageProbe(probe);
    applyProviderPresentation([
      { id: 'claude', icon_image_version: '1"><script>alert(1)</script>' },
      { id: 'my.ai', icon_image_version: '1' },
      { id: '../x', icon_image_version: '1' },
      { id: 'codex', icon_image_version: 5 as any },
    ]);
    expect(calls).toHaveLength(0);
    expect(symbolEls.size).toBe(0);
  });

  test('a custom provider drawn after its image loaded gets the image in its new symbol', async () => {
    const { state } = installFakeDocument();
    providerIconHtml('claude');
    setProviderIconImageProbe(async () => true);
    // an id no earlier test has drawn (the module remembers every symbol it has made)
    applyProviderPresentation([{ id: 'img-first-ai', icon_image_version: '9' }]);
    await flush();
    providerIconHtml('img-first-ai');
    expect(state.appended).toHaveLength(1);
    expect(state.appended[0]).toContain('<symbol id="aac-prov-img-first-ai"');
    expect(state.appended[0]).toContain('<image href="/api/provider-icons/img-first-ai?v=9"');
  });

  test('without a browser (no Image) nothing is treated as loaded', async () => {
    const { symbolEls } = installFakeDocument();
    providerIconHtml('claude');
    applyProviderPresentation([{ id: 'claude', icon_image_version: '1' }]);
    await flush();
    expect(providerIconLoadedImageHref('claude')).toBe('');
    expect(symbolEls.get('aac-prov-claude')).toBeUndefined();
  });
});

describe('icon image in the settings preview', () => {
  test('the Hub image URL and a chosen file (blob:) are drawn with their own clip id', () => {
    installFakeDocument();
    const hub = providerIconPreviewHtml('claude', { image_href: '/api/provider-icons/claude?v=1a' }, 32);
    expect(hub).not.toContain('<use');
    expect(hub).toContain('<image href="/api/provider-icons/claude?v=1a"');
    expect(hub).toContain('<clipPath id="aac-provclip-preview">');
    expect(hub).not.toContain('<text');
    const blob = providerIconPreviewHtml('ollama', { image_href: 'blob:http://127.0.0.1:47777/3f2a-11' }, 32);
    expect(blob).toContain('<image href="blob:http://127.0.0.1:47777/3f2a-11"');
    expect(blob).toContain('<rect');
  });

  test('anything else in image_href is ignored and the letters are drawn', () => {
    installFakeDocument();
    for (const bad of ['javascript:alert(1)', 'https://example.com/x.png', 'data:image/png;base64,AAAA', '/api/avatar', '"><script>', '/api/provider-icons/../x?v=1', '', undefined, 7]) {
      const html = providerIconPreviewHtml('claude', { image_href: bad as any }, 32);
      expect(html).not.toContain('<image');
      expect(html).toContain('>C</text>');
    }
  });
});
