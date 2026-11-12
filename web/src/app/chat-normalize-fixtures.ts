import assert from 'node:assert/strict';
import test from 'node:test';
import { normalizeChatText } from './chat-normalize.js';

// 監査 N34 の固定。時間の計測は node（ブラウザの Chrome と同じ V8）で回すこの fixture に置く。
// bun の test（別の JavaScript エンジン）では、行末の空白の正規表現は遅くならず、違いが出ない。

// 承認の印は、この file の文字としては分けて書く（印をそのまま書くと、画面の filter が本物と区別できない）。
const OPEN = `[${'MANY-AI-CLI'}]`;
const CLOSE = `[/${'MANY-AI-CLI'}]`;

// N34 より前の実装（行末の空白と、前後の改行を正規表現で落としていた）。結果が同じことの照合に使う。
function before(raw: string): string {
  if (!raw) return '';
  const s = raw.split(OPEN).map((part, index) => {
    if (index === 0) return part;
    const end = part.indexOf(CLOSE);
    return end < 0 ? OPEN + part : part.slice(end + CLOSE.length);
  }).join('');
  const out: string[] = [];
  let inFence = false;
  let blankRun = 0;
  for (let line of s.split(/\r?\n/)) {
    const isFenceMarker = /^\s*```/.test(line);
    if (isFenceMarker) inFence = !inFence;
    if (!inFence) {
      line = line.replace(/^[\s]*[▌>$#]\s?/, '');
      line = line.replace(/\s+$/, '');
    }
    if (!inFence && line.trim() === '') {
      blankRun++;
      if (blankRun <= 1) out.push('');
    } else {
      blankRun = 0;
      out.push(line);
    }
  }
  return out.join('\n').replace(/^\n+|\n+$/g, '');
}

function elapsed(run: () => void): number {
  const started = performance.now();
  run();
  return performance.now() - started;
}

// 直す前は、下の 2 つの入力にそれぞれ 10 秒前後かかった。
const LIMIT_MS = 2000;

test('normalizeChatText: 結果は今までと同じ', () => {
  const cases = [
    '',
    'plain',
    'trailing spaces   \t \nnext　 ﻿\n',
    '\n\n\nfirst\n\n\n\nsecond\n\n\n',
    '> quoted \n$ command  \n# heading\n▌ prompt',
    '   \n \t \nonly blank lines above\n   ',
    'before\n```\n  keep   \n\n\n\ninside\n```\nafter  ',
    '```\nunclosed fence   \n\n\n',
    'text\r\nwith crlf  \r\n\r\n\r\nend\r\n',
    'x   \ny      ',
    '\n',
    '   ',
    `${OPEN}\nq? (Y:1/N:0)\n${CLOSE}\nkept  \n`,
  ];
  for (const value of cases) {
    assert.equal(normalizeChatText(value), before(value), JSON.stringify(value));
  }
});

test('normalizeChatText: 空白が 20 万個続く 1 行でも、すぐ終わる', () => {
  const line = `x${' '.repeat(200_000)}y`;
  let result = '';
  const ms = elapsed(() => { result = normalizeChatText(`first\n${line}\nlast`); });
  assert.equal(result, `first\n${line}\nlast`);
  assert.ok(ms < LIMIT_MS, `${Math.round(ms)} ms`);
});

test('normalizeChatText: 空白だけの長い行・空白で始まる長い行でも、すぐ終わる', () => {
  const blank = ' '.repeat(200_000);
  const ms = elapsed(() => {
    assert.equal(normalizeChatText(`a\n${blank}\nb`), 'a\n\nb');
    assert.equal(normalizeChatText(`${blank}x${blank}`), `${blank}x`);
    assert.equal(normalizeChatText(`${'\t'.repeat(200_000)}> quoted`), 'quoted');
  });
  assert.ok(ms < LIMIT_MS, `${Math.round(ms)} ms`);
});

test('normalizeChatText: code fence の中に改行が 20 万個続いても、すぐ終わる', () => {
  const body = `\`\`\`\n${'\n'.repeat(200_000)}x\n\`\`\``;
  let result = '';
  const ms = elapsed(() => { result = normalizeChatText(`\n\n${body}\n\n`); });
  assert.equal(result, body);
  assert.ok(ms < LIMIT_MS, `${Math.round(ms)} ms`);
});
