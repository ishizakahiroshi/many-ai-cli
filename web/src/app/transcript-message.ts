// Pure transcript identity helpers. Kept DOM-free so the update-in-place
// contract can be regression-tested without starting the browser application.

export function transcriptMessageIdentity(msg: any): string {
  return String(msg?.message_id || msg?.messageId || '').trim();
}

export function transcriptMessageKey(msg: any): string {
  return JSON.stringify([
    msg?.role || '', msg?.kind || '', msg?.presentation || '', msg?.ts || '', msg?.text || '',
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
  return action === 'appended' || action === 'updated' || action === 'removed';
}

export type TranscriptReadPosition = { top: number; following: boolean };

export function transcriptReadPosition(top: number, height: number, viewport: number): TranscriptReadPosition {
  return { top, following: height - top - viewport < 60 };
}

export function transcriptRestoreTop(position: TranscriptReadPosition | undefined, height: number, viewport: number): number {
  const max = Math.max(0, height - viewport);
  return !position || position.following ? max : Math.max(0, Math.min(position.top, max));
}

// Thought bodies never enter the chat DOM. Codex legacy text has no reliable
// final/progress discriminator, while Anthropic text blocks remain useful
// readable answers without inventing a phase requirement for those providers.
export function transcriptMessagePresentation(msg: any, provider = ''): 'hidden' | 'answer' | 'progress' | 'unclassified' | 'tool' {
  if (msg?.role !== 'ai' || msg?.meta?.transcript !== true) return 'answer';
  if (msg.kind === 'thinking' || msg.kind === 'sidechain' || msg.meta.presentation === 'hidden') return 'hidden';
  const text = String(msg.normalizedText || msg.rawText || '');
  if (!text && Array.isArray(msg.meta.tools) && msg.meta.tools.length) return 'tool';
  if (!text) return 'hidden';
  if (msg.meta.presentation === 'progress') return 'progress';
  if (msg.meta.presentation === 'answer') return 'answer';
  if (msg.meta.presentation === 'unclassified' || provider === 'codex') return 'unclassified';
  return 'answer';
}

// Apply the subscriber-side DOM transition for an already rendered message.
// Keeping this small and DOM-adapter based makes the exact update-in-place
// contract testable without booting the full browser application.
export function updateRenderedChatMessage(timeline: any, renderedIds: Set<string>, msg: any, render: () => any): 'updated' | 'appended' | 'removed' | 'skipped' | 'ignored' {
  if (!timeline || !msg || !renderedIds || typeof render !== 'function') return 'ignored';
  const id = String(msg.id);
  const children = Array.from(timeline.children || []);
  const existing = children.find((child: any) => String(child?.dataset?.msgId || '') === id) as any;
  const replacement = render();
  if (!replacement) {
    renderedIds.delete(id);
    if (existing) {
      if (typeof existing.remove === 'function') existing.remove();
      else existing.parentNode?.removeChild(existing);
      return 'removed';
    }
    return 'skipped';
  }
  if (existing) {
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
  timeline.appendChild(replacement);
  return 'appended';
}
