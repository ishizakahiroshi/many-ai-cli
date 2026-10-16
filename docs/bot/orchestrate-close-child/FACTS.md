---
type: reference
status: draft
tags: [facts]
owner: user
related: [README.md, REVIEW.md, PROGRESS.md]
last_reviewed: 2026-10-16
---

# #P-20261016-013 facts

Each fact gives its source and the date it was checked. Line numbers are at `aae60ef4` unless stated otherwise.

## What happened (owner's machine, 2026-10-16, times in JST)

Checked on the owner's side on 2026-10-16 from the child CLI's own database and log, the orchestration board and the Hub's input record. The child's work content is private and is not reproduced here.

| Time | Event |
|---|---|
| 19:36:17 | The Hub started child #43 for conductor #39. The child ran opencode 1.19.0 with OpenCode's hosted model `big-pickle`. |
| 19:36:40 to 19:37:41 | The child worked normally. Its six model requests each returned within 5 to 11 seconds. |
| 19:37:42 | The child sent its seventh model request. Nothing came back: the CLI's log ends with the request, and the CLI's database holds an empty assistant message with no step start and no completion time. |
| 19:39:55 | The conductor ran `orchestrate send` with "cancel the task; change no files and finish". The Hub recorded it on the board and pasted it into the child's terminal. |
| 19:40:12 | The Enter key arrived. The CLI stored the text as a user message. No reply was ever created for it, because the CLI was still waiting on the seventh request. |
| 19:42:37 | The owner closed the child with the dashboard ✕. The board got `child dismissed from the dashboard`, and the CLI aborted the waiting request. |
| 19:43:16 | The conductor started a new child with another provider. |

## Code facts

- `send-child` always pastes the text and presses Enter, without an interrupt key: `rust/src/hub/child_control.rs` lines 249 to 257 call `start_input(..., text, true, false, ...)`. `start_input` (lines 284 to 341) sends ESC first only when `interrupt` is true (lines 300 to 302). `orchestrate send` accepts only `--role` (`rust/src/application/orchestrate_cli.rs` lines 299 to 319).
- The conductor has no command to close a child. `orchestrate` accepts `spawn`, `send` and `relay` (`orchestrate_cli.rs` lines 237 and 321 to 325); `relay stop` stops relays only (lines 446 to 483). The only way to close a child is the dashboard ✕, recorded by `rust/src/application/orchestration_program/lifecycle.rs` lines 40 to 66.
- A code path to close a session already exists: `SessionEngine::dismiss_bound` (`rust/src/terminal/session/lifecycle.rs` lines 843 to 849), used by relay cleanup (`rust/src/application/relay_program/lifecycle.rs` lines 350 to 359). The dashboard ✕ uses `dismiss_from_ui` (`lifecycle.rs` lines 850 to 857). Both go through `dismiss_authorized` (lines 858 onwards), which refuses while the session has pending spawn confirmations as a parent (lines 877 to 885).
- The Hub's own child watchdog would not have caught this case. In `orchestration_program/lifecycle.rs` lines 375 to 405 it measures activity as the latest of spawn time, last board write and last terminal output, and `last_output_at` is updated on every output chunk without any filter (`rust/src/terminal/session/observations.rs` lines 68 to 70). A child whose screen keeps redrawing (for example a spinner) never looks idle. Whether opencode redraws while it waits was not measured. Changing the watchdog is out of scope for this task.

## External facts

- opencode documents a default request timeout of 300000 ms (5 minutes), a header timeout of 5 minutes and a chunk timeout of 15 minutes: https://opencode.ai/docs/config/ (checked 2026-10-16). The owner closed the child 4 minutes 55 seconds after the request, so that timeout had not fired yet. What opencode does after the timeout was not checked.
- opencode's default key to interrupt a running session is Escape (`session_interrupt`): https://opencode.ai/docs/keybinds/ (checked 2026-10-16). Sending ESC before a message is not part of this task: the conductor wanted the child to stop, and closing it does that for every CLI.

## Owner decisions (2026-10-16)

1. A conductor may close its own children, also while they are working. It may never close another conductor's child.
2. Closing uses the same path as the dashboard ✕.
3. An earlier plan limited conductor closing to finished children. That limit is lifted by decision 1.
