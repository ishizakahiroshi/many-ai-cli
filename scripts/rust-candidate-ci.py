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

ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "x86_64-unknown-linux-gnu": "linux-x64",
    "x86_64-pc-windows-msvc": "windows-x64",
    "x86_64-apple-darwin": "macos-intel",
    "aarch64-apple-darwin": "macos-apple-silicon",
}

def capture(args: list[str], cwd: Path = ROOT) -> str:
    return subprocess.check_output(args, cwd=cwd, text=True).strip()

def run(args: list[str], env: dict[str, str], cwd: Path = ROOT) -> None:
    print("+ " + " ".join(args), flush=True)
    subprocess.run(args, cwd=cwd, env=env, check=True)

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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    args = parser.parse_args()
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
    run(["bun", "install", "--frozen-lockfile"], env, ROOT / "web")
    run(["bun", "run", "check"], env, ROOT / "web")
    run(["bun", "run", "build"], env, ROOT / "web")
    cargo = ["cargo", "--locked", "--manifest-path", "rust/Cargo.toml"]
    # Cargo places command before its command-specific manifest/lock arguments.
    run(["cargo", "fmt", "--manifest-path", "rust/Cargo.toml", "--", "--check"], env)
    run(["cargo", "clippy", *cargo[1:], "--target", args.target, "--all-targets", "--", "-D", "warnings"], env)
    run(["cargo", "test", *cargo[1:], "--target", args.target], env)
    run(["cargo", "build", *cargo[1:], "--target", args.target, "--release", "--bins"], env)
    output = ROOT / ".rust-candidate-artifacts" / TARGETS[args.target]
    output.mkdir(parents=True, exist_ok=False)
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
    receipt = {
        "schema_version": 1, "status": "native-build-only-not-accepted-for-cutover",
        "source_sha": source, "review_head_sha": env.get("MANY_AI_REVIEW_HEAD_SHA", source),
        "oracle_sha": "21d0bc7935a2c4696fb89ccff2e324157a528c2d",
        "target": args.target, "target_name": TARGETS[args.target],
        "rustc": rustc, "go": go, "bun": bun,
        "version": env["MANY_AI_BUILD_VERSION"], "build_time": env["MANY_AI_BUILD_TIME"],
        "cargo_lock_sha256": digest(ROOT / "rust" / "Cargo.lock"),
        "binaries": binary_entries, "embedded_web_assets": web_entries,
        "launcher_ui_sha256": digest(ROOT / "internal" / "launcher" / "ui" / "index.html"),
        "pending": ["full-application-behavior", "native-device-remote-acceptance", "signed-channel-packaging", "rollback-rehearsal", "user-cutover-approval"],
    }
    (output / "BUILD-RECEIPT.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    (output / "NOT-FOR-CUTOVER.txt").write_text("WIP Rust migration candidate. Native compilation and synthetic tests are not full compatibility, signing, packaging, rollback or user acceptance. Do not replace an installed Go binary.\n", encoding="utf-8")
    print(f"Native candidate receipt: {TARGETS[args.target]} / {source}", flush=True)

if __name__ == "__main__":
    main()
