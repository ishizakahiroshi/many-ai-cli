---
type: architecture-component
title: PTY Wrapper and Session Lifecycle
description: How internal/wrapper starts a provider CLI in a real OS PTY, the idempotent input-sequence protocol that prevents a resend from double-submitting a prompt, and the reconnect-grace state machine that keeps a session alive through a Hub crash or restart.
tags: [wrapper, pty, conpty, reconnect-grace, zombie-protection, input-sequence]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-44e824033e031f67a11c4a91
    resource: repo://internal/wrapper/pty_unix.go
  - id: openwiki-source-981256757052c28fc4ddc4fa
    resource: repo://internal/wrapper/pty_windows.go
  - id: openwiki-source-f8ec2ad8710460e82277e8d4
    resource: repo://internal/wrapper/wrapper.go
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## Two PTY backends, one interface

`internal/wrapper` starts the actual provider CLI process inside a real OS pseudo-terminal so the CLI itself behaves exactly as it would in a normal terminal — `many-ai-cli` never has to parse or reimplement a CLI's own TUI. The backend is OS-specific behind a build tag: `pty_unix.go` (non-Windows) wraps `github.com/creack/pty`'s `ptyProcess`, while `pty_windows.go` wraps `github.com/aymanbagabas/go-pty`'s ConPTY-backed `conPtyProcess` — Windows has no native POSIX PTY, so ConPTY (Windows' own pseudoconsole API) is the equivalent primitive. On the Unix side, `Close` is idempotent and layered: the first call asks the child to exit and closes the PTY master, only escalating to `SIGKILL` if `Wait` does not complete within a grace period; later calls are no-ops.

## Idempotent input delivery: `maxProcessedInputSeq`

Every `pty_input` message the Hub sends to a wrapper carries a monotonically increasing sequence number, and the wrapper tracks the highest sequence number it has actually finished writing to the PTY (`wrapperSession.maxProcessedInputSeq`) rather than trusting that every message arrives exactly once. This exists because the Hub resends an un-acknowledged input frame using its *original* sequence number rather than a new one, and a WebSocket can drop between the wrapper finishing a PTY write and its acknowledgment reaching the Hub (`abortCurrentConn` closes without waiting on the send mutex, so an in-flight ack can be lost) — without this guard, that resend would write the same bytes to the PTY a second time, and because a submitted prompt typically ends in a carriage return, writing it twice does not just duplicate visible text but submits an extra Enter, silently confirming whatever prompt happens to be showing next. `inputSeqAlreadyProcessed` treats any sequence number at or below the recorded maximum as an already-applied resend and acknowledges it without writing to the PTY again; `markInputSeqProcessed` is only ever called after a PTY write actually *succeeds*, specifically so a failed write's sequence number is never marked done and a resend for it can still get through — the one accepted gap in this scheme is that a failed write followed by a later *successful* write past it will cause the failed one's resend to also be treated as already-processed, but this is judged acceptable because a PTY write only fails when the PTY itself is broken, which already ends the whole session.

## Zombie protection: the reconnect-grace state machine

The `reconnectSupervisor` goroutine is what decides whether a wrapped session's process should die immediately or wait for the Hub to come back after its WebSocket connection drops — this is the mechanism behind the README's "running AI sessions wait up to 60 minutes for the Hub to come back before terminating themselves" behavior. On a disconnect, it distinguishes several cases rather than reacting to "the WebSocket closed" alone:

- **Genuinely intentional disconnect** — the disconnect was not flagged intentional by the wrapper itself, no local transport fault occurred, the wrapper did not just reattach, and a fresh probe confirms the Hub is still alive. Only when *all four* hold is the disconnect treated as deliberate (e.g., a user clicked dismiss/kill-all) and the PTY is closed immediately.
- **A local transport fault** (the wrapper's own PTY-output send queue filled up, or a write deadline was hit) is explicitly *not* treated as proof the Hub went down — the code comment is direct about why: without this distinction, a transient full output queue or a slow write would kill the wrapped CLI and every one of its own child agents while the Hub was still perfectly healthy, even with `auto_shutdown` enabled.
- **A disconnect within `postReattachGuard`** (10 seconds) **of a successful reattach** gets the same benefit of the doubt as a transport fault, even though a bare post-reattach WebSocket EOF never actually touches the transport-fault code path — this was added after a specific bug (a Codex reattach's plain EOF being misclassified as an intentional kill) where the fix for the queue-full case alone left this second immediate-death path still open.
- Everything else enters the **reconnect grace period**: `WrapperReconnectGraceSec` (`config.yaml`'s `hub.wrapper_reconnect_grace_sec`, extendable up to 24 hours for long-running autonomous tasks per the README) governs how long the wrapper waits for a reconnect before finally closing the PTY; a configured value of 0 disables grace entirely, restoring the older "kill immediately" behavior.

Only when `AutoShutdown` is enabled *and* the disconnect is none of the above protected cases does the supervisor treat "the Hub appears down" as sufficient reason to terminate the PTY without waiting out the grace period at all.
