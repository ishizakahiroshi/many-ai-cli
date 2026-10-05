---
type: reference
status: draft
tags: [rust, recovery, review]
owner: unknown
review_status: draft
related: [README.md, PROGRESS.md]
last_reviewed: 2026-10-05
---

# #3 recovered Rust: independent review

Use an independent reviewer, not the implementation lane's self-review. Read README.md and actual recovered-source publication commit first. Review both the inherited recovered implementation against the fixed Go oracle and the continuation diff from that publication commit to the exact final code SHA. An empty continuation diff does not validate the inherited implementation.

Verify concrete binary callers/owners, registration order and failure cleanup, auth/Host/Origin/method precedence, WS first frame/admission/inputACK/reconnect/relay, instruction journals/tokens/recovery/drain, DB transactions/WAL/SHM/config restore, and all native/GUI/process/file boundaries. Check actual safe failure behavior, not only source string presence or a mock returning the desired shape.

Inspect deletes, test changes, snapshot/fixture generation and negative-path coverage. Preserve private-file held-handle checks, size-cap+1 rejection, exact Go string quoting, Unicode/VT behavior and trial denial of account/network/GUI/PID effects. Signal-registration errors must not become user shutdown signals. PATH lookup and config/profile aliases must not reach outside the explicit trial root.

Reconcile HTTP prefix/method/suffix branches and lexical WS false positives without overwriting historical route acceptance. A source reference, unit pass, local browser empty state and four-OS artifact each establish a different scope. Check Cargo.lock/toolchains/assets/version/source SHA and both binaries at the same actual candidate; dirty checkout refusal must remain active.

Report the reviewed SHA, commands/results, severity and reproducible finding evidence, explicitly untested native/provider/device/product gates, and review limits. Fixes require a new code SHA and impact-focused re-review before acceptance. CI success and no-findings reports must state the actual executed target/jobs and skipped checks. No merge/cutover/release or real account actions.
