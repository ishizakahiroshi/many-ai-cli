---
type: architecture-component
title: Light Orchestration and the Relay Loop
description: How a conductor session spawns child AI sessions with a shared board.md, and how the Hub-driven relay state machine runs a plan through implementation, adversarial review, and fix without an AI conductor in the loop.
tags: [orchestration, relay, board, worktree, spawn-child, handoff]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-a8910515ddd14810ad43f5c1
    resource: repo://internal/config/config.go
  - id: openwiki-source-9f40d7b3cc897ef0c31212d0
    resource: repo://internal/handoff/handoff.go
  - id: openwiki-source-d7113d13301b3026bd8f2e4a
    resource: repo://internal/hub/relay_worktree.go
  - id: openwiki-source-3b26bb9dc20efa09a547cb22
    resource: repo://internal/hub/relay.go
  - id: openwiki-source-038c205276d1ecde82cd4593
    resource: repo://internal/orchestrate/orchestrate.go
  - id: openwiki-source-23775c3de52f3ab95a13cb8b
    resource: repo://README.md
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## Light orchestration: spawn-child and the shared board

`POST /api/sessions/:id/spawn-child` lets a conductor session create a child session with a role, provider, model, initial prompt, and optional cwd. The Hub creates `~/.many-ai-cli/orchestration/<orchestration_id>/board.md`, injects the board's path into the child's prompt, and polls the board for appended progress lines and a `## DONE <role> session=<child_id>` completion marker. By default each child runs in its own git worktree under `.many-ai-cli/worktrees/<orchestration_id>/<role>` when the parent cwd is a git repository; the Hub never auto-merges a child's branch — the conductor or user decides what to merge after reviewing the board and branch. Child sessions default to full permission bypass for unattended work (`orchestration.child_full_bypass`, default `true`): a Codex child starts with `--sandbox danger-full-access --ask-for-approval never`, and the others start in their own CLI's bypass-permissions equivalent; a conductor's own spawn action still waits for human confirmation by default (`orchestration.spawn_confirm_mode`), though relay children skip that confirmation by design (see below).

Notifications from an ordinary (non-relay) child are split into two kinds so a conductor is not flooded: **progress** (a human-facing "the shared board or a child's file updated" signal, delivered only to the conductor per `orchestration.board_notify_mode` — `soft-notify`/badge-only by default, or `queue-until-idle`/`interrupt` if explicitly configured) and **event** (information that should change the conductor's next action: completion via `DONE`/`SUCCESS`/session end, startup failure, a `## QUESTION <role> session=<id>` marker, timeout/idle, or a delayed spawn-result confirmation after an HTTP disconnect). Events land in a per-board FIFO capped at 256 entries regardless of notify mode, and are deliberately withheld while the conductor is mid-input, awaiting its own approval, running a workflow, or otherwise not ready to act on one.

## The `many-ai-cli orchestrate` CLI surface

`internal/orchestrate` implements the `orchestrate spawn`/`orchestrate send`/`orchestrate relay` subcommand family — its package doc states its purpose plainly: letting the AI running inside a conductor session spawn or message a child *without ever handling curl or the Hub token directly*. Authentication and resolving "which session am I" are both closed inside the command via environment variables the wrapper already injects into a conductor/child process (`MANY_AI_CLI_HUB_PORT`, `MANY_AI_CLI_SESSION_ID`, `MANY_AI_CLI_HUB_TOKEN`); the AI itself only ever sees `role`/`prompt` and optionally `provider`/`model` as command-line input, never a credential. A child's own answer to a `## QUESTION` marker is likewise sent back with `many-ai-cli orchestrate send --role <role> "<answer>"` rather than a raw API call. Client-side spawn wait defaults to 5 minutes (`MANY_AI_CLI_SPAWN_TIMEOUT` env override); a spawn confirmation that outlives that client-side timeout while a human is still deciding in the browser is not discarded on the Hub side — the CLI call errors out (and refuses to be retried) but the eventual approved spawn still reaches the conductor as a delayed event once it starts.

## The relay loop: a state machine, not an AI conductor

`relay.go`'s own header comment states the goal directly: drive plan → implementation → adversarial review → fix → next C **without an AI conductor** deciding any step. The Hub advances the state machine from exactly two signals: the *count* of `## DONE <role>` lines in a child's progress file (a count rather than mere presence, so the same long-lived child can be reused to complete several plan Cs in sequence) and the reviewer's `verdict:` line. Every side-effecting action — spawning, PTY injection, board bookkeeping, git operations, persistence — is routed through an injectable `relayDeps` interface specifically so the entire state machine can be exercised in unit tests without a real process. States:

| State | Entered from | Hub action | Exits to |
|---|---|---|---|
| `implementing` | relay start, or after a fix / advancing to the next C | starts the implementation role (or `implementation-strong`) and waits for that C's DONE | `reviewing` on DONE; `stopped` on timeout/exit |
| `reviewing` | implementation completed a C | reads `review-c<k>-r<r>.md` and records the verdict | next C or `completed` on pass; `fixing` on a `must`-level finding; `stopped` on `blocked`/a missing verdict |
| `fixing` | review left a `must` finding | sends a fix instruction to the same C's implementer; refuses once the round limit is exceeded | `reviewing` on DONE; `stopped` on timeout/exit/round-limit |
| `completed` | every plan C passed | retains the branch and result, sends a completion notification | terminal |
| `stopped` | user stop, round/timeout limit, child exit, missing verdict/review file, `blocked`, or a Hub restart | retains the stop reason and last review | `resume` → `implementing` for a resumable reason (Hub restart, child exit, timeout); otherwise terminal |

**Escalation to a stronger model** is optional and one-directional per C: the cheap `implementation` role handles every C by default; if a C fails review a configured number of times in a row, or the plan explicitly marks that C `[strong]` and the cheap implementer itself reports `escalate=true`, the Hub hands that one C to an `implementation-strong` role (if a child slot is available) — the Hub never judges difficulty on its own, only the review result and the plan's own annotation trigger escalation — and the *next* C always starts back on the cheap role.

Every text the Hub injects into a relay child, or sends up to the parent, is prefixed with `[relay <short id> <plan basename>]` specifically so a user or conductor running several relays at once never confuses which relay a message belongs to.

## Persistence and resumability

`relay.json` under the same `orchestration/<id>/` directory is rewritten on every state transition and is the relay's source of truth for surviving a Hub restart: state, round counters, completed Cs, child session IDs, roles, branch, worktree path, the last verdict, and a timeline. On Hub startup, any non-terminal relay is read back; if its wrapper children reconnect, the relay continues from the same state without restarting the plan. A relay that cannot reconnect is left `stopped`, and only the resumable stop reasons (`hub_restart`, `child_exited`, `timeout`) can be restarted via `relay-resume` — a relay whose worktree has been deleted is never resumed.

## Worktree isolation: one shared tree per relay, not per role

Unlike ordinary light orchestration (one worktree per child role), a relay's implementation and review children share a **single** worktree by default — `relay_worktree.go`'s header comment: `<parent cwd>/<worktree_dir_root>/<orchestration id>/relay` on branch `many-ai-cli/relay/<orchestration id>`, forked from the parent's `HEAD` at relay start. The implementer commits each C directly to that relay branch; the user's own checkout is never touched, and the Hub never auto-merges the relay branch — merging is left to the user once they have reviewed it. The worktree is never removed automatically (the branch is the deliverable), so cleanup is either a deliberate user action from the dashboard or something `many-ai-cli doctor`'s residue check (`internal/doctor/residue.go`) flags, both funneling through the same `cleanupRelayWorktree` git wrapper. `--same-tree` is an explicit escape hatch a user must opt into: relay children edit the user's actual working tree directly, so no other AI or the user may edit that tree concurrently while such a relay runs.

## Entry points and limits

A relay starts from one of two equivalent entry points — the CLI (`many-ai-cli orchestrate relay --plan <path>`, requiring `--impl provider[/model]` / `--review provider[/model]` only when no default role mapping is configured) or the Hub UI's relay dialog on a conductor session card or the orchestration dashboard — both calling the same token-authenticated API. `orchestration.max_children_per_parent` (default 4) bounds how many children one parent session may have running at once; an ordinary relay uses 2 slots (implementation + review), so the default comfortably fits two concurrent relays, but a relay using strong-model escalation can transiently need a third slot, so the limit should be raised when running several relays that may escalate concurrently.

## A related but distinct mechanism: session handoff records

`internal/handoff` is not part of the orchestration board/relay machinery, but solves an adjacent problem: letting a *different* AI CLI session safely pick up context after this one stops, without leaking anything sensitive into that handoff. Its package doc states the core design rule plainly: what is allowed into a `Record` is decided by the Go type itself, not by scanning free text for secrets afterward — the struct has fields only for a commit hash/subject, changed file *paths* (never contents or diffs), session metadata, and exactly one free-text field; secret-masking (`sessionlog.MaskSecrets`) is applied only to that one free-text field, since it is the only field an AI could put arbitrary content into. Widening the `Record` type is treated as a deliberate design decision rather than a routine field addition — a test-enforced allowlist in `handoff_test.go` fails on purpose if a new field is added without also being added there.
