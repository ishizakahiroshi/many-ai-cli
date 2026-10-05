---
type: reference
status: draft
tags: [rust-migration, recovery, source-audit]
owner: unknown
review_status: draft
related: []
last_reviewed: 2026-10-05
---

This snapshot records the current DERIVED source, separately from the historical `rust/src/hub/route_coverage.json`. The generator verifies that the historical file's SHA-256 does not change. Its original status values are retained.

The audit matches all 155 pinned Go HTTP registrations against the source inventory and records the Rust owner, route references, and actual main constructor or builder references. All 155 currently have source composition. This is evidence of implementation and binding; it does not establish correctness of every method, prefix suffix, configuration branch, or production service operation.

All 49 rows from the original lexical WS inventory are retained. Thirteen are incoming consumers. The 36 outgoing lexical rows include three DTO discriminators (`dir`, `file`, `text`) that are not WS message types. The `session_dismiss` outgoing literal is an internal relay request to Go's dismissal handler, not a socket emission; its Rust incoming consumer and `session_removed` response are present. The remaining outgoing entries have production Rust producer references. Source references are recorded independently of native acceptance.

The 32 CLI inventory entries record concrete binary dispatch references, including aliases, the no-argument path, usage relay's early native path, and the launcher binary. The audit does not claim that each command was executed against the user's installation. Setup, uninstall, tray, and native GUI behavior require separate synthetic and product acceptance.

The latest full receipt supplied to the generator is `resume-integrated-fiftysixth.log`: 1151 passed, zero failures, zero ignored, 31 summaries, exit0. It includes held transcript workers, instruction injection, boundary fixes, copied-data rollback and signal-error handling. Test execution was bounded to eight threads. Matching strict Clippy passed. A successful audit script remains source evidence and does not replace this separate Cargo receipt.

Unresolved integration and compatibility boundaries are explicit in the JSON: actual product operation of the composed instruction owner and platform/uninstall fixes, broad independent Go native corpora for Codex/Grok readers, optional services returning 503 when unavailable, and PID-only recovery's process-identity limitation. Trial rejects real GUI dispatch, external approval-pattern fetch, external PID termination, live push delivery, and vendor-file aliases outside its held root. No native product, browser, device, real provider/account, or destructive uninstall acceptance is recorded; accepted count is zero.

Regenerate with `python rust/scripts/audit-recovery-current.py --verification-log <all-target-log> --verification-exit-code 0` from this worktree. Without a supplied receipt the generator records source-only unverified status. The script performs no Cargo, Git, provider, server, account, or GUI calls. It verifies source registration counts, concrete CLI dispatch references, registration-owner composition, and the transcript worker's held-file parser boundary before writing only the new snapshot.
