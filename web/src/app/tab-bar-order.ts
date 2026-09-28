// 統合タブバー（#unified-tab-bar）のタブをドラッグ&ドロップで並べ替える。
//
// 使わないタブ（Files / Git / 履歴 等）を右へ寄せて、日常的に使うタブだけを
// 左端に集められるようにするための機能。並び順は localStorage にブラウザ単位で
// 保存する（サーバー設定には持たせない＝端末ごとに好みが違うため）。
//
// 設計メモ:
//  - タブは index.html に静的に書かれている。ここでは DOM の並びだけを入れ替え、
//    タブの中身・クリック配線（settings.ts の wireUnifiedTabBar）には触らない。
//  - 保存済みの順序に無いタブ（将来追加される新タブ）は末尾へ回す。消えたタブの
//    名前が保存に残っていても単に無視する。
//  - モバイルのドロワー（mobile-home.ts）は #unified-tab-bar の DOM 順を読むので、
//    ここで並べ替えると自動的に同じ順序になる。
//  - HTML5 の drag&drop はタッチ端末では発火しない。並べ替えはデスクトップ専用で、
//    タッチ環境では従来どおりクリックだけが動く。

import { t } from '../i18n.js';
import { activeSessionId } from './state.js';

const STORAGE_KEY = 'unifiedTabOrder';
const PANE_DRAG_TYPE = 'application/x-many-ai-cli-pane';
const NON_PANE_TABS = new Set(['multi', 'split']);

function barEl(): HTMLElement | null {
  return document.getElementById('unified-tab-bar');
}

function tabsOf(bar: HTMLElement): HTMLElement[] {
  return Array.from(bar.querySelectorAll<HTMLElement>('.view-tab'));
}

function placementButtonFor(tab: HTMLElement): HTMLElement | null {
  const next = tab.nextElementSibling as HTMLElement | null;
  return next?.classList.contains('tab-pane-placement-btn') && next.dataset.tab === tab.dataset.tab ? next : null;
}

function tabAt(target: EventTarget | null): HTMLElement | null {
  const element = target instanceof Element ? target : null;
  const tab = element?.closest<HTMLElement>('.view-tab');
  if (tab) return tab;
  const button = element?.closest<HTMLElement>('.tab-pane-placement-btn');
  const previous = button?.previousElementSibling as HTMLElement | null;
  return previous?.classList.contains('view-tab') ? previous : null;
}

function insertTabPairBefore(bar: HTMLElement, tab: HTMLElement, reference: Node | null): void {
  const button = placementButtonFor(tab);
  if (reference === tab || reference === button) return;
  bar.insertBefore(tab, reference);
  if (button) bar.insertBefore(button, reference);
}

function tabPairNextSibling(tab: HTMLElement): Node | null {
  return (placementButtonFor(tab) || tab).nextSibling;
}

function placementPayload(tabName: string): { kind: 'tab'; tabName: string; sessionId: number | null } {
  return { kind: 'tab', tabName, sessionId: activeSessionId };
}

function loadOrder(): string[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((v): v is string => typeof v === 'string');
  } catch (_) {
    return [];
  }
}

function saveOrder(names: string[]): void {
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(names)); } catch (_) { /* private mode 等は無視 */ }
}

/** 現在の DOM 順を localStorage へ書き戻す。 */
function persistCurrentOrder(bar: HTMLElement): void {
  const names = tabsOf(bar)
    .map(el => el.dataset.tab)
    .filter((v): v is string => !!v);
  saveOrder(names);
}

/**
 * 保存済みの順序を DOM へ適用する。
 * タブ群は HTML 上で連続しているので、末尾タブの次ノードを挿入基準にして
 * 順に insertBefore すれば、セッション情報チップや #main-tab-bar の位置は動かない。
 */
function applyOrder(bar: HTMLElement, order: string[]): void {
  const tabs = tabsOf(bar);
  if (tabs.length === 0) return;
  const known = new Map<string, HTMLElement>();
  tabs.forEach(el => { if (el.dataset.tab) known.set(el.dataset.tab, el); });

  const ordered: HTMLElement[] = [];
  for (const name of order) {
    const el = known.get(name);
    if (el && !ordered.includes(el)) ordered.push(el);
  }
  // 保存に無いタブ（新規追加分）は元の並びのまま末尾へ
  for (const el of tabs) if (!ordered.includes(el)) ordered.push(el);

  const anchor = tabPairNextSibling(tabs[tabs.length - 1]);
  for (const el of ordered) insertTabPairBefore(bar, el, anchor);
}

function clearDropMarks(bar: HTMLElement): void {
  tabsOf(bar).forEach(el => el.classList.remove('drop-before', 'drop-after'));
}

let dragSrc: HTMLElement | null = null;

/** タブバーの並び順を初期化直後の状態（index.html の記述順）へ戻す。 */
export function resetTabBarOrder(): void {
  const bar = barEl();
  if (!bar) return;
  try { localStorage.removeItem(STORAGE_KEY); } catch (_) { /* noop */ }
  applyOrder(bar, DEFAULT_ORDER);
}

// index.html に書かれている本来の並び（リセット用）。init 時に実 DOM から採取する。
let DEFAULT_ORDER: string[] = [];

export function initTabBarOrder(): void {
  const bar = barEl();
  if (!bar) return;

  DEFAULT_ORDER = tabsOf(bar).map(el => el.dataset.tab).filter((v): v is string => !!v);

  applyOrder(bar, loadOrder());

  tabsOf(bar).forEach(el => { el.draggable = true; });

  // タッチとキーボードでも配置できる入口。タブとは兄弟にして、既存のタブクリックと
  // モバイルドロワーの tab.innerHTML 複製へ操作ボタンが紛れ込まないようにする。
  for (const tab of tabsOf(bar)) {
    const tabName = tab.dataset.tab;
    if (!tabName || NON_PANE_TABS.has(tabName)) continue;
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'tab-pane-placement-btn';
    button.dataset.tab = tabName;
    button.textContent = '＋';
    const updateLabel = () => {
      const placeLabel = t('pane_show_in_pane');
      const tabLabel = tab.getAttribute('aria-label')
        || tab.querySelector<HTMLElement>('[data-i18n^="tab_"]')?.textContent?.trim()
        || tab.textContent?.trim()
        || tabName;
      button.title = placeLabel;
      button.setAttribute('aria-label', `${tabLabel}: ${placeLabel}`);
    };
    updateLabel();
    // i18n の辞書取得は非同期。初期化後に辞書とタブ名が揃った時点で更新する。
    document.addEventListener('i18n-ready', updateLabel, { once: true });
    button.addEventListener('click', () => {
      window.dispatchEvent(new CustomEvent('pane-placement-request', {
        detail: { ...placementPayload(tabName), anchor: button },
      }));
    });
    tab.after(button);
  }

  bar.addEventListener('dragstart', (e: DragEvent) => {
    const tab = (e.target as HTMLElement | null)?.closest<HTMLElement>('.view-tab');
    if (!tab || !bar.contains(tab)) return;
    dragSrc = tab;
    tab.classList.add('dragging');
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      // Firefox はデータを載せないと dragstart 自体が成立しない
      try { e.dataTransfer.setData('text/plain', tab.dataset.tab || ''); } catch (_) { /* noop */ }
      const tabName = tab.dataset.tab;
      if (tabName && !NON_PANE_TABS.has(tabName)) {
        try { e.dataTransfer.setData(PANE_DRAG_TYPE, JSON.stringify(placementPayload(tabName))); } catch (_) { /* noop */ }
      }
    }
  });

  bar.addEventListener('dragend', () => {
    if (dragSrc) dragSrc.classList.remove('dragging');
    dragSrc = null;
    clearDropMarks(bar);
  });

  // タブ上なら左右半分でその前後へ、タブ以外（バーの余白）なら末尾へ落とす
  bar.addEventListener('dragover', (e: DragEvent) => {
    if (!dragSrc) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    const tab = tabAt(e.target);
    clearDropMarks(bar);
    if (!tab || tab === dragSrc) return;
    const rect = tab.getBoundingClientRect();
    const after = e.clientX >= rect.left + rect.width / 2;
    tab.classList.add(after ? 'drop-after' : 'drop-before');
  });

  bar.addEventListener('dragleave', (e: DragEvent) => {
    const tab = tabAt(e.target);
    if (tab) tab.classList.remove('drop-before', 'drop-after');
  });

  bar.addEventListener('drop', (e: DragEvent) => {
    if (!dragSrc) return;
    e.preventDefault();
    clearDropMarks(bar);
    const tab = tabAt(e.target);
    if (tab === dragSrc) return;
    if (tab) {
      const rect = tab.getBoundingClientRect();
      const after = e.clientX >= rect.left + rect.width / 2;
      insertTabPairBefore(bar, dragSrc, after ? tabPairNextSibling(tab) : tab);
    } else {
      const tabs = tabsOf(bar);
      const last = tabs[tabs.length - 1];
      if (last && last !== dragSrc) insertTabPairBefore(bar, dragSrc, tabPairNextSibling(last));
    }
    persistCurrentOrder(bar);
  });

  const resetBtn = document.getElementById('tab-order-reset-btn');
  if (resetBtn) resetBtn.addEventListener('click', () => resetTabBarOrder());
}
