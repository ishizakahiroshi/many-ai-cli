#!/usr/bin/env python3
"""Assemble four native CI candidates; never publish or mark them accepted.

Input is the four target directories emitted by rust-candidate-ci.py. Output
must be new. Both archive and npm staging bytes come from validated receipts.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import zipfile

TARGETS = {
    "x86_64-pc-windows-msvc": ("windows-x64", "windows", "amd64"),
    "x86_64-unknown-linux-gnu": ("linux-x64", "linux", "amd64"),
    "x86_64-apple-darwin": ("macos-intel", "darwin", "amd64"),
    "aarch64-apple-darwin": ("macos-apple-silicon", "darwin", "arm64"),
}
NOTICES = ("THIRD-PARTY-NOTICES.md", "GO-LICENSE")
DOCS = ("README.md", "README.ja.md", "CHANGELOG.md", "LICENSE",
        "web/src/vendor/THIRD_PARTY_LICENSES.txt")
WARNING = ("Unaccepted Rust migration candidate. Archives and npm staging hashes "
           "are checked, but signing, distribution SBOM, installed updates, "
           "rollback and application acceptance remain pending. Not for public release.\n")


def sha(data):
    return hashlib.sha256(data).hexdigest()


def regular(path):
    # Reject directory symlinks too, including target folders under the input.
    if any(part.is_symlink() for part in (path, *path.parents)) or not path.is_file():
        raise ValueError(f"not a regular non-symlink input: {path.name}")
    data = path.read_bytes()
    if not data:
        raise ValueError(f"empty input: {path.name}")
    return data


def verified_entries(folder, entries, names, *, sizes=False):
    if (not isinstance(entries, list) or not all(isinstance(e, dict) for e in entries)
            or sorted(e.get("name", "") for e in entries) != sorted(names)):
        raise ValueError("receipt must list exactly the expected distinct files")
    result = {}
    for entry in entries:
        data = regular(folder / entry["name"])
        if sha(data) != entry.get("sha256") or (sizes and len(data) != entry.get("bytes")):
            raise ValueError(f"receipt hash/size mismatch: {entry['name']}")
        result[entry["name"]] = data
    return result


def load_candidates(inputs):
    candidates = []
    identity = None
    for target, (suffix, goos, goarch) in TARGETS.items():
        folder = inputs / suffix
        receipt_bytes = regular(folder / "BUILD-RECEIPT.json")
        receipt = json.loads(receipt_bytes)
        source = receipt.get("source_sha", "")
        if (receipt.get("schema_version") != 2
                or receipt.get("status") != "native-build-only-not-accepted-for-cutover"
                or receipt.get("target") != target or receipt.get("target_name") != suffix
                or not re.fullmatch(r"[0-9a-f]{40}", source)
                or receipt.get("review_head_sha") != source
                or receipt.get("version") != f"0.0.0-rust-candidate+{source[:12]}"
                or type(receipt.get("packaging_input_exit_code")) is not int
                or receipt["packaging_input_exit_code"] != 0):
            raise ValueError(f"invalid or incomplete native candidate receipt: {suffix}")
        lock = receipt.get("cargo_lock_sha256", "")
        web = receipt.get("generated_web_asset_inputs")
        if (not re.fullmatch(r"[0-9a-f]{64}", lock) or not isinstance(web, list)
                or not web or not all(isinstance(e, dict) and isinstance(e.get("path"), str)
                    and re.fullmatch(r"[0-9a-f]{64}", e.get("sha256", "")) for e in web)
                or len({e["path"] for e in web}) != len(web)):
            raise ValueError("missing or invalid lock/Web input evidence")
        current = (source, receipt["version"], lock,
                   json.dumps(sorted(web, key=lambda e: e["path"]), sort_keys=True))
        if identity is not None and current != identity:
            raise ValueError("mixed source/version/lock/Web inputs across native targets")
        identity = current
        extension = ".exe" if goos == "windows" else ""
        binaries = verified_entries(folder, receipt.get("binaries"),
                                    [n + extension for n in ("many-ai-cli", "many-ai-cli-launcher")], sizes=True)
        notices = verified_entries(folder, receipt.get("third_party_notices"), NOTICES)
        if goos == "windows":
            runtime = receipt.get("windows_runtime")
            if (not isinstance(runtime, dict) or runtime.get("source_sha") != source
                    or runtime.get("embedded_in") != "many-ai-cli.exe"
                    or runtime.get("embedded_bytes_verified") is not True):
                raise ValueError("Windows embedded runtime evidence is missing")
        candidates.append((suffix, goos, goarch, receipt, receipt_bytes, binaries, notices))
    return identity, candidates


def source_documents(repo, source):
    # Immutable source documents, rather than arbitrary dirty checkout contents.
    result = {}
    for name in (*DOCS, "unblock-windows.cmd"):
        result[name] = subprocess.run(["git", "-C", str(repo), "show", f"{source}:{name}"],
                                      check=True, stdout=subprocess.PIPE).stdout
        if not result[name]:
            raise ValueError(f"empty source document: {name}")
    return result


def assemble(inputs, output, repo):
    if output.exists():
        raise ValueError("output must be new; existing artifacts are never overwritten")
    identity, candidates = load_candidates(inputs)
    source, version = identity[:2]
    docs = source_documents(repo, source)
    # Validation and immutable document reads finish before any output mutation.
    output.mkdir(parents=True)
    artifacts, archives = [], []
    for suffix, goos, goarch, receipt, receipt_bytes, binaries, notices in candidates:
        staged = output / "binaries" / suffix
        staged.mkdir(parents=True)
        for name, data in binaries.items():
            path = staged / name
            path.write_bytes(data)
            if goos != "windows":
                path.chmod(0o755)
            artifacts.append({"type": "Binary", "name": name, "path": str(path.resolve()),
                              "goos": goos, "goarch": goarch,
                              "extra": {"ID": name.removesuffix(".exe")}})
        members = {**{name: docs[name] for name in DOCS}, **binaries, **notices,
                   "BUILD-RECEIPT.json": receipt_bytes, "NOT-FOR-CUTOVER.txt": WARNING.encode()}
        if goos == "windows":
            members["unblock-windows.cmd"] = docs["unblock-windows.cmd"]
        name = f"many-ai-cli-{version}-{suffix}.zip"
        path = output / name
        with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for member, data in members.items():
                info = zipfile.ZipInfo(member)
                info.create_system = 3
                info.external_attr = (0o100755 if member in binaries and goos != "windows" else 0o100644) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(info, data)
        with zipfile.ZipFile(path) as archive:
            if set(archive.namelist()) != set(members) or archive.testzip() is not None:
                raise ValueError("archive content readback failed")
            for member, data in members.items():
                if sha(archive.read(member)) != sha(data):
                    raise ValueError("archive hash readback failed")
        archives.append({"name": name, "sha256": sha(path.read_bytes()), "target": receipt["target"]})
    (output / "artifacts.json").write_text(json.dumps(artifacts, indent=2) + "\n", encoding="utf-8")
    (output / "SHA256SUMS.txt").write_text("".join(f"{a['sha256']}  {a['name']}\n" for a in archives), encoding="utf-8")
    (output / "PACKAGE-RECEIPT.json").write_text(json.dumps({
        "schema_version": 1, "status": "candidate-packaged-not-accepted-for-release",
        "source_sha": source, "version": version, "archives": archives,
        "scope": "CI receipt consistency and ZIP byte readback; no signing or channel acceptance",
    }, indent=2) + "\n", encoding="utf-8")
    (output / "NOT-FOR-CUTOVER.txt").write_text(WARNING, encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    assemble(args.inputs, args.output, args.repo)
    print("Four candidate archives assembled. Public release and cutover remain unaccepted.")


if __name__ == "__main__":
    main()
