// 派生起動ダイアログ（種別 = 引き継ぎ / 子）。
//
// 既存セッションを起点に新しいセッションを 1 本立てる操作を 1 つの入口にまとめる
// （子 plan: docs/local/plan_derived-session-launch_c3_derive-launch.md 内部 C2、
// 親 plan D1 / D5）。開閉・Escape・背景クリックの作法は handoff.ts の
// buildDialogShell と同じで、欄の構成は spawn 確認ダイアログ（spawn-confirm.ts）に
// 揃えてある。
//
// 送信内容そのものは derive-dialog-store.ts の純関数が決める。このファイルは DOM の
// 組み立てとイベント配線だけを持ち、「どちらの種別に何が載るか」の判断は一切しない
// （そこは web/tests/derive-dialog-store.test.ts が固定している）。
//
// 候補値は既存の取得口をそのまま使う: /api/info（effort / 実行モード / 権限段 /
// custom provider / 段ごとの実効権限）、subscriptions.ts の selectableProfiles、
// orchestration-roles.ts の一覧。ここで独自に組み直さない。
import { t } from '../i18n.js';
import { apiFetch, escapeHtml, showToast, token } from './util.js';
import { sessions } from './state.js';
import { activateSession } from './session-list.js';
import { ORCHESTRATION_CLI_OPTIONS, ORCHESTRATION_ROLE_DEFS } from './orchestration-roles.js';
import { loadSubscriptions, selectableProfiles } from './subscriptions.js';
import { appConfirm } from './settings.js';
import { approvalDisplayHtml, headlessUnsupportedNoticeHtml, permissionPresetLabel } from './spawn-confirm.js';
import {
  EXECUTION_MODE_SCHEMA,
  PERMISSION_PRESET_SCHEMA,
  childPermissionPreviewFor,
  effortLevelsFor,
  headlessUnsupportedForSelection,
  isExecutionModeAvailable,
  isPermissionPresetAvailable,
  rememberedRolePermission,
  setLaunchOptionChoices,
} from './spawn-confirm-store.js';
import {
  availableDeriveKinds,
  buildDeriveBody,
  deriveRequestPath,
  deriveSubmitBlockedReason,
  type DeriveKind,
  type DeriveSelection,
} from './derive-dialog-store.js';
import {
  fillModelDatalist,
  getCachedSpawnModelGroups,
  isModelCompatibleWithProvider,
  loadSpawnModelGroups,
} from './spawn-model-groups.js';

function tx(key: string, fallback: string, vars: Record<string, unknown> = {}): string {
  let value = t(key, vars);
  if (value === key) {
    value = fallback;
    Object.entries(vars).forEach(([name, replacement]) => {
      value = value.replaceAll(`{${name}}`, String(replacement));
    });
  }
  return value;
}

interface DerivePreviewResponse {
  ok?: boolean;
  exists?: boolean;
  live?: boolean;
  provider?: string;
  cwd?: string;
  markdown?: string;
  candidate_providers?: string[];
  /** 前任の会話ログの絶対パス（Hub 側で存在を確かめたものだけが載る）。 */
  transcript_path?: string;
  /** 前任が書いた引き継ぎメモの絶対パス（同上）。 */
  note_path?: string;
}

interface DeriveProviderOption {
  value: string;
  label: string;
}

// 起点の素性。看板（/api/handoff/<id>）と、生きていれば session 表の両方から埋める。
// Hub を再起動した後は session 表に居ないので、看板だけで開けることが要る。
interface DeriveSource {
  sessionID: number;
  provider: string;
  cwd: string;
  live: boolean;
  markdown: string;
  candidateProviders: string[];
  /**
   * 後継が自分のツールで読む 2 つのパス（子 plan:
   * docs/local/plan_derived-session-launch_c4_handoff-routes.md 内部 C1・C2）。
   * 中身は Hub を通らない。**渡す前に人が見る**ので、文面欄の md に載るのとは別に、
   * ダイアログにも 1 行ずつ出す（親 plan 不変条件 3）。
   */
  transcriptPath: string;
  notePath: string;
}

function providerLabel(value: string, extras: DeriveProviderOption[]): string {
  const custom = extras.find((o) => o.value === value);
  if (custom) return custom.label;
  const found = ORCHESTRATION_CLI_OPTIONS.find((o) => o.value === value);
  if (!found) return value;
  return found.label || tx(found.labelKey || '', found.labelKey || value);
}

// /api/info は候補値の正本。派生ダイアログは spawn パネルと同じ応答を読み、同じ
// 取り込み関数（setLaunchOptionChoices）へ通す。custom provider だけは子の CLI 候補に
// 必要なのでここで拾う。
async function loadLaunchOptions(): Promise<DeriveProviderOption[]> {
  try {
    const res = await fetch(`/api/info?token=${token}`);
    if (!res.ok) return [];
    const info = await res.json();
    setLaunchOptionChoices(info);
    const list = Array.isArray(info?.custom_providers) ? info.custom_providers : [];
    return list
      .map((entry: any) => ({ value: String(entry?.id || entry?.value || ''), label: String(entry?.label || entry?.name || entry?.id || '') }))
      .filter((o: DeriveProviderOption) => o.value);
  } catch (_) {
    return [];
  }
}

async function loadSource(sessionID: number): Promise<DeriveSource> {
  const live = sessions.get(sessionID) as any;
  const source: DeriveSource = {
    sessionID,
    provider: String(live?.provider || ''),
    cwd: String(live?.cwd || ''),
    live: !!live,
    markdown: '',
    candidateProviders: [],
    transcriptPath: '',
    notePath: '',
  };
  try {
    const res = await apiFetch(`/api/handoff/${encodeURIComponent(String(sessionID))}`);
    const data = await res.json() as DerivePreviewResponse;
    if (res.ok && data?.ok && data.exists) {
      source.provider = String(data.provider || source.provider);
      source.cwd = String(data.cwd || source.cwd);
      source.markdown = String(data.markdown || '');
      source.candidateProviders = Array.isArray(data.candidate_providers) ? data.candidate_providers : [];
      source.transcriptPath = String(data.transcript_path || '');
      source.notePath = String(data.note_path || '');
      // 看板側の live は Hub の session 表を見た結果なので、こちらを優先する。
      if (typeof data.live === 'boolean') source.live = data.live;
    }
  } catch (_) {
    // 看板が無くても子は立てられる。引き継ぎ側は文面が空のまま起動できない
    // （deriveSubmitBlockedReason が 'prompt' を返す）ので、黙って壊れはしない。
  }
  return source;
}

function optionsHtml(values: readonly { value: string; label: string; disabled?: boolean }[], selected: string): string {
  return values.map((o) => {
    const sel = o.value === selected ? ' selected' : '';
    const dis = o.disabled ? ' disabled' : '';
    return `<option value="${escapeHtml(o.value)}"${sel}${dis}>${escapeHtml(o.label)}</option>`;
  }).join('');
}

// 起動できた新しいセッションへ画面を切り替える。spawn-child の応答は子が register
// し終えてから返るが、UI 側の一覧（sessions）は WS の配信で埋まるので、応答の直後は
// まだ無いことがある。一覧に現れるまで短く待ち、現れなければ諦める（一覧には出るので
// 人が押せる）。/api/spawn（引き継ぎ種別）は session_id を返さないのでここは通らない。
const FOCUS_NEW_SESSION_RETRY_MS = 150;
const FOCUS_NEW_SESSION_MAX_TRIES = 10;
function focusNewSession(id: number, attempt = 0): void {
  if (!id) return;
  if (sessions.has(id)) {
    activateSession(id);
    return;
  }
  if (attempt >= FOCUS_NEW_SESSION_MAX_TRIES) return;
  window.setTimeout(() => focusNewSession(id, attempt + 1), FOCUS_NEW_SESSION_RETRY_MS);
}

// 「指定なし」。空文字は Hub 側で「この項目を送らなかった」と同じ扱いになる
// （derive-dialog-store.ts がキーごと落とす）。
function unsetOption(): { value: string; label: string } {
  return { value: '', label: tx('spawn_confirm_option_unset', 'Not specified') };
}

export async function openDeriveDialog(sessionID: number, opts: { kind?: DeriveKind } = {}): Promise<void> {
  const backdrop = document.createElement('div');
  backdrop.className = 'derive-dialog-backdrop aac-wheel-overlay';
  backdrop.innerHTML = `<div class="derive-dialog" role="dialog" aria-modal="true" aria-labelledby="derive-dialog-title">
    <h2 id="derive-dialog-title">${escapeHtml(tx('derive_dialog_title', '派生セッションを起動'))}</h2>
    <div class="derive-dialog-body"><p>${escapeHtml(tx('handoff_dialog_loading', '読み込み中…'))}</p></div>
    <p class="derive-dialog-error" data-derive-error hidden></p>
    <div class="derive-dialog-actions"><button type="button" data-derive-cancel>${escapeHtml(tx('handoff_dialog_cancel', 'キャンセル'))}</button></div>
  </div>`;
  document.body.appendChild(backdrop);

  const onKeyDown = (event: KeyboardEvent) => { if (event.key === 'Escape') close(); };
  const close = (): void => {
    document.removeEventListener('keydown', onKeyDown);
    backdrop.remove();
  };
  document.addEventListener('keydown', onKeyDown);
  backdrop.addEventListener('click', (event) => { if (event.target === backdrop) close(); });
  backdrop.querySelector('[data-derive-cancel]')?.addEventListener('click', close);

  const [customProviders, source] = await Promise.all([loadLaunchOptions(), loadSource(sessionID)]);
  await loadSubscriptions();
  if (!backdrop.isConnected) return; // 読み込み中に閉じられた

  renderDeriveForm(backdrop, source, customProviders, opts.kind, close);
}

function renderDeriveForm(
  backdrop: HTMLElement,
  source: DeriveSource,
  customProviders: DeriveProviderOption[],
  requestedKind: DeriveKind | undefined,
  close: () => void,
): void {
  const bodyEl = backdrop.querySelector('.derive-dialog-body') as HTMLElement | null;
  const actionsEl = backdrop.querySelector('.derive-dialog-actions') as HTMLElement | null;
  const errorEl = backdrop.querySelector('[data-derive-error]') as HTMLElement | null;
  if (!bodyEl || !actionsEl) return;

  const kinds = availableDeriveKinds(source.live);
  let kind: DeriveKind = requestedKind && kinds.includes(requestedKind) ? requestedKind : kinds[0];

  const kindOptions = [
    { value: 'handoff', label: tx('derive_kind_handoff', '引き継ぎ（この仕事を続ける後継）') },
    { value: 'child', label: tx('derive_kind_child', '子（部分作業をして親へ返す）'), disabled: !kinds.includes('child') },
  ];
  const roleOptions = ORCHESTRATION_ROLE_DEFS.map((def) => ({ value: def.key, label: `${tx(def.labelKey, def.key)}（${def.key}）` }));
  const executionModeOptions = [unsetOption(), ...EXECUTION_MODE_SCHEMA.map((v) => ({ value: String(v), label: String(v), disabled: !isExecutionModeAvailable(v) }))];
  const permissionOptions = [unsetOption(), ...PERMISSION_PRESET_SCHEMA.map((v) => ({ value: String(v), label: permissionPresetLabel(v), disabled: !isPermissionPresetAvailable(v) }))];

  // 渡すものを人が見てから押す（親 plan 不変条件 3）。文面欄の md にも同じパスが
  // 載るが、md は長いので、何が渡るのかはここで 1 行ずつ読めるようにする。
  const sourcePathHtml = (label: string, path: string): string => (path
    ? `<p class="derive-dialog-source" data-derive-source-path>${escapeHtml(label)}: ${escapeHtml(path)}</p>`
    : '');

  bodyEl.innerHTML = `
    <p class="derive-dialog-source">${escapeHtml(tx('handoff_dialog_source', '元セッション'))}: #${escapeHtml(String(source.sessionID))} ${escapeHtml(providerLabel(source.provider, customProviders))} — ${escapeHtml(source.cwd)}</p>
    ${sourcePathHtml(tx('derive_note_path', '引き継ぎメモ'), source.notePath)}
    ${sourcePathHtml(tx('derive_transcript_path', '前任の会話ログ'), source.transcriptPath)}
    <div class="derive-dialog-fields">
      <label class="derive-field">
        <span class="derive-field-label">${escapeHtml(tx('derive_kind_label', '種別'))}</span>
        <select name="kind" class="derive-select">${optionsHtml(kindOptions, kind)}</select>
      </label>
      <label class="derive-field" data-derive-role-field>
        <span class="derive-field-label">${escapeHtml(tx('spawn_role_table_role', '役割'))}</span>
        <select name="role" class="derive-select">${optionsHtml(roleOptions, roleOptions[0].value)}</select>
      </label>
      <label class="derive-field">
        <span class="derive-field-label">${escapeHtml(tx('spawn_role_table_cli', 'CLI'))}</span>
        <select name="provider" class="derive-select" data-derive-provider></select>
      </label>
      <label class="derive-field" data-derive-subscription-field>
        <span class="derive-field-label">${escapeHtml(tx('spawn_subscription_default', 'Subscription'))}</span>
        <select name="subscription" class="derive-select" data-derive-subscription></select>
      </label>
      <label class="derive-field" data-derive-model-field>
        <span class="derive-field-label">${escapeHtml(tx('spawn_role_table_model', 'Model'))}</span>
        <input name="model" class="derive-input" list="derive-model-datalist" spellcheck="false" autocomplete="off" placeholder="${escapeHtml(tx('spawn_confirm_model_placeholder', ''))}">
        <datalist id="derive-model-datalist"></datalist>
      </label>
      <label class="derive-field" data-derive-effort-field hidden>
        <span class="derive-field-label">${escapeHtml(tx('spawn_confirm_effort', 'Reasoning effort'))}</span>
        <select name="effort" class="derive-select" data-derive-effort></select>
      </label>
      <label class="derive-field">
        <span class="derive-field-label">${escapeHtml(tx('spawn_confirm_execution_mode', 'Execution mode'))}</span>
        <select name="execution_mode" class="derive-select">${optionsHtml(executionModeOptions, '')}</select>
      </label>
      <label class="derive-field">
        <span class="derive-field-label">${escapeHtml(tx('spawn_confirm_permission_preset', 'Permission tier'))}</span>
        <select name="permission_preset" class="derive-select">${optionsHtml(permissionOptions, '')}</select>
      </label>
      <label class="derive-remember-permission" data-derive-remember-field hidden>
        <input type="checkbox" name="remember_permission">
        <span>${escapeHtml(tx('spawn_confirm_remember_permission', 'Use this tier for this role next time'))}</span>
      </label>
    </div>
    <dl class="derive-dialog-meta">
      <dt>${escapeHtml(tx('spawn_confirm_approval', '渡る権限'))}</dt>
      <dd class="derive-dialog-approval" data-derive-approval></dd>
      <dt data-derive-cwd-term>${escapeHtml(tx('derive_cwd_label', '作業ディレクトリ'))}</dt>
      <dd class="derive-dialog-path" data-derive-cwd>${escapeHtml(source.cwd)}</dd>
    </dl>
    <label class="derive-same-tree" data-derive-same-tree-field hidden>
      <input type="checkbox" name="same_tree">
      <span>${escapeHtml(tx('derive_same_tree_label', '親と同じツリーで動かす（worktree を作らない）'))}</span>
    </label>
    <label class="derive-dialog-prompt-label" for="derive-dialog-prompt" data-derive-prompt-label></label>
    <textarea id="derive-dialog-prompt" class="derive-dialog-prompt" rows="12" spellcheck="false"></textarea>`;

  const kindSelect = bodyEl.querySelector('[name=kind]') as HTMLSelectElement;
  const roleField = bodyEl.querySelector('[data-derive-role-field]') as HTMLElement;
  const roleSelect = bodyEl.querySelector('[name=role]') as HTMLSelectElement;
  const providerSelect = bodyEl.querySelector('[data-derive-provider]') as HTMLSelectElement;
  const subscriptionField = bodyEl.querySelector('[data-derive-subscription-field]') as HTMLElement;
  const subscriptionSelect = bodyEl.querySelector('[data-derive-subscription]') as HTMLSelectElement;
  const modelField = bodyEl.querySelector('[data-derive-model-field]') as HTMLElement;
  const modelInput = bodyEl.querySelector('[name=model]') as HTMLInputElement;
  const modelDatalist = bodyEl.querySelector('#derive-model-datalist') as HTMLDataListElement;
  const effortField = bodyEl.querySelector('[data-derive-effort-field]') as HTMLElement;
  const effortSelect = bodyEl.querySelector('[data-derive-effort]') as HTMLSelectElement;
  const executionSelect = bodyEl.querySelector('[name=execution_mode]') as HTMLSelectElement;
  const presetSelect = bodyEl.querySelector('[name=permission_preset]') as HTMLSelectElement;
  const rememberField = bodyEl.querySelector('[data-derive-remember-field]') as HTMLElement;
  const rememberInput = bodyEl.querySelector('[name=remember_permission]') as HTMLInputElement;
  const sameTreeField = bodyEl.querySelector('[data-derive-same-tree-field]') as HTMLElement;
  const sameTreeInput = bodyEl.querySelector('[name=same_tree]') as HTMLInputElement;
  const approvalEl = bodyEl.querySelector('[data-derive-approval]') as HTMLElement;
  const cwdTerm = bodyEl.querySelector('[data-derive-cwd-term]') as HTMLElement;
  const cwdEl = bodyEl.querySelector('[data-derive-cwd]') as HTMLElement;
  const sourcePathEls = Array.from(bodyEl.querySelectorAll('[data-derive-source-path]')) as HTMLElement[];
  const promptLabel = bodyEl.querySelector('[data-derive-prompt-label]') as HTMLElement;
  const promptArea = bodyEl.querySelector('#derive-dialog-prompt') as HTMLTextAreaElement;

  let promptEdited = false;
  promptArea.addEventListener('input', () => { promptEdited = true; setError(''); refreshStartEnabled(); });

  // 段を人が触ったら、以後は役割の記憶で上書きしない（役割を選び直しても、その人が
  // 選んだ段が残る）。触るまでは記憶が初期値を決める。
  let presetEdited = false;

  // 引き継ぎ先の候補は看板が返した一覧だけ（元と別の provider。親 D7）。
  // 子は親と同じ provider も選べ、custom provider も対象になる。
  function providerChoices(): DeriveProviderOption[] {
    if (kind === 'handoff') {
      return source.candidateProviders.map((value) => ({ value, label: providerLabel(value, customProviders) }));
    }
    const builtin = ORCHESTRATION_CLI_OPTIONS.filter((o) => o.value)
      .map((o) => ({ value: o.value, label: o.label || tx(o.labelKey || '', o.labelKey || o.value) }));
    return [...builtin, ...customProviders];
  }

  function rebuildProviderSelect(): void {
    const previous = providerSelect.value;
    const choices = providerChoices();
    const keep = choices.some((o) => o.value === previous) ? previous : '';
    providerSelect.innerHTML = optionsHtml(
      [{ value: '', label: tx('handoff_dialog_provider_empty', 'CLI を選択…') }, ...choices],
      keep,
    );
  }

  function rebuildSubscriptionSelect(): void {
    const profiles = selectableProfiles(providerSelect.value);
    const previous = subscriptionSelect.value;
    const keep = profiles.some((p) => p.id === previous) ? previous : '';
    subscriptionSelect.innerHTML = optionsHtml(
      [
        { value: '', label: tx('spawn_subscription_default', 'Default') },
        ...profiles.map((p) => ({ value: p.id, label: p.name ? `${p.name} (${p.id})` : p.id })),
      ],
      keep,
    );
    // profile が 1 つしか無い provider では選ぶ意味が無い（relay ダイアログと同じ規則）。
    subscriptionField.hidden = profiles.length < 2;
  }

  function rebuildEffortSelect(): void {
    const levels = effortLevelsFor(providerSelect.value);
    const previous = effortSelect.value;
    const keep = levels.includes(previous) ? previous : '';
    effortSelect.innerHTML = optionsHtml(
      [{ value: '', label: tx('spawn_confirm_option_unset', 'Not specified') }, ...levels.map((v) => ({ value: v, label: v }))],
      keep,
    );
    // 写像が無い provider では欄ごと出さない（候補の無い select を見せない）。
    effortField.hidden = levels.length === 0;
  }

  function isCustomProvider(): boolean {
    return customProviders.some((o) => o.value === providerSelect.value);
  }

  async function refreshModelChoices(): Promise<void> {
    const provider = providerSelect.value;
    if (isCustomProvider()) {
      modelField.hidden = true;
      modelInput.value = '';
      fillModelDatalist(modelDatalist, [], provider);
      refreshStartEnabled();
      return;
    }
    modelField.hidden = false;
    try {
      await loadSpawnModelGroups(token);
    } catch (_) {
      // 候補が空でも手入力で起動できる。新規セッション画面と同じ。
    }
    const groups = getCachedSpawnModelGroups();
    fillModelDatalist(modelDatalist, groups, provider);
    if (!isModelCompatibleWithProvider(groups, provider, modelInput.value)) {
      modelInput.value = '';
    }
    refreshStartEnabled();
  }

  function refreshApprovalDisplay(): void {
    // headless を選んだのに定義が無い、は権限表が無くても言える（言わないと押して
    // から 400 で知ることになる）。権限欄と同じ場所へ 1 行で出す（子 plan 内部 C6）。
    const headlessUnsupported = headlessUnsupportedForSelection(providerSelect.value, executionSelect.value);
    const approval = childPermissionPreviewFor(providerSelect.value, presetSelect.value);
    if (!approval) {
      approvalEl.innerHTML = headlessUnsupported ? headlessUnsupportedNoticeHtml() : '';
      return;
    }
    // 引き継ぎは /api/spawn 経路で、段の表から高リスク確認を自動で通すことはしない
    // （spawn_handler.go: 「RiskConfirmed はここでは触らない」）。実際には下の
    // appConfirm が出るので、「確認済みで起動」とは見せない。
    approvalEl.innerHTML = approvalDisplayHtml(
      kind === 'handoff' ? { ...approval, riskConfirmed: false } : approval,
      headlessUnsupported,
    );
  }

  // 役割の記憶（/api/info の role_permission）を段の select へ入れる。
  //
  // **Hub は UI 起点の要求をこの記憶で埋め直さない**ので、ここで入れた段がそのまま
  // 送られる＝画面に見えている段で起動する（親 plan 不変条件 5）。記憶が無い役割へ
  // 切り替えたときは「指定なし」へ戻す: 前の役割の段が残ると、覚えてもいない段で
  // 起動することになる。人が段を触った後（presetEdited）は何もしない。
  function applyRememberedPermission(): void {
    if (kind !== 'child' || presetEdited) return;
    const tier = rememberedRolePermission(roleSelect.value);
    presetSelect.value = tier;
    rememberInput.checked = !!tier;
  }

  function defaultPrompt(): string {
    return kind === 'handoff' ? source.markdown : roleSelect.value;
  }

  function syncPrompt(): void {
    if (promptEdited) return;
    promptArea.value = defaultPrompt();
  }

  function currentSelection(riskConfirmed = false): DeriveSelection {
    return {
      kind,
      sourceSessionID: source.sessionID,
      provider: providerSelect.value,
      model: modelInput.value,
      role: roleSelect.value,
      effort: effortField.hidden ? '' : effortSelect.value,
      executionMode: executionSelect.value,
      permissionPreset: presetSelect.value,
      subscriptionProfileID: subscriptionField.hidden ? '' : subscriptionSelect.value,
      // 引き継ぎには役割が無いので、記憶の指示そのものを載せない（undefined）。
      rememberPermission: kind === 'child' ? rememberInput.checked : undefined,
      sameTree: kind === 'child' && sameTreeInput.checked,
      cwd: source.cwd,
      prompt: promptArea.value,
      riskConfirmed,
    };
  }

  function setError(message: string): void {
    if (!errorEl) return;
    errorEl.textContent = message;
    errorEl.hidden = !message;
  }

  const startBtn = document.createElement('button');
  const cancelBtn = document.createElement('button');

  function refreshStartEnabled(): void {
    const reason = deriveSubmitBlockedReason(currentSelection(), getCachedSpawnModelGroups());
    startBtn.disabled = submitting || reason !== '';
    startBtn.title = reason === 'provider' ? tx('derive_error_provider', 'CLI を選んでください')
      : reason === 'role' ? tx('derive_error_role', '役割を選んでください')
        : reason === 'prompt' ? tx('derive_error_prompt', '渡す文面を入れてください')
          : reason === 'model' ? tx('spawn_model_provider_mismatch', 'This model is not available for the selected provider.') : '';
  }

  function applyKind(): void {
    roleField.hidden = kind !== 'child';
    // 記憶は役割ごとなので、役割を持たない引き継ぎでは欄ごと出さない。
    rememberField.hidden = kind !== 'child';
    sameTreeField.hidden = kind !== 'child';
    cwdTerm.hidden = kind !== 'handoff';
    cwdEl.hidden = kind !== 'handoff';
    // 前任のメモ・会話ログは引き継ぎで渡すもの。子は親の続きではないので出さない。
    sourcePathEls.forEach((el) => { el.hidden = kind !== 'handoff'; });
    promptLabel.textContent = kind === 'handoff'
      ? tx('handoff_dialog_prompt_label', '初期プロンプト（編集可）')
      : tx('derive_prompt_label_child', '最初の指示（編集可・空でも可）');
    rebuildProviderSelect();
    rebuildSubscriptionSelect();
    rebuildEffortSelect();
    void refreshModelChoices();
    applyRememberedPermission();
    refreshApprovalDisplay();
    syncPrompt();
    // 看板が無い（記録される前に終わった・整理済み）セッションは文面が空で開く。
    // 自分で書けば起動できるので止めはしないが、なぜ空なのかは伝える。
    setError(kind === 'handoff' && !source.markdown && !promptEdited
      ? tx('handoff_dialog_no_record', 'この記録は見つかりません')
      : '');
    refreshStartEnabled();
  }

  kindSelect.addEventListener('change', () => {
    const picked = kindSelect.value as DeriveKind;
    kind = kinds.includes(picked) ? picked : kind;
    kindSelect.value = kind;
    setError('');
    applyKind();
  });
  roleSelect.addEventListener('change', () => {
    applyRememberedPermission();
    refreshApprovalDisplay();
    syncPrompt();
    refreshStartEnabled();
  });
  providerSelect.addEventListener('change', () => {
    setError('');
    rebuildSubscriptionSelect();
    rebuildEffortSelect();
    void refreshModelChoices();
    refreshApprovalDisplay();
    refreshStartEnabled();
  });
  presetSelect.addEventListener('change', () => { presetEdited = true; refreshApprovalDisplay(); });
  executionSelect.addEventListener('change', refreshApprovalDisplay);
  subscriptionSelect.addEventListener('change', refreshStartEnabled);
  modelInput.addEventListener('input', refreshStartEnabled);

  let submitting = false;

  // /api/spawn は permission_preset を埋めた結果が高リスクなら 400
  // risk_confirmation_required を返す。New Session フォームと同じく、人に 1 度
  // 聞いてから risk_confirmed を付けて送り直す（近道で自動的に通さない）。
  async function submit(riskConfirmed: boolean): Promise<void> {
    const selection = currentSelection(riskConfirmed);
    const response = await apiFetch(deriveRequestPath(selection), {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(buildDeriveBody(selection, getCachedSpawnModelGroups())),
    });
    const data = await response.json().catch(() => ({}));
    if (response.ok) {
      close();
      showToast(tx('handoff_dialog_started', '新しいセッションを起動しました'));
      // 子種別だけ応答に session_id が載る（spawn-child）。引き継ぎ種別の /api/spawn は
      // ok と orchestration_id しか返さないので、そちらは一覧に出るのを待つだけ。
      focusNewSession(Number(data?.session_id || 0));
      return;
    }
    if (!riskConfirmed && selection.kind === 'handoff' && String(data?.error || '') === 'risk_confirmation_required') {
      const agreed = await appConfirm({
        title: tx('derive_risk_confirm_title', '強い権限で起動します'),
        message: tx('derive_risk_confirm_message', '選んだ権限の段では、この CLI は承認を求めずに動きます。起動しますか?'),
        confirmText: tx('derive_risk_confirm_run', '起動する'),
        cancelText: tx('spawn_cancel', 'キャンセル'),
        kind: 'danger',
      });
      if (!agreed) return;
      await submit(true);
      return;
    }
    setError(String(data?.detail || data?.error || `HTTP ${response.status}`));
  }

  cancelBtn.type = 'button';
  cancelBtn.textContent = tx('handoff_dialog_cancel', 'キャンセル');
  cancelBtn.addEventListener('click', close);
  startBtn.type = 'button';
  startBtn.className = 'primary';
  startBtn.textContent = tx('handoff_dialog_start', '起動');
  startBtn.addEventListener('click', () => {
    if (startBtn.disabled) return;
    submitting = true;
    refreshStartEnabled();
    setError('');
    void submit(false).catch((error) => {
      setError(String(error instanceof Error ? error.message : error));
    }).finally(() => {
      submitting = false;
      refreshStartEnabled();
    });
  });
  actionsEl.innerHTML = '';
  actionsEl.appendChild(cancelBtn);
  actionsEl.appendChild(startBtn);

  kindSelect.value = kind;
  applyKind();
  // 起点の provider が子の候補にあれば初期値にする（親と同じ CLI で子を立てるのが
  // 既定の期待）。引き継ぎでは候補から外れているので、その場合は空のままになる。
  if (!providerSelect.value && source.provider) {
    const match = Array.from(providerSelect.options).find((o) => o.value === source.provider);
    if (match) {
      providerSelect.value = source.provider;
      rebuildSubscriptionSelect();
      rebuildEffortSelect();
      void refreshModelChoices();
      refreshApprovalDisplay();
    }
  }
  refreshStartEnabled();
  promptArea.focus();
}
