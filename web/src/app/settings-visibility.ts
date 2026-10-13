// Embedded Settings destinations share the screen lifecycle, including mobile Back.
export function isSettingsContentVisible(element: HTMLElement | null): boolean {
  if (!element || element.closest('[hidden]')) return false;
  const panel = element.closest('#settings-panel');
  if (!panel) return false;
  return !window.matchMedia?.('(max-width: 720px)').matches
    || panel.classList.contains('settings-detail-open');
}

export function notifySettingsViewChanged(): void {
  document.dispatchEvent(new CustomEvent('settings-view-changed'));
}
