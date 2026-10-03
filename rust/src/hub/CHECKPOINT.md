# C3 service checkpoint

Fixed Go oracle: 21d0bc7935a2c4696fb89ccff2e324157a528c2d.
This is incremental source/fixture evidence, not complete application acceptance.
Existing Go application and Web TypeScript remain unchanged.

## Tested slice

- Hub: 27 guard/settings/PIN/auth-port/network-policy tests, followed by five actual
  loopback HTTP/file-stream tests. The latter exercise token-before-method/Host
  with an unfinished advertised body, nanosecond-accurate PIN lockout boundaries, first-value JSON response without waiting
  for trailing body bytes, the actual 10-second HTTP/1 header deadline, bounded
  multipart held-file streaming, and partial-effect failure response semantics.
- Profile: 16 tests. Eight fixed-Go registry cases cover layer precedence,
  revisions encoded with shared Go-compatible typed bytes, builtin collisions,
  explicit overrides, diagnostics and capabilities. Nine fixed-Go validation
  inputs cover schema, shell syntax, Unicode icon approximation, duplicate
  fields, extra JSON values and unknown extreme numbers.
- Codex profile tests cover absent destination with user settings/hooks retained,
  exact generated temporary hooks removed, existing profile ownership,
  default-wins overrides, nested policy deletions, sync-off preservation,
  malformed input, read-only source symlinks, rejected destination symlinks,
  private 0600 writes and no auth.json propagation.
- Memo/image: six persistence/API tests cover CRUD/reload, failed writes,
  corrupt stores, server-derived project identity, UTF-8 byte bounds, image
  signature sniffing, exclusive references, post-save image removal and orphan
  cleanup.
- Routine scheduling: one test exercises 223 synthetic results generated from the
  fixed Go nextRoutineTime helper. This does not establish all-zone equality:
  the approved chrono-tz currently embeds 2025b, whereas pinned Go embeds 2025c.
  The source data delta is historical Baja California DST in 1953 and 1961–75;
  these modern/future samples are unaffected. Whole-database equality remains
  unproved.
- Updater: eight tests, including real task-owned synthetic A/B child processes,
  absent B, secondary launch candidates, unchanged immutable plans after registry
  changes, classification, update/spawn admission, timeout and failed-start lease
  cleanup. The resolved B path is shared by preview, log and executed process.

Commands (all exit 0 at this checkpoint):

    cargo test --manifest-path rust/Cargo.toml --lib hub:: -- --nocapture
    cargo test --manifest-path rust/Cargo.toml --lib hub::transport::tests -- --nocapture
    cargo test --manifest-path rust/Cargo.toml --lib profile::tests -- --nocapture
    cargo test --manifest-path rust/Cargo.toml --lib routine::tests -- --nocapture
    cargo test --manifest-path rust/Cargo.toml --lib update::tests -- --nocapture

Earlier failed checks were preserved in task execution output: initial macro
spacing and a raw/default setting fixture expectation; initial Codex hook
recognition incorrectly reused the Windows-style command tokenizer for a POSIX
quoted executable; and a transient missing files-lane test module during
concurrent edits. The focused passing reruns above follow those repairs.

## Wiring and remaining work

ServiceRouter requires the actual selected listener port. Production filesystem
roots never choose HTTP authority; explicit trial ports must still match.
HTTP/1 serving verifies IPv4 loopback binding and uses an injected shared
CoreEffectSink, retaining failed-effect index and never replaying a partially
applied batch. A failed effect after persisted auth rotation preserves the new
token/address in its error response rather than silently discarding them.

File responses own already-open descriptors; single and multipart ranges use
bounded chunks. Files/Git/attachment behavior and native capability proofs are
reported by the dedicated files lane. Windows private ACL creation and native
reparse/rename behavior remain unaccepted.

The router can connect actual files and memo services via explicit constructors.
Core WebSocket lifecycle/effect driver, spawn/dynamic operations, provider
persistence/distribution/history, usage/NIM, routine lifecycle, outbound
notification/push and voice/native runtime remain unfinished. Unsupported paths
never return placeholder success. route_coverage.json tracks all 155 registered
paths and 49 dynamic operations; partial implementations are not accepted routes.
No real provider, notification, audio capture/model installation, SSH/WSL,
production Hub, live credentials or copied user records were used in tests.

## Release-note input for eventual completed candidate

- New Codex profiles inherit user-maintained settings and hooks while omitting
  application-owned temporary session hooks; credential files remain separate.
- A configured CLI update executable is resolved independently from the launch
  executable, and eligibility, preview, logs and execution agree on that command.
  A missing updater never falls back to the launch command.

These notes describe intended candidate behavior, not authorization to release,
merge, deploy or switch the existing application.

The focused lint repair boxes the optional streamed file descriptor, factors a
transport read-task alias, and applies equivalent initializers/control flow.
PIN limiter deadlines/TTL bookkeeping now use nanosecond arithmetic with
full request timestamps; signed cookie expiry remains Unix seconds as in Go.
Fractional boundaries are covered without weakening the timing assertions.
