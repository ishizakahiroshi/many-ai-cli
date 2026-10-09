import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { execFile } from 'node:child_process';
import ts from 'typescript';

// Run the actual registered provider without booting the browser application.
// AST extraction keeps this test coupled to production logic, not a copy of it.
const source = ts.createSourceFile('terminal.ts', readFileSync(new URL('../src/app/terminal.ts', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true);
let provider = '';
let crunch = '';
function visit(node: ts.Node) {
  if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression)
      && node.expression.name.text === 'registerLinkProvider') provider = node.arguments[0].getText(source);
  if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'CRUNCH_LINK_RE') crunch = node.initializer!.getText(source);
  ts.forEachChild(node, visit);
}
visit(source);
if (!provider || !crunch) throw new Error('Terminal link provider extraction failed');
const providerCode = ts.transpile(`const CRUNCH_LINK_RE = ${crunch}; return ${provider};`, { target: ts.ScriptTarget.ES2022 });

async function detect(input: string, cols = 120, y = 1, activate = false, emptyFirstRows = 0): Promise<any> {
  // Isolate native xterm/regexp work in a process: it stays killable on timeout,
  // and its teardown cannot race the test runner's Bun Worker lifecycle.
  // Pass the large synthetic buffer over stdin, not the Windows command line.
  const script = `
    (async () => {
      let data = '';
      for await (const chunk of process.stdin) data += chunk;
      const d = JSON.parse(data);
      const [path, url, headless] = await Promise.all(d.modules.map(m => import(m)));
      const term = new headless.Terminal({ cols: d.cols, rows: 24, scrollback: 10000, allowProposedApi: true });
      await new Promise(resolve => term.write(d.input, resolve));
      // Keep real headless cells/wrap flags, but explicitly synthesize empty
      // translated rows to exercise duplicate offsets (written blanks survive).
      if (d.emptyFirstRows) {
        const buffer = term.buffer.active;
        const getLine = buffer.getLine.bind(buffer);
        buffer.getLine = index => {
          const line = getLine(index);
          if (line && index < d.emptyFirstRows) line.translateToString = () => '';
          return line;
        };
      }
      let callbacks = 0, finished = 0, links = [], actions = [];
      const scope = { ...path, ...url, term, id: 7,
        // Count only the outer span; links.* stage spans nest inside it.
        probeSpan: (_channel, fields) => fields().phase === 'terminal.links' ? () => { finished++; } : () => {},
        resolveTerminalPathCandidate: path.trimTerminalPathCandidate,
        scheduleHidePathPopup() {},
        showPathPopup: value => actions.push(['path', value]),
        showUrlPopup: value => actions.push(['url', value]),
        handleCrunchLinkClick: id => actions.push(['crunch', id]),
      };
      const provider = new Function(...Object.keys(scope), d.code)(...Object.values(scope));
      provider.provideLinks(d.y, found => { callbacks++; links = found; });
      if (d.activate) for (const link of links) link.activate({ preventDefault() {}, stopPropagation() {}, clientX: 1, clientY: 1 });
      const result = { callbacks, finished, count: links.length,
        links: d.input.length < 1000 ? links.map(({ text, range }) => ({ text, range })) : [], actions,
        first: links[0] && { text: links[0].text, range: links[0].range },
        last: links.length && { text: links[links.length - 1].text, range: links[links.length - 1].range },
        rowLengths: [0, 1, 2].map(index => term.buffer.active.getLine(index)?.translateToString(true).length),
      };
      term.dispose();
      console.log(JSON.stringify(result));
    })().catch(error => { console.error(error); process.exitCode = 1; });
  `;
  const data = { input, cols, y, activate, emptyFirstRows, code: providerCode, modules: [
    new URL('../src/app/path-detect.ts', import.meta.url).href,
    new URL('../src/app/url-detect.ts', import.meta.url).href,
    new URL('../node_modules/@xterm/headless/lib-headless/xterm-headless.js', import.meta.url).href,
  ] };
  return await new Promise((resolve, reject) => {
    const child = execFile(process.execPath, ['--eval', script], {
      timeout: 5000, killSignal: 'SIGKILL', windowsHide: true, encoding: 'utf8',
    }, (error, stdout) => {
      if (error) { reject(error); return; }
      try { resolve(JSON.parse(stdout)); } catch (error) { reject(error); }
    });
    child.stdin!.once('error', reject);
    child.stdin!.end(JSON.stringify(data));
  });
}

test('provider preserves URL/path/crunch priority, ranges and activation', async () => {
  const input = 'https://example.invalid/a "C:/file.ts" ./src/a.ts (ctrl+o to expand)';
  const result = await detect(input, 120, 1, true);
  const texts = ['https://example.invalid/a', 'C:/file.ts', './src/a.ts', '(ctrl+o to expand)'];
  expect(result.links).toEqual(texts.map(text => ({ text, range: {
    start: { x: input.indexOf(text) + 1, y: 1 }, end: { x: input.indexOf(text) + text.length, y: 1 },
  } })));
  expect(result.actions).toEqual([['url', texts[0]], ['path', texts[1]], ['path', texts[2]], ['crunch', 7]]);
  expect([result.callbacks, result.finished]).toEqual([1, 1]);
});

test('provider maps a soft-wrapped path and an indented hard-wrapped path', async () => {
  const soft = await detect('x'.repeat(75) + ' ./src/a.ts end', 80, 2);
  expect(soft.links).toEqual([{ text: './src/a.ts', range: { start: { x: 77, y: 1 }, end: { x: 6, y: 2 } } }]);
  const hard = await detect('./src/\r\n  a.ts', 80, 2);
  expect(hard.links).toEqual([{ text: './src/a.ts', range: { start: { x: 1, y: 1 }, end: { x: 6, y: 2 } } }]);
});

test('provider does not extend a path onto a wrap-marked row after a row that ended early', async () => {
  // 2026-10-09: "資料: <path>.html" ended mid-row, yet the next "S2 [AI][依頼] (次回)" row
  // carried xterm's wrap mark and the link grew to ".html  S2 [AI][依頼". Padding the row
  // with written blanks before the next text produces the same mark.
  const head = '  資料: D:\\dev\\kobo\\docs\\local\\design_2026-10-09.html';
  const cells = [...head].reduce((n, ch) => n + (/[\u3000-\u9fff\uff00-\uffef]/.test(ch) ? 2 : 1), 0);
  const input = head + ' '.repeat(80 - cells) + '  S2 [AI][依頼] (次回) まとめる';
  const path = 'D:\\dev\\kobo\\docs\\local\\design_2026-10-09.html';
  const first = await detect(input, 80, 1);
  expect(first.links).toEqual([{ text: path, range: { start: { x: cells - path.length + 1, y: 1 }, end: { x: cells, y: 1 } } }]);
  const second = await detect(input, 80, 2);
  expect(second.links).toEqual([]);
});

test('provider preserves full-width character cell coordinates', async () => {
  const result = await detect('日本語 ./src/a.ts');
  expect(result.links).toEqual([{ text: './src/a.ts', range: { start: { x: 8, y: 1 }, end: { x: 17, y: 1 } } }]);
});

test('provider skips empty soft-wrapped rows with equal string offsets', async () => {
  const written = await detect(' '.repeat(160) + './src/a.ts', 80, 3);
  expect(written.rowLengths).toEqual([80, 80, 10]);
  const result = await detect(' '.repeat(160) + './src/a.ts', 80, 3, false, 2);
  expect(result.rowLengths).toEqual([0, 0, 10]);
  expect(result.links).toEqual([{ text: './src/a.ts', range: { start: { x: 1, y: 3 }, end: { x: 10, y: 3 } } }]);
});

for (const piece of ['./src/a.ts ', 'C:/file.ts ']) {
  test(`provider completes a long logical row of ${piece.trim()} links`, async () => {
    const small = await detect(piece.repeat(120), 80, 1);
    expect(small.count).toBe(120);
    expect(small.last).toEqual({ text: piece.trim(), range: { start: { x: 30, y: 17 }, end: { x: 39, y: 17 } } });
    const result = await detect(piece.repeat(72000), 80, 1);
    expect([result.callbacks, result.finished]).toEqual([1, 1]);
    expect(result.count).toBe(72000);
    expect(result.first).toEqual({ text: piece.trim(), range: { start: { x: 1, y: 1 }, end: { x: 10, y: 1 } } });
    expect(result.last).toEqual({ text: piece.trim(), range: { start: { x: 70, y: 9900 }, end: { x: 79, y: 9900 } } });
  }, 10000);
}
