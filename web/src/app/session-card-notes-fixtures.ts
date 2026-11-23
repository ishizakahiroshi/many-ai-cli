import test from 'node:test';
import assert from 'node:assert/strict';
import { collectCardNotes, computeBaselines, type CardNoteSession } from './session-card-notes.js';

const normal = { permission_mode: 'bypassPermissions', branch: 'develop' };
const kinds = (session: CardNoteSession, group: CardNoteSession[]) => collectCardNotes(session, computeBaselines(group)).map(n => n.kind);

test('one session is its own baseline, including unspecified values', () => {
  assert.deepEqual(kinds(normal, [normal]), []);
  assert.deepEqual(kinds({}, [{}]), []);
  assert.deepEqual(computeBaselines([]), { permissionMode: null, branch: null });
});

test('identical values need no dot', () => {
  const group = [normal, { ...normal }, { ...normal }];
  for (const session of group) assert.deepEqual(kinds(session, group), []);
});

test('each field has its own most frequent value; only the outlier is marked', () => {
  const unusual = { permission_mode: 'plan', branch: 'feature/example' };
  const group = [normal, normal, unusual];
  assert.deepEqual(computeBaselines(group), { permissionMode: 'bypassPermissions', branch: 'develop' });
  assert.deepEqual(kinds(normal, group), []);
  assert.deepEqual(kinds(unusual, group), ['permission', 'branch']);
});

test('ties mark everyone, including values below the tied maximum', () => {
  const group = [normal, normal, { ...normal, branch: 'alpha' }, { ...normal, branch: 'alpha' }, { ...normal, branch: 'beta' }];
  assert.equal(computeBaselines(group).branch, null);
  for (const session of group) assert.deepEqual(kinds(session, group), ['branch']);
});

test('empty permission and non-git are real values in mixed groups', () => {
  const group = [{}, {}, normal];
  assert.deepEqual(computeBaselines(group), { permissionMode: '', branch: '' });
  assert.deepEqual(kinds({}, group), []);
  assert.deepEqual(kinds(normal, group), ['permission', 'branch']);
  for (const session of [{}, normal]) assert.deepEqual(kinds(session, [{}, normal]), ['permission', 'branch']);
  const majority = [normal, normal, {}];
  assert.deepEqual(collectCardNotes({}, computeBaselines(majority)).map(n => n.value), ['', '']);
});

test('non-git-only groups and absent launch flags have no difference', () => {
  const group = [{}, { permission_mode: '', branch: '' }, { permission_mode: '  ' }];
  for (const session of group) assert.deepEqual(kinds(session, group), []);
});

test('supplements make one notes list even when permission and branch match', () => {
  const session = { ...normal, note: '<synthetic note>', role: 'worker', execution_mode: 'headless', worktree_branch: 'feature/example', handoff_from: 10, board_notify_pending: true };
  const notes = collectCardNotes(session, computeBaselines([session]), 12);
  assert.deepEqual(notes.map(n => n.kind), ['note', 'role', 'headless', 'worktree', 'handoff-from', 'handoff-to', 'board']);
  assert.equal(notes[0].value, '<synthetic note>');
  assert.equal(notes[5].value, '12');
  assert.ok(notes.every(n => n.labelKey));
  assert.deepEqual(kinds({ orchestration_id: 'example' }, [{ orchestration_id: 'example' }]), ['role']);
});

test('parents and collapsed children count equally; input order does not break ties', () => {
  const parent = { ...normal, branch: 'alpha' };
  const children = [normal, normal];
  assert.deepEqual(kinds(parent, [parent, ...children]), ['branch']);
  assert.deepEqual(kinds(children[0], [parent, ...children]), []);
  const group = [{ branch: 'alpha' }, { branch: 'beta' }];
  assert.deepEqual(computeBaselines(group), computeBaselines([...group].reverse()));
});
