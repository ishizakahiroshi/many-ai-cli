import { describe, expect, test } from 'bun:test';
import { DeviceSessionViews, readViewPreference, resolveReadingMode, saveViewPreference, type ViewDevice } from '../src/app/session-view-preferences';

describe('device-local reading preference', () => {
  test('session mode survives viewport changes and numeric cleanup removes both', () => {
    let device: ViewDevice = 'mobile';
    const modes = new DeviceSessionViews(() => device);
    modes.set(7, 'chat'); device = 'desktop'; expect(modes.get(7)).toBeUndefined();
    modes.set(7, 'terminal'); device = 'mobile'; expect(modes.get(7)).toBe('chat');
    modes.delete(7); expect(modes.has(7)).toBe(false); device = 'desktop'; expect(modes.has(7)).toBe(false);
  });
  test('desktop and mobile choices do not overwrite each other', () => {
    const values = new Map<string, string>();
    const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
    saveViewPreference(storage, 'mobile', 'chat');
    saveViewPreference(storage, 'desktop', 'terminal');
    expect(readViewPreference(storage, 'mobile')).toBe('chat');
    expect(readViewPreference(storage, 'desktop')).toBe('terminal');
    saveViewPreference(storage, 'mobile', 'approval');
    expect(readViewPreference(storage, 'mobile')).toBe('chat');
  });
  test('explicit local choice precedes legacy shared default', () => {
    expect(resolveReadingMode('chat', 'terminal', 'mobile')).toBe('chat');
    expect(resolveReadingMode(null, 'terminal', 'mobile')).toBe('terminal');
    expect(resolveReadingMode(null, '', 'mobile')).toBe('chat');
    expect(resolveReadingMode(null, '', 'desktop')).toBe('terminal');
    // 廃止した split を固定値に保存していても、既定の表示へ落ちる
    expect(resolveReadingMode(null, 'split', 'desktop')).toBe('terminal');
  });
  test('corrupt and inaccessible storage remain usable', () => {
    expect(readViewPreference({ getItem: () => '{broken}', setItem() {} }, 'mobile')).toBeNull();
    const denied = { getItem(): string { throw new Error('denied'); }, setItem() { throw new Error('denied'); } };
    expect(readViewPreference(denied, 'mobile')).toBeNull();
    expect(() => saveViewPreference(denied, 'mobile', 'chat')).not.toThrow();
  });
});
