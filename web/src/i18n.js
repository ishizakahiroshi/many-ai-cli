(async () => {
  const stored = localStorage.getItem('ai_cli_hub_lang');
  const lang = stored || (navigator.language || 'ja').slice(0, 2);
  window.__lang = ['ja', 'en'].includes(lang) ? lang : 'ja';

  const res = await fetch('/i18n/' + window.__lang + '.json');
  const dict = await res.json();

  window.t = (key, vars) => {
    let s = dict[key] ?? key;
    if (vars) Object.entries(vars).forEach(([k, v]) => { s = s.replaceAll('{' + k + '}', v); });
    return String(s);
  };

  window.setLang = (lang) => {
    localStorage.setItem('ai_cli_hub_lang', lang);
    location.reload();
  };

  function applyI18n() {
    document.querySelectorAll('[data-i18n]').forEach(el => {
      el.textContent = window.t(el.dataset.i18n);
    });
    document.querySelectorAll('[data-i18n-placeholder]').forEach(el => {
      el.placeholder = window.t(el.dataset.i18nPlaceholder);
    });
    document.querySelectorAll('[data-i18n-tooltip]').forEach(el => {
      el.dataset.tooltip = window.t(el.dataset.i18nTooltip);
    });
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', applyI18n);
  } else {
    applyI18n();
  }

  document.dispatchEvent(new Event('i18n-ready'));
})();
