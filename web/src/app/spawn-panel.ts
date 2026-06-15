// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { escapeHtml, showToast, token } from './util.js';
import { CWD_HISTORY_MAX, STORAGE_CWD_HISTORY_KEY, STORAGE_CWD_FAVORITES_KEY, STORAGE_SPAWN_KEY, setUserPref } from './user-prefs.js';
import { set_pendingAutoSwitch, sessions } from './state.js';
import { providerIconHtml } from './session-list.js';
import { appConfirm, appConfirmOllamaEncoding } from './settings.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- 新規セッション spawn panel ----
(function () {
  const newSessionBtn   = document.getElementById('new-session-btn');
  const newSessionPanel = document.getElementById('new-session-panel');
  const spawnCwdInput   = document.getElementById('spawn-cwd');
  const spawnCwdBrowse  = document.getElementById('spawn-cwd-browse');
  const cwdDropdown     = document.getElementById('spawn-cwd-dropdown');
  const spawnCancelBtn  = document.getElementById('spawn-cancel-btn');
  const spawnLaunchBtn  = document.getElementById('spawn-launch-btn');
  const spawnProviderEl = document.getElementById('spawn-provider');
  const spawnProviderCombobox = document.getElementById('spawn-provider-combobox');
  const spawnProviderTrigger = document.getElementById('spawn-provider-trigger');
  const spawnProviderTriggerLabel = document.getElementById('spawn-provider-trigger-label');
  const spawnProviderTriggerIcon = document.getElementById('spawn-provider-trigger-icon');
  const spawnProviderList = document.getElementById('spawn-provider-list');
  const spawnCodexModelBtn = document.getElementById('spawn-codex-model-btn');
  const spawnClaudeModelBtn = document.getElementById('spawn-claude-model-btn');
  const spawnModelInput = document.getElementById('spawn-model');
  const spawnModelDatalist = document.getElementById('spawn-model-datalist');
  const spawnModelClearBtn = document.getElementById('spawn-model-clear');
  const spawnModelRefreshBtn = document.getElementById('spawn-model-refresh');
  let codexModelSelection: any = null;
  let claudeModelSelection: any = null;

  // ---- C2: Detached 設定 ----
  const spawnDetachedOpts   = document.getElementById('spawn-detached-opts');
  const spawnDetachedPreset = document.getElementById('spawn-detached-preset') as HTMLSelectElement | null;
  const spawnDetachedPreviewText = document.getElementById('spawn-detached-preview-text');

  function getSpawnOpenTarget(): string {
    const el = document.querySelector<HTMLInputElement>('input[name="spawn-open-target"]:checked');
    return el ? el.value : 'hub';
  }

  function getSpawnGridLayout(): string {
    const el = document.querySelector<HTMLInputElement>('input[name="spawn-grid-layout"]:checked');
    return el ? el.value : '1x1';
  }

  function updateDetachedPreview(): void {
    if (!spawnDetachedPreviewText) return;
    const target = getSpawnOpenTarget();
    if (target !== 'detached') { spawnDetachedPreviewText.textContent = ''; return; }
    const layout = getSpawnGridLayout();
    const provider = (spawnProviderEl as HTMLSelectElement).value || 'claude';
    const preset = spawnDetachedPreset ? spawnDetachedPreset.value : 'single';
    let desc = '';
    if (preset === 'project') {
      desc = t('spawn_preview_project_sessions');
    } else if (preset === 'multi') {
      desc = t('spawn_preview_current_multi');
    } else if (preset === 'claude-shell-2x2') {
      desc = t('spawn_preview_claude_shell_2x2', { provider });
    } else if (preset === 'shell-2x2') {
      desc = t('spawn_preview_shell_2x2');
    } else if (preset === 'shell-3x3') {
      desc = t('spawn_preview_shell_3x3');
    } else if (preset === 'advanced') {
      desc = t('spawn_preview_advanced');
    } else {
      desc = t('spawn_preview_single', { provider, layout });
    }
    spawnDetachedPreviewText.textContent = desc;
  }

  // Open target ラジオボタン変更 → detached opts の表示/非表示 + プレビュー更新
  document.querySelectorAll<HTMLInputElement>('input[name="spawn-open-target"]').forEach(radio => {
    radio.addEventListener('change', () => {
      const target = getSpawnOpenTarget();
      if (spawnDetachedOpts) spawnDetachedOpts.hidden = (target !== 'detached');
      updateDetachedPreview();
    });
  });

  // Grid layout ラジオボタン変更 → プレビュー更新
  document.querySelectorAll<HTMLInputElement>('input[name="spawn-grid-layout"]').forEach(radio => {
    radio.addEventListener('change', () => updateDetachedPreview());
  });

  // Preset 変更 → プレビュー更新
  if (spawnDetachedPreset) {
    spawnDetachedPreset.addEventListener('change', () => updateDetachedPreview());
  }

  // /api/models から取得した groups の最新キャッシュ。
  // populateModelDatalist と resolveRoute で共有する。
  let spawnModelGroups = null;
  // model id → route の即時参照 Map。
  const spawnModelRouteMap = new Map();
  let spawnModelFetchInFlight = null;
  let spawnProviderOpen = false;
  let spawnProviderActiveIndex = -1;

  function rebuildModelRouteMap(groups) {
    spawnModelRouteMap.clear();
    if (!Array.isArray(groups)) return;
    for (const g of groups) {
      if (!g || !Array.isArray(g.models)) continue;
      for (const m of g.models) {
        if (m && m.id) spawnModelRouteMap.set(m.id, g.route || '');
      }
    }
  }

  function getModelGroupsForProvider(provider) {
    if (!Array.isArray(spawnModelGroups)) return [];
    if (provider === 'copilot') {
      return spawnModelGroups.filter(g => g && g.provider === 'copilot' && Array.isArray(g.models));
    }
    if (provider === 'cursor-agent') {
      return spawnModelGroups.filter(g => g && g.provider === 'cursor-agent' && Array.isArray(g.models));
    }
    const groups = spawnModelGroups.filter(g => g && Array.isArray(g.models) && (!g.provider || g.provider === provider));
    groups.sort((a, b) => {
      const rank = (g) => {
        if (g.provider === provider) return 0;
        if (g.label === 'Ollama Cloud') return 1;
        if (g.label === 'Ollama Local') return 2;
        return 3;
      };
      return rank(a) - rank(b);
    });
    return groups;
  }

  function groupHasModel(group, model) {
    return !!group?.models?.some(m => m && m.id === model);
  }

  function isModelCompatibleWithProvider(provider, model) {
    const m = (model || '').trim();
    if (!m || !Array.isArray(spawnModelGroups)) return true;
    let known = false;
    for (const g of spawnModelGroups) {
      if (!groupHasModel(g, m)) continue;
      known = true;
      if (!g.provider || g.provider === provider) return true;
    }
    return !known;
  }

  function clearModelSelectionState() {
    codexModelSelection = null;
    claudeModelSelection = null;
  }

  function syncModelClearButton() {
    if (spawnModelClearBtn) spawnModelClearBtn.hidden = !spawnModelInput.value.trim();
  }

  function setSpawnModelValue(value) {
    spawnModelInput.value = value || '';
    syncModelClearButton();
  }

  function clearIncompatibleModelForProvider(provider) {
    if (!isModelCompatibleWithProvider(provider, spawnModelInput.value)) {
      setSpawnModelValue('');
      clearModelSelectionState();
    }
  }

  // dialog open 時、復元された model が Ollama route なら空にする。
  // 残しておくとそのまま spawn 実行で env 焼き付け → /model blocked の罠を踏むため、
  // Ollama は毎回明示的に選び直す運用に倒す（saveSpawnSettings 側でも保存しない）。
  function clearOllamaModelDefault() {
    const m = spawnModelInput.value.trim();
    if (!m) return;
    if (resolveRoute(spawnProviderEl.value, m) === 'ollama') {
      setSpawnModelValue('');
      clearModelSelectionState();
    }
  }

  function populateModelDatalist() {
    if (!spawnModelDatalist) return;
    spawnModelDatalist.innerHTML = '';
    if (!Array.isArray(spawnModelGroups)) return;
    const currentProvider = spawnProviderEl.value;
    // 並び順: 同 provider 専用 → Ollama Cloud → Ollama Local。
    // 他 provider 専用は非表示。Ollama 系は provider="" で両 provider に表示する。
    for (const g of getModelGroupsForProvider(currentProvider)) {
      for (const m of g.models) {
        const opt = document.createElement('option');
        opt.value = m.id;
        // <option label> はブラウザ実装差があるため、フォールバックとして
        // text content にも同じ表記を入れる。
        const label = `[${g.label}] ${m.label || m.id}`;
        opt.setAttribute('label', label);
        opt.textContent = label;
        opt.dataset.route = g.route || '';
        spawnModelDatalist.appendChild(opt);
      }
    }
  }

  function resolveRoute(provider, model) {
    const m = (model || '').trim();
    if (!m) return '';
    if (provider === 'copilot') {
      return '';
    }
    if (provider === 'cursor-agent') {
      return '';
    }
    for (const g of getModelGroupsForProvider(provider)) {
      if (groupHasModel(g, m)) return g.route || '';
    }
    if (spawnModelRouteMap.has(m) && isModelCompatibleWithProvider(provider, m)) return spawnModelRouteMap.get(m);
    if (m.includes(':cloud')) return 'ollama';
    if (provider === 'claude') return 'anthropic';
    if (provider === 'codex')  return 'openai';
    return '';
  }

  async function fetchModelGroups(force) {
    if (spawnModelFetchInFlight) return spawnModelFetchInFlight;
    const method = force ? 'POST' : 'GET';
    const url = `/api/models?token=${token}`;
    const p = (async () => {
      try {
        const res = await fetch(url, { method });
        if (!res.ok) throw new Error('HTTP ' + res.status);
        const data = await res.json();
        spawnModelGroups = Array.isArray(data.groups) ? data.groups : [];
        rebuildModelRouteMap(spawnModelGroups);
        populateModelDatalist();
        clearIncompatibleModelForProvider(spawnProviderEl.value);
        clearOllamaModelDefault();
        return data;
      } finally {
        spawnModelFetchInFlight = null;
      }
    })();
    spawnModelFetchInFlight = p;
    return p;
  }

  if (spawnModelRefreshBtn) {
    spawnModelRefreshBtn.addEventListener('click', async () => {
      spawnModelRefreshBtn.classList.add('is-loading');
      try {
        await fetchModelGroups(true);
      } catch (_) {
        alert(t('spawn_model_fetch_failed'));
      } finally {
        spawnModelRefreshBtn.classList.remove('is-loading');
      }
    });
  }

  function getSpawnProviderOptions() {
    return Array.from((spawnProviderEl as HTMLSelectElement).options).map((opt, index) => {
      const label = (opt.textContent || opt.label || opt.value).trim() || opt.value;
      return { value: opt.value, label, id: `spawn-provider-option-${index}` };
    });
  }

  function getSelectedSpawnProviderIndex() {
    const options = getSpawnProviderOptions();
    const idx = options.findIndex(opt => opt.value === spawnProviderEl.value);
    return idx >= 0 ? idx : 0;
  }

  function getSelectedSpawnProviderOption() {
    const options = getSpawnProviderOptions();
    return options.find(opt => opt.value === spawnProviderEl.value) || options[0] || {
      value: spawnProviderEl.value,
      label: spawnProviderEl.value,
      id: 'spawn-provider-option-0',
    };
  }

  function renderSpawnProviderOptions() {
    if (!spawnProviderList) return;
    const options = getSpawnProviderOptions();
    if (spawnProviderActiveIndex < 0 || spawnProviderActiveIndex >= options.length) {
      spawnProviderActiveIndex = getSelectedSpawnProviderIndex();
    }
    const selectedValue = spawnProviderEl.value;
    spawnProviderList.innerHTML = options.map((opt, index) => {
      const selected = opt.value === selectedValue;
      const active = spawnProviderOpen && index === spawnProviderActiveIndex;
      return (
        `<li id="${opt.id}" class="spawn-provider-option${selected ? ' is-selected' : ''}${active ? ' is-active' : ''}" ` +
        `role="option" aria-selected="${selected ? 'true' : 'false'}" data-value="${escapeHtml(opt.value)}" tabindex="-1">` +
        `<span class="spawn-provider-option-icon" aria-hidden="true">${providerIconHtml(opt.value, 14)}</span>` +
        `<span class="spawn-provider-option-label">${escapeHtml(opt.label)}</span>` +
        `<span class="spawn-provider-option-check" aria-hidden="true">${selected ? '✓' : ''}</span>` +
        `</li>`
      );
    }).join('');
    const activeOption = options[spawnProviderActiveIndex];
    if (spawnProviderOpen && activeOption && spawnProviderTrigger) {
      spawnProviderTrigger.setAttribute('aria-activedescendant', activeOption.id);
      document.getElementById(activeOption.id)?.scrollIntoView({ block: 'nearest' });
    } else if (spawnProviderTrigger) {
      spawnProviderTrigger.removeAttribute('aria-activedescendant');
    }
  }

  function updateSpawnProviderIcon() {
    const selected = getSelectedSpawnProviderOption();
    if (spawnProviderTriggerLabel) spawnProviderTriggerLabel.textContent = selected.label;
    if (spawnProviderTriggerIcon) spawnProviderTriggerIcon.innerHTML = providerIconHtml(selected.value, 14);
    renderSpawnProviderOptions();
  }

  function openSpawnProviderList() {
    if (!spawnProviderList || !spawnProviderTrigger || !spawnProviderCombobox) return;
    spawnProviderOpen = true;
    spawnProviderActiveIndex = getSelectedSpawnProviderIndex();
    spawnProviderList.hidden = false;
    spawnProviderTrigger.setAttribute('aria-expanded', 'true');
    spawnProviderTrigger.classList.add('is-open');
    spawnProviderCombobox.classList.add('is-open');
    renderSpawnProviderOptions();
  }

  function closeSpawnProviderList(focusTrigger = false) {
    if (!spawnProviderList || !spawnProviderTrigger || !spawnProviderCombobox) return;
    spawnProviderOpen = false;
    spawnProviderList.hidden = true;
    spawnProviderTrigger.setAttribute('aria-expanded', 'false');
    spawnProviderTrigger.removeAttribute('aria-activedescendant');
    spawnProviderTrigger.classList.remove('is-open');
    spawnProviderCombobox.classList.remove('is-open');
    renderSpawnProviderOptions();
    if (focusTrigger) spawnProviderTrigger.focus();
  }

  function setSpawnProviderActiveIndex(index) {
    const options = getSpawnProviderOptions();
    if (options.length === 0) return;
    spawnProviderActiveIndex = (index + options.length) % options.length;
    renderSpawnProviderOptions();
  }

  function selectSpawnProviderValue(value) {
    (spawnProviderEl as HTMLSelectElement).value = value;
    spawnProviderEl.dispatchEvent(new Event('change', { bubbles: true }));
    closeSpawnProviderList(true);
  }

  function handleSpawnProviderKeydown(e) {
    if (e.key === 'Enter' || e.key === ' ' || e.key === 'Spacebar') {
      e.preventDefault();
      if (!spawnProviderOpen) {
        openSpawnProviderList();
        return;
      }
      const opt = getSpawnProviderOptions()[spawnProviderActiveIndex];
      if (opt) selectSpawnProviderValue(opt.value);
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (!spawnProviderOpen) openSpawnProviderList();
      else setSpawnProviderActiveIndex(spawnProviderActiveIndex + 1);
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (!spawnProviderOpen) openSpawnProviderList();
      else setSpawnProviderActiveIndex(spawnProviderActiveIndex - 1);
      return;
    }
    if (e.key === 'Escape') {
      if (spawnProviderOpen) {
        e.preventDefault();
        closeSpawnProviderList(true);
      }
      return;
    }
    if (e.key === 'Tab' && spawnProviderOpen) {
      closeSpawnProviderList(false);
    }
  }

  if (spawnProviderTrigger) {
    spawnProviderTrigger.addEventListener('click', () => {
      if (spawnProviderOpen) closeSpawnProviderList(false);
      else openSpawnProviderList();
    });
    spawnProviderTrigger.addEventListener('keydown', handleSpawnProviderKeydown);
  }

  if (spawnProviderList) {
    spawnProviderList.addEventListener('click', (e) => {
      const item = e.target.closest('.spawn-provider-option');
      if (!item) return;
      selectSpawnProviderValue(item.dataset.value);
    });
    spawnProviderList.addEventListener('mousemove', (e) => {
      const item = e.target.closest('.spawn-provider-option');
      if (!item || !spawnProviderList.contains(item)) return;
      const items = [...spawnProviderList.querySelectorAll('.spawn-provider-option')];
      const idx = items.indexOf(item);
      if (idx >= 0 && idx !== spawnProviderActiveIndex) setSpawnProviderActiveIndex(idx);
    });
    spawnProviderList.addEventListener('keydown', handleSpawnProviderKeydown);
  }

  document.addEventListener('mousedown', (e) => {
    if (!spawnProviderOpen || !spawnProviderCombobox) return;
    if (!spawnProviderCombobox.contains(e.target)) closeSpawnProviderList(false);
  });

  spawnProviderEl.addEventListener('change', () => {
    updateSpawnProviderIcon();
    const p = spawnProviderEl.value;
    const isShell = (p === 'shell');
    // Shell は model input / datalist / provider-specific opts を隠す
    const modelRow = document.querySelector<HTMLElement>('.spawn-model-row');
    if (modelRow) modelRow.hidden = isShell;
    document.getElementById('spawn-claude-opts').hidden = (p !== 'claude');
    document.getElementById('spawn-codex-opts').hidden  = (p !== 'codex');
    const claudeNote = document.getElementById('spawn-claude-note');
    const codexNote = document.getElementById('spawn-codex-note');
    const copilotNote = document.getElementById('spawn-copilot-note');
    const cursorAgentNote = document.getElementById('spawn-cursor-agent-note');
    const shellNote = document.getElementById('spawn-shell-note');
    if (claudeNote) claudeNote.hidden = (p !== 'claude');
    if (codexNote) codexNote.hidden = (p !== 'codex');
    if (copilotNote) copilotNote.hidden = (p !== 'copilot');
    if (cursorAgentNote) cursorAgentNote.hidden = (p !== 'cursor-agent');
    if (shellNote) shellNote.hidden = !isShell;
    if (p !== 'codex')  codexModelSelection  = null;
    if (p !== 'claude') claudeModelSelection = null;
    populateModelDatalist();
    clearIncompatibleModelForProvider(p);
    updateDetachedPreview();
  });
  updateSpawnProviderIcon();

  // フォーカス時に入力値を一時クリアして datalist の全候補を表示し、
  // 未選択のまま離れたら元の値を復元する。
  let _savedModelValue = '';
  let _modelInputDirty = false;
  spawnModelInput.addEventListener('focus', () => {
    _savedModelValue = spawnModelInput.value;
    _modelInputDirty = false;
    spawnModelInput.value = '';
  });
  spawnModelInput.addEventListener('input', () => {
    _modelInputDirty = true;
    clearModelSelectionState();
    syncModelClearButton();
  });
  spawnModelInput.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      _modelInputDirty = false;
      setSpawnModelValue(_savedModelValue);
      spawnModelInput.blur();
    }
  });
  spawnModelInput.addEventListener('blur', () => {
    if (!_modelInputDirty) {
      setSpawnModelValue(_savedModelValue);
    }
  });
  if (spawnModelClearBtn) {
    spawnModelClearBtn.addEventListener('click', () => {
      setSpawnModelValue('');
      clearModelSelectionState();
      spawnModelInput.focus();
    });
  }

  function loadSpawnSettings() {
    try {
      const s = JSON.parse(localStorage.getItem(STORAGE_SPAWN_KEY) || '{}');
      if (s.provider) {
        spawnProviderEl.value = s.provider;
        const p = s.provider;
        const isShell = (p === 'shell');
        const modelRow = document.querySelector<HTMLElement>('.spawn-model-row');
        if (modelRow) modelRow.hidden = isShell;
        document.getElementById('spawn-claude-opts').hidden = (p !== 'claude');
        document.getElementById('spawn-codex-opts').hidden  = (p !== 'codex');
        const claudeNote = document.getElementById('spawn-claude-note');
        const codexNote = document.getElementById('spawn-codex-note');
        const copilotNote = document.getElementById('spawn-copilot-note');
        const cursorAgentNote = document.getElementById('spawn-cursor-agent-note');
        const shellNote = document.getElementById('spawn-shell-note');
        if (claudeNote) claudeNote.hidden = (p !== 'claude');
        if (codexNote) codexNote.hidden = (p !== 'codex');
        if (copilotNote) copilotNote.hidden = (p !== 'copilot');
        if (cursorAgentNote) cursorAgentNote.hidden = (p !== 'cursor-agent');
        if (shellNote) shellNote.hidden = !isShell;
      }
      if (s.cwd)              spawnCwdInput.value = s.cwd;
      if (s.model !== undefined) setSpawnModelValue(s.model);
      if (s.permission_mode)  document.getElementById('spawn-permission-mode').value = s.permission_mode;
      if (s.sandbox)          document.getElementById('spawn-sandbox').value = s.sandbox;
      if (s.ask_for_approval) document.getElementById('spawn-ask-approval').value = s.ask_for_approval;
      // C2: Detached 設定を復元
      if (s.open_target) {
        const radio = document.getElementById(`spawn-target-${s.open_target}`) as HTMLInputElement | null;
        if (radio && !radio.disabled) {
          radio.checked = true;
          if (spawnDetachedOpts) spawnDetachedOpts.hidden = (s.open_target !== 'detached');
        }
      }
      if (s.grid_layout) {
        const layoutMap: Record<string, string> = { '1x1': '1x1', '1x2': '1x2', '2x2': '2x2', '2x3': '2x3', '3x3': '3x3' };
        const normalizedLayout = layoutMap[s.grid_layout] || '1x1';
        const layoutRadio = document.getElementById(`spawn-layout-${normalizedLayout}`) as HTMLInputElement | null;
        if (layoutRadio) layoutRadio.checked = true;
      }
      if (s.detached_preset && spawnDetachedPreset) {
        spawnDetachedPreset.value = s.detached_preset;
      }
      updateSpawnProviderIcon();
      updateDetachedPreview();
      return !!s.cwd;
    } catch (_) { return false; }
  }

  function saveSpawnSettings(obj) {
    setUserPref('spawn.defaults', obj);
  }

  // C2: detached-grid URL を生成して別窓で開く
  function openDetachedGrid(sessionId: number, layout: string): void {
    const params = new URLSearchParams(window.location.search);
    const tokenVal = params.get('token') || token;
    const url = `/?view=detached-grid&layout=${encodeURIComponent(layout)}&session_ids=${sessionId}&token=${tokenVal}`;
    window.open(url, '_blank');
  }

  // C2: spawn 後に新しいセッションが WS 経由で登録されるのを待って別窓を開く。
  // /api/spawn レスポンスには session_id が含まれないため、spawn 前の最大 ID を
  // 記録しておき、その後に登録された最新 ID を検出する。
  function _waitForNewSessionAndOpenGrid(layout: string): void {
    const prevMax = sessions.size > 0
      ? Math.max(...Array.from(sessions.keys()))
      : 0;
    const TIMEOUT_MS = 8000;
    const POLL_MS = 200;
    const deadline = Date.now() + TIMEOUT_MS;

    function poll() {
      if (sessions.size > 0) {
        const allIds = Array.from(sessions.keys());
        const newIds = allIds.filter(id => id > prevMax);
        if (newIds.length > 0) {
          const latestId = Math.max(...newIds);
          openDetachedGrid(latestId, layout);
          return;
        }
      }
      if (Date.now() < deadline) {
        setTimeout(poll, POLL_MS);
      } else {
        // タイムアウト: 最後に登録されたセッションを使う
        if (sessions.size > 0) {
          const latestId = Math.max(...Array.from(sessions.keys()));
          openDetachedGrid(latestId, layout);
        }
      }
    }
    setTimeout(poll, POLL_MS);
  }

  function loadCwdHistory() {
    try { return JSON.parse(localStorage.getItem(STORAGE_CWD_HISTORY_KEY) || '[]'); } catch (_) { return []; }
  }

  function saveCwdHistory(cwd) {
    if (!cwd) return;
    const hist = loadCwdHistory().filter(v => v !== cwd);
    hist.unshift(cwd);
    if (hist.length > CWD_HISTORY_MAX) hist.length = CWD_HISTORY_MAX;
    setUserPref('cwd_history', hist);
  }

  function deleteCwdHistoryItem(cwd) {
    const hist = loadCwdHistory().filter(v => v !== cwd);
    setUserPref('cwd_history', hist);
  }

  function loadCwdFavorites() {
    try { return JSON.parse(localStorage.getItem(STORAGE_CWD_FAVORITES_KEY) || '[]'); } catch (_) { return []; }
  }

  function isCwdFavorite(cwd) {
    return loadCwdFavorites().includes(cwd);
  }

  function toggleCwdFavorite(cwd) {
    if (!cwd) return;
    const favs = loadCwdFavorites();
    const next = favs.includes(cwd) ? favs.filter(v => v !== cwd) : [cwd, ...favs];
    setUserPref('cwd_favorites', next);
  }

  // D&D（お気に入り並び替え）用の状態。
  let cwdDragValue = null;      // ドラッグ中のお気に入りパス（非ドラッグ時 null）
  let cwdDragMoved = false;     // 並び替え直後に発火する click 選択を1回抑止する
  let cwdSuppressReopen = false; // お気に入り選択で入力欄を再 focus する際の自動再オープンを1回抑止する

  // パス文字列を「親ディレクトリ」「末尾セグメント（basename）」に分割する。
  // 区切りは \ と / の両対応。末尾が区切り文字の場合は手前のセグメントを basename とする。
  function splitCwdPath(value) {
    const v = String(value);
    // 末尾の区切り文字は無視して basename 境界を探す。
    let end = v.length;
    while (end > 0 && (v[end - 1] === '/' || v[end - 1] === '\\')) end--;
    let start = end;
    while (start > 0 && v[start - 1] !== '/' && v[start - 1] !== '\\') start--;
    return { parent: v.slice(0, start), basename: v.slice(start) };
  }

  // 生テキスト raw を escapeHtml した上で、filter にマッチする部分のみ <mark> で囲む。
  // ⚠️ XSS: 分割は raw（未エスケープ）の小文字比較で位置だけ求め、出力は必ず
  //         escapeHtml 済みの各断片に対してのみ span/mark を組み立てる。
  function highlightCwdSegment(raw, filter) {
    const escaped = escapeHtml(raw);
    if (!filter) return escaped;
    const lowRaw = raw.toLowerCase();
    const lowFilter = filter.toLowerCase();
    let out = '';
    let i = 0;
    while (i < raw.length) {
      const hit = lowRaw.indexOf(lowFilter, i);
      if (hit < 0) { out += escapeHtml(raw.slice(i)); break; }
      out += escapeHtml(raw.slice(i, hit));
      out += `<mark class="cwd-dropdown-mark">${escapeHtml(raw.slice(hit, hit + filter.length))}</mark>`;
      i = hit + filter.length;
    }
    return out;
  }

  // 2トーン（親=muted / 末尾=強調）＋ filter マッチハイライトのラベル HTML を組み立てる。
  function buildCwdLabelHtml(value, filter) {
    const { parent, basename } = splitCwdPath(value);
    const parentHtml = parent
      ? `<span class="cwd-dropdown-path-parent">${highlightCwdSegment(parent, filter)}</span>`
      : '';
    const baseHtml = `<span class="cwd-dropdown-path-base">${highlightCwdSegment(basename, filter)}</span>`;
    return parentHtml + baseHtml;
  }

  // 末尾が `\` または `/` のとき、その親パス直下のサブフォルダ一覧を保持する。
  // input 値が変わるたびに更新され、renderCwdDropdown が先頭セクションとして描画する。
  const subdirsCache = new Map<string, string[]>();
  let subdirsCurrent: { parent: string; sep: string; items: string[] } | null = null;

  function detectPathSep(v: string): string {
    if (v.includes('\\')) return '\\';
    if (v.includes('/')) return '/';
    return '\\';
  }
  function endsWithSep(v: string): boolean {
    return v.endsWith('\\') || v.endsWith('/');
  }
  function stripTrailingSep(v: string): string {
    let end = v.length;
    while (end > 0 && (v[end - 1] === '\\' || v[end - 1] === '/')) end--;
    return v.slice(0, end);
  }

  async function fetchSubdirs(parent: string): Promise<string[]> {
    if (subdirsCache.has(parent)) return subdirsCache.get(parent)!;
    try {
      const res = await fetch(`/api/list-subdirs?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path: parent }),
      });
      if (!res.ok) { subdirsCache.set(parent, []); return []; }
      const data = await res.json();
      const items = Array.isArray(data.subdirs) ? data.subdirs : [];
      subdirsCache.set(parent, items);
      return items;
    } catch (_) {
      subdirsCache.set(parent, []);
      return [];
    }
  }

  function maybeUpdateSubdirs(value: string): void {
    if (!value || !endsWithSep(value)) {
      subdirsCurrent = null;
      return;
    }
    const sep = detectPathSep(value);
    const parent = stripTrailingSep(value);
    if (!parent) { subdirsCurrent = null; return; }
    if (subdirsCurrent && subdirsCurrent.parent === parent) return;
    subdirsCurrent = { parent, sep, items: subdirsCache.get(parent) ?? [] };
    fetchSubdirs(parent).then(items => {
      if (!subdirsCurrent || subdirsCurrent.parent !== parent) return;
      subdirsCurrent.items = items;
      renderCwdDropdown(spawnCwdInput.value.trim());
    });
  }

  function renderCwdDropdown(filter) {
    const favs = loadCwdFavorites();
    const favSet = new Set(favs);
    const hist = loadCwdHistory();
    const subItems: string[] = subdirsCurrent
      ? subdirsCurrent.items.map(name => subdirsCurrent!.parent + subdirsCurrent!.sep + name)
      : [];
    const favItems = (filter
      ? favs.filter(v => v.toLowerCase().includes(filter.toLowerCase()))
      : favs);
    const histItems = (filter
      ? hist.filter(v => !favSet.has(v) && v.toLowerCase().includes(filter.toLowerCase()))
      : hist.filter(v => !favSet.has(v)));
    const items = [...subItems, ...favItems, ...histItems];
    if (items.length === 0) { cwdDropdown.hidden = true; return; }

    function renderRow(v, fav, isSub = false) {
      const labelFilter = isSub ? '' : filter;
      return (
        `<li class="cwd-dropdown-item${fav ? ' is-favorite' : ''}${isSub ? ' is-subdir' : ''}" tabindex="-1"${fav ? ' draggable="true"' : ''} data-value="${escapeHtml(v)}">` +
        `<button class="cwd-dropdown-fav${fav ? ' is-on' : ''}" tabindex="-1" data-value="${escapeHtml(v)}" ` +
        `title="${escapeHtml(t(fav ? 'spawn_cwd_unfavorite' : 'spawn_cwd_favorite'))}">${fav ? '★' : '☆'}</button>` +
        `<span class="cwd-dropdown-label" title="${escapeHtml(v)}">${buildCwdLabelHtml(v, labelFilter)}</span>` +
        (isSub ? '' : `<button class="cwd-dropdown-del" tabindex="-1" data-value="${escapeHtml(v)}">×</button>`) +
        `</li>`
      );
    }

    // セクション見出し（クリック/フォーカス/キーボード移動/D&D の対象外）。
    // 各セクション0件なら見出しは出さない。
    let html = '';
    if (subItems.length > 0) {
      html += `<li class="cwd-dropdown-header" aria-hidden="true">${escapeHtml(t('spawn_cwd_section_subdirs'))}</li>`;
      html += subItems.map(v => renderRow(v, false, true)).join('');
    }
    if (favItems.length > 0) {
      html += `<li class="cwd-dropdown-header" aria-hidden="true">${escapeHtml(t('spawn_cwd_section_favorites'))}</li>`;
      html += favItems.map(v => renderRow(v, true)).join('');
    }
    if (histItems.length > 0) {
      html += `<li class="cwd-dropdown-header" aria-hidden="true">${escapeHtml(t('spawn_cwd_section_history'))}</li>`;
      html += histItems.map(v => renderRow(v, false)).join('');
    }
    cwdDropdown.innerHTML = html;
    cwdDropdown.hidden = false;
    applyDropdownMissingStatus();
    checkPathsExist(items).then(applyDropdownMissingStatus);
  }

  // path existence: 作業ディレクトリが実在しないと Cmd.Dir の chdir が
  // Windows で ERROR_DIRECTORY を返して spawn が失敗する。事前に弾いて
  // 起動ボタンを抑止し、ホバーで原因を出す。
  const pathExistsCache = new Map();
  let pathCheckDebounce = null;

  async function checkPathsExist(paths) {
    const unknown = [...new Set(paths)].filter(p => p && !pathExistsCache.has(p));
    if (unknown.length === 0) return;
    try {
      const res = await fetch(`/api/path-exists?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ paths: unknown }),
      });
      if (!res.ok) return;
      const data = await res.json();
      for (const [p, exists] of Object.entries(data.results || {})) {
        pathExistsCache.set(p, !!exists);
      }
    } catch (_) { /* 通信失敗時はキャッシュ更新せず楽観扱い */ }
  }

  function isPathMissing(p) {
    return pathExistsCache.has(p) && pathExistsCache.get(p) === false;
  }

  function applyCwdInputStatus() {
    const v = spawnCwdInput.value.trim();
    if (!v) {
      spawnCwdInput.classList.remove('is-missing');
      spawnCwdInput.removeAttribute('title');
      spawnLaunchBtn.disabled = true;
      spawnLaunchBtn.removeAttribute('title');
      return;
    }
    if (isPathMissing(v)) {
      spawnCwdInput.classList.add('is-missing');
      spawnCwdInput.title = t('spawn_cwd_missing', { path: v });
      spawnLaunchBtn.disabled = true;
      spawnLaunchBtn.title = t('spawn_cwd_missing_btn');
    } else {
      spawnCwdInput.classList.remove('is-missing');
      spawnCwdInput.removeAttribute('title');
      spawnLaunchBtn.disabled = false;
      spawnLaunchBtn.removeAttribute('title');
    }
  }

  async function refreshCwdInputStatus() {
    const v = spawnCwdInput.value.trim();
    if (v) await checkPathsExist([v]);
    applyCwdInputStatus();
  }

  function scheduleCwdInputCheck() {
    if (pathCheckDebounce) clearTimeout(pathCheckDebounce);
    pathCheckDebounce = setTimeout(refreshCwdInputStatus, 200);
  }

  function applyDropdownMissingStatus() {
    cwdDropdown.querySelectorAll('.cwd-dropdown-item').forEach(el => {
      const v = el.dataset.value;
      const delBtn = el.querySelector('.cwd-dropdown-del');
      if (isPathMissing(v)) {
        el.classList.add('is-missing');
        el.title = t('spawn_cwd_missing', { path: v });
        // 実在しないパスの × は常時表示。削除導線である旨をツールチップで強調する。
        if (delBtn) delBtn.title = t('spawn_cwd_remove_missing');
      } else {
        el.classList.remove('is-missing');
        el.removeAttribute('title');
        if (delBtn) delBtn.removeAttribute('title');
      }
    });
  }

  // ボタン押下: パネル表示 + 保存設定を復元 / 未保存時は /api/info から CWD を取得
  newSessionBtn.addEventListener('click', async () => {
    if (!newSessionPanel.hidden) { newSessionPanel.hidden = true; return; }
    const hasSavedCwd = loadSpawnSettings();
    if (!hasSavedCwd) {
      try {
        const res = await fetch(`/api/info?token=${token}`);
        if (res.ok) spawnCwdInput.value = (await res.json()).cwd || '';
      } catch (_) {}
    }
    newSessionPanel.hidden = false;
    updateSpawnProviderIcon();
    spawnCwdInput.focus();
    refreshCwdInputStatus();
    // モデル一覧を初回または stale なら裏で取得して datalist を埋める。
    // 失敗しても UI 起動はブロックしない（手入力で従来通り）。
    if (!spawnModelGroups) {
      fetchModelGroups(false).catch(() => {});
    } else {
      populateModelDatalist();
      clearOllamaModelDefault();
    }
  });

  // app.ts（shell セッション内で AI CLI 起動コマンドを検知した誘導）から呼ばれる。
  // 検知した provider と元 shell セッションの cwd をプリセットして新規セッションパネルを開く。
  async function openSpawnFor(provider: string, cwd: string): Promise<void> {
    loadSpawnSettings();
    if (cwd) {
      spawnCwdInput.value = cwd;
    } else if (!spawnCwdInput.value) {
      try {
        const res = await fetch(`/api/info?token=${token}`);
        if (res.ok) spawnCwdInput.value = (await res.json()).cwd || '';
      } catch (_) {}
    }
    if (provider) {
      (spawnProviderEl as HTMLSelectElement).value = provider;
      // change ハンドラに note/opts 表示・model datalist 更新を委譲する
      spawnProviderEl.dispatchEvent(new Event('change'));
    }
    newSessionPanel.hidden = false;
    updateSpawnProviderIcon();
    refreshCwdInputStatus();
    spawnCwdInput.focus();
    if (!spawnModelGroups) {
      fetchModelGroups(false).catch(() => {});
    } else {
      populateModelDatalist();
      clearOllamaModelDefault();
    }
  }
  (window as any).openSpawnFor = openSpawnFor;

  spawnCancelBtn.addEventListener('click', () => { newSessionPanel.hidden = true; });
  spawnLaunchBtn.addEventListener('click', spawnSession);
  if (spawnCwdBrowse) {
    spawnCwdBrowse.addEventListener('click', async () => {
      spawnCwdBrowse.disabled = true;
      try {
        const res = await fetch(`/api/pick-directory?token=${token}`, { method: 'POST' });
        if (res.ok) {
          const data = await res.json();
          if (data.ok && data.path) {
            spawnCwdInput.value = data.path;
            refreshCwdInputStatus();
            spawnCwdInput.focus();
            renderCwdDropdown('');
          }
        } else {
          // Non-2xx (typically 500 when no native folder picker is available,
          // e.g. Linux without zenity/kdialog). Surface the server message so
          // the click isn't silently ignored.
          let msg = '';
          try { msg = (await res.text()).trim(); } catch (_) {}
          showToast(msg ? `${t('link_open_error')}: ${msg}` : t('link_open_error'));
        }
      } catch (_) { showToast(t('link_open_error')); }
      finally { spawnCwdBrowse.disabled = false; }
    });
  }
  spawnCwdInput.addEventListener('focus', () => {
    // お気に入り選択直後の再 focus では再オープンしない（選択して閉じたのに即開き直る事故を防ぐ）。
    if (cwdSuppressReopen) { cwdSuppressReopen = false; return; }
    maybeUpdateSubdirs(spawnCwdInput.value.trim());
    renderCwdDropdown(''); refreshCwdInputStatus();
  });
  spawnCwdInput.addEventListener('click', () => {
    maybeUpdateSubdirs(spawnCwdInput.value.trim());
    renderCwdDropdown('');
  });
  spawnCwdInput.addEventListener('input', () => {
    const v = spawnCwdInput.value.trim();
    maybeUpdateSubdirs(v);
    renderCwdDropdown(v);
    scheduleCwdInputCheck();
  });
  spawnCwdInput.addEventListener('blur', (e) => {
    // フォーカスがドロップダウン内（上下キーで行へ移動）へ抜けた場合は閉じない。
    // これを忘れると ArrowDown で行に focus した瞬間に blur が発火し、150ms 後に
    // リストが消えて「上下キーで動かせない」状態になる。
    if (e.relatedTarget && cwdDropdown.contains(e.relatedTarget)) return;
    setTimeout(() => {
      if (cwdDragValue != null) return;                          // D&D 中は閉じない
      if (cwdDropdown.contains(document.activeElement)) return;  // フォーカスがまだ中にある
      cwdDropdown.hidden = true;
    }, 150);
  });
  spawnCwdInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter')  { cwdDropdown.hidden = true; if (!spawnLaunchBtn.disabled) spawnSession(); }
    if (e.key === 'Escape') { cwdDropdown.hidden = true; newSessionPanel.hidden = true; }
    if (e.key === 'ArrowDown' && !cwdDropdown.hidden) {
      e.preventDefault();
      const first = cwdDropdown.querySelector('.cwd-dropdown-item');
      if (first) first.focus();
    }
  });
  cwdDropdown.addEventListener('keydown', (e) => {
    const items = [...cwdDropdown.querySelectorAll('.cwd-dropdown-item')];
    const idx = items.indexOf(document.activeElement);
    if (e.key === 'ArrowDown') { e.preventDefault(); items[idx + 1]?.focus(); }
    if (e.key === 'ArrowUp')   { e.preventDefault(); idx > 0 ? items[idx - 1].focus() : spawnCwdInput.focus(); }
    if (e.key === 'Home')      { e.preventDefault(); items[0]?.focus(); }
    if (e.key === 'End')       { e.preventDefault(); items[items.length - 1]?.focus(); }
    if (e.key === 'Enter' && idx >= 0) { selectCwdItem(items[idx]); }
    // Backspace / Delete: 履歴行（非お気に入り）のみ削除し、近接行へフォーカスを移す。
    // お気に入り行・見出し行は誤操作防止のため削除しない。
    if ((e.key === 'Backspace' || e.key === 'Delete') && idx >= 0) {
      const item = items[idx];
      if (item.classList.contains('is-favorite') || item.classList.contains('is-subdir')) return;
      e.preventDefault();
      deleteCwdHistoryItem(item.dataset.value);
      renderCwdDropdown(spawnCwdInput.value.trim());
      const next = [...cwdDropdown.querySelectorAll('.cwd-dropdown-item')];
      if (next.length === 0) { focusInputNoReopen(); return; }
      (next[Math.min(idx, next.length - 1)] as HTMLElement).focus();
    }
    // 閉じて入力欄へ戻すだけ。focus() による再オープンを抑止しないと即開き直る。
    if (e.key === 'Escape') { cwdDropdown.hidden = true; focusInputNoReopen(); }
  });
  // 入力欄へフォーカスを戻す。入力欄が今フォーカスを持っていない場合のみ、
  // focus リスナによる再オープンを1回抑止する（持っている場合は focus() が no-op で
  // リスナが発火しないため抑止フラグを立てない＝フラグの立てっぱなしを防ぐ）。
  function focusInputNoReopen() {
    if (document.activeElement !== spawnCwdInput) cwdSuppressReopen = true;
    spawnCwdInput.focus();
  }

  function selectCwdItem(item) {
    spawnCwdInput.value = item.dataset.value;
    cwdDropdown.hidden = true;
    focusInputNoReopen();
    refreshCwdInputStatus();
  }

  cwdDropdown.addEventListener('mousedown', (e) => {
    const favBtn = e.target.closest('.cwd-dropdown-fav');
    if (favBtn) {
      e.preventDefault();
      toggleCwdFavorite(favBtn.dataset.value);
      renderCwdDropdown(spawnCwdInput.value.trim());
      spawnCwdInput.focus();
      return;
    }
    const delBtn = e.target.closest('.cwd-dropdown-del');
    if (delBtn) {
      e.preventDefault();
      deleteCwdHistoryItem(delBtn.dataset.value);
      renderCwdDropdown(spawnCwdInput.value.trim());
      spawnCwdInput.focus();
      return;
    }
    const item = e.target.closest('.cwd-dropdown-item');
    if (!item) { e.preventDefault(); return; }   // 余白クリックは入力欄フォーカス維持
    // お気に入り行は preventDefault しない: mousedown で preventDefault すると Chromium で
    // ネイティブ D&D が開始できなくなるため。選択確定は click 側に委ねる。
    if (item.classList.contains('is-favorite')) return;
    // 履歴行は従来どおり mousedown 即確定（フォーカス維持のため preventDefault）。
    e.preventDefault();
    selectCwdItem(item);
  });

  // お気に入り行の選択確定は click で行う（mousedown で確定するとドラッグ開始前に
  // ドロップダウンが閉じてしまうため）。ドラッグで並び替えた直後の click は抑止する。
  cwdDropdown.addEventListener('click', (e) => {
    if (cwdDragMoved) { cwdDragMoved = false; return; }
    if (e.target.closest('.cwd-dropdown-fav') || e.target.closest('.cwd-dropdown-del')) return;
    const item = e.target.closest('.cwd-dropdown-item.is-favorite');
    if (item) selectCwdItem(item);
  });

  // --- お気に入りの D&D 並び替え（C4）---
  function clearCwdDropIndicators() {
    cwdDropdown.querySelectorAll('.drop-before, .drop-after, .dragging')
      .forEach(el => el.classList.remove('drop-before', 'drop-after', 'dragging'));
  }
  function cwdDropIsAfter(item, clientY) {
    const r = item.getBoundingClientRect();
    return clientY > r.top + r.height / 2;
  }
  cwdDropdown.addEventListener('dragstart', (e) => {
    const item = e.target.closest('.cwd-dropdown-item.is-favorite');
    if (!item) { e.preventDefault(); return; }   // お気に入り以外はドラッグ不可
    cwdDragValue = item.dataset.value;
    cwdDragMoved = false;
    item.classList.add('dragging');
    try {
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData('text/plain', cwdDragValue);
    } catch (_) {}
  });
  cwdDropdown.addEventListener('dragover', (e) => {
    if (cwdDragValue == null) return;
    const item = e.target.closest('.cwd-dropdown-item.is-favorite');
    if (!item || item.dataset.value === cwdDragValue) return;
    e.preventDefault();                          // drop を許可
    try { e.dataTransfer.dropEffect = 'move'; } catch (_) {}
    cwdDropdown.querySelectorAll('.drop-before, .drop-after')
      .forEach(el => el.classList.remove('drop-before', 'drop-after'));
    item.classList.add(cwdDropIsAfter(item, e.clientY) ? 'drop-after' : 'drop-before');
  });
  cwdDropdown.addEventListener('drop', (e) => {
    if (cwdDragValue == null) return;
    const item = e.target.closest('.cwd-dropdown-item.is-favorite');
    if (!item || item.dataset.value === cwdDragValue) { clearCwdDropIndicators(); return; }
    e.preventDefault();
    const after = cwdDropIsAfter(item, e.clientY);
    const next = loadCwdFavorites().filter(v => v !== cwdDragValue);
    let ti = next.indexOf(item.dataset.value);
    if (ti < 0) { clearCwdDropIndicators(); return; }
    if (after) ti += 1;
    next.splice(ti, 0, cwdDragValue);
    cwdDragMoved = true;
    setUserPref('cwd_favorites', next);
    renderCwdDropdown(spawnCwdInput.value.trim());
    // 並び替え後もリストは開いたまま見せる。focus による再オープン(空フィルタ再描画)は抑止。
    focusInputNoReopen();
  });
  cwdDropdown.addEventListener('dragend', () => {
    clearCwdDropIndicators();
    cwdDragValue = null;
  });

  function isCodexHighRisk(currentModel, nextModel, sandbox, approval) {
    const current = (currentModel || '').trim();
    const next = (nextModel || '').trim();
    const modelChanged = !!next && next !== current;
    const permissionHigh = sandbox === 'danger-full-access' || approval === 'never';
    return modelChanged || permissionHigh;
  }

  function isClaudeHighRisk(currentModel, nextModel, permissionMode) {
    const current = (currentModel || '').trim();
    const next = (nextModel || '').trim();
    const modelChanged = !!next && next !== current;
    const permissionHigh = permissionMode === 'bypassPermissions';
    return modelChanged || permissionHigh;
  }

  // provider共通のモデル選択モーダル
  // isHighRiskFn(candidateModel) → bool
  // opts: { titleKey, permSummaryKey }
  function openModelModal(currentModel, isHighRiskFn, opts): Promise<any> {
    return new Promise<any>((resolve) => {
      const overlay = document.getElementById('model-picker-overlay');
      if (!overlay) { resolve(null); return; }
      const display = (currentModel || '').trim() || '(none)';
      overlay.innerHTML = '';
      overlay.hidden = false;

      const dialog = document.createElement('div');
      dialog.className = 'model-picker-dialog';
      dialog.innerHTML = `
        <div class="model-picker-title">${escapeHtml(t(opts.titleKey))}</div>
        <div class="model-picker-current">${escapeHtml(t('model_current', { model: display }))}</div>
        <label class="model-picker-note">${escapeHtml(t('model_candidate'))}</label>
        <input class="model-picker-input" id="model-candidate-input" type="text" list="spawn-model-datalist" value="${escapeHtml(currentModel || '')}">
        <div class="model-picker-note">${escapeHtml(t('model_summary'))}</div>
        <div class="model-picker-note">- ${escapeHtml(t('model_summary_cost'))}</div>
        <div class="model-picker-note">- ${escapeHtml(t(opts.permSummaryKey))}</div>
        <div class="model-picker-note">- ${escapeHtml(t('model_summary_compat'))}</div>
        <label class="model-picker-check" id="model-risk-check-wrap" hidden>
          <input id="model-risk-check" type="checkbox">
          <span>${escapeHtml(t('model_require_confirm'))}</span>
        </label>
        <div class="model-picker-actions">
          <button class="model-picker-btn" id="model-cancel-btn">${escapeHtml(t('model_cancel'))}</button>
          <button class="model-picker-btn primary" id="model-apply-btn">${escapeHtml(t('model_apply'))}</button>
        </div>
      `;
      overlay.appendChild(dialog);

      const input = document.getElementById('model-candidate-input');
      const riskWrap = document.getElementById('model-risk-check-wrap');
      const riskCheck = document.getElementById('model-risk-check');
      const applyBtn = document.getElementById('model-apply-btn');
      const cancelBtn = document.getElementById('model-cancel-btn');

      function refreshRisk() {
        const highRisk = isHighRiskFn(input.value);
        riskWrap.hidden = !highRisk;
        applyBtn.disabled = highRisk && !riskCheck.checked;
      }
      function close(v) {
        overlay.removeEventListener('click', onOverlayClick);
        overlay.hidden = true;
        overlay.innerHTML = '';
        resolve(v);
      }
      function onOverlayClick(e) {
        if (e.target === overlay) close(null);
      }

      input.addEventListener('input', refreshRisk);
      riskCheck.addEventListener('change', refreshRisk);
      cancelBtn.addEventListener('click', () => close(null));
      applyBtn.addEventListener('click', () => {
        const candidate = input.value.trim();
        if (!candidate) {
          alert(t('model_model_required'));
          return;
        }
        const highRisk = isHighRiskFn(candidate);
        close({
          model: candidate,
          mode: highRisk ? 'required' : 'explicit',
          risk_confirmed: highRisk ? !!riskCheck.checked : false,
        });
      });
      overlay.addEventListener('click', onOverlayClick);
      input.focus();
      refreshRisk();
    });
  }

  function openCodexModelModal() {
    populateModelDatalist();
    const currentModel = (spawnModelInput.value || '').trim();
    const sandbox = document.getElementById('spawn-sandbox').value;
    const approval = document.getElementById('spawn-ask-approval').value;
    return openModelModal(
      currentModel,
      (candidate) => isCodexHighRisk(spawnModelInput.value, candidate, sandbox, approval),
      { titleKey: 'codex_model_title', permSummaryKey: 'codex_model_summary_permission' }
    );
  }

  function openClaudeModelModal() {
    populateModelDatalist();
    const currentModel = (spawnModelInput.value || '').trim();
    const permMode = document.getElementById('spawn-permission-mode').value;
    return openModelModal(
      currentModel,
      (candidate) => isClaudeHighRisk(spawnModelInput.value, candidate, permMode),
      { titleKey: 'claude_model_title', permSummaryKey: 'claude_model_summary_permission' }
    );
  }

  if (spawnCodexModelBtn) {
    spawnCodexModelBtn.addEventListener('click', async () => {
      const picked = await openCodexModelModal();
      if (!picked) return;
      setSpawnModelValue(picked.model);
      codexModelSelection = picked;
    });
  }

  if (spawnClaudeModelBtn) {
    spawnClaudeModelBtn.addEventListener('click', async () => {
      const picked = await openClaudeModelModal();
      if (!picked) return;
      setSpawnModelValue(picked.model);
      claudeModelSelection = picked;
    });
  }

  async function spawnSession() {
    const provider = document.getElementById('spawn-provider').value;
    const cwd = spawnCwdInput.value.trim();
    spawnLaunchBtn.disabled = true;
    try {
      const model = spawnModelInput.value.trim();
      const label = document.getElementById('spawn-label').value.trim();
      if (model && !isModelCompatibleWithProvider(provider, model)) {
        setSpawnModelValue('');
        clearModelSelectionState();
        showToast(t('spawn_model_provider_mismatch'));
        spawnLaunchBtn.disabled = false;
        return;
      }
      const route = resolveRoute(provider, model);

      // Ollama route の場合: Windows + PowerShell + 非 UTF-8 環境を検出して警告
      let utf8Session = false;
      if (route === 'ollama') {
        try {
          const encRes = await fetch(`/api/encoding-check?token=${token}`);
          if (encRes.ok) {
            const encData = await encRes.json();
            if (encData.is_windows && encData.is_powershell && !encData.is_utf8) {
              const choice = await appConfirmOllamaEncoding();
              if (choice === null) {
                spawnLaunchBtn.disabled = false;
                return;
              }
              utf8Session = (choice === 'utf8');
            }
          }
        } catch (_) {}
      }

      // Shell は model / route / permission 系フィールドを送らない
      const bodyObj: any = { provider, cwd, label };
      if (provider !== 'shell') bodyObj.model = model;
      if (utf8Session) bodyObj.utf8_session = true;
      if (provider !== 'shell' && route) bodyObj.route = route;
      if (provider === 'claude') {
        const picked = claudeModelSelection;
        const permMode = document.getElementById('spawn-permission-mode').value;
        const highRisk = isClaudeHighRisk('', model, permMode);
        const pickedConfirmed = !!picked?.risk_confirmed;
        let riskConfirmed = pickedConfirmed;

        if (highRisk && !riskConfirmed) {
          riskConfirmed = await appConfirm({
            title: t('claude_model_confirm_title'),
            message: t('claude_model_confirm_message'),
            confirmText: t('claude_model_confirm_run'),
            cancelText: t('spawn_cancel'),
            kind: 'danger',
          });
          if (!riskConfirmed) {
            spawnLaunchBtn.disabled = false;
            return;
          }
        }

        bodyObj.permission_mode = permMode;
        bodyObj.model_selection_mode = picked ? picked.mode : 'auto';
        bodyObj.risk_confirmed = riskConfirmed;
      } else if (provider === 'codex') {
        const picked = codexModelSelection;
        const sandbox = document.getElementById('spawn-sandbox').value;
        const approval = document.getElementById('spawn-ask-approval').value;
        const highRisk = isCodexHighRisk('', model, sandbox, approval);
        const pickedConfirmed = !!picked?.risk_confirmed;
        let riskConfirmed = pickedConfirmed;

        if (highRisk && !riskConfirmed) {
          riskConfirmed = await appConfirm({
            title: t('codex_model_confirm_title'),
            message: t('codex_model_confirm_message'),
            confirmText: t('codex_model_confirm_run'),
            cancelText: t('spawn_cancel'),
            kind: 'danger',
          });
          if (!riskConfirmed) {
            spawnLaunchBtn.disabled = false;
            return;
          }
        }

        bodyObj.model_selection_mode = highRisk ? 'required' : (picked?.mode || 'auto');
        bodyObj.risk_confirmed = riskConfirmed;
        bodyObj.sandbox = sandbox;
        bodyObj.ask_for_approval = approval;
      }
      const res = await fetch(`/api/spawn?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(bodyObj),
      });
      if (res.ok) {
        saveCwdHistory(cwd);
        // Ollama route のモデルは default として保存しない。
        // 残すと次回 spawn dialog で Ollama モデルが pre-fill されたまま起動 →
        // spawn 時 env (ANTHROPIC_BASE_URL=localhost:11434 等) が焼き付き、
        // そのセッション内で /model が blocked になる罠を踏むため。
        // Claude/Codex の純正モデル選択は引き続き sticky に残す。
        const persistedModel = route === 'ollama' ? '' : model;
        const openTarget = getSpawnOpenTarget();
        const gridLayout = getSpawnGridLayout();
        const detachedPreset = spawnDetachedPreset ? spawnDetachedPreset.value : 'single';
        saveSpawnSettings({
          provider,
          cwd,
          model: persistedModel,
          open_target: openTarget,
          grid_layout: gridLayout,
          detached_preset: detachedPreset,
          ...(provider === 'claude' ? { permission_mode: bodyObj.permission_mode } : {}),
          ...(provider === 'codex'  ? { sandbox: bodyObj.sandbox, ask_for_approval: bodyObj.ask_for_approval } : {}),
        });
        document.getElementById('spawn-label').value = '';
        codexModelSelection  = null;
        claudeModelSelection = null;
        newSessionPanel.hidden = true;

        // C2 / C5: Detached window 選択時 — preset に応じた起動フローを実行する
        if (openTarget === 'detached') {
          if (detachedPreset === 'project') {
            // project プリセット: 現在の provider のプロジェクトグループのセッションを別窓表示
            if (typeof (window as any).openDetachedGridLauncher === 'function') {
              (window as any).openDetachedGridLauncher({ cwd });
            }
          } else if (detachedPreset === 'multi') {
            // multi プリセット: 現在の Multi layout のセッションを別窓へ切り出す
            if (typeof (window as any).launchDetachedPreset === 'function') {
              (window as any).launchDetachedPreset({ presetId: 'current-multi', layout: gridLayout }).catch(() => {});
            }
          } else if (detachedPreset === 'claude-shell-2x2') {
            // AI + Shell 2x2: 起動した AI session + Shell 3枚で grid を開く
            if (typeof (window as any).launchDetachedPreset === 'function') {
              (window as any).launchDetachedPreset({
                presetId: 'claude+shell-2x2',
                layout: gridLayout,
                count: 4,
                cwd,
                provider,
              }).catch(() => {});
            }
          } else if (detachedPreset === 'shell-2x2') {
            if (typeof (window as any).launchDetachedPreset === 'function') {
              (window as any).launchDetachedPreset({
                presetId: 'shell-2x2',
                layout: '2x2',
                count: 4,
                cwd,
              }).catch(() => {});
            }
          } else if (detachedPreset === 'shell-3x3') {
            if (typeof (window as any).launchDetachedPreset === 'function') {
              (window as any).launchDetachedPreset({
                presetId: 'shell-3x3',
                layout: '3x3',
                count: 9,
                cwd,
              }).catch(() => {});
            }
          } else if (detachedPreset === 'advanced') {
            // advanced: Launcher ダイアログを開く
            if (typeof (window as any).openDetachedGridLauncher === 'function') {
              (window as any).openDetachedGridLauncher({ cwd });
            }
          } else {
            // single (デフォルト): 新しいセッションを別窓 1x1 で表示
            _waitForNewSessionAndOpenGrid(gridLayout);
          }
          set_pendingAutoSwitch(false);
        } else {
          set_pendingAutoSwitch(true);
        }
      } else {
        alert(t('spawn_failed') + await res.text());
      }
    } catch (e) {
      alert(t('spawn_failed') + e.message);
    } finally {
      spawnLaunchBtn.disabled = false;
    }
  }
})();
