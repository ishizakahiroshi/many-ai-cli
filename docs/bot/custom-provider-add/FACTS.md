---
type: reference
status: draft
tags: [config, custom-providers, orchestrate]
owner: user
related: [README.md, REVIEW.md, PROGRESS.md]
last_reviewed: 2026-10-16
---

# #P-20261016-012 facts

Each fact has its source and the date it was checked. "Read" means the owner's side opened the source. "Located" means a read-only code search found it and the exact behavior still has to be traced by the implementer. Code paths are at commit `aa9c0505` unless stated.

## Owner decisions (2026-10-16)

| Decision | Source |
|---|---|
| Features are judged by how easy they are for users. Earlier restrictions on custom providers, including "do not add them from the UI", are abolished. | Owner, in the request conversation |
| A mistaken setting must not run in a strange state. Reset and overwrite keep the removed settings as a backup. One wrong entry is dropped with the reason shown, without stopping everything. | Owner. Implemented as C2 in `f4303e19` and `aa9c0505` |
| Muse sign-in and billing are on hold. The work must not need sign-in, and must not suggest signing up or paying. | Owner |
| Whether the Hub screen also gets an "add provider" form is decided after the command exists. It is not part of this request. | Owner's plan |
| All remaining steps of this work are approved. | Owner |

## Current code (C2 is included)

| Fact | Source | Status |
|---|---|---|
| `CustomProvider { id, label, command, approval_pattern_source, headless }` and `HeadlessDef { args, format, prompt_via }` | `rust/src/config/model.rs` about lines 625 to 648 | Located |
| Id checks: `valid_id_shape`, `is_reserved_provider_id` (built-ins plus `shell`), `custom_provider_id_problem`, `validate_custom_provider_id` | `rust/src/config/model.rs` about lines 3457 to 3551 | Located |
| Command splitting: `split_command_line`, `split_command_typed` (quotes, `""` escape, control characters refused, unterminated quote is an error) | `rust/src/config/model.rs` about lines 4822 to 4905 | Located |
| Headless values: format `text`, prompt_via `arg` or `stdin`; `validate_headless_def` | `rust/src/config/model.rs` lines 4249 to 4252, about 4536 | Read |
| Nothing under `rust/src/config/` checks PATH. `doctor` uses `look_path(&args[0])`. | `rust/src/application/diagnostics/supplemental.rs` about lines 129 to 131 | Located |
| C2 drops invalid custom provider entries into `ConfigProblem` with `text_en()` | `rust/src/config/safety.rs` `analyze_custom_providers` (about line 662), `analyze_raw_custom_providers` (about 770) | Located |
| Saving: `ConfigStore::persist` → `write_config`. `write_config` backs up only when the rewrite would lose content or the file is unknown (`ConfigBackups::create_in`, then `removed::record_replaced`). There is no single "back up, then write" helper for a CLI command. | `rust/src/config/model.rs` about lines 3911 to 4012, `rust/src/config/backup.rs` | Located |
| The rewrite drops comments and layout: `to_private_yaml` re-serializes a value | `rust/src/config/model.rs` about line 2821 | Located |
| `provider` today: `backup`, `reset`, `recover` on `ProviderRegistryStore`. No `add`, `list`, `remove`. | `rust/src/application/maintenance_cli/provider.rs` line 20 onward | Read |
| `provider` is not in `WORKS_WITHOUT_READABLE_CONFIG`, so it refuses while `config.yaml` is unreadable | `rust/src/cli.rs` | Read |
| Approval patterns: `<id>.json` is a JSON array of strings. `read`, `write_custom`, `copy_official` refuse ids outside `PROVIDERS` (7 built-ins plus `common`). `reload` and `remote_batch` handle a raw `<id>.json`. `remote_sync` takes custom sources from entries with a non-empty `approval_pattern_source`. | `rust/src/application/approval_patterns.rs` | Located |
| Remote pattern sources must be https on `raw.githubusercontent.com`. Whether local paths are accepted is unknown (a diagnostic message mentions them; `SlashIo::read` is not traced). | `rust/src/application/slash_commands/sources.rs`, `rust/src/application/diagnostics/supplemental.rs` about line 170 | Located, not traced |
| Child-AI text lists exactly 7 provider ids | `rust/src/wrapper/delegation_prompt.txt` lines 11 to 13 | Read |
| C2 appends a config-problem line to the child-AI text | `rust/src/wrapper/hooks.rs` `config_problems_notice`, `delegation_with_notice` | Located |
| `orchestrate spawn` forwards `--provider` without a local check. The Hub refuses ids outside `ORCHESTRATION_PROVIDERS` with 400. | `rust/src/application/orchestrate_cli.rs`, `rust/src/hub/child_routes.rs` about 175 to 192, `rust/src/orchestration/child_launch.rs` about 479 to 495, `rust/src/orchestration/child_options.rs` lines 13 to 21 | Located |
| Relay and orchestration role checks | `rust/src/application/relay_program.rs` about 611 and 1390, `rust/src/application/orchestration_program.rs` about 425, 490, 867 | Located |
| Ordinary spawn accepts any enabled registry definition, so custom ids already start from the Hub | `rust/src/application/ordinary_spawn.rs` about 190 to 210 | Located |
| The Hub spawn selector adds custom providers from `/api/info` | `web/src/app/spawn-panel.ts` about 741 to 800 | Located |
| Other hard-coded 7-id lists (check each one when you widen a rule; change only those your stage needs) | `rust/src/cli.rs` `BUILTIN_PROVIDERS`, `rust/src/config/model.rs` `BUILTIN_PROVIDER_IDS`, `rust/src/application/diagnostics/report.rs`, `rust/src/application/slash_commands.rs`, `rust/src/application/link_defaults.rs`, `rust/src/application/main_program/model_catalog.rs`, `rust/src/application/instruction_rules.rs`, `web/src/app/orchestration-roles.ts` | Located |
| `windows-sys` is only a Windows target dependency; Windows modules are gated with `cfg(windows)`. The crate probably compiles on Linux. | `rust/Cargo.toml` lines 58 to 62, module gates | Located; not built on Linux |
| Rust Windows CI runs only on pushes to `dots/rust-recovery-resume-3` | `.github/workflows/rust-windows-check.yml` | Read |
| Secret scan and validate run on pull requests | `.github/workflows/secret-scan.yml`, `.github/workflows/validate.yml` | Read |

## Muse Code (the first real example)

Checked in the official documentation under `https://dev.meta.ai/docs/muse-code` on 2026-10-16.

| Fact | Status |
|---|---|
| Headless: `muse exec "<prompt>"`. `--json` gives JSONL. `--prompt-file`. `--max-model-steps`. | Documented |
| Model flag: `--model` (default `muse-spark-1.2`) | Documented |
| Approval prompt options: `Allow once`, `Always allow in this workspace`, `Reject`. Modes: `--approval-mode on-request|untrusted|never`, `--disable-approval`, `--yolo`. | Documented |
| Settings file: `~/.config/muse/settings.json`. An environment variable that moves this folder is not documented. Where session history is stored is not documented. | Documented / not documented |
| Instruction files: the first found of `AGENTS.md`, `CLAUDE.md`, `.agents/AGENTS.md`, `.claude/CLAUDE.md` | Documented |
| On Windows the installer puts `muse.cmd` on the user PATH, which starts a PowerShell launcher and then the binary. | Observed on the owner's machine, 2026-10-16 (version 1.4.9) |
| Sign-in has not been done on the owner's machine, and is on hold. | Owner |

## Not in scope, recorded so nobody repeats the search

- Trial mode (`rust/src/application/main_program/context.rs`, `isolate_trial_environment`) points `HOME` and `USERPROFILE` at a trial home and redirects several built-in providers' config folders (`CODEX_HOME`, `CLAUDE_CONFIG_DIR` and others). A custom provider such as muse then probably reads its settings from the trial home (inferred, not run). That file is being edited by another session, so this request does not change it. The owner's side handles it.
