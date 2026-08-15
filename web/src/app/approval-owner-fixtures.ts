import assert from 'node:assert/strict';
import test from 'node:test';
import {
  actionBarNeedsRepaint,
  actionBarOwnedByOther,
  actionBarOwner,
  releaseActionBarOwnership,
  type ActionBarLike,
} from './approval-owner.js';

// #action-bar の最小 Fake。実 DOM のうち所有者判定が触る面だけを持つ。
class FakeActionBar implements ActionBarLike {
  dataset: Record<string, string | undefined> = {};
  innerHTML = '';
  private classes = new Set<string>();
  private childCount = 0;

  classList = {
    contains: (token: string) => this.classes.has(token),
    remove: (...tokens: string[]) => { for (const token of tokens) this.classes.delete(token); },
    add: (...tokens: string[]) => { for (const token of tokens) this.classes.add(token); },
  };

  get children() { return { length: this.childCount }; }

  /** showActionBar が描画完了時にやることの要約（dataset を焼き、visible にし、中身を持つ）。 */
  paintFor(sessionId: number, candidateKey = 'key-' + sessionId, sourceEpoch = 1) {
    this.dataset.approvalSessionId = String(sessionId);
    this.dataset.approvalCandidateKey = candidateKey;
    this.dataset.approvalSourceEpoch = String(sourceEpoch);
    this.classList.add('visible');
    this.innerHTML = `<button>option for #${sessionId}</button>`;
    this.childCount = 1;
    return this;
  }
}

test('a bar painted for another session is reported as owned by someone else', () => {
  const bar = new FakeActionBar().paintFor(8);

  assert.equal(actionBarOwner(bar), '8');
  assert.equal(actionBarOwnedByOther(bar, 6), true);
  assert.equal(actionBarOwnedByOther(bar, 8), false);
});

test('an unowned bar is not treated as belonging to someone else', () => {
  const bar = new FakeActionBar();

  assert.equal(actionBarOwner(bar), null);
  assert.equal(actionBarOwnedByOther(bar, 6), false);
  assert.equal(actionBarOwnedByOther(null, 6), false);
  assert.equal(actionBarOwnedByOther(undefined, 6), false);
});

// ケース 1: #A のパネルが出た状態で #B へ切替。#B に承認が無ければパネルは消えている。
test('switching away releases a panel that belongs to the previous session', () => {
  const bar = new FakeActionBar().paintFor(8);

  assert.equal(actionBarOwnedByOther(bar, 6), true);
  releaseActionBarOwnership(bar);

  assert.equal(bar.classList.contains('visible'), false);
  assert.equal(bar.innerHTML, '');
  assert.equal(actionBarOwner(bar), null);
  assert.equal(bar.dataset.approvalCandidateKey, undefined);
  assert.equal(bar.dataset.approvalSourceEpoch, undefined);
});

// ケース 2: 切替先にも承認がある場合、#B のパネルへ差し替わる（#A が残らない）。
// これが 2026-08-14 の不具合の中心。所有者を見ない再描画判定では、
// 「visible で children もある」ため描き直されず #A が残っていた。
test('a bar owned by another session always needs a repaint, even while visible and populated', () => {
  const bar = new FakeActionBar().paintFor(8);

  assert.equal(bar.classList.contains('visible'), true);
  assert.equal(bar.children.length, 1);
  assert.equal(actionBarNeedsRepaint(bar, 6), true, 'a panel owned by #8 must be repainted for #6');
  assert.equal(actionBarNeedsRepaint(bar, 8), false, 'the owner itself must not be repainted needlessly');
});

// ケース 3: 回答後に元のセッションへ戻ると、そのセッションのパネルが出る。
// 解放してから所有者を焼き直せば、戻り先の描画は妨げられない。
test('returning to the original session repaints it after the release', () => {
  const bar = new FakeActionBar().paintFor(8);

  releaseActionBarOwnership(bar);
  assert.equal(actionBarNeedsRepaint(bar, 6), true, 'an emptied bar always needs a repaint');

  bar.paintFor(6);
  assert.equal(actionBarOwner(bar), '6');
  assert.equal(actionBarNeedsRepaint(bar, 6), false);

  // #8 へ戻る番。#6 のパネルは #8 にとって他人のものなので描き直される。
  assert.equal(actionBarOwnedByOther(bar, 8), true);
  assert.equal(actionBarNeedsRepaint(bar, 8), true);
});

// ケース 4 の一部: 所有者が同じなら、解放も再描画も起こさない。
// ✕ で消した後の再表示抑止（manualHideState）は showActionBar 側の責務なので、
// ここで所有者判定が余計に描き直しを要求しないことだけを固定する。
test('the owner session is left alone so existing suppression is not overridden', () => {
  const bar = new FakeActionBar().paintFor(6, 'same-question', 3);

  assert.equal(actionBarOwnedByOther(bar, 6), false);
  assert.equal(actionBarNeedsRepaint(bar, 6), false);
  assert.equal(bar.dataset.approvalCandidateKey, 'same-question');
  assert.equal(bar.dataset.approvalSourceEpoch, '3');
});

test('an emptied or hidden bar needs a repaint regardless of ownership', () => {
  const missing = actionBarNeedsRepaint(null, 6);
  assert.equal(missing, true);

  const hidden = new FakeActionBar();
  hidden.dataset.approvalSessionId = '6';
  assert.equal(actionBarNeedsRepaint(hidden, 6), true, 'not visible -> repaint');

  const visibleButEmpty = new FakeActionBar().paintFor(6);
  visibleButEmpty.classList.remove('visible');
  assert.equal(actionBarNeedsRepaint(visibleButEmpty, 6), true);
});

test('releasing a bar that does not exist is a no-op', () => {
  releaseActionBarOwnership(null);
  releaseActionBarOwnership(undefined);
});
