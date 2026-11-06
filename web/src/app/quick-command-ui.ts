// クイックコマンドの一覧。2 か所に同じ内容を出す。
//  - 進捗帯（またはクイックコマンドのボタン）の右クリックで開く一覧: 各行で表示の ON/OFF・編集・削除、末尾で追加
//  - 設定画面の「クイックコマンド」: 同じ操作を行で出す（右クリックを知らなくても追加できるようにするため）
// 保存の形は settings.ts の「スロット番号ごとの cmd / show」のまま。内容が空のスロットは未登録として一覧に出さない。
// 帯の並び（input-icon-position.ts）と同じ順に並べる。
import { t } from '../i18n.js';
import { getDockedBandRect, getInputIconOrder } from './input-icon-position.js';
import {
  QUICK_CMD_SLOTS, deleteQuickCommand, editQuickCommand, firstFreeQuickSlot, getQuickCommand,
  getQuickCommandVisible, quickCommandButtonId, registeredQuickSlots, setQuickCommandText,
  setQuickCommandVisible,
} from './settings.js';
import { uiButtonClass, uiIconHtml } from './ui-components.js';

/** 登録済みのスロットを、帯のボタンと同じ並びで返す。 */
function slotsInBandOrder(): number[] {
  const order = getInputIconOrder() as readonly string[];
  const rank = (slot: number): number => {
    const index = order.indexOf(quickCommandButtonId(slot) || '');
    return index < 0 ? Number.MAX_SAFE_INTEGER : index;
  };
  return registeredQuickSlots().sort((a, b) => rank(a) - rank(b) || a - b);
}

function limitNote(): string {
  return t('quick_cmd_limit', { max: String(QUICK_CMD_SLOTS) });
}

function iconButton(kind: 'edit' | 'delete', label: string, onClick: () => void): HTMLButtonElement {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = uiButtonClass('quiet', 'compact', { icon: true });
  btn.innerHTML = uiIconHtml(kind);
  btn.setAttribute('aria-label', label);
  btn.dataset.tooltip = label;
  btn.addEventListener('click', event => {
    event.stopPropagation();
    onClick();
  });
  return btn;
}

// ---- 右クリックの一覧 ----

let menuEl: HTMLDivElement | null = null;

function closeMenu(): void {
  menuEl?.classList.remove('open');
}

function isMenuOpen(): boolean {
  return !!menuEl && menuEl.classList.contains('open');
}

function renderMenu(menu: HTMLDivElement): void {
  menu.replaceChildren();
  const title = document.createElement('div');
  title.className = 'quick-cmd-menu-title';
  title.textContent = t('quick_cmd_menu_title');
  menu.append(title);

  const slots = slotsInBandOrder();
  if (!slots.length) {
    const empty = document.createElement('div');
    empty.className = 'quick-cmd-menu-empty';
    empty.textContent = t('quick_cmd_empty');
    menu.append(empty);
  }
  for (const slot of slots) {
    const visible = getQuickCommandVisible(slot);
    const cmd = getQuickCommand(slot);
    const row = document.createElement('div');
    row.className = 'quick-cmd-menu-row';
    row.setAttribute('role', 'none');

    const toggle = document.createElement('button');
    toggle.type = 'button';
    toggle.className = `${uiButtonClass('quiet', 'compact', { menuItem: true })} quick-cmd-menu-toggle`;
    toggle.setAttribute('role', 'menuitemcheckbox');
    toggle.setAttribute('aria-checked', String(visible));
    toggle.dataset.tooltip = t('settings_quick_cmd_show_tooltip');
    const mark = document.createElement('span');
    mark.className = 'ctx-icon';
    mark.textContent = visible ? '✓' : '';
    const text = document.createElement('span');
    text.className = 'ctx-label quick-cmd-menu-text';
    text.textContent = cmd;
    toggle.append(mark, text);
    // 表示の切り替えでは一覧を閉じない（続けて何個か切り替えられるように）。
    toggle.addEventListener('click', () => setQuickCommandVisible(slot, !getQuickCommandVisible(slot)));

    row.append(
      toggle,
      iconButton('edit', t('quick_cmd_edit'), () => { closeMenu(); editQuickCommand(slot); }),
      iconButton('delete', t('quick_cmd_delete'), () => deleteQuickCommand(slot)),
    );
    menu.append(row);
  }

  const sep = document.createElement('div');
  sep.className = 'quick-cmd-menu-sep';
  menu.append(sep);
  const free = firstFreeQuickSlot();
  const add = document.createElement('button');
  add.type = 'button';
  add.className = uiButtonClass('quiet', 'compact', { menuItem: true });
  add.setAttribute('role', 'menuitem');
  const addIcon = document.createElement('span');
  addIcon.className = 'ctx-icon';
  addIcon.textContent = '＋';
  const addLabel = document.createElement('span');
  addLabel.className = 'ctx-label';
  addLabel.textContent = free === null ? limitNote() : t('quick_cmd_add_item');
  add.append(addIcon, addLabel);
  add.disabled = free === null;
  add.addEventListener('click', () => {
    const slot = firstFreeQuickSlot();
    if (slot === null) return;
    closeMenu();
    editQuickCommand(slot);
  });
  menu.append(add);
}

function ensureMenu(): HTMLDivElement {
  if (menuEl) return menuEl;
  const menu = document.createElement('div');
  menu.className = 'term-ctx-menu quick-cmd-menu';
  menu.setAttribute('role', 'menu');
  document.body.append(menu);
  document.addEventListener('mousedown', event => {
    if (isMenuOpen() && !menu.contains(event.target as Node)) closeMenu();
  }, true);
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape' && isMenuOpen()) closeMenu();
  });
  window.addEventListener('blur', closeMenu);
  window.addEventListener('resize', closeMenu);
  menuEl = menu;
  return menu;
}

function openMenu(x: number, y: number): void {
  const menu = ensureMenu();
  renderMenu(menu);
  menu.classList.add('open');
  const rect = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(0, Math.min(x, window.innerWidth - rect.width - 4))}px`;
  menu.style.top = `${Math.max(0, Math.min(y, window.innerHeight - rect.height - 4))}px`;
}

// ---- 設定画面の一覧 ----

function renderSettingsList(): void {
  const host = document.getElementById('quick-cmd-list');
  if (!host) return;
  // 文字を入力している途中は描き直さない（入力欄が作り直されて入力が途切れるため）。
  const active = document.activeElement;
  if (active instanceof HTMLInputElement && active.type === 'text' && host.contains(active)) return;

  host.replaceChildren();
  const slots = slotsInBandOrder();
  if (!slots.length) {
    const empty = document.createElement('div');
    empty.className = 'quick-cmd-empty';
    empty.textContent = t('quick_cmd_empty');
    host.append(empty);
  }
  for (const slot of slots) {
    const row = document.createElement('div');
    row.className = 'settings-row quick-cmd-row';

    const show = document.createElement('label');
    show.className = 'quick-cmd-show';
    show.dataset.tooltip = t('settings_quick_cmd_show_tooltip');
    const check = document.createElement('input');
    check.type = 'checkbox';
    check.className = 'settings-checkbox';
    check.checked = getQuickCommandVisible(slot);
    check.addEventListener('change', () => setQuickCommandVisible(slot, check.checked));
    const showText = document.createElement('span');
    showText.textContent = t('settings_quick_cmd_show_short');
    show.append(check, showText);

    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'ui-input settings-input-text quick-cmd-input';
    input.setAttribute('list', 'quick-cmd-presets');
    input.maxLength = 200;
    input.autocomplete = 'off';
    input.placeholder = t('settings_quick_cmd_placeholder');
    input.value = getQuickCommand(slot);
    input.setAttribute('aria-label', t('settings_quick_cmd_label'));
    // 保存は入力が確定したとき（change）。空にして確定すると登録が消える。
    input.addEventListener('change', () => {
      const before = getQuickCommand(slot);
      setQuickCommandText(slot, input.value);
      if (getQuickCommand(slot) === before) input.value = before;
      // 空にして確定した行は登録が消えたので、入力の途中ではなくなったこの時点で一覧を描き直す。
      if (getQuickCommand(slot) === '') {
        input.blur();
        renderSettingsList();
      }
    });

    row.append(show, input, iconButton('delete', t('quick_cmd_delete'), () => deleteQuickCommand(slot)));
    host.append(row);
  }

  const add = document.getElementById('quick-cmd-add-btn') as HTMLButtonElement | null;
  const note = document.getElementById('quick-cmd-limit-note');
  const free = firstFreeQuickSlot();
  if (add) add.disabled = free === null;
  if (note) note.textContent = free === null ? limitNote() : '';
}

function refreshLists(): void {
  if (isMenuOpen() && menuEl) renderMenu(menuEl);
  renderSettingsList();
}

let initialized = false;

/** 起動時に 1 回だけ呼ぶ。右クリックの受け口と設定画面の一覧をつなぐ。 */
export function initQuickCommandUi(): void {
  if (initialized) return;
  initialized = true;

  // 帯の中（メッセージ・空き地・ボタンの上）の右クリックは、どこでも同じ一覧を開く。
  // 帯は pointer-events: none なので、イベントの宛先ではなく座標で判定する。
  window.addEventListener('contextmenu', event => {
    const rect = getDockedBandRect();
    if (!rect) return;
    if (event.clientX < rect.left || event.clientX > rect.right
      || event.clientY < rect.top || event.clientY > rect.bottom) return;
    // 開いているダイアログ（設定・編集など）の上では、一覧を出さず通常の動作に任せる。
    if (document.querySelector('dialog[open]')) return;
    event.preventDefault();
    event.stopPropagation();
    openMenu(event.clientX, event.clientY);
  }, true);

  // 帯が無い画面（スマホ幅など）でも、クイックコマンドのボタン自身の右クリックで同じ一覧を開く。
  for (let slot = 1; slot <= QUICK_CMD_SLOTS; slot++) {
    const id = quickCommandButtonId(slot);
    document.getElementById(id || '')?.addEventListener('contextmenu', event => {
      event.preventDefault();
      event.stopPropagation();
      openMenu(event.clientX, event.clientY);
    });
  }

  document.getElementById('quick-cmd-add-btn')?.addEventListener('click', () => {
    const slot = firstFreeQuickSlot();
    if (slot !== null) editQuickCommand(slot);
  });
  document.addEventListener('quickcommands:changed', refreshLists);
  document.addEventListener('i18n-ready', refreshLists);
  // 設定の節を開いたとき、帯での並べ替えを一覧へ反映する。
  document.querySelector('details[data-section="quick-cmd"]')?.addEventListener('toggle', renderSettingsList);
  renderSettingsList();
}
