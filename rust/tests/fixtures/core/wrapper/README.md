# Wrapper runtime slice

Go oracle: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`.

This fixture lane uses only synthetic providers, owned temporary directories,
scripted transports and loopback HTTP/WebSockets. No real Hub, provider login,
provider API, user home, shell profile or transcript is an acceptance fixture.

## Implemented source and test seams

- `process/execpath.rs`: injected filesystem/environment lookup, Windows npm shims,
  custom argv precedence, shell defaults, Copilot/gh boundary; strict standalone
  command resolution is available to update/version callers.
- `process/pty.rs`, `pty_unix.rs`, `pty_windows.rs`: shared native PTY interface;
  Unix pre-exec session/group establishment and cancellable master IO; Windows
  ConPTY suspended creation, shared Job assignment before primary-thread resume,
  separate cancellable input/output threads, and positive DWORD exit values.
- `wrapper/launch.rs`, `shell.rs`, `entry.rs`: config-driven argument/metadata
  mapping, session environment, isolated prompt consumption/pointers, Claude
  native session IDs, shell-init output, interactive/native and existing headless
  execution composition.
- `wrapper/runtime.rs`, `transport.rs`, `input.rs`, `output.rs`: register before
  exec, authenticated loopback WS, one-use spawn-proof header, received/processed
  input watermarks, completed-write ACK, short writes, UTF-8 chunk boundaries,
  provider submit delays, bounded output retention/transport faults, replay,
  reattach pre-ACK frame retention, resize, explicit dismiss versus Hub shutdown,
  exit messages, optional raw logging, Windows mojibake repair, login completion.
- `wrapper/hooks.rs`: private Claude settings and delegation files; OpenCode
  exclusive owner lock, historical plain-PID recognition, stale rollback,
  bounded deny rules, concurrent-edit protection and capability-based cleanup.

## Evidence status

The registered Linux component run passed 210 library and 13 wrapper integration
tests, including native PTY and synthetic loopback transport. Strict all-targets
clippy subsequently passed after explicit byte typing, sha2 digest formatting,
equivalent boxing/formatting and a visible Unicode escape in a fixture. Native
resize errors and failed input writes now retain Go's warning/continue behavior;
failed input is not acknowledged. A later frozen aggregate receipt is recorded
in PROGRESS. The overall application, binary entry wiring and native target
matrix are not accepted by these component checks.

## Required integration and remaining gates

- Shared process/PTY/resolver and wrapper modules are registered; direct locked
  handshake-only WS dependency and Windows Pipes/IO features are wired. The
  suspended ConPTY child reuses the shared Job policy via attach_raw; native
  execution of that boundary is still required.
- Invoke `entry::run_cli` from both wrap forms; pass the already selected Hub,
  explicit runtime root, caller cwd/environment/home, terminal size and shutdown
  cancellation. Wire source-compatible initial Hub discovery/startup separately.
- The actual Hub spawner must hand startup ownership to the authenticated wrapper
  before successful registration acknowledgement, use durable file stdio, retain
  an adopted Windows Job handle until final wrapper exit, and retire failed or
  lost-ACK provisional registration without a label-only success path.
- Lost initial ACK must not launch a provider or retry a consumed proof. Reattach
  is only for initialized wrappers and never presents the initial proof.
- Native Windows tests must run on pre-24H2 and current Windows: early exit,
  assignment failure, detached descendants, output-held pipes, blocked input,
  teardown timing, cancellation, exact argv/env/cwd/resize and high DWORD exit.
- Native Unix tests must measure foreground groups and detached setsid descendants.
  Session/process-group cleanup is not a universal detached-descendant container;
  do not claim native containment acceptance from module tests.
- Frozen-Web pane/reconnect/input behavior, real provider transcript integration,
  real-device/native behavior and full CLI setup/diagnostics remain separate gates.

## Explicit compatibility/security decisions

- Trial prompt files cannot escape the selected runtime root or follow symlinks.
  Invalid UTF-8 prompts fail instead of being silently lossy-converted.
- If OpenCode's bounded deny overlay fails, the candidate refuses that launch
  rather than running `--auto` with a wider effective permission tier. Ordinary
  noncritical hook setup retains warning-and-continue source behavior.
- Only the initial wrapper registration receives the exact internal spawn proof;
  it is never a JSON Message field, diagnostic, durable record, or provider env.
- Shared `PtyExit.code` is i64: Windows DWORD exit status remains positive and is
  never classified as a POSIX signal. Unix signal termination retains exit=-1.
