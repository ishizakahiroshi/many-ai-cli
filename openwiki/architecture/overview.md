---
type: architecture-overview
title: System Architecture Overview
description: The three-process model behind many-ai-cli — wrapped provider CLIs, the local Hub daemon, and the browser Web UI — and how they connect over one WebSocket endpoint.
tags: [architecture, hub, wrapper, websocket, pty, overview]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-570ddc1a25c4b21237799d9d
    resource: repo://internal/hub/server.go
  - id: openwiki-source-f8ec2ad8710460e82277e8d4
    resource: repo://internal/wrapper/wrapper.go
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## The three processes

`many-ai-cli` is one Go binary playing three different roles, all communicating over a single local WebSocket endpoint:

1. **A wrapped provider session** — `many-ai-cli wrap <provider>` (or the shorthand `many-ai-cli claude` / `codex` / `copilot` / `cursor-agent` / `opencode` / `grok` / `command-code`) starts the real AI CLI inside a PTY (`internal/wrapper`) and passes its input/output through unmodified, while also registering itself with the Hub over WebSocket. See [PTY Wrapper and Session Lifecycle](/openwiki/wrapper/session-lifecycle.md).
2. **The Hub** — `many-ai-cli serve` (`internal/hub`) is the always-on local daemon: an `net/http` server serving both the embedded Web UI's static assets and an HTTP+WebSocket API, holding the in-memory session registry, and running approval detection, orchestration, notifications, and every other server-side feature. See [Hub HTTP/WebSocket Server and Session Registry](/openwiki/hub/server-and-sessions.md).
3. **The browser Web UI** — a TypeScript/xterm.js single-page app served by the Hub, connecting back to the same `/ws` endpoint with a UI role to render terminal output, approval prompts, and session state. See [Web Frontend Architecture](/openwiki/frontend/architecture.md).

The wrapped CLI process is unaware of the Hub's existence beyond the wrapper: `many-ai-cli` does not modify or proxy the underlying `claude`/`codex`/etc. binary's own network calls, config files, or credentials — it only owns the PTY around it.

## Everything meets at `/ws`

The Hub exposes a single WebSocket route, `/ws`, handled by `Server.handleWS`. The very first JSON message received on a new connection (a `proto.Message`) determines what kind of peer it is:

- **`Role: "ui"`** — a browser tab. The Hub registers a `uiConn`, replays a snapshot of session state and buffered PTY output (respecting a resume/reattach position so a reload does not lose scrollback), and enters a per-connection read loop (`uiLoopAtEpoch`) for the rest of the connection's life.
- **`Type: "register"`** — a wrapped provider session announcing itself for the first time; the Hub creates a new session record and enters `wrapperLoop`.
- **`Type: "reattach"`** — a wrapper reconnecting to a session that already exists in the Hub's registry (e.g., after a network blip), handled by `reattachLoop` instead of creating a duplicate session.

Every `/ws` connection — UI or wrapper — is authenticated the same way before any role-specific logic runs: the connection must present a valid token (or qualify for the `hub.allow_loopback_without_token` loopback exception) and, if remote-PIN protection is enabled for non-loopback callers, a valid PIN cookie. See [Security, Auth, and Remote Access Protection](/openwiki/hub/security-remote-access.md).

## How a wrapped session finds its Hub

A `wrap`ped process does not assume a Hub is already running. `ensureHub(cfg, logger)` in `internal/wrapper` decides what to do before dialing `/ws`:

- If the process was itself spawned *by* a Hub (`MANY_AI_CLI=1` in its environment — the "light orchestration" child path, or any Hub-launched session), it skips probing and startup entirely and trusts that a Hub is already up, because starting a second one risks killing the real Hub via a stale PID file.
- Otherwise it probes the runtime ledger / configured port for a live Hub (see the "runtime ledger" mechanism in [CLI Entrypoints and Subcommands](/openwiki/architecture/cli-entrypoints.md)), and if none is reachable, finds a free port near the configured one and launches `many-ai-cli serve --port <port>` as a detached child, waiting (with a timeout) for it to become ready before proceeding.

This is why a user can just double-click a provider-specific desktop launcher, or type `many-ai-cli claude` cold, with no Hub running yet — the wrapper brings one up transparently on first use, and every subsequent wrap on the same machine reuses it.

## HTTP surface alongside the WebSocket

Beyond `/ws`, the Hub's `net/http.ServeMux` (built in `NewServer`) serves the embedded static Web UI (`go:embed`-baked `web/dist`), a large `/api/*` surface for spawning sessions, approvals, files, git, subscriptions, orchestration, and settings (see [Hub HTTP/WebSocket Server and Session Registry](/openwiki/hub/server-and-sessions.md) for the categorized list), and `/api/info` — the lightweight liveness/identity probe both `hubruntime.RunningPort` and the wrapper's `ensureHub` use to decide whether an existing Hub is really alive before trusting it.

## Binding and process boundaries

The Hub binds to `127.0.0.1` only — never `0.0.0.0` — so on a single machine no traffic leaves the loopback interface; remote access is achieved by tunneling into that loopback port (SSH local forward, or the unified launcher — see [Unified Launcher and Remote Connectivity](/openwiki/launcher/remote-connectivity.md)) rather than by the Hub itself listening on a public interface. Each wrapped provider session is a genuinely separate OS process holding its own PTY; the Hub's in-memory session registry tracks them by session ID and reconnects them across brief network interruptions using the reattach path, but a Hub restart does not resurrect the child processes themselves — those survive independently for a configurable grace period waiting for the Hub to come back (see [PTY Wrapper and Session Lifecycle](/openwiki/wrapper/session-lifecycle.md)).
