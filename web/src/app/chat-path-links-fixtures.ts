import assert from 'node:assert/strict';
import test from 'node:test';
import { Worker } from 'node:worker_threads';
import { splitChatPaths } from './chat-path-links.js';

function paths(text: string): string[] {
  return splitChatPaths(text)
    .filter(p => p.path !== undefined)
    .map(p => p.path as string);
}

test('chat paths: the usual forms still become links', () => {
  assert.deepEqual(paths('see foo/bar.ts and ./a/b.txt'), ['foo/bar.ts', './a/b.txt']);
  assert.deepEqual(paths('open D:\\work\\notes.md, then /usr/local/bin.'), ['D:\\work\\notes.md', '/usr/local/bin']);
  assert.deepEqual(paths('(README.md)'), ['README.md']);
  assert.deepEqual(paths('plain words only'), []);
  const pieces = splitChatPaths('edit src/app.ts; done');
  assert.equal(pieces.map(p => p.text).join(''), 'edit src/app.ts; done');
});

test('chat paths: a relative path body is read up to 255 characters', () => {
  const name = 'a'.repeat(255) + '.ts';
  assert.deepEqual(paths(name), [name]);
  // A longer one is not cut into a link that names a different file.
  const long = 'b'.repeat(300) + '.ts';
  assert.deepEqual(paths(long), []);
  assert.equal(splitChatPaths(long).map(p => p.text).join(''), long);
  assert.deepEqual(paths('./' + long), ['./' + long]);
});

test('chat paths: a long line without spaces or extension completes without blocking the caller (U30)', { timeout: 15000 }, async () => {
  // The worker stays terminable if the pattern regresses to quadratic time.
  const moduleUrl = new URL(import.meta.url.endsWith('.ts') ? './chat-path-links.ts' : './chat-path-links.js', import.meta.url).href;
  const worker = new Worker(`
    const { parentPort, workerData } = require('node:worker_threads');
    (async () => {
      const m = await import(workerData);
      const hex = '0123456789abcdef'.repeat(16384);
      for (const text of [hex, 'a'.repeat(262144)]) {
        const pieces = m.splitChatPaths(text);
        if (pieces.length !== 1 || pieces[0].path !== undefined || pieces[0].text !== text) {
          parentPort.postMessage(false);
          return;
        }
      }
      parentPort.postMessage(true);
    })().catch(error => { throw error; });
  `, { eval: true, workerData: moduleUrl });
  try {
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('chat path detection exceeded 5 seconds')), 5000);
      worker.once('message', result => {
        clearTimeout(timer);
        if (result === true) resolve();
        else reject(new Error('a long line without a path was split or linked'));
      });
      worker.once('error', error => { clearTimeout(timer); reject(error); });
      worker.once('exit', () => { clearTimeout(timer); reject(new Error('chat path worker exited without a result')); });
    });
  } finally {
    await worker.terminate();
  }
});
