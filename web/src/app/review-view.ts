// --- ESM imports (generated) ---
import { token } from './util.js';
import { activeSessionId, sessions } from './state.js';
import { resolveCurrentReviewLoad } from './review-load-generation.js';

// ---- Review view ----
// ============================================================
// ReviewView — Review タブ contentEl 配下に、作業ツリーまたは AI ターン単位の
// per-file unified diff を描画する（Phase 1 + Phase 2）。
//
// 親 plan : docs/local/plan_turn-diff-viewer.md
// mock    : docs/local/mockups/turn-diff-viewer/index.html
//
// API:
//   GET /api/git-diff?session&token
//   GET /api/git-turns?session&token
//   GET /api/git-turn-diff?session&turn&token
// ============================================================
(function setupReviewView() {
  const VIEW_MODE_KEY = 'many-ai-cli.review.viewMode';

  function _esc(s: any) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, c => ({
      '&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'
    }[c]));
  }
  function _gt(key: any, fallback: any) {
    if (typeof window.t === 'function') {
      const v = window.t(key);
      if (v && v !== key) return v;
    }
    return fallback != null ? fallback : key;
  }

  // unified diff テキストを hunk（行番号付き）配列にパースする。
  // ファイルヘッダ（diff --git / index / --- / +++ 等）は最初の @@ までスキップ。
  function parseUnifiedDiff(text: any) {
    const hunks: any[] = [];
    let cur: any = null;
    for (const raw of String(text || '').split('\n')) {
      const m = raw.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
      if (m) {
        cur = { header: raw, oldStart: parseInt(m[1], 10), newStart: parseInt(m[2], 10), lines: [] };
        hunks.push(cur);
        continue;
      }
      if (!cur) continue;
      if (raw.startsWith('+'))      cur.lines.push({ t: 'add',  x: raw.slice(1) });
      else if (raw.startsWith('-')) cur.lines.push({ t: 'del',  x: raw.slice(1) });
      else if (raw.startsWith('\\')) cur.lines.push({ t: 'meta', x: raw });
      else                          cur.lines.push({ t: 'ctx',  x: raw.slice(1) });
    }
    return hunks;
  }

  // inline 表示: 旧/新の 2 列行番号つきで hunk を描画
  function renderHunkInline(h: any) {
    let o = h.oldStart, n = h.newStart;
    const out = [
      `<div class="review-line hunk"><span class="review-ln"></span><span class="review-ln"></span><span class="review-tx">${_esc(h.header)}</span></div>`,
    ];
    for (const l of h.lines) {
      let lnOld = '', lnNew = '';
      if (l.t === 'ctx')      { lnOld = String(o); lnNew = String(n); o++; n++; }
      else if (l.t === 'add') { lnNew = String(n); n++; }
      else if (l.t === 'del') { lnOld = String(o); o++; }
      const sign = l.t === 'add' ? '+' : l.t === 'del' ? '-' : ' ';
      const tx = l.t === 'meta' ? _esc(l.x) : _esc(sign + l.x);
      out.push(`<div class="review-line ${l.t}"><span class="review-ln">${lnOld}</span><span class="review-ln">${lnNew}</span><span class="review-tx">${tx || ' '}</span></div>`);
    }
    return out.join('');
  }

  // side-by-side 表示: 連続する del ブロックと add ブロックを行ペアに整列
  function renderHunkSbs(h: any) {
    const rows: any[] = [];
    let i = 0, o = h.oldStart, n = h.newStart;
    const L = h.lines;
    while (i < L.length) {
      if (L[i].t === 'meta') { rows.push({ l: { t: 'meta', ln: '', x: L[i].x }, r: { t: 'meta', ln: '', x: L[i].x } }); i++; continue; }
      if (L[i].t === 'ctx') {
        rows.push({ l: { t: 'ctx', ln: o, x: L[i].x }, r: { t: 'ctx', ln: n, x: L[i].x } });
        o++; n++; i++; continue;
      }
      const dels: any[] = [], adds: any[] = [];
      while (i < L.length && L[i].t === 'del') { dels.push({ t: 'del', ln: o, x: L[i].x }); o++; i++; }
      while (i < L.length && L[i].t === 'add') { adds.push({ t: 'add', ln: n, x: L[i].x }); n++; i++; }
      const m = Math.max(dels.length, adds.length);
      for (let j = 0; j < m; j++) rows.push({ l: dels[j] || null, r: adds[j] || null });
    }
    const cell = (c: any, side: any) => {
      if (!c) return `<div class="review-cell blank ${side}"><span class="review-ln"></span><span class="review-tx"></span></div>`;
      return `<div class="review-cell ${side} ${c.t}"><span class="review-ln">${c.ln}</span><span class="review-tx">${_esc(c.x) || ' '}</span></div>`;
    };
    let out = `<div class="review-cell l hunk"><span class="review-ln"></span><span class="review-tx">${_esc(h.header)}</span></div>` +
              `<div class="review-cell hunk"><span class="review-ln"></span><span class="review-tx"></span></div>`;
    for (const r of rows) out += cell(r.l, 'l') + cell(r.r, '');
    return `<div class="review-sbs">${out}</div>`;
  }

  // ───────────────────────────────────────────────────────
  // ReviewView クラス本体
  // ───────────────────────────────────────────────────────
  class ReviewView {
    [key: string]: any;

    constructor(containerEl: any, opts: any) {
      this.container = containerEl;
      this.opts = opts || {};
      this.sessionId = this.opts.sessionId;
      this.gitRoot   = this.opts.gitRoot || '';
      this.token     = token || '';

      this.files = [];
      this.turns = [];
      this.summary = null;
      this.branch = '';
      this.filterText = '';
      this.selectedPath = null;
      this.loading = false;
      this.loadGeneration = 0;
      this.scope = this.opts.turn ? `turn:${Number(this.opts.turn)}` : 'worktree';
      let vm = 'inline';
      try { vm = localStorage.getItem(VIEW_MODE_KEY) || 'inline'; } catch (_) {}
      this.viewMode = vm === 'sbs' ? 'sbs' : 'inline';

      this.els = {};
      this._renderShell();
      this.refresh().catch((err: any) => this._showError(err && err.message ? err.message : String(err)));
    }

    _renderShell() {
      const root = document.createElement('div');
      root.className = 'review-root';
      root.innerHTML = `
        <div class="review-header">
          <div class="review-title">± Review</div>
          <label class="review-scope-label">
            <span>${_esc(_gt('review_scope_label', 'Scope'))}</span>
            <select data-scope></select>
          </label>
          <div class="review-repo" data-repo>${_esc(this.gitRoot)}</div>
          <span class="review-branch" data-branch></span>
          <div class="review-spacer"></div>
          <div class="review-stat" data-stat>—</div>
          <div class="review-mode-group">
            <button class="git-icon-btn" data-mode="inline">${_esc(_gt('review_view_inline', 'Inline'))}</button>
            <button class="git-icon-btn" data-mode="sbs">${_esc(_gt('review_view_sbs', 'Side by side'))}</button>
          </div>
          <button class="git-icon-btn" data-refresh title="${_esc(_gt('review_view_refresh', 'Refresh'))}">↻</button>
        </div>
        <div class="review-body">
          <div class="review-diff-pane" data-diff-pane>
            <div class="review-message">${_esc(_gt('review_view_loading', 'Loading...'))}</div>
          </div>
          <div class="review-tree-pane">
            <input type="search" data-filter placeholder="${_esc(_gt('review_view_filter_placeholder', 'Filter files...'))}">
            <div class="review-tree" data-tree></div>
          </div>
        </div>
      `;
      this.container.appendChild(root);
      this.els.root      = root;
      this.els.repo      = root.querySelector('[data-repo]');
      this.els.scope     = root.querySelector('[data-scope]');
      this.els.branch    = root.querySelector('[data-branch]');
      this.els.stat      = root.querySelector('[data-stat]');
      this.els.diffPane  = root.querySelector('[data-diff-pane]');
      this.els.tree      = root.querySelector('[data-tree]');
      this.els.filter    = root.querySelector('[data-filter]');
      this.els.modeBtns  = Array.from(root.querySelectorAll('[data-mode]'));

      this.els.filter.addEventListener('input', (e: any) => {
        this.filterText = e.target.value || '';
        this._renderTree();
      });
      this.els.scope.addEventListener('change', (e: any) => {
        this.scope = e.target.value || 'worktree';
        this.selectedPath = null;
        this.load().catch((err: any) => this._showError(err && err.message ? err.message : String(err)));
      });
      this.els.modeBtns.forEach((b: any) => {
        b.addEventListener('click', () => this._setViewMode(b.dataset.mode));
      });
      root.querySelector('[data-refresh]').addEventListener('click', () => {
        this.refresh().catch((err: any) => this._showError(err && err.message ? err.message : String(err)));
      });
      this._syncModeButtons();
    }

    async load() {
      const generation = ++this.loadGeneration;
      this.loading = true;
      try {
        const outcome = await resolveCurrentReviewLoad(
          generation,
          () => this.loadGeneration,
          async () => {
            const params = new URLSearchParams({
              session: String(this.sessionId),
              token: this.token,
            });
            let endpoint = '/api/git-diff';
            let turnNo = 0;
            if (this.scope === 'last') {
              turnNo = this.turns.length ? Number(this.turns[this.turns.length - 1].turn || 0) : 0;
            } else if (String(this.scope).startsWith('turn:')) {
              turnNo = Number(String(this.scope).slice(5)) || 0;
            }
            if (turnNo > 0) {
              endpoint = '/api/git-turn-diff';
              params.set('turn', String(turnNo));
            }
            const res = await fetch(`${endpoint}?${params.toString()}`);
            const data = await res.json().catch(() => ({}));
            if (!res.ok || data.ok === false) {
              throw new Error(data && data.detail ? data.detail : `HTTP ${res.status}`);
            }
            return data;
          },
        );
        if (outcome.stale) return;
        const data = outcome.value || {};
        this.files   = Array.isArray(data.files) ? data.files : [];
        this.summary = data.summary || null;
        this.branch  = data.branch || '';
        if (data.git_root) {
          this.gitRoot = data.git_root;
          if (this.els.repo) this.els.repo.textContent = this.gitRoot;
        }
        this._renderAll();
      } finally {
        if (generation === this.loadGeneration) this.loading = false;
      }
    }

    async refresh() {
      // Invalidate an older in-flight scope/session request before refreshing
      // the turn list; otherwise its slower response can overwrite this view.
      this.loadGeneration++;
      this.selectedPath = null;
      await this._loadTurns();
      await this.load();
    }

    setSessionId(newSid: any) {
      if (newSid == null) return;
      if (String(this.sessionId) === String(newSid)) return;
      this.sessionId = newSid;
      this.scope = 'worktree';
      this.refresh().catch((err: any) => this._showError(err && err.message ? err.message : String(err)));
    }

    setScope(turnNo: any) {
      const turn = Number(turnNo || 0);
      this.scope = turn > 0 ? `turn:${turn}` : 'worktree';
      this.refresh().catch((err: any) => this._showError(err && err.message ? err.message : String(err)));
    }

    dispose() {
      try { this.container.innerHTML = ''; } catch (_) {}
    }

    _setViewMode(mode: any) {
      const m = mode === 'sbs' ? 'sbs' : 'inline';
      if (m === this.viewMode) return;
      this.viewMode = m;
      try { localStorage.setItem(VIEW_MODE_KEY, m); } catch (_) {}
      this._syncModeButtons();
      this._renderDiffPane();
    }

    _syncModeButtons() {
      this.els.modeBtns.forEach((b: any) => {
        b.classList.toggle('active', b.dataset.mode === this.viewMode);
      });
    }

    _showError(msg: any) {
      if (!this.els.diffPane) return;
      this.els.diffPane.innerHTML = `<div class="review-message error">${_esc(msg)}</div>`;
      if (this.els.tree) this.els.tree.innerHTML = '';
      if (this.els.stat) this.els.stat.textContent = '—';
    }

    async _loadTurns() {
      const sessionAtStart = String(this.sessionId);
      const params = new URLSearchParams({
        session: sessionAtStart,
        token: this.token,
      });
      const res = await fetch(`/api/git-turns?${params.toString()}`);
      const data = await res.json().catch(() => ({}));
      if (String(this.sessionId) !== sessionAtStart) return;
      if (!res.ok || data.ok === false) {
        // Phase 1 remains usable when the session is not a Git repository or
        // no turn snapshots exist yet; load() will display its own API error.
        this.turns = [];
      } else {
        this.turns = Array.isArray(data.turns) ? data.turns : [];
      }
      this._renderScopeOptions();
    }

    _renderScopeOptions() {
      if (!this.els.scope) return;
      const formatTime = (raw: any) => {
        const d = new Date(String(raw || ''));
        if (Number.isNaN(d.getTime())) return '';
        return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
      };
      const options = [
        `<option value="worktree">${_esc(_gt('review_scope_worktree', 'Uncommitted changes (vs HEAD)'))}</option>`,
      ];
      if (this.turns.length) {
        const last = this.turns[this.turns.length - 1];
        const lastLabel = String(_gt('review_scope_last', 'Last turn (#{n}, {time})'))
          .replace('{n}', String(last.turn || ''))
          .replace('{time}', formatTime(last.ended_at));
        options.push(`<option value="last">${_esc(lastLabel)}</option>`);
        for (const turn of [...this.turns].reverse()) {
          const label = String(_gt('review_scope_turn', 'Turn #{n} ({time})'))
            .replace('{n}', String(turn.turn || ''))
            .replace('{time}', formatTime(turn.ended_at));
          options.push(`<option value="turn:${Number(turn.turn)}">${_esc(label)}</option>`);
        }
      }
      this.els.scope.innerHTML = options.join('');
      const values = new Set(Array.from(this.els.scope.options).map((o: any) => o.value));
      if (!values.has(this.scope)) {
        this.scope = 'worktree';
      }
      this.els.scope.value = this.scope;
    }

    _renderAll() {
      if (this.els.branch) this.els.branch.textContent = this.branch ? `(${this.branch})` : '';
      this._renderStat();
      this._renderDiffPane();
      this._renderTree();
    }

    _renderStat() {
      const s = this.summary || { files_changed: this.files.length, added: 0, removed: 0 };
      const label = _gt('review_view_files', '{n} files').replace('{n}', String(s.files_changed || 0));
      this.els.stat.innerHTML = s.files_changed
        ? `${_esc(label)} <span class="file-stat-add">+${s.added || 0}</span> <span class="file-stat-del">-${s.removed || 0}</span>`
        : _esc(_gt('review_view_clean', 'Working tree is clean'));
    }

    _renderDiffPane() {
      if (!this.files.length) {
        this.els.diffPane.innerHTML = `<div class="review-message">${_esc(_gt('review_view_clean', 'Working tree is clean'))}</div>`;
        return;
      }
      const html = this.files.map((f: any) => {
        const hunks = parseUnifiedDiff(f.diff);
        let body = '';
        if (!f.diff) {
          body = `<div class="review-empty-diff">${_esc(_gt('review_view_no_diff', '(no diff)'))}</div>`;
        } else if (!hunks.length) {
          // バイナリ・truncated のみ等、hunk が無い diff は raw 表示
          body = `<pre class="review-raw">${_esc(f.diff)}</pre>`;
        } else if (this.viewMode === 'sbs') {
          body = hunks.map(h => renderHunkSbs(h)).join('');
        } else {
          body = `<pre class="review-code">${hunks.map(h => renderHunkInline(h)).join('')}</pre>`;
        }
        return `
          <div class="review-file" data-file-anchor="${_esc(f.path)}">
            <div class="diff-file-header review-file-header">
              <span class="file-status ${_esc(f.status || 'M')}" style="width:16px;height:16px;font-size:10px">${_esc(f.status || 'M')}</span>
              <span class="review-file-path">${_esc(f.path || '')}</span>
              <span class="file-stat-add">+${f.added || 0}</span>
              <span class="file-stat-del">-${f.removed || 0}</span>
            </div>
            ${body}
          </div>
        `;
      }).join('');
      this.els.diffPane.innerHTML = html;
      if (this.selectedPath) this._highlightSelected(false);
    }

    _renderTree() {
      if (!this.files.length) {
        this.els.tree.innerHTML = '';
        return;
      }
      const q = (this.filterText || '').trim().toLowerCase();
      const visible = this.files.filter((f: any) => !q || String(f.path || '').toLowerCase().includes(q));
      if (!visible.length) {
        this.els.tree.innerHTML = `<div class="review-tree-empty">${_esc(_gt('git_view_ref_no_match', 'No match'))}</div>`;
        return;
      }
      const groups: any = {};
      for (const f of visible) {
        const p = String(f.path || '');
        const idx = p.lastIndexOf('/');
        const dir = idx >= 0 ? p.slice(0, idx) : '';
        (groups[dir] = groups[dir] || []).push(f);
      }
      const html = Object.keys(groups).sort().map(dir => {
        const rows = groups[dir].map((f: any) => {
          const name = String(f.path || '').split('/').pop();
          const sel = this.selectedPath === f.path ? ' selected' : '';
          return `
            <div class="review-tree-file${sel}" data-path="${_esc(f.path)}">
              <span class="review-tree-name">${_esc(name)}</span>
              <span class="file-status ${_esc(f.status || 'M')}" style="width:14px;height:14px;font-size:9px">${_esc(f.status || 'M')}</span>
            </div>
          `;
        }).join('');
        return `<div class="review-tree-dir">${_esc(dir ? dir + '/' : './')}</div>${rows}`;
      }).join('');
      this.els.tree.innerHTML = html;
      this.els.tree.querySelectorAll('.review-tree-file').forEach((el: any) => {
        el.addEventListener('click', () => {
          this.selectedPath = el.dataset.path;
          this._renderTree();
          this._highlightSelected(true);
        });
      });
    }

    _highlightSelected(scroll: any) {
      this.els.diffPane.querySelectorAll('.review-file.selected').forEach((el: any) => el.classList.remove('selected'));
      if (!this.selectedPath) return;
      const target = this.els.diffPane.querySelector(`[data-file-anchor="${CSS.escape(this.selectedPath)}"]`);
      if (!target) return;
      target.classList.add('selected');
      if (scroll) {
        try { target.scrollIntoView({ block: 'start' }); } catch (_) {}
      }
    }
  }

  // ───────────────────────────────────────────────────────
  // Session-screen turn completion card
  // ───────────────────────────────────────────────────────
  const latestTurnBySession = new Map<number, any>();
  let turnCard: HTMLElement | null = null;

  function ensureTurnCard() {
    if (turnCard) return turnCard;
    const actionBar = document.getElementById('action-bar');
    if (!actionBar || !actionBar.parentElement) return null;
    const card = document.createElement('section');
    card.id = 'git-turn-card';
    card.className = 'git-turn-card';
    card.hidden = true;
    card.innerHTML = `
      <div class="git-turn-card-summary">
        <strong data-turn-title></strong>
        <span class="file-stat-add" data-turn-added></span>
        <span class="file-stat-del" data-turn-removed></span>
      </div>
      <button type="button" class="git-turn-review-btn" data-turn-review></button>
    `;
    card.querySelector('[data-turn-review]')?.addEventListener('click', () => {
      const sid = Number(card.dataset.sessionId || 0);
      const turn = Number(card.dataset.turn || 0);
      const sess: any = sessions.get(sid);
      if (!sid || !turn || !sess) return;
      window.dispatchEvent(new CustomEvent('many-ai-cli:review-turn-request', {
        detail: { sessionId: sid, turn, gitRoot: sess.git_root || sess.cwd || '' },
      }));
    });
    actionBar.parentElement.insertBefore(card, actionBar);
    turnCard = card;
    return card;
  }

  function hideTurnCard() {
    const card = ensureTurnCard();
    if (card) card.hidden = true;
  }

  function renderTurnCard(sessionID: number, turn: any) {
    const area = document.getElementById('display-area');
    if (Number(activeSessionId) !== Number(sessionID) || !area?.classList.contains('mode-terminal') || Number(turn?.files_changed || 0) <= 0) {
      hideTurnCard();
      return;
    }
    const card = ensureTurnCard();
    if (!card) return;
    const title = String(_gt('review_turn_card_title', 'Turn #{turn}: {n} files edited'))
      .replace('{turn}', String(turn.turn || ''))
      .replace('{n}', String(turn.files_changed || 0));
    const titleEl = card.querySelector('[data-turn-title]');
    const addEl = card.querySelector('[data-turn-added]');
    const removeEl = card.querySelector('[data-turn-removed]');
    const reviewBtn = card.querySelector('[data-turn-review]');
    if (titleEl) titleEl.textContent = title;
    if (addEl) addEl.textContent = `+${Number(turn.added || 0)}`;
    if (removeEl) removeEl.textContent = `-${Number(turn.removed || 0)}`;
    if (reviewBtn) reviewBtn.textContent = _gt('review_turn_card_button', 'Review');
    card.dataset.sessionId = String(sessionID);
    card.dataset.turn = String(turn.turn || '');
    card.hidden = false;
  }

  async function loadLatestTurnCard(sessionID: number) {
    if (!sessionID) {
      hideTurnCard();
      return;
    }
    try {
      const params = new URLSearchParams({ session: String(sessionID), token: token || '' });
      const res = await fetch(`/api/git-turns?${params.toString()}`);
      const data = await res.json().catch(() => ({}));
      const turns = res.ok && Array.isArray(data.turns) ? data.turns : [];
      const latest = turns.length ? turns[turns.length - 1] : null;
      if (!latest) {
        latestTurnBySession.delete(sessionID);
        hideTurnCard();
        return;
      }
      latestTurnBySession.set(sessionID, latest);
      renderTurnCard(sessionID, latest);
    } catch (_) {
      hideTurnCard();
    }
  }

  window.addEventListener('many-git-turn', (event: any) => {
    const detail = event?.detail || {};
    const sid = Number(detail.session_id || 0);
    if (!sid) return;
    latestTurnBySession.set(sid, detail);
    renderTurnCard(sid, detail);
  });

  window.addEventListener('session-view-mode-changed', (event: any) => {
    const sid = Number(event?.detail?.sid || activeSessionId || 0);
    if (event?.detail?.name !== 'terminal') {
      hideTurnCard();
      return;
    }
    const cached = latestTurnBySession.get(sid);
    if (cached) renderTurnCard(sid, cached);
    else void loadLatestTurnCard(sid);
  });

  window.ReviewView = ReviewView;
})();
