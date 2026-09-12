// 子セッション起動の承認ダイアログ。
// 元は ws-client.ts に直書きされていた showSpawnConfirmation を、
// plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md C3/C4 で
// 独立モジュールへ切り出した。session-list.ts の activateSession からも参照できる
// 必要があるため（ws-client.ts は session-list.ts を import しており、その逆向きの
// import は既存の循環 import と同じ形なので許容する）。
//
// 保留の純粋なストア（DOM に触れない部分）は spawn-confirm-store.ts に分離している
// （そちらのファイル冒頭コメントに理由がある）。このファイルは DOM 組み立てと
// イベント配線だけを持つ。
//
// 見た目の正本は web/src/styles/spawn-confirm.css。
import { t } from '../i18n.js';
import { escapeHtml, token } from './util.js';
import {
  fillModelDatalist,
  getCachedSpawnModelGroups,
  isModelCompatibleWithProvider,
  loadSpawnModelGroups,
} from './spawn-model-groups.js';
import { activeSessionId, sessions } from './state.js';
import { providerIconHtml } from './session-list.js';
import { ORCHESTRATION_CLI_OPTIONS, ORCHESTRATION_ROLE_DEFS } from './orchestration-roles.js';
import {
  clearAllSpawnConfirmationsForHubRestart,
  closeSpawnConfirmation,
  isDialogControllerOpen,
  noteSpawnConfirmationRequested,
  pendingSpawnConfirmationCount,
  registerDialogController,
  selectOldestPendingConfirmationFor,
  spawnConfirmDecisionFromHttp,
  unregisterDialogController,
  effortLevelsFor,
  isExecutionModeAvailable,
  isPermissionPresetAvailable,
  EXECUTION_MODE_SCHEMA,
  PERMISSION_PRESET_SCHEMA,
  INTERNAL_BOUNDED_PERMISSION_MODE,
  approvalForSelection,
  headlessUnsupportedForSelection,
  type ChildApproval,
  type SpawnConfirmationRecord,
} from './spawn-confirm-store.js';

export { clearAllSpawnConfirmationsForHubRestart, closeSpawnConfirmation, noteSpawnConfirmationRequested, pendingSpawnConfirmationCount };

const SPAWN_CONFIRM_PROMPT_LIMIT = 4000;
// 経過時間表示の更新間隔。秒単位で動かしても読む側には分単位でしか見えないので、
// タイマーの発火回数を抑えるためこの粒度にしている。
const SPAWN_CONFIRM_ELAPSED_TICK_MS = 30_000;
// 決定が付いた（approved/refused/superseded/parent_gone）後、ダイアログを
// 自動で閉じるまでの表示時間。ユーザーが結果を読める程度の長さにする。
// spawn_failed だけはここを使わず、ユーザーの操作でしか閉じない。
const SPAWN_CONFIRM_AUTO_CLOSE_MS = 1600;

function spawnConfirmRoleLabel(role: string): string {
  const raw = String(role || '').trim();
  if (!raw) return t('spawn_confirm_role_unset');
  const def = ORCHESTRATION_ROLE_DEFS.find((d) => d.key === raw);
  return def ? `${t(def.labelKey)}（${raw}）` : raw;
}

// その親がアクティブになった（session-list.ts の activateSession から呼ばれる）
// タイミングで、保留のうち最も古い 1 件だけを開く。決定して閉じたら、残りがあれば
// 続けて次を開く（showSpawnConfirmationDialog 内の cleanup() から再帰的に呼ばれる）。
export function openNextSpawnConfirmationFor(parentId: number): void {
  if (!parentId) return;
  const next = selectOldestPendingConfirmationFor(parentId);
  if (next) showSpawnConfirmationDialog(next);
}

function formatElapsed(requestedAtMs: number): string {
  if (!requestedAtMs) return t('spawn_confirm_elapsed_just_now');
  const minutes = Math.floor(Math.max(0, Date.now() - requestedAtMs) / 60000);
  if (minutes <= 0) return t('spawn_confirm_elapsed_just_now');
  return t('spawn_confirm_elapsed_minutes', { minutes });
}

function repoNameFromParent(parent: any): string {
  const projectId = String(parent?.project_id || parent?.cwd || '').trim();
  if (!projectId) return '';
  const parts = projectId.split(/[\\/]+/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : projectId;
}

// 作業ディレクトリ表示。record.cwd が指定されていればそれを、空なら親セッションの
// 実 cwd を解決して出す（「親と同じ」とだけ言われても実体が分からない、という
// 元の不満への対応）。親も見つからない最終フォールバックだけ旧来の汎用文言に戻す。
function cwdDisplayHtml(record: SpawnConfirmationRecord, parent: any): string {
  const explicit = String(record.cwd || '').trim();
  if (explicit) return escapeHtml(explicit);
  const parentCwd = String(parent?.cwd || '').trim();
  if (parentCwd) {
    return `${escapeHtml(parentCwd)} <span class="spawn-confirm-inherit-note">(${escapeHtml(t('spawn_confirm_cwd_inherited_note'))})</span>`;
  }
  return escapeHtml(t('spawn_confirm_cwd_inherit'));
}

// 起動要求の共通 3 項目の select。
//
// effort は provider ごとに写像がある場合だけ欄を出す（候補が無い select を
// 見せない）。実行モードと権限段はスキーマ全体を出し、この build で選べない値は
// disabled にする: 欄ごと消すより「まだ選べない」と分かる方がよい。受理の判断は
// 常に Hub 側（internal/config/effort.go）が行い、ここは表示だけを決める。
function effortFieldHidden(provider: string): boolean {
  return effortLevelsFor(provider).length === 0;
}

function choiceOptionsHtml(
  values: readonly string[],
  selected: string,
  isAvailable: (v: string) => boolean,
  label: (v: string) => string = (v) => v,
): string {
  const unset = `<option value=""${selected ? '' : ' selected'}>${escapeHtml(t('spawn_confirm_option_unset'))}</option>`;
  const opts = values.map((value) => {
    const disabled = isAvailable(value) ? '' : ' disabled';
    const sel = value === selected ? ' selected' : '';
    return `<option value="${escapeHtml(value)}"${sel}${disabled}>${escapeHtml(label(value))}</option>`;
  });
  return [unset, ...opts].join('');
}

// 段の名前は訳す。value は raw のまま（Hub が受け取るのは attended / bounded / full）。
// 「full」とだけ書かれていても何が起きるかは伝わらないので、選ぶ側には
// 「全許可」と読ませ、実効フラグは下の権限欄で見せる。
export function permissionPresetLabel(preset: string): string {
  switch (preset) {
    case 'attended':
      return t('spawn_confirm_tier_attended');
    case 'bounded':
      return t('spawn_confirm_tier_bounded');
    case 'full':
      return t('spawn_confirm_tier_full');
    default:
      return preset;
  }
}

function effortOptionsHtml(provider: string, selected: string): string {
  return choiceOptionsHtml(effortLevelsFor(provider), selected, () => true);
}

function executionModeOptionsHtml(selected: string): string {
  return choiceOptionsHtml(EXECUTION_MODE_SCHEMA, selected, isExecutionModeAvailable);
}

function permissionPresetOptionsHtml(selected: string): string {
  return choiceOptionsHtml(PERMISSION_PRESET_SCHEMA, selected, isPermissionPresetAvailable, permissionPresetLabel);
}

// 承認する人に「この子に何を渡すのか」を見せる欄。これが無いと、role / provider /
// model / cwd / 指示文しか出ないまま codex の子が --sandbox danger-full-access で
// 起動する（docs/local/bugfix_spawn-confirm-permission-disclosure_2026-09-08.md）。
// 値は Hub が権限の段の表を通して作ったものをそのまま出す。ここで provider から
// 権限を組み立て直すと、Hub 側の既定を変えたときに表示だけ古くなる。
// export しているのは派生ダイアログ（derive-dialog.ts）が同じ開示を出すため。
// DOM を組まず文字列を返すだけなので、呼ぶ側は innerHTML へ差すだけでよい。
// 開示を 2 通り書くと片方だけ古くなるので、増やさずここを共有する。
export function approvalDisplayHtml(approval: ChildApproval | undefined, headlessUnsupported = false): string {
  const flags: string[] = [];
  if (approval?.sandbox) flags.push(`--sandbox ${approval.sandbox}`);
  if (approval?.askForApproval) flags.push(`--ask-for-approval ${approval.askForApproval}`);
  // 内部マーカーはフラグ名として出さない（実在しない起動コマンドになる）。
  // その段が何をするかは段の名前と許可 tool の一覧の側で伝わる。
  if (approval?.permissionMode && approval.permissionMode !== INTERNAL_BOUNDED_PERMISSION_MODE) {
    flags.push(`--permission-mode ${approval.permissionMode}`);
  }
  const tools = approval?.allowedTools ?? [];
  const parts: string[] = [];
  if (approval?.tier) {
    parts.push(`<span class="spawn-confirm-approval-tier">${escapeHtml(permissionPresetLabel(approval.tier))}</span>`);
  }
  if (flags.length) {
    parts.push(`<code class="spawn-confirm-approval-flags">${escapeHtml(flags.join(' '))}</code>`);
  }
  if (tools.length) {
    parts.push(
      `<span class="spawn-confirm-approval-tools">${escapeHtml(t('spawn_confirm_approval_allowed_tools'))}: <code class="spawn-confirm-approval-flags">${escapeHtml(tools.join(' '))}</code></span>`,
    );
  }
  if (!flags.length && !tools.length) {
    parts.push(`<span class="spawn-confirm-approval-none">${escapeHtml(t('spawn_confirm_approval_none'))}</span>`);
  }
  if (approval?.riskConfirmed) {
    parts.push(`<span class="spawn-confirm-approval-risk">${escapeHtml(t('spawn_confirm_approval_risk'))}</span>`);
  }
  // 段 2 を選んだのに、その CLI には範囲を限る設定が無くて全許可へ落ちた場合。
  // 黙って落とさないのが親 plan の不変条件 5。
  if (approval?.fallbackFrom) {
    parts.push(`<span class="spawn-confirm-approval-fallback">${escapeHtml(t('spawn_confirm_tier_fallback'))}</span>`);
  }
  if (headlessUnsupported) {
    parts.push(headlessUnsupportedNoticeHtml());
  }
  return parts.join(' ');
}

// headless を選んだが、この CLI には非対話モードの定義が無い、の 1 行。Hub は起動を
// 400 で断る（黙って対話へ倒さない・親 plan D2）ので、押してから知るのではなく、
// 権限と同じ欄で先に言う（子 plan 内部 C6）。
//
// 単体で export しているのは、権限の表をまだ受け取っていない（古い Hub）ときでも
// この 1 行だけは出せるから。そこで approvalDisplayHtml を呼ぶと「何も足さない」と
// 併記され、知らないことを断定してしまう。
export function headlessUnsupportedNoticeHtml(): string {
  return `<span class="spawn-confirm-approval-fallback">${escapeHtml(t('spawn_confirm_headless_unsupported'))}</span>`;
}

function outcomeMessage(reason: string, m: any): string {
  switch (reason) {
    case 'approved':
      return t('spawn_confirm_result_approved', { id: Number(m?.spawn_child_session_id || 0) });
    case 'refused':
      return t('spawn_confirm_result_refused');
    case 'superseded':
      return t('spawn_confirm_result_superseded');
    case 'parent_gone':
      return t('spawn_confirm_result_parent_gone');
    case 'spawn_failed':
      return t('spawn_confirm_result_spawn_failed', { detail: String(m?.text || '') });
    default:
      return t('spawn_confirm_result_refused');
  }
}

function showSpawnConfirmationDialog(record: SpawnConfirmationRecord): void {
  if (!record.id) return;
  if (isDialogControllerOpen(record.id)) return;
  if (document.querySelector(`[data-spawn-confirmation-id="${CSS.escape(record.id)}"]`)) return;

  const parent = sessions.get(record.parentId) as any;
  const parentGone = !parent;

  const dialog = document.createElement('dialog');
  // aac-wheel-overlay: これが無いと document レベルの wheel ハンドラが背後の
  // ターミナルへホイールを転送し、指示文をスクロールできなくなる
  // （scripts/check-wheel-overlays.mjs が付け忘れを止める）。
  dialog.className = 'spawn-confirm-dialog aac-wheel-overlay';
  dialog.dataset.spawnConfirmationId = record.id;

  const prompt = record.initialPrompt.trim();
  const shownPrompt = prompt.slice(0, SPAWN_CONFIRM_PROMPT_LIMIT);
  const truncated = prompt.length > shownPrompt.length;
  const provider = record.provider;
  const providerOptions = ORCHESTRATION_CLI_OPTIONS.filter((o) => o.value);
  const knownProvider = providerOptions.some((o) => o.value === provider);
  const optionHtml = (value: string, label: string) =>
    `<option value="${escapeHtml(value)}"${value === provider ? ' selected' : ''}>${escapeHtml(label)}</option>`;
  const optionsHtml = [
    ...(provider && !knownProvider ? [optionHtml(provider, provider)] : []),
    ...providerOptions.map((o) => optionHtml(o.value, o.label || t(o.labelKey || ''))),
  ].join('');

  const repoLabel = repoNameFromParent(parent) || '—';
  const branchStr = String(parent?.branch || '').trim();
  const parentTitle = String(parent?.label || parent?.display_name || '').trim();

  dialog.innerHTML = `<form class="spawn-confirm-form">
  <div class="spawn-confirm-header">
    <h2 class="spawn-confirm-title">${escapeHtml(t('spawn_confirm_title'))}</h2>
    ${record.parentId ? `<span class="spawn-confirm-parent">${escapeHtml(t('spawn_confirm_parent', { id: record.parentId }))}${parentTitle ? ' · ' + escapeHtml(parentTitle) : ''}</span>` : ''}
  </div>
  <div class="spawn-confirm-body">
    <dl class="spawn-confirm-meta">
      <dt>${escapeHtml(t('spawn_confirm_repo'))}</dt>
      <dd>${escapeHtml(repoLabel)}</dd>
      ${branchStr ? `<dt>${escapeHtml(t('spawn_confirm_branch'))}</dt><dd>${escapeHtml(branchStr)}</dd>` : ''}
      <dt>${escapeHtml(t('spawn_role_table_role'))}</dt>
      <dd>${escapeHtml(spawnConfirmRoleLabel(record.role))}</dd>
      <dt>${escapeHtml(t('spawn_confirm_cwd'))}</dt>
      <dd class="spawn-confirm-path">${cwdDisplayHtml(record, parent)}</dd>
      <dt>${escapeHtml(t('spawn_confirm_approval'))}</dt>
      <dd class="spawn-confirm-approval" data-spawn-confirm-approval>${approvalDisplayHtml(approvalForSelection(record.approval, provider, record.permissionPreset))}</dd>
      <dt>${escapeHtml(t('spawn_confirm_elapsed_label'))}</dt>
      <dd data-spawn-confirm-elapsed>${escapeHtml(formatElapsed(record.requestedAtMs))}</dd>
    </dl>
    ${parentGone ? `<p class="spawn-confirm-parent-gone">${escapeHtml(t('spawn_confirm_parent_gone'))}</p>` : ''}
    <div class="spawn-confirm-fields">
      <label class="spawn-confirm-field">
        <span class="spawn-confirm-field-label">${escapeHtml(t('spawn_role_table_cli'))}</span>
        <span class="spawn-confirm-provider-row">
          <span class="spawn-confirm-provider-icon" data-spawn-confirm-icon>${providerIconHtml(provider)}</span>
          <select name="provider" class="spawn-select">${optionsHtml}</select>
        </span>
      </label>
      <label class="spawn-confirm-field">
        <span class="spawn-confirm-field-label">${escapeHtml(t('spawn_role_table_model'))}</span>
        <input name="model" class="spawn-input" list="spawn-confirm-model-datalist-${escapeHtml(record.id)}" value="${escapeHtml(record.model)}" placeholder="${escapeHtml(t('spawn_confirm_model_placeholder'))}" spellcheck="false" autocomplete="off">
        <datalist id="spawn-confirm-model-datalist-${escapeHtml(record.id)}"></datalist>
      </label>
      <label class="spawn-confirm-field" data-spawn-confirm-effort-field${effortFieldHidden(provider) ? ' hidden' : ''}>
        <span class="spawn-confirm-field-label">${escapeHtml(t('spawn_confirm_effort'))}</span>
        <select name="effort" class="spawn-select" data-spawn-confirm-effort>${effortOptionsHtml(provider, record.effort)}</select>
      </label>
      <label class="spawn-confirm-field">
        <span class="spawn-confirm-field-label">${escapeHtml(t('spawn_confirm_execution_mode'))}</span>
        <select name="execution_mode" class="spawn-select">${executionModeOptionsHtml(record.executionMode)}</select>
      </label>
      <label class="spawn-confirm-field">
        <span class="spawn-confirm-field-label">${escapeHtml(t('spawn_confirm_permission_preset'))}</span>
        <select name="permission_preset" class="spawn-select">${permissionPresetOptionsHtml(record.permissionPreset)}</select>
      </label>
      <label class="spawn-confirm-remember">
        <input type="checkbox" name="remember_permission"${record.rememberPermission ? ' checked' : ''}>
        <span>${escapeHtml(t('spawn_confirm_remember_permission'))}</span>
      </label>
    </div>
    <div class="spawn-confirm-prompt-block">
      <div class="spawn-confirm-prompt-head">
        <span>${escapeHtml(t('spawn_confirm_prompt'))}</span>
        ${truncated ? `<span class="spawn-confirm-prompt-note">${escapeHtml(t('spawn_confirm_prompt_truncated', { shown: shownPrompt.length, total: prompt.length }))}</span>` : ''}
      </div>
      <pre class="spawn-confirm-prompt${prompt ? '' : ' is-empty'}">${escapeHtml(prompt ? shownPrompt : t('spawn_confirm_prompt_empty'))}</pre>
    </div>
  </div>
  <p class="spawn-confirm-status" data-spawn-confirm-status hidden></p>
  <div class="spawn-confirm-actions" data-spawn-confirm-actions>
    <button type="button" class="spawn-confirm-btn" data-action="refuse">${escapeHtml(t('spawn_confirm_refuse'))}</button>
    <button type="button" class="spawn-confirm-btn is-primary" data-action="approve"${parentGone ? ' disabled' : ''}>${escapeHtml(t('spawn_confirm_approve'))}</button>
  </div>
</form>`;

  const providerSelect = dialog.querySelector('[name=provider]') as HTMLSelectElement | null;
  const modelInput = dialog.querySelector('[name=model]') as HTMLInputElement | null;
  const modelDatalist = dialog.querySelector('datalist') as HTMLDataListElement | null;
  const providerIcon = dialog.querySelector('[data-spawn-confirm-icon]');
  const statusEl = dialog.querySelector('[data-spawn-confirm-status]') as HTMLElement | null;
  const actionsEl = dialog.querySelector('[data-spawn-confirm-actions]') as HTMLElement | null;

  const approvalEl = dialog.querySelector('[data-spawn-confirm-approval]');
  const effortField = dialog.querySelector('[data-spawn-confirm-effort-field]') as HTMLElement | null;
  const effortSelect = dialog.querySelector('[data-spawn-confirm-effort]') as HTMLSelectElement | null;
  const presetSelect = dialog.querySelector('[name=permission_preset]') as HTMLSelectElement | null;
  const executionSelect = dialog.querySelector('[name=execution_mode]') as HTMLSelectElement | null;
  const rememberInput = dialog.querySelector('[name=remember_permission]') as HTMLInputElement | null;

  // 権限欄は provider と段の 2 つで決まる。どちらを差し替えても同じ 1 本を通して
  // 引き直す（片方だけ追従する経路を作らない）。値は起動時に受け取った表から引くので
  // Hub への往復は無い。実行モードも同じ 1 本に乗せる: headless を選んだ provider に
  // 定義が無ければ、承認しても Hub が 400 で断るので、その 1 行をここへ出す。
  function refreshApprovalDisplay(): void {
    if (!approvalEl) return;
    const provider = providerSelect?.value || record.provider;
    const tier = presetSelect ? presetSelect.value : record.permissionPreset;
    const mode = executionSelect ? executionSelect.value : record.executionMode;
    approvalEl.innerHTML = approvalDisplayHtml(
      approvalForSelection(record.approval, provider, tier),
      headlessUnsupportedForSelection(provider, mode),
    );
  }

  async function refreshModelChoices(): Promise<void> {
    const provider = providerSelect?.value || record.provider;
    try {
      await loadSpawnModelGroups(token);
    } catch (_) {
      // 候補が空でも手入力で承認できる。
    }
    const groups = getCachedSpawnModelGroups();
    fillModelDatalist(modelDatalist, groups, provider);
    if (modelInput && !isModelCompatibleWithProvider(groups, provider, modelInput.value)) {
      modelInput.value = '';
    }
  }

  if (providerSelect) {
    providerSelect.addEventListener('change', () => {
      if (providerIcon) providerIcon.innerHTML = providerIconHtml(providerSelect.value);
      // provider を差し替えたら渡る権限も変わる。アイコンだけ追従して権限欄が
      // 前の provider のまま残ると、承認する人が見ている情報が実物と食い違う。
      refreshApprovalDisplay();
      // effort の候補も provider ごとに違う。写像が無い provider を選んだら欄を隠し、
      // 前の provider の値を送ってしまわないよう選択も空へ戻す（Hub はその値を
      // 400 で弾くので、残ったままだと承認できなくなる）。
      if (effortSelect) {
        const levels = effortLevelsFor(providerSelect.value);
        effortSelect.innerHTML = effortOptionsHtml(providerSelect.value, levels.includes(effortSelect.value) ? effortSelect.value : '');
        if (effortField) effortField.hidden = levels.length === 0;
      }
      void refreshModelChoices();
    });
  }
  if (presetSelect) {
    presetSelect.addEventListener('change', refreshApprovalDisplay);
  }
  if (executionSelect) {
    executionSelect.addEventListener('change', refreshApprovalDisplay);
  }

  // 'open': まだ何も決めていない（Escape で閉じてよい・拒否は送らない）。
  // 'deciding': approve/refuse を押して POST 送信済み。結果（spawn_confirmation_closed か
  //             POST 自体のエラー応答）を待っている。この間は Escape を無効化する。
  // 'terminal': 結果が出た。approved/refused/superseded/parent_gone は自動で閉じる。
  //             spawn_failed / expired / decided_elsewhere / submit_failed はユーザーが
  //             閉じるまで残す。
  let state: 'open' | 'deciding' | 'terminal' = 'open';
  let closing = false;
  // Set after HTTP 200 if spawn_confirmation_closed never arrives, so Escape
  // and Close can dismiss the overlay without a second POST.
  let dismissibleWhileDeciding = false;
  let elapsedTimer: ReturnType<typeof setInterval> | null = setInterval(() => {
    const el = dialog.querySelector('[data-spawn-confirm-elapsed]');
    if (el) el.textContent = formatElapsed(record.requestedAtMs);
  }, SPAWN_CONFIRM_ELAPSED_TICK_MS);
  let autoCloseTimer: ReturnType<typeof setTimeout> | null = null;
  let closedFallbackTimer: ReturnType<typeof setTimeout> | null = null;

  function stopTimers(): void {
    if (elapsedTimer) { clearInterval(elapsedTimer); elapsedTimer = null; }
    if (autoCloseTimer) { clearTimeout(autoCloseTimer); autoCloseTimer = null; }
    if (closedFallbackTimer) { clearTimeout(closedFallbackTimer); closedFallbackTimer = null; }
  }

  // 決定済み（state !== 'open'）のときだけ「次の保留があれば続けて開く」。
  // 素の Escape（state === 'open' のまま閉じた）は何も決まっていないので連鎖させない
  // — 保留は Hub 側に残っており、もう一度この親を選べば同じ確認がまた開く。
  function cleanup(): void {
    if (closing) return;
    closing = true;
    stopTimers();
    unregisterDialogController(record.id);
    const shouldChain = state !== 'open';
    try { dialog.close(); } catch (_) {}
    dialog.remove();
    if (shouldChain && activeSessionId === record.parentId) {
      openNextSpawnConfirmationFor(record.parentId);
    }
  }

  function showStatus(text: string): void {
    if (!statusEl) return;
    statusEl.textContent = text;
    statusEl.hidden = false;
  }

  function setActionsToProcessing(): void {
    if (!actionsEl) return;
    actionsEl.innerHTML = `<span class="spawn-confirm-processing">${escapeHtml(t('spawn_confirm_processing'))}</span>`;
  }

  function setActionsToClose(): void {
    if (!actionsEl) return;
    actionsEl.innerHTML = `<button type="button" class="spawn-confirm-btn is-primary" data-action="close">${escapeHtml(t('spawn_confirm_close'))}</button>`;
  }

  function scheduleAutoClose(): void {
    if (autoCloseTimer) { clearTimeout(autoCloseTimer); autoCloseTimer = null; }
    autoCloseTimer = setTimeout(cleanup, SPAWN_CONFIRM_AUTO_CLOSE_MS);
  }

  function armClosedFallback(fallbackMs: number): void {
    if (closedFallbackTimer) { clearTimeout(closedFallbackTimer); closedFallbackTimer = null; }
    closedFallbackTimer = setTimeout(() => {
      closedFallbackTimer = null;
      if (state !== 'deciding') return;
      dismissibleWhileDeciding = true;
      showStatus(t('spawn_confirm_result_accepted'));
      setActionsToClose();
    }, fallbackMs);
  }

  async function decide(approved: boolean): Promise<void> {
    if (state !== 'open') return;
    const providerVal = providerSelect?.value || '';
    const rawModel = (modelInput?.value || '').trim();
    const groups = getCachedSpawnModelGroups();
    if (approved && rawModel && !isModelCompatibleWithProvider(groups, providerVal, rawModel)) {
      if (modelInput) modelInput.value = '';
      showStatus(t('spawn_model_provider_mismatch'));
      return;
    }
    state = 'deciding';
    if (providerSelect) providerSelect.disabled = true;
    if (modelInput) modelInput.disabled = true;
    for (const name of ['effort', 'execution_mode', 'permission_preset']) {
      const el = dialog.querySelector(`[name=${name}]`) as HTMLSelectElement | null;
      if (el) el.disabled = true;
    }
    if (rememberInput) rememberInput.disabled = true;
    setActionsToProcessing();
    showStatus(t('spawn_confirm_processing'));
    const modelVal = rawModel;
    // 3 項目は select が見せている実効値をそのまま送る。Hub は送られた値をそのまま
    // 使うので、欄を触らなければ要求どおり、「指定なし」を選べば空になる。effort 欄が
    // 隠れている provider（写像なし）では空を送り、要求時の effort を引きずらない。
    const selectValue = (name: string): string => {
      const el = dialog.querySelector(`[name=${name}]`) as HTMLSelectElement | null;
      return el?.value || '';
    };
    const effortVal = effortField?.hidden ? '' : selectValue('effort');
    const executionModeVal = selectValue('execution_mode');
    const permissionPresetVal = selectValue('permission_preset');
    // 「次回もこの段」は承認したときだけ送る。拒否は何も起きなかったのと同じなので、
    // 記憶を消す（false を送る）指示にしない — 欄ごと送らなければ Hub は記憶を触らない。
    const payload: Record<string, unknown> = {
      confirmation_id: record.id,
      approved,
      provider: providerVal,
      model: modelVal,
      effort: effortVal,
      execution_mode: executionModeVal,
      permission_preset: permissionPresetVal,
    };
    if (approved) payload.remember_permission = !!rememberInput?.checked;
    try {
      const res = await fetch(`/api/sessions/${record.parentId}/spawn-confirm?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });
      if (state !== 'deciding') return; // 既に broadcast 経由で terminal 済み。
      const decision = spawnConfirmDecisionFromHttp(res.ok, res.status);
      if (decision.waitForCloseBroadcast) {
        // Hub accepted the decision. Wait briefly for spawn_confirmation_closed,
        // then offer Close so a down/stale WS cannot wedge the overlay.
        armClosedFallback(decision.fallbackMs);
        return;
      }
      state = 'terminal';
      if (decision.terminalReason === 'expired') showStatus(t('spawn_confirm_result_expired'));
      else if (decision.terminalReason === 'decided_elsewhere') showStatus(t('spawn_confirm_result_decided_elsewhere'));
      else showStatus(t('spawn_confirm_submit_failed'));
      setActionsToClose();
    } catch (_) {
      if (state !== 'deciding') return;
      state = 'terminal';
      showStatus(t('spawn_confirm_submit_failed'));
      setActionsToClose();
    }
  }

  if (actionsEl) {
    actionsEl.addEventListener('click', (e) => {
      const target = (e.target as HTMLElement)?.closest('[data-action]') as HTMLElement | null;
      if (!target) return;
      const action = target.dataset.action;
      if (action === 'approve') decide(true);
      else if (action === 'refuse') decide(false);
      else if (action === 'close') cleanup();
    });
  }

  // 'deciding' の間は Escape を無効化する。HTTP 200 後に WS close が来なければ
  // fallback が Close を出し、その時点から Escape も許可する。
  dialog.addEventListener('cancel', (e) => {
    if (state === 'deciding' && !dismissibleWhileDeciding) e.preventDefault();
  });

  // ここが「Escape で拒否を送らない」の実装本体
  // (bugfix_spawn-confirm-modal-blocks-other-sessions_2026-09-02.md)。
  // ボタンはすべて type="button" なので、フォームの暗黙送信（旧: <form method="dialog">
  // が最初の submit ボタン＝拒否側を暗黙に踏んでいた）はそもそも起こらない。
  // close イベントは Escape（cleanup() 未実行のまま state が 'open' で届く）と、
  // cleanup() 自身が呼ぶ dialog.close() の両方から届くが、closing フラグで後者は無視する。
  dialog.addEventListener('close', () => {
    if (closing) return;
    cleanup();
  }, { once: true });

  registerDialogController(record.id, {
    applyClosed(m: any) {
      if (state === 'terminal') return; // 既にローカルの失敗表示で terminal 済み。
      if (closedFallbackTimer) { clearTimeout(closedFallbackTimer); closedFallbackTimer = null; }
      state = 'terminal';
      dismissibleWhileDeciding = false;
      const reason = String(m?.reason || '');
      showStatus(outcomeMessage(reason, m));
      if (reason === 'spawn_failed') {
        setActionsToClose();
        return;
      }
      if (actionsEl) actionsEl.innerHTML = '';
      scheduleAutoClose();
    },
  });

  document.body.appendChild(dialog);
  void refreshModelChoices();
  dialog.showModal();
}
