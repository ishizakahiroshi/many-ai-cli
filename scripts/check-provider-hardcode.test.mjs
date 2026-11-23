import test from 'node:test';
import assert from 'node:assert/strict';
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
