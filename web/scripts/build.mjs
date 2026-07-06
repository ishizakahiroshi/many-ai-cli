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

const entryPoints = await walk(srcDir);

const buildOptions = {
  entryPoints,
  outdir: distDir,
  outbase: srcDir,
  bundle: false,
  format: 'esm',
  target: 'es2022',
  sourcemap: true,
  logLevel: 'info',
};

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
  const hash = hasher.digest('hex').slice(0, 12);
  await writeFile(path.join(distDir, '.src-hash'), hash, 'utf8');
  return hash;
}

if (watch) {
  const ctx = await context(buildOptions);
  await ctx.watch();
  console.log('watching web/src -> web/dist');
} else {
  await build(buildOptions);
  const hash = await generateSrcHash();
  console.log(`web/src hash: ${hash}`);
}
