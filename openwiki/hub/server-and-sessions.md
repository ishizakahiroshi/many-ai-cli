---
type: architecture-component
title: Hub HTTP/WebSocket Server and Session Registry
description: The Hub's session record and three-axis activity model, the single flat proto.Message struct used for both WebSocket directions, the categorized HTTP API surface, and the allowlisted bug-report collector.
tags: [hub, session, proto, websocket, api, spawn, bug-report]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-570ddc1a25c4b21237799d9d
    resource: repo://internal/hub/server.go
  - id: openwiki-source-8c13dbb7feea50d7b751391c
    resource: repo://internal/hub/session_activity.go
  - id: openwiki-source-a2c2a2fd20e3204e35392d28
    resource: repo://internal/proto/messages.go
  - id: openwiki-source-37f4cafbe1cb6dd258455038
    resource: repo://internal/report/collect.go
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## One flat message struct for both WebSocket directions

`internal/proto.Message` is a single struct used for every `/ws` message in both directions — UI-to-Hub and wrapper-to-Hub — disambiguated at runtime by its `Type`/`Role` fields rather than by separate typed message structs per direction (see [System Architecture Overview](/openwiki/architecture/overview.md) for how `Role`/`Type` route an incoming connection). Most fields are `omitempty`, so a given message instance only actually carries the handful of fields relevant to its `Type`.

One field pair is worth calling out because of what it replaced: `Message` carries four boolean flags — `OutputIdle`, `WorkflowActive`, `AwaitingUser`, `AwaitingApproval` — plus an `Activity *SessionActivity` pointer that carries all four atomically (including a *false* transition, which the individual flat booleans cannot distinguish from "unset/omitted" once `omitempty` is in play). The comment on the struct is explicit that `State` (the older single-string label) remains only for compatibility display, and any consumer that needs a safe point to interject — e.g., is this session idle enough to auto-switch the visible approval to it — should compute `output_idle && !workflow_active` from the newer fields instead. `SessionActivity.IsIdle()` implements exactly that predicate, and its own comment explains why `AwaitingUser` is deliberately excluded from it: a prompt can safely be surfaced while output is quiet, but active workflow work must never be interrupted, so those are two different questions that a single "idle" boolean cannot answer at once.

## The session registry

The Hub's in-memory `session` struct (`server.go`) is the authoritative per-session record the registry (`s.sessions map[int]*session`, guarded by `sessionsMu`) holds. Beyond identity (`ID`, `Provider`, `CWD`, `Branch`) and display metadata (`Label`, `Color`, `Note`, `Pinned`), it carries: `ProjectID` (the root of the actual repository `CWD` belongs to — the sidebar tree groups by this, not by raw cwd, see [Session Views](/openwiki/frontend/session-views.md)); orchestration linkage (`ParentSessionID`, `Role`, `OrchestrationID`, `BoardPath`, `Depth`); worktree bookkeeping (`WorktreeBranch`, `NormalWorktree`, `WorktreeCleanup`); and `Relays []*proto.RelayStatus`, the relay loops this session conducts as a parent (see [Light Orchestration and the Relay Loop](/openwiki/hub/orchestration-relay.md)). `Relays` and `CrossSessionMessages` are both commented as "replace the slice, never mutate in place" specifically so a session snapshot copied out from under `sessionsMu` (for a WS broadcast or an HTTP JSON response) stays race-free while it is being marshalled concurrently with a later mutation.

## The HTTP API surface

Beyond `/ws`, the Hub's `net/http` mux serves a large `/api/*` surface — the design doc counted roughly 93 endpoints as of v0.3.0, excluding static assets — organized into categories: session management (`/api/spawn`, `/api/kill-all`, `/api/shutdown`, `/api/idle-timeout`, `/api/session-usage`, …), session history (`/api/session-chat`, `/api/session-log`, `/api/session-search`), approval (`/api/approval/status|enable|disable|dismiss`, pattern endpoints), Files, Git, model/slash-command sources, settings/notifications, user preferences, Web Push, voice/Whisper, path/open handlers, log/attachment maintenance, and orchestration (`/api/sessions/:id/spawn-child|spawn-confirm|send-child|inject|children|relay|relay-stop|relay-resume|relay-cleanup`, `/api/orchestration-config`). An older `/api/docs-*` family (a prior "docs browser" feature) has been fully migrated to the `/api/files-*` endpoints, including a one-shot migration of anything the older feature had stored in browser storage.

## Spawning a new session

`handleSpawn` (`spawn_handler.go`) is the entry point behind the Hub UI's "+ New Session" panel and the `POST /api/spawn` API — the same endpoint whether a human clicks the button or a conductor session's `many-ai-cli orchestrate spawn` reaches it indirectly through the spawn-child path (see [Light Orchestration and the Relay Loop](/openwiki/hub/orchestration-relay.md)). A related `handleSpawnGrid` exists for spawning directly into a multi-pane grid layout rather than as a single new session card.

## Diagnostic reports: allowlisted, not a dump

`internal/report` backs the `issue` subcommand (see [CLI Entrypoints and Subcommands](/openwiki/architecture/cli-entrypoints.md)) and its `CollectOptions` type is explicitly allowlisted rather than a generic "attach everything" bag — its own doc comment states plainly that `CollectOptions` "contains only values approved for inclusion in a bug report" and that "callers must not pass serialized configuration or session logs here." Only `Version`, `Provider`, `Model`, `UserAgent`, and a `*config.Config` (from which only a further allowlisted `AllowedConfig map[string]string` subset is surfaced into the report's `Environment`) are representable at all — the same allowlist-by-type discipline used by [`internal/handoff`](/openwiki/hub/orchestration-relay.md) for session handoff records.
