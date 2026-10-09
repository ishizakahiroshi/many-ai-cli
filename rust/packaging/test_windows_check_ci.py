"""Offline safety/selection/failure contracts; no Cargo, Windows runtime or network."""
import contextlib
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

SOURCE = Path(__file__).resolve().parents[2] / "scripts/rust-windows-check-ci.py"
spec = importlib.util.spec_from_file_location("windows_check_ci", SOURCE)
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)
SHA = "a" * 40
ENV = {"MANY_AI_REVIEW_HEAD_SHA": SHA, "RUNNER_OS": "Windows", "RUNNER_ARCH": "X64",
       "MANY_AI_WINDOWS_CHECK_CACHE_HIT": "true", "MANY_AI_WINDOWS_CHECK_CACHE_KEY": "synthetic-key",
       "MANY_AI_WINDOWS_CHECK_CACHE_MATCHED_KEY": "synthetic-prior-key",
       "MANY_AI_WINDOWS_CHECK_CACHE_SOURCE": "restore-prefix",
       "GITHUB_WORKFLOW_REF": "synthetic/repo/.github/workflows/rust-windows-check.yml@refs/heads/check",
       "GITHUB_WORKFLOW_SHA": "b" * 40}
VERSIONS = {"rustc": "rustc 1.90.0 (synthetic)\nhost: x86_64-pc-windows-msvc",
            "go": "go version go1.26.8 windows/amd64", "bun": "1.3.14"}


class WindowsCheckTests(unittest.TestCase):
    def test_cli_and_environment_inputs_are_bounded(self):
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(vars(ci.parse_args([])), {"suite": "all-tests", "repeat": 1})
            for suite in ci.SUITES:
                self.assertEqual(ci.parse_args(["--suite", suite, "--repeat", "3"]).repeat, 3)
        for env in ({"MANY_AI_WINDOWS_CHECK_SUITE": "appserver; exit 0"},
                    {"MANY_AI_WINDOWS_CHECK_REPEAT": "4"}, {"MANY_AI_WINDOWS_CHECK_REPEAT": "0"}):
            with patch.dict(os.environ, env, clear=True), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit):
                    ci.parse_args([])
        for args in (["--filter", "anything"], ["--repeat", "01"], ["--repeat", "-1"]):
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                ci.parse_args(args)

    def test_source_rejection_precedes_tools_and_artifacts(self):
        for status, source, review in ((" M rust/src/lib.rs", SHA, SHA), ("", "b" * 40, SHA), ("", SHA, "")):
            with self.subTest(status=status, source=source, review=review), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                with patch.object(ci, "ROOT", root), patch.dict(os.environ, {"MANY_AI_REVIEW_HEAD_SHA": review}, clear=True):
                    with patch.object(ci.candidate, "capture", side_effect=[status, source]) as capture:
                        with patch.object(ci.candidate, "run") as run, contextlib.redirect_stderr(io.StringIO()):
                            self.assertEqual(ci.main([]), 1)
                        run.assert_not_called()
                        self.assertEqual(list(root.iterdir()), [])
                        self.assertTrue(all(call.args[0][0] == "git" for call in capture.call_args_list))

    def test_native_host_and_python_are_required_but_image_label_is_observational(self):
        with patch.dict(os.environ, ENV, clear=True), patch.object(sys, "platform", "win32"), patch.object(sys, "version_info", (3, 12, 1)):
            ci.host_gate()
            for variable, value in (("RUNNER_OS", "Linux"), ("RUNNER_ARCH", "ARM64")):
                with patch.dict(os.environ, {variable: value}), self.assertRaisesRegex(ValueError, "native"):
                    ci.host_gate()
            with patch.object(sys, "platform", "linux"), self.assertRaisesRegex(ValueError, "native"):
                ci.host_gate()
            with patch.object(sys, "version_info", (3, 11, 9)), self.assertRaisesRegex(ValueError, "Python"):
                ci.host_gate()

    def test_final_source_gate_allows_only_its_exact_owned_untracked_artifacts(self):
        owned = {".rust-candidate-artifacts/windows-check/QUICK-CHECK.json",
                 ".rust-candidate-artifacts/windows-check/01-cargo-test.log"}
        status = "\n".join("?? " + path for path in sorted(owned))
        with patch.dict(os.environ, ENV, clear=True):
            with patch.object(ci.candidate, "capture", side_effect=[status, SHA]) as capture:
                self.assertEqual(ci.source_gate(owned_artifacts=owned), SHA)
                self.assertEqual(capture.call_args_list[0].args[0][-1], "--untracked-files=all")
            for extra in ("?? unrelated.txt", " M rust/src/lib.rs",
                          "?? .rust-candidate-artifacts/windows-check/not-owned.txt",
                          " M .rust-candidate-artifacts/windows-check/QUICK-CHECK.json"):
                with self.subTest(extra=extra), patch.object(ci.candidate, "capture", return_value=status + "\n" + extra):
                    with self.assertRaisesRegex(ValueError, "clean checkout"):
                        ci.source_gate(owned_artifacts=owned)

    def test_real_git_source_gate_distinguishes_owned_artifacts_and_source_changes(self):
        # Git writes are confined to this disposable fixture, never the checkout.
        with tempfile.TemporaryDirectory(prefix="windows-check-source-") as temp:
            root = Path(temp)
            env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
            env.update({"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                        "GIT_CONFIG_SYSTEM": os.devnull})

            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, env=env, text=True,
                                               stderr=subprocess.PIPE).strip()

            git("init", "--quiet")
            source_file = root / "source.txt"
            source_file.write_text("committed fixture\n", encoding="utf-8")
            git("add", "source.txt")
            git("-c", "user.name=CI fixture", "-c", "user.email=fixture@example.invalid",
                "-c", "commit.gpgsign=false", "-c", "core.hooksPath=" + str(root / "no-hooks"),
                "commit", "--quiet", "-m", "Synthetic source gate fixture")
            env["MANY_AI_REVIEW_HEAD_SHA"] = git("rev-parse", "HEAD")
            with patch.dict(os.environ, env, clear=True), patch.object(ci, "ROOT", root):
                self.assertEqual(ci.source_gate(), env["MANY_AI_REVIEW_HEAD_SHA"])
                output = root / ".rust-candidate-artifacts/windows-check"
                output.mkdir(parents=True)
                receipt = output / "QUICK-CHECK.json"
                receipt.write_text("{}\n", encoding="utf-8")
                allowed = {receipt.relative_to(root).as_posix()}
                with self.assertRaisesRegex(ValueError, "clean checkout"):
                    ci.source_gate()
                self.assertEqual(ci.source_gate(owned_artifacts=allowed), env["MANY_AI_REVIEW_HEAD_SHA"])
                extra = output / "not-owned.txt"
                extra.write_text("unrelated\n", encoding="utf-8")
                with self.assertRaisesRegex(ValueError, "clean checkout"):
                    ci.source_gate(owned_artifacts=allowed)
                extra.unlink()
                source_file.write_text("changed source\n", encoding="utf-8")
                with self.assertRaisesRegex(ValueError, "clean checkout"):
                    ci.source_gate(owned_artifacts=allowed)

    def test_each_pinned_toolchain_is_required(self):
        ci.toolchain_gate(VERSIONS)
        for key, value in (("rustc", "rustc 1.90.1 (synthetic)\nhost: x86_64-pc-windows-msvc"),
                           ("rustc", "rustc 1.90.0 (synthetic)\nhost: x86_64-unknown-linux-gnu"),
                           ("go", "go version go1.26.8 linux/amd64"),
                           ("go", "go version go1.26.9 windows/amd64"), ("bun", "1.3.15")):
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                ci.toolchain_gate({**VERSIONS, key: value})

    def test_receipt_labels_never_dump_unrelated_environment_values(self):
        with patch.dict(os.environ, {**ENV, "UNRELATED_PRIVATE_VALUE": "do-not-copy-synthetic", "GOCACHE": "private-machine-path"}, clear=True):
            encoded = json.dumps(ci.labels())
            self.assertNotIn("do-not-copy-synthetic", encoded)
            self.assertNotIn("private-machine-path", encoded)
            self.assertIn("synthetic-prior-key", encoded)
            with patch.dict(os.environ, {"MANY_AI_WINDOWS_CHECK_CACHE_SOURCE": "invalid\nvalue"}):
                with self.assertRaisesRegex(ValueError, "invalid CI identity/cache label"):
                    ci.labels()

    def test_fixed_mapping_targets_real_regressions_without_shell_filters(self):
        self.assertEqual(set(ci.SUITES), {"all-tests", "appserver", "relay", "git-turn", "conpty"})
        self.assertEqual(ci.SUITES["all-tests"], (("all-targets", ("--all-targets",), ""),))
        self.assertIn(("--test", "native_windows_spawn"), [scope[1] for scope in ci.SUITES["conpty"]])
        self.assertIn("files::git::tests::turn_snapshot_uses_private_temporary_index_and_preserves_real_index", ci.REQUIRED_TESTS["git"])
        for scopes in ci.SUITES.values():
            for _, selectors, test_filter in scopes:
                command = ci.test_command(selectors, test_filter)
                self.assertIn("--locked", command)
                self.assertIn("--no-fail-fast", command)
                self.assertEqual(command[-4:], ["--", "--format=pretty", "--color=never", "--test-threads=8"])
                if test_filter:
                    self.assertIn(test_filter, command)

    def test_inventory_rejects_zero_malformed_count_and_filter_escape(self):
        self.assertEqual(ci.inventory("scope::one: test\nscope::two: test\n\n2 tests, 0 benchmarks\n", "scope::"),
                         ["scope::one", "scope::two"])
        self.assertEqual(ci.inventory("src/lib.rs - demo (line 1): test\n1 test, 0 benchmarks\nall doctests ran in 0.03s; merged doctests compilation took 0.01s\n", ""),
                         ["src/lib.rs - demo (line 1)"])
        for listing in ("0 tests, 0 benchmarks\n", "oops\n", "one: test\n2 tests, 0 benchmarks\n",
                        "one: test\n1 test, 1 benchmark\n", "other::one: test\n1 test, 0 benchmarks\n"):
            with self.subTest(listing=listing), self.assertRaises(ValueError):
                ci.inventory(listing, "scope::")

    def run_synthetic(self, suite="all-tests", repeat=1, failure=None, ignored_only=False, count_delta=0):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        for relative in ("rust/Cargo.lock", "scripts/rust-windows-check-ci.py", "scripts/rust-candidate-ci.py",
                         "web/dist/index.html", "internal/launcher/ui/index.html"):
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("synthetic\n", encoding="utf-8")
        calls = []
        attempts = {}
        captured_envs = []

        def capture(command, cwd):
            if command[:2] == ["git", "status"]:
                return ""
            if command[:2] == ["git", "rev-parse"]:
                return SHA
            if command[:2] == ["go", "env"]:
                return "synthetic-cache-path"
            self.fail(f"unexpected capture {command}")

        def run(command, env, cwd, *, stdout_file=None, check=True):
            calls.append(command)
            captured_envs.append(env.copy())
            index = len(ci.candidate.STEPS) + 1
            log = ci.candidate.OUTPUT / f"{index:02d}.log"
            code = 0
            text = "synthetic diagnostic\n"
            if command[0] in VERSIONS:
                text = VERSIONS[command[0]]
            if command[:2] == ["cargo", "test"]:
                group = next((name for name, selectors, filt in (*ci.SUITES[suite], ("doctests", ("--doc",), ""))
                              if all(item in command for item in selectors) and (not filt or filt in command)), None)
                self.assertIsNotNone(group)
                names = list(ci.REQUIRED_TESTS.get(group, (f"{group}::synthetic",)))
                if "--list" in command:
                    if "--ignored" in command and not ignored_only:
                        names = []
                    text = "".join(name + ": test\n" for name in names) + f"{len(names)} tests, 0 benchmarks\n"
                else:
                    attempts[group] = attempts.get(group, 0) + 1
                    if failure == (group, attempts[group]):
                        code = 7
                    ignored = len(names) if ignored_only else 0
                    passed = len(names) - ignored - (1 if code else 0) + count_delta
                    text = f"test result: {'FAILED' if code else 'ok'}. {passed} passed; {1 if code else 0} failed; {ignored} ignored; 0 measured; 0 filtered out; finished in 0.01s\n"
            elif failure == tuple(command):
                code = 9
            log.write_text("synthetic original failure\n" if code else text, encoding="utf-8")
            if stdout_file:
                stdout_file.write_text(text, encoding="utf-8")
            ci.candidate.STEPS.append({"command": command, "exit_code": code, "log": log.name,
                                       "stdout": stdout_file.name if stdout_file else log.name})
            if check and code:
                raise subprocess.CalledProcessError(code, command)
            return code

        with contextlib.ExitStack() as stack:
            for obj, name, value in ((ci, "ROOT", root), (ci.candidate, "ROOT", root),
                                     (sys, "platform", "win32"), (sys, "version_info", (3, 12, 1))):
                stack.enter_context(patch.object(obj, name, value))
            stack.enter_context(patch.dict(os.environ, ENV, clear=True))
            stack.enter_context(patch.object(ci.candidate, "capture", side_effect=capture))
            stack.enter_context(patch.object(ci.candidate, "run", side_effect=run))
            runtime = stack.enter_context(patch.object(ci.candidate, "windows_runtime_receipt", return_value={"source_sha": SHA}))
            stack.enter_context(contextlib.redirect_stderr(io.StringIO()))
            code = ci.main(["--suite", suite, "--repeat", str(repeat)])
        output = root / ".rust-candidate-artifacts/windows-check"
        receipt = json.loads((output / "QUICK-CHECK.json").read_text(encoding="utf-8"))
        return code, receipt, calls, captured_envs, runtime, output

    def test_cache_hit_still_prepares_and_checks_all_inputs_without_release_or_collector(self):
        code, receipt, calls, envs, runtime, output = self.run_synthetic()
        self.assertEqual(code, 0)
        self.assertEqual(receipt["status"], "passed-partial-check-only")
        self.assertEqual(receipt["source_sha"], SHA)
        self.assertEqual(receipt["workflow"]["GITHUB_WORKFLOW_SHA"], "b" * 40)
        self.assertEqual(receipt["cache"]["matched_key"], "synthetic-prior-key")
        self.assertEqual(receipt["generated_web_asset_inputs"][0]["path"], "index.html")
        self.assertEqual(len(receipt["launcher_ui_input_sha256"]), 64)
        self.assertIn(["go", "mod", "download"], calls)
        self.assertTrue(any("prepare_windows_runtime.ps1" in " ".join(command) for command in calls))
        self.assertTrue(any(command[1:4] == ["-m", "unittest", "discover"] for command in calls))
        for command in (["bun", "install", "--frozen-lockfile"], ["bun", "run", "check"], ["bun", "run", "build"]):
            self.assertIn(command, calls)
        clippy = next(command for command in calls if command[:2] == ["cargo", "clippy"])
        self.assertEqual(clippy[-4:], ["--all-targets", "--", "-D", "warnings"])
        self.assertEqual(runtime.call_count, 2)
        for command, env in zip(calls, envs):
            if command[0] == "cargo":
                self.assertEqual(env["MANY_AI_REQUIRE_WINDOWS_RUNTIME"], "1")
                self.assertEqual(env["MANY_AI_BUILD_COMMIT"], SHA)
                self.assertTrue(env["MANY_AI_BUILD_TIME"])
        self.assertFalse(any("--release" in command or any("collect_inputs" in part for part in command) for command in calls))
        self.assertFalse((output / "BUILD-RECEIPT.json").exists())

    def test_focused_checks_keep_doctests_and_failed_first_repeat(self):
        code, receipt, _, _, _, output = self.run_synthetic("appserver", 3, ("appserver", 1))
        self.assertEqual(code, 1)
        self.assertEqual(receipt["status"], "failed")
        self.assertIn("tests outside selected groups are omitted", receipt["test_scope"])
        self.assertEqual([item["status"] for item in receipt["repeats"]], ["failed", "passed", "passed"])
        self.assertTrue(all(any(group["id"] == "doctests" for group in item["groups"]) for item in receipt["repeats"]))
        failed_step = next(step for step in receipt["validation_steps"] if step["exit_code"] == 7)
        self.assertIn("synthetic original failure", (output / failed_step["log"]).read_text())

    def test_ignored_only_and_execution_mismatch_cannot_pass(self):
        for options in ({"ignored_only": True}, {"count_delta": 1}):
            with self.subTest(options=options):
                code, receipt, *_ = self.run_synthetic("appserver", **options)
                self.assertEqual(code, 1)
                self.assertEqual(receipt["status"], "failed")

    def test_preparation_failure_keeps_receipt_and_blocks_cargo(self):
        code, receipt, calls, _, _, output = self.run_synthetic(failure=("bun", "run", "check"))
        self.assertEqual(code, 1)
        self.assertEqual(receipt["status"], "failed")
        self.assertEqual(receipt["failure"]["type"], "CalledProcessError")
        self.assertEqual(receipt["validation_steps"][-1]["exit_code"], 9)
        self.assertFalse(any(command[0] == "cargo" for command in calls))
        self.assertTrue((output / receipt["validation_steps"][-1]["log"]).is_file())

    def test_host_failure_after_source_gate_keeps_partial_receipt(self):
        with tempfile.TemporaryDirectory() as temp, patch.object(ci, "ROOT", Path(temp)):
            with patch.dict(os.environ, ENV, clear=True), patch.object(ci.candidate, "capture", side_effect=["", SHA]):
                with patch.object(ci, "host_gate", side_effect=ValueError("native host mismatch")):
                    with patch.object(ci.candidate, "run") as run, contextlib.redirect_stderr(io.StringIO()):
                        self.assertEqual(ci.main([]), 1)
                run.assert_not_called()
            receipt = json.loads((Path(temp) / ".rust-candidate-artifacts/windows-check/QUICK-CHECK.json").read_text())
            self.assertEqual(receipt["failure"]["reason"], "native host mismatch")
            self.assertEqual(receipt["validation_steps"], [])


if __name__ == "__main__":
    unittest.main()
