#!/usr/bin/env node
// Copy canonical native binaries into each npm platform package's
// bin/ directory so the packages can be published.
//
// Input: dist/artifacts.json, an array using the existing artifact-adapter
// schema: type="Binary", extra.ID="many-ai-cli" (or the main binary name),
// goos/goarch and path. The field names are retained for compatibility; a Rust
// release adapter can supply the same manifest without changing package names.
// Relative artifact paths use the repository root by default. Entries with
// pathBase="manifest" use the manifest directory, so downloaded candidate
// bundles remain usable after moving. Absolute artifact paths remain supported.
// This script neither builds artifacts nor verifies release signatures. The
// release workflow owns canonical artifact/checksum/signature acceptance.
//
// Usage:
//   node scripts/stage-npm-binaries.mjs            # reads dist/artifacts.json
//   node scripts/stage-npm-binaries.mjs <distDir>  # custom dist dir
//
// Preflight all four readable, nonempty binaries before changing npm outputs.
// Verify copied bytes with SHA256 and exit non-zero on any incomplete staging.

import { createHash, randomUUID } from 'node:crypto';
import { readFileSync, writeFileSync, chmodSync, mkdirSync, rmSync, renameSync, statSync, existsSync, lstatSync, realpathSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, isAbsolute, resolve, relative, sep } from 'node:path';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const distArg = process.argv[2];
const distDir = distArg ? (isAbsolute(distArg) ? distArg : join(repoRoot, distArg)) : join(repoRoot, 'dist');
const npmDir = join(repoRoot, 'npm');

// Artifact manifest OS/arch tokens -> { pkg, binName }.
// Package names match the GitHub release archive suffixes.
export const TARGETS = {
  'windows/amd64': { pkg: 'many-ai-cli-windows-x64', binName: 'many-ai-cli.exe' },
  'linux/amd64': { pkg: 'many-ai-cli-linux-x64', binName: 'many-ai-cli' },
  'darwin/amd64': { pkg: 'many-ai-cli-macos-intel', binName: 'many-ai-cli' },
  'darwin/arm64': { pkg: 'many-ai-cli-macos-apple-silicon', binName: 'many-ai-cli' },
};

function loadArtifacts() {
  const path = join(distDir, 'artifacts.json');
  let artifacts;
  try {
    artifacts = JSON.parse(readFileSync(path, 'utf8'));
  } catch (err) {
    throw new Error(
      `Cannot read artifact manifest at ${path}: ${err.message}\n` +
      'Supply the canonical release binaries and an artifacts.json manifest from the release artifact adapter.'
    );
  }
  if (!Array.isArray(artifacts)) throw new Error(`Artifact manifest at ${path} must be an array.`);
  return artifacts;
}

function binaryBytes(path) {
  const stat = statSync(path);
  if (!stat.isFile() || stat.size === 0) throw new Error(`Binary is not a nonempty regular file: ${path}`);
  // Reading the bytes also proves readability, rather than accepting an entry
  // whose path exists but cannot be copied by this process.
  const data = readFileSync(path);
  if (!data.length) throw new Error('binary became empty during preflight');
  return data;
}

// All bytes and destinations are checked before replacing any package payload.
// Backups also restore existing payloads if a filesystem error interrupts commit.
export function stagePayloads(items, boundary = npmDir) {
  const root = resolve(boundary);
  const observe = path => {
    try { return lstatSync(path); } catch (error) {
      if (error.code === 'ENOENT') return null;
      throw error;
    }
  };
  const directory = path => {
    const info = observe(path);
    if (info && (info.isSymbolicLink() || !info.isDirectory())) {
      throw new Error('npm output directory links/non-directories are forbidden');
    }
  };
  // Begin at npm/, allowing the repository's legitimate worktree junction above
  // this boundary. Check every destination before creating directories or temps.
  directory(root);
  const seen = new Set();
  for (const item of items) {
    const destination = resolve(item.dst);
    const inside = relative(root, destination);
    if (!inside || inside === '..' || inside.startsWith('..' + sep) || isAbsolute(inside)) {
      throw new Error('npm output must stay inside the declared output root');
    }
    if (seen.has(destination)) throw new Error('duplicate npm output path');
    seen.add(destination);
    let ancestor = root;
    for (const part of inside.split(sep).slice(0, -1)) {
      ancestor = join(ancestor, part);
      directory(ancestor);
    }
    if (item.data !== null && (!Buffer.isBuffer(item.data) || !item.data.length)) {
      throw new Error('npm payload must contain nonempty bytes');
    }
    const info = observe(destination);
    if (info && (!info.isFile() || info.isSymbolicLink())) {
      throw new Error('npm output must be a regular file');
    }
  }
  const id = randomUUID();
  try {
    for (const item of items) {
      mkdirSync(dirname(item.dst), { recursive: true });
      if (item.data !== null) {
        item.temp = `${item.dst}.stage-${id}`;
        writeFileSync(item.temp, item.data, { flag: 'wx' });
        if (item.exec) chmodSync(item.temp, 0o755);
        if (!readFileSync(item.temp).equals(item.data)) throw new Error('temporary npm payload differs');
      }
    }
    for (const item of items) {
      if (existsSync(item.dst)) {
        item.backup = `${item.dst}.backup-${id}`;
        renameSync(item.dst, item.backup);
      }
      if (item.data !== null) {
        renameSync(item.temp, item.dst);
        item.installed = true;
      }
    }
    for (const item of items) {
      if (item.data !== null && !readFileSync(item.dst).equals(item.data)) throw new Error('staged npm payload differs');
    }
  } catch (error) {
    let rollbackError;
    for (const item of [...items].reverse()) {
      try {
        if (item.installed) rmSync(item.dst, { force: true });
        if (item.backup) renameSync(item.backup, item.dst);
      } catch (failure) {
        item.keepBackup = true;
        rollbackError ??= failure;
      }
    }
    if (rollbackError) throw new AggregateError([error, rollbackError], 'npm staging failed; an original payload backup was retained for recovery');
    throw error;
  } finally {
    for (const item of items) {
      if (item.temp) rmSync(item.temp, { force: true });
      if (item.backup && !item.keepBackup) rmSync(item.backup, { force: true });
    }
  }
}

function main() {
  const artifacts = loadArtifacts();
  const binaries = artifacts.filter(
    (a) => a?.type === 'Binary' && (a.extra?.ID
      ? a.extra.ID === 'many-ai-cli'
      : a.name === 'many-ai-cli' || a.name === 'many-ai-cli.exe')
  );

  const errors = [];
  const pending = [];
  for (const [target, { pkg, binName }] of Object.entries(TARGETS)) {
    const [goos, goarch] = target.split('/');
    const matches = binaries.filter((a) => a.goos === goos && a.goarch === goarch);
    if (matches.length !== 1) {
      errors.push(`${target}: expected exactly one main binary, found ${matches.length}`);
      continue;
    }
    try {
      const art = matches[0];
      if (typeof art.path !== 'string' || !art.path.trim()) throw new Error('artifact path is missing');
      if (art.pathBase !== undefined && art.pathBase !== 'manifest') {
        throw new Error('unknown artifact pathBase; expected "manifest" or an omitted field');
      }
      const base = art.pathBase === 'manifest' ? distDir : repoRoot;
      const src = isAbsolute(art.path) ? art.path : join(base, art.path);
      const data = binaryBytes(src);
      const sha256 = createHash('sha256').update(data).digest('hex');
      const binDir = join(npmDir, pkg, 'bin');
      pending.push({ target, pkg, binName, data, sha256, binDir, dst: join(binDir, binName), goos });
    } catch (err) {
      errors.push(`${target}: ${err.message}`);
    }
  }

  if (errors.length) {
    throw new Error(`Preflight failed; npm outputs were not changed:\n${errors.join('\n')}`);
  }

  stagePayloads(pending.flatMap(item => [
    { dst: item.dst, data: item.data, exec: item.goos !== 'windows' },
    { dst: join(item.binDir, item.binName.endsWith('.exe') ? 'many-ai-cli' : 'many-ai-cli.exe'), data: null },
  ]));
  for (const item of pending) {
    console.log(`staged ${item.target} -> npm/${item.pkg}/bin/${item.binName} (SHA256 ${item.sha256})`);
  }
  console.log('\nall four platform binaries staged and byte-verified.');
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  try {
    main();
  } catch (err) {
    console.error(`npm staging failed: ${err.message}\nDo not publish these packages; correct the inputs and rerun staging.`);
    process.exitCode = 1;
  }
}
