#!/usr/bin/env node
// gofmt の差分検査。`make fmt-check` から呼ばれる。
//
// 素の `gofmt -l .` を使わない理由が 2 つある。
//
// 1. gofmt はパス走査なので、`.` を渡すと `.claude/worktrees/` 配下のエージェント用
//    ワークツリー（このリポジトリの複製）まで拾う。2026-08-27 の実測で 2315 行出た。
//    go vet / go test / staticcheck は `./...` がモジュール境界を跨がないので影響を
//    受けないが、gofmt だけはパスを名指しする必要がある。
//
// 2. このリポジトリは `.gitattributes` の `* text=auto` と `core.autocrlf=true` で
//    Windows の作業ツリーが CRLF になる。gofmt は LF へ正規化するので、`gofmt -l` は
//    改行だけを理由に全ファイルを列挙する（2026-08-27 実測: cmd + internal の 428 本中
//    285 本）。**本物の指摘がその中に埋もれて見えない。**
//    そこで比較前に CRLF を LF へ落とし、改行の違いは無視する。
//
// --fix を付けると gofmt を適用する。**元ファイルの改行コードは維持する**ので、
// CRLF のファイルは CRLF のまま整形される（作業ツリーに混在を持ち込まない）。
//
// CI から回すときは validate.yml へ step を足す。2026-08-27 時点では未配線で、
// gofmt を見ているのはこのスクリプトだけ。

import { readdirSync, readFileSync, writeFileSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { spawnSync } from 'node:child_process';

const ROOTS = ['cmd', 'internal'];
const FIX = process.argv.includes('--fix');

function collect(dir, out) {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const e of entries) {
    const p = join(dir, e.name);
    if (e.isDirectory()) {
      if (e.name === 'testdata' || e.name.startsWith('.')) continue;
      collect(p, out);
    } else if (e.name.endsWith('.go')) {
      out.push(p);
    }
  }
  return out;
}

const files = [];
for (const r of ROOTS) {
  try {
    if (statSync(r).isDirectory()) collect(r, files);
  } catch {
    console.error(`NG: 走査対象 ${r}/ が見つかりません`);
    process.exit(1);
  }
}

const offenders = [];
const parseErrors = [];

for (const f of files) {
  const raw = readFileSync(f);
  const hadCRLF = raw.includes('\r\n');
  const lf = Buffer.from(raw.toString('utf8').replace(/\r\n/g, '\n'), 'utf8');
  const res = spawnSync('gofmt', [], { input: lf, maxBuffer: 64 * 1024 * 1024 });
  if (res.error) {
    console.error('NG: gofmt を実行できません。Go が PATH にありますか');
    console.error(String(res.error.message));
    process.exit(1);
  }
  if (res.status !== 0) {
    parseErrors.push([f, String(res.stderr).trim().split('\n')[0]]);
    continue;
  }
  if (!res.stdout.equals(lf)) {
    offenders.push(f);
    if (FIX) {
      const text = res.stdout.toString('utf8');
      writeFileSync(f, hadCRLF ? text.replace(/\n/g, '\r\n') : text);
    }
  }
}

const scope = `${ROOTS.join(' + ')} · ${files.length} files`;

if (parseErrors.length) {
  console.error(`NG: gofmt が parse できないファイルが ${parseErrors.length} 件あります`);
  for (const [f, msg] of parseErrors) console.error(`  ${relative('.', f).split(sep).join('/')}: ${msg}`);
  process.exit(1);
}

if (!offenders.length) {
  console.log(`OK: gofmt に問題なし（${scope}・改行の違いは無視）`);
  process.exit(0);
}

if (FIX) {
  console.log(`FIXED: ${offenders.length} 件を整形しました（改行コードは維持）`);
  for (const f of offenders) console.log(`  ${relative('.', f).split(sep).join('/')}`);
  process.exit(0);
}

console.error(`NG: gofmt が必要なファイルが ${offenders.length} 件あります（${scope}）`);
for (const f of offenders) console.error(`  ${relative('.', f).split(sep).join('/')}`);
console.error('');
console.error('  直すには: make fmt');
process.exit(1);
