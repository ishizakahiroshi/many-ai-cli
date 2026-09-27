// Device-local: speakers and listening levels differ between browsers/devices.
// This scales only the dashboard's notification audio, never system volume.
export const STORAGE_NOTIFY_SOUND_VOLUME_KEY = 'ai_cli_hub_notify_sound_volume';
export const DEFAULT_NOTIFY_SOUND_VOLUME = 100;

export function normalizeNotificationVolume(value: number): number {
  return Number.isFinite(value) ? Math.min(100, Math.max(0, Math.round(value))) : DEFAULT_NOTIFY_SOUND_VOLUME;
}

export function getNotificationVolume(): number {
  try {
    const raw = localStorage.getItem(STORAGE_NOTIFY_SOUND_VOLUME_KEY);
    return raw == null || raw.trim() === '' ? DEFAULT_NOTIFY_SOUND_VOLUME : normalizeNotificationVolume(Number(raw));
  } catch (_) {
    return DEFAULT_NOTIFY_SOUND_VOLUME;
  }
}

export function setNotificationVolume(value: number): void {
  try { localStorage.setItem(STORAGE_NOTIFY_SOUND_VOLUME_KEY, String(normalizeNotificationVolume(value))); } catch (_) {}
}
