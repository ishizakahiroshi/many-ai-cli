---
type: reference
status: draft
tags: [rust, recovery, dots]
owner: unknown
review_status: draft
related: [README.md, REVIEW.md]
last_reviewed: 2026-10-05
---

# #3 recovered Rust: continuation progress

> 最終更新: 2026-10-05(月) 03:34:11 UTC

Repository: ishizakahiroshi/many-ai-cli. PR base: develop. Existing draft PR: #9. This continues existing #3; no replacement task number has been assigned.

Delivery: source and instructions published on `dots/rust-recovery-resume-3`; resume message prepared, delivery and acknowledgment pending. Initial publication commit: `7b937e0edc646a5b35d1b3ddd33bf86c3e2f2c5b`. Use the actual immutable commit supplied in the resume message, including subsequent publication fixes. The old PR cannot be used as the new candidate identity.

Local prior-source receipt: 1151 all-target tests / 0 failed / 0 ignored, strict Clippy, 3 doctests and fmt passed; both Windows release binaries built; main isolated CLI/HTTP/WS/settings/shutdown and desktop Chrome settings/version checked; standalone launcher --help checked. Source-entry digest: 307d2b5ecc7253dc73ba8f638a8d99fe53534e29aa86aafd319bc25e59276d78. These local reports require independent reproduction at the actual Git SHA and do not constitute product-wide acceptance.

| Stage | Owner | State | Evidence / next action |
|---|---|---|---|
| Source and instruction publication | Coordinator | published; CI cleanup in progress | Initial publication 7b937e0; actual final revision supplied in the resume message |
| Existing #3 acknowledgment | dots | acknowledged; preserved source continued | Immutable recovery source and R/V supplement read; one integration owner retained |
| Remaining behavior matrix and fixes | dots disjoint lanes | pending | Fixed Go oracle, actual caller and failure-path evidence |
| Clean four-target CI/artifacts | dots integration owner | pending | Exact code SHA, jobs, skips, lock/assets and both binary hashes |
| Independent review and fixes | Separate reviewer | pending | Inherited source plus continuation diff; re-review final fixed SHA |
| Windows native/browser smoke and synthetic copied-data rollback regression | Local coordinator | limited local scope passed | Binary smoke and synthetic data regression have separate receipts; repeat against the returned candidate, with provider/device/native installed gates pending |
| Cutover/release | User/authorized local owner | not authorized | Running Go retained; full product gates required first |

Instruction SHA: supplied externally at the final published revision. Code SHA: pending continuation. Reviewed SHA: pending. PR/CI/artifact links: pending. Do not put this board's own commit into its body as a self-reference; record it from history or a later receipt.

Append dated checkpoints with owners, changed paths, command/exit outcomes, failure classifications, actual code/review SHA and unresolved gates. Public entries must not contain private home paths, data, credentials or conversation locators.

2026-10-05 publication checkpoint: normal pre-commit secret/residue/index checks and pre-push instrumentation/staticcheck passed. The required generated omitnix index was refreshed; its unresolved/unknown/unclaimed entries remain explicit and are not proof of Rust compatibility. Initial push secret-scan run [37248305708](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37248305708) failed on two generated path-to-source-hash fields misclassified as API keys and one real public-IP test example. The preceding oracle branch scan [37129973113](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37129973113) passed. The generator now emits schema 2 records with separate path/sha256 fields; all 427 records were verified and only the test-edited source hash changed. The test now uses an RFC5737 address. Its exact DNS rejection test ran 1 passed / 0 failed / 0 ignored / 874 filtered, exit 0; fmt also passed. Scanners/configuration/allowlists were not changed. Preserve the initial failure; a successful later push scan does not establish PR-range or four-target build/test acceptance. The 1151-test/native receipts above identify the earlier frozen source and are not fresh full-suite results for later revisions.

## 2026-10-05 continuation acknowledgment

- This is the existing #3 continuation. The coordinator checked active workers before admission; no duplicate #3 integration owner was active. The immutable README, REVIEW and PROGRESS at `7c2d7d320e04cd1d550d937d97515ff692500c6a` were read. The regular checkout starts at that exact commit on `dots/rust-recovery-resume-3`.
- Integration owner exclusively owns shared Cargo/lock/config/protocol/process/lib/bin/router, startup, CI, Git and this board. Disjoint lanes cover core/approval/storage prompt callers, launcher/packaging inputs and trial wrapper executable confinement. A separate non-author reviewer covers inherited source and will re-review the exact final continuation diff.
- Executable environment is Linux x86_64 only. Cached Go1.26.8 executed successfully. Rust1.90.0 compiler/Cargo/rustfmt/Clippy component archives were verified against the official pinned manifest; Bun1.3.14's official Linux archive matched the recorded SHA256. `cargo fetch --manifest-path rust/Cargo.toml --locked` exited 0 for the current 316-package lock. Locked Bun install, TypeScript check and frontend build exited 0. All dependency downloads used ordinary permitted official/registry routes; no denied-download fallback was used.
- A separate immutable `7c2d7d3` checkout is running `cargo test --manifest-path rust/Cargo.toml --locked --offline --all-targets -- --test-threads=8`. Compilation is pending; this is not a new test-success receipt. Windows and both macOS native runners remain repository-CI work, and GUI/provider/account/device/installed-data acceptance remains with authorized target owners.
- GitHub readback confirms [push secret scan 37248803112](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37248803112) succeeded at the supplied source; [initial failure 37248305708](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37248305708) is retained. Neither proves a full PR-range scan or a four-target native build. Full PR-range examination must include the original published commits.
- Existing Draft [PR #9](https://github.com/ishizakahiroshi/many-ai-cli/pull/9) remains open at `9992696` on `feat/rust-migration` toward `develop`; it does not cover this recovered branch. No force-update or closure is authorized. The recovered-branch Draft PR will explicitly document supersession pending owner reconciliation.
- Independent inherited-source review has raised startup DB ownership/degradation, advisory shutdown delivery and trial self-bootstrap environment findings. Reproduction, focused fixes and re-review are in progress; these are not accepted fixes yet. Actual locked-crate licensing/SBOM inputs are being collected; the empty Windows whisper runtime payload remains an explicit native packaging gap.

Current code baseline: `7c2d7d320e04cd1d550d937d97515ff692500c6a`. Independent final reviewed SHA, final candidate test exits, CI runs and artifact identities: pending. The earlier 1151-test receipt remains historical; later targeted audit/IP fixes do not inherit that receipt.

## 2026-10-05 first repair checkpoint

[Recovered continuation Draft PR #11](https://github.com/ishizakahiroshi/many-ai-cli/pull/11) targets develop. PR #9 remains unchanged; reconciliation is owner-pending. Initial acknowledgment was published as `3a538ee8fb5424224eefc43b1e5fdafded66fe9d`. HTTPS Git publication had no local credentials, so publication used the authorized repository connector and remote readback.

The independent inherited-source review produced concrete startup/history, advisory shutdown, trial bootstrap, and pre-ACK instruction-order findings. This checkpoint contains lifetime runtime/database leases, optional history and stderr log degradation, shutdown delivery independence, the self-bootstrap home identity repair, and a register-before-ACK/reattach-after-ACK preparation barrier using the existing instruction and usage-hook owners. Focused inherited gate/write panic/board and wrapper-boundary regressions are retained. No same-root/two-live-Hub reproduction is rerun after a reviewer safety block; the existing artifact observation has no assigned final source SHA. The lifetime lock primitive is tested separately, and the integration limitation remains open. Empty/relative production log-directory compatibility remains under review.

Current working-source validation (pre-commit, build identity still based on acknowledgment HEAD; not final clean-CI identity):

| Command / scope | Exit | Observed result |
|---|---:|---|
| `cargo test --manifest-path rust/Cargo.toml --locked --offline --all-targets --no-fail-fast -- --test-threads=8` | 0 | Linux 1253 passed, 0 failed, 0 ignored; 32 summaries, including copied-data rollback after pinned Go cache preparation |
| `cargo clippy --manifest-path rust/Cargo.toml --locked --offline --all-targets -- -D warnings` | 0 | Linux strict check; later edits need their own repeat |
| `cargo fmt --manifest-path rust/Cargo.toml` / standalone checks | 0 | Formatting only |
| pinned Go1.26.8 preference/media generators | 0 | 83 / 170 cases agree; source hashes unchanged. Only two source-created filename placeholders need native separator adaptation; arbitrary user paths are not normalized. |
| `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s rust/packaging -p 'test_*.py'` | 0 | 25 packaging/CI receipt tests |

Immutable baseline `7c2d7d3` separately failed Linux all-target invocation at its library: 921 passed / 2 failed / 0 ignored, exit101; later targets were not run by that baseline command. These failures were Windows-origin expected filename separators. The prior Windows 1151 count remains a different historical receipt.

[Initial four-target run37250575786](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37250575786) at PR head `3a538ee8` is red. Linux and both macOS jobs stopped at strict Clippy casts in whisper/native.rs; subsequent test/build/artifact stages did not run. Windows passed Clippy but library had 841 passed / 34 failed, plus copied-data rollback failed because its deliberately offline Go child had no module cache. Windows source checkout CRLF and selected 8.3-vs-long path identities are being addressed, not waived. This checkpoint preserves committed bytes in CI, prepares selected Go caches, limits test concurrency to eight, runs all-targets and doctests explicitly, checks out the actual PR head, and retains command/exit/failure logs even when validation fails. Two release binaries and actual observed-build licensing/SBOM collection are still pending.

[PR-range secret scan37250575706](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37250575706) examined 24 commits and found the three original `7b937e0` findings already described above; it is NOT accepted. Any exact fingerprint exception requires coordinated approval outside the original path scope; no scanner/allowlist change is included here. [Validate37250575753](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37250575753) also exposed two standalone oracle-driver Go files entering `go build ./...`; explicit ignore build tags and the generator's named-input behavior were repaired under rust/tests/fixtures without changing the Go behavior oracle.

Remaining behavior audit has found five missing relay suffix handlers despite the bound session prefix, and trial webhook/ntfy transport requires a guard independent of web-push. Separate focused work continues before final acceptance. The K/A matrix will state concrete caller coverage; R01–R03/V01–V08 definitions have not been found in the checked-in instruction set and are awaiting an authoritative source. Final code SHA/review SHA/four-target artifacts and all native/provider/device/cutover gates remain pending.


## 2026-10-05 second continuation status (publication and validation separated)

### Published identity and roles

- The authorized one-time retry succeeded. All39 changed-file blobs and the tree were created, then the dedicated branch was fast-forwarded to `15db189db643143191e8b8063165fc6f2f5d2b14`; fresh branch and PR#11 reads agreed. The published tree `3a2dcc1fda976140f5b48dd699fd10413e44acbe` is identical to saved local checkpoint `53b8db7aa20fec7e31b1764c70479799da63538b`; commit identities differ because repository API commit metadata differs. Object creation and branch-ref publication are separate observed stages. PR#9/develop were not changed.
- Sole integration owner retains shared Cargo/config/protocol/core/router/main/CI/Git. Core/wrapper/packaging/Windows-path/notification/source-matrix lanes have returned their scoped checkpoints. The relay implementation lane is completing synthetic state-machine/Git-boundary regressions and strict checks. Independent inherited reviewer returned a checkpoint; final exact-SHA continuation review is still pending and has not been called clean.
- The authoritative README and [R/V contracts](RV-CONTRACTS.md) at `b454720b631f7004bff8a0ca6fe76900ae64ecc1` were read in full. Only those two documents were imported from the supplemental documentation branch; no older source/index tree replaced current work. [Behavior matrix](BEHAVIOR-MATRIX.md) preserves K01–K15/A01–A12, maps R01–R03, and assigns V01–V08 owners and next actions.

### Work after the published checkpoint

The following changes were working source when this status was prepared, not part of the published15db189d validation identity: relay's five HTTP operations and state/worktree/review/recovery owner; main/core/pre-ACK/database metadata composition; immutable trial notification denial; selected-root Windows spelling handling with held-root callers; production empty/relative log paths; session-prefix error ordering and distribution method fallback; CI Python pin and collector temp-path fixtures; current-lock dependency evidence and the R/V matrix. Their containing commit must be read from history after publication. No final source acceptance is implied by this list.

Relay reuses the existing admission, child launch, shared board, session, process and task owners. Headless DONE waits for observed process exit, and nonzero exit remains authoritative; this is an explicitly documented safety difference from the Go assumption that the old process has already exited, separate from R01–R03. Trial Git metadata must remain within the selected synthetic root; no real repository or provider is used for this evidence.

### Exact published CI and subsequent focused checks

- [Rust four-target run37252669282](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37252669282), head15db189d: all four jobs failed before aggregate tests/release builds. Linux Python3.10 lacked `tomllib`; macOS Intel/arm64 collector tests rejected the aliased temporary directory used by their own fixture; Windows strict Clippy rejected a Unix-only test import. Later test/build/artifact stages are **unrun**, not failed test results. Always-upload now retains failure receipts. Prepared fixes pin Python3.12, canonicalize only fixture roots (production archive/path rejection is retained), add alias-negative regressions, and cfg-scope the import. New native runs are pending.
- [Validate37252669305](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37252669305), head15db189d: Go build/test/vet/module verification succeeded on Windows, Linux and macOS; Web, instrumentation, third-party, tidy and govulncheck succeeded. Staticcheck/gosec failed on six additional standalone Rust-only Go oracle drivers entering `./...`. Explicit ignore build tags and matching generator output separate those drivers from production package discovery. Pinned Go1.26.8 `go list ./...` and all six explicit-file oracle invocations exited0, with JSON equal to the existing goldens. Fixed Go source and oracle behavior are unchanged. Scanner settings were not weakened.
- Earlier working-source aggregate1253/0 and strict Clippy exit0 remain the first repair receipt above. They are not fresh full-suite results for the relay/Windows/notification continuation. Focused later Linux checks: paths and three actual callers43/0; immutable trial notification policy6/0; relay program20/0 at its intermediate checkpoint; packaging/CI Python28/0 including aliased temporary-root reproduction. Current shared integration tests and further relay corrections require a fresh aggregate.
- Current strict Clippy attempt exited101 on a test enum typo and style diagnostics in new paths/relay code. Repairs are being checked. Formatting-only success does not replace compilation or tests. Final clean candidate all-targets/doctests/strict Clippy/two release binaries/four-target artifact hashes and actual build-input SBOM remain pending.

### Independent review and remaining gates

The independent inherited review's five findings concern lifetime DB ownership, degraded history/log startup, advisory shutdown delivery, trial self-bootstrap HOME, and registration instruction preparation before ACK. First checkpoint fixes were inspected, with explicit test limits. The newer empty/relative log and relay/main/core changes still require final review. The previously blocked same-root/two-live-Hub reproduction was not retried or delegated around; the primitive lock test is separate, and the missing integration receipt stays open.

R01 profile seed/user-hook preservation, R02 updater actual argv0 B, and R03 complete remote-script quoting have existing source/regressions and await exact final-candidate receipts. V01 failed Windows Job attachment with retained-grandchild pipes and V02 supported-version native ConPTY lifecycle are mandatory pending with the Windows process/ConPTY owner. V03 remains mandatory pending with integration/release-security owner: evaluate actual build/publish persistence and demonstrate immutable artifact transfer without real publish credentials. Build-only CI is not that sign-off. V04 filesystem/platform, V07 deployment/native identity and V08 installed-data/rollback/cutover gates remain with their assigned owners; only synthetic isolated fixtures are authorized here.

### Secret and dependency evidence

- PR-range [scan37252669269](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37252669269) retains the three historical migration findings; [push scan37252664956](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37252664956) succeeded. Neither implies the other scope succeeded. Exact fingerprint-only handling still awaits approval; `.gitleaksignore` and scanner configuration remain unchanged.
- Actual historical-value verification confirmed the two inventory fields at original commit7b937e0, lines8996/9026, equal SHA256s of that commit's `rust/src/approval/token.rs` and `rust/src/hub/auth.rs`. They are generated source hashes, not credentials. The third is a synthetic DNS-classification test address, replaced by RFC5737 at the supplied recovery commit. No raw flagged values are published.
- Separate gitleaks8.24.3 redacted `--all` scan covered847 local-ref commits at15db189d, exit7,38 findings: three known migration findings and35 older non-PR historical findings requiring individual triage. All result secret fields were redacted; a detected synthetic calibration marker was absent from report/log. The first sequential synthetic marker was not detected, so that attempt was not a passed redaction calibration. Private watchlists were unavailable and no secrets were externally uploaded. This full-history result remains unaccepted; no broad ignore or history rewrite is proposed.
- [V06 current-lock report](dependency-review/REVIEW.md): lock SHA2560466139df475d43177199440af0e9aa2455b26aabe55927cacb2d365f8cd0a12,316 registry archives and20,285 source files verified. Pinned RustSec October3 and KEV October4 snapshots have no active affected-range/alias matches within their stated scope. Publisher history is incomplete; SQLite3.53.2 later fixes/crafted-DB reachability and Cargo1.90 advisories require explicit assessment. Cached-source equality and zero range matches do not prove clean final-build/distribution safety. A cancelled metadata-collector poll was not retried or rerouted; saved completed receipts remain bounded evidence.

Next integration actions: finish current caller/relay checks, freeze and normally append the candidate, run all four clean native jobs and collect both binary hashes/notices, perform isolated actual-binary CLI/HTTP/WS/asset receipts, and obtain independent final-SHA review. Native GUI/device/provider/account/SSH/installed-data checks remain assigned pending. No merge/tag/release/registry publication, production replacement or Go cutover is authorized.


## 2026-10-05 relay repair checkpoint after interrupted validation

This preserves the existing staged continuation and its final unstaged repairs on the same branch/PR. The original source/index/patches were retained before integration. The prior committed public identity is `15db189db643143191e8b8063165fc6f2f5d2b14`; obtain this checkpoint's immutable identity from its containing commit and subsequent PR receipt. Publication is distinct from acceptance.

### New independent findings and fixes

The independent source review found cleanup/resume worktree deletion races, prefix-only trial Git metadata inspection, incomplete resume recovery records, retained-child replacement and recycled/cold session identity hazards. The current checkpoint adds:

- Cancellation-safe cleanup ownership, concurrent resume/cleanup rejection and final state/identity revalidation. This corrects a race also present in the fixed Go oracle.
- Cap+1 complete-input rejection for config/config.worktree, gitdir, commondir and alternates before trial Git execution.
- A durable stopped/hub_restart image before launch and nonterminal in-memory transition guarding, so interruption remains resumable. The registration callback persists the actual new child identity before implementing state.
- Resume refusal while a saved immutable child launch identity still exists. Retained/disconnected children must be explicitly closed first. Cleanup uses immutable launch identity plus expected-binding dismissal instead of trusting recycled numeric IDs.
- Persisted old-label revocation, installed before child dismissal and reinstated even from skipped completed records. Reattach checks it before persistence waits and before insertion, including when the actual metadata resolver returns None. Existing matching sessions are also recognized without relay metadata. This closes late/cold admission at the session owner; it does not prove native provider process exit.

The earlier inherited history degradation, advisory shutdown, bootstrap HOME, pre-ACK preparation and trial executable-resolution repairs received source-level review. The previously blocked two-live-Hub reproduction was not retried. Final immutable-SHA review remains pending.

### Actual checks and unaccepted work

- The latest completed focused Linux run, before the final metadata-free matching refinement, passed **67 tests, 0 failed, 0 ignored**, with951 filtered: `cargo test --manifest-path rust/Cargo.toml --locked --offline --lib relay -- --test-threads=8`, exit0. The final refinement adds an explicit actual-resolver/no-metadata retained-child regression; its execution and the new aggregate remain pending at this publication checkpoint.
- The resumed earlier aggregate exposed a fixture expecting404 instead of the contract's400 for an empty cleanup ID (library1012 passed/1 failed), then an integration fixture retained ordered registration effects while awaiting a later write. The owned hung run was interrupted, exit130. Completed summaries before interruption totalled1161 passed/1 failed. Both fixtures were corrected; no aggregate success is claimed.
- The attempted next strict-Clippy command was interrupted before execution was confirmed. Earlier Clippy and1253-test receipts do not apply to current changed bytes. Fresh full all-targets/doctests/Clippy, both release binaries, clean four-target CI/artifact hashes, actual-binary smoke and final exact-SHA review follow this checkpoint.
- [V03 source/workflow review](release-boundary-review/REVIEW.md) and its standalone credential-free custody model are prepared. The model ran22 cases with zero unexpected failures. Inert signature markers are not signatures, and separate directories are not isolated runners. Actual isolated native-artifact/signature evidence remains mandatory pending; production release workflow, permissions and credentials are unchanged.
- [V06 current-lock report](dependency-review/REVIEW.md) and [sanitized secret receipt](SECRET-SCAN-RECEIPT.json) remain bounded evidence. Historical fingerprint approval is still pending; `.gitleaksignore` and scanner configuration are unchanged. The cancelled upstream metadata poll was not retried.

Next: normally fast-forward this reviewable partial checkpoint, run its exact head on all four native targets, collect both binary/build-input identities, and finish final-SHA independent review. V01/V02 supported Windows lifecycle, V07 target/provider/device/remote and V08 installed-data/rollback/cutover acceptance remain assigned pending. No merge/tag/release, package publication or running-Go replacement is authorized.
