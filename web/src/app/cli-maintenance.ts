// cli-maintenance.ts — 導入状況一覧の「導入済み」行の共通部品（画面案 S-01〜S-03）。
// 初回画面（zero-session-empty-state.ts）と「設定 → AI CLI 連携」（provider-manager.ts）
// の両方から mountCliMaintenance(container, installed) で呼ばれる。
// plan_provider-cli-update_c4_list-ui.md。
//
// 「未導入」行（インストール手順リンク）は呼び出し側がそのまま持つ（子 plan C2:
// 「未導入の行の『インストール手順』リンクは今のまま残す」）。ここは provider 一覧
// そのものは受け取り、バージョン確認・更新可否・更新ジョブの API だけを自分で持つ。
import { t } from '../i18n.js';
import {
  cliUpdateEligibleCount,
  cliUpdateFailureDetail,
  cliUpdatePlanGroups,
  cliUpdateRowState,
  cliUpdateSummaryCounts,
  cliUpdateSummaryParts,
  cliVersionCellText,
  formatCliDateTime,
  type CliInstallStatus,
  type CliUpdateEligibilityEntry,
  type CliUpdateExcludedEntry,
  type CliUpdateProviderStatus,
  type CliVersionResult,
  type I18nMessage,
} from './cli-availability.js';
import { loadProviderSummaries } from './provider-store.js';
import { providerIconHtml } from './session-list.js';
import { apiFetch, escapeHtml, token } from './util.js';

// fetchInstallLinkDefaults は provider id → 公式インストール手順 URL。元は
// zero-session-empty-state.ts だけが持っていたが、導入状況一覧の取得元をここへ一本化
// したのでこちらへ移した（settings.ts の usage-link-defaults 取得と揃えた書き方: token を
// クエリに付ける生 fetch。取得できなくても一覧は出す方針なので、失敗時は空 map）。
export async function fetchInstallLinkDefaults(): Promise<Record<string, string>> {
  try {
    const res = await fetch(`/api/install-link-defaults?token=${encodeURIComponent(token || '')}`);
    if (!res.ok) return {};
    const body = await res.json();
    return body && typeof body === 'object' ? body : {};
  } catch (_) {
    return {};
  }
}

// missingCliRowsHtml は「未導入」行（zero-session-empty-state.ts の元の
// renderInstallStatusList から移した・見た目とキーは変えていない）。導入済みの行は
// mountCliMaintenance が別に描くので、ここは未導入の行だけを組み立てる。
export function missingCliRowsHtml(missing: CliInstallStatus[]): string {
  if (!missing.length) return '';
  const items = missing.map((status) => {
    const link = status.installUrl
      ? `<a class="zero-session-cli-install-link" href="${escapeHtml(status.installUrl)}" target="_blank" rel="noopener">${t('zero_session_install_link_label')}</a>`
      : `<span class="zero-session-cli-install-unavailable">${t('zero_session_install_link_unavailable')}</span>`;
    return (
      `<li class="zero-session-cli-item zero-session-cli-item-missing">` +
      providerIconHtml(status.id, 16) +
      `<span class="zero-session-cli-label">${escapeHtml(status.displayName)}</span>` +
      `<span class="zero-session-cli-status">${t('zero_session_install_status_missing')}</span>` +
      link +
      `</li>`
    );
  }).join('');
  return `<ul class="zero-session-cli-list zero-session-install-list">${items}</ul>`;
}

type CliVersionsResponse = { checked_at?: string; results: CliVersionResult[] };

async function fetchCliVersions(method: 'GET' | 'POST'): Promise<CliVersionsResponse | null> {
  try {
    const res = await apiFetch(
      '/api/cli-versions',
      method === 'POST'
        ? { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ providers: [] }) }
        : undefined,
    );
    if (!res.ok) return null;
    const body = await res.json();
    return body && Array.isArray(body.results) ? body : { results: [] };
  } catch (_) {
    return null;
  }
}

async function fetchCliUpdateEligibility(): Promise<CliUpdateEligibilityEntry[] | null> {
  try {
    const res = await apiFetch('/api/cli-update-eligibility');
    if (!res.ok) return null;
    const body = await res.json();
    return Array.isArray(body?.results) ? body.results : [];
  } catch (_) {
    return null;
  }
}

type CliUpdateCreateResponse = { job_id: string; accepted: string[]; excluded: CliUpdateExcludedEntry[] };

async function postCliUpdates(providers: string[], serial: boolean): Promise<CliUpdateCreateResponse | null> {
  try {
    const res = await apiFetch('/api/cli-updates', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ providers, serial }),
    });
    if (!res.ok) return null;
    const body = await res.json();
    return body && typeof body.job_id === 'string' ? body : null;
  } catch (_) {
    return null;
  }
}

type CliUpdateJobResponse = {
  job_id: string;
  started_at: string;
  serial: boolean;
  providers: CliUpdateProviderStatus[];
  excluded: CliUpdateExcludedEntry[];
};

async function fetchCliUpdateJob(jobId: string): Promise<CliUpdateJobResponse | null> {
  try {
    const res = await apiFetch(`/api/cli-updates/${encodeURIComponent(jobId)}`);
    if (!res.ok) return null;
    const body = await res.json();
    return body && Array.isArray(body.providers) ? body : null;
  } catch (_) {
    return null;
  }
}

async function fetchCliUpdateLog(jobId: string, providerId: string): Promise<{ content: string; truncated: boolean } | null> {
  try {
    const res = await apiFetch(`/api/cli-updates/${encodeURIComponent(jobId)}/${encodeURIComponent(providerId)}/log`);
    if (!res.ok) return null;
    const body = await res.json();
    return body && typeof body.content === 'string' ? body : null;
  } catch (_) {
    return null;
  }
}

const FAILING_STATES = new Set(['failed', 'file_in_use', 'login_required', 'unknown']);
const IN_FLIGHT_STATES = new Set(['queued', 'running']);

type RowRuntime = {
  version?: CliVersionResult;
  checking?: boolean;
  eligibility?: CliUpdateEligibilityEntry;
  jobStatus?: CliUpdateProviderStatus;
  jobId?: string;
};

export type CliMaintenanceHandle = {
  // refresh は provider 一覧が変わったとき（追加・削除・有効/無効切り替え）に
  // 呼び出し側が渡す最新の「導入済み」一覧で作り直す。バージョン確認・更新可否は
  // 内部で引き直す。
  refresh: (installed: CliInstallStatus[]) => Promise<void>;
  destroy: () => void;
};

// mountCliMaintenance は container の中身を「導入済み」行の一覧にする。provider
// 一覧そのもの（未導入行を含む）は呼び出し側が持ち、installed（導入済みのものだけ）を
// 渡す。バージョン確認・更新可否・更新ジョブの取得と実行はここが持つ。
export function mountCliMaintenance(container: HTMLElement, installed: CliInstallStatus[]): CliMaintenanceHandle {
  const rows = new Map<string, RowRuntime>();
  let statuses: CliInstallStatus[] = installed;
  let checkedAt: string | undefined;
  let checkingAll = false;
  let justSettledSummary: { updated: number; latest: number; failed: number } | null = null;
  let activeJob: { id: string; serial: boolean; pending: Set<string>; total: number } | null = null;
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  let destroyed = false;

  function dow(): string[] {
    try {
      const parsed = JSON.parse(t('dow'));
      if (Array.isArray(parsed) && parsed.length === 7) return parsed;
    } catch (_) { /* fallthrough */ }
    return ['日', '月', '火', '水', '木', '金', '土'];
  }

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

  function txm(msg: I18nMessage): string {
    return tx(msg.key, msg.fallback, msg.vars || {});
  }

  function displayNameFor(id: string): string {
    return statuses.find((s) => s.id === id)?.displayName || id;
  }

  // --- データ取得 ---------------------------------------------------------

  async function loadVersionsAndEligibility(): Promise<void> {
    if (destroyed) return;
    const [versions, eligibility] = await Promise.all([
      fetchCliVersions('GET'),
      fetchCliUpdateEligibility(),
    ]);
    if (destroyed) return;
    if (versions) {
      checkedAt = versions.checked_at;
      for (const result of versions.results) {
        const r = rows.get(result.provider) || {};
        r.version = result;
        r.checking = false;
        rows.set(result.provider, r);
      }
    }
    if (eligibility) {
      for (const entry of eligibility) {
        const r = rows.get(entry.provider) || {};
        r.eligibility = entry;
        rows.set(entry.provider, r);
      }
    }
    render();
  }

  // refresh は provider 一覧そのものが変わったときに呼び出し側が使う（追加・削除・
  // 有効/無効切り替え後）。バージョン確認・更新可否は毎回引き直す。
  async function refresh(nextInstalled: CliInstallStatus[]): Promise<void> {
    statuses = nextInstalled;
    await loadVersionsAndEligibility();
  }

  // --- 描画 ----------------------------------------------------------------

  function progressVars(): { done: number; total: number } {
    if (!activeJob) return { done: 0, total: 0 };
    return { done: activeJob.total - activeJob.pending.size, total: activeJob.total };
  }

  function updateButtonHtml(status: CliInstallStatus, state: ReturnType<typeof cliUpdateRowState>): string {
    if (state.kind === 'ready') {
      return `<button type="button" class="cli-maint-btn" data-action="update-one" data-provider="${escapeHtml(status.id)}">${escapeHtml(tx('cli_maintenance_update_btn', '更新'))}</button>`;
    }
    if (state.kind === 'running') {
      return `<span class="cli-maint-btn is-disabled" aria-disabled="true">${escapeHtml(tx('cli_maintenance_update_btn_running', '更新（実行中）'))}</span>`;
    }
    const note = txm(state.note);
    const pill = state.reason === 'running_sessions' && typeof state.note.vars?.count === 'number'
      ? `<span class="cli-maint-pill">${escapeHtml(tx('cli_maintenance_running_pill', '実行中 {count}', { count: state.note.vars.count }))}</span>`
      : '';
    return `<span class="cli-maint-note-inline" title="${escapeHtml(note)}">${escapeHtml(note)}</span>${pill}`;
  }

  function installedRowHtml(status: CliInstallStatus): string {
    const r = rows.get(status.id) || {};
    const name = escapeHtml(status.displayName);
    const icon = providerIconHtml(status.id, 18);
    if (r.jobStatus) return jobRowHtml(status, r);
    const cell = cliVersionCellText(r.version || null, !!r.checking, dow());
    const versionText = cell.kind === 'ok' ? (cell.versionText || '') : (cell.primary ? txm(cell.primary) : '');
    const detailText = cell.detail ? txm(cell.detail) : '';
    const detailHtml = detailText ? `<span class="cli-maint-version-sub" title="${escapeHtml(detailText)}">${escapeHtml(detailText)}</span>` : '';
    const btnState = cliUpdateRowState(r.eligibility || null, false);
    return `<div class="cli-maint-row" data-provider="${escapeHtml(status.id)}">
      <span class="cli-maint-chk" aria-hidden="true">✓</span>
      ${icon}
      <span class="cli-maint-name">${name}</span>
      <span class="cli-maint-version-cell"><span class="cli-maint-version cli-maint-version-${cell.kind}">${escapeHtml(versionText)}</span>${detailHtml}</span>
      <span class="cli-maint-status">${escapeHtml(tx('zero_session_install_status_installed', 'インストール済み'))}</span>
      <span class="cli-maint-actions">${updateButtonHtml(status, btnState)}</span>
    </div>`;
  }

  function stateTagClass(state: string): string {
    switch (state) {
      case 'updated': return 'is-ok';
      case 'running':
      case 'queued': return 'is-accent';
      case 'unknown': return 'is-warn';
      case 'latest': return '';
      default: return 'is-bad';
    }
  }

  const STATE_FALLBACK: Record<string, string> = {
    queued: '待機中', running: '更新中', updated: '更新しました', latest: 'もともと最新',
    unknown: '判定不可', failed: '失敗', file_in_use: '失敗', login_required: '失敗',
  };

  function jobRowHtml(status: CliInstallStatus, r: RowRuntime): string {
    const st = r.jobStatus!;
    const inFlight = IN_FLIGHT_STATES.has(st.state);
    const versionText = (!inFlight && st.version_before && st.version_after && st.version_before !== st.version_after)
      ? `${st.version_before} → ${st.version_after}`
      : (st.version_after || st.version_before || (inFlight ? tx('cli_maintenance_version_checking', '確認中…') : ''));
    const logBtn = (st.log_available && r.jobId)
      ? `<button type="button" class="cli-maint-btn" data-action="show-log" data-provider="${escapeHtml(status.id)}" data-job="${escapeHtml(r.jobId)}">${escapeHtml(tx('cli_maintenance_log_btn', 'ログ'))}</button>`
      : '';
    const failureDetail = cliUpdateFailureDetail(st, status.displayName);
    const detailRow = failureDetail
      ? `<div class="cli-maint-failure-row" data-provider="${escapeHtml(status.id)}">${escapeHtml(txm(failureDetail))}</div>`
      : '';
    return `<div class="cli-maint-row" data-provider="${escapeHtml(status.id)}">
      <span class="cli-maint-chk" aria-hidden="true">${inFlight ? '…' : '✓'}</span>
      ${providerIconHtml(status.id, 18)}
      <span class="cli-maint-name">${escapeHtml(status.displayName)}</span>
      <span class="cli-maint-version-cell"><span class="cli-maint-version">${escapeHtml(versionText)}</span></span>
      <span class="cli-maint-status"><span class="cli-maint-tag ${stateTagClass(st.state)}">${escapeHtml(tx(`cli_maintenance_state_${st.state}`, STATE_FALLBACK[st.state] || st.state))}</span></span>
      <span class="cli-maint-actions">${logBtn}</span>
    </div>${detailRow}`;
  }

  function currentFailingProviderIds(): string[] {
    const ids: string[] = [];
    for (const s of statuses) {
      const r = rows.get(s.id);
      if (r?.jobStatus && FAILING_STATES.has(r.jobStatus.state)) ids.push(s.id);
    }
    return ids;
  }

  function render(): void {
    const eligibleList = statuses
      .map((s) => rows.get(s.id)?.eligibility)
      .filter((e): e is CliUpdateEligibilityEntry => !!e);
    const allCount = cliUpdateEligibleCount(eligibleList);
    const busy = !!activeJob || checkingAll;

    const checkedAtLabel = checkedAt
      ? tx('cli_maintenance_checked_at', '最終確認: {when}', { when: formatCliDateTime(checkedAt, dow()) })
      : tx('cli_maintenance_checked_at_none', 'まだ確認していません');

    const toolbarHtml = activeJob
      ? `<span class="cli-maint-btn is-disabled" aria-disabled="true">${escapeHtml(tx('cli_maintenance_running_progress', '更新中… {done} / {total} 完了', progressVars()))}</span>`
      : `<button type="button" class="cli-maint-btn" data-action="check-all"${busy ? ' disabled' : ''}>${escapeHtml(tx('cli_maintenance_check_versions_btn', 'バージョン確認'))}</button>` +
        `<button type="button" class="cli-maint-btn is-primary" data-action="update-all"${(busy || allCount === 0) ? ' disabled' : ''}>${escapeHtml(tx('cli_maintenance_update_all_btn', '全部更新（{count} 件）', { count: allCount }))}</button>`;

    let summaryHtml = '';
    if (!activeJob && justSettledSummary) {
      const parts = cliUpdateSummaryParts(justSettledSummary);
      const separator = tx('cli_maintenance_summary_separator', '／');
      const line = tx('cli_maintenance_summary_done', '更新が終わりました — {parts}', { parts: parts.map((p) => txm(p)).join(separator) });
      const retryBtn = justSettledSummary.failed > 0
        ? `<button type="button" class="cli-maint-btn is-primary" data-action="retry-failed">${escapeHtml(tx('cli_maintenance_retry_failed_btn', '失敗した分を 1 本ずつやり直す'))}</button>`
        : '';
      summaryHtml = `<div class="cli-maint-summary">${parts.length ? `<span>${escapeHtml(line)}</span>` : ''}${retryBtn}</div>`;
    }

    const rowsHtml = statuses.map((s) => installedRowHtml(s)).join('');

    container.innerHTML = `
      <p class="cli-maint-heading">${escapeHtml(tx('zero_session_install_status_heading', 'このPCでの導入状況'))}</p>
      <div class="cli-maint-toolbar">${toolbarHtml}</div>
      <p class="cli-maint-note">${escapeHtml(checkedAtLabel)}${statuses.length ? ` ・ ${escapeHtml(tx('cli_maintenance_hidden_button_note', '更新ボタンが出ない AI は、設定で更新が OFF です'))}` : ''}</p>
      ${summaryHtml}
      <div class="cli-maint-list">${rowsHtml}</div>
    `;
  }

  // --- 操作 ------------------------------------------------------------------

  async function checkAll(): Promise<void> {
    if (activeJob || checkingAll) return;
    checkingAll = true;
    justSettledSummary = null;
    for (const s of statuses) {
      const r = rows.get(s.id) || {};
      r.checking = true;
      r.jobStatus = undefined;
      r.jobId = undefined;
      rows.set(s.id, r);
    }
    render();
    const result = await fetchCliVersions('POST');
    checkingAll = false;
    if (result) {
      checkedAt = result.checked_at;
      for (const s of statuses) {
        const r = rows.get(s.id);
        if (r) r.checking = false;
      }
      for (const item of result.results) {
        const r = rows.get(item.provider) || {};
        r.version = item;
        r.checking = false;
        rows.set(item.provider, r);
      }
    } else {
      for (const s of statuses) {
        const r = rows.get(s.id);
        if (r) r.checking = false;
      }
    }
    render();
  }

  function showConfirmDialog(mode: 'all' | 'one', entries: CliUpdateEligibilityEntry[]): void {
    const groups = cliUpdatePlanGroups(entries);
    const dialog = document.createElement('dialog');
    // aac-wheel-overlay: 全画面オーバーレイの wheel 除外クラス。無いと document レベルの
    // wheel ハンドラが背後のターミナルへホイールを転送する（scripts/check-wheel-overlays.mjs）。
    dialog.className = 'cli-maint-confirm-dialog aac-wheel-overlay';
    const soleProvider = groups.toUpdate[0]?.provider || entries[0]?.provider || '';
    const title = mode === 'all'
      ? tx('cli_maintenance_confirm_title_all', '{count} 件の AI を更新します', { count: groups.toUpdate.length })
      : tx('cli_maintenance_confirm_title_one', '{name} を更新します', { name: displayNameFor(soleProvider) });
    const toUpdateHtml = groups.toUpdate.length
      ? groups.toUpdate.map((item) => `<div class="cli-maint-confirm-item">${escapeHtml(displayNameFor(item.provider))} — <code>${escapeHtml(item.argv.join(' '))}</code></div>`).join('')
      : `<div class="cli-maint-confirm-item cli-maint-confirm-empty">${escapeHtml(tx('cli_maintenance_confirm_none_to_update', '更新できる AI がありません'))}</div>`;
    const excludedHtml = groups.excluded
      .map((item) => `<div class="cli-maint-confirm-item">${escapeHtml(displayNameFor(item.provider))} — ${escapeHtml(txm(item.note))}</div>`)
      .join('');
    const noteText = mode === 'all'
      ? tx('cli_maintenance_confirm_note', '同時に実行します。1 本が失敗しても残りは続けます。更新中はこれらの AI で新しいセッションを起動できません。出力は ~/.many-ai-cli/logs/cli-updates/ に残ります。')
      : tx('cli_maintenance_confirm_note_one', '更新中はこの AI で新しいセッションを起動できません。出力は ~/.many-ai-cli/logs/cli-updates/ に残ります。');
    dialog.innerHTML = `<div class="cli-maint-confirm-header">
        <h2 class="cli-maint-confirm-title">${escapeHtml(title)}</h2>
        <button type="button" class="cli-maint-btn" data-action="cancel" aria-label="${escapeHtml(tx('settings_close', 'Close'))}">${escapeHtml(tx('settings_close', 'Close'))}</button>
      </div>
      <div class="cli-maint-confirm-body">
        <div class="cli-maint-confirm-group"><strong>${escapeHtml(tx('cli_maintenance_confirm_to_update', '更新する'))}</strong>${toUpdateHtml}</div>
        ${excludedHtml ? `<div class="cli-maint-confirm-group"><strong>${escapeHtml(tx('cli_maintenance_confirm_to_skip', '今回は更新しない'))}</strong>${excludedHtml}</div>` : ''}
        <p class="cli-maint-confirm-note">${escapeHtml(noteText)}</p>
      </div>
      <div class="cli-maint-confirm-actions">
        <button type="button" class="cli-maint-btn" data-action="cancel">${escapeHtml(tx('cli_maintenance_confirm_cancel', 'やめる'))}</button>
        <button type="button" class="cli-maint-btn is-primary" data-action="run"${groups.toUpdate.length === 0 ? ' disabled' : ''}>${escapeHtml(mode === 'all' ? tx('cli_maintenance_confirm_run', '{count} 件を更新', { count: groups.toUpdate.length }) : tx('cli_maintenance_confirm_run_one', '更新'))}</button>
      </div>`;
    const close = (): void => {
      try { dialog.close(); } catch (_) { /* already closed */ }
      dialog.remove();
    };
    dialog.addEventListener('click', (e) => {
      const target = (e.target as HTMLElement)?.closest('[data-action]') as HTMLElement | null;
      if (!target) return;
      const action = target.dataset.action;
      if (action === 'cancel') close();
      else if (action === 'run') {
        close();
        void runUpdate(entries);
      }
    });
    dialog.addEventListener('close', () => dialog.remove(), { once: true });
    document.body.appendChild(dialog);
    dialog.showModal();
  }

  async function openConfirm(mode: 'all' | 'one', providerId?: string): Promise<void> {
    const fresh = await fetchCliUpdateEligibility();
    if (fresh) {
      for (const entry of fresh) {
        const r = rows.get(entry.provider) || {};
        r.eligibility = entry;
        rows.set(entry.provider, r);
      }
      render();
    }
    const entries = mode === 'all'
      ? statuses.map((s) => rows.get(s.id)?.eligibility).filter((e): e is CliUpdateEligibilityEntry => !!e)
      : (() => {
          const entry = providerId ? rows.get(providerId)?.eligibility : undefined;
          return entry ? [entry] : [];
        })();
    showConfirmDialog(mode, entries);
  }

  // runUpdate は実行の直前にもう一度 eligibility を引き直す（画面案 S-02「開いている間に
  // セッションが始まった」: 実行直前に対象を数え直し、外れた AI は今回は更新しないへ移す）。
  async function runUpdate(plannedEntries: CliUpdateEligibilityEntry[]): Promise<void> {
    const plannedIds = new Set(plannedEntries.map((e) => e.provider));
    const fresh = await fetchCliUpdateEligibility();
    const relevant = fresh ? fresh.filter((e) => plannedIds.has(e.provider)) : plannedEntries;
    if (fresh) {
      for (const entry of fresh) {
        const r = rows.get(entry.provider) || {};
        r.eligibility = entry;
        rows.set(entry.provider, r);
      }
    }
    const ids = relevant.filter((e) => e.eligible).map((e) => e.provider);
    if (ids.length === 0) {
      render();
      return;
    }
    const result = await postCliUpdates(ids, false);
    if (!result) {
      render();
      return;
    }
    startJob(result.job_id, false, result.accepted, result.excluded);
  }

  function startJob(jobId: string, serial: boolean, acceptedIds: string[], excluded: CliUpdateExcludedEntry[]): void {
    justSettledSummary = null;
    activeJob = { id: jobId, serial, pending: new Set(acceptedIds), total: acceptedIds.length };
    for (const id of acceptedIds) {
      const r = rows.get(id) || {};
      r.jobStatus = { provider: id, state: 'queued' };
      r.jobId = jobId;
      rows.set(id, r);
    }
    for (const ex of excluded) {
      const r = rows.get(ex.provider) || {};
      r.eligibility = { provider: ex.provider, eligible: false, reason: ex.reason, running_sessions: ex.running_sessions };
      rows.set(ex.provider, r);
    }
    render();
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = setInterval(() => { void pollJob(); }, 1000);
    void pollJob();
  }

  async function pollJob(): Promise<void> {
    if (!activeJob || destroyed) return;
    const jobId = activeJob.id;
    const job = await fetchCliUpdateJob(jobId);
    if (!activeJob || activeJob.id !== jobId || destroyed) return;
    if (!job) return;
    let allDone = true;
    for (const status of job.providers) {
      const r = rows.get(status.provider) || {};
      r.jobStatus = status;
      r.jobId = jobId;
      rows.set(status.provider, r);
      if (IN_FLIGHT_STATES.has(status.state)) {
        allDone = false;
      } else {
        activeJob.pending.delete(status.provider);
      }
    }
    if (allDone) {
      if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
      const finished = job.providers;
      activeJob = null;
      await settleJob(finished);
      return;
    }
    render();
  }

  // settleJob は「更新が終わったら、既存の導入状況（loadProviderSummaries）も読み直す」
  // （親 plan 決まったこと表）の実装本体。バージョン確認・更新可否はここで引き直す。結果
  // タグ（r.jobStatus / r.jobId）は loadVersionsAndEligibility が触らないフィールドなので、
  // 直後に描き直しても消えない — 「終わりました」の行を出したまま最新の状態に更新できる。
  // provider 一覧そのもの（導入状況）は、更新で入れ替わることがあり得る想定を満たすため
  // 読み直すだけ読み直し、結果表示に割り込ませない（await せず投げっぱなし）。
  async function settleJob(finalStatuses: CliUpdateProviderStatus[]): Promise<void> {
    justSettledSummary = cliUpdateSummaryCounts(finalStatuses);
    void loadProviderSummaries({ includeDisabled: true });
    await loadVersionsAndEligibility();
  }

  async function retryFailed(): Promise<void> {
    const ids = currentFailingProviderIds();
    if (!ids.length) return;
    const result = await postCliUpdates(ids, true);
    if (!result) return;
    startJob(result.job_id, true, result.accepted, result.excluded);
  }

  async function showLog(jobId: string, providerId: string): Promise<void> {
    if (!jobId || !providerId) return;
    const dialog = document.createElement('dialog');
    dialog.className = 'cli-maint-log-dialog aac-wheel-overlay';
    const name = displayNameFor(providerId);
    dialog.innerHTML = `<div class="cli-maint-confirm-header">
        <h2 class="cli-maint-confirm-title">${escapeHtml(tx('cli_maintenance_log_title', '{name} の更新ログ', { name }))}</h2>
        <button type="button" class="cli-maint-btn" data-action="close">${escapeHtml(tx('cli_maintenance_log_close', '閉じる'))}</button>
      </div>
      <div class="cli-maint-log-body"><pre class="cli-maint-log-pre"></pre></div>`;
    const pre = dialog.querySelector('.cli-maint-log-pre') as HTMLElement | null;
    if (pre) pre.textContent = tx('cli_maintenance_version_checking', '確認中…');
    const close = (): void => {
      try { dialog.close(); } catch (_) { /* already closed */ }
      dialog.remove();
    };
    dialog.addEventListener('click', (e) => {
      if ((e.target as HTMLElement)?.closest('[data-action="close"]')) close();
    });
    dialog.addEventListener('close', () => dialog.remove(), { once: true });
    document.body.appendChild(dialog);
    dialog.showModal();
    const log = await fetchCliUpdateLog(jobId, providerId);
    if (!pre) return;
    if (!log) {
      pre.textContent = tx('cli_maintenance_log_unavailable', 'ログがありません');
      return;
    }
    pre.textContent = log.truncated
      ? `${log.content}\n\n[${tx('cli_maintenance_log_truncated', 'ログが大きいため一部だけ表示しています。')}]`
      : log.content;
  }

  container.addEventListener('click', (e) => {
    const target = (e.target as HTMLElement)?.closest('[data-action]') as HTMLElement | null;
    if (!target) return;
    const action = target.dataset.action;
    if (action === 'check-all') void checkAll();
    else if (action === 'update-all') void openConfirm('all');
    else if (action === 'update-one') void openConfirm('one', target.dataset.provider || '');
    else if (action === 'show-log') void showLog(target.dataset.job || '', target.dataset.provider || '');
    else if (action === 'retry-failed') void retryFailed();
  });

  render();
  void loadVersionsAndEligibility();

  return {
    refresh,
    destroy: () => {
      destroyed = true;
      if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
    },
  };
}
