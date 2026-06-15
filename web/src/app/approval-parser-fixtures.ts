import assert from 'node:assert/strict';
import test from 'node:test';
import { approvalParser as parser } from './approval-parser.js';

function labels(options) {
  return (options || []).map(o => o.label);
}

function numbers(options) {
  return (options || []).map(o => o.num);
}

function detectFallback(provider, lines, matcher) {
  const extraction = parser.extractApprovalOptions(lines);
  const options = extraction.options;
  const contextLines = parser.approvalContextLines(lines, extraction.cluster);
  parser.markHubChoiceDefault(options, contextLines);
  const hasCursor = options.some(o => o.isCurrent);
  const hasNativePromptHint = contextLines.some(line => matcher(provider, line) || parser.matchNativeApprovalTrigger(line));
  const isShortcutApprovalMenu = (provider === 'codex' || provider === 'copilot' || provider === 'cursor-agent') && options.some(o => o._sendText) && hasNativePromptHint;
  const approvalNear = (parser.hasApprovalLikeLabel(options) &&
    (parser.linesHaveHint(provider, contextLines, matcher) || hasNativePromptHint)) ||
    parser.isHubChoicePrompt(contextLines, options) ||
    isShortcutApprovalMenu;
  const hasChoiceMenu = hasCursor && options.length > 0 && hasNativePromptHint;
  return (options.length > 0 && approvalNear && (hasCursor || isShortcutApprovalMenu)) || hasChoiceMenu
    ? options
    : [];
}

test('approval parser fixtures', () => {
  const triggerMatcher = (_provider, line) => /requires approval|would you like to run/i.test(String(line || ''));
  assert.equal(parser.userSpecifiesRe.test('User specifies'), true);
  assert.equal(parser.userSpecifiesRe.test('その他指定'), true);

  const hub = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'Proceed with this change? (Y:1/N:0)',
    '[/MANY-AI-CLI]',
  ]);
  assert.deepEqual(numbers(hub), [1, 0]);
  assert.equal(parser.approvalSig(hub), parser.approvalSig(parser.extractHubMarkerApproval([
    '[MANY-AI-CLI] Proceed with this change? (Y:1/N:0) [/MANY-AI-CLI]',
  ])));

  const plain = parser.extractPlainYesNoApproval([
    'Do you want to apply this patch? (Y:1/N:0)',
  ]);
  assert.deepEqual(labels(plain), ['Yes (1)', 'No (0)']);
  assert.deepEqual(labels(parser.extractPlainYesNoApproval([
    'A拠点・B拠点・C拠点の3台で連絡先関連機能をOFFにしますか？ （Y：1／N：0）',
  ])), ['Yes (1)', 'No (0)']);
  assert.deepEqual(labels(parser.extractPlainYesNoApproval([
    'A拠点・B拠点・C拠点の3台で連絡先関連機能をOFFにしますか？ (Y:1/',
    'N:0)',
  ])), ['Yes (1)', 'No (0)']);
  assert.equal(parser.extractPlainYesNoApproval([
    'question? (Y:1/N:0)',
  ]), null);
  assert.equal(parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'question? (Y:1/N:0)',
    '[/MANY-AI-CLI]',
  ]), null);

  // マーカー直前の地の文（前置き説明）を _preamble として取り出す。
  // 大きな段落区切り（2 行連続の空行）より上は対象外。
  const withPreamble = parser.extractHubMarkerApproval([
    'これは無関係な過去ログ。',
    '',
    '',
    'そのうえで、判断が要るものがあります。',
    'License 不在時は自動生成しない。',
    '[MANY-AI-CLI]',
    'Proceed? (Y:1/N:0)',
    '[/MANY-AI-CLI]',
  ]);
  assert.deepEqual(numbers(withPreamble), [1, 0]);
  assert.equal(
    (withPreamble as any)._preamble,
    'そのうえで、判断が要るものがあります。\nLicense 不在時は自動生成しない。',
  );
  // 直前に確定ブロックがあれば、それより前は前置きに含めない。
  const afterPrevBlock = parser.extractHubMarkerApproval([
    '前の質問の本文。',
    '[/MANY-AI-CLI-DONE] 完了サマリー [/MANY-AI-CLI-DONE]',
    '次の前置き。',
    '[MANY-AI-CLI]',
    'Proceed? (Y:1/N:0)',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal((afterPrevBlock as any)._preamble, '次の前置き。');

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
  assert.equal((singleQ as any)._freeInput, true);

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

  const codexLines = [
    'This command requires approval',
    '> 1. Yes (y)',
    '  2. Yes, and don\'t ask again for this command (p)',
    '  3. No (n)',
  ];
  const codex = detectFallback('codex', codexLines, triggerMatcher);
  assert.deepEqual(numbers(codex), [1, 2, 3]);
  assert.equal(codex[0]._sendText, 'y');
  assert.equal(codex[1]._sendText, 'p');
  assert.equal(codex[2]._sendText, 'n');

  const copilotLines = [
    'Permission required',
    '> 1. Allow once (y)',
    '  2. Deny once (n)',
    '  3. Allow all similar for this session (!)',
    '  4. Show details (?)',
  ];
  const copilot = detectFallback('copilot', copilotLines, triggerMatcher);
  assert.deepEqual(numbers(copilot), [1, 2, 3, 4]);
  assert.equal(copilot[0]._sendText, 'y');
  assert.equal(copilot[1]._sendText, 'n');
  assert.equal(copilot[2]._sendText, '!');
  assert.equal(copilot[3]._sendText, '?');

  // cursor-agent 実機 UI（キー表記のみのメニュー: (y)/(tab)/(shift+tab)/(esc or n)）は
  // Go バックエンドの native 検出（go_vt 経路）で処理する。ここでは fallback パーサが
  // cursor-agent を shortcut-menu provider として扱う汎用挙動（番号付きバリアント）のみ検証する。
  const cursorAgentLines = [
    'Permission required',
    '> 1. Allow once (y)',
    '  2. Deny once (n)',
    '  3. Allow all similar for this session (!)',
    '  4. Show details (?)',
  ];
  const cursorAgent = detectFallback('cursor-agent', cursorAgentLines, triggerMatcher);
  assert.deepEqual(numbers(cursorAgent), [1, 2, 3, 4]);
  assert.equal(cursorAgent[0]._sendText, 'y');
  assert.equal(cursorAgent[1]._sendText, 'n');
  assert.equal(cursorAgent[2]._sendText, '!');
  assert.equal(cursorAgent[3]._sendText, '?');

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

  // #multi が無い同形の番号付きリストは単一選択（multiSelect ではない）のまま。
  const singleSelect = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI]',
    'どれにしますか？',
    '1. A',
    '2. B',
    '[/MANY-AI-CLI]',
  ]);
  assert.equal(parser.isMultiSelectOptions(singleSelect), false);

  const numberedList = [
    'Implementation notes:',
    '1. Read the config',
    '2. Update the renderer',
    '3. Add a test',
  ];
  assert.deepEqual(detectFallback('claude', numberedList, triggerMatcher), []);
  assert.equal(parser.linesHaveHint('claude', numberedList, triggerMatcher), false);

  // Ink のカーソル位置制御描画で選択肢間の改行が失われ「1. … 2. … 3. … N. User specifies」が
  // 1行へ連結されたケースの回帰（=「承認ボタンが全部1つに潰れる」症状）。
  // フォールパック経路（extractApprovalOptions）で 3 選択肢へ復元でき、
  // N. User specifies が最後の選択肢ラベルへ混入しないこと。
  const gluedFallback = parser.extractApprovalOptions([
    '1. 質問2=「PCも含め全画面で効かせる」を選択（質問1の初期値はON=既定で表示のまま）(Recommended)2. 質問1=「初期値OFF=既定で非表示」を選択（質問2の適用範囲はスマホのみのまま）3. 両方とも option 2（初期値OFF かつ 全画面で効かせる） N. User specifies',
  ]);
  assert.deepEqual(numbers(gluedFallback.options), [1, 2, 3]);
  assert.equal(/User specifies/.test(gluedFallback.options[2].label), false);
  assert.equal(/Recommended/.test(gluedFallback.options[0].label), true);

  // xterm のハードラップで option 1(Recommended・最長本文)が物理2行へ折り返され、
  // 継続行が行頭に空白を持たないケース。フォールバック経路（extractApprovalOptions）が
  // 継続行で break して option 1 ごと脱落していた回帰（「確認メッセージの 1 が途切れる」頻発症状）。
  // 継続行を直前(上方)の option 1 ラベルへ結合し、1/2/3 すべて復元できること。
  const wrappedFallbackOpt1 = parser.extractApprovalOptions([
    'どこまで進めるか確認します。',
    '1. C4(docs) + C3の「全アクセス失効」ボタンのみ（＝最小構成完成・C1を実際に使え',
    'る状態に。PINは見送り）  (Recommended)',
    '2. 上記に加えて C2+C3のPIN一式も実装（任意PIN・ロックアウト・SEC-C新規デバイス通知まで全部）',
    '3. C4(docs)だけ先に作る（ボタンUIは後回し）',
    'N. User specifies',
  ]);
  assert.deepEqual(numbers(wrappedFallbackOpt1.options), [1, 2, 3]);
  assert.equal(/Recommended/.test(wrappedFallbackOpt1.options[0].label), true);
  assert.equal(/る状態に/.test(wrappedFallbackOpt1.options[0].label), true);

  // 連番でない「1. … 3. …」や小数「1.5」は誤分割しないこと（保守的分割の確認）。
  const notSequential = parser.extractApprovalOptions([
    '1. first 3. third',
  ]);
  assert.equal(notSequential.options.length <= 1, true);

  // marker 経路でブロック全体が1行へ完全に潰れたケースの回帰。
  // 見出し「1 …?」と選択肢「1. 2. 3.」が混在連結されても 3 選択肢へ復元できること。
  const gluedMarker = parser.extractHubMarkerApproval([
    '[MANY-AI-CLI] 1 「2」はどの設定を指していますか? 1. A を選択(Recommended)2. B を選択 3. 両方とも C N. User specifies [/MANY-AI-CLI]',
  ]);
  assert.deepEqual(numbers(gluedMarker), [1, 2, 3]);

  // Claude のネイティブ AskUserQuestion ピッカー（末尾に "Type something" /
  // "Chat about this" の自由入力肢を持つ arrow 駆動 UI）は extractApprovalOptions で
  // 抑止し Web ボタン化しないこと（VT スクレイプで番号がズレ誤選択を招くため）。
  const askUserQuestion = parser.extractApprovalOptions([
    'スキーマ差分の適用範囲は?',
    '❯ 1. 全差分を全環境へ適用',
    '  2. 必要なものだけ精査して適用',
    '  3. コードだけ先にデプロイ',
    '  4. 差分の中身を先に見たい',
    '  5. Type something.',
    '  6. Chat about this',
  ]);
  assert.deepEqual(askUserQuestion.options, []);

  // 標準のツール許可プロンプト（Yes / Yes, and / No）は "Type something" を
  // 含まないため抑止されず、これまで通り選択肢を返すこと（誤抑止の回帰防止）。
  const normalApproval = parser.extractApprovalOptions([
    'This command requires approval',
    '❯ 1. Yes',
    '  2. Yes, and don\'t ask again for this command',
    '  3. No',
  ]);
  assert.deepEqual(numbers(normalApproval.options), [1, 2, 3]);

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
  assert.equal(parser.approvalSig(chunkPath), parser.approvalSig(bufferPath));

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

  // claude /model のような承認ではないカーソル駆動 TUI 選択メニュー。
  // フッターの「Esc to cancel」を matchNativeApprovalTrigger が拾い、❯ カーソル付き選択肢が
  // あるため detectFallback の choice-menu 経路で検出される（=action-bar が出る）。
  // 承認ラベル（yes/no/allow/deny…）を含まないので「承認」ではなく「選択メニュー」として
  // 扱える（detectApproval 側で _selectMenu タグを付け、入力ガード・メニュータイトル表示に使う）。
  const modelMenuLines = [
    'Select model',
    'Switch between Claude models. Your pick becomes the default for new sessions.',
    '❯ 1. Default (recommended) ✔ Opus 4.8 with 1M context',
    '  2. Opus  Opus 4.8 with 1M context',
    '  3. Sonnet  Sonnet 4.6',
    '  4. Haiku  Haiku 4.5',
    '  5. Fable  Claude Fable 5 is currently unavailable',
    '',
    'Enter to set as default · s to use this session only · Esc to cancel',
  ];
  const modelMenu = detectFallback('claude', modelMenuLines, triggerMatcher);
  assert.deepEqual(numbers(modelMenu), [1, 2, 3, 4, 5]);
  assert.equal(modelMenu[0].isCurrent, true);
  // 承認ラベルを含まない（=承認ではなく選択メニュー）。
  assert.equal(parser.hasApprovalLikeLabel(modelMenu), false);
  // フッターは native approval/menu ヒントとして拾われる（choice-menu 経路の発火条件）。
  assert.equal(parser.matchNativeApprovalTrigger('Enter to set as default · s to use this session only · Esc to cancel'), true);

  // 選択メニューの本文に紛れた通常の番号付き箇条書き（フッターヒントなし）は誤検出しない。
  const plainNumbered = [
    'Here is the plan:',
    '1. Read the file',
    '2. Apply the patch',
    '3. Run tests',
  ];
  assert.deepEqual(detectFallback('claude', plainNumbered, triggerMatcher), []);
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
