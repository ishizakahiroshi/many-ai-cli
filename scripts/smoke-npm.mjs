#!/usr/bin/env node
// Local smoke checks for the npm packages — runnable without publishing or
// touching global install state.
//
// 1. `npm pack --dry-run` for every package and assert the published file set
//    is exactly what we expect (binary + shim + metadata, nothing else).
// 2. Require all four staged binaries, then exercise the current platform's
//    real shim resolution path (owned temporary layout) by running `--version` and
//    `version` through bin/many-ai-cli.mjs.
// Missing/empty binaries or an unsupported execution host are failures, not
// successful skips. Run only after staging the candidate's four artifacts.
//
// The global-install smoke (`pnpm add -g <tarball>` then `many-ai-cli --version`,
// which exercises the generated .cmd shim on Windows) is a manual / CI step
// because it mutates global state — see docs/manual_release.md.

import { execFileSync } from 'node:child_process';
import { accessSync, constants, copyFileSync, mkdtempSync, rmSync, mkdirSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { tmpdir } from 'node:os';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const npmDir = join(repoRoot, 'npm');

const PLATFORM_TARGETS = {
  'win32 x64': { pkg: 'many-ai-cli-windows-x64', binName: 'many-ai-cli.exe' },
  'linux x64': { pkg: 'many-ai-cli-linux-x64', binName: 'many-ai-cli' },
  'darwin x64': { pkg: 'many-ai-cli-macos-intel', binName: 'many-ai-cli' },
  'darwin arm64': { pkg: 'many-ai-cli-macos-apple-silicon', binName: 'many-ai-cli' },
};
const CURRENT = PLATFORM_TARGETS[`${process.platform} ${process.arch}`];

let failures = 0;
function check(name, fn) {
  try {
    fn();
    console.log(`  PASS  ${name}`);
  } catch (err) {
    failures++;
    console.log(`  FAIL  ${name}\n        ${err.message}`);
  }
}

function packFileList(pkgDir) {
  const out = execFileSync('npm', ['pack', '--dry-run', '--json'], {
    cwd: join(npmDir, pkgDir),
    encoding: 'utf8',
    shell: process.platform === 'win32',
  });
  const meta = JSON.parse(out);
  return (meta[0]?.files ?? []).map((f) => f.path).sort();
}

function assertSameSet(actual, expected, label) {
  const a = new Set(actual);
  const e = new Set(expected);
  const missing = [...e].filter((x) => !a.has(x));
  const extra = [...a].filter((x) => !e.has(x));
  if (missing.length || extra.length) {
    throw new Error(`${label}: missing=[${missing}] extra=[${extra}] got=[${actual}]`);
  }
}

function requireBinary(pkg, binName) {
  const binary = join(npmDir, pkg, 'bin', binName);
  const stat = statSync(binary);
  if (!stat.isFile() || stat.size === 0) throw new Error(`${pkg}: binary is not a nonempty regular file`);
  accessSync(binary, constants.R_OK);
  return binary;
}

console.log('pack content checks:');
check('root package ships only shim + readme + package.json', () => {
  assertSameSet(packFileList('many-ai-cli'), ['README.md', 'bin/many-ai-cli.mjs', 'package.json'], 'root');
});

for (const { pkg, binName } of Object.values(PLATFORM_TARGETS)) {
  check(`${pkg} ships only its binary + package.json`, () => {
    requireBinary(pkg, binName);
    assertSameSet(packFileList(pkg), ['package.json', `bin/${binName}`], pkg);
  });
}

console.log('\nshim execution check:');
if (!CURRENT) {
  check('current host can execute the staged native binary', () => {
    throw new Error(`Unsupported execution host ${process.platform} ${process.arch}; use a supported target to complete the smoke check.`);
  });
} else if (failures) {
  // A broken/missing platform package must stop acceptance even if the current
  // host could execute an old binary left over from a previous candidate.
  console.log('  NOT RUN  correct the package content failures before executing the shim.');
} else {
  // The shim resolves the platform package with createRequire(import.meta.url),
  // which walks up from the shim's own location. Mirror only the packaged files
  // in a fresh owned directory; never replace a developer's node_modules.
  let ownedTemp;
  try {
    ownedTemp = mkdtempSync(join(tmpdir(), 'many-ai-cli-npm-smoke-'));
    const rootPkg = join(ownedTemp, 'node_modules', 'many-ai-cli');
    const platformPkg = join(rootPkg, 'node_modules', CURRENT.pkg);
    mkdirSync(join(rootPkg, 'bin'), { recursive: true });
    mkdirSync(join(platformPkg, 'bin'), { recursive: true });
    copyFileSync(join(npmDir, 'many-ai-cli', 'package.json'), join(rootPkg, 'package.json'));
    const shim = join(rootPkg, 'bin', 'many-ai-cli.mjs');
    copyFileSync(join(npmDir, 'many-ai-cli', 'bin', 'many-ai-cli.mjs'), shim);
    copyFileSync(join(npmDir, CURRENT.pkg, 'package.json'), join(platformPkg, 'package.json'));
    copyFileSync(requireBinary(CURRENT.pkg, CURRENT.binName), join(platformPkg, 'bin', CURRENT.binName));
    for (const verArg of ['--version', 'version']) {
      check(`shim "${verArg}" exits 0 with output`, () => {
        const out = execFileSync(process.execPath, [shim, verArg], { encoding: 'utf8' });
        if (!out.trim()) throw new Error('no version output');
      });
    }
  } catch (err) {
    check('prepare isolated shim layout', () => { throw err; });
  } finally {
    // ownedTemp is the absolute directory returned by mkdtempSync above. Remove
    // only that exact owned path, never a repository or package-manager folder.
    if (ownedTemp) rmSync(ownedTemp, { recursive: true, force: true });
  }
}

console.log(failures ? `\n${failures} check(s) failed.` : '\nall smoke checks passed.');
process.exit(failures ? 1 : 0);
