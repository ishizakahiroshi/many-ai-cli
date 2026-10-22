#!/usr/bin/env node
// Copy canonical native binaries into each npm platform package's
// bin/ directory so the packages can be published.
//
// Input: dist/artifacts.json, an array using the existing artifact-adapter
// schema: type="Binary", extra.ID="many-ai-cli" (or the main binary name),
// goos/goarch and path. The field names are retained for compatibility; a Rust
// release adapter can supply the same manifest without changing package names.
// Relative artifact paths are resolved from the repository root, as before.
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
import { readFileSync, copyFileSync, chmodSync, mkdirSync, rmSync, renameSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, isAbsolute } from 'node:path';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const distArg = process.argv[2];
const distDir = distArg ? (isAbsolute(distArg) ? distArg : join(repoRoot, distArg)) : join(repoRoot, 'dist');
const npmDir = join(repoRoot, 'npm');

// Artifact manifest OS/arch tokens -> { pkg, binName }.
// Package names match the GitHub release archive suffixes.
const TARGETS = {
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

function binaryHash(path) {
  const stat = statSync(path);
  if (!stat.isFile() || stat.size === 0) throw new Error(`Binary is not a nonempty regular file: ${path}`);
  // Reading the bytes also proves readability, rather than accepting an entry
  // whose path exists but cannot be copied by this process.
  return createHash('sha256').update(readFileSync(path)).digest('hex');
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
      const src = isAbsolute(art.path) ? art.path : join(repoRoot, art.path);
      const sha256 = binaryHash(src);
      const binDir = join(npmDir, pkg, 'bin');
      pending.push({ target, pkg, binName, src, sha256, binDir, dst: join(binDir, binName), goos });
    } catch (err) {
      errors.push(`${target}: ${err.message}`);
    }
  }

  if (errors.length) {
    throw new Error(`Preflight failed; npm outputs were not changed:\n${errors.join('\n')}`);
  }

  const stagingID = randomUUID();
  try {
    // Keep existing binaries until every incoming file has been copied and
    // verified. Temporary names cannot be included by package.json files[].
    for (const item of pending) {
      mkdirSync(item.binDir, { recursive: true });
      item.temp = join(item.binDir, `${item.binName}.stage-${stagingID}`);
      copyFileSync(item.src, item.temp);
      if (item.goos !== 'windows') chmodSync(item.temp, 0o755);
      if (binaryHash(item.temp) !== item.sha256) throw new Error(`${item.target}: copied bytes differ from source`);
    }
    for (const item of pending) {
      // Both aliases are known package output filenames; remove no other files.
      for (const stale of ['many-ai-cli', 'many-ai-cli.exe']) {
        rmSync(join(item.binDir, stale), { force: true });
      }
      renameSync(item.temp, item.dst);
    }
    // Never report success while a destination contains stale or changed bytes.
    for (const item of pending) {
      if (binaryHash(item.dst) !== item.sha256) throw new Error(`${item.target}: staged bytes differ from source`);
    }
    for (const item of pending) {
      console.log(`staged ${item.target} -> npm/${item.pkg}/bin/${item.binName} (SHA256 ${item.sha256})`);
    }
    console.log('\nall four platform binaries staged and byte-verified.');
  } finally {
    for (const item of pending) {
      if (item.temp) rmSync(item.temp, { force: true });
    }
  }
}

try {
  main();
} catch (err) {
  console.error(`npm staging failed: ${err.message}\nDo not publish these packages; correct the inputs and rerun staging.`);
  process.exitCode = 1;
}
