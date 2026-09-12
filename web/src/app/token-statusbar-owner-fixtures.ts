import assert from 'node:assert/strict';
import test from 'node:test';
import {
  sentHistoryModalOwnerMatches,
  type SentHistoryModalOwner,
  type SentHistoryModalTarget,
} from './token-statusbar-owner.js';

interface SentHistoryItem {
  text: string;
  attachments: string[];
}

interface FakeSentHistoryModal {
  dataset: SentHistoryModalTarget;
  count: number;
  items: SentHistoryItem[];
}

function fakeModal(owner: SentHistoryModalOwner, items: SentHistoryItem[]): FakeSentHistoryModal {
  return {
    dataset: {
      sentHistorySessionId: String(owner.sessionId),
      sentHistoryModalInstanceId: String(owner.instanceId),
    },
    count: items.length,
    items: items.map((item) => ({ ...item, attachments: [...item.attachments] })),
  };
}

function renderIntoCurrentModal(
  current: FakeSentHistoryModal | null,
  owner: SentHistoryModalOwner,
  items: SentHistoryItem[],
): void {
  if (!current || !sentHistoryModalOwnerMatches(current.dataset, owner)) return;
  current.count = items.length;
  current.items = items.map((item) => ({ ...item, attachments: [...item.attachments] }));
}

function deferred<T>(): { promise: Promise<T>; resolve(value: T): void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

test('a delayed restore from one session cannot repaint the next session modal', async () => {
  const ownerA: SentHistoryModalOwner = { sessionId: 3, instanceId: 1 };
  const ownerB: SentHistoryModalOwner = { sessionId: 4, instanceId: 2 };
  let current: FakeSentHistoryModal | null = fakeModal(ownerA, []);
  const restoreA = deferred<SentHistoryItem[]>();
  const finishA = restoreA.promise.then((items) => renderIntoCurrentModal(current, ownerA, items));

  const modalB = fakeModal(ownerB, [{ text: 'B message', attachments: ['B.txt'] }]);
  current = modalB;
  restoreA.resolve([
    { text: 'A newer message', attachments: ['A.png'] },
    { text: 'A older message', attachments: [] },
  ]);
  await finishA;

  assert.equal(modalB.count, 1);
  assert.deepEqual(modalB.items, [{ text: 'B message', attachments: ['B.txt'] }]);
});

test('a delayed restore cannot repaint a reopened modal for the same session', async () => {
  const oldOwner: SentHistoryModalOwner = { sessionId: 4, instanceId: 1 };
  const reopenedOwner: SentHistoryModalOwner = { sessionId: 4, instanceId: 2 };
  let current: FakeSentHistoryModal | null = fakeModal(oldOwner, []);
  const restore = deferred<SentHistoryItem[]>();
  const finish = restore.promise.then((items) => renderIntoCurrentModal(current, oldOwner, items));

  const reopenedModal = fakeModal(reopenedOwner, [{ text: 'Current contents', attachments: ['current.txt'] }]);
  current = reopenedModal;
  restore.resolve([{ text: 'Stale contents', attachments: ['stale.txt'] }]);
  await finish;

  assert.equal(reopenedModal.count, 1);
  assert.deepEqual(reopenedModal.items, [{ text: 'Current contents', attachments: ['current.txt'] }]);
});
