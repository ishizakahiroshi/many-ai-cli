---
type: reference
status: draft
tags: [rust, recovery, dots]
owner: unknown
review_status: draft
related: [REVIEW.md, PROGRESS.md]
last_reviewed: 2026-10-05
---

# #3 many-ai-cli Rust: recovery continuation

This continues existing task #3, not GitHub issue/PR number 3 and not a second concurrent rewrite. The previous environment was lost; its recovery handoff reported that implementation/publication had stopped. Before resuming, acknowledge whether another #3 worker is active. Do not start a duplicate worker.

Repository: `ishizakahiroshi/many-ai-cli`. Existing draft PR: #9, `feat/rust-migration` → `develop`. This instruction is prepared locally and has not been published or delivered. The coordinator must publish the reviewed recovered source together with these instructions on a dedicated recovery branch and provide its actual immutable commit URL. The old PR head alone does not contain this recovery candidate. Read the actual supplied commit before editing; report its SHA and working branch in the acknowledgment.

## Goal and current point

Finish the Rust implementation and reproducible automated acceptance against fixed Go behavior SHA `21d0bc7935a2c4696fb89ccff2e324157a528c2d`. Preserve the original contracts in `../rust-migration/00-contracts.md` and integration instructions in `../rust-migration/05-integration-review.md`; these contain K01–K15, A01–A12, R01–R03 and V01–V08 and are referenced for their full behavior matrix. Source and tests at the fixed Go SHA remain the behavior oracle. Do not delegate application behavior back to a Go executable.

The recovered 775 checkpoint was preserved separately and its historical test success was not reproducible without repairs. Its 29 later recovered files have been inspected and integrated in the local candidate, along with missing application owners and regression fixes. The complete historical 1422-test source is still unavailable. Do not claim it was recovered or use 1422 as a target test count.

The latest local candidate has 1151 all-target tests passing, zero failures/ignored tests, strict Clippy passing, three doctests passing and fmt passing on Windows with Rust1.90.0 / Go1.26.8. Its 698-entry source digest is `307d2b5ecc7253dc73ba8f638a8d99fe53534e29aa86aafd319bc25e59276d78`. This digest identifies a local source-entry manifest, not a Git commit or published artifact. The local coordinator also built both release binaries, exercised the main binary's isolated CLI/HTTP/WebSocket/settings/shutdown path, inspected its desktop Chrome settings/version screen, and checked launcher `--help`. These are scope-limited local receipts, not a clean four-OS CI or full product acceptance. Reproduce your own checks at the supplied Git SHA.

Start by reading `rust/README.md`, `rust/inventory/recovery-current-20261005.json`, its notes, `rust/inventory/SHARED-API.md`, actual `rust/src/bin/**`, `application/main_program/**`, and `application/launcher_program.rs`. The current audit maps 155 HTTP registrations, 49 historical lexical WS rows and 32 CLI entries to owners. Those counts are source binding, not method/suffix/behavior acceptance. Three WS rows are DTO discriminator false positives; outgoing `session_dismiss` is an internal relay operation. Preserve the historical `rust/src/hub/route_coverage.json` rather than relabeling it to hide old gaps.

The locally checked Windows binary identities were main SHA256 `9fafd4a2827b2a7104c93ce063c65007bdc13019b4018e283159628e760f017c` and launcher SHA256 `a2e72cfc4fe0e0e0a385104e67ec5c5c6ff680230d5d3b9bec31fd5167b11231`. These identify prior local receipts; they are not promised hashes for your new build with different build metadata/environment or subsequent fixes.

## Work and ownership

1. Establish the exact provided recovered source SHA, compare it to the fixed Go oracle and old PR head, and produce a remaining behavior/acceptance matrix. Reuse existing integrated owners; do not restart modules from the old PR or overwrite the recovered work.
2. Fix demonstrated implementation gaps and failing caller/negative-path tests. Cover HTTP method/prefix suffix branches, auth/Host/Origin, first-frame WS/registration/relay/reconnect/inputACK, instruction injection/recovery/cleanup, admission/cancellation/drain, persisted approvals and DB ownership. Report unsupported configuration/native branches explicitly instead of treating a 503 response as successful service acceptance.
3. Complete clean build/test/artifact validation with `.github/workflows/rust-migration.yml` and `scripts/rust-candidate-ci.py`. The latter deliberately refuses a dirty checkout before building: use the actual committed candidate, not an old HEAD label for uncommitted bytes. Keep Windows, Linux and the two macOS architectures separate. If your environment cannot download dependencies, use permitted repository CI and report the limitation; do not repeatedly request unavailable sandbox downloads.
4. Review the two-binary packaging/channel inputs, native runtime payloads, third-party notices/licenses and SBOM against actual build inputs. Source ports and an empty Windows whisper payload do not establish packaged native voice acceptance. Prepare changes and receipts without publishing any package/release.
5. Submit a draft PR against `develop` with exact code SHA, independent review SHA, CI runs and artifact hashes. Preserve/reconcile existing PR#9 with the coordinator; do not silently force-push or start competing PRs from stale source.

Disjoint implementation lanes may run in parallel: core session/storage/approval; HTTP/service owners; launcher/packaging preparation. One integration owner exclusively owns Cargo.toml/lock, lib/bin/shared protocol and router, CI and shared progress files. Serialize Git and shared integration. An independent reviewer must read the actual final diff and re-review fixes; see REVIEW.md.

Allowed change scope: `rust/**`, `scripts/rust-candidate-ci.py`, `.github/workflows/rust-migration.yml`, these task instructions/progress, and explicitly necessary packaging metadata after documenting the affected paths. Preserve existing Go source, Web TypeScript and unrelated dirty changes. If another path is required, document the concrete caller/gap and coordinate ownership before editing it.

## Reproducible validation

From a clean candidate checkout, build the frontend with its locked Bun1.3.14 configuration before Cargo. `rust/build.rs` validates and embeds `web/dist` and the existing launcher HTML. Use the four-target workflow's commands/toolchains as the canonical clean-run entry. The Rust commands are:

```text
cargo +1.90.0 fmt --manifest-path rust/Cargo.toml -- --check
cargo +1.90.0 clippy --manifest-path rust/Cargo.toml --locked --all-targets -- -D warnings
cargo +1.90.0 test --manifest-path rust/Cargo.toml --locked --all-targets -- --test-threads=8
cargo +1.90.0 test --manifest-path rust/Cargo.toml --locked --doc
cargo +1.90.0 build --manifest-path rust/Cargo.toml --locked --release --bins
```

Use `--offline` only when all pinned inputs are already present. Preserve full failure logs, exit codes, test summaries, skips, source commit, lockfile/assets hashes and both binary hashes. Test generators must use Go1.26.8 and the frozen oracle, synthetic data and selected build/cache paths. Dependency upgrades are not a workaround for compatibility failures without evidence and provenance.

Trial startup requires explicit disjoint runtime root/port/cwd and synthetic child home/environment. Never share the running Go installation's config, token, DB/WAL/SHM, profile, registry, port, PID, temp or logs. Trial rejects actual provider probes, external approval-pattern reads, GUI dispatch, arbitrary external PID kills and real push delivery. Do not remove these guards to make fixtures pass. Local source checks must use held files/capabilities and reject aliases outside the selected root.

HTTP smoke JSON must accept legal empty property keys (`child_permission_preview` default tier). Origin CSRF rejection must be tested on a state-changing method; fixed Go permits ordinary GET Origin. For WS cleanup, verify first-frame/UI snapshots and send a bounded client close-output; do not demand an application close-ack receipt that fixed Go's `x/net/websocket` handler does not promise. Actual Hub HTTP shutdown, zero process exit, ledger/listener cleanup and unchanged synthetic installation remain required.

The copied-data rollback regression closes the Rust writer, preserves DB/WAL/SHM/config bytes and presence, mutates/reopens/restores a synthetic copy, then verifies fixed Go read/continuation and Rust reread/FTS. This is not rollback of the user's installed data or binary. Product provider/account, mobile/device, SSH, native launcher GUI/remote, installed-data migration/rollback, actual uninstall/COM/tray and soak acceptance belong to local/authorized target owners; report these pending if your environment cannot exercise them.

## Receipt and boundaries

Update PROGRESS.md at start, checkpoint, failure, fix and final review. Reply in the existing #3 conversation with the actual instruction SHA, code/review SHA, changed paths, commands/exit results, PR/CI/artifact links, unperformed gates and the next concrete action. Mask private absolute home paths in public errors. Never publish tokens, credentials, private conversation locators, real data or local private evidence paths.

The first reply must acknowledge #3/collision state, readability of the supplied immutable instruction/source commit, environment/OS/toolchain and dependency/CI capability, planned disjoint owners and independent reviewer. Do not claim delivery, download, test, review or native acceptance from preparation alone.

No running-Go cutover, merge, tag, release, registry publication, account/provider usage, real notifications, credential use, destructive installation/uninstall or production changes are authorized by this continuation. Prepare reviewable changes and leave those gates explicitly pending.
