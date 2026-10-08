---
type: reference
status: draft
tags: [ui, sidebar]
owner: user
related: [FACTS.md, REVIEW.md, PROGRESS.md]
last_reviewed: 2026-10-08
---

# #P-20261008-007 many-ai-cli: session-list tabs

Updated: 2026-10-08. Implement the approved [interactive mock](mock.html), using the actual application components and styles. Read [facts](FACTS.md), [independent review](REVIEW.md), and [progress board](PROGRESS.md).

## Target and purpose

Repository: `ishizakahiroshi/many-ai-cli` (public).
Work branch: `dots/P-20261008-007-session-list-tabs`.
Draft PR base: `dots/rust-recovery-resume-3` (Rust candidate PR #11).
Behavior baseline: `4a3c141a1e6573459d3ff2a8cc6c9962bce6def3`.
Management Issue: https://github.com/ishizakahiroshi/many-ai-cli/issues/12
Implementation diff starts at the first instruction publication commit `c29b58fa1c5bff2298197d9700c61b503b581b18`. That commit and the behavior baseline are different.

The sidebar currently shows many session cards in one vertical list. Add a tab strip between its existing toolbar and cards. Each tab contains multiple cards and may represent repositories or tasks. Selecting a sidebar tab changes the visible card list, not the active conversation or terminal. This is separate from existing conversation/files/git/pane workspace tabs and existing project groups.

The mock's four populated tabs are demonstration data. Real first use starts with one tab, `Tab 1` localized, containing all existing sessions. Users add tabs as needed. No fixed three/four-tab limit.

## Approved behavior

1. Keep toolbar and new tab strip visible while only the card list scrolls.
2. Change the existing new-session icon to a card outline plus a small plus. Keep its existing spawn workflow. Put a tab-outline-plus icon at the right edge of the new strip. Both have localized tooltips and accessible labels, without permanent text consuming strip width.
3. Add tab creates an empty tab immediately and shows it. Use stable unique IDs that remain unique after deletion/reload.
4. Tab context menu (right click, touch long press, keyboard) offers rename, delete, and left/right movement. Reject blank names; render names as text, ellipsize long names, expose full names in tooltips.
5. Delete an empty tab directly. For a populated tab, choose a surviving destination and move its session memberships before deleting. Never terminate/dismiss sessions or remove history. Keep at least one tab. Cancelling changes nothing.
6. Drag tabs to reorder with a visible insertion indicator. Drag a session card onto another tab to change membership with a distinct target highlight. Also provide a card context-menu destination chooser.
7. Preserve sidebar root/child relationships. Move a root and descendants together; an action on a child moves its whole root family with an explanatory label/message. Never modify `project`, `cwd`, or session parentage. Multiple independent roots of one repository may be split across tabs.
8. Additional tabs scroll horizontally; add-tab remains visible at the edge. Reveal the selected tab. Dragging near the strip edges scrolls it and stops on leave/drop/cancel/end.
9. Store tab names/order/memberships and selected tab/vertical scroll per tab in browser-local state scoped to the Hub origin. The approved mock is browser-local; do not add server synchronization or change Rust/Go preference schemas. Do not persist conversations, complete session records, or copied runtime status.
10. Use the application's stable session identity including lifecycle/start time, rather than a bare reusable session ID. Normalize older/malformed storage, missing tabs, duplicate memberships, and stale IDs. Unassigned/live sessions must stay visible in the default surviving tab. Do not permanently prune valid memberships against a partial initial snapshot.
11. A user-spawned session belongs to the tab captured when its spawn request starts. Match the actual creation response/event; switching or deleting a tab while spawning must not misassign/hide it. New externally created sessions use the default tab; children inherit their root's tab. Inspect all spawn callers and initial/reconnect/ongoing WS snapshots.
12. Show waiting/approval counts on inactive tabs from live Hub state. Keep existing notification/approval auto-switch semantics: if an existing action selects a session, reveal its owning tab as needed. A manual sidebar-tab switch alone does not select a session.
13. Long press opens menus without stealing normal touch scroll; movement/pointer cancellation clears the timer. Menu actions provide alternatives to desktop drag. Keyboard menus, Escape, focus restoration, and arrow navigation must work.
14. Reuse existing menu/modal/SVG/touch infrastructure and i18n (en/ja/vi). Match the application's visual language; do not ship mock-only reset/toast/synthetic data controls.

## Source entry points and boundaries

- `web/src/index.html`: `#new-session-bar`, `#new-session-btn`, sidebar.
- `web/src/app/session-list.ts`, `state.ts`, `sidebar-tree.ts`, `sidebar-tree-fixtures.ts`.
- `web/src/app/user-prefs.ts`, existing spawn and WS handlers, existing menu/modal, sidebar CSS, localization JSON.
- Introduce a focused sidebar-tab state module and meaningful fixture(s) if needed. Decide exact filenames after reading actual source.

`buildSidebarTree` remains the authoritative sidebar placement function. Derive/filter the visible bucket while preserving project, root, sibling order, favorites and collapse. Do not introduce an independent placement engine. Keep `sessionOrder`, `groupOrder`, pane ownership, existing session-card D&D to panes, and project reorder contracts intact. Use distinct drag payload kinds so tab reorder/session membership/project reorder/pane placement cannot consume one another's events.

User preferences have a full-snapshot PUT mapping. Keep new device-local state out of that mapping; unrelated preference saves must not erase new state or be mutated by tab actions. Session identity, default assignment, membership and synchronization races need pure-state tests and caller-level verification.

## Execution and permission

Implement sequentially: C2 state/migration/save → C3 UI/add/rename/delete → C4 D&D/move/touch/waiting. Shared files have one implementation owner. Assign a different dots worker to independent review using REVIEW.md, fix findings, and review the new code SHA. Return a Draft PR against the specified Rust candidate branch.

Allowed: code/test/docs commits and pushes to this task branch, Draft PR creation/update, progress-board updates and management-Issue comments, independent-review fixes, focused source validation using available existing tools/dependencies. Update CHANGELOG Unreleased and README for the actual implemented behavior.

Forbidden: direct push to main/develop/Rust base branch; merge/release/deploy; production Hub start/stop/reload; Go/Rust backend/config/API changes; credentials/real account probes/private data; new dependencies/services; CI-workflow changes; unrelated branches/Issues/Projects. Do not import another worker's unpublished attachment fixes. Local product builds/runtime acceptance belong to the owner. Do not turn inability to download dependencies into fabricated passing checks; report it and use the existing PR CI where available.

Stop for scope conflicts, new dependency/auth/service requirements, two failures of the same CI cause, unresolved severe findings after two fixes, unavailable required owner operation, or a stage exceeding 90 minutes. Otherwise continue sequentially without asking the owner to repeat already approved design decisions.

## Validation and delivery

Run `bun run check` from `web/`. Execute focused new fixtures and affected existing sidebar/tree/pane/spawn fixtures through the repository's available test path. CI may build artifacts as part of its existing configuration; do not edit workflows or trigger release/multi-OS release verification. Retain complete failure output locally, sanitize home paths to `~/...` in public reports.

Tests must cover initial/old/malformed storage, ID reuse, unassigned/root-family sessions, interrupted spawn/reconnect, uniqueness, rename/cancel/empty name, deletion/migration/last tab, moving/reordering, inactive waiting counts, storage exceptions and reload. Browser evidence should cover real render/callers, actual drag/drop, context menus, long press, overflow and save/reload where a browser is available. Mock tests alone do not prove product UI behavior.

Submit Draft PR URL, implementation code SHA, reviewed code SHA, test commands/exit results, CI scope/results, changed paths, remaining acceptance and rollback. The owner will inspect the real diff and perform a separate product/Windows acceptance; do not mark the whole task accepted.

## Communication

At acceptance, reply with recognized task number/collision result, instruction commit read, repository/branch access, TypeScript/Bun/Node/browser capabilities and dependency availability. Every reply's first line must be `#P-20261008-007 many-ai-cli：<report>` and remain in this task's original thread. Record acceptance/submission/review/stop transitions in the management Issue; PROGRESS links to those records. Distinguish sent/read-back/accepted/code submitted/reviewed/CI/owner accepted. No private conversation locators or machine paths in this public folder.

