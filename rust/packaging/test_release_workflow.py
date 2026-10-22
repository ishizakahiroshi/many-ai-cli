"""Offline release caller/handoff tests; no builds, credentials or network."""
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "rust-release-workflow.py"
SPEC = importlib.util.spec_from_file_location("release_workflow", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
SOURCE = "a" * 40
OTHER = "b" * 40


class ReleaseWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="release-fixture-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()

    def plan(self, **changes):
        args = dict(event="push", ref="refs/tags/v1.0.0", event_sha=SOURCE,
                    tag="v1.0.0", npm_only="false", channel="legacy", head=SOURCE, tag_commit=SOURCE)
        args.update(changes)
        return MODULE.release_plan(**args)

    def archives(self, directory=None, tag="v1.0.0"):
        directory = directory or self.root / "archives"
        directory.mkdir()
        for suffix in MODULE.SUFFIXES:
            path = directory / f"many-ai-cli-{tag[1:]}-{suffix}.zip"
            path.write_bytes((suffix + tag).encode())
        self.sums(directory)
        return directory

    def sums(self, directory):
        paths = sorted(p for p in directory.iterdir() if p.name != "SHA256SUMS.txt")
        (directory / "SHA256SUMS.txt").write_text("".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n" for p in paths), encoding="utf-8")

    def cli(self, *arguments, ok=True):
        result = subprocess.run([sys.executable, str(SCRIPT), *map(str, arguments)], capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode == 0, ok, result.stdout + result.stderr)
        return result

    def test_full_push_and_tag_dispatch_match(self):
        self.assertEqual(self.plan()["mode"], "full")
        self.assertEqual(self.plan(event="workflow_dispatch", channel="full")["source_sha"], SOURCE)

    def test_legacy_dispatch_retains_boolean_contract(self):
        self.assertEqual(self.plan(event="workflow_dispatch", npm_only="true", ref="refs/heads/main", head=OTHER, event_sha=OTHER)["mode"], "npm")
        self.assertEqual(self.plan(event="workflow_dispatch", npm_only="false")["mode"], "full")

    def test_explicit_channel_does_not_build(self):
        for channel in ("npm", "homebrew", "winget"):
            with self.subTest(channel=channel):
                self.assertEqual(self.plan(event="workflow_dispatch", ref="refs/heads/develop", head=OTHER, event_sha=OTHER, channel=channel)["mode"], channel)

    def test_unrelated_branch_cannot_full_release_even_after_tag_checkout(self):
        for changes in ({"ref": "refs/heads/main"}, {"event_sha": OTHER}, {"head": OTHER}, {"tag_commit": OTHER}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                self.plan(event="workflow_dispatch", channel="full", **changes)

    def test_canonical_tags_only(self):
        for tag in ("v01.0.0", "1.0.0", "v1.0.0+build", "v1.0.0-01", "v1.0.0-rc..1", "v1.0.0\n", "--help", "v1.0.0/../../x"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                MODULE.checked_tag(tag)
        self.assertEqual(MODULE.checked_tag("v1.0.0-rc.1"), "v1.0.0-rc.1")

    def test_notes_extracts_exact_section_and_rejects_empty_duplicate(self):
        text = "## [Unreleased]\nnext\n## [1.0.0] - date\n### Fixed\n- Useful fix\n## [0.9.0]\nold\n"
        self.assertEqual(MODULE.extract_notes(text, "v1.0.0"), "### Fixed\n- Useful fix\n")
        for bad in ("## [1.0.0]\n### Fixed\n<!-- placeholder -->", "## [1.0.0]\nx\n## [1.0.0]\ny", "## [1.0.01]\nx"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                MODULE.extract_notes(bad, "v1.0.0")

    def test_real_archive_verification_caller(self):
        archives = self.archives()
        self.cli("verify-archives", "--directory", archives, "--tag", "v1.0.0")
        (archives / "many-ai-cli-1.0.0-linux-x64.zip").write_bytes(b"changed")
        result = self.cli("verify-archives", "--directory", archives, "--tag", "v1.0.0", ok=False)
        self.assertIn("checksum mismatch", result.stderr)

    def test_partial_mixed_and_unsafe_checksums_rejected(self):
        for kind in ("partial", "mixed", "duplicate", "traversal"):
            directory = self.archives(self.root / kind)
            if kind == "partial":
                (directory / "many-ai-cli-1.0.0-linux-x64.zip").unlink()
            elif kind == "mixed":
                (directory / "many-ai-cli-0.9.0-linux-x64.zip").write_bytes(b"old")
            else:
                path = directory / "SHA256SUMS.txt"
                path.write_text(path.read_text() + (path.read_text().splitlines()[0] if kind == "duplicate" else "a" * 64 + "  ../outside.zip") + "\n")
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                MODULE.verify_archives(directory, "v1.0.0")

    def test_moved_tag_cannot_mix_new_package_metadata_with_signed_old_binaries(self):
        directory = self.archives()
        for path in directory.glob("*.zip"):
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr("BUILD-RECEIPT.json", json.dumps(dict(build_kind="release", release_tag="v1.0.0", source_sha=SOURCE, build_commit=SOURCE)))
        self.sums(directory)
        self.cli("verify-archives", "--directory", directory, "--tag", "v1.0.0", "--source", SOURCE)
        result = self.cli("verify-archives", "--directory", directory, "--tag", "v1.0.0", "--source", OTHER, ok=False)
        self.assertIn("does not match", result.stderr)

    def test_native_handoff_rejects_old_attempt_and_mixed_source(self):
        names = MODULE.artifact_names(SOURCE, "19", "2")
        artifacts = [{"name": name, "expired": False} for name in names]
        MODULE.check_native_artifacts(artifacts, SOURCE, "19", "2")
        for replacement in (names[0].replace("attempt-2", "attempt-1"), names[0].replace(SOURCE, OTHER)):
            mixed = [{"name": replacement}] + artifacts[1:]
            with self.subTest(replacement=replacement), self.assertRaises(ValueError):
                MODULE.check_native_artifacts(mixed, SOURCE, "19", "2")
        with self.assertRaises(ValueError):
            MODULE.check_native_artifacts(artifacts + artifacts[:1], SOURCE, "19", "2")

    def test_exact_source_ci_latest_failure_is_not_hidden(self):
        passed = dict(head_sha=SOURCE, event="push", status="completed", conclusion="success", run_number=1, run_attempt=1)
        MODULE.check_source_ci([passed], SOURCE)
        for latest in (dict(passed, run_attempt=2, conclusion="failure"), dict(passed, run_number=2, status="in_progress")):
            with self.subTest(latest=latest), self.assertRaises(ValueError):
                MODULE.check_source_ci([passed, latest], SOURCE)
        with self.assertRaises(ValueError):
            MODULE.check_source_ci([dict(passed, head_sha=OTHER)], SOURCE)

    def test_tag_triggered_scan_pending_is_polled_until_latest_result(self):
        now, sleeps, calls = [0], [], []
        passed = dict(head_sha=SOURCE, event="push", status="completed", conclusion="success", run_number=10, run_attempt=1)
        pending = dict(passed, status="in_progress", conclusion=None, run_number=11)
        def fetch(workflow):
            calls.append(workflow)
            if workflow == "secret-scan.yml":
                return [passed, pending if not sleeps else dict(pending, status="completed", conclusion="success")]
            return [passed]
        def sleep(seconds):
            sleeps.append(seconds)
            now[0] += seconds
        MODULE.wait_source_ci(fetch, SOURCE, timeout=60, interval=15, clock=lambda: now[0], sleep=sleep)
        self.assertEqual(sleeps, [15])
        self.assertEqual(calls.count("secret-scan.yml"), 2)

    def test_ci_polling_never_reuses_old_green_or_retries_completed_failure(self):
        now, sleeps = [0], []
        passed = dict(head_sha=SOURCE, event="push", status="completed", conclusion="success", run_number=10, run_attempt=1)
        failed = dict(passed, conclusion="failure", run_number=11)
        def sleep(seconds):
            sleeps.append(seconds)
            now[0] += seconds
        with self.assertRaisesRegex(ValueError, "unsuccessful"):
            MODULE.wait_source_ci(lambda _: [passed, failed], SOURCE, clock=lambda: now[0], sleep=sleep)
        self.assertEqual(sleeps, [])
        pending = dict(failed, status="queued", conclusion=None)
        with self.assertRaisesRegex(ValueError, "timed out"):
            MODULE.wait_source_ci(lambda _: [passed, pending], SOURCE, timeout=30, interval=10, clock=lambda: now[0], sleep=sleep)
        self.assertEqual(sleeps, [10, 10, 10])

    def test_ci_polling_timeout_includes_api_time_and_transport_failure_is_final(self):
        now = [0]
        def fetch(_):
            now[0] += 20
            return []
        def never_sleep(_):
            self.fail("deadline expired during API calls; no further sleep is allowed")
        with self.assertRaisesRegex(ValueError, "timed out"):
            MODULE.wait_source_ci(fetch, SOURCE, timeout=30, clock=lambda: now[0], sleep=never_sleep)
        def transport_failure(_):
            raise ValueError("transport failed")
        with self.assertRaisesRegex(ValueError, "transport failed"):
            MODULE.wait_source_ci(transport_failure, SOURCE, sleep=never_sleep)

    def test_asset_preflight_has_no_output_on_partial_or_candidate(self):
        bundle = self.archives()
        receipt = bundle / "PACKAGE-RECEIPT.json"
        receipt.write_text(json.dumps(dict(build_kind="release", release_tag="v1.0.0", build_commit=SOURCE, release_ready=False)))
        packages = self.root / "linux"
        packages.mkdir()
        output = self.root / "output"
        with self.assertRaises(ValueError):
            MODULE.prepare_assets(bundle, packages, output, "v1.0.0")
        self.assertFalse(output.exists())
        (packages / "many-ai-cli-1.0.0-linux-x64.deb").write_bytes(b"deb")
        (packages / "many-ai-cli-1.0.0-1.x86_64.rpm").write_bytes(b"rpm")
        receipt.write_text(json.dumps(dict(build_kind="candidate", release_tag="v1.0.0", build_commit=SOURCE, release_ready=False)))
        with self.assertRaises(ValueError):
            MODULE.prepare_assets(bundle, packages, output, "v1.0.0")
        self.assertFalse(output.exists())
        receipt.write_text(json.dumps(dict(build_kind="release", release_tag="v1.0.0", build_commit=SOURCE, release_ready=False)))
        self.cli("prepare-assets", "--bundle", bundle, "--packages", packages, "--output", output, "--tag", "v1.0.0")
        self.assertEqual(len(list(output.iterdir())), 6)
        for path in output.iterdir():
            origin = bundle / path.name if path.suffix == ".zip" else packages / path.name
            self.assertEqual(path.read_bytes(), origin.read_bytes())

    def test_prerelease_assets_skip_linux_containers(self):
        bundle = self.archives(tag="v1.0.0-rc.1")
        (bundle / "PACKAGE-RECEIPT.json").write_text(json.dumps(dict(build_kind="release", release_tag="v1.0.0-rc.1", build_commit=SOURCE, release_ready=False)))
        packages = self.root / "empty"
        packages.mkdir()
        output = self.root / "assets"
        MODULE.prepare_assets(bundle, packages, output, "v1.0.0-rc.1")
        self.assertEqual(len(list(output.iterdir())), 4)

    def test_checksum_and_signed_handoff_real_callers(self):
        assets = self.archives(tag="v1.0.0-rc.1")
        (assets / "SHA256SUMS.txt").unlink()
        for path in list(assets.iterdir()):
            document = Path(str(path) + ".spdx.json")
            document.write_text(json.dumps({"spdxVersion": "SPDX-2.3", "creationInfo": {}, "packages": []}))
            MODULE.complete_sbom(path, document, "v1.0.0-rc.1")
        self.cli("checksums", "--directory", assets, "--tag", "v1.0.0-rc.1")
        for name in ("SHA256SUMS.txt.sig", "SHA256SUMS.txt.pem"):
            (assets / name).write_bytes(b"synthetic signature; not cryptographic verification")
        self.cli("verify-assets", "--directory", assets, "--tag", "v1.0.0-rc.1")
        next(assets.glob("*.spdx.json")).write_bytes(b"changed")
        self.cli("verify-assets", "--directory", assets, "--tag", "v1.0.0-rc.1", ok=False)

    def test_wrong_sbom_pair_fails_before_checksum_creation(self):
        assets = self.archives(tag="v1.0.0-rc.1")
        (assets / "SHA256SUMS.txt").unlink()
        for path in list(assets.iterdir()):
            Path(str(path) + ".spdx.json").write_text(json.dumps({"spdxVersion": "SPDX-2.3", "packages": [{"name": path.name}]}))
        sbom = next(assets.glob("*.spdx.json"))
        sbom.rename(assets / "many-ai-cli-wrong.zip.spdx.json")
        with self.assertRaises(ValueError):
            MODULE.checksum_assets(assets, "v1.0.0-rc.1")
        self.assertFalse((assets / "SHA256SUMS.txt").exists())

    def test_sbom_for_another_payload_cannot_be_signed_as_current_distribution(self):
        assets = self.archives(tag="v1.0.0-rc.1")
        (assets / "SHA256SUMS.txt").unlink()
        for path in list(assets.iterdir()):
            document = Path(str(path) + ".spdx.json")
            document.write_text(json.dumps({"spdxVersion": "SPDX-2.3", "creationInfo": {}, "packages": []}))
            MODULE.complete_sbom(path, document, "v1.0.0-rc.1")
        first = next(assets.glob("*.spdx.json"))
        data = json.loads(first.read_text())
        data["packages"][0]["checksums"][0]["checksumValue"] = "b" * 64
        first.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, "does not bind"):
            MODULE.checksum_assets(assets, "v1.0.0-rc.1")
        self.assertFalse((assets / "SHA256SUMS.txt").exists())

    def test_real_shallow_tagless_checkout_cannot_feed_the_formal_packer(self):
        # Git fixtures live only in our temporary directory. They never change
        # the source checkout, its index, refs, hooks or user configuration.
        origin = self.root / "origin"
        clone = self.root / "shallow"
        hooks = self.root / "empty-hooks"
        hooks.mkdir()
        def git(*args, cwd=self.root, ok=True):
            command = ["git", "-c", f"core.hooksPath={hooks}", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.com", *map(str, args)]
            result = subprocess.run(command, cwd=cwd, text=True, capture_output=True, timeout=30)
            self.assertEqual(result.returncode == 0, ok, result.stdout + result.stderr)
            return result
        git("init", "--initial-branch=main", origin)
        (origin / "source.txt").write_text("synthetic source\n")
        git("add", "source.txt", cwd=origin)
        git("commit", "-m", "Synthetic release source", cwd=origin)
        git("tag", "v1.0.0", cwd=origin)
        git("clone", "--depth=1", "--no-tags", origin.as_uri(), clone)
        rejected = git("rev-parse", "--verify", "refs/tags/v1.0.0^{commit}", cwd=clone, ok=False)
        self.assertIn("fatal", rejected.stderr)
        git("fetch", "--unshallow", "--tags", "origin", cwd=clone)
        self.assertEqual(git("rev-parse", "--verify", "refs/tags/v1.0.0^{commit}", cwd=clone).stdout, git("rev-parse", "HEAD", cwd=clone).stdout)
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        package = workflow.split("  package:\n", 1)[1].split("  sign:\n", 1)[0]
        checkout = package.split("      - uses: actions/checkout@", 1)[1].split("      - ", 1)[0]
        self.assertIn("fetch-depth: 0", checkout)

    def test_stable_pointer_refuses_prerelease_before_any_api(self):
        class NoAPI:
            def api(self, *args, **kwargs):
                raise AssertionError("unexpected API access")
        for publisher in (MODULE.publish_homebrew, MODULE.publish_winget):
            with self.subTest(publisher=publisher), self.assertRaises(ValueError):
                publisher(self.root, "v1.0.0-rc.1", NoAPI())

    def test_linux_binary_or_notice_changes_are_rejected_after_package_readback(self):
        expected, deb, rpm = (self.root / name for name in ("expected", "deb", "rpm"))
        for directory in (expected, deb, rpm):
            for name in ("bin/many-ai-cli", "bin/many-ai-cli-launcher", "share/doc/many-ai-cli/LICENSE"):
                target = directory / "usr" / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(name.encode())
        # The CI operation is Linux-only. Synthetic Windows hosts cannot expose
        # Unix executable bits, so model only that independent mode observation.
        original = Path.stat
        def observed_stat(path, *args, **kwargs):
            value = original(path, *args, **kwargs)
            if path.name in ("many-ai-cli", "many-ai-cli-launcher") and path.parent.name == "bin":
                values = list(value)
                values[0] = (values[0] & ~0o777) | 0o755
                return type(value)(values)
            return value
        with patch.object(Path, "stat", observed_stat):
            MODULE.verify_linux_payloads(expected, deb, rpm)
            (rpm / "usr/share/doc/many-ai-cli/LICENSE").write_bytes(b"lost notice")
            with self.assertRaisesRegex(ValueError, "differ"):
                MODULE.verify_linux_payloads(expected, deb, rpm)

    def test_distribution_sbom_root_binds_opaque_payload_without_runtime_claim(self):
        asset = self.root / "many-ai-cli-1.0.0-linux-x64.deb"
        asset.write_bytes(b"synthetic container")
        document = self.root / "distribution.spdx.json"
        document.write_text(json.dumps({"spdxVersion": "SPDX-2.3", "SPDXID": "SPDXRef-DOCUMENT", "creationInfo": {"creators": ["Tool: synthetic-syft"]}, "packages": []}))
        self.cli("complete-sbom", "--asset", asset, "--document", document, "--tag", "v1.0.0")
        data = json.loads(document.read_text())
        self.assertEqual(data["packages"][0]["checksums"][0]["checksumValue"], hashlib.sha256(asset.read_bytes()).hexdigest())
        self.assertEqual(data["packages"][0]["licenseConcluded"], "NOASSERTION")
        self.assertIn("do not assert complete runtime", data["packages"][0]["sourceInfo"])

    def test_homebrew_existing_identical_payload_is_idempotent(self):
        (self.root / "homebrew").mkdir()
        (self.root / "homebrew" / "many-ai-cli.rb").write_bytes(b"synthetic cask\n")
        calls = []
        class FakeAPI:
            def api(self, endpoint, method="GET", payload=None):
                calls.append((endpoint, method, payload))
                return {"sha": SOURCE, "content": base64.b64encode(b"synthetic cask\n").decode()}
        MODULE.publish_homebrew(self.root, "v1.0.0", FakeAPI())
        self.assertEqual(len(calls), 1)

    def test_homebrew_update_uses_existing_sha_and_only_the_generated_cask(self):
        (self.root / "homebrew").mkdir()
        (self.root / "homebrew" / "many-ai-cli.rb").write_bytes(b"new synthetic cask\n")
        calls = []
        class FakeAPI:
            def api(self, endpoint, method="GET", payload=None):
                calls.append((endpoint, method, payload))
                return {"sha": SOURCE, "content": base64.b64encode(b"old cask\n").decode()}
        MODULE.publish_homebrew(self.root, "v1.0.0", FakeAPI())
        self.assertEqual(calls[-1][1], "PUT")
        self.assertEqual(calls[-1][2]["sha"], SOURCE)
        self.assertEqual(base64.b64decode(calls[-1][2]["content"]), b"new synthetic cask\n")

    def test_winget_update_is_scoped_and_never_force_pushes(self):
        (self.root / "winget").mkdir()
        for tail in ("", ".installer", ".locale.en-US"):
            (self.root / "winget" / f"ishizakahiroshi.many-ai-cli{tail}.yaml").write_text("synthetic manifest\n")
        for existing in (False, True):
            calls = []
            class FakeAPI:
                def api(self, endpoint, method="GET", payload=None):
                    calls.append((endpoint, method, payload))
                    if endpoint.endswith("/merge-upstream"):
                        return {}
                    if endpoint.endswith("/git/ref/heads/master"):
                        return {"object": {"sha": SOURCE}}
                    if "/git/matching-refs/" in endpoint:
                        return [{"ref": "refs/heads/many-ai-cli-1.0.0", "object": {"sha": SOURCE}}] if existing else []
                    if endpoint.endswith("/git/commits/" + SOURCE):
                        return {"tree": {"sha": "oldtree"}}
                    if endpoint.endswith("/git/trees"):
                        return {"sha": "newtree"}
                    if endpoint.endswith("/git/commits"):
                        return {"sha": OTHER}
                    if "/pulls?" in endpoint:
                        return [{"state": "closed", "merged_at": "synthetic-date"}] if existing else []
                    return {}
            MODULE.publish_winget(self.root, "v1.0.0", FakeAPI())
            tree = next(payload for endpoint, _, payload in calls if endpoint.endswith("/git/trees"))
            self.assertEqual(len(tree["tree"]), 3)
            self.assertTrue(all(e["path"].startswith("manifests/i/ishizakahiroshi/many-ai-cli/1.0.0/") for e in tree["tree"]))
            writes = [(endpoint, method, payload) for endpoint, method, payload in calls if "/git/refs" in endpoint]
            self.assertEqual(len(writes), 1)
            if existing:
                self.assertEqual(writes[0][1], "PATCH")
                self.assertIs(writes[0][2]["force"], False)
                self.assertFalse(any(method == "POST" and endpoint.endswith("/pulls") for endpoint, method, _ in calls))
            else:
                self.assertEqual(writes[0][2]["ref"], "refs/heads/many-ai-cli-1.0.0")
                self.assertTrue(any(method == "POST" and endpoint.endswith("/pulls") for endpoint, method, _ in calls))

    def test_workflow_uses_uncredentialed_single_native_caller_and_signed_reruns(self):
        text = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        jobs = dict(re.findall(r"^  ([a-z]+):\n(.*?)(?=^  [a-z]+:|\Z)", text.split("jobs:\n", 1)[1], re.M | re.S))
        self.assertIn("--release-tag", jobs["native"])
        self.assertEqual(jobs["native"].count("scripts/rust-candidate-ci.py"), 2)  # mutually exclusive native shells
        self.assertEqual(jobs["native"].count("target:"), 4)
        for name in ("native", "package"):
            self.assertNotRegex(jobs[name], r"secrets\.(?:NPM_TOKEN|PUBLISH_GITHUB_TOKEN)")
            self.assertNotIn("id-token: write", jobs[name])
        for name in ("sign", "publish", "canonical", "npm", "pointers"):
            self.assertNotRegex(jobs[name], r"cargo\s+(?:build|run)|rust-candidate-ci\.py|package-rust-candidates\.py")
        self.assertIn("cosign verify-blob", jobs["canonical"])
        self.assertIn("scripts/stage-npm-from-release.mjs ../.canonical", jobs["npm"])
        self.assertIn("ref: ${{ needs.plan.outputs.source_sha }}", jobs["npm"])
        self.assertIn("working-directory: .release-tag", jobs["npm"])
        self.assertNotIn("scripts/stage-npm-binaries.mjs", text)
        for step in text.split("      - ")[1:]:
            if "NODE_AUTH_TOKEN:" in step or "GH_TOKEN: ${{ secrets.PUBLISH_GITHUB_TOKEN }}" in step:
                self.assertNotRegex(step, r"bun (?:install|run)|npm install|cargo |apt-get|uses:")


if __name__ == "__main__":
    unittest.main()
