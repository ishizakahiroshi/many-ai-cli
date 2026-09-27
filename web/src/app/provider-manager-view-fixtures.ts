import assert from 'node:assert/strict';
import test from 'node:test';
import {
  createRequestGuard,
  formatDiagnosticMessages,
  isBuiltinProviderID,
  originLabelKey,
  providerErrorMessage,
  providerStatusKind,
  providerUpdateFormValuesFromUpdate,
  providerUpdateFromFormValues,
  providerUpdateIsDefault,
  providerUpdateLoginMayBeRequired,
  statusLabelKey,
  suggestProviderId,
  type ProviderUpdateFormValues,
} from './provider-manager-view.js';

test('suggestProviderId: 表示名から小文字スラッグを作る', () => {
  assert.equal(suggestProviderId('My CLI'), 'my-cli');
  assert.equal(suggestProviderId('  Example Agent  '), 'example-agent');
  assert.equal(suggestProviderId(''), '');
});

test('suggestProviderId: 予約IDは -cli を付けて避ける', () => {
  assert.equal(suggestProviderId('Claude'), 'claude-cli');
  assert.equal(suggestProviderId('shell'), 'shell-cli');
});

test('originLabelKey: 出所ごとに固定キーを返す', () => {
  assert.equal(originLabelKey('embedded'), 'settings_ai_providers_origin_embedded');
  assert.equal(originLabelKey('distribution'), 'settings_ai_providers_origin_distribution');
  assert.equal(originLabelKey('user'), 'settings_ai_providers_origin_user');
  assert.equal(originLabelKey('legacy'), 'settings_ai_providers_origin_legacy');
  assert.equal(originLabelKey('override'), 'settings_ai_providers_origin_override');
  assert.equal(originLabelKey('other'), 'settings_ai_providers_origin_unknown');
});

test('providerStatusKind: 無効と定義エラーは色以外の種別でも区別する', () => {
  assert.equal(providerStatusKind({ enabled: true }), 'available');
  assert.equal(providerStatusKind({ enabled: false }), 'disabled');
  assert.equal(providerStatusKind({ enabled: true, hasError: true }), 'error');
  assert.equal(statusLabelKey('available'), 'settings_ai_providers_status_available');
  assert.equal(statusLabelKey('disabled'), 'settings_ai_providers_status_disabled');
  assert.equal(statusLabelKey('error'), 'settings_ai_providers_status_error');
});

test('providerStatusKind: command missing は disabled や error より弱い', () => {
  assert.equal(providerStatusKind({ enabled: true, commandMissing: true }), 'missing');
  // 無効化されていれば「コマンドが無い」より「無効」の方が実用的な情報。
  assert.equal(providerStatusKind({ enabled: false, commandMissing: true }), 'disabled');
  // 定義エラーは常に最優先。
  assert.equal(providerStatusKind({ enabled: true, commandMissing: true, hasError: true }), 'error');
  assert.equal(statusLabelKey('missing'), 'settings_ai_providers_status_missing');
});

test('isBuiltinProviderID: 組み込み7種のみ true、shell と custom は false', () => {
  assert.equal(isBuiltinProviderID('claude'), true);
  assert.equal(isBuiltinProviderID('codex'), true);
  assert.equal(isBuiltinProviderID('command-code'), true);
  assert.equal(isBuiltinProviderID('shell'), false);
  assert.equal(isBuiltinProviderID('my-custom-cli'), false);
});

test('providerErrorMessage: offline / 409 / 5xx / その他 で別キーを返す', () => {
  assert.equal(providerErrorMessage({ kind: 'network' }).key, 'settings_ai_providers_offline');
  assert.equal(providerErrorMessage({ kind: 'http', status: 409 }).key, 'settings_ai_providers_revision_conflict');
  const serverError = providerErrorMessage({ kind: 'http', status: 503 });
  assert.equal(serverError.key, 'settings_ai_providers_server_error');
  assert.equal(serverError.vars.status, 503);
  const other = providerErrorMessage({ kind: 'http', status: 400 });
  assert.equal(other.key, 'settings_ai_providers_request_failed');
  assert.equal(other.vars.status, 400);
});

test('providerErrorMessage: 空にできない項目の拒否は、項目名を差し込んだ文にする', () => {
  const message = providerErrorMessage({
    kind: 'http',
    status: 422,
    code: 'provider_override_clears_value',
    detail: 'provider override cannot clear a distributed value: launch.model_args, update.args',
    fields: ['launch.model_args', 'update.args'],
  });
  assert.equal(message.key, 'settings_ai_providers_cannot_clear');
  assert.equal(message.vars.fields, 'launch.model_args, update.args');
});

test('providerErrorMessage: 理由付きの 4xx はサーバーの理由を文に入れる', () => {
  const detail = 'override payload is invalid: update.enabled: update.enabled is true but update.args is empty';
  const message = providerErrorMessage({ kind: 'http', status: 422, code: 'provider_override_failed', detail });
  assert.equal(message.key, 'settings_ai_providers_request_failed_detail');
  assert.equal(message.vars.detail, detail);
  assert.equal(message.vars.status, 422);
});

test('providerErrorMessage: 拒否の code でも項目名が無ければ理由付きの文へ落ちる', () => {
  const message = providerErrorMessage({
    kind: 'http',
    status: 422,
    code: 'provider_override_clears_value',
    detail: 'provider override cannot clear a distributed value: ',
    fields: [],
  });
  assert.equal(message.key, 'settings_ai_providers_request_failed_detail');
  assert.equal(message.vars.status, 422);
});

test('createRequestGuard: 後から begin した token だけが isCurrent になる', () => {
  const guard = createRequestGuard();
  const first = guard.begin();
  assert.equal(guard.isCurrent(first), true);
  const second = guard.begin();
  // A（first）がまだ解決していない間に B（second）が始まったら、A の
  // 遅延応答が届いても isCurrent は false のまま＝B のフォームを上書きしない。
  assert.equal(guard.isCurrent(first), false);
  assert.equal(guard.isCurrent(second), true);
});

test('formatDiagnosticMessages: 空メッセージを捨てて error 有無を返す', () => {
  const formatted = formatDiagnosticMessages([
    { severity: 'warning', message: 'unknown field is ignored' },
    { severity: 'error', message: '  id is required  ' },
    { severity: 'error', message: '' },
  ]);
  assert.equal(formatted.hasError, true);
  assert.equal(formatted.text, 'unknown field is ignored id is required');
});

test('providerUpdateFormValuesFromUpdate/providerUpdateFromFormValues: 空白区切りの往復変換', () => {
  const values = providerUpdateFormValuesFromUpdate({ version_args: ['--version'], args: ['update', '--yes'], executable: 'claude', enabled: true, timeout_seconds: 120 });
  assert.equal(values.versionArgs, '--version');
  assert.equal(values.args, 'update --yes');
  assert.equal(values.executable, 'claude');
  assert.equal(values.enabled, true);
  assert.equal(values.timeoutSeconds, '120');
  const result = providerUpdateFromFormValues(values);
  assert.equal(result.ok, true);
  if (result.ok) {
    assert.deepEqual(result.update.version_args, ['--version']);
    assert.deepEqual(result.update.args, ['update', '--yes']);
    assert.equal(result.update.executable, 'claude');
    assert.equal(result.update.enabled, true);
    assert.equal(result.update.timeout_seconds, 120);
  }
});

test('providerUpdateFormValuesFromUpdate: update 無しの定義は「更新しない・引数空」で読める', () => {
  const fromUndefined = providerUpdateFormValuesFromUpdate(undefined);
  assert.equal(fromUndefined.enabled, false);
  assert.equal(fromUndefined.versionArgs, '');
  assert.equal(fromUndefined.args, '');
  assert.equal(fromUndefined.executable, '');
  assert.equal(fromUndefined.timeoutSeconds, '');
  const fromNull = providerUpdateFormValuesFromUpdate(null);
  assert.equal(fromNull.enabled, false);
  assert.equal(fromNull.args, '');
});

test('providerUpdateFormValuesFromUpdate: enabled 省略時は args の有無から決める（UpdateEnabled と同じ規則）', () => {
  assert.equal(providerUpdateFormValuesFromUpdate({ args: ['update'] }).enabled, true);
  assert.equal(providerUpdateFormValuesFromUpdate({}).enabled, false);
});

test('providerUpdateFromFormValues: 更新の引数が空で ON はエラー', () => {
  const values: ProviderUpdateFormValues = { enabled: true, versionArgs: '', executable: '', args: '', timeoutSeconds: '' };
  const result = providerUpdateFromFormValues(values);
  assert.equal(result.ok, false);
  if (!result.ok) assert.equal(result.error.key, 'settings_ai_providers_update_enabled_without_args');
});

test('providerUpdateFromFormValues: 打ち切り秒が 0〜3600 の外ならエラー、範囲内と空欄は通る', () => {
  const base: ProviderUpdateFormValues = { enabled: false, versionArgs: '', executable: '', args: '', timeoutSeconds: '' };
  assert.equal(providerUpdateFromFormValues({ ...base, timeoutSeconds: '-1' }).ok, false);
  assert.equal(providerUpdateFromFormValues({ ...base, timeoutSeconds: '3601' }).ok, false);
  assert.equal(providerUpdateFromFormValues({ ...base, timeoutSeconds: '12.5' }).ok, false);
  assert.equal(providerUpdateFromFormValues({ ...base, timeoutSeconds: 'abc' }).ok, false);
  const zero = providerUpdateFromFormValues({ ...base, timeoutSeconds: '0' });
  assert.equal(zero.ok, true);
  if (zero.ok) assert.equal(zero.update.timeout_seconds, 0);
  const max = providerUpdateFromFormValues({ ...base, timeoutSeconds: '3600' });
  assert.equal(max.ok, true);
  if (max.ok) assert.equal(max.update.timeout_seconds, 3600);
  const blank = providerUpdateFromFormValues(base);
  assert.equal(blank.ok, true);
  if (blank.ok) assert.equal(blank.update.timeout_seconds, undefined);
});

test('providerUpdateFromFormValues: 空欄フィールドは明示的に undefined を返す（詳細JSONの古い値を上書きするため）', () => {
  const result = providerUpdateFromFormValues({ enabled: false, versionArgs: '', executable: '', args: '', timeoutSeconds: '' });
  assert.equal(result.ok, true);
  if (result.ok) {
    assert.equal(result.update.version_args, undefined);
    assert.equal(result.update.args, undefined);
    assert.equal(result.update.executable, undefined);
    assert.equal(result.update.enabled, false);
    assert.equal('version_args' in result.update, true);
    assert.equal('executable' in result.update, true);
  }
});

test('providerUpdateIsDefault: origin が embedded または未指定なら既定値扱い', () => {
  assert.equal(providerUpdateIsDefault(undefined), true);
  assert.equal(providerUpdateIsDefault({}), true);
  assert.equal(providerUpdateIsDefault({ update: { origin: 'embedded' } }), true);
  assert.equal(providerUpdateIsDefault({ update: {} }), true);
  assert.equal(providerUpdateIsDefault({ update: { origin: 'override' } }), false);
  assert.equal(providerUpdateIsDefault({ update: { origin: 'user' } }), false);
});

test('providerUpdateLoginMayBeRequired: login_may_be_required が true のときだけ true', () => {
  assert.equal(providerUpdateLoginMayBeRequired({ login_may_be_required: true }), true);
  assert.equal(providerUpdateLoginMayBeRequired({ login_may_be_required: false }), false);
  assert.equal(providerUpdateLoginMayBeRequired({}), false);
  assert.equal(providerUpdateLoginMayBeRequired(undefined), false);
});
