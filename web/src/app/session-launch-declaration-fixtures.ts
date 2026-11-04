// session-launch-declaration-fixtures.ts — 起動時の申告の取り込みと、実行モードの見え方を確かめる。
//
// 守るもの: 起動した直後の子が session_update で届けた execution_mode を、画面が必ず拾うこと
// （拾い忘れると「Headless」の印が出ず、入力欄も有効のままになる）。値はすべて合成データ。
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  executionModeKind,
  mergeLaunchDeclaration,
  type LaunchDeclaration,
} from './session-launch-declaration.js';

test('merge: a headless declaration in the register announce is taken', () => {
  // 起動直後の session_update。まだ手元に記録が無いセッションが相手。
  const cur: LaunchDeclaration = {};
  mergeLaunchDeclaration(cur, { execution_mode: 'headless', permission_mode: 'bypassPermissions' });
  assert.equal(cur.execution_mode, 'headless');
  assert.equal(cur.permission_mode, 'bypassPermissions');
});

test('merge: a later update without the fields keeps the declared values', () => {
  // 以降の session_update（状態の変化だけ）は 2 項目を送らない。消えてはいけない。
  const cur: LaunchDeclaration = { execution_mode: 'headless', permission_mode: 'bypassPermissions' };
  mergeLaunchDeclaration(cur, {});
  assert.equal(cur.execution_mode, 'headless');
  assert.equal(cur.permission_mode, 'bypassPermissions');
});

test('merge: each field is independent', () => {
  const cur: LaunchDeclaration = { execution_mode: 'headless' };
  mergeLaunchDeclaration(cur, { permission_mode: 'plan' });
  assert.equal(cur.execution_mode, 'headless');
  assert.equal(cur.permission_mode, 'plan');
  mergeLaunchDeclaration(cur, { execution_mode: '' });
  assert.equal(cur.execution_mode, '');
  assert.equal(cur.permission_mode, 'plan');
});

test('kind: only the exact headless declaration is headless', () => {
  assert.equal(executionModeKind('headless'), 'headless');
  assert.equal(executionModeKind(' headless '), 'headless');
});

test('kind: empty, missing and unknown values are interactive', () => {
  for (const value of ['', undefined, null, 'interactive', 'auto', 'Headless', 'headless-ish', 0, 1, {}, []]) {
    assert.equal(executionModeKind(value), 'interactive', String(value));
  }
});

test('end to end: a declaration merged from the announce is shown as headless', () => {
  const cur: LaunchDeclaration = {};
  mergeLaunchDeclaration(cur, { execution_mode: 'headless' });
  assert.equal(executionModeKind(cur.execution_mode), 'headless');
  // 対話で動くセッションは execution_mode を送らないので、対話のまま。
  const interactive: LaunchDeclaration = {};
  mergeLaunchDeclaration(interactive, {});
  assert.equal(executionModeKind(interactive.execution_mode), 'interactive');
});
