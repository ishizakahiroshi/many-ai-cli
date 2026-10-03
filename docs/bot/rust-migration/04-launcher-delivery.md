# #3 many-ai-cli rust: launcher and delivery

> 最終更新: 2026-10-03(土) 12:07:39

This is the launcher and distribution implementation brief for the GitHub-connected implementation bot. Start with this directory's README and shared contracts. Implement this brief after the foundation has fixed runtime roots, configuration DTOs, process execution, cancellation, errors, and binary wiring. If instructions conflict with actual source, report the discrepancy and use source and tests as the current behavior reference.

The user-facing task label is `#3 many-ai-cli rust`. Send progress to the integration owner for PROGRESS.md. The operator controls external messages; when directed continue only in the originating Slack or ChatGPT Web #3 conversation. Do not initiate messages or mix it with #1 or #2 tasks. This label is not GitHub issue or pull-request number 3.

The coordinator owns Cargo manifests and lockfiles, shared APIs, binary entry points, build scripts, CI workflows, Makefile, npm package changes, and user-facing release documentation. This unit owns only `rust/src/launcher/` and `rust/delivery/`, including its synthetic fixtures and module tests. These paths are proposed new Rust modules, not existing implementations. Confirm module layout with the foundation owner before writing. Do not edit Go source or another unit's files. Report requested integration changes to the coordinator.

Preserve the existing Web TypeScript application and launcher selection UI. Preserve the current command, wire, profile, persistence, process, and distribution contracts. Do not silently drop features or change stored formats. Do not launch the user's existing Hub, terminate unrelated processes, connect to real SSH endpoints, run subscription providers, collect live usage, change user profiles, publish packages, create release tags, or operate external accounts. Use synthetic identities and data throughout. Build, runtime, browser, and acceptance operations follow the repository's operational guidance and the coordinator's explicit execution assignment.

## Launcher contract and implementation

Read the following current sources before implementing. They are the reference implementation and should remain intact during migration:

- `cmd/many-ai-cli-launcher/main.go`: `--profile`, `--last`, `--ui`, default selection UI, console/banner, signal handling.
- `cmd/many-ai-cli/main.go`: the shared `connect` and `profile-export` command entries.
- `internal/launcher/profile.go`, `profile_test.go`, `connect.go`: YAML/JSON model, validation, selection, atomic save, connection lifecycle.
- `internal/launcher/connector_ssh.go`, `connector_ssh_test.go`, `import.go`, `import_test.go`, `export.go`, `shell.go`, `scan.go`: remote operations and their command/output boundaries.
- `internal/launcher/active.go`, `active_filelock_*.go`, `active_test.go`, `audit_active_lostupdate_test.go`, `pid.go`: concurrent connection registration and stale-process handling.
- `internal/launcher/ui_server.go`, `ui_server_test.go`, `ui/index.html`: authenticated loopback UI and ownership-aware connection actions.
- `internal/launcher/connector_wsl_windows.go`, `connector_wsl_unix.go`, `connector_for.go`, platform console/browser/process adapters: platform behavior.
- `internal/hub/servers.go` and `profiles_fetch.go`: the Hub callers that share the launcher library.

Implement profile load, normalization, validation, selection, atomic save, import/export, active registry, connector factory, browser/console/banner helpers, port allocation, and URL scanning as reusable library APIs. Both standalone launcher and main `connect`, plus the Hub Servers interface, must use these APIs. Provide wiring requirements to the coordinator rather than changing shared entry points yourself.

Keep profile snake_case fields and meanings: `name`, `type`, `distro`, `mode`, `host`, `user`, `ssh_port`, `identity_file`, `token_command`, `binary`, `cwd`, `hub_port`; file fields are `version`, `last_used`, and `profiles`. Preserve normalization of `user@host`, default SSH serve mode, tunnel token-command and port requirements, version handling, duplicate-name validation, and the current validation of leading dashes, whitespace, control characters, and port ranges. The existing validator tolerates a missing `last_used` reference; preserve selection behavior rather than introducing a new save-time rejection.

Keep atomic replacement in the same directory, file synchronization, restrictive permissions, and Windows ACL handling. Maintain read-modify-write serialization and cross-process active-file locking. Test failure behavior without writing actual user configuration. Pass the foundation's explicit runtime root to every launcher profile, active record, lock, temporary file, and process identity operation. A different port alone does not isolate the candidate from an installed Go instance.

Preserve SSH serve, tunnel, and remote profile import/export, including loopback port forwarding, host-label URL decoration, login-shell setup, custom binary/CWD, output scanning, retry and port-mismatch handling, timeout/cancellation, early exit, and cleanup. The existing serve retry policy uses up to five attempts with a port step of 100. Keep quiet Hub callers from echoing token-bearing connection URLs. Treat tokens as opaque values and never include them in diagnostic fixtures or logs.

Construct remote commands with two distinct quoting boundaries: quote each embedded shell value, then quote the entire script as one argument to the remote `bash -lc` or `bash -ilc` invocation. Verify the command as SSH transmits it after joining remote arguments. The script must arrive as a single bash argument, preserve the intended binary/CWD/environment, and perform exactly the intended synthetic operation. Apply the same command-boundary reasoning to profile import, tunnel token-command execution, and scoped cleanup. Preserve validation; a value rejected by the existing public profile contract must remain rejected.

Cleanup must match only the connection owned by the operation, using its binary, port, runtime root, and process identity as appropriate. Preserve literal regular-expression and shell quoting when matching remote processes. Do not broaden cleanup to all processes with the program name. Test that a Go instance, another profile, another port, or another runtime root survives candidate cleanup.

Keep the loopback selection UI, its token/Host/Origin guards, profile editing and import, connect/disconnect/stop/detach semantics, active badges, and connection ownership behavior. Reuse the current UI asset through coordinator-managed embedding. Preserve WSL profile schema and Windows adapters and the non-Windows unsupported behavior. Use a stub executor for WSL command tests; do not launch WSL during this task.

## Launcher fixtures and verification

Tests must cover callers and outcomes, not just quote-helper output. Use a process-executor stub and synthetic profile files under a temporary isolated root:

1. CLI selection and profile round-trip: missing/empty files, unknown version, duplicate names, missing `last_used`, defaults, normalized `user@host`, malformed fields, failed atomic replacement, and concurrent save/active updates.
2. Remote command boundaries: script generation, SSH remote-argument joining, outer shell parsing, and the resulting bash script argument. Test spaces, single quotes, semicolons, Unicode, and explicitly rejected newline/control or leading-dash inputs. Assert that argument and environment values arrive intact and no additional command executes.
3. SSH serve/import/tunnel lifecycle: correct URL, escaped token/host labels, stdout/stderr scanning, port mismatch, bounded retries, cancellation, timeout, early exit, and cleanup after both success and failure. Profile import must preserve JSON parsing and leading output-noise behavior.
4. Ownership and isolation: two candidate roots and a simulated installed instance; register/prune/disconnect/detach/cancel in one must not modify the others. Include PID reuse or stale-record cases consistent with current source behavior.
5. UI and platform behavior: authentication and connection-owner checks, all public entry points, quiet connection output, Windows console handling, non-Windows WSL rejection, and stubbed Windows WSL arguments.

Planned commands, once the coordinator has created the manifest and assigned execution rights:

```text
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo test --locked --manifest-path rust/Cargo.toml launcher
```

If module or crate names differ, request the actual command from the coordinator and report concrete test names and executed counts. A successful command with zero matching tests is not acceptance. The coordinator performs full-workspace checks and builds serially after parallel edits finish. Real browser/SSH/OS acceptance is a separate proof and must remain pending until performed.

## Distribution contract

Read `.goreleaser.yaml`, `.github/workflows/validate.yml`, `release.yml`, `whisper-binaries.yml`, `Makefile`, `winres/`, `web/web.go`, `scripts/stage-npm-binaries.mjs`, `stage-npm-from-release.mjs`, `smoke-npm.mjs`, `sync-npm-version.mjs`, `check-version-sources.mjs`, `check-instrumentation.mjs`, `check-artifact-clean.mjs`, and `npm/many-ai-cli/bin/many-ai-cli.mjs`. Existing workflow and GoReleaser files are the source of current packaging behavior; do not rely on prose alone.

The supported distribution targets are Windows x64, Linux x64, macOS Intel, and macOS Apple Silicon. Do not add Windows 32-bit, Windows ARM64, or Linux ARM64 implicitly. Main and standalone launcher are both migration targets. Current release ZIPs and Linux deb/rpm packages contain both binaries. Current npm platform packages contain the main binary only; preserve this difference unless a separate product decision changes it.

Keep archive suffixes `windows-x64`, `linux-x64`, `macos-intel`, and `macos-apple-silicon`, existing npm platform package names and optional dependency resolution, shim argument/stdio/exit handling, unsupported-platform and missing-optional-package errors, Unix executable permissions, and version synchronization. Preserve Windows icon and version resources, version/commit/build-time metadata, required documentation and licenses/notices, Windows unblock helper, SHA256 checksums, signature/certificate outputs, SBOMs, Homebrew and winget packaging expectations, and Linux package layout.

Use a separate candidate artifact directory and explicit version identity. Do not replace installed binaries or claim a candidate is the stable release. The coordinator determines native target triples, supported toolchain, linkers, runtime licensing, and approved pinned dependencies. Do not assume a local linker or runtime file proves clean CI can produce or redistribute the same artifact.

## Delivery implementation and verification

Under `rust/delivery/`, provide an artifact contract, manifest/adapter, packaging specifications, and synthetic verification for all target/channel combinations. Record source revision, lockfile identity, toolchain/build information, main/launcher artifacts, embedded Web/UI assets, runtime identity/hash, and per-file hashes. If existing npm staging is reused, produce the metadata it actually expects in `artifacts.json`; otherwise provide a narrowly scoped integration change for coordinator review.

The coordinator owns changes outside this unit. Return explicit integration requests for:

- Rust fmt/clippy/tests and both binaries on the supported OS matrix while preserving existing Go and Web comparison checks during migration.
- Reproducible frontend and launcher UI embedding, Windows resources, and clean native runtime acquisition and verification.
- Packaging and npm staging/smoke, archive/package content readback, version wiring, license/notice and SBOM verification.
- Isolated dependency/build/package jobs without publication credentials, followed by independent publish jobs that consume hash-verified artifacts. Publication must not rerun dependency installation or build lifecycle code.
- Existing source instrumentation checks and inspection of unpacked candidate binaries/assets/runtimes for release cleanliness.

Use pinned inputs and existing action-pinning conventions. Keep source checks, artifact checks, and native runtime/ABI acceptance separate. Do not drop old Go checks or treat cross compilation as proof that PTY, launcher, and native voice work on a real OS.

After build permissions and artifacts are available, the coordinator verifies clean-checkout creation, manifest and hash readback, archive/package contents, and npm smoke for every staged target. Planned artifact checks include:

```text
node scripts/check-instrumentation.mjs --release
node scripts/check-artifact-clean.mjs <unpacked-candidate-binaries-assets-runtimes>
```

Select concrete paths before execution. Missing targets, zero staged binaries, or a skipped smoke step are incomplete evidence, not successful packaging. Compare Go and Rust against the same synthetic contracts and explain intentional differences. Preserve old artifact hashes and restoration requirements for the cutover unit. Release publication, global install/update, external manifests, actual cutover, and real-device acceptance remain coordinator-controlled later phases.

## Completion report

Return changed files, launcher and distribution contract coverage, concrete tests and executed counts, command exit results, artifact identities and readback evidence if produced, integration requests, and unperformed acceptance items. Report source-only verification as source-only. Do not claim complete migration, publication, OS acceptance, or cutover from module tests or an implementation PR alone. The independent reviewer must inspect the entire diff, removed behavior, callers, persistence, process cleanup, packaging, and out-of-scope changes before adoption.
