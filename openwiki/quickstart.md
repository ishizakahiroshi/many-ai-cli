---
type: index
title: Quickstart
description: What many-ai-cli is and a task-routing map to the rest of this wiki, organized by what you're trying to do.
tags: [quickstart, index, routing]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-7bd911fdd3026b7b031a01e3
    resource: repo://go.mod
  - id: openwiki-source-be0f5ef7e317c7f0a92cd953
    resource: repo://internal/config/custom_provider.go
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## What this is

`many-ai-cli` (Go module `many-ai-cli`) is a single Go binary that wraps six AI coding CLIs (Claude Code, Codex CLI, GitHub Copilot CLI, Cursor Agent CLI, Grok Build CLI, opencode) in a PTY, watches them for approval prompts / completion / errors, and exposes a browser-based Hub UI (TypeScript + xterm.js) for approving, monitoring, and lightly orchestrating multiple sessions at once — locally, or over an SSH-tunneled/Docker-hosted remote connection. Start with [System Architecture Overview](/openwiki/architecture/overview.md) if you are new to the codebase; it explains the three-process model (wrapped session, Hub daemon, browser UI) everything else assumes.

## Route by task

**Understanding how the pieces fit together**
- [System Architecture Overview](/openwiki/architecture/overview.md) — the three-process model and the single `/ws` endpoint they all share
- [CLI Entrypoints and Subcommands](/openwiki/architecture/cli-entrypoints.md) — every `many-ai-cli` subcommand and its backing package
- [Configuration and On-Disk Layout](/openwiki/architecture/configuration.md) — `config.yaml` load/save/atomicity and `~/.many-ai-cli/`

**Working on session/PTY handling**
- [PTY Wrapper and Session Lifecycle](/openwiki/wrapper/session-lifecycle.md) — the OS-specific PTY backends, idempotent input delivery, and the reconnect-grace state machine
- [Hub HTTP/WebSocket Server and Session Registry](/openwiki/hub/server-and-sessions.md) — the session record, the `proto.Message` wire format, the HTTP API surface
- [Session Logging, Transcripts, and Attach](/openwiki/hub/session-logging-and-attach.md) — raw PTY logs, secret masking, clean transcripts, the SQLite history store, pasted/dropped attachments

**Working on approval detection**
- [Approval Detection and Marker System](/openwiki/hub/approval-detection.md) — transcript vs. VT-mirror marker sources, the `candidateKey`+`sourceEpoch` identity rule, pattern profiles, auto-approval
- [Approval UI and Marker Filtering](/openwiki/frontend/approval-ui.md) — the browser-side action bar, batch approvals, and the marker-tag-stripping filter

**Working on light orchestration or the relay loop**
- [Light Orchestration and the Relay Loop](/openwiki/hub/orchestration-relay.md) — `spawn-child`/board.md, the `orchestrate` CLI surface, and the Hub-driven relay state machine

**Working on the web frontend**
- [Web Frontend Architecture](/openwiki/frontend/architecture.md) — the unbundled esbuild pipeline, `app.ts`/`state.ts`/`terminal.ts`, runtime i18n
- [Session Views: Multi-Pane, Detached Grid, and Mobile](/openwiki/frontend/session-views.md) — the sidebar's one-tree placement rule, multi-pane, pop-out windows, mobile-specific views

**Working on multi-account / usage**
- [Multiple Subscriptions Per Provider](/openwiki/hub/subscriptions.md) — per-profile config directories, credential exclusion, profile seeding, remaining-quota reporting

**Working on security or remote access**
- [Security, Auth, and Remote Access Protection](/openwiki/hub/security-remote-access.md) — token auth, the revoke-all kill switch, the optional PIN, Tailscale `serve`, new-device notifications
- [Unified Launcher and Remote Connectivity](/openwiki/launcher/remote-connectivity.md) — WSL/SSH launcher profiles and the SmartScreen-avoiding terminal-launched alternatives

**Working on Files/Git, notifications, or voice**
- [Files and Git Tabs](/openwiki/hub/files-and-git.md) — read-scope trust model, no-clobber renames, secure permissions, `git` argv invocation
- [Notifications and Done Summaries](/openwiki/hub/notifications.md) — always-on in-UI visibility vs. opt-in Web Push / ntfy / webhook
- [Voice Input and Managed Whisper](/openwiki/hub/voice-input.md) — browser vs. local-Whisper dictation, false-trigger suppression, the managed Whisper binary lifecycle

**Extending or deploying**
- [Custom Providers](/openwiki/extensibility/custom-providers.md) — registering an arbitrary CLI via `custom_providers:` and its shell-free command parser
- [Docker and Remote Server Deployment](/openwiki/deployment/docker-remote-server.md) — the per-user GHCR container, the loopback socat relay, entrypoint lifecycle
- [Build, Packaging, and Release Pipeline](/openwiki/deployment/build-and-release.md) — `make` targets, git-tag-as-version-source, the tag-driven release workflow

**Working on tests or CI**
- [Audit Tests and Static Quality Gates](/openwiki/testing/audit-and-quality-gates.md) — the `audit_*_test.go` convention, the `instrumentation.json` ledger, and the CI static-check jobs
