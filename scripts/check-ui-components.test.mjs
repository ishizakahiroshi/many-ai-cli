import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { appearanceFindings, cssDeclarations, cssSyntaxIssues, codeFindings, audit } from './check-ui-components.mjs';

test('nested CSS respects quoted braces, functions and comments', () => {
  const result = cssDeclarations('@media (min-width: 400px) { .ui-copy { content: "}; #fff"; background-image: url("data:image/svg+xml;{a}"); color: var(--text); } }', 'web/src/styles/example.css');
  assert.deepEqual(result.errors, []);
  assert.equal(result.declarations.length, 3);
  assert.equal(appearanceFindings(result.declarations).length, 0);
});
test('new raw colors and font sizes fail outside shared tokens', () => {
  const result = cssDeclarations('.example { color: #abc; font-size: 13px; }', 'web/src/styles/example.css');
  assert.deepEqual(appearanceFindings(result.declarations).map(f => f.rule), ['literal-color', 'literal-font-size']);
});
test('local button appearance is blocked even if it uses a token', () => {
  const result = cssDeclarations('.new-button { color: var(--text); display: flex; }', 'web/src/styles/example.css');
  assert.equal(appearanceFindings(result.declarations).length, 1);
  assert.equal(appearanceFindings(result.declarations)[0].rule, 'local-control-appearance');
});
test('component tokens and pure layout do not need exceptions', () => {
  const result = cssDeclarations(':root { --example: #abc; } .row-button { gap: 8px; display: flex; }', 'web/src/styles.css');
  assert.equal(appearanceFindings(result.declarations).length, 0);
});
test('unterminated CSS cannot silently pass the appearance gate', () => {
  assert.equal(cssDeclarations('@media (min-width: 400px) { .example { color: var(--text); }', 'broken.css').errors.length, 1);
});
test('new Hub confirmations must declare their action; strings are not calls', () => {
  assert.equal(codeFindings("const example = 'appConfirm({})'; appConfirm({ action: 'delete', message: text });", 'example.ts').length, 0);
  assert.equal(codeFindings("appConfirm({ message: text, confirmText: 'OK' });", 'example.ts')[0].rule, 'confirmation-without-action');
});
test('new literal DOM colors are blocked while variable colors are preserved', () => {
  assert.equal(codeFindings("button.style.color = '#fff';", 'example.ts')[0].rule, 'literal-inline-appearance');
  assert.equal(codeFindings("button.style.color = 'var(--text)';", 'example.ts').length, 0);
});
test('reviewed exceptions have exact occurrence limits and require reasons', () => {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'many-ui-guard-'));
  try {
    fs.mkdirSync(path.join(temporary, 'web/src/styles'), { recursive: true });
    const file = path.join(temporary, 'web/src/styles/example.css');
    fs.writeFileSync(file, '.example { color: #abc; }');
    const first = audit(temporary);
    assert.equal(first.blocked.length, 1);
    const entry = { fingerprint: first.blocked[0].fingerprint, count: 1, reason: 'Synthetic semantic color example' };
    assert.equal(audit(temporary, { entries: [entry] }).blocked.length, 0);
    fs.writeFileSync(file, '.example { color: #abc; } .example { color: #abc; }');
    assert.equal(audit(temporary, { entries: [entry] }).blocked.length, 1);
    assert.equal(audit(temporary, { entries: [{ ...entry, reason: '' }] }).errors.length, 1);
  } finally {
    const root = path.resolve(os.tmpdir()) + path.sep;
    assert.ok(path.resolve(temporary).startsWith(root));
    fs.rmSync(temporary, { recursive: true });
  }
});
test('named colors and percentage font sizes also count as raw values', () => {
  const result = cssDeclarations('.example { color: red; font-size: 90%; }', 'web/src/styles/example.css');
  assert.deepEqual(appearanceFindings(result.declarations).map(f => f.rule), ['literal-color', 'literal-font-size']);
});

test('directional borders, uncommon named colors and literal cssText are blocked', () => {
  assert.equal(appearanceFindings(cssDeclarations('.new-button { border-top: 1px solid #123456; }', 'example.css').declarations).length, 2);
  assert.equal(appearanceFindings(cssDeclarations('.example { color: aliceblue; }', 'example.css').declarations)[0].rule, 'literal-color');
  assert.equal(codeFindings('button.style.cssText = "color: #123456; font-size: 13px;";', 'example.ts')[0].rule, 'literal-inline-appearance');
});

test('font shorthand cannot reintroduce a raw size', () => {
  assert.equal(appearanceFindings(cssDeclarations('.example { font: 14px sans-serif; }', 'example.css').declarations)[0].rule, 'literal-font-size');
});

test('the actual CLI exits nonzero when a clean fixture gains a raw color', () => {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'many-ui-cli-'));
  try {
    fs.mkdirSync(path.join(temporary, 'web/src/styles'), { recursive: true });
    const file = path.join(temporary, 'web/src/styles/example.css');
    const exceptions = path.join(temporary, 'exceptions.json');
    fs.writeFileSync(exceptions, JSON.stringify({ entries: [] }));
    fs.writeFileSync(file, '.row { display: flex; color: var(--text); }');
    const run = () => spawnSync(process.execPath, [fileURLToPath(new URL('./check-ui-components.mjs', import.meta.url)), '--root', temporary, '--exceptions', exceptions, '--json'], { encoding: 'utf8' });
    const clean = run();
    assert.equal(clean.status, 0, clean.stderr);
    assert.equal(JSON.parse(clean.stdout).cssFiles, 1); // a silent/skipped main must fail
    fs.appendFileSync(file, '\n.new-button { border-top: 1px solid #123456; }');
    const changed = run();
    assert.equal(changed.status, 1, changed.stderr);
    assert.ok(JSON.parse(changed.stdout).blocked.some(f => f.rule === 'literal-color'));
    console.log(`CLI mutation: clean exit=${clean.status}, added raw color exit=${changed.status}`);
  } finally {
    assert.ok(path.resolve(temporary).startsWith(path.resolve(os.tmpdir()) + path.sep));
    fs.rmSync(temporary, { recursive: true });
  }
});

test('dangling selectors and a selector joined to an at-rule fail CSS parsing', () => {
  assert.equal(cssSyntaxIssues('.view-tab.drop-before\n.view-tab.drop-after', 'example.css').length, 1);
  assert.ok(cssSyntaxIssues('.has-pending\n@media (pointer: coarse) { .badge { color: var(--text); } }', 'example.css').length > 0);
  assert.deepEqual(cssSyntaxIssues('@media (pointer: coarse) { .badge { color: var(--text); } }', 'example.css'), []);
});

test('screen-local token overrides cannot reintroduce a raw color', () => {
  const local = cssDeclarations('.new-button { --ui-button-bg: #123456; }', 'web/src/styles/example.css');
  assert.equal(appearanceFindings(local.declarations)[0].rule, 'literal-local-token-color');
  const semantic = cssDeclarations('.new-button.is-selected { --ui-button-bg: var(--accent-soft); }', 'web/src/styles/example.css');
  assert.equal(appearanceFindings(semantic.declarations).length, 0);
});
