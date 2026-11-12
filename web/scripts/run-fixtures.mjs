// dist の fixture（web/src/app/*-fixtures.ts を組み立てたもの）を node --test で全部回す。
//
// 以前は package.json の test に dist の path を手で並べていた。2 つの path の間の空白が
// 落ちて 1 語になり、その 2 本が回らないまま CI が成功していた（node --test は無い path を
// 黙って飛ばす。監査 N37）。一覧に入れ忘れた fixture も 3 本あった。
// ここでは一覧を src から作り（fixture-files.mjs）、組み立てた file が 1 本でも無ければ、
// 回す前に失敗させる。
//
// この file は読み込まれたら必ず回す（「直接起動されたときだけ回す」の判定を置かない。
// リンク越しの folder から起動すると判定が外れ、何も回さずに成功で終わるため）。
import { access } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fixtureFiles, rootDir } from './fixture-files.mjs';

const files = await fixtureFiles();
if (files.length === 0) {
  console.error('fixture が 1 本も見つからない（web/src/app/*-fixtures.ts）');
  process.exit(1);
}
const missing = [];
for (const file of files) {
  try {
    await access(path.join(rootDir, file));
  } catch {
    missing.push(file);
  }
}
if (missing.length > 0) {
  console.error(`組み立てた fixture が無い（先に build する）:\n  ${missing.join('\n  ')}`);
  process.exit(1);
}
console.log(`fixture ${files.length} 本を回す`);
const result = spawnSync(process.execPath, ['--test', ...files], { cwd: rootDir, stdio: 'inherit' });
process.exit(result.status ?? 1);
