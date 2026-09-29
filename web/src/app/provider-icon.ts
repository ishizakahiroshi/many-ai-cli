// AI（provider）のアイコンの単一ソース。図形・頭文字・色の受け口はこのファイルだけにある。
//
// 仕組み: ページに非表示の SVG スプライトを 1 つ置き、AI ごとに <symbol id="aac-prov-<id>"> を持たせる。
// providerIconHtml() は <use href="#aac-prov-<id>"> を返すだけなので、symbol を 1 つ書き換えれば、
// 描画済みのカード・チャット・ステータスバー等のアイコンも再描画なしで変わる（利用者が頭文字や色を
// 変えられるようにする plan_provider-icon-single-source.md C3 の土台）。
//
// 末端モジュールにする。session-list.ts など重いモジュールを import しない。session-list.ts は
// 循環 import が多く、読み込みの瞬間に相手の変数を同期で読むと TDZ で画面が「読み込み中...」のまま
// 固まる（web/src/app/boot-guard.ts の冒頭・scripts/check-web-module-init.mjs）。このファイルは
// 読み込みの瞬間に document を触らない（スプライトは初めて描画するときに作る）。
//
// 図形は 2 種類あり、使い分けは「単体で動くかどうか」で決まる。
//
//   丸（circle）  : それ単体で起動できるもの。`many-ai-cli wrap <provider>` の対象。
//   角丸（rect）  : 単体では動かず、Claude や Codex のセッションからラップして使うもの
//                   （モデル選択で選ぶ route。spawn-panel.ts の resolveRoute を参照）。
//
// **「ローカル実行かクラウドか」ではない。** Ollama はクラウドモデルも持つので、その軸で
// 読み替えると新しいアイコンを足すときに形を取り違える（2026-08-30 に実際に誤読があった）。
// 角丸なのは Ollama が単体で動かないからで、ローカルだからではない。
//
// 色は :root の CSS 変数 --prov-<id>（styles.css）が正本。図形の fill / stroke は symbol 内の
// インラインスタイルで var(--prov-<id>) を直接引く。<use> 配下の図形へ文書の CSS
// （.prov-shape.claude 等）が届くかはブラウザ実装に依存するため、届くことを前提にしない。
// クラス（prov-shape / prov-letter / <id>）は従来どおり残している。
//
// 利用者が設定で変えた頭文字と色（provider 定義の presentation.icon_text / presentation.color）は
// applyProviderPresentation() が受け取る。色は :root の --prov-<id> を上書きし、頭文字は symbol 内の
// <text> を書き換える。図形（丸 / 角丸）は変えさせない。値は Hub が検査したものだけが届くが、ここでも
// #RRGGBB と 1〜2 文字かを再検査し、合わなければ何も適用しない（CSS の注入を防ぐ）。
//
// 画像（plan_provider-icon-single-source.md C4）: 利用者が AI に画像を選んでいると、一覧の
// icon_image_version から /api/provider-icons/<id>?v=<版> を組み、symbol の中身を「図形で切り抜いた画像 +
// 縁取り」に差し替える。画像は new Image() で読み込めたことを確かめてから使う（CSP でインラインの
// onerror は書けない）。読み込めなければ、または画像を外すと、頭文字の表示に戻る。図形は変えない。

import { escapeHtml } from './util.js';

type IconShape = 'circle' | 'rect';

interface ProviderIconDef {
  shape: IconShape;
  letter: string;
}

// 同梱 9 provider の既定。キーは safeClassToken() 済みの id。
const PROVIDER_ICON_DEFS: ReadonlyMap<string, ProviderIconDef> = new Map<string, ProviderIconDef>([
  ['claude', { shape: 'circle', letter: 'C' }],
  ['codex', { shape: 'circle', letter: 'X' }],
  ['copilot', { shape: 'circle', letter: 'P' }],
  ['cursor-agent', { shape: 'circle', letter: 'r' }],
  ['ollama', { shape: 'rect', letter: 'O' }],
  ['lm-studio', { shape: 'rect', letter: 'L' }],
  ['opencode', { shape: 'circle', letter: 'O' }],
  ['grok', { shape: 'circle', letter: 'G' }],
  ['command-code', { shape: 'circle', letter: 'M' }],
]);

const FALLBACK_COLOR = '#6b7280';
const ICON_COLOR_PATTERN = /^#[0-9A-Fa-f]{6}$/;
const MAX_ICON_TEXT_GRAPHEMES = 2;
const SPRITE_ELEMENT_ID = 'aac-provider-icon-sprite';
const SYMBOL_ID_PREFIX = 'aac-prov-';
const CLIP_ID_PREFIX = 'aac-provclip-';

// クラスと href に入るので、英小文字・数字・- ・_ だけに絞る。
export function safeClassToken(value: unknown): string {
  const cleaned = String(value || '')
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64);
  return cleaned || 'unknown';
}

export function providerIconSymbolId(provider: unknown): string {
  return SYMBOL_ID_PREFIX + safeClassToken(provider);
}

// 利用者が設定した頭文字と色の検査。Hub 側（internal/provider/validation.go）と同じ規則。
export function isValidIconColor(value: unknown): value is string {
  return typeof value === 'string' && ICON_COLOR_PATTERN.test(value);
}

export function countGraphemes(text: string): number {
  const Segmenter = (Intl as any)?.Segmenter;
  if (typeof Segmenter === 'function') {
    let count = 0;
    for (const _ of new Segmenter(undefined, { granularity: 'grapheme' }).segment(text)) count++;
    return count;
  }
  return Array.from(text).length;
}

export function isValidIconText(value: unknown): value is string {
  if (typeof value !== 'string' || value === '' || value.length > 64) return false;
  if (value.trim() !== value) return false;
  if (/[\u0000-\u001f\u007f-\u009f]/.test(value)) return false;
  const count = countGraphemes(value);
  return count >= 1 && count <= MAX_ICON_TEXT_GRAPHEMES;
}

export type ProviderPresentation = { icon_text?: unknown; color?: unknown; image_href?: unknown };

// 画像の URL。id は Hub がファイル名にできる形（英小文字・数字・- ・_）だけ、版は Hub が付ける短い token だけ。
// それ以外は URL にしない（属性やパスへ入るので）。
const ICON_IMAGE_ID_PATTERN = /^[a-z0-9_-]{1,64}$/;
const ICON_IMAGE_VERSION_PATTERN = /^[A-Za-z0-9_-]{1,64}$/;
const ICON_IMAGE_URL_PATTERN = /^\/api\/provider-icons\/[a-z0-9_-]{1,64}\?v=[A-Za-z0-9_-]{1,64}$/;
const ICON_IMAGE_BLOB_PATTERN = /^blob:[^\s"'<>]+$/;

export function providerIconImageUrl(id: unknown, version: unknown): string {
  if (typeof id !== 'string' || typeof version !== 'string') return '';
  if (!ICON_IMAGE_ID_PATTERN.test(id) || !ICON_IMAGE_VERSION_PATTERN.test(version)) return '';
  return `/api/provider-icons/${id}?v=${version}`;
}

// symbol / プレビューの <image href> に入れてよいのは、Hub の画像 URL か、設定画面で選んだ画像の blob: URL だけ。
function isAllowedIconImageHref(value: unknown): value is string {
  return typeof value === 'string' && (ICON_IMAGE_URL_PATTERN.test(value) || ICON_IMAGE_BLOB_PATTERN.test(value));
}

// 利用者が設定した頭文字（token ごと）。色は :root の CSS 変数に直接書くのでここには持たない。
const userIconText = new Map<string, string>();

// 画像: 一覧が持つ画像の URL（wanted）と、そのうち読み込めたもの（loaded）。symbol に画像を出すのは loaded だけ。
const wantedImages = new Map<string, string>();
const loadedImages = new Map<string, string>();

type ImageProbe = (href: string) => Promise<boolean>;

function defaultImageProbe(href: string): Promise<boolean> {
  if (typeof Image !== 'function') return Promise.resolve(false);
  return new Promise<boolean>((resolve) => {
    const img = new Image();
    img.onload = () => resolve(true);
    img.onerror = () => resolve(false);
    img.src = href;
  });
}

let imageProbe: ImageProbe = defaultImageProbe;

// テスト用。null で既定（実ブラウザの Image）へ戻す。
export function setProviderIconImageProbe(probe: ImageProbe | null): void {
  imageProbe = probe ?? defaultImageProbe;
}

// token（safeClassToken 済み）ごとの symbol の中身。同梱以外（利用者が追加した AI）は、
// 引数の raw（id）の先頭 1 文字の丸を既定にする。
const symbolMarkups = new Map<string, string>();
const symbolRaws = new Map<string, unknown>();

function defaultLetter(token: string, raw: unknown): string {
  const def = PROVIDER_ICON_DEFS.get(token);
  if (def) return def.letter;
  const source = String(raw || '?').trim();
  return (Array.from(source)[0] || '?').toUpperCase();
}

interface SymbolInnerOptions {
  letter?: string;
  color?: string;
  // 画像を出すときの URL と、切り抜き用 clipPath の id（文書内で一意にする）。
  imageHref?: string;
  clipId?: string;
}

// symbol（と設定画面のプレビュー）の中身。letter / color は未指定なら利用者設定と CSS 変数に従う。
function buildSymbolInner(token: string, raw: unknown, options: SymbolInnerOptions = {}): string {
  const def = PROVIDER_ICON_DEFS.get(token);
  const color = options.color || `var(--prov-${token},${FALLBACK_COLOR})`;
  const shapeStyle = `fill:color-mix(in srgb,${color} 10%,white);stroke:${color}`;
  const letterStyle = `fill:${color}`;
  const cls = def ? ` ${token}` : '';
  if (isAllowedIconImageHref(options.imageHref) && options.clipId) {
    // 画像: 図形（丸 / 角丸）で切り抜き、縁取りの色は残す。下地を敷くので透過 PNG でも背景が抜けない。
    const rect = def?.shape === 'rect';
    const clipShape = rect
      ? '<rect x="1" y="1" width="14" height="14" rx="3"/>'
      : '<circle cx="8" cy="8" r="6"/>';
    const baseStyle = `fill:color-mix(in srgb,${color} 10%,white);stroke:none`;
    const ringStyle = `fill:none;stroke:${color}`;
    const base = rect
      ? `<rect x="1" y="1" width="14" height="14" rx="3" style="${baseStyle}"/>`
      : `<circle cx="8" cy="8" r="6" style="${baseStyle}"/>`;
    const ring = rect
      ? `<rect class="prov-shape${cls}" x="1" y="1" width="14" height="14" rx="3" stroke-width="2" style="${ringStyle}"/>`
      : `<circle class="prov-shape${cls}" cx="8" cy="8" r="6" stroke-width="2" style="${ringStyle}"/>`;
    return `<clipPath id="${options.clipId}">${clipShape}</clipPath>${base}<image href="${escapeHtml(options.imageHref)}" x="1" y="1" width="14" height="14" preserveAspectRatio="xMidYMid slice" clip-path="url(#${options.clipId})"/>${ring}`;
  }
  const shape = def?.shape === 'rect'
    ? `<rect class="prov-shape${cls}" x="1" y="1" width="14" height="14" rx="3" stroke-width="2" style="${shapeStyle}"/>`
    : `<circle class="prov-shape${cls}" cx="8" cy="8" r="6" stroke-width="2" style="${shapeStyle}"/>`;
  const letter = options.letter || userIconText.get(token) || defaultLetter(token, raw);
  const fontSize = countGraphemes(letter) > 1 ? '6' : '7.5';
  const text = `<text class="prov-letter${cls}" x="8" y="8" text-anchor="middle" dominant-baseline="central" font-size="${fontSize}" font-weight="bold" font-family="sans-serif" style="${letterStyle}">${escapeHtml(letter)}</text>`;
  return `${shape}${text}`;
}

// 共有 symbol の中身。画像が読み込めていれば画像、なければ頭文字。clipPath の id は symbol の id と別の接頭辞にする
// （token が "clip-x" の AI の symbol と、token が "x" の AI の clipPath が同じ id にならないように）。
function buildSharedSymbolInner(token: string, raw: unknown): string {
  return buildSymbolInner(token, raw, { imageHref: loadedImages.get(token), clipId: `${CLIP_ID_PREFIX}${token}` });
}

function buildSymbolMarkup(token: string, raw: unknown): string {
  return `<symbol id="${SYMBOL_ID_PREFIX}${token}" viewBox="0 0 16 16">${buildSharedSymbolInner(token, raw)}</symbol>`;
}

// 同梱 9 種はここで登録する（DOM には触らない）。
for (const token of PROVIDER_ICON_DEFS.keys()) {
  symbolRaws.set(token, token);
  symbolMarkups.set(token, buildSymbolMarkup(token, token));
}

let spriteEl: Element | null = null;

// スプライトを body に 1 つだけ作る。document / body が無いとき（読み込み中・テスト）は何もしない。
// display:none にはせず、幅高さ 0 の絶対配置で隠す。
export function ensureProviderIconSprite(): void {
  if (spriteEl && spriteEl.isConnected) return;
  if (typeof document === 'undefined' || !document.body) return;
  const existing = document.getElementById(SPRITE_ELEMENT_ID);
  if (existing) {
    spriteEl = existing as unknown as Element;
    return;
  }
  const symbols = Array.from(symbolMarkups.values()).join('');
  document.body.insertAdjacentHTML(
    'afterbegin',
    `<svg id="${SPRITE_ELEMENT_ID}" xmlns="http://www.w3.org/2000/svg" width="0" height="0" style="position:absolute;width:0;height:0;overflow:hidden" aria-hidden="true" focusable="false">${symbols}</svg>`,
  );
  spriteEl = document.getElementById(SPRITE_ELEMENT_ID) as unknown as Element | null;
}

// 未知の id は初めて描画するときに symbol を足す。スプライトがあれば DOM にも足す。
function registerSymbol(token: string, raw: unknown): void {
  if (symbolMarkups.has(token)) return;
  symbolRaws.set(token, raw);
  const markup = buildSymbolMarkup(token, raw);
  symbolMarkups.set(token, markup);
  if (spriteEl && spriteEl.isConnected) spriteEl.insertAdjacentHTML('beforeend', markup);
}

// 登録済みの symbol を、いまの利用者設定で作り直す。描画済みの <use> は symbol を参照しているので、
// 画面側の再描画なしで変わる。
function refreshSymbol(token: string): void {
  if (!symbolMarkups.has(token)) return;
  const raw = symbolRaws.get(token) ?? token;
  const markup = buildSymbolMarkup(token, raw);
  symbolMarkups.set(token, markup);
  if (typeof document === 'undefined') return;
  const existing = document.getElementById(SYMBOL_ID_PREFIX + token);
  if (existing) existing.innerHTML = buildSharedSymbolInner(token, raw);
  else if (spriteEl && spriteEl.isConnected) spriteEl.insertAdjacentHTML('beforeend', markup);
}

// 配布既定の色（利用者設定を上書きする前の :root の値）。設定画面の「既定のまま」の表示に使う。
const defaultColors = new Map<string, string>();
const appliedColors = new Set<string>();

function readCssColor(token: string): string {
  if (typeof document === 'undefined' || !document.documentElement || typeof getComputedStyle !== 'function') return FALLBACK_COLOR;
  const value = getComputedStyle(document.documentElement).getPropertyValue(`--prov-${token}`).trim();
  return isValidIconColor(value) ? value : FALLBACK_COLOR;
}

export function providerDefaultIconColor(provider: unknown): string {
  const token = safeClassToken(provider);
  return defaultColors.get(token) ?? readCssColor(token);
}

export function providerDefaultIconLetter(provider: unknown): string {
  return defaultLetter(safeClassToken(provider), provider);
}

// 一覧が持つ画像を揃える。外れた AI は画像を外して頭文字に戻す。新しい URL は読み込めたことを確かめてから
// symbol に反映する（読み込み中は前の表示のまま。読み込めなければ頭文字）。同じ URL を続けて渡されても何もしない。
function syncIconImages(next: Map<string, string>): void {
  for (const token of Array.from(wantedImages.keys())) {
    if (next.has(token)) continue;
    wantedImages.delete(token);
    if (loadedImages.delete(token)) refreshSymbol(token);
  }
  for (const [token, href] of next) {
    if (wantedImages.get(token) === href) continue;
    wantedImages.set(token, href);
    void imageProbe(href).then((ok) => {
      // 待っている間に別の画像・画像なしへ変わっていたら、この結果は捨てる。
      if (wantedImages.get(token) !== href) return;
      if (ok) {
        loadedImages.set(token, href);
        refreshSymbol(token);
      } else if (loadedImages.delete(token)) {
        refreshSymbol(token);
      }
    });
  }
}

// 読み込みを確かめ済みの画像の URL（無ければ ''）。設定画面が「いま付いている画像」を出すのに使う。
export function providerIconLoadedImageHref(provider: unknown): string {
  return loadedImages.get(safeClassToken(provider)) ?? '';
}

// Hub の一覧（/api/providers）から届いた presentation を全画面へ反映する。呼ばれるたびに
// 「一覧にある AI の利用者設定」だけが残るよう揃えるので、設定を外した AI や削除した AI は既定へ戻る。
// 検査に通らない値は無視して既定のままにする。
export function applyProviderPresentation(
  providers: Iterable<{ id?: unknown; presentation?: unknown; icon_image_version?: unknown }>,
): void {
  const nextText = new Map<string, string>();
  const nextColor = new Map<string, string>();
  const nextImage = new Map<string, string>();
  for (const provider of providers) {
    if (!provider || typeof provider.id !== 'string') continue;
    const token = safeClassToken(provider.id);
    const imageHref = providerIconImageUrl(provider.id, provider.icon_image_version);
    if (imageHref) nextImage.set(token, imageHref);
    const presentation = provider.presentation;
    if (!presentation || typeof presentation !== 'object') continue;
    const { icon_text: iconText, color } = presentation as ProviderPresentation;
    if (isValidIconText(iconText)) nextText.set(token, iconText);
    if (isValidIconColor(color)) nextColor.set(token, color);
  }
  syncIconImages(nextImage);

  for (const token of Array.from(userIconText.keys())) {
    if (nextText.has(token)) continue;
    userIconText.delete(token);
    refreshSymbol(token);
  }
  for (const [token, text] of nextText) {
    if (userIconText.get(token) === text) continue;
    userIconText.set(token, text);
    refreshSymbol(token);
  }

  const root = typeof document === 'undefined' ? null : document.documentElement;
  if (!root) return;
  for (const token of Array.from(appliedColors)) {
    if (nextColor.has(token)) continue;
    appliedColors.delete(token);
    defaultColors.delete(token);
    root.style.removeProperty(`--prov-${token}`);
  }
  for (const [token, color] of nextColor) {
    if (!appliedColors.has(token)) {
      defaultColors.set(token, readCssColor(token));
      appliedColors.add(token);
    }
    root.style.setProperty(`--prov-${token}`, color);
  }
}

// 設定画面のプレビュー。<use> を通さず、入力中の頭文字と色をそのまま描く（全画面のアイコンは変えない）。
// 未指定の項目は配布既定（頭文字は同梱の表、色は CSS 既定）で描く。値は検査に通ったものだけ使う。
export function providerIconPreviewHtml(provider: unknown, draft: ProviderPresentation, size: number = 32): string {
  const token = safeClassToken(provider);
  const parsedSize = Number(size);
  const safeSize = Number.isFinite(parsedSize) && parsedSize > 0 ? Math.min(Math.floor(parsedSize), 64) : 32;
  const letter = isValidIconText(draft?.icon_text) ? draft.icon_text : defaultLetter(token, provider);
  const color = isValidIconColor(draft?.color) ? draft.color : providerDefaultIconColor(provider);
  // 画像は「いま付いている画像」か「選んだばかりの画像（blob:）」の URL が来たときだけ使う。
  const draftHref = draft?.image_href;
  const imageHref = isAllowedIconImageHref(draftHref) ? draftHref : undefined;
  return `<svg class="card-provider-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="${safeSize}" height="${safeSize}" aria-hidden="true">${buildSymbolInner(token, provider, { letter, color, imageHref, clipId: `${CLIP_ID_PREFIX}preview` })}</svg>`;
}

export function providerIconHtml(provider: unknown, size: number = 16): string {
  const token = safeClassToken(provider);
  const parsedSize = Number(size);
  const safeSize = Number.isFinite(parsedSize) && parsedSize > 0 ? Math.min(Math.floor(parsedSize), 64) : 16;
  ensureProviderIconSprite();
  registerSymbol(token, provider);
  return `<svg class="card-provider-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="${safeSize}" height="${safeSize}" aria-hidden="true"><use href="#${SYMBOL_ID_PREFIX}${token}"/></svg>`;
}

// 画面に出す AI の表示名。Usage 欄・カード・起動パネルなど全画面がこの 1 つを使う。
// usage-panel.ts は session-list.ts を import できない（循環 import の TDZ）ので、末端のここに置く。
const PROVIDER_DISPLAY_NAMES: ReadonlyMap<string, string> = new Map<string, string>([
  ['claude', 'Claude'],
  ['codex', 'Codex'],
  ['copilot', 'Copilot'],
  ['cursor-agent', 'Cursor Agent'],
  ['ollama', 'Ollama'],
  ['lm-studio', 'LM Studio'],
  ['opencode', 'OpenCode'],
  ['grok', 'Grok Build'],
  ['command-code', 'Command Code'],
]);

export function providerDisplayName(provider: unknown): string {
  const key = String(provider || '').toLowerCase();
  return PROVIDER_DISPLAY_NAMES.get(key) || String(provider || '');
}

// 静的 HTML の器（<span data-provider-icon="<id>" [data-provider-icon-size="<px>"]>）へアイコンを入れる。
// 何度呼んでも同じ結果になる。index.html に図形を複製しないための口。
export function fillProviderIconSlots(root?: ParentNode | null): void {
  const scope = root ?? (typeof document === 'undefined' ? null : document);
  if (!scope) return;
  scope.querySelectorAll<HTMLElement>('[data-provider-icon]').forEach((el) => {
    const id = el.dataset.providerIcon;
    if (!id) return;
    const size = Number(el.dataset.providerIconSize);
    el.innerHTML = providerIconHtml(id, Number.isFinite(size) && size > 0 ? size : 16);
  });
}
