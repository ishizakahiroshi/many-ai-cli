#!/usr/bin/env python3
"""Partial Windows validation only; never a release, packaging or acceptance receipt."""
from __future__ import annotations

import argparse
import datetime
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

# Loading the shared driver must not dirty the checkout before its source gate.
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent
_spec = importlib.util.spec_from_file_location("windows_check_candidate", ROOT / "scripts/rust-candidate-ci.py")
candidate = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(candidate)
TARGET = "x86_64-pc-windows-msvc"
CARGO = ["--locked", "--manifest-path", "rust/Cargo.toml", "--target", TARGET]

# Only these reviewed Cargo targets and libtest filters are accepted. Inventory
# comes from the actual compiled harness, including Windows-only cfg selection.
SUITES = {
    "all-tests": (("all-targets", ("--all-targets",), ""),),
    "appserver": (("appserver", ("--lib",), "application::subscriptions::appserver::tests::"),),
    "relay": (("relay", ("--lib",), "application::relay_program::tests::"),),
    "git-turn": (
        ("git", ("--lib",), "files::git::tests::"),
        ("git-turn-observer", ("--lib",), "application::event_observer::tests::"),
    ),
    "conpty": (
        ("conpty-unit", ("--lib",), "process::pty_windows::tests::"),
        ("conpty-native", ("--test", "native_windows_spawn"),
         "actual_windows_spawn_waits_for_ui_input_and_delivers_pty_output_and_clean_end"),
    ),
}
REQUIRED_TESTS = {
    "appserver": ("application::subscriptions::appserver::tests::native_interactive_rpc_waits_each_response_and_reaps_child",),
    "relay": ("application::relay_program::tests::headless_done_waits_for_exit_and_times_out_without_launching_reviewer",),
    "git": ("files::git::tests::turn_snapshot_uses_private_temporary_index_and_preserves_real_index",),
    "git-turn-observer": (
        "application::event_observer::tests::start_capture_materialization_has_an_independent_resolution_budget",
        "application::event_observer::tests::end_capture_materialization_has_an_independent_resolution_budget"),
    "conpty-unit": ("process::pty_windows::tests::failed_job_attachment_cannot_execute_provider_code",),
    "conpty-native": ("actual_windows_spawn_waits_for_ui_input_and_delivers_pty_output_and_clean_end",),
}


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", default=os.environ.get("MANY_AI_WINDOWS_CHECK_SUITE", "all-tests"))
    parser.add_argument("--repeat", default=os.environ.get("MANY_AI_WINDOWS_CHECK_REPEAT", "1"))
    args = parser.parse_args(argv)
    # Validate defaults too: argparse choices do not validate string defaults.
    if args.suite not in SUITES:
        parser.error("suite must be one of: " + ", ".join(SUITES))
    if str(args.repeat) not in ("1", "2", "3"):
        parser.error("repeat must be 1, 2 or 3")
    args.repeat = int(args.repeat)
    return args


def source_gate(*, owned_artifacts: set[str] | None = None) -> str:
    review = os.environ.get("MANY_AI_REVIEW_HEAD_SHA", "")
    if not re.fullmatch(r"[0-9a-f]{40}", review):
        raise ValueError("MANY_AI_REVIEW_HEAD_SHA must identify the reviewed source SHA")
    mode = "all" if owned_artifacts is not None else "normal"
    status = candidate.capture(["git", "status", "--porcelain=v1", f"--untracked-files={mode}"], ROOT)
    # At completion only, allow exact files this invocation created. Do not
    # ignore a directory, tracked changes or unrelated untracked files.
    allowed = {"?? " + path for path in (owned_artifacts or set())}
    if any(line not in allowed for line in status.splitlines()):
        raise ValueError("Windows check requires a clean checkout")
    source = candidate.capture(["git", "rev-parse", "HEAD"], ROOT)
    if source != review:
        raise ValueError("actual source SHA does not match MANY_AI_REVIEW_HEAD_SHA")
    return source


def host_gate() -> None:
    if (sys.platform != "win32" or os.environ.get("RUNNER_OS") != "Windows"
            or os.environ.get("RUNNER_ARCH") != "X64"):
        raise ValueError("Windows check requires native windows-2022 x64 runner")
    if sys.version_info[:2] != (3, 12):
        raise ValueError("Windows check requires pinned Python 3.12")


def toolchain_gate(versions: dict[str, str]) -> None:
    if (not versions["rustc"].startswith("rustc 1.90.0 ")
            or candidate.host_from_verbose(versions["rustc"]) != TARGET):
        raise ValueError("native host or pinned Rust 1.90.0 mismatch")
    if versions["go"] != "go version go1.26.8 windows/amd64" or versions["bun"] != "1.3.14":
        raise ValueError("pinned native Go/Bun toolchain mismatch")


def labels() -> dict:
    # Explicit non-secret CI labels only, never dump the environment or paths.
    fields = {
        "workflow": {key: os.environ.get(key, "") for key in (
            "GITHUB_REPOSITORY", "GITHUB_WORKFLOW", "GITHUB_WORKFLOW_REF", "GITHUB_WORKFLOW_SHA",
            "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "GITHUB_JOB", "GITHUB_EVENT_NAME", "GITHUB_SHA")},
        "runner": {key: os.environ.get(key, "") for key in ("RUNNER_OS", "RUNNER_ARCH", "ImageOS", "ImageVersion")},
        "cache": {key: os.environ.get("MANY_AI_WINDOWS_CHECK_CACHE_" + key.upper(), "unknown")
                  for key in ("hit", "key", "matched_key", "source")},
    }
    for section in fields.values():
        for value in section.values():
            if len(value) > 512 or any(ord(char) < 32 for char in value):
                raise ValueError("invalid CI identity/cache label")
    return fields


def inventory(text: str, test_filter: str, *, allow_empty: bool = False) -> list[str]:
    names = []
    declared = []
    for line in text.splitlines():
        if not line:
            continue
        # Rustdoc 1.90 prints this after merged doctest discovery as well as
        # execution. It is metadata, never a test or a passing outcome.
        if re.fullmatch(r"all doctests ran in \d+\.\d+s; merged doctests compilation took \d+\.\d+s", line):
            continue
        summary = re.fullmatch(r"(\d+) tests?, (\d+) benchmarks?", line)
        if summary:
            if int(summary[2]):
                raise ValueError("unexpected benchmarks in test inventory")
            declared.append(int(summary[1]))
        elif line.endswith(": test") and line[:-6].strip():
            names.append(line[:-6])
        else:
            raise ValueError("malformed test inventory")
    if not declared or sum(declared) != len(names):
        raise ValueError("test inventory count mismatch")
    if not names and not allow_empty:
        raise ValueError("selected suite matched zero tests")
    if test_filter and any(test_filter not in name for name in names):
        raise ValueError("test inventory escaped the fixed suite filter")
    return names


def execution_counts(text: str) -> dict[str, int]:
    summaries = re.findall(r"^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;", text, re.MULTILINE)
    if not summaries:
        raise ValueError("missing executed test counts")
    return {key: sum(int(row[index]) for row in summaries)
            for index, key in enumerate(("passed", "failed", "ignored"))}


def test_command(selectors: tuple[str, ...], test_filter: str, *, listing: bool = False,
                 ignored: bool = False) -> list[str]:
    command = ["cargo", "test", *CARGO, *selectors, "--no-fail-fast"]
    if test_filter:
        command.append(test_filter)
    if listing:
        return [*command, "--", "--format=pretty", "--color=never", "--list", *(["--ignored"] if ignored else [])]
    return [*command, "--", "--format=pretty", "--color=never", "--test-threads=8"]


def write_receipt(output: Path, receipt: dict) -> None:
    receipt["validation_steps"] = candidate.STEPS
    (output / "QUICK-CHECK.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")


def owned_artifacts(output: Path) -> set[str]:
    names = {"QUICK-CHECK.json", "VALIDATION-STEPS.json", "WINDOWS-RUNTIME.json"}
    for step in candidate.STEPS:
        names.update((step["log"], step["stdout"]))
    return {(output / name).relative_to(ROOT).as_posix() for name in names}


def repeat_tests(groups: list[dict], repeat: int, env: dict[str, str], output: Path, receipt: dict) -> bool:
    failed = False
    for number in range(1, repeat + 1):
        outcome = {"repeat": number, "status": "running", "groups": []}
        receipt["repeats"].append(outcome)
        write_receipt(output, receipt)
        for group in groups:
            path = output / f"repeat-{number}-{group['id']}.txt"
            result = {"id": group["id"], "status": "running"}
            outcome["groups"].append(result)
            write_receipt(output, receipt)
            try:
                code = candidate.run(group["command"], env, ROOT, stdout_file=path, check=False)
            except OSError:
                code = 127
            result.update({"exit_code": code, "step": len(candidate.STEPS),
                           "status": "failed" if code else "passed"})
            try:
                counts = execution_counts(path.read_text(encoding="utf-8"))
                result["counts"] = counts
                if (counts["passed"] + counts["failed"] != len(group["selected_tests"])
                        or counts["ignored"] != len(group["ignored_tests"])):
                    raise ValueError("executed test counts differ from selected inventory")
                if counts["failed"]:
                    result["status"] = "failed"
            except (ValueError, OSError) as error:
                result["status"] = "failed"
                result["count_error"] = str(error) if isinstance(error, ValueError) else "missing test output"
            failed |= result["status"] == "failed"
            write_receipt(output, receipt)
        outcome["status"] = "failed" if any(item["status"] == "failed" for item in outcome["groups"]) else "passed"
        write_receipt(output, receipt)
    return failed


def validate(args: argparse.Namespace, source: str, output: Path, receipt: dict) -> bool:
    host_gate()
    env = os.environ.copy()
    env.update({
        "RUSTUP_TOOLCHAIN": "1.90.0", "GOTOOLCHAIN": "local",
        "CARGO_INCREMENTAL": "0", "CARGO_PROFILE_DEV_DEBUG": "0",
        "CARGO_PROFILE_TEST_DEBUG": "0", "CARGO_BUILD_JOBS": "2",
        "MANY_AI_BUILD_VERSION": f"0.0.0-rust-candidate+{source[:12]}",
        "MANY_AI_BUILD_COMMIT": source,
        "MANY_AI_BUILD_TIME": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "GOTELEMETRY": "off", "PYTHONDONTWRITEBYTECODE": "1",
    })
    receipt["build_metadata"] = {key: env[key] for key in (
        "MANY_AI_BUILD_VERSION", "MANY_AI_BUILD_COMMIT", "MANY_AI_BUILD_TIME")}
    receipt["cargo_lock_sha256"] = candidate.digest(ROOT / "rust/Cargo.lock")
    receipt["driver_sha256"] = candidate.digest(ROOT / "scripts/rust-windows-check-ci.py")
    receipt["shared_driver_sha256"] = candidate.digest(ROOT / "scripts/rust-candidate-ci.py")
    versions = {"python": sys.version.split()[0]}
    receipt["tool_versions"] = versions
    for name, command in (("rustc", ["rustc", "--version", "--verbose"]),
                          ("go", ["go", "version"]), ("bun", ["bun", "--version"])):
        path = output / f"{name}-version.txt"
        candidate.run(command, env, ROOT, stdout_file=path)
        versions[name] = path.read_text(encoding="utf-8").strip()
        write_receipt(output, receipt)
    toolchain_gate(versions)
    # Cache hits never bypass preparation or validation. Recompute dynamic build
    # metadata on every run; build.rs watches these environment variables.
    env["GOMODCACHE"] = candidate.capture(["go", "env", "GOMODCACHE"], ROOT)
    env["GOCACHE"] = candidate.capture(["go", "env", "GOCACHE"], ROOT)
    receipt["preparation"] = "running"
    write_receipt(output, receipt)
    candidate.run(["go", "mod", "download"], env, ROOT)
    runtime = output / "WINDOWS-RUNTIME.json"
    candidate.run(["pwsh", "-NoProfile", "-File", "rust/packaging/prepare_windows_runtime.ps1",
                   "-ReceiptPath", runtime.relative_to(ROOT).as_posix()], env, ROOT)
    receipt["windows_runtime"] = candidate.windows_runtime_receipt(runtime, source)
    env["MANY_AI_REQUIRE_WINDOWS_RUNTIME"] = "1"
    candidate.run([sys.executable, "-m", "unittest", "discover", "-s", "rust/packaging", "-p", "test_*.py"], env, ROOT)
    candidate.run(["bun", "install", "--frozen-lockfile"], env, ROOT / "web")
    candidate.run(["bun", "run", "check"], env, ROOT / "web")
    candidate.run(["bun", "run", "build"], env, ROOT / "web")
    receipt["generated_web_asset_inputs"] = [
        {"path": path.relative_to(ROOT / "web/dist").as_posix(), "sha256": candidate.digest(path)}
        for path in sorted((ROOT / "web/dist").rglob("*")) if path.is_file()]
    receipt["launcher_ui_input_sha256"] = candidate.digest(ROOT / "internal/launcher/ui/index.html")
    receipt["preparation"] = "passed"
    failed = False
    for command in (["cargo", "fmt", "--manifest-path", "rust/Cargo.toml", "--", "--check"],
                    ["cargo", "clippy", *CARGO, "--all-targets", "--", "-D", "warnings"]):
        failed |= candidate.run(command, env, ROOT, check=False) != 0
        write_receipt(output, receipt)
    groups = []
    for name, selectors, test_filter in (*SUITES[args.suite], ("doctests", ("--doc",), "")):
        path = output / f"tests-{name}.txt"
        command = test_command(selectors, test_filter)
        group = {"id": name, "selectors": selectors, "filter": test_filter, "command": command,
                 "required_tests": REQUIRED_TESTS.get(name, ()),
                 "selected_tests": [], "ignored_tests": [], "inventory_status": "running"}
        receipt["selection"].append(group)
        write_receipt(output, receipt)
        code = candidate.run(test_command(selectors, test_filter, listing=True), env, ROOT,
                             stdout_file=path, check=False)
        group["inventory_exit_code"] = code
        if code:
            group["inventory_status"] = "failed"
            failed = True
        else:
            try:
                names = inventory(path.read_text(encoding="utf-8"), test_filter)
                group["enumerated_tests"] = names
                ignored_path = output / f"tests-{name}-ignored.txt"
                ignored_code = candidate.run(test_command(selectors, test_filter, listing=True, ignored=True), env, ROOT,
                                             stdout_file=ignored_path, check=False)
                group["ignored_inventory_exit_code"] = ignored_code
                if ignored_code:
                    raise ValueError("ignored inventory command failed")
                ignored = inventory(ignored_path.read_text(encoding="utf-8"), test_filter, allow_empty=True)
                if any(item not in names for item in ignored):
                    raise ValueError("ignored inventory escaped selected tests")
                selected = names.copy()
                for item in ignored:
                    selected.remove(item)
                group["selected_tests"] = selected
                group["ignored_tests"] = ignored
                if not selected:
                    raise ValueError("selected suite matched zero runnable tests (ignored-only)")
                if any(item not in selected for item in REQUIRED_TESTS.get(name, ())):
                    raise ValueError("selected suite is missing an intended regression test")
                group["inventory_status"] = "passed"
            except ValueError as error:
                group["inventory_status"] = "failed"
                group["error"] = str(error)
                failed = True
        # Even a failed listing is retained as a failure; no later passing test
        # command erases it. Run every fixed group on every requested repeat.
        groups.append(group)
        write_receipt(output, receipt)
    failed |= repeat_tests(groups, args.repeat, env, output, receipt)
    # Verify source and runtime inputs stayed bound to this check, even when
    # restored caches were used. No binary embedding/release claim is made.
    source_gate(owned_artifacts=owned_artifacts(output))
    if candidate.windows_runtime_receipt(runtime, source) != receipt["windows_runtime"]:
        raise ValueError("runtime inputs changed during Windows checks")
    return failed


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    # Only read-only Git identity commands precede this gate. No output files,
    # tool probes, preparation or Cargo execution are allowed before it passes.
    try:
        source = source_gate()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        reason = str(error) if isinstance(error, ValueError) else "source identity could not be read"
        print(f"Windows check refused: {reason}", file=sys.stderr)
        return 1
    output = ROOT / ".rust-candidate-artifacts/windows-check"
    output.mkdir(parents=True, exist_ok=False)
    candidate.OUTPUT = output
    candidate.STEPS = []
    receipt = {
        "schema_version": 1, "kind": "partial-windows-quick-check",
        "status": "running", "source_sha": source, "review_head_sha": source,
        "target": TARGET, "suite": args.suite, "requested_repeats": args.repeat,
        "test_scope": "all-target tests and doctests" if args.suite == "all-tests" else
                      "focused allowlisted tests and full doctests; tests outside selected groups are omitted",
        "preparation": "pending", "selection": [], "repeats": [],
        "receipt_scope": "Partial native Windows validation; not release, SBOM, four-target or device acceptance",
        "omitted": ["optimized-release-build", "license-and-SBOM-collection"],
        "pending_acceptance": ["same-SHA-four-target-gate", "Validate", "secret-scan",
                               "Windows-device-browser-provider-signing-rollback", "user-cutover-approval"],
    }
    started = time.monotonic()
    write_receipt(output, receipt)
    try:
        receipt.update(labels())
        failed = validate(args, source, output, receipt)
        receipt["status"] = "failed" if failed else "passed-partial-check-only"
    except (Exception, KeyboardInterrupt) as error:
        receipt["status"] = "failed"
        if receipt["preparation"] == "running":
            receipt["preparation"] = "failed"
        # Command diagnostics remain in their logs. Do not serialize arbitrary
        # exception text, which can contain machine paths or environment data.
        receipt["failure"] = {"type": type(error).__name__, "step": len(candidate.STEPS)}
        if isinstance(error, ValueError):
            receipt["failure"]["reason"] = str(error)
        print(f"Windows check failed ({type(error).__name__}); see QUICK-CHECK.json and command logs", file=sys.stderr)
    finally:
        for repeat in receipt["repeats"]:
            if repeat["status"] == "running":
                repeat["status"] = "incomplete"
            for group in repeat["groups"]:
                if group["status"] == "running":
                    group["status"] = "incomplete"
        receipt["elapsed_seconds"] = round(time.monotonic() - started, 3)
        write_receipt(output, receipt)
    return 0 if receipt["status"] == "passed-partial-check-only" else 1


if __name__ == "__main__":
    raise SystemExit(main())
