import { test, expect } from 'bun:test';
import { readFileSync } from 'node:fs';

const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');
const screen = read('../src/app/agent-session.ts');
const model = read('../src/app/agent-session-model.ts');
const stripComments = (source: string) => source.split('\n').filter((line) => !line.trim().startsWith('//')).join('\n');

test('the screen never answers, retries or continues by itself, and never polls usage', () => {
  const code = stripComments(screen) + stripComments(model);
  expect(code).not.toMatch(/setInterval/);
  // 唯一の setTimeout は、待たずに返った長いポーリングが回り続けないための 1 秒の間。
  expect(code.match(/setTimeout/g)?.length).toBe(1);
  expect(code).not.toMatch(/\/api\/acp\/[^`'"]*usage/);
  expect(code).not.toMatch(/usage_url|\/usage/);
});

test('every Hub call is under /api/acp/ and permission bodies come from permissionAnswer', () => {
  const code = stripComments(screen);
  const paths = Array.from(code.matchAll(/['`](\/api\/[^'`]*)['`]/g)).map((match) => match[1]);
  expect(paths.length).toBeGreaterThan(0);
  for (const path of paths) expect(path.startsWith('/api/acp/')).toBe(true);
  expect(code).not.toMatch(/\bfetch\(/);
  expect(code.match(/permissionAnswer\(/g)?.length).toBe(2);
  // 許可のボタンは state.pending（Hub の保留中の記録）だけから作る。
  expect(code).toContain('state.pending.map(permissionBlock)');
});

test('the screen builds text with textContent only', () => {
  expect(stripComments(screen)).not.toMatch(/innerHTML|insertAdjacentHTML|outerHTML/);
});

test('the screen has no automation entry point and the dialog carries the wheel overlay class', () => {
  const code = stripComments(screen);
  expect(code).not.toMatch(/spawn-child|orchestrate|relay|routines|auto[-_ ]?approv/i);
  expect(code).toContain('aac-wheel-overlay');
});
