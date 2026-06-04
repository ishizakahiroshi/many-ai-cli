// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { escapeHtml, showToast, ti18n, token } from './util.js';
import { activeSessionId, sessions } from './state.js';
import { callOpenApi, computeRelPath, copyPathText, getFilesAssetUrl, isAnyAiCliPreviewable, isImagePath, isMediaPath, isVideoPath, showPathPopup } from './path-links.js';
import { refitActiveTerminalAfterLayout } from './terminal.js';
import { markTabLazyLoaded, refreshLazyTabClasses } from './settings.js';
import { openLightbox, terminalWrapper } from './attachments.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- Files tab manager ----
// ---- v2: Files タブシステム ----

// localStorage キー移行（旧 docs キー → 新 files キー、1 回限り）
(function migrateDocsToFilesLS() {
  const moves = [
    ['any-ai-cli.docs.tabs',        'any-ai-cli.files.tabs'],
    ['any_ai_cli_docs_tree_width',  'any_ai_cli_files_tree_width'],
  ];
  for (const [oldK, newK] of moves) {
    try {
      const v = localStorage.getItem(oldK);
      if (v != null && localStorage.getItem(newK) == null) {
        localStorage.setItem(newK, v);
      }
      if (v != null) localStorage.removeItem(oldK);
    } catch (_) {}
  }
})();

/**
 * FilesTabManager — メインエリアのタブ管理
 *
 * - openFilesTab(sessionId, projectKey, filesRoot) → tabId
 * - closeFilesTab(tabId)
 * - switchToSessionView()   ← セッションカード切替時に呼ぶ
 * - updateSessionTabLabel(label) ← セッション切替時にセッションタブの表示名を更新
 * - restoreFromLocalStorage()  ← 起動時に呼ぶ
 */
export const FilesTabManager = (function () {
  const LS_KEY = 'any-ai-cli.files.tabs';
  const tabList = document.getElementById('main-tab-list');
  const tabBar  = document.getElementById('main-tab-bar');
  const terminalWrapper = document.getElementById('terminal-wrapper');
  const filesContents   = document.getElementById('files-tab-contents');

  if (!tabList || !tabBar || !terminalWrapper || !filesContents) {
    return {
      openFilesTab: () => null,
      openGitTab: () => null,
      closeFilesTab: () => {},
      closeMainTab: () => {},
      switchToSessionView: () => {},
      updateSessionTabLabel: () => {},
      restoreFromLocalStorage: () => {},
      hasGitTabForRoot: () => false,
      hasFilesTabForRoot: () => false,
      onSessionRemoved: () => {},
      getSessionsRef: () => null,
      setSessionsRef: () => {},
    };
  }

  // タブデータ構造: { id, type/kind: 'session'|'files'|'git', label, sessionId?, filesRoot?, gitRoot?, viewRef?, projectName?, el, contentEl }
  // - kind は新エイリアス（'session'|'files'|'git'）。type は後方互換で残置（'files' のみ既存ロジック互換に使う）。
  let tabs = [];
  let activeTabId = null;
  let sessionTabEl = null;  // セッションタブ（常に1枚）

  // 外部から渡される sessions(Map) への参照（restore / 付け替えで使う）。
  // app.js 上部の `let sessions = new Map()` を直接参照できないので setter で受け取る。
  let sessionsRef = null;
  function setSessionsRef(map) { sessionsRef = map; }
  function getSessionsRef() { return sessionsRef; }

  // ─── LS 読み書き（schema v2: { v:2, files: {gitRoot:[...]}, git: [...] }） ─────
  function lsLoadRaw() {
    try { return JSON.parse(localStorage.getItem(LS_KEY) || '{}'); } catch (_) { return {}; }
  }
  function lsLoad() {
    const raw = lsLoadRaw();
    // v1（旧形式: gitRoot→entries[] のフラット map）→ v2 への一度きり migration
    if (!raw || typeof raw !== 'object') return { v: 2, files: {}, git: [] };
    if (raw.v === 2 && raw.files && Array.isArray(raw.git)) return raw;
    // v1 とみなす（{gitRoot: [{root, openedFile}, ...], ...} 直下に gitRoot キー）
    const files = {};
    for (const [k, v] of Object.entries(raw)) {
      if (k === 'v' || k === 'files' || k === 'git') continue;
      if (Array.isArray(v)) files[k] = v;
    }
    const migrated = { v: 2, files, git: [] };
    try { localStorage.setItem(LS_KEY, JSON.stringify(migrated)); } catch (_) {}
    return migrated;
  }
  function lsSave(data) {
    try { localStorage.setItem(LS_KEY, JSON.stringify(data)); } catch (_) {}
  }
  function lsAddTab(gitRoot, filesRoot) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!data.files[gitRoot]) data.files[gitRoot] = [];
    // 同じ root が既にある場合は重複しない
    if (!data.files[gitRoot].some(e => e.root === filesRoot)) {
      data.files[gitRoot].push({ root: filesRoot, openedFile: null });
    }
    lsSave(data);
  }
  function lsRemoveTab(gitRoot, filesRoot) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!data.files[gitRoot]) return;
    data.files[gitRoot] = data.files[gitRoot].filter(e => e.root !== filesRoot);
    if (data.files[gitRoot].length === 0) delete data.files[gitRoot];
    lsSave(data);
  }
  /** ファイル選択時に呼ぶ用の公開関数 */
  function lsUpdateOpenedFile(gitRoot, filesRoot, filePath) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!data.files[gitRoot]) return;
    const entry = data.files[gitRoot].find(e => e.root === filesRoot);
    if (entry) { entry.openedFile = filePath; lsSave(data); }
  }

  // ─── git タブ用 LS ─────────────────────────────────────────────────────
  function lsAddGitTab(gitRoot, projectName, viewRef) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!Array.isArray(data.git)) data.git = [];
    const existing = data.git.find(e => e.gitRoot === gitRoot);
    if (existing) {
      existing.projectName = projectName || existing.projectName;
      existing.viewRef = viewRef || existing.viewRef || '';
    } else {
      data.git.push({ gitRoot, projectName: projectName || '', viewRef: viewRef || '' });
    }
    lsSave(data);
  }
  function lsRemoveGitTab(gitRoot) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!Array.isArray(data.git)) return;
    data.git = data.git.filter(e => e.gitRoot !== gitRoot);
    lsSave(data);
  }
  function lsUpdateGitViewRef(gitRoot, viewRef) {
    if (!gitRoot) return;
    const data = lsLoad();
    if (!Array.isArray(data.git)) return;
    const entry = data.git.find(e => e.gitRoot === gitRoot);
    if (entry) { entry.viewRef = viewRef || ''; lsSave(data); }
  }

  // ─── DOM ──────────────────────────────────────────────────────────────
  function makeid() { return 'dtab-' + Math.random().toString(36).slice(2, 9); }
  function currentSessionId() {
    return (typeof activeSessionId !== 'undefined') ? activeSessionId : null;
  }
  function sameSessionId(a, b) {
    if (a == null || b == null) return a == null && b == null;
    return String(a) === String(b);
  }
  function sessionTabPrefix(sessionId) {
    return sessionId != null ? `#${sessionId} ` : '';
  }
  function updateTabLabelPrefix(tab, newSid) {
    if (!tab || !tab.labelEl) return;
    const cur = tab.labelEl.textContent || '';
    const stripped = cur.replace(/^#\d+\s+/, '');
    tab.labelEl.textContent = sessionTabPrefix(newSid) + stripped;
  }
  // タブの可視性判定:
  // - 同一セッションのタブは常に可視
  // - プロジェクトキーが取れる場合は、現セッションと同じプロジェクトに属するタブも可視
  //   （プロジェクト単位で files/git タブを共有するため）
  // - プロジェクトが '__no_project__' / 空 のセッション同士はプロジェクト共有しない
  function isVisibleInCurrentSession(tab) {
    const curSid = currentSessionId();
    if (sameSessionId(tab.sessionId, curSid)) return true;
    const tabPk = tab.projectKey;
    if (!tabPk || tabPk === '__no_project__') return false;
    if (curSid == null || !sessionsRef) return false;
    const curSess = sessionsRef.get(curSid);
    if (!curSess) return false;
    const curPk = curSess.project || '';
    if (!curPk || curPk === '__no_project__') return false;
    return tabPk === curPk;
  }
  function refreshVisibleTabs() {
    tabs.forEach(tab => {
      const visible = isVisibleInCurrentSession(tab);
      if (tab.el) tab.el.hidden = !visible;
      if (tab.contentEl && !visible) tab.contentEl.classList.remove('active');
    });
    const active = tabs.find(tab => tab.id === activeTabId);
    if (active && !isVisibleInCurrentSession(active)) {
      activeTabId = 'session';
    }
  }

  function ensureSessionTab() {
    if (sessionTabEl) return;
    sessionTabEl = document.createElement('button');
    sessionTabEl.className = 'main-tab main-tab-session active';
    sessionTabEl.dataset.tabId = 'session';
    sessionTabEl.textContent = t('files_tab_session_label');
    sessionTabEl.addEventListener('click', () => switchToSessionView());
    tabList.insertBefore(sessionTabEl, tabList.firstChild);
    ensureAddTabButton();
    activeTabId = 'session';
  }

  function placeAddTabButtonAfterSessionTab() {
    if (!addTabBtn || !sessionTabEl || sessionTabEl.parentNode !== tabList) return;
    if (addTabBtn.parentNode !== tabList || addTabBtn.previousSibling !== sessionTabEl) {
      tabList.insertBefore(addTabBtn, sessionTabEl.nextSibling);
    }
  }

  function showTabBar() {
    tabBar.style.display = '';
  }

  function setActive(tabId) {
    refreshVisibleTabs();
    const targetTab = tabs.find(tab => tab.id === tabId);
    if (targetTab && !isVisibleInCurrentSession(targetTab)) {
      tabId = 'session';
    }
    activeTabId = tabId;
    // タブボタン
    tabList.querySelectorAll('.main-tab').forEach(el => {
      el.classList.toggle('active', el.dataset.tabId === tabId);
    });
    // コンテンツ
    const isSession = (tabId === 'session');
    // C2: 統合タブバー導入後は display-area mode クラスで visibility を制御するため、
    // terminalWrapper.style.display は触らず、常に '' に保つ。
    terminalWrapper.style.display = '';
    if (isSession) {
      filesContents.classList.remove('visible');
      filesContents.querySelectorAll('.files-tab-content, .git-tab-content').forEach(el => el.classList.remove('active'));
      // display:none で隠れていた間に ResizeObserver が 0 幅で fit() を呼んでいる可能性があるため、
      // 表示復帰後にレイアウト確定を待って refit する。これをしないと xterm の cols が極小のまま残り、
      // 文字が縦に細く折り返される（depth padding 修正と同系統の "MD で出した narrow 表示" 事象）。
      if (typeof refitActiveTerminalAfterLayout === 'function') {
        refitActiveTerminalAfterLayout(true);
      }
    } else {
      filesContents.classList.add('visible');
      filesContents.querySelectorAll('.files-tab-content, .git-tab-content').forEach(el => {
        el.classList.toggle('active', el.dataset.tabId === tabId);
      });
      // C2: 外部から openFilesTab / openGitTab が呼ばれた場合は統合タブバーも追随
      if (targetTab) {
        const kind = targetTab.kind || targetTab.type;
        if (kind === 'files' || kind === 'git') {
          if (typeof window !== 'undefined' && typeof window.setActiveTab === 'function') {
            const sid = (typeof activeSessionId !== 'undefined') ? activeSessionId : null;
            if (sid !== null && sid !== undefined) {
              if (typeof markTabLazyLoaded === 'function') markTabLazyLoaded(sid, kind);
              if (typeof refreshLazyTabClasses === 'function') refreshLazyTabClasses(sid);
              window.setActiveTab(sid, kind);
            }
          }
        }
      }
    }
  }

  // ─── 公開 API ─────────────────────────────────────────────────────────

  function openFilesTab(sessionId, projectKey, filesRoot, gitRoot) {
    ensureSessionTab();
    showTabBar();
    ensureAddTabButton();

    // 同じ root のタブが既にあればそちらをアクティブに。
    // プロジェクト共有: 同じ projectKey（__no_project__ 除く）& 同じ filesRoot を再利用。
    // プロジェクト不明セッションは従来通り sessionId 一致のみ。
    const hasProject = projectKey && projectKey !== '__no_project__';
    const existing = tabs.find(t => {
      if ((t.kind || t.type) !== 'files') return false;
      if (t.filesRoot !== filesRoot) return false;
      if (hasProject) return t.projectKey === projectKey;
      return sameSessionId(t.sessionId, sessionId);
    });
    if (existing) {
      // 別セッションで作られたタブをアクティブセッションへ付け替え（API 呼び出しが現セッションに紐づくよう）
      if (sessionId != null && !sameSessionId(existing.sessionId, sessionId)) {
        existing.sessionId = sessionId;
        if (existing.contentEl) existing.contentEl.dataset.sessionId = String(sessionId);
        updateTabLabelPrefix(existing, sessionId);
      }
      setActive(existing.id);
      return existing.id;
    }

    const id = makeid();
    const displayName = projectKey && projectKey !== '__no_project__' ? projectKey : filesRoot;
    // タブラベルは通常 "📁 <projectName>/<開いたディレクトリの basename>"。
    // プロジェクト直下を開いた場合は projectName と basename が同じなので片方だけ表示する。
    const rootBase = (filesRoot || '').replace(/[\\/]+$/, '').split(/[\\/]/).pop() || filesRoot || '';
    const sameDisplay = String(displayName).toLowerCase() === String(rootBase).toLowerCase();
    const label = sessionTabPrefix(sessionId) + 'Files: ' + (sameDisplay ? displayName : displayName + '/' + rootBase);

    // タブボタン DOM
    const tabBtn = document.createElement('button');
    tabBtn.className = 'main-tab';
    tabBtn.dataset.tabId = id;

    const labelSpan = document.createElement('span');
    labelSpan.dataset.tabLabel = '1';
    labelSpan.textContent = label;
    tabBtn.appendChild(labelSpan);

    const closeBtn = document.createElement('button');
    closeBtn.className = 'main-tab-close';
    closeBtn.textContent = '×';
    closeBtn.title = t('files_tab_close_tooltip');
    closeBtn.addEventListener('click', (e) => { e.stopPropagation(); closeFilesTab(id); });
    tabBtn.appendChild(closeBtn);

    tabBtn.addEventListener('click', () => setActive(id));
    tabList.appendChild(tabBtn);

    // コンテンツ DOM（2ペインスケルトン）
    const contentEl = document.createElement('div');
    contentEl.className = 'files-tab-content';
    contentEl.dataset.tabId = id;
    contentEl.dataset.filesRoot = filesRoot;
    contentEl.dataset.sessionId = sessionId || '';
    contentEl.dataset.gitRoot = gitRoot || '';
    contentEl.dataset.projectKey = projectKey || '';
    contentEl.innerHTML = `
      <div class="files-tab-placeholder">
        <div class="files-tab-toolbar" data-files-toolbar="${id}"></div>
        <div class="files-tab-panes">
          <div class="files-tab-tree-pane" data-files-tree="${id}">${escapeHtml(t('files_tab_loading'))}</div>
          <div class="files-tab-tree-resizer" data-files-tree-resizer="${id}"></div>
          <div class="files-tab-preview-pane" data-files-preview="${id}">${escapeHtml(t('files_tab_loading'))}</div>
        </div>
      </div>
    `;
    filesContents.appendChild(contentEl);

    // ツリーペイン幅をリストア + リサイザー配線
    setupFilesTreeResizer(contentEl);

    const tabObj = { id, kind: 'files', type: 'files', label, sessionId, filesRoot, gitRoot, projectKey, el: tabBtn, contentEl, labelEl: labelSpan };
    tabs.push(tabObj);

    // localStorage に保存
    if (gitRoot) lsAddTab(gitRoot, filesRoot);

    setActive(id);
    notifyTabStateChanged();
    return id;
  }

  const FILES_TREE_WIDTH_KEY = 'any_ai_cli_files_tree_width';
  const FILES_TREE_MIN = 140;
  const FILES_TREE_MAX = 640;

  function getFilesTreeSavedWidth() {
    const v = parseInt(localStorage.getItem(FILES_TREE_WIDTH_KEY), 10);
    if (!isFinite(v)) return null;
    if (v < FILES_TREE_MIN || v > FILES_TREE_MAX) return null;
    return v;
  }

  function setupFilesTreeResizer(contentEl) {
    const pane = contentEl.querySelector('.files-tab-tree-pane');
    const resizer = contentEl.querySelector('.files-tab-tree-resizer');
    if (!pane || !resizer) return;
    const saved = getFilesTreeSavedWidth();
    if (saved != null) pane.style.width = saved + 'px';

    let startX = 0, startW = 0;

    function onMove(e) {
      const cx = e.clientX || (e.touches && e.touches[0] && e.touches[0].clientX) || 0;
      const dx = cx - startX;
      const w = Math.min(FILES_TREE_MAX, Math.max(FILES_TREE_MIN, startW + dx));
      // 全ての files タブのツリーペインを同期
      document.querySelectorAll('.files-tab-tree-pane').forEach(el => { el.style.width = w + 'px'; });
      try { localStorage.setItem(FILES_TREE_WIDTH_KEY, String(w)); } catch (_) {}
    }
    function onUp() {
      resizer.classList.remove('dragging');
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    }
    resizer.addEventListener('mousedown', (e) => {
      e.preventDefault();
      startX = e.clientX;
      startW = pane.getBoundingClientRect().width;
      resizer.classList.add('dragging');
      document.body.style.cursor = 'col-resize';
      document.body.style.userSelect = 'none';
      document.addEventListener('mousemove', onMove);
      document.addEventListener('mouseup', onUp);
    });
  }

  function closeFilesTab(id) {
    // backward-compat alias. kind 別の cleanup は closeMainTab に統合済み。
    return closeMainTab(id);
  }

  function closeMainTab(id) {
    const idx = tabs.findIndex(t => t.id === id);
    if (idx === -1) return;
    const tabObj = tabs[idx];
    // DOM 削除
    try { tabObj.el.remove(); } catch (_) {}
    try { tabObj.contentEl.remove(); } catch (_) {}
    tabs.splice(idx, 1);
    // kind 別の cleanup
    const kind = tabObj.kind || tabObj.type;
    if (kind === 'files') {
      if (tabObj.gitRoot) lsRemoveTab(tabObj.gitRoot, tabObj.filesRoot);
    } else if (kind === 'git') {
      if (tabObj.gitRoot) lsRemoveGitTab(tabObj.gitRoot);
      // GitGraphView インスタンスがあれば dispose
      try {
        if (tabObj.gitView && typeof tabObj.gitView.dispose === 'function') {
          tabObj.gitView.dispose();
        }
      } catch (_) {}
    }
    // アクティブだったら session ビューに戻る
    if (activeTabId === id) {
      switchToSessionView();
    }
    // Files/Git タブが 0 になってもセッションタブと + ボタンは残す
    // （+ ボタンから次の Files/Git タブを開けるようにするため）
    // カードの open マーカー再描画
    notifyTabStateChanged();
  }

  function switchToSessionView() {
    ensureSessionTab();
    showTabBar();
    refreshVisibleTabs();
    setActive('session');
  }

  // ─── + ボタン (タブバー右端) ───────────────────────────────────────────
  let addTabBtn = null;
  function ensureAddTabButton() {
    if (addTabBtn) return;
    addTabBtn = document.createElement('button');
    addTabBtn.className = 'main-tab-add-btn';
    addTabBtn.type = 'button';
    addTabBtn.textContent = '+';
    addTabBtn.title = ti18n('main_tab_add_tooltip', 'Add tab');
    addTabBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      openAddTabMenu(addTabBtn.getBoundingClientRect());
    });
    placeAddTabButtonAfterSessionTab();
  }
  function removeAddTabButton() {
    if (addTabBtn) { try { addTabBtn.remove(); } catch (_) {} addTabBtn = null; }
  }

  // ─── + ボタン → 簡易ドロップダウンメニュー ───────────────────────────
  let addTabMenuEl = null;
  function openAddTabMenu(anchorRect) {
    closeAddTabMenu();
    const menu = document.createElement('div');
    menu.className = 'main-tab-add-menu open';
    const sessId = (typeof activeSessionId !== 'undefined') ? activeSessionId : null;
    menu.innerHTML =
      `<button data-act="add-git" type="button"><span class="ico">⎇</span><span>${escapeHtml(ti18n('add_git_tab', 'Add Git tab'))}</span></button>` +
      `<button data-act="add-files" type="button"><span class="ico">📁</span><span>${escapeHtml(ti18n('add_files_tab', 'Add Files tab'))}</span></button>`;
    document.body.appendChild(menu);
    const r = menu.getBoundingClientRect();
    const x = Math.min(anchorRect.left, window.innerWidth - r.width - 4);
    const y = Math.min(anchorRect.bottom + 2, window.innerHeight - r.height - 4);
    menu.style.left = x + 'px';
    menu.style.top  = y + 'px';
    addTabMenuEl = menu;
    menu.querySelectorAll('button').forEach(b => {
      b.addEventListener('click', (e) => {
        e.stopPropagation();
        const act = b.dataset.act;
        closeAddTabMenu();
        if (sessId == null) return;
        const sess = sessionsRef ? sessionsRef.get(sessId) : null;
        if (!sess) return;
        if (act === 'add-git') {
          const gr = sess.git_root || sess.cwd || '';
          if (!gr) return;
          openGitTab(sessId, gr, sess.branch || '');
        } else if (act === 'add-files') {
          const gr = sess.git_root || sess.cwd || '';
          const pk = sess.project || (gr ? gr.split(/[\\/]/).filter(Boolean).pop() : '__no_project__');
          if (!gr) return;
          openFilesTab(sessId, pk, gr, gr);
        }
      });
    });
  }
  function closeAddTabMenu() {
    if (addTabMenuEl) { try { addTabMenuEl.remove(); } catch (_) {} addTabMenuEl = null; }
  }
  document.addEventListener('mousedown', (e) => {
    if (addTabMenuEl && !e.target.closest('.main-tab-add-menu') && !e.target.closest('.main-tab-add-btn')) {
      closeAddTabMenu();
    }
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && addTabMenuEl) closeAddTabMenu();
  });

  // ─── git タブを開く ────────────────────────────────────────────────────
  function openGitTab(sessionId, gitRoot, branch) {
    if (!gitRoot) return null;
    ensureSessionTab();
    showTabBar();
    ensureAddTabButton();

    // 渡された sessionId からプロジェクトキーを引き、プロジェクト単位でタブを共有する。
    // プロジェクト不明（__no_project__/空）の場合は従来通り sessionId 一致のみ。
    let projectKey = '';
    if (sessionsRef && sessionId != null) {
      const s = sessionsRef.get(sessionId);
      if (s) projectKey = s.project || '';
    }
    const hasProject = projectKey && projectKey !== '__no_project__';

    // 同 gitRoot の git タブがあれば activate + view ref 更新
    const existing = tabs.find(t => {
      if ((t.kind || t.type) !== 'git') return false;
      if (t.gitRoot !== gitRoot) return false;
      if (hasProject) return t.projectKey === projectKey;
      return sameSessionId(t.sessionId, sessionId);
    });
    if (existing) {
      const newRef = branch || existing.viewRef || '';
      existing.viewRef = newRef;
      // セッションが渡されていればそちらに付け替え（ラベル prefix も追従）
      if (sessionId != null && !sameSessionId(existing.sessionId, sessionId)) {
        existing.sessionId = sessionId;
        if (existing.contentEl) existing.contentEl.dataset.sessionId = String(sessionId);
        updateTabLabelPrefix(existing, sessionId);
        try {
          if (existing.gitView && typeof existing.gitView.setSessionId === 'function') {
            existing.gitView.setSessionId(sessionId);
          }
        } catch (_) {}
      } else if (sessionId != null) {
        existing.sessionId = sessionId;
      }
      // タブラベルの ref 部分を更新
      try {
        const refSpan = existing.el.querySelector('.ref');
        if (refSpan) refSpan.textContent = newRef ? `(${newRef})` : '';
      } catch (_) {}
      if (existing.gitRoot) lsUpdateGitViewRef(existing.gitRoot, newRef);
      try {
        if (existing.gitView && typeof existing.gitView.setViewRef === 'function' && newRef) {
          existing.gitView.setViewRef(newRef);
        }
      } catch (_) {}
      setActive(existing.id);
      notifyTabStateChanged();
      return existing.id;
    }

    // 新規作成
    const id = makeid();
    // projectName 推定: project キーがあればそれを使う。無ければ gitRoot の basename
    let projectName = hasProject ? projectKey : '';
    if (!projectName) {
      projectName = (gitRoot || '').replace(/[\\/]+$/, '').split(/[\\/]/).pop() || gitRoot;
    }

    const tabBtn = document.createElement('button');
    tabBtn.className = 'main-tab main-tab-git';
    tabBtn.dataset.tabId = id;
    tabBtn.type = 'button';

    const iconSpan = document.createElement('span');
    iconSpan.className = 'icon';
    iconSpan.textContent = '⎇';
    tabBtn.appendChild(iconSpan);

    const labelSpan = document.createElement('span');
    labelSpan.dataset.tabLabel = '1';
    labelSpan.textContent = sessionTabPrefix(sessionId) + 'Git: ' + projectName + ' ';
    tabBtn.appendChild(labelSpan);

    const refSpan = document.createElement('span');
    refSpan.className = 'ref';
    refSpan.textContent = branch ? `(${branch})` : '';
    tabBtn.appendChild(refSpan);

    const closeBtn = document.createElement('button');
    closeBtn.className = 'main-tab-close';
    closeBtn.type = 'button';
    closeBtn.textContent = '×';
    closeBtn.title = (typeof t === 'function' ? t('files_tab_close_tooltip') : 'Close');
    closeBtn.addEventListener('click', (e) => { e.stopPropagation(); closeMainTab(id); });
    tabBtn.appendChild(closeBtn);

    tabBtn.addEventListener('click', () => setActive(id));

    tabList.appendChild(tabBtn);

    // コンテンツ DOM
    const contentEl = document.createElement('div');
    contentEl.className = 'git-tab-content';
    contentEl.dataset.tabId = id;
    contentEl.dataset.gitRoot = gitRoot;
    contentEl.dataset.sessionId = sessionId != null ? String(sessionId) : '';
    // session 不明時の警告ヘッダ
    const warnHtml = (sessionId == null)
      ? `<div class="git-tab-placeholder-warning" data-git-no-session>${escapeHtml(ti18n('git_tab_no_session_warning', 'session: なし (元セッション削除済)'))}</div>`
      : '';
    contentEl.innerHTML = warnHtml +
      `<div class="git-tab-placeholder-body" data-git-placeholder-body>${escapeHtml(ti18n('git_tab_loading', 'Loading Git view...'))}</div>`;
    filesContents.appendChild(contentEl);

    const tabObj = {
      id, kind: 'git', type: 'git',
      label: 'git: ' + projectName,
      sessionId: sessionId != null ? sessionId : null,
      gitRoot, viewRef: branch || '', projectName,
      projectKey: hasProject ? projectKey : '',
      el: tabBtn, contentEl, labelEl: labelSpan,
      gitView: null,
    };
    tabs.push(tabObj);

    // GitGraphView インスタンス化（クラスが存在する場合のみ）
    try {
      if (typeof window.GitGraphView === 'function' && sessionId != null) {
        // placeholder を消してから GitGraphView をマウント
        const body = contentEl.querySelector('[data-git-placeholder-body]');
        if (body) body.remove();
        tabObj.gitView = new window.GitGraphView(contentEl, {
          sessionId,
          gitRoot,
          viewRef: branch || '',
        });
      }
    } catch (err) {
      console.warn('[FilesTabManager] GitGraphView mount failed:', err);
    }

    // LS 保存
    lsAddGitTab(gitRoot, projectName, branch || '');

    setActive(id);
    notifyTabStateChanged();
    return id;
  }

  // ─── カード open マーカー再描画通知 ───────────────────────────────────
  // 外部で listen する用。renderSessionList を直接呼ぶと循環するため
  // setTimeout で非同期化。
  function notifyTabStateChanged() {
    try {
      if (typeof window.dispatchEvent === 'function') {
        window.dispatchEvent(new CustomEvent('files-tab-state-changed'));
      }
    } catch (_) {}
  }

  // ─── 同 gitRoot のタブ存在チェック（カードマーカー用） ───────────────
  function hasGitTabForRoot(gitRoot) {
    if (!gitRoot) return false;
    const sessionId = arguments.length >= 2 ? arguments[1] : undefined;
    return tabs.some(t => (t.kind || t.type) === 'git' && t.gitRoot === gitRoot && (sessionId === undefined || sameSessionId(t.sessionId, sessionId)));
  }
  function hasFilesTabForRoot(gitRoot) {
    if (!gitRoot) return false;
    const sessionId = arguments.length >= 2 ? arguments[1] : undefined;
    return tabs.some(t => (t.kind || t.type) === 'files' && t.gitRoot === gitRoot && (sessionId === undefined || sameSessionId(t.sessionId, sessionId)));
  }

  // ─── セッション削除イベント: 紐づきタブを別セッションに付け替え ────
  function onSessionRemoved(removedSessionId) {
    if (!sessionsRef) return;
    let mutated = false;
    for (const tab of tabs) {
      if (tab.sessionId !== removedSessionId) continue;
      const kind = tab.kind || tab.type;
      if (kind !== 'git' && kind !== 'files') continue;
      // 同 gitRoot の別セッションを探す
      let candidate = null;
      if (tab.gitRoot && sessionsRef) {
        for (const s of sessionsRef.values()) {
          if (s.id === removedSessionId) continue;
          const gr = s.git_root || s.cwd || '';
          if (gr === tab.gitRoot || (gr && tab.gitRoot && gr.startsWith(tab.gitRoot))) {
            candidate = s;
            break;
          }
        }
      }
      if (candidate) {
        tab.sessionId = candidate.id;
        if (tab.contentEl) tab.contentEl.dataset.sessionId = String(candidate.id);
        updateTabLabelPrefix(tab, candidate.id);
        mutated = true;
        // git タブの警告ヘッダがあれば除去 + GitGraphView の sessionId 同期
        if (kind === 'git') {
          const warn = tab.contentEl.querySelector('[data-git-no-session]');
          if (warn) warn.remove();
          try {
            if (tab.gitView && typeof tab.gitView.setSessionId === 'function') {
              tab.gitView.setSessionId(candidate.id);
            }
          } catch (_) {}
        }
      } else {
        // 付け替え不可
        if (kind === 'git') {
          tab.sessionId = null;
          updateTabLabelPrefix(tab, null);
          if (tab.contentEl) {
            tab.contentEl.dataset.sessionId = '';
            // 警告ヘッダがなければ追加
            if (!tab.contentEl.querySelector('[data-git-no-session]')) {
              const warn = document.createElement('div');
              warn.className = 'git-tab-placeholder-warning';
              warn.setAttribute('data-git-no-session', '');
              warn.textContent = ti18n('git_tab_no_session_warning', 'session: なし (元セッション削除済)');
              tab.contentEl.insertBefore(warn, tab.contentEl.firstChild);
            }
          }
          mutated = true;
        } else if (kind === 'files') {
          // files タブはセッションがないと操作不能なので閉じる
          closeMainTab(tab.id);
        }
      }
    }
    if (mutated) notifyTabStateChanged();
  }

  function updateSessionTabLabel(label) {
    if (sessionTabEl) {
      sessionTabEl.textContent = label || t('files_tab_session_label');
    }
  }

  async function restoreFromLocalStorage() {
    // /api/info からセッション一覧を取得して gitRoot → projectKey のマッピングを試みる
    let sessions = [];
    try {
      const res = await fetch(`/api/info?token=${encodeURIComponent(token)}`);
      if (res.ok) { const d = await res.json(); sessions = d.sessions || []; }
    } catch (err) {
      console.warn('[FilesTabManager] /api/info fetch error:', err);
    }

    const data = lsLoad();
    // ─ git タブ復元（先に開いておくと files タブと順序が安定する）─────
    if (Array.isArray(data.git)) {
      const restorableGitTabs = [];
      for (const entry of data.git) {
        if (!entry || !entry.gitRoot) continue;
        const matched = sessions.find(s => {
          const gr = s.git_root || s.cwd || '';
          return gr === entry.gitRoot || (gr && gr.startsWith(entry.gitRoot));
        });
        if (matched) {
          restorableGitTabs.push(entry);
          openGitTab(matched.id, entry.gitRoot, entry.viewRef || matched.branch || '');
        }
      }
      if (restorableGitTabs.length !== data.git.length) {
        data.git = restorableGitTabs;
        lsSave(data);
      }
    }
    // ─ files タブ復元（既存ロジック維持）─────────────────────────
    const filesMap = data.files || {};
    for (const [gitRoot, entries] of Object.entries(filesMap)) {
      if (!Array.isArray(entries)) continue;
      // gitRoot に対応するセッションを探す（cwd が gitRoot で始まるもの）
      const matchedSession = sessions.find(s => s.cwd && (s.cwd === gitRoot || s.cwd.startsWith(gitRoot)));
      // 対応するセッションが無い場合は復元しない。
      // セッションが無いまま Hub の cwd と localStorage の root が重なると、
      // probe が偶然通って `..` だけのゴーストタブが復元されてしまうため。
      // localStorage は残すので、対象セッションを spawn し直した次回起動で自動復活する。
      if (!matchedSession) {
        console.info('[FilesTabManager] skip restoring tabs for', gitRoot, '(no matching session)');
        continue;
      }
      const sessionId  = matchedSession.id;
      const projectKey = matchedSession.project;

      for (const entry of entries) {
        if (!entry.root) continue;
        // 現在の Hub の許可ルート外なら復元しない（ゾンビタブ防止）。
        // また items が空（.md が一切無い）の場合も復元しない（"死骸タブ" 防止）。
        // 加えて全エントリの rel が `..` で始まる場合（filesRoot がセッション cwd の外側）も復元しない。
        // → tree が `..` 1 個だけで中身を確認できないため。
        try {
          const sessionQs = `&session=${encodeURIComponent(sessionId)}`;
          const probeUrl = `/api/files-list?root=${encodeURIComponent(entry.root)}&token=${encodeURIComponent(token)}${sessionQs}`;
          const probeRes = await fetch(probeUrl);
          if (!probeRes.ok) {
            console.info('[FilesTabManager] skip restoring tab (not accessible by current Hub):', entry.root, probeRes.status);
            continue;
          }
          const probeData = await probeRes.json();
          if (!probeData.exists) {
            console.info('[FilesTabManager] skip restoring tab (root not found):', entry.root);
            continue;
          }
          if (!Array.isArray(probeData.items) || probeData.items.length === 0) {
            console.info('[FilesTabManager] skip restoring tab (no files under root):', entry.root);
            continue;
          }
          const allOutsideCwd = probeData.items.every(it => {
            const rel = String(it && it.rel || '').replace(/\\/g, '/');
            return rel.startsWith('../') || rel === '..';
          });
          if (allOutsideCwd) {
            console.info('[FilesTabManager] skip restoring tab (root outside session cwd):', entry.root);
            continue;
          }
        } catch (err) {
          console.warn('[FilesTabManager] probe error for', entry.root, err);
          continue;
        }
        openFilesTab(sessionId, projectKey || gitRoot, entry.root, gitRoot);
      }
    }
  }

  /**
   * 指定ファイルを Files タブで開く（タブが無ければ作る、あればアクティブ化）。
   * バインド完了を待ってからプレビュー読み込みとツリー選択を呼ぶ。
   * `fileAbsPath` は許可ルート配下の絶対パスを想定。
   */
  function openFilesTabAtFile(sessionId, projectKey, filesRoot, gitRoot, fileAbsPath) {
    const id = openFilesTab(sessionId, projectKey, filesRoot, gitRoot);
    if (!id || !fileAbsPath) return id;
    if (gitRoot) lsUpdateOpenedFile(gitRoot, filesRoot, fileAbsPath);
    const tabObj = tabs.find(t => t.id === id);
    if (!tabObj) return id;

    const relPath = computeRelPath(filesRoot, fileAbsPath);
    let attempt = 0;
    const tryActivate = () => {
      const previewPane = tabObj.contentEl.querySelector('[data-files-preview]');
      const treePane = tabObj.contentEl.querySelector('[data-files-tree]');
      if (previewPane && previewPane._filesPreview) {
        previewPane._filesPreview.loadFile(fileAbsPath, relPath);
        if (treePane && treePane._filesTree && treePane._filesTree.selectFile) {
          treePane._filesTree.selectFile(fileAbsPath);
        }
        // ツリー描画完了後にスクロールイントゥ
        const scrollIntoView = (n = 0) => {
          const fileEl = treePane && treePane.querySelector(`.files-tree-item[data-abs-path="${CSS.escape(fileAbsPath)}"]`);
          if (fileEl) {
            try { fileEl.scrollIntoView({ block: 'center' }); } catch (_) {}
          } else if (n < 6) {
            setTimeout(() => scrollIntoView(n + 1), 250);
          }
        };
        setTimeout(scrollIntoView, 100);
        return;
      }
      if (attempt++ < 20) setTimeout(tryActivate, 100);
    };
    tryActivate();
    return id;
  }

  return {
    openFilesTab,
    openFilesTabAtFile,
    openGitTab,
    closeFilesTab,
    closeMainTab,
    switchToSessionView,
    updateSessionTabLabel,
    restoreFromLocalStorage,
    lsUpdateOpenedFile,
    hasGitTabForRoot,
    hasFilesTabForRoot,
    onSessionRemoved,
    setSessionsRef,
    getSessionsRef,
  };
})();

// Hub 起動時に localStorage からタブ状態を読み込む準備だけ行う。
// Files/Git 本体は統合タブの初回クリックまで復元・fetch しない。
// NOTE: state.js → session-list.js → files-view.js の循環 import により、本モジュールは
// state.js の本体評価より前に評価される。トップレベルで `sessions` を直接読むと TDZ
// (Cannot access 'sessions' before initialization) になるため、評価完了後のマイクロタスク
// に遅延する。sessionsRef の実利用は Files タブ初回クリック以降なので遅延しても影響しない。
queueMicrotask(() => { FilesTabManager.setSessionsRef(sessions); });

// ---- Files tree / preview ----
// ---- v2: Files ビュー本体 ----

/**
 * FilesTreeView — 左ペイン
 * bind(containerEl, { filesRoot, sessionId, gitRoot, onFileSelect })
 * unbind(containerEl)
 */
export const FilesTreeView = (function () {

  // ディレクトリの開閉状態を localStorage に保存する。
  // 既定は「全て折りたたみ」。展開済みのキー（filesRoot + "::" + relPath）のみを Set に持つ。
  const EXPANDED_STORAGE_KEY = 'any_ai_cli_files_tree_expanded';
  function loadExpandedSet() {
    try {
      const raw = localStorage.getItem(EXPANDED_STORAGE_KEY);
      if (!raw) return new Set();
      const arr = JSON.parse(raw);
      return new Set(Array.isArray(arr) ? arr : []);
    } catch (_) { return new Set(); }
  }
  function saveExpandedSet(set) {
    try { localStorage.setItem(EXPANDED_STORAGE_KEY, JSON.stringify(Array.from(set))); } catch (_) {}
  }
  const expandedSet = loadExpandedSet();

  // hover preview の状態は renderTree の外（IIFE スコープ）で管理する。
  // renderTree は再描画のたびに呼ばれるため、ローカル変数にすると古い preview が
  // document.body に残り続け、2つの popup が切り替わる「チカチカ」現象が起きる。
  let hoverPreviewEl = null;
  let hoverPreviewTimer = null;

  function hideHoverPreview() {
    if (hoverPreviewTimer) { clearTimeout(hoverPreviewTimer); hoverPreviewTimer = null; }
    if (hoverPreviewEl) { hoverPreviewEl.remove(); hoverPreviewEl = null; }
  }

  function positionHoverPreview(e) {
    if (!hoverPreviewEl) return;
    const margin = 12;
    const rect = hoverPreviewEl.getBoundingClientRect();
    let left = e.clientX + 14;
    let top = e.clientY + 14;
    if (left + rect.width + margin > window.innerWidth) left = e.clientX - rect.width - 14;
    if (top + rect.height + margin > window.innerHeight) top = window.innerHeight - rect.height - margin;
    hoverPreviewEl.style.left = Math.max(margin, left) + 'px';
    hoverPreviewEl.style.top = Math.max(margin, top) + 'px';
  }

  /** items[] を { name, relPath, absPath, type:'file'|'dir', children:[] } ツリーに変換 */
  // API (/api/files-list) は { path, rel, name, type, size, mtime, summary } のフラットな一覧を返す。
  // 古いレスポンスやファイル由来の親ディレクトリも扱えるよう、存在しない親 dir はここで補完する。
  function buildTree(items) {
    const root = { children: [] };
    const nodes = {};
    // rel（相対パス）でアルファベット昇順ソート
    const sorted = [...items].sort((a, b) => {
      const aRel = a.rel || '';
      const bRel = b.rel || '';
      return aRel.localeCompare(bRel);
    });
    for (const item of sorted) {
      const rawRel = item.rel || '';
      const absPath = item.path || '';
      if (!rawRel) continue;
      // Windows の API レスポンスは '\' 区切り。nodes[] キーを全て '/' 区切りに揃えることで
      // 親 dir 補完キー（'/' 区切り）と自エントリ登録キーの不整合による重複ノードを防ぐ。
      const relPath = rawRel.replace(/\\/g, '/');
      const parts = relPath.split('/');
      // OS のパス区切り文字を absPath から推定（Windows なら \\、それ以外は /）
      const sep = absPath.indexOf('\\') >= 0 ? '\\' : '/';
      const absParts = absPath.split(/[\\/]/);
      let parent = root;
      for (let i = 0; i < parts.length - 1; i++) {
        const key = parts.slice(0, i + 1).join('/');
        if (!nodes[key]) {
          // dir は file より (parts.length - 1 - i) 段浅い。absParts の末尾をその分だけ削れば dir の abs が得られる。
          const dropTail = (parts.length - 1) - i;
          const dirAbsParts = absParts.slice(0, absParts.length - dropTail);
          const dirAbs = dirAbsParts.length > 0 ? dirAbsParts.join(sep) : '';
          const dir = { name: parts[i], relPath: key, absPath: dirAbs, type: 'dir', children: [] };
          nodes[key] = dir;
          parent.children.push(dir);
        }
        parent = nodes[key];
      }
      const itemType = item.type === 'dir' ? 'dir' : 'file';
      if (itemType === 'dir') {
        if (!nodes[relPath]) {
          const node = { name: parts[parts.length - 1], relPath, absPath, type: 'dir', children: [] };
          nodes[relPath] = node;
          parent.children.push(node);
        } else if (!nodes[relPath].absPath && absPath) {
          nodes[relPath].absPath = absPath;
        }
      } else {
        const node = { name: parts[parts.length - 1], relPath, absPath, type: 'file', children: [] };
        nodes[relPath] = node;
        parent.children.push(node);
      }
    }
    // 各階層でディレクトリ先行・名前順に並べ替え
    function sortChildren(n) {
      n.children.sort((a, b) => {
        if (a.type !== b.type) return a.type === 'dir' ? -1 : 1;
        return a.name.localeCompare(b.name);
      });
      for (const c of n.children) {
        if (c.type === 'dir') sortChildren(c);
      }
    }
    sortChildren(root);
    return root;
  }

  function isPreviewable(name) {
    // /api/files-content の許可リストと一致する isTextPath を再利用。
    return typeof isAnyAiCliPreviewable === 'function'
      ? isAnyAiCliPreviewable(name)
      : /\.(md|txt|png|jpe?g|gif|webp|bmp|svg)$/i.test(name);
  }

  function isImagePreviewable(name) {
    return typeof isImagePath === 'function' ? isImagePath(name) : /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(name);
  }

  function isVideoPreviewable(name) {
    return typeof isVideoPath === 'function' ? isVideoPath(name) : /\.(mp4|webm|ogv|mov|m4v)$/i.test(name);
  }

  function renderTree(treeRoot, opts) {
    const { onFileClick, onContextMenu, filterText, filesRoot, sessionId } = opts;
    const filterLower = (filterText || '').toLowerCase();
    const rootKey = filesRoot || '';
    // renderTree 呼び出し時に前回の hover preview を必ず消す（複数残留防止）
    hideHoverPreview();

    function showHoverPreview(node, e) {
      hideHoverPreview();
      if (!node.absPath || (!isImagePreviewable(node.name) && !isVideoPreviewable(node.name))) return;
      hoverPreviewTimer = setTimeout(() => {
        const preview = document.createElement('div');
        preview.className = 'files-image-hover-preview';
        preview.dataset.filesSkipSearch = '1';
        const src = getFilesAssetUrl(node.absPath, sessionId || '');
        if (isVideoPreviewable(node.name)) {
          const video = document.createElement('video');
          video.muted = true;
          video.loop = true;
          video.playsInline = true;
          video.preload = 'metadata';
          video.src = src;
          preview.appendChild(video);
          video.play().catch(() => {});
        } else {
          const img = document.createElement('img');
          img.alt = node.name;
          img.src = src;
          preview.appendChild(img);
        }
        document.body.appendChild(preview);
        hoverPreviewEl = preview;
        positionHoverPreview(e);
      }, 180);
    }

    function nodeMatchesFilter(node) {
      if (!filterLower) return true;
      if (node.name.toLowerCase().includes(filterLower)) return true;
      if (node.type === 'dir' && node.children) {
        return node.children.some(c => nodeMatchesFilter(c));
      }
      return false;
    }

    // ancestorMatch=true なら、祖先 dir 名がフィルタにヒット済みなので
    // 自身および配下は無条件で表示する（dir 名検索時の期待動作）。
    function makeNode(node, depth, ancestorMatch) {
      if (filterLower && !ancestorMatch && !nodeMatchesFilter(node)) return null;

      const item = document.createElement('div');
      item.className = 'files-tree-item';
      item.dataset.type = node.type;
      item.dataset.relPath = node.relPath;
      if (node.absPath) item.dataset.absPath = node.absPath;
      // D&D: absPath が解決できているアイテムのみドラッグ可
      if (node.absPath) item.draggable = true;
      item.style.paddingLeft = (depth * 5 + 2) + 'px';

      const label = document.createElement('span');
      label.className = 'files-tree-label';

      if (node.type === 'dir') {
        item.classList.add('files-tree-dir');
        const arrow = document.createElement('span');
        arrow.className = 'files-tree-arrow';
        arrow.textContent = '▼';
        label.appendChild(arrow);
        const nameSpan = document.createElement('span');
        nameSpan.className = 'files-tree-name';
        nameSpan.textContent = node.name;
        label.appendChild(nameSpan);
        item.appendChild(label);

        const childrenEl = document.createElement('div');
        childrenEl.className = 'files-tree-children';
        // 既定は折りたたみ。localStorage に保存された展開状態を尊重する。
        // 検索フィルタ中はヒット確認のため強制展開（保存状態は変更しない）。
        const expandKey = rootKey + '::' + node.relPath;
        const forceExpand = !!filterLower;
        let expanded = forceExpand || expandedSet.has(expandKey);
        arrow.textContent = expanded ? '▼' : '▶';
        childrenEl.style.display = expanded ? '' : 'none';

        const toggle = () => {
          expanded = !expanded;
          arrow.textContent = expanded ? '▼' : '▶';
          childrenEl.style.display = expanded ? '' : 'none';
          if (!forceExpand) {
            if (expanded) expandedSet.add(expandKey); else expandedSet.delete(expandKey);
            saveExpandedSet(expandedSet);
          }
        };

        item.addEventListener('click', (e) => { e.stopPropagation(); toggle(); });
        item.addEventListener('contextmenu', (e) => { e.preventDefault(); e.stopPropagation(); if (onContextMenu) onContextMenu(e, node); });

        const dirNameMatches = !!filterLower && node.name.toLowerCase().includes(filterLower);
        const childAncestorMatch = ancestorMatch || dirNameMatches;
        const childList = (filterLower && !childAncestorMatch)
          ? node.children.filter(nodeMatchesFilter)
          : node.children;
        for (const child of childList) {
          const childEl = makeNode(child, depth + 1, childAncestorMatch);
          if (childEl) childrenEl.appendChild(childEl);
        }
        item.appendChild(childrenEl);
        return item;
      } else {
        // file
        const icon = document.createElement('span');
        icon.className = 'files-tree-file-icon';
        icon.textContent = isVideoPreviewable(node.name) ? '🎞️' : (isImagePreviewable(node.name) ? '🖼️' : (isPreviewable(node.name) ? '📄' : '📃'));
        label.appendChild(icon);
        const nameSpan = document.createElement('span');
        nameSpan.className = 'files-tree-name';
        nameSpan.textContent = node.name;
        label.appendChild(nameSpan);
        item.appendChild(label);

        if (isPreviewable(node.name)) {
          item.classList.add('files-tree-file--previewable');
          if (isImagePreviewable(node.name) || isVideoPreviewable(node.name)) {
            item.classList.add('files-tree-file--image');
            item.addEventListener('mouseenter', (e) => showHoverPreview(node, e));
            item.addEventListener('mousemove', positionHoverPreview);
            item.addEventListener('mouseleave', hideHoverPreview);
          }
        } else {
          item.classList.add('files-tree-file--other');
        }
        item.addEventListener('click', (e) => { e.stopPropagation(); if (onFileClick) onFileClick(e, node); });
        item.addEventListener('contextmenu', (e) => { e.preventDefault(); e.stopPropagation(); if (onContextMenu) onContextMenu(e, node); });
        return item;
      }
    }

    const container = document.createElement('div');
    container.className = 'files-tree-root';
    for (const child of treeRoot.children) {
      const el = makeNode(child, 0, false);
      if (el) container.appendChild(el);
    }
    return container;
  }

  /** containerEl 内に左ペインを構築 */
  function bind(containerEl, opts) {
    const { filesRoot, sessionId, gitRoot, onFileSelect } = opts;
    containerEl.innerHTML = '';
    containerEl.style.removeProperty('align-items');
    containerEl.style.removeProperty('justify-content');

    // ──── ツールバー ────
    const toolbar = document.createElement('div');
    toolbar.className = 'files-tree-toolbar';

    const reloadBtn = document.createElement('button');
    reloadBtn.className = 'files-tree-toolbar-btn';
    reloadBtn.title = t('files_tree_reload_tooltip') || 'Reload';
    reloadBtn.textContent = '🔄';

    const openFolderBtn = document.createElement('button');
    openFolderBtn.className = 'files-tree-toolbar-btn';
    openFolderBtn.title = t('files_tree_open_folder_tooltip') || 'Open folder in OS';
    openFolderBtn.textContent = '⛶';

    const searchBtn = document.createElement('button');
    searchBtn.className = 'files-tree-toolbar-btn';
    searchBtn.title = t('files_tree_search_tooltip') || 'Search';
    searchBtn.textContent = '🔍';

    const moveBtn = document.createElement('button');
    moveBtn.className = 'files-tree-toolbar-btn';
    moveBtn.title = t('files_tree_move_tooltip') || 'Move selected';
    moveBtn.textContent = '↗';
    moveBtn.disabled = true;

    const newFolderBtn = document.createElement('button');
    newFolderBtn.className = 'files-tree-toolbar-btn';
    newFolderBtn.title = t('files_tree_new_folder_tooltip') || 'Create new folder';
    newFolderBtn.textContent = '📁+';

    const closeTabBtn = document.createElement('button');
    closeTabBtn.className = 'files-tree-toolbar-btn';
    closeTabBtn.title = t('files_tree_close_tab_tooltip') || 'Close tab';
    closeTabBtn.textContent = '×';

    toolbar.appendChild(reloadBtn);
    toolbar.appendChild(openFolderBtn);
    toolbar.appendChild(searchBtn);
    toolbar.appendChild(moveBtn);
    toolbar.appendChild(newFolderBtn);

    // 検索インプット（初期非表示）
    const searchWrap = document.createElement('div');
    searchWrap.className = 'files-tree-search-wrap';
    searchWrap.hidden = true;
    const searchInput = document.createElement('input');
    searchInput.type = 'text';
    searchInput.className = 'files-tree-search-input';
    searchInput.placeholder = t('files_tree_search_placeholder') || 'Filter files…';
    const searchClearBtn = document.createElement('button');
    searchClearBtn.className = 'files-tree-toolbar-btn';
    searchClearBtn.textContent = '×';
    searchWrap.appendChild(searchInput);
    searchWrap.appendChild(searchClearBtn);
    toolbar.appendChild(searchWrap);

    // spacer
    const spacer = document.createElement('span');
    spacer.style.flex = '1';
    toolbar.appendChild(spacer);
    toolbar.appendChild(closeTabBtn);

    containerEl.appendChild(toolbar);

    // ──── ツリー本体 ────
    const treeArea = document.createElement('div');
    treeArea.className = 'files-tree-area';
    containerEl.appendChild(treeArea);

    let currentTree = null;
    let selectedAbsPaths = new Set();
    let lastClickedAbsPath = null;
    let filterText = '';
    // D&D: ドラッグ中のソース一覧と代表 ({ absPath, type })、ドロップ強調先 element
    let draggingSrcs = [];
    let draggingRep = null;
    let hoverDropEl = null;

    function highlightSelected() {
      treeArea.querySelectorAll('.files-tree-item').forEach(el => {
        el.classList.toggle('files-tree-item--selected', selectedAbsPaths.has(el.dataset.absPath || ''));
      });
    }

    // ──── D&D ヘルパー ────
    function clearDropHighlight() {
      if (!hoverDropEl) return;
      if (hoverDropEl === treeArea) {
        treeArea.classList.remove('files-tree-area--drop-target');
      } else {
        hoverDropEl.classList.remove('files-tree-item--drop-target');
      }
      hoverDropEl = null;
    }
    function setDropHighlight(el) {
      if (hoverDropEl === el) return;
      clearDropHighlight();
      if (el === treeArea) {
        treeArea.classList.add('files-tree-area--drop-target');
      } else if (el) {
        el.classList.add('files-tree-item--drop-target');
      }
      hoverDropEl = el || null;
    }
    // パス比較（OS 由来の \ / の差を吸収しつつ、Windows のドライブレターは大文字小文字無視）
    function normalizePath(p) {
      return (p || '').replace(/[\\/]+$/, '');
    }
    function pathsEqualCI(a, b) {
      const na = normalizePath(a);
      const nb = normalizePath(b);
      return na === nb || na.toLowerCase() === nb.toLowerCase();
    }
    function isUnderPath(child, parent) {
      const nc = normalizePath(child);
      const np = normalizePath(parent);
      if (!nc || !np) return false;
      const sep = nc.indexOf('\\') >= 0 || np.indexOf('\\') >= 0 ? '\\' : '/';
      return nc.toLowerCase().startsWith(np.toLowerCase() + sep);
    }
    function dirnameOf(p) {
      const n = normalizePath(p);
      const idx = Math.max(n.lastIndexOf('\\'), n.lastIndexOf('/'));
      return idx >= 0 ? n.slice(0, idx) : n;
    }
    // 与えられた dragover/drop イベントから「ドロップ先ディレクトリの絶対パス」と該当 element を解決する。
    // 戻り値: { el, dirAbs } or null（無効なドロップ先）
    function resolveDropTarget(e) {
      if (!draggingRep) return null;
      const dirItem = e.target.closest && e.target.closest('.files-tree-dir');
      if (dirItem) {
        const dirAbs = dirItem.dataset.absPath || '';
        if (!dirAbs) return null;
        // 自身への移動禁止
        if (pathsEqualCI(dirAbs, draggingRep.absPath)) return null;
        // dir をその子孫に入れる移動禁止
        if (draggingRep.type === 'dir' && isUnderPath(dirAbs, draggingRep.absPath)) return null;
        // 単一 src のときのみ同一親への no-op を拒否
        if (draggingSrcs.length === 1 && pathsEqualCI(dirAbs, dirnameOf(draggingRep.absPath))) return null;
        return { el: dirItem, dirAbs };
      }
      // ディレクトリ要素外（ツリーのルート領域）に落とした場合は filesRoot へ
      if (!filesRoot) return null;
      if (draggingSrcs.length === 1 && pathsEqualCI(filesRoot, dirnameOf(draggingRep.absPath))) return null;
      if (draggingRep.type === 'dir' && (pathsEqualCI(filesRoot, draggingRep.absPath) || isUnderPath(filesRoot, draggingRep.absPath))) return null;
      return { el: treeArea, dirAbs: filesRoot };
    }

    // ──── D&D ハンドラ（イベント委譲） ────
    treeArea.addEventListener('dragstart', (e) => {
      const item = e.target.closest && e.target.closest('.files-tree-item');
      if (!item || !item.dataset.absPath) { return; }
      const srcAbsPath = item.dataset.absPath;
      const srcType = item.dataset.type || 'file';
      draggingRep = { absPath: srcAbsPath, type: srcType };
      draggingSrcs = selectedAbsPaths.has(srcAbsPath) ? [...selectedAbsPaths] : [srcAbsPath];
      item.classList.add('files-tree-item--dragging');
      try {
        e.dataTransfer.effectAllowed = 'move';
        e.dataTransfer.setData('text/plain', srcAbsPath);
      } catch (_) {}
    });
    treeArea.addEventListener('dragend', (e) => {
      const item = e.target.closest && e.target.closest('.files-tree-item');
      if (item) item.classList.remove('files-tree-item--dragging');
      draggingSrcs = [];
      draggingRep = null;
      clearDropHighlight();
    });
    treeArea.addEventListener('dragover', (e) => {
      const target = resolveDropTarget(e);
      if (!target) { clearDropHighlight(); return; }
      e.preventDefault();
      try { e.dataTransfer.dropEffect = 'move'; } catch (_) {}
      setDropHighlight(target.el);
    });
    treeArea.addEventListener('dragleave', (e) => {
      // ツリー全体の外に出たら強調を消す
      if (e.target === treeArea && !treeArea.contains(e.relatedTarget)) {
        clearDropHighlight();
      }
    });
    // C2: 多ファイル移動の共通ロジック。失敗メッセージ配列を返す（空なら全件成功）。
    async function moveFiles(srcs, dstDir) {
      const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
      const url = `/api/files-move?token=${encodeURIComponent(token)}${sessionQs}`;
      const res = await fetch(url, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ srcs, dstDir }),
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        return [data.detail || data.error || `HTTP ${res.status}`];
      }
      const data = await res.json();
      const errors = [];
      const moveResults = Array.isArray(data.results) ? data.results : [];
      if (!data || !data.ok) {
        for (const r of moveResults) {
          if (r && r.error) errors.push((r.src || '?') + ': ' + r.error);
        }
        if (errors.length === 0) errors.push((data && (data.detail || data.error)) || '?');
        return errors;
      }
      for (let i = 0; i < moveResults.length; i++) {
        const r = moveResults[i];
        if (r && r.newAbs && selectedAbsPaths.has(r.src || srcs[i])) {
          selectedAbsPaths.delete(r.src || srcs[i]);
          selectedAbsPaths.add(r.newAbs);
        }
      }
      if (moveResults.length === 0 && data.newAbs && srcs.length === 1 && selectedAbsPaths.has(srcs[0])) {
        selectedAbsPaths.delete(srcs[0]);
        selectedAbsPaths.add(data.newAbs);
      }
      return errors;
    }

    treeArea.addEventListener('drop', async (e) => {
      const target = resolveDropTarget(e);
      clearDropHighlight();
      if (!target || draggingSrcs.length === 0) return;
      e.preventDefault();
      const srcs = [...draggingSrcs];
      const dstDir = target.dirAbs;
      draggingSrcs = [];
      draggingRep = null;
      try {
        const errors = await moveFiles(srcs, dstDir);
        if (errors.length > 0) {
          alert((t('files_tree_move_failed') || 'Move failed') + ':\n' + errors.join('\n'));
        }
        await loadTree();
      } catch (err) {
        alert((t('files_tree_move_failed') || 'Move failed') + ': ' + String(err));
      }
    });

    function renderAndMount(tree, filter) {
      const el = renderTree(tree, {
        filterText: filter,
        filesRoot,
        sessionId,
        onFileClick: (e, node) => {
          const isCtrl = e.ctrlKey || e.metaKey;
          const isShift = e.shiftKey;
          if (!isCtrl && !isShift) {
            selectedAbsPaths.clear();
            if (node.absPath) selectedAbsPaths.add(node.absPath);
            lastClickedAbsPath = node.absPath || null;
            highlightSelected();
            updateMoveBtn();
            if (isPreviewable(node.name) && onFileSelect) onFileSelect(node);
          } else if (isCtrl) {
            if (node.absPath) {
              if (selectedAbsPaths.has(node.absPath)) selectedAbsPaths.delete(node.absPath);
              else selectedAbsPaths.add(node.absPath);
            }
            lastClickedAbsPath = node.absPath || null;
            highlightSelected();
            updateMoveBtn();
          } else {
            if (node.absPath) {
              const allFileEls = Array.from(treeArea.querySelectorAll('.files-tree-item[data-type="file"]'));
              const allPaths = allFileEls.map(el => el.dataset.absPath || '').filter(Boolean);
              const lastIdx = lastClickedAbsPath ? allPaths.indexOf(lastClickedAbsPath) : -1;
              const curIdx = allPaths.indexOf(node.absPath);
              if (lastIdx >= 0 && curIdx >= 0) {
                const lo = Math.min(lastIdx, curIdx);
                const hi = Math.max(lastIdx, curIdx);
                for (let i = lo; i <= hi; i++) {
                  if (allPaths[i]) selectedAbsPaths.add(allPaths[i]);
                }
              } else {
                selectedAbsPaths.add(node.absPath);
              }
              lastClickedAbsPath = node.absPath;
              highlightSelected();
              updateMoveBtn();
            }
          }
        },
        onContextMenu: (e, node) => {
          if (node.absPath) showPathPopup(node.absPath, e.clientX, e.clientY, sessionId || '', node.type || 'file');
        },
      });
      treeArea.innerHTML = '';
      treeArea.appendChild(el);
      highlightSelected();
    }

    async function loadTree() {
      treeArea.innerHTML = `<div class="files-tree-loading">${escapeHtml(t('files_tab_loading') || 'Loading…')}</div>`;
      try {
        const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
        const url = `/api/files-list?root=${encodeURIComponent(filesRoot)}&token=${encodeURIComponent(token)}${sessionQs}`;
        const res = await fetch(url);
        if (!res.ok) { treeArea.innerHTML = `<div class="files-tree-error">${escapeHtml('HTTP ' + res.status)} — ${escapeHtml(filesRoot)}</div>`; return; }
        const data = await res.json();
        if (!data.exists) { treeArea.innerHTML = `<div class="files-tree-error">${escapeHtml(t('files_tree_not_found') || 'Directory not found')} — ${escapeHtml(filesRoot)}</div>`; return; }
        const items = data.items || [];
        if (items.length === 0) {
          treeArea.innerHTML = `<div class="files-tree-error">${escapeHtml(t('files_tree_empty') || 'No files found')}<br><small>${escapeHtml(filesRoot)}</small></div>`;
          currentTree = buildTree(items);
          return;
        }
        currentTree = buildTree(items);
        renderAndMount(currentTree, filterText);
      } catch (err) {
        treeArea.innerHTML = `<div class="files-tree-error">${escapeHtml(String(err))}</div>`;
      }
    }

    function updateMoveBtn() {
      moveBtn.disabled = selectedAbsPaths.size === 0;
    }

    function openMoveDialog() {
      if (!currentTree || selectedAbsPaths.size === 0) return;
      const dialog = document.createElement('dialog');
      dialog.className = 'files-move-dialog';

      const titleEl = document.createElement('p');
      titleEl.className = 'files-move-dialog-title';
      titleEl.textContent = (t('files_move_dialog_title') || 'Move {n} item(s) to folder…')
        .replace('{n}', String(selectedAbsPaths.size));
      dialog.appendChild(titleEl);

      const listEl = document.createElement('div');
      listEl.className = 'files-move-dialog-list';
      let dialogSelectedDir = null;
      let confirmBtn;

      function makeDialogDir(node, depth) {
        const item = document.createElement('div');
        item.className = 'files-move-dialog-dir';
        item.style.paddingLeft = (depth * 14 + 6) + 'px';
        item.textContent = '📁 ' + node.name;
        if (node.absPath) {
          item.dataset.absPath = node.absPath;
          item.addEventListener('click', () => {
            listEl.querySelectorAll('.files-move-dialog-dir--selected')
              .forEach(el => el.classList.remove('files-move-dialog-dir--selected'));
            item.classList.add('files-move-dialog-dir--selected');
            dialogSelectedDir = node.absPath;
            if (confirmBtn) confirmBtn.disabled = false;
          });
        }
        listEl.appendChild(item);
        for (const child of node.children || []) {
          if (child.type === 'dir') makeDialogDir(child, depth + 1);
        }
      }

      for (const child of currentTree.children) {
        if (child.type === 'dir') makeDialogDir(child, 0);
      }
      dialog.appendChild(listEl);

      const btnRow = document.createElement('div');
      btnRow.className = 'files-move-dialog-buttons';
      const cancelBtn = document.createElement('button');
      cancelBtn.textContent = t('files_move_dialog_cancel') || 'Cancel';
      cancelBtn.addEventListener('click', () => dialog.close());
      confirmBtn = document.createElement('button');
      confirmBtn.textContent = t('files_move_dialog_confirm') || 'Move';
      confirmBtn.className = 'files-move-dialog-confirm';
      confirmBtn.disabled = true;
      confirmBtn.addEventListener('click', async () => {
        if (!dialogSelectedDir) return;
        const srcs = [...selectedAbsPaths];
        dialog.close();
        try {
          const errors = await moveFiles(srcs, dialogSelectedDir);
          if (errors.length > 0) {
            alert((t('files_tree_move_failed') || 'Move failed') + ':\n' + errors.join('\n'));
          }
          selectedAbsPaths.clear();
          updateMoveBtn();
          highlightSelected();
          await loadTree();
        } catch (err) {
          alert((t('files_tree_move_failed') || 'Move failed') + ': ' + String(err));
        }
      });
      btnRow.appendChild(cancelBtn);
      btnRow.appendChild(confirmBtn);
      dialog.appendChild(btnRow);

      document.body.appendChild(dialog);
      dialog.showModal();
      dialog.addEventListener('close', () => dialog.remove());
    }

    moveBtn.addEventListener('click', () => openMoveDialog());

    newFolderBtn.addEventListener('click', async () => {
      const name = window.prompt(t('files_tree_new_folder_prompt') || 'Enter new folder name');
      if (name == null) return;
      const trimmed = name.trim();
      if (!trimmed) return;
      try {
        const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
        const url = `/api/files-mkdir?token=${encodeURIComponent(token)}${sessionQs}`;
        const res = await fetch(url, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ dir: filesRoot, name: trimmed }),
        });
        const data = await res.json().catch(() => ({}));
        if (res.ok && data.ok) {
          await loadTree();
          return;
        }
        const errCode = data && data.error;
        let errLabel;
        if (errCode === 'already_exists') {
          errLabel = t('files_tree_new_folder_error_already_exists') || 'A file or folder with that name already exists';
        } else if (errCode === 'bad_request') {
          errLabel = t('files_tree_new_folder_error_bad_request') || 'Invalid folder name';
        } else if (errCode === 'forbidden') {
          errLabel = t('files_tree_new_folder_error_forbidden') || 'Creating folders here is not allowed';
        } else {
          errLabel = (data && (data.detail || data.error)) || ('HTTP ' + res.status);
        }
        showToast(`${t('files_tree_new_folder_failed') || 'Failed to create folder'}: ${errLabel}`);
      } catch (err) {
        showToast(`${t('files_tree_new_folder_failed') || 'Failed to create folder'}: ${String(err)}`);
      }
    });

    reloadBtn.addEventListener('click', () => loadTree());

    openFolderBtn.addEventListener('click', () => {
      callOpenApi('/api/open-folder', filesRoot, 'link_open_error', sessionId);
    });

    searchBtn.addEventListener('click', () => {
      searchWrap.hidden = !searchWrap.hidden;
      if (!searchWrap.hidden) { searchInput.focus(); }
      else { searchInput.value = ''; filterText = ''; if (currentTree) renderAndMount(currentTree, ''); }
    });

    let _searchDebounceTimer = null;
    searchInput.addEventListener('input', () => {
      filterText = searchInput.value.trim();
      if (_searchDebounceTimer) clearTimeout(_searchDebounceTimer);
      _searchDebounceTimer = setTimeout(() => {
        _searchDebounceTimer = null;
        if (currentTree) renderAndMount(currentTree, filterText);
      }, 150);
    });

    searchClearBtn.addEventListener('click', () => {
      searchInput.value = '';
      filterText = '';
      searchWrap.hidden = true;
      if (currentTree) renderAndMount(currentTree, '');
    });

    // タブを閉じる — タブIDは contentEl の data-tab-id から
    closeTabBtn.addEventListener('click', () => {
      const contentEl = containerEl.closest('.files-tab-content');
      if (contentEl && contentEl.dataset.tabId) {
        FilesTabManager.closeFilesTab(contentEl.dataset.tabId);
      }
    });

    // ファイル変更（rename 等）を受けてツリーを再読み込み
    const filesChangedHandler = (e) => {
      const detail = (e && e.detail) || {};
      if (detail.kind === 'rename' && detail.oldAbs && detail.newAbs
          && selectedAbsPaths.has(detail.oldAbs)) {
        selectedAbsPaths.delete(detail.oldAbs);
        selectedAbsPaths.add(detail.newAbs);
      }
      loadTree();
    };
    window.addEventListener('any-ai-cli:files-changed', filesChangedHandler);

    // 初回ロード
    loadTree();

    // 外部から "ファイルを選択済み状態にする" 用
    containerEl._filesTree = {
      selectFile: (absPath) => {
        selectedAbsPaths.clear();
        if (absPath) selectedAbsPaths.add(absPath);
        lastClickedAbsPath = absPath || null;
        highlightSelected();
      },
    };
    containerEl._filesTreeCleanup = () => {
      window.removeEventListener('any-ai-cli:files-changed', filesChangedHandler);
    };
  }

  function unbind(containerEl) {
    if (typeof containerEl._filesTreeCleanup === 'function') {
      try { containerEl._filesTreeCleanup(); } catch (_) {}
    }
    delete containerEl._filesTreeCleanup;
    containerEl.innerHTML = '';
    delete containerEl._filesTree;
  }

  return { bind, unbind };
})();

/**
 * FilesPreview — 右ペイン
 * bind(containerEl, { sessionId, gitRoot, filesRoot })
 * showFile(containerEl, { absPath, relPath })
 * unbind(containerEl)
 */
export const FilesPreview = (function () {

  /** ファイル拡張子 → highlight.js 言語名のマップ（小文字で照合） */
  function detectHljsLangFromPath(absPath) {
    const m = /\.([A-Za-z0-9_+-]+)$/.exec(absPath || '');
    if (!m) return '';
    const ext = m[1].toLowerCase();
    const map = {
      js: 'javascript', mjs: 'javascript', cjs: 'javascript',
      ts: 'typescript', tsx: 'typescript', jsx: 'javascript',
      go: 'go', py: 'python', rb: 'ruby', rs: 'rust',
      html: 'xml', htm: 'xml', xml: 'xml', svg: 'xml',
      css: 'css', scss: 'scss', less: 'less',
      json: 'json', yaml: 'yaml', yml: 'yaml', toml: 'ini', ini: 'ini',
      sh: 'bash', bash: 'bash', zsh: 'bash', ps1: 'powershell',
      c: 'c', h: 'c', cpp: 'cpp', hpp: 'cpp', cc: 'cpp', cs: 'csharp',
      java: 'java', kt: 'kotlin', swift: 'swift',
      sql: 'sql', dockerfile: 'dockerfile', makefile: 'makefile',
      md: 'markdown',
    };
    return map[ext] || '';
  }

  /**
   * ソースコード文字列を hljs で色付けして <pre><code class="hljs"> を返す。
   * - 拡張子 → 言語マップで明示判定し、未対応なら highlightAuto にフォールバック
   * - 巨大ファイル（>=200KB）は重いので自動判定をスキップしプレーン表示
   * - hljs 未ロード時 / 例外時もプレーンで安全に表示する
   */
  function renderSourceToPre(content, absPath) {
    const pre = document.createElement('pre');
    pre.className = 'files-preview-raw';
    const code = document.createElement('code');
    const lang = detectHljsLangFromPath(absPath);
    let html = '';
    if (typeof hljs !== 'undefined') {
      try {
        if (lang && hljs.getLanguage(lang)) {
          html = hljs.highlight(content, { language: lang, ignoreIllegals: true }).value;
          code.className = 'hljs language-' + lang;
        } else if (content.length < 200000) {
          const auto = hljs.highlightAuto(content);
          html = auto.value;
          code.className = 'hljs' + (auto.language ? ' language-' + auto.language : '');
        } else {
          html = escapeHtml(content);
          code.className = 'hljs';
        }
      } catch (_) {
        html = escapeHtml(content);
        code.className = 'hljs';
      }
    } else {
      html = escapeHtml(content);
    }
    code.innerHTML = html;
    pre.appendChild(code);
    return pre;
  }

  let markedBaseConfigured = false;
  function ensureMarkedBaseConfigured() {
    if (markedBaseConfigured || typeof marked === 'undefined' || typeof marked.use !== 'function') return;
    marked.use({ gfm: true, breaks: false });
    markedBaseConfigured = true;
  }

  /** marked で Markdown → HTML 変換し、DOMPurify でサニタイズ */
  function renderMarkdown(content, baseDir, onLinkClick) {
    if (typeof marked === 'undefined') return `<pre>${escapeHtml(content)}</pre>`;
    ensureMarkedBaseConfigured();

    // marked の renderer をカスタマイズしてリンク処理を差し替え
    const renderer = new marked.Renderer();
    renderer.link = (href, title, text) => {
      if (href && typeof href === 'object') {
        const token = href;
        href = token.href || '';
        title = token.title || '';
        text = token.tokens ? token.tokens.map(t => t.raw || '').join('') : (token.text || href || '');
      }
      text = text || href || '';
      const safeHref = escapeHtml(href || '');
      const safeText = escapeHtml(text);
      const safeTitle = title ? ` title="${escapeHtml(title)}"` : '';

      if (/^https?:\/\//i.test(href)) {
        return `<a href="${safeHref}" target="_blank" rel="noopener noreferrer"${safeTitle}>${safeText}</a>`;
      }
      if (/\.(md|txt)$/i.test(href) && !/^https?:\/\//i.test(href) && !href.startsWith('/')) {
        // 相対 md リンク → data 属性で処理
        return `<a href="#" data-files-rel-link="${safeHref}" class="files-md-link"${safeTitle}>${safeText}</a>`;
      }
      // その他（絶対OSパス等）→ data 属性
      return `<a href="#" data-files-path-link="${safeHref}" class="files-md-link"${safeTitle}>${safeText}</a>`;
    };

    // ```lang ... ``` → highlight.js で色付け（hljs 未ロード時はプレーン表示にフォールバック）
    renderer.code = function (codeObj, infoArg) {
      let code = '';
      let lang = '';
      if (codeObj && typeof codeObj === 'object') {
        code = codeObj.text != null ? codeObj.text : '';
        lang = (codeObj.lang || '').trim().split(/\s+/)[0] || '';
      } else {
        code = codeObj || '';
        lang = (infoArg || '').trim().split(/\s+/)[0] || '';
      }
      let body = '';
      let langClass = '';
      if (typeof hljs !== 'undefined') {
        try {
          if (lang && hljs.getLanguage(lang)) {
            body = hljs.highlight(code, { language: lang, ignoreIllegals: true }).value;
            langClass = ' language-' + lang;
          } else if (lang) {
            // 未知の言語名 → エスケープのみ
            body = escapeHtml(code);
            langClass = ' language-' + lang;
          } else {
            // 言語指定なし → 自動判定（短いスニペットでは外しがちなので保険）
            const auto = hljs.highlightAuto(code);
            body = auto.value;
            if (auto.language) langClass = ' language-' + auto.language;
          }
        } catch (_) {
          body = escapeHtml(code);
        }
      } else {
        body = escapeHtml(code);
      }
      return `<pre><code class="hljs${langClass}">${body}</code></pre>`;
    };

    let html;
    try { html = marked.parse(content, { renderer, gfm: true, breaks: false }); } catch (_) { return `<pre>${escapeHtml(content)}</pre>`; }
    // DOMPurify でサニタイズ（href/src スキームは markdown プレビューで開く相対リンク/data-* も許容したいので
    // 既定の許可リストで十分。許可外スキームの a[href] は DOMPurify が落とす）
    if (typeof DOMPurify !== 'undefined') {
      html = DOMPurify.sanitize(html, {
        ADD_ATTR: ['target', 'data-files-rel-link', 'data-files-path-link', 'data-files-skip-search'],
      });
    } else {
      // fail-closed: サニタイザ不在なら HTML を描画せずプレーンテキストで返す
      return `<pre class="files-preview-plain">${escapeHtml(content)}</pre>`;
    }
    return html;
  }

  /**
   * 各 <pre> ブロックを wrapper で囲み、右上にコピー用ボタンを追加する。
   * 検索ハイライト walker の対象から除外できるよう、ボタンには `data-files-skip-search` を付ける。
   */
  function addCodeCopyButtons(el) {
    el.querySelectorAll('pre').forEach(pre => {
      if (pre.parentElement && pre.parentElement.classList.contains('files-preview-code-wrapper')) return;
      const wrapper = document.createElement('div');
      wrapper.className = 'files-preview-code-wrapper';
      pre.parentNode.insertBefore(wrapper, pre);
      wrapper.appendChild(pre);

      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'files-preview-code-copy-btn';
      btn.dataset.filesSkipSearch = '1';
      const defaultLabel = t('files_preview_code_copy_label') || 'Copy';
      const copiedLabel = t('files_preview_code_copied_label') || 'Copied';
      btn.textContent = defaultLabel;
      btn.title = t('files_preview_code_copy_tooltip') || 'Copy code';
      btn.addEventListener('click', async (e) => {
        e.preventDefault();
        e.stopPropagation();
        const code = pre.querySelector('code');
        const text = (code ? code.innerText : pre.innerText) || '';
        try {
          await navigator.clipboard.writeText(text);
          btn.textContent = copiedLabel;
          btn.classList.add('copied');
          setTimeout(() => {
            btn.textContent = defaultLabel;
            btn.classList.remove('copied');
          }, 1200);
        } catch (_) {
          showToast(t('copied_to_clipboard') || 'Copied', btn);
        }
      });
      wrapper.appendChild(btn);
    });
  }

  /**
   * <table> を wrapper で囲み、右上に「表をコピー」ボタンを追加する。
   * クリップボードには Markdown テーブル形式 (text/plain) と HTML テーブル (text/html) の両方を書き込む。
   */
  function addTableCopyButtons(el) {
    el.querySelectorAll('table').forEach(table => {
      if (table.parentElement && table.parentElement.classList.contains('files-preview-table-wrapper')) return;
      const wrapper = document.createElement('div');
      wrapper.className = 'files-preview-table-wrapper';
      table.parentNode.insertBefore(wrapper, table);
      wrapper.appendChild(table);

      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'files-preview-table-copy-btn';
      btn.dataset.filesSkipSearch = '1';
      const defaultLabel = t('files_preview_table_copy_label') || 'Copy table';
      const copiedLabel = t('files_preview_table_copied_label') || 'Copied';
      btn.textContent = defaultLabel;
      btn.title = t('files_preview_table_copy_tooltip') || 'Copy table';
      btn.addEventListener('click', async (e) => {
        e.preventDefault();
        e.stopPropagation();
        const md = tableToMarkdown(table);
        const htmlText = table.outerHTML;
        try {
          if (window.ClipboardItem && navigator.clipboard.write) {
            const item = new ClipboardItem({
              'text/plain': new Blob([md], { type: 'text/plain' }),
              'text/html': new Blob([htmlText], { type: 'text/html' }),
            });
            await navigator.clipboard.write([item]);
          } else {
            await navigator.clipboard.writeText(md);
          }
          btn.textContent = copiedLabel;
          btn.classList.add('copied');
          setTimeout(() => {
            btn.textContent = defaultLabel;
            btn.classList.remove('copied');
          }, 1200);
        } catch (_) {
          try { await navigator.clipboard.writeText(md); } catch (_) {}
          showToast(t('copied_to_clipboard') || 'Copied', btn);
        }
      });
      wrapper.appendChild(btn);
    });
  }

  /** <table> を GFM Markdown テーブル文字列に変換 */
  function tableToMarkdown(table) {
    const rowsFromSection = (section) => {
      if (!section) return [];
      return Array.from(section.rows).map(tr =>
        Array.from(tr.cells).map(cell => {
          // セル内の改行・パイプを Markdown 互換にエスケープ
          const text = (cell.innerText || '').replace(/\r?\n/g, ' ').replace(/\|/g, '\\|').trim();
          return text;
        })
      );
    };

    const headRows = rowsFromSection(table.tHead);
    const bodyRows = [];
    Array.from(table.tBodies).forEach(tb => bodyRows.push(...rowsFromSection(tb)));

    // tHead がない場合は最初の行をヘッダ扱いにする
    let header = headRows[0];
    let body = bodyRows;
    if (!header && bodyRows.length > 0) {
      header = bodyRows[0];
      body = bodyRows.slice(1);
    }
    if (!header) return '';

    const colCount = header.length;
    const padRow = (row) => {
      const r = row.slice(0, colCount);
      while (r.length < colCount) r.push('');
      return r;
    };

    const lines = [];
    lines.push('| ' + padRow(header).join(' | ') + ' |');
    lines.push('| ' + Array(colCount).fill('---').join(' | ') + ' |');
    body.forEach(row => {
      lines.push('| ' + padRow(row).join(' | ') + ' |');
    });
    return lines.join('\n');
  }

  /** containerEl 内に右ペインを構築 */
  function bind(containerEl, opts) {
    const { sessionId, gitRoot, filesRoot } = opts;
    containerEl.innerHTML = '';
    containerEl.style.removeProperty('align-items');
    containerEl.style.removeProperty('justify-content');
    containerEl.style.removeProperty('color');

    // ──── ツールバー ────
    const toolbar = document.createElement('div');
    toolbar.className = 'files-preview-toolbar';

    const breadcrumb = document.createElement('span');
    breadcrumb.className = 'files-preview-breadcrumb';
    const breadcrumbDir = document.createElement('span');
    breadcrumbDir.className = 'files-preview-breadcrumb-dir';
    const breadcrumbFile = document.createElement('span');
    breadcrumbFile.className = 'files-preview-breadcrumb-file';
    breadcrumb.appendChild(breadcrumbDir);
    breadcrumb.appendChild(breadcrumbFile);
    toolbar.appendChild(breadcrumb);

    const spacer = document.createElement('span');
    spacer.style.flex = '0 0 auto';
    toolbar.appendChild(spacer);

    // ツールバーボタン群（ファイル選択後に有効化）
    const openEditorBtn = document.createElement('button');
    openEditorBtn.className = 'files-preview-toolbar-btn';
    openEditorBtn.title = t('files_preview_open_editor_tooltip') || 'Open in editor';
    openEditorBtn.textContent = '📝';
    openEditorBtn.disabled = true;

    const openFolderBtn = document.createElement('button');
    openFolderBtn.className = 'files-preview-toolbar-btn';
    openFolderBtn.title = t('files_preview_open_folder_tooltip') || 'Open folder';
    openFolderBtn.textContent = '📁';
    openFolderBtn.disabled = true;

    const copyPathBtn = document.createElement('button');
    copyPathBtn.className = 'files-preview-toolbar-btn';
    copyPathBtn.title = t('files_preview_copy_path_tooltip') || 'Copy path';
    copyPathBtn.textContent = '🔗';
    copyPathBtn.disabled = true;

    const searchBtn = document.createElement('button');
    searchBtn.className = 'files-preview-toolbar-btn';
    searchBtn.title = t('files_preview_search_tooltip') || 'Search in page';
    searchBtn.textContent = '🔎';
    searchBtn.disabled = true;

    const reloadBtn = document.createElement('button');
    reloadBtn.className = 'files-preview-toolbar-btn';
    reloadBtn.title = t('files_preview_reload_tooltip') || 'Reload';
    reloadBtn.textContent = '🔄';
    reloadBtn.disabled = true;

    const editBtn = document.createElement('button');
    editBtn.className = 'files-preview-toolbar-btn';
    editBtn.title = t('files_preview_edit_tooltip') || 'Edit';
    editBtn.textContent = '✏️';
    editBtn.disabled = true;
    editBtn.hidden = true;

    const closeBtn = document.createElement('button');
    closeBtn.className = 'files-preview-toolbar-btn';
    closeBtn.title = t('files_preview_close_tooltip') || 'Close preview';
    closeBtn.textContent = '×';
    closeBtn.disabled = true;

    toolbar.appendChild(openEditorBtn);
    toolbar.appendChild(openFolderBtn);
    toolbar.appendChild(copyPathBtn);
    toolbar.appendChild(searchBtn);
    toolbar.appendChild(reloadBtn);
    toolbar.appendChild(editBtn);
    toolbar.appendChild(closeBtn);

    containerEl.appendChild(toolbar);

    // ──── ページ内検索バー ────
    const searchBar = document.createElement('div');
    searchBar.className = 'files-preview-search-bar';
    searchBar.hidden = true;
    const searchInput = document.createElement('input');
    searchInput.type = 'text';
    searchInput.className = 'files-preview-search-input';
    searchInput.placeholder = t('files_preview_search_placeholder') || 'Search…';
    const searchPrevBtn = document.createElement('button');
    searchPrevBtn.className = 'files-preview-search-nav-btn';
    searchPrevBtn.textContent = '▲';
    searchPrevBtn.title = t('files_preview_search_prev') || 'Previous';
    const searchNextBtn = document.createElement('button');
    searchNextBtn.className = 'files-preview-search-nav-btn';
    searchNextBtn.textContent = '▼';
    searchNextBtn.title = t('files_preview_search_next') || 'Next';
    const searchCountEl = document.createElement('span');
    searchCountEl.className = 'files-preview-search-count';
    const searchCloseBtn = document.createElement('button');
    searchCloseBtn.className = 'files-preview-search-nav-btn';
    searchCloseBtn.textContent = '×';
    searchCloseBtn.title = t('files_preview_search_close') || 'Close search';
    searchBar.appendChild(searchInput);
    searchBar.appendChild(searchPrevBtn);
    searchBar.appendChild(searchNextBtn);
    searchBar.appendChild(searchCountEl);
    searchBar.appendChild(searchCloseBtn);
    containerEl.appendChild(searchBar);

    // ──── 本文エリア ────
    const bodyEl = document.createElement('div');
    bodyEl.className = 'files-preview-body';
    const contentEl = document.createElement('div');
    contentEl.className = 'files-preview-markdown';
    bodyEl.appendChild(contentEl);
    containerEl.appendChild(bodyEl);

    let currentAbsPath = null;
    let currentRelPath = null;
    let highlightMatches = [];
    let highlightIndex = 0;

    // ──── 編集モード状態 ────
    let editMode = false;           // 編集モード中フラグ
    let editBaseContent = null;     // 編集開始時のオリジナル内容
    let editBaseMtime = null;       // 競合検出用 mtime（RFC3339 文字列）
    let editTextarea = null;        // 現在の <textarea> 要素
    let editSaveBtn = null;         // 保存ボタン
    let editDiscardBtn = null;      // 破棄ボタン
    let editStatusEl = null;        // ステータス行（競合通知等）
    let editBarEl = null;           // 編集モード操作バー

    function setBreadcrumb(pathText, fullPath) {
      const text = pathText || (t('files_preview_no_file') || 'No file selected');
      const normalized = String(text).replace(/\\/g, '/');
      const slashIdx = normalized.lastIndexOf('/');
      if (slashIdx >= 0) {
        breadcrumbDir.textContent = normalized.slice(0, slashIdx + 1);
        breadcrumbFile.textContent = normalized.slice(slashIdx + 1);
      } else {
        breadcrumbDir.textContent = '';
        breadcrumbFile.textContent = normalized;
      }
      breadcrumb.title = fullPath || text;
    }

    setBreadcrumb('');

    // ──── ページ内検索 ────
    function clearHighlights() {
      contentEl.querySelectorAll('.files-search-highlight').forEach(el => {
        el.replaceWith(el.firstChild || document.createTextNode(''));
      });
      contentEl.normalize();
      highlightMatches = [];
      highlightIndex = 0;
      searchCountEl.textContent = '';
    }

    function applySearch(query) {
      clearHighlights();
      if (!query) return;
      const walker = document.createTreeWalker(contentEl, NodeFilter.SHOW_TEXT, {
        acceptNode(n) {
          // コピーボタンの中の文字列は検索対象から除外
          if (n.parentNode && n.parentNode.closest && n.parentNode.closest('[data-files-skip-search]')) {
            return NodeFilter.FILTER_REJECT;
          }
          return NodeFilter.FILTER_ACCEPT;
        },
      });
      const textNodes = [];
      let node;
      while ((node = walker.nextNode())) textNodes.push(node);
      const lq = query.toLowerCase();
      for (const tn of textNodes) {
        const val = tn.nodeValue;
        const idx = val.toLowerCase().indexOf(lq);
        if (idx === -1) continue;
        const before = document.createTextNode(val.slice(0, idx));
        const mark = document.createElement('mark');
        mark.className = 'files-search-highlight';
        mark.textContent = val.slice(idx, idx + query.length);
        const after = document.createTextNode(val.slice(idx + query.length));
        const parent = tn.parentNode;
        parent.insertBefore(before, tn);
        parent.insertBefore(mark, tn);
        parent.insertBefore(after, tn);
        parent.removeChild(tn);
        highlightMatches.push(mark);
      }
      searchCountEl.textContent = highlightMatches.length > 0
        ? `${highlightIndex + 1}/${highlightMatches.length}`
        : t('files_search_no_results') || '0 results';
      if (highlightMatches.length > 0) scrollToMatch(0);
    }

    function scrollToMatch(idx) {
      highlightMatches.forEach((el, i) => el.classList.toggle('files-search-highlight--active', i === idx));
      if (highlightMatches[idx]) highlightMatches[idx].scrollIntoView({ block: 'center' });
      searchCountEl.textContent = `${idx + 1}/${highlightMatches.length}`;
    }

    let _previewSearchDebounceTimer = null;
    searchInput.addEventListener('input', () => {
      if (_previewSearchDebounceTimer) clearTimeout(_previewSearchDebounceTimer);
      _previewSearchDebounceTimer = setTimeout(() => {
        _previewSearchDebounceTimer = null;
        applySearch(searchInput.value);
      }, 150);
    });
    searchPrevBtn.addEventListener('click', () => {
      if (!highlightMatches.length) return;
      highlightIndex = (highlightIndex - 1 + highlightMatches.length) % highlightMatches.length;
      scrollToMatch(highlightIndex);
    });
    searchNextBtn.addEventListener('click', () => {
      if (!highlightMatches.length) return;
      highlightIndex = (highlightIndex + 1) % highlightMatches.length;
      scrollToMatch(highlightIndex);
    });
    searchInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        if (e.shiftKey) searchPrevBtn.click();
        else searchNextBtn.click();
      } else if (e.key === 'Escape') {
        searchCloseBtn.click();
      }
    });
    searchCloseBtn.addEventListener('click', () => {
      clearHighlights();
      searchInput.value = '';
      searchBar.hidden = true;
      searchBtn.classList.remove('active');
    });

    searchBtn.addEventListener('click', () => {
      if (!currentAbsPath) return;
      const show = searchBar.hidden;
      searchBar.hidden = !show;
      if (show) { searchInput.focus(); }
      else { clearHighlights(); searchInput.value = ''; }
    });

    // ──── 編集モード操作 ────

    /** 編集モードを終了してプレビュー表示に戻す（UI のみリセット） */
    function exitEditMode() {
      editMode = false;
      editBaseContent = null;
      editBaseMtime = null;
      editTextarea = null;
      editSaveBtn = null;
      editDiscardBtn = null;
      editStatusEl = null;
      if (editBarEl) {
        try { editBarEl.remove(); } catch (_) {}
        editBarEl = null;
      }
      // ツールバーボタン復元
      editBtn.hidden = true;
      editBtn.disabled = true;
      [openEditorBtn, openFolderBtn, copyPathBtn, searchBtn, reloadBtn].forEach(b => { b.disabled = !currentAbsPath; });
      searchBtn.disabled = !currentAbsPath || isMediaPath(currentAbsPath);
    }

    /**
     * テキストファイルを編集モードで開く。
     * content / mtime は loadFile 時に取得済みのものを使う。
     */
    function enterEditMode(content, mtime) {
      editMode = true;
      editBaseContent = content;
      editBaseMtime = mtime;

      // ツールバーボタンを無効化（編集中は検索・リロード等を封じる）
      [openEditorBtn, openFolderBtn, copyPathBtn, searchBtn, reloadBtn, editBtn].forEach(b => { b.disabled = true; });

      // contentEl を textarea に切り替え
      contentEl.innerHTML = '';
      const textarea = document.createElement('textarea');
      textarea.className = 'files-preview-edit-textarea';
      textarea.value = content;
      textarea.spellcheck = false;
      textarea.autocomplete = 'off';
      textarea.autocorrect = 'off';
      textarea.autocapitalize = 'off';
      contentEl.appendChild(textarea);
      editTextarea = textarea;
      textarea.focus();

      // 編集操作バー（保存 / 破棄 / ステータス）
      const bar = document.createElement('div');
      bar.className = 'files-preview-edit-bar';
      editBarEl = bar;

      const saveBtn = document.createElement('button');
      saveBtn.className = 'files-preview-toolbar-btn files-preview-edit-save-btn';
      saveBtn.textContent = t('files_preview_edit_save') || '保存';
      saveBtn.title = t('files_preview_edit_save_tooltip') || 'Save file';
      editSaveBtn = saveBtn;

      const discardBtn = document.createElement('button');
      discardBtn.className = 'files-preview-toolbar-btn';
      discardBtn.textContent = t('files_preview_edit_discard') || '破棄';
      discardBtn.title = t('files_preview_edit_discard_tooltip') || 'Discard changes';
      editDiscardBtn = discardBtn;

      const statusEl = document.createElement('span');
      statusEl.className = 'files-preview-edit-status';
      editStatusEl = statusEl;

      bar.appendChild(saveBtn);
      bar.appendChild(discardBtn);
      bar.appendChild(statusEl);
      containerEl.insertBefore(bar, bodyEl);

      // 保存
      saveBtn.addEventListener('click', () => doSave(false));

      // 破棄
      discardBtn.addEventListener('click', () => {
        const changed = editTextarea && editTextarea.value !== editBaseContent;
        if (changed) {
          const msg = t('files_preview_edit_discard_confirm') || 'Discard changes?';
          if (!window.confirm(msg)) return;
        }
        exitEditMode();
        loadFile(currentAbsPath, currentRelPath);
      });
    }

    /**
     * POST /api/files-save を呼ぶ。
     * forceOverwrite=true のときは baseMtime を serverMtime で上書きして再送する。
     */
    async function doSave(forceOverwrite, serverMtime) {
      if (!currentAbsPath || !editTextarea) return;
      if (editSaveBtn) editSaveBtn.disabled = true;
      if (editStatusEl) editStatusEl.textContent = t('files_preview_edit_saving') || '保存中…';

      const baseMtime = forceOverwrite ? serverMtime : editBaseMtime;
      const body = {
        path: currentAbsPath,
        content: editTextarea.value,
      };
      if (baseMtime) body.baseMtime = baseMtime;

      try {
        const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
        const url = `/api/files-save?token=${encodeURIComponent(token)}${sessionQs}`;
        const res = await fetch(url, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
        });
        const data = await res.json().catch(() => ({}));
        if (res.ok && data.ok) {
          // 保存成功 → 編集モード終了してプレビュー再取得
          exitEditMode();
          loadFile(currentAbsPath, currentRelPath);
          return;
        }
        if (res.status === 409) {
          // 競合: 他プロセスによる変更
          const newMtime = data.mtime || null;
          showConflict(newMtime);
          return;
        }
        // その他エラー
        const msg = data.detail || data.error || ('HTTP ' + res.status);
        if (editStatusEl) editStatusEl.textContent = t('files_preview_edit_save_error') || ('保存失敗: ' + msg);
        if (editSaveBtn) editSaveBtn.disabled = false;
      } catch (err) {
        if (editStatusEl) editStatusEl.textContent = String(err);
        if (editSaveBtn) editSaveBtn.disabled = false;
      }
    }

    /** 409 競合時の UI を表示する */
    function showConflict(serverMtime) {
      if (!editStatusEl || !editBarEl) return;
      editStatusEl.textContent = '';

      const conflictMsg = document.createElement('span');
      conflictMsg.className = 'files-preview-edit-conflict';
      conflictMsg.textContent = t('files_preview_edit_conflict_msg') || '他のプロセスがこのファイルを変更しました';

      const reloadLatestBtn = document.createElement('button');
      reloadLatestBtn.className = 'files-preview-toolbar-btn';
      reloadLatestBtn.textContent = t('files_preview_edit_conflict_reload') || '最新を読み直す';
      reloadLatestBtn.addEventListener('click', () => {
        exitEditMode();
        loadFile(currentAbsPath, currentRelPath);
      });

      const overwriteBtn = document.createElement('button');
      overwriteBtn.className = 'files-preview-toolbar-btn files-preview-edit-save-btn';
      overwriteBtn.textContent = t('files_preview_edit_conflict_overwrite') || '上書き保存';
      overwriteBtn.addEventListener('click', () => {
        editStatusEl.innerHTML = '';
        doSave(true, serverMtime);
      });

      editStatusEl.appendChild(conflictMsg);
      editStatusEl.appendChild(reloadLatestBtn);
      editStatusEl.appendChild(overwriteBtn);

      if (editSaveBtn) editSaveBtn.disabled = false;
    }

    // ──── ファイルロード ────
    async function loadFile(absPath, relPath) {
      // 編集モード中なら先にリセット（別ファイルを選んだ場合等）
      if (editMode) exitEditMode();

      contentEl.innerHTML = `<div class="files-preview-loading">${escapeHtml(t('files_tab_loading') || 'Loading…')}</div>`;
      clearHighlights();
      searchBar.hidden = true;
      searchInput.value = '';

      if (isMediaPath(absPath)) {
        const mediaUrl = getFilesAssetUrl(absPath, sessionId || '');
        const wrap = document.createElement('div');
        wrap.className = 'files-preview-image-wrap';
        wrap.dataset.filesSkipSearch = '1';
        let media;
        if (isVideoPath(absPath)) {
          media = document.createElement('video');
          media.className = 'files-preview-image files-preview-video';
          media.controls = true;
          media.preload = 'metadata';
          media.playsInline = true;
          media.src = mediaUrl;
          media.addEventListener('dblclick', () => openLightbox(mediaUrl, { type: 'video' }));
          const modalBtn = document.createElement('button');
          modalBtn.type = 'button';
          modalBtn.className = 'files-preview-media-modal-btn';
          modalBtn.title = 'Open video';
          modalBtn.textContent = '⛶';
          modalBtn.addEventListener('click', () => openLightbox(mediaUrl, { type: 'video' }));
          wrap.appendChild(modalBtn);
        } else {
          media = document.createElement('img');
          media.className = 'files-preview-image';
          media.alt = relPath || absPath;
          media.src = mediaUrl;
          media.addEventListener('click', () => openLightbox(mediaUrl));
        }
        const caption = document.createElement('div');
        caption.className = 'files-preview-image-caption';
        caption.textContent = relPath || absPath;
        wrap.appendChild(media);
        wrap.appendChild(caption);
        contentEl.innerHTML = '';
        contentEl.appendChild(wrap);
        bodyEl.scrollTop = 0;
        return;
      }

      try {
        const sessionQs = sessionId ? `&session=${encodeURIComponent(sessionId)}` : '';
        const url = `/api/files-content?path=${encodeURIComponent(absPath)}&token=${encodeURIComponent(token)}${sessionQs}`;
        const res = await fetch(url);
        if (!res.ok) {
          contentEl.innerHTML = `<div class="files-preview-error">${escapeHtml('HTTP ' + res.status)}</div>`;
          return;
        }
        const data = await res.json();
        const content = data.content || '';
        const isMd = /\.md$/i.test(absPath);

        // 編集ボタンの表示制御: truncated ファイルは編集不可
        const canEdit = !data.truncated;
        editBtn.hidden = !canEdit;
        editBtn.disabled = !canEdit;
        // content と mtime を保持（編集モード開始時・doSave の baseMtime に使う）
        editBaseContent = content;
        editBaseMtime = data.mtime || null;

        if (data.truncated) {
          const warn = document.createElement('div');
          warn.className = 'files-preview-truncated-warn';
          warn.textContent = t('files_preview_truncated_warn') || '⚠ File truncated (>1MiB). Showing partial content.';
          contentEl.innerHTML = '';
          contentEl.appendChild(warn);
          contentEl.appendChild(renderSourceToPre(content, absPath));
          addCodeCopyButtons(contentEl);
        } else if (isMd) {
          const html = renderMarkdown(content, absPath, null);
          contentEl.innerHTML = `<div class="files-preview-md-body">${html}</div>`;
          addCodeCopyButtons(contentEl);
          addTableCopyButtons(contentEl);

          // 相対リンク処理
          contentEl.querySelectorAll('a[data-files-rel-link]').forEach(a => {
            a.addEventListener('click', (e) => {
              e.preventDefault();
              const rel = a.dataset.filesRelLink;
              if (!rel) return;
              // 現在のファイルのディレクトリから絶対パスを計算
              const dir = currentAbsPath ? currentAbsPath.replace(/[/\\][^/\\]*$/, '') : filesRoot;
              const sep = dir.includes('\\') ? '\\' : '/';
              const target = dir + sep + rel.replace(/\//g, sep);
              containerEl._filesPreview && containerEl._filesPreview.loadFile(target, rel);
            });
          });
          contentEl.querySelectorAll('a[data-files-path-link]').forEach(a => {
            a.addEventListener('click', (e) => {
              e.preventDefault();
              const p = a.dataset.filesPathLink;
              if (p) showPathPopup(p, e.clientX, e.clientY, sessionId || '');
            });
          });
        } else {
          // .txt など — hljs.highlightAuto で自動判定（巨大ファイルはプレーン）
          contentEl.innerHTML = '';
          contentEl.appendChild(renderSourceToPre(content, absPath));
          addCodeCopyButtons(contentEl);
        }
        bodyEl.scrollTop = 0;
      } catch (err) {
        contentEl.innerHTML = `<div class="files-preview-error">${escapeHtml(String(err))}</div>`;
      }
    }

    // ──── ボタン Wire-up ────
    openEditorBtn.addEventListener('click', () => {
      if (currentAbsPath) callOpenApi('/api/open-file', currentAbsPath, 'link_open_error', sessionId);
    });
    openFolderBtn.addEventListener('click', () => {
      if (currentAbsPath) callOpenApi('/api/open-folder', currentAbsPath, 'link_open_error', sessionId);
    });
    copyPathBtn.addEventListener('click', (e) => {
      if (currentAbsPath) copyPathText(currentAbsPath, e.currentTarget).catch(() => {});
    });
    reloadBtn.addEventListener('click', () => {
      if (currentAbsPath) loadFile(currentAbsPath, currentRelPath);
    });
    closeBtn.addEventListener('click', () => {
      if (editMode) exitEditMode();
      currentAbsPath = null;
      currentRelPath = null;
      setBreadcrumb('');
      contentEl.innerHTML = '';
      clearHighlights();
      searchBar.hidden = true;
      editBtn.hidden = true;
      editBtn.disabled = true;
      [openEditorBtn, openFolderBtn, copyPathBtn, searchBtn, reloadBtn, closeBtn].forEach(b => { b.disabled = true; });
    });

    editBtn.addEventListener('click', () => {
      if (editMode || !currentAbsPath) return;
      // editBaseContent / editBaseMtime は loadFile 完了時に設定済み
      enterEditMode(editBaseContent || '', editBaseMtime);
    });

    // 外部 API
    containerEl._filesPreview = {
      loadFile: (absPath, relPath) => {
        currentAbsPath = absPath;
        currentRelPath = relPath;
        const dispPath = relPath || absPath;
        setBreadcrumb(dispPath, absPath);
        [openEditorBtn, openFolderBtn, copyPathBtn, searchBtn, reloadBtn, closeBtn].forEach(b => { b.disabled = false; });
        searchBtn.disabled = isMediaPath(absPath);
        // 編集ボタンはロード後に truncated 判定で設定する（初期は非表示）
        editBtn.hidden = true;
        editBtn.disabled = true;
        loadFile(absPath, relPath);
      },
    };
  }

  function unbind(containerEl) {
    containerEl.innerHTML = '';
    delete containerEl._filesPreview;
  }

  return { bind, unbind };
})();

/**
 * FilesViewMounter — タブ生成後に FilesTreeView + FilesPreview を該当 DOM に bind する
 *
 * FilesTabManager が openFilesTab() でスケルトンを作り contentEl を filesContents に追加しているので、
 * MutationObserver で検知してすぐ bind する。
 */
(function () {
  const filesContents = document.getElementById('files-tab-contents');
  if (!filesContents) return;

  function mountTab(contentEl) {
    if (contentEl._filesMounted) return;
    contentEl._filesMounted = true;

    const filesRoot  = contentEl.dataset.filesRoot  || '';
    const sessionId  = contentEl.dataset.sessionId  || '';
    const gitRoot    = contentEl.dataset.gitRoot    || '';
    const projectKey = contentEl.dataset.projectKey || '';
    const tabId      = contentEl.dataset.tabId      || '';

    const treePaneEl    = contentEl.querySelector('[data-files-tree]');
    const previewPaneEl = contentEl.querySelector('[data-files-preview]');
    if (!treePaneEl || !previewPaneEl) return;

    // ペインのローディングテキストを消してスタイルリセット
    treePaneEl.textContent    = '';
    previewPaneEl.textContent = '';

    // ─── FilesPreview を先に bind（onFileSelect から参照するため）───
    FilesPreview.bind(previewPaneEl, { sessionId, gitRoot, filesRoot });

    // ─── FilesTreeView を bind ───
    FilesTreeView.bind(treePaneEl, {
      filesRoot,
      sessionId,
      gitRoot,
      onFileSelect: (node) => {
        if (!previewPaneEl._filesPreview) return;
        const relPath = node.relPath;
        previewPaneEl._filesPreview.loadFile(node.absPath, relPath);
        // localStorage 更新
        FilesTabManager.lsUpdateOpenedFile(gitRoot, filesRoot, node.absPath);
      },
    });

    // ─── localStorage 復元: 最後に開いていたファイルを自動選択 ───
    try {
      const lsKey = 'any-ai-cli.files.tabs';
      const data = JSON.parse(localStorage.getItem(lsKey) || '{}');
      const entries = data[gitRoot] || [];
      const entry = entries.find(e => e.root === filesRoot);
      if (entry && entry.openedFile) {
        const openedFile = entry.openedFile;
        // ツリーが API 取得後に描画されるまで少し待つ
        const trySelect = () => {
          const fileEl = treePaneEl.querySelector(`.files-tree-item[data-abs-path="${CSS.escape(openedFile)}"]`);
          if (fileEl) {
            fileEl.click();
          } else {
            // ツリーがまだ描画されていない → 少し後に再試行（最大3回）
            if (!trySelect._count) trySelect._count = 0;
            if (trySelect._count++ < 6) setTimeout(trySelect, 300);
          }
        };
        setTimeout(trySelect, 400);
      }
    } catch (_) {}
  }

  // 既存 contentEl のマウント（restoreFromLocalStorage で追加済みのものを処理）
  filesContents.querySelectorAll('.files-tab-content').forEach(mountTab);

  // 新規追加を検知
  const observer = new MutationObserver(mutations => {
    for (const m of mutations) {
      m.addedNodes.forEach(node => {
        if (node.nodeType === 1 && node.classList.contains('files-tab-content')) {
          mountTab(node);
        }
      });
    }
  });
  observer.observe(filesContents, { childList: true });
})();
