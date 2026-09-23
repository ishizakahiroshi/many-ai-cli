import assert from 'node:assert/strict';
import test from 'node:test';
import type { ApprovalRecord } from '../types/proto.js';
import { _resetApprovalAnsweredStateForTest, recordAnsweredApprovalIdentity } from './approval-answered.js';
import {
  _resetApprovalStoreForTest,
  applyApprovalSnapshot,
  applyApprovalState,
  approvalRecordFor,
  approvalStoreVersion,
  approvalViewFor,
  approvalViewForRecord,
  _setApprovalFoldStorageForTest,
  foldApproval,
  forgetApprovalSession,
  isApprovalFolded,
  isApprovalPending,
  setApprovalFoldScope,
  subscribeApprovalStore,
  unfoldApproval,
  type ApprovalFoldStorage,
  type ApprovalStoreChange,
} from './approval-store.js';

// 保留中の承認の画面側ストア（approval-store.ts）の fixture。入力はすべて合成データ。
// 承認マーカーの文字列は部品から組み立てる（実文字列をそのまま書かない）。
const MARKER_NAME = ['MANY', 'AI', 'CLI'].join('-');
const OPEN = `[${MARKER_NAME}]`;
const CLOSE = `[/${MARKER_NAME}]`;

function nativeRecord(key: string, epoch = 1, extra: Partial<ApprovalRecord> = {}): ApprovalRecord {
  return {
    candidate_key: key,
    source_epoch: epoch,
    sig: `sig-${key}`,
    origin: 'native',
    source: 'go_vt',
    kind: 'native',
    question: 'Run: git status',
    options: [
      { num: 1, label: 'Yes', is_current: true },
      { num: 2, label: 'No' },
    ],
    summary: { command: 'git status', risk: 'low' },
    ...extra,
  };
}

// 既定の本文は見出し付きの単問（見出しの無い地の文は、パーサが前置きとして扱う）。
function markerRecord(key: string, epoch = 1, body: string[] = ['Q1 この変更を適用しますか?', '1. はい (Recommended)', '2. いいえ', 'N. User specifies']): ApprovalRecord {
  return {
    candidate_key: key,
    source_epoch: epoch,
    sig: `sig-${key}`,
    origin: 'marker',
    source: 'transcript',
    kind: 'marker',
    block: [OPEN, ...body, CLOSE].join('\n'),
  };
}

function reset(...ids: number[]): void {
  _resetApprovalStoreForTest();
  for (const id of ids) _resetApprovalAnsweredStateForTest(id);
}

function closeOf(record: ApprovalRecord, reason = 'answered') {
  return { candidate_key: record.candidate_key, source_epoch: record.source_epoch, sig: record.sig, origin: record.origin, reason };
}

function summarize(changes: ApprovalStoreChange[]) {
  return changes
    .map((c) => [c.sessionId, c.record?.candidate_key || null, c.previous?.candidate_key || null, c.announce])
    .sort((a, b) => Number(a[0]) - Number(b[0]));
}

test('a snapshot replaces every session and keeps no record from the previous connection', () => {
  reset(1, 2, 3);
  applyApprovalState(1, { version: 1, open: nativeRecord('a') });
  applyApprovalState(2, { version: 1, open: nativeRecord('b') });

  const changes = applyApprovalSnapshot([
    { session_id: 1, version: 4, record: nativeRecord('c', 2) },
    { session_id: 3, version: 0 },
  ]);

  assert.equal(approvalRecordFor(1)?.candidate_key, 'c');
  assert.equal(approvalRecordFor(2), null, 'a session missing from the snapshot must not keep its old record');
  assert.equal(approvalRecordFor(3), null);
  assert.equal(approvalStoreVersion(1), 4);
  // まとめで届いた記録は知らせない（接続・再接続のたびに音が鳴らないように）。
  assert.deepEqual(summarize(changes), [[1, 'c', 'a', false], [2, null, 'b', false]]);
});

test('a snapshot from a restarted Hub replaces the record even when its version is lower', () => {
  reset(1);
  applyApprovalState(1, { version: 9, open: nativeRecord('before-restart') });
  applyApprovalSnapshot([{ session_id: 1, version: 1, record: nativeRecord('after-restart') }]);
  assert.equal(approvalRecordFor(1)?.candidate_key, 'after-restart');
  assert.equal(approvalStoreVersion(1), 1);
});

test('a state older than or equal to the held version is ignored', () => {
  reset(1);
  applyApprovalState(1, { version: 5, open: nativeRecord('new') });

  assert.equal(applyApprovalState(1, { version: 4, open: nativeRecord('old') }), null);
  assert.equal(applyApprovalState(1, { version: 5, close: closeOf(nativeRecord('new')) }), null);
  assert.equal(approvalRecordFor(1)?.candidate_key, 'new');
  assert.equal(approvalStoreVersion(1), 5);
});

test('closing a record the screen never saw changes nothing but the version', () => {
  reset(1);
  let calls = 0;
  subscribeApprovalStore(() => { calls++; });

  // Hub が自動承認した記録は、開いたことが画面へ届かず閉じるだけが届く。
  const change = applyApprovalState(1, { version: 3, close: closeOf(nativeRecord('auto-approved')) });

  assert.equal(change, null);
  assert.equal(approvalRecordFor(1), null);
  assert.equal(approvalStoreVersion(1), 3);
  assert.equal(calls, 0, 'no listener is told about a close that changes nothing on screen');
});

test('a newer close always leaves the session without a record', () => {
  reset(1);
  // Hub は記録を 1 件しか持たず、閉じた版では記録が無い。手元の記録と違う記録の閉じるでも、
  // その版まで進んだ Hub には手元の記録も残っていない（間の開閉が逆順に届いて捨てられた場合）。
  applyApprovalState(1, { version: 2, open: nativeRecord('held') });
  applyApprovalState(1, { version: 6, close: closeOf(nativeRecord('other')) });
  assert.equal(approvalRecordFor(1), null);
});

test('a delayed open older than the close does not bring the record back', () => {
  reset(1);
  const record = markerRecord('m');
  applyApprovalState(1, { version: 2, open: record });
  applyApprovalState(1, { version: 3, close: closeOf(record, 'answered_terminal') });

  assert.equal(applyApprovalState(1, { version: 2, open: record }), null);
  assert.equal(approvalRecordFor(1), null);
});

test('an open that overtakes the close of the previous record wins', () => {
  reset(1);
  const first = nativeRecord('first');
  applyApprovalState(1, { version: 1, open: first });
  // 上書き: Hub は古い記録の閉じる（版 2）と新しい記録の開く（版 3）を送る。届く順は逆のことがある。
  applyApprovalState(1, { version: 3, open: nativeRecord('second') });
  assert.equal(applyApprovalState(1, { version: 2, close: closeOf(first, 'superseded') }), null);
  assert.equal(approvalRecordFor(1)?.candidate_key, 'second');
});

test('a record that arrived while another session was on screen is drawn when switched to', () => {
  reset(3, 7);
  // 報告の症状（bugfix_approval-panel-blank-on-switch_2026-09-23.md）。画面はセッション 3 を
  // 見ている間に、セッション 7 の承認を受け取る。ストアは見ているセッションに関係なく記録を持つので、
  // 7 へ切り替えた時点で描く中身がある（以前はタイマーと端末の走査を待ち、行き止まりに入っていた）。
  const seen: ApprovalStoreChange[] = [];
  subscribeApprovalStore((changes) => { seen.push(...changes); });
  applyApprovalState(7, { version: 1, open: markerRecord('while-away') });

  assert.deepEqual(summarize(seen), [[7, 'while-away', null, true]]);
  const view = approvalViewFor(7);
  assert.equal(view.kind, 'options');
  assert.deepEqual(view.options?.map((o: any) => [o.num, o.label]), [[1, 'はい (Recommended)'], [2, 'いいえ']]);
  assert.equal((view.options as any)._question, 'この変更を適用しますか?');
  assert.equal((view.options as any)._freeInput, true);
  assert.equal(approvalViewFor(3).kind, 'none');
});

test('only a new identity arriving as a state is announced', () => {
  reset(1);
  const record = nativeRecord('k', 4);
  assert.equal(applyApprovalState(1, { version: 1, open: record })?.announce, true);
  applyApprovalState(1, { version: 2, close: closeOf(record, 'vanished') });
  // 同じ世代で開き直した記録（ネイティブの承認が一瞬見えなくなった等）は鳴らし直さない。
  assert.equal(applyApprovalState(1, { version: 3, open: nativeRecord('k', 4) })?.announce, false);
  applyApprovalState(1, { version: 4, close: closeOf(record) });
  // 世代が進めば同じ質問でも新しい承認として知らせる。
  assert.equal(applyApprovalState(1, { version: 5, open: nativeRecord('k', 5) })?.announce, true);
});

test('a record first seen in a snapshot is not announced when the same record opens again', () => {
  reset(1);
  applyApprovalSnapshot([{ session_id: 1, version: 2, record: nativeRecord('k') }]);
  applyApprovalState(1, { version: 3, close: closeOf(nativeRecord('k'), 'vanished') });
  assert.equal(applyApprovalState(1, { version: 4, open: nativeRecord('k') })?.announce, false);
});

test('an answered record is not drawn and is no longer pending', () => {
  reset(1);
  const record = nativeRecord('answered', 2);
  applyApprovalState(1, { version: 1, open: record });
  assert.equal(isApprovalPending(1), true);

  recordAnsweredApprovalIdentity(1, record.candidate_key, record.source_epoch);

  assert.equal(approvalViewFor(1).kind, 'none');
  assert.equal(isApprovalPending(1), false);
  // 同じ質問でも世代の違う記録は別の承認なので描く。
  applyApprovalState(1, { version: 2, open: nativeRecord('answered', 3) });
  assert.equal(approvalViewFor(1).kind, 'options');
});

test('a native record carries the Hub options, summary and identity', () => {
  reset(1);
  const view = approvalViewForRecord(nativeRecord('native-key', 6, { kind: 'native_codex_shortcut', options: [{ num: 1, label: 'Yes (y)', send_text: 'y', is_current: true }, { num: 2, label: 'No (n)', send_text: 'n' }] }));
  assert.equal(view.kind, 'options');
  const options: any = view.options;
  assert.deepEqual(options.map((o: any) => [o.num, o.label, o._sendText, o.isCurrent]), [[1, 'Yes (y)', 'y', true], [2, 'No (n)', 'n', false]]);
  assert.equal(options._summary.command, 'git status');
  assert.equal(options._summary.risk, 'low');
  assert.equal(options._question, 'Run: git status');
  assert.equal(options._candidateKey, 'native-key');
  assert.equal(options._sourceEpoch, 6);
  assert.equal(options[0]._approvalSource, 'go_vt');
  assert.equal(options[0]._approvalSig, 'sig-native-key');
});

test('the AskUserQuestion notice is drawn as a notice without options', () => {
  reset();
  const view = approvalViewForRecord({ candidate_key: 'auq', source_epoch: 1, origin: 'native', kind: 'ask_user_question', question: 'どれにしますか?' });
  assert.equal(view.kind, 'notice');
  assert.equal(view.options, undefined);
});

test('a marker block the parser rejects is reported as unreadable', () => {
  reset();
  // 選択肢 1 が欠けたブロック（端末ミラーのずれで選択肢行が落ちた形）。
  const view = approvalViewForRecord(markerRecord('broken', 1, ['どれにしますか?', '3. A', '4. B']));
  assert.equal(view.kind, 'unreadable');
});

test('the text questions the Hub moved over are drawn from the record', () => {
  reset();
  const yesNo = approvalViewForRecord({
    candidate_key: 'yn', source_epoch: 1, origin: 'marker', kind: 'plain_yes_no',
    question: 'このまま進めますか?',
    block: 'このまま進めますか? (Y:1/N:0)',
    options: [{ num: 1, label: 'Yes (1)', is_current: true, preserve_order: true }, { num: 0, label: 'No (0)', preserve_order: true }],
  });
  assert.equal(yesNo.kind, 'options');
  assert.deepEqual(yesNo.options?.map((o: any) => [o.num, o.label, o.preserveOrder]), [[1, 'Yes (1)', true], [0, 'No (0)', true]]);
  assert.equal((yesNo.options as any)._question, 'このまま進めますか?');
  assert.equal(yesNo.options?.[0]._approvalSource, undefined, 'a text question is answered by a message, not a native key');

  const choice = approvalViewForRecord({
    candidate_key: 'hc', source_epoch: 1, origin: 'marker', kind: 'hub_choice',
    question: 'どちらで進めますか?',
    block: 'どちらで進めますか?\n1. A 案 (Recommended)\n2. B 案\nN. User specifies',
    options: [{ num: 1, label: 'A 案 (Recommended)', is_current: true }, { num: 2, label: 'B 案' }],
  });
  assert.equal(choice.kind, 'options');
  assert.equal((choice.options as any)._freeInput, true);

  const sequential = approvalViewForRecord({
    candidate_key: 'seq', source_epoch: 1, origin: 'marker', kind: 'sequential_choice',
    question: 'Q1: 対象は?\nQ2: 形式は?',
    block: 'Q1: 対象は?\n  1. 全部\n  2. 一部\nQ2: 形式は?\n  1. 表\n  2. 箇条',
    options: [{ num: 1, label: '全部' }, { num: 2, label: '一部' }, { num: 1, label: '表' }, { num: 2, label: '箇条' }],
  });
  assert.equal(sequential.kind, 'sequential');
  assert.deepEqual(sequential.prompts?.map((p: any) => [p.key, p.question, p.options.map((o: any) => o.label)]), [
    ['Q1', '対象は?', ['全部', '一部']],
    ['Q2', '形式は?', ['表', '箇条']],
  ]);
});

test('forgetting a session drops its record and tells listeners', () => {
  reset(1);
  applyApprovalState(1, { version: 1, open: nativeRecord('gone') });
  const seen: ApprovalStoreChange[] = [];
  subscribeApprovalStore((changes) => { seen.push(...changes); });
  forgetApprovalSession(1);
  assert.equal(approvalRecordFor(1), null);
  assert.deepEqual(summarize(seen), [[1, null, 'gone', false]]);
});

// ---- 畳む（✕）----
// 保存先（ブラウザでは sessionStorage）はメモリの偽物に差し替える。reset() はストアの写しと
// 畳み状態の写しだけを捨て、保存先は残す＝ページの読み込み直しに当たる。

class MemoryStorage implements ApprovalFoldStorage {
  private values = new Map<string, string>();
  getItem(key: string): string | null { return this.values.has(key) ? this.values.get(key)! : null; }
  setItem(key: string, value: string): void { this.values.set(key, value); }
  stored(): string[] { return JSON.parse(this.getItem('ai_cli_hub_approval_folded') || '[]'); }
}

function connect(scope: string, states: { session_id: number; version: number; record?: ApprovalRecord }[]): void {
  setApprovalFoldScope(scope);
  applyApprovalSnapshot(states);
}

test('a folded record is drawn as a band, but stays pending and keeps its options for the approval tab', () => {
  _setApprovalFoldStorageForTest(new MemoryStorage());
  reset(1);
  connect('hub-a', [{ session_id: 1, version: 1, record: markerRecord('fold-me') }]);

  assert.equal(approvalViewFor(1, { folded: true }).kind, 'options', 'a record that is not folded opens');
  assert.equal(foldApproval(1), true);
  assert.equal(foldApproval(1), false, 'folding twice changes nothing');

  assert.equal(isApprovalFolded(1), true);
  assert.equal(approvalViewFor(1, { folded: true }).kind, 'folded', 'the panel draws a band instead');
  assert.equal(approvalViewFor(1).kind, 'options', 'readers other than the panel still get the options');
  assert.equal(isApprovalPending(1), true, 'folding does not answer or close the approval');

  assert.equal(unfoldApproval(1), true);
  assert.equal(approvalViewFor(1, { folded: true }).kind, 'options', 'the band opens back into the panel');
});

test('the same record stays folded after a reload, and a new record opens', () => {
  const storage = new MemoryStorage();
  _setApprovalFoldStorageForTest(storage);
  reset(1);
  connect('hub-a', [{ session_id: 1, version: 1, record: nativeRecord('keep', 3) }]);
  foldApproval(1);

  // 読み込み直し: ストアも畳み状態の写しも捨て、Hub が送り直すまとめで埋め直す。
  reset(1);
  connect('hub-a', [{ session_id: 1, version: 1, record: nativeRecord('keep', 3) }]);
  assert.equal(approvalViewFor(1, { folded: true }).kind, 'folded', 'the same approval is still folded after a reload');

  // 同じ質問でも世代が進めば別の記録。開いて出る。
  applyApprovalState(1, { version: 2, open: nativeRecord('keep', 4) });
  assert.equal(approvalViewFor(1, { folded: true }).kind, 'options', 'a new record opens');

  // 置き換わった記録の項目は、次のまとめで刈り込まれる（件数の上限とあわせた掃除）。
  assert.equal(storage.stored().length, 1);
  connect('hub-a', [{ session_id: 1, version: 2, record: nativeRecord('keep', 4) }]);
  assert.deepEqual(storage.stored(), [], 'a fold that no snapshot record matches is pruned');
});

// ネイティブの承認は画面から一時的に消えて閉じ（vanished）、同じ候補・同じ世代で開き直すことがある。
// それは同じ承認なので、畳んだまま出す（原則 5）。世代が進んだ記録は別の承認なので開いて出る。
test('a record that closes and reopens with the same identity stays folded', () => {
  const storage = new MemoryStorage();
  _setApprovalFoldStorageForTest(storage);
  reset(1);
  const record = nativeRecord('flicker');
  connect('hub-a', [{ session_id: 1, version: 1, record }]);
  foldApproval(1);

  applyApprovalState(1, { version: 2, close: closeOf(record, 'vanished') });
  assert.equal(isApprovalFolded(1), false, 'no record, nothing folded');
  applyApprovalState(1, { version: 3, open: nativeRecord('flicker') });
  assert.equal(isApprovalFolded(1), true, 'the same approval comes back folded');

  applyApprovalState(1, { version: 4, close: closeOf(record, 'answered') });
  applyApprovalState(1, { version: 5, open: nativeRecord('flicker', 2) });
  assert.equal(isApprovalFolded(1), false, 'the next epoch is a new approval and opens');
});

test('folds are kept per session and per Hub start', () => {
  const storage = new MemoryStorage();
  _setApprovalFoldStorageForTest(storage);
  reset(1, 2);
  connect('hub-a', [
    { session_id: 1, version: 1, record: markerRecord('same-question') },
    { session_id: 2, version: 1, record: markerRecord('same-question') },
  ]);
  foldApproval(1);
  assert.equal(isApprovalFolded(1), true);
  assert.equal(isApprovalFolded(2), false, 'the same question in another session is not folded');

  // Hub を起動し直すとセッション番号も世代も振り直される。前の起動で畳んだものを持ち越さない。
  reset(1, 2);
  connect('hub-b', [{ session_id: 1, version: 1, record: markerRecord('same-question') }]);
  assert.equal(isApprovalFolded(1), false, 'a fold from the previous Hub start does not apply');
  assert.deepEqual(storage.stored(), [], 'folds that no snapshot record matches are pruned');
});

test('a snapshot prunes folds of records that closed while the page was away', () => {
  const storage = new MemoryStorage();
  _setApprovalFoldStorageForTest(storage);
  reset(1, 2);
  connect('hub-a', [
    { session_id: 1, version: 1, record: nativeRecord('stays') },
    { session_id: 2, version: 1, record: nativeRecord('closes-offline') },
  ]);
  foldApproval(1);
  foldApproval(2);

  reset(1, 2);
  connect('hub-a', [
    { session_id: 1, version: 1, record: nativeRecord('stays') },
    { session_id: 2, version: 5 },
  ]);
  assert.equal(isApprovalFolded(1), true);
  assert.equal(storage.stored().length, 1);
});

test('forgetting a session drops its fold, and the number of folds is capped', () => {
  const storage = new MemoryStorage();
  _setApprovalFoldStorageForTest(storage);
  reset();
  const states = Array.from({ length: 205 }, (_v, i) => ({ session_id: i + 1, version: 1, record: nativeRecord(`k${i + 1}`) }));
  connect('hub-a', states);
  for (const state of states) foldApproval(state.session_id);
  assert.equal(storage.stored().length, 200, 'only the latest folds are kept');
  assert.equal(isApprovalFolded(1), false, 'the oldest folds are the ones dropped');
  assert.equal(isApprovalFolded(205), true);

  forgetApprovalSession(205);
  assert.equal(storage.stored().length, 199);
  _setApprovalFoldStorageForTest(null);
});
