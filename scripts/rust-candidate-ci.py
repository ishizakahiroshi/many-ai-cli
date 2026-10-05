#!/usr/bin/env python3
"""Clean native build-only receipts for #3. Never publishes/releases or installs candidates."""
from __future__ import annotations
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "x86_64-unknown-linux-gnu": "linux-x64",
    "x86_64-pc-windows-msvc": "windows-x64",
    "x86_64-apple-darwin": "macos-intel",
    "aarch64-apple-darwin": "macos-apple-silicon",
}

def capture(args: list[str], cwd: Path = ROOT) -> str:
    return subprocess.check_output(args, cwd=cwd, text=True).strip()

STEPS: list[dict[str, object]] = []
OUTPUT: Path | None = None

def run(args: list[str], env: dict[str, str], cwd: Path = ROOT,
        *, stdout_file: Path | None = None, check: bool = True) -> int:
    """Retain command, exit and complete output, including failed validation."""
    assert OUTPUT is not None
    name = f"{len(STEPS) + 1:02d}-{Path(args[0]).stem}-{args[1] if len(args) > 1 else 'run'}"
    name = "".join(c if c.isalnum() or c in "-." else "-" for c in name)
    log = OUTPUT / f"{name}.log"
    step = {"command": args, "cwd": cwd.relative_to(ROOT).as_posix(),
            "log": log.name, "stdout": stdout_file.name if stdout_file else log.name}
    STEPS.append(step)
    print("+ " + " ".join(args), flush=True)
    started = time.monotonic()
    try:
        with log.open("wb") as diagnostics:
            if stdout_file is None:
                process = subprocess.Popen(args, cwd=cwd, env=env,
                                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                assert process.stdout is not None
                for line in process.stdout:
                    diagnostics.write(line)
                    diagnostics.flush()
                    sys.stdout.buffer.write(line)
                    sys.stdout.buffer.flush()
                code = process.wait()
                process.stdout.close()
            else:
                with stdout_file.open("wb") as output:
                    code = subprocess.run(args, cwd=cwd, env=env, stdout=output,
                                          stderr=diagnostics, check=False).returncode
        step["exit_code"] = code
    except OSError as error:
        step["exit_code"] = 127
        log.write_text(f"{type(error).__name__}: command could not execute\n", encoding="utf-8")
        raise
    finally:
        step["elapsed_seconds"] = round(time.monotonic() - started, 3)
        (OUTPUT / "VALIDATION-STEPS.json").write_text(json.dumps(STEPS, indent=2) + "\n", encoding="utf-8")
    if check and code:
        raise subprocess.CalledProcessError(code, args)
    return code

def digest(path: Path) -> str:
    if path.is_symlink() or not path.is_file():
        raise ValueError("artifact must be a regular non-symlink file")
    value = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            value.update(block)
    return value.hexdigest()

def host_from_verbose(version: str) -> str:
    return next((line.removeprefix("host: ") for line in version.splitlines() if line.startswith("host: ")), "")

def main() -> None:
    global OUTPUT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    args = parser.parse_args()
    # HEAD identifies the compiled inputs only for a clean source checkout.
    # Dirty local recovery candidates need their separate source-manifest receipt.
    if capture(["git", "status", "--porcelain=v1", "--untracked-files=normal"]):
        raise SystemExit("candidate CI requires a clean checkout; use a separate source-manifest receipt for local recovery builds")
    source = capture(["git", "rev-parse", "HEAD"])
    output = ROOT / ".rust-candidate-artifacts" / TARGETS[args.target]
    output.mkdir(parents=True, exist_ok=False)
    OUTPUT = output
    (output / "CANDIDATE-INPUT.json").write_text(json.dumps({
        "source_sha": source, "target": args.target, "status": "validation-started",
        "review_head_sha": os.environ.get("MANY_AI_REVIEW_HEAD_SHA", source),
        "cargo_lock_sha256": digest(ROOT / "rust" / "Cargo.lock"),
    }, indent=2) + "\n", encoding="utf-8")
    rustc = capture(["rustc", "--version", "--verbose"])
    if not rustc.startswith("rustc 1.90.0 ") or host_from_verbose(rustc) != args.target:
        raise SystemExit("native runner host or pinned Rust version does not match matrix target")
    go = capture(["go", "version"])
    bun = capture(["bun", "--version"])
    if not go.startswith("go version go1.26.8 ") or bun != "1.3.14":
        raise SystemExit("pinned Go/Bun toolchain mismatch")
    source = capture(["git", "rev-parse", "HEAD"])
    if len(source) != 40 or any(c not in "0123456789abcdef" for c in source):
        raise SystemExit("invalid source revision")
    env = os.environ.copy()
    env.update({
        "RUSTUP_TOOLCHAIN": "1.90.0", "GOTOOLCHAIN": "local",
        "CARGO_INCREMENTAL": "0", "CARGO_PROFILE_DEV_DEBUG": "0",
        "CARGO_PROFILE_TEST_DEBUG": "0", "CARGO_BUILD_JOBS": "2",
        "MANY_AI_BUILD_VERSION": f"0.0.0-rust-candidate+{source[:12]}",
        "MANY_AI_BUILD_COMMIT": source,
        "MANY_AI_BUILD_TIME": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    })
    # Populate selected caches before isolated Go fixtures deliberately disable
    # network access and replace HOME. Their child environment keeps these paths.
    env["GOMODCACHE"] = capture(["go", "env", "GOMODCACHE"])
    env["GOCACHE"] = capture(["go", "env", "GOCACHE"])
    env["GOTELEMETRY"] = "off"
    run(["go", "mod", "download"], env)
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    run([sys.executable, "-m", "unittest", "discover", "-s", "rust/packaging", "-p", "test_*.py"], env)
    run(["bun", "install", "--frozen-lockfile"], env, ROOT / "web")
    run(["bun", "run", "check"], env, ROOT / "web")
    run(["bun", "run", "build"], env, ROOT / "web")
    cargo = ["cargo", "--locked", "--manifest-path", "rust/Cargo.toml"]
    # Cargo places command before its command-specific manifest/lock arguments.
    run(["cargo", "fmt", "--manifest-path", "rust/Cargo.toml", "--", "--check"], env)
    run(["cargo", "clippy", *cargo[1:], "--target", args.target, "--all-targets", "--", "-D", "warnings"], env)
    run(["cargo", "test", *cargo[1:], "--target", args.target, "--all-targets", "--no-fail-fast", "--", "--test-threads=8"], env)
    run(["cargo", "test", *cargo[1:], "--target", args.target, "--doc"], env)
    build_messages = output / "cargo-build.jsonl"
    run(["cargo", "build", *cargo[1:], "--target", args.target, "--release", "--bins", "--message-format=json"], env, stdout_file=build_messages)
    metadata = output / "cargo-metadata.json"
    run(["cargo", "metadata", *cargo[1:], "--format-version", "1", "--filter-platform", args.target], env, stdout_file=metadata)
    target_root = Path(env.get("CARGO_TARGET_DIR", str(ROOT / "rust" / "target")))
    if not target_root.is_absolute():
        target_root = ROOT / target_root
    binary_entries = []
    for stem in ("many-ai-cli", "many-ai-cli-launcher"):
        name = stem + (".exe" if "windows" in args.target else "")
        source_file = target_root / args.target / "release" / name
        checksum = digest(source_file)
        if source_file.stat().st_size == 0:
            raise SystemExit("empty candidate binary")
        destination = output / name
        shutil.copy2(source_file, destination)
        if digest(destination) != checksum:
            raise SystemExit("candidate copy hash mismatch")
        binary_entries.append({"name": name, "bytes": destination.stat().st_size, "sha256": checksum})
    web_entries = [{"path": path.relative_to(ROOT / "web" / "dist").as_posix(), "sha256": digest(path)}
                   for path in sorted((ROOT / "web" / "dist").rglob("*")) if path.is_file()]
    notice_entries = []
    for relative in ("rust/THIRD-PARTY-NOTICES.md", "rust/src/approval/policy/go_regex/GO-LICENSE"):
        notice = ROOT / relative
        destination = output / notice.name
        checksum = digest(notice)
        shutil.copy2(notice, destination)
        if digest(destination) != checksum:
            raise SystemExit("candidate notice copy hash mismatch")
        notice_entries.append({"name": notice.name, "sha256": checksum})
    collector = ROOT / "rust" / "packaging" / "collect_inputs.py"
    packaging_exit = run([sys.executable, str(collector), "--lockfile", "rust/Cargo.lock",
                         "--metadata", str(metadata), "--build-messages", str(build_messages),
                         "--cargo-home", env.get("CARGO_HOME", str(Path.home() / ".cargo")),
                         "--target", args.target, "--source-revision", source,
                         "--output", str(output / "packaging-inputs")], env, check=False)
    receipt = {
        "schema_version": 2, "status": "native-build-only-not-accepted-for-cutover",
        "source_sha": source, "review_head_sha": env.get("MANY_AI_REVIEW_HEAD_SHA", source),
        "oracle_sha": "21d0bc7935a2c4696fb89ccff2e324157a528c2d",
        "target": args.target, "target_name": TARGETS[args.target],
        "rustc": rustc, "go": go, "bun": bun, "python": sys.version.split()[0],
        "version": env["MANY_AI_BUILD_VERSION"], "build_time": env["MANY_AI_BUILD_TIME"],
        "cargo_lock_sha256": digest(ROOT / "rust" / "Cargo.lock"),
        "binaries": binary_entries, "generated_web_asset_inputs": web_entries,
        "third_party_notices": notice_entries,
        "validation_steps": STEPS, "packaging_input_exit_code": packaging_exit,
        "launcher_ui_input_sha256": digest(ROOT / "internal" / "launcher" / "ui" / "index.html"),
        "receipt_scope": "native validation/build plus input hashes; linked runtime asset retention is not yet verified",
        "pending": ["binary-entrypoint-and-asset-retention", "full-application-behavior", "native-device-remote-acceptance", "signed-channel-packaging", "rollback-rehearsal", "user-cutover-approval"],
    }
    (output / "BUILD-RECEIPT.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    (output / "NOT-FOR-CUTOVER.txt").write_text("WIP Rust migration candidate. Native compilation and synthetic tests are not full compatibility, signing, packaging, rollback or user acceptance. Do not replace an installed Go binary.\n", encoding="utf-8")
    print("CANDIDATE-BINARY-HASHES " + json.dumps({"source_sha": source, "target": args.target,
          "cargo_lock_sha256": receipt["cargo_lock_sha256"], "binaries": binary_entries}), flush=True)
    if packaging_exit:
        raise SystemExit("native binaries built; packaging input evidence is incomplete (see receipt)")
    print(f"Native candidate receipt: {TARGETS[args.target]} / {source}", flush=True)

if __name__ == "__main__":
    main()
