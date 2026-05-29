// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- セッション管理 ----

function updateSessionListActiveCard(id) {
  const root = document.getElementById('sessions');
  if (!root) return;
  root.querySelectorAll('.card').forEach(card => {
    const cid = parseInt(card.dataset.sessionId, 10);
    card.classList.toggle('active', cid === id);
  });
}

// C4: focusSlot → activateSession → focusSlot の無限ループ防止フラグ
let _multiPaneFocusSyncing = false;

/**
 * C4: マルチペインのフォーカス切替用の軽量版 activateSession。
 * シングルビュー固有の処理（ensureTerminal / attachTerminal / FilesTabManager 等）は
 * 行わず、activeSessionId の更新と承認 UI 検出・サイドバー更新のみ実施する。
 * multi-pane.js の focusSlot() から呼ばれる。
 */
function activateSessionForMultiPane(id) {
  if (activeSessionId !== null && activeSessionId !== id) {
    saveInputStateFor(activeSessionId);
  }
  activeSessionId = id;
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
  detectApproval(id);
  revealApprovalPromptForSession(id);
  // サイドバーのアクティブカードを更新
  updateSessionListActiveCard(id);
  // チャット件数バッジ・セッション情報チップも更新
  if (typeof updateChatCountBadge === 'function') updateChatCountBadge();
  if (typeof renderSessionInfoChip === 'function') renderSessionInfoChip();
  if (typeof syncElapsedTimer === 'function') syncElapsedTimer();
}
// multi-pane.js から参照できるよう window に公開
window.activateSessionForMultiPane = activateSessionForMultiPane;

function activateSession(id) {
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
  activeSessionId = id;
  restoreInputStateFor(id);
  // files/git 表示からセッションカードへ戻る場合、先にターミナルを表示してから
  // attach/fit/detect しないと、承認 UI 検出と最下部スナップが hidden レイアウトを基準に走る。
  FilesTabManager.switchToSessionView();
  ensureTerminal(id);
  attachTerminal(id);
  updateScrollLockBtn();
  setMultiQuestionBannerVisible(!!multiQuestionVisibleCache.get(id));
  detectApproval(id);
  updateSessionListActiveCard(id);
  renderToolOutputs(id);
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

function updateShellBadge(id) {
  const el = document.getElementById('terminal-shell-info');
  if (!el) return;
  const s = id !== null ? sessions.get(id) : null;
  const shell = s?.shell || '';
  el.textContent = shell ? ' · ' + shell : '';
}

// provider 別の quick コマンドボタン制御。
// Ollama REPL は `/model` を持たず（近いのは `/load`）、Hub 側のスラッシュピッカーも
// Claude/Codex 専用候補で構成されているため、Ollama セッションでは両方を非活性化する。
// `/clear` は Ollama REPL にも実在し意味も一致するため活性のまま残す。
function updateQuickCmdButtons(id) {
  const s = id !== null ? sessions.get(id) : null;
  const provider = s?.provider || '';
  const isOllama = provider === 'ollama';
  const modelBtn  = document.getElementById('quick-model-btn');
  const pickerBtn = document.getElementById('slash-picker-btn');
  for (const btn of [modelBtn, pickerBtn]) {
    if (!btn) continue;
    btn.disabled = isOllama;
    if (isOllama) btn.setAttribute('aria-disabled', 'true');
    else btn.removeAttribute('aria-disabled');
  }
}

function stateLabel(state) {
  return t('state_' + state) || state;
}

function providerIconHtml(provider, size = 16) {
  const base = `class="card-provider-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="${size}" height="${size}" aria-hidden="true"`;
  const txt  = `text-anchor="middle" dominant-baseline="central" font-size="7.5" font-weight="bold" font-family="sans-serif"`;
  if (provider === 'claude') {
    return `<svg ${base}><circle cx="8" cy="8" r="6" fill="#FFF7ED" stroke="#F97316" stroke-width="2"/><text x="8" y="8" ${txt} fill="#F97316">C</text></svg>`;
  }
  if (provider === 'codex') {
    return `<svg ${base}><circle cx="8" cy="8" r="6" fill="#EFF6FF" stroke="#3B82F6" stroke-width="2"/><text x="8" y="8" ${txt} fill="#3B82F6">X</text></svg>`;
  }
  if (provider === 'ollama') {
    return `<svg ${base}><rect x="1" y="1" width="14" height="14" rx="3" fill="#FDF6E3" stroke="#C4973A" stroke-width="2"/><text x="8" y="8" ${txt} fill="#C4973A">O</text></svg>`;
  }
  if (provider === 'opencode') {
    return `<svg ${base}><rect x="1" y="1" width="14" height="14" rx="3" fill="#FAF5FF" stroke="#A855F7" stroke-width="2"/><text x="8" y="8" ${txt} fill="#A855F7">O</text></svg>`;
  }
  const letter = (provider || '?')[0].toUpperCase();
  return `<svg ${base}><circle cx="8" cy="8" r="6" fill="#F3F4F6" stroke="#6B7280" stroke-width="2"/><text x="8" y="8" ${txt} fill="#6B7280">${letter}</text></svg>`;
}

// C9: 整列ロジックは state.js の orderSessions に集約。getOrderedSessions は後方互換の薄い委譲。
function getOrderedSessions() {
  return orderSessions();
}

function switchSessionByTab(shift) {
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
    actionBarFocusIdx = -1;
  }
}

function jumpToSessionByIndex(n) {
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
function onSessionCardActivate(id) {
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

let _sessionListClickDelegated = false;
let _sessionCardPointerDown = null;

function renderSessionList() {
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

    // v2: files ボタン（__no_project__ 以外のプロジェクトにのみ表示）
    if (key !== '__no_project__') {
      const filesBtn = document.createElement('button');
      filesBtn.className = 'project-group-files-btn';
      filesBtn.textContent = '📁 files';
      filesBtn.title = t('files_group_btn_tooltip');
      filesBtn.addEventListener('click', async (e) => {
        e.stopPropagation();
        // セッション ID（グループの最初のアクティブセッション）
        const firstSession = groupSessions[0];
        const sessionId = firstSession ? firstSession.id : null;
        // セッションの cwd（= UI 上のプロジェクト直下）を直接開く。
        // /api/files-roots の gitRoot は cwd の親方向探索結果なので、
        // 親側に別の .git があるとプロジェクト外を指してしまう。ここでは使わない。
        const rootToOpen = firstSession ? firstSession.cwd : null;
        if (rootToOpen) {
          FilesTabManager.openFilesTab(sessionId, key, rootToOpen, rootToOpen);
        }
      });
      header.appendChild(filesBtn);
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
      dragSrcGroupKey = key;
      dragSrcId = null;
      groupEl.classList.add('dragging');
      e.dataTransfer.effectAllowed = 'move';
      e.stopPropagation();
    });
    header.addEventListener('dragend', () => {
      dragSrcGroupKey = null;
      if (dragOverGroupEl) { dragOverGroupEl.classList.remove('drag-over'); dragOverGroupEl = null; }
      root.querySelectorAll('.project-group').forEach(el => el.classList.remove('dragging'));
    });
    groupEl.addEventListener('dragover', (e) => {
      if (!dragSrcGroupKey || dragSrcGroupKey === key) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = 'move';
      if (dragOverGroupEl !== groupEl) {
        if (dragOverGroupEl) dragOverGroupEl.classList.remove('drag-over');
        dragOverGroupEl = groupEl;
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
      dragSrcGroupKey = null;
      if (!groupOrder.length) groupOrder = [...sortedGroupKeys];
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
      const providerName = s.provider === 'claude' ? 'Claude' : s.provider === 'codex' ? 'Codex' : (s.provider || '');
      const providerChipHtml = providerName ? `<span class="card-provider-chip ${s.provider || ''}">${providerName}</span>` : '';
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
      const metaRow = `<div class="card-meta-row"><span class="badge ${state}">${label}</span>${reasonHtml}${sessionLabel}${msgHtml}</div>`;
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
      c.innerHTML =
        `<div class="card-title-row"><b>#${s.id}</b> ${providerIconHtml(s.provider)} ${providerChipHtml}${modelBadge}${branchBadge}</div>` +
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
        dragSrcId = s.id;
        c.classList.add('dragging');
        e.dataTransfer.effectAllowed = 'move';
      });
      c.addEventListener('dragend', () => {
        c.classList.remove('dragging');
        if (dragOverCardEl) {
          dragOverCardEl.classList.remove('drag-over', 'drop-before', 'drop-after');
          dragOverCardEl = null;
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
          dragOverCardEl = c;
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
        dragSrcId = null;
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
function _addSidebarGroupLabels(root) {
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
    starLabel.textContent = '★ ' + (window.t ? window.t('sidebar_favorites', 'Favorites') : 'Favorites');
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

function updateMainTabStatus() {
  // D11: セッション情報チップも同タイミングで更新 (state badge を反映)
  if (typeof renderSessionInfoChip === 'function') renderSessionInfoChip();
  const wrap = document.getElementById('main-tab-status');
  if (!wrap) return;
  const sess = sessions.get(activeSessionId);
  if (!sess) {
    wrap.hidden = true;
    return;
  }
  const runtimeStr = sess.started_at ? formatStartedAt(sess.started_at) : '';
  const lastStr = sess.last_output_at ? formatLastOutputAt(sess.last_output_at) : '';
  if (!runtimeStr && !lastStr) {
    wrap.hidden = true;
    return;
  }
  const runtimeLabel = (typeof window.t === 'function') ? window.t('main_tab_runtime_label') : 'Runtime';
  const lastLabel = (typeof window.t === 'function') ? window.t('main_tab_last_label') : 'Last';
  const runtimeEl = wrap.querySelector('.main-tab-status-runtime');
  const lastEl = wrap.querySelector('.main-tab-status-last');
  if (runtimeEl) {
    runtimeEl.textContent = runtimeStr ? `${runtimeLabel} ${runtimeStr}` : '';
    runtimeEl.hidden = !runtimeStr;
  }
  if (lastEl) {
    lastEl.textContent = lastStr ? `${lastLabel} [${lastStr}]` : '';
    lastEl.hidden = !lastStr;
  }
  wrap.hidden = false;
}

// ---- タブ通知（保留バッジ） ----

let _faviconCanvas = null;
let _faviconCtx = null;
let _faviconBaseImg = null;
let _faviconBaseLoaded = false;
let _faviconPendingCount = 0;
let _faviconRenderedPendingCount = null;

let _titleBlinkInterval = null;
let _titleBlinkState = false;
let _titleBlinkCount = 0;

function startTitleBlink(pendingCount) {
  if (_titleBlinkInterval && _titleBlinkCount === pendingCount) return;
  stopTitleBlink();
  _titleBlinkCount = pendingCount;
  _titleBlinkState = true;
  _titleBlinkInterval = setInterval(() => {
    _titleBlinkState = !_titleBlinkState;
    document.title = _titleBlinkState ? `(${_titleBlinkCount}) ANY-AI-CLI` : 'ANY-AI-CLI';
  }, 800);
}

function stopTitleBlink() {
  if (_titleBlinkInterval) {
    clearInterval(_titleBlinkInterval);
    _titleBlinkInterval = null;
  }
}

function initFaviconCanvas() {
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

function drawFavicon(pendingCount, force = false) {
  initFaviconCanvas();
  if (!force && _faviconRenderedPendingCount === pendingCount) return;
  const ctx = _faviconCtx;
  const SIZE = 32;
  ctx.clearRect(0, 0, SIZE, SIZE);
  if (_faviconBaseLoaded) ctx.drawImage(_faviconBaseImg, 0, 0, SIZE, SIZE);

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

function updateTabNotification(pendingCount) {
  const faviconChanged = _faviconPendingCount !== pendingCount;
  _faviconPendingCount = pendingCount;

  if (pendingCount > 0) {
    startTitleBlink(pendingCount);
  } else {
    stopTitleBlink();
    document.title = 'ANY-AI-CLI';
  }

  if (faviconChanged) drawFavicon(pendingCount);
}

let _summaryResizeObserver = null;
function ensureSummaryResizeObserver() {
  if (_summaryResizeObserver) return;
  const el = document.getElementById('summary');
  if (!el) return;
  _summaryResizeObserver = new ResizeObserver(() => updateSummaryCompactMode());
  _summaryResizeObserver.observe(el);
  const parent = el.parentElement;
  if (parent) _summaryResizeObserver.observe(parent);
}

function updateSummaryCompactMode() {
  const el = document.getElementById('summary');
  if (!el) return;
  // scrollWidth > clientWidth は #summary が flex-wrap:nowrap + overflow:hidden の前提でのみ意味を持つ。
  el.classList.remove('summary--compact');
  const overflow = el.scrollWidth > el.clientWidth + 1;
  if (overflow) el.classList.add('summary--compact');
}

function renderSummaryAndNotifications() {
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

  const PROVIDER_LABELS = { claude: 'Claude', codex: 'Codex', ollama: 'Ollama', opencode: 'OpenCode' };
  const PROVIDER_ORDER = { claude: 0, ollama: 1, codex: 2, opencode: 3 };
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
      const label = PROVIDER_LABELS.ollama;
      const modelHtml = g.model ? `<span class="summary-ollama-model">${escapeHtml(g.model)}</span>` : '';
      const tip = g.model ? `Ollama · ${g.model} : ${g.count}` : `Ollama : ${g.count}`;
      return `<span class="summary-provider-chip" data-tooltip="${escapeHtml(tip)}">${providerIconHtml('ollama')}<span class="compact-hide"><span class="summary-provider-name ollama">${label}</span>${modelHtml}<span class="summary-provider-count">: ${g.count}</span></span><span class="compact-count">${g.count}</span></span>`;
    }
    const provider = g.provider;
    const label = PROVIDER_LABELS[provider] || provider;
    const tip = `${label} : ${g.count}`;
    return `<span class="summary-provider-chip" data-tooltip="${escapeHtml(tip)}">${providerIconHtml(provider)}<span class="compact-hide"><span class="summary-provider-name ${provider}">${label}</span><span class="summary-provider-count">: ${g.count}</span></span><span class="compact-count">${g.count}</span></span>`;
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

function sessionProjectKey(s) {
  const cwdStr = s?.cwd || '';
  const name = cwdStr
    ? cwdStr.replace(/\\/g, '/').split('/').filter(p => p.length > 0).pop() || ''
    : '';
  return name || '__no_project__';
}

function updateProjectGroupStatusChipsForSession(s) {
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

function updateSessionCardStateInPlace(id) {
  const root = document.getElementById('sessions');
  const s = sessions.get(id);
  if (!root || !s) return false;
  const card = root.querySelector(`.card[data-session-id="${CSS.escape(String(id))}"]`);
  if (!card) return false;
  const state = s.state || 'standby';
  card.classList.toggle('running', state === 'running');
  card.classList.toggle('waiting', state === 'waiting');
  card.classList.toggle('active', id === activeSessionId);
  const badge = card.querySelector('.badge');
  if (badge) {
    badge.className = `badge ${state}`;
    badge.textContent = stateLabel(state);
  }
  updateProjectGroupStatusChipsForSession(s);
  return true;
}

function renderSessionStateUpdate(id) {
  renderSummaryAndNotifications();
  const updated = updateSessionCardStateInPlace(id);
  updateMainTabStatus();
  if (typeof syncElapsedTimer === 'function') syncElapsedTimer();
  return updated;
}

function render() {
  renderSummaryAndNotifications();

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
