#!/usr/bin/env python3
"""Assemble four native CI candidates; never publish or mark them accepted.

Input is the four target directories emitted by rust-candidate-ci.py. Output
must be new. Both archive and npm staging bytes come from validated receipts.
"""
import argparse
import datetime
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
PACKAGING_INPUTS = ("BUILD-INPUTS.json", "CRATE-NOTICES.txt", "build-inputs.spdx.json")
DOCS = ("README.md", "README.ja.md", "CHANGELOG.md", "LICENSE",
        "web/src/vendor/THIRD_PARTY_LICENSES.txt")
WARNING = ("Unaccepted Rust migration candidate. Archives and npm staging hashes "
           "are checked, but signing, distribution SBOM, installed updates, "
           "rollback and application acceptance remain pending. Not for public release.\n")
RELEASE_WARNING = ("This receipt covers release stamping, archive/binary byte identity and collected build-input evidence. "
                   "Signing, native-device acceptance, rollback and distribution-license review require their separate evidence.\n")


def release_version(tag):
    if not isinstance(tag, str):
        raise ValueError("release tag must be strict SemVer with a v prefix")
    match = re.fullmatch(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
                         r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?", tag)
    if not match or (match[4] and any(part.isdigit() and len(part) > 1 and part[0] == "0"
                                    for part in match[4].split("."))):
        raise ValueError("release tag must be strict SemVer with a v prefix")
    return tag[1:]


def release_source_identity(repo, tag):
    release_version(tag)
    def git(*args):
        return subprocess.check_output(["git", "-C", str(repo), *args], text=True).strip()
    source = git("rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}")
    if not hex_digest(source, 40) or git("rev-parse", "HEAD") != source:
        raise ValueError("release tag commit does not match checkout HEAD")
    timestamp = git("show", "-s", "--format=%ct", source)
    if not re.fullmatch(r"[0-9]+", timestamp):
        raise ValueError("release source commit time is invalid")
    return source, datetime.datetime.fromtimestamp(int(timestamp), datetime.timezone.utc).isoformat()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def hex_digest(value, length=64):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{%d}" % length, value) is not None


def regular(path):
    # Reject directory symlinks too, including target folders under the input.
    if any(part.is_symlink() for part in (path, *path.parents)) or not path.is_file():
        raise ValueError(f"not a regular non-symlink input: {path.name}")
    data = path.read_bytes()
    if not data:
        raise ValueError(f"empty input: {path.name}")
    return data


def verified_entries(folder, entries, names, *, sizes=False):
    if (not isinstance(entries, list)
            or not all(isinstance(e, dict) and isinstance(e.get("name"), str) for e in entries)
            or sorted(e["name"] for e in entries) != sorted(names)):
        raise ValueError("receipt must list exactly the expected distinct files")
    result = {}
    for entry in entries:
        if (not hex_digest(entry.get("sha256")) or (sizes and
                (type(entry.get("bytes")) is not int or entry["bytes"] <= 0))):
            raise ValueError("receipt file hash/size evidence is invalid")
        data = regular(folder / entry["name"])
        if sha(data) != entry.get("sha256") or (sizes and len(data) != entry.get("bytes")):
            raise ValueError(f"receipt hash/size mismatch: {entry['name']}")
        result[entry["name"]] = data
    return result


def load_candidates(inputs, release_tag=None):
    candidates = []
    identity = None
    version = release_version(release_tag) if release_tag is not None else None
    for target, (suffix, goos, goarch) in TARGETS.items():
        folder = inputs / suffix
        receipt_bytes = regular(folder / "BUILD-RECEIPT.json")
        receipt = json.loads(receipt_bytes)
        if not isinstance(receipt, dict):
            raise ValueError(f"native candidate receipt must be an object: {suffix}")
        source = receipt.get("source_sha", "")
        if (type(receipt.get("schema_version")) is not int or receipt["schema_version"] != 2
                or receipt.get("status") != ("native-release-build-not-accepted-for-publication" if release_tag else "native-build-only-not-accepted-for-cutover")
                or receipt.get("target") != target or receipt.get("target_name") != suffix
                or not hex_digest(source, 40)
                or receipt.get("review_head_sha") != source
                or receipt.get("version") != (version if release_tag else f"0.0.0-rust-candidate+{source[:12]}")
                or type(receipt.get("packaging_input_exit_code")) is not int
                or receipt["packaging_input_exit_code"] != 0):
            raise ValueError(f"invalid or incomplete native candidate receipt: {suffix}")
        if release_tag:
            if (receipt.get("build_kind") != "release" or receipt.get("release_tag") != release_tag
                    or receipt.get("build_commit") != source or not isinstance(receipt.get("build_time"), str)):
                raise ValueError("release stamp identity is missing or inconsistent")
        elif receipt.get("build_kind") not in (None, "candidate") or receipt.get("release_tag") is not None:
            raise ValueError("release receipts require explicit release mode")
        lock = receipt.get("cargo_lock_sha256", "")
        launcher = receipt.get("launcher_ui_input_sha256")
        web = receipt.get("generated_web_asset_inputs")
        if (not hex_digest(lock) or not hex_digest(launcher) or not isinstance(web, list)
                or not web or not all(isinstance(e, dict) and isinstance(e.get("path"), str)
                    and e["path"] and hex_digest(e.get("sha256")) for e in web)
                or len({e["path"] for e in web}) != len(web)):
            raise ValueError("missing or invalid lock/Web/launcher-UI input evidence")
        current = (source, receipt["version"], lock, launcher,
                   json.dumps(sorted(web, key=lambda e: e["path"]), sort_keys=True))
        if release_tag:
            current += (receipt["build_time"],)
        if identity is not None and current != identity:
            raise ValueError("mixed source/version/lock/Web/launcher-UI inputs across native targets")
        identity = current
        extension = ".exe" if goos == "windows" else ""
        binaries = verified_entries(folder, receipt.get("binaries"),
                                    [n + extension for n in ("many-ai-cli", "many-ai-cli-launcher")], sizes=True)
        notices = verified_entries(folder, receipt.get("third_party_notices"), NOTICES)
        evidence = (verified_entries(folder / "packaging-inputs", receipt.get("packaging_inputs"),
                                     PACKAGING_INPUTS, sizes=True) if release_tag else {})
        if goos == "windows":
            runtime = receipt.get("windows_runtime")
            if (not isinstance(runtime, dict) or runtime.get("source_sha") != source
                    or runtime.get("embedded_in") != "many-ai-cli.exe"
                    or runtime.get("embedded_bytes_verified") is not True):
                raise ValueError("Windows embedded runtime evidence is missing")
            if release_tag:
                resources = receipt.get("windows_resources")
                if (not isinstance(resources, dict) or resources.get("version") != version
                        or not hex_digest(resources.get("icon_sha256"))):
                    raise ValueError("Windows release product version/icon evidence is missing")
                verified_entries(folder, resources.get("binaries"), list(binaries))
                for entry in resources["binaries"]:
                    if (entry.get("file_version") != version or entry.get("product_version") != version
                            or entry.get("icon_payloads_verified") is not True):
                        raise ValueError("Windows release product version/icon evidence is invalid")
        candidates.append((suffix, goos, goarch, receipt, receipt_bytes, binaries, notices, evidence))
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


def assemble(inputs, output, repo, release_tag=None):
    if output.exists():
        raise ValueError("output must be new; existing artifacts are never overwritten")
    identity, candidates = load_candidates(inputs, release_tag)
    source, version = identity[:2]
    if release_tag:
        if release_source_identity(repo, release_tag) != (source, identity[-1]):
            raise ValueError("release receipts do not match the tag commit/time")
    docs = source_documents(repo, source)
    # Validation and immutable document reads finish before any output mutation.
    output.mkdir(parents=True)
    artifacts, archives = [], []
    for suffix, goos, goarch, receipt, receipt_bytes, binaries, notices, evidence in candidates:
        staged = output / "binaries" / suffix
        staged.mkdir(parents=True)
        for name, data in binaries.items():
            path = staged / name
            path.write_bytes(data)
            if goos != "windows":
                path.chmod(0o755)
            artifacts.append({"type": "Binary", "name": name,
                              "path": path.relative_to(output).as_posix(), "pathBase": "manifest",
                              "goos": goos, "goarch": goarch,
                              "extra": {"ID": name.removesuffix(".exe")}})
        members = {**{name: docs[name] for name in DOCS}, **binaries, **notices,
                   "BUILD-RECEIPT.json": receipt_bytes}
        if release_tag:
            members["THIRD_PARTY_NOTICES.md"] = notices["THIRD-PARTY-NOTICES.md"]
            members["RELEASE-EVIDENCE.txt"] = RELEASE_WARNING.encode()
            members.update({f"packaging-inputs/{name}": data for name, data in evidence.items()})
            held = output / "evidence" / suffix
            (held / "packaging-inputs").mkdir(parents=True)
            (held / "THIRD_PARTY_NOTICES.md").write_bytes(members["THIRD_PARTY_NOTICES.md"])
            (held / "BUILD-RECEIPT.json").write_bytes(receipt_bytes)
            for name, data in evidence.items():
                (held / "packaging-inputs" / name).write_bytes(data)
        else:
            members["NOT-FOR-CUTOVER.txt"] = WARNING.encode()
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
        "schema_version": 1, "status": "release-packaged-not-accepted-for-publication" if release_tag else "candidate-packaged-not-accepted-for-release",
        "build_kind": "release" if release_tag else "candidate", "release_tag": release_tag,
        "build_commit": source, "release_ready": False,
        "build_time": identity[-1] if release_tag else None,
        "source_sha": source, "version": version, "archives": archives,
        "scope": "CI receipt consistency and ZIP byte readback; no signing or channel acceptance",
    }, indent=2) + "\n", encoding="utf-8")
    (output / ("RELEASE-EVIDENCE.txt" if release_tag else "NOT-FOR-CUTOVER.txt")).write_text(
        RELEASE_WARNING if release_tag else WARNING, encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--release-tag", help="assemble only release-stamped receipts for this strict SemVer tag")
    args = parser.parse_args()
    try:
        assemble(args.inputs, args.output, args.repo, args.release_tag)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(2, f"candidate assembly failed: {error}\n")
    print("Four release-stamped archives assembled; publication remains unaccepted." if args.release_tag
          else "Four candidate archives assembled. Public release and cutover remain unaccepted.")


if __name__ == "__main__":
    main()
