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

> 最終更新: 2026-10-05(月) 01:36:00 UTC

Repository: ishizakahiroshi/many-ai-cli. PR base: develop. Existing draft PR: #9. This continues existing #3; no replacement task number has been assigned.

Delivery: source and instructions published on `dots/rust-recovery-resume-3`; resume message prepared, delivery and acknowledgment pending. Initial publication commit: `7b937e0edc646a5b35d1b3ddd33bf86c3e2f2c5b`. Use the actual immutable commit supplied in the resume message, including subsequent publication fixes. The old PR cannot be used as the new candidate identity.

Local prior-source receipt: 1151 all-target tests / 0 failed / 0 ignored, strict Clippy, 3 doctests and fmt passed; both Windows release binaries built; main isolated CLI/HTTP/WS/settings/shutdown and desktop Chrome settings/version checked; standalone launcher --help checked. Source-entry digest: 307d2b5ecc7253dc73ba8f638a8d99fe53534e29aa86aafd319bc25e59276d78. These local reports require independent reproduction at the actual Git SHA and do not constitute product-wide acceptance.

| Stage | Owner | State | Evidence / next action |
|---|---|---|---|
| Source and instruction publication | Coordinator | published; CI cleanup in progress | Initial publication 7b937e0; actual final revision supplied in the resume message |
| Existing #3 acknowledgment | dots | pending | Confirm no active duplicate, exact commit read and environment/dependency/CI capability |
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
