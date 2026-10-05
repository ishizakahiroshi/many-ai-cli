"""Synthetic isolated cache/build receipts. No Cargo, network or real installations."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import subprocess
import sys

SPEC = importlib.util.spec_from_file_location("collect_inputs", Path(__file__).with_name("collect_inputs.py"))
collector = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(collector)


class InputEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.cache = self.root / "registry/cache/test-registry"
        self.cache.mkdir(parents=True)
        self.source = self.root / "registry/src/test-registry/example-1.0.0"
        self.source.mkdir(parents=True)
        self.members = {
            "Cargo.toml": b'[package]\nname="example"\nversion="1.0.0"\nlicense="MIT OR Apache-2.0"\n',
            "src/lib.rs": b'pub fn example() {}\n',
            "LICENSE-MIT": b'Synthetic MIT evidence text\n',
            "native/vendor/NOTICE": b'Synthetic bundled native notice\n',
        }
        self.package = {"name": "example", "version": "1.0.0", "source": collector.REGISTRY}
        self.archive()
        self.root_id = "path+file:///synthetic/repo/rust#many-ai-cli@0.0.0"
        self.dep_id = collector.REGISTRY + "#example@1.0.0"
        self.metadata = {"resolve": {"root": self.root_id}, "packages": [
            {"id": self.root_id, "name": "many-ai-cli", "version": "0.0.0", "source": None},
            {"id": self.dep_id, "name": "example", "version": "1.0.0", "source": collector.REGISTRY,
             "manifest_path": str(self.source / "Cargo.toml")},
        ]}
        self.messages = [self.artifact(self.dep_id, "example", "lib"),
                         self.artifact(self.dep_id, "build-script-build", "custom-build"),
                         {"reason": "build-script-executed", "package_id": self.dep_id},
                         self.artifact(self.root_id, "many-ai-cli", "bin"),
                         self.artifact(self.root_id, "many-ai-cli-launcher", "bin"),
                         {"reason": "build-finished", "success": True}]

    def archive(self, extra=None):
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w:gz") as tar:
            for name, data in self.members.items():
                member = tarfile.TarInfo("example-1.0.0/" + name)
                member.size = len(data)
                tar.addfile(member, io.BytesIO(data))
                path = self.source / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            if extra:
                tar.addfile(extra)
        data = buffer.getvalue()
        (self.cache / "example-1.0.0.crate").write_bytes(data)
        self.package["checksum"] = hashlib.sha256(data).hexdigest()

    @staticmethod
    def artifact(package_id, name, kind):
        return {"reason": "compiler-artifact", "package_id": package_id, "fresh": True,
                "features": ["synthetic"], "profile": {"test": False, "opt_level": "3"}, "target": {"name": name, "kind": [kind], "crate_types": [kind]},
                "executable": "/synthetic/target/" + name if kind == "bin" else None}

    def collect(self, extra_lock=""):
        lock = self.root / "Cargo.lock"
        lock.write_text('version = 4\n[[package]]\nname="many-ai-cli"\nversion="0.0.0"\n'
                        '[[package]]\n' + '\n'.join(f'{key}={json.dumps(value)}' for key, value in self.package.items())
                        + '\n' + extra_lock)
        metadata = self.root / "metadata.json"
        metadata.write_text(json.dumps(self.metadata))
        messages = self.root / "messages.jsonl"
        messages.write_text(''.join(json.dumps(message) + '\n' for message in self.messages))
        return collector.collect(lock, metadata, messages, [self.cache],
                                 "x86_64-unknown-linux-gnu", "a" * 40)

    def test_verified_actual_sources_legal_texts_and_build_kinds(self):
        report, notice = self.collect()
        self.assertEqual(report["status"], "complete-source-evidence")
        self.assertFalse(report["release_ready"])
        package = report["packages"][0]
        self.assertEqual(package["verified_source_files"], 4)
        self.assertEqual(len(package["observations"]), 3)
        self.assertIn("native/vendor/NOTICE", notice)
        self.assertNotIn(str(self.root), json.dumps(report))
        spdx = collector.spdx_document(report, "2026-10-05T00:00:00Z")
        self.assertEqual(spdx["packages"][0]["checksums"][0]["checksumValue"], self.package["checksum"])
        self.assertEqual(spdx["packages"][0]["licenseConcluded"], "NOASSERTION")
        self.assertIn("not a complete distribution SBOM", spdx["name"])

    def test_lock_only_packages_are_not_claimed_as_build_inputs(self):
        report, _ = self.collect('[[package]]\nname="unused"\nversion="9.0.0"\n'
                                 f'source={json.dumps(collector.REGISTRY)}\nchecksum="' + 'b' * 64 + '"\n')
        self.assertEqual(report["observed_registry_packages"], 1)
        self.assertEqual(report["locked_registry_packages"], 2)
        self.assertEqual(report["status"], "complete-source-evidence")

    def test_missing_archive_retains_incomplete_evidence(self):
        (self.cache / "example-1.0.0.crate").unlink()
        report, _ = self.collect()
        self.assertEqual(report["packages"][0]["status"], "missing-archive")
        self.assertEqual(report["status"], "incomplete-source-evidence")

    def test_checksum_mismatch_does_not_borrow_license_text(self):
        (self.cache / "example-1.0.0.crate").write_bytes(b"changed")
        report, notices = self.collect()
        self.assertEqual(report["packages"][0]["status"], "checksum-mismatch")
        self.assertNotIn("Synthetic MIT", notices)

    def test_modified_extracted_source_rejects_clean_archive(self):
        (self.source / "src/lib.rs").write_text("changed actual build source")
        report, notices = self.collect()
        self.assertEqual(report["packages"][0]["status"], "invalid-archive")
        self.assertNotIn("Synthetic MIT", notices)

    def test_metadata_cannot_read_source_outside_selected_cache(self):
        self.metadata["packages"][1]["manifest_path"] = "/outside/secret/Cargo.toml"
        report, _ = self.collect()
        self.assertEqual(report["packages"][0]["status"], "invalid-archive")
        self.assertNotIn("/outside", json.dumps(report))

    def test_archive_traversal_is_rejected_without_extracting(self):
        malicious = tarfile.TarInfo("example-1.0.0/../../escape")
        self.archive(malicious)
        report, _ = self.collect()
        self.assertEqual(report["packages"][0]["status"], "invalid-archive")
        self.assertFalse((self.root / "escape").exists())

    def test_archive_symlink_is_rejected(self):
        malicious = tarfile.TarInfo("example-1.0.0/LEGAL")
        malicious.type = tarfile.SYMTYPE
        malicious.linkname = "/outside"
        self.archive(malicious)
        report, _ = self.collect()
        self.assertEqual(report["packages"][0]["status"], "invalid-archive")

    def test_missing_expression_requires_manual_review(self):
        self.members["Cargo.toml"] = b'[package]\nname="example"\nversion="1.0.0"\nlicense-file="LICENSE-MIT"\n'
        self.archive()
        report, _ = self.collect()
        self.assertEqual(report["packages"][0]["status"], "license-review-required")

    def test_no_legal_text_is_not_accepted_from_expression_alone(self):
        del self.members["LICENSE-MIT"]
        del self.members["native/vendor/NOTICE"]
        self.archive()
        report, _ = self.collect()
        self.assertEqual(report["packages"][0]["status"], "license-review-required")

    def test_custom_license_filename_is_collected(self):
        self.members["Cargo.toml"] += b'license-file="legal/terms.txt"\n'
        self.members["legal/terms.txt"] = b'Synthetic custom license\n'
        self.archive()
        report, notices = self.collect()
        self.assertEqual(report["status"], "complete-source-evidence")
        self.assertIn("Synthetic custom license", notices)

    def test_source_identity_must_match_lock(self):
        self.members["Cargo.toml"] = self.members["Cargo.toml"].replace(b'1.0.0', b'2.0.0')
        self.archive()
        report, _ = self.collect()
        self.assertEqual(report["packages"][0]["status"], "invalid-archive")

    def test_failed_or_unfinished_build_cannot_produce_evidence(self):
        self.messages[-1]["success"] = False
        with self.assertRaisesRegex(ValueError, "successfully"):
            self.collect()
        self.messages.pop()
        with self.assertRaisesRegex(ValueError, "build-finished"):
            self.collect()

    def test_both_binaries_and_known_dependencies_required(self):
        self.messages.pop(-2)
        with self.assertRaisesRegex(ValueError, "both root binaries"):
            self.collect()
        self.messages.insert(-1, self.artifact(self.root_id, "many-ai-cli-launcher", "bin"))
        self.messages[0]["package_id"] = "unknown"
        with self.assertRaisesRegex(ValueError, "missing from Cargo metadata"):
            self.collect()

    def test_test_harness_cannot_masquerade_as_delivery_binary(self):
        self.messages[-2]["profile"]["test"] = True
        with self.assertRaisesRegex(ValueError, "test harness"):
            self.collect()

    def test_multiple_or_appended_build_runs_are_rejected(self):
        self.messages.append({"reason": "build-finished", "success": True})
        with self.assertRaisesRegex(ValueError, "one complete build log"):
            self.collect()

    def test_empty_dependency_log_is_rejected(self):
        self.messages = self.messages[-3:]
        with self.assertRaisesRegex(ValueError, "no dependency"):
            self.collect()

    def test_pinned_source_notice_excerpt_and_changed_hash(self):
        self.members["native/source.h"] = b"/* Synthetic source notice. */\nint example;\n"
        self.archive()
        selected = self.root / "source-notices.json"
        supplement = {"package": "example", "version": "1.0.0", "path": "native/source.h",
                      "sha256": collector.sha(self.members["native/source.h"]),
                      "first_line": 1, "last_line": 1, "scope": "synthetic source header"}
        selected.write_text(json.dumps({"notices": [supplement]}))
        with patch.object(collector, "SOURCE_NOTICES", selected):
            report, notices = self.collect()
            self.assertEqual(report["status"], "complete-source-evidence")
            self.assertIn("Synthetic source notice", notices)
            self.assertEqual(len(report["packages"][0]["source_notice_excerpts"]), 1)
            supplement["sha256"] = "0" * 64
            selected.write_text(json.dumps({"notices": [supplement]}))
            report, _ = self.collect()
            self.assertEqual(report["packages"][0]["status"], "invalid-archive")

    def test_cli_retains_incomplete_report_without_sbom_and_never_overwrites(self):
        self.collect()  # Materialize synthetic lock/metadata/log.
        output = self.root / "output"
        args = [sys.executable, str(Path(collector.__file__)), "--lockfile", str(self.root / "Cargo.lock"),
                "--metadata", str(self.root / "metadata.json"), "--build-messages", str(self.root / "messages.jsonl"),
                "--cache-dir", str(self.cache), "--target", "x86_64-unknown-linux-gnu",
                "--source-revision", "a" * 40, "--output", str(output)]
        (self.cache / "example-1.0.0.crate").unlink()
        result = subprocess.run(args, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1, result.stderr)
        report = (output / "BUILD-INPUTS.json").read_bytes()
        self.assertFalse((output / "build-inputs.spdx.json").exists())
        self.assertTrue((output / "CRATE-NOTICES.txt").exists())
        result = subprocess.run(args, capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(report, (output / "BUILD-INPUTS.json").read_bytes())


if __name__ == "__main__":
    unittest.main()
