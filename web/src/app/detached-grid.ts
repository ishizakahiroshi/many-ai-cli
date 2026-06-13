// detached-grid.ts — C1: 別窓 Session Grid の基盤
// URL: /?view=detached-grid&layout=2x2&session_ids=1,2,3,4&token=...
//
// 別窓専用の表示マネージャ。session の生死・承認・終了は Hub 本体が管理する。
// このモジュールは表示（xterm attach / resize / focus）だけを担当する。

'use strict';

import {
  disableWebglRenderer,
  enableWebglRenderer,
  releaseHiddenWebglRenderers,
} from './terminal.js';
import { sessions } from './state.js';
import type { SessionSnapshot } from '../types/proto.js';

// ─── URL parse ────────────────────────────────────────────────────────────

export interface DetachedGridParams {
  sessionIds: number[];
  cols: number;
  rows: number;
}

/** URL query から detached-grid パラメータを取得する。 */
export function parseDetachedGridParams(): DetachedGridParams | null {
  const params = new URLSearchParams(window.location.search);
  if (params.get('view') !== 'detached-grid') return null;

  const idsRaw = params.get('session_ids') || '';
  const sessionIds = idsRaw
    .split(',')
    .map(s => parseInt(s.trim(), 10))
    .filter(n => Number.isFinite(n) && n > 0);

  const layoutRaw = params.get('layout') || '2x2';
  const match = /^(\d+)x(\d+)$/i.exec(layoutRaw);
  let cols = 2;
  let rows = 2;
  if (match) {
    cols = Math.max(1, Math.min(6, parseInt(match[1], 10)));
    rows = Math.max(1, Math.min(3, parseInt(match[2], 10)));
  }

  return { sessionIds, cols, rows };
}

// ─── DetachedGridManager ──────────────────────────────────────────────────

/**
 * 別窓 Session Grid のマネージャ。
 * MultiPaneManager の xterm attach / resize / focus ロジックを参考に、
 * session id list を固定して grid を描画する。
 */
export class DetachedGridManager {
  [key: string]: any;

  private _cols: number;
  private _rows: number;
  private _sessionIds: number[];
  private _area: HTMLElement | null;
  private _slots: Array<{ sessionId: number; slotEl: HTMLElement | null }>;
  private _focusedIdx: number;

  constructor(params: DetachedGridParams) {
    this._cols = params.cols;
    this._rows = params.rows;
    this._sessionIds = params.sessionIds;
    this._area = document.getElementById('detached-grid-area');
    this._slots = [];
    this._focusedIdx = 0;

    if (this._area) {
      this._area.style.setProperty('--pane-cols', String(this._cols));
      this._area.style.setProperty('--pane-rows', String(this._rows));
    }

    this._applyFontScale();
  }

  /** grid を描画する。session が未存在の場合は空スロットを表示。 */
  render(): void {
    if (!this._area) return;

    // 既存スロットの xterm を detach
    this._area.querySelectorAll('.pane-slot').forEach(el => this._detachSlot(el as HTMLElement));
    this._area.innerHTML = '';
    this._slots = [];

    const total = Math.min(this._cols * this._rows, Math.max(this._sessionIds.length, this._cols * this._rows));

    for (let i = 0; i < total; i++) {
      const sessionId = this._sessionIds[i] ?? null;
      const session = sessionId !== null ? (sessions.get(sessionId) ?? null) : null;

      let paneEl: HTMLElement;
      if (session) {
        paneEl = this._buildPane(i, session);
      } else if (sessionId !== null) {
        // session_id は指定されているが Hub にまだない（起動中 or 不明）
        paneEl = this._buildPendingPane(i, sessionId);
      } else {
        paneEl = this._buildEmptyPane(i);
      }
      this._area.appendChild(paneEl);
      this._slots.push({ sessionId: sessionId ?? -1, slotEl: paneEl });
    }

    this._applyFontScale();

    // xterm をアタッチ
    this._slots.forEach((slot, i) => {
      if (slot.sessionId < 0 || !slot.slotEl) return;
      const session = sessions.get(slot.sessionId);
      if (!session) return;
      this._attachToSlot(slot.slotEl, session);
    });
  }

  /** 指定 session のスロットバッジを更新する（ws-client から呼ばれる）。 */
  updateSlotBadge(sessionId: number, status: string): void {
    const idx = this._slots.findIndex(s => s.sessionId === sessionId);
    if (idx < 0 || !this._area) return;
    const slotEls = this._area.querySelectorAll('.pane-slot');
    const el = slotEls[idx];
    if (!el) return;
    const badge = el.querySelector('.ph-badge');
    if (!badge) return;
    const cssStatus = status === 'idle' ? 'standby' : status;
    badge.className = `ph-badge ${cssStatus}`;
    badge.textContent = status === 'waiting' ? '⚠' : status === 'running' ? '●' : '—';
    el.classList.toggle('waiting-approval', status === 'waiting');
  }

  /** Hub からセッション更新を受けたときに再描画する。 */
  onSessionsUpdated(): void {
    // 既存スロットを更新（ヘッダバッジのみ再描画してコンテナは維持）
    this._slots.forEach((slot, i) => {
      if (slot.sessionId < 0 || !slot.slotEl) return;
      const session = sessions.get(slot.sessionId);
      if (!session) return;
      // header を再構築
      const oldHeader = slot.slotEl.querySelector('.pane-header');
      if (oldHeader) {
        const newHeader = this._buildHeader(i, session);
        slot.slotEl.replaceChild(newHeader, oldHeader);
      }
      // pending ラベルを消してターミナルエリアを追加（初回 session 登録時）
      const pending = slot.slotEl.querySelector('.pane-pending-label');
      if (pending) {
        pending.remove();
        // ターミナルエリアを追加
        const termArea = document.createElement('div');
        termArea.className = 'pane-terminal-area';
        slot.slotEl.appendChild(termArea);
        this._addScrollButtons(termArea, session);
        this._attachToSlot(slot.slotEl, session);
      }
    });
  }

  /** フォーカスをスロット idx に移動する。 */
  focusSlot(idx: number): void {
    this._focusedIdx = idx;
    if (this._area) {
      this._area.querySelectorAll('.pane-slot').forEach((el, i) => {
        el.classList.toggle('focused', i === idx);
      });
    }
    const slot = this._slots[idx];
    if (!slot || slot.sessionId < 0) return;
    if (typeof window.activateSessionForMultiPane === 'function') {
      window.activateSessionForMultiPane(slot.sessionId);
    }
  }

  // ─── DOM ビルダ ─────────────────────────────────────────────────────────

  private _buildPane(idx: number, session: SessionSnapshot): HTMLElement {
    const el = document.createElement('div');
    el.className = 'pane-slot' + (idx === this._focusedIdx ? ' focused' : '');
    el.dataset.slotIdx = String(idx);

    el.appendChild(this._buildHeader(idx, session));

    const termArea = document.createElement('div');
    termArea.className = 'pane-terminal-area';
    el.appendChild(termArea);
    this._addScrollButtons(termArea, session);

    el.addEventListener('mousedown', () => this.focusSlot(idx));
    el.addEventListener('mouseup', () => {
      const slot = this._slots[idx];
      if (!slot || slot.sessionId < 0) return;
      setTimeout(() => {
        const t = window.getTerminalEntry ? window.getTerminalEntry(slot.sessionId) : null;
        if (t && t.term && t.term.hasSelection && t.term.hasSelection()) return;
        const inputEl = document.getElementById('detached-grid-input');
        if (inputEl && typeof (inputEl as HTMLInputElement).focus === 'function') (inputEl as HTMLInputElement).focus();
      }, 50);
    });

    return el;
  }

  private _buildHeader(idx: number, session: SessionSnapshot): HTMLElement {
    const header = document.createElement('div');
    header.className = 'pane-header';

    // プロバイダ丸バッジ
    const provBadge = document.createElement('span');
    provBadge.className = `sc-provider ${session.provider || ''}`;
    provBadge.textContent =
      session.provider === 'claude'        ? 'C'
    : session.provider === 'codex'         ? 'X'
    : session.provider === 'copilot'       ? 'P'
    : session.provider === 'cursor-agent'  ? 'r'
    : session.provider === 'ollama'        ? 'O'
    : (session.provider || '?')[0].toUpperCase();
    header.appendChild(provBadge);

    // セッション ID
    const sid = document.createElement('span');
    sid.className = 'ph-sid';
    sid.textContent = `#${String(session.id).padStart(3, '0')}`;
    header.appendChild(sid);

    // ラベル
    const dir = document.createElement('span');
    dir.className = 'ph-dir';
    dir.textContent = session.label || session.cwd || '';
    dir.title = session.cwd || '';
    header.appendChild(dir);

    // ステータスバッジ
    const badge = document.createElement('span');
    badge.className = `ph-badge ${session.state || 'standby'}`;
    badge.textContent =
      session.state === 'waiting' ? '⚠'
    : session.state === 'running' ? '●'
    : '—';
    header.appendChild(badge);

    return header;
  }

  private _buildPendingPane(idx: number, sessionId: number): HTMLElement {
    const el = document.createElement('div');
    el.className = 'pane-slot' + (idx === this._focusedIdx ? ' focused' : '');
    el.dataset.slotIdx = String(idx);

    // 仮ヘッダ
    const header = document.createElement('div');
    header.className = 'pane-header';
    const sid = document.createElement('span');
    sid.className = 'ph-sid';
    sid.textContent = `#${String(sessionId).padStart(3, '0')}`;
    header.appendChild(sid);
    const lbl = document.createElement('span');
    lbl.className = 'ph-dir';
    lbl.textContent = 'Connecting…';
    header.appendChild(lbl);
    el.appendChild(header);

    const pending = document.createElement('span');
    pending.className = 'pane-pending-label';
    pending.textContent = `Waiting for session #${sessionId}…`;
    el.appendChild(pending);

    return el;
  }

  private _buildEmptyPane(idx: number): HTMLElement {
    const el = document.createElement('div');
    el.className = 'pane-slot empty';
    el.dataset.slotIdx = String(idx);
    const plus = document.createElement('span');
    plus.textContent = '＋';
    el.appendChild(plus);
    return el;
  }

  private _addScrollButtons(termArea: HTMLElement, session: SessionSnapshot): void {
    const topBtn = this._buildScrollButton('top');
    const bottomBtn = this._buildScrollButton('bottom');
    topBtn.addEventListener('click', (e) => {
      e.preventDefault(); e.stopPropagation();
      this._scrollSessionTo(session.id, 'top');
    });
    bottomBtn.addEventListener('click', (e) => {
      e.preventDefault(); e.stopPropagation();
      this._scrollSessionTo(session.id, 'bottom');
    });
    termArea.appendChild(topBtn);
    termArea.appendChild(bottomBtn);
  }

  private _buildScrollButton(edge: 'top' | 'bottom'): HTMLButtonElement {
    const btn = document.createElement('button');
    btn.className = `terminal-scroll-btn pane-scroll-${edge}`;
    btn.type = 'button';
    btn.textContent = edge === 'top' ? '↑ up' : '↓ down';
    btn.addEventListener('mousedown', (e) => e.stopPropagation());
    btn.addEventListener('mouseup', (e) => e.stopPropagation());
    return btn;
  }

  private _scrollSessionTo(sessionId: number, edge: 'top' | 'bottom'): void {
    const t = window.getTerminalEntry ? window.getTerminalEntry(sessionId)
            : (window.terminals ? window.terminals.get(sessionId) : null);
    if (!t || !t.term) return;
    if (edge === 'top') {
      t.autoScroll = false;
      t.term.scrollToTop();
    } else {
      t.autoScroll = true;
      t.term.scrollToBottom();
    }
  }

  // ─── xterm attach / detach ──────────────────────────────────────────────

  private _attachToSlot(slotEl: HTMLElement, session: SessionSnapshot): void {
    const termArea = slotEl.querySelector('.pane-terminal-area') as HTMLElement | null;
    if (!termArea) return;

    const t = window.getTerminalEntry ? window.getTerminalEntry(session.id)
            : (window.terminals ? window.terminals.get(session.id) : null);
    if (!t || !t.term) return;

    const forceBottom = String(session.provider || '').toLowerCase() === 'codex';
    if (forceBottom) t.autoScroll = true;
    this._ensureScrollHandler(t, session.id, slotEl);

    if (t.container) {
      if (!termArea.contains(t.container)) {
        disableWebglRenderer(t);
        termArea.appendChild(t.container);
        requestAnimationFrame(() => {
          this._fitTerminalInSlot(termArea, t, session.id);
          if (forceBottom || t.autoScroll) {
            if (forceBottom) t.autoScroll = true;
            t.term.scrollToBottom();
          }
          enableWebglRenderer(t);
        });
      }
    } else {
      const container = document.createElement('div');
      container.style.width = '100%';
      container.style.height = '100%';
      t.container = container;
      termArea.appendChild(container);
      requestAnimationFrame(() => {
        if (!termArea.isConnected || !termArea.contains(container)) return;
        if (container.clientWidth > 0 && container.clientHeight > 0) {
          t.term.open(container);
          enableWebglRenderer(t);
          t.everAttached = true;
          if (typeof window.flushPendingTerminalChunks === 'function') {
            window.flushPendingTerminalChunks(session.id);
          }
          this._installResizeObserver(slotEl, termArea, t, session.id);
          this._fitTerminalInSlot(termArea, t, session.id);
          if (forceBottom) t.term.scrollToBottom();
          else if (t.autoScroll) t.term.scrollToBottom();
        }
      });
      return;
    }

    if (typeof window.flushPendingTerminalChunks === 'function') {
      window.flushPendingTerminalChunks(session.id);
    }
    this._installResizeObserver(slotEl, termArea, t, session.id);
    this._fitTerminalInSlot(termArea, t, session.id);
    if (forceBottom) t.term.scrollToBottom();
  }

  private _detachSlot(slotEl: HTMLElement): void {
    const ro: ResizeObserver | undefined = (slotEl as any)._resizeObserver;
    if (ro) { ro.disconnect(); delete (slotEl as any)._resizeObserver; }
    const termArea = slotEl.querySelector('.pane-terminal-area') as HTMLElement | null;
    if (termArea) {
      Array.from(termArea.childNodes).forEach(child => {
        try { termArea.removeChild(child); } catch (_) {}
      });
      releaseHiddenWebglRenderers();
    }
  }

  private _installResizeObserver(slotEl: HTMLElement, termArea: HTMLElement, t: any, sessionId: number): void {
    const existing: ResizeObserver | undefined = (slotEl as any)._resizeObserver;
    if (existing) existing.disconnect();
    const ro = new ResizeObserver(() => {
      this._fitTerminalInSlot(termArea, t, sessionId);
    });
    ro.observe(termArea);
    (slotEl as any)._resizeObserver = ro;
  }

  private _fitTerminalInSlot(termArea: HTMLElement, t: any, sessionId: number): void {
    if (t.fitAddon && t.container && termArea.offsetWidth > 0 && t.container.offsetWidth > 0) {
      const prevCols = t.term.cols;
      const prevRows = t.term.rows;
      t.fitAddon.fit();
      if (
        (t.term.cols !== prevCols || t.term.rows !== prevRows) &&
        typeof window.sendResize === 'function'
      ) {
        window.sendResize(sessionId, t.term.cols, t.term.rows);
      }
    }
  }

  private _ensureScrollHandler(t: any, sessionId: number, _slotEl: HTMLElement): void {
    if (!t || !t.term || t.scrollHandlerInstalled || typeof t.term.onScroll !== 'function') return;
    t.scrollHandlerInstalled = true;
    t.scrollDisposable = t.term.onScroll(() => {
      const buf = t.term.buffer && t.term.buffer.active;
      t.autoScroll = !buf || (buf.viewportY + t.term.rows >= buf.length);
    });
  }

  private _applyFontScale(): void {
    const n = this._cols * this._rows;
    document.body.classList.remove('pane-fs-normal', 'pane-fs-small', 'pane-fs-tiny');
    document.body.classList.add(
      n <= 4  ? 'pane-fs-normal' :
      n <= 9  ? 'pane-fs-small'  :
                'pane-fs-tiny'
    );
  }
}

// ─── Detached モード初期化 ──────────────────────────────────────────────────

/** detached-grid モードを判定し、通常 Hub UI を隠して grid UI を初期化する。 */
export function initDetachedGridMode(): DetachedGridManager | null {
  const params = parseDetachedGridParams();
  if (!params) return null;

  // 通常 Hub UI 要素を隠す
  const hideIds = [
    'session-list',
    'sidebar-resizer',
    'settings-panel',
    'input-bar-outer',
    'token-statusbar',
    'action-bar',
    'multi-question-banner',
    'mobile-menu-btn',
    'mobile-spawn-btn',
    'mobile-drawer-backdrop',
    'mobile-keyboard-panel',
    'about-panel',
    'model-picker-overlay',
  ];
  hideIds.forEach(id => {
    const el = document.getElementById(id);
    if (el) { el.hidden = true; el.style.display = 'none'; }
  });

  // unified-tab-bar はテキスト入力 "Hub で開く" ボタン代わりに使わず非表示にする
  const tabBar = document.getElementById('unified-tab-bar');
  if (tabBar) tabBar.hidden = true;

  // terminal-column を flex コンテナとして調整
  const termColumn = document.getElementById('terminal-column');
  if (termColumn) {
    termColumn.style.display = 'flex';
    termColumn.style.flexDirection = 'column';
    termColumn.style.height = '100vh';
    termColumn.style.overflow = 'hidden';
  }

  // display-area を非表示にして grid エリアを表示する
  const displayArea = document.getElementById('display-area');
  if (displayArea) { displayArea.hidden = true; displayArea.style.display = 'none'; }
  const multiView = document.getElementById('multi-view');
  if (multiView) { multiView.hidden = true; }

  // header に "Hub で開く" ボタンを追加
  _insertHubOpenButton();

  // detached-grid-area を生成・挿入
  const gridArea = _ensureDetachedGridArea();

  const mgr = new DetachedGridManager(params);
  window.detachedGridManager = mgr;

  // session が揃ってから描画（sessions Map は ws-client が入れてから更新されるため
  // ページロード直後は空の可能性がある。少し遅延して描画し、以降は更新フックで再描画）
  mgr.render();

  // 1秒後に再描画（ws 接続で session が追加されるケースの対応）
  setTimeout(() => mgr.render(), 1000);
  setTimeout(() => mgr.render(), 3000);

  return mgr;
}

function _ensureDetachedGridArea(): HTMLElement {
  let gridArea = document.getElementById('detached-grid-area');
  if (gridArea) return gridArea;

  gridArea = document.createElement('div');
  gridArea.id = 'detached-grid-area';

  // terminal-column の中、display-area の前に挿入
  const termColumn = document.getElementById('terminal-column');
  const displayArea = document.getElementById('display-area');
  if (termColumn && displayArea) {
    termColumn.insertBefore(gridArea, displayArea);
  } else if (termColumn) {
    termColumn.appendChild(gridArea);
  } else {
    document.body.appendChild(gridArea);
  }
  return gridArea;
}

function _insertHubOpenButton(): void {
  const header = document.querySelector('header');
  if (!header) return;
  const existing = document.getElementById('detached-hub-open-btn');
  if (existing) return;

  const btn = document.createElement('button');
  btn.id = 'detached-hub-open-btn';
  btn.className = 'detached-hub-open-btn';
  btn.textContent = '⊞ Hub';
  btn.title = 'Hub 本体を開く';
  btn.addEventListener('click', () => {
    // 現在の origin (token なし) で Hub 本体を開く
    const params = new URLSearchParams(window.location.search);
    const token = params.get('token');
    const hubUrl = token ? `/?token=${token}` : '/';
    window.open(hubUrl, '_blank');
  });

  // header の末尾に挿入（settings-btn の前）
  const settingsBtn = document.getElementById('settings-btn');
  if (settingsBtn) {
    header.insertBefore(btn, settingsBtn);
  } else {
    header.appendChild(btn);
  }
}
