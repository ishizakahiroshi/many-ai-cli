# #3 many-ai-cli rust: progress

> 最終更新: 2026-10-03(土) 13:01:07

Task label: #3 many-ai-cli rust. This is a coordination label, not GitHub issue/PR number 3. Continue replies only in the thread where the operator starts this task; do not mix #1/#2 tasks. The operator starts dots from Slack or ChatGPT Web. This repository file and implementation diff are the reviewable progress sources.

Baseline Go SHA: 21d0bc7935a2c4696fb89ccff2e324157a528c2d.
Instruction/implementation branch: feat/rust-migration.
PR base: develop.
Current state: C1 foundation in progress in an isolated clean clone; the operator has started execution.
Rust implementation: initial shared foundation implemented; final C1 configuration and interface tests are being integrated. Isolated toolchains and locked Web build succeeded. Application callers, routes and both functional binaries remain pending.

| Stage | Owner | State | Evidence / next action |
|---|---|---|---|
| C1 foundation | dots integration owner | in_progress | Initial foundation code and source inventories exist; final C1 validation is in progress. All downstream implementation and acceptance remain pending. |
| C2 core | dots core lane | pending | After C1, parallel with C3/C4 |
| C3 services | dots services lane | pending | After C1, parallel with C2/C4 |
| C4 launcher/delivery | dots launcher lane | pending | After C1, parallel with C2/C3 |
| C5 integration/PR | dots integration owner; Codex independent review | pending | Complete traceability; build/test/CI; open develop draft PR |
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
| Implementation code SHA | Not yet verified locally; dots records actual code commit when available |
| Reviewed code SHA / reviewer | Pending; changes after review require the corrected SHA and re-review receipt |
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
