import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const BG_KEY = 'ai_cli_hub_live_status_bg';
const FG_KEY = 'ai_cli_hub_live_status_fg';

// Execute the actual initializer without loading user-prefs (which starts HTTP
// sync on import). Browser storage, styles and document events stay in memory.
function fixture(bg: string, fg: string) {
  const storage = new Map([[BG_KEY, bg], [FG_KEY, fg]]);
  const styles = new Map<string, string>();
  const document = Object.assign(new EventTarget(), {
    documentElement: {
      style: {
        setProperty: (name: string, value: string) => { styles.set(name, value); },
        removeProperty: (name: string) => { styles.delete(name); },
      },
    },
    // Palette construction is unrelated to applying colors after async sync.
    getElementById: () => null,
  });
  const ctx = {
    document,
    window: {},
    localStorage: {
      getItem: (key: string) => storage.get(key) ?? null,
    },
    STORAGE_LIVE_STATUS_BG_KEY: BG_KEY,
    STORAGE_LIVE_STATUS_FG_KEY: FG_KEY,
    setUserPref: () => { throw new Error('Color application must not save preferences'); },
  };
  const source = readFileSync(new URL('../src/app/live-status-color.ts', import.meta.url), 'utf8')
    .replace(/^import\s*\{[\s\S]*?\}\s*from '\.\/user-prefs\.js';\r?\n/m, '')
    .replace(/^export /gm, '');
  runInNewContext(new Bun.Transpiler({ loader: 'ts' }).transformSync(source)
    + '\ninitLiveStatusColor();', ctx);
  return {
    storage,
    styles,
    mirror(nextBg: string, nextFg: string) {
      // user-prefs.ts writes both localStorage values before dispatching this
      // document event. Reproduce that caller order, after initialization.
      storage.set(BG_KEY, nextBg);
      storage.set(FG_KEY, nextFg);
      document.dispatchEvent(new Event('user-prefs-mirrored'));
    },
  };
}

describe('live status colors after preference sync', () => {
  test('initial dark colors update when the server mirrors light colors', () => {
    const f = fixture('#0d1117', '#93c5fd');
    expect(f.styles.get('--live-status-bg')).toBe('#0d1117');
    expect(f.styles.get('--live-status-fg')).toBe('#93c5fd');
    f.mirror('#e2e8f0', '#1d4ed8');
    expect(f.styles.get('--live-status-bg')).toBe('#e2e8f0');
    expect(f.styles.get('--live-status-fg')).toBe('#1d4ed8');
  });

  test('mirrored empty colors remove inline overrides for the theme defaults', () => {
    const f = fixture('#0d1117', '#93c5fd');
    f.mirror('', '');
    expect(f.styles.has('--live-status-bg')).toBe(false);
    expect(f.styles.has('--live-status-fg')).toBe(false);
    expect(f.storage.get(BG_KEY)).toBe('');
    expect(f.storage.get(FG_KEY)).toBe('');
  });

  test('explicit dark colors remain applied and are not migrated or deleted', () => {
    const f = fixture('', '');
    expect(f.styles.size).toBe(0);
    f.mirror('#0d1117', '#93c5fd');
    expect(f.styles.get('--live-status-bg')).toBe('#0d1117');
    expect(f.styles.get('--live-status-fg')).toBe('#93c5fd');
    expect(f.storage.get(BG_KEY)).toBe('#0d1117');
    expect(f.storage.get(FG_KEY)).toBe('#93c5fd');
  });
});
