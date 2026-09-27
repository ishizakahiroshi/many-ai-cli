import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, copyFileSync, existsSync, rmSync } from 'node:fs';
import { join, resolve, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';

test('release requires explicit purge; purge preserves hooks and clears registration', () => {
  const root = mkdtempSync(join(tmpdir(), 'instrumentation-purge-test-'));
  try {
    mkdirSync(join(root, 'scripts'));
    mkdirSync(join(root, 'web/src/debug'), { recursive: true });
    copyFileSync(new URL('./check-instrumentation.mjs', import.meta.url), join(root, 'scripts/check-instrumentation.mjs'));
    assert.equal(spawnSync('git', ['init', '--quiet', root]).status, 0);
    writeFileSync(join(root, 'web/src/debug/index.ts'), "import './capture.js';\n");
    writeFileSync(join(root, 'web/src/debug/capture.ts'), '// sink\n');
    writeFileSync(join(root, 'web/src/debug/probe.ts'), '// permanent hook\n');
    writeFileSync(join(root, 'instrumentation.json'), JSON.stringify({ entries: [{
      id: 'capture', status: 'active', gate: 'debug build', due: '2999-01-01',
      artifactNeedles: ['capture-only'],
      files: ['web/src/debug/capture.ts'], sharedFiles: ['web/src/debug/probe.ts'],
    }] }));
    const run = (...args) => spawnSync(process.execPath, ['scripts/check-instrumentation.mjs', ...args], { cwd: root, encoding: 'utf8' });
    assert.equal(run().status, 0);
    const blocked = run('--release');
    assert.equal(blocked.status, 1);
    assert.match(blocked.stderr, /capture:.*purge/);
    assert.equal(run('--release', '--purge', '--id=capture').status, 1);
    assert.ok(existsSync(join(root, 'web/src/debug/capture.ts')));
    const purged = run('--purge', '--id=capture');
    assert.equal(purged.status, 0, purged.stderr);
    assert.ok(!existsSync(join(root, 'web/src/debug/capture.ts')));
    assert.equal(readFileSync(join(root, 'web/src/debug/probe.ts'), 'utf8'), '// permanent hook\n');
    assert.ok(!readFileSync(join(root, 'web/src/debug/index.ts'), 'utf8').includes('capture'));
    const ledger = JSON.parse(readFileSync(join(root, 'instrumentation.json'), 'utf8'));
    assert.equal(ledger.entries[0].status, 'removed');
    assert.equal(run('--release').status, 0);
    // Merely relabelling the ledger while leaving the sink behind must fail.
    writeFileSync(join(root, 'web/src/debug/capture.ts'), '// forgotten sink\n');
    assert.equal(run('--release').status, 1);
  } finally {
    assert.equal(dirname(resolve(root)), resolve(tmpdir()));
    rmSync(root, { recursive: true, force: true });
  }
});
