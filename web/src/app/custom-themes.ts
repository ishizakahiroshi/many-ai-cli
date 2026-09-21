import { STORAGE_CUSTOM_THEMES_KEY, setUserPref } from './user-prefs.js';
import {
  type CustomTheme,
  type ThemeMode,
  DEFAULT_CONTRAST,
  DEFAULT_HUE_DARK,
  DEFAULT_HUE_LIGHT,
  sanitizeCustomThemes,
} from './theme-tokens.js';

export function loadCustomThemes(): CustomTheme[] {
  try {
    const raw = localStorage.getItem(STORAGE_CUSTOM_THEMES_KEY);
    if (!raw) return [];
    return sanitizeCustomThemes(JSON.parse(raw));
  } catch (_) {
    return [];
  }
}

export function saveCustomThemes(list: CustomTheme[]): CustomTheme[] {
  const clean = sanitizeCustomThemes(list);
  setUserPref('display.custom_themes', clean);
  return clean;
}

export function defaultKnobs(mode: ThemeMode): { hue: number; contrast: number } {
  return {
    hue: mode === 'light' ? DEFAULT_HUE_LIGHT : DEFAULT_HUE_DARK,
    contrast: DEFAULT_CONTRAST,
  };
}

export function findCustomTheme(id: string, list: CustomTheme[] = loadCustomThemes()): CustomTheme | undefined {
  return list.find((t) => t.id === id);
}
