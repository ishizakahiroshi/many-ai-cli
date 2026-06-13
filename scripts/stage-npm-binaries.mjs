#!/usr/bin/env node
// Copy the GoReleaser-built native binaries into each npm platform package's
// bin/ directory so the packages can be published.
//
// Source of truth: dist/artifacts.json (written by `goreleaser release` /
// `goreleaser build`). We match the main `many-ai-cli` binary (not the
// launcher) for each goos/goarch and copy it to the matching npm package.
//
// Usage:
//   node scripts/stage-npm-binaries.mjs            # reads dist/artifacts.json
//   node scripts/stage-npm-binaries.mjs <distDir>  # custom dist dir
//
// Exits non-zero if any supported target's binary is missing.

import { readFileSync, copyFileSync, chmodSync, mkdirSync, rmSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, isAbsolute } from 'node:path';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const distArg = process.argv[2];
const distDir = distArg ? (isAbsolute(distArg) ? distArg : join(repoRoot, distArg)) : join(repoRoot, 'dist');
const npmDir = join(repoRoot, 'npm');

// goos/goarch -> { pkg, binName }
// Package names match the GitHub release archive suffixes.
const TARGETS = {
  'windows/amd64': { pkg: 'many-ai-cli-windows-x64', binName: 'many-ai-cli.exe' },
  'linux/amd64': { pkg: 'many-ai-cli-linux-x64', binName: 'many-ai-cli' },
  'darwin/amd64': { pkg: 'many-ai-cli-macos-intel', binName: 'many-ai-cli' },
  'darwin/arm64': { pkg: 'many-ai-cli-macos-apple-silicon', binName: 'many-ai-cli' },
};

function loadArtifacts() {
  const path = join(distDir, 'artifacts.json');
  if (!existsSync(path)) {
    console.error(`dist/artifacts.json not found at ${path}.\nRun "goreleaser release" or "goreleaser build" first.`);
    process.exit(1);
  }
  return JSON.parse(readFileSync(path, 'utf8'));
}

function main() {
  const artifacts = loadArtifacts();
  // The main binary build id is "many-ai-cli" (see .goreleaser.yaml builds[].id).
  const binaries = artifacts.filter(
    (a) => a.type === 'Binary' && (a.extra?.ID === 'many-ai-cli' || a.name === 'many-ai-cli' || a.name === 'many-ai-cli.exe')
  );

  const missing = [];
  for (const [target, { pkg, binName }] of Object.entries(TARGETS)) {
    const [goos, goarch] = target.split('/');
    const art = binaries.find((a) => a.goos === goos && a.goarch === goarch);
    if (!art) {
      missing.push(target);
      continue;
    }
    const src = isAbsolute(art.path) ? art.path : join(repoRoot, art.path);
    const binDir = join(npmDir, pkg, 'bin');
    const dst = join(binDir, binName);
    mkdirSync(binDir, { recursive: true });
    // Remove any stale binary before copying.
    for (const stale of ['many-ai-cli', 'many-ai-cli.exe']) {
      const p = join(binDir, stale);
      if (existsSync(p)) rmSync(p);
    }
    copyFileSync(src, dst);
    if (goos !== 'windows') chmodSync(dst, 0o755);
    console.log(`staged ${target} -> npm/${pkg}/bin/${binName}`);
  }

  if (missing.length) {
    console.error(`\nmissing binaries for: ${missing.join(', ')}`);
    process.exit(1);
  }
  console.log('\nall platform binaries staged.');
}

main();
