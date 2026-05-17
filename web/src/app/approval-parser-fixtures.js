const assert = require('assert/strict');
const parser = require('./approval-parser.js');

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
  const isCodexShortcutMenu = provider === 'codex' && options.some(o => o._sendText) && hasNativePromptHint;
  const approvalNear = (parser.hasApprovalLikeLabel(options) &&
    (parser.linesHaveHint(provider, contextLines, matcher) || hasNativePromptHint)) ||
    parser.isHubChoicePrompt(contextLines, options) ||
    isCodexShortcutMenu;
  return options.length > 0 && approvalNear && (hasCursor || isCodexShortcutMenu)
    ? options
    : [];
}

function run() {
  const triggerMatcher = (_provider, line) => /requires approval|would you like to run/i.test(String(line || ''));
  assert.equal(parser.userSpecifiesRe.test('User specifies'), true);
  assert.equal(parser.userSpecifiesRe.test('その他指定'), true);

  const hub = parser.extractHubMarkerApproval([
    '[ANY-AI-CLI]',
    'Proceed with this change? (Y:1/N:0)',
    '[/ANY-AI-CLI]',
  ]);
  assert.deepEqual(numbers(hub), [1, 0]);
  assert.equal(parser.approvalSig(hub), parser.approvalSig(parser.extractHubMarkerApproval([
    '[ANY-AI-CLI] Proceed with this change? (Y:1/N:0) [/ANY-AI-CLI]',
  ])));

  const plain = parser.extractPlainYesNoApproval([
    'Do you want to apply this patch? (Y:1/N:0)',
  ]);
  assert.deepEqual(labels(plain), ['Yes (1)', 'No (0)']);

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
    '[ANY-AI-CLI]',
    '1 first question?',
    ' 1. Approve',
    ' 2. Deny',
    '2 second question?',
    ' 1. Approve',
    ' 2. Deny',
    '[/ANY-AI-CLI]',
  ]);
  assert.equal(parser.isBatchOptions(batch), true);
  assert.equal(batch.length, 2);

  const numberedList = [
    'Implementation notes:',
    '1. Read the config',
    '2. Update the renderer',
    '3. Add a test',
  ];
  assert.deepEqual(detectFallback('claude', numberedList, triggerMatcher), []);
  assert.equal(parser.linesHaveHint('claude', numberedList, triggerMatcher), false);

  const chunkPath = parser.extractHubMarkerApproval([
    'noise',
    '[ANY-AI-CLI]',
    'Proceed? (Y:1/N:0)',
    '[/ANY-AI-CLI]',
  ]);
  const bufferPath = parser.extractHubMarkerApproval([
    '[ANY-AI-CLI]',
    'Proceed? (Y:1/N:0)',
    '[/ANY-AI-CLI]',
  ]);
  assert.equal(parser.approvalSig(chunkPath), parser.approvalSig(bufferPath));
}

run();
console.log('approval-parser fixtures passed');
