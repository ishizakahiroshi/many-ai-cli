"""Candidate identity/failure receipts; no toolchain or network needed."""
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / 'scripts' / 'rust-candidate-ci.py'
spec = importlib.util.spec_from_file_location('candidate_ci', SOURCE)
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)

class CandidateReceiptTests(unittest.TestCase):
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

    def test_stdio_diagnostic_report_is_optional_and_echoed_as_json(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with patch.object(sys, 'stdout', new_callable=io.StringIO) as output:
                ci.emit_windows_stdio_diagnostics(root)
                self.assertEqual(output.getvalue(), '')
                report = root / 'test-diagnostics' / 'windows-appserver-stdio.json'
                report.parent.mkdir()
                value = {'schema_version': 1, 'cases': []}
                report.write_text(json.dumps(value), encoding='utf-8')
                ci.emit_windows_stdio_diagnostics(root)
                self.assertEqual(json.loads(output.getvalue().split(': ', 1)[1]), value)

    def test_stdio_diagnostic_report_rejects_oversize_and_wrong_schema(self):
        with tempfile.TemporaryDirectory() as temp:
            report = Path(temp) / 'test-diagnostics' / 'windows-appserver-stdio.json'
            report.parent.mkdir()
            report.write_bytes(b' ' * 65537)
            with self.assertRaisesRegex(ValueError, 'invalid Windows'):
                ci.emit_windows_stdio_diagnostics(Path(temp))
            for value in ([], {'schema_version': 2}):
                report.write_text(json.dumps(value), encoding='utf-8')
                with self.assertRaisesRegex(ValueError, 'unsupported Windows'):
                    ci.emit_windows_stdio_diagnostics(Path(temp))

if __name__ == '__main__':
    unittest.main()
