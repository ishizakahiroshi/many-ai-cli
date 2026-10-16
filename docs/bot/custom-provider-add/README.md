---
type: reference
status: draft
tags: [config, custom-providers, orchestrate]
owner: user
related: [FACTS.md, REVIEW.md, PROGRESS.md]
last_reviewed: 2026-10-16
---

# #P-20261016-012 many-ai-cli: add custom providers with one command (C3), then let orchestrate use them (C4)

Updated: 2026-10-16. Read [facts](FACTS.md), [independent review](REVIEW.md) and [progress board](PROGRESS.md) in full before starting. The owner approved all remaining steps of this work on 2026-10-16, so run stage 1 and stage 2 in order without asking again unless a stop condition below applies.

## Target

- Repository: `ishizakahiroshi/many-ai-cli` (public). Everything you write must be fit for a public repository.
- Instruction and base branch: `dots/custom-provider-add`. It is the Rust build branch `dots/rust-recovery-resume-3` at `aae60ef4` plus the config-safety work (C2) in `f4303e19` and `aa9c0505`, plus these instructions.
- Work branch: create `dots/custom-provider-add-P-20261016-012` from the final instruction commit given in the Slack message.
- Draft PR base: `dots/custom-provider-add`.
- Behavior baseline (oracle): `aa9c0505`. The implementation review diff starts at the first instruction commit, which is a different commit.
- Management Issue: https://github.com/ishizakahiroshi/many-ai-cli/issues/22
- First instruction commit (start of the implementation review diff): `20fcc4e5a6b957d25446b96302113244a1f5ddf7`.
- Only the Rust crate (`rust/`), the web UI (`web/`) where stated, and the docs listed below are in scope. The Go code is frozen. Do not edit it.

## Purpose

Today a user who wants another AI CLI in many-ai-cli (for example Meta's Muse Code, command `muse`) must hand-edit `custom_providers` in `~/.many-ai-cli/config.yaml`. The owner wants this instead: the user tells their own AI "add muse to many-ai-cli", and that AI reads `many-ai-cli provider add --help`, runs one command, and finishes without opening `config.yaml` and without the Hub token appearing anywhere. Afterwards the new provider can be started from the Hub and can be chosen as an `orchestrate` child.

Owner decisions that apply (2026-10-16):

1. Features are judged by how easy they are for users. Earlier restrictions on custom providers (for example "do not add them from the UI") are abolished.
2. A mistaken setting must never leave things running in a strange state. Overwrites keep the previous settings as a backup instead of deleting them. C2 implemented this; reuse it.
3. Muse sign-in and billing are on hold. Nothing in this work may require signing in to muse. No text you write (help, docs, messages) may suggest signing up for or buying a muse plan.

## Stage 1 (C3): `many-ai-cli provider add | list | remove`

The existing `provider` command (`rust/src/application/maintenance_cli/provider.rs`, `provider_command`) has `backup`, `reset` and `recover`, which act on the distributed provider registry, not on `custom_providers`. Add the new sub-actions next to them. `provider list` does not collide with `provider backup list`.

### 1. `provider add`

Example for muse:

```text
many-ai-cli provider add --id muse --label "Muse Code" --command muse \
  --headless-args exec --headless-format text --prompt-via arg \
  --approval-option "Allow once" --approval-option "Always allow in this workspace" --approval-option "Reject"
```

At `aa9c0505` the accepted values are `text` for the format and `arg` or `stdin` for `prompt_via` (`HEADLESS_FORMAT_TEXT`, `HEADLESS_PROMPT_VIA_ARG`, `HEADLESS_PROMPT_VIA_STDIN` in `rust/src/config/model.rs`, checked by `validate_headless_def`). Use those values in the help and docs. Rename `--approval-option` if section 2 shows a clearer name. `provider_command` today receives only the provider registry store, so it needs access to the config store for these sub-actions.

Before writing, run every check below. If any check fails, leave `config.yaml` byte-identical, create no backup, exit non-zero, and print which flag is wrong, why, and how to fix it. Report all failures at once, not only the first.

- The id: `custom_provider_id_problem` (empty, too long, bad shape, reserved), and not equal to an existing custom provider id or a built-in id.
- The id must also not equal a top-level CLI command name (the names `cli::parse` matches, such as `provider`, `config`, `serve`). Such an id cannot be started as a bare command. Apply this rule only in `provider add`. Do not add it to the loader's reserved list, because that would start dropping entries that users already have.
- The command line: `split_command_typed` (unclosed quote, control characters).
- The headless definition, when any `--headless-*` or `--prompt-via` flag is given: `validate_headless_def`.
- The executable `argv[0]` must be found the same way `doctor` finds it (`look_path`, used in `rust/src/application/diagnostics/supplemental.rs`). On Windows this must find `muse.cmd` on PATH. Write that case as a `#[cfg(windows)]` test if your environment cannot run it. The owner's side will run the Windows tests.
- The config file must be readable. `provider` is already outside `WORKS_WITHOUT_READABLE_CONFIG` (`rust/src/cli.rs`), so an unreadable file is refused with the same message as other commands. Keep that.

When all checks pass:

- First back up the current `config.yaml` with the C2 backup module (`rust/src/config/backup.rs`, for example `backup_current_in`). Do this every time, even if the rewrite would not lose anything, so that each successful add or remove creates exactly one new backup. If no `config.yaml` exists yet, no backup is needed. Say so in the output.
- Then write through the existing store path (`ConfigStore::persist` → `write_config` in `rust/src/config/model.rs`). Do not write YAML by hand. Make sure `write_config` does not create a second backup for the same operation.
- Print the entry that was added, the backup id, and the next step. Tell the user whether a running Hub picks up the new provider by itself or needs a restart. Trace how the Hub reloads `config.yaml` and state only what the code does.
- The rewrite re-serializes the file, so comments and layout are lost (`to_private_yaml`). If the previous file had comment lines, print one line saying the comments were not kept and that the previous file is in backup `<id>`.
- Never print the Hub token. Do not print any other line of `config.yaml`.

### 2. `--approval-option` (repeatable)

These are the strings the CLI shows on its approval prompt (muse: `Allow once`, `Always allow in this workspace`, `Reject`). The goal is that the Hub's approval detection works for the new provider, the same way it works for the built-in providers.

- Find out what the pattern strings in `rust/src/application/approval_patterns/defaults.json` represent and how they are matched. Store the user's strings in the same format.
- Today `read`, `write_custom` and `copy_official` in `rust/src/application/approval_patterns.rs` refuse ids outside `PROVIDERS`, while `reload` and `remote_batch` handle a raw `<id>.json`. Make the smallest change that lets a custom provider's patterns be written and loaded.
- Set `approval_pattern_source` only if the loader really supports a local source. Trace `SlashIo::read` and `rust/src/application/slash_commands/sources.rs`. If it does not, leave the field empty and say so in the PR.
- A later remote sync must not overwrite the patterns the user gave.
- `provider remove` keeps the pattern file and prints its path, so that adding the provider again restores its patterns.

### 3. `provider list` (and `--json`)

- List the built-in providers, marked as built in, and the custom providers with id, label, command, whether a headless definition exists, and whether the entry is in use or was dropped. A dropped entry shows its C2 `ConfigProblem` reason in the same words that `config check` uses (`text_en()`).
- No token, and no `config.yaml` lines other than these fields.

### 4. `provider remove <id>`

- Only custom ids. A built-in id is refused and the message says that built-in providers cannot be removed.
- An id that does not exist: error, no write, no backup.
- Back up before writing, the same as `add`.
- An entry that C2 dropped (because it is invalid) can also be removed by its id.

### 5. Guidance for AIs (`provider add --help` and `provider --help`)

Write the help so that an AI with no other context can do the job:

- What to look up before adding a new CLI: the non-interactive (headless) command or flag, whether the prompt goes in as an argument or on stdin, the exact text of the approval prompt options, the model flag, and which instruction files the CLI reads.
- Tell the AI to show the user which facts came from the CLI's official documentation and which were not documented, before running `provider add`.
- Give muse as a worked example. Do not include sign-in steps or plan names.
- Add one line to the text that child AIs receive (`rust/src/wrapper/delegation_prompt.txt`): to add another AI CLI, run `many-ai-cli provider add --help`.

### 6. Docs

- `README.md` and `README.ja.md`: the custom providers section explains `provider add | list | remove` with the muse example. Also review that section's wording about Gemini: Gemini is not supported, and its terms of service must be checked before support is built.
- `CHANGELOG.md`: one entry under `Unreleased`, using the existing style.
- Do not edit `README.vi.md`. It is a dated snapshot.

### 7. Tests (stage 1)

Use synthetic temporary homes only. Never touch a real home directory. At least:

- A successful add creates exactly one backup and the entry. A second add of the same id fails.
- Each invalid input (reserved id, top-level command name as id, duplicate, missing executable, unclosed quote, bad headless value) leaves `config.yaml` byte-identical and creates no backup.
- Remove of a built-in id is refused. Remove of a missing id writes nothing. Remove of a custom id creates exactly one backup.
- The output of `list`, `list --json`, `add` and `remove` never contains the token. Use a fixture token and assert that it is absent.
- `--approval-option` produces a pattern file that the loader returns for that id.
- The comment notice appears only when the previous file had comments.

## Stage 2 (C4): custom providers as `orchestrate` children

Start after stage 1 is committed.

1. `orchestrate spawn --provider <custom id>` must be accepted for custom providers that are in use, not dropped. Today `rust/src/hub/child_routes.rs` (about lines 175 to 192) and `rust/src/orchestration/child_launch.rs` (about lines 479 to 495) require membership in `ORCHESTRATION_PROVIDERS` (`rust/src/orchestration/child_options.rs`). Ordinary spawn already accepts custom ids (`rust/src/application/ordinary_spawn.rs`). Follow its rule.
2. An id that is neither built in nor a usable custom provider is still refused with HTTP 400. The message lists the valid ids, including the custom ones.
3. `orchestrate relay`: check `rust/src/application/relay_program.rs` and `rust/src/application/orchestration_program.rs` to see whether a custom provider can run as a relay role. If it can, accept it. If it cannot (for example because relay needs a headless mode the entry does not define), refuse with a message that says why, and put the same reason in the child-AI text.
4. Child-AI text (`rust/src/wrapper/delegation_prompt.txt`, lines 11 to 13 at `aa9c0505`): the sentence "The exact accepted values are claude, codex, copilot, cursor-agent, opencode, grok and command-code — all lowercase, and nothing else is accepted" must also allow the custom provider ids shown by `many-ai-cli provider list`. Keep the warning that values are lowercase and exact, and keep the rejected examples.
5. If the Hub's approval dialog for orchestrate spawn lets the user change the provider from a fixed list (`web/src/app/orchestration-roles.ts` and nearby), add the custom providers from `/api/info`, the same way `web/src/app/spawn-panel.ts` does. Keep the en, ja and vi text in step if you add any text.
6. Tests: a usable custom id is accepted, a dropped custom id is refused, an unknown id is refused with the list, and relay behaves as you decided in item 3.
7. `CHANGELOG.md`: one entry under `Unreleased`.

## Files you must not touch

Another session is editing these on the build branch. Touching them causes merge conflicts.

- `rust/src/application/main_program/context.rs`
- `rust/src/application/session_workers/branch_refresh.rs`
- `rust/src/files/branch.rs`, `rust/src/logging/` (all files), `rust/src/process/mod.rs`
- `rust/src/terminal/session/` (all files), except `rust/src/terminal/session/observations.rs`, which Supplement 1 below allows
- `rust/Cargo.toml`, `instrumentation.json`, `scripts/rust-local.ps1`

Also: do not edit `.omitnix/index.json`. The owner's side regenerates it when taking your commits. Do not add dependencies, do not change CI workflows, and do not edit Go code.

## Supplement 1 (2026-10-16): custom providers in approval detection

This answers the stop report https://github.com/ishizakahiroshi/many-ai-cli/issues/22#issuecomment-6213695185. It is a supplement, not a restart. Keep your local work and continue from where you stopped.

- Finding confirmed on the owner's side: in `rust/src/terminal/session/observations.rs` (lines 82 to 86 at `34974b3a`), `eligible` contains `!s.custom_provider`, so custom provider sessions get no marker extraction and no native approval detection. The flag is set in `rust/src/terminal/session/lifecycle.rs` line 361.
- The frozen Go version treats this differently, and Go is the behavior oracle here: `sessionApprovalDetectionEligible` in `internal/hub/approval_handler.go` (line 110) returns true for a built-in AI CLI or a registered custom provider, and false for `shell` and unknown providers. `internal/hub/custom_provider_approval_test.go` (`TestSessionApprovalDetectionEligible`) pins it. The Rust exclusion came with the first port of the session code and is a divergence, not a decision.
- Owner's decision: you may edit `rust/src/terminal/session/observations.rs`, and only that file under `rust/src/terminal/session/`. Remove the custom-provider exclusion from `eligible` so that custom provider sessions get the same marker extraction and native approval detection as built-in providers, using the phrases from their own `<id>.json` plus `common` (`approval_phrases_for`). Keep the `usage_probe`, `shell`, terminal-state and resize-debounce conditions. Do not edit `lifecycle.rs` (another session has uncommitted changes there). Keep the `custom_provider` flag itself and its other uses.
- If an existing Rust test pins the exclusion, change it to the Go rule and say so in the PR. Add a test: a custom provider session whose patterns include its approval text produces an approval candidate, and a `shell` session still does not.
- If anything else under `rust/src/terminal/session/` would need to change, stop again and report it.

## How to proceed (one approval, then automatic)

- Approval is given once, by this request. Unless a stop condition applies, run stage 1 and then stage 2 without asking. In the conversation, report only at acceptance, when you stop, and at the end. In the management Issue, write a short comment at every step: acceptance, progress, question, stop, submission and completion. Slack-only reports are not detected on the owner's side.
- Stop conditions (stop and report what is done and what remains): the same CI failure twice; a change outside the scope above; a new dependency, cost, credential or external service; a serious review finding that is still there after 2 fixes; a conflict between these instructions and the code; an action only the owner can do (a GitHub confirmation screen, CI not starting); one stage taking more than 90 minutes.
- End each commit message with the line `Agent: dot`. Do not merge.

## Verification

- Do not download dependencies in your work environment just for this task. If Rust 1.90.0 (`rust/rust-toolchain.toml`) and the crates are already available, run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and the tests you touched, from `rust/`, and report the exact commands and exit codes. `rust/build.rs` needs `web/dist/index.html`, which `bun run build` in `web/` creates. If you cannot build, say so. Do not report an unrun check as passed.
- PR CI: secret scan and validate run on pull requests. The Rust Windows check runs only on `dots/rust-recovery-resume-3`, so it will not run on your PR. The owner's side runs the full Windows suite (`cargo test --tests`, fmt, clippy, `bun run check`, `bun run test`, `bun run build`) on your PR head before taking it. Three Go-version tests are known to fail locally and are not caused by your work: `fixed_go_failed_job_attachment_*`, `closed_writer_rollback::closed_writer_copy_restore_*`, `fixed_go_cross_read_and_rollback_continuation`.
- Not done by you, done on the owner's side afterwards: starting `muse` from a real Hub, the Windows PATH lookup of `muse.cmd`, and looking at the screens.

## Independent review

After stage 2, a different dots agent reviews the whole diff from the first instruction commit to your PR head, following [REVIEW.md](REVIEW.md). Fix serious findings, then have the new head re-reviewed. Earlier passes do not carry over to a new head. After that, the owner's side reads every changed line before taking the work.

## Submission

- Code: commits on `dots/custom-provider-add-P-20261016-012` and a Draft PR into `dots/custom-provider-add`. In the PR body give the code SHA, the commands you ran and their exit codes, what you could not run, remaining items, and how to roll back.
- Non-code results (the review result, run receipts): Google Drive `bot/blob/2026/10/P-20261016-012/v1/`, at paths relative to this instruction folder (for example `results/REVIEW-RESULT.md`). Save all other files first, read them back and compare bytes and SHA256, then save `manifest.json` (relative path, bytes and SHA256 of every file) last. A corrected version goes into `v2`, never over `v1`. In the management Issue and in Slack, give the Drive relative path, the file count, the total bytes, and the manifest's bytes and SHA256.

## Allowed

- Pushing code, tests and the docs named above to the work branch `dots/custom-provider-add-P-20261016-012`, and creating and updating the Draft PR into `dots/custom-provider-add`.
- Comments on the management Issue.
- Saving files to Google Drive `bot/blob/2026/10/P-20261016-012/`: into the given version folder and every subfolder under it (such as `results/`), including creating those folders.
- Fixing independent review findings within the scope above.

## Not allowed

- Pushing to `dots/custom-provider-add`, `dots/rust-recovery-resume-3`, `develop` or `main`, force pushes, and merges.
- Saving non-code results (review results, receipts) to GitHub, and editing `PROGRESS.md` (the owner's side updates it).
- Writing to other Drive folders, and overwriting, moving or deleting existing Drive files.
- Editing the files listed under "Files you must not touch", Go code, CI workflows and dependency manifests.
- Creating other Issues or Projects, releases, deploys, and using credentials.
- Any text that asks the user to sign in to or pay for muse.

## When a save is cancelled

- Code: if saving to the work branch or the Draft PR is cancelled, check the state and resend at most twice. Creating blobs, trees and commits before moving the branch can simply be resent. Before creating or updating the branch, read the branch head: if it is still the head from before your send (or the branch does not exist yet, for a creation), resend; if it is already the commit you sent, it went through, so continue; otherwise stop. Before resending a Draft PR creation, check that no PR from the same branch exists. On a third cancellation, stop and put the base SHA and the `git format-patch` output into the Drive version folder. If a review explicitly refuses the action, stop without going to Drive and report the branch head and whether a PR exists.
- Drive: if a save is cancelled, first check whether a file with the same name and the same bytes is already there. If not, resend at most twice. If a file with the same name but different content exists, stop without overwriting. On a third cancellation or an explicit refusal, stop and report what arrived and what remains.
- "Do not retry" in these instructions only refers to the same cancelled operation.

## Acceptance reply

Reply in the same Slack thread and in the management Issue with: the number you recognize and whether it collides, the instruction commit you read, who implements and who reviews independently, and whether Rust 1.90.0, the crates and bun are usable in your environment. Every report starts with `#P-20261016-012 many-ai-cli：` on its first line. Keep replies inside this thread.
