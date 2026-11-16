import assert from 'node:assert/strict';
import test from 'node:test';
import { PROVIDERS_UPDATED_EVENT, providersUpdatedDetail } from './providers-updated.js';

test('providersUpdatedDetail: providers と reason をそのまま詳細にする', () => {
  assert.deepEqual(
    providersUpdatedDetail({ providers: ['fixture-cli'], reason: 'created' }),
    { ids: ['fixture-cli'], action: 'created' },
  );
});

test('providersUpdatedDetail: 欠けた・崩れた中身でも例外にせず、空で返す', () => {
  assert.deepEqual(providersUpdatedDetail({}), { ids: [], action: '' });
  assert.deepEqual(
    providersUpdatedDetail({ providers: 'fixture-cli', reason: 3 }),
    { ids: [], action: '' },
  );
  assert.deepEqual(
    providersUpdatedDetail({ providers: ['a', 1, '', null, 'b'], reason: 'removed' }),
    { ids: ['a', 'b'], action: 'removed' },
  );
});

test('イベント名は固定（新しいセッションの一覧と設定の一覧が同じ名前を聞く）', () => {
  assert.equal(PROVIDERS_UPDATED_EVENT, 'providers-updated');
});
