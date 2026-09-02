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
  unregisterDialogController,
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
        <input name="model" class="spawn-input" value="${escapeHtml(record.model)}" placeholder="${escapeHtml(t('spawn_confirm_model_placeholder'))}" spellcheck="false" autocomplete="off">
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
  const providerIcon = dialog.querySelector('[data-spawn-confirm-icon]');
  const statusEl = dialog.querySelector('[data-spawn-confirm-status]') as HTMLElement | null;
  const actionsEl = dialog.querySelector('[data-spawn-confirm-actions]') as HTMLElement | null;

  if (providerSelect && providerIcon) {
    providerSelect.addEventListener('change', () => {
      providerIcon.innerHTML = providerIconHtml(providerSelect.value);
    });
  }

  // 'open': まだ何も決めていない（Escape で閉じてよい・拒否は送らない）。
  // 'deciding': approve/refuse を押して POST 送信済み。結果（spawn_confirmation_closed か
  //             POST 自体のエラー応答）を待っている。この間は Escape を無効化する。
  // 'terminal': 結果が出た。approved/refused/superseded/parent_gone は自動で閉じる。
  //             spawn_failed / expired / decided_elsewhere / submit_failed はユーザーが
  //             閉じるまで残す。
  let state: 'open' | 'deciding' | 'terminal' = 'open';
  let closing = false;
  let elapsedTimer: ReturnType<typeof setInterval> | null = setInterval(() => {
    const el = dialog.querySelector('[data-spawn-confirm-elapsed]');
    if (el) el.textContent = formatElapsed(record.requestedAtMs);
  }, SPAWN_CONFIRM_ELAPSED_TICK_MS);
  let autoCloseTimer: ReturnType<typeof setTimeout> | null = null;

  function stopTimers(): void {
    if (elapsedTimer) { clearInterval(elapsedTimer); elapsedTimer = null; }
    if (autoCloseTimer) { clearTimeout(autoCloseTimer); autoCloseTimer = null; }
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
    autoCloseTimer = setTimeout(cleanup, SPAWN_CONFIRM_AUTO_CLOSE_MS);
  }

  async function decide(approved: boolean): Promise<void> {
    if (state !== 'open') return;
    state = 'deciding';
    if (providerSelect) providerSelect.disabled = true;
    if (modelInput) modelInput.disabled = true;
    setActionsToProcessing();
    showStatus(t('spawn_confirm_processing'));
    const providerVal = providerSelect?.value || '';
    const modelVal = modelInput?.value || '';
    try {
      const res = await fetch(`/api/sessions/${record.parentId}/spawn-confirm?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ confirmation_id: record.id, approved, provider: providerVal, model: modelVal }),
      });
      if (res.ok) return; // 結果は spawn_confirmation_closed の broadcast を待って反映する。
      if (state !== 'deciding') return; // 既に broadcast 経由で terminal 済み。
      state = 'terminal';
      if (res.status === 404) showStatus(t('spawn_confirm_result_expired'));
      else if (res.status === 409) showStatus(t('spawn_confirm_result_decided_elsewhere'));
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

  // 'deciding' の間だけ Escape/cancel を無効化する。POST の応答か broadcast のどちらかで
  // 必ず 'terminal' か cleanup() のどちらかに落ち着くので、無限に固まることはない。
  dialog.addEventListener('cancel', (e) => {
    if (state === 'deciding') e.preventDefault();
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
      state = 'terminal';
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
  dialog.showModal();
}
