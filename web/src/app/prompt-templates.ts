import { STORAGE_TEMPLATES_KEY, setUserPref } from './user-prefs.js';
import { activeSessionId, sessions } from './state.js';

export type PromptTemplate = { label: string; body: string; providers: string[]; tags: string[]; frequency: number };

const DEFAULT_TEMPLATES: PromptTemplate[] = [
  { label: 'PR にして', body: '変更内容を確認して、PR を作成できる状態まで進めてください。', providers: [], tags: ['git'], frequency: 0 },
  { label: 'レビュー', body: 'この変更をレビューして、問題があれば重要度順に指摘してください。', providers: [], tags: ['review'], frequency: 0 },
  { label: '要約して', body: 'ここまでの作業内容、変更ファイル、検証結果、次にやることを要約してください。', providers: [], tags: ['summary'], frequency: 0 },
];

function normalize(raw: unknown): PromptTemplate[] {
  if (!Array.isArray(raw)) return [];
  return raw.filter((v): v is Record<string, unknown> => !!v && typeof v === 'object' && typeof v.label === 'string' && typeof v.body === 'string')
    .slice(0, 100).map((v) => ({
      label: String(v.label).slice(0, 80), body: String(v.body).slice(0, 8000),
      providers: Array.isArray(v.providers) ? v.providers.filter((p): p is string => typeof p === 'string').slice(0, 10) : [],
      tags: Array.isArray(v.tags) ? v.tags.filter((tag): tag is string => typeof tag === 'string').slice(0, 10) : [],
      frequency: Math.max(0, Math.min(100000, Number(v.frequency) || 0)),
    })).filter((v) => v.label && v.body);
}

export function getPromptTemplates(): PromptTemplate[] {
  try {
    const saved = normalize(JSON.parse(localStorage.getItem(STORAGE_TEMPLATES_KEY) || '[]'));
    return saved.length ? saved : DEFAULT_TEMPLATES;
  } catch (_) { return DEFAULT_TEMPLATES; }
}

function save(templates: PromptTemplate[]): void { setUserPref('templates', templates); }

function activeProvider(): string {
  return activeSessionId === null ? '' : sessions.get(activeSessionId)?.provider || '';
}

export function templatesForProvider(provider = activeProvider(), query = ''): PromptTemplate[] {
  const needle = query.trim().toLocaleLowerCase();
  return getPromptTemplates().filter((template) =>
    (!provider || template.providers.length === 0 || template.providers.includes(provider)) &&
    (!needle || `${template.label} ${template.body} ${template.tags.join(' ')}`.toLocaleLowerCase().includes(needle)),
  ).sort((a, b) => b.frequency - a.frequency || a.label.localeCompare(b.label, 'ja'));
}

export function insertPromptTemplate(template: PromptTemplate): void {
  const templates = getPromptTemplates();
  const index = templates.findIndex((candidate) => candidate.label === template.label && candidate.body === template.body);
  if (index >= 0) { templates[index] = { ...templates[index], frequency: templates[index].frequency + 1 }; save(templates); }
  window.dispatchEvent(new CustomEvent('many-ai-cli:insert-template', { detail: { text: template.body } }));
  window.dispatchEvent(new Event('prompt-templates:changed'));
}

function renderPalette(): void {
  const panel = document.getElementById('prompt-template-palette');
  const list = document.getElementById('prompt-template-list');
  const search = document.getElementById('prompt-template-search') as HTMLInputElement | null;
  if (!panel || !list || !search) return;
  const templates = templatesForProvider(undefined, search.value);
  list.replaceChildren();
  for (const template of templates) {
    const button = document.createElement('button'); button.type = 'button'; button.className = 'prompt-template-item';
    button.textContent = template.label;
    button.title = template.body;
    button.addEventListener('click', () => { insertPromptTemplate(template); panel.hidden = true; });
    list.append(button);
  }
  if (!templates.length) list.textContent = '該当するテンプレートはありません';
}

function initPalette(): void {
  const toggle = document.getElementById('prompt-template-toggle') as HTMLButtonElement | null;
  const panel = document.getElementById('prompt-template-palette');
  const search = document.getElementById('prompt-template-search') as HTMLInputElement | null;
  if (!toggle || !panel || !search) return;
  toggle.addEventListener('click', () => {
    panel.hidden = !panel.hidden; toggle.setAttribute('aria-expanded', String(!panel.hidden));
    if (!panel.hidden) { renderPalette(); search.focus(); }
  });
  search.addEventListener('input', renderPalette);
  document.addEventListener('session:activated', renderPalette);
  document.addEventListener('user-prefs-mirrored', () => { renderPalette(); window.dispatchEvent(new Event('prompt-templates:changed')); });
  window.addEventListener('prompt-templates:changed', renderPalette);
}

initPalette();
