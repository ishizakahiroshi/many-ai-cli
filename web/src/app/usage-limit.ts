export type UsagePercentSemantics = 'used' | 'remaining';

export interface UsageWindowInput {
  used_percent?: unknown;
  remaining_percent?: unknown;
  window_minutes?: unknown;
  resets_at?: unknown;
}

export interface WindowLabel {
  kind: 'five_hour' | 'weekly' | 'days' | 'hours' | 'minutes' | 'generic';
  amount?: number;
}

export type UsageSeverity = 'normal' | 'warning' | 'danger';

export function clampPercent(value: unknown): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null;
  return Math.max(0, Math.min(100, value));
}

export function normalizeRemainingPercent(value: unknown, semantics: string): number | null {
  const percent = clampPercent(value);
  if (percent === null) return null;
  if (semantics === 'used') return 100 - percent;
  if (semantics === 'remaining') return percent;
  return null;
}

export function remainingPercent(window: UsageWindowInput | undefined): number | null {
  if (!window) return null;
  const remaining = normalizeRemainingPercent(window.remaining_percent, 'remaining');
  if (remaining !== null) return remaining;
  return normalizeRemainingPercent(window.used_percent, 'used');
}

export function usedPercent(window: UsageWindowInput | undefined): number | null {
  const remaining = remainingPercent(window);
  return remaining === null ? null : 100 - remaining;
}

export function windowMinutes(window: UsageWindowInput | undefined): number {
  const value = window?.window_minutes;
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? Math.round(value) : 0;
}

export function windowLabel(minutes: number): WindowLabel {
  if (minutes === 300) return { kind: 'five_hour' };
  if (minutes === 10080) return { kind: 'weekly' };
  if (minutes > 0 && minutes % 1440 === 0) return { kind: 'days', amount: minutes / 1440 };
  if (minutes > 0 && minutes % 60 === 0) return { kind: 'hours', amount: minutes / 60 };
  if (minutes > 0) return { kind: 'minutes', amount: minutes };
  return { kind: 'generic' };
}

export function sortUsageWindows<T extends UsageWindowInput>(windows: T[]): T[] {
  return windows
    .map((window, index) => ({ window, index }))
    .sort((a, b) => windowMinutes(a.window) - windowMinutes(b.window) || a.index - b.index)
    .map(({ window }) => window);
}

export function usageSeverity(remaining: number): UsageSeverity {
  if (remaining <= 10) return 'danger';
  if (remaining <= 25) return 'warning';
  return 'normal';
}

export function resetState(resetAt: unknown, now = Date.now()): { epoch: number; stale: boolean; remainingMs: number } | null {
  if (typeof resetAt !== 'number' || !Number.isFinite(resetAt) || resetAt <= 0) return null;
  const epoch = resetAt * 1000;
  return { epoch, stale: epoch <= now, remainingMs: Math.max(0, epoch - now) };
}
