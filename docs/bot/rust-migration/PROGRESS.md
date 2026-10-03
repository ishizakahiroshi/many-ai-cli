# #3 many-ai-cli rust: progress

> 最終更新: 2026-10-03(土) 12:44:28

Task label: #3 many-ai-cli rust. This is a coordination label, not GitHub issue/PR number 3. Continue replies only in the thread where the operator starts this task; do not mix #1/#2 tasks. The operator starts dots from Slack or ChatGPT Web. This repository file and implementation diff are the reviewable progress sources.

Baseline Go SHA: 21d0bc7935a2c4696fb89ccff2e324157a528c2d.
Instruction/implementation branch: feat/rust-migration.
PR base: develop.
Current state: C1 foundation in progress in an isolated clean clone; the operator has started execution.
Rust implementation: source inventory and shared contracts being prepared. Rust/Go/Web builds and manual acceptance have not run. The operator has now approved isolated official-source toolchain installation; verified downloads are being attempted.

| Stage | Owner | State | Evidence / next action |
|---|---|---|---|
| C1 foundation | dots integration owner | in_progress | Instructions and fixed baseline read; inventory/root/interfaces next. Build validation blocked on absent approved toolchains. |
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
| Instruction supplement acknowledgment | Pending | dots records revision read at its next checkpoint |

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
