import { test } from 'node:test';
import assert from 'node:assert/strict';
import { unchangedReplacementHit } from './secrets-scan.baseline.mjs';

const diff = (before, after, header = '@@ -1 +1 @@') => `${header}\n-${before}\n+${after}\n`;
const hit = (before, after, needle = 'SyntheticName') => unchangedReplacementHit(diff(before, after), 1, after, needle);

test('existing occurrences wholly in identical prefix and suffix remain baseline', () => {
  assert.equal(hit('SyntheticName old SyntheticName', 'SyntheticName new SyntheticName'), true);
});
test('a new occurrence in the changed middle is blocked, even with an old occurrence', () => {
  assert.equal(hit('SyntheticName old suffix', 'SyntheticName SyntheticName suffix'), false);
});
test('a moved occurrence remains blocked', () => {
  assert.equal(hit('SyntheticName old suffix', 'old SyntheticName suffix'), false);
});
test('occurrences crossing either boundary of the changed region remain blocked', () => {
  assert.equal(hit('SyntheticNamo suffix', 'SyntheticName suffix'), false);
  assert.equal(hit('old XyntheticName', 'new SyntheticName'), false);
});
test('overlapping occurrences are all checked', () => {
  assert.equal(hit('aaaa old', 'aaaa new', 'aaa'), true);
  assert.equal(hit('aaa old', 'aaaa new', 'aaa'), false);
});
test('new files and multiline hunks keep the normal added-line gate', () => {
  assert.equal(unchangedReplacementHit('@@ -0,0 +1 @@\n+SyntheticName\n', 1, 'SyntheticName', 'SyntheticName'), false);
  assert.equal(unchangedReplacementHit(diff('SyntheticName old', 'SyntheticName new', '@@ -1,2 +1,2 @@'), 1, 'SyntheticName new', 'SyntheticName'), false);
});
test('a worktree line different from the index cannot gain an exemption', () => {
  assert.equal(unchangedReplacementHit(diff('SyntheticName old', 'SyntheticName new'), 1, 'SyntheticName secret', 'SyntheticName'), false);
});
test('line numbers, no-newline markers and later single-line hunks are respected', () => {
  const patch = diff('old', 'new') + diff('SyntheticName old', 'SyntheticName new', '@@ -8 +9 @@') + '\\ No newline at end of file\n';
  assert.equal(unchangedReplacementHit(patch, 9, 'SyntheticName new', 'SyntheticName'), true);
  assert.equal(unchangedReplacementHit(patch, 8, 'SyntheticName new', 'SyntheticName'), false);
});
test('invalid input and absent occurrences fail closed', () => {
  assert.equal(unchangedReplacementHit('', 1, 'SyntheticName', 'SyntheticName'), false);
  assert.equal(hit('old', 'new'), false);
  assert.equal(hit('old', 'new', ''), false);
});
