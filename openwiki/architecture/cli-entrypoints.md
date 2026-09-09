---
type: architecture-component
title: CLI Entrypoints and Subcommands
description: How the many-ai-cli binary dispatches its subcommands (serve, wrap, setup, doctor, tray, uninstall, and more), and what the supporting internal packages behind each one do.
tags: [cli, entrypoint, subcommands, setup, doctor, tray, uninstall, hubruntime]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-e8686ac829ccdb846b0aa13a
    resource: repo://cmd/many-ai-cli/main.go
  - id: openwiki-source-0046c2566046ebeb37401edb
    resource: repo://internal/doctor/doctor.go
  - id: openwiki-source-d3247796c73cc3e4c4c4a421
    resource: repo://internal/doctor/residue.go
  - id: openwiki-source-622d0b47281c620a9a87f705
    resource: repo://internal/hubruntime/runtime.go
  - id: openwiki-source-9a7f387de45d7e9375889c90
    resource: repo://internal/tray/tray.go
  - id: openwiki-source-e00913e4bcd242475f802bf7
    resource: repo://internal/uninstall/uninstall.go
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## Overview

`many-ai-cli` ships as a single Go binary (`cmd/many-ai-cli/main.go`) whose `main` function delegates everything to `run(args)`, a plain `switch` over `args[0]`. There is no subcommand framework — each case parses its own `flag.FlagSet` (when it takes flags) and calls into one `internal/*` package. Running the binary with **no arguments at all** is itself a case: it loads config, checks whether a Hub is already running, and either opens a browser tab against it or starts a new Hub with `SetAutoOpenBrowser(true)` — this is the path Windows desktop shortcuts and the tray use.

A top-level `defer recover()` in `main()` catches any panic that escapes `run` (including the blocking `serve` path), logs it with a stack trace via the file logger, and exits — so a crash always leaves a diagnostic trail in `hub.log` instead of vanishing silently.

## Version resolution

`displayVersion()` is the single path every surface uses to report a version (binary `version`, the Web UI, Windows executable metadata): it prefers the `-ldflags -X main.version=` value injected by the release build, and falls back to `git describe --tags --abbrev=0` run against the repository root (located by walking up from the executable or working directory looking for a `go.mod` with `module many-ai-cli`) when running from source (`version == "dev"`).

## Subcommand map

| Subcommand | Package | Purpose |
|---|---|---|
| *(none)* | `internal/hub`, `internal/config` | Start a Hub (or focus the running one) and auto-open the browser |
| `serve` | `internal/hub` | Start the Hub daemon; `--port`, `--open`, `--dev` (serve `./web/` uncompiled), `--debug` |
| `claude` / `codex` / `copilot` / `cursor-agent` / `opencode` / `grok` / `command-code`, and `wrap <provider>` | `internal/wrapper` | Wrap a provider CLI in a PTY as a Hub-managed session |
| `connect` | `internal/launcher` | Connect a terminal to a remote Hub using a saved launcher profile (`--profile` or `--last`) — the same connection flow as `many-ai-cli-launcher`, exposed as a subcommand so Windows users can avoid double-clicking a second `.exe` under SmartScreen |
| `profile-export` | `internal/launcher` | Print a connection profile (host, cwd, port — no keys) as JSON so a local machine's UI can pull it over SSH and auto-fill a connection form |
| `status` | `internal/hub` | Print whether a Hub is running |
| `stop` | `internal/hub` | Stop the running Hub |
| `tray` | `internal/tray` | Run the Windows-only tray-resident process |
| `doctor` | `internal/doctor` | Run local, non-mutating diagnostics; `--json` |
| `setup` | `internal/setupcmd` | Create OS-native double-click launchers (and, on Windows, the Startup-folder tray entry) |
| `uninstall` | `internal/uninstall` | Remove `~/.many-ai-cli`, autostart entries, and optionally (`--purge`) the binary itself |
| `log-clean` | `internal/sessionlog` | Regenerate a readable `.txt` transcript from a session's `.jsonl` log |
| `shell-init` | `internal/shell` | Print the opt-in shell integration script |
| `issue` | `cmd/many-ai-cli/issue.go`, `internal/report` | Interactively collect a redacted diagnostic report and open/attach it to a GitHub issue |
| `usage-relay` | `internal/usagerelay` | Hidden: invoked by Claude's statusLine hook / Codex's Stop hook to relay usage data |
| `orchestrate spawn\|send\|relay` | `internal/orchestrate` | Hidden: used by a conductor/child session's own AI to spawn or message orchestration children, or drive the relay loop, from inside a wrapped session |
| `version` / `--version` / `-v` | — | Print `displayVersion()` |
| `-h` / `--help` / `help` | — | Print the one-line usage summary |
| *(unrecognized)* | `internal/config` | If `cfg.IsCustomProviderID(cmd)` matches a configured `custom_providers:` id, dispatch to `wrapper.Run` exactly like a built-in provider id; otherwise `unknown command` |

`usage-relay` and `orchestrate` are deliberately left out of the printed `usage()` text — they are invocation targets for hooks and for an AI session's own tool calls, not something a human is expected to type.

## `internal/doctor`: local diagnostics

`doctor.Run(ctx, cfg)` performs bounded, local-only checks and returns a `Report` of `Check{Name, Level: OK|WARN|FAIL, Message, Fix}` entries — it never writes configuration or logs. The always-run checks cover provider binaries on `PATH`, the configured port, the auth token, ACLs, Ollama reachability, the managed Whisper install, Tailscale, and log/session-log health. Four checks are conditional and only appear when relevant, specifically to keep the report short when nothing needs attention:

- **Residue checks** (`residue.go`) — detect a leftover generated file (`opencode.json` written into a session's cwd, or the approval-rules block injected into `AGENTS.md`/`CLAUDE.md`) that a killed process never got to restore, and reclaim it on the next `doctor` run rather than leaving it to rot. The file's design-rule comment states the invariant this exists to enforce: any feature that rewrites a user's file for a session's duration and restores it on exit is broken by design if the restore only lives in a `defer` or graceful-shutdown path, because a hard kill skips both, and the next run must not treat the leftover as the original.
- **Command-code checks**
- **Subscription checks**
- **Custom-provider checks**

## `internal/hubruntime`: the runtime ledger

`internal/hubruntime` maintains `~/.many-ai-cli/hub-runtime.json` — a small, atomically-written record (`PID`, `Port`, `StartedAt`) of the last Hub that actually started, kept separate from the token. It exists because the *configured* port in `config.yaml` is not necessarily the port the running Hub actually bound to (the Hub can fall back to a nearby free port on collision). `RunningPort`/`RunningPortWith` resolve the real port with a double guard: try the configured port's `/api/info` first, and only fall back to the ledger's port after confirming the recorded PID is still alive (`pidAlive`) and re-probing `/api/info` on that port. `RemoveIfPID` only deletes the ledger if it still belongs to the given PID, so a late-exiting old Hub process can never clobber a newer Hub's record.

## `internal/setupcmd`: OS-native launchers

`setupcmd.Run()` is the implementation of `many-ai-cli setup`. It resolves the running executable's path, ensures the config directory exists, and creates OS-specific double-click launchers so a non-technical user never has to open a terminal: on Windows a single **"MANY-AI-CLI"** tray shortcut (desktop + Startup folder, replacing the older two-icon "Many AI Hub Start/Stop" pair from before tray consolidation in v0.7.0), and on macOS/Linux **"Many AI Hub Start"** / **"Many AI Hub Stop"** (`.command` / `.desktop`) launchers. Each result is reported as `[OK] created`, `[FAIL]`, or `[NOTE]` (a note-only result points at something already present, such as an old icon, rather than something newly created).

## `internal/tray`: Windows tray residency

`internal/tray` runs as its own process (`many-ai-cli tray`), not as part of `serve`, specifically so the tray icon outlives the Hub: if residency lived inside `serve`, stopping the Hub would also kill the tray, collapsing "stop Hub" and "quit tray" into the same action and making "start Hub from a stopped state" impossible from the tray. It opens no window — the UI is still the default browser tab — and emits no OS-level notifications, since the Web UI already signals approval-waiting state through title blinking, a favicon count badge, and a notification sound. A named mutex keeps the tray to a single instance; a second launch (e.g., autostart racing a manual double-click) just resolves and opens the existing Hub's URL instead of spawning a second tray icon. Tray support is Windows-only; `tray.Run` returns `tray.ErrUnsupported` on other platforms.

## `internal/uninstall`: removal

`uninstall.Run(purge bool)` stops a running Hub first, then removes the entire `~/.many-ai-cli` data directory with `os.RemoveAll`. Because `RemoveAll` does not follow symlinks, this is safe even though the data directory can contain symlinks/junctions into a subscription profile's rule files or linked `skills`/`commands`/`agents` directories — only the link itself is removed, never the target the link points at (`internal/uninstall/uninstall_test.go` locks this with `TestRemoveDataDirDoesNotFollowFileSymlink` / `TestRemoveDataDirDoesNotFollowDirectoryJunction`). Autostart registration is removed unconditionally (not gated on `--purge`) because, unlike a desktop icon, a lingering autostart entry runs on every login rather than sitting inert. `--purge` additionally removes the binary itself (`removeSelf`, OS-specific); without it, uninstall prints the executable's path for the user to delete manually.

## `issue`: diagnostic report → GitHub issue

The `issue` subcommand (`cmd/many-ai-cli/issue.go`) is not a thin wrapper package but lives directly in `cmd/many-ai-cli`; it builds an `issueDependencies` struct of injectable I/O, `exec.LookPath`, and command-running functions (so its flow is unit-testable without shelling out for real), collects a redacted diagnostic Markdown report via `internal/report.Collect`, and drives the user through titling and opening/attaching it as a GitHub issue against `ishizakahiroshi/many-ai-cli`.
