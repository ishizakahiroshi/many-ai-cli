// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { copyCleanText } from './util.js';
import { appendLinkedText } from './path-links.js';
import { sendText } from '../app.js';
import { scanBuffer } from './terminal.js';
import { scheduleApprovalCheck } from './approval.js';

// ---- 折りたたみマーカー展開ポップアップ ----
// ターミナル内の「… +N lines (ctrl+o to expand)」マーカーのクリックで起動する。
// ctrl+o は Claude Code 全体の detailed transcript 切替であり、ブロック単位の展開は
// できないため、ctrl+o 往復でバッファ差分（展開で増えた行）を取得し、
// クリック位置近傍の浮動ポップアップに表示する。

let popupEl = null;
let capturePending = false;

function getOrCreateExpandPopup() {
  if (popupEl) return popupEl;
  popupEl = document.createElement('div');
  popupEl.id = 'expand-capture-popup';
  popupEl.hidden = true;
  document.body.appendChild(popupEl);

  // ポップアップ外クリック/タップで閉じる（ポップアップ内の操作では閉じない）。
  // モバイルは mousedown の合成に頼らず touchstart も拾う（settings.js の外側閉じと同方式）。
  const onOutsidePointer = (e) => {
    if (popupEl.hidden) return;
    if (popupEl.contains(e.target)) return;
    popupEl.hidden = true;
  };
  document.addEventListener('mousedown', onOutsidePointer, true);
  document.addEventListener('touchstart', onOutsidePointer, true);
  // Esc で閉じる。表示中は他のグローバル Esc ハンドラ（PTY への Esc 送信等）に
  // 渡さないよう capture 段階で握りつぶす。
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || popupEl.hidden) return;
    e.preventDefault();
    e.stopPropagation();
    popupEl.hidden = true;
  }, true);
  return popupEl;
}

export function hideExpandPopup() {
  if (popupEl) popupEl.hidden = true;
}

function positionPopupNear(popup) {
  // モーダルとして画面中央に固定表示する（クリック位置には追従しない）。
  popup.style.left = '50%';
  popup.style.top = '50%';
  popup.style.transform = 'translate(-50%, -50%)';
}

function renderExpandPopup(sessionId, lines, clientX, clientY, loading) {
  const popup = getOrCreateExpandPopup();
  popup.innerHTML = '';
  popup.hidden = false;

  const header = document.createElement('div');
  header.className = 'expand-popup-header';
  const title = document.createElement('span');
  title.className = 'expand-popup-title';
  if (loading) {
    title.textContent = t('expand_popup_loading');
  } else if (lines.length === 0) {
    title.textContent = t('expand_popup_empty');
  } else {
    title.textContent = t('expand_popup_title', { n: lines.length });
  }
  header.appendChild(title);

  const btns = document.createElement('div');
  btns.className = 'expand-popup-btns';
  if (!loading && lines.length > 0) {
    const copyBtn = document.createElement('button');
    copyBtn.type = 'button';
    copyBtn.className = 'expand-popup-copy';
    copyBtn.textContent = '⧉';
    copyBtn.title = t('copy_to_clipboard');
    copyBtn.setAttribute('aria-label', t('copy_to_clipboard'));
    copyBtn.addEventListener('click', () => {
      copyCleanText(lines, copyBtn).catch(() => {});
    });
    btns.appendChild(copyBtn);
  }
  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.className = 'expand-popup-close';
  closeBtn.textContent = '✕';
  closeBtn.addEventListener('click', () => { popup.hidden = true; });
  btns.appendChild(closeBtn);
  header.appendChild(btns);
  popup.appendChild(header);

  if (!loading && lines.length > 0) {
    const pre = document.createElement('pre');
    pre.className = 'expand-popup-content';
    appendLinkedText(pre, lines.join('\n'), sessionId);
    popup.appendChild(pre);
  }

  positionPopupNear(popup);
}

export function handleCrunchLinkClick(sessionId, clientX, clientY) {
  if (capturePending) return;
  capturePending = true;

  // クリック直後にローディング表示を出し、ctrl+o 送信前のバッファをスナップショット
  renderExpandPopup(sessionId, [], clientX, clientY, true);
  const beforeSet = new Set(scanBuffer(sessionId));
  sendText(sessionId, '\x0f'); // ctrl+o（detailed transcript へ切替）

  // 800ms 後にバッファ差分を取得して表示し、ctrl+o を再送して元のコンパクト表示へ戻す
  // （戻さないと Claude Code が「Showing detailed transcript」モードに張り付き、
  //  入力プロンプトが見えなくなって「セッション切れ？」と誤認される）
  setTimeout(() => {
    const afterLines = scanBuffer(sessionId);
    const expanded = afterLines.filter(l => l.trim() && !beforeSet.has(l));

    sendText(sessionId, '\x0f'); // ctrl+o（コンパクト表示へ戻す）
    capturePending = false;

    // ローディング表示中にユーザーが閉じていたら結果は表示しない
    if (!popupEl || popupEl.hidden) return;
    renderExpandPopup(sessionId, expanded, clientX, clientY, false);
    // ポップアップ表示中に承認プロンプトが来ていた場合を検出するため再評価
    scheduleApprovalCheck(sessionId);
  }, 800);
}
