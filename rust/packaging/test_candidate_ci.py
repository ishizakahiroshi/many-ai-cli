"""Candidate identity/failure receipts; no toolchain or network needed."""
import importlib.util
import ctypes
import json
import os
from pathlib import Path
import subprocess
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / 'scripts' / 'rust-candidate-ci.py'
spec = importlib.util.spec_from_file_location('candidate_ci', SOURCE)
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)

class CandidateReceiptTests(unittest.TestCase):
    def test_node_runtime_rejects_missing_old_and_unselected_versions(self):
        ci.require_node_version('v22.23.4')
        for value in ('', 'v22.23.3', 'v22.22.0', 'v20.20.0', 'v24.22.1', '22.23.4', 'v22.23.4\nextra'):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, 'Node'):
                ci.require_node_version(value)

    def icon_api(self, resources):
        """Synthetic Win32 resource addresses, with observable loader/free calls."""
        class Call:
            def __init__(self, operation):
                self.operation = operation
            def __call__(self, *args):
                return self.operation(*args)
        class API:
            pass
        api = API()
        api.flags, api.lookups, api.freed = [], [], []
        handles = {key: index for index, key in enumerate(resources, 1)}
        buffers = {handles[key]: ctypes.create_string_buffer(data) for key, data in resources.items()}
        lengths = {handles[key]: len(data) for key, data in resources.items()}
        def load(path, unused, flags):
            api.flags.append(flags)
            return 71
        def find(module, name, kind):
            api.lookups.append((kind.value, name.value))
            return handles.get((kind.value, name.value), 0)
        api.LoadLibraryExW = Call(load)
        api.FindResourceW = Call(find)
        api.SizeofResource = Call(lambda module, found: lengths[found])
        api.LoadResource = Call(lambda module, found: found)
        api.LockResource = Call(lambda found: ctypes.addressof(buffers[found]))
        api.FreeLibrary = Call(lambda module: api.freed.append(module) or 1)
        return api

    def group_icon(self, payload, identifier=7):
        return b'\0\0\1\0\1\0' + struct.pack('<BBBBHHIH', 1, 1, 0, 0, 1, 32, len(payload), identifier)

    def test_dirty_refusal_precedes_toolchain_and_artifact_mutation(self):
        with patch.object(ci, 'capture', return_value=' M rust/src/lib.rs') as capture:
            with patch.object(sys, 'argv', ['candidate', '--target', 'x86_64-unknown-linux-gnu']):
                with self.assertRaisesRegex(SystemExit, 'clean checkout'):
                    ci.main()
        capture.assert_called_once_with(['git', 'status', '--porcelain=v1', '--untracked-files=normal'])

    def test_command_failure_preserves_output_and_actual_exit(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with patch.object(ci, 'ROOT', root), patch.object(ci, 'OUTPUT', root), patch.object(ci, 'STEPS', []):
                with self.assertRaises(subprocess.CalledProcessError) as error:
                    ci.run([sys.executable, '-c', 'print("synthetic failure");raise SystemExit(7)'], os.environ.copy(), root)
                self.assertEqual(error.exception.returncode, 7)
                receipt = json.loads((root / 'VALIDATION-STEPS.json').read_text())
                self.assertEqual(receipt[0]['exit_code'], 7)
                self.assertIn('synthetic failure', (root / receipt[0]['log']).read_text())

    def test_json_stdout_remains_separate_from_diagnostics(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with patch.object(ci, 'ROOT', root), patch.object(ci, 'OUTPUT', root), patch.object(ci, 'STEPS', []):
                code = ci.run([sys.executable, '-c', 'import sys;print("{}");print("synthetic diagnostic",file=sys.stderr)'],
                              os.environ.copy(), root, stdout_file=root / 'stdout.jsonl')
                self.assertEqual(code, 0)
                self.assertEqual(json.loads((root / 'stdout.jsonl').read_text()), {})
                self.assertIn('synthetic diagnostic', (root / ci.STEPS[0]['log']).read_text())

    def test_missing_command_has_failure_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with patch.object(ci, 'ROOT', root), patch.object(ci, 'OUTPUT', root), patch.object(ci, 'STEPS', []):
                with self.assertRaises(OSError):
                    ci.run([str(root / 'nonexistent-tool')], os.environ.copy(), root)
                self.assertEqual(json.loads((root / 'VALIDATION-STEPS.json').read_text())[0]['exit_code'], 127)

    def test_digest_rejects_directory(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(ValueError):
                ci.digest(Path(temp))

    def test_host_parser_does_not_guess_target(self):
        self.assertEqual(ci.host_from_verbose('rustc 1.90.0\nhost: aarch64-apple-darwin\n'), 'aarch64-apple-darwin')
        self.assertEqual(ci.host_from_verbose('rustc 1.90.0'), '')

    def test_release_semver_is_strict_and_stamp_uses_tag_commit_time(self):
        for tag in ('v1.0.0', 'v1.2.3-rc.1'):
            self.assertEqual(ci.release_version(tag), tag[1:])
        for tag in ('1.0.0', 'v01.0.0', 'v1.0', 'v1.0.0-01', 'v1.0.0-a..b', 'v1.0.0\n', '--help', 'v1.2.3+build.01'):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                ci.release_version(tag)
        with patch.object(ci, 'capture', side_effect=['a' * 40, '1700000000']):
            stamp = ci.build_stamp('a' * 40, 'v1.0.0')
        self.assertEqual(stamp, {'build_kind': 'release', 'release_tag': 'v1.0.0',
                                'build_commit': 'a' * 40, 'version': '1.0.0',
                                'build_time': '2023-11-14T22:13:20+00:00'})

    def test_release_tag_mismatch_precedes_toolchain_and_artifact_mutation(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            with patch.object(ci, 'ROOT', root), patch.object(ci, 'capture', side_effect=['', 'a' * 40, 'b' * 40]) as capture:
                with patch.object(sys, 'argv', ['native', '--target', 'x86_64-unknown-linux-gnu', '--release-tag', 'v1.0.0']):
                    with self.assertRaisesRegex(SystemExit, 'tag commit does not match'):
                        ci.main()
            self.assertFalse((root / '.rust-candidate-artifacts').exists())
            self.assertEqual(capture.call_count, 3)

    def test_build_metadata_tag_is_refused_before_artifacts_or_toolchain(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            with patch.object(ci, 'ROOT', root), patch.object(ci, 'capture', side_effect=['', 'a' * 40]) as capture:
                with patch.object(sys, 'argv', ['native', '--target', 'x86_64-unknown-linux-gnu', '--release-tag', 'v1.0.0+build.1']):
                    with self.assertRaisesRegex(SystemExit, 'strict SemVer'):
                        ci.main()
            self.assertEqual(capture.call_count, 2)
            self.assertFalse((root / '.rust-candidate-artifacts').exists())

    def synthetic_native(self, *, release=True, windows=False, moved_tag=False):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name).resolve()
        target = 'x86_64-pc-windows-msvc' if windows else 'x86_64-unknown-linux-gnu'
        suffix = ci.TARGETS[target]
        for relative in ('rust/Cargo.lock', 'web/dist/index.html', 'rust/assets/launcher/ui/index.html',
                         'rust/THIRD-PARTY-NOTICES.md', 'rust/src/approval/policy/go_regex/GO-LICENSE'):
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'synthetic source input')
        icon_payload = b'synthetic-ico-payload'
        icon = root / 'assets/many-ai-cli.ico'
        icon.parent.mkdir()
        icon.write_bytes(b'\0\0\1\0\1\0' + bytes([1, 1, 0, 0]) + struct.pack('<HHII', 1, 32, len(icon_payload), 22) + icon_payload)
        for stem in ('many-ai-cli', 'many-ai-cli-launcher'):
            binary = root / 'rust/target' / target / 'release' / (stem + ('.exe' if windows else ''))
            binary.parent.mkdir(parents=True, exist_ok=True)
            binary.write_bytes(b'synthetic-native-' + stem.encode() + icon_payload)
        calls, tag_reads = [], []
        def capture(args, cwd=None):
            if args[:2] == ['git', 'status']:
                return ''
            if args[:3] == ['git', 'rev-parse', '--verify']:
                tag_reads.append(args)
                return ('b' if moved_tag and len(tag_reads) > 1 else 'a') * 40
            if args[:2] == ['git', 'rev-parse']:
                return 'a' * 40
            if args[:2] == ['git', 'show']:
                return '1700000000'
            if args[0] == 'rustc':
                return f'rustc 1.90.0 synthetic\nhost: {target}'
            if args == ['go', 'version']:
                return 'go version go1.26.8 synthetic'
            if args == ['bun', '--version']:
                return '1.3.14'
            if args == ['node', '--version']:
                return 'v22.23.4'
            if args[:2] == ['go', 'env']:
                return str(root / 'synthetic-cache')
            raise AssertionError(f'unexpected mocked command: {args}')
        def run(args, env, cwd=None, **kwargs):
            calls.append((args, {key: env.get(key) for key in ('MANY_AI_BUILD_VERSION', 'MANY_AI_BUILD_COMMIT',
                          'MANY_AI_BUILD_TIME', 'MANY_AI_BUILD_KIND', 'MANY_AI_WINDOWS_RESOURCE_DIR')}))
            if '/fo' in args:
                Path(args[args.index('/fo') + 1]).write_bytes(b'synthetic SDK resource')
            if 'collect_inputs.py' in str(args[1] if len(args) > 1 else ''):
                folder = Path(args[args.index('--output') + 1])
                folder.mkdir()
                for name in ci.PACKAGING_INPUTS:
                    (folder / name).write_bytes(b'synthetic packaging evidence')
            if kwargs.get('stdout_file'):
                kwargs['stdout_file'].write_text('{}\n')
            return 0
        argv = ['native', '--target', target] + (['--release-tag', 'v1.0.0'] if release else [])
        api = self.icon_api({(14, 1): self.group_icon(icon_payload), (3, 7): icon_payload})
        with patch.object(ci, 'ROOT', root), patch.object(ci, 'capture', side_effect=capture), patch.object(ci, 'run', side_effect=run):
            with patch.object(ci.shutil, 'which', return_value='synthetic-rc.exe'):
                with patch.object(ci, 'windows_file_version', side_effect=lambda path: {'FileVersion': '1.0.0', 'ProductVersion': '1.0.0', 'OriginalFilename': path.name}):
                    with patch.object(ci, 'windows_runtime_receipt', return_value={'source_sha': 'a' * 40, 'embedded_in': 'many-ai-cli.exe', 'embedded_bytes_verified': True}):
                        with patch.dict(os.environ, {'MANY_AI_REVIEW_HEAD_SHA': 'a' * 40,
                                                     'CARGO_TARGET_DIR': str(root / 'rust/target')}), patch.object(sys, 'argv', argv), patch.object(ctypes, 'WinDLL', return_value=api, create=True):
                            if moved_tag:
                                with self.assertRaisesRegex(ValueError, 'tag commit does not match'):
                                    ci.main()
                            else:
                                ci.main()
        return root / '.rust-candidate-artifacts' / suffix, calls

    def test_release_native_mock_stamps_before_every_compile_and_binds_evidence(self):
        output, calls = self.synthetic_native()
        receipt = json.loads((output / 'BUILD-RECEIPT.json').read_text())
        self.assertEqual(receipt['status'], 'native-release-build-not-accepted-for-publication')
        self.assertEqual(receipt['release_tag'], 'v1.0.0')
        self.assertEqual(receipt['build_commit'], 'a' * 40)
        self.assertEqual(receipt['node'], 'v22.23.4')
        for args, env in calls:
            if args[0] == 'cargo':
                self.assertEqual(env['MANY_AI_BUILD_VERSION'], '1.0.0')
                self.assertEqual(env['MANY_AI_BUILD_COMMIT'], 'a' * 40)
                self.assertEqual(env['MANY_AI_BUILD_TIME'], '2023-11-14T22:13:20+00:00')
        self.assertEqual(len(receipt['packaging_inputs']), 3)
        self.assertTrue((output / 'RELEASE-EVIDENCE.txt').is_file())
        self.assertFalse((output / 'NOT-FOR-PUBLICATION.txt').exists())
        self.assertFalse((output / 'NOT-FOR-CUTOVER.txt').exists())

    def test_candidate_default_still_uses_candidate_stamp_and_warning(self):
        output, _ = self.synthetic_native(release=False)
        receipt = json.loads((output / 'BUILD-RECEIPT.json').read_text())
        self.assertEqual(receipt['version'], '0.0.0-rust-candidate+' + 'a' * 12)
        self.assertEqual(receipt['build_kind'], 'candidate')
        self.assertIsNone(receipt['release_tag'])
        self.assertTrue((output / 'NOT-FOR-CUTOVER.txt').exists())

    def test_tag_move_during_preparation_blocks_all_cargo_commands(self):
        _, calls = self.synthetic_native(moved_tag=True)
        self.assertFalse(any(args[0] == 'cargo' for args, _ in calls))

    def test_windows_resource_formatter_and_mocked_native_readback(self):
        script = ci.windows_resource_script('1.2.3-rc.1', Path('synthetic/icon.ico'), 'many-ai-cli-launcher.exe')
        self.assertIn('FILEVERSION 1,2,3,0', script)
        self.assertIn('PRODUCTVERSION 1,2,3,0', script)
        self.assertIn('FILEFLAGS 2', script)
        self.assertIn('VALUE "ProductVersion", "1.2.3-rc.1\\0"', script)
        self.assertIn('1 ICON "synthetic/icon.ico"', script)
        with self.assertRaises(ValueError):
            ci.windows_resource_script('65536.0.0', Path('icon.ico'), 'many-ai-cli.exe')
        output, calls = self.synthetic_native(windows=True)
        receipt = json.loads((output / 'BUILD-RECEIPT.json').read_text())
        self.assertEqual(len(receipt['windows_resources']['binaries']), 2)
        self.assertTrue(all(entry['icon_payloads_verified'] for entry in receipt['windows_resources']['binaries']))
        self.assertEqual(sum('/fo' in args for args, _ in calls), 2)
        self.assertTrue(all(env['MANY_AI_WINDOWS_RESOURCE_DIR'] for args, env in calls if args[0] == 'cargo'))

    def test_windows_resource_readback_rejects_changed_version_or_missing_icon(self):
        output, _ = self.synthetic_native(windows=True)
        receipt = json.loads((output / 'BUILD-RECEIPT.json').read_text())
        root = output.parents[1]
        def observed(path):
            return {'FileVersion': '1.0.0', 'ProductVersion': '1.0.0', 'OriginalFilename': path.name}
        with patch.object(ci, 'ROOT', root):
            with patch.object(ci, 'windows_file_version', return_value={'FileVersion': '0.0.0', 'ProductVersion': '1.0.0', 'OriginalFilename': 'many-ai-cli.exe'}):
                with self.assertRaisesRegex(ValueError, 'product version/icon readback'):
                    ci.verify_windows_resources(output, '1.0.0', receipt['windows_resources'])
            (output / 'many-ai-cli.exe').write_bytes(b'nonempty synthetic binary without icon')
            missing = self.icon_api({(14, 1): self.group_icon(b'missing referenced icon')})
            with patch.object(ci, 'windows_file_version', side_effect=observed), patch.object(ctypes, 'WinDLL', return_value=missing, create=True):
                with self.assertRaisesRegex(ValueError, 'group/icon resource is missing'):
                    ci.verify_windows_resources(output, '1.0.0', receipt['windows_resources'])

    def test_sdk_zero_ico_planes_normalization_accepts_only_exact_or_one(self):
        output, _ = self.synthetic_native(windows=True)
        receipt = json.loads((output / 'BUILD-RECEIPT.json').read_text())
        root = output.parents[1]
        icon = root / 'assets/many-ai-cli.ico'
        source = bytearray(icon.read_bytes())
        struct.pack_into('<H', source, 10, 0)
        icon.write_bytes(source)
        prepared = {**receipt['windows_resources'], 'icon_sha256': ci.digest(icon)}
        payload = b'synthetic-ico-payload'
        observed = lambda path: {'FileVersion': '1.0.0', 'ProductVersion': '1.0.0', 'OriginalFilename': path.name}
        for actual_planes in (0, 1):
            group = bytearray(self.group_icon(payload))
            struct.pack_into('<H', group, 10, actual_planes)
            api = self.icon_api({(14, 1): bytes(group), (3, 7): payload})
            with self.subTest(actual_planes=actual_planes), patch.object(ci, 'ROOT', root):
                with patch.object(ci, 'windows_file_version', side_effect=observed), patch.object(ctypes, 'WinDLL', return_value=api, create=True):
                    result = ci.verify_windows_resources(output, '1.0.0', prepared)
                self.assertEqual(len(result['binaries']), 2)
                self.assertTrue(all(item['icon_payloads_verified'] for item in result['binaries']))
                self.assertEqual(api.lookups, [(14, 1), (3, 7), (14, 1), (3, 7)])
                self.assertEqual(api.freed, [71, 71])

    def test_sdk_planes_exception_preserves_header_count_and_payload_rejections(self):
        output, _ = self.synthetic_native(windows=True)
        receipt = json.loads((output / 'BUILD-RECEIPT.json').read_text())
        root = output.parents[1]
        icon = root / 'assets/many-ai-cli.ico'
        source = bytearray(icon.read_bytes())
        struct.pack_into('<H', source, 10, 0)
        icon.write_bytes(source)
        prepared = {**receipt['windows_resources'], 'icon_sha256': ci.digest(icon)}
        payload = b'synthetic-ico-payload'
        observed = lambda path: {'FileVersion': '1.0.0', 'ProductVersion': '1.0.0', 'OriginalFilename': path.name}
        cases = []
        for label, offset, fmt, value in (('width', 6, '<B', 2), ('height', 7, '<B', 2),
                                          ('colors', 8, '<B', 1), ('reserved', 9, '<B', 1),
                                          ('planes', 10, '<H', 2), ('bit_depth', 12, '<H', 24),
                                          ('size', 14, '<I', len(payload) + 1), ('missing_id', 18, '<H', 8)):
            group = bytearray(self.group_icon(payload))
            struct.pack_into(fmt, group, offset, value)
            cases.append((label, bytes(group), payload))
        group = self.group_icon(payload)
        cases.extend([('payload', group, b'X' + payload[1:]),
                      ('extra_image', group[:4] + struct.pack('<H', 2) + group[6:] * 2, payload),
                      ('zero_count', group[:4] + b'\0\0', payload),
                      ('truncated_group', group[:-1], payload)])
        for label, group, actual_payload in cases:
            api = self.icon_api({(14, 1): group, (3, 7): actual_payload})
            with self.subTest(case=label), patch.object(ci, 'ROOT', root):
                with patch.object(ci, 'windows_file_version', side_effect=observed), patch.object(ctypes, 'WinDLL', return_value=api, create=True):
                    with self.assertRaises(ValueError):
                        ci.verify_windows_resources(output, '1.0.0', prepared)
            self.assertEqual(api.freed, [71])
        # A nonzero source value has no normalization exception, even if SDK
        # supplies one; actual arbitrary values are never normalized away.
        struct.pack_into('<H', source, 10, 2)
        icon.write_bytes(source)
        prepared['icon_sha256'] = ci.digest(icon)
        api = self.icon_api({(14, 1): self.group_icon(payload), (3, 7): payload})
        with patch.object(ci, 'ROOT', root), patch.object(ci, 'windows_file_version', side_effect=observed):
            with patch.object(ctypes, 'WinDLL', return_value=api, create=True):
                with self.assertRaisesRegex(ValueError, 'product version/icon readback'):
                    ci.verify_windows_resources(output, '1.0.0', prepared)

    def test_icon_reader_follows_group_ids_and_always_frees_resource_only_module(self):
        payload = b'synthetic referenced icon'
        api = self.icon_api({(14, 1): self.group_icon(payload), (3, 7): payload})
        with patch.object(ctypes, 'WinDLL', return_value=api, create=True):
            result = ci.windows_icon_resources(Path('synthetic.exe'))
        self.assertEqual(result, [((1, 1, 0, 0, 1, 32, len(payload)), payload)])
        self.assertEqual(api.lookups, [(14, 1), (3, 7)])
        self.assertEqual(api.flags, [0x22])
        self.assertEqual(api.freed, [71])
        missing = self.icon_api({(14, 1): self.group_icon(payload, identifier=8), (3, 7): payload})
        with patch.object(ctypes, 'WinDLL', return_value=missing, create=True):
            with self.assertRaisesRegex(ValueError, 'group/icon resource is missing'):
                ci.windows_icon_resources(Path('synthetic.exe'))
        self.assertEqual(missing.lookups, [(14, 1), (3, 8)])
        self.assertEqual(missing.freed, [71])

    def test_raw_icon_asset_without_actual_group_resource_cannot_pass(self):
        output, _ = self.synthetic_native(windows=True)
        receipt = json.loads((output / 'BUILD-RECEIPT.json').read_text())
        # Both files still contain the ICO image bytes as arbitrary raw assets.
        self.assertIn(b'synthetic-ico-payload', (output / 'many-ai-cli.exe').read_bytes())
        self.assertIn(b'synthetic-ico-payload', (output / 'many-ai-cli-launcher.exe').read_bytes())
        api = self.icon_api({})
        observed = lambda path: {'FileVersion': '1.0.0', 'ProductVersion': '1.0.0', 'OriginalFilename': path.name}
        with patch.object(ci, 'ROOT', output.parents[1]), patch.object(ci, 'windows_file_version', side_effect=observed):
            with patch.object(ctypes, 'WinDLL', return_value=api, create=True):
                with self.assertRaisesRegex(ValueError, 'group/icon resource is missing'):
                    ci.verify_windows_resources(output, '1.0.0', receipt['windows_resources'])
        self.assertEqual(api.lookups, [(14, 1)])
        self.assertEqual(api.freed, [71])

if __name__ == '__main__':
    unittest.main()
