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
});
