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
  const lang = stored || (navigator.language || 'ja').slice(0, 2);
  window.__lang = ['ja', 'en'].includes(lang) ? lang : 'ja';
  document.documentElement.lang = window.__lang;

  const res = await fetch('/i18n/' + window.__lang + '.json');
  const dict = await res.json();

  window.t = (key: string, vars?: I18nVars | string) => {
    let s = dict[key] ?? key;
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

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', applyI18n);
  } else {
    applyI18n();
  }

  document.dispatchEvent(new Event('i18n-ready'));
})();
