// --- ESM imports (generated) ---
import { inputEl } from '../app.js';
import { render } from './session-list.js';
import { disableWebglRenderer, enableWebglRenderer, releaseHiddenWebglRenderers, termArea } from './terminal.js';

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

    // CSS カスタムプロパティを初期値にセット
    if (this.area) {
      this.area.style.setProperty('--pane-cols', this.cols);
      this.area.style.setProperty('--pane-rows', this.rows);
    }

    // GridPicker は MultiPaneManager 生成後に作る
    this.picker = new GridPicker(this);
    this.picker.syncBadge();
  }

  /** レイアウトを変更してDOM再構築 */
  setLayout(cols, rows) {
    this.cols = Math.max(1, Math.min(6, cols));
    this.rows = Math.max(1, Math.min(3, rows));
    if (this.area) {
      this.area.style.setProperty('--pane-cols', this.cols);
      this.area.style.setProperty('--pane-rows', this.rows);
    }
    this._applyFontScale();
    this.render();
    this._saveLayout();
  }

  /** セッション一覧を id 昇順で取得してスロットに割当て、DOM を再構築 */
  render() {
    if (!this.area) return;
    const allSorted = window.getSortedSessions ? window.getSortedSessions() : [];
    for (const id of Array.from(this.dismissPendingSessionIds)) {
      if (!allSorted.some(s => s && s.id === id)) {
        this.dismissPendingSessionIds.delete(id);
      }
    }
    const sorted = allSorted.filter(s => !this.dismissPendingSessionIds.has(s.id));
    const total  = this.cols * this.rows;

    // 既存スロットを detach してから DOM を再構築
    this.area.querySelectorAll('.pane-slot').forEach(el => this.detachSlot(el));

    // slots 配列を更新（セッション終了後も位置を動かさない）
    this.slots = [];
    for (let i = 0; i < total; i++) {
      this.slots.push(sorted[i] ? { session: sorted[i] } : null);
    }

    // DOM を再構築
    this.area.innerHTML = '';
    for (let i = 0; i < total; i++) {
      const slot = this.slots[i];
      const pane = slot ? this._buildPane(i, slot.session) : this._buildEmptyPane(i);
      this.area.appendChild(pane);
    }

    this._applyFontScale();

    // 各スロットに xterm をアタッチ（DOM 追加後）
    this.area.querySelectorAll('.pane-slot:not(.empty)').forEach((slotEl, i) => {
      const slot = this.slots[i];
      if (slot && slot.session) this.attachToSlot(slotEl, slot.session);
    });

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
                          : session.provider === 'ollama' ? 'O'
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

    // ✕ 閉じるボタン
    const closeBtn = document.createElement('button');
    closeBtn.className = 'ph-close';
    closeBtn.type = 'button';
    closeBtn.textContent = '✕';
    closeBtn.title = 'セッションを閉じる';
    closeBtn.addEventListener('mousedown', (e) => e.stopPropagation());
    closeBtn.addEventListener('mouseup', (e) => e.stopPropagation());
    closeBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      this.closeSlot(idx);
    });
    header.appendChild(closeBtn);

    return header;
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
      t.autoScroll = false;
      t.term.scrollToTop();
      this._syncViewportToBuffer(t, { top: true });
      return;
    }
    t.autoScroll = true;
    this._scrollToBottomAndSync(t);
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
      this._syncViewportToBuffer(t);
    });
  }

  _scrollToBottomAndSync(t) {
    if (!t || !t.term) return;
    t.term.scrollToBottom();
    this._syncViewportToBuffer(t, { bottom: true });
    requestAnimationFrame(() => this._syncViewportToBuffer(t, { bottom: true }));
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
      this._scrollToBottomAndSync(t);
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

  _syncViewportToBuffer(t, opts: any = {}) {
    if (!t || !t.term || !t.container || !t.term.buffer) return;
    const vp = t.container.querySelector('.xterm-viewport');
    const buf = t.term.buffer.active;
    if (!vp || !buf) return;

    const maxScrollTop = Math.max(0, vp.scrollHeight - vp.clientHeight);
    let targetScrollTop;
    if (opts.bottom) {
      targetScrollTop = maxScrollTop;
    } else if (opts.top) {
      targetScrollTop = 0;
    } else {
      const maxViewportY = Math.max(0, buf.length - t.term.rows);
      const viewportY = Math.max(0, Math.min(buf.viewportY || 0, maxViewportY));
      targetScrollTop = maxViewportY > 0
        ? Math.round((viewportY / maxViewportY) * maxScrollTop)
        : 0;
    }

    if (Number.isFinite(targetScrollTop)) {
      vp.scrollTop = targetScrollTop;
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
    label.textContent = `スロット ${idx + 1}`;
    el.appendChild(label);

    return el;
  }

  /** スロットのセッションを終了して空にする */
  closeSlot(idx) {
    const slot = this.slots[idx];
    if (!slot || !slot.session) return;
    // app.js の dismissSession を利用（存在する場合）
    const session = slot.session;
    this.dismissPendingSessionIds.add(session.id);
    if (typeof window.dismissSession === 'function') {
      window.dismissSession(session.id);
    }
    this.slots[idx] = null;
    if (this.focusedIdx === idx) {
      const next = this.slots.findIndex((s, i) => i !== idx && s !== null);
      this.focusSlot(next >= 0 ? next : 0);
    }
    this.render();
  }

  onSessionRemoved(sessionId) {
    this.dismissPendingSessionIds.delete(sessionId);
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
    const session = (this.slots[idx] && this.slots[idx].session) || null;
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
    }
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
        // DOM 移動で .xterm-viewport の scrollTop がブラウザにリセットされるため、
        // 次フレームでスクロール位置を xterm 内部状態に合わせて再同期する
        requestAnimationFrame(() => {
          // DOM 再配置でレイアウトが確定してから fit し、新ペインの行数(rows)を PTY へ反映する。
          // 636 行の同期呼び出しは container 移動直後＝レイアウト未確定のサイズで判定されるため
          // PTY rows が更新されず、Codex が旧高さ前提の絶対座標（ESC[35;1H 等）で描画して
          // 回答本文が画面外へ消える（スタンバイでも結果が出ない）不具合の対策。
          this._fitTerminalInSlot(termArea, t, session.id);
          if (forceBottom || t.autoScroll) {
            if (forceBottom) t.autoScroll = true;
            this._scrollToBottomAndSync(t);
          } else {
            // autoScroll=false（ユーザーが上にスクロール中）の場合は viewportY に合わせる
            this._syncViewportToBuffer(t);
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
          enableWebglRenderer(t);
          t.everAttached = true;
          if (typeof window.flushPendingTerminalChunks === 'function') {
            window.flushPendingTerminalChunks(session.id);
          }
          this._installResizeObserver(slotEl, termArea, t, session.id);
          this._fitTerminalInSlot(termArea, t, session.id);
          if (forceBottom) this._stickToBottomSoon(t, { force: true, passes: 4 });
          else if (t.autoScroll) this._scrollToBottomAndSync(t);
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
        this._scrollToBottomAndSync(t);
      } else {
        this._syncViewportToBuffer(t);
      }
      if (
        (t.term.cols !== prevCols || t.term.rows !== prevRows) &&
        typeof window.sendResize === 'function'
      ) {
        window.sendResize(sessionId, t.term.cols, t.term.rows);
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
    this.area.querySelectorAll('.pane-slot').forEach(el => this.detachSlot(el));
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
