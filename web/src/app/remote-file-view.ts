// ssh 先（送り先）のファイルを 1 つ、ダウンロードせずに読み取り専用のモーダルで開く
// （docs/local/plan_remote-file-open-in-viewer.md C1）。入口（端末のパスのメニュー・「取ってくる」の窓）は
// attachments.ts 側から openRemoteFileView を呼ぶ。このファイルは表示だけを持つ。
//
// 中身の取り方は「取ってくる」（attachments.ts の fetchRemoteFile）と同じ Hub の経路
// （/api/files-download?session=…&remote_target=…&path=…。Hub が ssh の cat で読む。rust/src/files/remote_attach.rs）。
// Hub のある PC には何も書かず、ここでも保存しない。
//
// 破らない約束（PC のファイルを開かない）:
//   - ssh 先の Markdown の中の相対パス・/ で始まるパス・file: などのリンクと画像は、押せない文字・代わりの文字にする。
//     PC の Hub の /api/files-* を叩く URL を一切 DOM に置かない。http(s) のリンクだけ、通常のプレビューと同じ
//     ポップアップ（url-links.ts）を通す。変換は files-view.ts の FilesPreview.renderMarkdown(…, { remote: true })、
//     ここは描画前にもう一度、残った URL 属性を確かめる（scrubRemoteMarkdownDom）。
//   - 画面に出すのはこの経路で取った中身だけ。PC のパスを /api/files-content・/api/files-asset へ渡さない。
//   - 編集・保存・別窓・「フォルダを開く」など、PC のファイルを前提にした操作は置かない。
//
// 読み込み時に循環 import の相手の変数を同期で読まない（定数は literal だけ。他は関数の中で使う）。
import { t } from '../i18n.js';
import { apiFetch, showToast } from './util.js';
import { basenameForPath, closeFileModal, copyPathText, isImagePath, isTextPath, openFileModalShell } from './path-links.js';
import { FilesPreview } from './files-view.js';
import { bindUrlLinksIn, linkifyUrlsInElement } from './url-links.js';
import { openLightbox } from './lightbox.js';

/** 文字のファイル・Markdown をそのまま描く上限。これを超えたら描かず「取ってくる」を案内する（PC のプレビューの切り詰めと同じ 1 MiB）。 */
export const REMOTE_VIEW_MAX_TEXT_BYTES = 1024 * 1024;
/** 画像の上限。ブラウザの画像表示は大きくても耐えるが、Blob を丸ごとメモリに持つので Hub の取得上限（50 MB）より小さくする。 */
export const REMOTE_VIEW_MAX_IMAGE_BYTES = 20 * 1024 * 1024;

export type RemoteViewKind = 'markdown' | 'text' | 'image';

const IMAGE_MIME: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  bmp: 'image/bmp',
};

/** 表示の分け方は PC のプレビューと同じ（.md だけ Markdown、isTextPath は等幅、isImagePath は画像）。動画は出さない。 */
export function remoteViewKind(path: string): RemoteViewKind | null {
  const p = String(path || '').trim();
  if (/\.md$/i.test(p)) return 'markdown';
  if (isImagePath(p)) return 'image';
  if (isTextPath(p)) return 'text';
  return null;
}

/** プレビューできる種類か。入口がメニューの「開く」を出すかどうかに使う。 */
export function isRemoteViewablePath(path: string): boolean {
  return remoteViewKind(path) !== null;
}

/**
 * 送り先（ssh 先）のパスとして受けてよい形か。Hub 側（remote_attach.rs の validate_remote_path）と同じ規則で、
 * 相対パスは送り先の作業フォルダが分からないので受けない。「取ってくる」（attachments.ts の fetchRemoteFile）と
 * 「開く」（openRemoteFileView）が同じ規則を使うよう、規則の置き場はここ 1 か所にした。
 * attachments.ts から import される側なので、このファイルは attachments.ts を import しない（循環を増やさない）。
 */
export function isFetchableRemotePath(path: string): boolean {
  const rest = path.startsWith('~/') ? path.slice(2) : path.startsWith('/') ? path : '';
  return rest !== '' && rest !== '/' && !rest.endsWith('/') && path.length <= 4096 && !/[\u0000-\u001f\u007f-\u009f]/.test(path);
}

function formatMegabytes(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  return `${mb >= 10 ? Math.round(mb) : Math.round(mb * 10) / 10} MB`;
}

// ---- 描画前の確認（Markdown の URL 属性）----

const DATA_IMAGE_RE = /^data:image\/(?:png|jpe?g|gif|webp|bmp);base64,[A-Za-z0-9+/=]+$/i;

const URL_ATTRIBUTES = ['href', 'src', 'srcset', 'poster', 'action', 'formaction', 'data', 'background', 'ping', 'cite', 'longdesc', 'usemap', 'xlink:href'];

/**
 * ssh 先の Markdown を描いた DOM に残してよい属性か。URL を持つ属性で残してよいのは
 * <a href="http(s)://…"> と <img src="data:image/…"> だけ。on* 属性と、押したとき PC のパスを開く印（data-files-*）は
 * 残さない。それ以外の属性（class・title・alt など）は影響しないので残す。
 */
export function isAllowedRemoteAttribute(tag: string, name: string, value: string): boolean {
  const tagName = tag.toLowerCase();
  const attrName = name.toLowerCase();
  if (attrName.startsWith('on') || attrName.startsWith('data-files-')) return false;
  if (!URL_ATTRIBUTES.includes(attrName)) return true;
  const url = String(value).trim();
  return (tagName === 'a' && attrName === 'href' && /^https?:\/\//i.test(url))
    || (tagName === 'img' && attrName === 'src' && DATA_IMAGE_RE.test(url));
}

/**
 * ssh 先の Markdown を描いた DOM から、PC のファイルとして開かれうるものを外す。FilesPreview.renderMarkdown の変換
 * （marked の renderer と DOMPurify の ALLOWED_URI_REGEXP）が先に同じ結果にしているので、ここは二重の安全策
 * （DOMPurify は data-* 属性を通すので、生の HTML に書かれた data-files-path-link はここで外れる）。
 */
export function scrubRemoteMarkdownDom(root: Element): void {
  for (const el of Array.from(root.querySelectorAll('*'))) {
    for (const attr of Array.from(el.attributes)) {
      if (!isAllowedRemoteAttribute(el.tagName, attr.name, attr.value)) el.removeAttribute(attr.name);
    }
  }
}

function mountRemoteMarkdown(contentEl: HTMLElement, html: string): void {
  // DOMParser の文書は画像などを読み込まない。確かめてから画面の文書へ移すので、確かめる前に通信は起きない。
  const doc = new DOMParser().parseFromString(`<div class="files-preview-md-body">${html}</div>`, 'text/html');
  const root = doc.body.firstElementChild as HTMLElement;
  scrubRemoteMarkdownDom(root);
  const mounted = document.importNode(root, true) as HTMLElement;
  contentEl.appendChild(mounted);
  bindUrlLinksIn(mounted);
  mounted.querySelectorAll('pre code').forEach((code) => { linkifyUrlsInElement(code); });
}

// ---- 取得 ----

// reason で分ける（このプロジェクトは strictNullChecks が切ってあり、true/false の判別では型が絞れない）。
type FetchResult =
  | { reason: 'ok'; bytes: ArrayBuffer }
  | { reason: 'too_large'; size: number }
  | { reason: 'failed'; detail: string };

async function fetchRemoteBytes(sessionId: number, target: string, path: string, maxBytes: number, signal: AbortSignal): Promise<FetchResult> {
  const res = await apiFetch(
    `/api/files-download?session=${encodeURIComponent(sessionId)}&remote_target=${encodeURIComponent(target)}&path=${encodeURIComponent(path)}`,
    { signal },
  );
  if (!res.ok) {
    // 鍵が無い・ファイルが無い・大きすぎる等は利用者が直すものなので、Hub の detail をそのまま見せる（fetchRemoteFile と同じ）。
    const detail = await res.json().then((body) => String(body?.detail || ''), () => '');
    return { reason: 'failed', detail: detail || `HTTP ${res.status}` };
  }
  const declared = Number(res.headers.get('Content-Length'));
  if (Number.isFinite(declared) && declared > maxBytes) {
    try { await res.body?.cancel(); } catch (_) {}
    return { reason: 'too_large', size: declared };
  }
  const bytes = await res.arrayBuffer();
  if (bytes.byteLength > maxBytes) return { reason: 'too_large', size: bytes.byteLength };
  return { reason: 'ok', bytes };
}

// ---- 画面 ----

function iconButton(glyph: string, title: string, onClick: (button: HTMLButtonElement) => void): HTMLButtonElement {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'ui-button ui-button--quiet ui-button--compact files-preview-toolbar-btn';
  button.title = title;
  button.setAttribute('aria-label', title);
  button.textContent = glyph;
  button.addEventListener('click', () => onClick(button));
  return button;
}

function buildToolbar(target: string, path: string, onFetch: (() => void) | undefined): HTMLElement {
  const toolbar = document.createElement('div');
  toolbar.className = 'files-preview-toolbar';

  const breadcrumb = document.createElement('span');
  breadcrumb.className = 'files-preview-breadcrumb';
  breadcrumb.title = `${target}:${path}`;
  const dir = document.createElement('span');
  dir.className = 'files-preview-breadcrumb-dir';
  dir.textContent = `${target}:${path.slice(0, path.lastIndexOf('/') + 1)}`;
  const file = document.createElement('span');
  file.className = 'files-preview-breadcrumb-file';
  file.textContent = basenameForPath(path);
  breadcrumb.append(dir, file);
  toolbar.appendChild(breadcrumb);

  // ssh 先のパスをコピーするだけ。PC のファイルを前提にした操作（編集・保存・別窓・フォルダを開く）は置かない。
  toolbar.appendChild(iconButton('🔗', t('files_preview_copy_path_tooltip'), (button) => { void copyPathText(path, button).catch(() => {}); }));
  if (onFetch) toolbar.appendChild(iconButton('⬇️', t('remote_fetch_submit'), () => { onFetch(); }));
  toolbar.appendChild(iconButton('✕', t('files_preview_close_tooltip'), () => { closeFileModal(); }));
  return toolbar;
}

function showMessage(contentEl: HTMLElement, text: string, className: string): void {
  const message = document.createElement('div');
  message.className = className;
  message.textContent = text;
  contentEl.replaceChildren(message);
}

/**
 * ssh 先のファイルを読み取り専用のモーダルで開く。中身が画面に出たら true。
 * 失敗（鍵・パス違い・大きすぎ等、Hub が返した理由）はトーストに出してモーダルを閉じ、false を返す。
 * 大きすぎる・文字や画像として読めないときは、モーダルに「取ってくる」を案内する文言を出して false を返す。
 * onFetch を渡すと、ツールバーに「取ってくる」のボタンが付く（押すと onFetch を呼ぶだけで、モーダルは閉じない）。
 */
export async function openRemoteFileView(
  sessionId: number,
  target: string,
  remotePath: string,
  options: { onFetch?: () => void } = {},
): Promise<boolean> {
  const host = String(target || '').trim();
  const path = String(remotePath || '').trim();
  if (!host) {
    showToast(t('remote_fetch_no_target'));
    return false;
  }
  if (!isFetchableRemotePath(path)) {
    showToast(t('remote_fetch_invalid'));
    return false;
  }
  const name = basenameForPath(path) || path;
  const kind = remoteViewKind(path);
  if (!kind) {
    showToast(t('remote_view_unsupported', { name }), undefined, 8000);
    return false;
  }

  const state = { closed: false, objectUrl: '', abort: new AbortController() };
  const { dialogEl, body } = openFileModalShell(() => {
    // 閉じたとき（Esc・背景・✕・ほかのモーダルが開いたとき）。通信を止め、画像の Blob URL を解放する。
    state.closed = true;
    state.abort.abort();
    if (state.objectUrl) {
      const url = state.objectUrl;
      state.objectUrl = '';
      // 拡大表示（lightbox）がこの画像を開いたままだと、解放後は壊れた画像になるので一緒に片付ける。
      document.querySelectorAll('#image-lightbox').forEach((box) => {
        if (box.querySelector('img')?.getAttribute('src') === url) box.remove();
      });
      URL.revokeObjectURL(url);
    }
  });
  dialogEl.setAttribute('role', 'dialog');
  dialogEl.setAttribute('aria-label', `${host}:${path}`);

  const bodyEl = document.createElement('div');
  bodyEl.className = 'files-preview-body';
  const contentEl = document.createElement('div');
  contentEl.className = 'files-preview-markdown';
  bodyEl.appendChild(contentEl);
  body.replaceChildren(buildToolbar(host, path, options.onFetch), bodyEl);
  // 画面のどこを押しても、リンクの標準の動き（画面の移動）は起こさない。http(s) のリンクは url-links.ts が自分で止めて
  // ポップアップを出すので、ここまで届くのは「押せないはずのリンク」だけ。
  contentEl.addEventListener('click', (event) => {
    const anchor = (event.target as Element | null)?.closest?.('a') as HTMLAnchorElement | null;
    if (anchor && !anchor.dataset.urlLinkBound) event.preventDefault();
  });
  showMessage(contentEl, t('remote_view_loading', { target: host }), 'files-preview-loading');

  const limit = kind === 'image' ? REMOTE_VIEW_MAX_IMAGE_BYTES : REMOTE_VIEW_MAX_TEXT_BYTES;
  let result: FetchResult;
  try {
    result = await fetchRemoteBytes(sessionId, host, path, limit, state.abort.signal);
  } catch (_) {
    if (state.closed) return false; // 利用者が閉じて通信を止めた
    closeFileModal();
    showToast(t('remote_view_failed', { target: host, detail: 'network error' }), undefined, 8000);
    return false;
  }
  if (state.closed) return false;
  if (result.reason === 'too_large') {
    showMessage(
      contentEl,
      t('remote_view_too_large', { name, size: formatMegabytes(result.size), limit: formatMegabytes(limit) }),
      'files-preview-truncated-warn',
    );
    return false;
  }
  if (result.reason === 'failed') {
    closeFileModal();
    showToast(t('remote_view_failed', { target: host, detail: result.detail }), undefined, 8000);
    return false;
  }

  const unreadable = (): false => {
    showMessage(contentEl, t('remote_view_unreadable', { name }), 'files-preview-truncated-warn');
    return false;
  };
  contentEl.replaceChildren();
  if (kind === 'image') {
    const ext = (/\.([A-Za-z0-9]+)$/.exec(path)?.[1] || '').toLowerCase();
    const objectUrl = URL.createObjectURL(new Blob([result.bytes], { type: IMAGE_MIME[ext] || 'application/octet-stream' }));
    state.objectUrl = objectUrl;
    const wrap = document.createElement('div');
    wrap.className = 'files-preview-image-wrap';
    const image = document.createElement('img');
    image.className = 'files-preview-image';
    image.alt = path;
    image.addEventListener('click', () => openLightbox(objectUrl));
    image.addEventListener('error', () => { if (!state.closed) unreadable(); });
    image.src = objectUrl;
    const caption = document.createElement('div');
    caption.className = 'files-preview-image-caption';
    caption.textContent = `${host}:${path}`;
    wrap.append(image, caption);
    contentEl.appendChild(wrap);
    return true;
  }

  let text: string;
  try {
    text = new TextDecoder('utf-8', { fatal: true }).decode(result.bytes);
  } catch (_) {
    return unreadable();
  }
  if (text.includes('\u0000')) return unreadable();

  const note = document.createElement('div');
  note.className = 'files-preview-truncated-warn files-preview-readonly-note';
  note.textContent = t('remote_view_readonly_note', { target: host });
  contentEl.appendChild(note);
  if (kind === 'markdown') {
    mountRemoteMarkdown(contentEl, FilesPreview.renderMarkdown(text, '', null, { remote: true }));
  } else {
    const pre = document.createElement('pre');
    pre.className = 'files-preview-raw';
    pre.textContent = text; // 文字として入れる（HTML としては解釈されない）
    linkifyUrlsInElement(pre);
    contentEl.appendChild(pre);
  }
  return true;
}
