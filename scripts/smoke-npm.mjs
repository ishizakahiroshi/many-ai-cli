#!/usr/bin/env node
// Local smoke checks for the npm packages — runnable without publishing or
// touching global install state.
//
// 1. `npm pack --dry-run` for every package and assert the published file set
//    is exactly what we expect (binary + shim + metadata, nothing else).
// 2. If the current platform's binary is staged, exercise the real shim
//    resolution path (temp local node_modules) by running `--version` and
//    `version` through bin/many-ai-cli.mjs.
//
// The global-install smoke (`pnpm add -g <tarball>` then `many-ai-cli --version`,
// which exercises the generated .cmd shim on Windows) is a manual / CI step
// because it mutates global state — see docs/manual_release.md.

import { execFileSync } from 'node:child_process';
import { cpSync, existsSync, rmSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const npmDir = join(repoRoot, 'npm');

const PLATFORM_PKGS = [
  'many-ai-cli-windows-x64',
  'many-ai-cli-linux-x64',
  'many-ai-cli-macos-intel',
  'many-ai-cli-macos-apple-silicon',
];

const CURRENT = {
  'win32 x64': { pkg: 'many-ai-cli-windows-x64', binName: 'many-ai-cli.exe' },
  'linux x64': { pkg: 'many-ai-cli-linux-x64', binName: 'many-ai-cli' },
  'darwin x64': { pkg: 'many-ai-cli-macos-intel', binName: 'many-ai-cli' },
  'darwin arm64': { pkg: 'many-ai-cli-macos-apple-silicon', binName: 'many-ai-cli' },
}[`${process.platform} ${process.arch}`];

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

console.log('pack content checks:');
check('root package ships only shim + readme + package.json', () => {
  assertSameSet(packFileList('many-ai-cli'), ['README.md', 'bin/many-ai-cli.mjs', 'package.json'], 'root');
});

for (const pkg of PLATFORM_PKGS) {
  check(`${pkg} ships only its binary + package.json`, () => {
    const binName = pkg.includes('win32') ? 'many-ai-cli.exe' : 'many-ai-cli';
    const files = packFileList(pkg);
    // package.json is always included; the binary is only present once staged.
    const allowed = new Set(['package.json', `bin/${binName}`]);
    const extra = files.filter((f) => !allowed.has(f));
    if (extra.length) throw new Error(`${pkg}: unexpected files [${extra}]`);
  });
}

console.log('\nshim execution check:');
if (!CURRENT) {
  console.log(`  SKIP  unsupported dev platform ${process.platform} ${process.arch}`);
} else if (!existsSync(join(npmDir, CURRENT.pkg, 'bin', CURRENT.binName))) {
  console.log(`  SKIP  ${CURRENT.pkg}/bin/${CURRENT.binName} not staged (run stage-npm-binaries.mjs first)`);
} else {
  // The shim resolves the platform package with createRequire(import.meta.url),
  // which walks up from the shim's own location. Mirror the published layout by
  // placing the platform package under the root package's node_modules.
  const nm = join(npmDir, 'many-ai-cli', 'node_modules', CURRENT.pkg);
  try {
    rmSync(join(npmDir, 'many-ai-cli', 'node_modules'), { recursive: true, force: true });
    mkdirSync(nm, { recursive: true });
    cpSync(join(npmDir, CURRENT.pkg), nm, { recursive: true });
    const shim = join(npmDir, 'many-ai-cli', 'bin', 'many-ai-cli.mjs');
    for (const verArg of ['--version', 'version']) {
      check(`shim "${verArg}" exits 0 with output`, () => {
        const out = execFileSync('node', [shim, verArg], { encoding: 'utf8' });
        if (!out.trim()) throw new Error('no version output');
      });
    }
  } finally {
    rmSync(join(npmDir, 'many-ai-cli', 'node_modules'), { recursive: true, force: true });
  }
}

console.log(failures ? `\n${failures} check(s) failed.` : '\nall smoke checks passed.');
process.exit(failures ? 1 : 0);
