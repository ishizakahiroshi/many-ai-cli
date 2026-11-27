import test from 'node:test';
import assert from 'node:assert/strict';
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
import { countHits, compare } from './check-provider-hardcode.mjs';

test('quoted provider ids are counted; comments, tests and bare words are not', () => {
  const src = [
    '// "claude" in a comment',
    'let a = "codex";',
    "const b = 'cursor-agent';",
    'let c = claude_path;',
    '#[cfg(test)]',
    'mod t { let d = "grok"; }',
  ].join('\n');
  assert.equal(countHits('x.rs', src).length, 2);
  assert.equal(countHits('x.ts', 'const a = `opencode`; const b = "command-code";').length, 2);
});

test('only the item under #[cfg(test)] is skipped; code below it is still counted', () => {
  const src = [
    '#[cfg(test)]',
    '#[path = "x_tests.rs"]',
    'mod tests;',
    'let a = "codex";',
    '#[cfg(test)]',
    'use super::{a, b};',
    '#[cfg(test)]',
    'pub(crate) fn probe<A, B>(a: [u8; 3]) -> B',
    'where',
    '    A: Into<B>,',
    '{',
    '    let s = r#"',
    '}',
    '"#;',
    '    let c = \'}\'; // }',
    '    "grok"',
    '}',
    '#[cfg(test)]',
    'mod inline {',
    '    const J: &str = "{ \\" }";',
    '    const P: &str = "opencode";',
    '}',
    'let b = "claude";',
  ].join('\r\n');
  assert.deepEqual(countHits('x.rs', src).map((h) => h.line), [4, 23]);
  const field = 'struct S {\n#[cfg(test)]\nprobe: fn(u8, u8),\nname: &str,\n}\nlet d = "grok";';
  assert.deepEqual(countHits('x.rs', field).map((h) => h.line), [6]);
});

test('a file outside the baseline, or above its count, fails; below is only shrinkable', () => {
  const hit = [{ line: 1, text: 'x' }];
  const scanned = new Map([
    ['new.rs', hit],
    ['old.rs', [...hit, ...hit]],
    ['less.rs', hit],
  ]);
  const base = {
    'old.rs': { count: 1, reason: 'r' },
    'less.rs': { count: 3, reason: 'r' },
    'gone.rs': { count: 1, reason: '' },
  };
  const r = compare(scanned, base);
  assert.deepEqual(r.errors.map((e) => e.rel).sort(), ['new.rs', 'old.rs']);
  assert.deepEqual(r.shrinkable.map((e) => e.rel).sort(), ['gone.rs', 'less.rs']);
  assert.deepEqual(r.noReason, ['gone.rs']);
});

test('a pattern key with * pools the files it matches into one allowance', () => {
  const hit = [{ line: 1, text: 'x' }];
  const scanned = new Map([
    ['web/src/app/a-box.ts', [...hit, ...hit]],
    ['web/src/app/b-box.ts', hit],
    ['web/src/app/c-view.ts', hit],
  ]);
  const ok = compare(scanned, { 'web/src/app/*-box.ts': { count: 3, reason: 'r' } });
  assert.deepEqual(ok.errors.map((e) => e.rel), ['web/src/app/c-view.ts']);
  const over = compare(scanned, { 'web/src/app/*-box.ts': { count: 2, reason: 'r' } });
  assert.equal(over.errors[0].rel, 'web/src/app/*-box.ts');
});

test('a run through a junction or symlink still checks (and fails on a new hit)', () => {
  const work = mkdtempSync(join(tmpdir(), 'provider-hardcode-link-'));
  try {
    const repo = join(work, 'repo');
    mkdirSync(join(repo, 'scripts'), { recursive: true });
    mkdirSync(join(repo, 'rust/src'), { recursive: true });
    copyFileSync(new URL('./check-provider-hardcode.mjs', import.meta.url), join(repo, 'scripts/check-provider-hardcode.mjs'));
    writeFileSync(join(repo, 'scripts/provider-hardcode-baseline.json'), JSON.stringify({ files: {} }));
    writeFileSync(join(repo, 'rust/src/lib.rs'), 'fn f() -> &str { "codex" }\n');
    const link = join(work, 'link');
    symlinkSync(repo, link, 'junction');
    for (const root of [repo, link]) {
      const r = spawnSync(process.execPath, [join(root, 'scripts', 'check-provider-hardcode.mjs')], { encoding: 'utf8' });
      assert.equal(r.status, 1, `${root}: ${r.stdout}${r.stderr}`);
      assert.match(r.stderr, /rust\/src\/lib\.rs/);
    }
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
});
