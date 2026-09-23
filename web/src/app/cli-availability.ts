// PATH 上に公式 CLI があるかどうか。空画面の案内と spawn の起動ボタンが
// 同じ判定を使う。表示名と実行ファイル名は provider 定義（id / display_name）
// から組み立て、案内専用の一覧は持たない。

export const SHELL_PROVIDER_ID = 'shell';
export const ADD_PROVIDER_OPTION_VALUE = '__add-ai-provider__';

export type CliGuideEntry = {
  id: string;
  displayName: string;
};

export type SpawnLaunchBlock = 'cwd-empty' | 'cwd-missing' | 'cli-missing' | null;

export function isAiLaunchProvider(id: string): boolean {
  return !!id && id !== SHELL_PROVIDER_ID && id !== ADD_PROVIDER_OPTION_VALUE;
}

export function commandMissingIds(diagnostics: { code?: string; field?: string }[] | null | undefined): Set<string> {
  const ids = new Set<string>();
  if (!diagnostics) return ids;
  for (const diagnostic of diagnostics) {
    if (diagnostic?.code !== 'command_missing') continue;
    const field = String(diagnostic.field || '').trim();
    if (field) ids.add(field);
  }
  return ids;
}

export function aiCliGuideEntries(providers: { id: string; display_name?: string; enabled?: boolean }[]): CliGuideEntry[] {
  return providers
    .filter((provider) => provider.enabled !== false && isAiLaunchProvider(provider.id))
    .map((provider) => ({
      id: provider.id,
      displayName: (provider.display_name && provider.display_name.trim()) || provider.id,
    }));
}

export type CliInstallStatus = {
  id: string;
  displayName: string;
  installed: boolean;
  installUrl?: string;
};

// cliInstallStatuses は初回画面の導入状況一覧が使う。aiCliGuideEntries と同じ
// 除外条件（無効 provider / shell / 追加行を除く）で並べ、PATH 上の有無
// （missingIds）と公式手順の URL（installLinks）を突き合わせる。installUrl は
// https:// で始まる値だけを通す（Hub 側の sanitize と同じ最終防御を画面側にも置く）。
export function cliInstallStatuses(
  providers: { id: string; display_name?: string; enabled?: boolean }[],
  missingIds: Set<string>,
  installLinks: Record<string, string> | null | undefined,
): CliInstallStatus[] {
  return aiCliGuideEntries(providers).map((entry) => {
    const installed = !missingIds.has(entry.id);
    const rawUrl = installLinks ? installLinks[entry.id] : undefined;
    const status: CliInstallStatus = { id: entry.id, displayName: entry.displayName, installed };
    if (typeof rawUrl === 'string' && rawUrl.startsWith('https://')) {
      status.installUrl = rawUrl;
    }
    return status;
  });
}

export function hasAvailableAiCli(
  providers: { id: string; enabled?: boolean }[],
  missingIds: Set<string>,
): boolean {
  const enabledAi = providers.filter((provider) => (
    provider.enabled !== false && isAiLaunchProvider(provider.id)
  ));
  // 有効な AI が 1 本も無い（全部オフ）ときは入れ方案内を出さない。
  if (enabledAi.length === 0) return true;
  return enabledAi.some((provider) => !missingIds.has(provider.id));
}

export function spawnLaunchBlockReason(input: {
  cwd: string;
  cwdMissing: boolean;
  providerId: string;
  commandMissing: boolean;
}): SpawnLaunchBlock {
  if (!input.cwd.trim()) return 'cwd-empty';
  if (input.cwdMissing) return 'cwd-missing';
  if (input.commandMissing && isAiLaunchProvider(input.providerId)) return 'cli-missing';
  return null;
}

// ---- CLI バージョン確認・更新（導入状況一覧の共通部品。cli-maintenance.ts が使う） ----
//
// ここは DOM に触らない純粋関数だけを置く（子 plan C1: 「DOM に触らない」）。表示文言は
// i18n キーを直接 t() で引かず、{ key, fallback, vars } を返して呼び出し側の tx() 相当に
// 委ねる（provider-manager-view.ts の providerErrorMessage と同じ形）。

export type I18nMessage = { key: string; fallback: string; vars?: Record<string, unknown> };

// Go 側 cliVersionResult のミラー（internal/hub/cli_version.go）。
export type CliVersionResult = {
  provider: string;
  executable?: string;
  version_line?: string;
  version_text?: string;
  executable_modified_at?: string;
  exit_code: number;
  error?: string;
};

// Go 側 cliUpdateEligibilityEntry のミラー（internal/hub/cli_update.go）。
export type CliUpdateEligibilityEntry = {
  provider: string;
  eligible: boolean;
  reason?: string;
  running_sessions?: number;
  executable?: string;
  argv?: string[];
};

// Go 側 cliUpdateProviderStatus のミラー。state は queued/running/updated/latest/
// unknown/failed/file_in_use/login_required のいずれか。
export type CliUpdateProviderStatus = {
  provider: string;
  state: string;
  executable?: string;
  argv?: string[];
  version_before?: string;
  version_after?: string;
  exit_code?: number;
  started_at?: string;
  finished_at?: string;
  log_available?: boolean;
};

// Go 側 cliUpdateExcludedEntry のミラー。reason は CliUpdateEligibilityEntry と同じ
// 6 種の語彙を共有する。
export type CliUpdateExcludedEntry = {
  provider: string;
  reason: string;
  running_sessions?: number;
};

// formatCliDateTime は CLAUDE/coding.md の日時形式（"2026-05-06(水) 10:38:55"）へ ISO
// 文字列を変換する。Date.toLocaleString はロケール依存のため使わない。dow は i18n の
// 'dow' キー（日本語の曜日配列）をそのまま渡す — util.ts の formatLastOutputAt と同じ
// 元ネタをここでも使う（DOM に触らないここへ i18n 依存を持ち込まないため、配列で受ける）。
export function formatCliDateTime(isoStr: string | null | undefined, dow: string[]): string {
  if (!isoStr) return '';
  const d = new Date(isoStr);
  if (isNaN(d.getTime())) return '';
  const p = (n: number) => String(n).padStart(2, '0');
  const day = dow[d.getDay()] || '';
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}(${day}) ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

export type CliVersionCellKind = 'unchecked' | 'checking' | 'ok' | 'error';

// primary は unchecked/checking/error のときだけ使う（未翻訳の定型文）。ok のときは
// versionText（CLI 自身の出力そのまま・翻訳しない）を使う — 空文字列の i18n key を
// 「翻訳なしでそのまま出す」意味に流用すると t() の未ヒット判定と衝突しかねないため、
// 別フィールドに分けている。
export type CliVersionCell = {
  kind: CliVersionCellKind;
  primary?: I18nMessage;
  versionText?: string;
  detail?: I18nMessage;
};

function cliVersionErrorDetail(result: CliVersionResult): I18nMessage {
  const m = /^終了コード (-?\d+)$/.exec(result.error || '');
  if (m) {
    return { key: 'cli_maintenance_version_error_exit_code', fallback: '--version が終了コード {code} を返しました', vars: { code: m[1] } };
  }
  switch (result.error) {
    case '見つからない':
      return { key: 'cli_maintenance_version_error_not_found', fallback: '実行ファイルが見つかりません' };
    case '打ち切り':
      return { key: 'cli_maintenance_version_error_timeout', fallback: '応答が無いため打ち切りました' };
    case '出力が空':
      return { key: 'cli_maintenance_version_error_empty', fallback: '出力が空でした' };
    default:
      return { key: 'cli_maintenance_version_error_generic', fallback: '確認に失敗しました' };
  }
}

// cliVersionCellText はバージョン欄の文言を決める（画面案 S-01「表示のきまり」節の
// 4 状態: 未確認 / 確認中 / 確認済み / 取得失敗）。checking を最優先に見る。dow は
// formatCliDateTime にそのまま渡す曜日配列（呼び出し側が t('dow') から作る）。
export function cliVersionCellText(result: CliVersionResult | null | undefined, checking: boolean, dow: string[]): CliVersionCell {
  if (checking) {
    return { kind: 'checking', primary: { key: 'cli_maintenance_version_checking', fallback: '確認中…' } };
  }
  if (!result) {
    return {
      kind: 'unchecked',
      primary: { key: 'cli_maintenance_version_unchecked', fallback: '[未確認]' },
      detail: { key: 'cli_maintenance_version_unchecked_hint', fallback: '「バージョン確認」で取得します' },
    };
  }
  if (result.error) {
    return { kind: 'error', primary: { key: 'cli_maintenance_version_failed', fallback: '[取得失敗]' }, detail: cliVersionErrorDetail(result) };
  }
  const versionText = result.version_line || result.version_text || '';
  const detail = result.executable_modified_at
    ? {
        key: 'cli_maintenance_executable_modified_at',
        fallback: '実行ファイルの更新日: {date}',
        vars: { date: formatCliDateTime(result.executable_modified_at, dow) },
      }
    : undefined;
  return { kind: 'ok', versionText, detail };
}

export type CliUpdateButtonState =
  | { kind: 'ready' }
  | { kind: 'running' }
  | { kind: 'disabled'; reason: string; note: I18nMessage };

// cliUpdateDisabledNote は「更新できない 6 つの理由」それぞれに別の文言を割り当てる
// （子 plan C1 完了条件）。POST /api/cli-updates の excluded エントリと GET
// /api/cli-update-eligibility の非対象エントリは reason の語彙を共有するので、どちらの
// 入力でも使える形（reason/running_sessions だけの構造的部分型）にしてある。
export function cliUpdateDisabledNote(entry: { reason?: string; running_sessions?: number }): I18nMessage {
  switch (entry.reason) {
    case 'not_installed':
      return { key: 'cli_maintenance_update_reason_not_installed', fallback: '未インストールです' };
    case 'update_disabled':
      return { key: 'cli_maintenance_update_reason_disabled', fallback: '更新 OFF（設定で更新をオンにできます）' };
    case 'update_not_configured':
      return { key: 'cli_maintenance_update_reason_not_configured', fallback: '更新コマンドが未設定です' };
    case 'running_sessions':
      return {
        key: 'cli_maintenance_update_reason_running_sessions',
        fallback: '実行中のセッションが {count} 件あります',
        vars: { count: entry.running_sessions || 0 },
      };
    case 'already_updating':
      return { key: 'cli_maintenance_update_reason_already_updating', fallback: '更新を実行中です' };
    case 'login_may_be_required':
      return { key: 'cli_maintenance_update_reason_login', fallback: '更新 OFF（ログインが要るため）' };
    default:
      return { key: 'cli_maintenance_update_reason_unknown', fallback: '更新できません' };
  }
}

// cliUpdateRowState は行の更新ボタンの状態を決める。jobRunning は「今まさにこの
// provider を更新中のジョブが、いま見ている一覧上で進行中」を指す（他クライアント発の
// already_updating は eligibility 側の reason で表現される）。
export function cliUpdateRowState(entry: CliUpdateEligibilityEntry | null | undefined, jobRunning: boolean): CliUpdateButtonState {
  if (jobRunning) return { kind: 'running' };
  if (!entry) return { kind: 'disabled', reason: 'unknown', note: cliUpdateDisabledNote({}) };
  if (entry.eligible) return { kind: 'ready' };
  return { kind: 'disabled', reason: entry.reason || 'unknown', note: cliUpdateDisabledNote(entry) };
}

// cliUpdateEligibleCount は「全部更新（N 件）」の N。対象外（6 理由のいずれか）は
// 数えない（子 plan C1 完了条件）。
export function cliUpdateEligibleCount(entries: CliUpdateEligibilityEntry[]): number {
  return entries.filter((entry) => entry.eligible).length;
}

export type CliUpdatePlanGroup = {
  toUpdate: { provider: string; argv: string[] }[];
  excluded: { provider: string; note: I18nMessage }[];
};

// cliUpdatePlanGroups は確認ダイアログ（S-02）の「更新する」/「今回は更新しない」の
// 2 群を組み立てる。
export function cliUpdatePlanGroups(entries: CliUpdateEligibilityEntry[]): CliUpdatePlanGroup {
  const toUpdate: CliUpdatePlanGroup['toUpdate'] = [];
  const excluded: CliUpdatePlanGroup['excluded'] = [];
  for (const entry of entries) {
    if (entry.eligible) {
      toUpdate.push({ provider: entry.provider, argv: entry.argv || [] });
    } else {
      excluded.push({ provider: entry.provider, note: cliUpdateDisabledNote(entry) });
    }
  }
  return { toUpdate, excluded };
}

export type CliUpdateSummaryCounts = { updated: number; latest: number; failed: number };

// cliUpdateSummaryCounts は終了後のまとめ行の内訳。queued/running（未完了）は数えない。
// unknown/failed/file_in_use/login_required はすべて「失敗」にまとめる。
export function cliUpdateSummaryCounts(statuses: CliUpdateProviderStatus[]): CliUpdateSummaryCounts {
  let updated = 0;
  let latest = 0;
  let failed = 0;
  for (const status of statuses) {
    switch (status.state) {
      case 'updated':
        updated += 1;
        break;
      case 'latest':
        latest += 1;
        break;
      case 'queued':
      case 'running':
        break;
      default:
        failed += 1;
    }
  }
  return { updated, latest, failed };
}

// cliUpdateSummaryParts はまとめ行を組み立てる部品の配列。0 件の区分は出さない
// （子 plan C1 完了条件）。呼び出し側が区切り文字で join する。
export function cliUpdateSummaryParts(counts: CliUpdateSummaryCounts): I18nMessage[] {
  const parts: I18nMessage[] = [];
  if (counts.updated > 0) parts.push({ key: 'cli_maintenance_summary_updated', fallback: '更新 {count}', vars: { count: counts.updated } });
  if (counts.latest > 0) parts.push({ key: 'cli_maintenance_summary_latest', fallback: 'もともと最新 {count}', vars: { count: counts.latest } });
  if (counts.failed > 0) parts.push({ key: 'cli_maintenance_summary_failed', fallback: '失敗 {count}', vars: { count: counts.failed } });
  return parts;
}

// cliUpdateFailureDetail は失敗した行の下に出す 1 行（画面案 S-03）。使用中は確定文言、
// ログイン要求はターミナル案内、それ以外は終了コードを出す。成功系の state には null。
export function cliUpdateFailureDetail(status: CliUpdateProviderStatus, displayName: string): I18nMessage | null {
  switch (status.state) {
    case 'file_in_use':
      return {
        key: 'cli_maintenance_failure_file_in_use',
        fallback: '{name} を更新できませんでした。{name} の実行ファイルが使用中で、書き換えられません。{name} を使っているターミナルやエディタの拡張機能をすべて閉じてから、もう一度「更新」を押してください。many-ai-cli の外で動いている {name} も対象です。',
        vars: { name: displayName },
      };
    case 'login_required':
      return {
        key: 'cli_maintenance_failure_login_required',
        fallback: 'ログインが必要です。ターミナルで {command} login を実行してください。',
        vars: { command: status.executable || displayName },
      };
    case 'failed':
      return { key: 'cli_maintenance_failure_generic', fallback: '終了コード {code} で失敗しました。', vars: { code: status.exit_code ?? '' } };
    case 'unknown':
      return { key: 'cli_maintenance_failure_unknown', fallback: '前後のバージョンを比較できませんでした。' };
    default:
      return null;
  }
}
