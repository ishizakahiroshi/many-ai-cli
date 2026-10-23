"""Synthetic CLI contract tests; never build or execute product binaries."""
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest
import warnings
import zipfile

HELPER = Path(__file__).resolve().parents[2] / 'scripts/prepare-rust-release-channels.py'
SUFFIXES = ('windows-x64', 'linux-x64', 'macos-intel', 'macos-apple-silicon')
BINS = ('many-ai-cli', 'many-ai-cli-launcher')
DOCS = ('LICENSE', 'THIRD_PARTY_NOTICES.md', 'packaging-inputs/BUILD-INPUTS.json',
        'packaging-inputs/CRATE-NOTICES.txt', 'packaging-inputs/build-inputs.spdx.json')
ID = 'ishizakahiroshi.many-ai-cli'


def digest(data):
    return hashlib.sha256(data).hexdigest()


class ReleaseChannels(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name).resolve()
        self.archives = self.root / 'archives'
        self.archives.mkdir()
        self.output = self.root / 'output'
        self.version = '1.0.0'
        self.write_archives()

    def members(self, suffix):
        extension = '.exe' if suffix == 'windows-x64' else ''
        return {**{n + extension: f'SYNTHETIC NOT EXECUTABLE {suffix} {n}\n'.encode() for n in BINS},
                **{n: f'synthetic evidence {n}\n'.encode() for n in DOCS},
                'README.md': b'synthetic ignored readme\n',
                'THIRD-PARTY-NOTICES.md': b'synthetic historic alias\n'}

    def zip_path(self, suffix):
        return self.archives / f'many-ai-cli-{self.version}-{suffix}.zip'

    def write_zip(self, suffix, members=None, special=None):
        with zipfile.ZipFile(self.zip_path(suffix), 'w') as z:
            for name, data in (self.members(suffix) if members is None else members).items():
                z.writestr(name, data)
            if special:
                with warnings.catch_warnings():
                    warnings.simplefilter('ignore', UserWarning)
                    z.writestr(*special)

    def write_archives(self):
        for suffix in SUFFIXES:
            self.write_zip(suffix)
        self.sums()

    def sums(self):
        (self.archives / 'SHA256SUMS.txt').write_text(''.join(
            f'{digest(p.read_bytes())}  {p.name}\n' for p in sorted(self.archives.glob('*.zip'))), encoding='utf-8')

    def run_cli(self, tag=None, repo='example/project'):
        return subprocess.run([sys.executable, str(HELPER), '--archives', str(self.archives),
                               '--tag=' + (tag or 'v' + self.version), '--repo=' + repo,
                               '--output', str(self.output)], capture_output=True, text=True, timeout=60)

    def reject(self, **kwargs):
        result = self.run_cli(**kwargs)
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn('channel preparation failed', result.stderr)
        self.assertFalse(self.output.exists())

    def test_complete_stable_cli_and_exact_payloads(self):
        result = self.run_cli()
        self.assertEqual(result.returncode, 0, result.stderr)
        for name, data in self.members('linux-x64').items():
            if name == 'README.md':
                self.assertFalse((self.output / 'rpm/SOURCES' / name).exists())
                continue
            destination = 'usr/bin/' + name if name in BINS else 'usr/share/doc/many-ai-cli/' + name
            self.assertEqual((self.output / 'deb-root' / destination).read_bytes(), data)
            self.assertEqual((self.output / 'rpm/SOURCES' / name).read_bytes(), data)
            if name in BINS and os.name != 'nt':
                self.assertEqual(stat.S_IMODE((self.output / 'deb-root' / destination).stat().st_mode), 0o755)
        control = (self.output / 'deb-root/DEBIAN/control').read_text()
        for field in ('Package: many-ai-cli', 'Version: 1.0.0', 'Architecture: amd64', 'License: MIT'):
            self.assertIn(field, control)
        spec = (self.output / 'rpm/SPECS/many-ai-cli.spec').read_text()
        self.assertIn('BuildArch: x86_64', spec)
        self.assertIn('%global __os_install_post %{nil}', spec)
        self.assertNotIn('%post\n', spec)
        self.assertNotIn('cargo', spec)
        manifest = json.loads((self.output / 'channel-inputs.json').read_bytes())
        self.assertEqual(manifest['version'], self.version)
        self.assertEqual(manifest['skipped_channels'], {})
        self.assertEqual(len(manifest['archives']), 4)
        for item in manifest['outputs']:
            data = (self.output / item['path']).read_bytes()
            self.assertEqual((len(data), digest(data)), (item['bytes'], item['sha256']))
            self.assertFalse(Path(item['path']).is_absolute())
        for item in manifest['linux_payloads']:
            self.assertEqual(item['sha256'], digest(self.members('linux-x64')[item['name']]))

    def test_homebrew_and_winget_contract(self):
        self.assertEqual(self.run_cli().returncode, 0)
        cask = (self.output / 'homebrew/many-ai-cli.rb').read_text()
        installer = (self.output / f'winget/{ID}.installer.yaml').read_text()
        for suffix in ('macos-intel', 'macos-apple-silicon'):
            self.assertIn(f'https://github.com/example/project/releases/download/v1.0.0/{self.zip_path(suffix).name}', cask)
            self.assertIn(digest(self.zip_path(suffix).read_bytes()), cask)
        self.assertIn('on_intel do', cask)
        self.assertIn('on_arm do', cask)
        for name in BINS:
            self.assertIn(f'binary "{name}"', cask)
            self.assertIn(f'RelativeFilePath: {name}.exe', installer)
            self.assertIn(f'PortableCommandAlias: {name}\n', installer)
        for value in ('InstallerType: zip', 'NestedInstallerType: portable', 'Architecture: x64',
                      digest(self.zip_path('windows-x64').read_bytes()).upper()):
            self.assertIn(value, installer)
        yamls = list((self.output / 'winget').glob('*.yaml'))
        self.assertEqual(len(yamls), 3)
        for p in yamls:
            text = p.read_text()
            self.assertIn('ManifestVersion: 1.12.0', text)
            self.assertIn(f'PackageIdentifier: {ID}', text)
            self.assertIn('PackageVersion: "1.0.0"', text)

    def test_cask_explicitly_excludes_linux(self):
        self.assertEqual(self.run_cli().returncode, 0)
        cask = (self.output / 'homebrew/many-ai-cli.rb').read_text()
        self.assertIn('  depends_on :macos\n', cask)

    def test_release_evidence_is_allowed_but_not_copied(self):
        for suffix in SUFFIXES:
            self.write_zip(suffix, special=('RELEASE-EVIDENCE.txt', b'synthetic supplemental evidence'))
        self.sums()
        result = self.run_cli()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(list(self.output.rglob('RELEASE-EVIDENCE.txt')), [])
        manifest = json.loads((self.output / 'channel-inputs.json').read_bytes())
        self.assertNotIn('RELEASE-EVIDENCE.txt', [p['name'] for p in manifest['linux_payloads']])

    def test_deterministic_outputs(self):
        self.assertEqual(self.run_cli().returncode, 0)
        first = {p.relative_to(self.output).as_posix(): p.read_bytes() for p in self.output.rglob('*') if p.is_file()}
        self.output = self.root / 'second'
        self.assertEqual(self.run_cli().returncode, 0)
        self.assertEqual(first, {p.relative_to(self.output).as_posix(): p.read_bytes() for p in self.output.rglob('*') if p.is_file()})

    def test_prerelease_skips_stable_channels_and_maps_package_versions(self):
        for p in self.archives.glob('*.zip'):
            p.unlink()
        self.version = '1.0.0-rc.2'
        self.write_archives()
        self.assertEqual(self.run_cli().returncode, 0)
        self.assertFalse((self.output / 'homebrew').exists())
        self.assertFalse((self.output / 'winget').exists())
        manifest = json.loads((self.output / 'channel-inputs.json').read_bytes())
        self.assertEqual(set(manifest['skipped_channels']), {'homebrew', 'winget'})
        self.assertEqual(manifest['package_version'], '1.0.0~rc.2')
        for path in ('deb-root/DEBIAN/control', 'rpm/SPECS/many-ai-cli.spec'):
            self.assertIn('Version: 1.0.0~rc.2', (self.output / path).read_text())

    def test_reject_tags(self):
        for tag in ('1.0.0', 'v01.0.0', 'v1.00.0', 'v1.0.00', 'v1.0.0+build',
                    'v1.0.0-01', 'v1.0.0-rc.01', 'v1.0.0-alpha.1', 'v1.0.0-rc.1.x',
                    'v1.0.0\nInjected: x', 'v1.0.0-$(x)', 'v1.0.0-%{x}', 'v1.0.0-#{x}', 'v' + '1' * 129 + '.0.0'):
            with self.subTest(tag=tag):
                self.reject(tag=tag)

    def test_reject_repository_injection(self):
        for repo in ('owner/repo\nX: x', 'owner/repo#x', 'owner/../x', '../repo',
                     'owner/repo%{x}', 'owner/repo$(x)', 'owner/"x', '-owner/repo', 'owner-/repo', 'owner/repo;x'):
            with self.subTest(repo=repo):
                self.reject(repo=repo)

    def test_missing_target(self):
        self.zip_path('macos-apple-silicon').unlink()
        self.sums()
        self.reject()

    def test_mixed_version(self):
        self.zip_path('linux-x64').rename(self.archives / 'many-ai-cli-2.0.0-linux-x64.zip')
        self.sums()
        self.reject()

    def test_extra_archive_duplicate_target(self):
        (self.archives / 'copy-linux-x64.zip').write_bytes(self.zip_path('linux-x64').read_bytes())
        self.sums()
        self.reject()

    def test_checksum_mismatch(self):
        with self.zip_path('macos-apple-silicon').open('ab') as f:
            f.write(b'x')
        self.reject()

    def test_duplicate_checksum(self):
        p = self.archives / 'SHA256SUMS.txt'
        p.write_bytes(p.read_bytes() + p.read_bytes().splitlines(keepends=True)[0])
        self.reject()

    def test_malformed_checksum(self):
        (self.archives / 'SHA256SUMS.txt').write_text('not a checksum\n')
        self.reject()

    def test_unknown_or_unsafe_supplemental_checksums_are_rejected(self):
        names = ('unrelated.txt', 'many-ai-cli-2.0.0-linux-x64.zip',
                 'many-ai-cli-1.0.0-unknown.spdx.json', '../escape.spdx.json',
                 '/absolute.spdx.json', 'C:\\escape.spdx.json',
                 'bad name.spdx.json', 'NUL', 'notice;echo.spdx.json',
                 'notice%{macro}.spdx.json', 'notice\x00.spdx.json')
        path = self.archives / 'SHA256SUMS.txt'
        original = path.read_bytes()
        for name in names:
            with self.subTest(name=name):
                path.write_bytes(original + (('a' * 64) + '  ' + name + '\n').encode())
                self.reject()
        path.write_bytes(original)

    def canonical_checksums(self):
        self.sums()
        names = [self.zip_path(s).name + '.spdx.json' for s in SUFFIXES]
        if '-' not in self.version:
            names += [f'many-ai-cli-{self.version}-linux-x64.deb',
                      f'many-ai-cli-{self.version}-linux-x64.deb.spdx.json',
                      f'many-ai-cli-{self.version}-1.x86_64.rpm',
                      f'many-ai-cli-{self.version}-1.x86_64.rpm.spdx.json']
        path = self.archives / 'SHA256SUMS.txt'
        with path.open('a', encoding='utf-8') as stream:
            for name in names:
                stream.write(digest(('synthetic ' + name).encode()) + '  ' + name + '\n')
        self.assertTrue(all(not (self.archives / name).exists() for name in names))
        return names

    def test_canonical_stable_twelve_checksum_records(self):
        self.canonical_checksums()
        self.assertEqual(len((self.archives / 'SHA256SUMS.txt').read_text().splitlines()), 12)
        result = self.run_cli()
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = json.loads((self.output / 'channel-inputs.json').read_bytes())
        self.assertEqual(len(manifest['archives']), 4)

    def test_canonical_rc_eight_checksum_records(self):
        for path in self.archives.glob('*.zip'):
            path.unlink()
        self.version = '1.0.0-rc.1'
        self.write_archives()
        self.canonical_checksums()
        self.assertEqual(len((self.archives / 'SHA256SUMS.txt').read_text().splitlines()), 8)
        result = self.run_cli()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.output / 'homebrew').exists())
        self.assertFalse((self.output / 'winget').exists())

    def test_canonical_supplement_duplicate_and_invalid_hash(self):
        self.canonical_checksums()
        path = self.archives / 'SHA256SUMS.txt'
        original = path.read_bytes()
        last = original.splitlines(keepends=True)[-1]
        for extra in (last, b'not-a-hash  ' + last.split(b'  ', 1)[1]):
            with self.subTest(extra=extra):
                path.write_bytes(original + extra)
                self.reject()
        path.write_bytes(original.replace(last[:64], b'g' * 64))
        self.reject()

    def test_canonical_selected_zip_missing_or_mismatch(self):
        self.canonical_checksums()
        path = self.archives / 'SHA256SUMS.txt'
        original = path.read_bytes()
        first = original.splitlines(keepends=True)[0]
        for contents in (original.replace(first, b''), original.replace(first[:64], b'0' * 64)):
            with self.subTest(contents=contents):
                path.write_bytes(contents)
                self.reject()

    def test_rc_does_not_allow_stable_package_supplements(self):
        for path in self.archives.glob('*.zip'):
            path.unlink()
        self.version = '1.0.0-rc.1'
        self.write_archives()
        self.canonical_checksums()
        path = self.archives / 'SHA256SUMS.txt'
        original = path.read_bytes()
        for name in ('many-ai-cli-1.0.0-rc.1-linux-x64.deb',
                     'many-ai-cli-1.0.0~rc.1-1.x86_64.rpm',
                     'SHA256SUMS.txt', 'SHA256SUMS.txt.sig', 'SHA256SUMS.txt.pem'):
            with self.subTest(name=name):
                path.write_bytes(original + (('a' * 64) + '  ' + name + '\n').encode())
                self.reject()

    def test_missing_checksum(self):
        p = self.archives / 'SHA256SUMS.txt'
        p.write_bytes(b'\n'.join(p.read_bytes().splitlines()[1:]) + b'\n')
        self.reject()

    def test_empty_archive(self):
        self.zip_path('linux-x64').write_bytes(b'')
        self.sums()
        self.reject()

    def test_invalid_zip(self):
        self.zip_path('linux-x64').write_bytes(b'synthetic not a zip')
        self.sums()
        self.reject()

    def test_required_members_missing_or_empty_in_every_target(self):
        for suffix in SUFFIXES:
            required = [n + ('.exe' if suffix == 'windows-x64' else '') for n in BINS] + list(DOCS)
            for name in required:
                for empty in (True, False):
                    with self.subTest(suffix=suffix, name=name, empty=empty):
                        members = self.members(suffix)
                        if empty:
                            members[name] = b''
                        else:
                            del members[name]
                        self.write_zip(suffix, members)
                        self.sums()
                        self.reject()
                        self.write_zip(suffix)
        self.sums()

    def test_unsafe_ignored_members(self):
        for name in ('../escape', '/absolute', 'C:/escape', 'dir\\escape', 'a/../b',
                     'dir//file', 'file.', 'NUL', 'aux.txt', 'bad name', 'a\nname', 'directory/'):
            with self.subTest(name=name):
                # zipfile normalizes separators on Windows: patch both raw names
                # only after writing a safe, equal-length placeholder.
                placeholder = 'Z' * len(name)
                self.write_zip('linux-x64', special=(placeholder, b'synthetic'))
                path = self.zip_path('linux-x64')
                raw = path.read_bytes()
                self.assertEqual(raw.count(placeholder.encode()), 2)
                raw = raw.replace(placeholder.encode(), name.encode())
                self.assertEqual(raw.count(name.encode()), 2)
                path.write_bytes(raw)
                with zipfile.ZipFile(path) as archive:
                    self.assertEqual(archive.infolist()[-1].orig_filename, name)
                self.sums()
                self.reject()

    def test_duplicate_and_case_colliding_members(self):
        for name in ('many-ai-cli', 'MANY-AI-CLI'):
            with self.subTest(name=name):
                self.write_zip('linux-x64', special=(name, b'synthetic duplicate'))
                self.sums()
                self.reject()

    def test_symlink_and_special_members(self):
        for mode in (stat.S_IFLNK | 0o777, stat.S_IFIFO | 0o600, stat.S_IFDIR | 0o755):
            with self.subTest(mode=mode):
                info = zipfile.ZipInfo('ignored')
                info.create_system = 3
                info.external_attr = mode << 16
                self.write_zip('linux-x64', special=(info, b'target'))
                self.sums()
                self.reject()

    def test_nul_member_is_rejected_before_zip_name_truncation(self):
        self.write_zip('linux-x64', special=('badXname', b'synthetic'))
        path = self.zip_path('linux-x64')
        path.write_bytes(path.read_bytes().replace(b'badXname', b'bad\x00name'))
        self.sums()
        self.reject()

    def test_crc_error_in_ignored_member_is_preflight_failure(self):
        path = self.zip_path('macos-apple-silicon')
        path.write_bytes(path.read_bytes().replace(b'synthetic ignored readme', b'SYNTHETIC ignored readme'))
        self.sums()
        self.reject()

    def test_optional_alias_absent_is_supported(self):
        for suffix in SUFFIXES:
            members = self.members(suffix)
            del members['THIRD-PARTY-NOTICES.md']
            self.write_zip(suffix, members)
        self.sums()
        self.assertEqual(self.run_cli().returncode, 0)
        self.assertFalse((self.output / 'rpm/SOURCES/THIRD-PARTY-NOTICES.md').exists())

    def test_existing_output_unchanged(self):
        self.output.mkdir()
        sentinel = self.output / 'sentinel'
        sentinel.write_bytes(b'preserve')
        result = self.run_cli()
        self.assertEqual(result.returncode, 2)
        self.assertEqual(sentinel.read_bytes(), b'preserve')
        self.assertEqual(list(self.output.iterdir()), [sentinel])

    def test_late_target_failure_creates_no_parent(self):
        self.output = self.root / 'new-parent' / 'output'
        self.write_zip('macos-apple-silicon', {})
        self.sums()
        self.reject()
        self.assertFalse(self.output.parent.exists())

    def test_archive_path_symlink(self):
        original = self.zip_path('linux-x64')
        other = self.root / 'original'
        original.rename(other)
        try:
            original.symlink_to(other)
        except (OSError, NotImplementedError):
            self.skipTest('symlink creation unavailable')
        self.reject()

    def test_output_symlink(self):
        target = self.root / 'absent-target'
        try:
            self.output.symlink_to(target, target_is_directory=True)
        except (OSError, NotImplementedError):
            self.skipTest('symlink creation unavailable')
        result = self.run_cli()
        self.assertEqual(result.returncode, 2)
        self.assertFalse(target.exists())


if __name__ == '__main__':
    unittest.main()
