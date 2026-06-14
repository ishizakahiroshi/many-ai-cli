#!/usr/bin/env node
// Stage npm platform binaries by EXTRACTING them from the published GitHub
// Release archives, after verifying each archive against SHA256SUMS.txt.
//
// This is the identity-preserving path used by the npm-only re-run
// (workflow_dispatch). Because the binary npm ships is pulled out of the exact
// same .zip that GitHub serves and that winget/Homebrew reference (and verified
// against the cosign-signed SHA256SUMS), all four channels are provably the same
// bytes — not "rebuilt and assumed identical".
//
// Usage:
//   node scripts/stage-npm-from-release.mjs <dir>
//     <dir> must contain the 4 platform .zip files and SHA256SUMS.txt,
//     e.g. produced by:  gh release download <tag> --dir <dir>
//
// Requires the `unzip` CLI on PATH (present on ubuntu runners).
// Exits non-zero on any missing archive, checksum mismatch, or missing binary.

import { readFileSync, copyFileSync, chmodSync, mkdirSync, rmSync, existsSync, readdirSync, createReadStream } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, isAbsolute, basename } from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const dirArg = process.argv[2];
if (!dirArg) {
  console.error('usage: node scripts/stage-npm-from-release.mjs <dir-with-zips-and-SHA256SUMS.txt>');
  process.exit(1);
}
const srcDir = isAbsolute(dirArg) ? dirArg : join(repoRoot, dirArg);
const npmDir = join(repoRoot, 'npm');

// Archive name suffix -> { pkg, binName }. Archive base names look like
// many-ai-cli-<ver>-<suffix>.zip and unzip to a same-named top-level folder.
const TARGETS = [
  { suffix: 'windows-x64', pkg: 'many-ai-cli-windows-x64', binName: 'many-ai-cli.exe', exec: false },
  { suffix: 'linux-x64', pkg: 'many-ai-cli-linux-x64', binName: 'many-ai-cli', exec: true },
  { suffix: 'macos-intel', pkg: 'many-ai-cli-macos-intel', binName: 'many-ai-cli', exec: true },
  { suffix: 'macos-apple-silicon', pkg: 'many-ai-cli-macos-apple-silicon', binName: 'many-ai-cli', exec: true },
];

function sha256(path) {
  return new Promise((resolve, reject) => {
    const h = createHash('sha256');
    createReadStream(path)
      .on('error', reject)
      .on('data', (d) => h.update(d))
      .on('end', () => resolve(h.digest('hex')));
  });
}

// Parse "<hash>  <filename>" lines (sha256sum format) into a map by basename.
function loadChecksums() {
  const path = join(srcDir, 'SHA256SUMS.txt');
  if (!existsSync(path)) {
    console.error(`SHA256SUMS.txt not found in ${srcDir}. Download it with the release zips.`);
    process.exit(1);
  }
  const map = new Map();
  for (const line of readFileSync(path, 'utf8').split('\n')) {
    const m = line.trim().match(/^([0-9a-fA-F]{64})\s+\*?(.+)$/);
    if (m) map.set(basename(m[2]), m[1].toLowerCase());
  }
  return map;
}

function findZip(suffix) {
  const matches = readdirSync(srcDir).filter((f) => f.endsWith(`-${suffix}.zip`));
  if (matches.length !== 1) return null;
  return matches[0];
}

async function main() {
  if (!existsSync(srcDir)) {
    console.error(`source dir not found: ${srcDir}`);
    process.exit(1);
  }
  const sums = loadChecksums();
  const tmpRoot = join(srcDir, '.unzip');
  rmSync(tmpRoot, { recursive: true, force: true });
  mkdirSync(tmpRoot, { recursive: true });

  const problems = [];
  for (const { suffix, pkg, binName, exec } of TARGETS) {
    const zipName = findZip(suffix);
    if (!zipName) {
      problems.push(`no unique zip for ${suffix}`);
      continue;
    }
    const zipPath = join(srcDir, zipName);

    // 1) verify the archive against the (cosign-signed) checksums file.
    const expected = sums.get(zipName);
    if (!expected) {
      problems.push(`${zipName}: not listed in SHA256SUMS.txt`);
      continue;
    }
    const actual = (await sha256(zipPath)).toLowerCase();
    if (actual !== expected) {
      problems.push(`${zipName}: sha256 mismatch (expected ${expected.slice(0, 12)}, got ${actual.slice(0, 12)})`);
      continue;
    }

    // 2) extract and copy the main binary (not the launcher) into the npm pkg.
    const outDir = join(tmpRoot, suffix);
    mkdirSync(outDir, { recursive: true });
    const r = spawnSync('unzip', ['-o', '-q', zipPath, '-d', outDir], { stdio: 'inherit' });
    if (r.status !== 0) {
      problems.push(`${zipName}: unzip failed (exit ${r.status})`);
      continue;
    }
    const zipBase = zipName.replace(/\.zip$/, '');
    const src = join(outDir, zipBase, binName);
    if (!existsSync(src)) {
      problems.push(`${zipName}: ${binName} not found at expected path`);
      continue;
    }
    const binDir = join(npmDir, pkg, 'bin');
    mkdirSync(binDir, { recursive: true });
    for (const stale of ['many-ai-cli', 'many-ai-cli.exe']) {
      const p = join(binDir, stale);
      if (existsSync(p)) rmSync(p);
    }
    copyFileSync(src, join(binDir, binName));
    if (exec) chmodSync(join(binDir, binName), 0o755);
    console.log(`verified + staged ${suffix} (sha256 ${actual.slice(0, 12)}) -> npm/${pkg}/bin/${binName}`);
  }

  rmSync(tmpRoot, { recursive: true, force: true });

  if (problems.length) {
    console.error('\nstaging from release failed:');
    for (const p of problems) console.error(`  - ${p}`);
    process.exit(1);
  }
  console.log('\nall platform binaries verified against SHA256SUMS.txt and staged.');
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
