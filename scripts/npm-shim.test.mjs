// The npm command shim must start the platform executable only from inside
// its own installation (audit U38). Before, it used require.resolve, which
// goes on to every node_modules above the installation, NODE_PATH and the
// global folders; on Windows any signed-in user can create C:\node_modules.
//
// Each case builds an owned layout in a temporary directory. The
// "executables" are copies of this Node binary started with
// `-e process.stdout.write(process.execPath)`, so a case shows exactly which
// file the shim started.
import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, linkSync, mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = join(dirname(fileURLToPath(import.meta.url)), '..');
const SHIM = join(repo, 'npm', 'many-ai-cli', 'bin', 'many-ai-cli.mjs');
const PKG = {
  'win32 x64': 'many-ai-cli-windows-x64',
  'linux x64': 'many-ai-cli-linux-x64',
  'darwin x64': 'many-ai-cli-macos-intel',
  'darwin arm64': 'many-ai-cli-macos-apple-silicon',
}[`${process.platform} ${process.arch}`];
const BIN = process.platform === 'win32' ? 'many-ai-cli.exe' : 'many-ai-cli';
const skip = PKG ? false : `no platform package for ${process.platform} ${process.arch}`;

let work;
let nodeCopy;
let cases = 0;
after(() => {
  if (work) rmSync(work, { recursive: true, force: true });
});

function layout() {
  if (!work) {
    work = mkdtempSync(join(tmpdir(), 'many-ai-cli-shim-test-'));
    nodeCopy = join(work, `node-copy${process.platform === 'win32' ? '.exe' : ''}`);
    copyFileSync(process.execPath, nodeCopy);
  }
  const dir = join(work, `case-${++cases}`);
  mkdirSync(dir);
  return dir;
}

function executable(path) {
  mkdirSync(dirname(path), { recursive: true });
  try {
    linkSync(nodeCopy, path);
  } catch {
    copyFileSync(nodeCopy, path);
  }
  return path;
}

function rootPackage(packageDir) {
  mkdirSync(join(packageDir, 'bin'), { recursive: true });
  copyFileSync(SHIM, join(packageDir, 'bin', 'many-ai-cli.mjs'));
  writeFileSync(join(packageDir, 'package.json'), JSON.stringify({
    name: 'many-ai-cli',
    version: '1.0.0',
    type: 'module',
    optionalDependencies: { [PKG]: '1.0.0' },
  }));
  return join(packageDir, 'bin', 'many-ai-cli.mjs');
}

function platformPackage(dir, { name = PKG, version = '1.0.0', binary = true } = {}) {
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'package.json'), JSON.stringify({ name, version }));
  return binary ? executable(join(dir, 'bin', BIN)) : undefined;
}

function run(shim, env = {}) {
  return spawnSync(process.execPath, [shim, '-e', 'process.stdout.write(process.execPath)'], {
    encoding: 'utf8',
    timeout: 60000,
    env: { ...process.env, NODE_PATH: '', ...env },
  });
}

function assertStarted(result, binary) {
  assert.equal(result.status, 0, result.stderr);
  assert.equal(realpathSync.native(result.stdout), realpathSync.native(binary));
}

function assertRefused(result, detail) {
  assert.equal(result.stdout, '', 'an executable was started');
  assert.equal(result.status, 1, result.stderr);
  assert.match(result.stderr, /is not installed/);
  if (detail) assert.match(result.stderr, detail);
}

// The installation is <case>/home/npm, the layout of a global npm install.
// <case>/node_modules stands for C:\node_modules.
function globalInstall(root) {
  const packageDir = join(root, 'home', 'npm', 'node_modules', 'many-ai-cli');
  return { packageDir, shim: rootPackage(packageDir), planted: join(root, 'node_modules', PKG) };
}

test('an executable in a node_modules above the installation never starts', { skip }, () => {
  const { shim, planted } = globalInstall(layout());
  platformPackage(planted);
  assertRefused(run(shim));
});

test('a platform package without its executable does not fall through to a node_modules above', { skip }, () => {
  const { packageDir, shim, planted } = globalInstall(layout());
  platformPackage(join(packageDir, 'node_modules', PKG), { binary: false });
  platformPackage(planted);
  assertRefused(run(shim), /has no bin[\\/]/);
});

test('NODE_PATH is not searched', { skip }, () => {
  const root = layout();
  const { shim } = globalInstall(root);
  platformPackage(join(root, 'elsewhere', PKG));
  assertRefused(run(shim, { NODE_PATH: join(root, 'elsewhere') }));
});

for (const [label, manifest, detail] of [
  ['another version', { version: '0.9.0' }, /@0\.9\.0; this many-ai-cli needs .*@1\.0\.0/],
  ['another name', { name: 'not-many-ai-cli' }, /holds not-many-ai-cli@1\.0\.0/],
]) {
  test(`a platform package of ${label} is refused`, { skip }, () => {
    const { packageDir, shim } = globalInstall(layout());
    platformPackage(join(packageDir, 'node_modules', PKG), manifest);
    assertRefused(run(shim), detail);
  });
}

test('the platform package nested in a global npm installation starts', { skip }, () => {
  const { packageDir, shim, planted } = globalInstall(layout());
  const binary = platformPackage(join(packageDir, 'node_modules', PKG));
  platformPackage(planted);
  assertStarted(run(shim), binary);
});

test('a platform package hoisted next to many-ai-cli starts (npm, bun, yarn)', { skip }, () => {
  const modules = join(layout(), 'project', 'node_modules');
  const shim = rootPackage(join(modules, 'many-ai-cli'));
  const binary = platformPackage(join(modules, PKG));
  assertStarted(run(shim), binary);
});

test('a platform package hoisted to the top node_modules starts when many-ai-cli is nested', { skip }, () => {
  const modules = join(layout(), 'project', 'node_modules');
  const shim = rootPackage(join(modules, 'tool', 'node_modules', 'many-ai-cli'));
  const binary = platformPackage(join(modules, PKG));
  assertStarted(run(shim), binary);
});

test('a pnpm link to a store outside the installation is followed', { skip }, () => {
  const root = layout();
  const virtual = join(root, 'project', 'node_modules', '.pnpm', 'many-ai-cli@1.0.0', 'node_modules');
  const shim = rootPackage(join(virtual, 'many-ai-cli'));
  const store = join(root, 'store', PKG);
  const binary = platformPackage(store);
  symlinkSync(store, join(virtual, PKG), process.platform === 'win32' ? 'junction' : 'dir');
  assertStarted(run(shim), binary);
});

test('a command link outside the installation runs the shim from its real place', { skip }, (t) => {
  const root = layout();
  const { packageDir, shim } = globalInstall(root);
  const binary = platformPackage(join(packageDir, 'node_modules', PKG));
  const link = join(root, 'home', 'bin', 'many-ai-cli.mjs');
  mkdirSync(dirname(link), { recursive: true });
  try {
    symlinkSync(shim, link, 'file');
  } catch (error) {
    // Windows creates file links only with Developer Mode; npm uses .cmd there.
    if (process.platform === 'win32') return t.skip(`file link not available: ${error.code}`);
    throw error;
  }
  assertStarted(run(link), binary);
});
