---
type: extension-mechanism
title: Custom Providers
description: How a power user registers an arbitrary AI CLI as a spawn option via custom_providers in config.yaml, the shell-free command-line parsing rules, and what built-in behavior a custom provider does and does not inherit.
tags: [custom-providers, extensibility, config, spawn, approval-detection]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-95add79c933891b0a33f428e
    resource: repo://internal/config/custom_provider_command.go
  - id: openwiki-source-be0f5ef7e317c7f0a92cd953
    resource: repo://internal/config/custom_provider.go
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## Registration is hand-editing `config.yaml`, nothing else

`custom_providers:` is a list on the same `Config` struct as every other setting (see [Configuration and On-Disk Layout](/openwiki/architecture/configuration.md)), and hand-editing `config.yaml` is the *only* way to add, change, or remove an entry — there is no "Add provider" button anywhere in the Hub UI. `CustomProvider.go`'s own doc comment states this is deliberate: the Hub UI only displays a configured custom provider and lets a session spawn with it, never writes one, and the built-in-provider policy decisions many-ai-cli makes for its shipped list (why Gemini CLI is out of scope, etc.) explicitly do not apply to whatever a user points a custom `command:` at — that choice, and its terms-of-service implications, are the user's own.

A `CustomProvider` entry has four fields: `id` (the value a session spawns with), `label` (optional spawn-dropdown display text, falling back to `id` via `EffectiveLabel()` when empty), `command` (the command line many-ai-cli runs), and `approval_pattern_source` (optional — a local path or URL to that CLI's approval-trigger phrases).

## Tolerant parsing, validated later

`CustomProviders.UnmarshalYAML` decodes the list tolerantly: an entry missing `id` or `command` is silently dropped at parse time, and a YAML node that fails to decode into a `CustomProvider` at all is skipped rather than failing the whole file — because `config.yaml` is hand-written for this section, and `LoadOrCreate` treats a fully unparseable file as corruption (backing it up and regenerating a fresh token, which would change the Hub's URL). Built-in-id collisions and duplicate-id detection happen later, in `EffectiveCustomProviders`/`Warnings()`, specifically so `cfg.CustomProviders` still reflects exactly what the user typed and `Save()` round-trips it rather than silently deleting a mistyped entry — a bad entry is *reported* as a warning and excluded from the effective spawn list, not erased from the file.

`ValidateCustomProviderID` rejects an id that is empty, longer than 64 characters, contains characters other than lowercase letters/digits/dot/underscore/hyphen, or collides with `IsReservedProviderID` — which is the six built-in provider ids (`claude`, `codex`, `copilot`, `cursor-agent`, `opencode`, `grok`, plus `command-code`) *and* the extra reserved id `"shell"`. `"shell"` is reserved even though it is not a built-in provider, specifically because the wrapper's dispatch has a dedicated `"shell"` branch that opens an interactive shell and would silently swallow a `CustomProvider.Command` if a custom entry were allowed to claim that id.

## Command-line parsing: no shell involved

`internal/config.SplitCommandLine` is the single source of truth for turning a `command:` string into an argv slice, used identically by `internal/wrapper` (actual process launch) and `internal/doctor` (the PATH-existence probe) — its own doc comment states the README's "Custom providers" section documents the same rules and both must be updated together if the rules ever change. The rules, deliberately small and fixed because the string never reaches a real shell:

1. ASCII space/tab delimit tokens; runs of delimiters collapse to one, and leading/trailing delimiters are discarded.
2. A double-quoted span is one token or part of one — quoting can start and end mid-token (`--path="C:\a b\c"` becomes `--path=C:\a b\c`); the quotes themselves are removed from the result.
3. `""` inside a quoted span decodes to one literal `"` character (CSV-style escaping).
4. `\` is always a literal character, never an escape — so Windows paths like `C:\a\b.exe` need no special handling.
5. `'` has no special meaning; it is an ordinary character.
6. Nothing else is expanded or interpreted: environment variables (`$X`, `%X%`), `~`, globs, and shell operators (`|`, `&&`, `;`, `>`, `<`) all pass through as literal argv text.
7. An unterminated quote, a control character outside the space/tab delimiters, or zero resulting tokens are all rejected as parse errors.

## What a custom provider does not get

Per the README's "Custom providers" section, nothing built-in is attached automatically: no `--model`, no permission-mode/sandbox/ask-for-approval flag translation, no `ANTHROPIC_*`/`OPENAI_*` environment presets, no Ollama/LM Studio routing, and no subscription-profile selection — none of those have a defined meaning for an arbitrary CLI, so the spawn form hides the model field for a custom provider and the Hub never injects any of it; only `command`'s own arguments plus the common `MANY_AI_CLI*` session environment reach the process. The built-in approval-rules-injection hook that writes an approval-rules block into `CLAUDE.md`/`AGENTS.md` is likewise built-in-only and never applied to a custom provider.

## Approval detection for a custom provider

Every custom session automatically gets the same generic text heuristic (approval-shaped wording and option labels) that runs server-side for built-in providers. On top of that, an `approval_pattern_source` — constrained to either an absolute local path under `~/.many-ai-cli/` or an `https://raw.githubusercontent.com/...` URL — lets the Hub fetch or read that CLI's own trigger phrases once at startup into `~/.many-ai-cli/approval-patterns/<id>.json`, loaded by the browser the same way it loads the built-in providers' pattern files. See [Approval Detection and Marker System](/openwiki/hub/approval-detection.md) for how that detection pipeline works in general.

## Failure mode for a missing binary

If the executable `command` resolves to is not found on `PATH`, a spawned custom-provider session ends the same way a missing built-in CLI would (an `... not found in PATH` message); `many-ai-cli doctor` separately checks `PATH` for every configured custom provider's executable without ever actually running it, using the same `SplitCommandLine`-derived argv[0].
