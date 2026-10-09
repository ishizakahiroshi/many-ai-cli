"""Synthetic candidate packaging checks; no builds or network."""
import importlib.util
import json
from pathlib import Path
import subprocess
import shutil
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location("package_candidates", Path(__file__).resolve().parents[2] / "scripts/package-rust-candidates.py")
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class PackageCandidateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.inputs = self.root / "inputs"
        self.output = self.root / "output"
        self.source = "a" * 40
        self.receipts = {}
        for target, (suffix, goos, _) in package.TARGETS.items():
            folder = self.inputs / suffix
            folder.mkdir(parents=True)
            binaries, notices = [], []
            for stem in ("many-ai-cli", "many-ai-cli-launcher"):
                name = stem + (".exe" if goos == "windows" else "")
                data = f"synthetic {suffix} {stem}".encode()
                (folder / name).write_bytes(data)
                binaries.append({"name": name, "sha256": package.sha(data), "bytes": len(data)})
            for name in package.NOTICES:
                data = f"synthetic notice {name}".encode()
                (folder / name).write_bytes(data)
                notices.append({"name": name, "sha256": package.sha(data)})
            receipt = {
                "schema_version": 2, "status": "native-build-only-not-accepted-for-cutover",
                "source_sha": self.source, "review_head_sha": self.source,
                "version": f"0.0.0-rust-candidate+{self.source[:12]}", "target": target,
                "target_name": suffix, "packaging_input_exit_code": 0,
                "cargo_lock_sha256": "b" * 64,
                "generated_web_asset_inputs": [{"path": "index.html", "sha256": "c" * 64}],
                "binaries": binaries, "third_party_notices": notices,
                "windows_runtime": {"source_sha": self.source, "embedded_in": "many-ai-cli.exe",
                                    "embedded_bytes_verified": True} if goos == "windows" else None,
            }
            self.receipts[suffix] = receipt
            self.write_receipt(suffix)

    def write_receipt(self, suffix):
        (self.inputs / suffix / "BUILD-RECEIPT.json").write_text(json.dumps(self.receipts[suffix]), encoding="utf-8")

    def assemble(self):
        docs = {name: f"synthetic source doc {name}".encode() for name in (*package.DOCS, "unblock-windows.cmd")}
        with patch.object(package, "source_documents", return_value=docs):
            package.assemble(self.inputs, self.output, self.root)

    def test_archives_and_npm_inputs_share_verified_bytes(self):
        self.assemble()
        self.assertEqual(len(json.loads((self.output / "artifacts.json").read_text())), 8)
        archives = list(self.output.glob("*.zip"))
        self.assertEqual(len(archives), 4)
        for target, (suffix, goos, _) in package.TARGETS.items():
            path = next(p for p in archives if p.name.endswith(f"-{suffix}.zip"))
            with zipfile.ZipFile(path) as archive:
                self.assertIn("NOT-FOR-CUTOVER.txt", archive.namelist())
                self.assertIn("BUILD-RECEIPT.json", archive.namelist())
                for entry in self.receipts[suffix]["binaries"]:
                    self.assertEqual(archive.read(entry["name"]), (self.output / "binaries" / suffix / entry["name"]).read_bytes())
                    if goos != "windows":
                        self.assertEqual((archive.getinfo(entry["name"]).external_attr >> 16) & 0o777, 0o755)
            checksum = next(line.split()[0] for line in (self.output / "SHA256SUMS.txt").read_text().splitlines() if line.endswith(path.name))
            self.assertEqual(checksum, package.sha(path.read_bytes()))
        self.assertEqual(json.loads((self.output / "PACKAGE-RECEIPT.json").read_text())["status"], "candidate-packaged-not-accepted-for-release")

    def test_mixed_source_or_web_inputs_refused_before_output(self):
        for field, value in (("source_sha", "d" * 40), ("generated_web_asset_inputs", [{"path": "index.html", "sha256": "d" * 64}])):
            original = self.receipts["linux-x64"][field]
            self.receipts["linux-x64"][field] = value
            self.write_receipt("linux-x64")
            with self.assertRaises(ValueError):
                self.assemble()
            self.assertFalse(self.output.exists())
            self.receipts["linux-x64"][field] = original

    def test_altered_or_missing_binary_launcher_and_notice_refused(self):
        for name in ("many-ai-cli", "many-ai-cli-launcher", "GO-LICENSE"):
            path = self.inputs / "linux-x64" / name
            original = path.read_bytes()
            path.write_bytes(b"altered")
            with self.assertRaisesRegex(ValueError, "hash/size mismatch"):
                self.assemble()
            path.unlink()
            with self.assertRaises(ValueError):
                self.assemble()
            path.write_bytes(original)
            self.assertFalse(self.output.exists())

    def test_missing_runtime_failed_inputs_and_release_version_refused(self):
        for field, value in (("windows_runtime", None), ("packaging_input_exit_code", 1), ("version", "1.0.0")):
            original = self.receipts["windows-x64"][field]
            self.receipts["windows-x64"][field] = value
            self.write_receipt("windows-x64")
            with self.assertRaises(ValueError):
                self.assemble()
            self.assertFalse(self.output.exists())
            self.receipts["windows-x64"][field] = original

    def test_existing_output_preserved(self):
        self.output.mkdir()
        sentinel = self.output / "keep.txt"
        sentinel.write_text("preserve")
        with self.assertRaisesRegex(ValueError, "output must be new"):
            self.assemble()
        self.assertEqual(sentinel.read_text(), "preserve")

    def test_source_documents_read_receipt_commit(self):
        with patch.object(package.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, b"source")) as run:
            package.source_documents(self.root, self.source)
        for call in run.call_args_list:
            self.assertEqual(call.args[0][-2], "show")
            self.assertTrue(call.args[0][-1].startswith(self.source + ":"))

    @unittest.skipUnless(shutil.which("node"), "Node needed for npm staging integration")
    def test_existing_npm_stager_consumes_manifest_and_preserves_bytes(self):
        self.assemble()
        scripts = self.root / "scripts"
        scripts.mkdir()
        repo = Path(__file__).resolve().parents[2]
        stager = scripts / "stage-npm-binaries.mjs"
        shutil.copyfile(repo / "scripts/stage-npm-binaries.mjs", stager)
        subprocess.run(["node", str(stager), str(self.output)], check=True,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        for _, (suffix, goos, _) in package.TARGETS.items():
            name = "many-ai-cli" + (".exe" if goos == "windows" else "")
            staged = self.root / "npm" / f"many-ai-cli-{suffix}" / "bin" / name
            self.assertEqual(staged.read_bytes(), (self.output / "binaries" / suffix / name).read_bytes())


if __name__ == "__main__":
    unittest.main()
