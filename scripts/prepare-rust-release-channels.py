#!/usr/bin/env python3
"""Prepare offline channel inputs from four verified release ZIPs, never publish.

Caller authenticates SHA256SUMS.txt separately; this helper checks bytes, not a
signature. Canonical supplemental checksum rows are validated, but their files
are not fetched or byte-verified. Build-input evidence is not license clearance.
Only stable SemVer and rc.N prereleases are representable here. rc.N maps to
~rc.N in Debian/RPM and skips Homebrew/winget (never updates stable channels).
References: Debian Policy 5.6.12; rpm.org/docs/latest/manual/spec.html;
 github.com/microsoft/winget-pkgs/tree/master/doc/manifest/schema/1.12.0
Package builds/installation/update/rollback remain integration checks. Examples:
 dpkg-deb --build --root-owner-group OUT/deb-root many-ai-cli.deb
 rpmbuild -bb --define "_topdir ABS_OUT/rpm" \
   --define "_sourcedir ABS_OUT/rpm/SOURCES" OUT/rpm/SPECS/many-ai-cli.spec
Use an isolated absolute OUT with no RPM macro characters for integration.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import re
import stat
import zipfile
import zlib

SUFFIXES = ('windows-x64', 'linux-x64', 'macos-intel', 'macos-apple-silicon')
BINS = ('many-ai-cli', 'many-ai-cli-launcher')
WEB_NOTICE = 'web/src/vendor/THIRD_PARTY_LICENSES.txt'
WEB_NOTICE_OUTPUT = 'WEB-THIRD-PARTY-LICENSES.txt'
DOCS = ('LICENSE', 'THIRD_PARTY_NOTICES.md', 'packaging-inputs/BUILD-INPUTS.json',
        'packaging-inputs/CRATE-NOTICES.txt', 'packaging-inputs/build-inputs.spdx.json', WEB_NOTICE)
ALIAS = 'THIRD-PARTY-NOTICES.md'
ID = 'ishizakahiroshi.many-ai-cli'
SCHEMA = '1.12.0'
# Intentionally public project contact, already listed in doxguard.ci.json.
MAINTAINER_EMAIL = 'ishizakahiroshi.dev@gmail.com'  # doxguard: allow ishizakahiroshi.dev@gmail.com


def sha(data):
    return hashlib.sha256(data).hexdigest()


def identity(tag, repo):
    num = r'(?:0|[1-9][0-9]*)'
    identifier = r'(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)'
    match = re.fullmatch(r'v(' + num + r'\.' + num + r'\.' + num +
                         r')(?:-(' + identifier + r'(?:\.' + identifier + r')*))?', tag)
    if not match or len(tag[1:]) > 128:
        raise ValueError('tag must be strict vMAJOR.MINOR.PATCH[-prerelease], without build metadata')
    if (not re.fullmatch(r'[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})/[A-Za-z0-9][A-Za-z0-9._-]{0,99}', repo)
            or repo.split('/')[0].endswith('-') or repo.split('/')[1] in ('.', '..')):
        raise ValueError('invalid owner/repository')
    prerelease = match[2]
    if prerelease and not re.fullmatch(r'rc\.(?:0|[1-9][0-9]*)', prerelease):
        raise ValueError('unsupported Debian/RPM prerelease: only rc.N has an explicit mapping')
    return tag[1:], match[1] + ('~' + prerelease if prerelease else ''), bool(prerelease)


def no_links(path):
    if any(p.is_symlink() for p in (path, *path.parents)):
        raise ValueError('symlink paths are not accepted')


def regular(path):
    no_links(path)
    if not path.is_file():
        raise ValueError('input must be a regular file')
    data = path.read_bytes()
    if not data:
        raise ValueError('empty input')
    return data


def safe_member(name):
    # Validate even ignored members; never use extract()/extractall().
    parts = name.split('/')
    return (bool(name) and '\\' not in name and '\x00' not in name
            and all(re.fullmatch(r'[A-Za-z0-9_-][A-Za-z0-9._-]*', p)
                    and p not in ('.', '..') and not p.endswith('.')
                    and not re.fullmatch(r'(?i:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\..*)?', p)
                    for p in parts))


def preflight(archives, version):
    no_links(archives)
    if not archives.is_dir():
        raise ValueError('archives must be a directory')
    names = {s: f'many-ai-cli-{version}-{s}.zip' for s in SUFFIXES}
    if {p.name for p in archives.iterdir() if p.name.lower().endswith('.zip')} != set(names.values()):
        raise ValueError('expected exactly the four release ZIPs of the requested version')
    # Canonical signed lists also cover distribution evidence and stable Linux
    # packages. The retry caller downloads only ZIPs; do not require those other
    # files locally or claim their byte hashes were verified.
    allowed = set(names.values()) | {name + '.spdx.json' for name in names.values()}
    if '-' not in version:
        packages = (f'many-ai-cli-{version}-linux-x64.deb',
                    f'many-ai-cli-{version}-1.x86_64.rpm')
        allowed.update(packages)
        allowed.update(name + '.spdx.json' for name in packages)
    checksums = {}
    for line in regular(archives / 'SHA256SUMS.txt').decode('utf-8').splitlines():
        m = re.fullmatch(r'([0-9a-fA-F]{64}) [ *]([^/\\\s]+)', line)
        if not m or m[2] not in allowed or m[2] in checksums:
            raise ValueError('invalid or duplicate checksum record')
        checksums[m[2]] = m[1].lower()
    if not set(names.values()).issubset(checksums):
        raise ValueError('checksums must include all four release ZIPs')
    verified, payloads = [], {}
    for suffix, name in names.items():
        raw = regular(archives / name)
        if sha(raw) != checksums[name]:
            raise ValueError('ZIP checksum mismatch')
        with zipfile.ZipFile(io.BytesIO(raw)) as archive:
            seen = set()
            for member in archive.infolist():
                mode = member.external_attr >> 16
                key = member.filename.casefold()
                if (not safe_member(member.orig_filename) or member.orig_filename != member.filename
                        or key in seen or member.is_dir() or member.flag_bits & 1
                        or stat.S_IFMT(mode) not in (0, stat.S_IFREG)):
                    raise ValueError('unsafe, duplicate, encrypted, or non-regular ZIP member')
                seen.add(key)
            binaries = tuple(n + ('.exe' if suffix == 'windows-x64' else '') for n in BINS)
            required = (*binaries, *DOCS)
            if not set(required).issubset(archive.namelist()):
                raise ValueError('missing binary or required license/build-input evidence')
            selected = (*required, *((ALIAS,) if ALIAS in archive.namelist() else ()))
            payload = {WEB_NOTICE_OUTPUT if n == WEB_NOTICE else n: archive.read(n) for n in selected}
            if not all(payload.values()):
                raise ValueError('empty selected ZIP member')
            # CRC-check all bytes, including safe but ignored documentation.
            if archive.testzip() is not None:
                raise ValueError('ZIP CRC mismatch')
        verified.append({'suffix': suffix, 'name': name, 'sha256': sha(raw), 'bytes': len(raw)})
        payloads[suffix] = payload
    if len({payload[WEB_NOTICE_OUTPUT] for payload in payloads.values()}) != 1:
        raise ValueError('mixed Web vendor notices')
    return verified, payloads


def render(version, package_version, prerelease, repo, tag, verified, payloads):
    files = {}
    modes = {}
    def add(path, value, mode=0o644):
        files[path] = value.encode('utf-8') if isinstance(value, str) else value
        modes[path] = mode
    linux = payloads['linux-x64']
    for name, data in linux.items():
        dest = 'usr/bin/' + name if name in BINS else 'usr/share/doc/many-ai-cli/' + name
        add('deb-root/' + dest, data, 0o755 if name in BINS else 0o644)
        add('rpm/SOURCES/' + name, data, 0o755 if name in BINS else 0o644)
    add('deb-root/DEBIAN/control', f'''Package: many-ai-cli
Version: {package_version}
Section: utils
Priority: optional
Architecture: amd64
Maintainer: many-ai-cli maintainers <{MAINTAINER_EMAIL}>
Homepage: https://github.com/{repo}
License: MIT
Description: Local workspace for AI command-line tools
 Prebuilt release binaries; channel input preparation only.
''')
    docs = [name for name in linux if name not in BINS]
    install = '\n'.join(f'install -Dpm 0644 "%{{_sourcedir}}/{n}" "%{{buildroot}}/usr/share/doc/many-ai-cli/{n}"' for n in docs)
    docfiles = '\n'.join('/usr/share/doc/many-ai-cli/' + n for n in docs)
    add('rpm/SPECS/many-ai-cli.spec', f'''# Prebuilt bytes only; no compilation or runtime scriptlets.
%global debug_package %{{nil}}
%global __os_install_post %{{nil}}
Name: many-ai-cli
Version: {package_version}
Release: 1
Summary: Local workspace for AI command-line tools
License: MIT
URL: https://github.com/{repo}
BuildArch: x86_64
AutoReqProv: no

%description
Prebuilt release binaries. Build-input evidence is not distribution clearance.

%prep
%build
%install
install -Dpm 0755 "%{{_sourcedir}}/many-ai-cli" "%{{buildroot}}/usr/bin/many-ai-cli"
install -Dpm 0755 "%{{_sourcedir}}/many-ai-cli-launcher" "%{{buildroot}}/usr/bin/many-ai-cli-launcher"
{install}

%files
%defattr(-,root,root,-)
/usr/bin/many-ai-cli
/usr/bin/many-ai-cli-launcher
{docfiles}
''')
    urls = {a['suffix']: f"https://github.com/{repo}/releases/download/{tag}/{a['name']}" for a in verified}
    hashes = {a['suffix']: a['sha256'] for a in verified}
    skipped = {}
    if prerelease:
        skipped = dict.fromkeys(('homebrew', 'winget'), 'prerelease: stable channel metadata intentionally omitted')
    else:
        add('homebrew/many-ai-cli.rb', f'''cask "many-ai-cli" do
  version "{version}"
  on_intel do
    sha256 "{hashes['macos-intel']}"
    url "{urls['macos-intel']}"
  end
  on_arm do
    sha256 "{hashes['macos-apple-silicon']}"
    url "{urls['macos-apple-silicon']}"
  end
  depends_on :macos
  name "many-ai-cli"
  desc "Local workspace for AI command-line tools"
  homepage "https://github.com/{repo}"
  binary "many-ai-cli"
  binary "many-ai-cli-launcher"
end
''')
        common = f'PackageIdentifier: {ID}\nPackageVersion: "{version}"\n'
        def yaml(kind, body):
            return (f'# yaml-language-server: $schema=https://aka.ms/winget-manifest.{kind}.{SCHEMA}.schema.json\n'
                    + common + body + f'ManifestType: {kind}\nManifestVersion: {SCHEMA}\n')
        add(f'winget/{ID}.yaml', yaml('version', 'DefaultLocale: en-US\n'))
        add(f'winget/{ID}.locale.en-US.yaml', yaml('defaultLocale', f'''PackageLocale: en-US
Publisher: ishizakahiroshi
PackageName: many-ai-cli
License: MIT
ShortDescription: Local workspace for AI command-line tools
PackageUrl: https://github.com/{repo}
'''))
        add(f'winget/{ID}.installer.yaml', yaml('installer', f'''InstallerType: zip
NestedInstallerType: portable
NestedInstallerFiles:
- RelativeFilePath: many-ai-cli.exe
  PortableCommandAlias: many-ai-cli
- RelativeFilePath: many-ai-cli-launcher.exe
  PortableCommandAlias: many-ai-cli-launcher
Installers:
- Architecture: x64
  InstallerUrl: {urls['windows-x64']}
  InstallerSha256: {hashes['windows-x64'].upper()}
'''))
    manifest = {'schema_version': 1, 'status': 'prepared-only-not-built-installed-or-published',
                'tag': tag, 'version': version, 'repository': repo,
                'package_version': package_version, 'version_mapping': 'rc.N -> ~rc.N; stable unchanged',
                'archives': verified, 'skipped_channels': skipped,
                'linux_payloads': [{'name': n, 'sha256': sha(d), 'bytes': len(d)} for n, d in sorted(linux.items())],
                'outputs': [{'path': n, 'sha256': sha(d), 'bytes': len(d)} for n, d in sorted(files.items())],
                'scope': 'Four ZIP byte checksums verified, not signatures; allowed supplemental checksum records are syntax-checked only, not fetched or byte-verified. Build-input evidence is not a distribution SBOM or license clearance.'}
    add('channel-inputs.json', json.dumps(manifest, sort_keys=True, indent=2) + '\n')
    return files, modes


def prepare(archives, tag, repo, output):
    version, package_version, prerelease = identity(tag, repo)
    no_links(output)
    if output.exists():
        raise ValueError('output must be new; existing files are never overwritten')
    verified, payloads = preflight(archives, version)
    files, modes = render(version, package_version, prerelease, repo, tag, verified, payloads)
    # All four archives and every metadata byte validated before any output exists.
    output.mkdir(parents=True, exist_ok=False)
    for name, data in sorted(files.items()):
        path = output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open('xb') as stream:
            stream.write(data)
        path.chmod(modes[name])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archives', type=Path, required=True)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--repo', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        prepare(args.archives, args.tag, args.repo, args.output)
    except (ValueError, OSError, UnicodeError, zipfile.BadZipFile, zlib.error, RuntimeError, NotImplementedError):
        parser.exit(2, 'channel preparation failed: invalid inputs or unavailable output; nothing published\n')
    print('Channel inputs prepared; package builds, installation and publication are not performed.')


if __name__ == '__main__':
    main()
