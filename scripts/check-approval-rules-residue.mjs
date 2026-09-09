#!/usr/bin/env node
// check-approval-rules-residue — many-ai-cli が利用者のリポジトリへ書き込む
// 「承認ルール」の残骸が、このリポジトリ自身の commit に混ざるのを止める検査。
//
// なぜ要るか:
//   codex / copilot / cursor-agent / opencode を wrap すると、wrapper が
//   AGENTS.md の末尾へ承認ルールのブロックを追記する（internal/wrapper/approval_rules.go の
//   appendSharedBlock）。Claude 系には CLAUDE.md へ import 行を 1 本足す。
//   many-ai-cli の開発中に子セッションを起こすと、それが **このリポジトリの追跡ファイル**に
//   書かれる。2026-09-01 に実際に AGENTS.md へ 132 行が入った。
//
//   `many-ai-cli doctor` は既に置き去りを検出する（internal/doctor/residue.go）。
//   ただしあれは「次回起動時に気づく」経路で、**commit への混入は別の穴**。
//   git status に紛れるうえ `git add -A` で黙って入り、公開リポの履歴に
//   作者環境の運用ルールが残る。文章で「混ぜるな」と書いても破られたときに誰も気づかないので、
//   コミットの瞬間に落とす。
//
// 正本:
//   マーカーと注入対象   internal/wrapper/approval_rules.go
//   置き去りの検出       internal/doctor/residue.go
//
// モード:
//   （既定）--staged      ステージ済みの内容だけを見る。pre-commit フック用
//   --all-tracked         HEAD の内容を見る。CI 用（「過去に混入していないか」を答える）
//
// 探す文字列について:
//   旧名 any-ai-cli は新名 many-ai-cli の部分文字列（many = "m" + any）なので、
//   コメント開始記号を落とした旧名パターンで探すと新旧どちらのマーカーにも当たる。
//   逆に新名パターンで探すと旧名の残骸が 0 件に見える。
//   internal/wrapper/approval_rules.go の ApprovalRulesResidueNeedle と同じ考え方。

import { execFileSync } from 'node:child_process';
import path from 'node:path';

const BLOCK_NEEDLE = 'any-ai-cli:approval-rules';
const CLAUDE_IMPORT_NEEDLE = '@~/.many-ai-cli/approval-rules.md';

// wrapper が書き込む先のファイル名。ここに載っていないファイルは検査しない
// （設計書・CHANGELOG・Go のソースはマーカー文字列を正当に含む）。
const TARGET_BASENAMES = new Set([
  'AGENTS.md',
  'AGENTS.local.md',
  'CLAUDE.md',
  'CLAUDE.local.md',
  'GEMINI.md',
]);

// 配線の検査。マーカーを改名したのにこの検査が古い文字列を探し続ける状態を防ぐ。
const WIRING_FILE = 'internal/wrapper/approval_rules.go';

function git(args) {
  return execFileSync('git', args, { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
}

function gitOrNull(args) {
  try {
    return git(args);
  } catch (_) {
    return null;
  }
}

function checkWiring() {
  const src = gitOrNull(['show', `HEAD:${WIRING_FILE}`]);
  if (src === null) {
    return [`配線の検査に失敗: ${WIRING_FILE} が HEAD から読めません`];
  }
  const problems = [];
  if (!src.includes(BLOCK_NEEDLE)) {
    problems.push(
      `${WIRING_FILE} に "${BLOCK_NEEDLE}" が見つかりません。` +
      'マーカーを改名したなら本スクリプトの BLOCK_NEEDLE も直してください',
    );
  }
  if (!src.includes(CLAUDE_IMPORT_NEEDLE)) {
    problems.push(
      `${WIRING_FILE} に "${CLAUDE_IMPORT_NEEDLE}" が見つかりません。` +
      'import 行を変えたなら本スクリプトの CLAUDE_IMPORT_NEEDLE も直してください',
    );
  }
  return problems;
}

function stagedTargets() {
  const out = git(['diff', '--cached', '--name-only', '--diff-filter=ACMR']);
  return out.split('\n').map(s => s.trim()).filter(Boolean)
    .filter(p => TARGET_BASENAMES.has(path.basename(p)));
}

function trackedTargets() {
  const out = git(['ls-tree', '-r', '--name-only', 'HEAD']);
  return out.split('\n').map(s => s.trim()).filter(Boolean)
    .filter(p => TARGET_BASENAMES.has(path.basename(p)));
}

function readBlob(mode, file) {
  const spec = mode === 'staged' ? `:${file}` : `HEAD:${file}`;
  return gitOrNull(['show', spec]) ?? '';
}

function findHits(mode, files) {
  const hits = [];
  for (const file of files) {
    const text = readBlob(mode, file);
    if (!text) continue;
    const lines = text.split('\n');
    lines.forEach((line, i) => {
      if (line.includes(BLOCK_NEEDLE)) {
        hits.push({ file, line: i + 1, kind: '承認ルールのブロック' });
      } else if (line.trim() === CLAUDE_IMPORT_NEEDLE) {
        hits.push({ file, line: i + 1, kind: '承認ルールの import 行' });
      }
    });
  }
  return hits;
}

const mode = process.argv.includes('--all-tracked') ? 'all-tracked' : 'staged';

const wiringProblems = checkWiring();
if (wiringProblems.length > 0) {
  for (const p of wiringProblems) console.error(`NG: ${p}`);
  process.exit(1);
}

const files = mode === 'staged' ? stagedTargets() : trackedTargets();
const hits = findHits(mode, files);

if (hits.length > 0) {
  console.error('NG: many-ai-cli が注入した承認ルールが混ざっています');
  for (const h of hits) {
    console.error(`  ${h.file}:${h.line}  ${h.kind}`);
  }
  console.error('');
  console.error('これは子セッションを起こしたときに wrapper が書き込んだものです。');
  console.error('公開リポジトリの追跡ファイルなので commit しないでください。');
  console.error('対処: 該当ブロック（または import 行）を削除してから commit する。');
  console.error('      Hub を起動し直すと自動で回収されます（many-ai-cli doctor で確認できます）。');
  process.exit(1);
}

const scope = mode === 'staged' ? 'ステージ済み' : 'HEAD';
console.log(`OK: 承認ルールの混入なし（${scope}の対象ファイル ${files.length} 件を検査）`);
