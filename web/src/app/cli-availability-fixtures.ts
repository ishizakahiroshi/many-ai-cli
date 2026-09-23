import assert from 'node:assert/strict';
import test from 'node:test';
import {
  aiCliGuideEntries,
  cliInstallStatuses,
  cliUpdateDisabledNote,
  cliUpdateEligibleCount,
  cliUpdateFailureDetail,
  cliUpdatePlanGroups,
  cliUpdateRowState,
  cliUpdateSummaryCounts,
  cliUpdateSummaryParts,
  cliVersionCellText,
  commandMissingIds,
  formatCliDateTime,
  hasAvailableAiCli,
  isAiLaunchProvider,
  spawnLaunchBlockReason,
  type CliUpdateEligibilityEntry,
  type CliUpdateProviderStatus,
} from './cli-availability.js';

test('isAiLaunchProvider: shell と追加行は AI ではない', () => {
  assert.equal(isAiLaunchProvider('claude'), true);
  assert.equal(isAiLaunchProvider('my-cli'), true);
  assert.equal(isAiLaunchProvider('shell'), false);
  assert.equal(isAiLaunchProvider('__add-ai-provider__'), false);
  assert.equal(isAiLaunchProvider(''), false);
});

test('commandMissingIds: command_missing の field だけを集める', () => {
  const ids = commandMissingIds([
    { code: 'command_missing', field: 'claude' },
    { code: 'invalid_definition', field: 'codex' },
    { code: 'command_missing', field: 'grok' },
    { code: 'command_missing', field: '' },
  ]);
  assert.deepEqual([...ids].sort(), ['claude', 'grok']);
  assert.equal(commandMissingIds(null).size, 0);
  assert.equal(commandMissingIds(undefined).size, 0);
});

test('aiCliGuideEntries: 有効な AI だけ、表示名は provider 定義', () => {
  const entries = aiCliGuideEntries([
    { id: 'claude', display_name: 'Claude', enabled: true },
    { id: 'grok', display_name: 'Grok Build', enabled: true },
    { id: 'codex', display_name: 'Codex', enabled: false },
    { id: 'shell', display_name: 'Shell', enabled: true },
    { id: 'custom', display_name: '', enabled: true },
  ]);
  assert.deepEqual(entries, [
    { id: 'claude', displayName: 'Claude' },
    { id: 'grok', displayName: 'Grok Build' },
    { id: 'custom', displayName: 'custom' },
  ]);
});

test('hasAvailableAiCli: 有効な AI が 1 本でも PATH にあれば true', () => {
  const providers = [
    { id: 'claude', enabled: true },
    { id: 'grok', enabled: true },
    { id: 'shell', enabled: true },
  ];
  assert.equal(hasAvailableAiCli(providers, new Set(['claude', 'grok'])), false);
  assert.equal(hasAvailableAiCli(providers, new Set(['claude'])), true);
  assert.equal(hasAvailableAiCli(providers, new Set()), true);
});

test('hasAvailableAiCli: 有効な AI が 0 本なら案内を出さない', () => {
  assert.equal(hasAvailableAiCli([{ id: 'claude', enabled: false }, { id: 'shell', enabled: true }], new Set(['claude'])), true);
  assert.equal(hasAvailableAiCli([], new Set(['claude'])), true);
});

test('cliInstallStatuses: 導入済み・未導入+リンクあり・未導入+リンク無し・無効 provider は出ない・http は捨てる', () => {
  const providers = [
    { id: 'claude', display_name: 'Claude', enabled: true },
    { id: 'grok', display_name: 'Grok Build', enabled: true },
    { id: 'command-code', display_name: 'Command Code', enabled: true },
    { id: 'codex', display_name: 'Codex', enabled: false },
    { id: 'shell', display_name: 'Shell', enabled: true },
  ];
  const missingIds = new Set(['grok', 'command-code']);
  const installLinks = {
    grok: 'https://example.com/grok-install',
    // command-code はリンク無し
    codex: 'https://example.com/codex-install', // 無効 provider なので出ない
    claude: 'http://example.com/insecure', // http は捨てる
  };
  const statuses = cliInstallStatuses(providers, missingIds, installLinks);
  assert.deepEqual(statuses, [
    { id: 'claude', displayName: 'Claude', installed: true },
    { id: 'grok', displayName: 'Grok Build', installed: false, installUrl: 'https://example.com/grok-install' },
    { id: 'command-code', displayName: 'Command Code', installed: false },
  ]);
});

test('cliInstallStatuses: installLinks が null/undefined でも落ちない', () => {
  const providers = [{ id: 'claude', display_name: 'Claude', enabled: true }];
  assert.deepEqual(cliInstallStatuses(providers, new Set(['claude']), null), [
    { id: 'claude', displayName: 'Claude', installed: false },
  ]);
  assert.deepEqual(cliInstallStatuses(providers, new Set(), undefined), [
    { id: 'claude', displayName: 'Claude', installed: true },
  ]);
});

test('spawnLaunchBlockReason: cwd 欠けが先、次に CLI 未検出', () => {
  assert.equal(spawnLaunchBlockReason({
    cwd: '',
    cwdMissing: false,
    providerId: 'claude',
    commandMissing: true,
  }), 'cwd-empty');
  assert.equal(spawnLaunchBlockReason({
    cwd: 'C:\\dev\\app',
    cwdMissing: true,
    providerId: 'claude',
    commandMissing: true,
  }), 'cwd-missing');
  assert.equal(spawnLaunchBlockReason({
    cwd: 'C:\\dev\\app',
    cwdMissing: false,
    providerId: 'claude',
    commandMissing: true,
  }), 'cli-missing');
  assert.equal(spawnLaunchBlockReason({
    cwd: 'C:\\dev\\app',
    cwdMissing: false,
    providerId: 'shell',
    commandMissing: true,
  }), null);
  assert.equal(spawnLaunchBlockReason({
    cwd: 'C:\\dev\\app',
    cwdMissing: false,
    providerId: 'claude',
    commandMissing: false,
  }), null);
});

const DOW_JA = ['日', '月', '火', '水', '木', '金', '土'];

test('formatCliDateTime: CLAUDE/coding.md の日時形式へ整形する', () => {
  assert.equal(formatCliDateTime('2026-05-06T10:38:55+09:00', DOW_JA), '2026-05-06(水) 10:38:55');
  assert.equal(formatCliDateTime('', DOW_JA), '');
  assert.equal(formatCliDateTime(undefined, DOW_JA), '');
  assert.equal(formatCliDateTime('not-a-date', DOW_JA), '');
});

test('cliVersionCellText: 未確認・確認中・確認済み・取得失敗の4状態', () => {
  assert.equal(cliVersionCellText(null, false, DOW_JA).kind, 'unchecked');
  assert.equal(cliVersionCellText(null, true, DOW_JA).kind, 'checking');
  const ok = cliVersionCellText({ provider: 'claude', version_line: 'claude 2.3.14', exit_code: 0 }, false, DOW_JA);
  assert.equal(ok.kind, 'ok');
  assert.equal(ok.versionText, 'claude 2.3.14');
  const err = cliVersionCellText({ provider: 'claude', exit_code: 1, error: '終了コード 1' }, false, DOW_JA);
  assert.equal(err.kind, 'error');
  assert.equal(err.detail?.key, 'cli_maintenance_version_error_exit_code');
  assert.equal(err.detail?.vars?.code, '1');
});

test('cliUpdateDisabledNote: 6つの対象外理由がそれぞれ別の表示になる', () => {
  const reasons = [
    'not_installed',
    'update_disabled',
    'update_not_configured',
    'running_sessions',
    'already_updating',
    'login_may_be_required',
  ];
  const keys = reasons.map((reason) => cliUpdateDisabledNote({ reason, running_sessions: 2 }).key);
  assert.deepEqual(new Set(keys).size, keys.length, 'reason ごとに異なる key を返す');
  assert.equal(cliUpdateDisabledNote({ reason: 'running_sessions', running_sessions: 3 }).vars?.count, 3);
  // 未知の reason（サーバーが増やしても UI が黙って崩れない）
  assert.equal(cliUpdateDisabledNote({ reason: 'something_new' }).key, 'cli_maintenance_update_reason_unknown');
});

test('cliUpdateRowState: running > disabled(reason) > ready の優先順位', () => {
  const eligible: CliUpdateEligibilityEntry = { provider: 'claude', eligible: true };
  const ineligible: CliUpdateEligibilityEntry = { provider: 'codex', eligible: false, reason: 'running_sessions', running_sessions: 2 };
  assert.deepEqual(cliUpdateRowState(eligible, false), { kind: 'ready' });
  assert.deepEqual(cliUpdateRowState(eligible, true), { kind: 'running' });
  const disabled = cliUpdateRowState(ineligible, false);
  assert.equal(disabled.kind, 'disabled');
  assert.equal((disabled as { reason: string }).reason, 'running_sessions');
});

test('cliUpdateEligibleCount: 「全部更新」の N は対象外を数えない', () => {
  const entries: CliUpdateEligibilityEntry[] = [
    { provider: 'claude', eligible: true },
    { provider: 'codex', eligible: false, reason: 'running_sessions', running_sessions: 2 },
    { provider: 'copilot', eligible: true },
    { provider: 'cursor-agent', eligible: false, reason: 'login_may_be_required' },
  ];
  assert.equal(cliUpdateEligibleCount(entries), 2);
});

test('cliUpdatePlanGroups: eligible/非eligible を2群へ振り分ける', () => {
  const entries: CliUpdateEligibilityEntry[] = [
    { provider: 'claude', eligible: true, argv: ['claude', 'update'] },
    { provider: 'codex', eligible: false, reason: 'running_sessions', running_sessions: 2 },
  ];
  const groups = cliUpdatePlanGroups(entries);
  assert.deepEqual(groups.toUpdate, [{ provider: 'claude', argv: ['claude', 'update'] }]);
  assert.equal(groups.excluded.length, 1);
  assert.equal(groups.excluded[0].provider, 'codex');
  assert.equal(groups.excluded[0].note.key, 'cli_maintenance_update_reason_running_sessions');
});

test('cliUpdateSummaryCounts/Parts: 0件の区分は出さない', () => {
  const statuses: CliUpdateProviderStatus[] = [
    { provider: 'claude', state: 'updated' },
    { provider: 'copilot', state: 'latest' },
    { provider: 'opencode', state: 'failed' },
    { provider: 'grok', state: 'file_in_use' },
  ];
  const counts = cliUpdateSummaryCounts(statuses);
  assert.deepEqual(counts, { updated: 1, latest: 1, failed: 2 });
  const parts = cliUpdateSummaryParts(counts);
  assert.deepEqual(parts.map((p) => p.key), ['cli_maintenance_summary_updated', 'cli_maintenance_summary_latest', 'cli_maintenance_summary_failed']);

  const allUpdated = cliUpdateSummaryCounts([{ provider: 'claude', state: 'updated' }]);
  assert.deepEqual(cliUpdateSummaryParts(allUpdated).map((p) => p.key), ['cli_maintenance_summary_updated']);

  const queuedOnly = cliUpdateSummaryCounts([{ provider: 'claude', state: 'queued' }, { provider: 'codex', state: 'running' }]);
  assert.deepEqual(queuedOnly, { updated: 0, latest: 0, failed: 0 });
  assert.deepEqual(cliUpdateSummaryParts(queuedOnly), []);
});

test('cliUpdateFailureDetail: 使用中・ログイン要求・一般失敗・判定不可でそれぞれ別文言', () => {
  assert.equal(cliUpdateFailureDetail({ provider: 'grok', state: 'file_in_use' }, 'Grok Build').key, 'cli_maintenance_failure_file_in_use');
  const login = cliUpdateFailureDetail({ provider: 'claude', state: 'login_required', executable: 'claude' }, 'Claude');
  assert.equal(login?.key, 'cli_maintenance_failure_login_required');
  assert.equal(login?.vars?.command, 'claude');
  assert.equal(cliUpdateFailureDetail({ provider: 'opencode', state: 'failed', exit_code: 1 }, 'OpenCode').key, 'cli_maintenance_failure_generic');
  assert.equal(cliUpdateFailureDetail({ provider: 'opencode', state: 'unknown' }, 'OpenCode').key, 'cli_maintenance_failure_unknown');
  assert.equal(cliUpdateFailureDetail({ provider: 'claude', state: 'updated' }, 'Claude'), null);
  assert.equal(cliUpdateFailureDetail({ provider: 'claude', state: 'latest' }, 'Claude'), null);
});
