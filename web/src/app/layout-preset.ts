// 画面の配置を「型」で選ぶための状態を 1 か所に持つ。
//
// 型は 2 つ:
//   - bottom  下寄せ（従来どおりの配置。基準）
//   - top     上寄せ（入力まわり一式を端末の上に置く）
//
// 部品ごとの置き場所は下の表 1 本で持つ。画面はこの表から body のクラスを決める。
// 部品を足すときは表に行を足し、対応するクラスと CSS を足す。
//
// 設計メモ:
//  - DOM は動かさず #terminal-column の子の order を CSS で入れ替える（ui-side.ts と同じ方針。
//    端末の高さが変わらないので、型を切り替えても行数・桁数は変わらない）。
//  - 対象は PC（幅 721px 以上で、タッチ主体でない画面）だけ。スマホ幅とタッチ主体の画面は、
//    型に関係なく従来の並び（キーボードが下から出るため）。body のクラスは、この条件を
//    満たすときだけ付ける。クラスだけを見ている側（approval-dock.ts / action-bar-resize.ts /
//    各 CSS）は「クラスがある＝上寄せが実際に効いている」と読める。
//    条件は確認ポップアップをフロー配置へ戻す CSS（approval.css の
//    `(max-width: 720px), (pointer: coarse)`）のちょうど裏返しにしてある。ずらすと、タッチの
//    広い画面でフロー配置のバーに上ドックの top が当たり、バーが上へずれる。
//  - 保存先は localStorage（ブラウザ単位）。ui-side.ts と同じ扱い。保存が無い・不正な値は
//    下寄せ（＝更新しても誰の画面も変わらない）。
//  - 循環 import を避けるため、このファイルはほかの app モジュールを import しない。
//    型の変更を知りたい側は onLayoutPresetChange に登録する。

export type LayoutPreset = 'bottom' | 'top';

/** 部品ごとの置き場所の表の 1 行。 */
export interface LayoutPlacementRow {
  /** 部品の名前（表の中での識別子）。 */
  part: string;
  /** 型ごとの置き場所。 */
  place: Record<LayoutPreset, string>;
  /** 置き場所が下寄せ（基準）と違うとき body に付けるクラス。 */
  activeClass: string;
}

/** 部品ごとの置き場所の表。今回は入力まわり一式（#input-bar-outer）の 1 行だけ。 */
export const LAYOUT_PLACEMENTS: readonly LayoutPlacementRow[] = [
  { part: 'input-area', place: { bottom: 'below-terminal', top: 'above-terminal' }, activeClass: 'layout-input-top' },
];

const STORAGE_KEY = 'uiLayoutPreset';

/** 型が画面に効く条件。approval.css のモバイル用 `(max-width: 720px), (pointer: coarse)` の裏返し。 */
export const LAYOUT_PRESET_MEDIA = '(min-width: 721px) and (not (pointer: coarse))';

/** 保存された生の値を型へ引き当てる。無い・知らない値は下寄せ。 */
export function readLayoutPreset(raw: string | null): LayoutPreset {
  return raw === 'top' ? 'top' : 'bottom';
}

/** 画面に実際に効かせる型。選んだ型が上寄せでも、条件を満たさない画面では下寄せ。 */
export function effectiveLayoutPreset(selected: LayoutPreset, mediaMatches: boolean): LayoutPreset {
  return selected === 'top' && mediaMatches ? 'top' : 'bottom';
}

/** 型から部品の置き場所を引く。表に無い部品は undefined。 */
export function resolveLayoutPlace(preset: LayoutPreset, part: string): string | undefined {
  return LAYOUT_PLACEMENTS.find((row) => row.part === part)?.place[preset];
}

/** 表から body に付けるクラスの一覧を決める。 */
export function layoutBodyClasses(preset: LayoutPreset): string[] {
  return LAYOUT_PLACEMENTS.filter((row) => row.place[preset] !== row.place.bottom)
    .map((row) => row.activeClass);
}

/** 利用者が選んだ型（設定の選択欄に出す値）。画面に効いているかは別（下の effective）。 */
export function getLayoutPreset(): LayoutPreset {
  try {
    return readLayoutPreset(localStorage.getItem(STORAGE_KEY));
  } catch (_) {
    return 'bottom';
  }
}

/** いまの画面が、型を効かせる条件を満たしているか。判定できない環境では効かせない。 */
function mediaMatches(): boolean {
  try {
    return typeof window.matchMedia === 'function' && window.matchMedia(LAYOUT_PRESET_MEDIA).matches;
  } catch (_) {
    return false;
  }
}

function currentEffective(): LayoutPreset {
  return effectiveLayoutPreset(getLayoutPreset(), mediaMatches());
}

function applyBodyClasses(preset: LayoutPreset): void {
  const active = new Set(layoutBodyClasses(preset));
  LAYOUT_PLACEMENTS.forEach((row) => {
    document.body.classList.toggle(row.activeClass, active.has(row.activeClass));
  });
}

/** 渡されるのは「画面に実際に効いている型」。 */
type Listener = (preset: LayoutPreset) => void;
const listeners: Listener[] = [];

/** 効いている型が変わったときに呼ばれる（ボタンの置き直しなど、クラスだけでは追えない処理用）。 */
export function onLayoutPresetChange(cb: Listener): void {
  listeners.push(cb);
}

/** 効いている型を body へ反映し、変わっていれば登録先へ知らせる。 */
let lastApplied: LayoutPreset | null = null;
function applyEffective(): void {
  const effective = currentEffective();
  applyBodyClasses(effective);
  if (effective === lastApplied) return;
  const first = lastApplied === null;
  lastApplied = effective;
  // 起動時の最初の 1 回は知らせない（登録先はまだ初期化の途中。各自が自分の初期化で読む）。
  if (first) return;
  listeners.forEach((cb) => { try { cb(effective); } catch (_) { /* 1 つの失敗で他を止めない */ } });
}

export function setLayoutPreset(preset: LayoutPreset): void {
  try {
    // 下寄せは基準なので保存しない（ui-side の '' と同じ扱い）。
    if (preset === 'top') localStorage.setItem(STORAGE_KEY, 'top');
    else localStorage.removeItem(STORAGE_KEY);
  } catch (_) { /* private mode 等は無視 */ }
  syncSelect(getLayoutPreset());
  applyEffective();
}

function syncSelect(preset: LayoutPreset): void {
  const sel = document.getElementById('layout-preset-select') as HTMLSelectElement | null;
  if (sel && sel.value !== preset) sel.value = preset;
}

export function initLayoutPreset(): void {
  applyEffective();
  // 窓の幅を変えた・タッチとマウスが切り替わったときに、効かせるかどうかをやり直す。
  try {
    if (typeof window.matchMedia === 'function') {
      window.matchMedia(LAYOUT_PRESET_MEDIA).addEventListener('change', () => applyEffective());
    }
  } catch (_) { /* 監視できない環境では起動時の判定のまま */ }
  const sel = document.getElementById('layout-preset-select') as HTMLSelectElement | null;
  if (!sel) return;
  sel.value = getLayoutPreset();
  sel.addEventListener('change', () => {
    setLayoutPreset(sel.value === 'top' ? 'top' : 'bottom');
  });
}
