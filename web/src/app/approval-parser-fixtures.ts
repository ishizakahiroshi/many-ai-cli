// 画面のパーサは Hub の記録のブロックを選択肢にするだけ（approval-parser.ts の冒頭）。
// マーカー無しの Yes/No・旧形式の選択・推測での拾い上げ・複数質問の判定は Hub（Go）だけにあり、
// その入力と期待値は internal/hub/approval_text_question_test.go / approval_detector_test.go が持つ
// （2026-09-23 にここから移した。docs/local/plan_approval-display-single-source_c3_web-switch.md の C3）。
// 順次質問のパーサは両方にあり、Hub は記録の Block をこちらと同じ形で作る。片方を直すときはもう片方も直す。
import assert from 'node:assert/strict';
import test from 'node:test';
import { approvalParser as parser } from './approval-parser.js';

function labels(options) {
  return (options || []).map(o => o.label);
}

function numbers(options) {
  return (options || []).map(o => o.num);
}

// 選択肢の組の見た目の同一性（番号・ラベル・Yes/No の質問ハッシュ）。別の行の分け方から
// 同じ選択肢が取れているかを比べる。
function optionsSig(options) {
  return JSON.stringify((options || []).map(o => `${o.num}:${String(o.label || '').trim().replace(/\s+/g, ' ')}|${o._ctx || ''}`));
}

test('approval parser fixtures', () => {
  const hub = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'Proceed with this change? (Y:1/N:0)',
    '[/MANY-AI-CLI]',
  ]);
  assert.deepEqual(numbers(hub), [1, 0]);
  assert.equal(optionsSig(hub), optionsSig(parser.extractHubMarkerApproval([
    '[MANY-AI-CLI] Proceed with this change? (Y:1/N:0) [/MANY-AI-CLI]',
  ])));

  assert.equal(parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'question? (Y:1/N:0)',
    '[/MANY-AI-CLI]',
  ]), null);

  // マーカー内側で、最初の質問より前に置かれた地の文（前置き説明）を _preamble として取り出す。
  const withPreamble = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'そのうえで、判断が要るものがあります。',
    'License 不在時は自動生成しない。',
    'Proceed? (Y:1/N:0)',
    '[/MANY-AI-CLI]',
  ]);
  assert.deepEqual(numbers(withPreamble), [1, 0]);
  assert.equal(
    (withPreamble as any)._preamble,
    'そのうえで、判断が要るものがあります。\nLicense 不在時は自動生成しない。',
  );
  assert.equal((withPreamble as any)._question, 'Proceed?');

  // マーカー外のスピナー再描画ノイズや古い前置きは _preamble に混ぜない。
  const outsideNoise = parser.extractHubMarkerApproval([
    '\x1b[38;2;53;153;153m⠂⠐⠠⠐⠂ 実行: plan_xxx.md',
    'g…lg…ln…ei…vl…',
    '次の前置き。',
    '[MANY-AI-CLI]',
    'Proceed? (Y:1/N:0)',
    '[/MANY-AI-CLI]',
  ]);
  assert.deepEqual(numbers(outsideNoise), [1, 0]);
  assert.equal((outsideNoise as any)._preamble, undefined);

  // 単一ブロックで選択肢より前に置かれた質問文を _question として取り出す（見出し Q1 形式でなくても）。
  // 承認ポップアップに質問本文を出すための捕捉（CLI 画面を見ずに何を聞かれているか分かるように）。
  const singleQ = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'path-exists の挙動をどうしますか？',
    '1. 実在判定可にする (Recommended)',
    '2. 許可リストを維持',
    'N. User specifies',
    '[/MANY-AI-CLI]',
  ]);
  assert.deepEqual(numbers(singleQ), [1, 2]);
  assert.equal((singleQ as any)._question, 'path-exists の挙動をどうしますか？');
  assert.equal((singleQ as any)._preamble, 'path-exists の挙動をどうしますか？');
  assert.equal((singleQ as any)._freeInput, true);

  // マーカー文字列が質問/ラベルに漏れ込んだ壊れたパース結果は出さない（Grok 再描画残骸）。
  assert.equal(parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'スターター取込？ [MANY-AI-CLI] スターター取込？',
    '1. A',
    '2. B',
    '[/MANY-AI-CLI]',
  ]), null);

  // 複数行に折り返された質問文は 1 つに連結して捕捉する。
  const singleQWrapped = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'とても長い質問の前半部分が',
    '次の行に折り返されている場合？',
    '1. A',
    '2. B',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal((singleQWrapped as any)._question, 'とても長い質問の前半部分が 次の行に折り返されている場合？');

  // Yes/No 単一質問も質問本文を _question として取り出す。
  const ynQ = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'この変更を適用しますか？ (Y:1/N:0)',
    '[/MANY-AI-CLI]',
  ]);
  assert.deepEqual(numbers(ynQ), [1, 0]);
  assert.equal((ynQ as any)._question, 'この変更を適用しますか？');

  const seq = parser.extractSequentialChoicePrompts([
    'Q1: Choose branch',
    '  1. main',
    '  2. develop',
    '  N. User specifies',
    'Q2: Run tests',
    '  1. Yes',
    '  2. No',
    '  N. User specifies',
  ]);
  assert.equal(seq.length, 2);
  assert.equal(parser.sequentialChoiceSig(seq), parser.sequentialChoiceSig(parser.extractSequentialChoicePrompts(seq.flatMap(p => [
    `${p.key}: ${p.question}`,
    ...p.options.map(o => `  ${o.num}. ${o.label}`),
  ]))));

  const batch = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    '1 first question?',
    ' 1. Approve',
    ' 2. Deny',
    '2 second question?',
    ' 1. Approve',
    ' 2. Deny',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(batch), true);
  assert.equal(batch.length, 2);

  const batchWithPreamble = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'この変更は承認方式の表示だけに影響します。',
    '既存セッションの入力処理は変えません。',
    'Q1 first question?',
    ' 1. Approve',
    ' 2. Deny',
    'Q2 second question?',
    ' 1. Approve',
    ' 2. Deny',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(batchWithPreamble), true);
  assert.equal(batchWithPreamble.length, 2);
  assert.equal(
    (batchWithPreamble as any)._preamble,
    'この変更は承認方式の表示だけに影響します。\n既存セッションの入力処理は変えません。',
  );

  // 見出しの `Q1`/`Q2` プレフィックス対応（質問番号と選択肢番号の混同解消）。
  // 旧 `1 質問?` 形式（上の batch）と同様にバッチ復元できること。
  const qPrefixBatch = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'Q1 first question?',
    ' 1. Approve',
    ' 2. Deny',
    'Q2 second question?',
    ' 1. Approve',
    ' 2. Deny',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(qPrefixBatch), true);
  assert.equal(qPrefixBatch.length, 2);
  assert.equal(qPrefixBatch[0].num, 1);
  assert.equal(qPrefixBatch[1].num, 2);
  assert.deepEqual(labels(qPrefixBatch[0].options), ['Approve', 'Deny']);

  // 区切りゆれ（`Q1:` / `Q2.`）も見出しとして受理する。
  const qPrefixSep = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'Q1: 好きな麺は？',
    ' 1. うどん',
    ' 2. そば',
    'Q2. 好きな主食は？',
    ' 1. 白米',
    ' 2. パン',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(qPrefixSep), true);
  assert.equal(qPrefixSep.length, 2);
  assert.deepEqual(labels(qPrefixSep[0].options), ['うどん', 'そば']);

  const japaneseBatch = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    '',
    '1 次のうちどれが好きですか？',
    '',
    '  1.たこ',
    '',
    '  2.いか',
    '',
    '  3.えび',
    '',
    '2 次のうちどれが好きですか？',
    '',
    '  1.白米',
    '',
    '  2.パン',
    '',
    '  3.うどん',
    '',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(japaneseBatch), true);
  assert.equal(japaneseBatch.length, 2);
  assert.deepEqual(labels(japaneseBatch[0].options), ['たこ', 'いか', 'えび']);
  assert.deepEqual(labels(japaneseBatch[1].options), ['白米', 'パン', 'うどん']);

  // TUI 再描画で改行が抜け、次の質問見出しが直前の行へ連結されたケースの回帰。
  // 「N. User specifies」を区切りに再分割し、3 質問へ正しく復元できること。
  const glued = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    '1 マーカー指示の配置先は？',
    '1. AGENTS.md に1ブロック集約',
    '2. provider固有ファイルに分離（cursor=.cursor/rules/） N. User specifies 2 共有ブロックの削除タイミングは？',
    '3. 参照管理',
    '4. 常駐',
    'N. User specifies 3 今回の進め方は？',
    '5. まずplanに整理',
    '6. そのまま実装',
    'N. User specifies',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(glued), true);
  assert.equal(glued.length, 3);
  assert.deepEqual(glued.map(s => s.title), [
    'マーカー指示の配置先は？',
    '共有ブロックの削除タイミングは？',
    '今回の進め方は？',
  ]);
  assert.deepEqual(numbers(glued[0].options), [1, 2]);
  assert.deepEqual(numbers(glued[1].options), [3, 4]);
  assert.deepEqual(numbers(glued[2].options), [5, 6]);

  // 大きなマーカーブロックの回帰（行数非依存の検証）。複数質問・長い日本語ラベルが端末幅で
  // 折り返されるとブロックは容易に数十〜百行へ膨らむ。抽出は末尾 N 行固定窓で切らず
  // [MANY-AI-CLI]…[/MANY-AI-CLI] をアンカーでブロックごと取るため、ブロックがどれだけ大きくても
  // 開始マーカーを取りこぼさない。折り返し継続行（数字始まりでない行）は直前の選択肢ラベルへ
  // 結合される性質を使い、各選択肢に 30 行の継続を付けてブロックを ~190 行へ意図的に肥大化させ、
  // それでも 2 質問が両方とも復元されることを確認する（旧 40 行窓・暫定 240 行窓いずれも超える）。
  const wrapCont = (n) => Array.from({ length: n }, (_, i) => `（折り返し継続${i}）`);
  const bigBlock = [
    '[MANY-AI-CLI]',
    '1 監査対象（対象範囲）はどれにしますか？',
    ' 1. SAB 本体に絞る',
    ...wrapCont(30),
    ' 2. 全 Go サービス',
    ...wrapCont(30),
    ' 3. 特定の1サービスのみ',
    ...wrapCont(30),
    ' N. User specifies',
    '2 スコープ（修正までやるか）はどうしますか？',
    ' 4. 調査→検証で終了',
    ...wrapCont(30),
    ' 5. finding 報告のみ',
    ...wrapCont(30),
    ' 6. 修正＋再調査ループまで完走',
    ...wrapCont(30),
    ' N. User specifies',
    '[/MANY-AI-CLI]',
  ];
  assert.ok(bigBlock.length > 180, 'regression block must dwarf any fixed-size window');
  const bigBatch = parser.extractHubMarkerApproval(bigBlock);
  assert.equal(parser.isBatchOptions(bigBatch), true);
  assert.equal(bigBatch.length, 2);
  assert.deepEqual(numbers(bigBatch[0].options), [1, 2, 3]);
  assert.deepEqual(numbers(bigBatch[1].options), [4, 5, 6]);

  // 見出し（質問文）が option 1 と同一行へ連結され、option 2 以降が別行に残ったケースの回帰。
  // Ink 再描画で改行が抜けると「質問? 1. A」の 1 行になり、従来は option 1 が丸ごと捨てられて
  // 「選択肢の 1 が無い」症状になっていた。見出しを切り離し option 1 を復元できること。
  const gluedHeadingOpt1 = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'どの方式にしますか? 1. A方式 (Recommended)',
    '2. B方式',
    'N. User specifies',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(gluedHeadingOpt1), false);
  assert.deepEqual(numbers(gluedHeadingOpt1), [1, 2]);
  assert.deepEqual(labels(gluedHeadingOpt1), ['A方式 (Recommended)', 'B方式']);

  // 見出し末尾「…ますか?」の直後へ空白なしで option 1 が連結（「?1.」）し、さらに option 2 まで
  // 同一行に巻き込まれたケースの回帰。xterm ハードラップで改行と行頭空白が同時に落ちると発生する。
  // 文末記号「?」を分割境界に含めないと option 1/2 が見出しへ飲み込まれ、タブ見出しが
  // 「質問文＋選択肢1＋選択肢2」に化け、残りの番号からしかボタンが作られない症状になっていた。
  const gluedAfterQuestionMark = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    '1 スマホでセッションを「作成」した直後、画面はどうなりますか?1. 何も変わらない（作成前のセッション/画面のまま）(Recommended)2. 黒いターミナル画面に切り替わるが中身が空/止まっている',
    '3. エラーのトースト/赤いメッセージが一瞬出る',
    'N. User specifies',
    '2 スマホで ☰（左上メニュー）を開き直すと、作ったセッションは一覧に出ていますか?',
    '4. 一覧に出てこない（古いセッションだけ）(Recommended)',
    '5. 一覧には出るが、タップしても開けない',
    '6. 一覧に出る（PCと同じに見える）',
    'N. User specifies',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(gluedAfterQuestionMark), true);
  assert.equal(gluedAfterQuestionMark.length, 2);
  assert.equal(gluedAfterQuestionMark[0].title, 'スマホでセッションを「作成」した直後、画面はどうなりますか?');
  assert.deepEqual(numbers(gluedAfterQuestionMark[0].options), [1, 2, 3]);
  assert.equal(gluedAfterQuestionMark[0].options[0].label, '何も変わらない（作成前のセッション/画面のまま）(Recommended)');
  assert.deepEqual(numbers(gluedAfterQuestionMark[1].options), [4, 5, 6]);
  assert.equal(gluedAfterQuestionMark[0]._freeInput, true);

  // 単一質問でも「?1.」連結（空白なし）で option 1 が欠落していた回帰。
  // 文末記号を境界に含めることで marks=[1] の救済分岐（marks.length===1 && num===1）が働く。
  const gluedSingleAfterQ = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'どの方式にしますか?1. A方式 (Recommended)',
    '2. B方式',
    'N. User specifies',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(gluedSingleAfterQ), false);
  assert.deepEqual(numbers(gluedSingleAfterQ), [1, 2]);
  assert.deepEqual(labels(gluedSingleAfterQ), ['A方式 (Recommended)', 'B方式']);

  // 差分再描画型 TUI（Grok CLI・2026-07-12 実測）の回帰。行を絶対カーソル移動（CUP）で描画し、
  // 前フレームと同じセル（番号直後の「. 」）をカーソル前進（CUF）でスキップするため、
  // 単純な ANSI 削除だと「…(Recommended)2手動…3スキップ…」に連結されピリオドも失われる。
  // normalizeVtCursorOps が CUP→改行 / CUF→空白 / ECH→空白 に変換すること。
  assert.equal(parser.normalizeVtCursorOps('A\x1b[2;5HB'), 'A\nB');
  assert.equal(parser.normalizeVtCursorOps('2\x1b[2C原因'), '2  原因');
  assert.equal(parser.normalizeVtCursorOps('末尾\x1b[43X続き'), '末尾 続き');
  // SGR（色指定）は変換対象外（後段の stripAnsi が落とす）。
  assert.equal(parser.normalizeVtCursorOps('\x1b[38;2;1;2;3mA'), '\x1b[38;2;1;2;3mA');

  // 実ログの VT 構造を合成データで再現（マーカー行 + 空行クリア + 選択肢 3 行 + N 行連結）。
  // 選択肢 2・3 は番号のみ再描画され「. 」が CUF スキップで届かない形。
  const grokDiffRepaint =
    '\x1b[18;6H[MANY-AI-CLI]\x1b[1C設定ファイルの移行方式を選んでください？' +
    '\x1b[19;6H\x1b[13X\x1b[14C\x1b[83X' +
    '\x1b[20;6H1. 自動で移行する (Recommended)\x1b[32X' +
    '\x1b[21;6H2\x1b[2C手動で移行する（後で案内）\x1b[34X' +
    '\x1b[22;6H3\x1b[2Cスキップの手順だけ案内 N. User specifies\x1b[1C[/MANY-AI-CLI]';
  const diffRepaint = parser.extractHubMarkerApproval(
    parser.normalizeVtCursorOps(grokDiffRepaint).split(/\r\n|\r|\n/),
  );
  assert.equal(parser.isBatchOptions(diffRepaint), false);
  assert.deepEqual(numbers(diffRepaint), [1, 2, 3]);
  assert.deepEqual(labels(diffRepaint), [
    '自動で移行する (Recommended)',
    '手動で移行する（後で案内）',
    'スキップの手順だけ案内',
  ]);
  assert.equal((diffRepaint as any)._freeInput, true);
  assert.equal((diffRepaint as any)._question, '設定ファイルの移行方式を選んでください？');

  // 複数選択（#multi）: 1 問で任意個 ON/OFF。options に _multiSelect と _question が付き、
  // isMultiSelectOptions が true、isBatchOptions は false になること。
  const multi = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    '#multi 下バーに追加したい情報は？',
    '1. コンテキスト使用率',
    '2. 承認待ちバッジ',
    '3. キャッシュ率',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isMultiSelectOptions(multi), true);
  assert.equal(parser.isBatchOptions(multi), false);
  assert.equal(multi.length, 3);
  assert.deepEqual(numbers(multi), [1, 2, 3]);
  assert.deepEqual(labels(multi), ['コンテキスト使用率', '承認待ちバッジ', 'キャッシュ率']);
  assert.equal(multi[0]._question, '下バーに追加したい情報は？');

  const multiWithPreamble = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    '表示密度だけに関わる設定です。',
    '#multi 下バーに追加したい情報は？',
    '1. コンテキスト使用率',
    '2. 承認待ちバッジ',
    '3. キャッシュ率',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isMultiSelectOptions(multiWithPreamble), true);
  assert.deepEqual(numbers(multiWithPreamble), [1, 2, 3]);
  assert.equal((multiWithPreamble as any)._preamble, '表示密度だけに関わる設定です。');
  assert.equal(multiWithPreamble[0]._question, '下バーに追加したい情報は？');

  // #multi が無い同形の番号付きリストは単一選択（multiSelect ではない）のまま。
  const singleSelect = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'どれにしますか？',
    '1. A',
    '2. B',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isMultiSelectOptions(singleSelect), false);

  // marker 経路でブロック全体が1行へ完全に潰れたケースの回帰。
  // 見出し「1 …?」と選択肢「1. 2. 3.」が混在連結されても 3 選択肢へ復元できること。
  const gluedMarker = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI] 1 「2」はどの設定を指していますか? 1. A を選択(Recommended)2. B を選択 3. 両方とも C N. User specifies [/MANY-AI-CLI]',
  ]);
  assert.deepEqual(numbers(gluedMarker), [1, 2, 3]);

  const chunkPath = parser.extractHubMarkerApproval([
    'noise',
    '[MANY-AI-CLI]',
    'Proceed? (Y:1/N:0)',
    '[/MANY-AI-CLI]',
  ]);
  const bufferPath = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'Proceed? (Y:1/N:0)',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(optionsSig(chunkPath), optionsSig(bufferPath));

  // xterm がラベル/見出し途中で物理行に折り返し、続き行が数字始まりにならないケース。
  // 続き行を直前の選択肢ラベル・見出しタイトルへ結合し、「N. User specifies」は混入させない。
  const wrappedBatch = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    '1 codex-spawn-zombie の方針は?',
    '1. 案2 watchdog を実装（無言固着を「起動失敗」表示',
    'に）(Recommended)',
    '2. blocker 降格して defer（再',
    '発時に着手）',
    'N. User specifies',
    '2 detached-session-grid と security-audit（未着手・索',
    '引未掲載）は?',
    '3. consolidated-3 ④ に追記して追跡下に置く（securit',
    'y-audit は [保留] 降格）(Recommended)',
    '4. 触らない',
    'N. User specifies',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(wrappedBatch), true);
  assert.equal(wrappedBatch.length, 2);
  assert.equal(wrappedBatch[0].title, 'codex-spawn-zombie の方針は?');
  assert.deepEqual(numbers(wrappedBatch[0].options), [1, 2]);
  assert.equal(wrappedBatch[0].options[0].label, '案2 watchdog を実装（無言固着を「起動失敗」表示に）(Recommended)');
  assert.equal(wrappedBatch[0].options[1].label, 'blocker 降格して defer（再発時に着手）');
  assert.equal(wrappedBatch[1].title, 'detached-session-grid と security-audit（未着手・索引未掲載）は?');
  assert.equal(wrappedBatch[1].options[0].label, 'consolidated-3 ④ に追記して追跡下に置く（security-audit は [保留] 降格）(Recommended)');
  assert.deepEqual(numbers(wrappedBatch[1].options), [3, 4]);

  // 一括質問タブUI（plan_choice-tab-ui.md C1）: 選択肢先頭の `[短ラベル]` を
  // shortLabel/label に分離し、「N. User specifies」がある質問は _freeInput=true になること。
  const shortLabelBatch = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    '1 短ラベルの生成方法は？',
    '1. [AI付与] AI 側が各選択肢に短ラベルを付ける',
    '2. [Web短縮] Web 側で自動短縮する',
    'N. User specifies',
    '2 タブ溢れ時は？',
    '3. 横スクロール',
    '4. 折り返し',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(shortLabelBatch), true);
  assert.equal(shortLabelBatch.length, 2);
  assert.equal(shortLabelBatch[0].options[0].shortLabel, 'AI付与');
  assert.equal(shortLabelBatch[0].options[0].label, 'AI 側が各選択肢に短ラベルを付ける');
  assert.equal(shortLabelBatch[0].options[1].shortLabel, 'Web短縮');
  assert.equal(shortLabelBatch[0].options[1].label, 'Web 側で自動短縮する');
  // 「N. User specifies」がある質問は自由入力フラグが立つ
  assert.equal(shortLabelBatch[0]._freeInput, true);
  // 短ラベル表記が無い選択肢は shortLabel 未設定・label そのまま
  assert.equal(shortLabelBatch[1].options[0].shortLabel, undefined);
  assert.equal(shortLabelBatch[1].options[0].label, '横スクロール');
  // 「N. User specifies」が無い質問は _freeInput が立たない
  assert.equal(!!shortLabelBatch[1]._freeInput, false);

  // 単一質問もタブUIへ統合（plan_choice-tab-ui.md C5）: looseOpts でも短ラベルを分離し、
  // 「N. User specifies」があれば配列プロパティ _freeInput=true が立つ（構造は flat のまま）。
  const singleWithBracket = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'どれにしますか？',
    '1. [保留] そのまま',
    '2. 進める',
    'N. User specifies',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(singleWithBracket), false);
  assert.equal(singleWithBracket[0].shortLabel, '保留');
  assert.equal(singleWithBracket[0].label, 'そのまま');
  // 短ラベル表記が無い選択肢は shortLabel 未設定・label そのまま
  assert.equal(singleWithBracket[1].shortLabel, undefined);
  assert.equal(singleWithBracket[1].label, '進める');
  // 自由入力フラグは配列プロパティで保持される
  assert.equal((singleWithBracket as any)._freeInput, true);

  // 「N. User specifies」が無い単一質問では _freeInput が立たない
  const singleNoFree = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'どれにしますか？',
    '1. そのまま',
    '2. 進める',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(singleNoFree), false);
  assert.equal(!!(singleNoFree as any)._freeInput, false);

  // 単一質問でも `Q1 見出し` 形式のときは flat に落とした後も _freeInput / _question が残る
  // （sections[0] 経由でフラグを配列プロパティへ伝播する経路の回帰防止）。
  const singleQ1Heading = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'Q1 次のアクション',
    '1. 続行する (Recommended)',
    '2. やめる',
    'N. User specifies',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(singleQ1Heading), false);
  assert.deepEqual(numbers(singleQ1Heading), [1, 2]);
  assert.equal((singleQ1Heading as any)._freeInput, true);
  assert.equal((singleQ1Heading as any)._question, '次のアクション');

  // 選択肢本文に `Q1` を含む自然文（`Q1フォームを外し…` = 「Q1 のフォームを外し」の意）を
  // 疑似見出しとして分離すると、Q1 セクションが複数生成されて質問ポップアップが重複する
  // （bugfix_hub-approval-q-split-inside-option_2026-07-24.md の再発防止）。
  // 全 AI エージェント共通のマーカー規約なので、この防御は provider を問わず効く。
  const qInsideOption = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'Q1 どうする？（複数選択可）',
    '1. [HTML修正] Q1フォームを外し、「再実行で即時反映」の記述を訂正 (Recommended)',
    '2. [課題化] pending/plan として起票',
    '3. [今は口頭共有のみ] HTMLはこのまま',
    'N. User specifies',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(qInsideOption), false);
  assert.deepEqual(numbers(qInsideOption), [1, 2, 3]);
  assert.equal((qInsideOption as any)._question, 'どうする？（複数選択可）');
  assert.equal((qInsideOption as any)._freeInput, true);

});

// Detached Session Grid URL の layout parse ロジックを検証する。
// detached-grid.ts の parseDetachedGridParams は window.location.search に依存するため
// Node.js 環境では直接 import できない。レイアウト文字列のパースロジックを
// インラインで抽出して境界値を検証する。
test('detached-grid layout parse', () => {
  // detached-grid.ts の parseDetachedGridParams 内の layout parse ロジックと同等
  function parseLayout(layoutRaw: string): { cols: number; rows: number } {
    const match = /^(\d+)x(\d+)$/i.exec(layoutRaw || '2x2');
    let cols = 2;
    let rows = 2;
    if (match) {
      cols = Math.max(1, Math.min(6, parseInt(match[1], 10)));
      rows = Math.max(1, Math.min(3, parseInt(match[2], 10)));
    }
    return { cols, rows };
  }

  // 正常ケース
  assert.deepEqual(parseLayout('2x2'), { cols: 2, rows: 2 });
  assert.deepEqual(parseLayout('3x3'), { cols: 3, rows: 3 });
  assert.deepEqual(parseLayout('1x1'), { cols: 1, rows: 1 });
  assert.deepEqual(parseLayout('1x2'), { cols: 1, rows: 2 });
  assert.deepEqual(parseLayout('2x3'), { cols: 2, rows: 3 });
  assert.deepEqual(parseLayout('6x3'), { cols: 6, rows: 3 });

  // 上限クリップ: cols max=6, rows max=3
  assert.deepEqual(parseLayout('9x9'), { cols: 6, rows: 3 });
  assert.deepEqual(parseLayout('7x4'), { cols: 6, rows: 3 });

  // 下限クリップ: cols min=1, rows min=1
  assert.deepEqual(parseLayout('0x0'), { cols: 1, rows: 1 });

  // 大文字も受け付ける（/i フラグ）
  assert.deepEqual(parseLayout('2X2'), { cols: 2, rows: 2 });

  // session_ids parse ロジック（detached-grid.ts 内と同等）
  function parseSessionIds(raw: string): number[] {
    return (raw || '').split(',')
      .map((s: string) => parseInt(s.trim(), 10))
      .filter((n: number) => Number.isFinite(n) && n > 0);
  }

  assert.deepEqual(parseSessionIds('1,2,3,4'), [1, 2, 3, 4]);
  assert.deepEqual(parseSessionIds('5'), [5]);
  assert.deepEqual(parseSessionIds(''), []);
  assert.deepEqual(parseSessionIds('1, 2, 3'), [1, 2, 3]);
  // 無効値はフィルタされる
  assert.deepEqual(parseSessionIds('1,abc,3'), [1, 3]);
  assert.deepEqual(parseSessionIds('0,1,2'), [1, 2]); // 0 は除外（n > 0）
  assert.deepEqual(parseSessionIds('-1,1'), [1]);
});

test('ungluedApprovalLines splits glued numbered options for CLI display', () => {
  // parseHubBlock の前処理。export 経由でも同じ挙動になることを担保する。
  const glued = [
    '  どの方向でいきますか？',
    '1. [A] 説明A (Recommended)2. [B] 説明B3. [C] 説明C',
  ];
  const result = parser.ungluedApprovalLines(glued);
  // 連結された 1./2./3. が独立した行へ分割されること。
  assert.ok(result.some(l => /^1\./.test(l)));
  assert.ok(result.some(l => /^2\./.test(l)));
  assert.ok(result.some(l => /^3\./.test(l)));

  // 既に改行された入力は壊れない（重複/再分割しない）。
  const proper = [
    '  Q1 進め方',
    '  1. [A] 説明A (Recommended)',
    '  2. [B] 説明B',
    '   N. User specifies',
  ];
  const properResult = parser.ungluedApprovalLines(proper);
  assert.equal(properResult.filter(l => /^\s*1\./.test(l)).length, 1);
  assert.equal(properResult.filter(l => /^\s*2\./.test(l)).length, 1);
});

// 構造が壊れた承認ブロックは承認 UI を出さない。
// 背景: docs/local/archive/v0.5.x/bugfix_codex-approval-marker-vt-wrap-corruption_2026-07-31.md
// Hub の VT ミラーが実端末と乖離すると、開始/終了マーカーは揃ったまま選択肢行だけが失われ、
// 「選択肢が 3 から始まるパネル」「ボタン 1 個だけのパネル」が描画された（2026-07-31 実測）。
// 本文はすべて合成データ（実プロンプト・実プロジェクトの文言は貼らない）。
test('corrupt hub marker blocks are rejected', () => {
  // マーカーの実文字列を連続して書かない（AI エージェント経由の編集時に
  // Web ダッシュボードの hub-marker-filter が誤爆するため）。
  const OPEN = '[' + 'MANY-AI-CLI' + ']';
  const CLOSE = '[/' + 'MANY-AI-CLI' + ']';

  // --- 正常系: 誤爆しないこと ---

  assert.deepEqual(numbers(parser.extractHubMarkerApproval([
    OPEN,
    'どの方式で進めますか？',
    '1. 案 A (Recommended)',
    '2. 案 B',
    'N. User specifies',
    CLOSE,
  ])), [1, 2]);

  // ブロック通し番号のバッチ（Q1 が 1,2 / Q2 が 3,4）。
  const throughNumbered = parser.extractHubMarkerApproval([
    OPEN,
    '前置きの説明です。',
    'Q1 最初の質問ですか？',
    ' 1. 案 A (Recommended)',
    ' 2. 案 B',
    ' N. User specifies',
    'Q2 次の質問ですか？',
    ' 3. 案 C (Recommended)',
    ' 4. 案 D',
    ' N. User specifies',
    CLOSE,
  ]);
  assert.ok(throughNumbered, 'block-through numbering must be accepted');
  assert.equal(parser.isBatchOptions(throughNumbered), true);

  // 質問ごとに 1 から振り直すバッチ。
  const renumbered = parser.extractHubMarkerApproval([
    OPEN,
    'Q1 最初の質問ですか？',
    ' 1. 案 A (Recommended)',
    ' 2. 案 B',
    'Q2 次の質問ですか？',
    ' 1. 案 C (Recommended)',
    ' 2. 案 D',
    CLOSE,
  ]);
  assert.ok(renumbered, 'per-question renumbering must be accepted');

  // Y/N 形式は 1/0 を合成するので先頭が 1 になり、番号検査に掛からない。
  assert.deepEqual(numbers(parser.extractHubMarkerApproval([
    OPEN,
    'Proceed with this change? (Y:1/N:0)',
    CLOSE,
  ])), [1, 0]);

  // #multi は 1 起点の連番。
  assert.deepEqual(numbers(parser.extractHubMarkerApproval([
    OPEN,
    '#multi どの機能を有効にしますか？',
    '1. 機能 A',
    '2. 機能 B',
    '3. 機能 C',
    CLOSE,
  ])), [1, 2, 3]);

  // 前置き（経緯）の地の文に行頭数字が来ても弾かないこと。
  // parseHubBlock の optionRe は `^(\d+)\.` なので、前置きの IP アドレスや版数を
  // 擬似 option として拾う。実データで 192.168.68.100 を num=192 と拾った承認があり、
  // 「先頭が 1」で判定すると正常な承認が無音で消える（2026-07-31 の敵対レビューで検出）。
  const preambleNumber = parser.extractHubMarkerApproval([
    OPEN,
    '192.168.68.100 だとトークンが必要になり、設定の追加が要ります。',
    'どうしますか？',
    '1. 適用してビルド (Recommended)',
    '2. 修正内容だけ先に見せる',
    CLOSE,
  ]);
  assert.ok(preambleNumber, 'preamble line starting with a number must not be rejected');
  assert.equal(numbers(preambleNumber).includes(1), true);

  // 質問をまたいだ番号の再利用は approval-rules.md が許容している（同一質問内の重複だけが禁止）。
  const crossQuestionOverlap = parser.extractHubMarkerApproval([
    OPEN,
    'Q1 最初の質問ですか？',
    ' 1. 案 A',
    ' 2. 案 B',
    ' 3. 案 C',
    'Q2 次の質問ですか？',
    ' 3. 案 C を維持',
    ' 4. 案 D',
    ' 5. 案 E',
    CLOSE,
  ]);
  assert.ok(crossQuestionOverlap, 'cross-question number reuse must be accepted');

  // --- 破損系: 承認 UI を出さないこと ---

  // 2026-07-31 の実障害と同型。Q1 の選択肢 3 行が失われ、Q2 の 3./4. だけが残る。
  assert.equal(parser.extractHubMarkerApproval([
    OPEN,
    '計画上、判断が必要な項目があります。',
    'Q1 最初の質問ですか？',
    'Q2 次の質問ですか？',
    ' 3. 案 C (Recommended)',
    ' 4. 案 D',
    ' N. User specifies',
    CLOSE,
  ]), null);

  // 見出しごと失われ、選択肢が 1 個だけ残った形。
  assert.equal(parser.extractHubMarkerApproval([
    OPEN,
    '計画上、判断が必要な項目があります。',
    ' 4. 案 D',
    ' N. User specifies',
    CLOSE,
  ]), null);

  // 振り直しを伴わない番号の重複（再描画残骸の混入）。
  assert.equal(parser.extractHubMarkerApproval([
    OPEN,
    'Q1 質問ですか？',
    ' 1. 案 A',
    ' 2. 案 B',
    ' 2. 案 B の再描画残骸',
    ' 3. 案 C',
    CLOSE,
  ]), null);

  // TUI コンポーザの枠線が選択肢ラベルへ重なった形（2026-08-01 実測）。
  // マーカーの対も番号構造も無傷なので番号検査では捕まらない。approval-rules.md が
  // ブロック内の罫線を禁じているため、罫線の連続は再描画事故の証拠として扱う。
  assert.equal(parser.extractHubMarkerApproval([
    OPEN,
    'Q1 最初の質問ですか？',
    ' 1. 案 A の説明が途中で ' + '─'.repeat(26),
    ' 2. 案 B',
    ' N. User specifies',
    CLOSE,
  ]), null);

  // 罫線 1 文字（範囲表記など）は正常扱い＝しきい値 3 未満で誤爆しない。
  assert.ok(parser.extractHubMarkerApproval([
    OPEN,
    'どの範囲を対象にしますか？',
    '1. 10─20 件だけ (Recommended)',
    '2. 全件',
    CLOSE,
  ]));
});
