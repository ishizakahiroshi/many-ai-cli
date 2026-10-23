#!/usr/bin/env node
// The workflow verifies checksum signatures. Python 3.11+ stdlib zipfile checks
// every member of all four ZIPs before any npm payload is changed. It never
// extracts archive paths; only reviewed payload bytes enter a fresh OS temp dir.
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { stagePayloads, TARGETS } from './stage-npm-binaries.mjs';
const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const EVIDENCE = ['BUILD-INPUTS.json', 'CRATE-NOTICES.txt', 'build-inputs.spdx.json'];
const WEB_NOTICE = 'WEB-THIRD-PARTY-LICENSES.txt';
const SCOPE = Buffer.from('Cargo build-input evidence only. This is not a complete distribution SBOM or release/license acceptance.\n');
const PREFLIGHT = String.raw`
import hashlib,json,pathlib,re,stat,sys,zipfile
source,temp=map(pathlib.Path,sys.argv[1:])
targets=['windows-x64','linux-x64','macos-intel','macos-apple-silicon']
triples=['x86_64-pc-windows-msvc','x86_64-unknown-linux-gnu','x86_64-apple-darwin','aarch64-apple-darwin']
evidence=['BUILD-INPUTS.json','CRATE-NOTICES.txt','build-inputs.spdx.json']
web_notice='web/src/vendor/THIRD_PARTY_LICENSES.txt'
def require(ok,message):
    if not ok: raise ValueError(message)
def digest(data): return hashlib.sha256(data).hexdigest()
sums={}
for line in (source/'SHA256SUMS.txt').read_text(encoding='utf-8').splitlines():
    if not line.strip(): continue
    match=re.fullmatch(r'([0-9a-fA-F]{64})  \*?([^/\\:]+)',line)
    require(match is not None,'malformed checksum entry')
    name=match[2]
    require(name not in sums,'duplicate checksum entry')
    sums[name]=match[1].lower()
archives=list(source.glob('*.zip'))
require(len(archives)==4,'exactly four supported target ZIPs required')
selected=[];versions=set();sources=set();web_notices=set()
for suffix in targets:
    matches=[p for p in archives if p.name.endswith('-'+suffix+'.zip')]
    require(len(matches)==1,'missing or duplicate target ZIP: '+suffix)
    path=matches[0]
    version=path.name.removeprefix('many-ai-cli-').removesuffix('-'+suffix+'.zip')
    semver=re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?',version)
    require(path.name.startswith('many-ai-cli-') and semver is not None,'invalid release archive version; build metadata is unsupported')
    require(not semver[4] or all(not (p.isdigit() and len(p)>1 and p.startswith('0')) for p in semver[4].split('.')),'invalid numeric prerelease identifier')
    versions.add(version)
    require(path.is_file() and not path.is_symlink(),'archive must be regular')
    require(path.stat().st_size<=1024*1024*1024,'archive too large')
    require(path.name in sums and digest(path.read_bytes())==sums[path.name],'archive checksum mismatch: '+suffix)
    extension='.exe' if suffix=='windows-x64' else ''
    binary='many-ai-cli'+extension
    required={binary,'many-ai-cli-launcher'+extension,'LICENSE','THIRD_PARTY_NOTICES.md','BUILD-RECEIPT.json',web_notice,*['packaging-inputs/'+n for n in evidence]}
    allowed=required|{'README.md','README.ja.md','README.vi.md','CHANGELOG.md','GO-LICENSE','THIRD-PARTY-NOTICES.md','BUILD-RECEIPT.json','NOT-FOR-CUTOVER.txt','RELEASE-EVIDENCE.txt','PACKAGE-RECEIPT.json','web/src/vendor/THIRD_PARTY_LICENSES.txt'}
    if extension: allowed.add('unblock-windows.cmd')
    directories={p[:i+1] for p in allowed for i,c in enumerate(p) if c=='/'}
    members={};seen=set();total=0
    with zipfile.ZipFile(path) as archive:
        for info in archive.infolist():
            name=info.filename
            require(info.orig_filename==name and name not in seen,'duplicate or malformed ZIP member')
            seen.add(name)
            require(name in allowed or name in directories,'unknown or unsafe ZIP member: '+name)
            kind=stat.S_IFMT(info.external_attr>>16)
            require(kind in (0,stat.S_IFREG,stat.S_IFDIR),'ZIP links/special members forbidden')
            require(not (info.flag_bits&1),'encrypted ZIP member forbidden')
            require(info.file_size<=512*1024*1024,'ZIP member too large')
            total+=info.file_size
            require(total<=1024*1024*1024,'expanded ZIP too large')
            if name in directories:
                require(info.is_dir() and info.file_size==0,'invalid ZIP directory')
                continue
            require(not info.is_dir() and kind!=stat.S_IFDIR,'expected ZIP regular file')
            data=archive.read(info) # CRC is checked, including unused members
            require(len(data)>0,'empty ZIP member: '+name)
            members[name]=data
    require(required<=members.keys(),'Rust release receipt/notices/build-input evidence required; legacy Go archives cannot be republished as Rust packages: '+suffix)
    receipt=json.loads(members['BUILD-RECEIPT.json'])
    require(isinstance(receipt,dict) and receipt.get('build_kind')=='release' and receipt.get('release_tag')=='v'+version,'Rust release receipt identity is invalid')
    source_sha=receipt.get('source_sha')
    require(type(receipt.get('schema_version')) is int and receipt['schema_version']==2 and isinstance(source_sha,str) and re.fullmatch(r'[0-9a-f]{40}',source_sha),'Rust release source identity is invalid')
    require(receipt.get('build_commit')==source_sha and receipt.get('review_head_sha')==source_sha,'Rust release commit identity mismatch')
    sources.add(source_sha)
    require(receipt.get('target_name')==suffix and receipt.get('version')==version,'Rust release receipt target/version mismatch')
    binaries=receipt.get('binaries')
    require(isinstance(binaries,list) and len(binaries)==2 and all(isinstance(item,dict) for item in binaries),'Rust binary receipt is invalid')
    require({item.get('name') for item in binaries}=={binary,'many-ai-cli-launcher'+extension},'Rust binary receipt names mismatch')
    for item in binaries:
        data=members[item['name']]
        require(item.get('sha256')==digest(data) and type(item.get('bytes')) is int and item['bytes']==len(data),'Rust binary receipt bytes mismatch')
    entries=receipt.get('packaging_inputs')
    require(isinstance(entries,list) and len(entries)==3 and all(isinstance(item,dict) and isinstance(item.get('name'),str) for item in entries),'Rust input-evidence receipt is invalid')
    require({item['name'] for item in entries}==set(evidence),'Rust input-evidence receipt names mismatch')
    for item in entries:
        data=members['packaging-inputs/'+item['name']]
        require(item.get('sha256')==digest(data) and type(item.get('bytes')) is int and item['bytes']==len(data),'Rust input-evidence receipt bytes mismatch')
    report=json.loads(members['packaging-inputs/BUILD-INPUTS.json'])
    spdx=json.loads(members['packaging-inputs/build-inputs.spdx.json'])
    require(isinstance(report,dict) and report.get('release_ready') is False,'build-input report scope is invalid')
    require(report.get('source_revision')==source_sha and report.get('declared_target')==triples[targets.index(suffix)],'build-input source/target mismatch')
    require(isinstance(spdx,dict) and spdx.get('spdxVersion')=='SPDX-2.3','build-input SPDX is invalid')
    payload={binary:members[binary],'LICENSE':members['LICENSE'],'THIRD_PARTY_NOTICES.md':members['THIRD_PARTY_NOTICES.md']}
    payload['WEB-THIRD-PARTY-LICENSES.txt']=members[web_notice]
    web_notices.add(members[web_notice])
    payload.update({'build-inputs/'+n:members['packaging-inputs/'+n] for n in evidence})
    selected.append((suffix,payload))
require(len(versions)==1,'mixed release archive versions')
require(len(sources)==1,'mixed release archive sources')
require(len({payload['LICENSE'] for _,payload in selected})==1,'mixed release licenses')
require(len(web_notices)==1,'mixed Web vendor notices')
for suffix,payload in selected:
    for name,data in payload.items():
        dest=temp/suffix/name
        dest.parent.mkdir(parents=True,exist_ok=True)
        dest.write_bytes(data)
print(json.dumps({'version':next(iter(versions))}))
`;
let ownedTemp;
try {
  if (!process.argv[2]) throw new Error('usage: node scripts/stage-npm-from-release.mjs <release-directory>');
  const source = isAbsolute(process.argv[2]) ? process.argv[2] : join(repoRoot, process.argv[2]);
  ownedTemp = mkdtempSync(join(tmpdir(), 'many-ai-cli-release-stage-'));
  const python = process.env.MANY_AI_PYTHON || (process.platform === 'win32' ? 'python' : 'python3');
  const result = spawnSync(python, ['-c', PREFLIGHT, source, ownedTemp], { encoding: 'utf8', timeout: 60000, maxBuffer: 1024 * 1024 });
  if (result.error || result.status !== 0) throw new Error(`release preflight failed; npm outputs were not changed: ${result.error?.message || result.stderr.trim()}`);
  const { version } = JSON.parse(result.stdout);
  const items = [];
  const root = join(repoRoot, 'npm', 'many-ai-cli');
  const combinedNotices = [];
  let license;
  let webNotice;
  for (const { pkg, binName } of Object.values(TARGETS)) {
    const suffix = pkg.slice('many-ai-cli-'.length);
    const base = join(ownedTemp, suffix);
    const platform = join(repoRoot, 'npm', pkg);
    const add = (dst, data, exec = false) => items.push({ dst, data, exec });
    add(join(platform, 'bin', binName), readFileSync(join(base, binName)), !binName.endsWith('.exe'));
    add(join(platform, 'bin', binName.endsWith('.exe') ? 'many-ai-cli' : 'many-ai-cli.exe'), null);
    license = readFileSync(join(base, 'LICENSE'));
    add(join(platform, 'LICENSE'), license);
    const notice = readFileSync(join(base, 'THIRD_PARTY_NOTICES.md'));
    add(join(platform, 'THIRD_PARTY_NOTICES.md'), notice);
    webNotice = readFileSync(join(base, WEB_NOTICE));
    add(join(platform, WEB_NOTICE), webNotice);
    combinedNotices.push(Buffer.from('\n## '+suffix+'\n'), notice);
    for (const name of EVIDENCE) {
      const data = readFileSync(join(base, 'build-inputs', name));
      add(join(platform, 'build-inputs', name), data);
      add(join(root, 'build-inputs', suffix, name), data);
    }
    add(join(root, 'build-inputs', suffix, 'THIRD_PARTY_NOTICES.md'), notice);
    add(join(platform, 'build-inputs', 'README.txt'), SCOPE);
  }
  items.push({ dst: join(root, 'LICENSE'), data: license },
    { dst: join(root, WEB_NOTICE), data: webNotice },
    { dst: join(root, 'THIRD_PARTY_NOTICES.md'), data: Buffer.concat([SCOPE, ...combinedNotices]) },
    { dst: join(root, 'build-inputs', 'README.txt'), data: SCOPE });
  stagePayloads(items);
  console.log(`All four ${version} release ZIP payloads staged and byte-verified; build-input evidence is not a distribution SBOM.`);
} catch (error) {
  console.error(`npm release staging failed: ${error.message}`);
  process.exitCode = 1;
} finally {
  if (ownedTemp) rmSync(ownedTemp, { recursive: true, force: true });
}
