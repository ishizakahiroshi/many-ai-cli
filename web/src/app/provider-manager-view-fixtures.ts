import assert from 'node:assert/strict';
import test from 'node:test';
import {
  createRequestGuard,
  formatDiagnosticMessages,
  isBuiltinProviderID,
  originLabelKey,
  providerErrorMessage,
  providerStatusKind,
  statusLabelKey,
  suggestProviderId,
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
  assert.equal(providerErrorMessage({ kind: 'http', status: 409 }).key, 'settings_ai_providers_conflict');
  const serverError = providerErrorMessage({ kind: 'http', status: 503 });
  assert.equal(serverError.key, 'settings_ai_providers_server_error');
  assert.equal(serverError.vars.status, 503);
  const other = providerErrorMessage({ kind: 'http', status: 400 });
  assert.equal(other.key, 'settings_ai_providers_request_failed');
  assert.equal(other.vars.status, 400);
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
