// --- ESM imports (generated) ---
import { token } from './util.js';
import { STORAGE_LANG_KEY } from './user-prefs.js';
import { appConfirm } from './settings.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- Git graph view ----
// ============================================================
// GitGraphView (C3) — git タブ contentEl 配下に SVG ブランチグラフ /
// コミット一覧 / 上下 split 詳細パネル / ref ドロップダウン /
// Copy 右クリックメニュー を描画する。
//
// 親 plan : docs/local/plan_git_graph_view.md
// 子 plan : docs/local/plan_git_graph_view_c3_graph_view.md
// mock   : docs/local/mockup-git-graph.html
//
// API:
//   GET /api/git-log?session&token&ref&limit&skip
//   GET /api/git-show?session&token&hash
//   GET /api/git-refs?session&token
// ============================================================
(function setupGitGraphView() {
  const LANE_W = 16;
  const LANE_X0 = 12;
  const ROW_H  = 30;
  const DOT_R  = 4.5;
  const LANE_COLORS = ['lane-1', 'lane-2', 'lane-3', 'lane-4'];
  const PAGE_LIMIT = 100;

  function _esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, c => ({
      '&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'
    }[c]));
  }
  function _shortHash(h) { return (h || '').slice(0, 8); }
  function _laneX(idx) { return LANE_X0 + idx * LANE_W; }
  function _laneColor(idx) {
    return LANE_COLORS[((idx % LANE_COLORS.length) + LANE_COLORS.length) % LANE_COLORS.length];
  }
  function _gt(key, fallback) {
    if (typeof window.t === 'function') {
      const v = window.t(key);
      if (v && v !== key) return v;
    }
    return fallback != null ? fallback : key;
  }
  function _toast(title, body) {
    if (typeof window.showToast === 'function') {
      const msg = body ? `${title}: ${body}` : title;
      const one = msg.replace(/\n/g, ' ↵ ');
      window.showToast(one.length > 120 ? one.slice(0, 120) + '…' : one);
    }
  }
  function _formatDate(iso) {
    if (!iso) return '';
    try {
      const d = new Date(iso);
      if (isNaN(d.getTime())) return iso;
      const pad = (n) => String(n).padStart(2, '0');
      return `${d.getFullYear()}/${pad(d.getMonth()+1)}/${pad(d.getDate())} ` +
             `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
    } catch (_) { return iso; }
  }
  function _copyText(text) {
    try {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).catch(() => {});
        return;
      }
    } catch (_) {}
    try {
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.style.position = 'fixed'; ta.style.opacity = '0';
      document.body.appendChild(ta); ta.select();
      document.execCommand('copy');
      ta.remove();
    } catch (_) {}
  }

  // ─────────────────────────────────────────────────────────
  // lane allocation
  //   activeLanes[i] = "次に出現したら閉じたい hash" もしくは null
  //   行を上から順に走査:
  //     - その行の hash がアクティブな lane で待たれていればそこに dot
  //     - 待たれていなければ新規 lane に割当
  //     - parents[0] はその lane を継続。parents[1..] は別 lane を新規確保
  // ─────────────────────────────────────────────────────────
  function computeGraph(commits) {
    const activeLanes = [];
    const laneColors = [];

    function allocLane(color?: any) {
      for (let i = 0; i < activeLanes.length; i++) {
        if (activeLanes[i] == null) {
          activeLanes[i] = '__pending__';
          laneColors[i] = color || laneColors[i] || _laneColor(i);
          return i;
        }
      }
      activeLanes.push('__pending__');
      laneColors.push(color || _laneColor(activeLanes.length - 1));
      return activeLanes.length - 1;
    }

    const rows = [];
    for (const c of commits) {
      const hash = c.hash;
      let myLane = -1;
      const incoming = []; // 自分を待っていた他 lane (merge 入線元)
      for (let i = 0; i < activeLanes.length; i++) {
        if (activeLanes[i] === hash) {
          if (myLane < 0) myLane = i;
          else {
            incoming.push({ x: i, color: laneColors[i] || _laneColor(i) });
            activeLanes[i] = null;
          }
        }
      }
      if (myLane < 0) {
        myLane = allocLane();
      }

      const parents = c.parents || [];
      const isMerge = parents.length >= 2;

      // 描画用 lanes (現在の activeLanes スナップショット)
      const drawLanes = [];
      for (let i = 0; i < activeLanes.length; i++) {
        if (i === myLane) {
          drawLanes.push({
            x: i,
            type: isMerge ? 'merge' : 'dot',
            color: laneColors[i] || _laneColor(i),
            inFrom: isMerge ? incoming.slice() : undefined,
          });
        } else if (activeLanes[i] != null && activeLanes[i] !== '__pending__') {
          drawLanes.push({
            x: i,
            type: 'line',
            color: laneColors[i] || _laneColor(i),
          });
        }
      }

      // parents 展開
      if (parents.length === 0) {
        activeLanes[myLane] = null;
      } else {
        activeLanes[myLane] = parents[0];
        const baseColor = laneColors[myLane] || _laneColor(myLane);
        const mergeExtra = [];
        for (let pi = 1; pi < parents.length; pi++) {
          const ln = allocLane();
          // 追加 parent lane の色: 元 lane と区別するため新色
          const cc = _laneColor(ln);
          laneColors[ln] = cc;
          activeLanes[ln] = parents[pi];
          mergeExtra.push({ x: ln, color: cc });
          drawLanes.push({ x: ln, type: 'line', color: cc });
        }
        // merge dot に追加 parent の入線も付与（mock の inFrom と同等）
        if (isMerge) {
          const me = drawLanes.find(l => l.x === myLane);
          if (me) {
            const fromArr = (me.inFrom || []).slice();
            me.inFrom = fromArr.concat(mergeExtra);
          }
          // base color を残す
          if (laneColors[myLane] == null) laneColors[myLane] = baseColor;
        }
      }

      rows.push({ hashLane: myLane, lanes: drawLanes });
    }
    return rows;
  }

  function renderGraphSvg(row) {
    if (!row) return '';
    const lanes = row.lanes || [];
    const maxX = lanes.reduce((m, l) => Math.max(m, l.x), 0);
    const w = Math.max(110, _laneX(maxX + 1) + 6);
    const h = ROW_H;
    const parts = [];
    for (const lane of lanes) {
      const x = _laneX(lane.x);
      const color = `var(--${lane.color})`;
      if (lane.type === 'line') {
        parts.push(`<line x1="${x}" y1="0" x2="${x}" y2="${h}" stroke="${color}" stroke-width="2"/>`);
      } else if (lane.type === 'dot') {
        parts.push(`<line x1="${x}" y1="0" x2="${x}" y2="${h}" stroke="${color}" stroke-width="2"/>`);
        parts.push(`<circle cx="${x}" cy="${h/2}" r="${DOT_R}" fill="${color}" stroke="var(--bg)" stroke-width="1.5"/>`);
      } else if (lane.type === 'merge') {
        parts.push(`<line x1="${x}" y1="0" x2="${x}" y2="${h}" stroke="${color}" stroke-width="2"/>`);
        for (const inc of (lane.inFrom || [])) {
          const xi = _laneX(inc.x);
          const ic = `var(--${inc.color})`;
          parts.push(`<path d="M ${xi} 0 C ${xi} ${h*0.55}, ${x} ${h*0.45}, ${x} ${h/2}" stroke="${ic}" stroke-width="2" fill="none"/>`);
        }
        const s = DOT_R + 0.5;
        parts.push(
          `<rect x="${x - s}" y="${h/2 - s}" width="${s*2}" height="${s*2}" ` +
          `transform="rotate(45 ${x} ${h/2})" ` +
          `fill="${color}" stroke="var(--bg)" stroke-width="1.5"/>`
        );
      }
    }
    return `<svg width="${w}" height="${h}">${parts.join('')}</svg>`;
  }

  function renderRefsInline(refs, headHash, commitHash) {
    if (!refs || !refs.length) return '';
    return refs.map(r => {
      const kind = (r.kind || 'local').toLowerCase();
      const name = _esc(r.name || '');
      const isHead = (headHash && commitHash && headHash === commitHash &&
                      (kind === 'local' || kind === 'head'));
      if (isHead) return `<span class="ref-chip head">${name}</span>`;
      if (kind === 'remote') return `<span class="ref-chip remote">${name}</span>`;
      if (kind === 'tag')    return `<span class="ref-chip tag">${name}</span>`;
      return `<span class="ref-chip local">${name}</span>`;
    }).join('');
  }

  function splitSubject(subject) {
    const m = (subject || '').match(/^([a-zA-Z0-9_-]+:)\s+(.*)$/);
    if (m) return { prefix: m[1], rest: m[2] };
    return { prefix: '', rest: subject || '' };
  }
  function statusClass(status) {
    const s = String(status || 'M').slice(0, 1).toUpperCase();
    return /^[A-Z]$/.test(s) ? s : 'U';
  }

  // ─── Copy 右クリックメニュー (全 git タブ共有) ─────────────
  let _ctxMenuEl = null;
  let _ctxTarget = null;
  let _ctxTargetEl = null;
  let _ctxGithubBase = '';

  function _ensureCtxMenu() {
    if (_ctxMenuEl) return _ctxMenuEl;
    const m = document.createElement('div');
    m.className = 'git-ctx-menu';
    m.innerHTML = `
      <div class="ctx-header" data-ctx-header>commit</div>
      <button data-action="copy-short"><span class="ctx-icon">#</span><span class="ctx-label">${_esc(_gt('git_ctx_copy_short', 'Copy short hash'))}</span><span class="ctx-hint" data-hint-short></span></button>
      <button data-action="copy-full"><span class="ctx-icon">⎘</span><span class="ctx-label">${_esc(_gt('git_ctx_copy_full', 'Copy full hash'))}</span><span class="ctx-hint" data-hint-full></span></button>
      <div class="ctx-sep"></div>
      <button data-action="copy-subject"><span class="ctx-icon">✎</span><span class="ctx-label">${_esc(_gt('git_ctx_copy_subject', 'Copy subject'))}</span></button>
      <button data-action="copy-message"><span class="ctx-icon">¶</span><span class="ctx-label">${_esc(_gt('git_ctx_copy_message', 'Copy message (subject + body)'))}</span></button>
      <button data-action="copy-hash-subject"><span class="ctx-icon">⇋</span><span class="ctx-label">${_esc(_gt('git_ctx_copy_hash_subject', 'Copy hash + subject'))}</span></button>
      <div class="ctx-sep"></div>
      <button data-action="copy-github-url" data-ctx-gh><span class="ctx-icon">↗</span><span class="ctx-label">${_esc(_gt('git_ctx_copy_github', 'Copy GitHub link'))}</span><span class="ctx-hint" data-hint-gh></span></button>
    `;
    document.body.appendChild(m);
    m.addEventListener('click', (e) => {
      const btn = e.target.closest('button[data-action]');
      if (!btn || btn.disabled || !_ctxTarget) return;
      _ctxCopy(btn.dataset.action, _ctxTarget);
      _closeCtxMenu();
    });
    document.addEventListener('click', (e) => {
      if (!_ctxMenuEl) return;
      if (!_ctxMenuEl.contains(e.target)) _closeCtxMenu();
    });
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') _closeCtxMenu();
    });
    window.addEventListener('blur', _closeCtxMenu);
    _ctxMenuEl = m;
    return m;
  }

  function _openCtxMenu(x, y, commit, rowEl, githubBase) {
    const m = _ensureCtxMenu();
    _ctxTarget = commit;
    _ctxGithubBase = githubBase || '';
    if (_ctxTargetEl) _ctxTargetEl.classList.remove('context');
    _ctxTargetEl = rowEl;
    if (rowEl) rowEl.classList.add('context');

    m.querySelector('[data-ctx-header]').textContent =
      `${_shortHash(commit.hash)}  ${commit.author_name || ''}`;
    m.querySelector('[data-hint-short]').textContent = _shortHash(commit.hash);
    m.querySelector('[data-hint-full]').textContent  = (commit.hash || '').slice(0, 14) + '…';
    const ghBtn  = m.querySelector('[data-ctx-gh]');
    const ghHint = m.querySelector('[data-hint-gh]');
    if (_ctxGithubBase) {
      ghBtn.disabled = false;
      ghHint.textContent = 'github.com/…';
    } else {
      ghBtn.disabled = true;
      ghHint.textContent = '';
    }
    m.classList.add('open');
    const r = m.getBoundingClientRect();
    const maxX = window.innerWidth  - r.width  - 4;
    const maxY = window.innerHeight - r.height - 4;
    m.style.left = Math.max(0, Math.min(x, maxX)) + 'px';
    m.style.top  = Math.max(0, Math.min(y, maxY)) + 'px';
  }
  function _closeCtxMenu() {
    if (!_ctxMenuEl) return;
    _ctxMenuEl.classList.remove('open');
    if (_ctxTargetEl) { _ctxTargetEl.classList.remove('context'); _ctxTargetEl = null; }
    _ctxTarget = null;
  }
  function _ctxCopy(action, c) {
    const sub = splitSubject(c.subject);
    const subjectFull = (sub.prefix ? sub.prefix + ' ' : '') + sub.rest;
    let value = ''; let title = '';
    switch (action) {
      case 'copy-short':
        value = _shortHash(c.hash); title = _gt('git_ctx_copy_short', 'Copy short hash'); break;
      case 'copy-full':
        value = c.hash || ''; title = _gt('git_ctx_copy_full', 'Copy full hash'); break;
      case 'copy-subject':
        value = subjectFull; title = _gt('git_ctx_copy_subject', 'Copy subject'); break;
      case 'copy-message':
        value = subjectFull + (c.body ? '\n\n' + c.body : '');
        title = _gt('git_ctx_copy_message', 'Copy message'); break;
      case 'copy-hash-subject':
        value = `${_shortHash(c.hash)} ${subjectFull}`;
        title = _gt('git_ctx_copy_hash_subject', 'Copy hash + subject'); break;
      case 'copy-github-url':
        if (!_ctxGithubBase) return;
        value = `${_ctxGithubBase.replace(/\/$/, '')}/commit/${c.hash}`;
        title = _gt('git_ctx_copy_github', 'Copy GitHub link'); break;
    }
    _copyText(value);
    _toast(title, value);
  }

  // ───────────────────────────────────────────────────────
  // GitGraphView クラス本体
  // ───────────────────────────────────────────────────────
  class GitGraphView {
    [key: string]: any;

    constructor(containerEl, opts) {
      this.container = containerEl;
      this.opts = opts || {};
      this.sessionId = this.opts.sessionId;
      this.gitRoot   = this.opts.gitRoot || '';
      this.viewRef   = this.opts.viewRef || 'HEAD';

      this.token = token || '';

      this.commits = [];
      this.filteredCommits = [];
      this.graphRows = [];
      this.skip = 0;
      this.hasMore = false;
      this.headHash = '';
      this.sessionBranch = '';
      this.refs = [];
      this.githubUrl = '';
      this.selectedHash = null;
      this.selectedShow = null;
      this.activeTab = 'info';
      this.panelHeight = 340;
      this.filterText = '';
      this.loading = false;
      this.workingTree = null;
      this.commitModalState = {
        open: false,
        reviewed: false,
        busy: false,
        generating: false,
        error: '',
        subject: '',
        body: '',
      };

      this.els = {};
      this._renderShell();
      this._refDropdownOpen = false;

      this._docClickHandler = (e) => {
        if (!this._refDropdownOpen) return;
        if (this.els.refDropdown && this.els.refDropdown.contains(e.target)) return;
        if (this.els.viewRefBtn && this.els.viewRefBtn.contains(e.target)) return;
        this._closeRefDropdown();
      };
      this._docKeyHandler = (e) => {
        if (e.key === 'Escape' && this._refDropdownOpen) this._closeRefDropdown();
      };
      document.addEventListener('click', this._docClickHandler);
      document.addEventListener('keydown', this._docKeyHandler);

      this.load().catch(err => this._showError(err && err.message ? err.message : String(err)));
    }

    _renderShell() {
      const c = this.container;
      const root = document.createElement('div');
      root.className = 'git-graph-root';
      root.innerHTML = `
        <div class="git-graph-header">
          <div class="git-graph-title">⎇ Git</div>
          <div class="git-graph-repo" data-repo>${_esc(this.gitRoot)}</div>
          <button class="view-ref-btn" data-view-ref-btn>
            <span data-view-ref-label>${_esc(this.viewRef || 'HEAD')}</span>
            <span class="chev">▾</span>
          </button>
          <div class="session-head-chip" data-session-head-chip style="display:none">session HEAD: <span data-session-head-label></span></div>
          <div class="git-graph-spacer"></div>
          <div class="git-working-count" data-working-count>—</div>
          <div class="git-graph-count" data-count>${_esc(_gt('git_view_loading', 'loading...'))}</div>
          <button class="git-icon-btn" data-fetch-btn title="${_esc(_gt('git_view_fetch', 'Fetch'))}">↓ fetch</button>
          <button class="git-icon-btn" data-pull-btn title="${_esc(_gt('git_view_pull', 'Pull (fast-forward)'))}" disabled>↧ pull</button>
          <button class="git-icon-btn" data-refresh-btn title="${_esc(_gt('git_view_refresh', 'Refresh'))}">↻</button>
          <button class="git-icon-btn" data-loadmore-btn title="${_esc(_gt('git_view_load_more', 'Load 100 more'))}">+${PAGE_LIMIT}</button>
          <button class="git-commit-all-btn" data-commit-all-btn disabled>${_esc(_gt('git_commit_all', 'Commit all'))}</button>
        </div>

        <div class="git-graph-toolbar">
          <input type="search" data-filter placeholder="${_esc(_gt('git_view_filter_placeholder', 'subject / author / hash filter'))}">
          <div class="filter-group">
            <button class="git-icon-btn" data-toggle="HEAD" title="HEAD">HEAD</button>
            <button class="git-icon-btn" data-toggle="--all" title="--all">all</button>
          </div>
        </div>

        <div class="git-working-preview" data-working-preview></div>

        <div class="git-graph-split">
          <div class="git-graph-log-table" data-log-table>
            <div class="git-graph-loading">${_esc(_gt('git_view_loading', 'Loading...'))}</div>
          </div>
          <div class="split-divider" data-divider title="${_esc(_gt('git_view_divider_tip', 'Drag to resize'))}"></div>
          <div class="detail-panel" data-detail-panel>
            <div class="detail-tabbar">
              <button class="detail-tab active" data-tab="info">${_esc(_gt('git_view_tab_info', 'INFORMATION'))}</button>
              <button class="detail-tab" data-tab="changes">${_esc(_gt('git_view_tab_changes', 'CHANGES'))}</button>
              <button class="detail-tab" data-tab="files">${_esc(_gt('git_view_tab_files', 'FILES'))}</button>
              <div class="detail-tab-spacer"></div>
              <div class="detail-tab-meta" data-detail-meta>—</div>
              <button class="detail-close" data-detail-close title="${_esc(_gt('git_view_detail_close', 'Close'))}">✕</button>
            </div>
            <div class="detail-content" data-detail-content>
              <div class="detail-empty">${_esc(_gt('git_view_select_row_hint', 'Click a row to see commit detail'))}</div>
            </div>
          </div>
        </div>

        <div class="ref-dropdown" data-ref-dropdown>
          <div class="ref-dropdown-search">
            <input type="search" data-ref-filter placeholder="${_esc(_gt('git_view_ref_filter_placeholder', 'Search branches / tags...'))}">
          </div>
          <div class="ref-list" data-ref-list></div>
        </div>

        <div class="git-commit-modal-backdrop" data-commit-modal hidden>
          <div class="git-commit-modal" role="dialog" aria-modal="true">
            <div class="git-commit-modal-head">
              <div>
                <div class="git-commit-modal-title">${_esc(_gt('git_commit_all', 'Commit all'))}</div>
                <div class="git-commit-modal-sub" data-commit-summary>—</div>
              </div>
              <button class="git-commit-close" data-commit-close title="${_esc(_gt('git_view_detail_close', 'Close'))}">✕</button>
            </div>
            <div class="git-commit-warning">${_esc(_gt('git_commit_no_push_warning', 'Only git add -A and git commit are run. Push is not run.'))}</div>
            <div class="git-commit-generate-note">${_esc(_gt('git_commit_generate_note', 'Generate has any-ai-cli analyze the diff and fill in a draft commit message. This is a lightweight heuristic, so accuracy is limited. Review it before committing.'))}</div>
            <label class="git-commit-field">
              <span>${_esc(_gt('git_commit_subject', 'Commit message subject'))}</span>
              <input type="text" data-commit-subject maxlength="200">
            </label>
            <label class="git-commit-field">
              <span>${_esc(_gt('git_commit_body', 'Optional body'))}</span>
              <textarea data-commit-body rows="6"></textarea>
            </label>
            <div class="git-commit-review" data-commit-review-box hidden>
              ${_esc(_gt('git_commit_review_ready', 'Review complete. Commit will include all current working tree changes.'))}
            </div>
            <div class="git-commit-hint" data-commit-hint hidden></div>
            <div class="git-commit-error" data-commit-error hidden></div>
            <div class="git-commit-actions">
              <button class="git-secondary-btn" data-commit-generate>${_esc(_gt('git_commit_generate_message', 'Generate'))}</button>
              <div class="git-commit-action-spacer"></div>
              <button class="git-secondary-btn" data-commit-cancel>${_esc(_gt('confirm_cancel', 'Cancel'))}</button>
              <button class="git-secondary-btn" data-commit-review>${_esc(_gt('git_commit_review', 'Review'))}</button>
              <button class="git-commit-run-btn" data-commit-run disabled>${_esc(_gt('git_commit_commit', 'Commit'))}</button>
            </div>
          </div>
        </div>
      `;
      c.appendChild(root);

      this.els.root          = root;
      this.els.repo          = root.querySelector('[data-repo]');
      this.els.viewRefBtn    = root.querySelector('[data-view-ref-btn]');
      this.els.viewRefLabel  = root.querySelector('[data-view-ref-label]');
      this.els.sessionHeadChip  = root.querySelector('[data-session-head-chip]');
      this.els.sessionHeadLabel = root.querySelector('[data-session-head-label]');
      this.els.count         = root.querySelector('[data-count]');
      this.els.workingCount  = root.querySelector('[data-working-count]');
      this.els.fetchBtn      = root.querySelector('[data-fetch-btn]');
      this.els.pullBtn       = root.querySelector('[data-pull-btn]');
      this.els.refreshBtn    = root.querySelector('[data-refresh-btn]');
      this.els.loadmoreBtn   = root.querySelector('[data-loadmore-btn]');
      this.els.commitAllBtn  = root.querySelector('[data-commit-all-btn]');
      this.els.filter        = root.querySelector('[data-filter]');
      this.els.workingPreview = root.querySelector('[data-working-preview]');
      this.els.toggles       = root.querySelectorAll('.filter-group [data-toggle]');
      this.els.logTable      = root.querySelector('[data-log-table]');
      this.els.divider       = root.querySelector('[data-divider]');
      this.els.detailPanel   = root.querySelector('[data-detail-panel]');
      this.els.detailMeta    = root.querySelector('[data-detail-meta]');
      this.els.detailContent = root.querySelector('[data-detail-content]');
      this.els.detailClose   = root.querySelector('[data-detail-close]');
      this.els.detailTabs    = root.querySelectorAll('.detail-tab[data-tab]');
      this.els.refDropdown   = root.querySelector('[data-ref-dropdown]');
      this.els.refFilter     = root.querySelector('[data-ref-filter]');
      this.els.refList       = root.querySelector('[data-ref-list]');
      this.els.commitModal   = root.querySelector('[data-commit-modal]');
      this.els.commitSummary = root.querySelector('[data-commit-summary]');
      this.els.commitClose   = root.querySelector('[data-commit-close]');
      this.els.commitCancel  = root.querySelector('[data-commit-cancel]');
      this.els.commitSubject = root.querySelector('[data-commit-subject]');
      this.els.commitBody    = root.querySelector('[data-commit-body]');
      this.els.commitGenerate = root.querySelector('[data-commit-generate]');
      this.els.commitReview  = root.querySelector('[data-commit-review]');
      this.els.commitRun     = root.querySelector('[data-commit-run]');
      this.els.commitReviewBox = root.querySelector('[data-commit-review-box]');
      this.els.commitHint    = root.querySelector('[data-commit-hint]');
      this.els.commitError   = root.querySelector('[data-commit-error]');

      this.els.fetchBtn.addEventListener('click', () => this._gitFetch());
      this.els.pullBtn.addEventListener('click', () => this._gitPull());
      this.els.refreshBtn.addEventListener('click', () => this.refresh());
      this.els.loadmoreBtn.addEventListener('click', () => this.loadMore());
      this.els.commitAllBtn.addEventListener('click', () => this._openCommitModal());
      this.els.filter.addEventListener('input', (e) => {
        this.filterText = e.target.value || '';
        this._renderLogTable();
      });
      this.els.toggles.forEach(btn => {
        btn.addEventListener('click', () => this.setViewRef(btn.dataset.toggle));
      });
      this.els.viewRefBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        if (this._refDropdownOpen) this._closeRefDropdown();
        else this._openRefDropdown();
      });
      this.els.refFilter.addEventListener('input', (e) => {
        this._renderRefList(e.target.value || '');
      });
      this.els.detailClose.addEventListener('click', () => this._toggleDetailPanel());
      this.els.commitClose.addEventListener('click', () => this._closeCommitModal());
      this.els.commitCancel.addEventListener('click', () => this._closeCommitModal());
      this.els.commitModal.addEventListener('click', (e) => {
        if (e.target === this.els.commitModal) this._closeCommitModal();
      });
      this.els.commitSubject.addEventListener('input', () => {
        this.commitModalState.subject = this.els.commitSubject.value;
        this.commitModalState.reviewed = false;
        this._renderCommitModalState();
      });
      this.els.commitBody.addEventListener('input', () => {
        this.commitModalState.body = this.els.commitBody.value;
        this.commitModalState.reviewed = false;
        this._renderCommitModalState();
      });
      this.els.commitGenerate.addEventListener('click', () => this._generateCommitMessage());
      this.els.commitReview.addEventListener('click', () => this._reviewCommitMessage());
      this.els.commitRun.addEventListener('click', () => this._commitAll());
      this.els.detailTabs.forEach(tabBtn => {
        tabBtn.addEventListener('click', () => {
          this.activeTab = tabBtn.dataset.tab;
          this._syncDetailTabs();
          if (this.selectedShow) this._renderDetailContent();
        });
      });

      this._setupDividerDrag();
      this._syncToggleButtons();
      this._updateViewRefHeader();
    }

    async load() {
      this.skip = 0;
      this.commits = [];
      this._showLoading();
      const refsPromise = this._fetchRefs();
      const statusPromise = this._fetchStatus();
      await this._fetchLog({ append: false });
      await refsPromise;
      await statusPromise;
      this._renderLogTable();
      this._renderWorkingTreePreview();
      if (!this.selectedHash && this.commits.length) {
        const head = this.commits.find(c => c.hash === this.headHash) || this.commits[0];
        if (head) this.selectCommit(head.hash).catch(() => {});
      }
    }

    async loadMore() {
      if (this.loading || !this.hasMore) return;
      this.skip = this.commits.length;
      await this._fetchLog({ append: true });
      this._renderLogTable();
    }

    async refresh() {
      this.skip = 0;
      this.commits = [];
      this.selectedHash = null;
      this.selectedShow = null;
      this._showLoading();
      await Promise.all([this._fetchLog({ append: false }), this._fetchRefs(), this._fetchStatus()]);
      this._renderLogTable();
      this._renderWorkingTreePreview();
      this._renderDetailEmpty();
    }

    async selectCommit(hash) {
      if (!hash) return;
      this.selectedHash = hash;
      this._highlightSelected();
      try {
        const data = await this._fetchShow(hash);
        if (!data || data.ok === false) {
          this._renderDetailError(data && data.detail ? data.detail : 'git-show failed');
          return;
        }
        this.selectedShow = data;
        const panel = this.els.detailPanel;
        if (panel.classList.contains('collapsed')) {
          panel.classList.remove('collapsed');
          panel.style.height = (this.panelHeight || 340) + 'px';
        }
        this._renderDetailContent();
      } catch (err) {
        this._renderDetailError(err && err.message ? err.message : String(err));
      }
    }

    setViewRef(ref) {
      if (!ref) return;
      this.viewRef = ref;
      this._updateViewRefHeader();
      this._syncToggleButtons();
      this.refresh().catch(err => this._showError(err && err.message ? err.message : String(err)));
    }

    setSessionId(newSid) {
      if (newSid == null) return;
      if (String(this.sessionId) === String(newSid)) return;
      this.sessionId = newSid;
      this.selectedHash = null;
      this.selectedShow = null;
      this.load().catch(err => this._showError(err && err.message ? err.message : String(err)));
    }

    dispose() {
      try { document.removeEventListener('click', this._docClickHandler); } catch (_) {}
      try { document.removeEventListener('keydown', this._docKeyHandler); } catch (_) {}
      try { this.container.innerHTML = ''; } catch (_) {}
    }

    async _fetchLog({ append }) {
      this.loading = true;
      try {
        const params = new URLSearchParams({
          session: String(this.sessionId),
          token: this.token,
          ref: this.viewRef || 'HEAD',
          limit: String(PAGE_LIMIT),
          skip: String(this.skip),
        });
        const res = await fetch(`/api/git-log?${params.toString()}`);
        const data = await res.json().catch(() => ({}));
        if (!res.ok || data.ok === false) {
          throw new Error(data && data.detail ? data.detail : `HTTP ${res.status}`);
        }
        if (!append) this.commits = [];
        const got = Array.isArray(data.commits) ? data.commits : [];
        this.commits = this.commits.concat(got);
        this.headHash      = data.head_hash || '';
        this.sessionBranch = data.branch || '';
        this.hasMore       = !!data.has_more;
        if (data.git_root && !this.gitRoot) {
          this.gitRoot = data.git_root;
          if (this.els.repo) this.els.repo.textContent = this.gitRoot;
        }
      } catch (err) {
        if (!append) this._showError(err.message || String(err));
        throw err;
      } finally {
        this.loading = false;
      }
    }

    async _fetchRefs() {
      try {
        const params = new URLSearchParams({
          session: String(this.sessionId),
          token: this.token,
        });
        const res = await fetch(`/api/git-refs?${params.toString()}`);
        const data = await res.json().catch(() => ({}));
        if (!res.ok || data.ok === false) return;
        this.refs = Array.isArray(data.refs) ? data.refs : [];
        this.githubUrl = data.github_url || '';
        if (data.head) this.sessionBranch = data.head;
        this._updateViewRefHeader();
      } catch (_) { /* noop */ }
    }

    async _fetchStatus() {
      try {
        const params = new URLSearchParams({
          session: String(this.sessionId),
          token: this.token,
        });
        const res = await fetch(`/api/git-status?${params.toString()}`);
        const data = await res.json().catch(() => ({}));
        if (!res.ok || data.ok === false) {
          this.workingTree = null;
          this._renderWorkingTreePreview();
          return;
        }
        this.workingTree = data;
        if (data.git_root && !this.gitRoot) {
          this.gitRoot = data.git_root;
          if (this.els.repo) this.els.repo.textContent = this.gitRoot;
        }
      } catch (_) {
        this.workingTree = null;
      } finally {
        this._renderWorkingTreePreview();
      }
    }

    async _fetchShow(hash) {
      const params = new URLSearchParams({
        session: String(this.sessionId),
        token: this.token,
        hash,
      });
      const res = await fetch(`/api/git-show?${params.toString()}`);
      const data = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(data && data.detail ? data.detail : `HTTP ${res.status}`);
      return data;
    }

    _renderWorkingTreePreview() {
      const wt = this.workingTree;
      const files = wt && Array.isArray(wt.files) ? wt.files : [];
      const changed = wt && wt.summary ? wt.summary.files_changed || files.length : files.length;
      if (this.els.workingCount) {
        this.els.workingCount.textContent = changed
          ? _gt('git_commit_changed_count', '{n} files changed').replace('{n}', String(changed))
          : _gt('git_commit_no_changes', 'No changes');
      }
      if (this.els.commitAllBtn) {
        this.els.commitAllBtn.disabled = !changed;
      }
      this._updatePullButton();
      if (!this.els.workingPreview) return;
      if (!changed) {
        this.els.workingPreview.innerHTML = `
          <div class="git-working-preview-empty">${_esc(_gt('git_commit_no_changes', 'No changes'))}</div>
        `;
        return;
      }
      const summary = wt.summary || {};
      const max = 8;
      const rows = files.slice(0, max).map(f => {
        const added = f.added == null ? '' : `<span class="file-stat-add">+${f.added || 0}</span>`;
        const removed = f.removed == null ? '' : `<span class="file-stat-del">-${f.removed || 0}</span>`;
        return `
          <div class="git-working-file-row">
            <span class="file-status ${_esc(statusClass(f.status))}">${_esc(f.status || 'M')}</span>
            <span class="file-path">${_esc(f.path || '')}</span>
            ${added}${removed}
          </div>
        `;
      }).join('');
      const more = files.length > max
        ? `<div class="git-working-more">+${files.length - max} ${_esc(_gt('git_commit_more_files', 'more files'))}</div>`
        : '';
      this.els.workingPreview.innerHTML = `
        <div class="git-working-preview-head">
          <span>${_esc(_gt('git_commit_working_tree_preview', 'Working tree preview'))}</span>
          <span class="git-working-summary">${changed} files · +${summary.added || 0} -${summary.removed || 0}</span>
        </div>
        <div class="git-working-files">${rows}${more}</div>
      `;
    }

    _openCommitModal() {
      if (!this.workingTree || !this.workingTree.has_changes) return;
      this.commitModalState = {
        open: true,
        reviewed: false,
        busy: false,
        generating: false,
        error: '',
        subject: '',
        body: '',
      };
      this._renderCommitModalState();
      this.els.commitModal.hidden = false;
      _toast(
        _gt('git_commit_all', 'Commit all'),
        _gt('git_commit_open_toast', 'Commit all will stage all current working tree changes with git add -A, then commit them. Push is not run.')
      );
      setTimeout(() => { try { this.els.commitSubject.focus(); } catch (_) {} }, 0);
    }

    _closeCommitModal() {
      this.commitModalState.open = false;
      this.els.commitModal.hidden = true;
    }

    _renderCommitModalState() {
      const st = this.commitModalState;
      const wt = this.workingTree || {};
      const summary = wt.summary || {};
      if (this.els.commitSummary) {
        const repo = wt.repo_name || this.gitRoot || '';
        const branch = wt.branch || this.sessionBranch || 'HEAD';
        this.els.commitSummary.textContent =
          `${repo} · ${branch} · ${summary.files_changed || 0} files`;
      }
      if (this.els.commitSubject && this.els.commitSubject.value !== st.subject) {
        this.els.commitSubject.value = st.subject;
      }
      if (this.els.commitBody && this.els.commitBody.value !== st.body) {
        this.els.commitBody.value = st.body;
      }
      const hasSubject = (st.subject || '').trim() !== '';
      this.els.commitReview.disabled = !hasSubject || st.busy || st.generating;
      this.els.commitRun.disabled = !hasSubject || !st.reviewed || st.busy || st.generating;
      this.els.commitRun.title = !hasSubject
        ? _gt('git_commit_disabled_no_subject', 'Enter a subject first.')
        : (!st.reviewed ? _gt('git_commit_disabled_needs_review', 'Press Review to enable Commit.') : '');
      this.els.commitReview.classList.toggle('primary', hasSubject && !st.reviewed && !st.busy && !st.generating);
      this.els.commitGenerate.disabled = st.busy || st.generating;
      this.els.commitSubject.disabled = st.busy;
      this.els.commitBody.disabled = st.busy;
      this.els.commitReviewBox.hidden = !st.reviewed;
      if (this.els.commitHint) {
        const showHint = hasSubject && !st.reviewed && !st.busy && !st.generating;
        this.els.commitHint.hidden = !showHint;
        this.els.commitHint.textContent = showHint
          ? _gt('git_commit_review_required_hint', 'Press Review to enable Commit.')
          : '';
      }
      if (st.error) {
        this.els.commitError.hidden = false;
        this.els.commitError.textContent = st.error;
      } else {
        this.els.commitError.hidden = true;
        this.els.commitError.textContent = '';
      }
      this.els.commitGenerate.textContent = st.generating
        ? _gt('git_commit_generating', 'Generating...')
        : _gt('git_commit_generate_message', 'Generate');
      this.els.commitRun.textContent = st.busy
        ? _gt('git_commit_committing', 'Committing...')
        : _gt('git_commit_commit', 'Commit');
    }

    async _generateCommitMessage() {
      const st = this.commitModalState;
      st.generating = true;
      st.error = '';
      this._renderCommitModalState();
      try {
        const res = await fetch(`/api/git-commit-message?token=${encodeURIComponent(this.token)}`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            session: this.sessionId,
            token: this.token,
            mode: 'generate',
            language: (localStorage.getItem(STORAGE_LANG_KEY) || 'ja').startsWith('en') ? 'en' : 'ja',
          }),
        });
        const data = await res.json().catch(() => ({}));
        if (!res.ok || data.ok === false) {
          throw new Error(data && data.detail ? data.detail : `HTTP ${res.status}`);
        }
        st.subject = data.subject || '';
        st.body = data.body || '';
        st.reviewed = false;
      } catch (err) {
        st.error = err && err.message ? err.message : String(err);
      } finally {
        st.generating = false;
        this._renderCommitModalState();
      }
    }

    _reviewCommitMessage() {
      const st = this.commitModalState;
      st.subject = this.els.commitSubject.value || '';
      st.body = this.els.commitBody.value || '';
      st.error = '';
      st.reviewed = st.subject.trim() !== '';
      this._renderCommitModalState();
    }

    async _gitFetch() {
      const btn = this.els.fetchBtn;
      if (btn.disabled) return;
      btn.disabled = true;
      const origText = btn.textContent;
      btn.textContent = '…';
      try {
        const res = await fetch(`/api/git-fetch?token=${encodeURIComponent(this.token)}`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ session: this.sessionId, token: this.token }),
        });
        const data = await res.json().catch(() => ({}));
        if (!res.ok || data.ok === false) {
          const detail = data && data.detail ? data.detail : `HTTP ${res.status}`;
          _toast(_gt('git_fetch_failed', 'Fetch failed'), detail);
          return;
        }
        _toast(_gt('git_fetch_done', 'Fetch complete'), '');
        await this.refresh();
      } catch (err) {
        _toast(_gt('git_fetch_failed', 'Fetch failed'), err && err.message ? err.message : String(err));
      } finally {
        btn.textContent = origText;
        btn.disabled = false;
      }
    }

    async _gitPull() {
      const btn = this.els.pullBtn;
      if (!btn || btn.disabled) return;
      // pull は working tree を書き換える操作のため、実行前に確認を挟む
      const ok = await appConfirm({
        title: _gt('git_pull_confirm_title', 'Pull remote changes'),
        message: _gt('git_pull_confirm_message', 'Pull (fast-forward) from upstream and update the working tree. Continue?'),
        confirmText: _gt('git_pull_confirm_run', 'Pull'),
        cancelText: _gt('spawn_cancel', 'Cancel'),
        kind: 'default',
      });
      if (!ok) return;
      // 実行中は _updatePullButton にラベルを上書きさせない（'…' を保持する）
      btn.dataset.busy = '1';
      btn.disabled = true;
      btn.textContent = '…';
      try {
        const res = await fetch(`/api/git-pull?token=${encodeURIComponent(this.token)}`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ session: this.sessionId, token: this.token }),
        });
        const data = await res.json().catch(() => ({}));
        if (!res.ok || data.ok === false) {
          const detail = data && data.detail ? data.detail : `HTTP ${res.status}`;
          if (data && data.error === 'not_fast_forward') {
            _toast(_gt('git_pull_diverged', 'Branch has diverged — manual merge needed'), detail);
          } else {
            _toast(_gt('git_pull_failed', 'Pull failed'), detail);
          }
          return;
        }
        _toast(_gt('git_pull_done', 'Pull complete'), '');
        await this.refresh();
      } catch (err) {
        _toast(_gt('git_pull_failed', 'Pull failed'), err && err.message ? err.message : String(err));
      } finally {
        delete btn.dataset.busy;
        this._updatePullButton();
      }
    }

    // _updatePullButton は workingTree の ahead/behind 状態から pull ボタンの
    // ラベル・活性・強調を更新する。behind>0 のときのみ活性化し「↧ N」を表示する。
    _updatePullButton() {
      const btn = this.els.pullBtn;
      if (!btn) return;
      if (btn.dataset.busy === '1') return; // 実行中は '…' を保持
      const wt = this.workingTree;
      const behind = wt && Number.isFinite(wt.behind) ? wt.behind : 0;
      const hasUpstream = !!(wt && wt.has_upstream);
      btn.title = _gt('git_view_pull', 'Pull (fast-forward)');
      if (hasUpstream && behind > 0) {
        btn.textContent = `↧ ${behind}`;
        btn.disabled = false;
        btn.classList.add('git-behind');
      } else {
        btn.textContent = '↧ pull';
        btn.disabled = true;
        btn.classList.remove('git-behind');
      }
    }

    async _commitAll() {
      const st = this.commitModalState;
      if (!st.reviewed || !(st.subject || '').trim()) return;
      st.busy = true;
      st.error = '';
      this._renderCommitModalState();
      try {
        const res = await fetch(`/api/git-commit-all?token=${encodeURIComponent(this.token)}`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            session: this.sessionId,
            token: this.token,
            subject: st.subject,
            body: st.body,
          }),
        });
        const data = await res.json().catch(() => ({}));
        if (!res.ok || data.ok === false) {
          const msg = data && data.error === 'commit_identity_missing'
            ? _gt('git_commit_identity_missing', 'Git user.name / user.email is not configured.')
            : (data && data.detail ? data.detail : `HTTP ${res.status}`);
          throw new Error(msg);
        }
        this._closeCommitModal();
        _toast(_gt('git_commit_created', 'Commit created'), `${data.short_hash || ''} ${data.subject || ''}`.trim());
        await this.refresh();
      } catch (err) {
        st.error = err && err.message ? err.message : String(err);
      } finally {
        st.busy = false;
        this._renderCommitModalState();
      }
    }

    _showLoading() {
      this.els.logTable.innerHTML =
        `<div class="git-graph-loading">${_esc(_gt('git_view_loading', 'Loading...'))}</div>`;
      if (this.els.count) this.els.count.textContent = _gt('git_view_loading', 'Loading...');
    }
    _showError(msg) {
      this.els.logTable.innerHTML =
        `<div class="git-graph-error">${_esc(msg)}</div>`;
      if (this.els.count) this.els.count.textContent = '—';
    }

    _filterCommits() {
      const f = (this.filterText || '').trim().toLowerCase();
      if (!f) return this.commits.slice();
      return this.commits.filter(c =>
        (c.subject || '').toLowerCase().includes(f)
        || (c.author_name || '').toLowerCase().includes(f)
        || (c.author_email || '').toLowerCase().includes(f)
        || (c.hash || '').toLowerCase().includes(f)
        || (c.short_hash || '').toLowerCase().includes(f)
      );
    }

    _renderLogTable() {
      const list = this._filterCommits();
      this.filteredCommits = list;
      this.graphRows = computeGraph(list);

      if (!list.length) {
        this.els.logTable.innerHTML =
          `<div class="git-graph-loading">${_esc(_gt('git_view_no_commits', 'No commits'))}</div>`;
      } else {
        const html = list.map((c, i) => {
          const row = this.graphRows[i] || { lanes: [] };
          const sub = splitSubject(c.subject);
          const isHead = (this.headHash && c.hash === this.headHash);
          const refsHtml = renderRefsInline(c.refs, this.headHash, c.hash);
          return `
            <div class="log-row${isHead ? ' head' : ''}${this.selectedHash === c.hash ? ' selected' : ''}" data-hash="${_esc(c.hash)}">
              <div class="col-graph">${renderGraphSvg(row)}</div>
              <div class="col-refs">${refsHtml}</div>
              <div class="col-subject">${sub.prefix ? `<span class="prefix">${_esc(sub.prefix)}</span>` : ''}${_esc(sub.rest)}</div>
              <div class="col-author">${_esc(c.author_name || '')}</div>
              <div class="col-hash-wrap"><span class="col-hash">${_esc(c.short_hash || _shortHash(c.hash))}</span></div>
              <div class="col-date">${_esc(_formatDate(c.author_date))}</div>
            </div>
          `;
        }).join('');
        this.els.logTable.innerHTML = html;
        this.els.logTable.querySelectorAll('.log-row').forEach(el => {
          el.addEventListener('click', () => {
            const h = el.dataset.hash;
            this.selectCommit(h).catch(() => {});
          });
          el.addEventListener('contextmenu', (e) => {
            e.preventDefault();
            const h = el.dataset.hash;
            const c = this.commits.find(x => x.hash === h);
            if (c) _openCtxMenu(e.clientX, e.clientY, c, el, this.githubUrl);
          });
        });
        this.els.logTable.onscroll = () => _closeCtxMenu();
      }

      const total = this.commits.length;
      const fcount = list.length;
      const filtered = (this.filterText || '').trim() !== '';
      if (this.els.count) {
        if (filtered) {
          this.els.count.textContent = `${fcount}/${total} commits (filtered)`;
        } else {
          this.els.count.textContent = `${total} commits${this.hasMore ? ' · has more' : ''}`;
        }
      }
      if (this.els.loadmoreBtn) this.els.loadmoreBtn.disabled = !this.hasMore || filtered;
    }

    _highlightSelected() {
      if (!this.els.logTable) return;
      this.els.logTable.querySelectorAll('.log-row.selected').forEach(el => el.classList.remove('selected'));
      if (this.selectedHash) {
        const sel = this.els.logTable.querySelector(`.log-row[data-hash="${CSS.escape(this.selectedHash)}"]`);
        if (sel) sel.classList.add('selected');
      }
    }

    _renderDetailEmpty() {
      this.els.detailMeta.textContent = '—';
      this.els.detailContent.innerHTML =
        `<div class="detail-empty">${_esc(_gt('git_view_select_row_hint', 'Click a row to see commit detail'))}</div>`;
    }
    _renderDetailError(msg) {
      this.els.detailContent.innerHTML =
        `<div class="git-graph-error">${_esc(msg)}</div>`;
    }
    _toggleDetailPanel() {
      const p = this.els.detailPanel;
      if (p.classList.contains('collapsed')) {
        p.classList.remove('collapsed');
        p.style.height = (this.panelHeight || 340) + 'px';
      } else {
        const h = p.getBoundingClientRect().height;
        if (h > 60) this.panelHeight = h;
        p.classList.add('collapsed');
      }
    }
    _syncDetailTabs() {
      this.els.detailTabs.forEach(t => {
        t.classList.toggle('active', t.dataset.tab === this.activeTab);
      });
    }
    _renderDetailContent() {
      const c = this.selectedShow;
      if (!c) { this._renderDetailEmpty(); return; }
      this.els.detailMeta.textContent =
        `${_shortHash(c.hash)} · ${c.author_name || ''} · ${_formatDate(c.author_date)}`;
      if (this.activeTab === 'info')         this.els.detailContent.innerHTML = this._renderInfoTab(c);
      else if (this.activeTab === 'changes') this.els.detailContent.innerHTML = this._renderChangesTab(c);
      else if (this.activeTab === 'files')   this.els.detailContent.innerHTML = this._renderFilesTab(c);
      this._wireDetailLinks(c);
    }

    _renderInfoTab(c) {
      const sub = splitSubject(c.subject);
      const subjectFull = (sub.prefix ? sub.prefix + ' ' : '') + sub.rest;
      const initial = ((c.author_name || '?')[0] || '?').toUpperCase();
      const parents = Array.isArray(c.parents) ? c.parents : [];
      const refs = c.refs || [];
      const refsHtml = refs.length
        ? renderRefsInline(refs, this.headHash, c.hash)
        : `<span style="color:var(--muted);font-size:11.5px">—</span>`;
      const files = Array.isArray(c.files) ? c.files : [];
      const filesSection = files.length
        ? `<div class="info-label">FILES</div><div class="info-value">${files.map(f => `
            <div class="file-row" data-file-action="changes" data-path="${_esc(f.path)}">
              <span class="file-status ${_esc(f.status || 'M')}">${_esc(f.status || 'M')}</span>
              <span class="file-path">${_esc(f.path || '')}</span>
              <span class="file-stat-add">+${f.added || 0}</span>
              <span class="file-stat-del">-${f.removed || 0}</span>
            </div>`).join('')}</div>`
        : `<div class="info-label">FILES</div><div class="info-value" style="color:var(--muted);font-size:11.5px">—</div>`;

      const ghDisabled = this.githubUrl ? '' : 'disabled';
      const bodyHtml = c.body
        ? _esc(c.body)
        : `<span style="color:var(--muted);font-size:11.5px">${_esc(_gt('git_view_no_body', '(no body)'))}</span>`;

      return `
        <div class="info-grid">
          <div class="info-label">AUTHOR</div>
          <div class="info-value info-author">
            <div class="info-avatar">${_esc(initial)}</div>
            <div>
              <div><span class="info-author-name">${_esc(c.author_name || '')}</span><span class="info-author-email">${_esc(c.author_email || '')}</span></div>
              <div class="info-author-date">${_esc(_formatDate(c.author_date))}</div>
            </div>
          </div>

          <div class="info-label">SHA</div>
          <div class="info-value info-sha">
            <span class="info-sha-value">${_esc(c.hash || '')}</span>
            <button class="info-mini-btn" data-action="copy-full">⎘ Copy</button>
            <button class="info-mini-btn" data-action="copy-github-url" ${ghDisabled}>↗ GH</button>
          </div>

          <div class="info-label">PARENTS</div>
          <div class="info-value">${parents.length
            ? parents.map(p => `<span class="info-parent-link" data-parent="${_esc(p)}">${_esc(_shortHash(p))}</span>`).join('')
            : `<span style="color:var(--muted);font-size:11.5px">(root commit)</span>`}</div>

          <div class="info-label">REFS</div>
          <div class="info-value">${refsHtml}</div>

          <div class="info-label">MESSAGE</div>
          <div class="info-value"><div class="info-message"><span class="subject">${_esc(subjectFull)}</span>${bodyHtml}</div></div>

          ${filesSection}
        </div>
      `;
    }

    _renderFilesTab(c) {
      const files = Array.isArray(c.files) ? c.files : [];
      if (!files.length) return `<div class="detail-empty">${_esc(_gt('git_view_no_files', 'No file changes'))}</div>`;
      return files.map(f => `
        <div class="file-row" data-file-action="changes" data-path="${_esc(f.path)}">
          <span class="file-status ${_esc(f.status || 'M')}">${_esc(f.status || 'M')}</span>
          <span class="file-path">${_esc(f.path || '')}</span>
          <span class="file-stat-add">+${f.added || 0}</span>
          <span class="file-stat-del">-${f.removed || 0}</span>
        </div>
      `).join('');
    }

    _renderChangesTab(c) {
      const files = Array.isArray(c.files) ? c.files : [];
      if (!files.length) return `<div class="detail-empty">${_esc(_gt('git_view_no_diff', 'No diff'))}</div>`;
      return files.map(f => {
        const diff = (f.diff || '').split('\n').map(line => {
          let cls = 'ctx';
          if (line.startsWith('@@'))      cls = 'hunk';
          else if (line.startsWith('+++') || line.startsWith('---')) cls = 'ctx';
          else if (line.startsWith('+'))  cls = 'add';
          else if (line.startsWith('-'))  cls = 'del';
          return `<span class="diff-line ${cls}">${_esc(line) || ' '}</span>`;
        }).join('');
        return `
          <div class="diff-file">
            <div class="diff-file-header">
              <span class="file-status ${_esc(f.status || 'M')}" style="width:16px;height:16px;font-size:10px">${_esc(f.status || 'M')}</span>
              <span style="flex:1">${_esc(f.path || '')}</span>
              <span class="file-stat-add">+${f.added || 0}</span>
              <span class="file-stat-del">-${f.removed || 0}</span>
            </div>
            <div class="diff-body">${diff}</div>
          </div>
        `;
      }).join('');
    }

    _wireDetailLinks(c) {
      const content = this.els.detailContent;
      const ghBase = this.githubUrl;
      content.querySelectorAll('[data-action]').forEach(btn => {
        btn.addEventListener('click', () => {
          _ctxGithubBase = ghBase;
          _ctxCopy(btn.dataset.action, c);
        });
      });
      content.querySelectorAll('[data-parent]').forEach(link => {
        link.addEventListener('click', () => this._jumpToCommit(link.dataset.parent));
      });
      content.querySelectorAll("[data-file-action='changes']").forEach(row => {
        row.addEventListener('click', () => {
          this.activeTab = 'changes';
          this._syncDetailTabs();
          this._renderDetailContent();
        });
      });
    }

    _jumpToCommit(hash) {
      const target = this.commits.find(x => x.hash === hash);
      if (!target) {
        _toast(_gt('git_view_parent_not_in_list', 'Parent commit not in list'),
               `${_shortHash(hash)}`);
        return;
      }
      const el = this.els.logTable.querySelector(`.log-row[data-hash="${CSS.escape(hash)}"]`);
      if (el) {
        this.selectCommit(hash).catch(() => {});
        try { el.scrollIntoView({ block: 'center', behavior: 'smooth' }); } catch (_) {}
      }
    }

    _setupDividerDrag() {
      const divider = this.els.divider;
      const panel   = this.els.detailPanel;
      let startY = 0, startH = 0;
      const onMove = (e) => {
        const dy = startY - e.clientY;
        const newH = Math.max(0, Math.min(window.innerHeight - 180, startH + dy));
        panel.style.height = newH + 'px';
        if (newH > 60) {
          panel.classList.remove('collapsed');
          this.panelHeight = newH;
        }
      };
      const onUp = () => {
        document.removeEventListener('mousemove', onMove);
        document.removeEventListener('mouseup', onUp);
        divider.classList.remove('dragging');
        document.body.style.cursor = '';
        if (parseInt(panel.style.height || '0', 10) < 40) {
          panel.classList.add('collapsed');
        }
      };
      divider.addEventListener('mousedown', (e) => {
        startY = e.clientY;
        startH = panel.getBoundingClientRect().height;
        divider.classList.add('dragging');
        document.body.style.cursor = 'row-resize';
        document.addEventListener('mousemove', onMove);
        document.addEventListener('mouseup', onUp);
        e.preventDefault();
      });
    }

    _syncToggleButtons() {
      const v = this.viewRef;
      this.els.toggles.forEach(b => {
        const target = b.dataset.toggle;
        b.classList.toggle('active', target === v);
      });
    }

    _openRefDropdown() {
      const dd = this.els.refDropdown;
      const btn = this.els.viewRefBtn;
      const r = btn.getBoundingClientRect();
      dd.style.left = r.left + 'px';
      dd.style.top  = (r.bottom + 4) + 'px';
      dd.classList.add('open');
      this._refDropdownOpen = true;
      this._renderRefList('');
      if (this.els.refFilter) {
        this.els.refFilter.value = '';
        setTimeout(() => { try { this.els.refFilter.focus(); } catch (_) {} }, 0);
      }
    }
    _closeRefDropdown() {
      this.els.refDropdown.classList.remove('open');
      this._refDropdownOpen = false;
    }
    _renderRefList(filter) {
      const f = (filter || '').trim().toLowerCase();
      const KIND_ICON = { local: '⎇', remote: '↑', tag: '🏷', special: '★' };
      const special = [
        { kind: 'special', name: 'HEAD',  hash: '' },
        { kind: 'special', name: '--all', hash: '' },
      ];
      const grouped = { local: [], remote: [], tag: [] };
      for (const r of (this.refs || [])) {
        const k = (r.kind || 'local').toLowerCase();
        if (k === 'head') continue;
        if (grouped[k]) grouped[k].push(r);
      }
      const sections = [
        { kind: 'special', label: _gt('git_view_ref_section_special', 'Special'), items: special },
        { kind: 'local',   label: _gt('git_view_ref_section_local',   'Local branches'), items: grouped.local },
        { kind: 'remote',  label: _gt('git_view_ref_section_remote',  'Remote branches'), items: grouped.remote },
        { kind: 'tag',     label: _gt('git_view_ref_section_tag',     'Tags'), items: grouped.tag },
      ];
      const matches = (r) => !f || (r.name || '').toLowerCase().includes(f);

      const html = sections.map(sec => {
        const items = sec.items.filter(matches);
        if (!items.length) return '';
        return `<div class="ref-section">${_esc(sec.label)}</div>` + items.map(r => {
          const isActive = (r.name === this.viewRef);
          return `
            <div class="ref-item${isActive ? ' active' : ''}" data-ref="${_esc(r.name)}">
              <span class="ref-check">${isActive ? '✓' : ''}</span>
              <span class="ref-name ${_esc(r.kind || 'local')}"><span class="ico">${_esc(KIND_ICON[r.kind] || '?')}</span>${_esc(r.name)}</span>
              ${r.hash ? `<span class="ref-hash">${_esc(_shortHash(r.hash))}</span>` : '<span></span>'}
            </div>
          `;
        }).join('');
      }).join('');

      this.els.refList.innerHTML = html ||
        `<div class="ref-section" style="opacity:0.6">${_esc(_gt('git_view_ref_no_match', 'No match'))}</div>`;
      this.els.refList.querySelectorAll('.ref-item').forEach(el => {
        el.addEventListener('click', () => {
          this._closeRefDropdown();
          this.setViewRef(el.dataset.ref);
        });
      });
    }

    _updateViewRefHeader() {
      const label = this.els.viewRefLabel;
      const btn   = this.els.viewRefBtn;
      const chip  = this.els.sessionHeadChip;
      const chipLabel = this.els.sessionHeadLabel;
      if (!label || !btn || !chip) return;

      const displayName = (this.viewRef === 'HEAD') ? (this.sessionBranch || 'HEAD') : this.viewRef;
      label.textContent = displayName;

      const sessionHead = this.sessionBranch || '';
      const diverged = (this.viewRef !== 'HEAD' &&
                        this.viewRef !== sessionHead &&
                        this.viewRef !== '--all' &&
                        sessionHead !== '');
      btn.classList.toggle('diverged', diverged);
      chip.style.display = diverged ? '' : 'none';
      if (chipLabel) chipLabel.textContent = sessionHead;
    }
  }

  window.GitGraphView = GitGraphView;
})();
