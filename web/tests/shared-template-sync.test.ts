import { expect, test } from 'bun:test';
import { SharedTemplateSync } from '../src/app/shared-template-sync.ts';

test('an unrelated settings save retains the newer shared list instead of its browser cache', () => {
  const sync = new SharedTemplateSync();
  const server = { templates: [{ body: 'New shared instruction' }], display: { theme: 'dark' } };
  const snapshot = sync.mergeInto(server, [{ body: 'Old cached instruction' }]);
  expect(snapshot).toBeNull();
  expect(server.templates).toEqual([{ body: 'New shared instruction' }]);
});

test('local edits including deletion of all rows are written and acknowledged only on success', () => {
  const sync = new SharedTemplateSync();
  sync.markDirty();
  const server = { templates: [{ body: 'Delete this row' }] };
  const snapshot = sync.mergeInto(server, []);
  expect(server.templates).toEqual([]);
  expect(sync.isDirty()).toBe(true); // A failed request must not acknowledge.
  expect(sync.canMirror(sync.readVersion())).toBe(false);
  expect(sync.acknowledge(snapshot)).toBe(true);
  expect(sync.isDirty()).toBe(false);
});

test('an older PUT response cannot clear a newer edit made while the request was in flight', () => {
  const sync = new SharedTemplateSync();
  sync.markDirty();
  const first = sync.mergeInto({}, [{ body: 'First edit' }]);
  sync.markDirty();
  expect(sync.acknowledge(first)).toBe(false);
  const next = { templates: [] };
  const second = sync.mergeInto(next, [{ body: 'Second edit' }]);
  expect(next.templates).toEqual([{ body: 'Second edit' }]);
  expect(sync.acknowledge(second)).toBe(true);
});

test('a remote read started before an edit cannot overwrite it even after its PUT succeeds', () => {
  const sync = new SharedTemplateSync();
  const read = sync.readVersion();
  sync.markDirty();
  const write = sync.mergeInto({}, [{ body: 'Local edit' }]);
  sync.acknowledge(write);
  expect(sync.canMirror(read)).toBe(false);
  expect(sync.canMirror(sync.readVersion())).toBe(true);
});

test('reload retains a persisted dirty list; an explicit discard invalidates pending reads', () => {
  const sync = new SharedTemplateSync(true);
  const read = sync.readVersion();
  expect(sync.canMirror(read)).toBe(false);
  sync.discard();
  expect(sync.canMirror(read)).toBe(false);
  expect(sync.canMirror(sync.readVersion())).toBe(true);
});
