// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { apiFetch, showToast, token } from './util.js';
import { activeSessionId, sessions, terminals } from './state.js';
import { copyPathText } from './path-links.js';
import { inputEl, isInteractiveFocusTarget, stagePastedText, updateInputAffordance } from '../app.js';
import { pushMessage } from './chat-history.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- ファイル転送 (attach) ----

export const attachDropZone = document.getElementById('attach-drop-zone');
export const attachPanel = document.getElementById('attach-panel');
export const attachFileInput = document.getElementById('attach-file-input');
export const attachThumbnails = document.getElementById('attach-thumbnails');
export const attachClearBtn = document.getElementById('attach-clear-btn');
// B2: スマホ専用カメラ撮影 input/btn（PC は CSS で非表示）。
export const attachCameraBtn = document.getElementById('attach-camera-btn');
export const attachCameraInput = document.getElementById('attach-camera-input');
export const pendingAttachFiles = []; // {buf, filename, entry, wrapper} — ステージング済み未送信ファイル
export const MAX_ATTACH_BYTES = 8 * 1024 * 1024;

export function isImageFile(file) {
  return file.type.startsWith('image/');
}

export function updateAttachClearBtn() {
  if (!attachThumbnails) return;
  const hasAttachments = attachThumbnails.querySelectorAll('.attach-thumb-wrapper').length > 0;
  attachPanel?.classList.toggle('has-attachments', hasAttachments);
  if (attachClearBtn) attachClearBtn.hidden = !hasAttachments;
}

if (attachClearBtn) {
  attachClearBtn.addEventListener('click', () => {
    if (!attachThumbnails) return;
    pendingAttachFiles.length = 0;
    attachThumbnails.querySelectorAll('.attach-thumb-wrapper').forEach(wrapper => {
      const img = wrapper.querySelector('img');
      if (img) URL.revokeObjectURL(img.src);
      wrapper.remove();
    });
    updateAttachClearBtn();
    updateInputAffordance();
  });
}

// 📁: アクティブセッションの作業ディレクトリパスをクリップボードへコピーするだけの常設ボタン。
// エクスプローラーを自前で開かない（利用者が常駐させている既存ウィンドウへ貼る運用のため）。
export const attachCopyCwdBtn = document.getElementById('attach-copy-cwd-btn');
if (attachCopyCwdBtn) {
  attachCopyCwdBtn.addEventListener('click', () => {
    const cwd = activeSessionId !== null ? sessions.get(activeSessionId)?.cwd : '';
    if (!cwd) return;
    void copyPathText(cwd, attachCopyCwdBtn).catch(() => {});
  });
}

type AgentLogResponse = {
  available?: boolean;
  path?: string;
  label?: string;
  reason?: string;
};

export const agentLogBtn = document.getElementById('agent-log-btn') as HTMLButtonElement | null;
let agentLogPopup: HTMLDivElement | null = null;

function closeAgentLogPopup() {
  if (agentLogPopup) agentLogPopup.hidden = true;
  agentLogBtn?.setAttribute('aria-expanded', 'false');
}

function getAgentLogPopup() {
  if (agentLogPopup) return agentLogPopup;
  const popup = document.createElement('div');
  popup.className = 'agent-log-popup';
  popup.hidden = true;
  popup.setAttribute('role', 'dialog');
  popup.setAttribute('aria-label', t('agent_log_button'));
  popup.addEventListener('click', (e) => e.stopPropagation());
  document.body.appendChild(popup);
  agentLogPopup = popup;
  return popup;
}

function showAgentLogPopup(anchor: HTMLButtonElement, sessionId: number, detail: AgentLogResponse) {
  const path = String(detail.path || '');
  if (!path) return;
  const popup = getAgentLogPopup();
  popup.replaceChildren();

  const title = document.createElement('div');
  title.className = 'agent-log-popup-title';
  title.textContent = detail.label || t('agent_log_location');
  const pathEl = document.createElement('code');
  pathEl.className = 'agent-log-popup-path';
  pathEl.textContent = path;
  const actions = document.createElement('div');
  actions.className = 'agent-log-popup-actions';

  const copyBtn = document.createElement('button');
  copyBtn.type = 'button';
  copyBtn.textContent = `📋 ${t('agent_log_copy_path')}`;
  copyBtn.addEventListener('click', () => {
    void copyPathText(path, copyBtn).catch(() => showToast(t('agent_log_copy_error'), copyBtn));
  });
  const openBtn = document.createElement('button');
  openBtn.type = 'button';
  openBtn.textContent = `📁 ${t('agent_log_open_folder')}`;
  openBtn.addEventListener('click', async () => {
    try {
      const res = await apiFetch(`/api/agent-log/open?session_id=${encodeURIComponent(sessionId)}`, { method: 'POST' });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
    } catch (_) {
      showToast(t('agent_log_open_error'), openBtn);
    }
  });
  actions.append(copyBtn, openBtn);
  popup.append(title, pathEl, actions);
  popup.hidden = false;
  const rect = anchor.getBoundingClientRect();
  const width = popup.offsetWidth;
  popup.style.left = `${Math.max(8, Math.min(window.innerWidth - width - 8, rect.right - width))}px`;
  popup.style.top = `${Math.max(8, rect.top - popup.offsetHeight - 8)}px`;
  anchor.setAttribute('aria-expanded', 'true');
}

if (agentLogBtn) {
  agentLogBtn.addEventListener('click', async (event) => {
    event.stopPropagation();
    const sessionId = activeSessionId;
    if (sessionId === null) {
      showToast(t('agent_log_no_session'), agentLogBtn);
      return;
    }
    try {
      const res = await apiFetch(`/api/agent-log?session_id=${encodeURIComponent(sessionId)}`);
      const detail = await res.json() as AgentLogResponse;
      if (!res.ok || !detail.available) {
        showToast(detail.reason || t('agent_log_unavailable'), agentLogBtn);
        return;
      }
      showAgentLogPopup(agentLogBtn, sessionId, detail);
    } catch (_) {
      showToast(t('agent_log_unavailable'), agentLogBtn);
    }
  });
  document.addEventListener('click', closeAgentLogPopup);
  window.addEventListener('resize', closeAgentLogPopup);
}

window.addEventListener('paste', (e) => {
  if (activeSessionId === null) return;
  const items = e.clipboardData?.items;
  if (!items) return;

  // ファイルを優先（画像 or その他ファイル）
  let hasFile = false;
  for (const item of items) {
    if (item.kind !== 'file') continue;
    const file = item.getAsFile();
    if (!file) continue;
    hasFile = true;
    if (isImageFile(file)) stageAttach(file, { normalize: true });
    else stageFileAttach(file);
  }
  if (hasFile) return;

  // 長いテキストはチップに折りたたむ
  const text = e.clipboardData?.getData('text');
  if (text && stagePastedText(text)) e.preventDefault();
});

if (attachDropZone) {
  attachDropZone.addEventListener('dragover', (e) => {
    e.preventDefault();
    attachDropZone.classList.add('dragover');
  });
  attachDropZone.addEventListener('dragleave', () => attachDropZone.classList.remove('dragover'));
  attachDropZone.addEventListener('drop', (e) => {
    e.preventDefault();
    attachDropZone.classList.remove('dragover');
    if (activeSessionId === null) return;
    for (const file of e.dataTransfer?.files ?? []) {
      if (isImageFile(file)) stageAttach(file);
      else stageFileAttach(file);
    }
  });
  attachDropZone.addEventListener('click', () => attachFileInput?.click());
}

if (attachFileInput) {
  attachFileInput.addEventListener('change', () => {
    for (const file of attachFileInput.files ?? []) {
      if (isImageFile(file)) stageAttach(file);
      else stageFileAttach(file);
    }
    attachFileInput.value = '';
  });
}

// B2: スマホからの「📷 撮影」ボタン → カメラ起動 input.click()。
// クリップボード画像と同じ normalize:true ルートで PNG 圧縮してから添付する。
if (attachCameraBtn && attachCameraInput) {
  attachCameraBtn.addEventListener('click', () => attachCameraInput?.click());
  attachCameraInput.addEventListener('change', () => {
    for (const file of attachCameraInput.files ?? []) {
      if (isImageFile(file)) stageAttach(file, { normalize: true });
    }
    attachCameraInput.value = '';
  });
}

// セッション内どこでもD&D（terminal-wrapper全体）
export const terminalWrapper = document.getElementById('terminal-wrapper');
if (terminalWrapper) {
  terminalWrapper.addEventListener('click', (e) => {
    if (activeSessionId === null) return;
    if (isInteractiveFocusTarget(e.target)) return;
    // DOM テキスト選択中は入力欄へフォーカスすると選択が消える
    const domSel = window.getSelection();
    if (domSel && !domSel.isCollapsed && String(domSel).length > 0) return;
    const xt = terminals.get(activeSessionId);
    if (!xt?.term.hasSelection()) inputEl.focus();
  });

  // xterm.js canvas が click イベントを止める場合のフォールバック:
  // mouseup は canvas からもバブルするため、こちらで確実にフォーカスを戻す。
  // ただし Grok 会話履歴ビューア等の DOM オーバーレイ上では、xterm 未選択の
  // まま input にフォーカスするとブラウザ選択が消えてコピー不能になる。
  document.getElementById('terminal-area-wrapper')?.addEventListener('mouseup', (e) => {
    if (activeSessionId === null) return;
    if (isInteractiveFocusTarget(e.target)) return;
    const domSel = window.getSelection();
    if (domSel && !domSel.isCollapsed && String(domSel).length > 0) return;
    const xt = terminals.get(activeSessionId);
    // 50ms 待って xterm の選択状態が確定してから判定
    setTimeout(() => {
      if (isInteractiveFocusTarget(e.target)) return;
      const still = window.getSelection();
      if (still && !still.isCollapsed && String(still).length > 0) return;
      if (!xt?.term.hasSelection()) inputEl.focus();
    }, 50);
  });

  terminalWrapper.addEventListener('dragenter', (e) => {
    if (!e.dataTransfer?.types.includes('Files')) return;
    e.preventDefault();
    terminalWrapper.classList.add('drag-active');
  });
  terminalWrapper.addEventListener('dragleave', (e) => {
    if (!terminalWrapper.contains(e.relatedTarget)) {
      terminalWrapper.classList.remove('drag-active');
    }
  });
  terminalWrapper.addEventListener('dragover', (e) => {
    if (e.dataTransfer?.types.includes('Files')) e.preventDefault();
  });
  terminalWrapper.addEventListener('drop', (e) => {
    e.preventDefault();
    terminalWrapper.classList.remove('drag-active');
    if (activeSessionId === null) return;
    for (const file of e.dataTransfer?.files ?? []) {
      if (isImageFile(file)) stageAttach(file);
      else stageFileAttach(file);
    }
  });
}

// チャットペインへの D&D
export const chatPane = document.getElementById('chat-pane');
if (chatPane) {
  chatPane.addEventListener('dragenter', (e) => {
    if (!e.dataTransfer?.types.includes('Files')) return;
    e.preventDefault();
    chatPane.classList.add('drag-active');
  });
  chatPane.addEventListener('dragleave', (e) => {
    if (!chatPane.contains(e.relatedTarget)) {
      chatPane.classList.remove('drag-active');
    }
  });
  chatPane.addEventListener('dragover', (e) => {
    if (e.dataTransfer?.types.includes('Files')) e.preventDefault();
  });
  chatPane.addEventListener('drop', (e) => {
    e.preventDefault();
    chatPane.classList.remove('drag-active');
    if (activeSessionId === null) return;
    for (const file of e.dataTransfer?.files ?? []) {
      if (isImageFile(file)) stageAttach(file);
      else stageFileAttach(file);
    }
  });
}

export async function stageAttach(file, opts: any = {}) {
  const normalized = opts.normalize ? await normalizeAttachImage(file) : file;
  const buf = await normalized.arrayBuffer();
  if (buf.byteLength > MAX_ATTACH_BYTES) {
    showToast(`Attachment too large: ${(buf.byteLength / (1024 * 1024)).toFixed(1)}MB (max 8MB)`);
    return;
  }
  const entry: any = {};
  const wrapper = addAttachThumbnail(normalized, () => {
    const idx = pendingAttachFiles.findIndex(p => p.entry === entry);
    if (idx !== -1) pendingAttachFiles.splice(idx, 1);
    updateInputAffordance();
  });
  entry.wrapper = wrapper;
  pendingAttachFiles.push({ buf, filename: normalized.name || '', entry, wrapper });
  updateInputAffordance();
}

export async function stageFileAttach(file) {
  const buf = await file.arrayBuffer();
  if (buf.byteLength > MAX_ATTACH_BYTES) {
    showToast(`Attachment too large: ${(buf.byteLength / (1024 * 1024)).toFixed(1)}MB (max 8MB)`);
    return;
  }
  const entry: any = {};
  const wrapper = addFileChip(file, () => {
    const idx = pendingAttachFiles.findIndex(p => p.entry === entry);
    if (idx !== -1) pendingAttachFiles.splice(idx, 1);
    updateInputAffordance();
  });
  entry.wrapper = wrapper;
  pendingAttachFiles.push({ buf, filename: file.name || '', entry, wrapper });
  updateInputAffordance();
}

// クリップボード画像は元ファイル名がないことが多いため、PNG として保存する。
// PNG が大きすぎる場合は、容量に応じて段階的に縮小する。
// D&D/ファイル選択では元ファイルをそのまま送る。
export async function normalizeAttachImage(file) {
  let bmp: ImageBitmap | null = null;
  try {
    const maxEdge = 1568;
    bmp = await createImageBitmap(file);
    const w = bmp.width;
    const h = bmp.height;
    let scale = Math.min(1, maxEdge / Math.max(w, h));
    let lastBlob: Blob | null = null;

    for (let i = 0; i < 12; i++) {
      const outW = Math.max(1, Math.round(w * scale));
      const outH = Math.max(1, Math.round(h * scale));
      const canvas = document.createElement('canvas');
      canvas.width = outW;
      canvas.height = outH;
      const ctx = canvas.getContext('2d');
      if (!ctx) return file;
      ctx.drawImage(bmp, 0, 0, outW, outH);

      const blob = await new Promise<Blob | null>((resolve) => {
        canvas.toBlob(resolve, 'image/png');
      });
      if (!blob) return file;
      lastBlob = blob;
      if (blob.size <= MAX_ATTACH_BYTES) {
        const base = (file.name || 'image').replace(/\.[^.]+$/, '');
        return new File([blob], `${base}.png`, { type: 'image/png' });
      }

      const sizeRatio = MAX_ATTACH_BYTES / blob.size;
      const shrink = Math.max(0.35, Math.min(0.85, Math.sqrt(sizeRatio) * 0.9));
      scale *= shrink;
      if (outW === 1 && outH === 1) break;
    }

    if (lastBlob) {
      const base = (file.name || 'image').replace(/\.[^.]+$/, '');
      return new File([lastBlob], `${base}.png`, { type: 'image/png' });
    }
    return file;
  } catch (_) {
    return file;
  } finally {
    if (bmp) bmp.close();
  }
}

export function arrayBufferToBase64(buf) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const s = String(reader.result || '');
      const comma = s.indexOf(',');
      resolve(comma >= 0 ? s.slice(comma + 1) : s);
    };
    reader.onerror = () => reject(reader.error || new Error('base64 encode failed'));
    reader.readAsDataURL(new Blob([buf]));
  });
}

export async function flushPendingAttach(sessionId) {
  if (pendingAttachFiles.length === 0) return [];
  const toSend = pendingAttachFiles.splice(0);
  const injects = [];
  // chatHistory 用: 送信に成功した添付の情報を集める
  const historyAttachments = [];
  for (const { buf, filename, wrapper } of toSend) {
    try {
      const formData = new FormData();
      formData.append('file', new Blob([buf]), filename || 'blob');
      const res = await fetch(
        `/api/attach?token=${encodeURIComponent(token)}&session_id=${encodeURIComponent(sessionId)}`,
        { method: 'POST', body: formData }
      );
      if (!res.ok) {
        showToast(`Attachment failed: HTTP ${res.status}`);
      } else {
        try {
          const data = await res.json();
          if (data && data.inject) injects.push(data.inject);
          const attachKind = /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(filename || '') ? 'image' : 'file';
          const blob = new Blob([buf]);
          historyAttachments.push({
            filename: filename || '',
            byteLength: (buf && buf.byteLength) || 0,
            kind: attachKind,
            path: data && data.saved_path ? data.saved_path : null,
            url: attachKind === 'image' ? URL.createObjectURL(blob) : null,
          });
        } catch (_) {
          showToast('Attachment response parse failed');
        }
      }
    } catch (_) {
      showToast('Attachment send failed');
    }
    if (wrapper) {
      wrapper.remove();
      updateAttachClearBtn();
    }
  }
  // chatHistory: attach を user/attach として 1 メッセージにまとめて push
  if (historyAttachments.length > 0) {
    pushMessage(sessionId, {
      role: 'user',
      kind: 'attach',
      attachments: historyAttachments,
      rawText: '',
    });
  }
  return injects;
}

export function addAttachThumbnail(file, onRemove) {
  if (!attachThumbnails) return;
  const url = URL.createObjectURL(file);

  const wrapper = document.createElement('div');
  wrapper.className = 'attach-thumb-wrapper';

  const img = document.createElement('img');
  img.src = url;
  img.className = 'attach-thumb';
  img.title = (file.name || 'image') + t('expand_image');
  img.addEventListener('click', () => openLightbox(img.src));

  const removeBtn = document.createElement('button');
  removeBtn.className = 'attach-thumb-remove';
  removeBtn.textContent = t('remove');
  removeBtn.title = t('delete_attach');
  removeBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    URL.revokeObjectURL(url);
    wrapper.remove();
    updateAttachClearBtn();
    onRemove?.();
  });

  wrapper.appendChild(img);
  wrapper.appendChild(removeBtn);
  attachThumbnails.prepend(wrapper);

  const wrappers = attachThumbnails.querySelectorAll('.attach-thumb-wrapper');
  for (let i = 10; i < wrappers.length; i++) {
    URL.revokeObjectURL(wrappers[i].querySelector('img').src);
    wrappers[i].remove();
  }
  updateAttachClearBtn();
  return wrapper;
}

export function addFileChip(file, onRemove) {
  if (!attachThumbnails) return;
  const wrapper = document.createElement('div');
  wrapper.className = 'attach-thumb-wrapper attach-file-chip';

  const label = document.createElement('span');
  label.className = 'attach-file-name';
  label.textContent = t('file_chip_label', { name: file.name || 'file' });
  label.title = file.name || 'file';

  const removeBtn = document.createElement('button');
  removeBtn.className = 'attach-thumb-remove';
  removeBtn.textContent = t('remove');
  removeBtn.title = t('delete_attach');
  removeBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    wrapper.remove();
    updateAttachClearBtn();
    onRemove?.();
  });

  wrapper.appendChild(label);
  wrapper.appendChild(removeBtn);
  attachThumbnails.prepend(wrapper);
  updateAttachClearBtn();
  return wrapper;
}

export function openLightbox(src, opts: any = {}) {
  const overlay = document.createElement('div');
  overlay.id = 'image-lightbox';
  const isVideo = opts.type === 'video';
  const media: any = document.createElement(isVideo ? 'video' : 'img');
  if (isVideo) {
    media.controls = true;
    media.autoplay = true;
    media.playsInline = true;
  }
  media.src = src;
  overlay.appendChild(media);
  document.body.appendChild(overlay);
  const close = () => {
    if (isVideo) {
      try { media.pause(); } catch (_) {}
    }
    overlay.remove();
    document.removeEventListener('keydown', onKey);
  };
  const onKey = (e) => { if (e.key === 'Escape') close(); };
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) close();
  });
  document.addEventListener('keydown', onKey);
}
