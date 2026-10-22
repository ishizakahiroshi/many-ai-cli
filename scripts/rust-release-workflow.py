#!/usr/bin/env python3
"""Release orchestration, without building or executing product binaries.

The native runner/assembler owns build identity. This helper validates the
caller, immutable handoff and publisher inputs; it does not grant acceptance.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
from urllib.parse import quote
import zipfile

TAG = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?\Z")
SHA = re.compile(r"[0-9a-f]{40}\Z")
REPO = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\Z")
SUFFIXES = ("windows-x64", "linux-x64", "macos-intel", "macos-apple-silicon")


def checked_tag(tag: str) -> str:
    match = TAG.fullmatch(tag)
    if not match or any(x.isdigit() and len(x) > 1 and x.startswith("0")
                        for x in (match.group(4) or "").split(".")):
        raise ValueError("release tag must be a canonical vMAJOR.MINOR.PATCH[-prerelease]")
    return tag


def checked_repo(repo: str) -> str:
    if not REPO.fullmatch(repo) or any(x in (".", "..") for x in repo.split("/")):
        raise ValueError("invalid repository")
    return repo


def run(command: list[str], *, cwd=None) -> str:
    result = subprocess.run(command, cwd=cwd, text=True, capture_output=True, check=False, timeout=60)
    if result.returncode:
        # Preserve diagnostics, but never dump the environment or command arguments.
        sys.stderr.write(result.stdout)
        sys.stderr.write(result.stderr)
        raise ValueError(f"{command[0]} failed ({result.returncode})")
    return result.stdout.strip()


def release_plan(*, event: str, ref: str, event_sha: str, tag: str,
                 npm_only: str, channel: str, head: str, tag_commit: str) -> dict:
    checked_tag(tag)
    if not SHA.fullmatch(tag_commit) or not SHA.fullmatch(head):
        raise ValueError("invalid source commit")
    if event not in ("push", "workflow_dispatch") or npm_only not in ("true", "false"):
        raise ValueError("unsupported release event/mode")
    if channel not in ("legacy", "full", "npm", "homebrew", "winget"):
        raise ValueError("unsupported release channel")
    mode = "full" if event == "push" else (("npm" if npm_only == "true" else "full") if channel == "legacy" else channel)
    if mode == "full":
        # A tag checkout is not enough: dispatching old tag sources from an
        # unrelated branch would sign under that branch's OIDC identity.
        if ref != f"refs/tags/{tag}" or event_sha != tag_commit or head != tag_commit:
            raise ValueError("full release must execute at the requested tag and exact tag commit")
    return {"tag": tag, "version": tag[1:], "mode": mode, "source_sha": tag_commit,
            "scripts_sha": head, "prerelease": "true" if "-" in tag else "false"}


def extract_notes(changelog: str, tag: str) -> str:
    checked_tag(tag)
    lines = changelog.splitlines()
    heading = re.compile(r"^## \[" + re.escape(tag[1:]) + r"\](?:\s.*)?$")
    starts = [i for i, line in enumerate(lines) if heading.fullmatch(line)]
    if len(starts) != 1:
        raise ValueError("CHANGELOG must contain exactly one section for the release tag")
    start = starts[0] + 1
    stop = next((i for i in range(start, len(lines)) if lines[i].startswith("## [")), len(lines))
    body = "\n".join(lines[start:stop]).strip()
    meaningful = re.sub(r"<!--.*?-->", "", body, flags=re.S)
    if not any(line.strip() and not line.lstrip().startswith("#") for line in meaningful.splitlines()):
        raise ValueError("release CHANGELOG section is empty")
    return body + "\n"


def regular(path: Path) -> Path:
    if any(parent.is_symlink() for parent in (path, *path.parents)) or not path.is_file():
        raise ValueError(f"missing or non-regular release input: {path.name}")
    return path


def digest(path: Path) -> str:
    with regular(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def load_sums(directory: Path) -> dict[str, str]:
    sums = {}
    for line in regular(directory / "SHA256SUMS.txt").read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9_.-]+)", line)
        if not match or match.group(2) in sums:
            raise ValueError("unsafe or duplicate checksum entry")
        sums[match.group(2)] = match.group(1)
    if not sums:
        raise ValueError("empty checksums")
    return sums


def verify_archives(directory: Path, tag: str, source: str | None = None) -> list[Path]:
    checked_tag(tag)
    if source is not None and not SHA.fullmatch(source):
        raise ValueError("invalid canonical source identity")
    sums = load_sums(directory)
    archives = [directory / f"many-ai-cli-{tag[1:]}-{suffix}.zip" for suffix in SUFFIXES]
    actual_zips = {p.name for p in directory.glob("*.zip")}
    if actual_zips != {p.name for p in archives}:
        raise ValueError("canonical release needs exactly the four requested-version ZIPs")
    for path in archives:
        if sums.get(path.name) != digest(path):
            raise ValueError(f"archive checksum mismatch: {path.name}")
        if source is not None:
            # Bind release-tag metadata/shims to the signed binary source, so a
            # moved tag cannot silently mix new package inputs with old ZIPs.
            with zipfile.ZipFile(path) as archive:
                entries = [e for e in archive.infolist() if e.filename == "BUILD-RECEIPT.json"]
                if len(entries) != 1 or entries[0].file_size > 2 * 1024 * 1024:
                    raise ValueError("canonical Rust ZIP needs one bounded build receipt")
                receipt = json.loads(archive.read(entries[0]))
            if (receipt.get("build_kind") != "release" or receipt.get("release_tag") != tag
                    or receipt.get("source_sha") != source or receipt.get("build_commit") != source):
                raise ValueError("canonical ZIP source does not match the release tag commit")
    return archives


def prepare_assets(bundle: Path, packages: Path, output: Path, tag: str) -> None:
    """Copy only reviewed canonical inputs; reject partial or mixed packaging."""
    archives = verify_archives(bundle, tag)
    receipt = json.loads(regular(bundle / "PACKAGE-RECEIPT.json").read_text(encoding="utf-8"))
    if (receipt.get("build_kind") != "release" or receipt.get("release_tag") != tag
            or not SHA.fullmatch(receipt.get("build_commit", ""))
            or receipt.get("release_ready") is not False):
        raise ValueError("missing formal native package identity")
    debs = list(packages.glob("*.deb"))
    rpms = list(packages.glob("*.rpm"))
    if "-" in tag and (debs or rpms):
        raise ValueError("prerelease Linux packages are deliberately skipped")
    if "-" not in tag and (len(debs) != 1 or len(rpms) != 1):
        raise ValueError("exactly one deb and rpm are required")
    inputs = archives + debs + rpms
    if output.exists():
        raise ValueError("release asset output must be new")
    for path in inputs:
        regular(path)
    output.mkdir(parents=True)
    for path in inputs:
        shutil.copyfile(path, output / path.name)


def verify_linux_payloads(expected: Path, deb: Path, rpm: Path) -> None:
    def inventory(root: Path) -> dict:
        if root.is_symlink() or not root.is_dir():
            raise ValueError("Linux payload root must be an actual directory")
        files = {}
        for path in root.rglob("*"):
            if path.is_symlink():
                raise ValueError("Linux payload contains a symlink")
            if path.is_file():
                files[path.relative_to(root).as_posix()] = digest(path)
            elif not path.is_dir():
                raise ValueError("Linux payload contains a special file")
        return files
    wanted = inventory(expected / "usr")
    if not {"bin/many-ai-cli", "bin/many-ai-cli-launcher"} <= set(wanted):
        raise ValueError("Linux package input lacks both binaries")
    for output in (deb, rpm):
        if inventory(output / "usr") != wanted:
            raise ValueError("packaged Linux binaries/notices differ from canonical ZIP payload")
        if any(p.name != "usr" for p in output.iterdir()):
            raise ValueError("unexpected Linux package root entry")
        for name in ("many-ai-cli", "many-ai-cli-launcher"):
            if (output / "usr/bin" / name).stat().st_mode & 0o777 != 0o755:
                raise ValueError("packaged Linux binary mode must be 0755")


def complete_sbom(asset: Path, document: Path, tag: str) -> None:
    """Bind Syft observations to one opaque shipped payload, without clearance claims.

    A Rust binary without auditable metadata can have no catalogued packages.
    An explicit distribution package still gives the document a hash-bound root;
    embedded Cargo input evidence remains separate from runtime classification.
    """
    checked_tag(tag)
    data = json.loads(regular(document).read_text(encoding="utf-8"))
    if data.get("spdxVersion") != "SPDX-2.3" or not isinstance(data.get("creationInfo"), dict):
        raise ValueError("Syft did not produce SPDX 2.3")
    root = "SPDXRef-ManyAiDistribution"
    packages = data.setdefault("packages", [])
    if not isinstance(packages, list) or any(p.get("SPDXID") == root for p in packages):
        raise ValueError("duplicate or invalid SPDX root")
    packages.append({"SPDXID": root, "name": asset.name, "versionInfo": tag[1:],
                     "downloadLocation": "NOASSERTION", "filesAnalyzed": False,
                     "licenseConcluded": "NOASSERTION", "licenseDeclared": "NOASSERTION",
                     "copyrightText": "NOASSERTION",
                     "checksums": [{"algorithm": "SHA256", "checksumValue": digest(asset)}],
                     "sourceInfo": "Shipped distribution container; Syft observations and embedded Cargo build-input evidence do not assert complete runtime dependency classification or license clearance."})
    data.setdefault("relationships", []).append({"spdxElementId": data.get("SPDXID", "SPDXRef-DOCUMENT"), "relationshipType": "DESCRIBES", "relatedSpdxElement": root})
    document.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def checksum_assets(directory: Path, tag: str) -> None:
    checked_tag(tag)
    files = sorted(directory.iterdir(), key=lambda p: p.name)
    allowed = re.compile(r"many-ai-cli-[A-Za-z0-9_.+-]+(?:\.zip|\.deb|\.rpm)(?:\.spdx\.json)?\Z")
    if any(not allowed.fullmatch(p.name) for p in files):
        raise ValueError("unexpected file in unsigned asset directory")
    count = 4 if "-" in tag else 6
    if len(files) != count * 2 or sum(p.name.endswith(".spdx.json") for p in files) != count:
        raise ValueError("each required payload needs its distribution SBOM")
    payloads = {p.name for p in files if not p.name.endswith(".spdx.json")}
    sboms = {p.name.removesuffix(".spdx.json") for p in files if p.name.endswith(".spdx.json")}
    if payloads != sboms:
        raise ValueError("distribution SBOMs must pair with the exact payload names")
    expected_zips = {f"many-ai-cli-{tag[1:]}-{suffix}.zip" for suffix in SUFFIXES}
    if {name for name in payloads if name.endswith(".zip")} != expected_zips:
        raise ValueError("distribution payload target/version mismatch")
    if "-" not in tag and (sum(name.endswith(".deb") for name in payloads) != 1 or sum(name.endswith(".rpm") for name in payloads) != 1):
        raise ValueError("stable distribution requires exactly one deb and rpm")
    for path in files:
        regular(path)
        if path.name.endswith(".spdx.json"):
            sbom = json.loads(path.read_text(encoding="utf-8"))
            if sbom.get("spdxVersion") != "SPDX-2.3" or not sbom.get("packages"):
                raise ValueError("distribution SPDX output is missing packages")
            roots = [p for p in sbom["packages"] if p.get("SPDXID") == "SPDXRef-ManyAiDistribution"]
            payload = directory / path.name.removesuffix(".spdx.json")
            if (len(roots) != 1 or roots[0].get("name") != payload.name
                    or roots[0].get("versionInfo") != tag[1:]
                    or roots[0].get("checksums") != [{"algorithm": "SHA256", "checksumValue": digest(payload)}]):
                raise ValueError("distribution SBOM root does not bind its payload/version")
    (directory / "SHA256SUMS.txt").write_text("".join(f"{digest(path)}  {path.name}\n" for path in files), encoding="utf-8")
    verify_archives(directory, tag)


def verify_assets(directory: Path, tag: str) -> None:
    """Checksums are verified only after the caller verifies their cosign signature."""
    verify_archives(directory, tag)
    sums = load_sums(directory)
    names = {p.name for p in directory.iterdir()}
    expected = set(sums) | {"SHA256SUMS.txt", "SHA256SUMS.txt.sig", "SHA256SUMS.txt.pem"}
    if names != expected or len(sums) != (8 if "-" in tag else 12):
        raise ValueError("signed handoff is partial or contains unexpected assets")
    for name, value in sums.items():
        if digest(directory / name) != value:
            raise ValueError(f"signed asset checksum mismatch: {name}")
    for name in ("SHA256SUMS.txt.sig", "SHA256SUMS.txt.pem"):
        if regular(directory / name).stat().st_size == 0:
            raise ValueError("empty checksum signature/certificate")


def artifact_names(source: str, run_id: str, attempt: str) -> list[str]:
    if not SHA.fullmatch(source) or not re.fullmatch(r"[1-9][0-9]*", run_id) or not re.fullmatch(r"[1-9][0-9]*", attempt):
        raise ValueError("invalid native run identity")
    targets = ("x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu", "x86_64-apple-darwin", "aarch64-apple-darwin")
    return [f"rust-release-{target}-{source}-{run_id}-attempt-{attempt}" for target in targets]


def check_native_artifacts(artifacts: list[dict], source: str, run_id: str, attempt: str) -> None:
    expected = set(artifact_names(source, run_id, attempt))
    selected = [a for a in artifacts if a.get("name") in expected and not a.get("expired")]
    if len(selected) != 4 or {a["name"] for a in selected} != expected:
        raise ValueError("native matrix handoff incomplete; rerun all native jobs in this attempt")


class SourceCiPending(ValueError):
    """The latest exact-source CI result has not completed yet."""


def check_source_ci(runs: list[dict], source: str) -> None:
    matching = [r for r in runs if r.get("head_sha") == source and r.get("event") in ("push", "pull_request")]
    latest = max(matching, key=lambda r: (r.get("run_number", 0), r.get("run_attempt", 1)), default=None)
    if not latest or latest.get("status") != "completed":
        raise SourceCiPending("latest exact-source CI is not visible or still running")
    if latest.get("conclusion") != "success":
        raise ValueError("latest completed source CI was unsuccessful for the exact tag commit")


def wait_source_ci(fetch, source: str, *, timeout=600, interval=15,
                   clock=time.monotonic, sleep=time.sleep) -> None:
    """Wait for the newest source runs, never substituting an older green run.

    A tag push can start a new secret scan at the same source while planning is
    already running. Only pending/not-yet-visible runs are retried. Transport
    errors and completed failures remain fail-closed. Requests have a separate
    60-second subprocess bound; the workflow gives this ten-minute deadline a
    fifteen-minute job budget.
    """
    if timeout <= 0 or interval <= 0:
        raise ValueError("CI polling timeout/interval must be positive")
    deadline = clock() + timeout
    workflows = ("validate.yml", "doxguard.yml", "secret-scan.yml")
    while True:
        pending = []
        for workflow in workflows:
            if clock() >= deadline:
                raise ValueError("timed out waiting for latest exact-source CI")
            try:
                check_source_ci(fetch(workflow), source)
            except SourceCiPending:
                pending.append(workflow)
        if not pending:
            return
        remaining = deadline - clock()
        if remaining <= 0:
            raise ValueError("timed out waiting for latest exact-source CI")
        print("Waiting for latest exact-source CI: " + ", ".join(pending), flush=True)
        sleep(min(interval, remaining))


class GitHub:
    """Use gh authentication without placing tokens in arguments or Git config."""
    def api(self, endpoint: str, method="GET", payload=None):
        command = ["gh", "api", "--method", method, endpoint]
        if payload is None:
            return json.loads(run(command) or "null")
        with tempfile.TemporaryDirectory(prefix="release-api-") as temp:
            path = Path(temp) / "request.json"
            path.write_text(json.dumps(payload), encoding="utf-8")
            return json.loads(run(command + ["--input", str(path)]) or "null")


def publish_homebrew(directory: Path, tag: str, gh: GitHub) -> None:
    checked_tag(tag)
    if "-" in tag:
        raise ValueError("Homebrew is stable-only")
    body = regular(directory / "homebrew" / "many-ai-cli.rb").read_bytes()
    endpoint = "repos/ishizakahiroshi/homebrew-tap/contents/Casks/many-ai-cli.rb"
    existing = gh.api(endpoint + "?ref=main")
    if base64.b64decode(existing["content"]) == body:
        print("Homebrew already matches canonical release")
        return
    gh.api(endpoint, "PUT", {"message": f"Update many-ai-cli to {tag}", "branch": "main",
                            "sha": existing["sha"], "content": base64.b64encode(body).decode("ascii")})


def publish_winget(directory: Path, tag: str, gh: GitHub) -> None:
    checked_tag(tag)
    if "-" in tag:
        raise ValueError("winget is stable-only")
    manifests = sorted((directory / "winget").glob("*.yaml"))
    identifier = "ishizakahiroshi.many-ai-cli"
    if {p.name for p in manifests} != {f"{identifier}.yaml", f"{identifier}.installer.yaml", f"{identifier}.locale.en-US.yaml"}:
        raise ValueError("winget needs three generated manifests")
    for path in manifests:
        regular(path)
    repo = "ishizakahiroshi/winget-pkgs"
    try:
        gh.api(f"repos/{repo}/merge-upstream", "POST", {"branch": "master"})
    except ValueError:
        # An optional fork sync must never block the canonical release. If the
        # subsequent pointer publication fails, that channel remains visibly red.
        print("::warning::winget fork sync failed; attempting the canonical pointer update")
    upstream = gh.api("repos/microsoft/winget-pkgs/git/ref/heads/master")["object"]["sha"]
    branch = f"many-ai-cli-{tag[1:]}"
    # Read matching refs without treating authentication/network errors as 404.
    refs = gh.api(f"repos/{repo}/git/matching-refs/heads/{branch}")
    exact = [r for r in refs if r["ref"] == f"refs/heads/{branch}"]
    parent = exact[0]["object"]["sha"] if exact else upstream
    prefix = f"manifests/i/ishizakahiroshi/many-ai-cli/{tag[1:]}/"
    tree = gh.api(f"repos/{repo}/git/commits/{parent}")["tree"]["sha"]
    entries = [{"path": prefix + p.name, "mode": "100644", "type": "blob", "content": p.read_text(encoding="utf-8")} for p in manifests]
    new_tree = gh.api(f"repos/{repo}/git/trees", "POST", {"base_tree": tree, "tree": entries})["sha"]
    if new_tree != tree:
        commit = gh.api(f"repos/{repo}/git/commits", "POST", {"message": f"New version: ishizakahiroshi.many-ai-cli {tag[1:]}", "tree": new_tree, "parents": [parent]})["sha"]
        if exact:
            gh.api(f"repos/{repo}/git/refs/heads/{branch}", "PATCH", {"sha": commit, "force": False})
        else:
            gh.api(f"repos/{repo}/git/refs", "POST", {"ref": f"refs/heads/{branch}", "sha": commit})
    head = f"ishizakahiroshi:{branch}"
    prs = gh.api("repos/microsoft/winget-pkgs/pulls?state=all&base=master&head=" + quote(head, safe=""))
    if any(pr.get("state") == "open" or pr.get("merged_at") for pr in prs):
        print("winget pull request already exists or was merged")
    else:
        gh.api("repos/microsoft/winget-pkgs/pulls", "POST", {"title": f"New version: ishizakahiroshi.many-ai-cli {tag[1:]}", "head": head, "base": "master", "body": f"Update to the checksum-verified canonical release {tag}."})


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    subs = parser.add_subparsers(dest="command", required=True)
    plan = subs.add_parser("plan")
    for name in ("event", "ref", "sha", "tag", "npm-only", "channel", "github-output"):
        plan.add_argument("--" + name, required=True)
    for command in ("verify-archives", "checksums", "verify-assets"):
        sub = subs.add_parser(command)
        sub.add_argument("--directory", type=Path, required=True)
        sub.add_argument("--tag", required=True)
        if command == "verify-archives":
            sub.add_argument("--source")
    assets = subs.add_parser("prepare-assets")
    for name in ("bundle", "packages", "output"):
        assets.add_argument("--" + name, type=Path, required=True)
    assets.add_argument("--tag", required=True)
    linux = subs.add_parser("verify-linux-payloads")
    for name in ("expected", "deb", "rpm"):
        linux.add_argument("--" + name, type=Path, required=True)
    sbom = subs.add_parser("complete-sbom")
    sbom.add_argument("--asset", type=Path, required=True)
    sbom.add_argument("--document", type=Path, required=True)
    sbom.add_argument("--tag", required=True)
    notes = subs.add_parser("notes")
    notes.add_argument("--tag", required=True)
    notes.add_argument("--output", type=Path, required=True)
    native = subs.add_parser("check-native-artifacts")
    for name in ("repository", "source", "run-id", "attempt"):
        native.add_argument("--" + name, required=True)
    ci = subs.add_parser("check-source-ci")
    ci.add_argument("--repository", required=True)
    ci.add_argument("--source", required=True)
    publisher = subs.add_parser("publish-pointer")
    publisher.add_argument("--channel", choices=("homebrew", "winget"), required=True)
    publisher.add_argument("--directory", type=Path, required=True)
    publisher.add_argument("--tag", required=True)
    args = parser.parse_args()
    if args.command == "plan":
        checked_tag(args.tag)
        head = run(["git", "rev-parse", "HEAD"])
        commit = run(["git", "rev-parse", "--verify", f"refs/tags/{args.tag}^{{commit}}"])
        result = release_plan(event=args.event, ref=args.ref, event_sha=args.sha,
                              tag=args.tag, npm_only=args.npm_only, channel=args.channel,
                              head=head, tag_commit=commit)
        result["build_time"] = run(["git", "show", "-s", "--format=%cI", commit])
        with Path(args.github_output).open("a", encoding="utf-8") as output:
            output.write("".join(f"{key}={value}\n" for key, value in result.items()))
    elif args.command == "notes":
        args.output.write_text(extract_notes(Path("CHANGELOG.md").read_text(encoding="utf-8"), args.tag), encoding="utf-8")
    elif args.command == "verify-archives":
        verify_archives(args.directory, args.tag, args.source)
    elif args.command == "prepare-assets":
        prepare_assets(args.bundle, args.packages, args.output, args.tag)
    elif args.command == "checksums":
        checksum_assets(args.directory, args.tag)
    elif args.command == "verify-linux-payloads":
        verify_linux_payloads(args.expected, args.deb, args.rpm)
    elif args.command == "complete-sbom":
        complete_sbom(args.asset, args.document, args.tag)
    elif args.command == "verify-assets":
        verify_assets(args.directory, args.tag)
    elif args.command == "check-native-artifacts":
        checked_repo(args.repository)
        artifact_names(args.source, args.run_id, args.attempt)
        response = GitHub().api(f"repos/{args.repository}/actions/runs/{args.run_id}/artifacts?per_page=100")
        check_native_artifacts(response["artifacts"], args.source, args.run_id, args.attempt)
    elif args.command == "check-source-ci":
        checked_repo(args.repository)
        if not SHA.fullmatch(args.source):
            raise ValueError("invalid CI source identity")
        def fetch(workflow):
            response = GitHub().api(f"repos/{args.repository}/actions/workflows/{workflow}/runs?head_sha={args.source}&per_page=100")
            return response["workflow_runs"]
        wait_source_ci(fetch, args.source)
    elif args.command == "publish-pointer":
        {"homebrew": publish_homebrew, "winget": publish_winget}[args.channel](args.directory, args.tag, GitHub())


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, json.JSONDecodeError, zipfile.BadZipFile, subprocess.TimeoutExpired) as error:
        print(f"release workflow blocked: {error}", file=sys.stderr)
        raise SystemExit(1)
