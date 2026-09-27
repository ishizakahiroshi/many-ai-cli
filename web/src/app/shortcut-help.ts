// shortcut-help.ts — `?` キーで開くキーボードショートカット一覧のオーバーレイ。
//
// README.md / README.ja.md の「キーボードショートカット」表と同じ内容を画面からも
// 見られるようにする（ショートカットは既にいくつもあるのに、画面から一覧を見る
// 手段が無かった）。
//
// 子 plan: docs/local/plan_ux-notify-palette-review_c3_palette.md 内部 C3。
import { t } from '../i18n.js';

let overlay: HTMLElement | null = null;

// README の 12 行 + Ctrl+K・Alt+1..9・? の 3 行。キー表記と i18n キーの対応をここに
// 1 本化しておく（README の表を更新するときはこの配列も一緒に見直すこと）。
const SHORTCUT_ROWS: ReadonlyArray<readonly [string, string]> = [
  ['Enter', 'shortcut_help_row_send'],
  ['Shift+Enter', 'shortcut_help_row_newline'],
  ['Tab / Shift+Tab', 'shortcut_help_row_tab'],
  ['← / →', 'shortcut_help_row_arrows'],
  ['Enter', 'shortcut_help_row_action_bar_run'],
  ['Alt+V', 'shortcut_help_row_voice'],
  ['Ctrl+Shift+G', 'shortcut_help_row_git_tab'],
  ['Ctrl+Shift+F', 'shortcut_help_row_files_tab'],
  ['Ctrl+V', 'shortcut_help_row_paste_image'],
  ['Ctrl+C', 'shortcut_help_row_sigint'],
  ['Ctrl+D', 'shortcut_help_row_eof'],
  ['Ctrl+O', 'shortcut_help_row_expand_fold'],
  ['Ctrl+K', 'shortcut_help_row_palette'],
  ['Alt+1..9', 'shortcut_help_row_jump_session'],
  ['?', 'shortcut_help_row_show_this'],
];

function isTypingTarget(el: Element | null): boolean {
  if (!el) return false;
  const tag = el.tagName;
  if (tag === 'INPUT' || tag === 'TEXTAREA') return true;
  return (el as HTMLElement).isContentEditable === true;
}

// 他の全画面オーバーレイ（.aac-wheel-overlay、自分自身は hidden の間しか該当しない）が
// 開いている間は何もしない。
function isOtherOverlayOpen(): boolean {
  return !!document.querySelector('.aac-wheel-overlay:not([hidden])');
}

function ensureOverlay(): HTMLElement {
  if (overlay) return overlay;
  const root = document.createElement('div');
  root.id = 'shortcut-help-overlay';
  root.classList.add('aac-wheel-overlay');
  root.hidden = true;
  root.setAttribute('role', 'dialog');
  root.setAttribute('aria-modal', 'true');
  root.setAttribute('aria-label', t('shortcut_help_title'));
  root.addEventListener('mousedown', (event) => { if (event.target === root) closeShortcutHelp(); });

  const dialog = document.createElement('section');
  dialog.className = 'shortcut-help-dialog';

  const header = document.createElement('div');
  header.className = 'shortcut-help-header';
  const title = document.createElement('strong');
  title.textContent = t('shortcut_help_title');
  const close = document.createElement('button');
  close.type = 'button';
  close.className = 'shortcut-help-close';
  close.textContent = 'Esc';
  close.addEventListener('click', closeShortcutHelp);
  header.append(title, close);

  const table = document.createElement('table');
  table.className = 'shortcut-help-table';
  const tbody = document.createElement('tbody');
  for (const [key, labelKey] of SHORTCUT_ROWS) {
    const row = document.createElement('tr');
    const keyCell = document.createElement('td');
    keyCell.className = 'shortcut-help-key';
    const kbd = document.createElement('kbd');
    kbd.textContent = key;
    keyCell.appendChild(kbd);
    const actionCell = document.createElement('td');
    actionCell.textContent = t(labelKey);
    row.append(keyCell, actionCell);
    tbody.appendChild(row);
  }
  table.appendChild(tbody);

  dialog.append(header, table);
  root.appendChild(dialog);
  document.body.appendChild(root);
  overlay = root;
  return root;
}

export function openShortcutHelp(): void {
  const el = ensureOverlay();
  el.hidden = false;
}

export function closeShortcutHelp(): void {
  if (overlay) overlay.hidden = true;
}

export function initShortcutHelp(): void {
  document.addEventListener('keydown', (event) => {
    if (event.key !== '?') return;
    const isOpen = !!overlay && !overlay.hidden;
    if (isOpen) {
      event.preventDefault();
      closeShortcutHelp();
      return;
    }
    if (isTypingTarget(document.activeElement)) return;
    if (isOtherOverlayOpen()) return;
    event.preventDefault();
    openShortcutHelp();
  }, true);
}
