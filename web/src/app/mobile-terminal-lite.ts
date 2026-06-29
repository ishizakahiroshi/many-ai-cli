// mobile-terminal-lite.ts
// スマホ幅で xterm.js のフル描画を隠し、scanBuffer(activeSessionId) の末尾 N 行を
// プレーン pre 表示する簡易ターミナルビュー（plan_mobile-ui-v0.4-scope.md A3）。
// xterm.js 自身は背後で生きているため、承認検出・PTY 受信・スクロールバック保持は
// すべてそのまま動く。本ビューは「見せ方の置き換え」だけで、データ経路には触れない。
//
// プラン記載:
//   - 既定は「直近 N 行のプレーンテキスト」表示
//   - 折り返しはしない（横スクロール許容）
//   - 承認時のみ詳細展開ボタンで xterm.js モーダルを開く
//
// MVP では「詳細を見る」モーダルを全行 pre で出すだけ（v0.4.1 で xterm 詳細表示に拡張余地）。

import { t } from '../i18n.js';
import { activeSessionId } from './state.js';
import { scanBuffer } from './terminal.js';

// 既定 N（プラン推奨値）。設定 UI は v0.4.1。
const DEFAULT_TAIL_LINES = 40;

// C3: スマホでは可視行数が稼げないため、CLI の装飾出力 (連続空行・水平線) を畳んで密度を上げる。
//   - 2 行以上連続する空行は 1 行に集約 (まず情報量を残す方向で 2 行→ 1 行から始める)
//   - 連続する水平線 (`_`, `-`, `─`, `=`, `*` のみで構成) は 1 本に集約
// xterm.js のスクロールバックは触らず、表示用バッファだけ整形する。
const _RULE_RE = /^[\s_\-─=*]+$/;
function _isRuleLine(s: string): boolean {
  const t2 = s.trim();
  return t2.length >= 3 && _RULE_RE.test(t2);
}
function collapseDecorations(lines: string[]): string[] {
  const out: string[] = [];
  let blankRun = 0;
  let prevRule = false;
  for (const raw of lines) {
    const isBlank = raw.trim() === '';
    if (isBlank) {
      blankRun++;
      if (blankRun === 1) out.push(raw); // 最初の 1 行は残す
      prevRule = false;
      continue;
    }
    blankRun = 0;
    if (_isRuleLine(raw)) {
      if (prevRule) continue; // 連続水平線は 1 本に集約
      prevRule = true;
      out.push(raw);
      continue;
    }
    prevRule = false;
    out.push(raw);
  }
  return out;
}

// スマホ幅判定（モジュール内のみ使用）。
const _mtlMql = (typeof window !== 'undefined' && typeof window.matchMedia === 'function')
  ? window.matchMedia('(max-width: 720px)') : null;
function isMobileViewport(): boolean { return !!_mtlMql?.matches; }

let _refreshTimer: number | null = null;

function getOrCreateLiteContainer(): HTMLElement | null {
  let el = document.getElementById('mobile-terminal-lite');
  if (el) return el;
  const wrapper = document.getElementById('terminal-wrapper');
  if (!wrapper) return null;

  el = document.createElement('div');
  el.id = 'mobile-terminal-lite';

  // C4: 「詳細を見る (全画面)」を中央配置の幅広ボタンから、ターミナル右上の小さなアイコンに変更。
  // ターミナル本文と入力欄の間にあった大きなボタン帯を撤去して視線の流れを途切れさせない。
  const detailBtn = document.createElement('button');
  detailBtn.id = 'mobile-terminal-lite-detail';
  detailBtn.type = 'button';
  detailBtn.className = 'mtl-detail-icon';
  detailBtn.textContent = '⛶';
  detailBtn.title = t('mobile_terminal_lite_show_detail');
  detailBtn.setAttribute('aria-label', t('mobile_terminal_lite_show_detail'));
  detailBtn.addEventListener('click', openDetailModal);
  el.appendChild(detailBtn);

  const pre = document.createElement('pre');
  pre.id = 'mobile-terminal-lite-pre';
  pre.className = 'mtl-pre';
  el.appendChild(pre);

  // terminal-area-wrapper の直後に挿入（同じ階層）。CSS で個別セッション + mobile 幅時のみ表示。
  wrapper.appendChild(el);
  return el;
}

// PTY バッファの末尾 N 行を pre に流し込む。1 秒ごとの定期更新と、
// activateSession 直後の明示呼び出しから入る。
export function refreshMobileTerminalLite(): void {
  if (!isMobileViewport()) return;
  if (activeSessionId === null) return;
  const el = getOrCreateLiteContainer();
  if (!el) return;
  const pre = el.querySelector<HTMLElement>('#mobile-terminal-lite-pre');
  if (!pre) return;
  // C3: 末尾余裕を見て多めに取り、collapse 後に末尾 N 行へ切る。
  //   scanBuffer(N) だと collapse で行数が減ったとき表示が痩せる。
  const raw = scanBuffer(activeSessionId, DEFAULT_TAIL_LINES * 2);
  while (raw.length > 0 && raw[raw.length - 1].trim() === '') raw.pop();
  const collapsed = collapseDecorations(raw);
  const lines = collapsed.length > DEFAULT_TAIL_LINES
    ? collapsed.slice(collapsed.length - DEFAULT_TAIL_LINES)
    : collapsed;
  pre.textContent = lines.join('\n');
  // 末尾追従。
  pre.scrollTop = pre.scrollHeight;
}

// 「詳細を見る」モーダル: scanBuffer 全行を pre で全画面表示する。
function openDetailModal(e?: Event): void {
  if (e) e.stopPropagation();
  if (activeSessionId === null) return;
  // 詳細モーダルは「全行を確認できる」が目的なので collapse は適用しない（生のログを保つ）。
  const allLines = scanBuffer(activeSessionId);
  while (allLines.length > 0 && allLines[allLines.length - 1].trim() === '') allLines.pop();

  const overlay = document.createElement('div');
  overlay.className = 'mtl-detail-overlay';

  const box = document.createElement('div');
  box.className = 'mtl-detail-box';

  const head = document.createElement('div');
  head.className = 'mtl-detail-head';

  const title = document.createElement('span');
  title.className = 'mtl-detail-title';
  title.textContent = t('mobile_terminal_lite_detail_title', { n: String(allLines.length) });
  head.appendChild(title);

  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.className = 'mtl-detail-close';
  closeBtn.textContent = '✕';
  closeBtn.setAttribute('aria-label', t('settings_close'));
  closeBtn.addEventListener('click', () => overlay.remove());
  head.appendChild(closeBtn);

  box.appendChild(head);

  const pre = document.createElement('pre');
  pre.className = 'mtl-detail-pre';
  pre.textContent = allLines.join('\n');
  box.appendChild(pre);

  overlay.appendChild(box);
  overlay.addEventListener('click', (ev) => {
    if (ev.target === overlay) overlay.remove();
  });
  document.addEventListener('keydown', function onKey(ev) {
    if (ev.key !== 'Escape') return;
    ev.preventDefault();
    ev.stopPropagation();
    overlay.remove();
    document.removeEventListener('keydown', onKey, true);
  }, true);

  document.body.appendChild(overlay);
  // 末尾追従。
  setTimeout(() => { pre.scrollTop = pre.scrollHeight; }, 0);
}

function startRefresh(): void {
  if (_refreshTimer !== null) return;
  _refreshTimer = window.setInterval(() => {
    if (!isMobileViewport()) return;
    if (activeSessionId === null) return;
    refreshMobileTerminalLite();
  }, 1000);
}
startRefresh();

// 幅変化（スマホ↔PC）で初回描画が抜けないよう、change でも refresh する。
if (_mtlMql) {
  _mtlMql.addEventListener('change', () => {
    if (isMobileViewport()) refreshMobileTerminalLite();
  });
}

// 他モジュール（activateSession 後の即時 refresh 用）から呼べるよう公開。
(window as any).refreshMobileTerminalLite = refreshMobileTerminalLite;
