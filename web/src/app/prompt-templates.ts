import { t } from '../i18n.js';
import { STORAGE_TEMPLATES_KEY, STORAGE_TEMPLATE_SEND_IMMEDIATE_KEY, setUserPref } from './user-prefs.js';
import { activeSessionId, sessions } from './state.js';
import { showToast } from './util.js';

const MAX_TEMPLATES = 100;
const MAX_BODY_LENGTH = 8000;
const MOBILE_LABEL_LENGTH = 5;

export type PromptTemplate = { body: string; providers: string[]; tags: string[]; frequency: number };

const DEFAULT_TEMPLATES: PromptTemplate[] = [
  { body: '変更内容を確認して、PR を作成できる状態まで進めてください。', providers: [], tags: ['git'], frequency: 0 },
  { body: 'この変更をレビューして、問題があれば重要度順に指摘してください。', providers: [], tags: ['review'], frequency: 0 },
  { body: 'ここまでの作業内容、変更ファイル、検証結果、次にやることを要約してください。', providers: [], tags: ['summary'], frequency: 0 },
];

function defaultTemplates(): PromptTemplate[] {
  return DEFAULT_TEMPLATES.map((template) => ({ ...template, providers: [], tags: [...template.tags] }));
}

function normalizeBody(raw: unknown): string {
  return typeof raw === 'string' ? raw.trim().slice(0, MAX_BODY_LENGTH) : '';
}

function normalize(raw: unknown): PromptTemplate[] {
  if (!Array.isArray(raw)) return [];
  return raw
    .filter((v): v is Record<string, unknown> => !!v && typeof v === 'object' && typeof v.body === 'string')
    .slice(0, MAX_TEMPLATES)
    .map((v) => ({
      // label は旧保存データとの互換のため読み飛ばす。body が正本。
      body: normalizeBody(v.body),
      providers: Array.isArray(v.providers) ? v.providers.filter((p): p is string => typeof p === 'string').slice(0, 10) : [],
      tags: Array.isArray(v.tags) ? v.tags.filter((tag): tag is string => typeof tag === 'string').slice(0, 10) : [],
      frequency: Math.max(0, Math.min(100000, Number(v.frequency) || 0)),
    }))
    .filter((v) => v.body);
}

export function getPromptTemplates(): PromptTemplate[] {
  try {
    const stored = localStorage.getItem(STORAGE_TEMPLATES_KEY);
    if (stored === null) return defaultTemplates();
    const parsed: unknown = JSON.parse(stored);
    return Array.isArray(parsed) ? normalize(parsed) : defaultTemplates();
  } catch (_) {
    return defaultTemplates();
  }
}

function notifyTemplatesChanged(): void {
  window.dispatchEvent(new Event('prompt-templates:changed'));
}

function save(templates: PromptTemplate[]): void {
  setUserPref('templates', templates);
  notifyTemplatesChanged();
}

export function addPromptTemplate(body: string): boolean {
  const normalized = normalizeBody(body);
  if (!normalized) return false;
  const templates = getPromptTemplates();
  if (templates.length >= MAX_TEMPLATES || templates.some((template) => template.body === normalized)) return false;
  templates.push({ body: normalized, providers: [], tags: [], frequency: 0 });
  save(templates);
  return true;
}

export function promptTemplateLabel(body: string, maxChars = MOBILE_LABEL_LENGTH): string {
  const chars = [...(body || '')];
  return chars.length <= maxChars ? chars.join('') : chars.slice(0, maxChars).join('') + '…';
}

function activeProvider(): string {
  return activeSessionId === null ? '' : sessions.get(activeSessionId)?.provider || '';
}

export function templatesForProvider(provider = activeProvider(), query = ''): PromptTemplate[] {
  const needle = query.trim().toLocaleLowerCase();
  return getPromptTemplates().filter((template) =>
    (!provider || template.providers.length === 0 || template.providers.includes(provider)) &&
    (!needle || `${template.body} ${template.tags.join(' ')}`.toLocaleLowerCase().includes(needle)),
  ).sort((a, b) => b.frequency - a.frequency || a.body.localeCompare(b.body, 'ja'));
}

// テンプレートを選んだときに即時送信するか（true）、入力欄へ反映するだけか（false・既定）。
// サーバ同期（user_prefs.template_send.immediate）なので端末を跨いで同じ挙動になる。
export function isTemplateSendImmediate(): boolean {
  try { return localStorage.getItem(STORAGE_TEMPLATE_SEND_IMMEDIATE_KEY) === '1'; } catch (_) { return false; }
}

export function setTemplateSendImmediate(value: boolean): void {
  setUserPref('template_send.immediate', value);
}

export function insertPromptTemplate(template: PromptTemplate): void {
  const templates = getPromptTemplates();
  const index = templates.findIndex((candidate) => candidate.body === template.body);
  if (index >= 0) {
    templates[index] = { ...templates[index], frequency: templates[index].frequency + 1 };
    save(templates);
  }
  // send は選択時点の設定を焼き付けて渡す（受け手の app.ts 側で再判定しない）。
  window.dispatchEvent(new CustomEvent('many-ai-cli:insert-template', { detail: { text: template.body, send: isTemplateSendImmediate() } }));
}

type UndoState = { template: PromptTemplate; index: number };
let editingBody: string | null = null;
let editDraft = '';
let adding = false;
let addDraft = '';
let undoState: UndoState | null = null;

function text(key: string, fallback: string): string {
  const translated = t(key);
  return translated === key ? fallback : translated;
}

function templateIndex(templates: PromptTemplate[], body: string): number {
  return templates.findIndex((template) => template.body === body);
}

function makeActionButton(icon: string, label: string, handler: (event: MouseEvent) => void): HTMLButtonElement {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'prompt-template-action';
  button.textContent = icon;
  button.title = label;
  button.setAttribute('aria-label', label);
  button.addEventListener('click', handler);
  return button;
}

function renderEditor(row: HTMLElement, mode: 'add' | 'edit', template?: PromptTemplate): void {
  const editor = document.createElement('div');
  editor.className = 'prompt-template-editor';

  const textarea = document.createElement('textarea');
  textarea.rows = 3;
  textarea.maxLength = MAX_BODY_LENGTH;
  textarea.className = 'prompt-template-editor-input';
  textarea.placeholder = text('template_body_placeholder', 'テンプレート本文');
  textarea.value = mode === 'edit' ? editDraft : addDraft;
  textarea.addEventListener('input', () => {
    if (mode === 'edit') editDraft = textarea.value;
    else addDraft = textarea.value;
  });

  const actions = document.createElement('div');
  actions.className = 'prompt-template-editor-actions';
  const saveButton = document.createElement('button');
  saveButton.type = 'button';
  saveButton.className = 'prompt-template-editor-save';
  saveButton.textContent = text('template_save', '保存');
  const cancelButton = document.createElement('button');
  cancelButton.type = 'button';
  cancelButton.className = 'prompt-template-editor-cancel';
  cancelButton.textContent = text('template_cancel', 'キャンセル');

  saveButton.addEventListener('click', (event) => {
    event.stopPropagation();
    const body = normalizeBody(textarea.value);
    if (!body) {
      showToast(text('template_body_required', '本文を入力してください'));
      textarea.focus();
      return;
    }
    if (mode === 'add') {
      if (getPromptTemplates().some((candidate) => candidate.body === body)) {
        showToast(text('template_duplicate', '同じ内容のテンプレートがあります'));
        textarea.focus();
        return;
      }
      if (addPromptTemplate(body)) {
        adding = false;
        addDraft = '';
        renderPalette();
      } else {
        showToast(text('template_add_failed', 'テンプレートを追加できませんでした'));
      }
      return;
    }

    const templates = getPromptTemplates();
    const index = template ? templateIndex(templates, template.body) : -1;
    if (index < 0) return;
    if (templates.some((candidate, candidateIndex) => candidateIndex !== index && candidate.body === body)) {
      showToast(text('template_duplicate', '同じ内容のテンプレートがあります'));
      textarea.focus();
      return;
    }
    templates[index] = { ...templates[index], body };
    editingBody = null;
    editDraft = '';
    save(templates);
  });

  cancelButton.addEventListener('click', (event) => {
    event.stopPropagation();
    if (mode === 'add') {
      adding = false;
      addDraft = '';
    } else {
      editingBody = null;
      editDraft = '';
    }
    renderPalette();
  });

  actions.append(saveButton, cancelButton);
  editor.append(textarea, actions);
  row.append(editor);
  requestAnimationFrame(() => {
    textarea.focus();
    textarea.setSelectionRange(textarea.value.length, textarea.value.length);
  });
}

function renderTemplateRow(row: HTMLElement, template: PromptTemplate, panel: HTMLElement): void {
  if (editingBody === template.body) {
    renderEditor(row, 'edit', template);
    return;
  }

  const selectButton = document.createElement('button');
  selectButton.type = 'button';
  selectButton.className = 'prompt-template-item';
  selectButton.setAttribute('role', 'menuitem');
  selectButton.textContent = template.body;
  selectButton.title = template.body;
  selectButton.addEventListener('click', () => {
    insertPromptTemplate(template);
    closePalette(panel);
  });

  const actions = document.createElement('div');
  actions.className = 'prompt-template-actions';
  actions.append(
    makeActionButton('✏', text('template_edit', '編集'), (event) => {
      event.preventDefault();
      event.stopPropagation();
      editingBody = template.body;
      editDraft = template.body;
      renderPalette();
    }),
    makeActionButton('✕', text('template_delete', '削除'), (event) => {
      event.preventDefault();
      event.stopPropagation();
      const templates = getPromptTemplates();
      const index = templateIndex(templates, template.body);
      if (index < 0) return;
      const deleted = templates.splice(index, 1)[0];
      undoState = deleted ? { template: deleted, index } : null;
      if (editingBody === template.body) editingBody = null;
      save(templates);
    }),
  );
  row.append(selectButton, actions);
}

function renderUndoRow(list: HTMLElement): void {
  if (!undoState) return;
  const row = document.createElement('div');
  row.className = 'prompt-template-undo';
  const message = document.createElement('span');
  message.textContent = text('template_deleted', '削除しました');
  const undoButton = document.createElement('button');
  undoButton.type = 'button';
  undoButton.textContent = text('template_undo', '元に戻す');
  undoButton.addEventListener('click', (event) => {
    event.stopPropagation();
    if (!undoState) return;
    const templates = getPromptTemplates();
    if (!templates.some((template) => template.body === undoState?.template.body)) {
      templates.splice(Math.min(undoState.index, templates.length), 0, undoState.template);
      undoState = null;
      save(templates);
    }
  });
  row.append(message, undoButton);
  list.append(row);
}

function renderPalette(): void {
  const panel = document.getElementById('prompt-template-palette');
  const list = document.getElementById('prompt-template-list');
  const search = document.getElementById('prompt-template-search') as HTMLInputElement | null;
  if (!panel || !list || !search) return;
  const templates = templatesForProvider(undefined, search.value);
  list.replaceChildren();
  if (!templates.length && !adding) {
    const empty = document.createElement('div');
    empty.className = 'prompt-template-empty';
    empty.textContent = text('template_empty', '該当するテンプレートはありません');
    list.append(empty);
  }
  for (const template of templates) {
    const row = document.createElement('div');
    row.className = 'prompt-template-row';
    renderTemplateRow(row, template, panel);
    list.append(row);
  }
  renderUndoRow(list);

  if (adding) {
    const row = document.createElement('div');
    row.className = 'prompt-template-row';
    renderEditor(row, 'add');
    list.append(row);
  } else {
    const addButton = document.createElement('button');
    addButton.type = 'button';
    addButton.className = 'prompt-template-add';
    addButton.textContent = text('template_add', '＋ 追加');
    addButton.addEventListener('click', (event) => {
      event.stopPropagation();
      adding = true;
      addDraft = '';
      renderPalette();
    });
    list.append(addButton);
  }
}

function closePalette(panel: HTMLElement): void {
  panel.hidden = true;
  document.getElementById('prompt-template-toggle')?.setAttribute('aria-expanded', 'false');
  editingBody = null;
  editDraft = '';
  adding = false;
  addDraft = '';
  undoState = null;
}

function syncSendModeToggle(): void {
  const input = document.getElementById('prompt-template-send-immediate') as HTMLInputElement | null;
  if (!input) return;
  const immediate = isTemplateSendImmediate();
  input.checked = immediate;
  const note = immediate
    ? text('template_send_immediate_note_on', '選んだテンプレートをそのまま送信します')
    : text('template_send_immediate_note_off', '選んだテンプレートを入力欄に入れます');
  input.setAttribute('aria-label', text('template_send_immediate', '選んだら即送信'));
  input.title = note;
  const noteEl = document.getElementById('prompt-template-mode-note');
  if (noteEl) noteEl.textContent = note;
}

function initSendModeToggle(): void {
  const input = document.getElementById('prompt-template-send-immediate') as HTMLInputElement | null;
  if (!input) return;
  input.addEventListener('change', () => {
    setTemplateSendImmediate(input.checked);
    syncSendModeToggle();
  });
  document.addEventListener('user-prefs-mirrored', syncSendModeToggle);
  document.addEventListener('i18n-ready', syncSendModeToggle);
  syncSendModeToggle();
}

function initPalette(): void {
  const toggle = document.getElementById('prompt-template-toggle') as HTMLButtonElement | null;
  const panel = document.getElementById('prompt-template-palette');
  const search = document.getElementById('prompt-template-search') as HTMLInputElement | null;
  if (!toggle || !panel || !search) return;
  toggle.addEventListener('click', () => {
    if (!panel.hidden) {
      closePalette(panel);
      return;
    }
    panel.hidden = false;
    toggle.setAttribute('aria-expanded', 'true');
    syncSendModeToggle();
    renderPalette();
    search.focus();
  });
  initSendModeToggle();
  search.addEventListener('input', renderPalette);
  document.addEventListener('session:activated', renderPalette);
  document.addEventListener('user-prefs-mirrored', () => { renderPalette(); window.dispatchEvent(new Event('prompt-templates:changed')); });
  document.addEventListener('i18n-ready', renderPalette);
  window.addEventListener('prompt-templates:changed', renderPalette);
  document.addEventListener('click', (event) => {
    const target = event.target;
    if (!panel.hidden && target instanceof Node && !panel.contains(target) && !toggle.contains(target)) closePalette(panel);
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && !panel.hidden) closePalette(panel);
  });
  window.addEventListener('blur', () => closePalette(panel));
}

initPalette();
