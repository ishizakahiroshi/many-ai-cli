// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { escapeHtml, ti18n, token } from './util.js';
import { activeSessionId, collapsedGroups, dragOverCardEl, dragOverGroupEl, dragSrcGroupKey, dragSrcId, favorites, groupOrder, multiQuestionVisibleCache, orderSessions, projectFavorites, saveFavorites, saveGroupOrder, saveProjectFavorites, saveSessionOrder, sessionOrder, sessions, set_actionBarFocusIdx, set_activeSessionId, set_dragOverCardEl, set_dragOverGroupEl, set_dragSrcGroupKey, set_dragSrcId, set_groupOrder, terminals } from './state.js';
import { dismissSession, inputEl, requestSessionHistoryReset, restoreInputStateFor, saveInputStateFor, updateInputAffordance } from '../app.js';
import { attachTerminal, ensureTerminal, refitAndStickTerminalToBottomAfterLayoutSettles, refitAndStickTerminalToBottomSoon, revealApprovalPromptForSession, scrollTerminalToBottomSoon, updateScrollLockBtn } from './terminal.js';
import { applyActiveSessionViewMode, filterFirstMessage, openCardCtxMenu, renderSessionInfoChip, updateChatCountBadge } from './settings.js';
import { syncElapsedTimer } from './ws-client.js';
import { setMultiQuestionBannerVisible } from './approval-ui.js';
import { detectApproval, setActionBarFocus } from './approval.js';
import { onActiveSessionChanged } from './token-statusbar.js';
import { rewireChatHistorySub } from './chat-history.js';
import { FilesTabManager } from './files-view.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- セッション管理 ----

export function updateSessionListActiveCard(id) {
  const root = document.getElementById('sessions');
  if (!root) return;
  root.querySelectorAll('.card').forEach(card => {
    const cid = parseInt(card.dataset.sessionId, 10);
    card.classList.toggle('active', cid === id);
  });
}

// C4: focusSlot → activateSession → focusSlot の無限ループ防止フラグ
export let _multiPaneFocusSyncing = false;

/**
 * C4: マルチペインのフォーカス切替用の軽量版 activateSession。
 * シングルビュー固有の処理（ensureTerminal / attachTerminal / FilesTabManager 等）は
 * 行わず、activeSessionId の更新と承認 UI 検出・サイドバー更新のみ実施する。
 * multi-pane.js の focusSlot() から呼ばれる。
 */
export function activateSessionForMultiPane(id) {
  if (activeSessionId !== null && activeSessionId !== id) {
    saveInputStateFor(activeSessionId);
  }
  set_activeSessionId(id);
  if (typeof window.syncMobileLayoutState === 'function') window.syncMobileLayoutState();
  if (typeof window.closeMobileSessionDrawer === 'function') window.closeMobileSessionDrawer();
  restoreInputStateFor(id);
  const t = terminals.get(id);
  if (t) {
    t.autoScroll = true;
    const switchStartedAt = Date.now();
    scrollTerminalToBottomSoon(id, { force: true, passes: 4, startedAt: switchStartedAt });
    refitAndStickTerminalToBottomAfterLayoutSettles(id, {
      force: true,
      passes: 4,
      startedAt: switchStartedAt,
    });
  }
  // 承認 UI をフォーカスセッション向きに更新
  setMultiQuestionBannerVisible(!!multiQuestionVisibleCache.get(id));
  // フォーカスセッションの実行中状態を入力欄／送信ボタンへ反映
  updateInputAffordance();
  detectApproval(id);
  revealApprovalPromptForSession(id);
  // サイドバーのアクティブカードを更新
  updateSessionListActiveCard(id);
  // チャット件数バッジ・セッション情報チップも更新
  if (typeof updateChatCountBadge === 'function') updateChatCountBadge();
  if (typeof renderSessionInfoChip === 'function') renderSessionInfoChip();
  if (typeof syncElapsedTimer === 'function') syncElapsedTimer();
  onActiveSessionChanged();
}
// multi-pane.js から参照できるよう window に公開
window.activateSessionForMultiPane = activateSessionForMultiPane;

export function activateSession(id) {
  // C4: マルチタブが開いているとき、外部からの activateSession 呼び出し（承認自動移動等）は
  // 軽量版にリダイレクトし、シングルビュー固有の処理（attachTerminal 等）を実行しない。
  // _multiPaneFocusSyncing フラグが立っているときは再帰防止のためスキップ。
  if (!_multiPaneFocusSyncing) {
    const multiView = document.getElementById('multi-view');
    const mgr = window.multiPaneManager;
    if (multiView && !multiView.hidden && mgr) {
      // フォーカス対象スロットのインデックスを探す
      const slotIdx = mgr.slots.findIndex(s => s && s.session && s.session.id === id);
      _multiPaneFocusSyncing = true;
      try {
        if (slotIdx >= 0) {
          // スロットが見つかった: フォーカスを移動（DOM + activeSessionId 更新）
          mgr.focusSlot(slotIdx);
        } else {
          // スロット外のセッション（表示数超過）: activeSessionId のみ更新
          activateSessionForMultiPane(id);
        }
      } finally {
        _multiPaneFocusSyncing = false;
      }
      return;
    }
  }
  if (activeSessionId !== null && activeSessionId !== id) {
    saveInputStateFor(activeSessionId);
  }
  set_activeSessionId(id);
  if (typeof window.syncMobileLayoutState === 'function') window.syncMobileLayoutState();
  if (typeof window.closeMobileSessionDrawer === 'function') window.closeMobileSessionDrawer();
  restoreInputStateFor(id);
  // files/git 表示からセッションカードへ戻る場合、先にターミナルを表示してから
  // attach/fit/detect しないと、承認 UI 検出と最下部スナップが hidden レイアウトを基準に走る。
  FilesTabManager.switchToSessionView();
  ensureTerminal(id);
  attachTerminal(id);
  updateScrollLockBtn();
  // 切替先セッションの実行中状態に合わせて入力欄プレースホルダ／送信ボタンを更新
  updateInputAffordance();
  setMultiQuestionBannerVisible(!!multiQuestionVisibleCache.get(id));
  detectApproval(id);
  updateSessionListActiveCard(id);
  updateShellBadge(id);
  updateQuickCmdButtons(id);
  // C2: D11 セッション情報チップ更新 + D13 セッション毎モード復元 + チャット件数バッジ購読
  if (typeof renderSessionInfoChip === 'function') renderSessionInfoChip();
  if (typeof applyActiveSessionViewMode === 'function') applyActiveSessionViewMode();
  if (typeof rewireChatHistorySub === 'function') rewireChatHistorySub(id);
  inputEl.focus();
  if (typeof window._wakewordSessionChanged === 'function') window._wakewordSessionChanged();
  const sessionInfo = sessions.get(id);
  if (sessionInfo) {
    const label = sessionInfo.label
      ? `[${sessionInfo.label}] #${id}`
      : `#${id}`;
    FilesTabManager.updateSessionTabLabel(label);
  }
  if (typeof syncElapsedTimer === 'function') syncElapsedTimer();
  onActiveSessionChanged();
  const switchStartedAt = Date.now();
  scrollTerminalToBottomSoon(id, { force: true, passes: 4, startedAt: switchStartedAt });
  requestAnimationFrame(() => {
    if (activeSessionId !== id) return;
    detectApproval(id);
    refitAndStickTerminalToBottomSoon(id, { force: true, passes: 4, startedAt: switchStartedAt });
  });
  refitAndStickTerminalToBottomAfterLayoutSettles(id, {
    force: true,
    passes: 4,
    startedAt: switchStartedAt,
  });
}

export function updateShellBadge(id) {
  const el = document.getElementById('terminal-shell-info');
  if (!el) return;
  const s = id !== null ? sessions.get(id) : null;
  const shell = s?.shell || '';
  el.textContent = shell ? ' · ' + shell : '';
}

// provider 別の quick コマンドボタン制御。
// Ollama REPL は `/model` を持たず（近いのは `/load`）、Hub 側のスラッシュピッカーも
// Claude/Codex 専用候補で構成されているため、Ollama セッションでは両方を非活性化する。
// Shell session も同様に `/model` とスラッシュピッカーを非活性化する。
// `/clear` は Ollama REPL にも実在し意味も一致するため活性のまま残す。
export function updateQuickCmdButtons(id) {
  const s = id !== null ? sessions.get(id) : null;
  const provider = s?.provider || '';
  const isOllama = provider === 'ollama';
  const isShell  = provider === 'shell';
  const shouldDisable = isOllama || isShell;
  const modelBtn  = document.getElementById('quick-model-btn');
  const pickerBtn = document.getElementById('slash-picker-btn');
  for (const btn of [modelBtn, pickerBtn]) {
    if (!btn) continue;
    btn.disabled = shouldDisable;
    if (shouldDisable) btn.setAttribute('aria-disabled', 'true');
    else btn.removeAttribute('aria-disabled');
  }
}

export function stateLabel(state) {
  return t('state_' + state) || state;
}

export function safeClassToken(value) {
  const cleaned = String(value || '')
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64);
  return cleaned || 'unknown';
}

export function providerDisplayName(provider) {
  const key = String(provider || '').toLowerCase();
  const labels = {
    claude: 'Claude',
    codex: 'Codex',
    copilot: 'Copilot',
    'cursor-agent': 'Cursor Agent',
    ollama: 'Ollama',
    opencode: 'OpenCode',
  };
  return labels[key] || String(provider || '');
}

export function providerIconHtml(provider, size = 16) {
  const key = String(provider || '').toLowerCase();
  const parsedSize = Number(size);
  const safeSize = Number.isFinite(parsedSize) && parsedSize > 0 ? Math.min(Math.floor(parsedSize), 64) : 16;
  const base = `class="card-provider-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="${safeSize}" height="${safeSize}" aria-hidden="true"`;
  const txt  = `text-anchor="middle" dominant-baseline="central" font-size="7.5" font-weight="bold" font-family="sans-serif"`;
  if (key === 'claude') {
    return `<svg ${base}><circle class="prov-shape claude" cx="8" cy="8" r="6" stroke-width="2"/><text class="prov-letter claude" x="8" y="8" ${txt}>C</text></svg>`;
  }
  if (key === 'codex') {
    return `<svg ${base}><circle class="prov-shape codex" cx="8" cy="8" r="6" stroke-width="2"/><text class="prov-letter codex" x="8" y="8" ${txt}>X</text></svg>`;
  }
  if (key === 'copilot') {
    return `<svg ${base}><circle class="prov-shape copilot" cx="8" cy="8" r="6" stroke-width="2"/><text class="prov-letter copilot" x="8" y="8" ${txt}>P</text></svg>`;
  }
  if (key === 'cursor-agent') {
    return `<svg ${base}><circle class="prov-shape cursor-agent" cx="8" cy="8" r="6" stroke-width="2"/><text class="prov-letter cursor-agent" x="8" y="8" ${txt}>r</text></svg>`;
  }
  if (key === 'ollama') {
    return `<svg ${base}><rect class="prov-shape ollama" x="1" y="1" width="14" height="14" rx="3" stroke-width="2"/><text class="prov-letter ollama" x="8" y="8" ${txt}>O</text></svg>`;
  }
  if (key === 'opencode') {
    return `<svg ${base}><rect class="prov-shape opencode" x="1" y="1" width="14" height="14" rx="3" stroke-width="2"/><text class="prov-letter opencode" x="8" y="8" ${txt}>O</text></svg>`;
  }
  const letter = escapeHtml((String(provider || '?').trim()[0] || '?').toUpperCase());
  return `<svg ${base}><circle class="prov-shape" cx="8" cy="8" r="6" stroke-width="2"/><text class="prov-letter" x="8" y="8" ${txt}>${letter}</text></svg>`;
}

// C9: 整列ロジックは state.js の orderSessions に集約。getOrderedSessions は後方互換の薄い委譲。
export function getOrderedSessions() {
  return orderSessions();
}

export function switchSessionByTab(shift) {
  if (sessions.size <= 1) return;
  const all = getOrderedSessions();
  const currentIdx = all.findIndex(s => s.id === activeSessionId);
  if (currentIdx === -1) return;
  const nextIdx = shift
    ? (currentIdx - 1 + all.length) % all.length
    : (currentIdx + 1) % all.length;
  activateSession(all[nextIdx].id);
  const bar = document.getElementById('action-bar');
  if (bar && bar.classList.contains('visible') && !bar.classList.contains('batch')) {
    setActionBarFocus(0);
  } else {
    set_actionBarFocusIdx(-1);
  }
}

export function jumpToSessionByIndex(n) {
  const all = getOrderedSessions();
  const target = all[n - 1];
  if (!target) return;
  activateSession(target.id);
  requestAnimationFrame(() => {
    const card = document.querySelector(`.card[data-session-id="${target.id}"]`);
    card?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  });
}

document.addEventListener('keydown', (e) => {
  if (!e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
  const n = parseInt(e.key, 10);
  if (!(n >= 1 && n <= 9)) return;
  const active = document.activeElement;
  const tag = active?.tagName;
  const isOtherInput = (tag === 'INPUT' || tag === 'TEXTAREA') && active.id !== 'input';
  if (isOtherInput) return;
  e.preventDefault();
  jumpToSessionByIndex(n);
});

// ─── C5: セッションカードクリック — マルチタブ時のフォーカス切替 ──────────
export function onSessionCardActivate(id) {
  const multiView = document.getElementById('multi-view');
  const isMultiOpen = multiView && !multiView.hidden;
  if (isMultiOpen) {
    // マルチタブが開いているとき: スロット内セッションへのフォーカス移動
    const mgr = window.multiPaneManager;
    if (mgr) {
      const slotIdx = mgr.slots.findIndex(slot => slot && slot.session && slot.session.id === id);
      if (slotIdx >= 0) {
        mgr.focusSlot(slotIdx);
      }
      // スロット外（表示数超過）のセッションは何もしない
    }
    return;
  }
  // シングルビュー: 既存の動作
  activateSession(id);
}

export let _sessionListClickDelegated = false;
export let _sessionCardPointerDown = null;

export function renderSessionList() {
  const root = document.getElementById('sessions');
  const scrollEl = document.getElementById('session-list');
  // innerHTML クリアで scrollTop が 0 に戻るのを防ぐ。経過時間更新の 1Hz 再描画や
  // session_update 受信のたびにユーザのスクロール位置がトップへ吹き飛ぶのを避ける。
  const prevScrollTop = scrollEl ? scrollEl.scrollTop : 0;
  if (!_sessionListClickDelegated) {
    _sessionListClickDelegated = true;
    root.addEventListener('pointerdown', (e) => {
      if (e.button !== 0) return;
      const card = e.target.closest('.card');
      if (!card || e.target.closest('.card-actions') || e.target.closest('.card-branch')) {
        _sessionCardPointerDown = null;
        return;
      }
      _sessionCardPointerDown = {
        id: parseInt(card.dataset.sessionId, 10),
        x: e.clientX,
        y: e.clientY,
      };
    });
    root.addEventListener('pointerup', (e) => {
      const down = _sessionCardPointerDown;
      _sessionCardPointerDown = null;
      if (!down || isNaN(down.id)) return;
      const card = e.target.closest('.card');
      if (!card || e.target.closest('.card-actions') || e.target.closest('.card-branch')) return;
      const id = parseInt(card.dataset.sessionId, 10);
      if (id !== down.id) return;
      const moved = Math.hypot(e.clientX - down.x, e.clientY - down.y);
      if (moved <= 8) onSessionCardActivate(id);
    });
    root.addEventListener('click', (e) => {
      const card = e.target.closest('.card');
      if (!card || e.target.closest('.card-actions')) return;
      // branch バッジクリックは git タブ open（stopPropagation 役）
      const branchEl = e.target.closest('.card-branch');
      if (branchEl) {
        e.stopPropagation();
        if (branchEl.getAttribute('data-disabled') === 'true') return;
        const sid = parseInt(branchEl.dataset.sid, 10);
        if (isNaN(sid)) return;
        const sess = sessions.get(sid);
        if (!sess) return;
        const gr = sess.git_root || sess.cwd || '';
        if (!gr) return;
        activateSession(sid);
        FilesTabManager.openGitTab(sid, gr, sess.branch || '');
        return;
      }
      const id = parseInt(card.dataset.sessionId, 10);
      if (!isNaN(id)) onSessionCardActivate(id);
    });
    // branch バッジのキーボード操作 (Enter / Space)
    root.addEventListener('keydown', (e) => {
      const branchEl = e.target.closest && e.target.closest('.card-branch');
      if (!branchEl) return;
      if (e.key !== 'Enter' && e.key !== ' ' && e.key !== 'Spacebar') return;
      e.preventDefault();
      e.stopPropagation();
      if (branchEl.getAttribute('data-disabled') === 'true') return;
      const sid = parseInt(branchEl.dataset.sid, 10);
      if (isNaN(sid)) return;
      const sess = sessions.get(sid);
      if (!sess) return;
      const gr = sess.git_root || sess.cwd || '';
      if (!gr) return;
      activateSession(sid);
      FilesTabManager.openGitTab(sid, gr, sess.branch || '');
    });
    // カード右クリック context menu
    root.addEventListener('contextmenu', (e) => {
      const card = e.target.closest('.card');
      if (!card) return;
      const id = parseInt(card.dataset.sessionId, 10);
      if (isNaN(id)) return;
      e.preventDefault();
      openCardCtxMenu(e.clientX, e.clientY, id);
    });
  }
  root.innerHTML = '';
  if (sessions.size === 0) {
    const p = document.createElement('div');
    p.className = 'no-sessions';
    p.textContent = t('no_sessions');
    root.appendChild(p);
    return;
  }

  // セッションをプロジェクト別にグループ化
  const groups = new Map();
  getOrderedSessions().forEach(s => {
    const cwdStr = s.cwd || '';
    const name = cwdStr
      ? cwdStr.replace(/\\/g, '/').split('/').filter(p => p.length > 0).pop() || ''
      : '';
    const key = name || '__no_project__';
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(s);
  });

  // ピン留め優先、その中で groupOrder に従ってソート（未登録キーは末尾）
  const _projectFavIdx = new Map(projectFavorites.map((k, i) => [k, i]));
  const _groupOrderIdx = new Map(groupOrder.map((k, i) => [k, i]));
  const sortedGroupKeys = [...groups.keys()].sort((a, b) => {
    const aPin = _projectFavIdx.has(a);
    const bPin = _projectFavIdx.has(b);
    if (aPin !== bPin) return aPin ? -1 : 1;
    if (aPin && bPin) return _projectFavIdx.get(a) - _projectFavIdx.get(b);
    const ai = _groupOrderIdx.has(a) ? _groupOrderIdx.get(a) : -1;
    const bi = _groupOrderIdx.has(b) ? _groupOrderIdx.get(b) : -1;
    if (ai === -1 && bi === -1) return 0;
    if (ai === -1) return 1;
    if (bi === -1) return -1;
    return ai - bi;
  });

  sortedGroupKeys.forEach(key => {
    const groupSessions = groups.get(key);
    const projectDisplayName = key === '__no_project__' ? t('no_project') : key;
    const isCollapsed = collapsedGroups.has(key);
    const groupEl = document.createElement('div');
    groupEl.className = 'project-group' + (projectFavorites.includes(key) ? ' project-group--pinned' : '');

    const header = document.createElement('div');
    header.className = 'project-group-header' + (isCollapsed ? ' project-group-header--collapsed' : '');
    header.dataset.project = key;

    const chevron = document.createElement('span');
    chevron.className = 'project-group-chevron';
    chevron.textContent = '▼';
    header.appendChild(chevron);

    const nameSpan = document.createElement('span');
    nameSpan.textContent = projectDisplayName;
    header.appendChild(nameSpan);

    // Files 導線はステータスバーの 📁project セグメントへ移設（plan_statusbar-files-open.md C1）。
    // ここ（プロジェクトグループ header）の「📁 Files」ボタンは廃止した。
    if (key !== '__no_project__') {
      // C3: "Open running sessions in grid" ボタン
      const runningSessionsInGroup = groupSessions.filter(s => s.state === 'running' || s.state === 'waiting' || (s.state || 'standby') === 'standby');
      if (runningSessionsInGroup.length > 0) {
        const gridBtn = document.createElement('button');
        gridBtn.className = 'project-group-grid-btn';
        gridBtn.textContent = '⊞';
        gridBtn.title = t('ctx_open_project_in_grid');
        gridBtn.setAttribute('aria-label', t('ctx_open_project_in_grid'));
        gridBtn.addEventListener('click', (e) => {
          e.stopPropagation();
          openDetachedGridForSessions(runningSessionsInGroup.map(s => s.id));
        });
        header.appendChild(gridBtn);
      }
    }

    const runningCount  = groupSessions.filter(s => s.state === 'running').length;
    const waitingCount  = groupSessions.filter(s => s.state === 'waiting').length;
    const standbyCount  = groupSessions.filter(s => (s.state || 'standby') === 'standby').length;
    const chipsEl = document.createElement('span');
    chipsEl.className = 'group-status-chips';
    chipsEl.innerHTML =
      `<span class="status-chip status-chip--running">${runningCount}</span>` +
      `<span class="status-chip status-chip--waiting">${waitingCount}</span>` +
      `<span class="status-chip status-chip--standby">${standbyCount}</span>`;
    header.appendChild(chipsEl);

    // プロジェクト ☆/✕ ボタン
    const projActions = document.createElement('div');
    projActions.className = 'project-group-actions';

    const projStarBtn = document.createElement('button');
    const isProjFav = projectFavorites.includes(key);
    projStarBtn.className = 'star-btn' + (isProjFav ? ' starred' : '');
    projStarBtn.textContent = isProjFav ? '★' : '☆';
    projStarBtn.title = isProjFav ? t('project_favorite_remove') : t('project_favorite_add');
    projStarBtn.onclick = (e) => {
      e.stopPropagation();
      const idx = projectFavorites.indexOf(key);
      if (idx !== -1) { projectFavorites.splice(idx, 1); } else { projectFavorites.push(key); }
      saveProjectFavorites();
      renderSessionList();
    };
    projActions.appendChild(projStarBtn);

    const projXBtn = document.createElement('button');
    projXBtn.className = 'dismiss-btn';
    projXBtn.textContent = t('remove');
    projXBtn.title = t('project_dismiss');
    projXBtn.onclick = (e) => {
      e.stopPropagation();
      groupSessions.forEach(s => dismissSession(s.id));
    };
    projActions.appendChild(projXBtn);

    header.appendChild(projActions);

    header.addEventListener('click', () => {
      if (dragSrcGroupKey) return;
      if (collapsedGroups.has(key)) {
        collapsedGroups.delete(key);
      } else {
        collapsedGroups.add(key);
      }
      renderSessionList();
    });

    // グループD&D
    header.draggable = true;
    header.addEventListener('dragstart', (e) => {
      set_dragSrcGroupKey(key);
      set_dragSrcId(null);
      groupEl.classList.add('dragging');
      e.dataTransfer.effectAllowed = 'move';
      e.stopPropagation();
    });
    header.addEventListener('dragend', () => {
      set_dragSrcGroupKey(null);
      if (dragOverGroupEl) { dragOverGroupEl.classList.remove('drag-over'); set_dragOverGroupEl(null); }
      root.querySelectorAll('.project-group').forEach(el => el.classList.remove('dragging'));
    });
    groupEl.addEventListener('dragover', (e) => {
      if (!dragSrcGroupKey || dragSrcGroupKey === key) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = 'move';
      if (dragOverGroupEl !== groupEl) {
        if (dragOverGroupEl) dragOverGroupEl.classList.remove('drag-over');
        set_dragOverGroupEl(groupEl);
        groupEl.classList.add('drag-over');
      }
    });
    groupEl.addEventListener('dragleave', (e) => {
      if (!groupEl.contains(e.relatedTarget)) groupEl.classList.remove('drag-over');
    });
    groupEl.addEventListener('drop', (e) => {
      e.preventDefault();
      groupEl.classList.remove('drag-over');
      if (!dragSrcGroupKey || dragSrcGroupKey === key) return;
      const srcKey = dragSrcGroupKey;
      set_dragSrcGroupKey(null);
      if (!groupOrder.length) set_groupOrder([...sortedGroupKeys]);
      sortedGroupKeys.forEach(k => { if (!groupOrder.includes(k)) groupOrder.push(k); });
      const srcIdx = groupOrder.indexOf(srcKey);
      const dstIdx = groupOrder.indexOf(key);
      if (srcIdx !== -1) groupOrder.splice(srcIdx, 1);
      groupOrder.splice(dstIdx, 0, srcKey);
      saveGroupOrder();
      renderSessionList();
    });

    groupEl.appendChild(header);

    const body = document.createElement('div');
    body.className = 'project-group-body' + (isCollapsed ? ' hidden' : '');

    groupSessions.forEach(s => {
      const c = document.createElement('div');
      const state = s.state || 'standby';
      const stateClass = (state === 'running' || state === 'waiting') ? ` ${state}` : '';
      c.className = 'card' + stateClass + (s.id === activeSessionId ? ' active' : '');
      c.tabIndex = isCollapsed ? -1 : 0;
      const label = stateLabel(state);
      const filteredMsg = filterFirstMessage(s.last_message || s.first_message || '');
      const cwdStr = s.cwd || '';
      const sessionLabel = s.label ? `<span class="card-label">[${escapeHtml(s.label)}]</span>` : '';
      const msgHtml = filteredMsg
        ? `<span class="card-msg" data-tooltip="${escapeHtml(filteredMsg)}">${escapeHtml(filteredMsg)}</span>`
        : `<span class="card-msg"></span>`;
      const providerName = providerDisplayName(s.provider);
      const providerChipHtml = providerName ? `<span class="card-provider-chip ${safeClassToken(s.provider)}">${escapeHtml(providerName)}</span>` : '';
      const isDeadState = state === 'error' || state === 'disconnected';
      let reasonText = '';
      if (isDeadState && s.end_reason) {
        const key = 'end_reason_' + s.end_reason;
        const translated = window.t(key, { provider: providerName || s.provider || '' });
        // 未知の reason コードは window.t がキー文字列をそのまま返すため、その場合は非表示にする。
        if (translated !== key) reasonText = translated;
      }
      const reasonHtml = reasonText
        ? `<span class="card-end-reason" data-tooltip="${escapeHtml(reasonText)}">${escapeHtml(reasonText)}</span>`
        : '';
      // 状態は title-row の #N 直後（ステータスバーと同じ並び）へ移したため、meta-row からは外す。
      const metaRow = `<div class="card-meta-row">${reasonHtml}${sessionLabel}${msgHtml}</div>`;
      c.dataset.sessionId = s.id;
      const isOllamaBackedSess = (s.route === 'ollama');
      let modelBadge = '';
      if (s.model) {
        const badgeProviderKey = isOllamaBackedSess ? 'ollama' : (s.provider || '');
        const badgeProviderLabel = isOllamaBackedSess ? 'Ollama' : providerName;
        const tip = badgeProviderLabel ? `${badgeProviderLabel} · ${s.model}` : s.model;
        modelBadge = ` <span class="card-model card-model--with-icon" data-tooltip="${escapeHtml(tip)}">${providerIconHtml(badgeProviderKey)}<span class="card-model-text">${escapeHtml(s.model)}</span></span>`;
      }
      const branchStr = s.branch || '';
      const branchTip = branchStr
        ? ti18n('card_branch_tooltip', `Open Git view (${branchStr})`, { branch: branchStr })
        : ti18n('card_branch_disabled_tooltip', 'No git repository');
      const branchDisabledAttr = branchStr ? '' : ' data-disabled="true"';
      // 空 branch でも常に span を表示（git 外であることが分かるよう "(no git)" を表示）
      const branchLabel = branchStr || ti18n('card_branch_no_git', '(no git)');
      const branchBadge = ` <span class="card-branch" role="button" tabindex="0" data-sid="${s.id}"${branchDisabledAttr} data-tooltip="${escapeHtml(branchTip)}" aria-label="${escapeHtml(branchTip)}">${escapeHtml(branchLabel)}</span>`;
      // 状態 pill（ステータスバー .tsb-pill と同じ ●ドット付き形状）。並び順も下のバーに合わせ #N の直後に置く。
      const statePillHtml = ` <span class="card-state-pill ${safeClassToken(state)}"><span class="card-pdot"></span><span class="card-state-text">${escapeHtml(label)}</span></span>`;
      c.innerHTML =
        `<div class="card-title-row"><b>#${s.id}</b>${statePillHtml} ${providerIconHtml(s.provider)} ${providerChipHtml}${modelBadge}${branchBadge}</div>` +
        metaRow;

      const actions = document.createElement('div');
      actions.className = 'card-actions';
      c.appendChild(actions);

      // ☆/★ボタン
      const starBtn = document.createElement('button');
      const isFav = favorites.includes(s.id);
      starBtn.className = 'star-btn' + (isFav ? ' starred' : '');
      starBtn.textContent = isFav ? '★' : '☆';
      starBtn.title = isFav ? t('favorite_remove') : t('favorite_add');
      starBtn.onclick = (e) => {
        e.stopPropagation();
        const idx = favorites.indexOf(s.id);
        if (idx !== -1) { favorites.splice(idx, 1); } else { favorites.push(s.id); }
        saveFavorites();
        // C5: マルチタブが開いているときはペインスロット順も更新
        // render() → renderSessionList() の再帰を防ぐため _c5SidebarUpdating フラグを立てておく
        const _mv = document.getElementById('multi-view');
        if (_mv && !_mv.hidden && window.multiPaneManager) {
          window._c5SidebarUpdating = true;
          try { window.multiPaneManager.render(); }
          finally { window._c5SidebarUpdating = false; }
        }
        renderSessionList();
      };
      actions.appendChild(starBtn);


      const resetBtn = document.createElement('button');
      resetBtn.className = 'session-history-reset-btn';
      resetBtn.textContent = '↺';
      resetBtn.title = t('session_history_reset_tooltip');
      resetBtn.onclick = (e) => {
        e.stopPropagation();
        requestSessionHistoryReset(s.id);
      };
      actions.appendChild(resetBtn);

      const xBtn = document.createElement('button');
      xBtn.className = 'dismiss-btn';
      xBtn.textContent = t('remove');
      xBtn.title = t('dismiss_session');
      xBtn.onclick = (e) => { e.stopPropagation(); dismissSession(s.id); };
      actions.appendChild(xBtn);

      // D&Dドラッグ順序変更
      c.draggable = true;
      c.addEventListener('dragstart', (e) => {
        set_dragSrcId(s.id);
        c.classList.add('dragging');
        e.dataTransfer.effectAllowed = 'move';
      });
      c.addEventListener('dragend', () => {
        c.classList.remove('dragging');
        if (dragOverCardEl) {
          dragOverCardEl.classList.remove('drag-over', 'drop-before', 'drop-after');
          set_dragOverCardEl(null);
        }
        setTimeout(() => inputEl.focus(), 0);
      });
      c.addEventListener('dragover', (e) => {
        if (dragSrcGroupKey) { e.dataTransfer.dropEffect = 'none'; return; }
        if (!dragSrcId || dragSrcId === s.id) return;
        // C5: グループ跨ぎドロップを許可（★→非★ / 非★→★ の自動切替）
        e.preventDefault();
        e.dataTransfer.dropEffect = 'move';
        if (dragOverCardEl !== c) {
          if (dragOverCardEl) dragOverCardEl.classList.remove('drag-over', 'drop-before', 'drop-after');
          set_dragOverCardEl(c);
        }
        // C5: 上半分 → drop-before、下半分 → drop-after
        const rect = c.getBoundingClientRect();
        const pos = e.clientY < rect.top + rect.height / 2 ? 'before' : 'after';
        c.classList.remove('drag-over', 'drop-before', 'drop-after');
        c.classList.add(pos === 'before' ? 'drop-before' : 'drop-after');
      });
      c.addEventListener('dragleave', () => {
        c.classList.remove('drag-over', 'drop-before', 'drop-after');
      });
      c.addEventListener('drop', (e) => {
        e.preventDefault();
        c.classList.remove('drag-over', 'drop-before', 'drop-after');
        if (!dragSrcId || dragSrcId === s.id) return;
        const srcId = dragSrcId;
        set_dragSrcId(null);
        // C5: ドロップ位置（上半分=before / 下半分=after）を判定
        const rect = c.getBoundingClientRect();
        const dropAfter = e.clientY >= rect.top + rect.height / 2;
        const srcIsStarred = favorites.includes(srcId);
        const dstIsStarred = favorites.includes(s.id);
        if (srcIsStarred === dstIsStarred) {
          // 同一グループ内の並び替え
          if (srcIsStarred) {
            const srcIdx = favorites.indexOf(srcId);
            let dstIdx = favorites.indexOf(s.id);
            favorites.splice(srcIdx, 1);
            // srcIdx 削除後に dstIdx がずれる場合を補正
            if (srcIdx < dstIdx) dstIdx--;
            const insertAt = dropAfter ? dstIdx + 1 : dstIdx;
            favorites.splice(insertAt, 0, srcId);
            saveFavorites();
          } else {
            if (!sessionOrder.includes(srcId)) sessionOrder.push(srcId);
            if (!sessionOrder.includes(s.id)) sessionOrder.push(s.id);
            const srcIdx = sessionOrder.indexOf(srcId);
            let dstIdx = sessionOrder.indexOf(s.id);
            sessionOrder.splice(srcIdx, 1);
            if (srcIdx < dstIdx) dstIdx--;
            const insertAt = dropAfter ? dstIdx + 1 : dstIdx;
            sessionOrder.splice(insertAt, 0, srcId);
            saveSessionOrder();
          }
        } else {
          // C5: グループ跨ぎ → ★/非★ を自動切替
          if (srcIsStarred) {
            // ★ → 非★グループ: favorites から除外し、sessionOrder の dstId 位置に挿入
            const favIdx = favorites.indexOf(srcId);
            if (favIdx !== -1) favorites.splice(favIdx, 1);
            saveFavorites();
            if (!sessionOrder.includes(srcId)) sessionOrder.push(srcId);
            if (!sessionOrder.includes(s.id)) sessionOrder.push(s.id);
            const srcSoIdx = sessionOrder.indexOf(srcId);
            let dstSoIdx = sessionOrder.indexOf(s.id);
            sessionOrder.splice(srcSoIdx, 1);
            if (srcSoIdx < dstSoIdx) dstSoIdx--;
            const insertAt = dropAfter ? dstSoIdx + 1 : dstSoIdx;
            sessionOrder.splice(insertAt, 0, srcId);
            saveSessionOrder();
          } else {
            // 非★ → ★グループ: favorites の dstId 位置に挿入し、sessionOrder から除外
            const soIdx = sessionOrder.indexOf(srcId);
            if (soIdx !== -1) sessionOrder.splice(soIdx, 1);
            saveSessionOrder();
            let dstFavIdx = favorites.indexOf(s.id);
            const insertAt = dropAfter ? dstFavIdx + 1 : dstFavIdx;
            if (dstFavIdx !== -1) {
              favorites.splice(insertAt, 0, srcId);
            } else {
              favorites.push(srcId);
            }
            saveFavorites();
          }
        }
        // C5: マルチタブが開いているときはペインスロット順も更新
        // render() → renderSessionList() の再帰を防ぐため _c5SidebarUpdating フラグを立てておく
        const _mv = document.getElementById('multi-view');
        if (_mv && !_mv.hidden && window.multiPaneManager) {
          window._c5SidebarUpdating = true;
          try { window.multiPaneManager.render(); }
          finally { window._c5SidebarUpdating = false; }
        }
        renderSessionList();
      });

      // C5: マルチタブが開いているとき、スロット内セッションに P<n> バッジを追加
      const multiView = document.getElementById('multi-view');
      const isMultiOpen = multiView && !multiView.hidden;
      if (isMultiOpen) {
        const mgr = window.multiPaneManager;
        if (mgr && mgr.slots) {
          const slotIdx = mgr.slots.findIndex(slot => slot && slot.session && slot.session.id === s.id);
          if (slotIdx >= 0) {
            c.classList.add('in-pane');
          }
        }
      }

      body.appendChild(c);
    });

    groupEl.appendChild(body);
    root.appendChild(groupEl);
  });

  // C5: ★/非★ グループ区切りラベルをサイドバーに追加
  // （プロジェクトグループ化の後で、全体の先頭付近に挿入する）
  _addSidebarGroupLabels(root);

  if (scrollEl) {
    const max = scrollEl.scrollHeight - scrollEl.clientHeight;
    scrollEl.scrollTop = Math.max(0, Math.min(prevScrollTop, max));
  }

  updateMainTabStatus();
}

// C5: ★/非★ グループ区切りラベルをサイドバー（#sessions）に挿入する。
// renderSessionList() 後に呼ばれる。favorites に含まれるセッションを持つグループの
// カードに .is-favorite-group クラスを付け、最初の非★グループの直前に区切りを挿入する。
export function _addSidebarGroupLabels(root) {
  if (!root) return;
  const hasAnyFav = favorites.length > 0 && Array.from(sessions.keys()).some(id => favorites.includes(id));
  if (!hasAnyFav) return;
  // 全 .card を走査して最初の非★カードの直前に区切りを挿入する。
  // プロジェクトグループ構造の中に挿入するため、最初の非★セッションを含む
  // .project-group-body を特定して、その直前（project-group 要素）の直前に区切りを置く。
  const cards = root.querySelectorAll('.card');
  let firstNonFavCard = null;
  let firstFavCard = null;
  for (const card of cards) {
    const sid = parseInt(card.dataset.sessionId, 10);
    if (isNaN(sid)) continue;
    if (favorites.includes(sid)) {
      if (!firstFavCard) firstFavCard = card;
    } else {
      if (!firstNonFavCard) firstNonFavCard = card;
    }
  }
  if (!firstFavCard || !firstNonFavCard) return;

  // ★グループラベル: #sessions の最初の子要素の直前に挿入
  const firstChild = root.firstElementChild;
  if (firstChild) {
    const starLabel = document.createElement('div');
    starLabel.className = 'sidebar-group-label';
    const starText = document.createElement('span');
    starText.textContent = '★ ' + (window.t ? window.t('sidebar_favorites', 'Favorites') : 'Favorites');
    starLabel.appendChild(starText);

    // C3: ★ セッションを別窓 Grid で開くボタン
    const favGridBtn = document.createElement('button');
    favGridBtn.className = 'sidebar-group-label-grid-btn';
    favGridBtn.type = 'button';
    favGridBtn.textContent = '⊞';
    const favGridLabel = window.t ? window.t('ctx_open_selected_in_grid', 'Open selected in grid') : 'Open selected in grid';
    favGridBtn.title = favGridLabel;
    favGridBtn.setAttribute('aria-label', favGridLabel);
    favGridBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      const activeFavIds = favorites.filter(id => sessions.has(id));
      openDetachedGridForSessions(activeFavIds);
    });
    starLabel.appendChild(favGridBtn);

    root.insertBefore(starLabel, firstChild);
  }

  // 非★グループラベル: 最初の非★カードの所属する .project-group の直前に挿入
  const nonFavGroup = firstNonFavCard.closest('.project-group');
  if (nonFavGroup) {
    const otherLabel = document.createElement('div');
    otherLabel.className = 'sidebar-group-label';
    otherLabel.textContent = window.t ? window.t('sidebar_others', 'Others') : 'Others';
    root.insertBefore(otherLabel, nonFavGroup);
  }
}

export function updateMainTabStatus() {
  // D11: セッション情報チップも同タイミングで更新 (state badge を反映)
  if (typeof renderSessionInfoChip === 'function') renderSessionInfoChip();
}

// ---- タブ通知（保留バッジ） ----

export let _faviconCanvas = null;
export let _faviconCtx = null;
export let _faviconBaseImg = null;
export let _faviconBaseLoaded = false;
export let _faviconPendingCount = 0;
export let _faviconRenderedPendingCount = null;
// 環境略号（L/W/V/T）。タイトルに出さず favicon に焼き込む。
export let _faviconEnvShort = '';
export let _faviconEnvColor = '';

export let _titleBlinkInterval = null;
export let _titleBlinkState = false;
export let _titleBlinkCount = 0;

export function startTitleBlink(pendingCount) {
  if (_titleBlinkInterval && _titleBlinkCount === pendingCount) return;
  stopTitleBlink();
  _titleBlinkCount = pendingCount;
  _titleBlinkState = true;
  _titleBlinkInterval = setInterval(() => {
    _titleBlinkState = !_titleBlinkState;
    document.title = _titleBlinkState ? `(${_titleBlinkCount}) MANY-AI-CLI` : 'MANY-AI-CLI';
  }, 800);
}

export function stopTitleBlink() {
  if (_titleBlinkInterval) {
    clearInterval(_titleBlinkInterval);
    _titleBlinkInterval = null;
  }
}

export function initFaviconCanvas() {
  if (_faviconCanvas) return;
  _faviconCanvas = document.createElement('canvas');
  _faviconCanvas.width = 32;
  _faviconCanvas.height = 32;
  _faviconCtx = _faviconCanvas.getContext('2d');
  _faviconBaseImg = new Image();
  _faviconBaseImg.onload = () => {
    _faviconBaseLoaded = true;
    drawFavicon(_faviconPendingCount, true);
  };
  _faviconBaseImg.src = '/icon.svg';
}

export function drawFavicon(pendingCount, force = false) {
  initFaviconCanvas();
  if (!force && _faviconRenderedPendingCount === pendingCount) return;
  const ctx = _faviconCtx;
  const SIZE = 32;
  ctx.clearRect(0, 0, SIZE, SIZE);
  // ベースは常に CLI ロゴ。環境（local/wsl/remote…）が設定されているときは、
  // ロゴを潰さずに済むよう略号 1 文字のバッジを左下隅へ小さく重ねる。
  // 左右半分ずつ並べる旧方式は中央で重なって見えた（「センターマン」）ため隅オーバーレイに変更。
  if (_faviconBaseLoaded) {
    ctx.drawImage(_faviconBaseImg, 0, 0, SIZE, SIZE);
  }
  if (_faviconEnvShort && _faviconEnvColor) {
    const BW = 18, BH = 18;           // バッジ箱（32 のうち左下に配置）
    const bx = 0, by = SIZE - BH;
    const R = 5;
    // ロゴから分離して見えるよう白縁を一段敷く
    ctx.fillStyle = '#ffffff';
    ctx.beginPath();
    ctx.roundRect(bx, by, BW, BH, R);
    ctx.fill();
    ctx.fillStyle = _faviconEnvColor;
    ctx.beginPath();
    ctx.roundRect(bx + 1.5, by + 1.5, BW - 3, BH - 3, R - 1);
    ctx.fill();
    ctx.fillStyle = '#ffffff';
    ctx.font = 'bold 13px sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(_faviconEnvShort, bx + BW / 2, by + BH / 2 + 1);
  }

  if (pendingCount > 0) {
    const R = 9;
    const cx = SIZE - R, cy = R;
    ctx.beginPath();
    ctx.arc(cx, cy, R, 0, Math.PI * 2);
    ctx.fillStyle = '#ef4444';
    ctx.fill();
    ctx.fillStyle = '#ffffff';
    ctx.font = `bold ${pendingCount > 9 ? 10 : 12}px sans-serif`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(pendingCount > 9 ? '9+' : String(pendingCount), cx, cy);
  }

  let link = document.querySelector("link[rel~='icon']");
  if (!link) { link = document.createElement('link'); link.rel = 'icon'; document.head.appendChild(link); }
  link.type = 'image/png';
  link.href = _faviconCanvas.toDataURL('image/png');
  _faviconRenderedPendingCount = pendingCount;
}

// 環境略号バッジ（L/W/V/T）を favicon に焼き込む。タイトル文字列には出さず favicon 側で表現する。
export function setFaviconEnvBadge(short, color) {
  _faviconEnvShort = String(short || '').trim();
  _faviconEnvColor = String(color || '').trim();
  drawFavicon(_faviconPendingCount, true);
}

export function updateTabNotification(pendingCount) {
  const faviconChanged = _faviconPendingCount !== pendingCount;
  _faviconPendingCount = pendingCount;

  if (pendingCount > 0) {
    startTitleBlink(pendingCount);
  } else {
    stopTitleBlink();
    document.title = 'MANY-AI-CLI';
  }

  if (faviconChanged) drawFavicon(pendingCount);
}

export let _summaryResizeObserver = null;
export function ensureSummaryResizeObserver() {
  if (_summaryResizeObserver) return;
  const el = document.getElementById('summary');
  if (!el) return;
  _summaryResizeObserver = new ResizeObserver(() => updateSummaryCompactMode());
  _summaryResizeObserver.observe(el);
  const parent = el.parentElement;
  if (parent) _summaryResizeObserver.observe(parent);
}

export function updateSummaryCompactMode() {
  const el = document.getElementById('summary');
  if (!el) return;
  // scrollWidth > clientWidth は #summary が flex-wrap:nowrap + overflow:hidden の前提でのみ意味を持つ。
  el.classList.remove('summary--compact');
  const overflow = el.scrollWidth > el.clientWidth + 1;
  if (overflow) el.classList.add('summary--compact');
}

export function renderSummaryAndNotifications() {
  const stateCounts = { running: 0, waiting: 0, standby: 0 };
  // groupKey -> { provider, model, isOllamaBacked, count }
  // Ollama backend sessions are split per-model so each model gets its own chip.
  const providerGroups = new Map();
  sessions.forEach(s => {
    const provider = s.provider || 'unknown';
    const route = s.route || '';
    const model = s.model || '';
    const isOllamaBacked = route === 'ollama';
    const key = isOllamaBacked ? `ollama::${model}` : provider;
    const g = providerGroups.get(key);
    if (g) g.count++;
    else providerGroups.set(key, { provider, model, isOllamaBacked, count: 1 });
    const st = s.state || 'standby';
    if (st === 'running') stateCounts.running++;
    else if (st === 'waiting') stateCounts.waiting++;
    else stateCounts.standby++;
  });
  const totalWaiting = stateCounts.waiting;

  const PROVIDER_ORDER = { claude: 0, ollama: 1, codex: 2, copilot: 3, opencode: 4, 'cursor-agent': 5 };
  const sortedGroups = Array.from(providerGroups.values()).sort((a, b) => {
    const ka = a.isOllamaBacked ? 'ollama' : a.provider;
    const kb = b.isOllamaBacked ? 'ollama' : b.provider;
    const oa = ka in PROVIDER_ORDER ? PROVIDER_ORDER[ka] : 99;
    const ob = kb in PROVIDER_ORDER ? PROVIDER_ORDER[kb] : 99;
    if (oa !== ob) return oa - ob;
    return (a.model || '').localeCompare(b.model || '');
  });
  const providerParts = sortedGroups.map(g => {
    if (g.isOllamaBacked) {
      const label = providerDisplayName('ollama');
      const modelHtml = g.model ? `<span class="summary-ollama-model">${escapeHtml(g.model)}</span>` : '';
      const tip = g.model ? `Ollama · ${g.model} : ${g.count}` : `Ollama : ${g.count}`;
      return `<span class="summary-provider-chip" data-tooltip="${escapeHtml(tip)}">${providerIconHtml('ollama')}<span class="compact-hide"><span class="summary-provider-name ollama">${escapeHtml(label)}</span>${modelHtml}<span class="summary-provider-count">: ${g.count}</span></span><span class="compact-count">${g.count}</span></span>`;
    }
    const provider = g.provider;
    const label = providerDisplayName(provider);
    const tip = `${label} : ${g.count}`;
    return `<span class="summary-provider-chip" data-tooltip="${escapeHtml(tip)}">${providerIconHtml(provider)}<span class="compact-hide"><span class="summary-provider-name ${safeClassToken(provider)}">${escapeHtml(label)}</span><span class="summary-provider-count">: ${g.count}</span></span><span class="compact-count">${g.count}</span></span>`;
  }).join('');

  let summary = '';
  if (stateCounts.running > 0) {
    summary += `<span class="session-chip running"><span class="chip-dot"></span>${stateCounts.running} running</span>`;
  }
  if (stateCounts.waiting > 0) {
    summary += `<span class="session-chip waiting"><span class="chip-dot"></span>${stateCounts.waiting} waiting</span>`;
  }
  if (stateCounts.standby > 0) {
    summary += `<span class="session-chip standby"><span class="chip-dot"></span>${stateCounts.standby} standby</span>`;
  }
  if (providerParts) summary += `<span class="summary-sep">|</span>${providerParts}`;
  document.getElementById('summary').innerHTML = summary;
  ensureSummaryResizeObserver();
  updateSummaryCompactMode();
  updateTabNotification(totalWaiting);
}

export function sessionProjectKey(s) {
  const cwdStr = s?.cwd || '';
  const name = cwdStr
    ? cwdStr.replace(/\\/g, '/').split('/').filter(p => p.length > 0).pop() || ''
    : '';
  return name || '__no_project__';
}

export function updateProjectGroupStatusChipsForSession(s) {
  const root = document.getElementById('sessions');
  if (!root || !s) return false;
  const key = sessionProjectKey(s);
  const header = root.querySelector(`.project-group-header[data-project="${CSS.escape(key)}"]`);
  const chipsEl = header ? header.querySelector('.group-status-chips') : null;
  if (!chipsEl) return false;
  const groupSessions = getOrderedSessions().filter(sess => sessionProjectKey(sess) === key);
  const counts = { running: 0, waiting: 0, standby: 0 };
  groupSessions.forEach(sess => {
    const state = sess.state || 'standby';
    if (state === 'running') counts.running++;
    else if (state === 'waiting') counts.waiting++;
    else counts.standby++;
  });
  const running = chipsEl.querySelector('.status-chip--running');
  const waiting = chipsEl.querySelector('.status-chip--waiting');
  const standby = chipsEl.querySelector('.status-chip--standby');
  if (running) running.textContent = String(counts.running);
  if (waiting) waiting.textContent = String(counts.waiting);
  if (standby) standby.textContent = String(counts.standby);
  return true;
}

export function updateSessionCardStateInPlace(id) {
  const root = document.getElementById('sessions');
  const s = sessions.get(id);
  if (!root || !s) return false;
  const card = root.querySelector(`.card[data-session-id="${CSS.escape(String(id))}"]`);
  if (!card) return false;
  const state = s.state || 'standby';
  card.classList.toggle('running', state === 'running');
  card.classList.toggle('waiting', state === 'waiting');
  card.classList.toggle('active', id === activeSessionId);
  const pill = card.querySelector('.card-state-pill');
  if (pill) {
    pill.className = `card-state-pill ${safeClassToken(state)}`;
    const txt = pill.querySelector('.card-state-text');
    if (txt) txt.textContent = stateLabel(state);
  }
  updateProjectGroupStatusChipsForSession(s);
  return true;
}

export function renderSessionStateUpdate(id) {
  renderSummaryAndNotifications();
  if (typeof window.syncMobileLayoutState === 'function') window.syncMobileLayoutState();
  const updated = updateSessionCardStateInPlace(id);
  updateMainTabStatus();
  if (typeof syncElapsedTimer === 'function') syncElapsedTimer();
  return updated;
}

export function render() {
  renderSummaryAndNotifications();
  if (typeof window.syncMobileLayoutState === 'function') window.syncMobileLayoutState();

  // C9: 初回アクティブ化の早期 return を明示。まだアクティブセッションが無く候補が
  // 現れた場合、最初のセッションをアクティブ化してこのパスを終える。activateSession()
  // 自身がアクティブカード描画（updateSessionListActiveCard）まで行うため、ここで
  // renderSessionList() は呼ばない（フル描画は次の render() 呼び出しで追従する）。
  // activateSession() は render() を再帰呼び出ししない（DOM を直接更新する）点に注意。
  if (activeSessionId === null && sessions.size > 0) {
    activateSession(sessions.keys().next().value);
    return;
  }
  renderSessionList();
  // C5: マルチタブが開いているときはペインスロット配列も更新
  // （セッション削除などで slots が古くなった場合に P<n> バッジと整合させる）
  const _mv5 = document.getElementById('multi-view');
  if (_mv5 && !_mv5.hidden && window.multiPaneManager && !window._c5SidebarUpdating) {
    window._c5SidebarUpdating = true;
    try { window.multiPaneManager.render(); }
    finally { window._c5SidebarUpdating = false; }
  }
  if (typeof syncElapsedTimer === 'function') syncElapsedTimer();
}

// ─── C3: Detached Grid 導線ヘルパー ──────────────────────────────────────

/**
 * session 数からグリッドレイアウト文字列を自動算出する。
 * 1→1x1, 2→1x2, 3-4→2x2, 5-6→2x3, 7-9→3x3, 10-12→4x3, 13-18→6x3
 */
export function calcDetachedLayout(count: number): string {
  if (count <= 1) return '1x1';
  if (count <= 2) return '1x2';
  if (count <= 4) return '2x2';
  if (count <= 6) return '2x3';
  if (count <= 9) return '3x3';
  if (count <= 12) return '4x3';
  return '6x3';
}

/**
 * 指定した session id 群を Detached Grid で別窓に開く。
 * layout は count から自動算出。
 */
export function openDetachedGridForSessions(sessionIds: number[]): void {
  if (sessionIds.length === 0) return;
  const layout = calcDetachedLayout(sessionIds.length);
  const params = new URLSearchParams(window.location.search);
  const tokenVal = params.get('token') || token;
  const idsStr = sessionIds.join(',');
  const url = `/?view=detached-grid&layout=${encodeURIComponent(layout)}&session_ids=${idsStr}&token=${tokenVal}`;
  window.open(url, '_blank');
}
