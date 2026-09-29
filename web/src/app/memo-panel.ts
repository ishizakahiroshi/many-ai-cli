// plan_memo-panel.md C3 — メモボタンと右ドロワー UI。
// 非モーダル: showModal() は使わず、開いたままターミナルを操作できる（data-wheel-native）。
// グループ化・並び順・件数の純粋ロジックは memo-model.ts にある。
import { t } from '../i18n.js';
import { apiFetch, token } from './util.js';
import { openLightbox } from './lightbox.js';
import { activeSessionId, sessions } from './state.js';
import { openSpawnPanelWith } from './spawn-panel.js';
import { basenameForPath, openFileModal } from './path-links.js';
import { findPathCandidates } from './path-detect.js';
import {
  countOpenMemosForProject,
  findOpenMemoByText,
  groupMemosByProject,
  MEMO_IMAGE_MAX_BYTES,
  MEMO_IMAGES_PER_MEMO,
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
// 別窓（plan_detached-tab-windows.md C4）。どちらも何も import しない葉モジュール。
import { isDetachedTabView, openDetachedTabWindow } from './detached-view-mode.js';
import { onWindowMessage, postWindowMessage, requestMainWindow } from './window-channel.js';
import { showToast } from './util.js';

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
// 入力欄に貼り付け・ドロップした画像（Hub へ保存済みの画像名）。メモを追加すると空になる。
// × で外した画像や、追加せずに終わった画像は Hub が次回起動時に回収する（memo_images.go）。
let pendingImages: string[] = [];
let pendingImagesEl: HTMLElement;
// 送信中のアップロード。Enter が先に押されても、終わるのを待ってからメモを作る。
const inflightUploads = new Set<Promise<void>>();

function memoImageUrl(name: string): string {
  return `/api/memo-images/${encodeURIComponent(name)}?token=${encodeURIComponent(token)}`;
}

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = '', className = ''): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  el.textContent = text;
  el.className = className;
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
  let pos = 0;
  for (const c of candidates) {
    if (c.start > pos) container.appendChild(document.createTextNode(text.slice(pos, c.start)));
    const resolved = resolveMemoPathTarget(c.text, project);
    const link = document.createElement('span');
    link.className = 'tool-output-path-link';
    link.textContent = c.text;
    link.tabIndex = 0;
    const openIt = (): void => { openFileModal(resolved, activeSessionId, project); };
    link.addEventListener('click', (e) => { e.preventDefault(); e.stopPropagation(); openIt(); });
    link.addEventListener('contextmenu', (e) => { e.preventDefault(); e.stopPropagation(); openIt(); });
    link.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openIt(); }
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
  if (!window.confirm(t('memo_delete_confirm'))) return;
  try {
    await request(`/api/memos/${encodeURIComponent(memo.id)}`, { method: 'DELETE' });
    cache = cache.filter((m) => m.id !== memo.id);
    notifyMemosChanged();
    announceMemosChangedToOtherWindows();
    renderList();
  } catch (error) { showFeedback(error); }
}

function beginEdit(memo: Memo, rowBody: HTMLElement, textEl: HTMLElement): void {
  if (rowBody.querySelector('textarea')) return;
  const textarea = document.createElement('textarea');
  textarea.className = 'memo-row-edit';
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
  checkbox.checked = memo.done;
  checkbox.setAttribute('aria-label', t('memo_mark_done'));
  checkbox.addEventListener('change', () => void toggleDone(memo, checkbox.checked));

  const rowBody = node('div', '', 'memo-row-body');
  const textEl = node('div', '', 'memo-row-text');
  textEl.tabIndex = 0;
  textEl.title = t('memo_edit_hint');
  renderMemoText(textEl, memo.text, project);
  textEl.addEventListener('dblclick', () => beginEdit(memo, rowBody, textEl));
  rowBody.append(textEl);
  if (memo.images && memo.images.length > 0) {
    renderMemoImages(rowBody, memo.images);
    // 画像だけのメモは本文が空。ダブルクリックで後から一言足せるよう、薄い案内を出す。
    if (!memo.text) {
      textEl.classList.add('memo-row-text-empty');
      textEl.textContent = t('memo_image_add_note');
    }
  }

  const actions = node('div', '', 'memo-row-actions');
  const launch = node('button', t('memo_launch'), 'memo-row-launch');
  launch.type = 'button';
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
  const del = node('button', t('memo_delete'), 'memo-row-delete');
  del.type = 'button';
  del.addEventListener('click', (e) => { e.stopPropagation(); void removeMemo(memo); });
  actions.append(launch, del);

  row.append(checkbox, rowBody, actions);
  return row;
}

function renderList(): void {
  if (!listEl) return;
  listEl.replaceChildren();
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
      closeDrawer();
      openDetachedTabWindow('memo', activeSessionId);
    });
    headerActions.append(popOut);
  }
  headerActions.append(closeBtn);
  header.append(titleWrap, headerActions);

  // 見出し・説明文で「自分用のメモ」と明示する（AI が書く引き継ぎ看板とは別物）。
  const description = node('p', t('memo_description'), 'memo-drawer-description');

  const inputWrap = node('div', '', 'memo-input-wrap');
  inputEl = document.createElement('textarea');
  inputEl.className = 'memo-input';
  inputEl.rows = 2;
  inputEl.placeholder = t('memo_input_placeholder');
  inputEl.addEventListener('keydown', (e) => {
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
  listEl = node('div', '', 'memo-list');

  drawer.append(header, description, inputWrap, feedbackEl, listEl);
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
  if (el.hidden) openMemoDrawer(); else closeDrawer();
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
