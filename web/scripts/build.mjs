import { mkdir, readdir, rm, stat, copyFile, readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, context } from 'esbuild';

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const srcDir = path.join(rootDir, 'src');
const distDir = path.join(rootDir, 'dist');
const watch = process.argv.includes('--watch');

const staticEntries = [
  'index.html',
  'styles.css',
  'styles',
  'vendor',
  'icons',
  'i18n',
  'icon.svg',
  'manifest.webmanifest',
];

async function walk(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    const rel = path.relative(srcDir, fullPath);
    if (rel.split(path.sep).includes('vendor')) continue;
    if (entry.isDirectory()) {
      files.push(...await walk(fullPath));
      continue;
    }
    if (entry.isFile() && /\.(?:js|ts)$/.test(entry.name) && !entry.name.endsWith('.d.ts')) {
      files.push(fullPath);
    }
  }
  return files;
}

async function copyRecursive(from, to) {
  const info = await stat(from);
  if (info.isDirectory()) {
    await mkdir(to, { recursive: true });
    const entries = await readdir(from, { withFileTypes: true });
    for (const entry of entries) {
      await copyRecursive(path.join(from, entry.name), path.join(to, entry.name));
    }
    return;
  }
  await mkdir(path.dirname(to), { recursive: true });
  await copyFile(from, to);
}

async function copyStaticAssets() {
  for (const entry of staticEntries) {
    await copyRecursive(path.join(srcDir, entry), path.join(distDir, entry));
  }
}

async function cleanDist() {
  await rm(distDir, { recursive: true, force: true });
  await mkdir(distDir, { recursive: true });
}

// 観測コード（web/src/debug/ 配下の sink）は既定でビルド対象から外す。
// 有効化は必ずオプトイン（MAI_DEBUG=1 か --debug）。付け忘れたら成果物は
// クリーンな側へ倒れる。詳細は docs/local/plan_instrumentation-probe-lifecycle.md。
const debugBuild = process.env.MAI_DEBUG === '1' || process.argv.includes('--debug');

// probe.ts は製品コード側の記録点から import されるので常に出力する
// （本体は define の __MAI_DEBUG__=false で到達不能になる）。
const debugDir = path.join(srcDir, 'debug');
const debugKeepAlways = new Set([path.join(debugDir, 'probe.ts')]);

function isExcludedDebugEntry(file) {
  if (debugBuild) return false;
  if (!file.startsWith(debugDir + path.sep)) return false;
  return !debugKeepAlways.has(file);
}

const entryPoints = (await walk(srcDir)).filter((f) => !isExcludedDebugEntry(f));

const buildOptions = {
  entryPoints,
  outdir: distDir,
  outbase: srcDir,
  bundle: false,
  format: 'esm',
  target: 'es2022',
  sourcemap: true,
  logLevel: 'info',
  define: { __MAI_DEBUG__: String(debugBuild) },
};

// 既定ビルドでは debug/index.ts を出力しないため、app.js の副作用 import が
// 解決できるよう空の index.js を置く（bundle:false なので import 文は残る）。
async function writeEmptyDebugIndex() {
  if (debugBuild) return;
  const outDebugDir = path.join(distDir, 'debug');
  await mkdir(outDebugDir, { recursive: true });
  await writeFile(path.join(outDebugDir, 'index.js'), '', 'utf8');
}

await cleanDist();
await copyStaticAssets();

async function generateSrcHash() {
  const files = (await walk(srcDir)).sort((a, b) => a.localeCompare(b));
  const hasher = createHash('sha256');
  for (const f of files) {
    const rel = path.relative(srcDir, f).replaceAll(path.sep, '/');
    hasher.update(rel + '\n');
    hasher.update(await readFile(f));
    hasher.update('\n');
  }
  // 同じ src でも MAI_DEBUG の有無で dist の中身が変わるため hash に混ぜる。
  hasher.update('MAI_DEBUG=' + (debugBuild ? '1' : '0'));
  const hash = hasher.digest('hex').slice(0, 12);
  await writeFile(path.join(distDir, '.src-hash'), hash, 'utf8');
  return hash;
}

if (watch) {
  const ctx = await context(buildOptions);
  await ctx.watch();
  await writeEmptyDebugIndex();
  console.log(`watching web/src -> web/dist (instrumentation: ${debugBuild ? 'on' : 'off'})`);
} else {
  await build(buildOptions);
  await writeEmptyDebugIndex();
  const hash = await generateSrcHash();
  console.log(`web/src hash: ${hash} (instrumentation: ${debugBuild ? 'on' : 'off'})`);
}
