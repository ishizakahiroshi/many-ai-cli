// Dirty generations fence remote reads and PUT acknowledgements. Only edits to
// the shared list may replace the server list; unrelated preferences use the
// freshly fetched server value instead of an old browser cache.
export class SharedTemplateSync {
  private generation = 0;
  private dirty: boolean;

  constructor(dirty = false) { this.dirty = dirty; }

  markDirty(): void { this.generation++; this.dirty = true; }
  discard(): void { this.generation++; this.dirty = false; }
  isDirty(): boolean { return this.dirty; }
  readVersion(): number { return this.generation; }
  canMirror(version: number): boolean { return !this.dirty && version === this.generation; }

  mergeInto(prefs: Record<string, any>, local: unknown): number | null {
    if (!this.dirty || !Array.isArray(local)) return null;
    prefs.templates = local;
    return this.generation;
  }

  acknowledge(version: number | null): boolean {
    if (version === null || version !== this.generation) return false;
    this.dirty = false;
    return true;
  }
}
