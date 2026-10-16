---
type: reference
status: draft
tags: [orchestrate, hub, child-sessions]
owner: user
related: [FACTS.md, REVIEW.md, PROGRESS.md]
last_reviewed: 2026-10-16
---

# #P-20261016-013 many-ai-cli: let the conductor close its own child, even while the child is working (`orchestrate close`)

Updated: 2026-10-16. Read [facts](FACTS.md), [independent review](REVIEW.md) and [progress board](PROGRESS.md) in full before starting. The owner approved all remaining steps of this work on 2026-10-16, so run it to the end without asking again unless a stop condition below applies.

## Target

- Repository: `ishizakahiroshi/many-ai-cli` (public). Everything you write must be fit for a public repository.
- Instruction and base branch: `dots/orchestrate-close-child`. It is the Rust build branch `dots/rust-recovery-resume-3` at `aae60ef4` plus these instructions.
- Work branch: create `dots/orchestrate-close-child-P-20261016-013` from the final instruction commit given in the Slack message.
- Draft PR base: `dots/orchestrate-close-child`.
- Behavior baseline (oracle): `aae60ef4`. The implementation review diff starts at the first instruction commit, which is a different commit.
- Management Issue: given in the Slack message and in [PROGRESS.md](PROGRESS.md).
- Only the Rust crate (`rust/`) and the docs listed below are in scope. The Go code is frozen. Do not edit it.

## Purpose

A conductor session starts children with `many-ai-cli orchestrate spawn` and instructs them with `orchestrate send`. Today the conductor has no way to stop a child. On 2026-10-16 a child's AI CLI kept waiting for its model and produced nothing for about five minutes. The conductor noticed, decided to hand the task to a new child, and sent "cancel the task and finish" with `orchestrate send`. The child's CLI stored that text as its next message but never processed it, because it was still waiting on the model. The child stayed stuck until the owner closed it with the ✕ on the dashboard. Details are in [FACTS.md](FACTS.md).

Owner decision (2026-10-16): a conductor may close its own children, including a child that is still working. It may never close another conductor's child. Closing must go through the same path as the dashboard ✕, so the CLI process ends, the session leaves the Hub, and the child stops counting toward the per-parent child limit.

## The change

### 1. CLI: `many-ai-cli orchestrate close --role <role> [--id <child session id>]`

- Add `close` next to `spawn`, `send` and `relay` in `rust/src/application/orchestrate_cli.rs` (`OrchestrateCli::run`, about lines 234 to 326 at `aae60ef4`). Parse flags with the same `flags::parse` helper. `--role` is required. `--id` is optional and selects one child when several children have the same role.
- The calling conductor is identified the same way as for `send` (`MANY_AI_CLI_SESSION_ID`, then the Hub port and token from the environment). Never print the token.
- Update every usage string that lists the subcommands: `orchestrate_cli.rs` line 237 (`orchestrate <spawn|send|relay>`) and lines 321 to 325 (`want spawn|send|relay`), and `rust/src/cli.rs` line 117 (`orchestrate <spawn|send>`). Also update `orchestrate --help` wherever that text lives.
- Print one line with the closed child's role, session id and the state it was in when it was closed. On refusal, exit non-zero and print the Hub's reason.

### 2. Hub: `POST /api/sessions/{parent}/close-child`

- Body: `{"role": "<role>", "id": <optional child session id>}`.
- Register it beside `send-child`: `is_control_route` (`rust/src/hub/child_control.rs` lines 72 to 78), the match in `ChildControl::handle_authenticated` (lines 120 to 264), the router entry near `rust/src/hub/router.rs` line 1800, and both `send-child` entries in `rust/src/hub/route_coverage.json` (near lines 1714 and 3842). Follow the existing shape exactly.
- Choosing the child:
  - Normalize the role exactly as `send-child` does (`child_control.rs` lines 156 to 159).
  - Consider only sessions whose `parent_session_id` is the caller. A child of another conductor must never be closed. Answer as if it does not exist (404), so the response does not reveal other conductors' children.
  - Unlike `send-child` (lines 169 to 191), include children in finished states (`completed`, `done`, `error`, `timeout`, `disconnected`) and children that are not connected, because closing a finished child frees its slot. Exclude children already removed from the Hub.
  - With `id`, close that child only if it matches the caller and the role. Without `id`, close the newest matching child (highest session id), the same rule `send-child` uses.
  - No match: 404 with a message that lists the caller's current children as `role=<role> id=<id> state=<state>`.
- Closing: call `SessionEngine::dismiss_bound(binding, now)` (`rust/src/terminal/session/lifecycle.rs` lines 843 to 849) and apply the returned effects the way relay cleanup does (`rust/src/application/relay_program/lifecycle.rs` lines 350 to 359). That is the same removal the dashboard ✕ uses (`dismiss_from_ui` → `dismiss_authorized`), so the wrapper receives `session_dismissed` and ends the CLI. Do not kill the process by any other means.
- `dismiss_authorized` refuses when the session has pending spawn confirmations as a parent (`lifecycle.rs` lines 877 to 885, it broadcasts `session_dismiss_refused`). Detect that case and answer 409 with a message that says why. Do not edit `rust/src/terminal/session/lifecycle.rs` (see "Files you must not touch"). If you cannot detect the refusal without editing it, stop and report.
- Response on success: `{"ok": true, "session_id": <id>, "role": "<role>", "state_before": "<state>", "board_path": "<path>"}`.

### 3. Board record and conductor notice

- Today, when a child session ends with state `dismissed`, `OrchestrationProgram::session_ended` (`rust/src/application/orchestration_program/lifecycle.rs` lines 14 to 67) writes `child dismissed from the dashboard ... (closed by the user; its work may be unfinished)` to the board and queues `[orchestration] child dismissed ... (closed from the dashboard by the user ...)` for the conductor.
- When the conductor closed the child, write instead, on the board: `child closed by the conductor: role=<role> session=<id> (state when closed: <state>; its work may be unfinished)`, and to the conductor: `[orchestration] child closed by you role=<role> id=<id> state_before=<state> (treat its work as unfinished unless it reported DONE)`. Keep the dashboard wording unchanged when the user closes a child.
- Carry the "closed by the conductor" fact from the close request to `session_ended`, for example by marking the board's child entry in `OrchestrationProgram` before calling `dismiss_bound`. Choose the smallest change. The mark must not leak to a different child.

### 4. Text the conductor AI reads

- `rust/src/wrapper/delegation_prompt.txt`, the command list at lines 3 to 7: add one line, for example `- Stop and close a child (also while it is working): many-ai-cli orchestrate close --role <role>`. Another task (#P-20261016-012) edits lines 11 to 13 of this file. Do not touch those lines.
- The conductor operating rules in `rust/src/application/session_workers/transcript_path.rs` (the text at line 187): add a rule that a child which is waiting on its model or has stopped making progress does not read new messages, so to cancel its task the conductor should close it with `<MANY_AI_CLI_BIN> orchestrate close --role <role>` and spawn a new child, instead of sending a stop message.
- Keep the existing style and keep the token warning ("Do not call the Hub HTTP API or handle any auth token directly").

### 5. Docs

- `README.md`, section "Light orchestration": one or two sentences on `orchestrate close` (who may close which child, that it also works while the child is working, and that the board records it). Mirror the same content in `README.ja.md`.
- `CHANGELOG.md`: one entry under `## [Unreleased]`, in the existing style.
- Do not edit `README.vi.md`. It is a dated snapshot.

### 6. Tests

Use synthetic sessions and temporary homes only. Put new tests in a new test module for `child_control` (for example `rust/src/hub/child_control/tests.rs` or a `#[cfg(test)]` module in `child_control.rs`). Do not edit `rust/src/hub/child_routes/tests.rs`. At least:

- A working child of the caller is closed: it leaves the session engine, the board gets the conductor wording, and the conductor gets the conductor notice.
- A finished (`completed`) child of the caller is closed the same way.
- A child of another conductor is refused with 404 and stays.
- An unknown role is refused with 404, and the message lists the caller's children.
- With two children of the same role, `id` closes exactly the one named; without `id` the newest is closed.
- The refusal case (pending spawn confirmations) answers 409 and the child stays.
- A dashboard ✕ dismissal still writes today's dashboard wording.
- CLI: `orchestrate close` without `--role` fails with the usage line, and the usage strings list `close`.

## Files you must not touch

Other sessions are editing these on the build branch. Touching them causes merge conflicts.

- `rust/src/terminal/session/` (all files). You may call `dismiss_bound`, but do not edit `lifecycle.rs` or any other file there.
- `rust/src/hub/websocket.rs`, `rust/src/hub/ws_transport_tests.rs`
- `rust/src/hub/child_routes.rs`, `rust/src/hub/child_routes/tests.rs`, `rust/src/orchestration/child_launch.rs`, `rust/src/orchestration/child_options.rs` (task #P-20261016-012 works there)
- `rust/src/application/instruction_rules.rs`, `rust/src/application/instruction_rules/tests.rs`
- `rust/src/application/main_program/context.rs`, `rust/src/application/session_workers/branch_refresh.rs`
- `rust/src/files/branch.rs`, `rust/src/logging/` (all files), `rust/src/process/mod.rs`
- `rust/Cargo.toml`, `instrumentation.json`, `scripts/rust-local.ps1`

Also: do not edit `.omitnix/index.json`. The owner's side regenerates it when taking your commits. Do not add dependencies, do not change CI workflows, and do not edit Go code.

## How to proceed (one approval, then automatic)

- Approval is given once, by this request. Unless a stop condition applies, implement, test, have it reviewed independently, and fix findings without asking. In the conversation, report only at acceptance, when you stop, and at the end. In the management Issue, write a short comment at every step: acceptance, progress, question, stop, submission and completion. Slack-only reports are not detected on the owner's side.
- Stop conditions (stop and report what is done and what remains): the same CI failure twice; a change outside the scope above, including any need to edit a file under "Files you must not touch"; a new dependency, cost, credential or external service; a serious review finding that is still there after 2 fixes; a conflict between these instructions and the code; an action only the owner can do (a GitHub confirmation screen, CI not starting); the work taking more than 90 minutes.
- End each commit message with the line `Agent: dot`. Do not merge.

## Verification

- Do not download dependencies in your work environment just for this task. If Rust 1.90.0 (`rust/rust-toolchain.toml`) and the crates are already available, run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and the tests you touched, from `rust/`, and report the exact commands and exit codes. `rust/build.rs` needs `web/dist/index.html`, which `bun run build` in `web/` creates. If you cannot build, say so. Do not report an unrun check as passed.
- PR CI: secret scan and validate run on pull requests. The Rust Windows check runs only on `dots/rust-recovery-resume-3`, so it will not run on your PR. The owner's side runs the Windows tests on your PR head and reads every changed line before taking it.
- Not done by you, done on the owner's side afterwards: closing a real child from a real Hub and looking at the dashboard.

## Independent review

After implementation, a different dots agent reviews the whole diff from the first instruction commit to your PR head, following [REVIEW.md](REVIEW.md). Fix serious findings, then have the new head re-reviewed. Earlier passes do not carry over to a new head.

## Submission

- Code: commits on `dots/orchestrate-close-child-P-20261016-013` and a Draft PR into `dots/orchestrate-close-child`. In the PR body give the code SHA, the commands you ran and their exit codes, what you could not run, remaining items, and how to roll back.
- Non-code results (the review result, run receipts): Google Drive `bot/blob/2026/10/P-20261016-013/v1/`, at paths relative to this instruction folder (for example `results/REVIEW-RESULT.md`). Save all other files first, read them back and compare bytes and SHA256, then save `manifest.json` (relative path, bytes and SHA256 of every file) last. A corrected version goes into `v2`, never over `v1`. In the management Issue and in Slack, give the Drive relative path, the file count, the total bytes, and the manifest's bytes and SHA256.

## Allowed

- Pushing code, tests and the docs named above to the work branch `dots/orchestrate-close-child-P-20261016-013`, and creating and updating the Draft PR into `dots/orchestrate-close-child`.
- Comments on the management Issue.
- Saving files to Google Drive `bot/blob/2026/10/P-20261016-013/`: into the given version folder and every subfolder under it (such as `results/`), including creating those folders.
- Fixing independent review findings within the scope above.

## Not allowed

- Pushing to `dots/orchestrate-close-child`, `dots/rust-recovery-resume-3`, `develop` or `main`, force pushes, and merges.
- Saving non-code results (review results, receipts) to GitHub, and editing `PROGRESS.md` (the owner's side updates it).
- Writing to other Drive folders, and overwriting, moving or deleting existing Drive files.
- Editing the files listed under "Files you must not touch", Go code, CI workflows and dependency manifests.
- Creating other Issues or Projects, releases, deploys, and using credentials.

## When a save is cancelled

- Code: if saving to the work branch or the Draft PR is cancelled, check the state and resend at most twice. Creating blobs, trees and commits before moving the branch can simply be resent. Before creating or updating the branch, read the branch head: if it is still the head from before your send (or the branch does not exist yet, for a creation), resend; if it is already the commit you sent, it went through, so continue; otherwise stop. Before resending a Draft PR creation, check that no PR from the same branch exists. On a third cancellation, stop and put the base SHA and the `git format-patch` output into the Drive version folder. If a review explicitly refuses the action, stop without going to Drive and report the branch head and whether a PR exists.
- Drive: if a save is cancelled, first check whether a file with the same name and the same bytes is already there. If not, resend at most twice. If a file with the same name but different content exists, stop without overwriting. On a third cancellation or an explicit refusal, stop and report what arrived and what remains.
- "Do not retry" in these instructions only refers to the same cancelled operation.

## Acceptance reply

Reply in the same Slack thread and in the management Issue with: the number you recognize and whether it collides, the instruction commit you read, who implements and who reviews independently, and whether Rust 1.90.0, the crates and bun are usable in your environment. Every report starts with `#P-20261016-013 many-ai-cli：` on its first line. Keep replies inside this thread.
