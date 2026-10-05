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

Delivery: prepared locally; no immutable instruction commit, published recovered-source revision, delivery or acknowledgment exists yet. The published old PR cannot be used as the new candidate identity. Coordinator supplies the actual recovery branch/commit after review and authorized publication.

Local prior-source receipt: 1151 all-target tests / 0 failed / 0 ignored, strict Clippy, 3 doctests and fmt passed; both Windows release binaries built; main isolated CLI/HTTP/WS/settings/shutdown and desktop Chrome settings/version checked; standalone launcher --help checked. Source-entry digest: 307d2b5ecc7253dc73ba8f638a8d99fe53534e29aa86aafd319bc25e59276d78. These local reports require independent reproduction at the actual Git SHA and do not constitute product-wide acceptance.

| Stage | Owner | State | Evidence / next action |
|---|---|---|---|
| Source and instruction publication | Coordinator | prepared | Reviewed recovered source and these instructions must be published at an immutable commit |
| Existing #3 acknowledgment | dots | pending | Confirm no active duplicate, exact commit read and environment/dependency/CI capability |
| Remaining behavior matrix and fixes | dots disjoint lanes | pending | Fixed Go oracle, actual caller and failure-path evidence |
| Clean four-target CI/artifacts | dots integration owner | pending | Exact code SHA, jobs, skips, lock/assets and both binary hashes |
| Independent review and fixes | Separate reviewer | pending | Inherited source plus continuation diff; re-review final fixed SHA |
| Windows native/browser smoke and synthetic copied-data rollback regression | Local coordinator | limited local scope passed | Binary smoke and synthetic data regression have separate receipts; repeat against the returned candidate, with provider/device/native installed gates pending |
| Cutover/release | User/authorized local owner | not authorized | Running Go retained; full product gates required first |

Instruction SHA: pending publication. Code SHA: pending continuation. Reviewed SHA: pending. PR/CI/artifact links: pending. Do not put this board's own commit into its body as a self-reference; record it from history or a later receipt.

Append dated checkpoints with owners, changed paths, command/exit outcomes, failure classifications, actual code/review SHA and unresolved gates. Public entries must not contain private home paths, data, credentials or conversation locators.
