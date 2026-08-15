import assert from 'node:assert/strict';
import test from 'node:test';
import {
  _resetApprovalAnsweredStateForTest,
  answeredApprovalCandidates,
  clearReplayAnsweredApprovalCandidate,
  isAnsweredApprovalCandidate,
  noteApprovalSourceEpoch,
  recordAnsweredApprovalCandidate,
  recordAnsweredApprovalIdentity,
  setApprovalProviderResolver,
} from './approval-answered.js';

// provider は本番では sessions から引く。ここでは固定値を差して DOM 抜きで試す。
const providers = new Map<number, string>();
setApprovalProviderResolver((id) => providers.get(id) || '');

// answeredApprovalCandidates が持つトークンの形（state.ts の answeredCandidateToken と同契約）。
function token(sourceEpoch: number, candidateKey: string): string {
  return `${sourceEpoch}\0${candidateKey}`;
}

function hasAnsweredToken(id: number, sourceEpoch: number, candidateKey: string): boolean {
  return !!answeredApprovalCandidates.get(id)?.has(token(sourceEpoch, candidateKey));
}

function resetSession(id: number, provider = 'claude'): void {
  _resetApprovalAnsweredStateForTest(id);
  providers.set(id, provider);
}

/** Hub のマーカー承認が UI へ渡す形の最小再現。label は表示用、_sendText が送信実体。 */
function markerOptions(question: string, labels: string[], sendTexts?: string[]): any[] {
  const opts: any = labels.map((label, i) => ({
    num: i + 1,
    label,
    _sendText: sendTexts ? sendTexts[i] : String(i + 1),
  }));
  opts._question = question;
  return opts;
}

test('同じ世代では回答済み候補を再表示しない', () => {
  resetSession(1);
  const opts = markerOptions('この変更を適用しますか?', ['はい', 'いいえ']);
  assert.equal(isAnsweredApprovalCandidate(1, opts, 'marker'), false);
  recordAnsweredApprovalCandidate(1, opts, 'marker');
  assert.equal(isAnsweredApprovalCandidate(1, opts, 'marker'), true);
});

// 撤去した approvalConsumedSig は 5〜10 秒のタイマーで失効していた。世代で持つ
// 現在の state は時間で消えないので、何秒後に再流入しても抑止が続く。
test('回答済みの抑止は時間で失効しない', async () => {
  resetSession(2);
  const opts = markerOptions('この変更を適用しますか?', ['はい', 'いいえ']);
  recordAnsweredApprovalCandidate(2, opts, 'marker');
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(isAnsweredApprovalCandidate(2, opts, 'marker'), true);
});

test('TUI の再描画でラベルが揺れても同じ候補として抑止する', () => {
  resetSession(3);
  recordAnsweredApprovalCandidate(3, markerOptions('この変更を適用しますか?', ['はい', 'いいえ']), 'marker');
  // 折返し・余白・罫線の混入でラベルだけが変わった再描画。番号と送信文字列は同じ。
  const redrawn = markerOptions('この変更を適用しますか?  ', ['は い', '── いいえ']);
  assert.equal(isAnsweredApprovalCandidate(3, redrawn, 'marker'), true);
});

test('世代が進めば同じ質問文でも新しい候補として表示する', () => {
  resetSession(4);
  const opts = markerOptions('この変更を適用しますか?', ['はい', 'いいえ']);
  recordAnsweredApprovalCandidate(4, opts, 'marker');
  assert.equal(isAnsweredApprovalCandidate(4, opts, 'marker'), true);
  // 新しい live prompt 境界で Hub が同じ質問を出し直した場合。
  noteApprovalSourceEpoch(4, 2);
  assert.equal(isAnsweredApprovalCandidate(4, opts, 'marker'), false);
});

test('質問文が違えば別候補として表示する', () => {
  resetSession(5);
  recordAnsweredApprovalCandidate(5, markerOptions('A を消しますか?', ['はい', 'いいえ']), 'marker');
  assert.equal(isAnsweredApprovalCandidate(5, markerOptions('B を消しますか?', ['はい', 'いいえ']), 'marker'), false);
});

test('送信文字列が違えば別候補として表示する', () => {
  resetSession(6);
  recordAnsweredApprovalCandidate(6, markerOptions('どれにしますか?', ['A', 'B'], ['a', 'b']), 'marker');
  assert.equal(isAnsweredApprovalCandidate(6, markerOptions('どれにしますか?', ['A', 'B'], ['a', 'c']), 'marker'), false);
});

// marker 承認と Codex 等の native 承認は別の parse 経路から別の配列で届く。
// 質問文が同じでも承認種別が違えば別候補として扱う（片方の回答でもう片方を隠さない）。
test('marker と native は同じ質問文でも相互に抑止しない', () => {
  resetSession(7);
  recordAnsweredApprovalCandidate(7, markerOptions('コマンドを実行しますか?', ['はい', 'いいえ']), 'marker');
  const nativeOpts = markerOptions('コマンドを実行しますか?', ['はい', 'いいえ']);
  assert.equal(isAnsweredApprovalCandidate(7, nativeOpts, 'native'), false);
});

// recordAnsweredApprovalCandidate は渡された配列へ _candidateKey を焼く
// （annotateApprovalIdentity）。同じ配列を後から別種別で判定すると、焼かれた key が
// 優先されて種別の違いが効かなくなる。呼び出し側が配列を使い回さない前提を固定する。
test('回答時に候補 key が配列へ焼かれる', () => {
  resetSession(14);
  const opts = markerOptions('コマンドを実行しますか?', ['はい', 'いいえ']);
  const identity = recordAnsweredApprovalCandidate(14, opts, 'marker');
  assert.ok(identity);
  assert.equal((opts as any)._candidateKey, identity!.candidateKey);
  assert.equal((opts as any)._sourceEpoch, identity!.sourceEpoch);
});

test('回答済みの記録はセッションを跨がない', () => {
  resetSession(8);
  resetSession(9);
  const opts = markerOptions('この変更を適用しますか?', ['はい', 'いいえ']);
  recordAnsweredApprovalCandidate(8, opts, 'marker');
  assert.equal(isAnsweredApprovalCandidate(9, opts, 'marker'), false);
});

test('replay 由来の抑止は Hub が同じ候補を announce したら解ける', () => {
  resetSession(10);
  const identity = { candidateKey: 'hub-key-10', sourceEpoch: 1, shape: '' };
  // reattach_replay_done が「回答済み」として復元した状態。
  recordAnsweredApprovalIdentity(10, identity.candidateKey, identity.sourceEpoch, '', true);
  assert.equal(hasAnsweredToken(10, 1, 'hub-key-10'), true);
  // Hub が同じ候補をあらためて配信した = まだ承認待ちなので抑止を取り消す。
  clearReplayAnsweredApprovalCandidate(10, identity);
  assert.equal(hasAnsweredToken(10, 1, 'hub-key-10'), false);
});

// replay 由来でない（実際にこの画面で回答した）記録まで取り消すと、回答済みの
// 質問が再表示される。取り消しの対象は replay で復元した分だけに限る。
test('この画面で回答した記録は replay の取り消し対象にしない', () => {
  resetSession(11);
  const identity = { candidateKey: 'hub-key-11', sourceEpoch: 1, shape: '' };
  recordAnsweredApprovalIdentity(11, identity.candidateKey, identity.sourceEpoch, '', false);
  clearReplayAnsweredApprovalCandidate(11, identity);
  assert.equal(hasAnsweredToken(11, 1, 'hub-key-11'), true);
});

test('Hub が付けた candidateKey は shape より優先される', () => {
  resetSession(12);
  const opts = markerOptions('この変更を適用しますか?', ['はい', 'いいえ']);
  (opts as any)._candidateKey = 'hub-key-12';
  (opts as any)._sourceEpoch = 4;
  recordAnsweredApprovalCandidate(12, opts, 'marker');
  assert.equal(hasAnsweredToken(12, 4, 'hub-key-12'), true);
});

test('選択肢が空の入力は回答済みとして記録しない', () => {
  resetSession(13);
  assert.equal(recordAnsweredApprovalCandidate(13, [], 'marker'), null);
  assert.equal(isAnsweredApprovalCandidate(13, [], 'marker'), false);
});
