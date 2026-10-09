#!/usr/bin/env python3
"""Observe approval injection using the complete, clean, pinned Go checkout.

Usage: python3 generate_oracle.py --go /absolute/go --oracle-root /absolute/checkout
The caller supplies the installed Go 1.26.8 toolchain and populated module cache.
No fixture is replaced until Go observation and both source checks succeed.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
ORACLE = "d8fbf8598c3effd4e2f837e43ad6f0488c461de3"
START = "<!-- many-ai-cli:approval-rules -->"
END = "<!-- /many-ai-cli:approval-rules -->"
GO_INPUTS = ["*.go", "go.mod", "go.sum"]


def cases():
    bodies = [
        b"",
        b"custom no newline",
        b"custom\n",
        b"custom\r\n",
        b"custom\xff\xfe",
        b"\n@~/.many-ai-cli/approval-rules.md\n",
        b"custom\n\n @~/.many-ai-cli/approval-rules.md \r\nmore\n",
        b"custom\n@~/.many-ai-cli/approval-rules.md\n@~/.many-ai-cli/approval-rules.md\n",
        f"before\n{START}\n<!-- version: 1 -->\nstale\n{END}\nafter".encode(),
        # Keep the original case ID and bytes: v24 must now be upgraded.
        f"before\n{START}\n<!-- version: 24 -->\nkeep\n{END}\nafter".encode(),
        b"<!-- any-ai-cli:approval-rules -->\nold\n<!-- /any-ai-cli:approval-rules -->\n",
        START.encode() + b"\nunterminated",
        b"\n<!-- many-ai-cli:delegation -->\nbody\n<!-- /many-ai-cli:delegation -->\n",
        f"before\n{START}\n<!-- version: 25 -->\nkeep\n{END}\nafter".encode(),
    ]
    return [
        {"Name": f"{provider}-{index}", "Provider": provider,
         "Body": base64.b64encode(body).decode("ascii")}
        for provider in ["claude", "codex", "copilot", "cursor-agent", "opencode", "grok", "shell"]
        for index, body in enumerate(bodies)
    ]


def run(command, *, cwd, env, data=None):
    result = subprocess.run(command, cwd=cwd, env=env, input=data,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if result.returncode:
        raise RuntimeError(result.stderr.decode("utf-8", errors="replace"))
    return result.stdout


def source_state(root, env):
    def git(*args):
        return run(["git", "-C", str(root), *args], cwd=root, env=env)

    if Path(os.fsdecode(git("rev-parse", "--show-toplevel")).strip()).resolve() != root:
        raise ValueError("--oracle-root must be the Go repository root")
    if git("rev-parse", "HEAD").decode("ascii").strip() != ORACLE:
        raise ValueError(f"Go oracle checkout HEAD must be {ORACLE}")
    dirty = git("diff", "--no-ext-diff", "--no-textconv", "--name-only", "HEAD", "--", *GO_INPUTS)
    if dirty:
        raise ValueError("tracked Go inputs differ from the pinned commit: " + os.fsdecode(dirty))
    # Ignored Go files can also enter a package, so do not exclude ignored paths.
    if git("ls-files", "--others", "-z", "--", *GO_INPUTS):
        raise ValueError("oracle checkout contains untracked or ignored Go inputs")
    entries = git("ls-tree", "-r", "--full-tree", "-z", "HEAD").split(b"\0")
    hashes = {}
    for entry in sorted(item for item in entries if item):
        metadata, raw = entry.split(b"\t", 1)
        relative = os.fsdecode(raw)
        if not relative.endswith(".go") and relative not in {"go.mod", "go.sum"}:
            continue
        mode, kind, blob = metadata.split()
        path = root / relative
        if kind != b"blob" or mode not in {b"100644", b"100755"} or path.is_symlink() or not path.is_file():
            raise ValueError(f"Go input is not a regular file: {relative}")
        data = path.read_bytes()
        actual_blob = hashlib.sha1(b"blob " + str(len(data)).encode("ascii") + b"\0" + data).hexdigest()
        if actual_blob != blob.decode("ascii"):
            raise ValueError(f"Go input bytes differ from the pinned Git object: {relative}")
        hashes[relative] = hashlib.sha256(data).hexdigest()
    for required in ["go.mod", "go.sum", "internal/wrapper/approval_rules.go"]:
        if required not in hashes:
            raise ValueError(f"missing tracked Go input: {required}")
    return hashes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--go", type=Path, required=True)
    parser.add_argument("--oracle-root", type=Path, required=True)
    args = parser.parse_args()
    if not args.go.is_absolute() or not args.oracle_root.is_absolute():
        parser.error("--go and --oracle-root must be absolute paths")
    go = args.go.resolve(strict=True)
    root = args.oracle_root.resolve(strict=True)
    if not go.is_file() or not root.is_dir():
        parser.error("the Go executable and oracle checkout must exist")
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    for key in ["GOOS", "GOARCH", "GOROOT"]:
        env.pop(key, None)
    env.update(GOPROXY="off", GOSUMDB="off", GOTOOLCHAIN="local", GOENV="off",
               GOWORK="off", GO111MODULE="on", GOTELEMETRY="off",
               GOFLAGS="-mod=readonly -buildvcs=false")
    source_hashes = source_state(root, env)
    version = run([str(go), "version"], cwd=root, env=env).decode("utf-8").strip()
    if not version.startswith("go version go1.26.8 "):
        raise ValueError("oracle generation requires Go 1.26.8")
    # Resolve existing caches before replacing HOME with an owned synthetic root.
    caches = json.loads(run([str(go), "env", "-json", "GOCACHE", "GOMODCACHE"], cwd=root, env=env))
    env.update(caches)
    corpus = cases()
    case_bytes = (json.dumps(corpus, indent=2) + "\n").encode("utf-8")
    observer = (HERE / "oracle.go").read_bytes()
    with tempfile.TemporaryDirectory(prefix=".approval-rules-oracle-", dir=root) as temporary:
        owned = Path(temporary)
        driver = owned / "main.go"
        driver.write_bytes(observer)
        home = owned / "home"
        scratch = owned / "tmp"
        home.mkdir()
        scratch.mkdir()
        env.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home),
                   TMPDIR=str(scratch), TMP=str(scratch), TEMP=str(scratch))
        rules_output = owned / "rules.txt"
        golden = run([str(go), "run", str(driver), "--rules-output", str(rules_output)],
                     cwd=root, env=env, data=case_bytes)
        rules = rules_output.read_bytes()
        if not rules.startswith(b"<!-- version: 25 -->\n"):
            raise ValueError("pinned Go did not emit version 25 rules")
        rules.decode("utf-8")
        observed = json.loads(golden)
        if not isinstance(observed, list) or len(observed) != len(corpus):
            raise ValueError("Go observation count does not match the corpus")
        for case, row in zip(corpus, observed):
            if (not isinstance(row, dict)
                    or set(row) != {"Name", "Injected", "Removed", "Stripped", "Error"}
                    or row["Name"] != case["Name"]
                    or type(row["Error"]) is not bool
                    or row["Error"] != (case["Provider"] in {"grok", "shell"})):
                raise ValueError(f"unexpected Go observation for {case['Name']}")
            for field in ["Injected", "Removed", "Stripped"]:
                base64.b64decode(row[field], validate=True)
    # Only our temporary directory was removed. Recheck after its Go file is gone.
    if source_state(root, env) != source_hashes:
        raise ValueError("Go inputs changed during oracle generation")
    outputs = {"cases.json": case_bytes, "golden.json": golden, "rules.txt": rules}
    provenance = {
        "oracle_sha": ORACLE,
        "go_version": version,
        "case_count": len(corpus),
        "source_verification": "pinned HEAD and clean tracked/untracked Go inputs before and after execution",
        "tracked_go_input_count": len(source_hashes),
        "tracked_go_inputs_sha256": hashlib.sha256(
            json.dumps(source_hashes, sort_keys=True, separators=(",", ":")).encode("utf-8")
        ).hexdigest(),
        "approval_rules_go_sha256": source_hashes["internal/wrapper/approval_rules.go"],
        "generator_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "observer_sha256": hashlib.sha256(observer).hexdigest(),
        "outputs_sha256": {name: hashlib.sha256(data).hexdigest() for name, data in outputs.items()},
    }
    outputs["provenance.json"] = (json.dumps(provenance, indent=2) + "\n").encode("utf-8")
    for name, data in outputs.items():
        (HERE / name).write_bytes(data)
    print(f"Generated {len(corpus)} approval injection/removal cases from Go {ORACLE}")


if __name__ == "__main__":
    main()
