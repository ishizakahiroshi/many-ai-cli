---
type: architecture-component
title: Approval UI and Marker Filtering
description: The browser-side approval action bar and batch-question panel, the single-source candidateKey+sourceEpoch identity model in approval-answered.ts, and the tag-stripping hub-marker-filter.ts that must tolerate CLI-owned alternate screens.
tags: [frontend, approval, hub-marker-filter, candidate-key, action-bar, batch-approval]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-4e05624b47e9514fe3968c32
    resource: repo://web/src/app/approval-answered.ts
  - id: openwiki-source-d6b1b0c48ae4480933b75234
    resource: repo://web/src/app/approval-owner.ts
  - id: openwiki-source-ade1c11a3772b4993b316bf6
    resource: repo://web/src/app/approval-parser.ts
  - id: openwiki-source-fd59600b2ce89a0c8ea7d408
    resource: repo://web/src/app/hub-marker-filter.ts
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## Two independent problems

The frontend's approval handling splits into two deliberately separate concerns: **deciding whether a given approval prompt is "the same one" the user already answered** (`approval-answered.ts`, `approval-owner.ts`), and **getting the Hub's `[MANY-AI-CLI]...[/MANY-AI-CLI]` / `[MANY-AI-CLI-DONE]...[/MANY-AI-CLI-DONE]` marker tags out of the terminal byte stream without corrupting what the CLI itself is drawing** (`hub-marker-filter.ts`). Both are written as DOM/xterm-free pure functions specifically so they can be exercised from `node --test` fixtures without a browser.

## Candidate identity: one pair of values, one place

`approval-answered.ts` is the single place that owns "identity" and "answered" state for approvals — its header comment records that this used to be split across three different state mechanisms (a timer-expiring signature, a permanent full-block-hash mark, and a manual-dismiss question key), each with a different definition of "same question," and the mismatches between them were the recurring cause of an already-answered approval reappearing. The current model uses exactly one pair per candidate:

- **`candidateKey`** — built from provider, approval kind, the normalized question text, and the sorted `optionNumber:sendText` pairs. Label whitespace, box-drawing characters, and line-wrap differences are excluded, so a TUI redraw does not change the key.
- **`sourceEpoch`** — the live prompt's generation counter. Replay and terminal reflow do not advance it, so an intentional re-ask of the same question text in a new generation is still shown, while the same content replayed within the same generation is recognized as already-answered.

`approvalCandidateIdentity()` prefers an explicit `candidateKey`/`sourceEpoch` pair carried on the wire from the Hub (via `_candidateKey`/`_sourceEpoch` annotations) over a locally computed fallback, which exists only for older Hub messages or browser-only parser paths. `isAnsweredApprovalShapeAcrossEpochs()` is a narrower, generation-blind query over the same ledger, used only to detect a stale-history repaint (content the user already answered, redrawn by the CLI's own scrollback paging) rather than to decide whether to show a *new* prompt.

`isHubMarkerAuthoritative()` governs a second kind of conflict: when the Hub's server-side marker detection has delivered a candidate for the current generation, the browser's own local text-tail scan is suppressed from raising a *new* candidate, because the local scan works off a naively concatenated PTY byte stream that misses characters an Ink-style differential redraw never re-sent — a logged 2026-08-26 incident had a re-parse turn "SSH して" into "SSH て" six seconds after the user answered, producing a different candidate key that fell outside the answered-ledger and made an already-answered approval light back up.

## Action bar ownership

`#action-bar` is one shared DOM element across all sessions; `approval.ts`'s `showActionBar` stamps `dataset.approvalSessionId` with the session it was drawn for. `approval-owner.ts` centralizes the single question "does the currently visible panel belong to session X" (`actionBarOwnedByOther`) so that both call sites that need the answer — discarding a stale panel on session switch, and deciding whether a redraw is needed — read the same logic instead of each carrying its own copy, which had previously let the two drift and reintroduce a bug where switching sessions could show another session's approval panel. An owner that was never stamped is treated as "unknown," not as "belongs to someone else" — an empty bar is not swept just because ownership can't be proven.

## Batch (multi-question) approvals

The approval parser distinguishes a **batch** payload — an array of `{num, title, options: [...]}` sections, detected by `isBatchOptions()` checking that the first array element carries an `options` array — from a single flat list of options. `candidateQuestionText()` in `approval-answered.ts` handles this by joining each batch section's title into the identity's question text, so a candidate key is computed over the whole multi-question set rather than per-question; the UI presents multiple numbered questions under one action bar with one shared submit action.

## `hub-marker-filter.ts`: stripping tags without touching CLI-drawn content

`hub-marker-filter.ts` is a pure function that strips the `[MANY-AI-CLI]`/`[/MANY-AI-CLI]` and `[MANY-AI-CLI-DONE]`/`[/MANY-AI-CLI-DONE]` tag byte sequences out of the raw PTY byte stream before it reaches xterm, while passing everything else — including the marker block's own body text and the CLI's cursor-position escape sequences — straight through unmodified. Its header comment documents an escalating sequence of designs (labeled "案 E" through "案 J" in the source) that converged on this rule the hard way:

- Earlier designs ("案 E"/"案 F"/"案 G") buffered the marker block's body, stripped ANSI escapes from it, and wrote it back at the *current* cursor position (or, for DONE blocks, dropped the body entirely). This corrupted the display whenever the wrapped CLI manages the screen through an alternate-screen buffer with absolute-coordinate cell addressing (as Claude Code's Ink-based UI does): the rewritten text landed on cells outside Ink's own bookkeeping and never got cleaned up by any subsequent redraw. Measured replays of real session logs (2026-08-26) reproduced line-for-line the exact corruption users had screenshotted, while simply stripping only the tag strings produced zero corruption.
- **"案 H"** (2026-08-26) settled on stripping only the tag literals for approval blocks — the CLI has already drawn the body correctly itself, so the filter adds nothing and removes nothing else.
- **"案 I"** (2026-09-02) found that the marker tag string itself can be split across a terminal line-wrap boundary — a CLI wrapping at its configured terminal width can interleave CR/LF, padding spaces, and cursor-movement CSI sequences into the middle of a marker literal — which made a strict contiguous-byte match miss the CLOSE tag entirely, latch the parser into "inside a block" indefinitely, and silently drop up to ~31% of a session's terminal output until a 32KB safety-valve forced a reset. The fix, `matchMarkerAllowingWrap`, matches a marker's literal characters while skipping over CR/LF/space/tab/complete ANSI escape sequences between them, but treats any *printable* interposed character as a non-match, so ordinary terminal text is never mistaken for part of a marker.
- **"案 J"** (2026-09-03) removed the last "discard until CLOSE" state machine entirely, after finding that on an alternate screen a redraw can contain an OPEN tag with the matching CLOSE never drawn at all — not wrapped, genuinely absent from that frame — which no amount of wrap-tolerant matching can find. `inDone`/`inMarker` are now only used to decide whether a stray CLOSE-shaped byte sequence should be accepted regardless of position; there is no longer any code path that buffers and discards terminal output, so the display can never freeze waiting for a CLOSE that never arrives.

A residual `MAX_MARKER_BUFFER_BYTES` (32KB) latch-reset threshold remains from the pre-"案 J" design; because "案 J" already passes marker-block body content straight through, output is never lost while the flag is (incorrectly) latched — the only consequence of an unreset latch is that a later stray CLOSE-shaped byte sequence could be accepted out of position, which the threshold bounds.
