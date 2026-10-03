# Routine service implementation checkpoint

Baseline: Go `21d0bc7935a2c4696fb89ccff2e324157a528c2d`.

This slice adds one private durable owner for definitions, runs, request aliases,
missing-session clocks and retryable result snapshots. It adds an HTTP operation
adapter and a runner using the actual shared SessionCore admission/spawn boundary.
No second session manager or successful fallback launcher is provided.

## Source traceability

- `routine_store.go`: record fields, version-one historical loading, transactional
  publication and new-input validation. Historical loading does not revalidate
  provider names, paths or schedules. The initialized Version=1/null boundary uses
  shared raw-member and typed Go JSON readers. Held directory capabilities replace
  pathname-based mutation without claiming native filesystem acceptance.
- `routine_handlers.go`: route/method precedence after the ordinary Hub guard,
  first-value JSON decoding, request-ID limits, stale-edit/delete/count errors,
  reverse history/filtering and allocated empty arrays. Handler errors hide local
  persistence/process details.
- `routine_runner.go`: persist before launching; concurrent/alias idempotence;
  immutable launch labels; exact 30/60/90-second boundaries; startup/sleep skips;
  live/database/instance identifiers; no replay for missing sessions; result
  masking and UTF-8 limits; committed terminal records remain immutable.
- A failed final-result save leaves the durable run active. A later DONE may
  replace its still-unpersisted pending result, exactly as Go lines329–352 do.
  A successful final commit prevents later DONE or launch-wait results from
  replacing it. No stronger first-pending-result policy was adopted.
- Existing `schedule.rs` retains its 223-case modern/future Go corpus. Whole-TZDB
  equality remains unproved: chrono-tz2025b versus Go2025c historical Baja changes.

## Integration contract

The integration owner released registration after capturing the prior frozen
export. `routine::{model,validation,store,runner}` and `hub::routines` are now
registered. Construct a private
held directory from the explicit RuntimePaths root, then RoutineStore::open.
RoutineHttp::handle_authenticated must be called after the ordinary Hub guard.
It does not independently authenticate requests. Routine DELETE ignores its
body in Go and must bypass the generic body reader when wired. Bind the actual core, home,
Hub instance and Hub-owned TaskCancellation into RoutineRunner.

RoutineLaunchPreparation must use the actual provider registry and pending initial
prompt owner to choose argument versus post-registration delivery. It returns the
shared WrappedSpawnSpec; core creates its admission lease and one-use proof.
The service checks immutable provider/cwd/model/label identity, and its launch
waiter is independent of HTTP cancellation. The preparation capability is
required; absence returns an explicit503 launcher-unavailable gate before any
run is admitted. This is a migration integration gate, not a source-parity success.
No production preparation adapter is included in this slice.

Route DoneSummary events to runner.record_done. Run runner.run once with Hub
cancellation, then join_launches during shutdown. Runtime observations use
SessionDetails.last_output_at directly, preserving nanoseconds. Main-app routing,
actual provider preparation, event subscription and shutdown wiring remain with
the integration owner.

## Evidence as of 2026-10-03

- All new source files parse through pinned rustfmt; existing registered modules
  were not changed while the integration export was frozen. The four routine
  module declarations were added only after the explicit registration release.
- Both fixed-Go generators ran successfully by exact filename. The store oracle
  emitted12 synthetic record cases. The result oracle uses the exact Go
  commit/record-DONE/refresh functions and emitted4 failure/retry/terminal stages.
  Build-ignore tags keep these standalone tools out of package discovery.
- `cargo test --locked --offline --manifest-path rust/Cargo.toml --lib routine:: -- --nocapture`:
  **21 passed, 0 failed** (11 new store +3 new real SessionEngine/spawn-admission
  callers with synthetic launch transport +7 existing memo/schedule tests).
- `cargo test --locked --offline --manifest-path rust/Cargo.toml --lib hub::routines:: -- --nocapture`:
  **4 passed, 0 failed**. These are HTTP operation tests after the guard boundary,
  not complete served-route/browser acceptance.
- `cargo clippy --locked --offline --manifest-path rust/Cargo.toml --lib --tests -- -D warnings`:
  **exit0**. Commands used pinned Rust1.90.0, the shared runtime target, jobs2,
  incremental0 and dev/test debug0. Full logs were retained under
  the retained author test logs.
- Tested source is frozen for integration. Production preparation, dispatcher,
  event and scheduler lifecycle wiring remain explicit integration work.
- Existing memo and schedule source is unchanged. No real routine, provider,
  home, external transport, notification or audio/model operation was performed.

HTTP inventory acceptance, production integration, native OS behavior and real
browser/provider acceptance remain open.
