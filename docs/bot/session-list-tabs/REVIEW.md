---
type: reference
status: draft
tags: [ui, review]
owner: user
related: [README.md, FACTS.md]
last_reviewed: 2026-10-08
---

# Independent review — #P-20261008-007 many-ai-cli

Use a worker independent of implementation. Review first instruction publication commit → exact implementation code SHA. State both SHAs; do not reuse an earlier PASS after a fix.

Check every README requirement against actual caller behavior, not only the new pure module:

1. Sidebar tab switch preserves conversation selection; existing session selection and approval auto-switch reveal correct ownership. No session disappears during startup/reconnect/spawn races.
2. Existing project/root/child/sibling/favorite/collapse invariants remain true. Root families move atomically; no project/cwd/parent change or duplicate membership.
3. Drag payloads cannot cross-consume pane placement, project reorder, tab reorder or session move. Real listeners render feedback and clear it on cancel/end/leave; edge auto-scroll cannot continue after drag.
4. Delete moves memberships first, preserves every session/history/process, supports cancel, and prevents deleting the final tab. Renaming cannot inject HTML. ID reuse/start-time changes and malformed storage cannot hide another live session.
5. New-tab IDs remain unique after remove/reload. Spawn affiliation is captured at request start and tied to the matching response; deleted target falls back safely. Externally created sessions stay discoverable.
6. Browser-local settings do not mutate the full-snapshot server preference API. Storage failure, partial snapshot reconciliation and per-tab scroll restoration are tested. Only configuration/identities are persisted, not conversations/runtime records.
7. Desktop/touch/keyboard, long name, empty bucket, many tabs, waiting/approval badge and fixed add icon are usable. Long press does not hijack vertical scrolling; menus/modals handle Escape, cancel and focus.
8. README/CHANGELOG/localization describe only implemented behavior. No mock-only buttons/data, new dependencies, Go edits, unrelated backend edits, workflow edits or unrelated private content. The approved 2026-10-09 requirement-11 extension permits focused Rust spawn/registration/WS changes only.

9. The optional request correlation value uses authenticated/trusted registration paths and cannot be confused with internal secrets/proofs, labels or another browser's pending request. All ordinary/grid/orchestration callers and reconnect/lifecycle serialization carry the exact contract. Cancellation, partial grid failure and response loss do not create duplicate or guessed ownership. Manual moves survive reconnect. State is bounded and never persists copied runtime records.

Report findings with severity, path, reproduction, expected/actual behavior, exact commands/results, reviewed SHA and unverified environments. Recheck affected behavior on each new fix SHA. CI/static success is separate from real product browser/Windows acceptance. The owner will perform additional independent diff/acceptance review before any integration.
