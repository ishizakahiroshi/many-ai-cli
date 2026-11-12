import { describe, expect, test } from 'bun:test';
import { readFileSync, readdirSync } from 'node:fs';
import { fixtureFiles } from '../scripts/fixture-files.mjs';

// 監査 N37: dist の fixture を回す一覧は、手で並べず src から作る。
describe('fixture を回す一覧', () => {
  test('web/src/app の fixture が全部入る', async () => {
    const files = (await fixtureFiles()).map((file) => file.replaceAll('\\', '/'));
    const sources = readdirSync(new URL('../src/app/', import.meta.url))
      .filter((name) => name.endsWith('-fixtures.ts'));
    expect(files.length).toBe(sources.length);
    expect(files.length).toBeGreaterThanOrEqual(37);
    for (const file of files) expect(file).toMatch(/^dist\/app\/[a-z0-9-]+-fixtures\.js$/);
    // 一覧から落ちていた 2 本と、入れ忘れていた 3 本。
    for (const name of [
      'chat-path-links-fixtures', 'provider-manager-view-fixtures',
      'chat-history-dom-fixtures', 'transcript-message-fixtures', 'usage-limit-fixtures', 'chat-normalize-fixtures',
    ]) {
      expect(files).toContain(`dist/app/${name}.js`);
    }
  });

  test('package.json の test は、path を手で並べずにこの一覧を使う', () => {
    const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
    expect(pkg.scripts.test).toBe('bun run build && node scripts/run-fixtures.mjs');
  });
});
