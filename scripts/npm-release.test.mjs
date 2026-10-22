import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, readdirSync, copyFileSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { publishPackages, expectedFiles, npmCommand, releaseVersion } from './publish-npm.mjs';
import { TARGETS, stagePayloads } from './stage-npm-binaries.mjs';

const repo = join(dirname(fileURLToPath(import.meta.url)), '..');
const python = process.env.MANY_AI_PYTHON || (process.platform === 'win32' ? 'python' : 'python3');
const packages = [...Object.values(TARGETS).map(t => t.pkg), 'many-ai-cli'];
function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'many-ai-cli-npm-test-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, 'scripts'));
  for (const name of ['stage-npm-binaries.mjs', 'stage-npm-from-release.mjs']) {
    copyFileSync(name === 'stage-npm-from-release.mjs' && process.env.MANY_AI_NPM_BASELINE_FROM_RELEASE
      ? process.env.MANY_AI_NPM_BASELINE_FROM_RELEASE : join(repo, 'scripts', name), join(root, 'scripts', name));
  }
  for (const pkg of packages) {
    const dir = join(root, 'npm', pkg);
    for (const file of expectedFiles(pkg)) {
      mkdirSync(dirname(join(dir, file)), { recursive: true });
      writeFileSync(join(dir, file), `old synthetic ${pkg}/${file}`);
    }
    const files = JSON.parse(readFileSync(join(repo, 'npm', pkg, 'package.json'))).files;
    writeFileSync(join(dir, 'package.json'), JSON.stringify({ name: pkg, version: '1.0.0', files,
      ...(pkg === 'many-ai-cli' ? { optionalDependencies: Object.fromEntries(packages.slice(0, -1).map(p => [p, '1.0.0'])) } : {}) }));
  }
  return root;
}
function snapshot(dir, prefix = '') {
  return Object.fromEntries(readdirSync(dir, { withFileTypes: true }).flatMap(item => {
    const path = join(dir, item.name), name = prefix + item.name;
    return item.isDirectory() ? Object.entries(snapshot(path, name + '/')) : [[name, readFileSync(path).toString('base64')]];
  }));
}
function archives(root, mutate = () => {}) {
  const dir = join(root, 'zips'); mkdirSync(dir);
  const definitions = Object.values(TARGETS).map(({ pkg, binName }, index) => ({ suffix: pkg.slice('many-ai-cli-'.length), version: '1.0.0', members: [
    [binName, `new synthetic ${pkg}`], [binName.replace('many-ai-cli', 'many-ai-cli-launcher'), 'synthetic launcher'],
    ['LICENSE', 'synthetic license'], ['THIRD_PARTY_NOTICES.md', `synthetic notice ${pkg}`],
    ['packaging-inputs/BUILD-INPUTS.json', JSON.stringify({ release_ready: false, source_revision: 'a'.repeat(40),
      declared_target: ['x86_64-pc-windows-msvc','x86_64-unknown-linux-gnu','x86_64-apple-darwin','aarch64-apple-darwin'][index] })],
    ['packaging-inputs/CRATE-NOTICES.txt', `synthetic crate notice ${pkg}`],
    ['packaging-inputs/build-inputs.spdx.json', JSON.stringify({ spdxVersion: 'SPDX-2.3' })],
  ] }));
  mutate(definitions);
  const result = spawnSync(python, ['-c', String.raw`
import json,pathlib,sys,zipfile,hashlib
root=pathlib.Path(sys.argv[1]);sums=[]
for target in json.load(sys.stdin):
    path=root/('many-ai-cli-'+target['version']+'-'+target['suffix']+'.zip')
    binary=target['members'][0]
    launcher=target['members'][1]
    receipt={'schema_version':2,'source_sha':'a'*40,'build_commit':'a'*40,'review_head_sha':'a'*40,'build_kind':'release','release_tag':'v'+target['version'],'version':target['version'],'target_name':target['suffix'],'binaries':[{'name':p[0],'sha256':hashlib.sha256(p[1].encode()).hexdigest(),'bytes':len(p[1].encode())} for p in [binary,launcher]],'packaging_inputs':[{'name':p[0].split('/')[-1],'sha256':hashlib.sha256(p[1].encode()).hexdigest(),'bytes':len(p[1].encode())} for p in target['members'] if p[0].startswith('packaging-inputs/')]}
    receipt.update(target.get('receipt_patch',{}))
    if not target.get('omit_receipt'): target['members'].append(['BUILD-RECEIPT.json',json.dumps(receipt)])
    with zipfile.ZipFile(path,'w',compression=zipfile.ZIP_DEFLATED) as archive:
        for entry in target['members']:
            info=zipfile.ZipInfo(entry[0]);info.create_system=3;info.external_attr=(entry[2] if len(entry)>2 else 0o100644)<<16
            info.compress_type=zipfile.ZIP_DEFLATED
            archive.writestr(info,entry[1].encode())
    sums.append(hashlib.sha256(path.read_bytes()).hexdigest()+'  '+path.name+'\n')
(root/'SHA256SUMS.txt').write_text(''.join(sums),encoding='utf-8')
`, dir], { input: JSON.stringify(definitions), encoding: 'utf8', timeout: 10000 });
  assert.equal(result.status, 0, result.stderr);
  return dir;
}
function stage(root, dir) {
  const preloads = [];
  if (process.env.MANY_AI_NPM_BASELINE_FROM_RELEASE) {
    // The old script uses unzip. Substitute only that OS boundary with stdlib
    // extraction in this owned fixture; no tool install or product execution.
    const extractor = String.raw`import pathlib,sys,zipfile
root=pathlib.Path(sys.argv[2]).resolve()
with zipfile.ZipFile(sys.argv[1]) as archive:
 for info in archive.infolist():
  dest=(root/info.filename).resolve()
  assert dest.is_relative_to(root)
 archive.extractall(root)`;
    const preload = `import cp from 'node:child_process';import{syncBuiltinESMExports}from'node:module';const original=cp.spawnSync;cp.spawnSync=(cmd,args,options)=>cmd==='unzip'?original(${JSON.stringify(python)},['-c',${JSON.stringify(extractor)},args[2],args[4]],options):original(cmd,args,options);syncBuiltinESMExports();`;
    preloads.push('--import', 'data:text/javascript,' + encodeURIComponent(preload));
  }
  return spawnSync(process.execPath, [...preloads, join(root, 'scripts/stage-npm-from-release.mjs'), dir], {
    cwd: root, encoding: 'utf8', timeout: 15000,
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1', MANY_AI_PYTHON: python },
  });
}

test('four release ZIPs stage main-only binaries, exact target notices and root evidence', t => {
  const root = fixture(t), dir = archives(root), result = stage(root, dir);
  assert.equal(result.status, 0, result.stderr);
  for (const { pkg, binName } of Object.values(TARGETS)) {
    assert.equal(readFileSync(join(root, 'npm', pkg, 'bin', binName), 'utf8'), `new synthetic ${pkg}`);
    assert.deepEqual(readdirSync(join(root, 'npm', pkg, 'bin')), [binName]);
    assert.match(readFileSync(join(root, 'npm', pkg, 'build-inputs/README.txt'), 'utf8'), /not a complete distribution SBOM/);
    const suffix = pkg.slice('many-ai-cli-'.length);
    assert.equal(readFileSync(join(root, 'npm/many-ai-cli/build-inputs', suffix, 'CRATE-NOTICES.txt'), 'utf8'), `synthetic crate notice ${pkg}`);
  }
});
const invalid = {
  'last target missing': d => d.pop(),
  'unknown target ZIP': d => { d[3].suffix = 'unknown'; },
  'mixed versions': d => { d[3].version = '1.0.1'; },
  'empty main binary': d => { d[3].members[0][1] = ''; },
  'duplicate binary member': d => { d[3].members.push(d[3].members[0]); },
  'unknown member': d => { d[3].members.push(['surprise.txt', 'synthetic']); },
  'traversal member': d => { d[3].members.push(['../escape', 'synthetic']); },
  'absolute member': d => { d[3].members.push(['/escape', 'synthetic']); },
  'Windows drive member': d => { d[3].members.push(['C:/escape', 'synthetic']); },
  'backslash member': d => { d[3].members.push(['dir\\escape', 'synthetic']); },
  'symlink member': d => { d[3].members[0][2] = 0o120777; },
  'missing notices': d => { d[3].members.splice(3, 1); },
  'missing SPDX': d => { d[3].members.pop(); },
  'false distribution-ready report': d => { d[3].members[4][1] = '{"release_ready":true}'; },
  'missing Rust receipt': d => { d[3].omit_receipt = true; },
  'wrong Rust receipt target': d => { d[3].receipt_patch = { target_name: 'linux-x64' }; },
  'wrong Rust receipt binary hashes': d => { d[3].receipt_patch = { binaries: [{ name: 'many-ai-cli', bytes: 1, sha256: '0'.repeat(64) }, { name: 'many-ai-cli-launcher', bytes: 1, sha256: '0'.repeat(64) }] }; },
  'build-metadata version': d => { d[3].version = '1.0.0+build'; },
  'leading-zero core': d => { d[3].version = '01.0.0'; },
  'leading-zero prerelease': d => { d[3].version = '1.0.0-rc.01'; },
};
for (const [name, mutate] of Object.entries(invalid)) test(`preflight refuses ${name} and preserves all npm outputs`, t => {
  const root = fixture(t), before = snapshot(join(root, 'npm'));
  const result = stage(root, archives(root, mutate));
  assert.notEqual(result.status, 0);
  assert.deepEqual(snapshot(join(root, 'npm')), before);
  assert.match(result.stderr, /npm outputs were not changed/);
});
for (const duplicate of [false, true]) test(`preflight refuses ${duplicate ? 'duplicate checksum' : 'altered archive'} before output`, t => {
  const root = fixture(t), dir = archives(root), before = snapshot(join(root, 'npm'));
  const sums = join(dir, 'SHA256SUMS.txt');
  if (duplicate) writeFileSync(sums, readFileSync(sums, 'utf8') + readFileSync(sums, 'utf8').split('\n')[0] + '\n');
  else writeFileSync(join(dir, 'many-ai-cli-1.0.0-macos-apple-silicon.zip'), 'synthetic corruption');
  assert.notEqual(stage(root, dir).status, 0);
  assert.deepEqual(snapshot(join(root, 'npm')), before);
});
test('transaction preflight refuses invalid output destinations without changing earlier files', t => {
  const root = fixture(t), before = snapshot(join(root, 'npm'));
  assert.throws(() => stagePayloads([{ dst: join(root, 'npm/many-ai-cli/LICENSE'), data: Buffer.from('new') },
    { dst: join(root, 'npm/many-ai-cli/bin'), data: Buffer.from('new') }], join(root, 'npm')), /regular file/);
  assert.deepEqual(snapshot(join(root, 'npm')), before);
});

for (const linked of ['bin', 'build-inputs']) test(`transaction refuses ${linked} ancestor links before changing any payload`, t => {
  const root = fixture(t);
  const npmRoot = join(root, 'npm');
  const target = join(npmRoot, 'many-ai-cli-windows-x64', linked);
  const outside = join(root, 'outside-package'); mkdirSync(outside);
  const name = linked === 'bin' ? 'many-ai-cli.exe' : 'CRATE-NOTICES.txt';
  writeFileSync(join(outside, name), 'outside original');
  rmSync(target, { recursive: true, force: true });
  symlinkSync(outside, target, process.platform === 'win32' ? 'junction' : 'dir');
  const earlier = join(npmRoot, 'many-ai-cli/LICENSE');
  const before = readFileSync(earlier);
  assert.throws(() => stagePayloads([
    { dst: earlier, data: Buffer.from('earlier replacement') },
    { dst: join(target, name), data: Buffer.from('outside replacement') },
  ], npmRoot), /directory links/);
  assert.deepEqual(readFileSync(earlier), before);
  assert.equal(readFileSync(join(outside, name), 'utf8'), 'outside original');
  assert.deepEqual(readdirSync(outside), [name]);
});

test('transaction refuses an npm output root link while allowing a repository junction above npm', t => {
  const root = fixture(t), actual = join(root, 'actual-repository'); mkdirSync(actual);
  mkdirSync(join(actual, 'npm'));
  const repositoryLink = join(root, 'repository-link');
  symlinkSync(actual, repositoryLink, process.platform === 'win32' ? 'junction' : 'dir');
  const destination = join(repositoryLink, 'npm', 'package', 'file');
  stagePayloads([{ dst: destination, data: Buffer.from('valid repository junction') }], join(repositoryLink, 'npm'));
  assert.equal(readFileSync(join(actual, 'npm/package/file'), 'utf8'), 'valid repository junction');
  const outside = join(root, 'outside-root'); mkdirSync(outside);
  writeFileSync(join(outside, 'file'), 'outside root original');
  const outputLink = join(root, 'linked-npm');
  symlinkSync(outside, outputLink, process.platform === 'win32' ? 'junction' : 'dir');
  assert.throws(() => stagePayloads([{ dst: join(outputLink, 'file'), data: Buffer.from('replacement') }], outputLink), /directory links/);
  assert.equal(readFileSync(join(outside, 'file'), 'utf8'), 'outside root original');
});

test('real offline npm pack preserves the reviewed platform/root evidence allowlists without lifecycle execution', t => {
  const root = fixture(t), out = join(root, 'packed'); mkdirSync(out);
  for (const pkg of packages) {
    const result = npmCommand(['pack', join(root, 'npm', pkg), '--offline', '--ignore-scripts', '--cache', join(root, 'npm-cache'), '--json', '--pack-destination', out], root);
    assert.equal(result.status, 0, result.stderr);
    const [pack] = JSON.parse(result.stdout);
    assert.deepEqual(pack.files.map(f => f.path).sort(), expectedFiles(pkg));
    assert.equal(pack.integrity, 'sha512-' + createHash('sha512').update(readFileSync(join(out, pack.filename))).digest('base64'));
  }
});
test('release tag parser accepts SemVer prereleases and rejects registry-ambiguous versions', () => {
  assert.equal(releaseVersion('v1.0.0-rc.1'), '1.0.0-rc.1');
  assert.equal(releaseVersion('1.0.0-0'), '1.0.0-0');
  for (const tag of ['v01.0.0','v1.0.0+build','v1.0.0-01','v1.0.0-rc..1','v1.0.0-','vv1.0.0']) assert.throws(() => releaseVersion(tag));
});

function npmStub(root, { existing = 'absent', conflict = false, badPack = false, unavailable = false } = {}) {
  const calls = [], integrities = new Map(), visible = new Map();
  const run = args => {
    calls.push(args);
    if (args[0] === 'pack') {
      const pkg = JSON.parse(readFileSync(join(args[1], 'package.json'))).name;
      const filename = `${pkg}-1.0.0.tgz`, data = Buffer.from(`synthetic packed ${pkg}`);
      writeFileSync(join(args.at(-1), filename), data);
      const integrity = 'sha512-' + createHash('sha512').update(data).digest('base64');
      integrities.set(pkg, integrity);
      const files = expectedFiles(pkg).map(path => ({ path }));
      if (badPack) files.push({ path: 'surprise' });
      return { status: 0, stdout: JSON.stringify([{ filename, integrity, files }]) };
    }
    if (args[0] === 'view') {
      const pkg = args[1].split('@')[0];
      if (unavailable) return { status: 1, stderr: 'synthetic E503' };
      const integrity = visible.get(pkg) || (existing === 'same' ? integrities.get(pkg) : existing === 'different' ? 'sha512-c3ludGhldGlj' : null);
      return integrity ? { status: 0, stdout: JSON.stringify(integrity) } : { status: 1, stderr: 'synthetic E404' };
    }
    if (args[0] === 'publish') {
      const pkg = packages.find(p => args[1].endsWith(`/${p}-1.0.0.tgz`) || args[1].endsWith(`\\${p}-1.0.0.tgz`));
      assert.ok(pkg);
      visible.set(pkg, conflict === 'different' ? 'sha512-c3ludGhldGlj' : integrities.get(pkg));
      return conflict ? { status: 1, stderr: 'synthetic EPUBLISHCONFLICT' } : { status: 0, stdout: '' };
    }
    throw new Error('unexpected synthetic npm call');
  };
  return { run, calls };
}
test('publisher packs all packages and checks registry before platform-first publication', async t => {
  const root = fixture(t), stub = npmStub(root);
  await publishPackages('v1.0.0', { root, ...stub, sleep: async () => {} });
  assert.equal(stub.calls.filter(a => a[0] === 'pack').length, 5);
  assert.equal(stub.calls.slice(0, 10).filter(a => a[0] === 'view').length, 5);
  const published = stub.calls.filter(a => a[0] === 'publish');
  assert.equal(published.length, 5);
  assert.ok(published.at(-1)[1].endsWith('many-ai-cli-1.0.0.tgz'));
  assert.ok(published.every(a => a.includes('--ignore-scripts') && a.at(-1) === 'latest'));
});
test('publisher resumes only byte-identical existing tarballs without publishing', async t => {
  const root = fixture(t), stub = npmStub(root, { existing: 'same' });
  await publishPackages('v1.0.0', { root, ...stub });
  assert.equal(stub.calls.filter(a => a[0] === 'publish').length, 0);
});
for (const options of [{ existing: 'different' }, { unavailable: true }, { badPack: true }]) {
  test(`publisher refuses unverifiable/different content before publish: ${JSON.stringify(options)}`, async t => {
    const root = fixture(t), stub = npmStub(root, options);
    await assert.rejects(publishPackages('v1.0.0', { root, ...stub }));
    assert.equal(stub.calls.filter(a => a[0] === 'publish').length, 0);
  });
}
for (const conflict of [true, 'different']) test(`publish conflict requires exact registry integrity: ${conflict}`, async t => {
  const root = fixture(t), stub = npmStub(root, { conflict });
  const result = publishPackages('v1.0.0', { root, ...stub, sleep: async () => {} });
  if (conflict === true) await result;
  else await assert.rejects(result, /different or unverifiable/);
});
