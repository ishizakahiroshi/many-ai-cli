# C3 service checkpoint

Fixed Go oracle: 21d0bc7935a2c4696fb89ccff2e324157a528c2d.
This is incremental source/fixture evidence, not complete application acceptance.
Existing Go application and Web TypeScript remain unchanged.

## Tested slice

- Hub: 57 tests pass in the latest frozen full run (the earlier focused run had 56). The existing guard/settings/
  PIN/auth-port/network-policy and five HTTP/file-stream tests remain green.
  Fourteen socket/effect tests exercise acknowledgement ordering, a reserved
  writer before core publication, stale binding isolation, bounded writes,
  explicit best-effort broadcasts, ordered persistence tickets and typed Git
  events. Five first-frame/auth/framing tests cover whole-value Go decoding and
  bounded internal registration proofs. Five real loopback RFC6455 tests use
  the actual SessionEngine for UI snapshots, existing Web text input, output,
  reattach acknowledgement/old-socket closure, signed-64-bit declared exits and
  accepted paste/Enter draining across revocation. They use synthetic roots,
  socket peers and explicitly unavailable provider spawning.
- Files/Git/attachments: 39 focused tests pass, including independent append
  handles, private creation/repair, symlink/FIFO/renamed-parent cases, journal
  injection, and a test-owned TZ=Asia/Tokyo subprocess proving local RFC3339
  Git-turn timestamps without mutating the parent test environment.
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
The WebSocket lifecycle/effect driver now has focused real-core loopback evidence.
Production bootstrap and service observers, spawn/dynamic operations, provider
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


## Transport/capability follow-up verification

Latest focused commands used the pinned toolchain, locked offline dependencies,
a task-owned compact `CARGO_TARGET_DIR`,
`CARGO_INCREMENTAL=0`, zero dev/test debug info and two build jobs. Each whole
process had a 180-second deadline; no parallel Cargo was used.

- `cargo test --locked --offline --manifest-path rust/Cargo.toml --lib hub:: -- --nocapture`: 56 passed, exit 0.
- `cargo test --locked --offline --manifest-path rust/Cargo.toml --lib files:: -- --nocapture`: 39 passed, exit 0.
- `cargo clippy --locked --offline --manifest-path rust/Cargo.toml --lib --tests -- -D warnings`: exit 0.

Corrections before these passing runs: qualified the shared register method;
fixed the actual Web input direction (`text` from UI, `data` to wrapper);
preserved the minimal `reattach_ack` including mandatory false token_statusbar;
reserved pending writers before asynchronous core publication; separated the
input/ping pumps so an awaited UI write cannot suspend accepted PTY work; and
applied the focused collapsible-if lint repair.

Actual attachment entry points require `FilesService::with_history` using the
same C2 journal as the effect driver. No direct SQLite fallback bypasses the log
gate. The production journal warns/continues for source-tolerated history errors;
an explicitly failing injected sink is reported after the attachment save and is
not retried. Native Windows ACL/reparse/append behavior remains unaccepted.
The new directory helper restricts only the requested final existing directory;
existing ancestors retain their policy. Existing hard-link alias behavior has
been raised separately and is not claimed to be resolved by no-follow handles.

Live `/api/log-config` and `/api/input-config` changes still need the actual
runtime journal/timing refresh wiring. Voice/notification sources and 32 prepared
tests remain outside this registered checkpoint pending dependency registration
and execution. All 155 route registrations and 49 dynamic operations remain in the
coverage inventory; component implementation is not route acceptance.


## Frozen aggregate verification and settings correction

The integration owner verified **430 passing tests** in the index-only frozen
snapshot on 2026-10-03. This includes the registered Hub/files/profile/memo/
scheduling/updater slice and shared/core/launcher components. The new wrapper,
voice and notification lanes were excluded. Full-snapshot fmt, strict all-targets clippy, gosec and staticcheck subsequently
passed. Staticcheck required an explicit writable isolated cache; the earlier
focused C3 strict lib/tests clippy receipt above remains separately valid.

The six existing settings callers now use the shared source-specific
`publish_then_persist_legacy` entry. Their Go handlers publish the mutation before
Save. The previous Rust implementation and its rollback fixture incorrectly
assumed transactional publication. The replacement fixture covers all six routes:
failed persistence returns 500/save_failed, leaves the new values and revision
visible in memory, preserves existing filesystem contents and removes temporary
files. Auth, token and PIN mutation handlers were not changed by this correction.
An additional fixture confirms null and empty orchestration provider lists both
project to JSON null; the already-correct projection code was retained.

The owned receipt metadata still reports **57 partial / 98 unresolved registered
routes, zero accepted routes**, with all 155 registrations and 49 dynamic
operations retained. The aggregate test count is not production, browser, native,
remote, provider, audio/model or notification acceptance. Actual runtime config
consumer wiring and remaining service/capability integration remain open.
