// 入力欄の下段ボタン列（■送信/停止・🎤・▤・クイックコマンド・/ ▾・⌫ 等）を
// ドラッグ&ドロップで並べ替える。統合タブバーの並べ替え（tab-bar-order.ts）と同じ操作感。
//
// 設計メモ:
//  - DOM は動かさず、CSS の order だけで並びを変える。DOM 順は ⇄（左右切替・app.ts の
//    initToolsFlip）が側ごとに組み直すので、ここで DOM を動かすと互いに上書きし合う。
//  - #input-tools の中のボタンも外のボタンと自由に混ぜられるよう、並べ替えが有効な間は
//    #input-tools を display: contents にして子を #input-wrap の flex item に昇格させる
//    （styles/terminal.css の .custom-tool-order）。
//  - 保存するのは「ツールが右側にあるとき」の左→右の順。左側のときは鏡写しで適用する
//    （⇄ の既定の並びも概ね鏡写しなので、切り替えても入力欄に近いボタンが変わらない）。
//  - 保存が無いときは order を一切付けない＝従来の見た目のまま。
//  - HTML5 の drag&drop はタッチ端末では発火しない。並べ替えはデスクトップ専用で、
//    order を効かせる CSS もデスクトップのメディアクエリ内だけに置いてある。
//  - 並び順は localStorage にブラウザ単位で保存する（端末ごとに好みが違うため）。

import { onUiSideChange, toolsOnLeft } from './ui-side.js';

const STORAGE_KEY = 'inputToolsOrder';
const CUSTOM_CLASS = 'custom-tool-order';

// 並べ替えの対象。#input-tools の子も同じ階層として扱う。
// モバイル専用のボタン（＋ / ⌨）と ⇄ 自身、テキスト欄は対象外。
const ITEM_IDS = [
  'input-clear-btn',
  'send-btn',
  'voice-btn',
  'prompt-template-wrap',
  'voice-wakeword-btn',
  'quick-clear-btn',
  'quick-model-btn',
  'quick-cmd-btn-3',
  'quick-cmd-btn-4',
  'quick-cmd-btn-5',
  'slash-picker-btn',
  'buf-clear-btn',
];

function wrapEl(): HTMLElement | null {
  return document.getElementById('input-wrap');
}

function itemOf(target: EventTarget | null): HTMLElement | null {
  let el = target as HTMLElement | null;
  while (el && el.id !== 'input-wrap') {
    if (ITEM_IDS.includes(el.id)) return el;
    el = el.parentElement;
  }
  return null;
}

function loadOrder(): string[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((v): v is string => typeof v === 'string' && ITEM_IDS.includes(v));
  } catch (_) {
    return [];
  }
}

function saveOrder(ids: string[]): void {
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(ids)); } catch (_) { /* private mode 等は無視 */ }
}

/** 保存済みの順序に、保存に無い（将来追加された）ボタンを既定位置で補って返す。 */
function completeOrder(saved: string[]): string[] {
  const out = [...saved];
  ITEM_IDS.forEach((id, i) => {
    if (out.includes(id)) return;
    // 既定の並びで直前にあるボタンの後ろへ入れる
    const prev = ITEM_IDS.slice(0, i).reverse().find(p => out.includes(p));
    out.splice(prev ? out.indexOf(prev) + 1 : 0, 0, id);
  });
  return out;
}

/** 今画面に出ている左→右の並び（order 未設定なら DOM 順）を ID で返す。 */
function currentVisualOrder(wrap: HTMLElement): string[] {
  const saved = loadOrder();
  if (saved.length > 0) {
    const right = completeOrder(saved);
    return toolsOnLeft() ? [...right].reverse() : right;
  }
  const ids: string[] = [];
  for (const child of Array.from(wrap.children) as HTMLElement[]) {
    if (child.id === 'input-tools') {
      for (const inner of Array.from(child.children) as HTMLElement[]) {
        if (ITEM_IDS.includes(inner.id)) ids.push(inner.id);
      }
    } else if (ITEM_IDS.includes(child.id)) {
      ids.push(child.id);
    }
  }
  return completeOrder(ids);
}

function applyOrder(): void {
  const wrap = wrapEl();
  if (!wrap) return;
  const saved = loadOrder();
  const flip = document.getElementById('tools-flip-btn');
  if (saved.length === 0) {
    wrap.classList.remove(CUSTOM_CLASS);
    ITEM_IDS.forEach(id => document.getElementById(id)?.style.removeProperty('--tool-order'));
    flip?.style.removeProperty('--tool-order');
    return;
  }
  const right = completeOrder(saved);
  const visual = toolsOnLeft() ? [...right].reverse() : right;
  visual.forEach((id, i) => {
    document.getElementById(id)?.style.setProperty('--tool-order', String(i + 1));
  });
  // ⇄ は常にツール列の外側の端（右側なら右端・左側なら左端）
  flip?.style.setProperty('--tool-order', toolsOnLeft() ? '0' : String(visual.length + 1));
  wrap.classList.add(CUSTOM_CLASS);
}

function persistVisualOrder(visual: string[]): void {
  saveOrder(toolsOnLeft() ? [...visual].reverse() : visual);
  applyOrder();
}

/** 入力欄ツールの並びを既定（⇄ が組む DOM 順）へ戻す。 */
export function resetInputToolsOrder(): void {
  try { localStorage.removeItem(STORAGE_KEY); } catch (_) { /* noop */ }
  applyOrder();
}

function clearDropMarks(wrap: HTMLElement): void {
  wrap.querySelectorAll('.tool-drop-before, .tool-drop-after')
    .forEach(el => el.classList.remove('tool-drop-before', 'tool-drop-after'));
}

let dragSrc: HTMLElement | null = null;

export function initInputToolsOrder(): void {
  const wrap = wrapEl();
  if (!wrap) return;

  applyOrder();
  onUiSideChange(() => applyOrder());

  // ▤ はラッパー（パレットを含む）ではなくボタン側を掴ませる。
  // ラッパーごと draggable にすると、パレット内の検索欄で文字を選択できなくなる。
  ITEM_IDS.forEach(id => {
    const el = document.getElementById(id);
    if (!el) return;
    const handle = id === 'prompt-template-wrap' ? document.getElementById('prompt-template-toggle') : el;
    if (handle) handle.draggable = true;
  });

  wrap.addEventListener('dragstart', (e: DragEvent) => {
    const item = itemOf(e.target);
    if (!item) return;
    dragSrc = item;
    item.classList.add('tool-dragging');
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      // Firefox はデータを載せないと dragstart 自体が成立しない
      try { e.dataTransfer.setData('text/plain', item.id); } catch (_) { /* noop */ }
    }
  });

  wrap.addEventListener('dragend', () => {
    if (dragSrc) dragSrc.classList.remove('tool-dragging');
    dragSrc = null;
    clearDropMarks(wrap);
  });

  // ボタン上なら左右半分でその前後へ落とす。ボタン以外（テキスト欄・余白）には落とさない。
  // 並べ替え中は既定のドロップを常に止める。止めないとテキスト欄に落としたとき
  // dataTransfer の ID 文字列が入力欄へ挿入される。
  wrap.addEventListener('dragover', (e: DragEvent) => {
    if (!dragSrc) return;
    e.preventDefault();
    const item = itemOf(e.target);
    clearDropMarks(wrap);
    if (!item || item === dragSrc) {
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'none';
      return;
    }
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    const rect = item.getBoundingClientRect();
    const after = e.clientX >= rect.left + rect.width / 2;
    item.classList.add(after ? 'tool-drop-after' : 'tool-drop-before');
  });

  wrap.addEventListener('drop', (e: DragEvent) => {
    if (!dragSrc) return;
    e.preventDefault();
    const item = itemOf(e.target);
    clearDropMarks(wrap);
    if (!item || item === dragSrc) return;
    const rect = item.getBoundingClientRect();
    const after = e.clientX >= rect.left + rect.width / 2;
    const visual = currentVisualOrder(wrap).filter(id => id !== dragSrc!.id);
    const at = visual.indexOf(item.id);
    visual.splice(after ? at + 1 : at, 0, dragSrc.id);
    persistVisualOrder(visual);
  });

  const resetBtn = document.getElementById('input-tools-order-reset-btn');
  if (resetBtn) resetBtn.addEventListener('click', () => resetInputToolsOrder());
}
