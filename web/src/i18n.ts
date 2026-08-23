// --- ESM late-bound i18n wrappers (generated; window.t is set by the IIFE below at runtime) ---
export type I18nVars = Record<string, unknown>;

export function t(key: string, vars?: I18nVars | string): string {
  return (typeof window.t === 'function') ? window.t(key, vars as I18nVars) : key;
}

export function setLang(v: string): void | undefined {
  return (typeof window.setLang === 'function') ? window.setLang(v) : undefined;
}

(async () => {
  const stored = localStorage.getItem('ai_cli_hub_lang');
  // navigator.language: "vi", "vi-VN", "en-US", "ja-JP", ...
  const nav = (navigator.language || 'ja').toLowerCase();
  const nav2 = nav.startsWith('vi') ? 'vi' : nav.slice(0, 2);
  const lang = stored || nav2;
  window.__lang = ['ja', 'en', 'vi'].includes(lang) ? lang : 'ja';
  document.documentElement.lang = window.__lang;

  let dict: Record<string, unknown> = {};
  try {
    const res = await fetch('/i18n/' + window.__lang + '.json');
    if (!res.ok) throw new Error(`i18n dictionary request failed: ${res.status}`);
    const loaded = await res.json();
    if (loaded && typeof loaded === 'object' && !Array.isArray(loaded)) {
      dict = loaded as Record<string, unknown>;
    }
  } catch (err) {
    // An unavailable dictionary must not prevent the app modules from
    // receiving i18n-ready. The key itself is the safe final fallback.
    console.warn('[i18n] dictionary unavailable; using key fallbacks', err);
  }

  window.t = (key: string, vars?: I18nVars | string) => {
    let s = String(dict[key] ?? key);
    if (vars && typeof vars === 'object') Object.entries(vars).forEach(([k, v]) => { s = s.replaceAll('{' + k + '}', String(v)); });
    return String(s);
  };

  window.setLang = (lang: string) => {
    localStorage.setItem('ai_cli_hub_lang', lang);
    location.reload();
  };

  function applyI18n() {
    document.querySelectorAll('[data-i18n]').forEach(el => {
      const target = el as HTMLElement;
      target.textContent = t(target.dataset.i18n || '');
    });
    document.querySelectorAll('[data-i18n-placeholder]').forEach(el => {
      const target = el as HTMLInputElement;
      target.placeholder = t(target.dataset.i18nPlaceholder || '');
    });
    document.querySelectorAll('[data-i18n-tooltip]').forEach(el => {
      const target = el as HTMLElement;
      target.dataset.tooltip = t(target.dataset.i18nTooltip || '');
    });
  }

  const ready = () => {
    applyI18n();
    document.dispatchEvent(new Event('i18n-ready'));
  };
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', ready, { once: true });
  } else {
    ready();
  }
})();
