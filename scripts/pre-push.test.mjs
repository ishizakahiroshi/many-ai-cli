import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, copyFileSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';

const ZERO = '0'.repeat(40);
const LOCAL = 'a'.repeat(40);
const REMOTE = 'b'.repeat(40);
const hookSource = process.env.MANY_AI_PREPUSH_BASELINE || new URL('../.githooks/pre-push', import.meta.url);
const windows = process.platform === 'win32';
const shell = windows ? 'C:/Program Files/Git/bin/bash.exe' : '/bin/sh';

// This converts only owned fixture paths for Git Bash. No WSL launcher, shell
// interpolation of source paths, real Git mutation or real Go invocation occurs.
function shellPath(path) {
  return windows ? path.replaceAll('\\', '/').replace(/^([A-Za-z]):\//, (_, drive) => `/${drive.toLowerCase()}/`) : path;
}

function invoke(t, { rootModule = false, frozenModule = true,
  files = ['README.md'], rangeExit = 0, nodeExit = 0, goExit = 0,
  newBranch = false, deletedBranch = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'many-ai-cli-prepush-'));
  t.after(() => {
    assert.equal(dirname(resolve(root)), resolve(tmpdir()));
    rmSync(root, { recursive: true, force: true });
  });
  const bin = join(root, 'stub bin');
  mkdirSync(bin);
  mkdirSync(join(root, '.githooks'));
  copyFileSync(hookSource, join(root, '.githooks/pre-push'));
  if (rootModule) writeFileSync(join(root, 'go.mod'), 'module example.invalid/active\n');
  if (frozenModule) {
    mkdirSync(join(root, 'go'));
    writeFileSync(join(root, 'go/go.mod'), 'module example.invalid/frozen\n');
  }
  const calls = join(root, 'calls.txt');
  writeFileSync(calls, '');
  const changed = join(root, 'changed-files.bin');
  writeFileSync(changed, Buffer.from(files.map(file => `${file}\0`).join('')));
  const stubs = {
    git: `#!/bin/sh
printf 'git:%s\\n' "$*" >> "$STUB_CALLS"
case "$1" in
  diff|log) cat "$STUB_CHANGED"; exit "$STUB_RANGE_EXIT" ;;
  *) exit 93 ;;
esac
`,
    node: `#!/bin/sh
printf 'node:%s\\n' "$*" >> "$STUB_CALLS"
exit "$STUB_NODE_EXIT"
`,
    go: `#!/bin/sh
printf 'go:%s\\n' "$*" >> "$STUB_CALLS"
exit "$STUB_GO_EXIT"
`,
  };
  for (const [name, body] of Object.entries(stubs)) writeFileSync(join(bin, name), body, { mode: 0o755 });
  const launcher = windows
    ? 'PATH="$STUB_BIN:$PATH"; export PATH; exec "$BASH" .githooks/pre-push "$@"'
    : 'PATH="$STUB_BIN:$PATH"; export PATH; exec /bin/sh .githooks/pre-push "$@"';
  const args = [...(windows ? ['--noprofile', '--norc'] : []), '-c', launcher,
    'pre-push-fixture', 'origin', 'https://example.invalid/repository.git'];
  const result = spawnSync(shell, args, {
    cwd: root, encoding: 'utf8', timeout: 15000,
    input: `refs/heads/develop ${deletedBranch ? ZERO : LOCAL} refs/heads/develop ${newBranch ? ZERO : REMOTE}\n`,
    env: { ...process.env, MANY_AI_CLI_SKIP_PREPUSH: '',
      STUB_BIN: shellPath(bin), STUB_CALLS: shellPath(calls), STUB_CHANGED: shellPath(changed),
      STUB_RANGE_EXIT: String(rangeExit), STUB_NODE_EXIT: String(nodeExit), STUB_GO_EXIT: String(goExit) },
  });
  assert.ifError(result.error);
  return { ...result, calls: readFileSync(calls, 'utf8').trim().split('\n').filter(Boolean) };
}

test('frozen Go file deletions run instrumentation but never root staticcheck', t => {
  const result = invoke(t, { files: ['go/internal/hub/debug_ui_freeze.go', 'go/internal/hub/debug_ui_freeze_test.go'] });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.deepEqual(result.calls.filter(call => call.startsWith('node:')), ['node:scripts/check-instrumentation.mjs']);
  assert.equal(result.calls.filter(call => call.startsWith('go:')).length, 0);
  assert.match(result.stdout, /go\.mod.*staticcheck/);
});

test('an active root Go module retains pinned staticcheck after instrumentation', t => {
  const result = invoke(t, { rootModule: true, files: ['cmd/example/main.go'] });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.deepEqual(result.calls.slice(1), ['node:scripts/check-instrumentation.mjs', 'go:run honnef.co/go/tools/cmd/staticcheck@v0.7.0 ./...']);
});

test('active root staticcheck failure stops the push', t => {
  const result = invoke(t, { rootModule: true, files: ['cmd/example/main.go'], goExit: 7 });
  assert.equal(result.status, 1, result.stdout + result.stderr);
  assert.equal(result.calls.filter(call => call.startsWith('go:')).length, 1);
  assert.match(result.stdout, /staticcheck.*止めました/);
});

for (const rootModule of [false, true]) test(`without Go changes staticcheck stays idle (root module=${rootModule})`, t => {
  const result = invoke(t, { rootModule });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.equal(result.calls.filter(call => call.startsWith('node:')).length, 1);
  assert.equal(result.calls.filter(call => call.startsWith('go:')).length, 0);
});

for (const rootModule of [false, true]) test(`instrumentation failure stops every layout before Go (root module=${rootModule})`, t => {
  const result = invoke(t, { rootModule, files: ['go/frozen.go'], nodeExit: 8 });
  assert.equal(result.status, 1, result.stdout + result.stderr);
  assert.equal(result.calls.filter(call => call.startsWith('node:')).length, 1);
  assert.equal(result.calls.filter(call => call.startsWith('go:')).length, 0);
  assert.match(result.stdout, /台帳検査で止めました/);
});

test('new branch Go changes retain active-module lint', t => {
  const result = invoke(t, { rootModule: true, files: ['cmd/example/main.go'], newBranch: true });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.ok(result.calls[0].startsWith('git:log '));
  assert.equal(result.calls.filter(call => call.startsWith('go:')).length, 1);
});

for (const rootModule of [false, true]) test(`unknown push range remains conservative for active modules (root module=${rootModule})`, t => {
  const result = invoke(t, { rootModule, files: [], rangeExit: 9 });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.equal(result.calls.filter(call => call.startsWith('node:')).length, 1);
  assert.equal(result.calls.filter(call => call.startsWith('go:')).length, rootModule ? 1 : 0);
});

test('deleting a remote branch still runs the instrumentation gate', t => {
  const result = invoke(t, { rootModule: true, deletedBranch: true });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.deepEqual(result.calls, ['node:scripts/check-instrumentation.mjs']);
});
