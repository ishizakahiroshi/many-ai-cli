"""Actual release assembler -> ZIP -> npm and channel CLIs, using synthetic receipts.

Only Git source-document/tag lookup is stubbed. No compiler, product binary,
package installation, registry or publication is executed. Linux package
containers and Syft observation documents are synthetic; the workflow's real
asset/checksum functions compose their inventory for the channel-retry test.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

REPO = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--packer", type=Path, default=REPO / "scripts/package-rust-candidates.py")
parser.add_argument("--channels-helper", type=Path, default=REPO / "scripts/prepare-rust-release-channels.py")
parser.add_argument("--without-release-marker", action="store_true", help="negative control: restore the previous npm member allowlist")
ARGS, TEST_ARGS = parser.parse_known_args()


class PackerNpmIntegration(unittest.TestCase):
    def test_actual_assembler_to_npm_and_channels_preserves_canonical_bytes(self):
        with tempfile.TemporaryDirectory(prefix="many-ai-cli-packer-npm-") as temporary:
            root = Path(temporary).resolve()
            scripts = root / "scripts"
            scripts.mkdir()
            packer = scripts / "package-rust-candidates.py"
            shutil.copyfile(ARGS.packer, packer)
            for name in ("stage-npm-binaries.mjs", "stage-npm-from-release.mjs"):
                shutil.copyfile(REPO / "scripts" / name, scripts / name)
            if ARGS.without_release_marker:
                path = scripts / "stage-npm-from-release.mjs"
                text = path.read_text(encoding="utf-8")
                self.assertIn("'RELEASE-EVIDENCE.txt',", text)
                path.write_text(text.replace("'RELEASE-EVIDENCE.txt',", ""), encoding="utf-8")
            spec = importlib.util.spec_from_file_location("release_packer_integration", packer)
            package = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(package)
            source, version, tag = "a" * 40, "1.0.0", "v1.0.0"
            build_time = "2026-10-23T00:00:00+00:00"
            inputs, output = root / "native-inputs", root / "assembled"

            def entry(folder, name, data):
                path = folder / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
                return {"name": name, "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}

            for target, (suffix, goos, _) in package.TARGETS.items():
                folder = inputs / suffix
                extension = ".exe" if goos == "windows" else ""
                binaries = [entry(folder, stem + extension, f"synthetic {suffix} {stem}".encode())
                            for stem in ("many-ai-cli", "many-ai-cli-launcher")]
                notices = [entry(folder, name, f"synthetic notice {name}".encode()) for name in package.NOTICES]
                evidence_data = {
                    "BUILD-INPUTS.json": json.dumps({"release_ready": False, "source_revision": source,
                                                     "declared_target": target}).encode(),
                    "CRATE-NOTICES.txt": f"synthetic crate input notices {suffix}".encode(),
                    "build-inputs.spdx.json": json.dumps({"spdxVersion": "SPDX-2.3"}).encode(),
                }
                evidence = [entry(folder / "packaging-inputs", name, evidence_data[name]) for name in package.PACKAGING_INPUTS]
                receipt = {
                    "schema_version": 2, "status": "native-release-build-not-accepted-for-publication",
                    "source_sha": source, "review_head_sha": source, "build_commit": source,
                    "version": version, "release_tag": tag, "build_kind": "release", "build_time": build_time,
                    "target": target, "target_name": suffix, "packaging_input_exit_code": 0,
                    "cargo_lock_sha256": "b" * 64, "launcher_ui_input_sha256": "c" * 64,
                    "generated_web_asset_inputs": [{"path": "index.html", "sha256": "d" * 64}],
                    "binaries": binaries, "third_party_notices": notices, "packaging_inputs": evidence,
                }
                if goos == "windows":
                    receipt["windows_runtime"] = {"source_sha": source, "embedded_in": "many-ai-cli.exe",
                                                  "embedded_bytes_verified": True}
                    receipt["windows_resources"] = {
                        "version": version, "icon_sha256": "e" * 64,
                        "binaries": [{**item, "file_version": version, "product_version": version,
                                      "icon_payloads_verified": True} for item in binaries],
                    }
                (folder / "BUILD-RECEIPT.json").write_text(json.dumps(receipt), encoding="utf-8")
            docs = {name: f"synthetic source document {name}".encode() for name in (*package.DOCS, "unblock-windows.cmd")}
            web_notice_source = "web/src/vendor/THIRD_PARTY_LICENSES.txt"
            web_notice_output = "WEB-THIRD-PARTY-LICENSES.txt"
            docs[web_notice_source] = b"synthetic Web vendor license\r\nCopyright \xc2\xa9 fixture authors\n"
            with patch.object(package, "release_source_identity", return_value=(source, build_time)), \
                    patch.object(package, "source_documents", return_value=docs):
                package.assemble(inputs, output, root, release_tag=tag)
            self.assertEqual(len(list(output.glob("*.zip"))), 4)
            for suffix, _, _ in package.TARGETS.values():
                with zipfile.ZipFile(output / f"many-ai-cli-{version}-{suffix}.zip") as archive:
                    self.assertIn("RELEASE-EVIDENCE.txt", archive.namelist())
                    self.assertNotIn("NOT-FOR-CUTOVER.txt", archive.namelist())
                    self.assertEqual(archive.read(web_notice_source), docs[web_notice_source])
            environment = os.environ.copy()
            environment.update(PYTHONDONTWRITEBYTECODE="1", MANY_AI_PYTHON=sys.executable)
            result = subprocess.run(["node", str(scripts / "stage-npm-from-release.mjs"), str(output)],
                                    cwd=root, env=environment, capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            for suffix, goos, _ in package.TARGETS.values():
                binary = "many-ai-cli" + (".exe" if goos == "windows" else "")
                platform = root / "npm" / f"many-ai-cli-{suffix}"
                with zipfile.ZipFile(output / f"many-ai-cli-{version}-{suffix}.zip") as archive:
                    self.assertEqual((platform / "bin" / binary).read_bytes(), archive.read(binary))
                    self.assertEqual([p.name for p in (platform / "bin").iterdir()], [binary])
                    self.assertEqual((platform / "THIRD_PARTY_NOTICES.md").read_bytes(), archive.read("THIRD_PARTY_NOTICES.md"))
                    self.assertEqual((platform / web_notice_output).read_bytes(), archive.read(web_notice_source))
                    for name in package.PACKAGING_INPUTS:
                        expected = archive.read("packaging-inputs/" + name)
                        self.assertEqual((platform / "build-inputs" / name).read_bytes(), expected)
                        self.assertEqual((root / "npm/many-ai-cli/build-inputs" / suffix / name).read_bytes(), expected)
            self.assertEqual((root / "npm/many-ai-cli" / web_notice_output).read_bytes(), docs[web_notice_source])

            def channels(archives, destination):
                result = subprocess.run([sys.executable, str(ARGS.channels_helper.resolve()),
                                         "--archives", str(archives), "--tag", tag,
                                         "--repo", "example/project", "--output", str(destination)],
                                        cwd=root, env=environment, capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                with zipfile.ZipFile(output / f"many-ai-cli-{version}-linux-x64.zip") as archive:
                    for binary in ("many-ai-cli", "many-ai-cli-launcher"):
                        expected = archive.read(binary)
                        self.assertEqual((destination / "deb-root/usr/bin" / binary).read_bytes(), expected)
                        self.assertEqual((destination / "rpm/SOURCES" / binary).read_bytes(), expected)
                    for member in ("LICENSE", "THIRD_PARTY_NOTICES.md", *["packaging-inputs/" + name for name in package.PACKAGING_INPUTS], web_notice_source):
                        expected = archive.read(member)
                        name = web_notice_output if member == web_notice_source else member
                        self.assertEqual((destination / "deb-root/usr/share/doc/many-ai-cli" / name).read_bytes(), expected)
                        self.assertEqual((destination / "rpm/SOURCES" / name).read_bytes(), expected)

            channels(output, root / "channels-before-publication")
            workflow_spec = importlib.util.spec_from_file_location("release_workflow_integration", REPO / "scripts/rust-release-workflow.py")
            workflow = importlib.util.module_from_spec(workflow_spec)
            workflow_spec.loader.exec_module(workflow)
            containers = root / "synthetic-linux-containers"
            containers.mkdir()
            for name in ("many-ai-cli-1.0.0-linux-x64.deb", "many-ai-cli-1.0.0-1.x86_64.rpm"):
                (containers / name).write_bytes(b"synthetic container, not a built or installable package")
            distribution = root / "distribution-assets"
            workflow.prepare_assets(output, containers, distribution, tag)
            for asset in list(distribution.iterdir()):
                document = Path(str(asset) + ".spdx.json")
                document.write_text(json.dumps({"spdxVersion": "SPDX-2.3", "creationInfo": {}, "packages": []}), encoding="utf-8")
                workflow.complete_sbom(asset, document, tag)
            workflow.checksum_assets(distribution, tag)
            canonical = root / "downloaded-canonical-zips"
            canonical.mkdir()
            for path in [*distribution.glob("*.zip"), distribution / "SHA256SUMS.txt"]:
                shutil.copyfile(path, canonical / path.name)
            self.assertEqual(len((canonical / "SHA256SUMS.txt").read_text().splitlines()), 12)
            workflow.verify_archives(canonical, tag, source)
            channels(canonical, root / "channels-after-canonical-download")


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0], *TEST_ARGS])
