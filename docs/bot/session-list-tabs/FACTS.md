---
type: reference
status: draft
tags: [ui, evidence]
owner: user
related: [README.md, mock.html]
last_reviewed: 2026-10-08
---

# Facts — #P-20261008-007 many-ai-cli

Confirmed 2026-10-08. Behavior oracle: `4a3c141a1e6573459d3ff2a8cc6c9962bce6def3`, Rust candidate PR #11, branch `dots/rust-recovery-resume-3`.

- Owner approved the interactive mock and requested dots implementation. Tabs contain card lists; they are not one-tab-per-conversation shortcuts. Source: this task's [mock](mock.html) and README requirements.
- Add operations are distinguished by card-plus and tab-plus icons. Owner wants right-click rename/delete, tab drag reorder, and session movement between tabs. Source: mock and README.
- The mock stores synthetic state locally and demonstrates four buckets. Production starts with one bucket and existing sessions; the number is extensible. Browser persistence is part of the approved design, cross-device synchronization is not.
- `web/src/index.html` contains `#new-session-bar` and `#new-session-btn`; reuse the established sidebar and spawn flow.
- `web/src/app/sidebar-tree.ts` describes the one-tree placement invariant. `state.ts` derives session ordering from it and persists existing sibling/group ordering. `session-list.ts` uses it when rendering projects and roots. Keep these invariants.
- `web/src/app/user-prefs.ts` maps synchronized preference fields to localStorage and warns that the server PUT replaces the full preference snapshot. New browser-local bucket settings must not enter this mapping or require new backend fields.
- `web/package.json` defines `check` as `tsc --noEmit`; existing tests include sidebar-tree, session-strip, flexible-pane, project-view-memory and spawn-panel fixtures. Its aggregate test command builds frontend output, so distinguish local syntax validation from CI/test artifact generation.
- Local mock validation used synthetic browser drag/pointer events, reload and narrow-layout checks. It is design evidence, not a product/native-input acceptance receipt.
- Other local work on attachment handling is not part of the committed oracle or this task. Do not assume unseen fixes are on GitHub, and do not add them to this task's diff.

There are no real customer data, credentials or private machine paths required for this implementation. Keep tests synthetic. Read source at the fixed oracle and record any newer baseline explicitly before implementation.
