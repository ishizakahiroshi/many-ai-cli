import { t } from '../i18n.js';
import { STORAGE_TEMPLATES_KEY, STORAGE_TEMPLATE_SEND_IMMEDIATE_KEY, setUserPref } from './user-prefs.js';
import { activeSessionId, sessions } from './state.js';
import { showToast } from './util.js';

const MAX_TEMPLATES = 100;
const MAX_BODY_LENGTH = 8000;
const MOBILE_LABEL_LENGTH = 5;

export type PromptTemplate = { body: string; providers: string[]; tags: string[] };

const DEFAULT_TEMPLATES: PromptTemplate[] = [
  { body: '変更内容を確認して、PR を作成できる状態まで進めてください。', providers: [], tags: ['git'] },
  { body: 'この変更をレビューして、問題があれば重要度順に指摘してください。', providers: [], tags: ['review'] },
  { body: 'ここまでの作業内容、変更ファイル、検証結果、次にやることを要約してください。', providers: [], tags: ['summary'] },
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
      // label / frequency は旧保存データとの互換のため読み飛ばす。body が正本で、
      // 並び順は配列そのものが正本（frequency による自動ソートは廃止）。
      body: normalizeBody(v.body),
      providers: Array.isArray(v.providers) ? v.providers.filter((p): p is string => typeof p === 'string').slice(0, 10) : [],
      tags: Array.isArray(v.tags) ? v.tags.filter((tag): tag is string => typeof tag === 'string').slice(0, 10) : [],
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
  templates.push({ body: normalized, providers: [], tags: [] });
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

// 並び順は保存配列の順（ユーザーがドラッグで決めた手動順）をそのまま使う。
// 使用頻度による自動ソートは行わない（勝手に順位が入れ替わらないことを優先）。
export function templatesForProvider(provider = activeProvider(), query = ''): PromptTemplate[] {
  const needle = query.trim().toLocaleLowerCase();
  return getPromptTemplates().filter((template) =>
    (!provider || template.providers.length === 0 || template.providers.includes(provider)) &&
    (!needle || `${template.body} ${template.tags.join(' ')}`.toLocaleLowerCase().includes(needle)),
  );
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
  // 選んだだけでは保存内容を変えない（頻度の記録をやめたので保存不要）。
  // send は選択時点の設定を焼き付けて渡す（受け手の app.ts 側で再判定しない）。
  window.dispatchEvent(new CustomEvent('many-ai-cli:insert-template', { detail: { text: template.body, send: isTemplateSendImmediate() } }));
}

type UndoState = { template: PromptTemplate; index: number };
let editingBody: string | null = null;
let editDraft = '';
let adding = false;
let addDraft = '';
let undoState: UndoState | null = null;
// 掴んでいる間の再描画抑止（renderPalette が行を作り直すと掴みが外れるため）。
let draggingRow: HTMLElement | null = null;
// 並べ替え直後につまみへフォーカスを戻す対象（キーボード操作の連打用）。
let focusHandleBody: string | null = null;

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

// --- 並べ替え -------------------------------------------------------------
// HTML5 dnd はタッチで発火しないので pointer events + pointer capture で自前実装する。
// 行の並びは DOM を正本として動かし、指を離した時点で保存配列へ書き戻す。

const AUTO_SCROLL_ZONE = 22;
const AUTO_SCROLL_STEP = 6;

function reorderRows(list: HTMLElement): HTMLElement[] {
  return [...list.querySelectorAll<HTMLElement>('.prompt-template-row[data-template-index]')];
}

// DOM の行順を保存配列へ書き戻す。表示中テンプレが元々占めていた絶対位置（slots）へ
// 並べ替え後の順で詰め直すので、provider 絞り込みで非表示の分は位置が動かない。
function commitTemplateOrder(list: HTMLElement): boolean {
  const order = reorderRows(list).map((row) => Number(row.dataset.templateIndex));
  if (order.some((index) => !Number.isInteger(index) || index < 0)) return false;
  const slots = [...order].sort((a, b) => a - b);
  if (slots.every((slot, i) => slot === order[i])) return false; // 並びに変化なし
  const templates = getPromptTemplates();
  // 別タブでの編集などで保存内容がずれていたら触らない（取り違え防止）。
  if (slots.length !== new Set(order).size || slots.some((slot) => slot >= templates.length)) return false;
  const picked = order.map((index) => templates[index]);
  slots.forEach((slot, i) => { templates[slot] = picked[i]; });
  save(templates);
  return true;
}

// 掴んでいる行を、隣の行と 1 つ入れ替える。入れ替えたら true。
function swapDraggedRowOnce(list: HTMLElement, row: HTMLElement, clientY: number): boolean {
  for (const other of reorderRows(list)) {
    if (other === row) continue;
    const rect = other.getBoundingClientRect();
    const middle = rect.top + rect.height / 2;
    const rowIsAfter = !!(other.compareDocumentPosition(row) & Node.DOCUMENT_POSITION_FOLLOWING);
    if (rowIsAfter && clientY < middle) { list.insertBefore(row, other); return true; }
    if (!rowIsAfter && clientY > middle) { list.insertBefore(row, other.nextSibling); return true; }
  }
  return false;
}

// 1 イベントで複数行ぶん飛んだ場合（タッチの粗いサンプリング等）に追随できるよう、
// 落ち着くまで入れ替えを繰り返す。行数を上限にして無限ループを防ぐ。
function moveDraggedRow(list: HTMLElement, row: HTMLElement, clientY: number): void {
  for (let guard = reorderRows(list).length; guard > 0; guard--) {
    if (!swapDraggedRowOnce(list, row, clientY)) return;
  }
}

function startReorder(event: PointerEvent, list: HTMLElement, row: HTMLElement, handle: HTMLElement): void {
  if (event.button > 0) return;
  event.preventDefault(); // タッチのスクロール・長押し選択を止める
  // 捕捉は取れれば取る（タッチの追随が安定する）が、取れなくても成立させる。
  // 終了イベントは window で受けるので、捕捉の有無に依存しない。
  try { handle.setPointerCapture(event.pointerId); } catch (_) { /* 捕捉なしで続行 */ }
  draggingRow = row;
  row.classList.add('is-dragging');
  let clientY = event.clientY;
  let frame = 0;

  // リスト端に留まっている間はスクロールし続ける（pointermove 頼みだと止まってしまう）。
  const tick = (): void => {
    frame = 0;
    const rect = list.getBoundingClientRect();
    const dir = clientY < rect.top + AUTO_SCROLL_ZONE ? -1 : clientY > rect.bottom - AUTO_SCROLL_ZONE ? 1 : 0;
    if (!dir) return;
    list.scrollTop += dir * AUTO_SCROLL_STEP;
    moveDraggedRow(list, row, clientY);
    frame = requestAnimationFrame(tick);
  };

  const onMove = (moveEvent: PointerEvent): void => {
    if (moveEvent.pointerId !== event.pointerId) return;
    clientY = moveEvent.clientY;
    moveDraggedRow(list, row, clientY);
    if (!frame) frame = requestAnimationFrame(tick);
  };

  // 終了は必ず 1 回だけ走らせる（pointerup と blur が続けて来ることがある）。
  let finished = false;
  const onEnd = (endEvent?: Event): void => {
    if (finished) return;
    if (endEvent instanceof PointerEvent && endEvent.pointerId !== event.pointerId) return;
    finished = true;
    window.removeEventListener('pointermove', onMove, true);
    window.removeEventListener('pointerup', onEnd, true);
    window.removeEventListener('pointercancel', onEnd, true);
    window.removeEventListener('blur', onEnd);
    if (frame) cancelAnimationFrame(frame);
    row.classList.remove('is-dragging');
    draggingRow = null;
    commitTemplateOrder(list);
  };

  // 掴み要素ではなく window で受ける。行を動かす insertBefore は「DOM から外して入れ直す」
  // 操作なので、掴み要素のポインタ捕捉がその時点で解放されることがある。掴み要素にだけ
  // 終了リスナを付けていると pointerup が届かず draggingRow が残り、renderPalette が
  // 二度と走らなくなる（削除も編集も画面に反映されなくなる）。
  window.addEventListener('pointermove', onMove, true);
  window.addEventListener('pointerup', onEnd, true);
  window.addEventListener('pointercancel', onEnd, true);
  window.addEventListener('blur', onEnd);
}

// つまみにフォーカスした状態の ↑ / ↓（ドラッグできない環境・支援技術向けの代替手段）。
function stepReorder(list: HTMLElement, row: HTMLElement, body: string, delta: number): boolean {
  const sibling = delta < 0 ? row.previousElementSibling : row.nextElementSibling;
  if (!(sibling instanceof HTMLElement) || sibling.dataset.templateIndex === undefined) return false;
  if (delta < 0) list.insertBefore(row, sibling);
  else list.insertBefore(row, sibling.nextSibling);
  focusHandleBody = body;
  // 保存されなかった場合はフォーカス指定を残さない（次の再描画で奪わないように）。
  if (!commitTemplateOrder(list)) focusHandleBody = null;
  return true;
}

function makeDragHandle(list: HTMLElement, row: HTMLElement, template: PromptTemplate): HTMLButtonElement {
  const handle = document.createElement('button');
  handle.type = 'button';
  handle.className = 'prompt-template-drag';
  handle.textContent = '⠿';
  const label = text('template_reorder', 'ドラッグで並べ替え');
  handle.title = label;
  handle.setAttribute('aria-label', label);
  handle.addEventListener('pointerdown', (event) => startReorder(event, list, row, handle));
  handle.addEventListener('keydown', (event) => {
    const delta = event.key === 'ArrowUp' ? -1 : event.key === 'ArrowDown' ? 1 : 0;
    if (!delta) return;
    event.preventDefault();
    event.stopPropagation();
    stepReorder(list, row, template.body, delta);
  });
  return handle;
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

function renderTemplateRow(row: HTMLElement, template: PromptTemplate, panel: HTMLElement, list: HTMLElement, reorderable: boolean): void {
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
  if (reorderable) {
    const handle = makeDragHandle(list, row, template);
    row.append(handle);
    if (focusHandleBody === template.body) {
      focusHandleBody = null;
      requestAnimationFrame(() => handle.focus());
    }
  }
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
  if (draggingRow) return; // ドラッグ中に作り直すと掴みが外れる
  const panel = document.getElementById('prompt-template-palette');
  const list = document.getElementById('prompt-template-list');
  const search = document.getElementById('prompt-template-search') as HTMLInputElement | null;
  if (!panel || !list || !search) return;
  const templates = templatesForProvider(undefined, search.value);
  // 検索で絞り込んでいる間は並べ替えさせない（見えていない行との前後関係が決められない）。
  const reorderable = !search.value.trim();
  const stored = getPromptTemplates();
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
    const index = reorderable ? templateIndex(stored, template.body) : -1;
    if (index >= 0) row.dataset.templateIndex = String(index);
    renderTemplateRow(row, template, panel, list, index >= 0);
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
