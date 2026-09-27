// Reading preferences belong to this browser, not the Hub's shared user_prefs.
export type ReadingMode = 'terminal' | 'chat';
export type ViewDevice = 'mobile' | 'desktop';
export interface ViewPreferenceStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export function readingMode(value: unknown): ReadingMode | null {
  return value === 'terminal' || value === 'chat' ? value : null;
}

export function viewPreferenceKey(device: ViewDevice): string {
  return `ai_cli_hub_reading_mode_${device}`;
}

export function readViewPreference(storage: ViewPreferenceStorage | null, device: ViewDevice): ReadingMode | null {
  try { return readingMode(storage?.getItem(viewPreferenceKey(device))); } catch (_) { return null; }
}

export function saveViewPreference(storage: ViewPreferenceStorage | null, device: ViewDevice, mode: unknown): void {
  const valid = readingMode(mode);
  if (!valid) return;
  try { storage?.setItem(viewPreferenceKey(device), valid); } catch (_) { /* private browsing may deny storage */ }
}

export function resolveReadingMode(saved: unknown, locked: unknown, device: ViewDevice): string {
  return readingMode(saved) || (locked === 'split' ? 'split' : readingMode(locked)) || (device === 'mobile' ? 'chat' : 'terminal');
}

// Preserve the existing numeric Map API, including cleanupRemovedSessionState.delete(id).
export class DeviceSessionViews extends Map<number, string> {
  private readonly mobile = new Map<number, string>();
  constructor(private readonly device: () => ViewDevice) { super(); }
  override get(id: number): string | undefined { return this.device() === 'mobile' ? this.mobile.get(id) : super.get(id); }
  override has(id: number): boolean { return this.device() === 'mobile' ? this.mobile.has(id) : super.has(id); }
  override set(id: number, value: string): this {
    if (this.device() === 'mobile') this.mobile.set(id, value); else super.set(id, value);
    return this;
  }
  override delete(id: number): boolean {
    const removedMobile = this.mobile.delete(id);
    return super.delete(id) || removedMobile;
  }
  override clear(): void { this.mobile.clear(); super.clear(); }
}
