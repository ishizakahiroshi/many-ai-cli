# #3 many-ai-cli rust: progress

> 最終更新: 2026-10-03(土) 16:19:11

Task label: #3 many-ai-cli rust. This is a coordination label, not GitHub issue/PR number 3. Continue replies only in the thread where the operator starts this task; do not mix #1/#2 tasks. The operator starts dots from Slack or ChatGPT Web. This repository file and implementation diff are the reviewable progress sources.

Baseline Go SHA: 21d0bc7935a2c4696fb89ccff2e324157a528c2d.
Instruction/implementation branch: feat/rust-migration.
PR base: develop.
Current state: reviewed C1 handoff released; C2/C3 implementation and C4 source integration in progress in the isolated clone. Draft PR #9 targets develop.
Rust implementation: C1 independently reviewed at e7c1c18. The next scoped checkpoint implements pure terminal/replay/input, the HTTP/auth/settings kernel, and provider update planning/execution, with 149 passing tests. Real session/transport integration, remaining routes, launcher and functional binaries remain pending.

| Stage | Owner | State | Evidence / next action |
|---|---|---|---|
| C1 foundation | dots integration owner | reviewed | Independent Codex PASS at e7c1c18; 103 tests and shared handoff reviewed. Additive HTTP/time APIs below require their own caller review. |
| C2 core | dots core + disjoint storage lanes | in_progress | Terminal/replay/input checkpoint: 15 tests / 200 Go snapshots. Storage/approval implementation in progress; native/caller integration pending. |
| C3 services | dots services + disjoint files lanes | in_progress | Hub kernel 19 tests, updater 8 tests; 27 routes partial / 128 unresolved / 0 accepted. Files/profiles and remaining services in progress. |
| C4 launcher/delivery | dots integration owner | in_progress | Source/caller mapping and shared timestamp/profile/process needs; functional launcher and packaging still pending. |
| C5 integration/PR | dots integration owner; Codex independent review | in_progress | [Develop draft PR #9](https://github.com/ishizakahiroshi/many-ai-cli/pull/9) open; final integration/review/CI/artifacts pending. |
| C6 manual/data/cutover | operator; Codex evidence review | pending | Candidate accepted, rollback rehearsal, real devices, stability |

## Delivery, acknowledgment and capabilities

The following checkpoint is based on the published progress commit `f48c6086a2be662233f09d051373a716ca44ce6a`; it preserves the bot's report without treating it as independently observed Slack or runtime evidence.

| Receipt | Current knowledge | Evidence level / owner |
|---|---|---|
| Start request sent and full body read back | Not verified by local Codex | Operator checks actual #3 conversation; a different task's send is not evidence |
| Recognized task and number collision | Board identifies #3; collision acknowledgment not independently verified | Operator records dots reply |
| Instruction read | dots reports reading initial commit 6b0fb8e and 00–06 briefs | Bot self-report in C1 checkpoint |
| Start conversation | Private locator maintained by operator; not supplied to this checkout | Do not expose private Slack/thread identifiers publicly |
| Execution capabilities | dots initially reported missing Rust/Bun/Go, then reported isolated installation authorization and downloads pending | Preserve checkpoint times; tools usable, fetch complete and tests pass need distinct receipts |
| Instruction supplement acknowledgment | dots read revision 30c37b78fca608718fe3bc6597bada162ffb4547 before this code checkpoint | Bot self-report; unchanged task #3 scope |

## Revision identities

| Identity | Value / rule |
|---|---|
| Go behavioral oracle | 21d0bc7935a2c4696fb89ccff2e324157a528c2d |
| Initial instruction commit | 6b0fb8e198750245ef8b4b475fbac9070db0ac3e |
| Progress checkpoint read before this supplement | f48c6086a2be662233f09d051373a716ca44ce6a; progress-only commit |
| Implementation code SHA | Latest independently reviewed C1 code e7c1c18ffed4bcd96fdf09fbdb103edfb6595f4a; next C2/C3 checkpoint SHA recorded in subsequent receipt |
| Reviewed code SHA / reviewer | C1 e7c1c18 PASS. C2/C3 a5f75c9 reviewed: 149 tests and corpora reproduced; bound-port P2 required repair, explanatory P3 corrected in b674b8c. Newer working source is not covered. |
| Board update commit | Read actual commit from GitHub history, or record it in a later receipt; no self-reference |
| Supplemental instruction revision | dots records the published revision it actually reads; supplement is not a new task |

Use immutable commit URLs for requested instructions and the branch URL for current progress. A board-only commit does not change the tested/reviewed code SHA. Record capability and receipt claims with the actor, time and verification level; do not change a pending acknowledgment to verified just because the board says work began.

## Update format

Append time, stage, actual owner, state, changed paths, exact commands/exit codes, candidate commit/PR/CI/artifact links, unperformed cases and next action. Update the table in the same commit. Evidence must not include secrets, private records, real home paths or copied data contents.

Allowed states: pending, in_progress, implementation_complete, reviewed, accepted, blocked. A blocker includes cause and the input/environment required to resolve it. Preserve earlier receipts; do not replace a failure with a success claim lacking rerun evidence.

## Acceptance ledger

All contract groups below remain pending until meaningful caller fixtures and acceptance receipts exist. Inventory and scaffolding are not functional compatibility evidence.

| Contract | Acceptance | Current evidence / remaining |
|---|---|---|
| K01 both CLI binaries/aliases/env | A01 | Baseline entry dispatch identified; parser, process-observed environment and caller tests pending |
| K02 HTTP/WS/auth | A02 | Exhaustive route inventory in progress; Rust handlers pending |
| K03 terminal/replay/input | A03, A04 | Core source/test inventory in progress; parser/caller/native evidence pending |
| K04 storage/queue/reset | A05 | Schema and API inventory in progress; synthetic cross-read/rollback pending |
| K05 approval/transcript identity | A06 | Source/caller inventory in progress; integrated ledger/poll/UI evidence pending |
| K06 files/Git/attachments/mentions | A07 | Source/caller inventory in progress; scoped filesystem fixtures pending |
| K07 providers/profiles/models/usage | A08 | Source/caller inventory in progress; synthetic profile and update cases pending |
| K08 CLI updater | A09 | Source/caller inventory in progress; independent updater executable fixture pending |
| K09 existing Web assets/UI | A10 | Existing TypeScript unchanged; locked Web build and served UI pending |
| K10 orchestration/headless/handoff | A11 | Core source/test inventory in progress; bounded synthetic lifecycle and OS evidence pending |
| K11 routines/memo/preferences | A05, A11 | Persistence/caller inventory in progress; reload and failure fixtures pending |
| K12 notify/push/tray | A12 | Opt-in fake transport and native device acceptance pending |
| K13 voice/runtime | A12 | Fake audio/transport/process fixtures and native acceptance pending |
| K14 launcher | A09, A12 | Shared root/process design pending; SSH parsing/lifecycle/UI fixtures pending |
| K15 distribution | A12 | Four existing targets preserved; reproducible candidate artifact receipts pending |

## Preparation record

Codex prepared these instructions after source and existing-fixture review with parallel subagents. No Go/Rust source changes, builds, Hub start/stop, real provider probes, data migration or deployment were performed in instruction preparation. The operator's initiation and dots execution receipts will be added here when they happen.

## 2026-10-03(土) 12:22:21 JST — C1 start checkpoint

Owner: dots integration owner. State: in_progress; compilation validation blocked.
Changed paths in this checkpoint: docs/bot/rust-migration/PROGRESS.md only.

- Read README and 00–06 instruction files, PROGRESS, AGENTS.md, CLAUDE.md and the relevant development/operations/deployment/coding guides. The migration README explicitly authorizes isolated builds/tests and PR work; this task-specific authorization governs the older generic user-owned-build rule. It does not authorize software installation, a daily Hub restart, credentials, live provider probes or cutover.
- `git clone --branch feat/rust-migration --single-branch https://github.com/ishizakahiroshi/many-ai-cli.git <isolated-checkout>`: exit 0. Initial instruction head: 6b0fb8e198750245ef8b4b475fbac9070db0ac3e. No Rust implementation was present.
- `git status --short`: exit 0, empty before edits. `git diff --stat 21d0bc7935a2c4696fb89ccff2e324157a528c2d..HEAD`: exit 0; owner changes were instruction/index files only, preserved. Go/Web source is unchanged from the fixed baseline.
- `command -v cargo rustc rustup bun`: exit 1, no supported Rust or Bun toolchain found. Checked standard installed-tool locations without reading credentials. `go version`: exit 1; the program named go is not the Go compiler. No compiler, package manager or dependencies were installed.
- Source-size census (Python standard library): exit 0; 640 internal Go files and 166636 Go lines across internal/ and cmd/, including tests. This is a full application migration; a few implemented routes cannot establish completion.
- `.omitnix/index.json` was read first; generated.commit equals the fixed baseline, with its recorded dirty=true. Source/tests, not index summaries, remain the oracle.
- Ownership: integration owner exclusively controls Cargo/lock/build/lib/bin/proto/config/process/Git and progress. Two read-only preparation lanes own their separate new core/service inventories. C2/C3/C4 implementation is not released until shared interfaces have usable tests.

Blocker: the required Rust toolchain, Bun 1.3.14 and Go 1.26.8 are absent. Separate operator permission for isolated official-source tool installation or a prepared execution environment is required before the prescribed compilation/build comparison receipts can be produced. Source inventory and safe code preparation continue meanwhile.

Next: exhaustive CLI/protocol/resource inventory, pinned dependency evidence, root/private-IO/process contracts, then focused C1 tests once tools are authorized. All manual, cross-OS/native, provider, copied-data/rollback, cutover, release and stability cases remain unperformed. No Hub was started/stopped, no real user data was read, and no merge/tag/release was performed.

## 2026-10-03(土) 12:24:17 JST — C1 toolchain authorization and source discrepancies

The operator explicitly approved installing required pinned official toolchains into this task's isolated cloud directory. No workstation settings will change. Download/hash verification remains pending; installation has not yet succeeded. Source work continues independently.

- `node scripts/check-text-hygiene.mjs`: exit 0.
- `node scripts/check-instrumentation.mjs`: exit 0 (one existing opt-in instrumentation entry remains; no release-clean claim).
- `node scripts/secrets-scan.mjs --staged --block`: exit 0, 1 staged file scanned with 4 structural patterns. Private KB/family/filename watchlists were unavailable and skipped; this is not a full history secret audit.
- `node scripts/check-approval-rules-residue.mjs`: exit 0, 0 applicable staged files.
- `git diff --cached --check`: exit 0.
- Core review found an instruction/oracle difference: the baseline SQLite async close may leave queued events after its bounded wait; the requested complete drain will be recorded as an intentional strengthened shutdown contract and tested, not described as baseline equality.
- Core review confirmed the legacy database basename is `any-ai-cli.db`; retain it. Windows failed Job attachment plus descendant-held output pipes requires new bounded teardown fixtures and remains unproved.
- Services review counted 155 server registrations; dynamic suffix handling and caller/auth/shape inventories are still being expanded.

Next: obtain and hash-check official Rust/Bun/Go inputs, create testable C1 interfaces, and preserve all unimplemented routes as pending. No C2/C3/C4 implementation release yet.

## 2026-10-03(土) 12:54:40 JST — C1 implementation checkpoint preparation

Owner: dots integration owner, with exclusive C1 config-model and shared-core subparts. State: in_progress, not API-frozen or accepted yet.
Changed paths: new rust/Cargo.toml, Cargo.lock, rust-toolchain.toml, build.rs, src/{bin,proto,config,process}, cli.rs/assets.rs, tests/fixtures/oracle, scripts and inventory; this progress file. Existing Go/Web sources remain unchanged.

Toolchain blocker resolved: operator-authorized official-source archives downloaded and SHA-256 verified before isolated installation. Rust/Cargo 1.90.0, Go 1.26.8 and Bun 1.3.14 version commands exited 0. Exact official archive identities and direct dependency features/provenance are recorded in rust/inventory/dependencies.json. All four supported Rust standard-library targets were hash-verified; native linkers/devices are separate pending gates.

Implementation scope at this checkpoint:
- All 22 baseline protocol structs/302 tagged fields; 44 synthetic Go-generated zero/filled golden cases; explicit base64 bytes, omitted zero fields, false activity, pointer false and null/empty required arrays.
- Explicit disjoint trial root/resource paths, legacy database basename, private atomic replacement and source-level Windows ACL/replacement adapter.
- Cancellable bounded subprocess capture with actual-child environment tests, output caps, readers before prompt write, timeout and descendant-held pipe bounds. Windows Job attachment/real OS behavior is not accepted from source checks.
- Shared typed session/storage/approval/input/spawn/update interfaces; all 34 Store method names and typed caller records, distinct identities/epochs, verified-origin grants and atomic admission/registration-gap exclusion helpers.
- Typed configuration model and Go-generated synthetic fixtures now integrating. Launcher and main runtime commands remain explicitly unintegrated; the entries fail clearly rather than claiming success or delegating to Go.
- Exhaustive source inventories: 31 main CLI dispatch forms, 155 route registrations, 49 dynamic operations and downstream source/test references. The inventory lists outstanding work; source test discovery does not mean those tests ran in Rust.

Executed command receipts (task-only tool/cache paths supplied through process-local environment):
- `cd web && bun install --frozen-lockfile && bun run check && bun run build`: exit 0; existing generated Web source hash 019c3e80ec0f, instrumentation off. No Web source edited.
- `python3 rust/scripts/generate-proto.py`: exit 0. `go run ./rust/tests/oracle > rust/tests/fixtures/foundation/proto-golden.json`: exit 0.
- First `cargo test --manifest-path rust/Cargo.toml`: exit 101, 14 passed/1 failed; the descendant-pipe fixture exposed polling a completed JoinHandle again after timeout. Fixed by keeping a single owned join coordinator and abort handles; regression retained.
- Repeated `cargo test --manifest-path rust/Cargo.toml`: exit 0, 15 unit tests plus 1 golden test (44 cases). Binary test counts were zero and are not CLI acceptance.
- `cargo test --locked --manifest-path rust/Cargo.toml --test shared_contracts`: exit 0, 16 tests. `cargo test --locked --manifest-path rust/Cargo.toml --lib`: exit 0, 28 tests at that point.
- Initial `cargo check --locked --manifest-path rust/Cargo.toml --all-targets --target x86_64-pc-windows-msvc`: exit 101, invalid std::size_of import. Corrected to std::mem::size_of; repeated Windows target check exited 0.
- `cargo check --locked --manifest-path rust/Cargo.toml --all-targets --target x86_64-apple-darwin` and same for `aarch64-apple-darwin`: exit 0 each for the initial foundation. Recheck final shared/config code before freeze. These are source compilation, not produced native binaries.
- Initial foundation `cargo clippy --locked --manifest-path rust/Cargo.toml --all-targets -- -D warnings`: exit 0. After shared/config integration, exit 101 on seven style/large-enum findings; fixes and final rerun pending in this checkpoint preparation.
- `go test ./internal/proto ./internal/config ./internal/securefile ./internal/launcher`: exit 0 across all four unchanged baseline packages.
- `go run honnef.co/go/tools/cmd/staticcheck@v0.7.0 ./...`: first two attempts failed on non-writable default sumdb/cache locations; rerun with task-local GOPATH/XDG_CACHE_HOME exited 0. No shared/home settings changed.
- Official RustSec database comparison: 77 resolved registry packages, 19 matching advisory/version rows, all selected versions patched/unaffected at the recorded snapshot. See rust/inventory/advisory-receipt.json. This is a source/range comparison, not cargo-audit or proof against unknown vulnerabilities.

Intentional strengthening under review: a held provider-spawn lease closes the baseline launch-before-registration update gap. Full async storage shutdown drain remains a requested strengthening for C2, not already implemented. Configuration recovery behavior follows the Go backup/regeneration path for actual parse failure, while malformed optional sections retain their tolerant decoders.

Next: final configuration/golden/interface tests, fmt/clippy/all-target source checks and staged secret/residue checks; publish a durable code checkpoint before C2/C3/C4 start. C1 is not declared finished yet. HTTP/WS callers, PTY, SQLite implementation, approval/transcript integration, services, launcher, distribution, complete CLI behavior, clean CI/native artifacts, browser/OS/provider acceptance, copied-data rollback and cutover all remain pending.

## 2026-10-03(土) 12:57:53 JST — C1 validated code checkpoint

State: C1 in_progress. The shared foundation slice is validated locally; downstream application compatibility is not claimed.

- `cargo fmt --manifest-path rust/Cargo.toml -- --check`: final formatting check follows the formatter; no manual formatting bypass.
- `cargo clippy --locked --manifest-path rust/Cargo.toml --all-targets -- -D warnings`: exit 0 after fixing the seven code findings plus a nine-argument object-safety test helper. No lint suppression added.
- `cargo test --locked --manifest-path rust/Cargo.toml`: exit 0, 29 library unit tests + 18 configuration contracts + 1 protocol golden test (44 Go-generated cases) + 16 shared contracts + 3 compile-fail doctests = 67 tests. No ignored tests. The two binary unit targets have zero tests and are not counted as functional binary acceptance.
- Final `cargo check --locked --manifest-path rust/Cargo.toml --all-targets --target x86_64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`: each exit 0 after configuration/shared interface integration.
- `cargo build --locked --manifest-path rust/Cargo.toml --bins`: exit 0, Linux diagnostic candidate entry binaries only. Runtime main/launcher functionality is still pending.
- All 41 typed configuration structs/233 source fields have synthetic Go projection fixtures. Tolerant optional input, secret-free public projection, preserved corrupt-config backup/regeneration, private save/failed replacement/stale revision and trial path validation passed.

Outstanding C1/integration items before unqualified API freeze: inspect shared interface usability with implementation owners; provider legacy definition conversion, WSL-launcher log-root selection, exhaustive runtime resource propagation and native version/resource packaging are still caller/build integrations. No downstream module may report application success from these interfaces alone. Trial resource path checks do not establish adversarial filesystem rename/symlink-race acceptance; OS-root handle based mutation remains part of files/launcher integration.

Next durable commit includes this ledger and the complete tested foundation slice. Continue remaining C1 integration contracts, then C2/C3/C4 with exclusive ownership. Full K/A acceptance and C6 remain pending.

### Supplemental instructions acknowledged before publication

Read owner supplement 30c37b78fca608718fe3bc6597bada162ffb4547 (README, 00-contracts, 05-integration-review and board additions) after the pre-publication branch check. Preserved the owner's concurrent additions; this remains coordination task #3 many-ai-cli rust, distinct from task #1 benchmark, #2 MANYHub and #4 school, not GitHub issue/PR 3. Initial instruction SHA remains 6b0fb8e; Go oracle remains 21d0bc7. Implementation review will diff from the initial instruction commit and identify instruction supplements separately. No independent review SHA exists yet.

Added the synthetic `historical_reads_and_new_write_validation_are_separate` fixture: legacy numeric-string session IDs and an unknown optional section load without changing file/token; a newly invalid allowed host is rejected on write; supported data survives an explicit valid write/reload. This does not accept older SQLite schemas or launcher versions, which remain C2/C4 fixtures. Final count after this added case will be recorded from its actual run.

Public progress reports are self-reported execution receipts until independently reviewed. Private conversation locators are intentionally absent.

Final supplement fixture rerun: `cargo test --locked --manifest-path rust/Cargo.toml --test config_contracts` exit 0, 19 tests. `cargo clippy --locked --manifest-path rust/Cargo.toml --all-targets -- -D warnings` exit 0. The final suite now comprises 68 tests (29 units, 19 config, 1 protocol golden/44 cases, 16 shared, 3 compile-fail docs). The last full-suite receipt was 67 tests before the isolated added fixture; its targeted rerun is the new evidence.

## 2026-10-03(土) 13:41:35 JST — C1 independent review and repair in progress

Published implementation code SHA: [756b28154a77dd10a258f01cd2fc69c076d5bc87](https://github.com/ishizakahiroshi/many-ai-cli/commit/756b28154a77dd10a258f01cd2fc69c076d5bc87). This code checkpoint was based on owner supplement 30c37b7. Current working changes below are not covered by that reviewed SHA.

Independent Codex foundation reviewer reports an isolated locked/offline full rerun at 756b281: 68 tests passed; fmt/clippy passed; Go/cmd/Web source unchanged from the behavioral oracle. Its review is limited to this foundation checkpoint, not C5 or complete compatibility. It found important defects requiring repair before API freeze:

- Historical YAML scalar lexical spelling and ignored extension values were not preserved before JSON coercion; this can alter string settings or trigger inappropriate config recovery. A schema-aware raw-node bridge and Go-generated historical fixtures are being added. This gate remains open.
- Existing-config loading did not always harden the config directory if no write occurred. Unconditional private directory setup now has a synthetic 0755/0644 startup regression; targeted test exited 0 without rewriting the file.
- Subprocess cleanup did not own descendants/IO tasks if its async future was dropped or the direct process exited while descendants redirected their stdio. Added group/task ownership guard; two Unix regression tests exited 0. Windows now creates suspended, attaches to Job before child code, then resumes its owned primary thread. Windows native acceptance remains pending.
- Go JSON decoder case-folding/duplicate-field compatibility was not covered by canonical DTO goldens; additional decode fixtures and the chosen compatibility boundary remain pending.

Further C1 work: provider DTOs/legacy config conversion and exact Go JSON byte fixtures; exhaustive CLI/delegated flag/env fixtures; full generated Web asset validation; exact production/trial resource names and WSL-launcher log/database-root selection. New runtime resource helpers are source-grounded; all downstream callers still need to use them.

Receipts so far: Web asset contract tests 3/3 passed; initial provider contract run 7/8 passed with a test that bypassed the actual tolerant Config::from_yaml entry (being corrected); process ownership tests 2/2 passed; root-layout tests 4/4 passed; existing-config privacy regression 1/1 passed. A Cargo filter still compiles every integration-test target: an initial filter run failed because new provider module tests landed before module registration, then was rerun after correct wiring. Do not count those compile failures as behavioral passes.

No C1 freeze, C2/C3/C4 release, whole-migration acceptance, native acceptance, merge or cutover has been declared. Next: integrate schema-aware YAML and CLI decoder corrections, full serial validation, new code checkpoint, and independent re-review of repaired paths.

## 2026-10-03(土) 14:09:12 JST — C1 repair validation checkpoint

State: in_progress, awaiting independent re-review of a new code SHA. Source changes are confined to rust/ and this ledger. The fixed Go/cmd/Web source diff remains empty.

Repairs and expanded handoff surface:
- Replaced whole-YAML-to-JSON coercion with schema-directed raw nodes. Historical auth/string lexemes, Go numeric forms, ignored YAML-only subtrees, aliases/merge precedence, null-list behavior and tolerant pointer-bool partial decoding are compared against 42 actual Go cases. Unknown `.nan`/complex-key extensions no longer trigger config backup/token rotation.
- Restored unconditional private config-directory hardening before an existing-file read. Existing bytes/token remain unchanged on read-only startup.
- Added owned process-group/IO task drop guards, cleanup of detached-stdio descendants, and a streaming ManagedProcess handle with idempotent close/wait and explicit output lag. Windows uses suspended create → Job attachment → primary-thread resume; attach failure cannot run the child. Source checks pass; no Windows native acceptance is claimed.
- Added 19 actual Go JSON decoder cases for case-folded/duplicate fields, pointer-object merge, null values, nested nil maps and byte/base64 behavior. Wire callers must use decode_wire, not canonical-only derive decoding.
- Added 25 shared provider DTOs and pure legacy conversion, with 73 exact Go byte/value wire cases and 9 legacy cases. Hash/signature callers use the Go-compatible encoder.
- Completed CLI discovery: both binaries, 32 command forms, 17 delegated forms, 16 FlagSets/59 declared flags, 4 manually parsed options, 60 environment-read sites; 907 Go flag goldens, 65 dispatch projections and 97 extracted source/boundary cases. Runtime implementations remain pending.
- Generated Web validation now requires the complete frozen build-output asset set, nonempty boot/runtime assets and source identity. Added resource-name/layout, custom-log database placement, WSL-launcher log-root, hook-temp, dangling-symlink and unresolved-installed-root-alias checks.

Final executed receipts before publication:
- `cargo test --locked --manifest-path rust/Cargo.toml`: exit 0, 103 tests: 38 library, 3 asset, 9 CLI, 20 config, 1 protocol decoder, 1 protocol golden, 8 provider, 16 shared, 4 YAML, 3 compile-fail docs. Golden case counts are separate from test counts. No ignored tests.
- `cargo clippy --locked --manifest-path rust/Cargo.toml --all-targets -- -D warnings`: exit 0. `cargo fmt --manifest-path rust/Cargo.toml -- --check` is required again on the staged snapshot.
- All-target `cargo check --locked` for Windows x64 and both macOS targets: exit 0; Linux diagnostic `cargo build --locked --bins`: exit 0. These do not establish native execution/artifact acceptance.
- `go run honnef.co/go/tools/cmd/staticcheck@v0.7.0 ./...`: exit 0 with isolated caches. `python3 rust/scripts/inventory-cli.py --check`: exit 0.
- The streaming order fixture now uses an explicit file handshake, not a scheduler-sensitive sleep. Targeted streaming tests and clippy reran successfully after that test-only adjustment.

Intermediate failures retained: provider test initially bypassed the actual tolerant YAML entry (corrected); one YAML fixture run raced a 38→42 corpus update (rerun after the lane stopped edits); a too-strong roundtrip assertion conflicted with baseline empty hallucination-list omission/default restoration (corrected using actual Go marshaled omission evidence); the documented tolerant invalid subscription *bool allocation was ported. No application behavior was weakened merely to obtain a green test.

Explicit decoding boundaries: YAML above 8 MiB/200000 nodes/depth128/1000000 expansion steps and non-UTF8 binary scalars are rejected without backup/regeneration; the original bytes remain for operator recovery. They are not silently reinterpreted as corrupt config. Provider/native/runtime and copied-data cases remain pending.

Next: publish a source-changing repair checkpoint, identify its immutable code SHA separately from board updates, obtain independent re-review, then freeze the shared API handoff in rust/inventory/SHARED-API.md before C2/C3/C4 implementation. No all-K acceptance, complete functional binaries, release, merge, real-data migration or cutover has occurred.

## 2026-10-03(土) 14:30:21 JST — C1 Go JSON decoder repair checkpoint

Published/reviewed code SHA: [0bad88656776904a320e8b9fc22442185b86c137](https://github.com/ishizakahiroshi/many-ai-cli/commit/0bad88656776904a320e8b9fc22442185b86c137). Independent Codex re-review used an immutable blob-verified source snapshot, reran all 103 tests/fmt/clippy, and reproduced the fixes for the three original P1 findings. It found a remaining P2: an invalid earlier repeated JSON field could be overwritten without retaining the Go type error; ignored overflowing numbers and lone escaped surrogates also differed. This receipt does not treat the following new code as already reviewed.

Changed paths: rust/src/proto/wire.rs, decoder oracle/fixtures/test, Cargo.toml and dependency feature receipt, SHARED-API.md, this ledger. Cargo.lock package versions/checksums are unchanged.

- Replaced eager value conversion with syntax-validated RawValue fields, then destination-typed validation of each known occurrence. Invalid earlier scalar/object/map/array/base64 occurrences cannot be hidden by a valid later duplicate. Unknown fields are skipped without floating-point conversion.
- Matched Go's replacement of lone UTF-16 surrogates and each invalid UTF-8 byte inside JSON strings, preserved valid pairs/escaped literals, and retained the scanner's 10000-depth limit, including ignored values. Invalid JSON syntax remains an error.
- Additional actual-Go probes exposed repeated-slice backing storage reuse. The decoder now retains earlier struct/byte elements through a shorter repeated slice and resets them for null/empty slices, matching the oracle.
- Expanded Go-generated decoder corpus from 19 to 63 cases, including raw input bytes encoded separately from display text. Corrected shared documentation to the exported AdmissionState name.

Executed receipts for this repair:
- `go run rust/tests/fixtures/foundation/proto-decode-oracle.go`: exit 0; 63 synthetic cases regenerated from the frozen Go source.
- Targeted decoder run initially failed for negative integer zero; using destination integer parsing fixed it. Expanded duplicate-slice probes initially failed for Go backing-array reuse; the state-preserving decoder fixed them. Both regressions remain in the corpus.
- `cargo test --offline --locked --manifest-path rust/Cargo.toml`: exit 0, all 103 tests, no ignored tests. Test count is unchanged; the decoder test now executes 63 cases.
- `cargo clippy --offline --locked --manifest-path rust/Cargo.toml --all-targets -- -D warnings` and `cargo fmt --manifest-path rust/Cargo.toml -- --check`: exit 0.
- `cargo check --offline --locked --manifest-path rust/Cargo.toml --all-targets --target <target>`: exit 0 for Windows x64 MSVC, macOS Intel and macOS Apple Silicon. Source checks remain distinct from native execution.
- Go/cmd/Web source diff versus 21d0bc7 remains empty. Repository staged guards and staticcheck are rerun before publication.

An isolated dependency feasibility probe outside the repository also compiled pinned rusqlite 0.40.2, portable-pty 0.9.0 and Axum 0.8.9 on Linux. Bundled SQLite 3.53.2 passed a synthetic in-memory external-content FTS5 query. This is not yet a dependency addition, full storage implementation, native PTY acceptance or network service test.

Next: publish the source-changing decoder repair, obtain narrow independent re-review, then release tested shared APIs to C2/C3/C4 with exclusive ownership. Native/resources/caller integration and all application acceptance gates remain pending. No Hub, paid provider, real SSH host, daily data, merge, release or cutover was used.

Pre-publication checks: `go run honnef.co/go/tools/cmd/staticcheck@v0.7.0 ./...` exited 0. Staged secrets scan exited 0 (8 files, four structural patterns; private watchlists unavailable), approval-residue/text-hygiene/instrumentation checks and `git diff --cached --check` exited 0. This is not a full history secret scan.

## 2026-10-03(土) 14:57:25 JST — reviewed C1 handoff and first C2/C3 checkpoint

Independent review PASS: [e7c1c18ffed4bcd96fdf09fbdb103edfb6595f4a](https://github.com/ishizakahiroshi/many-ai-cli/commit/e7c1c18ffed4bcd96fdf09fbdb103edfb6595f4a). The reviewer regenerated the 63-case Go decoder oracle, reran all 103 tests/fmt/clippy, confirmed the prior YAML/privacy/process fixes were unchanged, and found the shared handoff viable within documented boundaries. This does not review the additional implementation below.

[Draft PR #9](https://github.com/ishizakahiroshi/many-ai-cli/pull/9) was created against develop at that reviewed SHA. It explicitly remains WIP. C2 terminal and C3 services were released in parallel; separate storage and files/Git/attachment lanes have exclusive directories. Only the integration owner changes shared interfaces, dependencies, binary wiring and this ledger.

Scoped implementation being checkpointed:
- C2 terminal: exact source width ranges, streaming VT bytes/UTF8/CSI/string sequences, cursor/wide-cell erasure, reset/resize/scrollback, replay retention/alternate-screen prefix, input ACK/reconnect/resend/caps. Fifteen tests compare 200 actual Go snapshots over every two-way byte split and chunk sizes 1–17. Actual SessionEngine/transport/epoch/native PTY integration is pending.
- Deliberate source discrepancy: the Go parser loses split ESC-backslash string terminators. The requested chunk-equivalence contract is implemented and separately tested as a strengthening, not mislabeled oracle equality. PrimingQueue is currently a generic unbound helper; its capacity is not permission to change the Go UI queue's live disconnect policy.
- C3 HTTP kernel: token/method/Host/Origin order, trusted/local/logical-remote distinctions, PIN cookie/nonce/device/expiry/lockout, reauthentication/static assets, six persisted settings handlers, body decoding and failure behavior. Nineteen focused tests pass. Its complete route coverage ledger has 155 registrations / 49 dynamic operations, 27 partial routes, 128 unresolved and zero accepted. There is no live Axum/WS listener or browser acceptance yet; revocation effects require the actual core driver.
- C3 updater: immutable independently resolved updater argv owns eligibility/preview/log/execution. Eight tests exercise launch=A/update=B, missing B without A fallback, secondary candidates, synthetic B execution, timeout/spawn failure and lease release. HTTP job/caller integration remains pending.
- Shared HTTP decoder accepts explicit service DTO schemas and exactly one JSON value like Go Decoder.Decode, preserving the separate whole-frame WS boundary. Twenty actual-Go cases pass. Settings zero-disable persistence/reload has a new regression; the source Save call does apply defaults, but those defaults do not overwrite idle/reconnect/log-retention zeros, so no unsafe persistence bypass was added.
- Shared timestamp helpers use pinned chrono calendars and runtime-local or explicit offsets, matching 100 Go format and 18 parser cases, including pre-epoch/year-zero, fractional precision, comma fractions and tolerated offset bounds.
- New pinned dependency/feature and public advisory receipts cover SQLite/PTY/HTTP/crypto/TOML/regex/time foundations. All 204 locked packages were compared to the official RustSec snapshot; 59 matching advisory/version rows are patched/unaffected. This is not cargo-audit, a full vendor audit or a license/SBOM acceptance.

Executed receipts:
- Full `cargo test --offline --locked --manifest-path rust/Cargo.toml`: exit 0, 149 tests, none ignored. Binary unit targets still have zero tests and do not establish functional binaries.
- `cargo clippy --offline --locked --manifest-path rust/Cargo.toml --all-targets -- -D warnings`: exit 0. Formatting applied to the frozen registered snapshot; final fmt-check is rerun before staging.
- Bundled SQLite 3.53.2 external-content FTS5 fixture: passed against the actual crate dependency. Linux source compilation passes.
- Expanded `cargo check --locked --manifest-path rust/Cargo.toml --lib --target x86_64-pc-windows-msvc`: exit 101 at libsqlite3-sys C build because this Linux executor has no MSVC lib.exe. The pure-Rust C1 cross-target receipts remain valid only for their earlier SHA; this dependency stage needs clean native Windows/macOS CI toolchains and execution receipts.
- Intermediate failures: a C3 macro parse transient blocked the first C2 test build and was fixed/retested by its owner; an updater test incorrectly expected normalized force rather than the source's raw empty default and was corrected; a timestamp test exposed Go's positive +00:00 after negative sub-minute offset truncation and the formatter was fixed. These are not discarded acceptance failures.

Next: publish this scoped checkpoint and request independent review while continuing storage/approval, real HTTP/WS/session wiring, handle-safe files, profile/settings merge and launcher/library integration. Go/cmd/Web sources remain unchanged. C2/C3 unit/corpus success does not accept whole K groups, native cleanup, real providers/remotes/devices, copied-data rollback or cutover.

Pre-publication: terminal oracle generator and provenance are included; regeneration reproduced the committed 200-case corpus. Go staticcheck exited 0. Staged structural secret scan (private watchlists unavailable), approval-residue, text-hygiene, instrumentation, fmt-check and diff-check exited 0. These do not substitute for independent review or full-history secret scanning.


### First PR CI feedback at e7c1c18

[Validate run](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37100527486) passed the three Go OS tests, staticcheck, govulncheck, Web and other reported guards, but gosec failed on the added config fixture generator's argv-selected file read. The generator now accepts bounded stdin (`--stdin`), avoiding arbitrary path input; canonical and tolerant oracle readback are rechecked.

[PR secret-scan run](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37100527479) reported three generic-api-key matches in the historical 756b281 services source-hash map (lines 1999, 2042, 2049). Each is a SHA-256 digest of public baseline Go source: auth_handlers.go, pin_auth.go and relay_api.go, respectively. All inventory hashes were recomputed against 21d0bc7 and matched. Schema v2 separates file keys and labeled sha256 objects to avoid future ambiguous formatting. `.gitleaksignore` contains only those three exact commit/path/rule/line fingerprints; default rules and all other history remain scanned. This narrow false-positive handling requires independent review. No matching secret bytes are included in this ledger, and no history rewrite or broad path/rule exclusion was used. The next CI result remains pending.

The first live-checkout gosec rerun encountered in-progress, unstaged standalone generators from other lanes (duplicate main declarations and a synthetic rollback-root read). Those are excluded from this immutable checkpoint and were returned to their owners for correction. Validation is repeated from the staged-index-only snapshot rather than reporting a contaminated workspace scan as this checkpoint's result. Canonical and tolerant config oracle outputs after the stdin change match their committed fixtures byte-for-byte.

The staged-index-only snapshot passed both gosec v2.27.1 (high severity/high confidence) and staticcheck v0.7.0, exit 0. Its first scan could not obtain VCS stamping because it is an exported index, so the successful static-analysis rerun set process-local GOFLAGS=-buildvcs=false; this is not a release-build identity receipt. Existing Go/Web source stays frozen, and CI on the next actual commit remains the authoritative remote rerun.


## 2026-10-03(土) 15:22:58 JST — first C2/C3 checkpoint remote CI receipt

Code checkpoint: [a5f75c9ced8bd10f36406c0472c4d3a3cc07e15c](https://github.com/ishizakahiroshi/many-ai-cli/commit/a5f75c9ced8bd10f36406c0472c4d3a3cc07e15c), published and remote-ref verified. This board-only correction does not change its source SHA or extend the earlier C1 review.

- [Validate](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37102478058): all applicable Go Linux/Windows/macOS, Web, staticcheck, gosec, govulncheck, third-party, module and instrumentation checks completed successfully. release-token-scopes intentionally skipped. These existing jobs do not yet build/test Rust on a native matrix.
- [PR secret scan](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37102477977) and [push secret scan](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37102474802): gitleaks success.
- Independent Codex review of immutable a5f75c9 reran 149 tests successfully and independently verified the three exact source-hash ignore fingerprints. Auth/update/terminal and supplemental boundary review is still in progress; no overall PASS yet.
- Receipt correction: historical 756b281 physical lines 1999/2042/2049 name auth_handlers.go/pin_auth.go/relay_api.go. The prior prose misidentified a nearby nvidia_nim source-map entry; corrected above after direct historical-line verification. Hash recomputation and exact ignore scope were correct, and no ignored fingerprint changed.

Continuing work is unstaged/unreviewed separately: SQLite repository and synthetic sequential Go/Rust cross-read, approval/transcript, handle-safe files/Git/attachments, profile/routine and HTTP transport. Full application/launcher/native/Rust CI/artifact/cutover gates remain pending.


## 2026-10-03(土) 16:19:11 JST — persistence/files/approval slice frozen for validation

Owner: dots integration owner with exclusive component/service lanes. Current code parent is a5f75c9; b674b8c is the progress-only correction. The independent reviewer reproduced 149 tests/fmt/clippy and terminal/HTTP/time corpora at a5f75c9, and found the HTTP authority incorrectly used the production root's default port. This P2 is repaired below; its immutable re-review is still required.

Frozen scope for the next code checkpoint:
- SQLite: all 34 Store methods, migrations/WAL/FTS/deadlines, queue/drain/generation reset/prune and private trial paths. Eleven unit and sixteen integration tests include normal sequential Rust→Go→Rust read/write continuation. The Go source is verified against a committed SHA-256 manifest copied to an isolated module, so shallow checkouts/plain archives do not silently substitute mutable Go source. This is synthetic compatibility, not real-data backup/restore acceptance.
- Approval/reader components: fifteen approval tests with actual Go native/risk/summary cases, fifteen bounded transcript/parser tests, and two ordered event-bus tests. Actual single-session state owner, text-question/poll/ledger/action/caller integration remains unfinished; provider parser differential gaps remain explicit.
- Files/Git/attachments: thirty-four focused tests cover scoped reads, descriptor-anchored mutation, synthetic races, metadata/time/ranges, bounded held-file streaming, Git operations and attachment policies. No real repository remote or user files used. Windows ancestor pins deny write/delete sharing; private creation receives a protected ACL at create time. Full Windows metadata source check of this adapter passed separately, but native reparse/ACL behavior remains unaccepted.
- Profiles/provider registry: sixteen tests including first seed, owned-hook removal while preserving user content, existing/default-wins, private atomic writes, malformed preservation, layered registry/hash/diagnostics, and raw JSON unknown/Unicode behavior. Memo/images/schedule: seven tests, including 223 Go modern/future schedule samples. Entire service/router/runtime coverage remains partial.
- HTTP: actual bound-port authority is explicit (configured/fallback port and trial mismatch fixtures), fixing review P2. HTTP1 transport has actual 10-second header-timeout and auth-before-body/first-value/held-file tests. Full request time now controls fractional PIN lockout; cookie expiry intentionally stays seconds. Hub tests report 32 passes. No WS/complete SessionCore claim.
- Shared additions: ordered raw JSON members/arrays and scalar/mtime decoding; injected effect/transport/event/persistence ports and source-grounded lifecycle types; no-deadline timeout=0 with cancellation and overflow rejection before spawning; handle-based/creation-time Windows private security descriptor. These are new APIs requiring caller and independent review.

Deliberate differences and pending source gaps stay explicit: full storage shutdown drain, split string-sequence recovery and unsupported timestamp range protection were already identified. The routine IANA data versions differ (Rust 2025b / pinned Go 2025c); only the sampled modern/future cases are compared. Notify/voice nil-list and publish-before-save-failure semantics are being reconciled in a later slice on the shared store, not a shadow configuration.

Validation in progress:
- The registered working-tree full test run passed 281 tests, zero ignored. The first full clippy run failed on response size/style findings and an intentionally exclusive test lock; owners corrected implementation/style and retained a narrowly reasoned expectation for the barrier test rather than weakening it.
- The index was exported to a separate immutable snapshot; every staged file's Git blob hash was rechecked against the export. In-progress SessionEngine/headless/voice/notify source and tests are excluded from this checkpoint.
- First snapshot link exited 101 with rust-lld signal 7. Filesystem verification found the task's 32GB overlay full, zero available; this was an environmental link failure, not a completed failing test. Only generated failed-snapshot/obsolete package artifacts were cleaned through cargo clean; source, fixtures, failure logs, dependency caches and Git were preserved. Free space recovered to 13GB.
- Exact frozen snapshot rerun uses process-local CARGO_INCREMENTAL=0, CARGO_PROFILE_DEV_DEBUG=0, CARGO_PROFILE_TEST_DEBUG=0 and CARGO_BUILD_JOBS=2. Assertions/tests are not disabled. Final results are recorded below when complete.
- The expanded locked dependency set has 277 packages / 87 matching public RustSec advisory-version rows, all patched/unaffected at the recorded snapshot. This is not a claim of complete vendor/native/license audit.

Next: finish frozen validation, publish this bounded source checkpoint, obtain independent re-review (especially P2, persistence/capability/crypto boundaries), then release waiting integration modules. Launcher and both full CLI runtimes, all remaining routes/WS/session lifecycle, clean native Rust CI/artifacts, copied-data/manual/stability and cutover remain pending. No daily Hub, real provider, real remote, user data, merge or release was used.

### Frozen-snapshot final results (2026-10-03 16:33 JST)

- `cargo fmt --manifest-path rust/Cargo.toml -- --check`: exit 0.
- `cargo test --locked --offline --manifest-path rust/Cargo.toml`: **283 passed, 0 failed, 0 ignored**, including doctests and sequential synthetic Go/Rust SQLite readback.
- `cargo clippy --locked --offline --manifest-path rust/Cargo.toml --all-targets -- -D warnings`: exit 0.
- `go run github.com/securego/gosec/v2/cmd/gosec@v2.27.1 -quiet -severity high -confidence high ./...` and `go run honnef.co/go/tools/cmd/staticcheck@v0.7.0 ./...`: exit 0 from the staged export with process-local `GOFLAGS=-buildvcs=false` (archive has no VCS metadata).
- The successful rerun used the same frozen sources with the compact build settings above. Generated snapshot target was about 936MB and about 12GB remained free afterward. Original disk-full linker logs were retained; no source/fixture/Git cleanup occurred.
- Staged structural secret scan initially identified only synthetic boundary literals (private IPv4 policy inputs, fake URL userinfo/home paths, and GitHub's public SSH service principal). Unnecessary fixture identifiers were changed to reserved example/RFC5737 values; mandatory policy cases/public principal carry exact-line explanatory scanner annotations. No default rule, broad path exclusion, production address policy or assertion was weakened. Rerun: exit 0, 82 scanned text files; private watchlists are unavailable here. This is separate from full-history gitleaks in remote CI.

The registered checkpoint is ready for immutable review; the new review and remote CI are pending. It does not register the still-unfinished session/WS/headless/voice/notification runtime modules. Main application and launcher completion remain open.

## 2026-10-03(土) 17:17 JST — narrow storage repair checkpoint

Reviewed code base: `8355b1060ca8753b8d469ec1244a33c355d307bd`. Independent Codex review reproduced 283 tests/fmt/clippy and closed the actual-bound-port P2. It identified a P1 in reset-marker path reuse: after moving an opened trial directory and replacing its old pathname, a later marker write could go to the replacement directory. Author regression separately reproduced a writer-observer/concurrent-close deadlock against the old mutex ordering (exit 101, bounded child test), then verified the repair.

This bounded slice retains a private directory capability for storage marker replace/read, recovery deletion, initial file creation and permission setup. The writer thread is rejected before acquiring the close/join locks. Required private-directory/append capability helpers and synthetic tests are included; unrelated session/WS/headless/launcher/voice/config integration work is excluded from its frozen export. See the source-grounded [storage checkpoint](../../../rust/tests/fixtures/core/storage/CHECKPOINT.md) and [architecture proposal](../../../rust/tests/fixtures/core/storage/VFS-DESIGN.md).

The native SQLite DB/WAL/SHM namespace is still pathname-based, like the fixed Go baseline. These marker fixes do not establish stronger native confinement under concurrent replacement. No custom VFS, unlimited descriptor retention, fixed root cap or new platform-disable branch was adopted. A proposed per-store Linux fd-path shim was discarded after source inspection found shared SHM pathname lifetime could outlive that connection. Architecture/native review remains a blocker to the stronger trial security acceptance. Ordinary workspace hard-link semantics remain unchanged; directory capabilities are not a general sandbox against a same-user actor controlling filesystem entries.

Validation: isolated frozen source export, pinned Rust/Go and compact build settings. `cargo fmt -- --check`, full `cargo test --offline --locked`, and `cargo clippy --offline --locked --all-targets -- -D warnings` all exit 0. Storage-focused green receipt is 13 unit + 17 integration tests, including sequential Go rollback readback. Exact aggregate count and pre-publication guards are recorded below.

Remote CI at 8355b10: [Validate succeeded](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37106965456); [gitleaks failed](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37106965425) on one synthetic redaction regression. The independently inspected test contains fixed PEM markers surrounding only literal `synthetic-only-not-a-key`, not encoded key bytes. Future markers are built at runtime. `.gitleaksignore` adds only `8355b1060ca8753b8d469ec1244a33c355d307bd:rust/src/storage/text.rs:private-key-block:211`; no broad rule/path allowlist or history rewrite. This exact exception requires readback/remote recheck on the repair SHA.

Status remains WIP. Neither this repair, the earlier 283 tests nor existing Go/Web CI establishes complete Rust runtime/native/launcher compatibility. Next: publish this exact repair, review its immutable diff and architecture options, then continue already isolated functional lanes.

Frozen repair aggregate: **289 passed, 0 failed, 0 ignored** (including doctests). Source bytes outside this slice are not part of that receipt.
Pre-publication staged structural secret scan, approval-residue, text-hygiene, instrumentation and diff checks all exit 0. Private secret watchlists remain unavailable; full-history gitleaks is a distinct remote check. All frozen source blobs were compared to the intended index before publication.

## 2026-10-03(土) 18:23 JST — registered runtime/launcher slice

Current code parent is `19329e0a5e3e3a0b45a360c393e1c33453fb63ab`. Its [Validate](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37109379385) and [gitleaks](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37109379386) completed successfully. The narrow marker/close repair's independent security re-review remains unverified; the earlier independent review does not close its remaining native SQLite namespace boundary. No full security acceptance is asserted.

The next bounded code slice registers actual library callers:
- C2: one SessionEngine owner, ordered input/persistence tickets with cancellation/drop handling, private journals, registration/reattach, old-connection cleanup, ACK/high-watermark handling, accepted UI work draining through revocation, approval component actions, and typed Git-turn delivery through the same priming queue. Warm reconnect now preserves the source's orthogonal activity/clock/metadata instead of reconstructing them from replay length. User-input history follows the actual send/defer decision. The composed lifecycle interface is separate from still-unfinished SpawnConfirmations; no successful confirmation stub is used.
- C3: real HTTP/WS transport + owned socket driver, authentication/first-frame gates, typed bound persistence and real loopback integration with SessionEngine. Tests use the frozen Web's `pty_input.text` field, not the Hub-to-wrapper bytes field. Header timeout, failed-writer behavior, old-socket identity and accepted-input/revoke ordering have synthetic caller evidence. Route inventory remains **155 registrations / 49 dynamic operations, 57 partial routes, 98 unresolved, zero accepted**.
- Source-specific configuration behavior: six settings handlers publish in memory before Save, returning500 on persistence failure while retaining the changed state and unchanged old disk contents, as Go does. The previous transactional fixture assumption was wrong. Default ConfigStore.persist remains transactional; the explicit legacy method and bounded reconfiguration hook are separate. Nullable notify lists and first-value untyped JSON decoding are shared additions; their notification callers are not in this slice. Generic JSON projection has an explicit128-container resource limit, narrower than Go's syntax limit.
- Headless/handoff/subagent: actual owned process execution, output parser/framing, path-only opt-in handoff records, three provider tree readers, bounded/capability-based append/read/prune/render and stale-generation handling. Focused49 tests cover60 Go parser,40 framing,7 handoff and10 tree/signature cases. Runtime provider/observer/handoff integration is still partial.
- Launcher: reusable profiles, command construction, fake SSH serve/tunnel/import, managed connection registry, loopback selection UI, shared main-connect library flow and delivery-manifest/hash validation. Focused24 tests cover129 actual Go observations, dual shell quoting, concurrent registry and ownership cleanup. Windows hostname source now uses Go's physical DNS name API; native execution and the exact launcher write deadline remain pending. Neither binary's complete command dispatch nor real packaging is accepted.
- Integration foundation: held-directory config persistence/private append, reusable schema-directed YAML, Go byte-preserving masking, explicit subprocess window/PID helpers, and an isolated runtime ledger/probe. No real home/config/provider/SSH/WSL/notification/model invocation was used.

Frozen validation:
1. An initial live-checkout full test attempt exited101 only because the next wrapper lane's unregistered integration target was auto-discovered. It is excluded from the index-only source export, not ignored inside the test suite. One fmt check then found module ordering in lib.rs; corrected before validation.
2. `cargo fmt --manifest-path rust/Cargo.toml -- --check`: exit0 on the fixed export.
3. `cargo test --offline --locked --manifest-path rust/Cargo.toml --no-fail-fast`: **430 passed, 0 failed, 0 ignored**, including181 unit tests,21 session,8 journal,24 launcher,49 headless/handoff/subagent,6 runtime-ledger and existing compatibility suites/doctests.
4. `cargo clippy --offline --locked --manifest-path rust/Cargo.toml --all-targets -- -D warnings`: exit0. Earlier cross-lane lint findings were repaired without blanket allowances.
5. `go run github.com/securego/gosec/v2/cmd/gosec@v2.27.1 -quiet -severity high -confidence high ./...`: exit0. Staticcheck v0.7.0 initially could not initialize its default read-only cache; rerun with an explicit isolated XDG_CACHE_HOME exited0. The export uses GOFLAGS=-buildvcs=false, not a release identity claim.
6. Compact Cargo settings remain active; about6GB workspace space remains. Existing Go/cmd/Web source is byte-unchanged against the fixed oracle.

A new uncredentialed native CI workflow is prepared for Linux x64, Windows x64 MSVC, macOS Intel and Apple Silicon, with pinned Rust1.90.0/Go1.26.8/Bun1.3.14. It checks actual runner architecture, locked Web/Rust validation, both binaries and hash/asset/build receipts. Standard runner labels were verified against [GitHub's runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners). It does not request publish credentials, sign/release/install candidates or replace Go. The four native jobs have not run for this source yet; a build-only artifact is explicitly marked unaccepted for cutover.

Excluded next-slice work: actual PTY/wrapper/execpath/native startup ownership, voice/notification crypto/archive and callers, one-tap live selection intent, complete orchestration/confirmation/poller/initial-prompt flows, routine CRUD/runner, remaining service routes and full CLI/packaging. Known small follow-ups include resize-write best-effort source semantics and overflow cleanup of provisional spawn admission. These gaps remain before full caller parity claims; documentation is not a substitute for their implementation. C2/C3/C4/C5 remain in progress, and all native/manual/rollback/cutover gates remain open. Draft PR#9 stays WIP; the progress ledger carries newer receipts than its current body.

Final publication guards (2026-10-03 18:48 JST): the staged secret scan initially rejected four email-shaped synthetic SSH fixture values. Replaced only the synthetic domain with the existing reserved example.invalid convention, regenerated all129 fixed-Go observations, and verified that the resulting corpus differs only by that domain spelling. Included the generator's Go test input explicitly so a clean checkout can reproduce it. No scanner rule or broad allowlist was added. After one formatting correction, the affected `cargo test --offline --locked --manifest-path rust/Cargo.toml --test launcher_contracts -- --test-threads=4` rerun passed24/24 (exit0), and whole-export fmt check passed. The earlier430-test/full-clippy/gosec/staticcheck receipt remains the aggregate source receipt; the final changed fixture has this additional affected-suite receipt.

Staged structural secret scan (100 text files), approval-residue, text-hygiene, instrumentation and diff whitespace checks exit0. Private watchlists remain unavailable. All100 changed Git-index blobs, including the final receipt, are frozen for publication and compared with their Git blob hashes; remote native CI and immutable functional review are the next verification stages. The separate PTY/wrapper and pure journal-helper work stays outside this commit and its receipts.

## 2026-10-03(土) 19:05 JST — first native CI results and bounded repair

Published registered-runtime code: `a02223c622345e44c6da8cbcc4e2e9da1b225ded` (tree `4d4ae62d9590cbfe944290fa06e0cf778c9ebd92`). [Validate passed](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37114398456). The first [native candidate run](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37114398599) passed Linux x64 and retained a build-only artifact containing both candidate binaries and receipts. Windows and both macOS jobs failed; no four-target acceptance is claimed. The artifact is compiled from PR merge SHA `3b06e47db392e361bd2c0232d1b8a3aee29d3530`, with a02223c recorded separately as the review head.

The macOS compiler rejected unpromoted u16 mode_t arguments to variadic openat. The repair uses full-width integer arguments at those two calls without changing flags or permission values. Windows strict clippy identified Unix-only test imports/fixtures and a Windows re-export below a test module; cfg placement and item ordering are corrected without suppressing warnings. The receipt script now labels asset digests as build inputs, not proof of linked runtime retention; the still-incomplete binary entrypoints and actual retained asset behavior remain explicit gates.

[Full-history gitleaks](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37114398665) identified one synthetic Go masking oracle at `rust/tests/fixtures/core/journal/mask_oracle.go:28`. Its body contains only literal abc followed by an invalid FF byte, not encoded key material. Future delimiter construction preserves the exact bytes; running the fixed-Go generator reproduced all16 committed observations byte-for-byte. The only added historical exception is `a02223c622345e44c6da8cbcc4e2e9da1b225ded:rust/tests/fixtures/core/journal/mask_oracle.go:private-key-block:28`; no rule/path allowlist or historical rewrite. Independent classification/readback and remote recheck are distinct from this author receipt.

Bounded repair validation, index-only frozen export: `cargo fmt --manifest-path rust/Cargo.toml -- --check`, `cargo test --offline --locked --manifest-path rust/Cargo.toml --no-fail-fast` (**430 passed, 0 failed, 0 ignored**), and `cargo clippy --offline --locked --manifest-path rust/Cargo.toml --all-targets -- -D warnings` all exit0. Pinned tools/compact flags remain unchanged. Python receipt script AST check and fixed-Go mask generator/corpus comparison exit0. Native CI must rerun on the repair commit; local Linux success cannot close native failures.

Independent ordinary functional review at a02223c reproduced151 scoped tests and found two caller defects: reattach did not invoke pending/inflight input flush after ACK, and a failed separate submit-Enter lost its settle/confirmation marker. Those functional repairs and real-WS/failure-path regressions are in the next, separate work slice, together with PTY/wrapper registration. The marker/native SQLite security review remains unverified and outside this functional review. Full application/launcher wiring, remaining service routes, native/manual/rollback/cutover gates stay open.

## 2026-10-03(土) 19:28 JST — PTY/wrapper caller and input-review repairs

Native-build repair `fc00d54e27067c89f23fb2ec21d89cc787cd4467` is published. Its [Validate](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37115517565) and [full-history gitleaks](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37115517652) passed. In [native run2](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37115517604), Linux again passed; Windows compiled but had35 unit failures, mostly directory-handle sharing errors. Both Macs compiled but had22 unit failures, primarily strict walking of the OS /var-to-/private/var alias. These are real native failures, not ignored tests or successful platform acceptance.

The next frozen slice contains:
- `process/{execpath,pty,pty_unix,pty_windows}` and `wrapper/{launch,entry,runtime,input,transport,hooks,output,shell}`. Actual Linux PTY tests observe received argv/environment/cwd, initial dimensions/resize/input, exit values, idempotent close/wait and bounded owned-group shutdown while an unrelated owned child survives. Actual loopback/scripted wrapper tests cover register-before-exec, lost initial ACK with no provider launch/proof replay, short writes/ACK/deduplication, replay/reattach, backpressure, manual-Hub-restart survival and explicit dismissal. Windows ConPTY creates suspended, uses the shared Job policy before resume, and owns cancellable IO threads; its native behavior is not yet accepted.
- Wrapper entry code composes the existing headless executor rather than duplicating it. Prompt/hook ownership, opt-in raw logging, UTF-8 carry, Windows mojibake repair, shell-init and executable/npm-shim preparation are registered. Full binary entry dispatch, Hub startup/spawn-ownership handoff and provider transcript integration remain pending. Failed PTY input remains unacknowledged and warns/continues, and failed resize is best-effort, matching the fixed Go source rather than the initial uncompiled implementation's session termination.
- The functional review's two input fixes: actual reattach starts an owned flush after ACK while continuing to poll wrapper output/ACK frames; failed separate Enter restores its settle/confirmation marker, including stale/cancelled paths, and warm reconnect preserves it. Regression coverage includes exact original-sequence resend before disconnected pending input and fresh subsequent input, and failed body-then-CR delivery without replaying the paste body. The first real-WS fixture was corrected to advertise ACK capability as Go requires; the source intentionally does not replay legacy non-ACK-capable inflight frames.
- Additional caller corrections: resize effects explicitly tolerate only best-effort wrapper delivery while input stays strict; ID/incarnation exhaustion is checked before consuming registration proof/admission; SessionDetails exposes the exact core output clock for nanosecond-sensitive routine timing. The wrapper's journal path helper reuses the same offset-preserving implementation without adding a second journal owner.
- Proposed native-path repairs: only the three standard macOS root aliases (/var, /tmp, /etc) are expanded after verifying their exact /private target. Arbitrary caller paths are not canonicalized and later symlinks remain rejected. Windows pins use metadata access with read/write sharing and continue withholding delete sharing, instead of denying ordinary access. Protected ACL creation/repair remains. Concurrent Windows reparse mutation and native SQLite DB/WAL/SHM namespace confinement are explicitly unproved; these functional path repairs do not close the stronger security gate. A trial-path fixture now compares to the canonical selected root on both Mac and Windows.
- Cargo adds a direct handshake-only edge to the already locked tokio-tungstenite0.29.0 and Windows Pipes/IO features. All277 resolved package/version/source/checksum rows are unchanged; dependency/features and lock-digest receipts were updated. No new package download occurred.

Frozen Linux validation: `cargo fmt --manifest-path rust/Cargo.toml -- --check`, full `cargo test --offline --locked --manifest-path rust/Cargo.toml --no-fail-fast` (**477 passed, 0 failed, 0 ignored**, including212 unit,24 session,13 wrapper tests), and `cargo clippy --offline --locked --manifest-path rust/Cargo.toml --all-targets -- -D warnings`: exit0. Earlier failures retained separately were sha2 digest formatting, an unconstrained fixture byte vector, equivalent enum-boxing/format lint fixes and the ACK-capability fixture precondition. No broad lint allowance or assertion weakening was used. Existing Go/cmd/Web source stays unchanged. Independent review of the fixed immutable SHA and the new native matrix remain pending.

Routine CRUD/runner/HTTP modules, voice/notification, one-tap live selection intent, full confirmation/orchestration/poller/initial-prompt callers, both complete command dispatchers and delivery/manual/rollback/cutover acceptance remain outside this checkpoint. Routine source work continues separately and is excluded from the frozen test receipt. No running Go replacement, real provider/remote/notification, merge or release occurred.

## 2026-10-03(土) 19:46 JST — native fixture correction and review status

Published wrapper/input repair: `83f7cf9cc39c2d380f341c3ed33db7f3d9bb0bff`. Its full-history [secret scan passed](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37116766338). The latest ordinary independent re-review could not be completed: the two prior input P2 findings and the wrapper slice remain **independently unverified** despite the author's477-test receipt. The earlier review is not treated as approval of these fixes. A fixed-SHA review handoff remains required; the separate native filesystem/security gate is also open.

In [native run3](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37116766313), Linux passed. Windows stopped at strict lint on three Unix-only wrapper-test imports; the ConPTY library compiled, but its tests did not execute in that run. ARM macOS passed the prior22 unit failures after standard-system-alias handling, then found a handoff fixture comparing a physical canonical path to its /var spelling. The small correction scopes only those imports and compares the returned path against the canonical same owned temporary root. It does not broaden path access or skip the containment assertion. Native tests will use --no-fail-fast so later suites also produce evidence when one suite fails; the job still fails on any test failure.

Frozen three-file source/script correction: `cargo fmt --manifest-path rust/Cargo.toml -- --check`, `cargo test --offline --locked --manifest-path rust/Cargo.toml --test handoff_contracts --test wrapper_contracts` (**25 passed, 0 failed, 0 ignored**) and `cargo clippy --offline --locked --manifest-path rust/Cargo.toml --all-targets -- -D warnings` all exit0. The last full aggregate remains477; this receipt is an affected-suite rerun, not a second full suite. New core-action, routine and executable-composition work is excluded. Full native rerun and independent review remain pending.

## 2026-10-03(土) 20:15 JST — native path spelling and precise clock proposal

Published fixture checkpoint: `010d8ad38c443c531900602a49856246376ce5f4`. [Validate](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37117757889) and [full-history secrets](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37117757873) passed. In [native run4](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37117757872), Linux passed. ARM macOS ran all suites and had only three failures comparing physical /private/var paths with their /var spelling; its212 unit tests passed. The journal containment, launcher cwd and actual PTY cwd fixtures now compare the canonical same owned temporary directory, keeping the complete containment/cwd assertions.

Windows proceeded through native tests. Prior sharing errors are replaced by an incorrect-function error while the held-directory walker opens a verbatim drive prefix before the root separator. The proposed correction skips only the Prefix component's premature open, then pins the root and every actual directory with the same metadata/reparse and no-delete-sharing policy. A Windows-only ordinary owned-directory regression covers canonical open, child creation and readback. Linux compilation cannot verify that native behavior, and this correction does not close the separate filesystem security gate.

Windows also exposed real precision loss: SystemTime represents FILETIME at100ns resolution, while Go's logical parsed/constructed time retains nanoseconds. The exact Go corpus and memo assertions remain unchanged. `TIMESTAMP-DESIGN.md` records the proposed normalized seconds/nanoseconds type, explicit OS conversion boundaries and affected-consumer inventory before changing shared APIs. The clock repair is a separate slice; Windows compatibility is still failing here.

Frozen source validation: `cargo fmt --manifest-path rust/Cargo.toml -- --check`; `cargo test --offline --locked --manifest-path rust/Cargo.toml --test journal_contracts --test launcher_contracts --test wrapper_contracts` (**45 passed, 0 failed, 0 ignored**); `cargo clippy --offline --locked --manifest-path rust/Cargo.toml --all-targets -- -D warnings`: all exit0. Last full aggregate remains477. This checkpoint excludes new typed actions, child options, routines, command composition and timestamp implementation. Both latest independent functional re-review and storage-security re-review remain unverified. Full native, service/CLI, manual, rollback and cutover gates remain open.
