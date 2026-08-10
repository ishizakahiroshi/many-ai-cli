// Pure transcript identity helpers. Kept DOM-free so the update-in-place
// contract can be regression-tested without starting the browser application.

export function transcriptMessageIdentity(msg: any): string {
  return String(msg?.message_id || msg?.messageId || '').trim();
}

export function transcriptMessageKey(msg: any): string {
  return JSON.stringify([
    msg?.role || '', msg?.kind || '', msg?.ts || '', msg?.text || '',
    Array.isArray(msg?.thinking) ? msg.thinking : [],
    Array.isArray(msg?.tools) ? msg.tools : [],
  ]);
}

export function transcriptMessageCategory(role: string, kind: string): string {
  if (role === 'system' || kind === 'approval') return 'approval';
  if (kind === 'attach') return 'attach';
  if (role === 'user') return 'user';
  if (role === 'ai') return 'ai';
  return 'other';
}

export function evaluateTranscriptMessage(text: string, role: string, kind: string, query: string, activeFilters: Set<unknown> | string[]): { category: string; filterOk: boolean; searchOk: boolean } {
  const category = transcriptMessageCategory(role || '', kind || '');
  const filters = activeFilters instanceof Set ? activeFilters : new Set(activeFilters || []);
  const normalizedQuery = String(query || '').toLowerCase();
  return {
    category,
    filterOk: filters.size === 0 || filters.has(category),
    searchOk: !normalizedQuery || String(text || '').toLowerCase().includes(normalizedQuery),
  };
}

export function shouldRefreshChatDerivedState(action: string): boolean {
  return action === 'appended' || action === 'updated';
}

// Apply the subscriber-side DOM transition for an already rendered message.
// Keeping this small and DOM-adapter based makes the exact update-in-place
// contract testable without booting the full browser application.
export function updateRenderedChatMessage(timeline: any, renderedIds: Set<string>, msg: any, render: () => any): 'updated' | 'appended' | 'skipped' | 'ignored' {
  if (!timeline || !msg || !renderedIds || typeof render !== 'function') return 'ignored';
  const id = String(msg.id);
  const children = Array.from(timeline.children || []);
  const existing = children.find((child: any) => String(child?.dataset?.msgId || '') === id) as any;
  if (existing) {
    const replacement = render();
    if (typeof existing.replaceWith === 'function') {
      existing.replaceWith(replacement);
    } else if (existing.parentNode && typeof existing.parentNode.replaceChild === 'function') {
      existing.parentNode.replaceChild(replacement, existing);
    } else {
      return 'ignored';
    }
    return 'updated';
  }
  if (renderedIds.has(id)) return 'skipped';
  renderedIds.add(id);
  timeline.appendChild(render());
  return 'appended';
}
