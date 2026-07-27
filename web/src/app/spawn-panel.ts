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
  const orchestrationBtn = document.getElementById('orchestration-btn');
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

  // ---- C1: オーケストレーション（plan_orchestration-spawn-ui-exposure.md） ----
  // 「オーケストレーション」ボタンから開いたときだけ true。同じ起動フォームを共用し、
  // true のときだけ子役割の詳細設定アコーディオンを見せ、spawn リクエストに
  // orchestration フラグ（+ 設定されていれば役割マッピング）を載せる。
  let spawnOrchestrationMode = false;
  const spawnOrchestrationSection  = document.getElementById('spawn-orchestration-section');
  const spawnOrchestrationBypassNote = document.getElementById('spawn-orchestration-bypass-note');
  const spawnOrchestrationSummary  = document.getElementById('spawn-orchestration-summary');
  const spawnRoleTableBody         = document.getElementById('spawn-role-table-body');

  const ORCHESTRATION_ROLE_DEFS = [
    { key: 'implementation', labelKey: 'spawn_role_implementation' },
    { key: 'test',           labelKey: 'spawn_role_test' },
    { key: 'review',         labelKey: 'spawn_role_review' },
  ];
  // Shell はロールの CLI 候補としては意味を持たないため除外。
  const ORCHESTRATION_CLI_OPTIONS = [
    { value: '',             labelKey: 'spawn_role_cli_none' },
    { value: 'claude',       label: 'Claude Code' },
    { value: 'codex',        label: 'Codex CLI' },
    { value: 'copilot',      label: 'GitHub Copilot' },
    { value: 'cursor-agent', label: 'Cursor Agent' },
    { value: 'opencode',     label: 'OpenCode' },
    { value: 'grok',         label: 'Grok Build' },
  ];

  function syncRoleModelDisabledState(): void {
    spawnRoleTableBody?.querySelectorAll('tr').forEach(tr => {
      const cli = tr.querySelector<HTMLSelectElement>('.spawn-role-cli');
      const model = tr.querySelector<HTMLInputElement>('.spawn-role-model');
      if (!cli || !model) return;
      model.disabled = !cli.value;
      if (!cli.value) model.value = '';
    });
  }

  // role → {provider, model} | null のマッピングと、実際に設定された件数を返す。
  function collectOrchestrationRoles(): { roles: Record<string, { provider: string; model: string } | null>; count: number } {
    const roles: Record<string, { provider: string; model: string } | null> = {};
    let count = 0;
    spawnRoleTableBody?.querySelectorAll('tr').forEach(tr => {
      const role = (tr as HTMLElement).dataset.role;
      const cli = tr.querySelector<HTMLSelectElement>('.spawn-role-cli');
      const model = tr.querySelector<HTMLInputElement>('.spawn-role-model');
      if (!role || !cli) return;
      if (!cli.value) { roles[role] = null; return; }
      roles[role] = { provider: cli.value, model: (model?.value || '').trim() };
      count++;
    });
    return { roles, count };
  }

  function updateOrchestrationSummary(): void {
    if (!spawnOrchestrationSummary) return;
    const { count } = collectOrchestrationRoles();
    spawnOrchestrationSummary.textContent = count > 0
      ? t('spawn_orchestration_roles_summary_set', { count })
      : t('spawn_orchestration_roles_summary_empty');
  }

  function buildOrchestrationRoleTable(): void {
    if (!spawnRoleTableBody) return;
    spawnRoleTableBody.innerHTML = ORCHESTRATION_ROLE_DEFS.map(r => {
      const cliOptions = ORCHESTRATION_CLI_OPTIONS.map(o =>
        `<option value="${o.value}">${escapeHtml(o.label || t(o.labelKey))}</option>`
      ).join('');
      return (
        `<tr data-role="${r.key}">` +
        `<td>${escapeHtml(t(r.labelKey))}</td>` +
        `<td><select class="spawn-role-cli">${cliOptions}</select></td>` +
        `<td><input type="text" class="spawn-role-model" data-i18n-placeholder="spawn_role_model_placeholder" placeholder="${escapeHtml(t('spawn_role_model_placeholder'))}" disabled></td>` +
        `</tr>`
      );
    }).join('');
    spawnRoleTableBody.querySelectorAll('.spawn-role-cli').forEach(sel => {
      sel.addEventListener('change', () => { syncRoleModelDisabledState(); updateOrchestrationSummary(); });
    });
    spawnRoleTableBody.querySelectorAll('.spawn-role-model').forEach(inp => {
      inp.addEventListener('input', updateOrchestrationSummary);
    });
    syncRoleModelDisabledState();
    updateOrchestrationSummary();
  }

  function setSpawnOrchestrationMode(on: boolean): void {
    spawnOrchestrationMode = on;
    if (spawnOrchestrationSection) (spawnOrchestrationSection as HTMLDetailsElement).hidden = !on;
    // 子セッションが承認スキップ（全許可）で自走する旨の注意はオーケストレーション時のみ見せる
    if (spawnOrchestrationBypassNote) spawnOrchestrationBypassNote.hidden = !on;
    if (on && spawnRoleTableBody && !spawnRoleTableBody.children.length) buildOrchestrationRoleTable();
    if (spawnOrchestrationSection) (spawnOrchestrationSection as HTMLDetailsElement).open = false;
  }

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
    if (provider === 'grok') {
      return spawnModelGroups.filter(g => g && g.provider === 'grok' && Array.isArray(g.models));
    }
    const groups = spawnModelGroups.filter(g => g && Array.isArray(g.models) && (!g.provider || g.provider === provider));
    groups.sort((a, b) => {
      const rank = (g) => {
        if (g.provider === provider) return 0;
        if (g.label === 'Ollama Cloud') return 1;
        if (g.label === 'Ollama Local') return 2;
        if (g.label === 'LM Studio') return 3;
        return 4;
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
    if (['ollama', 'lm-studio'].includes(resolveRoute(spawnProviderEl.value, m))) {
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
    if (provider === 'grok') {
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
    const opencodeNote = document.getElementById('spawn-opencode-note');
    const grokNote = document.getElementById('spawn-grok-note');
    const shellNote = document.getElementById('spawn-shell-note');
    if (claudeNote) claudeNote.hidden = (p !== 'claude');
    if (codexNote) codexNote.hidden = (p !== 'codex');
    if (copilotNote) copilotNote.hidden = (p !== 'copilot');
    if (cursorAgentNote) cursorAgentNote.hidden = (p !== 'cursor-agent');
    if (opencodeNote) opencodeNote.hidden = (p !== 'opencode');
    if (grokNote) grokNote.hidden = (p !== 'grok');
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
        const opencodeNote = document.getElementById('spawn-opencode-note');
        const grokNote = document.getElementById('spawn-grok-note');
        const shellNote = document.getElementById('spawn-shell-note');
        if (claudeNote) claudeNote.hidden = (p !== 'claude');
        if (codexNote) codexNote.hidden = (p !== 'codex');
        if (copilotNote) copilotNote.hidden = (p !== 'copilot');
        if (cursorAgentNote) cursorAgentNote.hidden = (p !== 'cursor-agent');
        if (opencodeNote) opencodeNote.hidden = (p !== 'opencode');
        if (grokNote) grokNote.hidden = (p !== 'grok');
        if (shellNote) shellNote.hidden = !isShell;
      }
      if (s.cwd)              spawnCwdInput.value = s.cwd;
      // モデル欄は保存値を prefill しない（空 = 各 CLI の既定モデルを尊重）。
      // 以前は前回モデルを復元して明示送信していたが、それが claude CLI の
      // /model 既定（1M 窓など）を 200K へ上書きする原因だった。非既定モデルを
      // 使いたいときは datalist から明示選択する（その選択はそのセッションにのみ適用）。
      // s.model は後方互換のため保存自体は残すが、ここでは読み込まない。
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

  // localStorage に非配列 JSON（null/数値/object）が紛れ込んでも .filter / .unshift が
  // TypeError で落ちないよう Array.isArray ガードで [] にフォールバックする
  // （user-prefs サーバー同期で非配列がプッシュされた場合の防御）。
  function loadCwdHistory() {
    try {
      const v = JSON.parse(localStorage.getItem(STORAGE_CWD_HISTORY_KEY) || '[]');
      return Array.isArray(v) ? v : [];
    } catch (_) { return []; }
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
    try {
      const v = JSON.parse(localStorage.getItem(STORAGE_CWD_FAVORITES_KEY) || '[]');
      return Array.isArray(v) ? v : [];
    } catch (_) { return []; }
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

  // お気に入り表示順: 最終フォルダ名（basename）昇順。同名時はフルパスで安定化。
  function compareCwdByBasename(a: string, b: string): number {
    const ba = splitCwdPath(a).basename;
    const bb = splitCwdPath(b).basename;
    return ba.localeCompare(bb) || a.localeCompare(b);
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
      // 失敗時は空配列をキャッシュに書かない（次回呼び出しでリトライできるよう）。
      // 一時的な 5xx / 403 / 認証 rotate 直後の 401 で permanent に空表示になる回帰を防ぐ。
      if (!res.ok) { return []; }
      const data = await res.json();
      const items = Array.isArray(data.subdirs) ? data.subdirs : [];
      // 成功（hit=true）のときだけキャッシュへ書く。空ディレクトリも valid な結果。
      subdirsCache.set(parent, items);
      return items;
    } catch (_) {
      // ネットワーク例外もキャッシュしない（同上の理由）。
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

  // ---- C1: 検索ドリブン ロジック層 ----
  // D9 スコアリング用の型。C2 で描画に使う。export しない（module 内参照のみ）。
  interface SearchResult {
    path: string;
    basename: string;
    isFav: boolean;
    isHist: boolean;
    isUnregistered: boolean;
    histIndex: number;
    score: number;
  }

  // 入力値を prefix / query / isPath の 3 値に分解する。
  // 判定順: D2 に従い、Windows パス先頭 (<英字>:\) は isPath 扱いで prefix 抽出しない。
  function parseCwdInput(value: string): { prefix: string | null; query: string; isPath: boolean } {
    // \ か / を含む、もしくは Windows パス先頭 (<英字>:\) → isPath
    if (/[/\\]/.test(value) || /^[A-Za-z]:\\/.test(value)) {
      return { prefix: null, query: value, isPath: true };
    }
    // prefix 判定: 先頭が <英字 1+>: で、: 直後が \ でも / でもない場合のみ
    const m = value.match(/^([A-Za-z]+):([^/\\].*)?$/);
    if (m) {
      return { prefix: m[1], query: m[2] ?? '', isPath: false };
    }
    return { prefix: null, query: value, isPath: false };
  }

  // お気に入りリストから既知ルートを派生させる。
  // 各 fav の親ディレクトリの末尾セグメントを短縮名キー（値はフルパス）とする。
  // 同名衝突時は親 1 段追加した形（例: "github\public"）に変更する。
  function deriveRootsFromFavorites(favs: string[]): Map<string, string> {
    // 各 fav の親ディレクトリを取得する。
    function parentOf(p: string): string {
      const v = p.replace(/[/\\]+$/, '');
      const sep = v.includes('\\') ? '\\' : '/';
      const idx = Math.max(v.lastIndexOf('\\'), v.lastIndexOf('/'));
      if (idx < 0) return v;
      return v.slice(0, idx) || sep;
    }
    function segmentOf(p: string, depth = 1): string {
      const parts = p.replace(/[/\\]+$/, '').split(/[/\\]/);
      return parts.slice(-depth).join('\\');
    }

    // まず全 fav の親パスを集める（重複排除）。
    const parents = [...new Set(favs.map(parentOf))];
    // 短縮名 → フルパスの候補マップ（衝突検出用）。
    const nameToPath = new Map<string, string>();
    const collisions = new Set<string>();
    for (const p of parents) {
      const name = segmentOf(p, 1);
      if (nameToPath.has(name) && nameToPath.get(name) !== p) {
        collisions.add(name);
      } else {
        nameToPath.set(name, p);
      }
    }
    // 衝突したエントリは depth=2 で再登録する。
    const result = new Map<string, string>();
    for (const p of parents) {
      const name = segmentOf(p, 1);
      if (collisions.has(name)) {
        result.set(segmentOf(p, 2), p);
      } else {
        result.set(name, p);
      }
    }
    return result;
  }

  // 全ルートを並列 pre-scan して subdirsCache を充填する。
  // dropdown が開いた瞬間に呼び出し、結果が来たら必要に応じて再描画するよう設計。
  // TODO(D11): 隠しフォルダ除外は API 確認後（/api/list-subdirs 側の返却内容次第）
  async function prescanRoots(roots: Map<string, string>): Promise<void> {
    await Promise.all([...roots.values()].map(p => fetchSubdirs(p)));
  }

  // query / prefix に基づいて全ルートのサブディレクトリを横断検索し、SearchResult[] を返す。
  // prefix 指定時はそのルートのみ検索。無指定時は全ルート。
  // 重複パスは favSet / histSet の状態で統合される（set 内にあれば isFav/isHist が立つ）。
  function buildSearchResults(
    query: string,
    prefix: string | null,
    allSubdirs: Map<string, string>,
    favSet: Set<string>,
    histSet: Set<string>,
    histIndex: Map<string, number>,
  ): SearchResult[] {
    const lowQuery = query.toLowerCase();
    const seen = new Set<string>();
    const results: SearchResult[] = [];

    for (const [shortName, rootPath] of allSubdirs) {
      // prefix が指定されていてこのルートと一致しない場合はスキップ。
      if (prefix !== null && shortName.toLowerCase() !== prefix.toLowerCase()) continue;

      const sep = rootPath.includes('\\') ? '\\' : '/';
      const subdirs = subdirsCache.get(rootPath) ?? [];
      for (const name of subdirs) {
        const fullPath = rootPath + sep + name;
        if (seen.has(fullPath)) continue;
        // basename に query が含まれるもののみ結果対象。query が空の場合は全件。
        if (lowQuery && !name.toLowerCase().includes(lowQuery)) continue;
        seen.add(fullPath);

        const isFav = favSet.has(fullPath);
        const isHist = histSet.has(fullPath);
        const isUnregistered = !isFav && !isHist;
        const historyOrder = histIndex.get(fullPath) ?? Number.MAX_SAFE_INTEGER;

        // D9 スコアリング: 一致種別 + 登録種別の合算。
        let matchScore = 0;
        const lowName = name.toLowerCase();
        if (lowQuery) {
          if (lowName === lowQuery) matchScore = 100;
          else if (lowName.startsWith(lowQuery)) matchScore = 50;
          else matchScore = 10;
        }
        const regScore = isFav ? 5 : isHist ? 2 : 0;

        results.push({
          path: fullPath,
          basename: name,
          isFav,
          isHist,
          isUnregistered,
          histIndex: historyOrder,
          score: matchScore + regScore,
        });
      }
    }

    // お気に入りは最終フォルダ名昇順、履歴は cwd_history の先頭（最新）から降順で出す。
    // 未登録の候補だけ従来通りスコア優先にして、検索時の発見性を保つ。
    results.sort((a, b) => {
      if (a.isFav || b.isFav) {
        if (a.isFav && b.isFav) return a.basename.localeCompare(b.basename) || a.path.localeCompare(b.path);
        return a.isFav ? -1 : 1;
      }
      if (a.isHist || b.isHist) {
        if (a.isHist && b.isHist) return a.histIndex - b.histIndex || a.basename.localeCompare(b.basename);
        return a.isHist ? -1 : 1;
      }
      return b.score - a.score || a.basename.localeCompare(b.basename);
    });
    return results;
  }
  // ---- /C1: 検索ドリブン ロジック層 ----

  function renderCwdDropdown(filter) {
    const favs = loadCwdFavorites();
    const favSet = new Set(favs);
    const hist = loadCwdHistory();
    const histSet = new Set(hist);
    const histIndex = new Map(hist.map((v, i) => [v, i]));
    const roots = deriveRootsFromFavorites(favs);

    // 入力値を解析して prefix / query / isPath を得る。
    const parsed = parseCwdInput(filter);

    // isPath のときは既存の subdirs 展開ロジックに完全に委ねる。
    // chip 行は出してもよいが検索結果セクションは出さない。
    const isPathMode = parsed.isPath;

    // subdirs 展開（末尾区切り文字入力時）。
    const subItems: string[] = subdirsCurrent
      ? subdirsCurrent.items.map(name => subdirsCurrent!.parent + subdirsCurrent!.sep + name)
      : [];

    // ---- chip 行 ----
    // お気に入りが 0 件で roots も空のときは chip 行を出さない。
    const hasRoots = roots.size > 0;

    // 各 chip のマッチ件数（キャッシュ済みサブディレクトリ数をカウント）。
    function countForRoot(shortName: string, rootPath: string): number {
      const subdirs = subdirsCache.get(rootPath) ?? [];
      if (!parsed.query) return subdirs.length;
      const low = parsed.query.toLowerCase();
      return subdirs.filter(n => n.toLowerCase().includes(low)).join('').length > 0
        ? subdirs.filter(n => n.toLowerCase().includes(low)).length
        : subdirs.filter(n => n.toLowerCase().includes(low)).length;
    }

    // chip の active 判定: input が "<prefix>:" で始まっているか。
    function isChipActive(shortName: string): boolean {
      if (!parsed.prefix) return false;
      return parsed.prefix.toLowerCase() === shortName.toLowerCase();
    }

    // ---- 検索結果（isPath でないとき）----
    let searchItems: SearchResult[] = [];
    if (!isPathMode && roots.size > 0) {
      searchItems = buildSearchResults(parsed.query, parsed.prefix, roots, favSet, histSet, histIndex);
    }

    // 検索結果に出たパスセットを記録して fav/hist から除外する。
    const searchPathSet = new Set(searchItems.map(r => r.path));

    // fav/hist の絞り込み（検索結果に出たものは除外）。
    // お気に入りは最終フォルダ名（basename）昇順で固定表示する。
    const favItems = (filter
      ? favs.filter(v => !searchPathSet.has(v) && v.toLowerCase().includes(filter.toLowerCase()))
      : favs.filter(v => !searchPathSet.has(v)))
      .slice()
      .sort(compareCwdByBasename);
    const histItems = (filter
      ? hist.filter(v => !favSet.has(v) && !searchPathSet.has(v) && v.toLowerCase().includes(filter.toLowerCase()))
      : hist.filter(v => !favSet.has(v) && !searchPathSet.has(v)));

    // isPath モードのときは検索結果を出さないので fav/hist の除外も不要にリセット。
    const effectiveFavItems = isPathMode
      ? (filter ? favs.filter(v => v.toLowerCase().includes(filter.toLowerCase())) : favs.slice())
          .sort(compareCwdByBasename)
      : favItems;
    const effectiveHistItems = isPathMode
      ? (filter ? hist.filter(v => !favSet.has(v) && v.toLowerCase().includes(filter.toLowerCase())) : hist.filter(v => !favSet.has(v)))
      : histItems;

    const allItems = [...subItems, ...(isPathMode ? [] : searchItems.map(r => r.path)), ...effectiveFavItems, ...effectiveHistItems];
    const hasAny = allItems.length > 0 || hasRoots;
    if (!hasAny) { cwdDropdown.hidden = true; return; }

    function renderRow(v: string, fav: boolean, isSub = false) {
      const labelFilter = isSub ? '' : filter;
      return (
        `<li class="cwd-dropdown-item${fav ? ' is-favorite' : ''}${isSub ? ' is-subdir' : ''}" tabindex="-1" data-value="${escapeHtml(v)}">` +
        `<button class="cwd-dropdown-fav${fav ? ' is-on' : ''}" tabindex="-1" data-value="${escapeHtml(v)}" ` +
        `title="${escapeHtml(t(fav ? 'spawn_cwd_unfavorite' : 'spawn_cwd_favorite'))}">${fav ? '★' : '☆'}</button>` +
        `<span class="cwd-dropdown-label" title="${escapeHtml(v)}">${buildCwdLabelHtml(v, labelFilter)}</span>` +
        (isSub ? '' : `<button class="cwd-dropdown-del" tabindex="-1" data-value="${escapeHtml(v)}">×</button>`) +
        `</li>`
      );
    }

    function renderSearchRow(r: SearchResult) {
      const fav = r.isFav;
      return (
        `<li class="cwd-dropdown-item${fav ? ' is-favorite' : ''}" tabindex="-1" data-value="${escapeHtml(r.path)}">` +
        `<button class="cwd-dropdown-fav${fav ? ' is-on' : ''}" tabindex="-1" data-value="${escapeHtml(r.path)}" ` +
        `title="${escapeHtml(t(fav ? 'spawn_cwd_unfavorite' : 'spawn_cwd_favorite'))}">${fav ? '★' : '☆'}</button>` +
        `<span class="cwd-dropdown-mag" aria-hidden="true">🔍</span>` +
        `<span class="cwd-dropdown-label" title="${escapeHtml(r.path)}">${buildCwdLabelHtml(r.path, parsed.query)}</span>` +
        `<button class="cwd-dropdown-del" tabindex="-1" data-value="${escapeHtml(r.path)}">×</button>` +
        `</li>`
      );
    }

    const noMatch = !isPathMode && searchItems.length === 0 && effectiveFavItems.length === 0 && effectiveHistItems.length === 0 && subItems.length === 0;

    let html = '';

    // chip 行（roots がある場合のみ）。launcher chip 1 個に圧縮し hover/click で popover を開く。
    if (hasRoots) {
      const chipsClass = noMatch ? 'cwd-dropdown-chips has-no-match' : 'cwd-dropdown-chips';
      // active root と合計件数を集計。launcher の表示に使う。
      let activeRoot: string | null = null;
      let totalCount = 0;
      for (const [shortName, rootPath] of roots) {
        if (isChipActive(shortName)) activeRoot = shortName;
        totalCount += countForRoot(shortName, rootPath);
      }
      const launcherInner = activeRoot
        ? `${escapeHtml(activeRoot)}:`
        : `<span class="chip-count">${totalCount}</span>`;
      let chipsHtml = `<li class="${chipsClass}">` +
        `<div class="cwd-dropdown-chip-collapsed">` +
        `<button class="cwd-dropdown-chip-launcher${activeRoot ? ' is-active' : ''}" type="button" aria-haspopup="true">` +
        `<span class="chip-icon" aria-hidden="true">🗂</span> ${launcherInner} <span class="chip-caret" aria-hidden="true">▾</span>` +
        `</button>` +
        `<div class="cwd-dropdown-chip-popover" role="menu">`;
      for (const [shortName, rootPath] of roots) {
        const count = countForRoot(shortName, rootPath);
        const activeClass = isChipActive(shortName) ? ' is-active' : '';
        chipsHtml +=
          `<button class="cwd-dropdown-chip${activeClass}" type="button" data-prefix="${escapeHtml(shortName)}">` +
          `${escapeHtml(shortName)}:` +
          `<span class="chip-count">${count}</span>` +
          `</button>`;
      }
      chipsHtml += `</div></div>`;
      if (noMatch) {
        chipsHtml += `</li>` +
          `<li class="cwd-dropdown-no-match" aria-hidden="true">${escapeHtml(t('spawn_cwd_no_match_hint'))}</li>`;
      } else {
        chipsHtml += `</li>`;
      }
      html += chipsHtml;
    } else if (noMatch) {
      // roots が空でも 0 件案内は出す。
      html += `<li class="cwd-dropdown-no-match" aria-hidden="true">${escapeHtml(t('spawn_cwd_no_match_hint'))}</li>`;
    }

    // subdirs セクション（末尾区切り入力時）。
    if (subItems.length > 0) {
      html += `<li class="cwd-dropdown-header" aria-hidden="true">${escapeHtml(t('spawn_cwd_section_subdirs'))}</li>`;
      html += subItems.map(v => renderRow(v, false, true)).join('');
    }

    // 検索結果セクション（isPath でないとき）。
    if (!isPathMode && searchItems.length > 0) {
      html += `<li class="cwd-dropdown-header is-search" aria-hidden="true">${escapeHtml(t('spawn_cwd_section_search'))}</li>`;
      html += searchItems.map(r => renderSearchRow(r)).join('');
    }

    // fav セクション。
    if (effectiveFavItems.length > 0) {
      html += `<li class="cwd-dropdown-header" aria-hidden="true">${escapeHtml(t('spawn_cwd_section_favorites'))}</li>`;
      html += effectiveFavItems.map(v => renderRow(v, true)).join('');
    }

    // history セクション。
    if (effectiveHistItems.length > 0) {
      html += `<li class="cwd-dropdown-header" aria-hidden="true">${escapeHtml(t('spawn_cwd_section_history'))}</li>`;
      html += effectiveHistItems.map(v => renderRow(v, false)).join('');
    }

    cwdDropdown.innerHTML = html;
    cwdDropdown.hidden = false;
    applyDropdownMissingStatus();
    // chip クリックは cwdDropdown の委譲 mousedown ハンドラ（下方）で処理するため
    // ここでの個別リスナ登録は不要。
    checkPathsExist(allItems).then(applyDropdownMissingStatus);
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
  async function openSpawnPanelForMode(orchestration: boolean): Promise<void> {
    if (!newSessionPanel.hidden) { newSessionPanel.hidden = true; return; }
    setSpawnOrchestrationMode(orchestration);
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
  }

  newSessionBtn.addEventListener('click', () => { openSpawnPanelForMode(false); });
  // C1: plan_orchestration-spawn-ui-exposure.md — 通常の起動フォームを共用しつつ、
  // このボタン経由の起動だけ orchestration フラグを立てる。
  if (orchestrationBtn) {
    orchestrationBtn.addEventListener('click', () => { openSpawnPanelForMode(true); });
  }

  // app.ts（shell セッション内で AI CLI 起動コマンドを検知した誘導）から呼ばれる。
  // 検知した provider と元 shell セッションの cwd をプリセットして新規セッションパネルを開く。
  async function openSpawnFor(provider: string, cwd: string): Promise<void> {
    // shell セッションからの誘導起動は常に通常モード（オーケストレーションではない）。
    setSpawnOrchestrationMode(false);
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
    // ドロップダウン開く前に全ルートをバックグラウンドで pre-scan してキャッシュを充填する。
    { const favs = loadCwdFavorites(); prescanRoots(deriveRootsFromFavorites(favs)); }
    if (!spawnModelGroups) {
      fetchModelGroups(false).catch(() => {});
    } else {
      populateModelDatalist();
      clearOllamaModelDefault();
    }
  }
  (window as any).openSpawnFor = openSpawnFor;

  spawnCancelBtn.addEventListener('click', () => { newSessionPanel.hidden = true; setSpawnOrchestrationMode(false); });
  spawnLaunchBtn.addEventListener('click', spawnSession);
  // Web folder browser — used when the OS native picker is unavailable
  // (headless Linux / remote Hub with no zenity|kdialog) or when env_kind is
  // remote (native dialogs would open on the server, not the user's browser).
  // Navigates via /api/list-subdirs and writes the chosen path into #spawn-cwd.
  let webDirBrowserEl: HTMLElement | null = null;
  let webDirBrowserPath = '';

  function webDirSep(p: string): string {
    return p.includes('\\') && !p.startsWith('/') ? '\\' : '/';
  }
  function webDirJoin(parent: string, name: string): string {
    const sep = webDirSep(parent);
    if (!parent) return name;
    if (parent.endsWith('/') || parent.endsWith('\\')) return parent + name;
    return parent + sep + name;
  }
  function webDirParent(p: string): string {
    const clean = stripTrailingSep(p);
    if (!clean) return p;
    // Windows drive root "C:\"
    if (/^[A-Za-z]:$/.test(clean)) return clean + '\\';
    if (/^[A-Za-z]:\\$/.test(p) || clean === '/' || /^[A-Za-z]:$/.test(clean)) return clean.includes(':') ? clean + '\\' : '/';
    const sep = webDirSep(clean);
    const idx = Math.max(clean.lastIndexOf('/'), clean.lastIndexOf('\\'));
    if (idx <= 0) return sep === '/' ? '/' : clean;
    // Keep "C:\" not "C:"
    if (/^[A-Za-z]:$/.test(clean.slice(0, idx))) return clean.slice(0, idx + 1);
    return clean.slice(0, idx) || (sep === '/' ? '/' : clean);
  }

  function ensureWebDirBrowser(): HTMLElement {
    if (webDirBrowserEl) return webDirBrowserEl;
    const overlay = document.createElement('div');
    overlay.id = 'spawn-web-dir-browser';
    overlay.className = 'spawn-web-dir-overlay';
    overlay.hidden = true;
    overlay.innerHTML =
      `<div class="spawn-web-dir-modal" role="dialog" aria-modal="true" aria-labelledby="spawn-web-dir-title">` +
      `<div class="spawn-web-dir-header">` +
      `<strong id="spawn-web-dir-title" class="spawn-web-dir-title"></strong>` +
      `<button type="button" class="spawn-web-dir-close" data-action="close" aria-label="Close">✕</button>` +
      `</div>` +
      `<div class="spawn-web-dir-pathrow">` +
      `<button type="button" class="spawn-web-dir-up" data-action="up" title="..">⬆</button>` +
      `<input type="text" class="spawn-web-dir-path" spellcheck="false" autocomplete="off">` +
      `<button type="button" class="spawn-web-dir-go" data-action="go">→</button>` +
      `</div>` +
      `<div class="spawn-web-dir-status" hidden></div>` +
      `<ul class="spawn-web-dir-list" role="listbox"></ul>` +
      `<div class="spawn-web-dir-actions">` +
      `<button type="button" class="spawn-web-dir-cancel" data-action="close"></button>` +
      `<button type="button" class="spawn-web-dir-select" data-action="select"></button>` +
      `</div>` +
      `</div>`;
    document.body.appendChild(overlay);

    const applyLabels = () => {
      const title = overlay.querySelector('.spawn-web-dir-title');
      const cancel = overlay.querySelector('.spawn-web-dir-cancel');
      const select = overlay.querySelector('.spawn-web-dir-select');
      const up = overlay.querySelector('.spawn-web-dir-up') as HTMLButtonElement | null;
      if (title) title.textContent = t('spawn_web_dir_title') || 'Browse folder';
      if (cancel) cancel.textContent = t('spawn_web_dir_cancel') || 'Cancel';
      if (select) select.textContent = t('spawn_web_dir_select') || 'Select this folder';
      if (up) up.title = t('spawn_web_dir_up') || 'Parent folder';
    };
    applyLabels();
    document.addEventListener('i18n-ready', applyLabels);

    const pathInput = overlay.querySelector('.spawn-web-dir-path') as HTMLInputElement;
    const listEl = overlay.querySelector('.spawn-web-dir-list') as HTMLElement;
    const statusEl = overlay.querySelector('.spawn-web-dir-status') as HTMLElement;

    const setStatus = (msg: string, isError = false) => {
      if (!statusEl) return;
      if (!msg) { statusEl.hidden = true; statusEl.textContent = ''; return; }
      statusEl.hidden = false;
      statusEl.textContent = msg;
      statusEl.classList.toggle('is-error', isError);
    };

    const renderList = (parent: string, subdirs: string[]) => {
      listEl.innerHTML = '';
      if (subdirs.length === 0) {
        const empty = document.createElement('li');
        empty.className = 'spawn-web-dir-empty';
        empty.textContent = t('spawn_web_dir_empty') || 'No subfolders';
        listEl.appendChild(empty);
        return;
      }
      for (const name of subdirs) {
        const li = document.createElement('li');
        li.className = 'spawn-web-dir-item';
        li.setAttribute('role', 'option');
        li.tabIndex = 0;
        const full = webDirJoin(parent, name);
        li.dataset.path = full;
        li.innerHTML =
          `<span class="spawn-web-dir-icon" aria-hidden="true">📁</span>` +
          `<span class="spawn-web-dir-name"></span>`;
        const nameEl = li.querySelector('.spawn-web-dir-name');
        if (nameEl) nameEl.textContent = name;
        const enter = () => { void navigateWebDir(full); };
        li.addEventListener('click', enter);
        li.addEventListener('keydown', (e) => {
          if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); enter(); }
        });
        listEl.appendChild(li);
      }
    };

    async function navigateWebDir(path: string): Promise<void> {
      const raw = String(path || '').trim();
      // Keep filesystem roots intact; stripping the trailing separator from
      // `/` or `C:\` would turn them into an empty path or `C:`.
      const target = raw === '/' || /^[A-Za-z]:[\\/]?$/.test(raw)
        ? raw
        : stripTrailingSep(raw);
      if (!target) return;
      setStatus(t('spawn_web_dir_loading') || 'Loading…');
      listEl.innerHTML = '';
      try {
        const res = await fetch(`/api/list-subdirs?token=${token}`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ path: target }),
        });
        const data = await res.json().catch(() => ({} as any));
        if (!res.ok) {
          const detail = data?.detail || data?.error || res.statusText;
          setStatus(String(detail || (t('spawn_web_dir_load_failed') || 'Failed to list folder')), true);
          return;
        }
        if (data.ok === false && (!Array.isArray(data.subdirs) || data.subdirs.length === 0)) {
          setStatus(t('spawn_web_dir_not_dir') || 'Not a directory or inaccessible', true);
          return;
        }
        const resolved = String(data.path || target);
        webDirBrowserPath = resolved;
        pathInput.value = resolved;
        setStatus('');
        renderList(resolved, Array.isArray(data.subdirs) ? data.subdirs : []);
        // Warm the spawn dropdown cache for this parent.
        subdirsCache.set(stripTrailingSep(resolved), Array.isArray(data.subdirs) ? data.subdirs : []);
      } catch (_) {
        setStatus(t('spawn_web_dir_load_failed') || 'Failed to list folder', true);
      }
    }

    const closeBrowser = () => {
      overlay.hidden = true;
      spawnCwdBrowse && (spawnCwdBrowse.disabled = false);
    };

    const selectCurrent = () => {
      if (!webDirBrowserPath) return;
      spawnCwdInput.value = webDirBrowserPath;
      refreshCwdInputStatus();
      closeBrowser();
      spawnCwdInput.focus();
      // Show subfolders under the selected path in the combobox.
      const withSep = webDirBrowserPath.endsWith('/') || webDirBrowserPath.endsWith('\\')
        ? webDirBrowserPath
        : webDirBrowserPath + webDirSep(webDirBrowserPath);
      maybeUpdateSubdirs(withSep);
      renderCwdDropdown(spawnCwdInput.value.trim());
    };

    overlay.addEventListener('click', (e) => {
      if (e.target === overlay) closeBrowser();
    });
    overlay.addEventListener('click', (e) => {
      const btn = (e.target as HTMLElement).closest('[data-action]') as HTMLElement | null;
      if (!btn || !overlay.contains(btn)) return;
      const action = btn.dataset.action;
      if (action === 'close') closeBrowser();
      else if (action === 'select') selectCurrent();
      else if (action === 'up') void navigateWebDir(webDirParent(webDirBrowserPath || pathInput.value));
      else if (action === 'go') void navigateWebDir(pathInput.value.trim());
    });
    pathInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        void navigateWebDir(pathInput.value.trim());
      } else if (e.key === 'Escape') {
        e.preventDefault();
        closeBrowser();
      }
    });
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && !overlay.hidden) {
        e.preventDefault();
        closeBrowser();
      }
    });

    (overlay as any)._navigate = navigateWebDir;
    webDirBrowserEl = overlay;
    return overlay;
  }

  async function openWebDirBrowser(startPath?: string): Promise<void> {
    const overlay = ensureWebDirBrowser();
    overlay.hidden = false;
    let start = String(startPath || spawnCwdInput.value || '').trim();
    if (!start || !/[/\\]/.test(start) && !/^[A-Za-z]:/.test(start)) {
      try {
        const res = await fetch(`/api/info?token=${token}`);
        if (res.ok) {
          const info = await res.json();
          if (info?.cwd) start = String(info.cwd);
        }
      } catch (_) { /* keep start */ }
    }
    if (!start) {
      const favs = loadCwdFavorites();
      const hist = loadCwdHistory();
      start = favs[0] || hist[0] || '/';
    }
    await (overlay as any)._navigate(start);
    const pathInput = overlay.querySelector('.spawn-web-dir-path') as HTMLInputElement | null;
    pathInput?.focus();
    pathInput?.select();
  }

  async function shouldPreferWebDirBrowser(): Promise<boolean> {
    // Remote Hub: native OS dialogs open on the server host, never in the user's browser.
    try {
      const res = await fetch(`/api/info?token=${token}`);
      if (res.ok) {
        const info = await res.json();
        if (info?.env_kind === 'remote') return true;
      }
    } catch (_) { /* fall through */ }
    return false;
  }

  if (spawnCwdBrowse) {
    spawnCwdBrowse.addEventListener('click', async () => {
      spawnCwdBrowse.disabled = true;
      try {
        if (await shouldPreferWebDirBrowser()) {
          await openWebDirBrowser();
          return;
        }
        const res = await fetch(`/api/pick-directory?token=${token}`, { method: 'POST' });
        if (res.ok) {
          const data = await res.json();
          if (data.ok && data.path) {
            spawnCwdInput.value = data.path;
            refreshCwdInputStatus();
            spawnCwdInput.focus();
            renderCwdDropdown('');
            return;
          }
          // User cancelled native dialog (ok:false, no path) — do nothing.
          if (data && data.ok === false && !data.path) return;
        }
        // Native picker unavailable (Linux without zenity/kdialog, etc.) → web browser.
        await openWebDirBrowser();
      } catch (_) {
        try {
          await openWebDirBrowser();
        } catch (_) {
          showToast(t('link_open_error'));
        }
      } finally {
        // openWebDirBrowser keeps the button disabled while modal is open;
        // re-enable only when modal was not shown (native success/cancel).
        if (!webDirBrowserEl || webDirBrowserEl.hidden) {
          spawnCwdBrowse.disabled = false;
        }
      }
    });
  }
  // placeholder ヒント: フォーカス前に一度設定（i18n 初期化後に評価される）。
  (spawnCwdInput as HTMLInputElement).placeholder = t('spawn_cwd_placeholder_hint') || (spawnCwdInput as HTMLInputElement).placeholder;

  spawnCwdInput.addEventListener('focus', () => {
    // お気に入り選択直後の再 focus では再オープンしない（選択して閉じたのに即開き直る事故を防ぐ）。
    if (cwdSuppressReopen) { cwdSuppressReopen = false; return; }
    // ドロップダウンを開く前に全ルートをバックグラウンドで pre-scan してキャッシュを充填する。
    { const favs = loadCwdFavorites(); prescanRoots(deriveRootsFromFavorites(favs)); }
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
      if (cwdDropdown.contains(document.activeElement)) return;  // フォーカスがまだ中にある
      cwdDropdown.hidden = true;
    }, 150);
  });
  spawnCwdInput.addEventListener('keydown', (e) => {
    // Tab 補完: 部分 prefix → <root>: に補完。例: `pub` + Tab → `public:query` 部分を保持。
    if (e.key === 'Tab') {
      const cur = (spawnCwdInput as HTMLInputElement).value;
      const parsed = parseCwdInput(cur);
      if (!parsed.isPath) {
        // 補完候補: parsed.prefix があれば完全一致のルートを探す。
        // なければ parsed.query をプレフィックス前半として部分一致するルートを探す。
        const favs = loadCwdFavorites();
        const roots = deriveRootsFromFavorites(favs);
        const fragment = parsed.prefix !== null ? parsed.prefix : parsed.query;
        const low = fragment.toLowerCase();
        const matches = [...roots.keys()].filter(k => k.toLowerCase().startsWith(low) && k.toLowerCase() !== low);
        if (matches.length === 1) {
          e.preventDefault();
          const query = parsed.prefix !== null ? parsed.query : '';
          (spawnCwdInput as HTMLInputElement).value = matches[0] + ':' + query;
          renderCwdDropdown((spawnCwdInput as HTMLInputElement).value);
        } else if (matches.length === 0 && parsed.prefix !== null) {
          // 入力済み prefix が既存 root と完全一致（大文字小文字問わず）→ 何もしない。
        }
      }
    }
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
    // launcher chip クリック: popover を toggle 表示する（hover でも開くが、タッチ環境向けの保険）。
    const launcherBtn = e.target.closest('.cwd-dropdown-chip-launcher');
    if (launcherBtn) {
      e.preventDefault();
      const collapsed = launcherBtn.closest('.cwd-dropdown-chip-collapsed');
      collapsed?.classList.toggle('is-open');
      return;
    }
    // chip クリック: input の prefix を insert / replace して検索を絞り込む。
    const chipBtn = e.target.closest('.cwd-dropdown-chip');
    if (chipBtn) {
      e.preventDefault();
      const prefix = (chipBtn as HTMLElement).dataset.prefix ?? '';
      const cur = (spawnCwdInput as HTMLInputElement).value;
      const curParsed = parseCwdInput(cur);
      // 入力がフルパス（isPath=true）のときは query にパス全体が入っているため、
      // chip と連結すると `public:C:\...` のような壊れた値になる。空クエリへ戻す。
      const next = prefix + ':' + (curParsed.isPath ? '' : curParsed.query);
      (spawnCwdInput as HTMLInputElement).value = next;
      spawnCwdInput.focus();
      renderCwdDropdown(next);
      return;
    }
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
    // mousedown 即確定（フォーカス維持のため preventDefault）。
    e.preventDefault();
    selectCwdItem(item);
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
      // C1: plan_orchestration-spawn-ui-exposure.md — 「オーケストレーション」ボタン経由の起動
      // だけ orchestration フラグを立てる。役割マッピングは詳細設定を開いて設定した場合のみ添える。
      if (spawnOrchestrationMode) {
        bodyObj.orchestration = true;
        const { roles, count } = collectOrchestrationRoles();
        if (count > 0) bodyObj.orchestration_roles = roles;
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
        const persistedModel = (route === 'ollama' || route === 'lm-studio') ? '' : model;
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
        setSpawnOrchestrationMode(false);

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
