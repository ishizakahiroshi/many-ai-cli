#!/usr/bin/env node
// リリース成果物へ調査用の観測コードが混入していないかを検査する。
//
// channel 名は共有ファイル側の恒久コードにも現れるため、成果物検査の needle
// には使わない。instrumentation.json の artifactNeedles は sink にしか現れない
// 文字列を登録する。

import { existsSync, lstatSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const ledgerPath = join(repoRoot, 'instrumentation.json');

function displayPath(file) {
  const rel = relative(repoRoot, file);
  return rel && !rel.startsWith('..')
    ? rel.split(sep).join('/')
    : file;
}

function loadNeedles() {
  if (!existsSync(ledgerPath)) {
    throw new Error('instrumentation.json が無い。観測コードの台帳は必須。');
  }
  const ledger = JSON.parse(readFileSync(ledgerPath, 'utf8'));
  const needles = new Map();
  for (const entry of ledger.entries || []) {
    if (entry.status !== 'active' && entry.status !== 'removed') continue;
    for (const needle of entry.artifactNeedles || []) {
      if (typeof needle !== 'string' || needle.length === 0) continue;
      if (!needles.has(needle)) needles.set(needle, entry.id || '(unknown)');
    }
  }
  return [...needles.entries()].map(([needle, id]) => ({ needle, id }));
}

function collectFiles(target, out, seen) {
  const info = lstatSync(target);
  if (info.isDirectory()) {
    for (const entry of readdirSync(target, { withFileTypes: true })) {
      collectFiles(join(target, entry.name), out, seen);
    }
    return;
  }

  // リリース成果物は通常のファイルだが、ファイルへの symlink は検査対象に
  // 含める。ディレクトリ symlink は循環を避けるため辿らない。
  if (info.isSymbolicLink()) {
    try {
      if (!statSync(target).isFile()) return;
    } catch {
      return;
    }
  } else if (!info.isFile()) {
    return;
  }

  const key = resolve(target);
  if (seen.has(key)) return;
  seen.add(key);
  out.push(target);
}

function main() {
  const args = process.argv.slice(2);
  if (args.length === 0) {
    console.error('使い方: node scripts/check-artifact-clean.mjs <path...>');
    process.exit(1);
  }

  let needles;
  try {
    needles = loadNeedles();
  } catch (error) {
    console.error(`BLOCKED: ${error.message}`);
    process.exit(1);
  }

  const files = [];
  const seen = new Set();
  for (const arg of args) {
    const target = resolve(repoRoot, arg);
    if (!existsSync(target)) {
      console.error(`BLOCKED: 検査対象がありません: ${arg}`);
      process.exit(1);
    }
    try {
      collectFiles(target, files, seen);
    } catch (error) {
      console.error(`BLOCKED: 検査対象を読めません: ${arg} (${error.message})`);
      process.exit(1);
    }
  }

  const hits = [];
  for (const file of files) {
    let body;
    try {
      // latin1 ならバイナリ中の ASCII 文字列を 1 byte = 1 文字で検索できる。
      body = readFileSync(file, 'latin1');
    } catch (error) {
      console.error(`BLOCKED: 成果物を読めません: ${displayPath(file)} (${error.message})`);
      process.exit(1);
    }
    for (const { needle, id } of needles) {
      const offset = body.indexOf(needle);
      if (offset !== -1) hits.push({ file, needle, id, offset });
    }
  }

  if (hits.length > 0) {
    console.error('BLOCKED: 観測コードの文字列がリリース成果物に見つかりました');
    for (const hit of hits) {
      console.error(`  needle=${JSON.stringify(hit.needle)} id=${hit.id} file=${displayPath(hit.file)} offset=${hit.offset}`);
    }
    process.exit(1);
  }

  console.log(`instrumentation: not-shipped（${needles.length} needles / ${files.length} files）`);
}

main();
