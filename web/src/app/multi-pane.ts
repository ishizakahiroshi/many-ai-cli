// --- ESM imports (generated) ---
import { inputEl } from '../app.js';
import { canPageAltBuffer, disableWebglRenderer, enableWebglRenderer, releaseHiddenWebglRenderers, scrollAltBufferPage, termArea } from './terminal.js';
import { ensureAltScrollRail, requestEdge } from './alt-scroll-rail-view.js';
import { openProjectKey } from './state.js';
import { projectKeyForSession } from './sidebar-tree.js';
import { isValidTabName } from './project-view-memory.js';
import {
  FLEXIBLE_PANE_STORAGE_KEY,
  PANE_DRAG_MIME,
  addPaneContent,
  migrateSessionTabsToWorkspaces,
  normalizeFlexibleLayouts,
  paneContentKey,
  paneLayoutKey,
  sessionPaneLayoutKey,
  parsePaneDragPayload,
  previewPaneDrop,
  type FlexibleLayouts,
  type PaneContent,
} from './flexible-pane-state.js';
import {
  MULTI_SCOPE_ALL,
  MULTI_SCOPE_BOX,
  STORAGE_MULTI_SCOPE_KEY,
  computeRenderPlan,
  effectiveScope,
  normalizeScope,
  overflowCount,
  reorderForScope,
} from './multi-scope.js';

// multi-pane.js — MultiPaneManager + GridPicker (C3: xterm マルチインスタンス + WS ルーティング)
// index.html で app.js より前に読み込む

'use strict';

// ─── GridPicker ────────────────────────────────────────────────
export class GridPicker {
  [key: string]: any;

  constructor(manager) {
    this.manager = manager;
    this.popup   = document.getElementById('grid-picker-popup');
    this.grid    = document.getElementById('picker-grid');
    this.label   = document.getElementById('picker-hover-label');
    this._open   = false;

    this.build();
    this._bindOutsideClick();
  }

  /** 6×3 = 18 セルを生成し、プリセットボタンも追加 */
  build() {
    if (!this.grid) return;
    this.grid.innerHTML = '';
    for (let r = 1; r <= 3; r++) {
      for (let c = 1; c <= 6; c++) {
        const cell = document.createElement('div');
        cell.className = 'picker-cell';
        cell.dataset.c = String(c);
        cell.dataset.r = String(r);
        cell.addEventListener('mouseover', () => this.hover(c, r));
        cell.addEventListener('click', () => this.apply(c, r));
        this.grid.appendChild(cell);
      }
    }
    this.grid.addEventListener('mouseleave', () => {
      // ホバー解除 → 現在選択中のレイアウトをハイライト
      this._highlightCurrent();
    });

    // プリセットボタン: 2/4/6/9/12/18 ペイン相当
    const presets = this.popup && this.popup.querySelector('.picker-presets');
    if (presets) {
      presets.innerHTML = '';
      const configs = [
        { label: '2',  cols: 2, rows: 1 },
        { label: '4',  cols: 2, rows: 2 },
        { label: '6',  cols: 3, rows: 2 },
        { label: '9',  cols: 3, rows: 3 },
        { label: '12', cols: 4, rows: 3 },
        { label: '18', cols: 6, rows: 3 },
      ];
      configs.forEach(({ label, cols, rows }) => {
        const btn = document.createElement('button');
        btn.className = 'picker-preset-btn';
        btn.type = 'button';
        btn.textContent = label;
        btn.title = `${cols}×${rows}`;
        btn.addEventListener('click', () => this.apply(cols, rows));
        presets.appendChild(btn);
      });
    }

    // 初期状態でハイライト
    this._highlightCurrent();
  }

  /** セル (c, r) にホバーしたとき左上矩形をハイライト */
  hover(c, r) {
    if (!this.grid) return;
    this.grid.querySelectorAll('.picker-cell').forEach(el => {
      const hovered = +el.dataset.c <= c && +el.dataset.r <= r;
      el.classList.toggle('hovered', hovered);
      el.classList.remove('selected');
    });
    if (this.label) {
      this.label.textContent = `${c}×${r} — ${c * r} ペイン`;
    }
  }

  /** セル選択 → レイアウト適用 */
  apply(c, r) {
    this.manager.setLayout(c, r);
    this.syncBadge();
    this.hide();
  }

  /** マルチタブバッジを現在の cols×rows に更新 */
  syncBadge() {
    const badge = document.getElementById('multi-tab-layout-badge');
    if (badge) badge.textContent = `${this.manager.cols}×${this.manager.rows}`;
  }

  /** 現在のレイアウトをセルにハイライト（selected クラス） */
  _highlightCurrent() {
    if (!this.grid) return;
    const { cols, rows } = this.manager;
    this.grid.querySelectorAll('.picker-cell').forEach(el => {
      const selected = +el.dataset.c <= cols && +el.dataset.r <= rows;
      el.classList.toggle('selected', selected);
      el.classList.remove('hovered');
    });
    if (this.label) {
      this.label.textContent = `${cols}×${rows} — ${cols * rows} ペイン`;
    }
  }

  /** #tab-multi の位置に合わせてポップアップを表示 */
  show() {
    if (!this.popup) return;
    const tabEl = document.getElementById('tab-multi');
    if (tabEl) {
      const tabRect = tabEl.getBoundingClientRect();
      this.popup.style.left = tabRect.left + 'px';
      this.popup.style.top  = tabRect.bottom + 'px';
    }
    // display: flex で表示 (hidden 属性を外す)
    this.popup.hidden = false;
    this.popup.classList.add('open');
    this._open = true;
    this._highlightCurrent();
  }

  /** ポップアップを非表示 */
  hide() {
    if (!this.popup) return;
    this.popup.hidden = true;
    this.popup.classList.remove('open');
    this._open = false;
  }

  /** show/hide をトグル */
  toggle() {
    if (this._open) {
      this.hide();
    } else {
      this.show();
    }
  }

  /** タブバー外クリックで自動クローズ */
  _bindOutsideClick() {
    document.addEventListener('click', (e) => {
      if (!this._open) return;
      const tabEl = document.getElementById('tab-multi');
      if (
        (tabEl && tabEl.contains(e.target)) ||
        (this.popup && this.popup.contains(e.target))
      ) return;
      this.hide();
    });
  }
}

// ─── MultiPaneManager ──────────────────────────────────────────
export class MultiPaneManager {
  [key: string]: any;

  constructor() {
    // localStorage から復元（デフォルト 2×2）
    const saved = this._loadLayout();
    this.cols = saved.cols;
    this.rows = saved.rows;
    this.area = document.getElementById('multi-view');
    this.slots = [];        // { session } | null
    this.focusedIdx = 0;
    this.dismissPendingSessionIds = new Set();

    // B: ユーザーが D&D で並べ替えたセッション順（sessionId の配列）。
    //    render() のたびに live セッションで再構築し、新規は末尾へ追加する。
    //    **ここは常に全セッションの並び。** 範囲（this.scope）で絞った列は visibleIds が
    //    別に持ち、そちらは保存しない（multi-scope.ts 冒頭の不変条件）。
    this.order = this._loadOrder();
    // C3: 表示する範囲（'all' = 全部 / 'box' = いま開いている箱だけ）。端末ごとの設定。
    this.scope = this._loadScope();
    // C3: いま表示している ID の列。render() が作り直す。**保存しない。**
    this.visibleIds = [];
    // A: 列／行ごとのサイズ比率（fr 値の配列）。境界ドラッグで更新する。
    this.colFracs = this._loadFracs('Cols', this.cols);
    this.rowFracs = this._loadFracs('Rows', this.rows);
    this.customLayouts = this._loadCustomLayouts();
    this.activeLayoutKey = null;
    this.workspaceSession = null;
    this.dragPayload = null;
    this.dropPreview = null;
    this.mobileActiveSlot = 0;

    // CSS カスタムプロパティを初期値にセット
    if (this.area) {
      this.area.style.setProperty('--pane-cols', this.cols);
      this.area.style.setProperty('--pane-rows', this.rows);
    }

    // GridPicker は MultiPaneManager 生成後に作る
    this.picker = new GridPicker(this);
    this.picker.syncBadge();
    this._wireNormalDropTarget();
    document.addEventListener('dragstart', (event) => {
      this.dragPayload = parsePaneDragPayload(event.dataTransfer?.getData(PANE_DRAG_MIME) || '');
    });
    document.addEventListener('dragend', () => this._clearDropPreview());
    this.area?.addEventListener('dragover', event => {
      if (!this.area || this.area.hidden || this._dragFromIdx != null ||
          !Array.from(event.dataTransfer?.types || []).includes(PANE_DRAG_MIME)) return;
      event.preventDefault();
      this._showDropPreview(this.area);
    });
    this.area?.addEventListener('dragleave', event => {
      if (!this.area?.contains(event.relatedTarget as Node)) this._clearDropPreview();
    });
  }

  /** レイアウトを変更してDOM再構築 */
  setLayout(cols, rows) {
    this._selectLayoutForCurrentScope();
    this.cols = Math.max(1, Math.min(6, cols));
    this.rows = Math.max(1, Math.min(3, rows));
    if (this.area) {
      this.area.style.setProperty('--pane-cols', this.cols);
      this.area.style.setProperty('--pane-rows', this.rows);
    }
    // A: 列／行数が変わったらサイズ比率を等分にリセットする
    this.colFracs = this._equalFracs(this.cols);
    this.rowFracs = this._equalFracs(this.rows);
    const layout = this._currentCustomLayout();
    if (layout) {
      layout.cols = this.cols;
      layout.rows = this.rows;
      layout.colFracs = this.colFracs.slice();
      layout.rowFracs = this.rowFracs.slice();
      layout.slots = Array.from({ length: this.cols * this.rows }, (_, i) => layout.slots[i] || null);
      layout.autoSplit = false;
      this._saveCustomLayouts();
    }
    this._saveFracs();
    this._applyFontScale();
    this.render();
    this._saveLayout();
  }

  /** セッション一覧を state.js の orderSessions（サイドバーの木を深さ優先でたどった順）で取得してスロットに割当て、DOM を再構築 */
  render() {
    if (!this.area) return;
    this._clearDropPreview();
    const allSorted = window.getSortedSessions ? window.getSortedSessions() : [];
    this._selectLayoutForCurrentScope();
    for (const id of Array.from(this.dismissPendingSessionIds)) {
      if (!allSorted.some(s => s && s.id === id)) {
        this.dismissPendingSessionIds.delete(id);
      }
    }
    const sorted = allSorted.filter(s => !this.dismissPendingSessionIds.has(s.id));
    const total  = this.cols * this.rows;

    // 既存スロットを detach してから DOM を再構築
    this.area.querySelectorAll('.pane-slot').forEach(el => this.detachSlot(el));

    // B: order（ユーザー並べ替え順）を live セッションで再構築する。
    //    既存順のうち生存しているものを保持し、未登録の新規を sort 順で末尾追加。
    //
    // C3: **ここへ範囲（scope）の絞り込みを混ぜない。** 混ぜると他の箱のセッション ID が
    //     order から落ちたまま _saveOrder() で保存され、「全部」へ戻したときに利用者が
    //     手で並べ替えた順序が失われる（multi タブを 1 回開くだけで起きる）。
    //     保存する列（order）と表示する列（visibleIds）は computeRenderPlan が 1 か所で
    //     作り分ける。保存するのは前者だけ。
    const byId = new Map(sorted.filter(s => !this.workspaceSession ||
      (s.id === this.workspaceSession.sessionId && String(s.started_at || '') === this.workspaceSession.startedAt))
      .map(s => [s.id, s]));
    const plan = computeRenderPlan({
      prevOrder: this.order,
      liveSortedIds: sorted.map(s => s.id),
      scope: this.scope,
      openProjectKey,
      projectKeyOf: this._projectKeyResolver(allSorted),
      capacity: total,
    });
    this.order = plan.order;
    this._saveOrder();
    this.visibleIds = plan.visibleIds;   // 保存しない（表示のためだけの列）
    this.scopeOverflow = plan.overflow;

    // slots 配列を更新（visibleIds の先頭 total 件を表示）
    const custom = this._currentCustomLayout();
    this.slots = custom ? custom.slots.map(content => {
      if (!content) return null;
      if (content.kind === 'tab') {
        if (content.sessionId !== null) {
          const owner = byId.get(content.sessionId);
          if (!owner || String(owner.started_at || '') !== content.startedAt) return null;
        }
        return { tab: content };
      }
      const session = byId.get(content.sessionId);
      return session && String(session.started_at || '') === content.startedAt ? { session } : null;
    }) : plan.slotIds.map(id => {
      const session = (id != null) ? byId.get(id) : null;
      return session ? { session } : null;
    });

    // DOM を再構築
    this.area.querySelectorAll('.pane-slot').forEach(el => this._unmountTabSlot(el));
    this.area.innerHTML = '';
    for (let i = 0; i < total; i++) {
      const slot = this.slots[i];
      const pane = slot && slot.session ? this._buildPane(i, slot.session)
        : slot && slot.tab ? this._buildTabPane(i, slot.tab)
        : this._buildEmptyPane(i);
      this.area.appendChild(pane);
    }

    // A: グリッドのサイズ比率を適用し、境界スプリッタを生成する
    this._applyGridTemplate();
    this._buildSplitters();

    this._applyFontScale();

    // 各スロットに xterm をアタッチ（DOM 追加後）
    this.area.querySelectorAll('.pane-slot').forEach((slotEl, i) => {
      const slot = this.slots[i];
      if (slot && slot.session) this.attachToSlot(slotEl, slot.session);
      else if (slot && slot.tab) this._mountTabSlot(slotEl, slot.tab, i);
    });
    this._applyMobileActiveSlot();
    this._notifyLayoutChanged();

    // C5: サイドバーの P<n> バッジを更新（スロット割当が変わったため）
    // renderSessionList は app.js のスコープ変数なので window 経由でアクセス
    // _c5SidebarUpdating フラグで再帰呼び出しを防ぐ
    if (!window._c5SidebarUpdating && typeof window.renderSessionList === 'function') {
      window._c5SidebarUpdating = true;
      try { window.renderSessionList(); }
      finally { window._c5SidebarUpdating = false; }
    }
  }

  /** ペインスロット DOM を生成 */
  _buildPane(idx, session) {
    const el = document.createElement('div');
    el.className = 'pane-slot' + (idx === this.focusedIdx ? ' focused' : '');
    el.dataset.slotIdx = idx;

    const header = this._buildHeader(idx, session);
    el.appendChild(header);

    this._wireDropTarget(el, idx);

    const termArea = document.createElement('div');
    termArea.className = 'pane-terminal-area';
    // C3: xterm は attachToSlot() でアタッチする
    el.appendChild(termArea);
    this._addScrollButtons(termArea, session);

    // クリックでフォーカス
    // click は xterm.js が mousedown を消費した場合に発火しないことがあるため
    // mousedown を使って確実にスロットをアクティブ化する。
    el.addEventListener('mousedown', () => this.focusSlot(idx));

    // 選択操作後の mouseup で入力欄にフォーカスを戻す（シングルビューと同じパターン）。
    // xterm.js が click を止めるケースのフォールバック。
    el.addEventListener('mouseup', () => {
      const slot = this.slots[idx];
      const session = slot && slot.session;
      if (!session) return;
      // 50ms 待って xterm の選択状態が確定してから判定
      setTimeout(() => {
        const t = window.getTerminalEntry ? window.getTerminalEntry(session.id) : null;
        if (t && t.term && t.term.hasSelection && t.term.hasSelection()) return;
        const inputEl = document.getElementById('input');
        if (inputEl && typeof inputEl.focus === 'function') inputEl.focus();
      }, 50);
    });

    return el;
  }

  /** Non-terminal views have one DOM owner. C4 mounts that owner in this host. */
  _buildTabPane(idx, tab) {
    const el = document.createElement('div');
    el.className = 'pane-slot pane-tab-slot' + (idx === this.focusedIdx ? ' focused' : '');
    el.dataset.slotIdx = String(idx);
    (el as any)._paneDescriptor = tab;
    const header = document.createElement('div');
    header.className = 'pane-header';
    const title = document.createElement('span');
    title.className = 'ph-dir';
    title.textContent = tab.tabName;
    header.appendChild(title);
    this._appendPaneControls(header, idx);
    el.appendChild(header);
    const host = document.createElement('div');
    host.className = 'pane-view-area';
    el.appendChild(host);
    this._wireDropTarget(el, idx);
    el.addEventListener('mousedown', () => this.focusSlot(idx));
    return el;
  }

  _mountTabSlot(slotEl, tab, idx) {
    const host = slotEl.querySelector('.pane-view-area');
    if (!host) return;
    // The view owner must move the original root; cloning would duplicate IDs/state.
    const mount = (window as any).mountFlexiblePaneView;
    if (typeof mount === 'function') mount(tab, host, idx);
    else host.textContent = tab.tabName;
  }

  _unmountTabSlot(slotEl) {
    const idx = Number(slotEl.dataset.slotIdx);
    const tab = (slotEl as any)._paneDescriptor;
    const host = slotEl.querySelector('.pane-view-area');
    if (!tab || !host) return;
    const unmount = (window as any).unmountFlexiblePaneView;
    if (typeof unmount === 'function') unmount(tab, host, idx);
  }

  /** ペインヘッダ DOM（プロバイダ丸・#id・ラベル・バッジ・✕ボタン） */
  _buildHeader(idx, session) {
    const header = document.createElement('div');
    header.className = 'pane-header';

    // プロバイダ丸バッジ
    const provBadge = document.createElement('span');
    provBadge.className = `sc-provider ${session.provider || ''}`;
    provBadge.textContent = session.provider === 'claude' ? 'C'
                          : session.provider === 'codex'  ? 'X'
                          : session.provider === 'copilot' ? 'P'
                          : session.provider === 'cursor-agent' ? 'r'
                          : session.provider === 'grok' ? 'G'
                          : session.provider === 'ollama' ? 'O'
                          : session.provider === 'command-code' ? 'M'
                          : (session.provider || '?')[0].toUpperCase();
    header.appendChild(provBadge);

    // セッション ID
    const sid = document.createElement('span');
    sid.className = 'ph-sid';
    sid.textContent = `#${String(session.id).padStart(3, '0')}`;
    header.appendChild(sid);

    // ラベル（作業ディレクトリ or ラベル）
    const dir = document.createElement('span');
    dir.className = 'ph-dir';
    dir.textContent = session.label || session.cwd || '';
    dir.title = session.cwd || '';
    header.appendChild(dir);

    // ステータスバッジ
    const badge = document.createElement('span');
    badge.className = `ph-badge ${session.state || 'standby'}`;
    badge.textContent = session.state === 'waiting' ? '⚠'
                      : session.state === 'running' ? '●'
                      : '—';
    header.appendChild(badge);

    this._appendPaneControls(header, idx);
    return header;
  }

  _appendPaneControls(header, idx) {
    const moveBtn = document.createElement('button');
    moveBtn.className = 'ph-move';
    moveBtn.type = 'button';
    moveBtn.textContent = '⠿';
    moveBtn.title = this._t('pane_move', 'ペインを移動');
    moveBtn.setAttribute('aria-label', moveBtn.title);
    moveBtn.draggable = true;
    moveBtn.addEventListener('mousedown', (e) => e.stopPropagation());
    moveBtn.addEventListener('dragstart', (e) => {
      this._dragFromIdx = idx;
      if (e.dataTransfer) {
        e.dataTransfer.effectAllowed = 'move';
        e.dataTransfer.setData('text/plain', String(idx));
      }
      header.parentElement?.classList.add('dragging');
    });
    moveBtn.addEventListener('dragend', () => {
      this._dragFromIdx = null;
      this.area?.querySelectorAll('.pane-slot').forEach(s => s.classList.remove('dragging', 'drag-over'));
    });
    moveBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      window.dispatchEvent(new CustomEvent('pane-placement-request', {
        detail: { kind: 'move', fromIdx: idx, anchor: moveBtn },
      }));
    });
    header.appendChild(moveBtn);

    // ✕ removes only this display placement; the session continues.
    const closeBtn = document.createElement('button');
    closeBtn.className = 'ph-close';
    closeBtn.type = 'button';
    closeBtn.textContent = '✕';
    closeBtn.title = this._t('pane_remove', 'ペインの配置を外す');
    closeBtn.setAttribute('aria-label', closeBtn.title);
    closeBtn.addEventListener('mousedown', (e) => e.stopPropagation());
    closeBtn.addEventListener('mouseup', (e) => e.stopPropagation());
    closeBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      this.removeSlot(idx);
    });
    header.appendChild(closeBtn);

  }

  /** 通常ターミナルと同じスクロール補助ボタンをペイン内に生成 */
  _addScrollButtons(termArea, session) {
    const topBtn = this._buildScrollButton('top', 'scroll_to_top', '↑ up', 'scroll_to_top_tooltip', 'up');
    const bottomBtn = this._buildScrollButton('bottom', 'scroll_to_bottom', '↓ down', 'scroll_to_bottom_tooltip', 'down');

    topBtn.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      this._scrollSessionTo(session.id, 'top');
    });
    bottomBtn.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      this._scrollSessionTo(session.id, 'bottom');
    });

    termArea.appendChild(topBtn);
    termArea.appendChild(bottomBtn);
  }

  _buildScrollButton(edge, labelKey, fallbackLabel, tipKey, fallbackTip) {
    const btn = document.createElement('button');
    btn.className = `terminal-scroll-btn pane-scroll-${edge}`;
    btn.type = 'button';
    btn.dataset.i18n = labelKey;
    btn.dataset.i18nTooltip = tipKey;
    btn.textContent = this._t(labelKey, fallbackLabel);
    btn.dataset.tooltip = this._t(tipKey, fallbackTip);
    btn.addEventListener('mousedown', (e) => e.stopPropagation());
    btn.addEventListener('mouseup', (e) => e.stopPropagation());
    return btn;
  }

  _scrollSessionTo(sessionId, edge) {
    const t = window.getTerminalEntry ? window.getTerminalEntry(sessionId)
            : (window.terminals ? window.terminals.get(sessionId) : null);
    if (!t || !t.term) return;
    if (edge === 'top') {
      if (typeof window.markTerminalManualScrollIntent === 'function') {
        window.markTerminalManualScrollIntent();
      }
      if (canPageAltBuffer(sessionId, t)) {
        if (!requestEdge(sessionId, 'top')) {
          scrollAltBufferPage(sessionId, t, -1);
        }
        t.autoScroll = false;
        return;
      }
      t.autoScroll = false;
      t.term.scrollToTop();
      return;
    }
    if (canPageAltBuffer(sessionId, t)) {
      if (!requestEdge(sessionId, 'bottom')) {
        scrollAltBufferPage(sessionId, t, 1);
      }
      t.autoScroll = true;
      return;
    }
    t.autoScroll = true;
    t.term.scrollToBottom();
  }

  _terminalIsAtBottom(t) {
    if (!t || !t.term || !t.term.buffer) return true;
    const buf = t.term.buffer.active;
    return !buf || (buf.viewportY + t.term.rows >= buf.length);
  }

  _ensureScrollHandler(t, sessionId) {
    if (!t || !t.term || t.scrollHandlerInstalled || typeof t.term.onScroll !== 'function') return;
    t.scrollHandlerInstalled = true;
    t.scrollDisposable = t.term.onScroll(() => {
      const atBottom = this._terminalIsAtBottom(t);
      t.autoScroll = atBottom;
      if (
        sessionId === window.activeSessionId &&
        typeof window.updateScrollLockBtn === 'function'
      ) {
        window.updateScrollLockBtn(!atBottom);
      }
    });
  }

  _scrollToBottom(t) {
    if (!t || !t.term) return;
    t.term.scrollToBottom();
  }

  _shouldForceBottomOnAttach(session) {
    return String(session && session.provider || '').toLowerCase() === 'codex';
  }

  _stickToBottomSoon(t, opts: any = {}) {
    if (!t || !t.term) return;
    const force = !!opts.force;
    let remaining = Math.max(1, opts.passes || 4);
    if (force) t.autoScroll = true;
    const snap = () => {
      if (!t || !t.term) return;
      if (!force && !t.autoScroll) return;
      if (force) t.autoScroll = true;
      this._scrollToBottom(t);
    };
    const next = () => {
      if (remaining <= 0) return;
      remaining--;
      requestAnimationFrame(() => {
        snap();
        next();
      });
    };
    snap();
    next();
    for (const delay of [80, 220]) {
      setTimeout(snap, delay);
    }
  }

  _t(key, fallback) {
    const v = window.t ? window.t(key) : key;
    return (v === key && fallback != null) ? fallback : v;
  }

  /** 空スロット DOM */
  _buildEmptyPane(idx) {
    const el = document.createElement('div');
    el.className = 'pane-slot empty';
    el.dataset.slotIdx = idx;

    const plus = document.createElement('span');
    plus.textContent = '＋';
    el.appendChild(plus);

    const label = document.createElement('span');
    label.className = 'empty-num';
    label.textContent = `${this._t('pane_slot', 'スロット')} ${idx + 1}`;
    el.appendChild(label);

    // 空きだけの段（または列）に属するスロットには、その線ごと消すボタンを出す
    const line = this._emptyLineOf(idx);
    if (line) {
      const removeBtn = document.createElement('button');
      removeBtn.className = 'empty-remove';
      removeBtn.type = 'button';
      removeBtn.textContent = line.axis === 'row'
        ? `✕ ${this._t('pane_remove_empty_row', 'この段を消す')}`
        : `✕ ${this._t('pane_remove_empty_col', 'この列を消す')}`;
      removeBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        this.removeEmptyLine(line.axis, line.index);
      });
      el.appendChild(removeBtn);
    }

    // B: 空スロットもドロップ先にする（末尾への移動）
    this._wireDropTarget(el, idx);

    return el;
  }

  /** B: ペイン要素をドロップ先として配線する */
  _wireDropTarget(el, idx) {
    el.addEventListener('dragover', (e) => {
      const hasPayload = Array.from(e.dataTransfer?.types || []).includes(PANE_DRAG_MIME);
      if (!hasPayload && (this._dragFromIdx == null || this._dragFromIdx === idx)) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
      el.classList.add('drag-over');
    });
    el.addEventListener('dragleave', () => el.classList.remove('drag-over'));
    el.addEventListener('drop', (e) => {
      el.classList.remove('drag-over');
      const payload = parsePaneDragPayload(e.dataTransfer?.getData(PANE_DRAG_MIME) || '');
      if (payload) {
        e.preventDefault();
        window.dispatchEvent(new CustomEvent('pane-placement-commit', {
          detail: { ...payload, targetIdx: idx },
        }));
        this._dragFromIdx = null;
        return;
      }
      const from = this._dragFromIdx;
      this._dragFromIdx = null;
      if (from == null || from === idx) return;
      e.preventDefault();
      this.moveSlot(from, idx);
    });
  }

  _wireNormalDropTarget() {
    const area = document.getElementById('display-area');
    if (!area) return;
    area.addEventListener('dragover', (e) => {
      if (Array.from(e.dataTransfer?.types || []).includes(PANE_DRAG_MIME)) {
        e.preventDefault();
        if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
        area.classList.add('pane-drop-ready');
        this._showDropPreview(area);
      }
    });
    area.addEventListener('dragleave', (e) => {
      if (!area.contains(e.relatedTarget)) {
        area.classList.remove('pane-drop-ready');
        this._clearDropPreview();
      }
    });
    area.addEventListener('drop', (e) => {
      area.classList.remove('pane-drop-ready');
      this._clearDropPreview();
      const payload = parsePaneDragPayload(e.dataTransfer?.getData(PANE_DRAG_MIME) || '');
      if (!payload) return;
      e.preventDefault();
      window.dispatchEvent(new CustomEvent('pane-placement-request', {
        detail: { payload, targetIdx: null, source: 'display-area' },
      }));
    });
  }

  _layoutKey() {
    if (this.workspaceSession) return sessionPaneLayoutKey(this.workspaceSession.sessionId, this.workspaceSession.startedAt);
    return paneLayoutKey(effectiveScope(this.scope, openProjectKey), openProjectKey);
  }

  _clearDropPreview() {
    this.dropPreview?.remove();
    this.dropPreview = null;
    document.getElementById('display-area')?.classList.remove('pane-drop-ready');
    this.area?.querySelectorAll('.pane-slot.drag-over').forEach(slot => slot.classList.remove('drag-over'));
  }

  _previewLayout(payload, host) {
    const content = this._contentForPayload(payload);
    if (!content) return null;
    const localTab = payload.kind === 'tab' && payload.sessionId != null;
    let owner = null;
    let base;
    if (localTab) {
      owner = this._contentForSession(payload.sessionId);
      if (!owner) return null;
      const key = sessionPaneLayoutKey(owner.sessionId, owner.startedAt);
      base = this.customLayouts[key];
      if (!base) {
        const current = (window as any).currentFlexiblePaneView?.();
        const first = current?.kind === 'tab' && current.sessionId === owner.sessionId &&
          current.tabName !== payload.tabName ? this._contentForPayload(current) : null;
        base = { cols: 2, rows: 1, colFracs: [1, 1], rowFracs: [1],
          slots: [owner, first], autoSplit: true };
      }
    } else if (host.id === 'display-area' && (window as any).currentFlexiblePaneView?.()) {
      const current = (window as any).currentFlexiblePaneView();
      base = { cols: 2, rows: 1, colFracs: [1, 1], rowFracs: [1],
        slots: [this._contentForPayload(current), null] };
    } else if (this.area && !this.area.hidden && !this.workspaceSession) {
      base = this._currentCustomLayout() || {
        cols: this.cols, rows: this.rows, colFracs: this.colFracs, rowFracs: this.rowFracs,
        slots: this.slots.map(slot => slot?.session ? this._contentForSession(slot.session.id) : slot?.tab || null),
      };
    } else {
      const overviewKey = paneLayoutKey(effectiveScope(this.scope, openProjectKey), openProjectKey);
      base = this.customLayouts[overviewKey];
      if (!base) {
        const current = (window as any).currentFlexiblePaneView?.();
        const first = current ? this._contentForPayload(current) : null;
        base = { cols: 2, rows: 1, colFracs: [1, 1], rowFracs: [1], slots: [first, null] };
      }
    }
    const prediction = previewPaneDrop(base, content);
    if (!prediction) return null;
    return { ...prediction, owner };
  }

  _showDropPreview(host) {
    const payload = this.dragPayload;
    if (!payload || host.hidden) return;
    const prediction = this._previewLayout(payload, host);
    if (!prediction) return;
    if (this.dropPreview?.parentElement === host && this.dropPreview.dataset.payload === JSON.stringify(payload)) return;
    this._clearDropPreview();
    const { layout, suggestedIdx, existing, owner } = prediction;
    const preview = document.createElement('div');
    preview.className = 'pane-drop-preview';
    preview.dataset.payload = JSON.stringify(payload);
    preview.setAttribute('aria-hidden', 'true');
    preview.style.gridTemplateColumns = layout.colFracs.map(fr => `minmax(0, ${fr}fr)`).join(' ');
    preview.style.gridTemplateRows = layout.rowFracs.map(fr => `minmax(0, ${fr}fr)`).join(' ');
    const heading = document.createElement('div');
    heading.className = 'pane-drop-preview-heading';
    heading.textContent = owner ? `#${owner.sessionId} ${this._t('pane_workspace', '作業面')}`
      : this._t('pane_overview', 'マルチ');
    preview.appendChild(heading);
    layout.slots.forEach((slot, idx) => {
      const cell = document.createElement('div');
      cell.className = 'pane-drop-preview-cell';
      const candidate = existing ? idx === suggestedIdx : !slot;
      if (candidate) cell.classList.add('candidate');
      if (existing && idx === suggestedIdx) cell.classList.add('already');
      const number = document.createElement('strong');
      number.textContent = `${idx + 1}/${layout.slots.length}`;
      const label = document.createElement('span');
      label.textContent = slot ? (slot.kind === 'session' ? `#${slot.sessionId} Terminal`
        : `${slot.sessionId == null ? '' : `#${slot.sessionId} `}${slot.tabName}`)
        : `${owner ? `#${owner.sessionId} ` : ''}${payload.kind === 'session' ? 'Terminal' : payload.tabName}`;
      cell.append(number, label);
      if (candidate) {
        cell.addEventListener('dragover', event => {
          event.preventDefault();
          if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
          preview.querySelectorAll('.pane-drop-preview-cell.hover').forEach(el => el.classList.remove('hover'));
          cell.classList.add('hover');
        });
        cell.addEventListener('drop', event => {
          event.preventDefault();
          event.stopPropagation();
          const actual = parsePaneDragPayload(event.dataTransfer?.getData(PANE_DRAG_MIME) || '');
          this._clearDropPreview();
          if (!actual) return;
          window.dispatchEvent(new CustomEvent(host.id === 'display-area' ? 'pane-placement-request' : 'pane-placement-commit', {
            detail: host.id === 'display-area'
              ? { payload: actual, targetIdx: idx, source: 'display-area' }
              : { ...actual, targetIdx: idx },
          }));
        });
      } else {
        cell.addEventListener('dragover', event => event.preventDefault());
        cell.addEventListener('drop', event => {
          event.preventDefault();
          event.stopPropagation();
        });
      }
      preview.appendChild(cell);
    });
    host.appendChild(preview);
    this.dropPreview = preview;
  }

  setWorkspaceSession(sessionId = null) {
    const next = sessionId == null ? null : this._contentForSession(sessionId);
    const oldKey = this._layoutKey();
    this.workspaceSession = next;
    if (oldKey !== this._layoutKey()) this.activeLayoutKey = null;
    return next !== null || sessionId == null;
  }

  hasSessionWorkspace(sessionId) {
    const session = this._contentForSession(sessionId);
    return !!session && !!this.customLayouts[sessionPaneLayoutKey(session.sessionId, session.startedAt)];
  }

  clearSinglePaneWorkspace() {
    if (!this.workspaceSession) return;
    const key = this._layoutKey();
    delete this.customLayouts[key];
    this.activeLayoutKey = null;
    this._saveCustomLayouts();
  }

  _currentCustomLayout() {
    return this.customLayouts[this._layoutKey()] || null;
  }

  _selectLayoutForCurrentScope() {
    const key = this._layoutKey();
    if (key === this.activeLayoutKey) return;
    this.activeLayoutKey = key;
    const layout = this.customLayouts[key];
    if (layout) {
      this.cols = layout.cols;
      this.rows = layout.rows;
      this.colFracs = layout.colFracs.slice();
      this.rowFracs = layout.rowFracs.slice();
    } else {
      const legacy = this._loadLayout();
      this.cols = legacy.cols;
      this.rows = legacy.rows;
      this.colFracs = this._loadFracs('Cols', this.cols);
      this.rowFracs = this._loadFracs('Rows', this.rows);
    }
    this.area?.style.setProperty('--pane-cols', String(this.cols));
    this.area?.style.setProperty('--pane-rows', String(this.rows));
    this.picker?.syncBadge();
  }

  _loadCustomLayouts() {
    try {
      const layouts = normalizeFlexibleLayouts(JSON.parse(localStorage.getItem(FLEXIBLE_PANE_STORAGE_KEY) || '{}'));
      if (migrateSessionTabsToWorkspaces(layouts)) {
        try { localStorage.setItem(FLEXIBLE_PANE_STORAGE_KEY, JSON.stringify(layouts)); }
        catch (_) { /* Keep migrated placements in memory for this tab. */ }
      }
      return layouts;
    }
    catch (_) { return {}; }
  }

  _saveCustomLayouts() {
    try { localStorage.setItem(FLEXIBLE_PANE_STORAGE_KEY, JSON.stringify(this.customLayouts)); }
    catch (_) { /* Private browsing may disallow storage. Current placement still works. */ }
  }

  _contentForSession(sessionId): Extract<PaneContent, { kind: 'session' }> | null {
    const id = Number(sessionId);
    const session = (window.getSortedSessions ? window.getSortedSessions() : []).find(s => s.id === id);
    if (!session) return null;
    return { kind: 'session', sessionId: id, startedAt: String(session.started_at || '') };
  }

  _contentForTab(tabName, sessionId): PaneContent | null {
    if (!isValidTabName(tabName) || tabName === 'multi') return null;
    if (tabName === 'terminal') return this._contentForSession(sessionId);
    const globalTab = tabName === 'approval' || tabName === 'history' || tabName === 'orchestration';
    if (globalTab) return { kind: 'tab', tabName, sessionId: null, startedAt: null };
    const session = this._contentForSession(sessionId);
    if (!session) return null;
    return { kind: 'tab', tabName, sessionId: session.sessionId, startedAt: session.startedAt };
  }

  _contentForPayload(payload): PaneContent | null {
    if (!payload || typeof payload !== 'object') return null;
    if (payload.kind === 'session') return this._contentForSession(payload.sessionId);
    if (payload.kind === 'tab') return this._contentForTab(payload.tabName, payload.sessionId);
    return null;
  }

  _materializeCustomLayout(autoSplit = false) {
    const key = this._layoutKey();
    if (this.customLayouts[key]) return this.customLayouts[key];
    const slots = Array.from({ length: this.cols * this.rows }, (_, i) => {
      const slot = this.slots[i];
      return slot?.session ? this._contentForSession(slot.session.id) : slot?.tab || null;
    });
    const layout = { cols: this.cols, rows: this.rows, colFracs: this.colFracs.slice(),
      rowFracs: this.rowFracs.slice(), slots, autoSplit };
    this.customLayouts[key] = layout;
    return layout;
  }

  /** Seed both panes before setActiveTab('multi') renders the grid. */
  beginAutoSplit(existing, incoming) {
    const first = this._contentForPayload(existing);
    const second = this._contentForPayload(incoming);
    if (!first || !second || paneContentKey(first) === paneContentKey(second)) return false;
    this.cols = 2;
    this.rows = 1;
    this.colFracs = [1, 1];
    this.rowFracs = [1];
    this.customLayouts[this._layoutKey()] = {
      cols: 2, rows: 1, colFracs: [1, 1], rowFracs: [1], slots: [first, second], autoSplit: true,
    };
    this.mobileActiveSlot = 1;
    this._saveCustomLayouts();
    return true;
  }

  /** D&D adds another visible pane without replacing one already on screen. */
  _addContent(content, preferredIdx) {
    if (!content) return false;
    const layout = this._materializeCustomLayout();
    const existing = layout.slots.findIndex(slot => slot && paneContentKey(slot) === paneContentKey(content));
    if (existing >= 0) {
      this.focusSlot(existing);
      return true;
    }
    const target = addPaneContent(layout, content, preferredIdx);
    if (target === null) return false;
    this.cols = layout.cols;
    this.rows = layout.rows;
    this.colFracs = layout.colFracs.slice();
    this.rowFracs = layout.rowFracs.slice();
    this.area?.style.setProperty('--pane-cols', String(this.cols));
    this.area?.style.setProperty('--pane-rows', String(this.rows));
    this.picker?.syncBadge();
    this.focusedIdx = this.mobileActiveSlot = target;
    this._saveCustomLayouts();
    this.render();
    this.focusSlot(target);
    return true;
  }

  addTab(tabName, sessionId = null, preferredIdx = undefined) {
    return this._addContent(this._contentForTab(tabName, sessionId), preferredIdx);
  }

  addSession(sessionId, preferredIdx = undefined) {
    return this._addContent(this._contentForSession(sessionId), preferredIdx);
  }

  /** Move to an empty slot, or swap two occupied slots. */
  moveSlot(fromIdx, toIdx) {
    const count = this.cols * this.rows;
    if (!Number.isInteger(fromIdx) || !Number.isInteger(toIdx) ||
        fromIdx < 0 || toIdx < 0 || fromIdx >= count || toIdx >= count || fromIdx === toIdx ||
        !this.slots[fromIdx]) return false;
    const layout = this._materializeCustomLayout();
    [layout.slots[fromIdx], layout.slots[toIdx]] = [layout.slots[toIdx], layout.slots[fromIdx]];
    this.focusedIdx = toIdx;
    this.mobileActiveSlot = toIdx;
    this._saveCustomLayouts();
    this.render();
    return true;
  }

  /** Remove a placement only. Never call dismissSession from a pane close control. */
  removeSlot(idx) {
    if (!Number.isInteger(idx) || idx < 0 || idx >= this.cols * this.rows || !this.slots[idx]) return false;
    const layout = this._materializeCustomLayout();
    layout.slots[idx] = null;
    if (this.focusedIdx === idx) {
      const next = layout.slots.findIndex(slot => slot !== null);
      this.focusedIdx = next >= 0 ? next : 0;
    }
    if (this.mobileActiveSlot === idx) this.mobileActiveSlot = this.focusedIdx;
    this._saveCustomLayouts();
    this.render();
    this._maybeAutoCollapse(layout);
    return true;
  }

  /** idx のスロットが属する、空きだけの段（優先）または列。消せる線が無ければ null。 */
  _emptyLineOf(idx) {
    const { cols, rows } = this;
    const row = Math.floor(idx / cols);
    const col = idx % cols;
    if (rows > 1 && Array.from({ length: cols }, (_, c) => row * cols + c).every(i => !this.slots[i])) {
      return { axis: 'row', index: row };
    }
    if (cols > 1 && Array.from({ length: rows }, (_, r) => r * cols + col).every(i => !this.slots[i])) {
      return { axis: 'col', index: col };
    }
    return null;
  }

  /**
   * 空きだけの段（axis='row'）または列（axis='col'）を消してレイアウトを詰める。
   * 中身のあるスロットは消さない（空きでなければ false を返して何もしない）。
   * 段・列を減らす処理は setLayout に任せ、ここでは残るスロットの並び替えと
   * フォーカス位置の付け替えだけを行う。
   */
  removeEmptyLine(axis, index) {
    const oldCols = this.cols;
    const oldRows = this.rows;
    const count = oldCols * oldRows;
    const onLine = (i) => (axis === 'row' ? Math.floor(i / oldCols) : i % oldCols) === index;
    if ((axis !== 'row' && axis !== 'col') || !Number.isInteger(index) || index < 0 ||
        index >= (axis === 'row' ? oldRows : oldCols) ||
        (axis === 'row' ? oldRows : oldCols) < 2 ||
        Array.from({ length: count }, (_, i) => i).some(i => onLine(i) && this.slots[i])) return false;

    const keptIdx = Array.from({ length: count }, (_, i) => i).filter(i => !onLine(i));
    const layout = this._currentCustomLayout();
    if (layout) layout.slots = keptIdx.map(i => layout.slots[i] || null);

    let nextFocus = keptIdx.indexOf(this.focusedIdx);
    if (nextFocus < 0) nextFocus = Math.max(0, keptIdx.findIndex(i => this.slots[i]));
    const nextMobile = keptIdx.indexOf(this.mobileActiveSlot);
    this.focusedIdx = nextFocus;
    this.mobileActiveSlot = nextMobile < 0 ? nextFocus : nextMobile;

    this.setLayout(axis === 'col' ? oldCols - 1 : oldCols, axis === 'row' ? oldRows - 1 : oldRows);
    this.picker?.syncBadge();
    return true;
  }

  _maybeAutoCollapse(layout) {
    if (!layout?.autoSplit || !this.area || this.area.hidden ||
        layout !== this._currentCustomLayout()) return;
    const remaining = layout.slots.filter(Boolean);
    if (remaining.length === 1) {
      window.dispatchEvent(new CustomEvent('flexible-pane-auto-collapse', {
        detail: { remaining: remaining[0] },
      }));
    }
  }

  setMobileActiveSlot(idx) {
    if (!Number.isInteger(idx) || idx < 0 || idx >= this.cols * this.rows) return false;
    if (idx === this.mobileActiveSlot) return true;
    this.mobileActiveSlot = idx;
    this.focusSlot(idx);
    this._notifyLayoutChanged();
    const session = this.slots[idx]?.session;
    if (session) requestAnimationFrame(() => {
      const slotEl = this.area?.querySelectorAll('.pane-slot')[idx];
      const termArea = slotEl?.querySelector('.pane-terminal-area');
      const terminal = window.getTerminalEntry?.(session.id);
      if (termArea && terminal) this._fitTerminalInSlot(termArea, terminal, session.id);
    });
    return true;
  }

  getMobileActiveSlot() { return this.mobileActiveSlot; }

  _applyMobileActiveSlot() {
    const count = this.cols * this.rows;
    if (this.mobileActiveSlot >= count) this.mobileActiveSlot = 0;
    this.area?.querySelectorAll('.pane-slot').forEach((el, i) =>
      el.classList.toggle('mobile-pane-active', i === this.mobileActiveSlot));
  }

  _notifyLayoutChanged() {
    window.dispatchEvent(new CustomEvent('flexible-pane-layout-changed', {
      detail: { slots: this.slots, cols: this.cols, rows: this.rows, activeSlot: this.mobileActiveSlot },
    }));
  }

  /**
   * B: スロット from を slot to の位置へ移動する。
   * - to が埋まっている場合は両者を入れ替え（swap）
   * - to が空（表示範囲外）の場合は from を表示されている列の末尾へ移動
   *
   * C3: **スロット番号を this.order の添字として直接使わない。** 範囲が 'box' のときは
   *     間が飛ぶので、そのまま添字にすると掴んでいない別の箱のセッションが動く。
   *     変換は multi-scope.ts の orderIndexForSlot（reorderForScope が内部で通す）1 か所。
   */
  _reorderSlots(from, to) {
    if (this._currentCustomLayout()) {
      this.moveSlot(from, to);
      return;
    }
    const visible = Array.isArray(this.visibleIds) ? this.visibleIds : this.order;
    if (from < 0 || from >= visible.length) return;
    this.order = reorderForScope(this.order, visible, from, to);
    this._saveOrder();
    // フォーカスはドロップ先スロットへ移す
    this.focusedIdx = Math.min(to, this.cols * this.rows - 1);
    this.render();
  }

  /** Compatibility alias: the close control only removes the placement. */
  closeSlot(idx) {
    this.removeSlot(idx);
  }

  onSessionRemoved(sessionId) {
    this.dismissPendingSessionIds.delete(sessionId);
    this.onSessionEnded(sessionId);
  }

  onSessionEnded(sessionId) {
    let changed = false;
    for (const [key, layout] of Object.entries(this.customLayouts as FlexibleLayouts)) {
      if (key.startsWith(`session:${sessionId}:`)) {
        delete this.customLayouts[key];
        changed = true;
        continue;
      }
      for (let i = 0; i < layout.slots.length; i++) {
        if (layout.slots[i]?.sessionId === sessionId) {
          layout.slots[i] = null;
          changed = true;
        }
      }
    }
    if (changed) {
      this._saveCustomLayouts();
      if (this.area && !this.area.hidden) this.render();
      const layout = this._currentCustomLayout();
      if (layout) queueMicrotask(() => this._maybeAutoCollapse(layout));
    }
  }

  /** Call after a complete Hub snapshot, never during WebSocket reconnect gaps. */
  reconcileSessionsAfterSnapshot(snapshotSessions) {
    if (!Array.isArray(snapshotSessions)) return;
    // The caller's complete snapshot is authoritative. The UI sessions Map can
    // still contain entries from before a WebSocket reconnect at this point.
    const live = new Map(snapshotSessions.map(session => [session.id, session]));
    let changed = false;
    for (const [key, layout] of Object.entries(this.customLayouts as FlexibleLayouts)) {
      if (key.startsWith('session:')) {
        const owner = layout.slots.find(slot => slot?.kind === 'session');
        const liveOwner = owner && live.get(owner.sessionId);
        if (!owner || !liveOwner || String(liveOwner.started_at || '') !== owner.startedAt ||
            ['completed', 'error', 'disconnected'].includes(liveOwner.state)) {
          delete this.customLayouts[key];
          changed = true;
          continue;
        }
      }
      for (let i = 0; i < layout.slots.length; i++) {
        const content = layout.slots[i];
        if (!content || content.sessionId === null) continue;
        const session = live.get(content.sessionId);
        if (!session || String(session.started_at || '') !== content.startedAt ||
            ['completed', 'error', 'disconnected'].includes(session.state)) {
          layout.slots[i] = null;
          changed = true;
        }
      }
    }
    if (!changed) return;
    this._saveCustomLayouts();
    if (this.area && !this.area.hidden) this.render();
    this._maybeAutoCollapse(this._currentCustomLayout());
  }

  /** フォーカスをスロット idx に移動 */
  focusSlot(idx) {
    this.focusedIdx = idx;
    if (this.area) {
      this.area.querySelectorAll('.pane-slot').forEach((el, i) => {
        el.classList.toggle('focused', i === idx);
      });
    }
    // buf-clear-btn のターゲットをフォーカスセッションに更新
    const bufBtn = document.getElementById('buf-clear-btn');
    const slot = this.slots[idx];
    const session = (slot && slot.session) || null;
    const tabSessionId = slot?.tab?.sessionId;
    if (bufBtn) {
      bufBtn._targetSession = session;
    }
    // C4: マルチビューが表示中のとき activeSessionId をフォーカスペインのセッションに更新
    // → 既存の action-bar・input bar が自動的にこのセッションへ向く
    // activateSession 完全版はシングルビュー向けの処理（attachTerminal 等）を含むため
    // マルチタブ用の軽量版切替関数を使う
    if (session && this.area && !this.area.hidden) {
      if (typeof window.activateSessionForMultiPane === 'function') {
        window.activateSessionForMultiPane(session.id);
      }
    } else if (tabSessionId && this.area && !this.area.hidden &&
               typeof window.activateSessionForMultiPane === 'function') {
      window.activateSessionForMultiPane(tabSessionId);
    }
    this._applyMobileActiveSlot();
  }

  /**
   * C4: ペインヘッダのステータスバッジをリアルタイム更新
   * @param {number} sessionId - セッション ID
   * @param {'waiting'|'running'|'idle'} status - 新しいステータス
   */
  updateSlotBadge(sessionId, status) {
    const slotIdx = this.slots.findIndex(s => s && s.session && s.session.id === sessionId);
    if (slotIdx < 0) return;
    if (!this.area) return;
    const slotEls = this.area.querySelectorAll('.pane-slot');
    const el = slotEls[slotIdx];
    if (!el) return;
    const badge = el.querySelector('.ph-badge');
    if (!badge) return;
    // クラスと表示テキストを更新
    // 'idle' は CSS では 'standby' に対応するため変換する
    const cssStatus = status === 'idle' ? 'standby' : status;
    badge.className = `ph-badge ${cssStatus}`;
    badge.textContent = status === 'waiting' ? '⚠' : status === 'running' ? '●' : '—';
    // 承認待ちペインに薄黄アウトライン
    el.classList.toggle('waiting-approval', status === 'waiting');
  }

  /**
   * 子 plan: docs/local/plan_session-card-label-edit.md C2。
   * カード右クリックの改名は session_update（session_meta）で届くが、状態遷移が
   * 無いため updateSlotBadge の経路には乗らない。ペイン見出しの ph-dir だけを
   * その場で書き換え、フルの render() は起こさない（他ペインの再アタッチを避ける）。
   */
  updateSlotLabel(sessionId, text, cwd) {
    const slotIdx = this.slots.findIndex(s => s && s.session && s.session.id === sessionId);
    if (slotIdx < 0) return;
    if (!this.area) return;
    const slotEls = this.area.querySelectorAll('.pane-slot');
    const el = slotEls[slotIdx];
    if (!el) return;
    const dir = el.querySelector('.ph-dir');
    if (!dir) return;
    dir.textContent = text || '';
    dir.title = cwd || '';
  }

  // ─── C3: xterm アタッチ管理 ──────────────────────────────────

  /**
   * スロット要素にセッションの xterm をアタッチする。
   * - terminals Map は app.js スコープにあるため window.terminals 経由でアクセス
   * - t.container（xterm 親 div）を .pane-terminal-area に移動する
   * - ResizeObserver でペインサイズ変化を監視して fitAddon.fit() を呼ぶ
   */
  attachToSlot(slotEl, session) {
    const termArea = slotEl.querySelector('.pane-terminal-area');
    if (!termArea) return;

    // terminals Map は app.js スコープ変数。window 経由でアクセスできるよう公開が必要。
    // app.js で window.terminals を公開していない場合は getTerminalEntry を使う。
    const t = window.getTerminalEntry ? window.getTerminalEntry(session.id)
            : (window.terminals ? window.terminals.get(session.id) : null);
    if (!t || !t.term) return;
    const forceBottom = this._shouldForceBottomOnAttach(session);
    if (forceBottom) t.autoScroll = true;
    this._ensureScrollHandler(t, session.id);

    if (t.container) {
      // 既に open 済み: container を termArea に移動する
      if (!termArea.contains(t.container)) {
        // DOM 再配置で WebGL canvas の描画バッファが失われるため、移動前に破棄する
        disableWebglRenderer(t);
        termArea.appendChild(t.container);
        // 次フレームでレイアウト確定後に fit する（下のコメント参照）。xterm 5.x までは
        // ここで DOM 再配置による .xterm-viewport.scrollTop のリセットも re-sync していたが、
        // xterm 6.0 では .xterm-viewport 自体がスクロールしなくなり無効な処理だったため削除した
        // （pending_xterm6-viewport-scrolltop-deadcode.md。autoScroll=false 側の実際の表示位置は
        // buf.viewportY に基づく xterm 自身の描画がそのまま正しく、DOM 再配置の影響を受けない）。
        requestAnimationFrame(() => {
          // DOM 再配置でレイアウトが確定してから fit し、新ペインの行数(rows)を PTY へ反映する。
          // 636 行の同期呼び出しは container 移動直後＝レイアウト未確定のサイズで判定されるため
          // PTY rows が更新されず、Codex が旧高さ前提の絶対座標（ESC[35;1H 等）で描画して
          // 回答本文が画面外へ消える（スタンバイでも結果が出ない）不具合の対策。
          this._fitTerminalInSlot(termArea, t, session.id);
          if (forceBottom || t.autoScroll) {
            if (forceBottom) t.autoScroll = true;
            this._scrollToBottom(t);
          }
          // 配置・fit 確定後に WebGL レンダラを再生成する
          enableWebglRenderer(t);
        });
      }
    } else {
      // まだ open していない: termArea に直接 open
      const container = document.createElement('div');
      container.style.width = '100%';
      container.style.height = '100%';
      t.container = container;
      termArea.appendChild(container);
      // open() はコンテナがレイアウト済みでないと cols が狂うため rAF で遅延
      requestAnimationFrame(() => {
        if (!termArea.isConnected || !termArea.contains(container)) return;
        if (container.clientWidth > 0 && container.clientHeight > 0) {
          t.term.open(container);
          ensureAltScrollRail(session.id, t);
          enableWebglRenderer(t);
          t.everAttached = true;
          if (typeof window.flushPendingTerminalChunks === 'function') {
            window.flushPendingTerminalChunks(session.id);
          }
          this._installResizeObserver(slotEl, termArea, t, session.id);
          this._fitTerminalInSlot(termArea, t, session.id);
          if (forceBottom) this._stickToBottomSoon(t, { force: true, passes: 4 });
          else if (t.autoScroll) this._scrollToBottom(t);
        }
      });
      return;
    }

    if (typeof window.flushPendingTerminalChunks === 'function') {
      window.flushPendingTerminalChunks(session.id);
    }

    this._installResizeObserver(slotEl, termArea, t, session.id);
    this._fitTerminalInSlot(termArea, t, session.id);
    if (forceBottom) this._stickToBottomSoon(t, { force: true, passes: 4 });
  }

  // ResizeObserver でペインリサイズ時に自動フィット
  _installResizeObserver(slotEl, termArea, t, sessionId) {
    if (slotEl._resizeObserver) slotEl._resizeObserver.disconnect();
    const ro = new ResizeObserver(() => {
      this._fitTerminalInSlot(termArea, t, sessionId);
    });
    ro.observe(termArea);
    slotEl._resizeObserver = ro;
  }

  _fitTerminalInSlot(termArea, t, sessionId) {
    if (t.fitAddon && t.container && termArea.offsetWidth > 0 && t.container.offsetWidth > 0) {
      const prevCols = t.term.cols;
      const prevRows = t.term.rows;
      // fit() 前に「底にいたか」を記録し、fit() 後に底へ戻す。
      const buf = t.term.buffer && t.term.buffer.active;
      const wasAtBottom = !!t.autoScroll || !buf || (buf.viewportY + t.term.rows >= buf.length);
      t.fitAddon.fit();
      if (wasAtBottom) {
        t.autoScroll = true;
        this._scrollToBottom(t);
      }
      // wasAtBottom=false（履歴を読んでいる途中）側は xterm 自身の fit() 後の再描画が
      // buf.viewportY をそのまま反映するので、ここでの追加の同期は不要
      // （pending_xterm6-viewport-scrolltop-deadcode.md。xterm 6.0 で .xterm-viewport が
      // スクロールしなくなり、旧実装の再同期は既に無効化していた）。
      if (
        (t.term.cols !== prevCols || t.term.rows !== prevRows) &&
        typeof window.sendResize === 'function'
      ) {
        window.sendResize(sessionId, t.term.cols, t.term.rows, 'multi-pane-fit');
      }
    }
  }

  /**
   * スロット要素から xterm を切り離す（破棄しない）。
   * ResizeObserver を解除し、container を DOM から取り出す。
   */
  detachSlot(slotEl) {
    // ResizeObserver を解除
    if (slotEl._resizeObserver) {
      slotEl._resizeObserver.disconnect();
      delete slotEl._resizeObserver;
    }
    // xterm の container を termArea から取り出す（破棄しない）
    const termArea = slotEl.querySelector('.pane-terminal-area');
    if (termArea) {
      // container の子要素一覧を安全にコピーして取り出す
      const children = Array.from(termArea.childNodes);
      children.forEach(child => {
        try { termArea.removeChild(child); } catch (_) {}
      });
      // 非表示になったターミナルの WebGL コンテキストを解放する
      // （次の attach 時に enableWebglRenderer が再生成する）
      releaseHiddenWebglRenderers();
    }
  }

  /**
   * マルチタブ離脱時に全スロットを detach する。
   * app.js の setActiveTab（他タブへ切替時）から呼ぶ。
   */
  teardown() {
    if (!this.area) return;
    this._clearDropPreview();
    this.area.querySelectorAll('.pane-slot').forEach(el => {
      this.detachSlot(el);
      this._unmountTabSlot(el);
    });
  }

  // ─── A: グリッドのリサイズ（境界スプリッタ） ──────────────────

  /** fr 配列から grid-template-columns / rows を適用する */
  _applyGridTemplate() {
    if (!this.area) return;
    this.area.style.gridTemplateColumns = this.colFracs.map(f => `minmax(0, ${f.toFixed(4)}fr)`).join(' ');
    this.area.style.gridTemplateRows    = this.rowFracs.map(f => `minmax(0, ${f.toFixed(4)}fr)`).join(' ');
  }

  /** 内部境界ごとにドラッグ用スプリッタを生成し area に重ねる */
  _buildSplitters() {
    if (!this.area) return;
    // 既存スプリッタを除去（pane-slot は残す）
    this.area.querySelectorAll('.pane-splitter').forEach(el => el.remove());

    const sum = (arr, n) => arr.slice(0, n).reduce((a, b) => a + b, 0);
    const colTotal = this.colFracs.reduce((a, b) => a + b, 0) || 1;
    const rowTotal = this.rowFracs.reduce((a, b) => a + b, 0) || 1;

    // 列境界（縦バー）: k = 0..cols-2
    for (let k = 0; k < this.cols - 1; k++) {
      const sp = document.createElement('div');
      sp.className = 'pane-splitter col';
      sp.style.left = (sum(this.colFracs, k + 1) / colTotal * 100) + '%';
      this._wireSplitter(sp, 'col', k);
      this.area.appendChild(sp);
    }
    // 行境界（横バー）: k = 0..rows-2
    for (let k = 0; k < this.rows - 1; k++) {
      const sp = document.createElement('div');
      sp.className = 'pane-splitter row';
      sp.style.top = (sum(this.rowFracs, k + 1) / rowTotal * 100) + '%';
      this._wireSplitter(sp, 'row', k);
      this.area.appendChild(sp);
    }
  }

  /** スプリッタにポインタドラッグを配線する（境界 k と k+1 の比率を移動） */
  _wireSplitter(sp, axis, k) {
    const onDown = (e) => {
      e.preventDefault();
      e.stopPropagation();
      const rect = this.area.getBoundingClientRect();
      const isCol = axis === 'col';
      const fracs = isCol ? this.colFracs : this.rowFracs;
      const total = fracs.reduce((a, b) => a + b, 0) || 1;
      const containerPx = isCol ? rect.width : rect.height;
      const startPos = isCol ? e.clientX : e.clientY;
      const a0 = fracs[k];
      const b0 = fracs[k + 1];
      const minFrac = total * 0.08; // 1セルが極端に潰れないよう下限を設ける

      const onMove = (ev) => {
        const pos = isCol ? ev.clientX : ev.clientY;
        const deltaPx = pos - startPos;
        const deltaFrac = (deltaPx / Math.max(1, containerPx)) * total;
        let na = a0 + deltaFrac;
        let nb = b0 - deltaFrac;
        if (na < minFrac) { nb -= (minFrac - na); na = minFrac; }
        if (nb < minFrac) { na -= (minFrac - nb); nb = minFrac; }
        fracs[k] = na;
        fracs[k + 1] = nb;
        this._applyGridTemplate();
        this._repositionSplitters();
      };
      const onUp = () => {
        window.removeEventListener('pointermove', onMove);
        window.removeEventListener('pointerup', onUp);
        document.body.classList.remove('pane-resizing');
        this._saveFracs();
      };
      document.body.classList.add('pane-resizing');
      window.addEventListener('pointermove', onMove);
      window.addEventListener('pointerup', onUp);
    };
    sp.addEventListener('pointerdown', onDown);
  }

  /** ドラッグ中にスプリッタ位置だけ再計算する（DOM 再構築なし） */
  _repositionSplitters() {
    if (!this.area) return;
    const sum = (arr, n) => arr.slice(0, n).reduce((a, b) => a + b, 0);
    const colTotal = this.colFracs.reduce((a, b) => a + b, 0) || 1;
    const rowTotal = this.rowFracs.reduce((a, b) => a + b, 0) || 1;
    let ci = 0, ri = 0;
    this.area.querySelectorAll('.pane-splitter').forEach(sp => {
      if (sp.classList.contains('col')) {
        sp.style.left = (sum(this.colFracs, ci + 1) / colTotal * 100) + '%';
        ci++;
      } else {
        sp.style.top = (sum(this.rowFracs, ri + 1) / rowTotal * 100) + '%';
        ri++;
      }
    });
  }

  /** 指定長の等分 fr 配列を返す */
  _equalFracs(n) {
    return new Array(Math.max(1, n)).fill(1);
  }

  /** ペイン数に応じてフォントスケールクラスを付与 */
  _applyFontScale() {
    const n = this.cols * this.rows;
    document.body.classList.remove('pane-fs-normal', 'pane-fs-small', 'pane-fs-tiny');
    document.body.classList.add(
      n <= 4  ? 'pane-fs-normal' :
      n <= 9  ? 'pane-fs-small'  :
                'pane-fs-tiny'
    );
  }

  _saveLayout() {
    const custom = this._currentCustomLayout();
    if (custom) {
      custom.cols = this.cols;
      custom.rows = this.rows;
      this._saveCustomLayouts();
      return;
    }
    try {
      localStorage.setItem('multiPaneCols', this.cols);
      localStorage.setItem('multiPaneRows', this.rows);
    } catch (_) {}
  }

  _loadLayout() {
    try {
      const cols = parseInt(localStorage.getItem('multiPaneCols') || '2', 10);
      const rows = parseInt(localStorage.getItem('multiPaneRows') || '2', 10);
      return {
        cols: (isNaN(cols) || cols < 1 || cols > 6) ? 2 : cols,
        rows: (isNaN(rows) || rows < 1 || rows > 3) ? 2 : rows,
      };
    } catch (_) {
      return { cols: 2, rows: 2 };
    }
  }

  // ─── B: 並べ替え順の永続化 ──────────────────────────────────
  _saveOrder() {
    try { localStorage.setItem('multiPaneOrder', JSON.stringify(this.order)); } catch (_) {}
  }
  _loadOrder() {
    try {
      const raw = localStorage.getItem('multiPaneOrder');
      if (!raw) return [];
      const arr = JSON.parse(raw);
      return Array.isArray(arr) ? arr.filter(n => Number.isFinite(n)) : [];
    } catch (_) { return []; }
  }

  // ─── C3: 表示する範囲（この箱だけ / 全部）─────────────────────

  /**
   * セッション ID → 箱のキーを返す関数。範囲が 'box' のときだけ呼ばれるので、
   * ここで初めて表を作る（'all' では 1 度も作らない）。
   */
  _projectKeyResolver(allSorted) {
    let cache = null;
    return (id) => {
      if (!cache) {
        cache = new Map();
        for (const s of allSorted) {
          if (s) cache.set(s.id, projectKeyForSession(s, allSorted));
        }
      }
      const key = cache.get(id);
      return key === undefined ? null : key;
    };
  }

  /** 利用者が選んでいる範囲（保存値そのまま）。 */
  getScope() {
    return normalizeScope(this.scope);
  }

  /**
   * 範囲を切り替える。**行列数・比率・並び順のいずれも保存し直さない。**
   * 端末も作り直さない（render() が detachSlot してから組み直すので Terminal は生き続ける）。
   */
  setScope(next) {
    const value = normalizeScope(next);
    if (value === normalizeScope(this.scope)) return;
    this.scope = value;
    this._saveScope();
    this.render();
  }

  /**
   * 開いている箱が変わったときの追従。範囲が 'box' で multi を表示中のときだけ描き直す。
   * （multi を開いていなければ、次にタブを開いたときの render() で足りる）
   */
  onOpenProjectChanged() {
    if (normalizeScope(this.scope) !== MULTI_SCOPE_BOX) return;
    const view = document.getElementById('multi-view');
    if (!view || view.hidden) return;
    this.render();
  }

  /** 範囲トグル（session-strip.ts）が読む状態。直近の render() の結果をそのまま返す。 */
  getScopeStatus() {
    const capacity = this.cols * this.rows;
    const visibleCount = Array.isArray(this.visibleIds) ? this.visibleIds.length : 0;
    return {
      scope: normalizeScope(this.scope),
      effective: effectiveScope(this.scope, openProjectKey),
      projectKey: openProjectKey,
      visibleCount,
      capacity,
      overflow: Number.isFinite(this.scopeOverflow)
        ? this.scopeOverflow
        : overflowCount(visibleCount, capacity),
    };
  }

  _saveScope() {
    try { localStorage.setItem(STORAGE_MULTI_SCOPE_KEY, normalizeScope(this.scope)); } catch (_) {}
  }
  _loadScope() {
    try { return normalizeScope(localStorage.getItem(STORAGE_MULTI_SCOPE_KEY)); } catch (_) { return MULTI_SCOPE_ALL; }
  }

  // ─── A: サイズ比率の永続化 ──────────────────────────────────
  _saveFracs() {
    const custom = this._currentCustomLayout();
    if (custom) {
      custom.colFracs = this.colFracs.slice();
      custom.rowFracs = this.rowFracs.slice();
      this._saveCustomLayouts();
      return;
    }
    try {
      localStorage.setItem('multiPaneColFracs', JSON.stringify(this.colFracs));
      localStorage.setItem('multiPaneRowFracs', JSON.stringify(this.rowFracs));
    } catch (_) {}
  }
  /** 'Cols' | 'Rows' の比率を読み込み、長さが n と一致しなければ等分にフォールバック */
  _loadFracs(which, n) {
    try {
      const raw = localStorage.getItem('multiPane' + which + 'Fracs');
      if (raw) {
        const arr = JSON.parse(raw);
        if (Array.isArray(arr) && arr.length === n && arr.every(v => Number.isFinite(v) && v > 0)) {
          return arr;
        }
      }
    } catch (_) {}
    return this._equalFracs(n);
  }
}

// ─── グローバル公開 ────────────────────────────────────────────
// C9: getSortedSessions のフォールバック定義は撤去。整列ロジックは state.js の
// orderSessions に集約され、state.js は本ファイルより前にロードされるため
// window.getSortedSessions は常に解決される（state.js でエイリアス定義済み）。

// スクリプトは </body> 直前に配置されるため DOM は既に存在する。
// app.js より前に読み込まれるため、ここでインスタンスを生成して window に公開する。
// app.js の初期化コードはグローバルスコープで実行されるため、
// この時点で sessions 変数は未定義だが、getSortedSessions は呼び出し時に解決される。
(function () {
  window.multiPaneManager = new MultiPaneManager();
})();
