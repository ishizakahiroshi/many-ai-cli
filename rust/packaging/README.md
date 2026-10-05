# Candidate packaging input evidence

This is preparation for K15/A12, not a publisher or a replacement packaging
owner. `src/launcher/delivery.rs` remains the target/channel/manifest contract.
No package, release, installation, account, GUI or native-voice acceptance is
established by these helpers. The fixed Go oracle remains
`21d0bc7935a2c4696fb89ccff2e324157a528c2d`.

## Offline collection from an actual successful build

Python 3.11 or later is required for the standard-library `tomllib`; candidate CI
uses the pinned Python 3.12 toolchain. The synthetic fixtures canonicalize their
newly created temporary root before constructing cache/metadata paths, because
macOS can supply temporary paths through `/var` -> `/private/var`. This does not
relax the collector's no-symlink source/cache checks: aliased source ancestors
remain rejected and have a separate negative regression.

The integration owner captures one complete build's Cargo JSON stdout, preserves
stderr and rendered compiler diagnostics, and checks its exit code independently.
The two commands must use the same clean source, lockfile, Cargo home, toolchain,
target and feature selection as the resulting binaries:

```sh
cargo metadata --locked --manifest-path rust/Cargo.toml --format-version 1 \
  --filter-platform "$TARGET" > cargo-metadata.json
cargo build --locked --manifest-path rust/Cargo.toml --target "$TARGET" \
  --release --bins --message-format=json > cargo-build.jsonl
python3 rust/packaging/collect_inputs.py \
  --lockfile rust/Cargo.lock --metadata cargo-metadata.json \
  --build-messages cargo-build.jsonl --cargo-home "$CARGO_HOME" \
  --target "$TARGET" --source-revision "$SOURCE_SHA" --output new-evidence-dir
```

The integration owner, not this tool, obtains authorized dependencies and checks
clean source/build identity. Use `--offline` only with complete pinned caches.
If `CARGO_HOME` is unset, pass its actual selected absolute location explicitly;
there is deliberately no implicit home discovery. Repeat `--cache-dir` for exact
`registry/cache/<registry-id>` directories when necessary. The paired extracted
sources must remain under `registry/src/<registry-id>` so the archive receipt is
tied to the manifests Cargo actually resolved. Do not point this at installed
application data. The output directory must not already exist.

The collector:

- Requires successful `build-finished` and executable compiler-artifact receipts
  for **both** root binaries; rejects concatenated/failed/incomplete builds.
- Selects dependencies from observed compiler-artifact/build-script-executed IDs,
  cross-checks them with Cargo metadata and `Cargo.lock`, and preserves target
  kind, crate type, feature and freshness observations. It does not label all
  lockfile packages as built, all built packages as linked, or host proc-macro/
  build-script inputs as target runtime libraries.
- Does not invoke Cargo or any package code. It hashes the selected `.crate`
  archive against the lock checksum, verifies every archived source file against
  the actual resolved extracted source, then reads the archive's manifest and
  license/notice/copyright/copying files, including nested native-vendor notices.
- Rejects unsafe archive paths, links, duplicate files, mismatched sources and
  oversized evidence. It never extracts files to disk. Report errors omit local
  cache/home paths. Input/report hashes bind the metadata, complete build log,
  lockfile and verified texts. They do not attest to a hostile mutable build host.
- Includes explicitly reviewed source-header excerpts from `source-notices.json`.
  The current SQLite header entry is tied to the full upstream source-file hash,
  version and line range. A dependency change requires re-review; it cannot
  silently reuse the old header. Archive-wide native notices are conservative
  evidence; Cargo features/build output establish which bundled code was used.

Outputs:

- `BUILD-INPUTS.json`: observations, source and notice hashes, gaps and explicit
  `release_ready: false`.
- `CRATE-NOTICES.txt`: exact UTF-8 texts of verified source notices. A partial
  bundle is retained when some sources are unavailable and labeled as evidence.
- `build-inputs.spdx.json`: generated only when every observed package has verified
  archive/source bytes, a nonempty license expression and discovered notice text.
  This SPDX 2.3 document describes **Cargo build inputs**, not a complete shipped
  distribution. `licenseDeclared` and `licenseConcluded` remain `NOASSERTION`;
  original Cargo expressions are preserved without silently parsing legacy
  expressions or making legal conclusions. Review expressions, source-header-only
  notices and composition before treating it as a distribution SBOM.

Exit 0 means source evidence collection completed, not release/license clearance.
Exit 1 retains the report but blocks source-evidence readiness. Exit 2 means an
invalid build input or output collision; it must not be treated as a successful
receipt. A successful binary build is a separate result even when this step fails.

Synthetic tests (no Cargo/network/installed state):

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s rust/packaging -p 'test_*.py' -v
```

## Source-reviewed gap matrix (2026-10-05)

Baseline reviewed: recovered source `7c2d7d320e04cd1d550d937d97515ff692500c6a`.
The current lock contains 316 registry packages; its SHA256 is
`0466139df475d43177199440af0e9aa2455b26aabe55927cacb2d365f8cd0a12`.
These observations are not a final candidate/CI/release receipt.

| Boundary | Existing source contract | Verified gap / concrete next gate |
|---|---|---|
| Targets and roles | `delivery::Target`, `.goreleaser.yaml`: Windows x64, Linux x64, macOS Intel/Apple Silicon; ZIP and Linux deb/rpm contain both binaries | Candidate CI produces separate loose binary outputs. Assemble same-source four-target receipts into the existing delivery owner; perform archive/package content and hash readback before accepting packaged artifacts. |
| npm | `package_files(..., Npm)`, `scripts/stage-npm-binaries.mjs`, platform `package.json`: main binary only; existing shim forwards argv/stdio and current exit behavior | Keep main-only payload. Coordinator must fix `scripts/smoke-npm.mjs` testing `includes('win32')` against `windows-x64`, missing-binary acceptance and skip-as-success. Real packed binary/shim tests on each native target remain pending. |
| Delivery manifest | Existing schema requires eight binaries, Web and launcher entry assets and four runtime DLL entries, with per-file readback | Nonempty runtime strings and hashes are input identities, not authentication/licensing/ABI proof. Manifest validation is not release readiness and is not yet connected to real assembled channel artifacts. Portable relative paths now reject Windows drives/ADS and normalized aliases on every host. |
| Windows resources | `winres/winres.json`, `winres/winres-launcher.json`, Go release hooks create icon/manifest/version resources | Rust `build.rs` embeds Web/UI and test fixture but does not create corresponding product resources. Coordinator must preserve both executables' icon/manifest/version input identity and inspect native PE output. |
| Windows Whisper runtimes | Go `release.yml` prepares all four DLLs through `fetch_windows_runtime.ps1`; checks Microsoft Authenticode and x64 PE, excludes System32 release fallback | Rust production `application/main_program/serve.rs` passes empty `runtime_payload`; Rust build/CI does not acquire/embed it. Native Windows owner must retain VS-redist provenance, hashes, version, signer and redistribution evidence, wire verified bytes and test on a machine without relying on an already installed VC runtime. Empty payload is not acceptance. |
| Other native voice inputs | `whisper-binaries.yml`, existing managed manifests and download hashes | Archive/runtime/model manifests and source licenses must be retained and associated with actual downloaded bytes. Linux/macOS ABI, model and device acceptance are separate from process-stub tests or four DLL names. No native downloads or model execution performed by this lane. |
| Locked Rust dependencies | `Cargo.lock` includes AWS-LC, bundled SQLite, ring, zlib-rs and transitive Rust sources | Prior `inventory/dependencies.json` has 28 direct entries and an older lock hash. Use actual observed build inputs plus verified archives, not that snapshot, to assemble notices/SBOM. Collector now supplies the reusable evidence step. |
| Native embedded sources | Verified `aws-lc-sys 0.45.0` archive has a compound license expression, root and AWS-LC licenses plus fiat license; `libsqlite3-sys 0.38.2` contains SQLite 3.53.2 | Do not reduce native obligations to wrapper crate MIT/Apache labels. Collector retains nested native texts and the checksum-pinned SQLite public-domain header. Review actual features/native build output, generated code, toolchain and system libraries separately. |
| Notice distribution | ZIP includes root Go `THIRD_PARTY_NOTICES.md` and Web vendor texts; npm platform allowlist includes only main binary; deb/rpm configuration lists binaries | Rust source notice currently describes Go-derived regex only. Candidate CI originally copied that notice and GO-LICENSE only. Add reviewed Rust/license/SBOM materials to each actual candidate channel with explicit coordinated metadata changes; this is a documented addition to notice content, not adding the launcher to npm. |
| SBOM, checksum and signing | GoReleaser defines archive SBOMs and cosign checksum signature/certificate | Cargo build-input SPDX is one ingredient; compose frontend, copied Go data, toolchain/system/native payload identities and bind both binary/archive hashes. Check schema and unpacked contents, then separately sign only when authorized. No signature/publication performed. |
| Credentials and publishing | Go before-hooks run outside explicit publish credentials | The GoReleaser publish step still builds within the credential-bearing process after `--skip=before`. Do not transplant this as Rust build/publish separation: uncredentialed build/package job must hand immutable hash-verified artifacts to publish-only job, with no dependency or build lifecycle execution there. |

An offline lock-wide audit verified all 316 archive checksums and corresponding
extracted sources. 308 packages had discoverable license/notice texts; eight had
no such files in their packaged sources: `jni 0.22.4`, `jni-macros 0.22.4`,
`jni-sys-macros 0.4.1`, `r-efi 5.3.0` / `6.0.0`,
`rustls-platform-verifier-android 0.2.0`, and both
`winapi-*-pc-windows-gnu 0.4.0` packages. This is **lock-wide source inspection**,
not proof these packages build on any of the four supported targets. The collector
selects observed IDs to avoid treating unrelated Android/UEFI/GNU rows as compiled
MSVC/Linux/macOS inputs. Missing evidence for an observed package stays a failure.

Remaining shared changes belong to the integration owner: build/log/metadata CI
wiring, runtime embedding and resources, channel staging/smoke and notice
allowlists, receipt composition and immutable artifact handoff. No Go/Web,
release/package scripts, shared Cargo/build/protocol paths or Git state are
changed by this lane.
