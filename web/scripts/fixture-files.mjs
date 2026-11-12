// node --test で回す fixture の一覧。web/src/app/*-fixtures.ts から作る（手で並べない。監査 N37）。
import { readdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

// 回す file（web からの相対 path）。名前の順。
export async function fixtureFiles() {
  const names = await readdir(path.join(rootDir, 'src', 'app'));
  return names
    .filter((name) => /-fixtures\.(?:ts|js)$/.test(name) && !name.endsWith('.d.ts'))
    .sort()
    .map((name) => path.join('dist', 'app', name.replace(/\.ts$/, '.js')));
}
