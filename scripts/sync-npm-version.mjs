#!/usr/bin/env node
// Sync the version across every npm/ package.json so the published packages
// never drift from the release tag.
//
// Usage:
//   node scripts/sync-npm-version.mjs 0.3.0
//   node scripts/sync-npm-version.mjs            # derive from the latest git tag (vX.Y.Z)
//
// Updates:
//   - npm/many-ai-cli/package.json            -> version + optionalDependencies.*
//   - npm/many-ai-cli-<os>-<arch>/package.json -> version
//
// The root package's optionalDependencies are pinned to the EXACT same version
// so a published root never resolves a mismatched platform binary.

import { execSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const npmDir = join(repoRoot, 'npm');

const ROOT_PKG = 'many-ai-cli';
const PLATFORM_PKGS = [
  'many-ai-cli-windows-x64',
  'many-ai-cli-linux-x64',
  'many-ai-cli-macos-intel',
  'many-ai-cli-macos-apple-silicon',
];

function deriveVersion() {
  const arg = process.argv[2];
  if (arg) return arg.replace(/^v/, '');
  const tag = execSync('git describe --tags --abbrev=0', { cwd: repoRoot })
    .toString()
    .trim();
  return tag.replace(/^v/, '');
}

function isSemver(v) {
  return /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(v);
}

function patch(pkgDir, mutate) {
  const path = join(npmDir, pkgDir, 'package.json');
  const json = JSON.parse(readFileSync(path, 'utf8'));
  mutate(json);
  writeFileSync(path, JSON.stringify(json, null, 2) + '\n');
  console.log(`updated ${pkgDir}/package.json -> ${json.version}`);
}

function main() {
  const version = deriveVersion();
  if (!isSemver(version)) {
    console.error(`refusing to write invalid version: "${version}"`);
    process.exit(1);
  }

  patch(ROOT_PKG, (json) => {
    json.version = version;
    json.optionalDependencies ??= {};
    for (const dep of PLATFORM_PKGS) {
      if (dep in json.optionalDependencies) {
        json.optionalDependencies[dep] = version;
      }
    }
  });

  for (const pkg of PLATFORM_PKGS) {
    patch(pkg, (json) => {
      json.version = version;
    });
  }

  console.log(`\nall npm packages set to ${version}`);
}

main();
