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
