// Desktop placement of the session list tab bar. Mobile drawers keep the top strip.
export type SessionListTabLayout = 'top' | 'left' | 'right' | 'bottom';

const STORAGE_KEY = 'sessionListTabsLayout';
const DESKTOP_QUERY = '(min-width: 721px) and (not (pointer: coarse))';

function parseLayout(value: string | null): SessionListTabLayout {
  if (value === 'vertical') return 'left'; // Original left-rail preference.
  if (value === 'left' || value === 'right' || value === 'bottom') return value;
  return 'top'; // Includes the original horizontal preference.
}

function readLayout(): SessionListTabLayout {
  try { return parseLayout(localStorage.getItem(STORAGE_KEY)); }
  catch { return 'top'; }
}

let preferredLayout: SessionListTabLayout = 'top';

export function isSessionListTabsVertical(): boolean {
  return document.getElementById('session-list')?.classList.contains('tabs-vertical') ?? false;
}

function applyLayout(): void {
  const desktop = window.matchMedia?.(DESKTOP_QUERY).matches;
  const position = desktop ? preferredLayout : 'top';
  const vertical = position === 'left' || position === 'right';
  const sidebar = document.getElementById('session-list');
  if (sidebar) {
    sidebar.dataset.tabPosition = position;
    sidebar.classList.toggle('tabs-vertical', vertical);
  }
  document.getElementById('session-list-tabs')?.setAttribute('aria-orientation', vertical ? 'vertical' : 'horizontal');
  for (const select of layoutSelectors()) select.value = preferredLayout;
}

function layoutSelectors(): NodeListOf<HTMLSelectElement> {
  return document.querySelectorAll<HTMLSelectElement>('#session-tab-position, #session-list-tab-layout-select');
}

export function initSessionListTabLayout(): void {
  preferredLayout = readLayout();
  applyLayout();
  for (const select of layoutSelectors()) {
    select.addEventListener('change', () => {
      preferredLayout = parseLayout(select.value);
      try {
        if (preferredLayout === 'top') localStorage.removeItem(STORAGE_KEY);
        else localStorage.setItem(STORAGE_KEY, preferredLayout);
      } catch { /* Keep the choice for this page when storage is unavailable. */ }
      applyLayout();
    });
  }
  window.matchMedia?.(DESKTOP_QUERY).addEventListener('change', applyLayout);
}
