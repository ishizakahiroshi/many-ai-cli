# V01 native Windows headless oracle

> 最終更新: 2026-10-07(水) 06:59:46 UTC

The native Rust supervisor in `rust/tests/headless_windows_contracts.rs` runs this
fixture. It builds an overlay-added test against unchanged Go production sources
from `d8fbf8598c3effd4e2f837e43ad6f0488c461de3`, using Go 1.26.8 windows/amd64.
The helper is excluded from ordinary builds/tests by the explicit
`windows && many_ai_v01_oracle` constraint. No provider, account, user data, host
Job adjustment, or process-name termination is used.

`v01_windows_oracle_sources.json` pins raw SHA-256 bytes for the same-module import
closure: 47 Go files across eight packages, eight embedded JSON files, and
`go.mod`/`go.sum`. It includes existing headless tests because `go test -c` compiles
them. The supervisor resolves the current committed HEAD once, checks its complete
expected source/embed membership with ls-tree, and reads raw blobs with cat-file
--batch. Every raw hash must match the fixed Go manifest before materializing a
private build tree. No replacement refs, lazy fetch, hooks, filters, textconv,
working-tree newline normalization or global Git changes are used. This avoids
Windows checkout CRLF conversion while pinning the bytes actually compiled.
The overlay helper is embedded in the Rust test executable and hashed in its
receipt. CI never regenerates the fixed source pin from its candidate.

For a reviewed source-pin regeneration, start from a clean checkout of the exact
Go SHA and run this Python at the Rust repository root; pass the fixed checkout
as the first argument. This reads source and writes only the fixture manifest:

```python
from pathlib import Path
import hashlib, json, re, subprocess, sys
source = Path(sys.argv[1]).resolve()
sha = "d8fbf8598c3effd4e2f837e43ad6f0488c461de3"
assert subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, text=True).strip() == sha
assert not subprocess.check_output(["git", "status", "--porcelain=v1", "--untracked-files=no"], cwd=source)
pending, packages, files, embeds = ["internal/headless"], set(), {}, {}
def pin(path):
    data = path.read_bytes()
    files[path.relative_to(source).as_posix()] = hashlib.sha256(data).hexdigest()
    return data.decode()
while pending:
    package = pending.pop()
    if package in packages:
        continue
    packages.add(package)
    for path in sorted((source / package).glob("*.go")):
        if path.name.endswith("_test.go") and package != "internal/headless":
            continue
        text = pin(path)
        pending.extend(re.findall(r'"many-ai-cli/(internal/[^"\n]+)"', text))
        for line in re.findall(r'^//go:embed\s+(.+)$', text, re.M):
            for pattern in line.split():
                embeds.setdefault(package, []).append(pattern)
                matches = sorted((source / package).glob(pattern))
                assert matches
                for match in matches:
                    pin(match)
for name in ("go.mod", "go.sum"):
    pin(source / name)
manifest = dict(schema=1, go_source_sha=sha,
    go_version="go version go1.26.8 windows/amd64",
    overlay_test="internal/headless/v01_windows_oracle_test.go",
    package="./internal/headless", entry_test="TestV01WindowsOracle",
    build_tag="many_ai_v01_oracle", packages=sorted(packages),
    embed_patterns=embeds, files=dict(sorted(files.items())))
Path("rust/tests/fixtures/core/headless/v01_windows_oracle_sources.json").write_text(
    json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
```

The supervisor generates an overlay JSON with one `Replace` entry: the absolute
virtual `internal/headless/v01_windows_oracle_test.go` path in the private Go
tree maps to the private copy of the compile-time embedded helper. After source
verification, its build command in that private module is:

```text
go test -c -mod=readonly -buildvcs=false -tags=many_ai_v01_oracle -overlay=<private overlay.json> -o <private headless-v01.exe> ./internal/headless
```

Prepared GOCACHE/GOMODCACHE/GOPATH are measured with bounded `go env` before
isolating HOME. Compilation reuses those exact caches, `GOTOOLCHAIN=local`, `GOENV=off`,
`GOWORK=off`, `GOTELEMETRY=off`, `GOPROXY=off`, `GOSUMDB=off`, `CGO_ENABLED=0`, and a
private home/temp root. The executable must run through the native supervisor,
which owns its kill-on-close Job and retained process handles. Do not run it
without that guard.

The helper entry is `-test.run=^TestV01WindowsOracle$ -test.v`. Its environment is
`MANY_AI_V01_ROOT`, `MANY_AI_V01_FIXTURE` (the native Rust test executable),
`MANY_AI_V01_CASE=cancel|deadline`, and `MANY_AI_V01_TIMEOUT_MS=5000`. It waits for
the stdin admission byte `G` after private Job assignment. Both fixture processes
must be alive and both native output streams observed before `go-ready.json` is
published. Readiness allows20s in the cancel case; in the deadline case it must
precede the actual5s product timeout. The cancel case waits for `cancel-run`;
the deadline case uses actual
unchanged `Run` timeout logic.

`go-ready.json` records a real diagnostic second `attachProcessJob` call on the
same live direct child, including native error 5, retained identities, and the
measured private Job UI restriction. The helper does not observe the original
call's discarded `jobErr`; its initial failure is a source-based inference under
the same verified Job policy. This is explicitly recorded as
`original_run_attach_error_directly_observed=false`.

After direct-child exit, the supervisor requests fresh grandchild writes through
both inherited pipes. Go callbacks acknowledge them in `go-stdout-retained` and
`go-stderr-retained`. Only after proving that `Run` is still pending does the
supervisor publish `harness-cleanup` and terminate its retained grandchild handle.
The helper publishes `go-run-returned` immediately when actual `Run` returns,
before native exit waits or final receipt work. The pending observation requires
that marker and the final receipt both to remain absent. Cleanup-marker presence
is captured at actual return, so subsequent harness work cannot mask early return.
`go-result.json` records actual `Run` return, classification and elapsed time.
That release is harness cleanup, not successful Go descendant cleanup. The test
returns normally; the supervisor verifies process exit and an empty owned Job.

A host Job incompatible with the private UI-limited Job is an explicit setup
failure. No breakaway, policy change, skipped test, or synthetic error substitutes
for this native receipt. Rust failed-before-resume prevention and successful-Job
descendant cleanup are separate assertions; neither alone is the Go compound
reproduction.

Retained Windows process handles keep their process objects and IDs alive
through the diagnostic probe; see [Microsoft on PID lifetime](https://devblogs.microsoft.com/oldnewthing/20110107-00/?p=11803).
Termination uses the retained supervisor handles.

Native API references: [nested Jobs](https://learn.microsoft.com/en-us/windows/win32/procthread/nested-jobs),
[attachment requirements](https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-assignprocesstojobobject),
[asynchronous process termination](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-terminateprocess),
and [Go build overlays](https://pkg.go.dev/cmd/go#hdr-Compile_packages_and_dependencies).
