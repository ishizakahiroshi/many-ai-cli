// vtype-core（音声入力エンジン）を、隣に置いた vtype リポジトリから src/vendor/vtype-core/ へ写す。
//
// web のビルドは bundle:false で import 文をそのまま残すため、`import 'vtype-core'` のような
// 裸の指定子はブラウザで解決できない（Hub の CSP は script-src 'self' なのでインラインの
// import map も使えない）。xterm などと同じく src/vendor/ に置いてコミットし、相対パスで import する。
// 写したファイルは web/dist を経てバイナリへ埋め込まれるので、GitHub Release・winget・Homebrew・
// npm・Docker のどの配布経路にも同じものが載る。package.json には依存として書かない
// （書くと CI の `bun install --frozen-lockfile` が vtype の場所を解決できずに落ちる）。
//
// 正本は vtype リポジトリの packages/core。vendor 側を手で直さず、vtype 側で
// `pnpm -F vtype-core build` してからこのスクリプトを流す。
//
//   node scripts/sync-vtype-core.mjs          写す
//   node scripts/sync-vtype-core.mjs --check  差分があれば exit 1（vtype が手元にあるときだけ使える）
//
// vtype の場所は既定で many-ai-cli と同じ親フォルダの vtype/packages/core。
// 別の場所なら環境変数 VTYPE_CORE_DIR で指定する。
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const webDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const coreDir = process.env.VTYPE_CORE_DIR
  ? path.resolve(process.env.VTYPE_CORE_DIR)
  : path.resolve(webDir, '..', '..', 'vtype', 'packages', 'core');
const fromDir = path.join(coreDir, 'dist');
const toDir = path.join(webDir, 'src', 'vendor', 'vtype-core');
const check = process.argv.includes('--check');

try {
  await stat(fromDir);
} catch {
  console.error(`vtype-core の dist が見つからない: ${fromDir}`);
  console.error('vtype で `pnpm -F vtype-core build` を実行するか、VTYPE_CORE_DIR で場所を指定する');
  process.exit(2);
}

const distFiles = (await readdir(fromDir)).filter((f) => /\.(?:js|d\.ts)$/.test(f)).sort();
const pairs = [
  ...distFiles.map((f) => [path.join(fromDir, f), path.join(toDir, f)]),
  [path.join(coreDir, 'LICENSE'), path.join(toDir, 'LICENSE')],
];

// .map は写さないので、.js 末尾の sourceMappingURL 行も外す（開発者ツールの 404 を避ける）。
async function vendoredContent(from) {
  const buf = await readFile(from);
  if (!from.endsWith('.js')) return buf;
  return Buffer.from(buf.toString('utf8').replace(/\n\/\/# sourceMappingURL=\S+\s*$/, '\n'), 'utf8');
}

let drift = 0;
for (const [from, to] of pairs) {
  const want = await vendoredContent(from);
  if (check) {
    const have = await readFile(to).catch(() => null);
    if (!have || !want.equals(have)) {
      console.error(`vtype-core vendor drift: ${path.relative(webDir, to)}`);
      drift++;
    }
    continue;
  }
  await mkdir(toDir, { recursive: true });
  await writeFile(to, want);
}

if (check) {
  if (drift > 0) {
    console.error('run: node scripts/sync-vtype-core.mjs');
    process.exit(1);
  }
  console.log(`vtype-core vendor is in sync (${pairs.length} files)`);
} else {
  console.log(`vtype-core -> src/vendor/vtype-core (${pairs.length} files, from ${coreDir})`);
}
