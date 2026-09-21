---
type: architecture-component
title: Multiple Subscriptions Per Provider
description: How many-ai-cli isolates several logins for one provider CLI purely through per-profile config-directory environment variables, never touching credentials itself, plus how remaining-quota usage figures are read.
tags: [subscriptions, credentials, usage-relay, environment-variable, seed]
sources:
  - id: openwiki-source-2cb9bd598df77afe2b3ea92c
    resource: repo://internal/doctor/subscriptions.go
  - id: openwiki-source-1886c841f204522a4918c6bd
    resource: repo://internal/subscription/adapter.go
  - id: openwiki-source-7c7a84ff4b04a95328f7fcaf
    resource: repo://internal/subscription/claude.go
  - id: openwiki-source-c861326e412ade6e53a3f566
    resource: repo://internal/subscription/codex.go
  - id: openwiki-source-080bff77739560626a95c9d2
    resource: repo://internal/subscription/grok.go
  - id: openwiki-source-adffd48498d7f55f078c06af
    resource: repo://internal/subscription/seed.go
  - id: openwiki-source-967dc3cc3b95466f6539c1df
    resource: repo://internal/usagerelay/usagerelay.go
generated: { by: "claude-code", at: "2026-09-21T12:35:03.565Z" }
verified:
  - by: openwiki/0.5.0
    at: 2026-09-21T12:35:03.565Z
---

## One fact, one mechanism

`internal/subscription`'s package doc states the entire design rests on a single confirmed fact: every supported vendor CLI selects its configuration directory from an environment variable, and pointing two sessions at two different directories keeps their logins independent. `many-ai-cli` therefore only ever creates a directory and sets that one environment variable — it "never reads, writes, parses, or stores the credential itself." Four rules, each pinned by a named test where possible, keep this from drifting: no code may read/write/parse an auth file (sign-in state comes only from the vendor CLI's own status subcommand, since file formats change across CLI releases); no token/API-key/PAT may be kept in `config.yaml` or any store of the Hub's own — allowing that would turn the tool into a credential vault, which is exactly why GitHub Copilot CLI and Cursor Agent CLI are recorded as **not supported** for multiple profiles (their tokens live in the OS credential store / a fixed file with no relocating environment variable); a spawn with no profile selected must produce a byte-for-byte identical environment to an unmodified spawn (`TestSubscriptionLaunchWithoutProfileLeavesEnvUnchanged`); and a live session's credentials are never swapped mid-session (`TestLiveSessionAuthIsNeverSwapped` scans the source for such an assignment rather than relying on a runtime test alone). A spawn naming a missing or disabled profile fails outright rather than silently falling back to another account or the default login.

| Provider | Variable | Status |
|---|---|---|
| Claude Code | `CLAUDE_CONFIG_DIR` | supported |
| Codex CLI | `CODEX_HOME` | supported |
| Grok Build CLI | `GROK_HOME` | supported |
| opencode | `XDG_DATA_HOME` | supported (only its credential store moves — see below) |
| GitHub Copilot CLI | — | not supported: token lives in the OS credential store |
| Cursor Agent CLI | — | not supported: token lives in a fixed `~/.cursor/cli-config.json` with no relocating variable |

`XDG_DATA_HOME` is a generic XDG variable rather than an opencode-specific one, so any other XDG-aware tool the agent happens to run *inside that session* also writes under the profile directory; opencode has no dedicated environment variable of its own today, and this switches to one if it ever grows one.

## Seeding a new profile: additive, own-tree-only, named entries only

A fresh profile directory starts empty except for what `internal/subscription/seed.go` explicitly carries in — and that carrying-in exists at all only because of a measured incident (2026-08-23): pointing a vendor CLI at a fresh directory separates the login, which is the goal, but it also silently separates *everything else* that happens to live in that same directory (the user's `CLAUDE.md`/`AGENTS.md`, skills, slash commands, approval allowlist/policy, trusted folders) — with nothing reporting this, a freshly profiled session simply behaved as if the user had never configured the CLI at all, invisible from both sides, and it cost an afternoon of investigation before the cause was traced. Three rules bound seeding so it never turns into "many-ai-cli edits your CLI config":

- **Additive only, with one named exception** — an entry is carried in only when the profile does not already have it; nothing a profile holds is renamed or deleted, and a value the user changed inside a profile wins. The exception is the vendor settings file (`SeedSyncFile`, below): its *policy* keys are re-read from the default on every pass, while the keys the vendor CLI writes itself stay the profile's and a key only the profile has is never touched.
- **Inside our own tree only** — every write lands under `~/.many-ai-cli/subscriptions/`; the user's real `~/.claude`, `~/.codex`, `~/.grok` are read and never written, so `many-ai-cli uninstall` still removes everything a profile created.
- **Named entries only** — each provider's adapter lists exactly which entries to carry by name; there is no "copy the whole directory" mode, which would drag a credential file across right along with everything else and defeat the separation the whole mechanism exists to provide.

Several `SeedKind`s implement this, chosen per entry based on how the vendor CLI treats that specific file. **`SeedCopyFile`** copies a file once, as-is, for files the vendor CLI rewrites (e.g. Grok's `trusted_folders.toml`) — a symlink there would push the profile's own edits back into the user's default configuration, which seeding must not do. **`SeedSyncFile`** is used for Claude's `settings.json` and Codex's/Grok's `config.toml`: rather than a one-time copy (which meant a switch the user later added to the default never reached a profile, with nothing on screen to say so), every pass takes the policy keys — hooks, permissions, feature switches — from the default, while the `StateKeys` the vendor CLI writes itself (chosen model, theme, and similar) stay the profile's; `many-ai-cli doctor` reports a profile whose file has drifted from the default. **`SeedJSONKeys`** writes only named top-level keys of Claude's `.claude.json`, which mixes account identity with a few real preferences. **`SeedLinkDir`** links a whole directory (symlink, or a junction on Windows where a plain symlink needs a privilege) for content the user maintains in exactly one place and expects every profile to see immediately — skills, slash commands, prompts — so adding a skill later reaches every profile without a re-seed. Rule files (`CLAUDE.md`/`AGENTS.md`) get a special-cased exception to the copy-vs-link split: the vendor CLI does not rewrite them, so if the user's own default copy is *itself* already a symlink, a profile mirrors that as a symlink to the same resolved target rather than taking a static snapshot — editing the original then reaches every profile with no re-seed needed, while a plain (non-symlinked) default file is still copied normally.

## Remaining quota: three separate local data paths, not a query API

The package doc is explicit that remaining usage/quota **cannot be queried** from any of the vendor CLIs (measured 2026-08-17) — there is deliberately no `ReadUsage` method on the provider `Adapter` interface. That is a distinct fact from whether usage data exists at all: three separate local data paths do exist and the Hub reads them, all as local files rather than API calls — **Claude** pushes rate-limit figures via its own `statusLine` hook, **Codex** leaves rate-limit data inside its own rollout JSONL log, and **Grok** leaves billing data in a `unified.jsonl` file (parsed by `internal/usagelocal`, e.g. its `codexRateLimits`/`codexRolloutEvent` types for the Codex path). A provider with none of these three paths (Copilot, Cursor Agent, opencode) is deliberately left out of the Usage panel's quota display entirely, rather than being shown with a synthetic "Unknown" placeholder — the package doc calls out that reading its opening sentence as "no usage data exists anywhere" is a real misreading risk, since it would lead someone to (incorrectly) document a shipped feature as missing.

## `usage-relay`: getting Claude's/Codex's own hook data to the Hub

`internal/usagerelay` implements the hidden `many-ai-cli usage-relay` subcommand (see [CLI Entrypoints and Subcommands](/openwiki/architecture/cli-entrypoints.md)) invoked by Claude's `statusLine` hook and Codex's `Stop` hook — reading JSON from stdin and POSTing extracted numeric metadata to the Hub's `/api/session-usage`. Its package doc states the security requirement as a hard C1 completion condition: only numeric metadata (token counts, cost, model name, elapsed time) is ever sent to the Hub — no prompt text, code, or tool input/output content leaves the process. For Codex's rollout JSONL specifically, only the numeric fields of `token_count` events are extracted; conversation-body lines are read and discarded without ever being held in memory. If the HTTP POST to the Hub fails, the relay logs a warning to stderr but still exits 0 — a network hiccup or a Hub that is not currently running must never break the vendor CLI's own hook or statusLine rendering.
