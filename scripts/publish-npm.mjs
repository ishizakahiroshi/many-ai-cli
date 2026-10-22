#!/usr/bin/env node
// Local packs are prepared with lifecycle scripts disabled. A published version
// is resumable only when its registry integrity matches these exact tar bytes.
import { createHash } from 'node:crypto';
import { readFileSync, mkdtempSync, rmSync, statSync, realpathSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';
import { TARGETS } from './stage-npm-binaries.mjs';

export const EVIDENCE = ['BUILD-INPUTS.json', 'CRATE-NOTICES.txt', 'build-inputs.spdx.json'];
export function releaseVersion(tag) {
  const version = tag?.replace(/^v/, '');
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/.exec(version || '');
  if (!match || match[4]?.split('.').some(part => /^\d+$/.test(part) && part.length > 1 && part.startsWith('0'))) {
    throw new Error('invalid release tag; SemVer without build metadata is required');
  }
  return version;
}
export function expectedFiles(pkg) {
  const root = pkg === 'many-ai-cli';
  const target = Object.values(TARGETS).find(t => t.pkg === pkg);
  if (!root && !target) throw new Error('unknown npm platform package');
  return ['package.json', 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'build-inputs/README.txt',
    ...(root ? ['README.md', 'bin/many-ai-cli.mjs', ...Object.values(TARGETS).flatMap(t =>
      [...EVIDENCE, 'THIRD_PARTY_NOTICES.md'].map(n => `build-inputs/${t.pkg.slice('many-ai-cli-'.length)}/${n}`))]
      : [`bin/${target.binName}`, ...EVIDENCE.map(n => `build-inputs/${n}`)])].sort();
}

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
export function npmCommand(args, cwd) {
  // Avoid cmd.exe argument interpretation for paths containing shell syntax.
  const cli = join(dirname(process.execPath), 'node_modules', 'npm', 'bin', 'npm-cli.js');
  if (process.platform === 'win32' && !existsSync(cli)) throw new Error('npm CLI is required beside the Node installation');
  return spawnSync(process.platform === 'win32' ? process.execPath : 'npm',
    process.platform === 'win32' ? [cli, ...args] : args,
    { cwd, encoding: 'utf8', timeout: 60000, maxBuffer: 4 * 1024 * 1024 });
}
function success(result) { return !result.error && result.status === 0; }
function output(result) { return `${result.stdout || ''}\n${result.stderr || ''}`; }
function json(result, label) {
  if (!success(result)) throw new Error(`${label} failed (exit ${result.status ?? 'unavailable'})`);
  try { return JSON.parse(result.stdout); } catch { throw new Error(`${label} returned invalid JSON`); }
}
function registry(run, cwd, pkg, version) {
  const result = run(['view', `${pkg}@${version}`, 'dist.integrity', '--json'], cwd);
  if (!success(result)) {
    if (/\bE404\b/.test(output(result))) return null;
    throw new Error(`Cannot verify existing registry content for ${pkg}`);
  }
  const integrity = json(result, 'registry integrity query');
  if (typeof integrity !== 'string' || !/^sha512-[A-Za-z0-9+/]+={0,2}$/.test(integrity)) {
    throw new Error(`Missing or unsupported registry integrity for ${pkg}`);
  }
  return integrity;
}

export async function publishPackages(tag, { root = repoRoot, run = npmCommand, sleep = delay } = {}) {
  const version = releaseVersion(tag);
  const packages = [...Object.values(TARGETS).map(t => t.pkg), 'many-ai-cli'];
  const packed = [];
  const ownedTemp = mkdtempSync(join(tmpdir(), 'many-ai-cli-npm-publish-'));
  try {
    for (const pkg of packages) {
      const dir = join(root, 'npm', pkg);
      const metadata = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'));
      if (metadata.name !== pkg || metadata.version !== version) throw new Error(`package/tag identity mismatch: ${pkg}`);
      if (pkg === 'many-ai-cli' && packages.slice(0, -1).some(p => metadata.optionalDependencies?.[p] !== version)) {
        throw new Error('root optionalDependencies must match the release tag');
      }
      for (const file of expectedFiles(pkg)) {
        const path = join(dir, file);
        if (!statSync(path).isFile() || statSync(path).size === 0) throw new Error(`missing/empty package payload: ${pkg}/${file}`);
      }
      const result = json(run(['pack', dir, '--ignore-scripts', '--offline', '--json', '--pack-destination', ownedTemp], root), 'local npm pack');
      if (!Array.isArray(result) || result.length !== 1) throw new Error('expected exactly one local npm pack');
      const pack = result[0];
      const files = pack.files?.map(f => f.path).sort();
      if (JSON.stringify(files) !== JSON.stringify(expectedFiles(pkg))) throw new Error(`unexpected packed payload: ${pkg}`);
      if (typeof pack.filename !== 'string' || !/^[A-Za-z0-9_.-]+\.tgz$/.test(pack.filename)) throw new Error('unsafe npm pack filename');
      const tar = join(ownedTemp, pack.filename);
      const integrity = 'sha512-' + createHash('sha512').update(readFileSync(tar)).digest('base64');
      if (pack.integrity !== integrity) throw new Error(`local npm pack integrity mismatch: ${pkg}`);
      packed.push({ pkg, tar, integrity });
    }
    // Check every existing version before publishing any new platform package.
    for (const item of packed) {
      item.existing = registry(run, root, item.pkg, version);
      if (item.existing && item.existing !== item.integrity) throw new Error(`Published content differs: ${item.pkg}@${version}`);
    }
    const distTag = version.includes('-') ? 'next' : 'latest';
    for (const item of packed) {
      if (item.existing) { console.log(`SKIP ${item.pkg}: exact tarball integrity verified`); continue; }
      for (let attempt = 0; ; attempt++) {
        const result = run(['publish', item.tar, '--ignore-scripts', '--access', 'public', '--provenance', '--tag', distTag], root);
        if (success(result)) break;
        if (/cannot publish over|previously published|EPUBLISHCONFLICT|409 Conflict/i.test(output(result))) {
          if (registry(run, root, item.pkg, version) === item.integrity) break;
          throw new Error(`Publish conflict has different or unverifiable content: ${item.pkg}`);
        }
        if (/rate.?limit|429|too many requests/i.test(output(result)) && attempt < 5) {
          await sleep(30000 * 2 ** attempt);
          continue;
        }
        throw new Error(`npm publish failed: ${item.pkg}`);
      }
      let visible = null;
      for (let attempt = 0; attempt < 6; attempt++) {
        visible = registry(run, root, item.pkg, version);
        if (visible !== null) break;
        await sleep(500 * 2 ** attempt);
      }
      if (visible !== item.integrity) throw new Error(`Published tarball integrity verification failed: ${item.pkg}`);
      console.log(`OK ${item.pkg}: published tarball integrity verified`);
    }
  } finally { rmSync(ownedTemp, { recursive: true, force: true }); }
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  publishPackages(process.argv[2]).catch(error => {
    console.error(`npm publication stopped: ${error.message}`);
    process.exitCode = 1;
  });
}
