// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { escapeHtml, showToast, token } from './util.js';
import { CWD_HISTORY_MAX, STORAGE_CWD_HISTORY_KEY, STORAGE_CWD_FAVORITES_KEY, STORAGE_SPAWN_KEY, STORAGE_SPAWN_PROVIDER_ORDER_KEY, flushUserPrefsPut, setUserPref } from './user-prefs.js';
import { set_pendingAutoSwitch, sessions } from './state.js';
import { providerIconHtml } from './session-list.js';
import { appConfirm, appConfirmOllamaEncoding } from './settings.js';
import { loadSubscriptions, onSubscriptionsChanged, selectableProfiles } from './subscriptions.js';
import { ORCHESTRATION_CLI_OPTIONS, ORCHESTRATION_ROLE_DEFS } from './orchestration-roles.js';
import { DEFAULT_APPROVAL_FORM_SETTINGS, hasProviderBooleanSetting, isApprovalSettingsMemoryEnabled, mergeApprovalSettings, mergeProviderBooleanSetting, restoreApprovalSettings, restoreProviderBooleanSetting, type ProviderBooleanSettingName } from './spawn-approval-memory.js';
import { compareCwdByBasename, filterCwdSubdirItems, joinCwdChild, splitCwdPath, splitCwdTypeahead } from './cwd-path.js';
import { effortForSpawnBody, effortLevelsFor, setLaunchOptionChoices } from './spawn-confirm-store.js';
import {
  fillModelDatalist,
  getCachedSpawnModelGroups,
  getModelGroupsForProvider,
  groupHasModel,
  isModelCompatibleWithProvider,
  loadSpawnModelGroups,
} from './spawn-model-groups.js';

export { compareCwdByBasename, sortCwdSubdirItems, splitCwdPath } from './cwd-path.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

let resetSpawnProviderOrderImpl: (() => void) | null = null;

/** 新規セッションの provider 並び順を index.html の既定順へ戻す。 */
export function resetSpawnProviderOrder(): void {
  try { localStorage.removeItem(STORAGE_SPAWN_PROVIDER_ORDER_KEY); } catch (_) { /* noop */ }
  resetSpawnProviderOrderImpl?.();
}

// ---- 新規セッション spawn panel ----
(function () {
  // node:test が純関数だけ import するとき document は無い。配線を走らせない。
  if (typeof document === 'undefined') return;
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
  const spawnProviderNoteHelp = document.getElementById('spawn-provider-note-help');
  const spawnRememberApprovalSettings = document.getElementById('spawn-remember-approval-settings') as HTMLInputElement | null;
  const spawnRememberApprovalLabel = document.getElementById('spawn-remember-approval-label');
  const spawnCodexModelBtn = document.getElementById('spawn-codex-model-btn');
  const spawnClaudeModelBtn = document.getElementById('spawn-claude-model-btn');
  const spawnOpenCodeOpts = document.getElementById('spawn-opencode-opts');
  const spawnOpenCodeFullAllow = document.getElementById('spawn-opencode-full-allow') as HTMLInputElement | null;
  const spawnPermissionOpts = document.getElementById('spawn-permission-opts');
  const spawnPermissionMode = document.getElementById('spawn-permission-mode') as HTMLSelectElement | null;
  const spawnIsolateWorktree = document.getElementById('spawn-isolate-worktree') as HTMLInputElement | null;
  const spawnIsolateWorktreeNote = document.getElementById('spawn-isolate-worktree-note');
  const spawnIsolateWorktreeHelp = document.getElementById('spawn-isolate-worktree-help');
  const spawnDelegation = document.getElementById('spawn-delegation') as HTMLInputElement | null;
  // plan_session-handoff-board_c4_prompted-spawn.md C2: 最初に渡す指示（任意）。
  const spawnInitialPrompt = document.getElementById('spawn-initial-prompt') as HTMLTextAreaElement | null;
  const spawnModelInput = document.getElementById('spawn-model');
  const spawnModelDatalist = document.getElementById('spawn-model-datalist');
  const spawnModelClearBtn = document.getElementById('spawn-model-clear');
  const spawnModelRefreshBtn = document.getElementById('spawn-model-refresh');
  const spawnSubscriptionRow = document.getElementById('spawn-subscription-row');
  const spawnSubscriptionSelect = document.getElementById('spawn-subscription') as HTMLSelectElement | null;
  let codexModelSelection: any = null;
  let claudeModelSelection: any = null;

  type SpawnInlineHelpController = {
    setOpen(open: boolean): void;
    setNote(note: HTMLElement | null): void;
  };

  // ? ボタンと、その本文の表示・ツールチップをまとめて扱う。
  // ツールチップも本文と同じ data-i18n キーから作ることで、翻訳を二重に持たない。
  function setupSpawnInlineHelp(
    helpButton: HTMLElement | null,
    initialNote: HTMLElement | null,
  ): SpawnInlineHelpController {
    let note = initialNote;
    let isOpen = false;

    const sync = (): void => {
      if (note) note.hidden = !isOpen;
      if (!helpButton) return;
      helpButton.setAttribute('aria-expanded', isOpen ? 'true' : 'false');
      if (note?.id) helpButton.setAttribute('aria-controls', note.id);
      else helpButton.removeAttribute('aria-controls');
    };

    const updateTooltip = (): void => {
      if (!helpButton) return;
      const key = note?.dataset.i18n || '';
      if (key) helpButton.dataset.tooltip = t(key);
      else delete helpButton.dataset.tooltip;
    };

    const setOpen = (open: boolean): void => {
      isOpen = open;
      sync();
    };

    const setNote = (nextNote: HTMLElement | null): void => {
      if (note && note !== nextNote) note.hidden = true;
      note = nextNote;
      sync();
      updateTooltip();
    };

    helpButton?.addEventListener('click', () => setOpen(!isOpen));
    document.addEventListener('i18n-ready', updateTooltip);
    sync();
    updateTooltip();

    return { setOpen, setNote };
  }

  const spawnProviderNoteIds: Record<string, string> = {
    claude: 'spawn-claude-note',
    codex: 'spawn-codex-note',
    copilot: 'spawn-copilot-note',
    'cursor-agent': 'spawn-cursor-agent-note',
    opencode: 'spawn-opencode-note',
    grok: 'spawn-grok-note',
    'command-code': 'spawn-command-code-note',
    shell: 'spawn-shell-note',
  };
  const spawnProviderInlineHelp = setupSpawnInlineHelp(
    spawnProviderNoteHelp,
    document.getElementById(spawnProviderNoteIds.claude),
  );

  function syncRememberApprovalSettingsLabel(): void {
    const tooltip = t('spawn_remember_approval_tooltip');
    if (spawnRememberApprovalLabel) {
      spawnRememberApprovalLabel.dataset.tooltip = tooltip;
    }
    spawnRememberApprovalSettings?.setAttribute('aria-label', tooltip);
  }

  document.addEventListener('i18n-ready', syncRememberApprovalSettingsLabel);
  syncRememberApprovalSettingsLabel();

  // 前回起動時に選んだサブスクリプションは provider ごとに覚える（profile 一覧は
  // provider 固有なので、1 つの値を使い回すと provider を切り替えた瞬間に無関係な
  // ID が残る）。spawn.defaults は map[string]string なので、入れ子ではなく
  // `subscription_<provider>` のフラットなキーで往復させる。
  const SUBSCRIPTION_PREF_PREFIX = 'subscription_';

  function readSpawnDefaults(): Record<string, unknown> {
    try {
      const s = JSON.parse(localStorage.getItem(STORAGE_SPAWN_KEY) || '{}');
      return (s && typeof s === 'object' && !Array.isArray(s)) ? s : {};
    } catch (_) { return {}; }
  }

  function savedSubscriptionFor(provider: string): string {
    const defaults = readSpawnDefaults();
    // C3 (plan_spawn-form-per-provider-memory.md): 記憶 OFF のときは、過去に書かれた
    // 値が残っていても復元しない（許可設定・worktree・委譲の記憶と同じ扱い）。
    if (!isApprovalSettingsMemoryEnabled(defaults)) return '';
    const v = defaults[SUBSCRIPTION_PREF_PREFIX + provider];
    return typeof v === 'string' ? v : '';
  }

  // 直前に options を組んだ provider。provider が変わったときは画面に残っている
  // 選択値ではなく、その provider の保存値から復元する。
  let subscriptionSelectorProvider: string | null = null;

  // ---- subscription selector（plan_multi-subscription-pool C4）----
  // profile が 2 件以上ある provider でのみ出す。0〜1 件のときは行ごと隠したまま
  // にして、送信 JSON も従来と完全に同じ形（subscription_profile_id 無し）に保つ。
  function refreshSubscriptionSelector(opts?: { restoreSaved?: boolean }): void {
    if (!spawnSubscriptionRow || !spawnSubscriptionSelect) return;
    const provider = (spawnProviderEl as HTMLSelectElement).value;
    const providerChanged = subscriptionSelectorProvider !== provider;
    subscriptionSelectorProvider = provider;
    const profiles = provider === 'shell' ? [] : selectableProfiles(provider);
    if (profiles.length < 2) {
      spawnSubscriptionRow.hidden = true;
      spawnSubscriptionSelect.innerHTML = '';
      return;
    }
    // options がまだ 1 つも無いとき（起動直後、subscriptions の取得完了前に
    // 一度呼ばれている）は「画面上の選択」が存在しないので保存値から復元する。
    const restoreSaved = !!opts?.restoreSaved || providerChanged
      || spawnSubscriptionSelect.options.length === 0;
    const previous = restoreSaved ? savedSubscriptionFor(provider) : spawnSubscriptionSelect.value;
    // 先頭は常に「CLI 自身のログイン環境」。既存利用者が profile を足しただけで
    // 起動先が勝手に変わらないよう、これを既定の選択肢にする。
    const options = [`<option value="">${escapeHtml(t('spawn_subscription_default'))}</option>`];
    // auto は Hub 側の予約語。有効な profile から 1 つ選ぶ（実際に選ばれた ID が
    // セッションに記録される）。
    options.push(`<option value="auto">${escapeHtml(t('spawn_subscription_auto'))}</option>`);
    for (const p of profiles) {
      const label = p.name ? `${p.name} (${p.id})` : p.id;
      options.push(`<option value="${escapeHtml(p.id)}">${escapeHtml(label)}</option>`);
    }
    spawnSubscriptionSelect.innerHTML = options.join('');
    if (previous === 'auto' || (previous && profiles.some((p) => p.id === previous))) {
      spawnSubscriptionSelect.value = previous;
    }
    spawnSubscriptionRow.hidden = false;
  }

  // ---- effort（起動要求の共通 3 項目のうち、画面から選べるのは effort だけ）----
  // 子 plan: docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C5。
  // 写像がある provider（/api/info の effort_levels にキーがある provider）でだけ欄を
  // 出す。隠れている間は送信 JSON に effort キー自体が現れないので、写像が無い
  // provider の起動は従来と 1 バイトも変わらない。
  // 記憶は subscription と同じ `effort_<provider>` のフラットなキーで往復させる。
  const EFFORT_PREF_PREFIX = 'effort_';
  const spawnEffortRow = document.getElementById('spawn-effort-row');
  const spawnEffortSelect = document.getElementById('spawn-effort') as HTMLSelectElement | null;

  function savedEffortFor(provider: string): string {
    const defaults = readSpawnDefaults();
    if (!isApprovalSettingsMemoryEnabled(defaults)) return '';
    const v = defaults[EFFORT_PREF_PREFIX + provider];
    return typeof v === 'string' ? v : '';
  }

  // provider が変わるたびに候補を組み直す。前の provider で選んだ値が新しい provider
  // の候補に無ければ「指定なし」へ戻す（Hub はその値を 400 で弾くため）。
  function syncEffortField(provider: string): void {
    if (!spawnEffortRow || !spawnEffortSelect) return;
    const levels = effortLevelsFor(provider);
    if (levels.length === 0) {
      spawnEffortRow.hidden = true;
      spawnEffortSelect.innerHTML = '';
      return;
    }
    const saved = savedEffortFor(provider);
    const previous = levels.includes(spawnEffortSelect.value) ? spawnEffortSelect.value : saved;
    const options = [`<option value="">${escapeHtml(t('spawn_effort_unset'))}</option>`];
    for (const level of levels) {
      options.push(`<option value="${escapeHtml(level)}">${escapeHtml(level)}</option>`);
    }
    spawnEffortSelect.innerHTML = options.join('');
    spawnEffortSelect.value = levels.includes(previous) ? previous : '';
    spawnEffortRow.hidden = false;
  }

  function selectedEffort(): string {
    if (!spawnEffortRow || spawnEffortRow.hidden || !spawnEffortSelect) return '';
    return spawnEffortSelect.value || '';
  }

  // 選んだ時点で覚える（subscription と同じ理由: 選び直してから起動をやめても
  // 次に開いたときの初期値になる）。記憶 OFF のあいだは保存しない。
  async function persistEffortSelection(): Promise<void> {
    if (!spawnEffortRow || spawnEffortRow.hidden || !spawnEffortSelect) return;
    const defaults = readSpawnDefaults();
    if (!isApprovalSettingsMemoryEnabled(defaults)) return;
    const provider = (spawnProviderEl as HTMLSelectElement).value;
    const next = { ...defaults, [EFFORT_PREF_PREFIX + provider]: spawnEffortSelect.value || '' };
    setUserPref('spawn.defaults', next);
    await flushUserPrefsPut();
  }
  spawnEffortSelect?.addEventListener('change', () => {
    void persistEffortSelection();
  });

  function selectedSubscriptionID(): string {
    if (!spawnSubscriptionRow || spawnSubscriptionRow.hidden || !spawnSubscriptionSelect) return '';
    return spawnSubscriptionSelect.value || '';
  }

  // C3 (plan_spawn-form-per-provider-memory.md): サブスクリプション選択を変更した時点で
  // 保存する。以前は spawn 成功時にしか保存されず、選び直してから起動をやめると忘れていた。
  // 行が隠れている（profile が 0〜1 件、または shell）ときは「選ばなかった」だけなので保存
  // しない（既存の判断・spawn 成功時の分岐と同じ）。記憶 OFF のあいだも保存しない
  // （persistProviderBooleanSetting と同じ理由: OFF のまま起動しても記憶が復活しないため）。
  async function persistSubscriptionSelection(): Promise<void> {
    if (!spawnSubscriptionRow || spawnSubscriptionRow.hidden || !spawnSubscriptionSelect) return;
    const defaults = readSpawnDefaults();
    if (!isApprovalSettingsMemoryEnabled(defaults)) return;
    const provider = spawnProviderEl.value;
    const next = { ...defaults, [SUBSCRIPTION_PREF_PREFIX + provider]: spawnSubscriptionSelect.value || '' };
    setUserPref('spawn.defaults', next);
    await flushUserPrefsPut();
  }
  spawnSubscriptionSelect?.addEventListener('change', () => {
    void persistSubscriptionSelection();
  });

  onSubscriptionsChanged(() => {
    refreshSubscriptionSelector();
    syncAllRoleSubscriptionOptions();
  });
  void loadSubscriptions().then(() => {
    refreshSubscriptionSelector();
    syncAllRoleSubscriptionOptions();
  });

  // ---- C1: オーケストレーション（plan_orchestration-spawn-ui-exposure.md） ----
  // 「オーケストレーション」ボタンから開いたときだけ true。同じ起動フォームを共用し、
  // true のときだけ子役割の詳細設定アコーディオンを見せ、spawn リクエストに
  // orchestration フラグ（+ 設定されていれば役割マッピング）を載せる。
  let spawnOrchestrationMode = false;
  const spawnOrchestrationSection  = document.getElementById('spawn-orchestration-section');
  const spawnOrchestrationBypassNote = document.getElementById('spawn-orchestration-bypass-note');
  const spawnOrchestrationSummary  = document.getElementById('spawn-orchestration-summary');
  const spawnRoleTableBody         = document.getElementById('spawn-role-table-body');

  function syncRoleModelDisabledState(): void {
    spawnRoleTableBody?.querySelectorAll('tr').forEach(tr => {
      const cli = tr.querySelector<HTMLSelectElement>('.spawn-role-cli');
      const model = tr.querySelector<HTMLInputElement>('.spawn-role-model');
      if (!cli || !model) return;
      model.disabled = !cli.value;
      if (!cli.value) model.value = '';
    });
  }

  function syncRoleSubscriptionOptions(tr: Element | null): void {
    if (!tr) return;
    const cli = tr.querySelector<HTMLSelectElement>('.spawn-role-cli');
    const subscription = tr.querySelector<HTMLSelectElement>('.spawn-role-subscription');
    if (!cli || !subscription) return;

    const provider = cli.value;
    const profiles = provider ? selectableProfiles(provider) : [];
    const providerChanged = subscription.dataset.provider !== provider;
    subscription.dataset.provider = provider;
    if (profiles.length < 2) {
      subscription.hidden = true;
      subscription.disabled = true;
      subscription.innerHTML = '';
      return;
    }

    const previous = providerChanged ? '' : subscription.value;
    const options = [`<option value="">${escapeHtml(t('spawn_subscription_default'))}</option>`];
    options.push(`<option value="auto">${escapeHtml(t('spawn_subscription_auto'))}</option>`);
    for (const p of profiles) {
      const label = p.name ? `${p.name} (${p.id})` : p.id;
      options.push(`<option value="${escapeHtml(p.id)}">${escapeHtml(label)}</option>`);
    }
    subscription.innerHTML = options.join('');
    if (previous === 'auto' || (previous && profiles.some((p) => p.id === previous))) {
      subscription.value = previous;
    }
    subscription.hidden = false;
    subscription.disabled = false;
  }

  function syncAllRoleSubscriptionOptions(): void {
    spawnRoleTableBody?.querySelectorAll('tr').forEach(tr => syncRoleSubscriptionOptions(tr));
  }

  // role → {provider, model, subscription} | null のマッピングと、実際に設定された件数を返す。
  function collectOrchestrationRoles(): { roles: Record<string, { provider: string; model: string; subscription: string } | null>; count: number } {
    const roles: Record<string, { provider: string; model: string; subscription: string } | null> = {};
    let count = 0;
    spawnRoleTableBody?.querySelectorAll('tr').forEach(tr => {
      const role = (tr as HTMLElement).dataset.role;
      const cli = tr.querySelector<HTMLSelectElement>('.spawn-role-cli');
      const model = tr.querySelector<HTMLInputElement>('.spawn-role-model');
      const subscriptionSelect = tr.querySelector<HTMLSelectElement>('.spawn-role-subscription');
      if (!role || !cli) return;
      if (!cli.value) { roles[role] = null; return; }
      const subscription = (subscriptionSelect && !subscriptionSelect.hidden) ? subscriptionSelect.value : '';
      roles[role] = { provider: cli.value, model: (model?.value || '').trim(), subscription };
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
        `<option value="${o.value}">${escapeHtml(o.label || t(o.labelKey || ''))}</option>`
      ).join('');
      return (
        `<tr data-role="${r.key}">` +
        `<td>${escapeHtml(t(r.labelKey))}</td>` +
        `<td><select class="spawn-role-cli">${cliOptions}</select></td>` +
        `<td><input type="text" class="spawn-role-model" data-i18n-placeholder="spawn_role_model_placeholder" placeholder="${escapeHtml(t('spawn_role_model_placeholder'))}" disabled></td>` +
        `<td><select class="spawn-role-subscription" hidden disabled></select></td>` +
        `</tr>`
      );
    }).join('');
    spawnRoleTableBody.querySelectorAll('.spawn-role-cli').forEach(sel => {
      sel.addEventListener('change', () => {
        syncRoleModelDisabledState();
        syncRoleSubscriptionOptions(sel.closest('tr'));
        updateOrchestrationSummary();
      });
    });
    spawnRoleTableBody.querySelectorAll('.spawn-role-model').forEach(inp => {
      inp.addEventListener('input', updateOrchestrationSummary);
    });
    syncRoleModelDisabledState();
    syncAllRoleSubscriptionOptions();
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

  // model id → route の即時参照 Map。groups 本体は spawn-model-groups.ts のキャッシュ。
  const spawnModelRouteMap = new Map();
  let spawnModelFetchInFlight = null;
  let spawnProviderOpen = false;
  let spawnProviderActiveIndex = -1;
  let spawnProviderDragSrc: HTMLElement | null = null;
  let suppressSpawnProviderClickUntil = 0;

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
    if (!isModelCompatibleWithProvider(getCachedSpawnModelGroups(), provider, spawnModelInput.value)) {
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
    fillModelDatalist(spawnModelDatalist as HTMLDataListElement | null, getCachedSpawnModelGroups(), spawnProviderEl.value);
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
    for (const g of getModelGroupsForProvider(getCachedSpawnModelGroups(), provider)) {
      if (groupHasModel(g, m)) return g.route || '';
    }
    if (spawnModelRouteMap.has(m) && isModelCompatibleWithProvider(getCachedSpawnModelGroups(), provider, m)) return spawnModelRouteMap.get(m);
    if (m.includes(':cloud')) return 'ollama';
    if (provider === 'claude') return 'anthropic';
    if (provider === 'codex')  return 'openai';
    return '';
  }

  async function fetchModelGroups(force) {
    if (spawnModelFetchInFlight) return spawnModelFetchInFlight;
    const p = (async () => {
      try {
        const groups = await loadSpawnModelGroups(token, force);
        rebuildModelRouteMap(groups);
        populateModelDatalist();
        clearIncompatibleModelForProvider(spawnProviderEl.value);
        clearOllamaModelDefault();
        return { groups };
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

  function loadSpawnProviderOrder(): string[] {
    try {
      const raw = localStorage.getItem(STORAGE_SPAWN_PROVIDER_ORDER_KEY);
      if (!raw) return [];
      const parsed = JSON.parse(raw);
      if (!Array.isArray(parsed)) return [];
      return parsed.filter((value): value is string => typeof value === 'string');
    } catch (_) {
      return [];
    }
  }

  function saveSpawnProviderOrder(values: string[]): void {
    try { localStorage.setItem(STORAGE_SPAWN_PROVIDER_ORDER_KEY, JSON.stringify(values)); } catch (_) { /* private mode 等は無視 */ }
  }

  function persistCurrentSpawnProviderOrder(): void {
    if (!spawnProviderList) return;
    const values = Array.from(spawnProviderList.querySelectorAll<HTMLElement>('.spawn-provider-option'))
      .map((item) => item.dataset.value)
      .filter((value): value is string => !!value);
    saveSpawnProviderOrder(values);
  }

  function clearSpawnProviderDropMarks(): void {
    spawnProviderList?.querySelectorAll<HTMLElement>('.spawn-provider-option').forEach((item) => {
      item.classList.remove('drop-before', 'drop-after');
    });
  }

  function getSpawnProviderOptions() {
    const defaults = Array.from((spawnProviderEl as HTMLSelectElement).options).map((opt) => {
      const label = (opt.textContent || opt.label || opt.value).trim() || opt.value;
      return { value: opt.value, label };
    });
    const known = new Map(defaults.map((option) => [option.value, option]));
    const ordered = [];
    const seen = new Set<string>();
    for (const value of loadSpawnProviderOrder()) {
      const option = known.get(value);
      if (option && !seen.has(value)) {
        ordered.push(option);
        seen.add(value);
      }
    }
    for (const option of defaults) {
      if (!seen.has(option.value)) ordered.push(option);
    }
    return ordered.map((option, index) => ({ ...option, id: `spawn-provider-option-${index}` }));
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
        `role="option" aria-selected="${selected ? 'true' : 'false'}" data-value="${escapeHtml(opt.value)}" draggable="true" tabindex="-1">` +
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

  resetSpawnProviderOrderImpl = () => {
    spawnProviderActiveIndex = getSelectedSpawnProviderIndex();
    updateSpawnProviderIcon();
  };

  // custom_providers:（config.yaml の玄人設定。Hub UI に追加・編集の導線は無い）を
  // spawn の選択肢へ混ぜる。/api/info が返した一覧をネイティブ <select> の <option>
  // として注入するだけで、既存の並び順記憶・アイコン解決（providerIconHtml の
  // フォールバック）・選択ロジックはそのまま使い回す。未設定なら custom_providers
  // が空配列で返るため、この関数は何もしない。
  //
  // injectedCustomProviderIds は注入した id の記憶（plan_custom-provider-spawn-execution.md
  // C2）。custom provider には built-in の provider 別引数・env 注入を一切行わない
  // 決定のうち、model 欄だけはこの Set を見て syncSpawnProviderFields / 送信ボディの
  // 両方で明示的に抑止する必要がある（permission 欄は providerHasPermissionSelect が
  // 既知 provider だけを許可するため、subscription 欄は selectableProfiles が未知 id に
  // 対して自然に空を返すため、どちらも custom provider では追加対応なしで元々隠れる）。
  const injectedCustomProviderIds = new Set<string>();
  function isCustomProviderValue(p: string): boolean {
    return injectedCustomProviderIds.has(p);
  }
  async function injectCustomProviderOptions(): Promise<void> {
    if (!spawnProviderEl) return;
    try {
      const res = await fetch(`/api/info?token=${token}`);
      if (!res.ok) return;
      const info = await res.json();
      // 同じ応答から起動要求の共通 3 項目の候補値も取り込む（追加の fetch をしない）。
      setLaunchOptionChoices(info);
      syncEffortField((spawnProviderEl as HTMLSelectElement).value);
      const list = Array.isArray(info?.custom_providers) ? info.custom_providers : [];
      if (list.length === 0) return;
      const select = spawnProviderEl as HTMLSelectElement;
      const existing = new Set(Array.from(select.options).map((opt) => opt.value));
      let added = false;
      for (const entry of list) {
        const id = typeof entry?.id === 'string' ? entry.id.trim() : '';
        if (!id || existing.has(id)) continue; // built-in や重複と衝突する id は表示しない
        const opt = document.createElement('option');
        opt.value = id;
        opt.textContent = typeof entry?.label === 'string' && entry.label ? entry.label : id;
        select.appendChild(opt);
        existing.add(id);
        injectedCustomProviderIds.add(id);
        added = true;
      }
      if (added) {
        updateSpawnProviderIcon();
        syncSpawnProviderFields(spawnProviderEl.value);
      }
    } catch (_) { /* オフライン等: 追加されないだけで既存の選択肢はそのまま動く */ }
  }
  void injectCustomProviderOptions();

  // 全項目を出し切るのに要る高さ。max-height を外した状態でしか測れないので、
  // 開いた直後に 1 回だけ測って使い回す（毎回測ると、リスト自身をスクロール中に
  // max-height を外した瞬間 scrollTop が飛ぶ）。
  let spawnProviderListContentHeight = 0;

  function measureSpawnProviderListContent(): void {
    if (!spawnProviderList) return;
    spawnProviderList.style.maxHeight = '';
    // scrollHeight は padding を含み border を含まないので上下 border ぶんを足す。
    spawnProviderListContentHeight = spawnProviderList.scrollHeight + 2;
  }

  // ドロップダウンは position:fixed（親 #session-list の overflow に切られないため）。
  // 位置と高さは CSS で決められないので、開くたびにトリガーの画面座標から実測して入れる。
  // 高さを固定値で持つと provider が増えた分だけ黙って見えなくなる（8 件を 180px 固定で
  // 出していて 6 件しか見えていなかった）ので、入るなら全件、入らないなら空いている側へ開く。
  function positionSpawnProviderList(): void {
    if (!spawnProviderList || !spawnProviderTrigger || spawnProviderList.hidden) return;
    const GAP = 2;      // トリガーとの隙間
    const MARGIN = 8;   // 画面端に貼り付かせない余白
    const MIN_HEIGHT = 120; // これ未満しか置けない側は「下に入らない」と見なす
    if (!spawnProviderListContentHeight) measureSpawnProviderListContent();
    const content = spawnProviderListContentHeight;
    const rect = spawnProviderTrigger.getBoundingClientRect();
    const below = window.innerHeight - rect.bottom - GAP - MARGIN;
    const above = rect.top - GAP - MARGIN;
    const openUp = below < Math.min(content, MIN_HEIGHT) && above > below;
    const space = Math.max(0, openUp ? above : below);
    spawnProviderList.style.left = `${Math.round(rect.left)}px`;
    spawnProviderList.style.width = `${Math.round(rect.width)}px`;
    if (openUp) {
      spawnProviderList.style.top = '';
      spawnProviderList.style.bottom = `${Math.round(window.innerHeight - rect.top + GAP)}px`;
    } else {
      spawnProviderList.style.bottom = '';
      spawnProviderList.style.top = `${Math.round(rect.bottom + GAP)}px`;
    }
    spawnProviderList.style.maxHeight = `${Math.round(Math.min(content, space))}px`;
  }

  // 開いている間だけ追従させる（サイドバーのスクロール・ウィンドウリサイズで
  // トリガーが動くと、fixed のリストは置き去りになる）。scroll は capture で拾う。
  const repositionSpawnProviderList = (e?: Event) => {
    if (!spawnProviderOpen) return;
    // リスト自身のスクロールでは動かさない（位置は変わらない）。
    if (e && e.target === spawnProviderList) return;
    positionSpawnProviderList();
  };

  function openSpawnProviderList() {
    if (!spawnProviderList || !spawnProviderTrigger || !spawnProviderCombobox) return;
    spawnProviderOpen = true;
    document.body.classList.add('spawn-provider-list-open');
    spawnProviderActiveIndex = getSelectedSpawnProviderIndex();
    spawnProviderList.hidden = false;
    spawnProviderTrigger.setAttribute('aria-expanded', 'true');
    spawnProviderTrigger.classList.add('is-open');
    spawnProviderCombobox.classList.add('is-open');
    renderSpawnProviderOptions();
    measureSpawnProviderListContent();
    positionSpawnProviderList();
    window.addEventListener('resize', repositionSpawnProviderList);
    window.addEventListener('scroll', repositionSpawnProviderList, true);
  }

  function closeSpawnProviderList(focusTrigger = false) {
    if (!spawnProviderList || !spawnProviderTrigger || !spawnProviderCombobox) return;
    spawnProviderOpen = false;
    document.body.classList.remove('spawn-provider-list-open');
    window.removeEventListener('resize', repositionSpawnProviderList);
    window.removeEventListener('scroll', repositionSpawnProviderList, true);
    spawnProviderList.hidden = true;
    spawnProviderList.style.top = '';
    spawnProviderList.style.bottom = '';
    spawnProviderList.style.left = '';
    spawnProviderList.style.width = '';
    spawnProviderList.style.maxHeight = '';
    spawnProviderListContentHeight = 0;
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
      if (Date.now() < suppressSpawnProviderClickUntil) {
        suppressSpawnProviderClickUntil = 0;
        e.preventDefault();
        return;
      }
      const item = (e.target as Element | null)?.closest<HTMLElement>('.spawn-provider-option');
      if (!item) return;
      selectSpawnProviderValue(item.dataset.value);
    });
    spawnProviderList.addEventListener('mousemove', (e) => {
      if (spawnProviderDragSrc) return;
      const item = (e.target as Element | null)?.closest<HTMLElement>('.spawn-provider-option');
      if (!item || !spawnProviderList.contains(item)) return;
      const items = [...spawnProviderList.querySelectorAll('.spawn-provider-option')];
      const idx = items.indexOf(item);
      if (idx >= 0 && idx !== spawnProviderActiveIndex) setSpawnProviderActiveIndex(idx);
    });
    spawnProviderList.addEventListener('keydown', handleSpawnProviderKeydown);

    spawnProviderList.addEventListener('dragstart', (e: DragEvent) => {
      const item = (e.target as Element | null)?.closest<HTMLElement>('.spawn-provider-option');
      if (!item || !spawnProviderList.contains(item)) return;
      spawnProviderDragSrc = item;
      item.classList.add('dragging');
      if (e.dataTransfer) {
        e.dataTransfer.effectAllowed = 'move';
        // Firefox はデータを載せないと dragstart 自体が成立しない。
        try { e.dataTransfer.setData('text/plain', item.dataset.value || ''); } catch (_) { /* noop */ }
      }
    });

    spawnProviderList.addEventListener('dragend', () => {
      const hadDrag = !!spawnProviderDragSrc;
      if (spawnProviderDragSrc) spawnProviderDragSrc.classList.remove('dragging');
      spawnProviderDragSrc = null;
      clearSpawnProviderDropMarks();
      if (hadDrag) suppressSpawnProviderClickUntil = Date.now() + 250;
    });

    // リストは縦並びなので、項目の上下半分で挿入位置を決める。
    spawnProviderList.addEventListener('dragover', (e: DragEvent) => {
      if (!spawnProviderDragSrc) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
      const item = (e.target as Element | null)?.closest<HTMLElement>('.spawn-provider-option');
      clearSpawnProviderDropMarks();
      if (!item || item === spawnProviderDragSrc || !spawnProviderList.contains(item)) return;
      const rect = item.getBoundingClientRect();
      const after = e.clientY >= rect.top + rect.height / 2;
      item.classList.add(after ? 'drop-after' : 'drop-before');
    });

    spawnProviderList.addEventListener('dragleave', (e: DragEvent) => {
      const item = (e.target as Element | null)?.closest<HTMLElement>('.spawn-provider-option');
      if (item) item.classList.remove('drop-before', 'drop-after');
    });

    spawnProviderList.addEventListener('drop', (e: DragEvent) => {
      if (!spawnProviderDragSrc) return;
      e.preventDefault();
      const source = spawnProviderDragSrc;
      const item = (e.target as Element | null)?.closest<HTMLElement>('.spawn-provider-option');
      clearSpawnProviderDropMarks();
      if (item && item !== source && spawnProviderList.contains(item)) {
        const rect = item.getBoundingClientRect();
        const after = e.clientY >= rect.top + rect.height / 2;
        spawnProviderList.insertBefore(source, after ? item.nextSibling : item);
      } else if (!item) {
        const items = [...spawnProviderList.querySelectorAll<HTMLElement>('.spawn-provider-option')];
        const last = items[items.length - 1];
        if (last && last !== source) spawnProviderList.insertBefore(source, last.nextSibling);
      }
      persistCurrentSpawnProviderOrder();
      renderSpawnProviderOptions();
    });
  }

  document.addEventListener('mousedown', (e) => {
    if (!spawnProviderOpen || !spawnProviderCombobox) return;
    if (!spawnProviderCombobox.contains(e.target)) closeSpawnProviderList(false);
  });

  function providerHasPermissionSelect(p: string): boolean {
    return p === 'claude' || p === 'grok' || p === 'copilot' || p === 'cursor-agent' || p === 'command-code';
  }

  function permissionAutoLabel(p: string): string {
    if (p === 'copilot') return t('spawn_permission_auto_copilot');
    if (p === 'cursor-agent') return t('spawn_permission_auto_cursor');
    return t('spawn_permission_auto');
  }

  function permissionBypassLabel(p: string): string {
    if (p === 'grok') return t('spawn_permission_bypass_grok');
    if (p === 'copilot') return t('spawn_permission_bypass_copilot');
    if (p === 'cursor-agent') return t('spawn_permission_bypass_cursor');
    if (p === 'command-code') return t('spawn_permission_bypass_command_code');
    return t('spawn_permission_bypass');
  }

  function syncPermissionModeOptions(p: string): void {
    if (!spawnPermissionMode) return;
    const claudeLike = p === 'claude' || p === 'grok';
    // Command Code は plan / acceptEdits に対応するが auto に相当する mode を持たない
    // （親 plan の対応表: plan → --permission-mode plan、acceptEdits → --auto-accept、
    // bypassPermissions → --yolo）。claudeLike の条件は書き換えず、専用の分岐を足すだけにする。
    const commandCode = p === 'command-code';
    for (const opt of Array.from(spawnPermissionMode.options)) {
      if (opt.value === 'plan' || opt.value === 'acceptEdits') {
        opt.hidden = !(claudeLike || commandCode);
        opt.disabled = !(claudeLike || commandCode);
      }
      if (opt.value === 'auto') {
        opt.hidden = commandCode;
        opt.disabled = commandCode;
        opt.textContent = permissionAutoLabel(p);
      }
      if (opt.value === 'bypassPermissions') opt.textContent = permissionBypassLabel(p);
    }
    const selected = spawnPermissionMode.selectedOptions[0];
    if (selected && (selected.hidden || selected.disabled)) {
      spawnPermissionMode.value = 'default';
    }
  }

  function syncSpawnProviderFields(p: string): void {
    const isShell = (p === 'shell');
    const modelRow = document.querySelector<HTMLElement>('.spawn-model-row');
    // custom provider には built-in の model 引数を一切渡さない決定
    // （plan_custom-provider-spawn-execution.md 決定事項2）。欄ごと隠す。
    if (modelRow) modelRow.hidden = isShell || isCustomProviderValue(p);
    const claudeOpts = document.getElementById('spawn-claude-opts');
    if (claudeOpts) claudeOpts.hidden = (p !== 'claude');
    const codexOpts = document.getElementById('spawn-codex-opts');
    if (codexOpts) codexOpts.hidden = (p !== 'codex');
    if (spawnOpenCodeOpts) spawnOpenCodeOpts.hidden = (p !== 'opencode');
    if (spawnPermissionOpts) spawnPermissionOpts.hidden = !providerHasPermissionSelect(p);
    // 注意書きは既定で全部畳む。選択中の 1 本を出すかどうかは ? の開閉状態が決めるので、
    // 可視性の判断は setNote 側の 1 箇所に持たせる（ここで「選択中は表示」と書くと、
    // 直後の setNote が上書きするだけの死んだ分岐になる）。
    const selectedNote = document.getElementById(spawnProviderNoteIds[p] || '');
    for (const id of Object.values(spawnProviderNoteIds)) {
      const note = document.getElementById(id);
      if (note) note.hidden = true;
    }
    spawnProviderInlineHelp.setNote(selectedNote);
    syncPermissionModeOptions(p);
    syncEffortField(p);
  }

  function applySpawnApprovalSettings(provider: string, defaults: Record<string, unknown>): void {
    const approval = {
      ...DEFAULT_APPROVAL_FORM_SETTINGS,
      ...restoreApprovalSettings(provider, defaults),
    };
    if (spawnPermissionMode) {
      spawnPermissionMode.value = approval.permission_mode;
      syncPermissionModeOptions(provider);
    }
    const sandbox = document.getElementById('spawn-sandbox') as HTMLSelectElement | null;
    if (sandbox) sandbox.value = approval.sandbox;
    const askForApproval = document.getElementById('spawn-ask-approval') as HTMLSelectElement | null;
    if (askForApproval) askForApproval.value = approval.ask_for_approval;
    if (spawnOpenCodeFullAllow) {
      spawnOpenCodeFullAllow.checked = approval.opencode_permission_mode === 'bypassPermissions';
    }
  }

  spawnProviderEl.addEventListener('change', () => {
    updateSpawnProviderIcon();
    const p = spawnProviderEl.value;
    syncSpawnProviderFields(p);
    if (p !== 'codex')  codexModelSelection  = null;
    if (p !== 'claude') claudeModelSelection = null;
    populateModelDatalist();
    clearIncompatibleModelForProvider(p);
    // C2 (plan_spawn-form-per-provider-memory.md): 許可設定・worktree・委譲・
    // サブスクリプションの記憶を、切り替えた先の provider のぶんへ差し替える。
    restoreProviderMemory(p);
    updateDetachedPreview();
  });
  updateSpawnProviderIcon();
  document.addEventListener('i18n-ready', () => {
    syncPermissionModeOptions(spawnProviderEl.value);
  });

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

  // worktree 隔離の説明は、チェックが入ったときは従来どおり自動で開く（未追跡ファイル
  // が引き継がれない点を起動前に見せるため）。加えて ? ボタンでいつでも開閉できる。
  // 自動表示だけだと、チェックを入れるまで「これが何なのか」を読む手段が無かった。
  const isolateWorktreeInlineHelp = setupSpawnInlineHelp(spawnIsolateWorktreeHelp, spawnIsolateWorktreeNote);

  // チェック状態に説明の開閉を合わせる（change 時と設定復元時に使う）。
  function syncIsolateWorktreeNote(): void {
    isolateWorktreeInlineHelp.setOpen(!!spawnIsolateWorktree?.checked);
  }

  // C2 (plan_spawn-form-per-provider-memory.md): worktree / 委譲を変更した時点で、
  // 許可設定の queueApprovalSettingsPersistence と同じ形で即座に保存する。
  // C3: 記憶 OFF のあいだは保存もしない（OFF のまま起動しても記憶が復活しないため）。
  async function persistProviderBooleanSetting(name: ProviderBooleanSettingName, checked: boolean): Promise<void> {
    const defaults = readSpawnDefaults();
    if (!isApprovalSettingsMemoryEnabled(defaults)) return;
    const provider = spawnProviderEl.value;
    const values = { [name]: checked } as Partial<Record<ProviderBooleanSettingName, boolean>>;
    const next = mergeProviderBooleanSetting(defaults, provider, values);
    setUserPref('spawn.defaults', next);
    await flushUserPrefsPut();
  }

  if (spawnIsolateWorktree) {
    spawnIsolateWorktree.addEventListener('change', () => {
      syncIsolateWorktreeNote();
      void persistProviderBooleanSetting('isolate_worktree', spawnIsolateWorktree.checked);
    });
  }
  if (spawnDelegation) {
    spawnDelegation.addEventListener('change', () => {
      void persistProviderBooleanSetting('delegation', spawnDelegation.checked);
    });
  }

  // C2: 許可設定・worktree・委譲・サブスクリプションの復元をこの 1 本の入口へ通す。
  // パネルを開いたとき（loadSpawnSettings）と provider を切り替えたとき（change ハンドラ）の
  // 両方から呼ぶ。どちらか一方にしか配線しないと、同じ症状（前の provider の値が画面に
  // 残る）が別の欄で再発する。
  function restoreProviderMemory(provider: string): void {
    const defaults = readSpawnDefaults();
    applySpawnApprovalSettings(provider, defaults);
    if (spawnIsolateWorktree) {
      spawnIsolateWorktree.checked = restoreProviderBooleanSetting('isolate_worktree', provider, defaults);
      syncIsolateWorktreeNote();
    }
    if (spawnDelegation) {
      spawnDelegation.checked = restoreProviderBooleanSetting('delegation', provider, defaults);
    }
    // パネルを開き直したときと同じ「前回起動したときの選択」に戻す（開きっぱなしで
    // いじった一時的な選択ではなく保存値が正）。
    refreshSubscriptionSelector({ restoreSaved: true });
  }

  function currentApprovalSettings(provider: string): Record<string, string> {
    if (providerHasPermissionSelect(provider)) {
      return {
        permission_mode: spawnPermissionMode?.value || DEFAULT_APPROVAL_FORM_SETTINGS.permission_mode,
      };
    }
    if (provider === 'codex') {
      const sandbox = document.getElementById('spawn-sandbox') as HTMLSelectElement | null;
      const askForApproval = document.getElementById('spawn-ask-approval') as HTMLSelectElement | null;
      return {
        sandbox: sandbox?.value || DEFAULT_APPROVAL_FORM_SETTINGS.sandbox,
        ask_for_approval: askForApproval?.value || DEFAULT_APPROVAL_FORM_SETTINGS.ask_for_approval,
      };
    }
    if (provider === 'opencode') {
      return {
        opencode_permission_mode: spawnOpenCodeFullAllow?.checked ? 'bypassPermissions' : 'default',
      };
    }
    return {};
  }

  async function persistApprovalSettings(): Promise<void> {
    if (!spawnRememberApprovalSettings) return;
    const defaults = readSpawnDefaults();
    const provider = spawnProviderEl.value;
    const next = mergeApprovalSettings(
      defaults,
      provider,
      currentApprovalSettings(provider),
      spawnRememberApprovalSettings.checked,
    );
    setUserPref('spawn.defaults', next);
    await flushUserPrefsPut();
  }

  const queueApprovalSettingsPersistence = (): void => {
    void persistApprovalSettings();
  };

  spawnRememberApprovalSettings?.addEventListener('change', () => {
    queueApprovalSettingsPersistence();
  });
  spawnPermissionMode?.addEventListener('change', queueApprovalSettingsPersistence);
  document.getElementById('spawn-sandbox')?.addEventListener('change', queueApprovalSettingsPersistence);
  document.getElementById('spawn-ask-approval')?.addEventListener('change', queueApprovalSettingsPersistence);
  spawnOpenCodeFullAllow?.addEventListener('change', queueApprovalSettingsPersistence);

  function loadSpawnSettings() {
    if (spawnRememberApprovalSettings) spawnRememberApprovalSettings.checked = true;
    applySpawnApprovalSettings(spawnProviderEl.value, {});
    try {
      const parsed = JSON.parse(localStorage.getItem(STORAGE_SPAWN_KEY) || '{}');
      const s: Record<string, any> = parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : {};
      if (typeof s.provider === 'string' && s.provider) {
        spawnProviderEl.value = s.provider;
        syncSpawnProviderFields(s.provider);
      }
      if (s.cwd)              spawnCwdInput.value = s.cwd;
      // モデル欄は保存値を prefill しない（空 = 各 CLI の既定モデルを尊重）。
      // 以前は前回モデルを復元して明示送信していたが、それが claude CLI の
      // /model 既定（1M 窓など）を 200K へ上書きする原因だった。非既定モデルを
      // 使いたいときは datalist から明示選択する（その選択はそのセッションにのみ適用）。
      // s.model は後方互換のため保存自体は残すが、ここでは読み込まない。
      if (spawnRememberApprovalSettings) {
        spawnRememberApprovalSettings.checked = isApprovalSettingsMemoryEnabled(s);
      }
      // C2: 許可設定・worktree・委譲・サブスクリプションの復元は 1 本の入口に通す
      // （restoreProviderMemory は provider の change ハンドラからも呼ばれる）。
      restoreProviderMemory(spawnProviderEl.value);
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
      syncSpawnProviderFields(spawnProviderEl.value);
      updateDetachedPreview();
      return !!s.cwd;
    } catch (_) { return false; }
  }

  async function saveSpawnSettings(obj: Record<string, string>): Promise<void> {
    setUserPref('spawn.defaults', obj);
    await flushUserPrefsPut();
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

  function clearCwdHistory() {
    setUserPref('cwd_history', []);
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

  function clearCwdFavorites() {
    setUserPref('cwd_favorites', []);
  }

  // 行の × 用: 「その行を消す」導線なので履歴とお気に入りの両方から削除する。
  // 履歴だけ消すと、お気に入り行では行が残り続けて「押しても何も起きない」ボタンになる。
  function deleteCwdEntry(cwd) {
    if (!cwd) return;
    deleteCwdHistoryItem(cwd);
    const favs = loadCwdFavorites();
    if (favs.includes(cwd)) setUserPref('cwd_favorites', favs.filter(v => v !== cwd));
  }

  let cwdSuppressReopen = false; // お気に入り選択で入力欄を再 focus する際の自動再オープンを1回抑止する

  // 今ドロップダウンに適用されているフィルタ文字列。renderCwdDropdown が毎回更新する。
  // × / ★ / 一括削除の後は「入力欄の値」ではなくこの値で描き直す。入力欄に確定済みのパスが
  // 残ったまま focus/click で開いた（＝フィルタ '' の全件表示）状態を入力値で描き直すと、
  // 全件表示だったリストが 0〜1 件へ絞られて「押した瞬間に閉じた」ように見えるため。
  let cwdActiveFilter = '';

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
  // baseOnlyFilter は「フォルダ名にだけ効くフィルタ」。サブフォルダ行は打ちかけセグメントに
  // 一致した部分だけをフォルダ名の中で光らせたく、フルパスの filter を渡すと親側まで光るため。
  function buildCwdLabelHtml(value, filter, baseOnlyFilter = '') {
    const { parent, basename } = splitCwdPath(value);
    const parentHtml = parent
      ? `<span class="cwd-dropdown-path-parent">${highlightCwdSegment(parent, filter)}</span>`
      : '';
    const baseHtml = `<span class="cwd-dropdown-path-base">${highlightCwdSegment(basename, baseOnlyFilter || filter)}</span>`;
    return parentHtml + baseHtml;
  }

  // 入力値に区切りが含まれるとき、最後の区切りより前（＝親）の直下サブフォルダ一覧を保持する。
  // input 値が変わるたびに更新され、renderCwdDropdown が先頭セクションとして描画する。
  // 打ちかけセグメント（区切りより後ろ）はここに持たない。理由は maybeUpdateSubdirs のコメント参照。
  const subdirsCache = new Map<string, string[]>();
  let subdirsCurrent: { parent: string; sep: string; items: string[] } | null = null;

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

  // 親の解決と取得だけを担当する。打ちかけセグメント（`...\public\o` の `o`）は
  // ここで捨て、renderCwdDropdown が filter から毎回導出する。状態として持つと、
  // 絞り込み済みの入力欄へフォーカスし直したとき（focus/click ハンドラは
  // renderCwdDropdown('') を呼ぶ）に全件へ戻れず 0 件へ落ちるため。
  function maybeUpdateSubdirs(value: string): void {
    const ta = value ? splitCwdTypeahead(value) : null;
    if (!ta) {
      subdirsCurrent = null;
      return;
    }
    const { parent, sep } = ta;
    if (!parent) { subdirsCurrent = null; return; }
    if (subdirsCurrent && subdirsCurrent.parent === parent) return;
    subdirsCurrent = { parent, sep, items: subdirsCache.get(parent) ?? [] };
    fetchSubdirs(parent).then(items => {
      if (!subdirsCurrent || subdirsCurrent.parent !== parent) return;
      subdirsCurrent.items = items;
      rerenderCwdDropdown();
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

  // 表示中のフィルタを維持したまま描き直す。行の追加/削除でリストの見え方を変えたくない
  // 操作（× 削除・★ トグル・一括削除・subdirs の遅延到着）から使う。
  function rerenderCwdDropdown() {
    renderCwdDropdown(cwdActiveFilter);
  }

  function renderCwdDropdown(filter) {
    cwdActiveFilter = filter ?? '';
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

    // subdirs 展開（入力値に区切りが含まれるとき）。
    // ドライブ直下 / POSIX ルートでは parent が区切りで終わる（`C:\` `/`）ので、素朴に
    // parent + sep + name と書くと `C:\\name` になる。連結は joinCwdChild に任せる
    // （区切りが混在した入力では parent の末尾と sep が一致しないため、sep との比較では足りない）。
    const subCur = subdirsCurrent;
    // 打ちかけセグメントは状態に持たず filter から導出する。
    const typeahead = splitCwdTypeahead(filter ?? '');
    // filter が '' の描き直し（focus / click / 行の追加削除の後）では、入力欄が区切りで
    // 終わっているときだけサブフォルダ節を出す。確定したパスが入っているだけの状態で
    // 兄弟フォルダを並べると、パネルを開いた既定の見え方（お気に入り・履歴）が変わり、
    // しかも下の重複除去でお気に入り行がサブフォルダ節へ移動してしまうため。
    const showSubdirs = !!subCur && (
      typeahead
        ? typeahead.parent === subCur.parent
        : /[\\/]$/.test(String((spawnCwdInput as HTMLInputElement).value).trim())
    );
    const subItems: string[] = showSubdirs && subCur
      ? subCur.items.map(name => joinCwdChild(subCur.parent, subCur.sep, name))
      : [];
    const subPartial = (typeahead && subCur && typeahead.parent === subCur.parent)
      ? typeahead.partial
      : '';
    const sortedSubItems = filterCwdSubdirItems(subItems, favSet, subPartial);

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

    // サブフォルダ節に出したパスは fav/hist 節から除く（同じ行が 2 度出ないように）。
    // 検索結果節が searchPathSet でやっているのと同じ手法。
    const subPathSet = new Set(sortedSubItems);

    // isPath モードのときは検索結果を出さないので fav/hist の除外も不要にリセット。
    const effectiveFavItems = (isPathMode
      ? (filter ? favs.filter(v => v.toLowerCase().includes(filter.toLowerCase())) : favs.slice())
          .sort(compareCwdByBasename)
      : favItems).filter(v => !subPathSet.has(v));
    const effectiveHistItems = (isPathMode
      ? (filter ? hist.filter(v => !favSet.has(v) && v.toLowerCase().includes(filter.toLowerCase())) : hist.filter(v => !favSet.has(v)))
      : histItems).filter(v => !subPathSet.has(v));

    // 絞り込みで落ちた行は描画されないので、存在確認も絞り込み後の集合に対して行う。
    const allItems = [...sortedSubItems, ...(isPathMode ? [] : searchItems.map(r => r.path)), ...effectiveFavItems, ...effectiveHistItems];
    const hasAny = allItems.length > 0 || hasRoots;
    if (!hasAny) { cwdDropdown.hidden = true; return; }

    function renderRow(v: string, fav: boolean, isSub = false) {
      const labelFilter = isSub ? '' : filter;
      // サブフォルダ行は親側を光らせず、打ちかけセグメントに一致した部分だけをフォルダ名で光らせる。
      const baseOnlyFilter = isSub ? subPartial : '';
      return (
        `<li class="cwd-dropdown-item${fav ? ' is-favorite' : ''}${isSub ? ' is-subdir' : ''}" tabindex="-1" data-value="${escapeHtml(v)}">` +
        `<button class="cwd-dropdown-fav${fav ? ' is-on' : ''}" tabindex="-1" data-value="${escapeHtml(v)}" ` +
        `title="${escapeHtml(t(fav ? 'spawn_cwd_unfavorite' : 'spawn_cwd_favorite'))}">${fav ? '★' : '☆'}</button>` +
        `<span class="cwd-dropdown-label" title="${escapeHtml(v)}">${buildCwdLabelHtml(v, labelFilter, baseOnlyFilter)}</span>` +
        (isSub ? '' : `<button class="cwd-dropdown-del" tabindex="-1" data-value="${escapeHtml(v)}" ` +
          `title="${escapeHtml(t('spawn_cwd_remove_entry'))}">✕</button>`) +
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
        `<button class="cwd-dropdown-del" tabindex="-1" data-value="${escapeHtml(r.path)}" ` +
        `title="${escapeHtml(t('spawn_cwd_remove_entry'))}">✕</button>` +
        `</li>`
      );
    }

    // 0 件判定は絞り込み後の件数で行う（打ちかけで全部落ちたときに「該当なし」を出すため）。
    const noMatch = !isPathMode && searchItems.length === 0 && effectiveFavItems.length === 0 && effectiveHistItems.length === 0 && sortedSubItems.length === 0;

    let html = '';

    // chip 行（roots がある場合、または履歴/お気に入りの一括削除ボタンを出す場合）。
    // launcher chip 1 個に圧縮し hover/click で popover を開く。
    const showChipsRow = hasRoots || hist.length > 0 || favs.length > 0;
    if (showChipsRow) {
      const chipsClass = noMatch ? 'cwd-dropdown-chips has-no-match' : 'cwd-dropdown-chips';
      let chipsHtml = `<li class="${chipsClass}">`;
      if (hasRoots) {
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
        chipsHtml += `<div class="cwd-dropdown-chip-collapsed">` +
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
      }
      // 履歴/お気に入りの一括削除ボタン（chip 行の右寄せ）。
      if (hist.length > 0 || favs.length > 0) {
        chipsHtml += `<div class="cwd-dropdown-chip-actions">`;
        if (hist.length > 0) {
          chipsHtml += `<button class="cwd-dropdown-clear-history" type="button" title="${escapeHtml(t('spawn_cwd_clear_history'))}">${escapeHtml(t('spawn_cwd_clear_history'))}</button>`;
        }
        if (favs.length > 0) {
          chipsHtml += `<button class="cwd-dropdown-clear-favorites" type="button" title="${escapeHtml(t('spawn_cwd_clear_favorites'))}">${escapeHtml(t('spawn_cwd_clear_favorites'))}</button>`;
        }
        chipsHtml += `</div>`;
      }
      if (noMatch) {
        chipsHtml += `</li>` +
          `<li class="cwd-dropdown-no-match" aria-hidden="true">${escapeHtml(t('spawn_cwd_no_match_hint'))}</li>`;
      } else {
        chipsHtml += `</li>`;
      }
      html += chipsHtml;
    } else if (noMatch) {
      // roots・履歴・お気に入りが全て空でも 0 件案内は出す。
      html += `<li class="cwd-dropdown-no-match" aria-hidden="true">${escapeHtml(t('spawn_cwd_no_match_hint'))}</li>`;
    }

    // subdirs セクション（入力値に区切りが含まれるとき）。
    if (sortedSubItems.length > 0) {
      html += `<li class="cwd-dropdown-header" aria-hidden="true">${escapeHtml(t('spawn_cwd_section_subdirs'))}</li>`;
      html += sortedSubItems.map(v => renderRow(v, favSet.has(v), true)).join('');
    } else if (subPartial && subCur && subCur.items.length > 0) {
      // 一覧は届いているのに打ちかけセグメントで全部落ちた、と分かるときだけ出す。
      // 取得前（items が空）は「該当なし」ではなく「まだ来ていない」なので出さない。
      html += `<li class="cwd-dropdown-no-match" aria-hidden="true">${escapeHtml(t('spawn_cwd_no_subdir_match'))}</li>`;
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
        // missing 用の強調ツールチップから通常の削除ツールチップへ戻す（消してしまうと
        // 一度 missing 判定を通った行だけ説明が失われる）。
        if (delBtn) delBtn.title = t('spawn_cwd_remove_entry');
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
    if (!getCachedSpawnModelGroups()) {
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
    if (!getCachedSpawnModelGroups()) {
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
    overlay.className = 'spawn-web-dir-overlay aac-wheel-overlay';
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
      for (const name of [...subdirs].sort(compareCwdByBasename)) {
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
      rerenderCwdDropdown();
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
    const clearHistBtn = e.target.closest('.cwd-dropdown-clear-history');
    if (clearHistBtn) {
      e.preventDefault();
      if (window.confirm(t('spawn_cwd_clear_history_confirm'))) {
        clearCwdHistory();
        rerenderCwdDropdown();
      }
      focusInputNoReopen();
      return;
    }
    const clearFavBtn = e.target.closest('.cwd-dropdown-clear-favorites');
    if (clearFavBtn) {
      e.preventDefault();
      if (window.confirm(t('spawn_cwd_clear_favorites_confirm'))) {
        clearCwdFavorites();
        rerenderCwdDropdown();
      }
      focusInputNoReopen();
      return;
    }
    const favBtn = e.target.closest('.cwd-dropdown-fav');
    if (favBtn) {
      e.preventDefault();
      toggleCwdFavorite(favBtn.dataset.value);
      rerenderCwdDropdown();
      focusInputNoReopen();
      return;
    }
    const delBtn = e.target.closest('.cwd-dropdown-del');
    if (delBtn) {
      e.preventDefault();
      deleteCwdEntry(delBtn.dataset.value);
      rerenderCwdDropdown();
      // focus() 経由の focus リスナ（renderCwdDropdown('') で全件へ描き直す）を踏むと
      // せっかく維持したフィルタが飛ぶので、再オープン抑止つきで戻す。
      focusInputNoReopen();
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

  // spawn 失敗レスポンスを人が読める 1 行にする。Hub は {"ok":false,"error":…,"detail":…}
  // を返すので、生の JSON をそのまま alert へ流すと何が起きたのか伝わらない。
  async function spawnErrorMessage(res: Response): Promise<string> {
    const raw = await res.text();
    try {
      const err = JSON.parse(raw);
      if (err?.error === 'worktree_error') return t('spawn_worktree_error');
      if (err?.detail) return String(err.detail);
    } catch (_) {}
    return raw;
  }

  async function spawnSession() {
    const provider = document.getElementById('spawn-provider').value;
    const cwd = spawnCwdInput.value.trim();
    spawnLaunchBtn.disabled = true;
    try {
      const model = spawnModelInput.value.trim();
      const label = document.getElementById('spawn-label').value.trim();
      if (model && !isModelCompatibleWithProvider(getCachedSpawnModelGroups(), provider, model)) {
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

      // Shell と custom provider は model / route / permission 系フィールドを送らない
      // （custom は plan_custom-provider-spawn-execution.md 決定事項2）。
      const skipsAIFields = provider === 'shell' || isCustomProviderValue(provider);
      const bodyObj: any = { provider, cwd, label };
      if (!skipsAIFields) bodyObj.model = model;
      if (utf8Session) bodyObj.utf8_session = true;
      // C2 (plan_spawn-form-per-provider-memory.md): この provider の記憶がある場合だけ
      // 明示的に true/false を送る。記憶が無い provider ではキーごと省略し、config の
      // user_prefs.spawn.worktree_auto / delegation_auto を Hub 側の既定として温存する
      // （internal/hub/spawn_handler.go は *bool の nil を「設定に従う」と解釈するため、
      // 常時 false を送ると設定ファイルで常時 ON にしている利用者の既定が壊れる）。
      const providerMemoryDefaults = readSpawnDefaults();
      if (hasProviderBooleanSetting('isolate_worktree', provider, providerMemoryDefaults)) {
        bodyObj.isolate_worktree = !!spawnIsolateWorktree?.checked;
      }
      if (hasProviderBooleanSetting('delegation', provider, providerMemoryDefaults)) {
        bodyObj.delegation = !!spawnDelegation?.checked;
      }
      // plan_session-handoff-board_c4_prompted-spawn.md C2: 空欄なら initial_prompt キー自体を
      // 送らない（Hub 側は空文字と省略を同じ「何も注入しない」として扱うが、送信 JSON の形も
      // 従来と同一に保つ）。
      const initialPrompt = spawnInitialPrompt?.value.trim() || '';
      if (initialPrompt) bodyObj.initial_prompt = initialPrompt;
      if (!skipsAIFields && route) bodyObj.route = route;
      // 「Default CLI login」を選んだときはキーごと送らない（従来リクエストと同一）。
      const subscriptionID = selectedSubscriptionID();
      if (subscriptionID) bodyObj.subscription_profile_id = subscriptionID;
      // 欄が隠れている（写像が無い provider）か「指定なし」ならキーごと送らない。
      const effortLevel = skipsAIFields ? '' : effortForSpawnBody(provider, selectedEffort());
      if (effortLevel) bodyObj.effort = effortLevel;
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
      } else if (provider === 'opencode') {
        const fullAllow = !!spawnOpenCodeFullAllow?.checked;
        let riskConfirmed = false;

        if (fullAllow) {
          riskConfirmed = await appConfirm({
            title: t('opencode_confirm_title'),
            message: t('opencode_confirm_message'),
            confirmText: t('opencode_confirm_run'),
            cancelText: t('spawn_cancel'),
            kind: 'danger',
          });
          if (!riskConfirmed) {
            spawnLaunchBtn.disabled = false;
            return;
          }
        }

        bodyObj.permission_mode = fullAllow ? 'bypassPermissions' : 'default';
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
      } else if (provider === 'grok' || provider === 'copilot' || provider === 'cursor-agent' || provider === 'command-code') {
        const permMode = spawnPermissionMode ? spawnPermissionMode.value : 'default';
        if (permMode === 'bypassPermissions') {
          const riskConfirmed = await appConfirm({
            title: t('full_allow_confirm_title'),
            message: t('full_allow_confirm_message'),
            confirmText: t('full_allow_confirm_run'),
            cancelText: t('spawn_cancel'),
            kind: 'danger',
          });
          if (!riskConfirmed) {
            spawnLaunchBtn.disabled = false;
            return;
          }
          bodyObj.risk_confirmed = true;
        }
        bodyObj.permission_mode = permMode;
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
        // C2 (plan_spawn-form-per-provider-memory.md): 既存の記憶（readSpawnDefaults()）を
        // 土台にして今回の値だけ上書きする。まっさらなオブジェクトを作ると、ここに列挙して
        // いない provider 別キー（他 provider の許可設定・worktree・委譲・サブスクリプション）
        // が保存のたびに全部消えてしまう。
        const baseDefaults: Record<string, unknown> = {
          ...readSpawnDefaults(),
          provider,
          cwd,
          model: persistedModel,
          open_target: openTarget,
          grid_layout: gridLayout,
          detached_preset: detachedPreset,
        };
        // サブスクリプション選択は provider ごとに記憶する。行が隠れている
        // （profile が 0〜1 件 / shell）ときは「選ばなかった」だけなので、既存の
        // 記憶を空で上書きしない。
        if (spawnSubscriptionRow && !spawnSubscriptionRow.hidden) {
          baseDefaults[SUBSCRIPTION_PREF_PREFIX + provider] = subscriptionID;
        }
        const withProviderBooleans = mergeProviderBooleanSetting(baseDefaults, provider, {
          isolate_worktree: !!spawnIsolateWorktree?.checked,
          delegation: !!spawnDelegation?.checked,
        });
        const rememberApprovalSettings = spawnRememberApprovalSettings?.checked ?? true;
        const persistedDefaults = mergeApprovalSettings(
          withProviderBooleans,
          provider,
          { ...bodyObj, opencode_permission_mode: bodyObj.permission_mode },
          rememberApprovalSettings,
        );
        await saveSpawnSettings(persistedDefaults);
        document.getElementById('spawn-label').value = '';
        // 一度届けたら消す。label と同じく「次回また送られる」を避ける一回性の欄。
        if (spawnInitialPrompt) spawnInitialPrompt.value = '';
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
        alert(t('spawn_failed') + await spawnErrorMessage(res));
      }
    } catch (e) {
      alert(t('spawn_failed') + e.message);
    } finally {
      spawnLaunchBtn.disabled = false;
    }
  }
})();
