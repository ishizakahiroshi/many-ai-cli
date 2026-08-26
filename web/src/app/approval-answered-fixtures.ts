import assert from 'node:assert/strict';
import test from 'node:test';
import {
  _resetApprovalAnsweredStateForTest,
  answeredApprovalCandidates,
  approvalCandidateShape,
  clearReplayAnsweredApprovalCandidate,
  isAnsweredApprovalCandidate,
  isAnsweredApprovalShapeAcrossEpochs,
  isHubMarkerAuthoritative,
  isStaleHistoryRepaint,
  getApprovalSourceEpoch,
  noteApprovalSourceEpoch,
  noteHubMarkerDelivered,
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


// --- ページ送りで過去の画面が描き直されたときの判定（approval-ui.ts の showOptions が使う）---
//
// 代替画面バッファの provider ではホイールが PgUp として CLI へ届き、CLI が過去の位置を
// 描き直す。Hub は VT ミラー＝今の画面から承認を取り出すので、遡って読んでいるだけで
// 回答済みの承認が新しい世代の候補として届く。世代込みの判定では拾えないため、
// 「中身に一度でも答えたか」だけを見る経路を別に用意している。

test('世代が進んでいても、同じ中身に答えた記録は shape で拾える', () => {
  resetSession(15);
  const opts = markerOptions('この変更を適用しますか?', ['はい', 'いいえ']);
  const shape = approvalCandidateShape(15, opts, 'marker');
  recordAnsweredApprovalCandidate(15, opts, 'marker');
  noteApprovalSourceEpoch(15, 9);
  // 世代込みの判定は「新しい候補」と見る（意図的な再質問を出すための仕様）。
  assert.equal(isAnsweredApprovalCandidate(15, markerOptions('この変更を適用しますか?', ['はい', 'いいえ']), 'marker'), false);
  // 遡り表示中だけはこちらを見て、同じ中身の描き直しを出さない。
  assert.equal(isAnsweredApprovalShapeAcrossEpochs(15, shape), true);
});

// ここが false のままであることが、遡り中に届いた新しい承認を握り潰さない根拠。
test('答えたことのない中身は shape でも回答済みにならない', () => {
  resetSession(16);
  recordAnsweredApprovalCandidate(16, markerOptions('A を消しますか?', ['はい', 'いいえ']), 'marker');
  const other = markerOptions('B を消しますか?', ['はい', 'いいえ']);
  assert.equal(isAnsweredApprovalShapeAcrossEpochs(16, approvalCandidateShape(16, other, 'marker')), false);
  assert.equal(isAnsweredApprovalShapeAcrossEpochs(16, ''), false);
});

test('shape の回答済み判定はセッションを跨がない', () => {
  resetSession(17);
  resetSession(18);
  const opts = markerOptions('この変更を適用しますか?', ['はい', 'いいえ']);
  const shape = approvalCandidateShape(17, opts, 'marker');
  recordAnsweredApprovalCandidate(17, opts, 'marker');
  assert.equal(isAnsweredApprovalShapeAcrossEpochs(18, shape), false);
});

// --- bugfix_approval-waiting-stuck-after-answer_2026-08-26.md の回帰 ---

test('Hub がこの世代のマーカーを配信していれば、ローカル走査は候補を立てない', () => {
  resetSession(19);
  noteHubMarkerDelivered(19, getApprovalSourceEpoch(19));
  assert.equal(isHubMarkerAuthoritative(19), true);
});

// Hub が何も配信できていない世代（開始マーカーが画面外にある等）では、
// ローカル走査が最後の砦として働き続ける必要がある。
test('Hub が何も配信していなければ、ローカル走査は従来どおり候補を立てられる', () => {
  resetSession(20);
  assert.equal(isHubMarkerAuthoritative(20), false);
});

test('世代が進んだら Hub の正本扱いは持ち越さない', () => {
  resetSession(21);
  noteHubMarkerDelivered(21, getApprovalSourceEpoch(21));
  noteApprovalSourceEpoch(21, getApprovalSourceEpoch(21) + 1);
  assert.equal(isHubMarkerAuthoritative(21), false);
});

// 2026-08-26 の実測そのもの。回答済みの承認が、差分再描画で 1 文字欠けた本文として
// 再パースされると回答済み台帳を外す。Hub 側は同じ入力で候補キーが揺れていないので、
// この世代ではローカル走査から候補を立てないことで再点灯を止める。
test('1 文字欠けた再パースは回答済み台帳を外すが、Hub 正本の世代では候補にならない', () => {
  resetSession(22);
  const answered = markerOptions('本番 mer へ SSH して read-only の確認クエリを流してよいですか', ['いいえ', 'はい'], ['0', '1']);
  recordAnsweredApprovalCandidate(22, answered, 'marker');
  noteHubMarkerDelivered(22, getApprovalSourceEpoch(22));

  const corrupted = markerOptions('本番 mer へ SSH て read-only の確認クエリを流してよいですか', ['いいえ', 'はい'], ['0', '1']);
  // 台帳は外れる（ここが症状の起点。shape の完全一致で引くため）。
  assert.equal(isAnsweredApprovalCandidate(22, corrupted, 'marker'), false);
  // それでもローカル走査からは立てないので 保留中 は戻らない。
  assert.equal(isHubMarkerAuthoritative(22), true);
});

test('遡り表示中に届いた回答済みの中身だけを落とす', () => {
  resetSession(23);
  const opts = markerOptions('この変更を適用しますか?', ['はい', 'いいえ']);
  const shape = approvalCandidateShape(23, opts, 'marker');
  recordAnsweredApprovalCandidate(23, opts, 'marker');
  noteApprovalSourceEpoch(23, 9);

  // 遡っていなければ落とさない（世代が進んだ再質問は出す仕様のまま）。
  assert.equal(isStaleHistoryRepaint(23, shape, false), false);
  // 遡り中で、かつ一度でも答えた中身なら落とす。
  assert.equal(isStaleHistoryRepaint(23, shape, true), true);
  // 遡り中でも、答えたことのない中身は落とさない（F-12 型の握り潰しを避ける）。
  const fresh = markerOptions('別の質問ですか?', ['はい', 'いいえ']);
  assert.equal(isStaleHistoryRepaint(23, approvalCandidateShape(23, fresh, 'marker'), true), false);
});
