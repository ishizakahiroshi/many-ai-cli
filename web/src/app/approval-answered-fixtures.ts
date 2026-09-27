import assert from 'node:assert/strict';
import test from 'node:test';
import {
  _resetApprovalAnsweredStateForTest,
  approvalCandidateShape,
  forgetAnsweredApprovals,
  isAnsweredApprovalIdentity,
  isAnsweredApprovalShapeAcrossEpochs,
  isStaleHistoryRepaint,
  recordAnsweredApprovalIdentity,
  setApprovalProviderResolver,
} from './approval-answered.js';

// provider は本番では sessions から引く。ここでは固定値を差して DOM 抜きで試す。
const providers = new Map<number, string>();
setApprovalProviderResolver((id) => providers.get(id) || '');

function resetSession(id: number, provider = 'claude'): void {
  _resetApprovalAnsweredStateForTest(id);
  providers.set(id, provider);
}

/** Hub のマーカーの記録を画面のパーサが選択肢にした形の最小再現。label は表示用、_sendText が送信実体。 */
function markerOptions(question: string, labels: string[], sendTexts?: string[]): any[] {
  const opts: any = labels.map((label, i) => ({
    num: i + 1,
    label,
    _sendText: sendTexts ? sendTexts[i] : String(i + 1),
  }));
  opts._question = question;
  return opts;
}

// 回答済みの印は Hub の記録の同一性（candidate_key + source_epoch）そのもので付ける。
// 画面は同一性を組み立て直さない（approval-answered.ts の冒頭）。

test('同じ記録には回答済みの印が付き、描き直さない', () => {
  resetSession(1);
  assert.equal(isAnsweredApprovalIdentity(1, 'hub-key-1', 3), false);
  recordAnsweredApprovalIdentity(1, 'hub-key-1', 3);
  assert.equal(isAnsweredApprovalIdentity(1, 'hub-key-1', 3), true);
});

// 撤去した approvalConsumedSig は 5〜10 秒のタイマーで失効していた。世代で持つ
// 現在の state は時間で消えないので、何秒後に同じ記録が届いても描かない。
test('回答済みの印は時間で失効しない', async () => {
  resetSession(2);
  recordAnsweredApprovalIdentity(2, 'hub-key-2', 1);
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(isAnsweredApprovalIdentity(2, 'hub-key-2', 1), true);
});

test('世代が進めば同じ候補でも新しい記録として描く', () => {
  resetSession(3);
  recordAnsweredApprovalIdentity(3, 'hub-key-3', 1);
  // 新しい live prompt 境界で Hub が同じ質問を出し直した場合（意図的な再質問）。
  assert.equal(isAnsweredApprovalIdentity(3, 'hub-key-3', 2), false);
});

test('別の候補には回答済みの印が付かない', () => {
  resetSession(4);
  recordAnsweredApprovalIdentity(4, 'hub-key-4a', 1);
  assert.equal(isAnsweredApprovalIdentity(4, 'hub-key-4b', 1), false);
});

test('回答済みの印はセッションを跨がない', () => {
  resetSession(5);
  resetSession(6);
  recordAnsweredApprovalIdentity(5, 'hub-key-shared', 1);
  assert.equal(isAnsweredApprovalIdentity(6, 'hub-key-shared', 1), false);
});

test('同一性の欠けた入力は回答済みとして記録しない', () => {
  resetSession(7);
  recordAnsweredApprovalIdentity(7, '', 1);
  recordAnsweredApprovalIdentity(7, 'hub-key-7', 0);
  assert.equal(isAnsweredApprovalIdentity(7, '', 1), false);
  assert.equal(isAnsweredApprovalIdentity(7, 'hub-key-7', 0), false);
});

test('セッションを消したら回答済みの印も消える', () => {
  resetSession(8);
  const shape = approvalCandidateShape(8, markerOptions('この変更を適用しますか?', ['はい', 'いいえ']), 'marker');
  recordAnsweredApprovalIdentity(8, 'hub-key-8', 1, shape);
  forgetAnsweredApprovals(8);
  assert.equal(isAnsweredApprovalIdentity(8, 'hub-key-8', 1), false);
  assert.equal(isAnsweredApprovalShapeAcrossEpochs(8, shape), false);
});

// --- 形（shape）: provider・種別・質問・選択肢番号と送信文字列。世代を含まない ---

test('TUI の再描画でラベルが揺れても形は変わらない', () => {
  resetSession(9);
  const original = approvalCandidateShape(9, markerOptions('この変更を適用しますか?', ['はい', 'いいえ']), 'marker');
  // 折返し・余白・罫線の混入でラベルだけが変わった再描画。番号と送信文字列は同じ。
  const redrawn = approvalCandidateShape(9, markerOptions('この変更を適用しますか?  ', ['は い', '── いいえ']), 'marker');
  assert.equal(redrawn, original);
});

test('質問文・送信文字列・種別が違えば形も違う', () => {
  resetSession(10);
  const base = approvalCandidateShape(10, markerOptions('A を消しますか?', ['A', 'B'], ['a', 'b']), 'marker');
  assert.notEqual(approvalCandidateShape(10, markerOptions('B を消しますか?', ['A', 'B'], ['a', 'b']), 'marker'), base);
  assert.notEqual(approvalCandidateShape(10, markerOptions('A を消しますか?', ['A', 'B'], ['a', 'c']), 'marker'), base);
  // マーカーとネイティブは同じ質問文でも別の中身（片方の回答でもう片方を隠さない）。
  assert.notEqual(approvalCandidateShape(10, markerOptions('A を消しますか?', ['A', 'B'], ['a', 'b']), 'native'), base);
});

// --- ページ送りで過去の画面が描き直されたときの判定（approval-ui.ts の showOptions が使う）---
//
// 代替画面バッファの provider ではホイールが PgUp として CLI へ届き、CLI が過去の位置を
// 描き直す。Hub は VT ミラー＝今の画面から承認を取り出すので、遡って読んでいるだけで
// 回答済みの承認が新しい世代の記録として届く。世代込みの判定では拾えないため、
// 「中身に一度でも答えたか」だけを見る経路を別に用意している。

test('世代が進んでいても、同じ中身に答えた記録は shape で拾える', () => {
  resetSession(15);
  const shape = approvalCandidateShape(15, markerOptions('この変更を適用しますか?', ['はい', 'いいえ']), 'marker');
  recordAnsweredApprovalIdentity(15, 'hub-key-15', 1, shape);
  // 世代込みの判定は「新しい記録」と見る（意図的な再質問を出すための仕様）。
  assert.equal(isAnsweredApprovalIdentity(15, 'hub-key-15', 9), false);
  // 遡り表示中だけはこちらを見て、同じ中身の描き直しを出さない。
  assert.equal(isAnsweredApprovalShapeAcrossEpochs(15, shape), true);
});

// ここが false のままであることが、遡り中に届いた新しい承認を握り潰さない根拠。
test('答えたことのない中身は shape でも回答済みにならない', () => {
  resetSession(16);
  const answered = approvalCandidateShape(16, markerOptions('A を消しますか?', ['はい', 'いいえ']), 'marker');
  recordAnsweredApprovalIdentity(16, 'hub-key-16', 1, answered);
  const other = markerOptions('B を消しますか?', ['はい', 'いいえ']);
  assert.equal(isAnsweredApprovalShapeAcrossEpochs(16, approvalCandidateShape(16, other, 'marker')), false);
  assert.equal(isAnsweredApprovalShapeAcrossEpochs(16, ''), false);
});

test('shape の回答済み判定はセッションを跨がない', () => {
  resetSession(17);
  resetSession(18);
  const shape = approvalCandidateShape(17, markerOptions('この変更を適用しますか?', ['はい', 'いいえ']), 'marker');
  recordAnsweredApprovalIdentity(17, 'hub-key-17', 1, shape);
  assert.equal(isAnsweredApprovalShapeAcrossEpochs(18, shape), false);
});

test('遡り表示中に届いた回答済みの中身だけを落とす', () => {
  resetSession(23);
  const shape = approvalCandidateShape(23, markerOptions('この変更を適用しますか?', ['はい', 'いいえ']), 'marker');
  recordAnsweredApprovalIdentity(23, 'hub-key-23', 1, shape);

  // 遡っていなければ落とさない（世代が進んだ再質問は出す仕様のまま）。
  assert.equal(isStaleHistoryRepaint(23, shape, false), false);
  // 遡り中で、かつ一度でも答えた中身なら落とす。
  assert.equal(isStaleHistoryRepaint(23, shape, true), true);
  // 遡り中でも、答えたことのない中身は落とさない（F-12 型の握り潰しを避ける）。
  const fresh = markerOptions('別の質問ですか?', ['はい', 'いいえ']);
  assert.equal(isStaleHistoryRepaint(23, approvalCandidateShape(23, fresh, 'marker'), true), false);
});
