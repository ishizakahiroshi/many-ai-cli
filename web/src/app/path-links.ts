// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { showToast, token } from './util.js';
import { sessions } from './state.js';
import { FilesTabManager, FilesPreview } from './files-view.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- ターミナルパスリンクポップアップ ----

export let pathPopupEl = null;
export let pathPopupHideTimer = null;

export function getOrCreatePathPopup() {
  if (pathPopupEl) return pathPopupEl;
  pathPopupEl = document.createElement('div');
  pathPopupEl.id = 'path-link-popup';
  pathPopupEl.className = 'path-link-popup';
  pathPopupEl.addEventListener('mouseenter', () => {
    if (pathPopupHideTimer) { clearTimeout(pathPopupHideTimer); pathPopupHideTimer = null; }
  });
  pathPopupEl.addEventListener('mouseleave', () => { scheduleHidePathPopup(); });
  document.body.appendChild(pathPopupEl);
  return pathPopupEl;
}

export function scheduleHidePathPopup() {
  if (pathPopupHideTimer) clearTimeout(pathPopupHideTimer);
  pathPopupHideTimer = setTimeout(() => {
    if (pathPopupEl) pathPopupEl.hidden = true;
    pathPopupHideTimer = null;
  }, 300);
}

export async function copyPathText(text, anchor) {
  if (!text) return;
  await navigator.clipboard.writeText(text);
  showToast(t('copied_to_clipboard'), anchor);
}

export function isImagePath(filePath) {
  return /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(String(filePath || '').trim());
}

export function isVideoPath(filePath) {
  return /\.(mp4|webm|ogv|mov|m4v)$/i.test(String(filePath || '').trim());
}

export function isMediaPath(filePath) {
  return isImagePath(filePath) || isVideoPath(filePath);
}

export function isTextPath(filePath) {
  const path = String(filePath || '').trim();
  if (/(^|[\\/])(Dockerfile|Makefile|README|LICENSE|CHANGELOG|NOTICE)$/i.test(path)) return true;
  return /\.(txt|md|markdown|rst|log|json|jsonl|yaml|yml|toml|ini|cfg|conf|env|csv|tsv|xml|html?|css|scss|sass|less|js|mjs|cjs|jsx|ts|tsx|vue|go|rs|py|rb|php|java|kt|kts|c|cc|cpp|cxx|h|hh|hpp|cs|sh|bash|zsh|fish|ps1|psm1|bat|cmd|sql|graphql|gql|proto|diff|patch|gitignore|gitattributes|editorconfig)$/i.test(path);
}

// MANY-AI-CLI 内蔵プレビューが扱える拡張子（バックエンド /api/files-content の許可リストと一致させること）
export function isAnyAiCliPreviewable(filePath) {
  return isTextPath(filePath) || isMediaPath(filePath);
}

export function getFilesAssetUrl(absPath, sessionId) {
  const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
  return `/api/files-asset?path=${encodeURIComponent(absPath)}&token=${encodeURIComponent(token)}${sessionQs}`;
}

export function getFilesDownloadUrl(absPath, sessionId) {
  const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
  return `/api/files-download?path=${encodeURIComponent(absPath)}&token=${encodeURIComponent(token)}${sessionQs}`;
}

export function getPathOpenItem(filePath, sessionId) {
  // 種別を問わず OS 既定の関連付けアプリで開く（画像・動画・テキスト・その他で挙動は同一）。
  return { icon: '🚀', key: 'link_open_default', action: () => callOpenApi('/api/open-default-file', filePath, 'link_open_default_error', sessionId) };
}

export function showPathPopup(filePath, clientX, clientY, sessionId, pathType = 'file') {
  const popup = getOrCreatePathPopup();
  popup.innerHTML = '';
  popup.hidden = false;

  const items = [];
  const isDir = pathType === 'dir';
  if (!isDir && isAnyAiCliPreviewable(filePath)) {
    items.push({
      icon: '📖',
      key: 'link_open_any_ai_cli',
      action: () => {
        const ses = sessions.get(sessionId);
        const cwd = ses?.cwd;
        if (!cwd) { showToast(t('link_open_error')); return; }
        const projectKey = cwd.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || cwd;
        FilesTabManager.openFilesTabAtFile(sessionId, projectKey, cwd, cwd, filePath);
      },
    });
    items.push({
      icon: '🗔',
      key: 'link_open_modal',
      action: () => openFileModal(filePath, sessionId),
    });
  }
  if (!isDir) {
    items.push(getPathOpenItem(filePath, sessionId));
  }
  items.push(
    { icon: '📁', key: 'link_open_folder', action: () => {
      if (isDir) return callOpenApi('/api/open-default-file', filePath, 'link_open_default_error', sessionId);
      return callOpenApi('/api/open-folder', filePath, 'link_open_error', sessionId);
    }},
    { icon: '💻', key: 'link_open_terminal', action: () => {
      const dir = isDir ? filePath : (dirnameForPath(filePath) || sessions.get(sessionId)?.cwd || filePath);
      callOpenApi('/api/open-terminal', dir, 'link_open_error', sessionId);
    }},
    { icon: '📋', key: 'link_copy_path', action: (anchor) => {
      return copyPathText(filePath, anchor).catch(() => {});
    }},
    { icon: '📋', key: 'link_copy_rel_path', action: (anchor) => {
      const ses = sessions.get(sessionId);
      const cwd = ses?.cwd || '';
      const rel = cwd ? computeRelPath(cwd, filePath) : filePath;
      return copyPathText(rel, anchor).catch(() => {});
    }},
  );
  if (isDir) {
    items.push(
      { icon: '✏️', key: 'link_rename', action: () => renameFileViaApi(filePath, sessionId) },
      { icon: '🗑️', key: 'link_delete_dir', action: () => deleteDirViaApi(filePath, sessionId) },
    );
  } else {
    items.push(
      { icon: '✏️', key: 'link_rename', action: () => renameFileViaApi(filePath, sessionId) },
      { icon: '⬇️', key: 'link_download_file', action: (anchor) => downloadPathViaBrowser(filePath, sessionId, anchor) },
    );
  }

  renderPathPopupItems(popup, items, clientX, clientY);
}

// renderPathPopupItems は items 配列をボタン化して popup に並べ、画面端からはみ出さない
// よう位置調整する。showPathPopup / showFileActionsPopup で共有する。
export function renderPathPopupItems(popup, items, clientX, clientY) {
  for (const item of items) {
    const btn = document.createElement('button');
    btn.className = 'path-link-popup-item';
    btn.textContent = item.icon + ' ' + t(item.key);
    btn.addEventListener('click', () => {
      Promise.resolve(item.action(btn)).finally(() => {
        popup.hidden = true;
      });
    });
    popup.appendChild(btn);
  }

  // 位置調整: 画面端からはみ出さないようにする
  popup.style.left = '0';
  popup.style.top = '0';
  const rect = popup.getBoundingClientRect();
  const vw = window.innerWidth, vh = window.innerHeight;
  let left = clientX + 8;
  let top = clientY + 8;
  if (left + rect.width > vw - 8) left = clientX - rect.width - 8;
  if (top + rect.height > vh - 8) top = clientY - rect.height - 8;
  popup.style.left = Math.max(4, left) + 'px';
  popup.style.top = Math.max(4, top) + 'px';
}

// showFileActionsPopup は Git タブのファイル行向けの軽量ポップアップ。
// CLI 画面の右クリックメニュー（showPathPopup）と同じ部品を流用しつつ、項目を
// 「🗔 モーダルで開く」「🚀 既定のアプリで開く」の 2 つだけに絞る。
// プレビュー不能な拡張子のときはモーダル項目を出さず既定アプリのみにする。
export function showFileActionsPopup(filePath, clientX, clientY, sessionId) {
  const popup = getOrCreatePathPopup();
  popup.innerHTML = '';
  popup.hidden = false;

  const items = [];
  if (isAnyAiCliPreviewable(filePath)) {
    items.push({ icon: '🗔', key: 'link_open_modal', action: () => openFileModal(filePath, sessionId) });
  }
  items.push(getPathOpenItem(filePath, sessionId));

  renderPathPopupItems(popup, items, clientX, clientY);
}

// ---- ファイルプレビューモーダル（FilesPreview をツリー無しで単体表示）----

export let fileModalEl = null;
let fileModalKeyHandler = null;

export function closeFileModal() {
  if (fileModalKeyHandler) {
    document.removeEventListener('keydown', fileModalKeyHandler, true);
    fileModalKeyHandler = null;
  }
  if (fileModalEl) {
    fileModalEl.remove();
    fileModalEl = null;
  }
}

export function openFileModal(filePath, sessionId) {
  const ses = sessions.get(sessionId);
  const cwd = ses?.cwd;
  if (!cwd) { showToast(t('link_open_error')); return; }
  closeFileModal(); // 二重起動防止

  const overlay = document.createElement('div');
  overlay.className = 'aac-file-modal-overlay';
  const panel = document.createElement('div');
  panel.className = 'aac-file-modal';
  const body = document.createElement('div');
  body.className = 'aac-file-modal-body';
  panel.appendChild(body);
  overlay.appendChild(panel);
  document.body.appendChild(overlay);
  fileModalEl = overlay;

  // 背景（オーバーレイ自身）クリックで閉じる。パネル内クリックは閉じない。
  overlay.addEventListener('click', (e) => { if (e.target === overlay) closeFileModal(); });

  // Esc で閉じる（capture で他ハンドラより先に処理し、ターミナル等への波及を防ぐ）
  fileModalKeyHandler = (e) => {
    if (e.key === 'Escape') { e.stopPropagation(); closeFileModal(); }
  };
  document.addEventListener('keydown', fileModalKeyHandler, true);

  // FilesPreview を単体バインド（ツリー無し）。モーダルなので filesRoot/gitRoot は cwd 起点。
  FilesPreview.bind(body, { sessionId, gitRoot: cwd, filesRoot: cwd });

  // FilesPreview 内蔵の閉じる(×)はプレビューを空にするだけでモーダルは残るため隠し、
  // モーダル全体を閉じる専用ボタンに差し替える。
  const toolbar = body.querySelector('.files-preview-toolbar');
  if (toolbar) {
    toolbar.querySelectorAll('.files-preview-toolbar-btn').forEach((b) => {
      if (b.textContent === '×') b.hidden = true;
    });
    const modalClose = document.createElement('button');
    modalClose.className = 'files-preview-toolbar-btn';
    modalClose.title = t('files_preview_close_tooltip') || 'Close';
    modalClose.textContent = '×';
    modalClose.addEventListener('click', () => closeFileModal());
    toolbar.appendChild(modalClose);
  }

  const relPath = computeRelPath(cwd, filePath);
  // _filesPreview は FilesPreview.bind が要素へ動的に付与する外部 API（files-view.ts 参照）。
  // 型定義上は存在しないため any 経由でアクセスする。
  const preview = (body as any)._filesPreview;
  if (preview) preview.loadFile(filePath, relPath);
}

export function basenameForPath(filePath) {
  const normalized = String(filePath || '').replace(/[\\/]+$/, '');
  const slash = Math.max(normalized.lastIndexOf('/'), normalized.lastIndexOf('\\'));
  return slash >= 0 ? normalized.slice(slash + 1) : normalized;
}

export function downloadPathViaBrowser(filePath, sessionId, anchorEl = null) {
  const path = String(filePath || '').trim();
  if (!path) {
    showToast(t('link_open_error'), anchorEl);
    return;
  }
  const a = document.createElement('a');
  a.href = getFilesDownloadUrl(path, sessionId);
  a.download = basenameForPath(path) || 'download';
  a.hidden = true;
  a.style.display = 'none';
  document.body.appendChild(a);
  a.click();
  a.remove();
}

export async function renameFileViaApi(filePath, sessionId) {
  const current = basenameForPath(filePath);
  const input = window.prompt(t('link_rename_prompt') || 'Enter new file name', current);
  if (input == null) return;
  const newName = input.trim();
  if (!newName || newName === current) return;
  try {
    const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
    const url = `/api/files-rename?token=${encodeURIComponent(token)}${sessionQs}`;
    const res = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ src: filePath, newName }),
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok || !data.ok) {
      const msg = (data && (data.detail || data.error)) ? (data.detail || data.error) : ('HTTP ' + res.status);
      showToast(`${t('link_rename_failed') || 'Failed to rename'}: ${msg}`);
      return;
    }
    window.dispatchEvent(new CustomEvent('many-ai-cli:files-changed', {
      detail: { kind: 'rename', oldAbs: filePath, newAbs: data.newAbs },
    }));
  } catch (err) {
    showToast(`${t('link_rename_failed') || 'Failed to rename'}: ${String(err)}`);
  }
}

export async function callOpenApi(endpoint, path, errorKey = 'link_open_error', sessionId = null) {
  try {
    const sessionQs = (sessionId != null && sessionId !== '') ? `&session=${encodeURIComponent(sessionId)}` : '';
    const res = await fetch(`${endpoint}?token=${encodeURIComponent(token)}${sessionQs}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path }),
    });
    if (res.ok) {
      const data = await res.json();
      if (!data.ok) {
        const msg = data.detail || data.error;
        showToast(msg ? `${t(errorKey)}: ${msg}` : t(errorKey));
      }
    } else {
      const data = await res.json().catch(() => ({}));
      const msg = data.detail || data.error;
      showToast(msg ? `${t(errorKey)}: ${msg}` : t(errorKey));
    }
  } catch (_) { showToast(t(errorKey)); }
}

export function dirnameForPath(filePath) {
  const normalized = String(filePath || '').replace(/[\\/]+$/, '');
  const slash = Math.max(normalized.lastIndexOf('/'), normalized.lastIndexOf('\\'));
  if (slash <= 0) return '';
  if (/^[A-Za-z]:[\\/]?[^\\/]*$/.test(normalized)) return normalized.slice(0, slash + 1);
  return normalized.slice(0, slash);
}

export function computeRelPath(from, to) {
  // 異なるファイルシステム間（Windows ドライブ ↔ Unix ルート、ドライブ違い、UNC 等）は
  // 相対化不能なので絶対パスをそのまま返す。
  const fromDrive = /^([A-Za-z]:)[\\/]/.exec(from);
  const toDrive = /^([A-Za-z]:)[\\/]/.exec(to);
  const fromUnix = from.startsWith('/');
  const toUnix = to.startsWith('/');
  if (!!fromDrive !== !!toDrive) return to;
  if (fromDrive && toDrive && fromDrive[1].toUpperCase() !== toDrive[1].toUpperCase()) return to;
  if (fromUnix !== toUnix && !fromDrive && !toDrive) return to;
  // OS セパレータを / に正規化
  const sep = to.includes('\\') ? '\\' : '/';
  const fromParts = from.replace(/\\/g, '/').split('/').filter(Boolean);
  const toParts = to.replace(/\\/g, '/').split('/').filter(Boolean);
  let common = 0;
  while (common < fromParts.length && common < toParts.length && fromParts[common] === toParts[common]) common++;
  const ups = fromParts.length - common;
  const rel = [...Array(ups).fill('..'), ...toParts.slice(common)].join(sep);
  return rel || '.';
}

export function trimTerminalPathCandidate(path) {
  let text = String(path || '').trim().replace(/(?:\s*[,;:'"<>\])}]+)+$/, '');
  // 拡張子の直後に全角/日本語が続く場合はそこで切る（相対パス・Unix 絶対パスにも適用）。
  // Windows 絶対パスは下の trimWindowsPathCandidate で同等処理を行う。
  text = text.replace(/(\.[a-zA-Z0-9]{1,15})\s*[぀-ヿ㐀-鿿＀-￯一-鿿].*$/u, '$1');
  if (/^[A-Za-z]:[\\/]/.test(text)) text = trimWindowsPathCandidate(text);
  text = stripTerminalLineSuffix(text);
  return text;
}

export function trimWindowsPathCandidate(path) {
  let text = String(path || '');
  text = text.replace(/([\\/])\s+.*$/, '$1');
  text = text.replace(/(\.[a-zA-Z0-9]{1,15})\s*[぀-ヿ㐀-鿿＀-￯一-鿿].*$/u, '$1');
  text = text.replace(/\s+[぀-ヿ㐀-鿿＀-￯].*$/u, '');
  text = text.replace(/\s+[A-Za-z]$/, '');
  return text.replace(/(?:\s*[,;:'"<>\])}]+)+$/, '');
}

export function stripTerminalLineSuffix(path) {
  const text = String(path || '');
  return text.replace(/([^\s:]):\d+(?::\d+)?$/, '$1');
}

export function isAbsolutePath(path) {
  return /^[A-Za-z]:[\\/]/.test(path) || path.startsWith('/');
}

export function joinPath(base, rel) {
  if (!base || !rel) return rel || base || '';
  const sep = base.includes('\\') ? '\\' : '/';
  const baseNorm = base.replace(/[\\/]+$/, '');
  return normalizePathSegments(baseNorm + sep + rel.replace(/^[\\/]+/, '').replace(/[\\/]/g, sep), sep);
}

export function normalizePathSegments(path, sep) {
  const drive = /^[A-Za-z]:/.test(path) ? path.slice(0, 2) : '';
  const rest = drive ? path.slice(2) : path;
  const rooted = rest.startsWith(sep);
  const parts = rest.split(/[\\/]+/);
  const out = [];
  for (const part of parts) {
    if (!part || part === '.') continue;
    if (part === '..' && out.length > 0 && out[out.length - 1] !== '..') out.pop();
    else if (part !== '..' || !rooted) out.push(part);
  }
  return drive + (rooted ? sep : '') + out.join(sep);
}

export function resolveTerminalPathCandidate(path, sessionId) {
  const cleaned = trimTerminalPathCandidate(path);
  if (!cleaned) return '';
  if (isAbsolutePath(cleaned)) return cleaned;
  const cwd = sessions.get(sessionId)?.cwd || '';
  if (!cwd) return cleaned;
  return joinPath(cwd, cleaned);
}

// Windows drive paths can appear with either backslashes or forward slashes
// in terminal output, e.g. C:\dev\app.go or C:/Users/me/.claude/CLAUDE.md.
export const ABS_WIN_PATH_RE = /([A-Za-z]:[\\/](?:(?!\s+[A-Za-z]:[\\/])[^\x00-\x1f<>:"|?*(])+)/g;
// 空白を挟んだ説明文中の区切り（例: "hljs / highlight / prism"）を
// Unix 絶対パスとして誤検出しないよう、セグメント内の空白は許可しない。
export const ABS_UNIX_PATH_RE = /(\/[^\s\/\x00-\x1f"'<>`|(]+(?:\/[^\s\/\x00-\x1f"'<>`|(]*)*)/g;
export const REL_PATH_RE = /(^|[\s([{"'`])((?:\.{1,2}[\\/]|[A-Za-z0-9_.-]+[\\/])(?:[^\s\x00-\x1f"'<>`|(]+[\\/])*[^\s\x00-\x1f"'<>`|(]+)/g;

// `Y/N` / `1/2` / `bash/zsh` 等を誤検出しないための post-filter。
// 受理条件: `./` `../` 始まり、またはセパレータ 2 個以上、または末尾拡張子あり。
export function isLikelyRelPath(path) {
  if (!path) return false;
  if (/^\.{1,2}[\\/]/.test(path)) return true;
  const sepCount = (path.match(/[\\/]/g) || []).length;
  if (sepCount >= 2) return true;
  if (/\.[a-zA-Z0-9]{1,15}$/.test(path)) return true;
  return false;
}

export function isTerminalPathStartBoundary(text, start) {
  if (start <= 0) return true;
  return /[\s([{"'`]/.test(text[start - 1] || '');
}

export function findPathCandidates(text) {
  const candidates = [];
  for (const re of [ABS_WIN_PATH_RE, ABS_UNIX_PATH_RE]) {
    re.lastIndex = 0;
    let m;
    while ((m = re.exec(text)) !== null) {
      if (re === ABS_UNIX_PATH_RE && !isTerminalPathStartBoundary(text, m.index)) continue;
      const pathStr = trimTerminalPathCandidate(m[1]);
      if (pathStr.length >= 3) candidates.push({ start: m.index, end: m.index + pathStr.length, text: pathStr });
    }
  }
  REL_PATH_RE.lastIndex = 0;
  let m;
  while ((m = REL_PATH_RE.exec(text)) !== null) {
    const pathStr = trimTerminalPathCandidate(m[2]);
    if (pathStr.length < 3) continue;
    if (!isLikelyRelPath(pathStr)) continue;
    candidates.push({ start: m.index + m[1].length, end: m.index + m[1].length + pathStr.length, text: pathStr });
  }
  candidates.sort((a, b) => a.start - b.start || b.end - a.end);
  const out = [];
  for (const c of candidates) {
    if (out.some(x => c.start < x.end && c.end > x.start)) continue;
    out.push(c);
  }
  return out;
}

export function appendLinkedText(container, text, sessionId) {
  const candidates = findPathCandidates(text);
  if (candidates.length === 0) {
    container.textContent = text;
    return;
  }
  container.textContent = '';
  let pos = 0;
  for (const c of candidates) {
    if (c.start > pos) container.appendChild(document.createTextNode(text.slice(pos, c.start)));
    const resolvedPath = resolveTerminalPathCandidate(c.text, sessionId);
    const link = document.createElement('span');
    link.className = 'tool-output-path-link';
    link.textContent = c.text;
    link.tabIndex = 0;
    link.addEventListener('click', (e) => showPathPopup(resolvedPath, e.clientX, e.clientY, sessionId));
    link.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        const r = link.getBoundingClientRect();
        showPathPopup(resolvedPath, r.left, r.bottom, sessionId);
      }
    });
    container.appendChild(link);
    pos = c.end;
  }
  if (pos < text.length) container.appendChild(document.createTextNode(text.slice(pos)));
}

export async function deleteDirViaApi(filePath, sessionId) {
  const name = basenameForPath(filePath);
  const message = (t('link_delete_dir_confirm') || 'Delete this folder and all contents?').replace('{name}', name);
  if (!window.confirm(message)) return;
  try {
    const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
    const url = `/api/files-delete-dir?token=${encodeURIComponent(token)}${sessionQs}`;
    const res = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ src: filePath }),
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok || !data.ok) {
      const msg = (data && (data.detail || data.error)) ? (data.detail || data.error) : ('HTTP ' + res.status);
      showToast(`${t('link_delete_dir_failed') || 'Failed to delete folder'}: ${msg}`);
      return;
    }
    window.dispatchEvent(new CustomEvent('many-ai-cli:files-changed', {
      detail: { kind: 'delete-dir', oldAbs: filePath },
    }));
  } catch (err) {
    showToast(`${t('link_delete_dir_failed') || 'Failed to delete folder'}: ${String(err)}`);
  }
}
