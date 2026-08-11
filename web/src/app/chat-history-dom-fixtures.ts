import assert from 'node:assert/strict';
import test from 'node:test';
import { evaluateTranscriptMessage, shouldRefreshChatDerivedState, updateRenderedChatMessage } from './transcript-message.js';

class FakeNode {
  dataset: { msgId: string; role: string; kind: string };
  textContent: string;
  parentNode: FakeTimeline | null = null;

  constructor(messageID: string, text: string, role = 'ai', kind = 'tool') {
    this.dataset = { msgId: messageID, role, kind };
    this.textContent = text;
  }

  replaceWith(replacement: FakeNode) {
    if (!this.parentNode) throw new Error('node is not mounted');
    this.parentNode.replaceChild(replacement, this);
  }
}

class FakeTimeline {
  children: FakeNode[] = [];

  appendChild(node: FakeNode) {
    node.parentNode = this;
    this.children.push(node);
    return node;
  }

  replaceChild(replacement: FakeNode, previous: FakeNode) {
    const index = this.children.indexOf(previous);
    if (index < 0) throw new Error('previous node is not mounted');
    replacement.parentNode = this;
    this.children[index] = replacement;
  }
}

function derivedState(timeline: FakeTimeline, query: string, filters: Set<string>) {
  const visible: string[] = [];
  const hits: string[] = [];
  const marks: string[] = [];
  for (const node of timeline.children) {
    const state = evaluateTranscriptMessage(node.textContent, node.dataset.role, node.dataset.kind, query, filters);
    if (state.filterOk && state.searchOk) visible.push(node.dataset.msgId);
    if (state.searchOk && query) {
      hits.push(node.dataset.msgId);
      marks.push(node.dataset.msgId);
    }
  }
  return { visible, hits, marks };
}

test('subscriber update replaces the visible tool bubble without duplicating it', () => {
  const timeline = new FakeTimeline();
  const renderedIDs = new Set(['1']);
  timeline.appendChild(new FakeNode('1', 'pending: shell'));

  const store = [{ id: 1, meta: { transcript_message_id: 'tool:call-1' }, text: 'pending: shell' }];
  const subscribers = [(message: any) => updateRenderedChatMessage(
    timeline,
    renderedIDs,
    message,
    () => new FakeNode(String(message.id), message.text),
  )];
  const result = { ...store[0], text: 'result: C:/work' };
  store[0] = result;
  for (const subscriber of subscribers) subscriber(result);

  assert.equal(store[0].text, 'result: C:/work');
  assert.equal(timeline.children.length, 1);
  assert.equal(timeline.children[0].textContent, 'result: C:/work');
  assert.equal(timeline.children[0].dataset.msgId, '1');
});

test('updated tool result reapplies search/filter state without retaining the old DOM node', () => {
  const timeline = new FakeTimeline();
  const renderedIDs = new Set(['1']);
  const pending = new FakeNode('1', 'pending: shell');
  timeline.appendChild(pending);
  const filters = new Set(['ai']);
  const query = 'result';

  assert.deepEqual(derivedState(timeline, query, filters), { visible: [], hits: [], marks: [] });
  const result = updateRenderedChatMessage(
    timeline,
    renderedIDs,
    { id: 1, text: 'result: C:/work' },
    () => new FakeNode('1', 'result: C:/work'),
  );

  assert.equal(result, 'updated');
  assert.equal(shouldRefreshChatDerivedState(result), true);
  assert.equal(timeline.children.length, 1);
  assert.notEqual(timeline.children[0], pending);
  assert.deepEqual(derivedState(timeline, query, filters), { visible: ['1'], hits: ['1'], marks: ['1'] });

  const noLongerMatching = updateRenderedChatMessage(
    timeline,
    renderedIDs,
    { id: 1, text: 'completed: C:/work' },
    () => new FakeNode('1', 'completed: C:/work'),
  );
  assert.equal(noLongerMatching, 'updated');
  assert.equal(shouldRefreshChatDerivedState(noLongerMatching), true);
  assert.equal(timeline.children.length, 1);
  assert.deepEqual(derivedState(timeline, query, filters), { visible: [], hits: [], marks: [] });
});
