---
type: architecture-component
title: "Session Views: Multi-Pane, Detached Grid, and Mobile"
description: The alternate ways the Hub UI renders live sessions beyond one-terminal-at-a-time — the multi-pane grid, pop-out detached windows, the sidebar's single-tree placement rule, and the mobile-specific lite/home views.
tags: [frontend, multi-pane, detached-grid, sidebar-tree, mobile, chat-history]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-a2fe66d68c72192cb107a241
    resource: repo://web/src/app/chat-history.ts
  - id: openwiki-source-f5b469c2e335c80bec0f6d17
    resource: repo://web/src/app/detached-grid.ts
  - id: openwiki-source-35a15cde66a88a6e49d5d0f2
    resource: repo://web/src/app/mobile-home.ts
  - id: openwiki-source-8f42e874b3a3b8eef6f4c579
    resource: repo://web/src/app/mobile-terminal-lite.ts
  - id: openwiki-source-196daea0cb3951191a3910b0
    resource: repo://web/src/app/multi-pane.ts
  - id: openwiki-source-a1ebf1e25581aa987010bcb4
    resource: repo://web/src/app/sidebar-tree.ts
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## Sidebar placement: one tree, one rule

`sidebar-tree.ts` is, by its own header comment, "the only place that decides where a sidebar card appears," reduced to one governing sentence: *the sidebar draws one tree, and the tree's shape is decided by data alone — user action changes only the ordering of siblings under the same parent, and whether a node is collapsed, never the shape of the tree itself.* This was extracted as a pure, DOM-free module (verified against invariants in `sidebar-tree-fixtures.ts`) after the placement logic had grown to four separate mechanisms living inside `renderSessionList` — `sessionOrder`, `groupOrder`, `projectFavorites`, and `pinned` — the last of which did not just reorder but actually rewrote which container a session belonged to, and because the logic lived inline in DOM-building code, it could not be unit tested and the "one tree" rule was enforced only by comment. Three invariants are now fixed in code: a child node always lands under a root ancestor from the same project; no node crosses a container for any reason other than a color filter; and every linear ordering the UI needs (multi-pane slot order, swipe order) is derived from exactly one depth-first traversal of that same tree, rather than each view computing its own order.

## Multi-pane grid

`multi-pane.ts` implements `MultiPaneManager` plus its `GridPicker` popup (a 6×3 = 18-cell grid picker with preset buttons for choosing a layout). The manager owns a fixed number of visual "slots," each independently able to attach a live session's xterm instance (`attachToSlot`), detach it (`detachSlot`), or sit empty; sessions can be dragged from slot to slot (`_wireDropTarget`/`_reorderSlots`), and each slot gets its own `ResizeObserver`-driven terminal fit (`_installResizeObserver`/`_fitTerminalInSlot`) plus its own bottom-follow/scroll-button wiring, since the panes are independently sized and scrolled. Slot layout itself is grid-template-driven with draggable splitters (`_applyGridTemplate`/`_buildSplitters`/`_wireSplitter`) rather than fixed-size panes.

## Detached Session Grid

`detached-grid.ts` is the display layer for a session grid popped into its own browser window/tab — its own header comment states explicitly that "session life/death, approval, and completion are managed by the Hub itself; this module handles only display (xterm attach/resize/focus)." It parses its own operating parameters straight from the URL (`?view=detached-grid&layout=2x2&session_ids=1,2,3,4&token=...` via `parseDetachedGridParams()`) rather than from any shared app state, since a detached window is a separate page load with its own `location.search`. It reuses the same WebGL-renderer enable/disable and alt-scroll-rail helpers from `terminal.ts` that the main grid and multi-pane views use, rather than reimplementing terminal rendering for the pop-out case.

## Mobile: two different substitutions, not one responsive layout

Rather than a single responsive layout that reflows at narrow widths, the mobile experience is built from two purpose-specific modules, both gated behind the same `matchMedia('(max-width: 720px)')` check and both designed as strict additions that never run any code path when that check is false:

- **`mobile-home.ts`** renders `#mobile-home` and a left drawer — a session-list-centric monitoring view (search, per-status buckets: pending/running/waiting/error) distinct from the desktop sidebar tree. It works around a specific mobile browser quirk: when a `session_update` or approval-queue event rebuilds `body.innerHTML` mid-gesture, the row `<button>` a finger is resting on can be removed from the DOM before the tap completes, and iOS silently drops the resulting `click` event — so row taps are captured on `pointerdown` (remembering the target id) and only committed on a window-level `pointerup`, the same pattern used by the desktop session-card pointer handling.
- **`mobile-terminal-lite.ts`** replaces the full xterm.js rendering with a chat-transcript-style view built from clean-text diffs of `scanBuffer(activeSessionId)`. Its header comment is explicit that this is a display substitution only: xterm.js keeps running in the background exactly as on desktop, so approval detection, PTY reception, and scrollback retention are entirely unaffected — nothing about the underlying data path changes, only how it is presented on a narrow screen.

## Chat/transcript history (`chat-history.ts`)

`chat-history.ts` (marked, like `state.ts`, as "extracted from `app.js`... classic-script global scope; no module wrapper") renders the bubble-style conversation history view, and includes its own ANSI-stripping (`stripAnsiBasic`) and "thinking spinner" line filter (`isThinkingNoiseLine`) so that reconstructed chat bubbles do not carry escape-sequence noise or the repeated redraw frames of a CLI's animated "thinking..." status line (matched against a restricted set of spinner glyphs — star dingbats U+2722–U+273F and Braille patterns — deliberately narrow so ordinary symbols like ✓/✗ are never mistaken for spinner noise); its own comment notes this mirrors the equivalent server-side classifier, `sessionlog.IsThinkingNoiseLine`, so the same "what counts as noise, not conversation" rule is applied on both sides rather than only when generating clean transcripts server-side.
