"""Synthetic candidate packaging checks; no builds or network."""
import copy
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import shutil
import sys
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
                "launcher_ui_input_sha256": "e" * 64,
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
        manifest = json.loads((self.output / "artifacts.json").read_text())
        self.assertEqual(len(manifest), 8)
        for entry in manifest:
            self.assertEqual(entry["pathBase"], "manifest")
            self.assertFalse(Path(entry["path"]).is_absolute())
            self.assertEqual((self.output / entry["path"]).name, entry["name"])
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
        original = copy.deepcopy(self.receipts["linux-x64"])
        for changes in (
                {"source_sha": "d" * 40, "review_head_sha": "d" * 40,
                 "version": "0.0.0-rust-candidate+" + "d" * 12},
                {"cargo_lock_sha256": "d" * 64},
                {"generated_web_asset_inputs": [{"path": "index.html", "sha256": "d" * 64}]}):
            self.receipts["linux-x64"] = {**original, **changes}
            self.write_receipt("linux-x64")
            with self.assertRaisesRegex(ValueError, "mixed source/version/lock/Web"):
                self.assemble()
            self.assertFalse(self.output.exists())

    def test_mixed_launcher_ui_inputs_refused_before_output(self):
        self.receipts["linux-x64"]["launcher_ui_input_sha256"] = "f" * 64
        self.write_receipt("linux-x64")
        with self.assertRaisesRegex(ValueError, "launcher-UI"):
            self.assemble()
        self.assertFalse(self.output.exists())

    def test_malformed_receipt_types_are_validation_errors_before_output(self):
        original = copy.deepcopy(self.receipts["linux-x64"])
        mutations = (
            ("schema_version", 2.0), ("source_sha", None), ("source_sha", 123),
            ("cargo_lock_sha256", []), ("launcher_ui_input_sha256", None),
            ("launcher_ui_input_sha256", 123), ("packaging_input_exit_code", False),
            ("generated_web_asset_inputs", [{"path": "index.html", "sha256": 123}]),
            ("generated_web_asset_inputs", [{"path": [], "sha256": "c" * 64}]),
            ("generated_web_asset_inputs", [{"path": "", "sha256": "c" * 64}]),
            ("binaries", [{"name": [], "bytes": 2, "sha256": "c" * 64}]),
            ("binaries", [None]),
        )
        for field, value in mutations:
            with self.subTest(field=field, value=value):
                self.receipts["linux-x64"] = {**original, field: value}
                self.write_receipt("linux-x64")
                with self.assertRaises(ValueError):
                    self.assemble()
                self.assertFalse(self.output.exists())
        for field, value in (("bytes", True), ("bytes", "4"), ("bytes", 0), ("sha256", 123)):
            with self.subTest(binary_field=field, value=value):
                self.receipts["linux-x64"] = copy.deepcopy(original)
                self.receipts["linux-x64"]["binaries"][0][field] = value
                self.write_receipt("linux-x64")
                with self.assertRaises(ValueError):
                    self.assemble()
                self.assertFalse(self.output.exists())
        for receipt in (None, [], "synthetic"):
            with self.subTest(root=receipt):
                self.receipts["linux-x64"] = receipt
                self.write_receipt("linux-x64")
                with self.assertRaises(ValueError):
                    self.assemble()
                self.assertFalse(self.output.exists())

    def test_cli_malformed_receipt_has_clean_validation_error(self):
        self.receipts["windows-x64"] = []
        self.write_receipt("windows-x64")
        result = subprocess.run([sys.executable, str(Path(package.__file__)),
                                 "--inputs", str(self.inputs), "--output", str(self.output)],
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 2)
        self.assertIn("candidate assembly failed", result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertFalse(self.output.exists())

    def test_missing_target_and_duplicate_binary_refused_before_output(self):
        launcher = self.inputs / "macos-intel" / "BUILD-RECEIPT.json"
        original = launcher.read_bytes()
        launcher.unlink()
        with self.assertRaises(ValueError):
            self.assemble()
        self.assertFalse(self.output.exists())
        launcher.write_bytes(original)
        self.receipts["linux-x64"]["binaries"][1] = self.receipts["linux-x64"]["binaries"][0]
        self.write_receipt("linux-x64")
        with self.assertRaisesRegex(ValueError, "exactly the expected distinct files"):
            self.assemble()
        self.assertFalse(self.output.exists())

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
        stager = self.copy_stager()
        self.run_stager(stager, self.output)
        self.assert_npm_bytes(self.output)

    def copy_stager(self):
        scripts = self.root / "scripts"
        scripts.mkdir(exist_ok=True)
        repo = Path(__file__).resolve().parents[2]
        stager = scripts / "stage-npm-binaries.mjs"
        shutil.copyfile(repo / "scripts/stage-npm-binaries.mjs", stager)
        return stager

    def run_stager(self, stager, bundle):
        result = subprocess.run(["node", str(stager), str(bundle)],
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def assert_npm_bytes(self, bundle):
        for _, (suffix, goos, _) in package.TARGETS.items():
            name = "many-ai-cli" + (".exe" if goos == "windows" else "")
            staged = self.root / "npm" / f"many-ai-cli-{suffix}" / "bin" / name
            self.assertEqual(staged.read_bytes(), (bundle / "binaries" / suffix / name).read_bytes())
            archive_path = next(bundle.glob(f"*-{suffix}.zip"))
            with zipfile.ZipFile(archive_path) as archive:
                self.assertEqual(staged.read_bytes(), archive.read(name))
            self.assertEqual([path.name for path in staged.parent.iterdir()], [name])

    @unittest.skipUnless(shutil.which("node"), "Node needed for npm staging integration")
    def test_moved_bundle_stages_all_main_binaries_without_original_output(self):
        self.assemble()
        stager = self.copy_stager()
        moved = self.root / "downloaded" / "candidate-bundle"
        shutil.copytree(self.output, moved)
        # Recursive deletion is restricted to this fixture's known temporary root.
        self.assertEqual(self.output.resolve().parent, self.root)
        shutil.rmtree(self.output)
        self.assertFalse(self.output.exists())
        self.run_stager(stager, moved)
        self.assert_npm_bytes(moved)

    @unittest.skipUnless(shutil.which("node"), "Node needed for npm staging integration")
    def test_stager_keeps_absolute_and_legacy_repo_relative_manifest_paths(self):
        self.assemble()
        stager = self.copy_stager()
        path = self.output / "artifacts.json"
        original = json.loads(path.read_text())
        for mode in ("absolute", "repo-relative"):
            with self.subTest(mode=mode):
                entries = copy.deepcopy(original)
                for entry in entries:
                    absolute = self.output / entry["path"]
                    entry["path"] = (str(absolute) if mode == "absolute"
                                     else absolute.relative_to(self.root).as_posix())
                    del entry["pathBase"]
                path.write_text(json.dumps(entries), encoding="utf-8")
                self.run_stager(stager, self.output)
                self.assert_npm_bytes(self.output)

    @unittest.skipUnless(shutil.which("node"), "Node needed for npm staging integration")
    def test_stager_unknown_path_base_fails_before_changing_outputs(self):
        self.assemble()
        stager = self.copy_stager()
        self.run_stager(stager, self.output)
        path = self.output / "artifacts.json"
        entries = json.loads(path.read_text())
        entries[0]["pathBase"] = "unknown"
        path.write_text(json.dumps(entries), encoding="utf-8")
        result = subprocess.run(["node", str(stager), str(self.output)],
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 1)
        self.assertIn("unknown artifact pathBase", result.stderr)
        self.assertIn("npm outputs were not changed", result.stderr)
        self.assert_npm_bytes(self.output)


class CandidateAssemblyWorkflowTests(unittest.TestCase):
    def test_native_matrix_and_assembly_remain_manual_and_attempt_scoped(self):
        repo = Path(__file__).resolve().parents[2]
        workflow = (repo / ".github/workflows/rust-migration.yml").read_text(encoding="utf-8")
        triggers = workflow.split("\non:\n", 1)[1].split("\npermissions:", 1)[0]
        self.assertEqual([line.strip() for line in triggers.splitlines()
                          if line.strip() and not line.lstrip().startswith("#")], ["workflow_dispatch:"])
        native, assemble = workflow.split("\n  assemble:\n", 1)
        self.assertEqual(set(re.findall(r"^\s+target: (\S+)$", native, re.MULTILINE)), set(package.TARGETS))
        self.assertIn("\n    needs: native\n", assemble)
        source = "${{ github.event.pull_request.head.sha || github.sha }}"
        self.assertIn(f"\n          ref: {source}\n", native)
        self.assertIn(f"\n          ref: {source}\n", assemble)
        self.assertIn("\n          persist-credentials: false\n", assemble)
        self.assertIn("python-version: '3.12'", assemble)
        pattern = ("rust-candidate-{" + ",".join(package.TARGETS) + "}-" + source
                   + "-${{ github.run_id }}-attempt-${{ github.run_attempt }}")
        self.assertIn(f"\n          pattern: {pattern}\n", assemble)
        self.assertIn("actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093", assemble)
        self.assertIn("\n          merge-multiple: true\n", assemble)
        self.assertIn("--inputs .rust-candidate-inputs --output .rust-candidate-packages", assemble)
        self.assertIn("actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02", assemble)
        self.assertIn("\n          path: .rust-candidate-packages/\n", assemble)
        self.assertIn("\n          if-no-files-found: error\n", assemble)
        self.assertIn("\n          include-hidden-files: true\n", assemble)
        self.assertNotIn("secrets.", assemble)
        self.assertNotRegex(assemble, r"contents:\s*write|id-token:\s*write|gh release|npm publish|cargo build")


if __name__ == "__main__":
    unittest.main()
