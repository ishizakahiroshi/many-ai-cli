---
type: reference
status: draft
tags: [rust-migration, recovery, source-audit]
owner: unknown
review_status: draft
related: []
last_reviewed: 2026-10-05
---

# Current recovery source and receipt scopes

> 最終更新: 2026-10-06(火) 01:32:26 UTC

This snapshot is bound to immutable code `4dc961ce600d12aa0911e5a9620e50411ab13d9f` and its 444 .rs files under rust/src. Source-reference positions and SHA256 values are refreshed from actual files:60 of the prior427 files changed and17 were added. A later documentation commit must not be substituted for this tested code identity.

All 155 pinned Go HTTP registrations have concrete Rust owners and main composition, and all 32 CLI inventory entries have binary dispatch, including aliases/default, usage-relay's early native path and the launcher. The source-only auditor completed successfully against4dc961ce with output redirected to an isolated review draft. No `todo!` or `unimplemented!` invocation was found under rust/src. These are bounded source observations, not proof that every command, method, dynamic suffix or configuration is correct.

`rust/src/hub/route_coverage.json` now has an authoritative `current_source_coverage` overlay with 155 source-bound registration rows and49 mapped dynamic operations. Its existing top-level rows, old status values and receipts remain unchanged and are labelled historical. Their71 partial/84 unresolved registrations and10 partial/39 unresolved dynamic operations must not be reported as current missing implementations. The original file is available at [immutable4dc961ce source](https://github.com/ishizakahiroshi/many-ai-cli/blob/4dc961ce600d12aa0911e5a9620e50411ab13d9f/rust/src/hub/route_coverage.json). The existing Rust test's legacy schema/enum is preserved; no test or runtime source changes are required by this overlay.

All 49 historical lexical WS rows are retained:13 incoming consumers,32 actual outgoing producer literals,three DTO discriminators (`dir`, `file`, `text`) and one internal Go `session_dismiss` request rather than a WS emission. The incoming consumer and `session_removed` response are present. This lexical inventory cannot measure every DTO field or lifecycle behavior.

The current receipt is [VALIDATION-4dc961ce.json](../../docs/bot/rust-recovery-resume/VALIDATION-4dc961ce.json), with [artifact identities](../../docs/bot/rust-recovery-resume/ARTIFACTS-4dc961ce.md): Windows library 987/all-targets 1266; Linux 1047/1354; both macOS architectures1047/1353. Zero tests failed or were ignored. Three doctests per target are separate; library is a subset of all-targets. Fmt, strict all-target Clippy and both binaries passed. Keep the native target/attempt records separate, and do not sum them into a unique test count.

The old 1151 local Windows all-target receipt (`resume-integrated-fiftysixth.log`,31 summaries,exit0) remains under `historical_verification`. The older 1149 all-target paragraph did not bind an exact SHA/OS/log;1253 was a pre-commit Linux all-target working-source aggregate (32 summaries). Neither is the final987 Windows library-only count. The original generator accepts one text log and hard-codes a local-Windows scope: running it cannot reconstruct the new per-target CI receipt fields. Regeneration must preserve those separately verified metadata and the historical/current overlay distinction rather than replacing them with a single aggregate.

G1–G4 are source-closed and their composed HTTP regression passed in the 4dc961ce native library suites. At the immutable4dc baseline, the identified missing runtime implementation was the empty Windows Whisper supplemental `runtime_payload`; route/manager composition exists, but the owner has superseded the fixed-pin prerequisite with the existing Go VS-only acquisition policy and per-build version/hash receipts; this continuation implements that path, with final native receipt pending. Separately, the owner reports all 444 .rs files under rust/src reviewed by 11 separate-product AI reviews with zero high-severity findings and nine lower-severity fixes requested. Those fixes and additions10/11 were implemented at646637281c1b9f90c43434f364da2c03d4ba2203. Its native CI has two Windows failures and one Linux failure; the authorized A/B follow-up is in progress. Implementation status is distinct from final validation/product acceptance. The broader native/runtime, provider, browser/device, GUI/SSH, installed-data, signed distribution and cutover acceptance gates remain pending; product-accepted count stays zero. Optional-service503 can be an expected unavailable contract and is not successful configured-service acceptance. V03 remains documentation/evidence-only and production release.yml is unchanged.

Full38-commit PR coverage is separately established by the verified union of the 30-commit PR scan, six one-commit push scans and final two-commit push. This does not fix the action's pagination limit or accept older history findings/private-watchlist gaps. No new scan or exclusion is part of this source reconciliation.
