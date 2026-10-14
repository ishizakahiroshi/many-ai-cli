// Appearance only: callers retain events, focus, cleanup and return values.
export type UIButtonVariant = 'primary' | 'secondary' | 'danger' | 'quiet' | 'warn';
export type UIButtonSize = 'compact' | 'regular';
export type UIModalSize = 'sm' | 'md' | 'lg';

// menuItem: ポップアップ・右クリックメニューの 1 行。幅いっぱい・左寄せにする（ボタンの既定は中央寄せ）。
export function uiButtonClass(variant: UIButtonVariant = 'secondary', size: UIButtonSize = 'regular', options?: { icon?: boolean; touch?: boolean; menuItem?: boolean }): string {
  return [`ui-button`, `ui-button--${variant}`, `ui-button--${size}`, options?.icon && 'ui-button--icon', options?.touch && 'ui-button--touch', options?.menuItem && 'ui-button--menu-item'].filter(Boolean).join(' ');
}

export function uiModalClass(size: UIModalSize = 'sm'): string {
  return `ui-modal ui-modal--${size}`;
}

export type UIIconName = 'close' | 'copy' | 'edit' | 'delete' | 'refresh' | 'check' | 'stop';
const ICON_PATHS: Record<UIIconName, string> = {
  close: '<path d="m6 6 12 12M18 6 6 18"/>',
  copy: '<rect x="9" y="9" width="12" height="12" rx="2"/><path d="M15 9V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h4"/>',
  edit: '<path d="m16 3 5 5-12 12-6 1 1-6L16 3Z"/>',
  delete: '<path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7"/>',
  refresh: '<path d="M20 7v5h-5M4 17v-5h5M6 6a8 8 0 0 1 14 6M18 18a8 8 0 0 1-14-6"/>',
  check: '<path d="m4 12 5 5L20 6"/>',
  stop: '<rect x="5" y="5" width="14" height="14" rx="1"/>',
};

// Decorative SVG only; the surrounding control must supply its accessible name.
// Provider and session-state icons keep their existing dedicated suppliers.
export function uiIconHtml(name: UIIconName, size: 'sm' | 'md' = 'sm'): string {
  return `<svg class="ui-icon ui-icon--${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">${ICON_PATHS[name]}</svg>`;
}
