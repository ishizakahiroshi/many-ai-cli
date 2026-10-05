#!/usr/bin/env python3
"""Offline Cargo build-input evidence; never fetches, builds, installs or publishes.

Exit 0: observed crate sources and license texts verified (not release acceptance).
Exit 1: incomplete evidence, with a retained report. Exit 2: invalid input.
"""
from __future__ import annotations

import argparse
import datetime
import hashlib
import io
import json
import re
import sys
import tarfile
import tomllib
from pathlib import Path, PurePosixPath
from urllib.parse import quote

REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
TARGETS = {"x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu",
           "x86_64-apple-darwin", "aarch64-apple-darwin"}
LEGAL = re.compile(r"^(licen[cs]e|copying|copyright|notice|unlicense)(?:$|[._-])", re.I)
MAX_ARCHIVE = 256 * 1024 * 1024
MAX_TEXT = 8 * 1024 * 1024
MAX_TEXT_TOTAL = 32 * 1024 * 1024
SOURCE_NOTICES = Path(__file__).with_name("source-notices.json")


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_regular(path: Path, limit: int) -> bytes:
    # These are explicitly selected build/cache inputs, never home discovery.
    if path.is_symlink() or not path.is_file():
        raise ValueError("input must be a regular non-symlink file")
    with path.open("rb") as source:
        data = source.read(limit + 1)
    if len(data) > limit:
        raise ValueError("input exceeds evidence size limit")
    return data


def load_json(path: Path) -> tuple[object, str]:
    data = read_regular(path, MAX_ARCHIVE)
    return json.loads(data), sha(data)


def safe_member(name: str, prefix: str) -> str:
    path = PurePosixPath(name)
    if ("\\" in name or path.is_absolute() or not path.parts
            or any(part in ("", ".", "..") for part in name.split("/"))
            or path.parts[0] != prefix):
        raise ValueError("unsafe crate archive member")
    return "/".join(path.parts[1:])


def crate_evidence(package: dict, cache_dirs: list[Path]) -> tuple[dict, list[dict]]:
    name, version = package["name"], package["version"]
    if not re.fullmatch(r"[A-Za-z0-9_-]+", name) or not re.fullmatch(r"[0-9A-Za-z.+-]+", version):
        raise ValueError("unsafe crate identity")
    filename = f"{name}-{version}.crate"
    archives = [directory / filename for directory in cache_dirs
                if (directory / filename).exists() or (directory / filename).is_symlink()]
    if not archives:
        return {"status": "missing-archive", "problems": ["locked crate archive is unavailable"]}, []
    # Ambiguous caches cannot silently choose between different input bytes.
    blobs = [read_regular(path, MAX_ARCHIVE) for path in archives]
    if any(sha(data) != package["checksum"] for data in blobs):
        return {"status": "checksum-mismatch", "problems": ["crate archive does not match Cargo.lock"]}, []
    manifest_path = Path(package.pop("manifest_path"))
    source_root = manifest_path.parent
    expected_roots = [directory.parent.parent / "src" / directory.name / f"{name}-{version}"
                      for directory in cache_dirs]
    # Tie archive evidence to the source tree Cargo actually resolved. Never read
    # arbitrary metadata paths, and never claim an altered extracted cache matches.
    if source_root not in expected_roots or manifest_path.name != "Cargo.toml":
        raise ValueError("Cargo source is outside the selected registry cache")
    for part in (source_root, *source_root.parents):
        if part.is_symlink():
            raise ValueError("symlink in Cargo source path")
    contents: dict[str, bytes] = {}
    source_notices = []
    members: dict[str, tarfile.TarInfo] = {}
    total = 0
    with tarfile.open(fileobj=io.BytesIO(blobs[0]), mode="r:gz") as archive:
        for member in archive:
            # Cargo archives can contain directory entries with a trailing '/'.
            relative = safe_member(member.name.rstrip("/") if member.isdir() else member.name,
                                   f"{name}-{version}")
            if member.isdir():
                continue
            if relative in members:
                raise ValueError("duplicate crate archive member")
            members[relative] = member
        verified_source_files = 0
        for relative, member in members.items():
            if not member.isfile() or member.size > MAX_ARCHIVE:
                raise ValueError("unsafe crate source file")
            source_file = source_root.joinpath(*PurePosixPath(relative).parts)
            for parent in source_file.parents:
                if parent == source_root:
                    break
                if parent.is_symlink():
                    raise ValueError("symlink in extracted crate source")
            handle = archive.extractfile(member)
            assert handle is not None
            archived = handle.read(MAX_ARCHIVE + 1)
            if sha(read_regular(source_file, MAX_ARCHIVE)) != sha(archived):
                raise ValueError("resolved Cargo source differs from locked crate archive")
            verified_source_files += 1
        manifest = members.get("Cargo.toml")
        if manifest is None or not manifest.isfile() or manifest.size > MAX_TEXT:
            raise ValueError("crate manifest is absent or unsafe")
        handle = archive.extractfile(manifest)
        assert handle is not None
        manifest_bytes = handle.read(MAX_TEXT + 1)
        source_package = tomllib.loads(manifest_bytes.decode("utf-8"))["package"]
        if (source_package.get("name"), source_package.get("version")) != (name, version):
            raise ValueError("crate manifest identity does not match Cargo.lock")
        license_file = source_package.get("license-file")
        if license_file:
            if not isinstance(license_file, str):
                raise ValueError("invalid crate license-file")
            safe_member(f"{name}-{version}/{license_file}", f"{name}-{version}")
        selected = {path for path in members if LEGAL.match(PurePosixPath(path).name)}
        if license_file:
            selected.add(license_file)
        for path in sorted(selected):
            member = members.get(path)
            if member is None or not member.isfile() or member.size > MAX_TEXT:
                raise ValueError("crate license/notice file is absent or unsafe")
            handle = archive.extractfile(member)
            assert handle is not None
            data = handle.read(MAX_TEXT + 1)
            total += len(data)
            if len(data) > MAX_TEXT or total > MAX_TEXT_TOTAL:
                raise ValueError("crate license/notice text exceeds evidence size limit")
            data.decode("utf-8")  # Never silently replace source license bytes.
            contents[path] = data
        supplements = json.loads(SOURCE_NOTICES.read_text(encoding="utf-8"))
        for supplement in supplements["notices"]:
            if (supplement["package"], supplement["version"]) != (name, version):
                continue
            path = supplement["path"]
            member = members.get(path)
            if member is None or not member.isfile() or member.size > MAX_ARCHIVE:
                raise ValueError("pinned source notice is absent")
            handle = archive.extractfile(member)
            assert handle is not None
            source = handle.read(MAX_ARCHIVE + 1)
            if sha(source) != supplement["sha256"]:
                raise ValueError("pinned source notice changed; review it before updating")
            lines = source.decode("utf-8").splitlines(keepends=True)
            first, last = supplement["first_line"], supplement["last_line"]
            if not 1 <= first <= last <= len(lines):
                raise ValueError("invalid source notice excerpt")
            excerpt = "".join(lines[first - 1:last]).encode("utf-8")
            label = f"{path} (source notice lines {first}-{last})"
            contents[label] = excerpt
            source_notices.append({**supplement, "excerpt_sha256": sha(excerpt)})
    declared = source_package.get("license")
    problems = []
    if not isinstance(declared, str) or not declared.strip():
        problems.append("Cargo license expression is missing; manual review required")
    if not contents or not any(data.strip() for data in contents.values()):
        problems.append("no nonempty license/notice text found; source-header review required")
    files = [{"path": path, "sha256": sha(data), "bytes": len(data)}
             for path, data in contents.items()]
    return {"status": "verified" if not problems else "license-review-required",
            "archive_sha256": sha(blobs[0]), "archive_bytes": len(blobs[0]),
            "manifest_sha256": sha(manifest_bytes), "verified_source_files": verified_source_files,
            "license_expression": declared,
            "license_file": license_file, "license_files": files,
            "source_notice_excerpts": source_notices, "problems": problems}, [
                {**entry, "text": contents[entry["path"]].decode("utf-8")} for entry in files]


def observed_inputs(metadata: dict, lines: bytes, locked: dict) -> tuple[list[dict], list[dict]]:
    by_id = {package["id"]: package for package in metadata["packages"]}
    if len(by_id) != len(metadata["packages"]):
        raise ValueError("duplicate Cargo metadata package ID")
    root_id = metadata.get("resolve", {}).get("root")
    if root_id not in by_id or by_id[root_id].get("source") is not None:
        raise ValueError("Cargo metadata must resolve the local root package")
    if by_id[root_id]["name"] != "many-ai-cli":
        raise ValueError("unexpected Cargo root package")
    seen: dict[str, list[dict]] = {}
    root_artifacts = []
    finished = False
    for line in lines.splitlines():
        if not line.strip():
            continue
        message = json.loads(line)
        if finished:
            raise ValueError("data after Cargo build-finished; use one complete build log")
        reason = message.get("reason")
        if reason == "build-finished":
            if message.get("success") is not True:
                raise ValueError("Cargo build did not finish successfully")
            finished = True
        elif reason in ("compiler-artifact", "build-script-executed"):
            package_id = message["package_id"]
            if package_id not in by_id:
                raise ValueError("build package is missing from Cargo metadata")
            target = message.get("target", {})
            observation = {"reason": reason, "target_name": target.get("name"),
                           "target_kind": target.get("kind", []),
                           "crate_types": target.get("crate_types", []),
                           "features": message.get("features", []),
                           "fresh": message.get("fresh"), "profile": message.get("profile"),
                           "artifact_basenames": [PurePosixPath(path.replace("\\", "/")).name
                                                  for path in message.get("filenames", [])]}
            seen.setdefault(package_id, []).append(observation)
            if package_id == root_id and reason == "compiler-artifact":
                if "bin" in target.get("kind", []) and message.get("executable"):
                    executable = PurePosixPath(message["executable"].replace("\\", "/")).name
                    if message.get("profile", {}).get("test") is not False:
                        raise ValueError("root test harness is not a deliverable binary")
                    if executable not in (target["name"], target["name"] + ".exe"):
                        raise ValueError("root executable basename does not match its role")
                    root_artifacts.append({**observation, "executable_basename": executable})
    if not finished:
        raise ValueError("Cargo build-finished success receipt is missing")
    if {entry["target_name"] for entry in root_artifacts} != {"many-ai-cli", "many-ai-cli-launcher"}:
        raise ValueError("successful build receipts for both root binaries are required")
    inputs = []
    for package_id in sorted(seen):
        package = by_id[package_id]
        if package_id == root_id:
            continue
        key = (package["name"], package["version"], package.get("source"))
        if key not in locked or key[2] != REGISTRY:
            raise ValueError("observed dependency is absent from the official locked registry inputs")
        inputs.append({"name": key[0], "version": key[1], "source": key[2],
                       "package_id": package_id, "manifest_path": package["manifest_path"],
                       "checksum": locked[key]["checksum"],
                       "observations": seen[package_id]})
    if not inputs:
        raise ValueError("no dependency compiler artifacts were observed")
    return inputs, root_artifacts


def spdx_document(report: dict, created: str) -> dict:
    packages = []
    for entry in report["packages"]:
        identifier = "SPDXRef-Crate-" + sha(entry["package_id"].encode())[:24]
        packages.append({"SPDXID": identifier, "name": entry["name"],
                         "versionInfo": entry["version"], "filesAnalyzed": False,
                         "downloadLocation": f"https://static.crates.io/crates/{entry['name']}/{entry['name']}-{entry['version']}.crate",
                         "checksums": [{"algorithm": "SHA256", "checksumValue": entry["checksum"]}],
                         "licenseConcluded": "NOASSERTION", "licenseDeclared": "NOASSERTION",
                         "copyrightText": "NOASSERTION",
                         "externalRefs": [{"referenceCategory": "PACKAGE-MANAGER", "referenceType": "purl",
                                           "referenceLocator": f"pkg:cargo/{entry['name']}@{quote(entry['version'], safe='')}"}],
                         "comment": "Observed Cargo build input, not proof of binary linkage. Cargo license expression: "
                                    + entry["license_expression"] + ". Verified texts are in CRATE-NOTICES.txt; SPDX expression/legal review remains pending."})
    identity = sha((json.dumps(report, sort_keys=True) + created).encode())
    return {"spdxVersion": "SPDX-2.3", "dataLicense": "CC0-1.0", "SPDXID": "SPDXRef-DOCUMENT",
            "name": "many-ai-cli candidate Cargo build inputs (not a complete distribution SBOM)",
            "documentNamespace": "https://spdx.org/spdxdocs/many-ai-cli-build-inputs-" + identity,
            "creationInfo": {"created": created, "creators": ["Tool: many-ai-cli-collect-inputs-1"]},
            "documentComment": "Contains observed target and host build/proc-macro inputs. Does not claim all packages are linked. Excludes toolchain/system libraries, frontend, copied Go data and downloaded native payloads; compose and review before distribution.",
            "packages": packages,
            "relationships": [{"spdxElementId": "SPDXRef-DOCUMENT", "relationshipType": "DESCRIBES",
                               "relatedSpdxElement": package["SPDXID"]} for package in packages]}


def collect(lockfile: Path, metadata_path: Path, messages_path: Path,
            cache_dirs: list[Path], target: str, source_revision: str) -> tuple[dict, str]:
    if target not in TARGETS or not re.fullmatch(r"[0-9a-f]{40}", source_revision):
        raise ValueError("supported target and lowercase 40-character source revision are required")
    lock_bytes = read_regular(lockfile, MAX_ARCHIVE)
    all_locked = tomllib.loads(lock_bytes.decode("utf-8"))["package"]
    locked = {(p["name"], p["version"], p.get("source")): p for p in all_locked}
    if len(locked) != len(all_locked):
        raise ValueError("duplicate Cargo.lock package identity")
    metadata, metadata_sha = load_json(metadata_path)
    if not isinstance(metadata, dict):
        raise ValueError("Cargo metadata must be an object")
    messages = read_regular(messages_path, MAX_ARCHIVE)
    inputs, root_artifacts = observed_inputs(metadata, messages, locked)
    notices = ["Verified source license/notice texts for observed Cargo build inputs.\n"
               "This is evidence collection, not legal clearance or a complete distribution notice.\n"]
    for package in inputs:
        try:
            evidence, texts = crate_evidence(package, cache_dirs)
        except (ValueError, OSError, KeyError, TypeError, tarfile.TarError, tomllib.TOMLDecodeError, UnicodeError) as error:
            # Never expose local cache/home paths or package-controlled text in reports.
            evidence, texts = {"status": "invalid-archive", "problems": [
                "archive inspection failed (" + type(error).__name__ + "); inspect selected input locally"]}, []
        package.pop("manifest_path", None)
        package.update(evidence)
        for entry in texts:
            notices.append(f"\n===== {package['name']} {package['version']} / {entry['path']} =====\n"
                           f"SHA256: {entry['sha256']}\n\n{entry['text']}\n")
    complete = all(package["status"] == "verified" for package in inputs)
    return {"schema_version": 1, "status": "complete-source-evidence" if complete else "incomplete-source-evidence",
            "release_ready": False, "source_revision": source_revision, "declared_target": target,
            "cargo_lock_sha256": sha(lock_bytes), "metadata_sha256": metadata_sha,
            "build_messages_sha256": sha(messages), "locked_registry_packages": sum(p.get("source") == REGISTRY for p in all_locked),
            "observed_registry_packages": len(inputs), "root_binary_observations": root_artifacts,
            "packages": inputs,
            "pending": ["distribution SPDX/license review including source-header-only notices",
                        "toolchain and system-library provenance", "frontend and copied Go-data notices",
                        "native runtime payload provenance, redistribution evidence and ABI acceptance",
                        "channel notice inclusion and archive/npm content readback"]}, "".join(notices)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("lockfile", "metadata", "build-messages", "output"):
        parser.add_argument("--" + option, type=Path, required=True)
    parser.add_argument("--cargo-home", type=Path)
    parser.add_argument("--cache-dir", type=Path, action="append", default=[])
    parser.add_argument("--target", choices=sorted(TARGETS), required=True)
    parser.add_argument("--source-revision", required=True)
    args = parser.parse_args()
    try:
        caches = [path.absolute() for path in args.cache_dir]
        if args.cargo_home:
            caches.extend(sorted((args.cargo_home.absolute() / "registry" / "cache").glob("*")))
        report, notices = collect(args.lockfile, args.metadata, args.build_messages,
                                  caches, args.target, args.source_revision)
        args.output.mkdir(parents=True, exist_ok=False)
        (args.output / "BUILD-INPUTS.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        (args.output / "CRATE-NOTICES.txt").write_text(notices, encoding="utf-8")
        complete = report["status"] == "complete-source-evidence"
        if complete:
            created = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
            (args.output / "build-inputs.spdx.json").write_text(
                json.dumps(spdx_document(report, created), indent=2) + "\n", encoding="utf-8")
        print(f"{report['status']}: {report['observed_registry_packages']} observed / "
              f"{report['locked_registry_packages']} locked registry packages; release acceptance remains pending")
        return 0 if complete else 1
    except (ValueError, OSError, KeyError, TypeError, tomllib.TOMLDecodeError) as error:
        print("invalid packaging evidence input (" + type(error).__name__ + "); inspect selected files locally", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
