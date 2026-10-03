# Portable wall-clock representation (candidate implementation, 2026-10-03)

## Native evidence and scope

At `010d8ad38c443c531900602a49856246376ce5f4`, Windows native CI fails the unchanged Go timestamp corpus and memo timestamp fixture: logical `123456789` nanoseconds becomes `123456700`, and year 0001 plus one nanosecond loses its fraction. Rust's platform-dependent `SystemTime` stores Windows FILETIME ticks at 100 ns precision. The fixed Go corpus must not be rounded, skipped, or regenerated from Rust to make that job pass. See [native run4](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37117757872) and [Rust SystemTime platform precision](https://doc.rust-lang.org/1.90.0/std/time/struct.SystemTime.html#platform-specific-behavior).

This is a logical wall-clock compatibility correction, not a new scheduler, timezone database, clock source, wire schema, or monotonic timeout policy. Existing RFC3339/RFC3339Nano/explicit-offset output selection remains unchanged. `Instant`, Tokio sleeps/timeouts, and `Duration` remain the elapsed-time primitives.

## Representation and conversions

Shared candidate `proto::time::Timestamp`: normalized signed Unix seconds (`i64`) plus a nanosecond remainder (`u32`, strictly less than 1,000,000,000). It is Copy, ordered, and immutable; constructors enforce normalization/range. A negative fractional instant uses floor seconds plus positive remainder. Arithmetic with `Duration` checks the seconds range, and signed duration comparison retains the current before/after semantics.

- Parsing RFC3339 and constructing deterministic fixture times produce Timestamp directly, never through `SystemTime`.
- Existing formatters consume Timestamp and retain the corpus's exact nine-digit maximum precision and Go offset quirks.
- Reading the OS clock or filesystem metadata is an explicit native-to-logical conversion. This preserves the precision actually supplied by the OS; it does not invent additional precision.
- Conversion back to `SystemTime` is an explicit checked boundary. A round-trip comparison rejects an out-of-range or sub-tick value instead of silently rounding. Callers that only format, compare, persist or send a timestamp never need this conversion.
- Chrono civil-time scheduling converts through seconds/nanoseconds directly. It must not transit through the OS clock representation.
- Protocol JSON fields that are already strings stay strings. No database schema or Go-compatible stored text changes are intended.

## Consumer inventory before changes

Inventory command: `rg -n 'SystemTime|UNIX_EPOCH|parse_rfc3339' rust/src rust/tests --glob '*.rs'`.

Logical wall-clock consumers to move together with their typed contracts and fixtures:

1. `proto/{time,core}`, approval token/record/transcript detection, terminal input/session lifecycle/observations/UI/approval/input and journal: event clocks, exact last-output clock, history timestamps, approval expiry, pending confirmations.
2. `hub/{router,pin,transport,websocket,routines}`, `routine/{memo,schedule,model,validation,store,runner}`: one captured request time, PIN guard timing, memo timestamps and routine scheduling/inactivity. New confirmation APIs must use Timestamp rather than adding more platform-dependent boundaries.
3. `storage/{mod,repository,history}` and `orchestration/{handoff,subagent,subagent/codex}`: database event clocks and parsed transcript times. Filesystem modified times in handoff/subagent scanning are explicit conversion points.
4. `files/{time,git_turns,content,attachments}`: distinguish parsed logical client timestamps from metadata timestamps. Conditional file comparisons must compare the two in the logical domain; HTTP-date formatting still deliberately has second precision.
5. `application/hub_runtime`, launcher active-file timestamps, wrapper hook cleanup, and retained notification/voice modules: native clock acquisition converts at entry. Pure filesystem cleanup may keep native times if no parsed/logical timestamp enters that calculation.
6. Contract fixtures for time, approvals, terminal/session, storage, routines, handoff, journal, subagents, runtime ledger, files and Hub: create logical timestamps directly. Actual file `set_times` calls remain explicit OS boundaries and use exactly representable fixture values.

## Validation and acceptance

The candidate adds normalization/arithmetic/conversion regressions for negative fractions, bounds, exact OS round-trip and Windows sub-tick rejection. Inventoried logical callers migrated without changing output assertions; the immutable full locked suite, formatting and strict clippy pass. The existing Go formatting/parsing corpus is byte-identical, with an added direct parse-format nine-digit regression on every platform. Re-running all four actual native targets remains required.

Author test and native CI receipts are distinct from independent review. The currently blocked independent re-review, native filesystem confinement, full command/service integration, copied-data rollback and cutover gates remain open. This document records the bounded implementation and its author validation; native and independent acceptance remain separate.

## Implemented boundary and author validation

The initial logical migration registers `proto::time::Timestamp` and changes the logical callers listed above. File metadata formatters, HTTP dates, attachment retention, launcher stale-temp cleanup and wrapper hook cleanup retain native SystemTime where they only consume native filesystem values; formatting/concurrency checks cross explicitly into Timestamp. File save's parsed `base_mtime` is compared to a converted metadata timestamp, without any conversion of the client's value back to native precision. Handoff/subagent metadata scanning also converts explicitly. Newly retained notification/voice sources remain outside the compiled receipt until their modules/dependencies are registered.

Initial `cargo check --offline --locked --all-targets` passes after resolving every typed native/civil-time boundary. The first full author test invocation selected a non-compiler `go` from PATH for the database rollback oracle, so that one test failed with “Go: Unknown option: run”; all other targets passed. A clean root-package rebuild in the immutable clock export, with pinned Go selected, subsequently passed520 tests, formatting and strict all-target clippy. This earlier attempt remains an environment selection error, not a Go/Rust behavioral divergence.
