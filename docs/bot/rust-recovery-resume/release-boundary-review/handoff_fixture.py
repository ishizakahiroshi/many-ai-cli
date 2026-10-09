#!/usr/bin/env python3
"""Offline V03 custody/policy model. No real build, signing, credentials or publishing.

All payloads, identities, signature markers and the canary are deliberately synthetic.
The copied directories share this host/UID: they DO NOT establish runner isolation.
"""
from __future__ import annotations
import argparse, copy, gzip, hashlib, io, json, os, stat, subprocess, sys, tarfile, zipfile
from pathlib import Path

TARGETS = ('windows-x64', 'linux-x64', 'macos-intel', 'macos-apple-silicon')
IDENTITY = {'source_sha': 'f' * 40, 'tag': 'v0.0.0-v03-fixture', 'version': '0.0.0-v03-fixture',
            'workflow': 'synthetic://reviewed-workflow', 'run_id': 'synthetic-run-1', 'run_attempt': 1}
LIMIT = 1024 * 1024

def sha(data): return hashlib.sha256(data).hexdigest()
def enc(obj): return (json.dumps(obj, sort_keys=True, separators=(',', ':')) + '\n').encode()
def require(value, why):
    if not value: raise ValueError(why)
def binary_name(target, launcher=False):
    return 'many-ai-cli' + ('-launcher' if launcher else '') + ('.exe' if target == 'windows-x64' else '')
def safe_name(name):
    require(isinstance(name, str) and name and not any(c in name for c in '\\:\x00'), 'unsafe path')
    require(all(p and p not in ('.', '..') for p in name.split('/')), 'unsafe path')
def zip_bytes(members):
    b = io.BytesIO()
    with zipfile.ZipFile(b, 'w', compression=zipfile.ZIP_STORED) as z:
        for name, data in sorted(members.items()):
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            info.create_system = 3; info.external_attr = (stat.S_IFREG | 0o644) << 16
            z.writestr(info, data)
    return b.getvalue()
def tgz_bytes(members):
    b = io.BytesIO()
    with tarfile.open(fileobj=b, mode='w', format=tarfile.USTAR_FORMAT) as t:
        for name, data in sorted(members.items()):
            info = tarfile.TarInfo(name); info.size = len(data); info.mode = 0o644; info.mtime = 0
            t.addfile(info, io.BytesIO(data))
    return gzip.compress(b.getvalue(), mtime=0)
def read_zip(data):
    result = {}; total = 0
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        for item in z.infolist():
            safe_name(item.filename)
            require(item.filename not in result, 'duplicate ZIP member')
            require(not item.is_dir() and stat.S_ISREG(item.external_attr >> 16), 'nonregular ZIP member')
            total += item.file_size; require(total <= LIMIT, 'oversized ZIP')
            result[item.filename] = z.read(item)
    return result
def read_tgz(data):
    result = {}; total = 0
    # Cap the decompressed tar stream before the archive parser sees it.
    with gzip.GzipFile(fileobj=io.BytesIO(data)) as g: raw = g.read(LIMIT + 1)
    require(len(raw) <= LIMIT, 'oversized tar stream')
    with tarfile.open(fileobj=io.BytesIO(raw), mode='r:') as t:
        for item in t:
            safe_name(item.name); require(item.name not in result, 'duplicate tar member')
            require(item.isfile(), 'nonregular tar member')
            total += item.size; require(total <= LIMIT, 'oversized tar members')
            result[item.name] = t.extractfile(item).read()
    return result

def make_bundle(main_suffix=b''):
    files = {}; rows = []; consumers = {}
    for target in TARGETS:
        main = b'SYNTHETIC NONEXECUTABLE MAIN ' + target.encode() + b'\n' + main_suffix
        launcher = b'SYNTHETIC NONEXECUTABLE LAUNCHER ' + target.encode() + b'\n'
        names = (binary_name(target), binary_name(target, True))
        archive = f'many-ai-cli-{IDENTITY["version"]}-{target}.zip'
        files[archive] = zip_bytes({names[0]: main, names[1]: launcher, 'LICENSE': b'Synthetic notice\n'})
        package = f'many-ai-cli-{target}'
        pkg_file = package + '.tgz'
        files[pkg_file] = tgz_bytes({'package/package.json': enc({'name': package, 'version': IDENTITY['version']}),
                                    'package/bin/' + names[0]: main})
        consumers[target] = {'archive': archive, 'npm': pkg_file, 'main_sha256': sha(main), 'launcher_sha256': sha(launcher)}
    files['many-ai-cli.tgz'] = tgz_bytes({
        'package/package.json': enc({'name': 'many-ai-cli', 'version': IDENTITY['version'],
                                    'optionalDependencies': {f'many-ai-cli-{t}': IDENTITY['version'] for t in TARGETS}}),
        'package/bin/many-ai-cli.mjs': b'// SYNTHETIC SHIM. NEVER EXECUTE.\n'})
    files['SHA256SUMS.txt'] = ''.join(f'{sha(data)}  {name}\n' for name, data in sorted(files.items())).encode()
    # Inert markers model exact transport of an already-produced signature/cert.
    # They are emphatically NOT cryptographic signatures and verify no identity.
    files['SHA256SUMS.txt.sig.fixture'] = b'NOT A SIGNATURE\n' + sha(files['SHA256SUMS.txt']).encode() + b'\n'
    files['SHA256SUMS.txt.pem.fixture'] = b'NOT A CERTIFICATE\nsynthetic identity only\n'
    manifest = {'schema': 1, 'identity': IDENTITY, 'signature_status': 'NOT_CRYPTOGRAPHICALLY_SIGNED',
                'consumers': consumers, 'files': [{'name': n, 'sha256': sha(d), 'bytes': len(d)} for n, d in sorted(files.items())]}
    files['RELEASE-MANIFEST.json'] = enc(manifest)
    return files

def expectation(files):
    return {'identity': copy.deepcopy(IDENTITY), 'manifest_sha256': sha(files['RELEASE-MANIFEST.json'])}

def verify(files, trusted):
    # trusted must be a control-plane pin, not a self-declared hash inside files.
    require(sha(files['RELEASE-MANIFEST.json']) == trusted['manifest_sha256'], 'manifest digest mismatch')
    manifest = json.loads(files['RELEASE-MANIFEST.json'])
    require(manifest['identity'] == trusted['identity'], 'source/tag/workflow/run identity mismatch')
    require(manifest['signature_status'] == 'NOT_CRYPTOGRAPHICALLY_SIGNED', 'fixture misrepresented as signed')
    rows = manifest['files']; names = [r['name'] for r in rows]
    require(len(set(names)) == len(names), 'duplicate manifest file')
    require(set(names) | {'RELEASE-MANIFEST.json'} == set(files), 'undeclared or missing file')
    require(set(manifest['consumers']) == set(TARGETS), 'target set mismatch')
    expected_names = {'many-ai-cli.tgz', 'SHA256SUMS.txt', 'SHA256SUMS.txt.sig.fixture', 'SHA256SUMS.txt.pem.fixture'}
    expected_names |= {f'many-ai-cli-{IDENTITY["version"]}-{t}.zip' for t in TARGETS}
    expected_names |= {f'many-ai-cli-{t}.tgz' for t in TARGETS}
    require(set(names) == expected_names, 'forbidden transfer file')
    for row in rows:
        safe_name(row['name']); data = files[row['name']]
        require(len(data) == row['bytes'] <= LIMIT and sha(data) == row['sha256'], 'file content mismatch')
    sums = files['SHA256SUMS.txt']
    expected_sums = ''.join(f'{sha(data)}  {name}\n' for name, data in sorted(files.items())
                           if name.endswith(('.zip', '.tgz'))).encode()
    require(sums == expected_sums, 'checksums do not match exact payload set')
    require(files['SHA256SUMS.txt.sig.fixture'] == b'NOT A SIGNATURE\n' + sha(sums).encode() + b'\n', 'signature marker binding mismatch')
    observed = {}
    for target in TARGETS:
        row = manifest['consumers'][target]
        require(row['archive'] == f'many-ai-cli-{IDENTITY["version"]}-{target}.zip', 'archive identity mismatch')
        require(row['npm'] == f'many-ai-cli-{target}.tgz', 'npm identity mismatch')
        archive = read_zip(files[row['archive']]); npm = read_tgz(files[row['npm']])
        main, launcher = binary_name(target), binary_name(target, True)
        require(set(archive) == {main, launcher, 'LICENSE'}, 'ZIP role set mismatch')
        require(set(npm) == {'package/package.json', 'package/bin/' + main}, 'npm must contain main only')
        package = json.loads(npm['package/package.json'])
        require(package == {'name': f'many-ai-cli-{target}', 'version': trusted['identity']['version']}, 'npm metadata mismatch')
        require(sha(archive[main]) == row['main_sha256'] == sha(npm['package/bin/' + main]), 'consumer main byte mismatch')
        require(sha(archive[launcher]) == row['launcher_sha256'], 'launcher byte mismatch')
        observed[target] = {'main': sha(archive[main]), 'launcher': sha(archive[launcher])}
    root = read_tgz(files['many-ai-cli.tgz'])
    require(set(root) == {'package/package.json', 'package/bin/many-ai-cli.mjs'}, 'root npm roles mismatch')
    require(json.loads(root['package/package.json']) == {'name': 'many-ai-cli', 'version': trusted['identity']['version'],
        'optionalDependencies': {f'many-ai-cli-{t}': trusted['identity']['version'] for t in TARGETS}}, 'root package identity mismatch')
    return observed

def update_manifest(files, mutate):
    m = json.loads(files['RELEASE-MANIFEST.json']); mutate(m); files['RELEASE-MANIFEST.json'] = enc(m)
def refresh_file_rows(files):
    update_manifest(files, lambda m: m.update(files=[{'name': n, 'sha256': sha(d), 'bytes': len(d)}
        for n, d in sorted(files.items()) if n != 'RELEASE-MANIFEST.json']))

def main():
    ap = argparse.ArgumentParser(description=__doc__); ap.add_argument('--output', type=Path, required=True)
    out = ap.parse_args().output; out.mkdir(parents=True, exist_ok=False)
    original = make_bundle(); trust = expectation(original); tests = []
    observed = verify(original, trust); tests.append({'name': 'four_targets_two_roles_and_npm_main_identity', 'result': 'PASS'})
    def reject(name, mutate, repin=False):
        files = copy.deepcopy(original); mutate(files)
        try: verify(files, expectation(files) if repin else trust)
        except (ValueError, KeyError) as e: tests.append({'name': name, 'result': 'PASS_REJECTED', 'reason': str(e)})
        else: raise AssertionError(name + ' unexpectedly accepted')
    zipname = f'many-ai-cli-{IDENTITY["version"]}-linux-x64.zip'
    reject('archive_tamper', lambda f: f.__setitem__(zipname, f[zipname] + b'tampered'))
    reject('signature_marker_tamper', lambda f: f.__setitem__('SHA256SUMS.txt.sig.fixture', b'changed'))
    reject('checksum_tamper', lambda f: f.__setitem__('SHA256SUMS.txt', b'changed'))
    reject('manifest_self_rewrite_without_control_pin', lambda f: update_manifest(f, lambda m: m['identity'].update(source_sha='e' * 40)))
    for field, value in [('source_sha', 'e' * 40), ('tag', 'v9.9.9'), ('workflow', 'synthetic://other'), ('run_id', 'other'), ('run_attempt', 2)]:
        reject('reject_wrong_' + field, lambda f, k=field, v=value: update_manifest(f, lambda m: m['identity'].update({k: v})), True)
    reject('missing_target_even_with_new_digest', lambda f: update_manifest(f, lambda m: m['consumers'].pop('linux-x64')), True)
    reject('undeclared_workspace_file', lambda f: f.__setitem__('node_modules/poison.js', b'inert'))
    def forbidden(f): f['dependency-hook.py'] = b'INERT'; refresh_file_rows(f)
    reject('declared_dependency_hook_rejected', forbidden, True)
    def inconsistent_npm(f):
        target = 'linux-x64'; name = f'many-ai-cli-{target}.tgz'
        f[name] = tgz_bytes({'package/package.json': enc({'name': f'many-ai-cli-{target}', 'version': IDENTITY['version']}),
                             'package/bin/many-ai-cli': b'DIFFERENT INERT NPM MAIN BYTES'})
        f['SHA256SUMS.txt'] = ''.join(f'{sha(d)}  {n}\n' for n, d in sorted(f.items()) if n.endswith(('.zip', '.tgz'))).encode()
        f['SHA256SUMS.txt.sig.fixture'] = b'NOT A SIGNATURE\n' + sha(f['SHA256SUMS.txt']).encode() + b'\n'
        refresh_file_rows(f)
    reject('selfconsistent_outer_hashes_cannot_hide_npm_main_drift', inconsistent_npm, True)
    # Parser checks are direct unit checks, independent of the enclosing digest gate.
    linked_zip = io.BytesIO()
    with zipfile.ZipFile(linked_zip, 'w') as z:
        item = zipfile.ZipInfo('main'); item.create_system = 3; item.external_attr = (stat.S_IFLNK | 0o777) << 16
        z.writestr(item, 'target')
    linked_tar = io.BytesIO()
    with tarfile.open(fileobj=linked_tar, mode='w') as t:
        item = tarfile.TarInfo('package/link'); item.type = tarfile.SYMTYPE; item.linkname = 'target'; t.addfile(item)
    for name, payload, parser in [
        ('path_traversal_zip', zip_bytes({'../escape': b'inert'}), read_zip),
        ('symlink_zip', linked_zip.getvalue(), read_zip),
        ('symlink_tar', gzip.compress(linked_tar.getvalue(), mtime=0), read_tgz),
        ('absolute_path_tar', tgz_bytes({'/escape': b'inert'}), read_tgz),
        ('oversized_tar', tgz_bytes({'package/large': b'a' * (LIMIT + 1)}), read_tgz)]:
        try: parser(payload)
        except ValueError as e: tests.append({'name': name, 'result': 'PASS_REJECTED', 'reason': str(e)})
        else: raise AssertionError(name + ' unexpectedly accepted')
    # Mutation of every consumer plus its legitimately re-pinned manifest passes:
    # custody hashes cannot establish that an authorized producer built benign code.
    hostile = make_bundle(b'INERT TEXT MODEL OF COMPROMISED BUILDER OUTPUT\n')
    verify(hostile, expectation(hostile))
    tests.append({'name': 'compromised_producer_cannot_be_detected_by_custody_hashes', 'result': 'EXPECTED_RESIDUAL_RISK'})
    # Demonstrate step-local environment absence does not undo persistent files.
    # Only a PUBLIC fixture canary is exposed; subprocess inherits no credentials.
    same = out / 'same-runner-model'; same.mkdir()
    helper = same / 'later-step.py'
    helper.write_text("import os\nfrom pathlib import Path\nPath('PUBLIC-CANARY-OBSERVED.txt').write_text(os.environ['V03_PUBLIC_CANARY'])\n")
    public = 'PUBLIC FIXTURE CANARY - NOT A SECRET OR CREDENTIAL'
    subprocess.run([sys.executable, '-I', str(helper.resolve())], cwd=same,
                   env={'V03_PUBLIC_CANARY': public}, check=True)
    require((same / 'PUBLIC-CANARY-OBSERVED.txt').read_text() == public, 'persistence model failed')
    tests.append({'name': 'same_directory_later_step_reads_public_canary', 'result': 'PASS_MODEL_ONLY'})
    # Allowlist-only transfer, then data-only validation in a second directory.
    # No files or processes from the builder's workspace are executable inputs here.
    transport = out / 'artifact-transfer'; transport.mkdir()
    for name, data in original.items(): (transport / name).write_bytes(data)
    fresh = out / 'fresh-directory-model'; fresh.mkdir()
    require(not (fresh / 'later-step.py').exists(), 'unexpected builder helper')
    received = {p.name: p.read_bytes() for p in transport.iterdir()}
    require(verify(received, trust) == observed, 'transfer identity changed')
    (fresh / 'VERIFIED-CONSUMERS.json').write_bytes(enc(observed))
    (out / 'CONTROL-PLANE-PIN.json').write_bytes(enc(trust))
    tests.append({'name': 'allowlisted_transfer_preserves_bytes_excludes_helper', 'result': 'PASS_MODEL_ONLY'})
    receipt = {'scope': 'synthetic offline model, not actual runner isolation/build/signing/publishing',
               'signature_verification': 'NOT RUN; inert transport markers only', 'tests': tests,
               'counts': {'cases': len(tests), 'unexpected_failures': 0},
               'payload_identity': observed, 'manifest_sha256': trust['manifest_sha256'],
               'limitations': ['same host and UID', 'no real native artifacts or platform smoke',
                               'no authenticity/cryptographic signature verification',
                               'no token/OIDC/network use', 'no actual GitHub artifact service transfer',
                               'no deb/rpm generation or validation', 'not a production verifier']}
    (out / 'DEMO-RECEIPT.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps({'cases': len(tests), 'unexpected_failures': 0, 'signature': 'not cryptographic', 'receipt': str(out / 'DEMO-RECEIPT.json')}))
if __name__ == '__main__': main()
