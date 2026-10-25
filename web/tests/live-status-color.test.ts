import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const BG = 'ai_cli_hub_live_status_bg';
const FG = 'ai_cli_hub_live_status_fg';
const LIGHT_BG = 'ai_cli_hub_live_status_light_bg';
const LIGHT_FG = 'ai_cli_hub_live_status_light_fg';
const DARK_BG = 'ai_cli_hub_live_status_dark_bg';
const DARK_FG = 'ai_cli_hub_live_status_dark_fg';

class Element extends EventTarget {
  children: Element[] = [];
  attributes = new Map<string, string>();
  style: Record<string, any> = {};
  className = ''; textContent = ''; value = ''; hidden = false; type = ''; title = '';
  constructor(public tagName: string) { super(); }
  append(...children: Element[]) { this.children.push(...children); }
  setAttribute(name: string, value: string) { this.attributes.set(name, value); }
  getAttribute(name: string) { return this.attributes.get(name) ?? null; }
  contains(value: unknown): boolean { return this === value || this.children.some(child => child.contains(value)); }
  querySelectorAll(selector: string): Element[] {
    const matches = (item: Element) => selector.startsWith('.')
      ? item.className.split(' ').includes(selector.slice(1)) : item.tagName === selector;
    return this.children.flatMap(child => [...(matches(child) ? [child] : []), ...child.querySelectorAll(selector)]);
  }
  querySelector(selector: string) { return this.querySelectorAll(selector)[0] ?? null; }
  getBoundingClientRect() { return { right: 600, top: 700 }; }
  click() { this.dispatchEvent(new Event('click')); }
}

// Run the real initializer, palette and listeners. Replace only browser/HTTP
// boundaries, so these tests also exercise how the controls call persistence.
function fixture(initial: Record<string, string> = {}, mode = 'dark', mobile = false) {
  const storage = new Map(Object.entries(initial));
  const styles = new Map<string, string>();
  const writes: Array<[string, string]> = [];
  const observers: Array<() => void> = [];
  const timers = new Map<number, () => void>(); let timerId = 0;
  const root = new Element('html'); root.setAttribute('data-theme', mode);
  root.style = {
    setProperty: (name: string, value: string) => styles.set(name, value),
    removeProperty: (name: string) => styles.delete(name),
  };
  const bar = new Element('div'); const body = new Element('body');
  const document = Object.assign(new EventTarget(), {
    documentElement: root, body,
    createElement: (name: string) => new Element(name),
    getElementById: (id: string) => id === 'terminal-live-status' ? bar : null,
  });
  const ctx = {
    document, Event,
    window: { matchMedia: () => ({ matches: mobile }), innerWidth: 1000, innerHeight: 900 },
    MutationObserver: class { constructor(callback: () => void) { observers.push(callback); } observe() {} },
    localStorage: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key),
    },
    setTimeout: (callback: () => void) => { timers.set(++timerId, callback); return timerId; },
    clearTimeout: (id: number) => timers.delete(id),
    STORAGE_LIVE_STATUS_BG_KEY: BG, STORAGE_LIVE_STATUS_FG_KEY: FG,
    STORAGE_LIVE_STATUS_LIGHT_BG_KEY: LIGHT_BG, STORAGE_LIVE_STATUS_LIGHT_FG_KEY: LIGHT_FG,
    STORAGE_LIVE_STATUS_DARK_BG_KEY: DARK_BG, STORAGE_LIVE_STATUS_DARK_FG_KEY: DARK_FG,
    setUserPref: (path: string, value: string) => writes.push([path, value]),
  };
  const source = readFileSync(new URL('../src/app/live-status-color.ts', import.meta.url), 'utf8')
    .replace(/^import\s*\{[\s\S]*?\}\s*from '\.\/user-prefs\.js';\r?\n/m, '').replace(/^export /gm, '');
  runInNewContext(new Bun.Transpiler({ loader: 'ts' }).transformSync(source) + '\ninitLiveStatusColor();', ctx);
  const button = bar.querySelector('.live-status-palette-btn')!;
  const pop = body.querySelector('.live-status-palette-popover')!;
  const [bgInput, fgInput] = pop.querySelectorAll('input');
  return {
    storage, styles, writes, bar, button, pop, bgInput, fgInput,
    modeText: pop.querySelector('.live-status-palette-mode')!, reset: pop.querySelector('.live-status-palette-reset')!,
    theme(next: string) { root.setAttribute('data-theme', next); for (const callback of observers) callback(); },
    mirror(values: Record<string, string>, event = 'user-prefs-mirrored') {
      for (const [key, value] of Object.entries(values)) storage.set(key, value);
      document.dispatchEvent(new Event(event));
    },
    edit(input: Element, value: string, event = 'input') { input.value = value; input.dispatchEvent(new Event(event)); },
    outside() { document.dispatchEvent(new Event('click')); },
    escape() { document.dispatchEvent(Object.assign(new Event('keydown'), { key: 'Escape' })); },
    translationsReady() {
      Object.assign(ctx.window, { t: (key: string) => `translated:${key}` });
      document.dispatchEvent(new Event('i18n-ready'));
    },
    longPress() {
      bar.dispatchEvent(new Event('touchstart')); const pending = [...timers.values()]; timers.clear();
      for (const callback of pending) callback();
    },
    cancelTouch() {
      bar.dispatchEvent(new Event('touchstart')); bar.dispatchEvent(new Event('touchmove'));
      for (const callback of timers.values()) callback();
    },
  };
}

describe('theme-specific live status palette through actual module', () => {
  test('legacy colors remain dark-only and are never deleted by theme changes', () => {
    const f = fixture({ [BG]: '#0d1117', [FG]: '#93c5fd' });
    expect(f.styles.get('--live-status-bg')).toBe('#0d1117');
    f.theme('light'); expect(f.styles.size).toBe(0);
    expect(f.storage.get(BG)).toBe('#0d1117'); expect(f.storage.get(FG)).toBe('#93c5fd');
    f.theme('dark'); expect(f.styles.get('--live-status-fg')).toBe('#93c5fd'); expect(f.writes).toEqual([]);
  });
  test('scoped colors override legacy and follow light/dark attribute changes', () => {
    const f = fixture({ [BG]: '#000000', [LIGHT_BG]: '#f1f5f9', [LIGHT_FG]: '#1e3a8a', [DARK_BG]: '#172554', [DARK_FG]: '#bfdbfe' }, 'light');
    expect(f.styles.get('--live-status-bg')).toBe('#f1f5f9'); expect(f.styles.get('--live-status-fg')).toBe('#1e3a8a');
    f.theme('dark'); expect(f.styles.get('--live-status-bg')).toBe('#172554'); expect(f.styles.get('--live-status-fg')).toBe('#bfdbfe');
  });
  test('background and text preview without saving, then save distinct scoped paths', () => {
    const f = fixture({}, 'light'); f.button.click(); expect(f.pop.hidden).toBe(false);
    expect(f.modeText.textContent).toContain('live_status_palette_auto');
    const labels = f.pop.querySelectorAll('label');
    expect(labels[0].children[0].textContent).toBe('live_status_palette_bg');
    expect(labels[1].children[0].textContent).toBe('live_status_palette_fg');
    expect(f.bgInput.value).toBe('#e2e8f0'); expect(f.fgInput.value).toBe('#1d4ed8');
    f.edit(f.bgInput, '#fafafa'); expect(f.styles.get('--live-status-bg')).toBe('#fafafa');
    expect(f.storage.size).toBe(0); expect(f.writes).toEqual([]);
    f.edit(f.bgInput, '#fafafa', 'change'); f.edit(f.fgInput, '#102030', 'change');
    expect(f.writes).toEqual([['display.live_status_light_bg', '#fafafa'], ['display.live_status_light_fg', '#102030']]);
    expect(f.storage.get(LIGHT_BG)).toBe('#fafafa'); expect(f.storage.get(LIGHT_FG)).toBe('#102030');
    expect(f.styles.get('--live-status-fg')).toBe('#102030'); expect(f.modeText.textContent).toContain('live_status_palette_custom');
  });
  test('light reset clears only light; dark reset also clears legacy fallback', () => {
    const f = fixture({ [BG]: '#010203', [FG]: '#aabbcc', [LIGHT_BG]: '#eeeeee', [LIGHT_FG]: '#112233', [DARK_BG]: '#223344', [DARK_FG]: '#ccddee' }, 'light');
    f.button.click(); f.reset.click(); expect(f.styles.size).toBe(0);
    expect(f.storage.has(LIGHT_BG)).toBe(false); expect(f.storage.has(LIGHT_FG)).toBe(false);
    expect(f.storage.get(DARK_BG)).toBe('#223344'); expect(f.storage.get(BG)).toBe('#010203');
    expect(f.writes).toEqual([['display.live_status_light_bg', ''], ['display.live_status_light_fg', '']]);
    f.theme('dark'); f.button.click(); f.reset.click(); expect(f.storage.size).toBe(0); expect(f.styles.size).toBe(0);
    expect(f.writes.slice(2)).toEqual([['display.live_status_dark_bg', ''], ['display.live_status_dark_fg', ''], ['display.live_status_bg', ''], ['display.live_status_fg', '']]);
  });
  test('toggle, outside click and Escape discard uncommitted previews', () => {
    for (const close of ['toggle', 'outside', 'escape'] as const) {
      const f = fixture({ [LIGHT_BG]: '#eeeeee' }, 'light'); f.button.click(); f.edit(f.bgInput, '#abcdef');
      if (close === 'toggle') f.button.click(); else f[close]();
      expect(f.pop.hidden).toBe(true); expect(f.styles.get('--live-status-bg')).toBe('#eeeeee'); expect(f.writes).toEqual([]);
    }
  });
  test('theme switch closes palette and rejects late native picker change', () => {
    const f = fixture({ [DARK_BG]: '#172554' }, 'light'); f.button.click(); f.edit(f.bgInput, '#abcdef');
    f.theme('dark'); expect(f.pop.hidden).toBe(true); f.edit(f.bgInput, '#abcdef', 'change');
    expect(f.writes).toEqual([]); expect(f.styles.get('--live-status-bg')).toBe('#172554'); expect(f.storage.has(LIGHT_BG)).toBe(false);
    f.button.click(); f.edit(f.fgInput, '#ddeeff', 'change'); expect(f.writes).toEqual([['display.live_status_dark_fg', '#ddeeff']]);
  });
  test('remote sync refreshes open controls and empty values restore defaults', () => {
    const f = fixture({ [LIGHT_BG]: '#eeeeee' }, 'light'); f.button.click(); f.edit(f.bgInput, '#abcdef');
    f.mirror({ [LIGHT_BG]: '#ffeedd', [LIGHT_FG]: '#123456' }); expect(f.pop.hidden).toBe(false);
    expect(f.bgInput.value).toBe('#ffeedd'); expect(f.fgInput.value).toBe('#123456'); expect(f.styles.get('--live-status-bg')).toBe('#ffeedd');
    f.mirror({ [LIGHT_BG]: '', [LIGHT_FG]: '' }); expect(f.styles.size).toBe(0); expect(f.bgInput.value).toBe('#e2e8f0');
    expect(f.modeText.textContent).toContain('live_status_palette_auto'); expect(f.writes).toEqual([]);
  });
  test('mobile long press opens same controls and touch movement cancels', () => {
    const f = fixture({}, 'light', true); f.cancelTouch(); expect(f.pop.hidden).toBe(true); f.longPress(); expect(f.pop.hidden).toBe(false);
    f.edit(f.bgInput, '#ffeedd', 'change'); expect(f.writes).toEqual([['display.live_status_light_bg', '#ffeedd']]);
    const desktop = fixture({}, 'light', false); desktop.longPress(); expect(desktop.pop.hidden).toBe(true);
  });
  test('accepted palette-only mirrors refresh colors and open controls', () => {
    const f = fixture({ [LIGHT_BG]: '#eeeeee' }, 'light'); f.button.click();
    f.mirror({ [LIGHT_BG]: '#ffeedd', [LIGHT_FG]: '#102030' }, 'live-status-colors-mirrored');
    expect(f.styles.get('--live-status-bg')).toBe('#ffeedd');
    expect(f.styles.get('--live-status-fg')).toBe('#102030');
    expect(f.bgInput.value).toBe('#ffeedd'); expect(f.fgInput.value).toBe('#102030');
    expect(f.pop.hidden).toBe(false); expect(f.writes).toEqual([]);
  });
  test('invalid saved hex values never become CSS overrides or picker values', () => {
    const f = fixture({ [LIGHT_BG]: 'red', [LIGHT_FG]: '#123', [BG]: '#010203' }, 'light'); expect(f.styles.size).toBe(0);
    f.button.click(); expect(f.bgInput.value).toBe('#e2e8f0'); expect(f.fgInput.value).toBe('#1d4ed8');
  });
  test('late translation loading updates visible labels and accessible button name', () => {
    const f = fixture({}, 'light');
    f.translationsReady(); f.button.click();
    const labels = f.pop.querySelectorAll('label');
    expect(labels[0].children[0].textContent).toBe('translated:live_status_palette_bg');
    expect(labels[1].children[0].textContent).toBe('translated:live_status_palette_fg');
    expect(f.reset.textContent).toBe('translated:live_status_palette_reset');
    expect(f.button.getAttribute('aria-label')).toBe('translated:live_status_palette_tooltip');
    expect(f.modeText.textContent).toContain('translated:live_status_palette_light');
  });
  test('reapplying the same theme mode leaves the open palette available', () => {
    const f = fixture({}, 'light'); f.button.click(); f.edit(f.bgInput, '#abcdef');
    f.theme('light'); expect(f.pop.hidden).toBe(false);
    expect(f.styles.get('--live-status-bg')).toBe('#abcdef');
    expect(f.writes).toEqual([]);
  });
});
