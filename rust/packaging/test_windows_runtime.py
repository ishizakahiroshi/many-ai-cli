"""Synthetic runtime receipt/embedded-byte checks; no DLL download or execution."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / 'scripts/rust-candidate-ci.py'
spec = importlib.util.spec_from_file_location('runtime_candidate_ci', SOURCE)
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)


class WindowsRuntimeReceiptTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.patcher = patch.object(ci, 'ROOT', self.root)
        self.patcher.start()
        self.addCleanup(self.patcher.stop)
        self.source = 'a' * 40
        script = self.root / 'internal/whisperruntime/fetch_windows_runtime.ps1'
        script.parent.mkdir(parents=True)
        script.write_bytes(b'synthetic acquisition script')
        self.payload = script.parent / 'files/windows-amd64'
        self.payload.mkdir(parents=True)
        self.receipt = {
            'schema_version': 1, 'source_sha': self.source, 'target': 'x86_64-pc-windows-msvc',
            'acquisition': 'visual-studio-redist', 'source_script': script.relative_to(self.root).as_posix(),
            'source_script_sha256': ci.digest(script), 'files': [],
        }
        self.binary = self.root / 'many-ai-cli.exe'
        embedded = bytearray(b'synthetic-main')
        for index, name in enumerate(ci.WINDOWS_RUNTIME_NAMES):
            data = bytearray(71)
            data[:2] = b'MZ'
            data[0x3c:0x40] = (64).to_bytes(4, 'little')
            data[64:70] = b'PE\0\0\x64\x86'
            data[70] = index
            (self.payload / name).write_bytes(data)
            embedded.extend(data)
            self.receipt['files'].append({
                'name': name, 'file_version': 'synthetic.1', 'bytes': len(data),
                'sha256': hashlib.sha256(data).hexdigest(), 'signature_status': 'Valid',
                'microsoft_signer': True, 'pe_machine': '0x8664',
            })
        self.binary.write_bytes(embedded)
        self.path = self.root / 'WINDOWS-RUNTIME.json'

    def read(self, receipt=None, binary=None):
        self.path.write_text(json.dumps(receipt or self.receipt), encoding='utf-8')
        return ci.windows_runtime_receipt(self.path, self.source, binary=binary)

    def test_all_four_version_hashes_bind_to_retained_binary_bytes(self):
        actual = self.read(binary=self.binary)
        self.assertEqual(actual['files'], self.receipt['files'])
        self.assertTrue(actual['embedded_bytes_verified'])
        self.assertEqual(actual['embedded_in'], 'many-ai-cli.exe')

    def test_missing_duplicate_and_changed_dlls_fail(self):
        for entries in [self.receipt['files'][:3], [self.receipt['files'][0]] * 4]:
            altered = {**self.receipt, 'files': entries}
            with self.assertRaisesRegex(ValueError, 'exactly four'):
                self.read(altered)
        (self.payload / ci.WINDOWS_RUNTIME_NAMES[0]).write_bytes(b'tampered')
        with self.assertRaisesRegex(ValueError, 'identity mismatch'):
            self.read()

    def test_source_signature_version_and_architecture_fail_closed(self):
        for field, value in [('signature_status', 'NotSigned'), ('microsoft_signer', False),
                             ('file_version', ''), ('pe_machine', '0x14c')]:
            altered = copy.deepcopy(self.receipt)
            altered['files'][0][field] = value
            with self.assertRaises(ValueError):
                self.read(altered)
        for field, value in [('source_sha', 'b' * 40), ('acquisition', 'system32'),
                             ('source_script_sha256', '0' * 64)]:
            with self.assertRaisesRegex(ValueError, 'provenance mismatch'):
                self.read({**self.receipt, field: value})

    def test_receipt_alone_does_not_prove_linked_retention(self):
        self.binary.write_bytes(b'no runtime data')
        with self.assertRaisesRegex(ValueError, 'absent from main binary'):
            self.read(binary=self.binary)

    def test_matching_hash_cannot_disguise_non_pe_payload(self):
        altered = copy.deepcopy(self.receipt)
        name = ci.WINDOWS_RUNTIME_NAMES[0]
        data = b'synthetic non-PE input'
        (self.payload / name).write_bytes(data)
        altered['files'][0].update(bytes=len(data), sha256=hashlib.sha256(data).hexdigest())
        with self.assertRaisesRegex(ValueError, 'must be x64 PE'):
            self.read(altered)


if __name__ == '__main__':
    unittest.main()
