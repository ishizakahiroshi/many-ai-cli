// plan_memo-panel.md C3 — メモボタンと右ドロワー UI。
// 非モーダル: showModal() は使わず、開いたままターミナルを操作できる（data-wheel-native）。
// グループ化・並び順・件数の純粋ロジックは memo-model.ts にある。
import { t } from '../i18n.js';
import { uiButtonClass, uiIconHtml } from './ui-components.js';
import { apiFetch } from './util.js';
import { openLightbox } from './lightbox.js';
import { activeSessionId, sessions } from './state.js';
import { openSpawnPanelWith } from './spawn-panel.js';
import { basenameForPath, openFileModal, showPathPopup } from './path-links.js';
import { findPathCandidates } from './path-detect.js';
import {
  countOpenMemosForProject,
  cwdMatchesMemoProject,
  findOpenMemoByText,
  formatMemoTimestamp,
  groupMemosByProject,
  memoWasEdited,
  pruneMemoSelection,
  MEMO_IMAGE_MAX_BYTES,
  MEMO_IMAGES_PER_MEMO,
  memoPathType,
  MEMOS_CHANGED_EVENT,
  pickMemoImageFiles,
  registerMemoSaver,
  resolveMemoPathTarget,
  setSharedMemoCache,
  sortDoneMemos,
  sortOpenMemos,
  totalOpenMemoCount,
} from './memo-model.js';
import type { Memo } from './memo-model.js';
// 書き出し／取り込みの純粋ロジック（memo-model.js しか import しない葉モジュール）。
import {
  MEMO_EXPORT_FORMATS,
  buildMemoExport,
  memoExportFileName,
  parseMemoImport,
  planMemoImport,
  selectMemosForExport,
} from './memo-export.js';
import type { ImportItem, ImportTarget, MemoExportFormat, MemoExportLabels, MemoExportResult } from './memo-export.js';
// 別窓（plan_detached-tab-windows.md C4）。どちらも何も import しない葉モジュール。
import { isDetachedTabView, openDetachedTabWindow } from './detached-view-mode.js';
import { onWindowMessage, postWindowMessage, requestMainWindow } from './window-channel.js';
import { showToast, COPY_ICON, flashCopied } from './util.js';

// C4（バッジ）向けの通知。件数そのものは getOpenMemoCountForProject / totalOpenMemoCount を
// 都度呼んで取る（このイベントは「取り直せ」のシグナルであって件数を運ばない）。
// 定義本体は memo-model.ts（session-list.ts が循環 import なしに読めるようにするため。
// 同ファイルのコメント参照）。ここでは既存の import 元（memo-panel.ts）を保つため re-export する。
export { MEMOS_CHANGED_EVENT };

let cache: Memo[] = [];
let drawer: HTMLElement | null = null;
let listEl: HTMLElement;
let feedbackEl: HTMLElement;
let inputEl: HTMLTextAreaElement;
let subtitleEl: HTMLElement;
let loadGeneration = 0;
// 一括操作。選択モード中だけ、行の ☑ が「完了」ではなく「選択」を表す。
let selectMode = false;
let selected = new Set<string>();
let toolbarEl: HTMLElement;
// 書き出し（コピー）メニューと取り込みパネル。どちらもドロワーを組み立てるときに 1 度だけ作る
// 永続の要素で、renderList() が走ってもツールバーに付け直すだけ（開閉・入力の状態が消えない）。
// 「最後に選んだ形式」は端末ごとの好みなので localStorage の 1 キーだけで持つ（サーバ同期しない）。
const MEMO_EXPORT_FORMAT_KEY = 'ai_cli_hub_memo_export_format';
const MEMO_IMPORT_FILE_MAX_BYTES = 1024 * 1024;
let exportFormat: MemoExportFormat = loadExportFormat();
let exportIncludeDone = false;
let exportWrapEl: HTMLElement | null = null;
let exportButtonEl: HTMLButtonElement | null = null;
let exportMenuEl: HTMLElement | null = null;
let exportSummaryEl: HTMLElement | null = null;
let exportDoneCheckEl: HTMLInputElement | null = null;
let exportCopyBtn: HTMLButtonElement | null = null;
let exportSaveBtn: HTMLButtonElement | null = null;
let importPanelEl: HTMLElement | null = null;
let importTextEl: HTMLTextAreaElement | null = null;
let importResultEl: HTMLElement | null = null;
let importRunBtn: HTMLButtonElement | null = null;
let importing = false;
// 入力欄に貼り付け・ドロップした画像（Hub へ保存済みの画像名）。メモを追加すると空になる。
// × で外した画像や、追加せずに終わった画像は Hub が次回起動時に回収する（memo_images.go）。
let pendingImages: string[] = [];
let pendingImagesEl: HTMLElement;
// 送信中のアップロード。Enter が先に押されても、終わるのを待ってからメモを作る。
const inflightUploads = new Set<Promise<void>>();

function memoImageUrl(name: string): string {
  // Cookie (MANY_AI_CLI_token) is enough for same-origin <img>; avoid ?token= in HTML/URL bar.
  return `/api/memo-images/${encodeURIComponent(name)}`;
}

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = '', className = ''): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  el.textContent = text;
  el.className = tag === 'button' ? `${uiButtonClass(className.includes('delete') ? 'danger' : className.includes('launch') ? 'primary' : 'quiet', 'compact', { icon: !className.includes('launch') })} ${className}` : className;
  return el;
}

function notifyMemosChanged(): void {
  // session-list.ts のバッジは memo-model.ts の共有キャッシュだけを読むので、
  // イベントを出す前に必ず書き込む（読む側がイベントを受けた時点で古い値を掴まない）。
  setSharedMemoCache(cache);
  updateMemoButtonBadge();
  window.dispatchEvent(new CustomEvent(MEMOS_CHANGED_EVENT));
}

// 自分の窓で足した・直した・消したときだけ、ほかの窓へ「取り直して」と伝える。
// refresh() からは呼ばない（受けた窓が取り直すたびに流し返すと往復が止まらない）。
function announceMemosChangedToOtherWindows(): void {
  postWindowMessage({ type: 'memos-changed' });
}

function ensureMemoButtonBadge(): HTMLElement | null {
  if (typeof document === 'undefined') return null;
  const btn = document.querySelector<HTMLElement>('[data-open-memo]');
  if (!btn) return null;
  let badge = btn.querySelector<HTMLElement>('.memo-button-badge');
  if (!badge) {
    badge = document.createElement('span');
    badge.className = 'memo-button-badge';
    badge.setAttribute('aria-hidden', 'true');
    badge.hidden = true; // 件数が分かるまでは出さない（0 件の空バッジを一瞬見せない）
    btn.appendChild(badge);
  }
  return badge;
}

/** C4: メモボタン自身のバッジ（全プロジェクト合計の未完了件数。0 件なら隠す）。 */
function updateMemoButtonBadge(): void {
  const badge = ensureMemoButtonBadge();
  if (!badge) return;
  const count = totalOpenMemoCount();
  badge.hidden = count <= 0;
  badge.textContent = count > 99 ? '99+' : String(count);
  if (count > 0) badge.title = t('memo_open_count_badge', { count });
  else badge.removeAttribute('title');
}

/** C4 のバッジ（セッション一覧の行・メモボタン自身）が使う件数取得。 */
export function getOpenMemoCountForProject(project: string): number {
  return countOpenMemosForProject(cache, project);
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await apiFetch(path, init);
  if (!response.ok) {
    const raw = await response.text();
    let detail = '';
    let code = '';
    try {
      const error = JSON.parse(raw);
      if (typeof error.error === 'string') code = error.error;
      if (typeof error.detail === 'string') detail = error.detail.slice(0, 350);
    } catch (_) { /* プロキシのエラーページはダイアログに出しても分からない。 */ }
    // 画像の容量上限は利用者が対処できる（不要なメモを消す）ので、言語に合わせた文で出す。
    if (code === 'memo_images_full') throw new Error(t('memo_images_full'));
    throw new Error(`${t('memo_request_failed')} (${response.status})${detail ? ' ' + detail : ''}`);
  }
  return await response.json() as T;
}
const body = (method: string, data: unknown): RequestInit =>
  ({ method, headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(data) });

function activeProject(): string {
  if (activeSessionId === null) return '';
  return sessions.get(activeSessionId)?.project_id || '';
}

function showFeedback(error: unknown): void {
  feedbackEl.textContent = error instanceof Error ? error.message : t('memo_request_failed');
}

function renderPendingImages(): void {
  if (!pendingImagesEl) return;
  pendingImagesEl.replaceChildren();
  pendingImagesEl.hidden = pendingImages.length === 0;
  for (const name of pendingImages) {
    const item = node('div', '', 'memo-image-thumb memo-image-thumb-pending');
    const img = document.createElement('img');
    img.src = memoImageUrl(name);
    img.alt = '';
    img.addEventListener('click', () => openLightbox(img.src));
    const remove = node('button', '✕', 'memo-image-remove');
    remove.type = 'button';
    remove.title = t('memo_image_remove');
    remove.setAttribute('aria-label', t('memo_image_remove'));
    remove.addEventListener('click', () => {
      pendingImages = pendingImages.filter((n) => n !== name);
      renderPendingImages();
      inputEl.focus();
    });
    item.append(img, remove);
    pendingImagesEl.append(item);
  }
}

/** 貼り付け・ドロップした画像を Hub へ保存し、入力欄の下にサムネイルで並べる。 */
function addPendingImageFiles(files: File[]): void {
  feedbackEl.textContent = '';
  for (const file of files) {
    if (pendingImages.length + inflightUploads.size >= MEMO_IMAGES_PER_MEMO) {
      feedbackEl.textContent = t('memo_image_limit', { count: MEMO_IMAGES_PER_MEMO });
      return;
    }
    if (file.size > MEMO_IMAGE_MAX_BYTES) {
      feedbackEl.textContent = t('memo_image_too_large');
      continue;
    }
    const upload = (async () => {
      try {
        const { image } = await request<{ image: string }>('/api/memo-images',
          { method: 'POST', headers: { 'Content-Type': file.type }, body: file });
        pendingImages.push(image);
        renderPendingImages();
      } catch (error) { showFeedback(error); }
    })();
    inflightUploads.add(upload);
    void upload.finally(() => inflightUploads.delete(upload));
  }
}

function renderMemoImages(container: HTMLElement, images: readonly string[]): void {
  const wrap = node('div', '', 'memo-row-images');
  for (const name of images) {
    const item = node('div', '', 'memo-image-thumb');
    const img = document.createElement('img');
    img.src = memoImageUrl(name);
    img.alt = '';
    img.loading = 'lazy';
    img.title = t('memo_image_open');
    img.addEventListener('click', (e) => { e.stopPropagation(); openLightbox(img.src); });
    item.append(img);
    wrap.append(item);
  }
  container.append(wrap);
}

// appendLinkedText（path-links.ts）はライブセッションの cwd で相対パスを解決するが、
// メモの時点ではセッションが終わっていることが多い。memo.project を基点に解決し直す。
function renderMemoText(container: HTMLElement, text: string, project: string): void {
  const candidates = findPathCandidates(text);
  if (candidates.length === 0) { container.textContent = text; return; }
  container.textContent = '';
  // 右クリック・ContextMenu キー・Shift+F10 は CLI 画面と同じメニュー（showPathPopup）。
  // 「作業メモに保存」は出さない（すでにメモの中のパス）。別窓では本体の窓のファイルタブを開けないので
  // 「MANY-AI-CLI で開く」も出さない。左クリック・Enter・Space はこれまでどおりプレビュー。
  const menuContext = {
    filesRoot: project || undefined,
    projectKey: project ? basenameForPath(project) : undefined,
    relativeBase: project || undefined,
    hideSaveToMemo: true,
    hideOpenInApp: isDetachedTabView(),
  };
  let pos = 0;
  for (const c of candidates) {
    if (c.start > pos) container.appendChild(document.createTextNode(text.slice(pos, c.start)));
    const resolved = resolveMemoPathTarget(c.text, project);
    const pathType = memoPathType(resolved);
    const link = document.createElement('span');
    link.className = 'tool-output-path-link';
    link.textContent = c.text;
    link.tabIndex = 0;
    const openIt = (): void => { openFileModal(resolved, activeSessionId, project); };
    const openMenu = (x: number, y: number): void => {
      showPathPopup(resolved, x, y, activeSessionId, pathType, [], menuContext);
    };
    link.addEventListener('click', (e) => { e.preventDefault(); e.stopPropagation(); openIt(); });
    link.addEventListener('contextmenu', (e) => { e.preventDefault(); e.stopPropagation(); openMenu(e.clientX, e.clientY); });
    link.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openIt(); }
      else if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
        e.preventDefault();
        const r = link.getBoundingClientRect();
        openMenu(r.left, r.bottom);
      }
    });
    container.appendChild(link);
    pos = c.end;
  }
  if (pos < text.length) container.appendChild(document.createTextNode(text.slice(pos)));
}

async function refresh(): Promise<void> {
  const epoch = ++loadGeneration;
  feedbackEl.textContent = t('memo_loading');
  try {
    const { memos } = await request<{ memos: Memo[] }>('/api/memos');
    if (epoch !== loadGeneration) return;
    cache = memos;
    selected = pruneMemoSelection(selected, cache);
    feedbackEl.textContent = '';
    notifyMemosChanged();
    renderList();
  } catch (error) {
    if (epoch === loadGeneration) showFeedback(error);
  }
}

function applyUpdate(updated: Memo): void {
  const idx = cache.findIndex((m) => m.id === updated.id);
  if (idx >= 0) cache[idx] = updated; else cache.push(updated);
  notifyMemosChanged();
  announceMemosChangedToOtherWindows();
  renderList();
}

async function toggleDone(memo: Memo, done: boolean): Promise<void> {
  try {
    const { memo: updated } = await request<{ memo: Memo }>(
      `/api/memos/${encodeURIComponent(memo.id)}`, body('PATCH', { done }));
    applyUpdate(updated);
  } catch (error) { showFeedback(error); await refresh(); }
}

async function updateText(memo: Memo, text: string): Promise<void> {
  try {
    const { memo: updated } = await request<{ memo: Memo }>(
      `/api/memos/${encodeURIComponent(memo.id)}`, body('PATCH', { text }));
    applyUpdate(updated);
  } catch (error) { showFeedback(error); await refresh(); }
}

async function removeMemo(memo: Memo): Promise<void> {
  try {
    await request(`/api/memos/${encodeURIComponent(memo.id)}`, { method: 'DELETE' });
    cache = cache.filter((m) => m.id !== memo.id);
    notifyMemosChanged();
    announceMemosChangedToOtherWindows();
    renderList();
  } catch (error) { showFeedback(error); }
}

function setSelected(ids: readonly string[], on: boolean): void {
  for (const id of ids) { if (on) selected.add(id); else selected.delete(id); }
  renderList();
}

function setSelectMode(on: boolean): void {
  selectMode = on;
  selected = new Set();
  renderList();
}

// 選択したメモをまとめて消す。1 件ずつ DELETE を送り、成功した分だけ画面から外す
// （途中で失敗しても、消えたものと残ったものが画面と Hub で食い違わない）。
async function bulkDelete(): Promise<void> {
  const ids = Array.from(selected);
  const results = await Promise.allSettled(
    ids.map((id) => request(`/api/memos/${encodeURIComponent(id)}`, { method: 'DELETE' })));
  const gone = new Set(ids.filter((_, i) => results[i].status === 'fulfilled'));
  cache = cache.filter((m) => !gone.has(m.id));
  for (const id of gone) selected.delete(id);
  const failed = ids.length - gone.size;
  if (failed > 0) feedbackEl.textContent = t('memo_bulk_failed', { count: failed });
  notifyMemosChanged();
  announceMemosChangedToOtherWindows();
  renderList();
}

async function bulkMarkDone(): Promise<void> {
  const targets = cache.filter((m) => selected.has(m.id) && !m.done);
  const results = await Promise.allSettled(targets.map((m) =>
    request<{ memo: Memo }>(`/api/memos/${encodeURIComponent(m.id)}`, body('PATCH', { done: true }))));
  let failed = 0;
  results.forEach((r) => {
    if (r.status === 'fulfilled') {
      const idx = cache.findIndex((m) => m.id === r.value.memo.id);
      if (idx >= 0) cache[idx] = r.value.memo;
    } else failed++;
  });
  selected = new Set();
  if (failed > 0) feedbackEl.textContent = t('memo_bulk_failed', { count: failed });
  notifyMemosChanged();
  announceMemosChangedToOtherWindows();
  renderList();
}

function toolbarBtn(label: string, variant: 'quiet' | 'danger' = 'quiet'): HTMLButtonElement {
  const b = document.createElement('button');
  b.type = 'button';
  b.textContent = label;
  b.className = `${uiButtonClass(variant, 'compact')} memo-toolbar-btn`;
  return b;
}

// ── 書き出し（コピー・ファイル保存）と JSON 取り込み ─────────────────────────

function loadExportFormat(): MemoExportFormat {
  try {
    const raw = localStorage.getItem(MEMO_EXPORT_FORMAT_KEY);
    const found = MEMO_EXPORT_FORMATS.find((f) => f === raw);
    if (found) return found;
  } catch (_) { /* localStorage が使えなくても既定で動く */ }
  return 'markdown';
}

function saveExportFormat(format: MemoExportFormat): void {
  try { localStorage.setItem(MEMO_EXPORT_FORMAT_KEY, format); } catch (_) { /* 覚えられないだけ */ }
}

const EXPORT_FORMAT_LABEL_KEYS: Record<MemoExportFormat, string> = {
  markdown: 'memo_export_format_markdown',
  text: 'memo_export_format_text',
  json: 'memo_export_format_json',
};

/** 書き出す対象。選択モードで 1 件以上選んでいればその分だけ、それ以外は未完了（＋チェック時は完了済み）。 */
function exportTargets(): Memo[] {
  return selectMemosForExport(cache, selectMode ? selected : new Set<string>(), exportIncludeDone);
}

function updateExportSummary(): void {
  if (!exportSummaryEl) return;
  const targets = exportTargets();
  const useSelection = selectMode && selected.size > 0;
  if (targets.length === 0) exportSummaryEl.textContent = t('memo_export_empty');
  else exportSummaryEl.textContent = t(useSelection ? 'memo_export_summary_selected' : 'memo_export_summary', { count: targets.length });
  if (exportCopyBtn) exportCopyBtn.disabled = targets.length === 0;
  if (exportSaveBtn) exportSaveBtn.disabled = targets.length === 0;
  // 選んだメモを書き出すときは、完了済みを含めるかどうかは効かない。
  if (exportDoneCheckEl) exportDoneCheckEl.disabled = useSelection;
}

function setExportMenuOpen(open: boolean): void {
  if (!exportMenuEl || !exportButtonEl) return;
  exportMenuEl.hidden = !open;
  exportButtonEl.setAttribute('aria-expanded', String(open));
  if (open) updateExportSummary();
}

function buildCurrentExport(): MemoExportResult | null {
  const targets = exportTargets();
  if (targets.length === 0) return null;
  const labels: MemoExportLabels = {
    title: (count, date) => t('memo_export_doc_title', { count, date }),
    unclassified: t('memo_group_unclassified'),
    images: (count) => t('memo_export_images', { count }),
    doneMark: t('memo_export_done_mark'),
  };
  return buildMemoExport(targets, exportFormat, labels, { activeProject: activeProject(), now: new Date() });
}

async function copyExport(): Promise<void> {
  const result = buildCurrentExport();
  if (!result) { updateExportSummary(); return; }
  try {
    if (!navigator.clipboard || typeof navigator.clipboard.writeText !== 'function') throw new Error('clipboard unavailable');
    await navigator.clipboard.writeText(result.content);
  } catch (_) {
    // 失敗を成功と見せない。メニューは開いたままにして、ファイル保存へ切り替えられるようにする。
    showToast(t('memo_export_copy_failed'), undefined, 5000);
    return;
  }
  setExportMenuOpen(false);
  showToast(t('memo_export_copied', { count: result.count }), undefined, 2500);
}

function saveExport(): void {
  const result = buildCurrentExport();
  if (!result) { updateExportSummary(); return; }
  const url = URL.createObjectURL(new Blob([result.content], { type: result.mime }));
  const link = document.createElement('a');
  link.href = url;
  link.download = memoExportFileName(exportFormat);
  link.hidden = true;
  document.body.append(link);
  link.click();
  link.remove();
  // ダウンロードの開始に間に合うよう、少し待ってから解放する。
  window.setTimeout(() => URL.revokeObjectURL(url), 10_000);
  setExportMenuOpen(false);
  showToast(t('memo_export_saved', { count: result.count }), undefined, 2500);
}

/** ボタンとメニューを作る（1 度だけ）。ツールバーが作り直されても、この要素を付け直して使う。 */
function buildExportControls(): HTMLElement {
  if (exportWrapEl) return exportWrapEl;
  const wrap = node('div', '', 'memo-export-wrap');
  const button = toolbarBtn(`${COPY_ICON} ${t('memo_export_button')} ▾`);
  button.title = t('memo_export_button_title');
  button.setAttribute('aria-haspopup', 'true');
  button.setAttribute('aria-expanded', 'false');
  button.addEventListener('click', () => setExportMenuOpen(Boolean(exportMenuEl?.hidden)));

  const menu = node('div', '', 'memo-export-menu');
  menu.hidden = true;
  menu.setAttribute('role', 'group');
  menu.setAttribute('aria-label', t('memo_export_button_title'));

  const formatRow = node('div', '', 'memo-export-row');
  formatRow.append(node('span', t('memo_export_format')));
  const formats = node('div', '', 'memo-export-formats');
  for (const format of MEMO_EXPORT_FORMATS) {
    const label = document.createElement('label');
    const radio = document.createElement('input');
    radio.type = 'radio';
    radio.name = 'memo-export-format';
    radio.value = format;
    radio.checked = format === exportFormat;
    radio.addEventListener('change', () => {
      if (!radio.checked) return;
      exportFormat = format;
      saveExportFormat(format);
      updateExportSummary();
    });
    label.append(radio, document.createTextNode(` ${t(EXPORT_FORMAT_LABEL_KEYS[format])}`));
    formats.append(label);
  }
  formatRow.append(formats);

  const doneRow = node('div', '', 'memo-export-row');
  const doneLabel = document.createElement('label');
  const doneCheck = document.createElement('input');
  doneCheck.type = 'checkbox';
  doneCheck.checked = exportIncludeDone;
  doneCheck.addEventListener('change', () => { exportIncludeDone = doneCheck.checked; updateExportSummary(); });
  doneLabel.append(doneCheck, document.createTextNode(` ${t('memo_export_include_done')}`));
  doneRow.append(doneLabel);

  const summary = node('p', '', 'memo-export-summary');
  summary.setAttribute('role', 'status');

  const actions = node('div', '', 'memo-export-actions');
  const copyBtn = toolbarBtn(t('memo_export_copy'));
  copyBtn.addEventListener('click', () => void copyExport());
  const saveBtn = toolbarBtn(t('memo_export_save'));
  saveBtn.addEventListener('click', () => saveExport());
  actions.append(copyBtn, saveBtn);

  const sep = node('div', '', 'memo-export-sep');
  const openImport = document.createElement('button');
  openImport.type = 'button';
  openImport.textContent = t('memo_import_open');
  openImport.className = `${uiButtonClass('quiet', 'compact', { menuItem: true })} memo-import-open`;
  openImport.addEventListener('click', () => openImportPanel());

  menu.append(formatRow, doneRow, summary, actions, sep, openImport);
  wrap.append(button, menu);

  exportWrapEl = wrap;
  exportButtonEl = button;
  exportMenuEl = menu;
  exportSummaryEl = summary;
  exportDoneCheckEl = doneCheck;
  exportCopyBtn = copyBtn;
  exportSaveBtn = saveBtn;

  // 外側クリックと Esc で閉じる。ドロワーは別窓（detached view）でも同じ document の中にある。
  document.addEventListener('click', (event) => {
    if (menu.hidden) return;
    if (event.target instanceof Node && wrap.contains(event.target)) return;
    setExportMenuOpen(false);
  });
  document.addEventListener('keydown', (event) => {
    if (event.key !== 'Escape' || menu.hidden) return;
    const hadFocus = wrap.contains(document.activeElement);
    setExportMenuOpen(false);
    if (hadFocus) button.focus();
  });
  return wrap;
}

function setImportResult(text: string, kind: '' | 'ok' | 'error' = ''): void {
  if (!importResultEl) return;
  importResultEl.textContent = text;
  importResultEl.classList.toggle('is-ok', kind === 'ok');
  importResultEl.classList.toggle('is-error', kind === 'error');
}

function openImportPanel(): void {
  if (!importPanelEl) return;
  setExportMenuOpen(false);
  importPanelEl.hidden = false;
  importTextEl?.focus();
}

/** 取り込み先を決める。同じフォルダで開いているセッション（id の小さいもの）があればその project、無ければ未分類。 */
function resolveImportTarget(item: ImportItem): ImportTarget {
  if (item.project !== '') {
    const ids = Array.from(sessions.keys()).sort((a, b) => a - b);
    for (const id of ids) {
      const cwd = sessions.get(id)?.cwd ?? '';
      if (cwdMatchesMemoProject(cwd, item.project)) {
        return { sessionId: id, belongs: (p) => cwdMatchesMemoProject(cwd, p) };
      }
    }
  }
  return { sessionId: null, belongs: (p) => p === '' };
}

async function runImport(): Promise<void> {
  if (importing || !importTextEl) return;
  setImportResult('');
  const parsed = parseMemoImport(importTextEl.value);
  if (!parsed.ok) { setImportResult(t('memo_import_invalid'), 'error'); return; }
  if (parsed.items.length === 0) { setImportResult(t('memo_import_none'), 'error'); return; }
  const plan = planMemoImport(parsed.items, cache, resolveImportTarget);
  importing = true;
  if (importRunBtn) importRunBtn.disabled = true;
  let created = 0;
  let failed = 0;
  try {
    // 作成順を保つため、1 件ずつ順に送る。1 件でも失敗したら（件数上限など）残りは打ち切って失敗に数える。
    for (let i = 0; i < plan.create.length; i++) {
      const { item, sessionId } = plan.create[i];
      try {
        const payload: NewMemoPayload = { text: item.text };
        if (sessionId !== null) payload.session_id = sessionId;
        const { memo } = await request<{ memo: Memo }>('/api/memos', body('POST', payload));
        cache.push(memo);
        created++;
        if (item.done) {
          try {
            const { memo: updated } = await request<{ memo: Memo }>(
              `/api/memos/${encodeURIComponent(memo.id)}`, body('PATCH', { done: true }));
            const idx = cache.findIndex((m) => m.id === updated.id);
            if (idx >= 0) cache[idx] = updated;
          } catch (_) { /* 作成はできている。完了にできなかっただけなので未完了のまま残す */ }
        }
      } catch (_) {
        failed += plan.create.length - i;
        break;
      }
    }
  } finally {
    importing = false;
    if (importRunBtn) importRunBtn.disabled = false;
    // 途中で失敗しても、作れた分は必ず画面・バッジ・ほかの窓へ反映する。
    if (created > 0) {
      notifyMemosChanged();
      announceMemosChangedToOtherWindows();
    }
    renderList();
  }
  setImportResult(t('memo_import_result', { created, duplicates: plan.duplicates, failed }), failed > 0 ? 'error' : 'ok');
  if (failed === 0) importTextEl.value = '';
}

function buildImportPanel(): HTMLElement {
  if (importPanelEl) return importPanelEl;
  const panel = node('div', '', 'memo-import-panel');
  panel.hidden = true;

  const title = document.createElement('strong');
  title.textContent = t('memo_import_title');
  const hint = node('p', t('memo_import_hint'), 'memo-plaintext-note');

  const textarea = document.createElement('textarea');
  textarea.className = 'ui-input memo-import-textarea';
  textarea.rows = 6;
  textarea.placeholder = t('memo_import_placeholder');
  textarea.spellcheck = false;

  const fileInput = document.createElement('input');
  fileInput.type = 'file';
  fileInput.accept = '.json,application/json';
  fileInput.hidden = true;
  fileInput.addEventListener('change', () => {
    const file = fileInput.files?.[0];
    fileInput.value = ''; // 同じファイルをもう一度選べるようにする
    if (!file) return;
    if (file.size > MEMO_IMPORT_FILE_MAX_BYTES) { setImportResult(t('memo_import_invalid'), 'error'); return; }
    file.text().then(
      (text) => { textarea.value = text; setImportResult(''); },
      () => setImportResult(t('memo_import_invalid'), 'error'),
    );
  });

  const actions = node('div', '', 'memo-import-actions');
  const pick = toolbarBtn(t('memo_import_file'));
  pick.addEventListener('click', () => fileInput.click());
  const run = document.createElement('button');
  run.type = 'button';
  run.textContent = t('memo_import_run');
  run.className = `${uiButtonClass('primary', 'compact')} memo-toolbar-btn`;
  run.addEventListener('click', () => void runImport());
  const close = toolbarBtn(t('memo_import_close'));
  close.addEventListener('click', () => { panel.hidden = true; });
  actions.append(pick, run, close, fileInput);

  const result = node('p', '', 'memo-import-result');
  result.setAttribute('role', 'status');

  panel.append(title, hint, textarea, actions, result);
  importPanelEl = panel;
  importTextEl = textarea;
  importResultEl = result;
  importRunBtn = run;
  return panel;
}

function renderToolbar(): void {
  if (!toolbarEl) return;
  toolbarEl.replaceChildren();
  // 0 件でも取り込みは使えるので、ツールバーは常に出す（0 件のときは「コピー ▾」だけ）。
  toolbarEl.hidden = false;
  const exportControls = exportWrapEl;
  if (!selectMode || cache.length === 0) {
    if (cache.length > 0) {
      const enter = toolbarBtn(t('memo_select_mode'));
      enter.type = 'button';
      enter.addEventListener('click', () => setSelectMode(true));
      toolbarEl.append(enter);
    }
    if (exportControls) toolbarEl.append(exportControls);
    updateExportSummary();
    return;
  }
  const allIds = cache.map((m) => m.id);
  const allOn = allIds.length > 0 && allIds.every((id) => selected.has(id));
  const selectAll = toolbarBtn(allOn ? t('memo_select_none') : t('memo_select_all'));
  selectAll.type = 'button';
  selectAll.addEventListener('click', () => setSelected(allIds, !allOn));
  const count = node('span', t('memo_selected_count', { count: selected.size }), 'memo-toolbar-count');
  const done = toolbarBtn(t('memo_bulk_done'));
  done.type = 'button';
  done.disabled = !cache.some((m) => selected.has(m.id) && !m.done);
  done.addEventListener('click', () => void bulkMarkDone());
  // 行の 🗑 と同じ 2 段階。1 回目で「N 件を本当に削除」に変わり、もう一度押すと消す。
  const del = toolbarBtn(t('memo_bulk_delete'), 'danger');
  del.type = 'button';
  del.disabled = selected.size === 0;
  let armTimer: number | undefined;
  const disarm = (): void => {
    window.clearTimeout(armTimer);
    del.classList.remove('memo-row-delete-armed');
    del.textContent = t('memo_bulk_delete');
  };
  del.addEventListener('click', () => {
    if (del.classList.contains('memo-row-delete-armed')) { disarm(); void bulkDelete(); return; }
    del.classList.add('memo-row-delete-armed');
    del.textContent = t('memo_bulk_delete_confirm', { count: selected.size });
    armTimer = window.setTimeout(disarm, 3000);
  });
  del.addEventListener('blur', disarm);
  const exit = toolbarBtn(t('memo_select_exit'));
  exit.type = 'button';
  exit.addEventListener('click', () => setSelectMode(false));
  toolbarEl.append(selectAll, count);
  if (exportControls) toolbarEl.append(exportControls);
  toolbarEl.append(done, del, exit);
  updateExportSummary();
}

function beginEdit(memo: Memo, rowBody: HTMLElement, textEl: HTMLElement): void {
  if (rowBody.querySelector('textarea')) return;
  const textarea = document.createElement('textarea');
  textarea.className = 'ui-input memo-row-edit';
  textarea.value = memo.text;
  textEl.hidden = true;
  rowBody.appendChild(textarea);
  textarea.focus();
  textarea.setSelectionRange(textarea.value.length, textarea.value.length);
  let settled = false;
  const finish = (commit: boolean): void => {
    if (settled) return;
    settled = true;
    textarea.remove();
    textEl.hidden = false;
    if (commit) {
      const next = textarea.value.trim();
      // 画像を持つメモは本文を空にしてよい（画像だけのメモとして残る）。
      const allowEmpty = (memo.images?.length ?? 0) > 0;
      if ((next || allowEmpty) && next !== memo.text) void updateText(memo, next);
    }
  };
  textarea.addEventListener('keydown', (e) => {
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === 'Escape') { e.preventDefault(); finish(false); }
    else if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); finish(true); }
  });
  textarea.addEventListener('blur', () => finish(true));
}

function buildMemoRow(memo: Memo, project: string): HTMLElement {
  const row = node('div', '', 'memo-row' + (memo.done ? ' memo-row-done' : ''));
  const checkbox = document.createElement('input');
  checkbox.type = 'checkbox';
  checkbox.className = 'memo-row-check';
  if (selectMode) {
    checkbox.checked = selected.has(memo.id);
    checkbox.setAttribute('aria-label', t('memo_select_row'));
    checkbox.addEventListener('change', () => setSelected([memo.id], checkbox.checked));
    if (checkbox.checked) row.classList.add('memo-row-selected');
  } else {
    checkbox.checked = memo.done;
    checkbox.setAttribute('aria-label', t('memo_mark_done'));
    checkbox.addEventListener('change', () => void toggleDone(memo, checkbox.checked));
  }

  const rowBody = node('div', '', 'memo-row-body');
  const textEl = node('div', '', 'memo-row-text');
  textEl.tabIndex = 0;
  textEl.title = t('memo_edit_hint');
  renderMemoText(textEl, memo.text, project);
  // 本文の 1 回クリックで編集に入る。パスの青い文字（左クリックはプレビュー）と、
  // 文字を選択するためのドラッグ（選択が残っている）では編集に入れない。
  textEl.addEventListener('click', (e) => {
    if ((e.target as Element | null)?.closest?.('.tool-output-path-link')) return;
    if (window.getSelection()?.toString()) return;
    // 選択モード中は本文のクリックで選択を切り替える（編集には入らない）。
    if (selectMode) { setSelected([memo.id], !selected.has(memo.id)); return; }
    beginEdit(memo, rowBody, textEl);
  });
  rowBody.append(textEl);
  // 作成日時と更新日時（本文を直したときだけ更新を足す）。完了済みは完了日時も出す。
  const stamps = [`${t('memo_created')} ${formatMemoTimestamp(memo.created_at)}`];
  if (memoWasEdited(memo)) stamps.push(`${t('memo_updated')} ${formatMemoTimestamp(memo.updated_at)}`);
  if (memo.done && memo.done_at) stamps.push(`${t('memo_completed')} ${formatMemoTimestamp(memo.done_at)}`);
  if (formatMemoTimestamp(memo.created_at)) {
    const meta = node('div', stamps.join(' ・ '), 'memo-row-meta');
    meta.title = [memo.created_at, memo.updated_at].filter(Boolean).join(' / ');
    rowBody.append(meta);
  }
  if (memo.images && memo.images.length > 0) {
    renderMemoImages(rowBody, memo.images);
    // 画像だけのメモは本文が空。クリックで後から一言足せるよう、薄い案内を出す。
    if (!memo.text) {
      textEl.classList.add('memo-row-text-empty');
      textEl.textContent = t('memo_image_add_note');
    }
  }

  // 操作ボタンは横 1 列: ✎（編集）→ 📋（コピー）→ ▶ 起動 → 🗑（削除）。
  const actions = node('div', '', 'memo-row-actions');
  const editBtn = node('button', '✎', 'memo-row-edit-btn');
  editBtn.type = 'button';
  editBtn.title = t('memo_edit');
  editBtn.setAttribute('aria-label', t('memo_edit'));
  editBtn.innerHTML = uiIconHtml('edit');
  editBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    beginEdit(memo, rowBody, textEl);
  });
  actions.append(editBtn);
  const launch = node('button', `▶ ${t('memo_launch_short')}`, 'memo-row-launch');
  launch.type = 'button';
  launch.title = t('memo_launch');
  launch.setAttribute('aria-label', t('memo_launch'));
  launch.addEventListener('click', (e) => {
    e.stopPropagation();
    if (isDetachedTabView()) {
      // 別窓では起動画面（サイドバーの中にある）を隠しているので、本体の窓で開いてもらう。
      void requestMainWindow({ type: 'open-spawn', cwd: memo.project, prompt: memo.text }).then((ok) => {
        if (!ok) showToast(t('detached_main_window_missing'), undefined, 4000);
      });
      return;
    }
    openSpawnPanelWith({ cwd: memo.project, prompt: memo.text });
  });
  const del = node('button', '🗑', 'memo-row-delete');
  del.type = 'button';
  del.title = t('memo_delete');
  del.setAttribute('aria-label', t('memo_delete'));
  del.innerHTML = uiIconHtml('delete');
  // ブラウザ標準の確認ダイアログは使わない（画面の隅に出て、押した場所から遠い）。
  // 1 回目の押下でボタン自身が「本当に削除」に変わり、同じ位置をもう一度押すと消える。
  // 3 秒放置・フォーカス移動・マウスが離れたら元に戻す。
  let armTimer: number | undefined;
  const disarm = (): void => {
    window.clearTimeout(armTimer);
    del.classList.remove('memo-row-delete-armed');
    del.innerHTML = uiIconHtml('delete');
  };
  del.addEventListener('click', (e) => {
    e.stopPropagation();
    if (del.classList.contains('memo-row-delete-armed')) { disarm(); void removeMemo(memo); return; }
    del.classList.add('memo-row-delete-armed');
    del.textContent = t('memo_delete_confirm_now');
    armTimer = window.setTimeout(disarm, 3000);
  });
  del.addEventListener('blur', disarm);
  del.addEventListener('mouseleave', disarm);
  // 本文のコピー（アプリ共通の 📋 → ✓）。画像だけのメモは本文が無いので出さない。
  if (memo.text) {
    const copy = node('button', COPY_ICON, 'memo-row-copy');
    copy.type = 'button';
    copy.title = t('copy_to_clipboard');
    copy.setAttribute('aria-label', t('copy_to_clipboard'));
    copy.addEventListener('click', (e) => {
      e.stopPropagation();
      navigator.clipboard.writeText(memo.text).then(
        () => flashCopied(copy),
        () => showToast(t('settings_doctor_copy_failed'), undefined, 3000),
      );
    });
    actions.append(copy);
  }
  actions.append(launch, del);

  // 選択モード中は行の操作ボタンを隠す（誤って 1 件だけ消す・起動するのを防ぐ）。
  row.append(checkbox, rowBody);
  if (!selectMode) row.append(actions);
  return row;
}

function renderList(): void {
  if (!listEl) return;
  listEl.replaceChildren();
  renderToolbar();
  const project = activeProject();
  const groups = groupMemosByProject(cache, project);
  if (groups.length === 0) {
    listEl.append(node('p', t('memo_empty'), 'memo-empty'));
    return;
  }
  for (const group of groups) {
    const section = node('section', '', 'memo-group');
    const heading = node('div', '', 'memo-group-heading');
    const label = node('strong', group.project ? basenameForPath(group.project) : t('memo_group_unclassified'));
    if (group.project) label.title = group.project;
    if (selectMode) {
      // グループ見出しの ☑ で、そのプロジェクトのメモ（完了済みも）をまとめて選ぶ。
      const ids = [...group.open, ...group.done].map((m) => m.id);
      const groupCheck = document.createElement('input');
      groupCheck.type = 'checkbox';
      groupCheck.className = 'memo-group-check';
      groupCheck.checked = ids.length > 0 && ids.every((id) => selected.has(id));
      groupCheck.setAttribute('aria-label', t('memo_select_group'));
      groupCheck.addEventListener('change', () => setSelected(ids, groupCheck.checked));
      heading.append(groupCheck);
    }
    heading.append(label, node('span', String(group.open.length), 'memo-group-count'));
    section.append(heading);

    const openRows = node('div', '', 'memo-group-rows');
    for (const memo of sortOpenMemos(group.open)) openRows.append(buildMemoRow(memo, group.project));
    section.append(openRows);

    if (group.done.length > 0) {
      const details = document.createElement('details');
      details.className = 'memo-group-done';
      const summary = document.createElement('summary');
      summary.textContent = t('memo_done_toggle', { count: group.done.length });
      details.append(summary);
      const doneRows = node('div', '', 'memo-group-rows');
      for (const memo of sortDoneMemos(group.done)) doneRows.append(buildMemoRow(memo, group.project));
      details.append(doneRows);
      section.append(details);
    }
    listEl.append(section);
  }
}

function updateSubtitle(): void {
  if (!subtitleEl) return;
  const project = activeProject();
  if (project) {
    subtitleEl.textContent = t('memo_active_project', { project: basenameForPath(project) });
    subtitleEl.title = project;
  } else {
    subtitleEl.textContent = t('memo_active_project_none');
    subtitleEl.removeAttribute('title');
  }
}

type NewMemoPayload = { text: string; session_id?: number; images?: string[] };

// 入力欄からの追加と、パスの右クリックメニューからの保存（saveTextToMemo）の共通部分。
async function createMemo(payload: NewMemoPayload): Promise<void> {
  const { memo } = await request<{ memo: Memo }>('/api/memos', body('POST', payload));
  cache.push(memo);
  notifyMemosChanged();
  announceMemosChangedToOtherWindows();
  renderList();
}

// plan_path-menu-save-to-memo.md: パスの右クリックメニューの「作業メモに保存」の実体。
// memo-model.ts の saveTextToMemo から呼ばれる（循環 import を避けるための登録方式）。
// sessionId は呼び出し元によって number のことも '' のこともある。数値として読めなければ
// 選択中のセッションで分類し、それも無ければ未分類にする。
async function saveTextFromOutside(text: string, sessionId?: number | string | null): Promise<'saved' | 'exists'> {
  const trimmed = text.trim();
  if (!trimmed) throw new Error(t('memo_request_failed'));
  if (findOpenMemoByText(cache, trimmed)) return 'exists';
  const payload: NewMemoPayload = { text: trimmed };
  const sid = Number(sessionId);
  if (Number.isInteger(sid) && sid > 0) payload.session_id = sid;
  else if (activeSessionId !== null) payload.session_id = activeSessionId;
  await createMemo(payload);
  return 'saved';
}
registerMemoSaver(saveTextFromOutside);

async function submitNewMemo(): Promise<void> {
  if (!inputEl.value.trim() && pendingImages.length === 0 && inflightUploads.size === 0) return;
  inputEl.disabled = true;
  try {
    // 貼り付けた直後に Enter されても画像を取りこぼさない。
    if (inflightUploads.size > 0) await Promise.all(Array.from(inflightUploads));
    const text = inputEl.value.trim();
    const images = pendingImages.slice();
    if (!text && images.length === 0) return;
    const payload: NewMemoPayload = { text };
    if (images.length > 0) payload.images = images;
    if (activeSessionId !== null) payload.session_id = activeSessionId;
    await createMemo(payload);
    inputEl.value = '';
    pendingImages = [];
    renderPendingImages();
  } catch (error) {
    showFeedback(error);
  } finally {
    inputEl.disabled = false;
    inputEl.focus();
  }
}

function ensureDrawer(): HTMLElement {
  if (drawer) return drawer;
  drawer = document.createElement('div');
  drawer.id = 'memo-drawer';
  drawer.className = 'memo-drawer';
  drawer.hidden = true;
  // 端末に重なるだけの部分ドロワー（画面全体は覆わない）。.aac-wheel-overlay ではなく
  // data-wheel-native を付け、ドロワー上の wheel はネイティブスクロールに任せる
  // （CLAUDE.md 設計原則の索引「全画面オーバーレイには…」の行。ドロワーは全画面ではない）。
  drawer.setAttribute('data-wheel-native', '');

  const header = node('div', '', 'memo-drawer-header');
  const titleWrap = node('div', '', 'memo-drawer-title-wrap');
  titleWrap.append(node('h2', t('memo_title'), 'memo-drawer-title'));
  subtitleEl = node('p', '', 'memo-drawer-subtitle');
  titleWrap.append(subtitleEl);
  const closeBtn = node('button', '✕', 'memo-drawer-close');
  closeBtn.type = 'button';
  closeBtn.setAttribute('aria-label', t('memo_close'));
  closeBtn.addEventListener('click', () => closeDrawer());
  const headerActions = node('div', '', 'memo-drawer-header-actions');
  if (!isDetachedTabView()) {
    // サブモニターへ常駐させたいとき用。ドロワーを閉じ、同じ中身を別窓で開く。
    const popOut = node('button', '🗗', 'memo-drawer-popout');
    popOut.type = 'button';
    popOut.title = t('detached_open_in_window');
    popOut.setAttribute('aria-label', t('detached_open_in_window'));
    popOut.addEventListener('click', () => {
      // 開けなかった（ポップアップが止められた）ときはドロワーを残して案内する
      if (openDetachedTabWindow('memo', activeSessionId)) closeDrawer();
      else showToast(t('detached_popup_blocked'), undefined, 6000);
    });
    headerActions.append(popOut);
  }
  headerActions.append(closeBtn);
  header.append(titleWrap, headerActions);

  // 見出し・説明文で「自分用のメモ」と明示する（AI が書く引き継ぎ看板とは別物）。
  const description = node('p', t('memo_description'), 'memo-drawer-description');

  const inputWrap = node('div', '', 'memo-input-wrap');
  inputEl = document.createElement('textarea');
  inputEl.className = 'ui-input memo-input';
  inputEl.rows = 2;
  inputEl.placeholder = t('memo_input_placeholder');
  inputEl.addEventListener('keydown', (e) => {
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); void submitNewMemo(); }
  });
  // スクリーンショットの貼り付け（Ctrl+V）。文字も一緒に入っているクリップボード
  // （表計算ソフトのセル等）は文字の貼り付けを優先し、画像としては取り込まない。
  inputEl.addEventListener('paste', (e) => {
    const data = e.clipboardData;
    if (!data) return;
    const files = pickMemoImageFiles(Array.from(data.files));
    if (files.length === 0 || data.types.includes('text/plain')) return;
    e.preventDefault();
    addPendingImageFiles(files);
  });
  // 画像ファイルのドロップ。
  inputWrap.addEventListener('dragover', (e) => {
    if (e.dataTransfer?.types.includes('Files')) { e.preventDefault(); inputWrap.classList.add('memo-input-dragover'); }
  });
  inputWrap.addEventListener('dragleave', () => inputWrap.classList.remove('memo-input-dragover'));
  inputWrap.addEventListener('drop', (e) => {
    inputWrap.classList.remove('memo-input-dragover');
    const files = pickMemoImageFiles(Array.from(e.dataTransfer?.files ?? []));
    if (files.length === 0) return;
    e.preventDefault();
    addPendingImageFiles(files);
  });
  pendingImagesEl = node('div', '', 'memo-input-images');
  pendingImagesEl.hidden = true;
  inputWrap.append(inputEl, pendingImagesEl, node('p', t('memo_plaintext_note'), 'memo-plaintext-note'));

  feedbackEl = node('p', '', 'memo-feedback');
  feedbackEl.setAttribute('role', 'status');
  toolbarEl = node('div', '', 'memo-toolbar');
  buildExportControls();
  const importPanel = buildImportPanel();
  listEl = node('div', '', 'memo-list');

  drawer.append(header, description, inputWrap, feedbackEl, toolbarEl, importPanel, listEl);
  renderToolbar(); // 0 件でも「コピー ▾」と取り込みを使えるよう、一覧を取る前から出す
  document.body.append(drawer);
  return drawer;
}

export function openMemoDrawer(): void {
  const el = ensureDrawer();
  el.hidden = false;
  updateSubtitle();
  void refresh();
}
function closeDrawer(): void {
  // 別窓では閉じる先が無い（閉じると空の窓が残る）。閉じるボタンも出していない。
  if (drawer && !isDetachedTabView()) drawer.hidden = true;
}

/**
 * 作業メモの別窓（/?view=detached-tab&tab=memo）。ドロワーと同じ中身を host の中に
 * 全面で出す（plan_detached-tab-windows.md C4）。detached-view.ts が i18n-ready 後に呼ぶ。
 */
export function mountMemoWindow(host: HTMLElement): void {
  const el = ensureDrawer();
  el.classList.add('memo-drawer--window');
  el.querySelector<HTMLElement>('.memo-drawer-close')?.setAttribute('hidden', '');
  host.append(el);
  openMemoDrawer();
}

/** 表示中のセッション（＝新しいメモの分類先）が変わったときに見出しを直す。 */
export function refreshMemoContext(): void {
  updateSubtitle();
  renderList();
}
function toggleDrawer(): void {
  const el = ensureDrawer();
  if (!el.hidden) { closeDrawer(); return; }
  // 既定は別窓（サブモニターへ常駐させる使い方が主）。開けなかった（ポップアップが
  // 止められた）ときだけ、これまでのドロワーで開いて案内する。
  if (!isDetachedTabView()) {
    if (openDetachedTabWindow('memo', activeSessionId)) return;
    showToast(t('detached_popup_blocked'), undefined, 6000);
  }
  openMemoDrawer();
}

// node:test が純関数だけ import するとき document は無い。配線を走らせない
// （memo-model.test.ts はこのファイルではなく memo-model.ts だけを import するが、
// 同じ安全策を他の app/*.ts ファイルに揃える）。
if (typeof document !== 'undefined') {
  document.addEventListener('click', (event) => {
    if ((event.target as Element)?.closest?.('[data-open-memo]')) toggleDrawer();
  });
  // C4: ボタン・セッション行のバッジは開いていなくても件数が要るので、routines.ts の
  // i18n-ready 起動と同じ合図で一度だけ先読みする。ensureDrawer() は非表示のまま
  // ドロワーを組み立てるだけなので見た目には出ない（openMemoDrawer と同じ refresh 経路）。
  document.addEventListener('i18n-ready', () => { ensureDrawer(); void refresh(); }, { once: true });
  // ほかの窓（本体・作業メモの別窓）でメモが変わったら取り直す。件数バッジを揃えるため、
  // ドロワーを開いていなくても取り直す。
  onWindowMessage((msg) => {
    if (msg.type === 'memos-changed' && drawer) void refresh();
  });
}
