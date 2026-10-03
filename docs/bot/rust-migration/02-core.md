# #3 many-ai-cli rust: session core and process lifecycle

> 最終更新: 2026-10-03(土) 12:07:36

Entry point: [README.md](README.md). Shared contracts: [00-contracts.md](00-contracts.md).

Task label: `#3 many-ai-cli rust`. This is a coordination label, not GitHub issue or PR number 3. Keep progress in PROGRESS.md through the integration owner. The operator controls external messages; when directed use the same Slack or ChatGPT Web conversation where this task was started. Do not initiate messages or report into #1/#2 tasks.

## Goal and ownership

Implement the Rust session core while preserving the existing Go application's user operations, wire protocol, persistence, and process lifecycle. The reference source baseline is commit `21d0bc7935a2c4696fb89ccff2e324157a528c2d`. Read the source and tests at that commit; do not infer behavior from names alone. Leave Go source unchanged. Reuse the existing TypeScript frontend.

Use the single package at `rust/Cargo.toml`, with `src/lib.rs` and two binary entry points. This task owns the planned modules `terminal`, `storage`, `approval`, and `orchestration`, and their tests. The module/file names below are proposed implementation destinations, not claims that those files already exist. Read the shared contract document before implementing and reconcile destinations with the actual foundation code.

The foundation owner controls Cargo dependencies, shared `proto` and `config`, `lib.rs` module registration, and binary dispatch. Request changes through that owner rather than editing those files concurrently. The services and delivery tasks own their modules; the parent integration owner controls public release documentation. Integrate through shared interfaces; do not redesign their contracts independently.

No existing Hub may be stopped, restarted, updated, or used as a test instance. Tests must use separate temporary configuration, logs, data, profiles, tokens, runtime files, active-process registries, ports, executable paths, and process ownership. Distinct ports alone are insufficient isolation. Use synthetic provider processes and synthetic data. Do not call paid AI providers, read real credentials, modify actual profile settings, or use real session transcripts as fixtures.

## Implementation order

1. Fix shared interfaces and trace each source entry point to its Rust destination and acceptance test.
2. Implement pure terminal state machines and storage independently in parallel, using exclusive file ownership.
3. Implement approval identity, detection, records, and transcript source selection using terminal and storage interfaces.
4. Integrate OS-specific PTY/wrapper adapters and session registration, streaming, reconnection, and termination.
5. Integrate headless execution, child admission, orchestration, relay state, recovery, handoff, and subagent readers.

Do not treat an empty handler, constant success result, ignored failure, hard-coded response, or unimplemented command as a compatible implementation. Maintain a visible inventory of unresolved entry points and acceptance gaps. Tests, native OS acceptance, frontend acceptance, and release-artifact acceptance are distinct completion states.

## Terminal, replay, and input

Source: `internal/hub/vt_buffer.go`, `ui_broadcast.go`, `input_gate.go`, `wrapper_loop.go`; process adapters and replay behavior under `internal/wrapper/`. There is no `internal/vt` package at the reference commit.

Planned destinations: `rust/src/terminal/{mod,vt,replay,input}.rs`, `rust/tests/terminal_contracts.rs`, and synthetic fixtures under `rust/tests/fixtures/core/terminal/`.

Preserve these behaviors:

- UTF-8 and escape sequences can span output chunks. Keep pending bytes and parser state across chunks.
- Preserve wide characters, combining/zero-width characters, deferred wrapping, cursor movement, erasure, scrollback, and escape/string sequence limits and recovery.
- Preserve alternate-screen state independently of screen content. History reset retains mode; a size change discards stale scrollback as the existing implementation does.
- When reconnect replay starts while the session is in alternate-screen mode, emit the required mode prefix before replay bytes.
- Input queues are bounded; preserve the current pending/inflight limits, sequence IDs, acknowledgements, reconnect resend behavior, injection gates, and provider-specific submit timing.
- Replaying output or resizing a pane must not create a new live prompt epoch. Preserve resize ownership across panes.

Port expectations from `vt_buffer_test.go`, `vt_buffer_ech_test.go`, `audit_vt_buffer_string_seq_test.go`, `ui_broadcast_test.go`, `reattach_altscreen_test.go`, and `internal/wrapper/pty_replay_test.go`. Include wide-character erasure, split Unicode/CSI/OSC/DCS sequences, overlong unterminated sequences, resize, reset, reconnect, duplicate ACK, and queue-cap cases. Use byte-by-byte and alternate chunk divisions to verify equivalent terminal snapshots.

Completion requires both pure state-machine tests and integration that passes replay mode, epoch, and input sequence state through the actual session caller. A terminal parser by itself does not complete this area.

## SQLite, history, and reset

Source: `internal/sessionstore/store.go` and its tests. Planned destinations: `rust/src/storage/{mod,schema,repository,writer,history}.rs`, `rust/tests/storage_contracts.rs`, and synthetic storage fixtures.

Preserve the existing schema, columns, indexes, SQL semantics, and additive migrations. The base tables are `sessions`, `events`, `messages`, `approvals`, and `attachments`; the external-content FTS5 table is `messages_fts`. Select actual dependency versions and features through the foundation owner and prove FTS behavior with tests rather than assuming a feature name.

Preserve connection-local busy timeout, synchronous mode, foreign-key settings, WAL, incremental auto-vacuum, connection ownership, transaction boundaries, and FTS-unavailable fallback. Persistent database session identity and live session identity are distinct. Keep their read/write mappings compatible.

The asynchronous writer must preserve bounded capacity, error notification, shutdown drain, and generation barriers. Events queued before a history reset must not repopulate the reset history. Preserve active sessions, approval resolution, message/event selection for transcript-backed providers, bounded pruning, pending file reset, and recovery behavior.

Port expectations from `store_test.go`, `approval_ledger_test.go`, `noise_output_test.go`, and `audit_store_test.go`, including:

- Restore, search, prune, persistent identity, and reattach.
- Queue drain and reset with queued events from an older generation.
- Preservation of active rows during history reset.
- Busy timeout on the first connection, FTS fallback, incremental vacuum, and pending file reset.
- Synthetic user-input masking and transcript-backed message selection.

Start with clean and older-schema synthetic databases. Compare query results and schema metadata. Demonstrate that the synthetic database remains readable by the reference implementation after Rust writes. Real-data backup, copied-data migration, and restoration require separate integration evidence; never run old and new instances as writers against the same database.

Completion requires the full repository API, event/message/approval transactions, asynchronous writer, reset, and pruning contracts. Opening a database and implementing search alone is insufficient.

## Approval identity, records, and transcript polling

Source: `internal/hub/approval_identity.go`, `approval_record.go`, `approval_marker_transcript.go`, related `approval_*.go`, `auto_approval.go`, transcript polling/readers, Codex thread-following code, and `provider_feature_source.go`. Frontend counterparts are `web/src/app/approval-store.ts` and `approval-answered.ts`.

Planned destinations: `rust/src/approval/{mod,identity,detect,record,transcript}.rs`, `rust/tests/approval_contracts.rs`, and synthetic approval fixtures.

Preserve a single answered-state identity based on `candidateKey` plus `sourceEpoch`. The record signature is a reference, not a second identity. Normalize the same question/options/send text and contextual identity distinctions as the reference implementation. Advance epochs on live user-turn boundaries, not replay or reflow. Preserve the reference carry and narrowly scoped persisted-ledger recovery behavior.

Create immutable pending records and expose the current `approval_state` and `approval_snapshot` wire contracts. Keep native/marker origins, detection source, kind, block, options, summary, close reasons, version, and timestamps compatible. The frontend renders Hub records; do not add terminal-derived approval state or resurrect legacy per-approval messages.

Use one marker source per session. Select transcript-backed detection from the actual provider feature and a resolved, readable, unambiguous transcript. Preserve fallback after the existing miss threshold and recovery when the transcript becomes readable. Do not confuse structured-chat support with approval-marker source support. Preserve first-path-resolution polling, prime-last-message restoration, user-message closure, cursor adoption, and the next poll's resumption.

Port expectations from `approval_identity_guard_test.go`, `approval_record_test.go`, `approval_marker_transcript_test.go`, `approval_replay_fixture_test.go`, detector/native tests, `internal/hub/testdata/approval_codex_shortcut_ansi.ansi`, and `approval_summary_cases.json`. Include initial resolution, source fallback, user answer then repeated question, replay suppression, reattach versions, Hub-restart restoration, late consumed replies, multi-question markers, and provider-specific redraw contexts.

Completion requires the actual poll → detection → record → wire → persisted ledger → next poll integration. Merely translating parser regexes or source-scanning guard tests does not prove this contract.

## PTY, wrapper, and session lifecycle

Source: `internal/wrapper/pty_unix.go`, `pty_windows.go`, wrapper/replay/launch-prompt/environment code, `internal/execpath/`, and `internal/hub/wrapper_loop.go`, `input_gate.go`, and PTY-size/reconnect tests.

Planned destinations: `rust/src/terminal/{pty,pty_unix,pty_windows,wrapper,session,launch}.rs`, `rust/tests/wrapper_contracts.rs`, and synthetic process fixtures.

Implement OS-specific adapters behind the shared session interface. Preserve read/write/resize semantics, startup size, executable and argv resolution, custom-provider precedence, npm shim handling, prompt transport, environment precedence/unsetting, and output/exit classification. Windows process exit status must not be presented as a POSIX signal.

Close and wait must each be idempotent and have a single result. Preserve termination escalation and completion of output readers. Test early exit, cancellation, output backpressure, reconnect identity, replay, ACK, double close, and double wait using fake PTY/process adapters.

Native acceptance must separately measure normal children, detached children, inherited output pipes, early exits, cancellation, and remaining processes. Record the Windows version and termination timing. Evaluate process-containment changes using observed results and compatibility effects; do not assume that adding a Job object is sufficient or harmless. Unix process-group behavior also needs native acceptance.

Completion requires registration → streaming → input/ACK → reconnect → termination through actual callers. Fake-process success does not establish native process cleanup or visible frontend behavior.

## Headless execution, orchestration, relay, and handoff

Source: `internal/headless/`, `internal/handoff/`, `internal/hub/orchestration*.go`, `relay*.go`, `handoff*.go`, `subagent_source_*.go`, and `internal/provider/subagent_adapter.go`.

Planned destinations: `rust/src/orchestration/{mod,headless,process_unix,process_windows,admission,board,relay,worktree,handoff,subagent}.rs`, `rust/tests/headless_contracts.rs`, `rust/tests/orchestration_contracts.rs`, and synthetic orchestration fixtures.

Preserve child admission, reservations, per-parent/global limits, deduplication, confirmation, request origin, role/provider/profile selection, execution modes, permission defaults, timeouts/retries, board-writer authorization, and initial prompt delivery. Admission must be atomic: concurrent requests cannot exceed the configured limit, and failed or completed starts release reservations.

Headless readers must start before potentially blocking prompt writes. Close stdin according to the prompt transport contract. Preserve output parsing, stderr events, oversized-line handling, cancellation of owned process trees, opt-in raw logs, and exit classification. Test containment setup failure and descendants retaining output pipes; a deadline must not leave the caller waiting indefinitely.

Keep relay decisions in the Hub state machine. Preserve per-unit completion counters, review verdict requirements, stronger review transitions, stale-exit rejection, persistence, recovery, and one headless process per instruction. Do not nudge headless children. Operate on registered task worktrees and preserve the user's branch and unrelated work.

Handoff records retain the current typed field allowlist and JSONL version semantics. Transcript/note fields name paths; do not copy file contents, environment values, diffs, or raw input into records. Preserve private/atomic writes and provider-specific subagent reader behavior.

Port `internal/headless/runner_test.go` helper-process cases, `format_test.go`, orchestration child-default/inject/composer tests, `relay_test.go`, `relay_headless_test.go`, `relay_store_test.go`, handoff allowlist/render tests, and each subagent-source provider test. Git/worktree tests use temporary synthetic repositories only. No paid provider or existing worktree is a test fixture.

Completion requires correct limits under concurrent admission, reservation release, timeout/early-exit/restart handling, review failure behavior, evidence retention, persisted recovery, and task worktree cleanup. Test output must show which paths ran; skipped native cases remain pending.

## Verification and delivery

After the foundation interfaces exist, run formatting, applicable unit/integration tests, and static lint against the actual manifest and selected features. Planned commands are `cargo fmt --manifest-path rust/Cargo.toml -- --check`, `cargo test --manifest-path rust/Cargo.toml`, and `cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings`. Adapt filters to the implemented package; retain complete failures without secrets. Do not claim native-platform acceptance from compilation alone.

Deliver a source → Rust destination → test → caller/wire/persistence trace, test receipts, native/UI acceptance gaps, and unimplemented-path inventory with the PR. Coordinate binary wiring, integration, and release documentation with their owners. Stop and report if preserving public behavior requires destructive data conversion, incompatible public semantics, a platform-library mismatch that cannot be resolved within the design, or a major test regression. Routine naming and implementation choices do not require a new approval round.
