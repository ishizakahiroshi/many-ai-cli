#!/usr/bin/env node
// 画面が端末の文字から承認を作る経路を戻させない。
//
// 背景:
//   以前の画面は、Hub から承認ごとに一度きりの通知を受け取るだけで「いまの状態」を渡されず、
//   端末の文字を読んで自分で承認を作り（チャンクを受け取ったときと、セッションを切り替えたとき）、
//   文字照合で閉じ（H9 / bgApprovalMiss）、取りこぼしをタイマーと台帳の取り直しで救っていた。
//   経路ごとに早期 return が積み重なり、別のセッションを見ている間に届いた承認が、切り替えても
//   描かれない行き止まりができた（docs/local/bugfix_approval-panel-blank-on-switch_2026-09-23.md）。
//
//   2026-09-23 に、Hub が持つ保留中の記録（internal/hub/approval_record.go）を描くだけの形へ
//   切り替え、上の経路を撤去した（docs/local/plan_approval-display-single-source_c3_web-switch.md）。
//   画面の入口は web/src/app/approval-store.ts の 1 つだけ。撤去した関数と state は、名前ごと
//   戻した瞬間にここで落とす。Hub 側の同じ固定は internal/hub/approval_display_guard_test.go。
//
// 検査内容:
//   web/src/**/*.ts（fixture を除く）に、下の BANNED の名前が単語として現れたら落とす。
//   コメントも対象にする。撤去した名前をコメントで参照すると、読んだ人が「まだある」と誤読するため。
//   Hub が承認ごとに送っていたメッセージの種類名（と、画面が可視を申告していた種類名）も、
//   文字列として現れたら落とす。
//   ↻ 承認のボタンとその文言（子 plan C4 で撤去。✕ は帯に畳み、取り直しは Hub への問い直し）は
//   ソースではなく web/src の .html / .css / .json に現れるので、そちらを文字列で見る。
//
//   残した Hub ブロック用のパーサとパネルを描く関数は、呼んでよいファイルを限る（下の CALLERS）。
//   名前を変えずに、パーサを端末の文字へ当ててパネルへ直接渡す戻し方を止めるため。
//
// この検査が見ないもの（既知の盲点）:
//   1. 名前を変えて同じ処理を書き直すこと。名前の検査なので、別名の推測検出は通る。
//      入口が approval-store.ts の 1 つであること（下の ANCHORS）と、レビューで止める。
//      種類名を文字列の連結やテンプレートで組み立てることも通る。
//   2. fixture（*-fixtures.ts）。撤去した挙動を「起きないこと」として試す場合に名前が要るため。
//
// exit 0 = 問題なし / exit 1 = ブロック。

import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const webSrc = join(repoRoot, 'web', 'src');
// Windows のパス区切り。表示用に / へ揃える。
const WIN_SEP = String.fromCharCode(92);

// 撤去した名前と、戻すと何が起きるか。
const BANNED = {
  // 端末の文字から承認を作る処理（チャンク時と切り替え時）と、その推測の材料
  trackApprovalHintFromChunk: '端末の文字から承認を作る処理',
  detectApproval: '端末の文字から承認を作る処理',
  scheduleApprovalCheck: '端末の文字から承認を作る処理',
  extractApprovalOptions: '端末の文字から承認を作る処理（推測での拾い上げは Hub の approval_text_question.go だけ）',
  extractPlainYesNoApproval: '端末の文字から承認を作る処理（マーカー無しの Yes/No は Hub だけ）',
  approvalContextLines: '端末の文字から承認を作る処理',
  approvalLineHasHint: '端末の文字から承認を作る処理',
  approvalLinesHaveHint: '端末の文字から承認を作る処理',
  isMultiQuestionPrompt: '端末の文字から承認を作る処理（複数質問の判定は Hub だけ）',
  isHubChoicePrompt: '端末の文字から承認を作る処理（旧形式の選択は Hub だけ）',
  markHubChoiceDefault: '端末の文字から承認を作る処理',
  matchNativeApprovalTrigger: '端末の文字から承認を作る処理',
  matchProviderApprovalTrigger: '端末の文字から承認を作る処理（承認パターンは Hub が読む）',
  providerApprovalTriggers: '端末の文字から承認を作る処理（承認パターンは Hub が読む）',
  hasApprovalLikeLabel: '端末の文字から承認を作る処理',
  pendingTextTail: '端末の文字から承認を作る処理の材料',
  APPROVAL_PENDING_TEXT_TAIL_LIMIT: '端末の文字から承認を作る処理の材料',
  isSelectMenuActive: '端末の文字から作った選択メニューの判定',
  // Hub の旧メッセージの受け口
  handleGoApprovalDetected: '旧メッセージの受け口（承認は approval_state だけで届く）',
  handleHubApprovalMarker: '旧メッセージの受け口（承認は approval_state だけで届く）',
  handleGoApprovalCleared: '旧メッセージの受け口（承認は approval_state だけで届く）',
  isGoNativeApprovalActive: '旧メッセージの受け口の state',
  // 画面が文字照合で閉じる処理
  h9RestoreMisses: '画面の文字照合で承認を閉じる処理（H9）',
  scheduleH9Revalidate: '画面の文字照合で承認を閉じる処理（H9）',
  cachedOptionsOnScreen: '画面の文字照合で承認を閉じる処理（H9 / bgApprovalMiss）',
  trackBgApprovalMiss: '画面の文字照合で承認を閉じる処理（bgApprovalMiss）',
  bgApprovalMisses: '画面の文字照合で承認を閉じる処理（bgApprovalMiss）',
  // 救済のタイマーと台帳の取り直し
  scheduleApprovalHintConfirm: '承認のヒント確定タイマー',
  cancelApprovalHintConfirm: '承認のヒント確定タイマー',
  approvalHintConfirmTimers: '承認のヒント確定タイマー',
  approvalHintConfirmTrusted: '承認のヒント確定タイマー',
  scheduleApprovalSuppressRescan: '回答直後の読み直しタイマー',
  approvalSuppressUntil: '回答直後の読み直しタイマー',
  approvalSwitchCandidates: '表示中の承認の切り替え待ちタイマー',
  APPROVAL_LEDGER_RESTORE_DELAY_MS: '600ms 後の台帳の取り直し',
  scheduleApprovalLedgerRestore: '600ms 後の台帳の取り直し',
  cancelApprovalLedgerRestore: '600ms 後の台帳の取り直し',
  maybeRestorePendingApprovalFromLedger: '600ms 後の台帳の取り直し',
  restoreApprovalFromLedger: '600ms 後の台帳の取り直し',
  // 画面が持っていた「表示中」の記録（Hub の記録の写しは approval-store.ts だけ）
  approvalVisibleCache: '画面が持つ「表示中」の記録',
  approvalRawOptionsCache: '画面が持つ「表示中」の記録',
  approvalSourceCache: '画面が持つ「表示中」の記録',
  multiQuestionVisibleCache: '画面が持つ「表示中」の記録',
  multiQuestionLatchAt: '画面が持つ「表示中」の記録',
  hubMarkerDeliveredEpoch: '端末の文字と Hub のどちらが正本かの state',
  isHubMarkerAuthoritative: '端末の文字と Hub のどちらが正本かの state',
  noteHubMarkerDelivered: '端末の文字と Hub のどちらが正本かの state',
  // 同一性を画面で組み立て直す処理と replay の関所（同一性は Hub の記録が持って届く）
  recordAnsweredApprovalCandidate: '同一性を画面で組み立て直す処理',
  isAnsweredApprovalCandidate: '同一性を画面で組み立て直す処理',
  approvalCandidateIdentity: '同一性を画面で組み立て直す処理',
  approvalSourceEpochCache: '同一性を画面で組み立て直す処理',
  noteApprovalSourceEpoch: '同一性を画面で組み立て直す処理',
  getApprovalSourceEpoch: '同一性を画面で組み立て直す処理',
  inheritMarkerBlockSig: '同一性を画面で組み立て直す処理',
  approvalQuestionKey: '同一性を画面で組み立て直す処理',
  approvalReplayState: 'replay の関所',
  beginApprovalReplay: 'replay の関所',
  finishApprovalReplay: 'replay の関所',
  isApprovalReplayPending: 'replay の関所',
  replayAnsweredApprovalTokens: 'replay の関所',
  // Hub へ「承認が見えている」と申告する処理（「保留中」は Hub の記録からだけ出す）
  setApprovalVisible: '画面から Hub へ可視を申告する処理',
  sendSessionHint: '画面から Hub へ可視を申告する処理',
  reassertApprovalHints: '画面から Hub へ可視を申告する処理',
  cacheApprovalOptions: '画面が持つ「表示中」の記録',
  clearApprovalOptions: '画面が持つ「表示中」の記録',
  sendApprovalConsumed: '同一性を画面で組み立て直す回答の通知（noteApprovalAnswerSent が記録の同一性で送る）',
  maybeSendDirectApprovalConsumed: '同一性を画面で組み立て直す回答の通知',
  // 隠すだけの ✕ と ↻ 承認（docs/local/plan_approval-display-single-source_c4_controls.md の C1・C2）。
  // ✕ は帯に畳み（畳み状態は approval-store.ts が記録ごとに持つ）、帯を押せば開く。
  // 画面の写しは Hub の記録からしか作らないので、取り直しは Hub への問い直し（approval_resync）。
  reshowActionBar: '↻ 承認の再表示（畳んだ承認は帯を押して開く。取り直しは requestApprovalResync で Hub へ問い直す）',
  manuallyHideActionBar: 'パネルを隠すだけの ✕（✕ は foldApprovalPanel で帯に畳む）',
  manualHideState: 'パネルを隠すだけの ✕ の印（畳み状態は approval-store.ts が記録ごとに持つ）',
  isManualHideActive: 'パネルを隠すだけの ✕ の印（畳み状態は approval-store.ts が記録ごとに持つ）',
  rememberManualHide: 'パネルを隠すだけの ✕ の印（畳み状態は approval-store.ts が記録ごとに持つ）',
  clearManualHide: 'パネルを隠すだけの ✕ の印（畳み状態は approval-store.ts が記録ごとに持つ）',
  multiQuestionDismissedCache: '告知バナーを隠すだけの ✕ の印（告知の ✕ も帯に畳む）',
  manuallyDismissedApprovalSigs: 'モバイルのシートを隠すだけの印（シートを閉じると帯に畳む）',
  latchManualDismissal: 'モバイルのシートを隠すだけの印（シートを閉じると帯に畳む）',
};

// 文字列として現れたら落とすメッセージの種類。
const BANNED_MESSAGE_TYPES = ['approval_detected', 'approval_marker', 'approval_cleared', 'session_hint'];

// 呼んでよいファイル（web/src からの相対パス）。ここに無いファイルに単語として現れたら落とす。
// 定義しているファイル自身も並べる。fixture は走査しない。
const CALLERS = {
  // Hub の記録のブロックを解くのは、ストア（描く中身）と承認タブの履歴（Hub の台帳の行）だけ
  extractHubMarkerApproval: ['app/approval-parser.ts', 'app/approval-store.ts', 'app/approval-queue-tab.ts'],
  extractSequentialChoicePrompts: ['app/approval-parser.ts', 'app/approval-store.ts'],
  hubBlockLines: ['app/approval-store.ts', 'app/approval-queue-tab.ts'],
  normalizeVtCursorOps: ['app/approval-parser.ts', 'app/approval-store.ts'],
  approvalParser: ['app/approval-parser.ts', 'types/window.d.ts'],
  // パネルを描くのは approval.ts（ストアから描く renderApprovalFromStore）と approval-ui.ts だけ
  showActionBar: ['app/approval.ts', 'app/approval-ui.ts'],
  showBatchActionBar: ['app/approval.ts'],
  showMultiSelectActionBar: ['app/approval.ts'],
  showSingleSectionBar: ['app/approval.ts'],
  approvalUiAdapter: ['app/approval.ts', 'app/approval-ui.ts', 'types/window.d.ts'],
};

// web/src の .html / .css / .json に文字列として現れたら落とすもの（↻ 承認のボタンと文言）。
const BANNED_MARKUP = {
  'approval-reshow-btn': '↻ 承認のボタン（端末右下には ✕ 承認だけを置く）',
  approval_reshow: '↻ 承認の文言（i18n のキー）',
};

// 画面の入口がここにあること（検査の前提がずれて素通りしていないか）を確かめる。
const ANCHORS = [
  { file: 'app/approval-store.ts', text: 'export function applyApprovalState' },
  { file: 'app/approval-store.ts', text: 'export function applyApprovalSnapshot' },
  { file: 'app/ws-client.ts', text: 'applyApprovalState(' },
];

function tsFiles(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (name === 'vendor' || name === 'node_modules') continue;
    const st = statSync(path);
    if (st.isDirectory()) out.push(...tsFiles(path));
    else if (name.endsWith('.ts') && !name.endsWith('-fixtures.ts')) out.push(path);
  }
  return out;
}

function markupFiles(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (name === 'vendor' || name === 'node_modules') continue;
    const st = statSync(path);
    if (st.isDirectory()) out.push(...markupFiles(path));
    else if (/\.(html|css|json)$/.test(name)) out.push(path);
  }
  return out;
}

function display(path) {
  return relative(repoRoot, path).split(WIN_SEP).join('/');
}

const failures = [];

for (const anchor of ANCHORS) {
  const path = join(webSrc, ...anchor.file.split('/'));
  if (!existsSync(path) || !readFileSync(path, 'utf8').includes(anchor.text)) {
    failures.push(`web/src/${anchor.file}: "${anchor.text}" が無い。承認の入口が移った。この検査の前提（ANCHORS）を見直すこと`);
  }
}

const files = tsFiles(webSrc);
if (files.length === 0) failures.push('web/src に .ts が 0 件。走査の起点がずれている');

const identRe = new RegExp(`(?<![A-Za-z0-9_$])(${Object.keys(BANNED).join('|')})(?![A-Za-z0-9_$])`, 'g');
const callerRe = new RegExp(`(?<![A-Za-z0-9_$])(${Object.keys(CALLERS).join('|')})(?![A-Za-z0-9_$])`, 'g');
const typeRe = new RegExp(`(['"\`])(${BANNED_MESSAGE_TYPES.join('|')})\\1`, 'g');

for (const file of files) {
  const lines = readFileSync(file, 'utf8').split(/\r?\n/);
  lines.forEach((line, index) => {
    for (const m of line.matchAll(identRe)) {
      failures.push(`${display(file)}:${index + 1}: ${m[1]} — ${BANNED[m[1]]}`);
    }
    for (const m of line.matchAll(typeRe)) {
      failures.push(`${display(file)}:${index + 1}: '${m[2]}' — 撤去したメッセージの種類（承認は approval_state / approval_snapshot だけ）`);
    }
    const rel = relative(webSrc, file).split(WIN_SEP).join('/');
    for (const m of line.matchAll(callerRe)) {
      if (!CALLERS[m[1]].includes(rel)) {
        failures.push(`${display(file)}:${index + 1}: ${m[1]} — ここからは呼ばない（呼んでよいのは ${CALLERS[m[1]].join(' / ')}。承認は Hub の記録からストア経由で描く）`);
      }
    }
  });
}

// 許可リストの前提（定義や入口の場所）がずれて素通りしていないか。
for (const [name, allowed] of Object.entries(CALLERS)) {
  const seen = allowed.some((rel) => {
    const path = join(webSrc, ...rel.split('/'));
    return existsSync(path) && new RegExp(`(?<![A-Za-z0-9_$])${name}(?![A-Za-z0-9_$])`).test(readFileSync(path, 'utf8'));
  });
  if (!seen) failures.push(`${name}: 許可したファイルのどれにも現れない。移ったなら CALLERS を見直すこと`);
}

const markup = markupFiles(webSrc);
if (markup.length === 0) failures.push('web/src に .html / .css / .json が 0 件。走査の起点がずれている');
for (const file of markup) {
  const lines = readFileSync(file, 'utf8').split(/\r?\n/);
  lines.forEach((line, index) => {
    for (const [text, reason] of Object.entries(BANNED_MARKUP)) {
      if (line.includes(text)) failures.push(`${display(file)}:${index + 1}: ${text} — ${reason}`);
    }
  });
}

if (failures.length > 0) {
  console.error('承認の表示は Hub の保留中の記録を描くだけです（web/src/app/approval-store.ts）。撤去した経路が戻っています:');
  for (const f of failures) console.error(`  ${f}`);
  console.error('');
  console.error('経緯: docs/local/plan_approval-display-single-source_c3_web-switch.md の C3 と _c4_controls.md の C1・C2。規則の正本は internal/hub/approval_record.go の冒頭。');
  process.exit(1);
}

console.log(`check-approval-display-source: OK（${files.length} ファイル・撤去した名前 ${Object.keys(BANNED).length} 個・メッセージの種類 ${BANNED_MESSAGE_TYPES.length} 個・呼び出し元を限る名前 ${Object.keys(CALLERS).length} 個・マークアップ ${markup.length} ファイル）`);
