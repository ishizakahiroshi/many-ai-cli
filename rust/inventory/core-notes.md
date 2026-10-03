# Core interface inventory for the Rust migration

Status: C1 preparation only. Baseline: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`.

No implementation, build, installation, provider invocation, Git mutation, or Hub interaction was performed by this lane. All future fixtures must be synthetic. `core.json` is the machine-readable companion: 22 contract groups, 93 source files, 809 distinct existing Go test identifiers, and the 34 public Store methods. Counts describe indexed evidence, not implemented or accepted behavior.

## Evidence and scope

Read the root AGENTS/CLAUDE guidance, coding/development/operations guides, migration README and 00/01/02 instructions. Read `.omitnix/index.json` before source-role searches. Its generated commit matches the instructed baseline, but `dirty: true` is recorded; the parent owns repository/baseline verification. Its 13 unresolved sessionstore SQL statements are retained verbatim in the JSON inventory. A table reference count of zero is not evidence that a path is unused.

The primary source boundaries were inspected in `server.go`, `wrapper_loop.go`, `input_gate.go`, `ui_broadcast.go`, `reattach_state.go`, `reattach_replay.go`, `session_activity.go`, `vt_buffer.go`, `sessionstore/store.go`, approval identity/record/action/transcript files, transcript polling, wrapper/process/headless sources, and orchestration/admission/relay/handoff/subagent boundaries. Large parser/detector/provider-specific files are indexed for follow-on semantic review; listing their declarations is not a claim to have reviewed every branch. Existing tests were read selectively at important boundaries and all cited test names were checked against actual declarations.

The JSON `lexical_external_caller_candidates` are exact file/line occurrences, not a Go AST call graph. They deliberately exclude generic short names such as `Run` and `Close`; those callers are recorded in contract-level traces. They can include references in string fixtures, callbacks, or another method with the same name. No absence in this lexical list means an entry is unused. Every Rust path and proposed test ID is a proposed destination, not an existing implementation.

## C1 freeze recommendations

### 1. Disjoint identity types and read-only snapshots

Freeze separate newtypes, preserving signed/unsigned wire widths rather than globally replacing every integer with one type:

- `LiveSessionId` for the running Hub identity; `DbSessionId` for SQLite `sessions.id`.
- `WrapperConnectionId` for a particular connection; `SessionIncarnation` for the logical object against which delayed work was scheduled.
- `InputSeq` (`i64`, zero means legacy/untracked); `ReplayEpoch`, `ApprovalSourceEpoch`, `ApprovalStateVersion`, and `HistoryGeneration` (`u64`, separate domains).
- `OrchestrationId`, `AdmissionId`, `SpawnConfirmationId`, and `RelayProgressId` stay distinct from live session IDs. Restored relay progress-file IDs can differ from newly allocated live IDs.

`SessionSnapshot` must preserve the existing JSON session shape, including `activity` and the optional metadata objects in `server.go`. Do not put secrets, home paths, parser state, websocket pointers, mutexes, timers, pending raw input, or approval action reservations in a serializable generic map. Keep `launch_label` distinct from user-editable `label`. Keep `handoff_from` distinct from parent-child ancestry. Preserve `execution_mode` and `permission_mode` as the actual wrapper declaration, not a request that might not have been applied.

Proposed core-facing methods (names are proposals for C1 approval):

- `SessionCore::register(RegisterRequest, WrapperConnectionId, now) -> Result<Registration, SessionError>`
- `SessionCore::reattach(ReattachRequest, WrapperConnectionId, now) -> Result<Reattachment, SessionError>`
- `SessionCore::snapshot(LiveSessionId) -> Option<SessionSnapshot>`
- `SessionCore::snapshots() -> Vec<SessionSnapshot>`
- `SessionCore::observe_output(SessionBinding, OutputChunk, now) -> CoreEffects`
- `SessionCore::observe_end(SessionBinding, ExitOutcome, now) -> CoreEffects`
- `SessionCore::resize(UiConnectionId, LiveSessionId, TerminalSize, now) -> ResizeOutcome`
- `SessionCore::reset_history(LiveSessionId, now) -> Result<CoreEffects, SessionError>`
- `SessionCore::dismiss(LiveSessionId, now) -> Result<CoreEffects, SessionError>`

`SessionBinding` contains live ID + incarnation + wrapper connection ID. All delayed output, ACK, termination and action callbacks must validate it. A raw live ID is insufficient after reattach/reuse. Registration must hold outbound wrapper messages until `registered` is sent first (`register_ack_order_test.go`). Reattach must retain pre-ACK queued messages on the wrapper side (`TestDialAndReattachWithPendingKeepsMessagesBeforeAck`).

`CoreEffects` is an ordered set of existing proto messages, persistence operations, and explicitly typed observer notifications. It must not introduce a new external wire format. Database writes, websocket sends, filesystem reads and process waits happen after releasing state locks. Callbacks whose validity depends on an incarnation carry that binding. C3 auth epoch/revocation must be checked when queued UI input actually executes, not just when received.

`SessionActivity` is four booleans: `output_idle`, `workflow_active`, `awaiting_user`, `awaiting_approval`. Approval implies awaiting user. Safe noninterrupting input is `output_idle && !workflow_active`; legacy display state cannot replace it. Waiting takes display precedence, then running, then standby.

### 2. Input queue, ACK, epoch, and submit receipt

Proposed API:

- `InputQueue::submit(SessionBinding, InputRequest, now) -> InputDisposition`
- `InputQueue::reserve_frame(SessionBinding, bytes) -> Option<InputFrame>` before transport write
- `InputQueue::acknowledge(SessionBinding, InputSeq) -> AckDisposition`
- `InputQueue::transport_failed(SessionBinding, now) -> InputEffects`
- `InputQueue::flush(SessionBinding, now) -> InputEffects`
- `InputQueue::clear_initial_gate(LiveSessionId) -> InputEffects`
- `InputQueue::mark_processed(InputSeq)` on the wrapper side only after successful complete PTY write

`InputFrame { seq: InputSeq, bytes: Vec<u8> }`; `InputRequest` distinguishes regular user input, initial-prompt bypass, and a bound native-approval action. `InputDisposition` must distinguish absent session, deferred initial prompt, deferred disconnected wrapper, websocket-written, and failed remainder. A websocket write succeeding is not a provider acceptance receipt and not an ACK. Preserve public `input_deferred.reason` values `initial_prompt` and `wrapper`.

All input for one session is FIFO including output-settle sleeps and delayed Enter. Keep the input mutex/actor lifetime shared across reattach; check the current registration again immediately before each frame. Native approval actions additionally pin the original wrapper rather than silently retargeting a reattached wrapper.

Baseline limits and ordering:

- pending/inflight/resend bounds are 100 each; discard oldest when over capacity
- reserve inflight before transport write; undo only that reservation on failure
- ACK marks both connection and session as ACK-capable even when seq is late/duplicate/nonpositive
- remove inflight only when the ACK's connection matches
- move unacknowledged frames to resend only for known ACK-capable wrappers
- resend old sequence numbers first, sorted, before newly pending input
- wrapper's processed high watermark suppresses duplicate PTY writes; received-but-not-finished watermark reserves allocation without suppressing retry
- zero sequence stays legacy/untracked; empty frame remains untracked and unacknowledged
- 90-second initial injection gate; bypass belongs only to the initial prompt and must pass older gated user input
- a failed delayed Enter queues only Enter, not the already-sent paste body

Hub submit timing: 120ms idle settle, 120ms ordinary minimum, 700ms Codex/OpenCode minimum, 30s maximum, 20ms poll, 1500ms post-Enter output confirmation. The wrapper additionally writes UTF-8-safe 1024-byte chunks with 3ms pauses; trailing Enter is split by 20ms or 180ms for Codex/OpenCode. Cursor Agent's leading Ctrl-U is split with 20ms separation. These layers are not interchangeable.

### 3. Storage repository and asynchronous writer

Keep all 34 public methods from `core.json.storage_public_method_inventory`, including methods that currently have no direct `s.sessionStore` caller. Preserve two explicit read families by live ID and DB ID; do not replace them with an ambiguous `get_session(id)`.

Recommended facade:

- `SessionRepository::open(&RuntimePaths, StorageOptions) -> Result<Self, StorageError>`
- `start_session(SessionStart) -> Result<DbSessionId, StorageError>`
- all baseline card metadata, state, end, chat, approvals, search, history, timeline, usage, stale, reset, prune and mention methods with equivalent argument meanings
- `try_store_event(LiveSessionId, HistoryEvent) -> EnqueueOutcome`
- `store_event(LiveSessionId, HistoryEvent) -> Result<(), StorageError>` for synchronous paths
- `set_write_error_handler(Arc<dyn Fn(LiveSessionId, StorageError) + Send + Sync>)`
- `reset_history(&[LiveSessionId]) -> Result<ResetResult, StorageError>`
- `close(ShutdownPolicy) -> ShutdownReport`

`HistoryEvent` may retain a serde JSON object because baseline event payloads are heterogeneous; this does not authorize a generic data bag for handoff or SubagentNode. Owned immutable event data must be copied/moved into the writer. `EnqueueOutcome` should distinguish queued, dropped-with-cumulative-count, and unavailable/closed internally, then preserve caller-visible baseline behavior. Queue cap is 4096, never await disk on PTY output. Capture `HistoryGeneration` when enqueuing, hold the reset barrier while writing, and discard older generations after reset.

Storage filename is still `any-ai-cli.db`, not `many-ai-cli.db`. It resides beside the configured logs directory, with `-wal`, `-shm` and `.reset-pending` companions. Under trial mode these must all be derived from `RuntimePaths`; no helper may derive real home when a trial path is absent.

Persistent identity is `jsonl_path UNIQUE`, with `virtual-live-{live_id}` only when no JSONL path was supplied. StartSession upsert preserves user-edited card label and card metadata while refreshing live metadata and reviving ended rows. On Hub startup close stale rows before registration so a reused live ID cannot update an old live row.

Connection-local pragmas, on every replacement: first `busy_timeout=3000`, then `synchronous=NORMAL`, `foreign_keys=ON`. Init adds incremental auto-vacuum and WAL. One physical connection. Init timeout30s; normal query timeout3s. Preserve five tables, indexes, additive migrations and external-content FTS5; FTS-unavailable and query-error fallback use LIKE. Failed FTS indexing does not discard an otherwise saved message.

`pty_output` has no events-table row. It creates AI messages only for non-Claude/non-Codex sessions and only after visible-text/noise filtering. User input is masked. `MessagesMentionText` searches user messages only, because AI text must not grant read-only file-scope exceptions.

Read-limit compatibility matters: chat limit outside1..1000 becomes400; search outside1..100 becomes50; session list outside1..500 becomes100; timeline outside1..2000 becomes400; approvals nonpositive becomes100 and above500 clamps500. Chat/timeline return ascending order after selecting newest rows; approvals are detected_at DESC then id DESC; sessions use coalesced activity/end/start/update timestamp DESC then id DESC. Keep nil/null/empty differences until C3's actual handler normalization, especially chat versus approvals.

Reset uses pre-reset total counts, not actual deleted counts. It preserves only newest session metadata rows for selected positive live IDs while deleting all events/messages/approvals/attachments and clearing derived session summary fields. Short independent transactions and 2000-row child batches avoid monopolizing the single connection; pruning uses50-session batches and60s budget. SQL or vacuum failure schedules `.reset-pending`. On next open, the marker is removed only after DB deletion succeeds.

### 4. Approval identity, record, and atomic action

Shared proto structs must preserve all baseline fields and omission rules. Core types:

- `CandidateIdentity { key, shape, source_epoch }`
- immutable `ApprovalRecord` containing candidate identity, sig, origin, source, kind, block, question, context, options, summary, detected_at
- `ApprovalSessionState { version, record: Option<_> }`
- `ApprovalCloseReason` wire values `answered`, `answered_terminal`, `superseded`, `vanished`, `session_end`, `history_reset`
- `ApprovalActionBinding { session_binding, candidate_key, source_epoch, sig }`

Proposed methods:

- `ApprovalState::observe(candidate, source_context, now) -> ApprovalEffects`
- `ApprovalState::confirmed_user_turn(turn, vt_snapshot, now) -> ApprovalEffects`
- `ApprovalState::consume(ApprovalActionBinding, selected_text, now) -> ApprovalEffects`
- `ApprovalState::close(reason, now) -> ApprovalEffects`
- `ApprovalState::snapshot() -> proto::ApprovalSessionState`
- `ApprovalActions::prepare_and_send(NativeActionRequest) -> Result<ReservedAction, ApprovalActionError>`
- `ApprovalActions::commit(ReservedAction) -> bool` / `release(ReservedAction)`

An action lease spans validation, transmission, nonce consumption and state commit. It binds the live screen candidate and wrapper; it must be rechecked after waiting for the input lane and before each delayed frame/commit. A successful send must not clear a replacement prompt. On error, release the action reservation. Auto-policy rule must still be enabled/matching immediately before send. This is required for one-tap, batch and automatic actions; ordinary generic input is not a safe substitute.

Only candidate key plus source epoch determines answered state. Sig is a ledger/UI reference. Version is delivery ordering. Normalize whitespace/ANSI, provider/kind, question, option number/send text and applicable command context exactly. Advance source epoch only for live prompt boundaries. Record closure and subsequent insertion for a reused sig must occur in that order outside session lock. A VT marker must not replace a visible native record; transcript markers are not subject to that VT-only exception.

The frontend only receives `approval_state` and `approval_snapshot`, never legacy per-approval messages or terminal-derived state. Snapshot includes sessions with null record and sorts by session ID. Preserve narrow `vtQuestionAnsweredInLedger` recovery and the carry rule while the answered question remains the latest VT question. Do not invent another TTL or permanent answered-signature set.

One-tap token manager remains purpose-limited:120s HMAC token, session/sig/epoch/action/nonce binding, per-Hub secret, nonce not consumed by GET or failed input. High-risk approval is forbidden; batch and auto approve low risk only and never choose persistent/session-wide options.

### 5. Transcript reader and single approval source

Do not merge provider `StructuredTranscript` and `ApprovalMarker` capabilities. Claude/Codex have both native; Command Code has native structured chat but derived/VT approval. Unknown/custom defaults remain no structured transcript and derived approval.

Proposed interface:

- `TranscriptResolver::resolve(TranscriptSessionIdentity, RuntimePaths, now) -> Result<ResolvedTranscript, ResolveError>`
- `TranscriptReader::read(ResolvedTranscript, ReaderState, ReadBudget) -> Result<TranscriptBatch, ReadError>`
- `TranscriptBatch { messages, next_state, safe_offset, decode_committed, reached_eof, budget_exhausted, completions }`
- `TranscriptPoller::apply(SessionIncarnation, PollGeneration, TranscriptBatch, now) -> PollEffects`

The parser owns its cursor. Never replace its committed safe offset with a later file size/stat. An incomplete prime retries the same tail page without promoting it to forward state. Resolve outside the session lock; apply only if incarnation/generation still current. Poll current path is passed to approval scanning before session path adoption, so first resolution restores the pending record immediately. Scan approval before chat broadcasts, which may return early. Tail prime only restores the final message and never emits an old Codex completion as a new event.

Readable source predicate: capability native AND nonempty actually resolved path AND miss streak<3 AND no ambiguous Codex thread. Fallback after3 misses; recovery resets streak. New user messages close the previous marker and permit a repeated new question. Codex thread follow must avoid peer-claimed and subagent transcripts and ambiguous switches.

Bounds to preserve:8MiB max record,64KiB read buffer,200 live messages,8MiB batch,64KiB text,64 thinking/tool entries,256 pending tool references/4MiB pending bytes; normal read4MiB/256 records/100ms; page16MiB/512 records. Poll1s, idle stop1min, early kick80ms. C3 HTTP pagination and error mapping are separate from this core reader contract.

### 6. ProcessPlan, ManagedProcess, ExitOutcome, Cancellation

Use the foundation names the parent is preparing. A process plan must represent a fully resolved executable (not only provider name), argv, cwd, explicit environment result/unsets, stdio or PTY mode, startup terminal size, provider-declared execution/permission mode, prompt transport, optional raw-log paths, ownership scope, and temporary cleanup handles. Shared planner identity must be used by preview, log and actual launch. A launch=A/update=B case belongs to C3 updates but shares this same executable-resolution primitive.

Proposed additional requirements on foundation methods:

- `spawn(ProcessPlan, Cancellation) -> Result<ManagedProcess, SpawnError>`
- `ManagedProcess::read_output`, `write_all_input`, `resize`, `close`, `wait`, and `cancel` preserve one idempotent completion result
- `ManagedProcess::containment_status() -> ContainmentStatus` reports owned group/job attachment or explicit failure
- `ExitOutcome` distinguishes exit code, POSIX signal (Unix only), requested cancellation, timeout, and reader shutdown/truncation; spawn failure stays a separate error
- cancellation contains reason/deadline and process ownership token; it is not merely a dropped future
- bounded reader shutdown is available independently of process-tree containment so inherited output pipes cannot hang a caller forever

The baseline PTY abstraction is exactly Read/Write/Close/Wait/Resize. Unix closes with TERM then PTY master close then2s grace/KILL of the direct child. Windows closes ConPTY then2s grace/direct kill. Both Wait results are cached. Do not claim these prove group cleanup, detached-descendant cleanup or Windows ConPTY Close completion.

Headless callers need stdout/stderr readers running before prompt writes. Stdin is closed for stdin and argv modes. Parsing has no provider switch; unknown format rejects before launch. Emit is serialized CRLF text; stdout lines above1MiB are split and drained, never stop reading. Process exit code is authoritative even if parsed model output claims success/failure. Nonzero exit is a completed launch with error state, not SpawnError. Raw logs require explicit opt-in and private isolated paths.

### 7. Orchestration/admission and cancellation domains

Use one admission ledger across ordinary child spawn, pending human confirmation, relay start/resume and strong-role escalation.

Proposed interface:

- `AdmissionController::reserve(parent, slots, AdmissionKind, replace: Option<AdmissionId>) -> Result<AdmissionLease, LimitError>`
- `AdmissionLease::consume_registered(slots)` and `release()` are idempotent; Drop releases only owned unconsumed slots
- `SpawnService::prepare(AuthorizedSpawnRequest, AdmissionLease) -> Result<PreparedSpawn, SpawnError>`
- `SpawnService::dispatch(PreparedSpawn, Cancellation) -> Result<ChildSpawnResult, SpawnError>`
- `ConfirmationStore::register(parent, requested, resolved) -> Result<PendingConfirmation, LimitError>`
- `ConfirmationStore::decide(id, HumanDecision) -> DecisionResult`
- `ConfirmationStore::wait(id, HttpRequestCancellation) -> WaitOutcome` does not cancel the confirmation or reservation
- `OrchestrationService::parent_ended(parent)` expires confirmations/releases reservations without logging a user refusal

The live-session snapshot and reservation ledger must be checked in one atomic admission region. Never query counts, unlock, then reserve. Baseline automatic child cap defaults10 with maximum256; verified human confirmation/UI kinds use256 and skip autonomous aggregate limit. Automatic aggregate max is max(configured total, childlimit+1). Human UI origin is verified server-side capability/same-origin binding, not trusted JSON `origin:"ui"`.

`SameTree` and `RememberPermission` are tri-state. Decision effort/execution/permission are optional strings: None leaves requested value unchanged; Some("") clears it. AllowedTools and GrantFolderTrust are internal fields and cannot be accepted from conductor spawn JSON. Human confirmation may separately set folder trust. Preserve existing full-bypass defaults rather than silently changing them.

Keep three cancellation domains separate: HTTP waiter cancellation, user cancellation of a pending/active task, and Hub/process shutdown. A timed-out HTTP waiter cannot erase an unanswered human confirmation. Failed preparations/start, refusal, supersession, parent termination and terminal relay must release the corresponding reservations exactly once.

### 8. Relay, handoff, subagents and cross-owner events

Relay should keep pure state plus an injected effect interface equivalent to `relayDeps`: spawn child, input, append board, notify parent, git head/changed files, clock, persistence, publish completion. `RelayService::{start,stop,resume,cleanup,status,events,on_child_progress,on_child_exit,on_child_idle,on_child_timeout}` must carry orchestration ID and current child identity. Count DONE per unit and role; presence alone is not completion. Preserve verdict/evidence requirements, escalation and stale-exit rejection. Headless roles get one process per instruction and no nudge. Recovery of headless runs stops, not an invented still-running process.

Persist `relay.json` v1 with complete role selection, labels, live/progress IDs, counters/baselines, parent started/provider/cwd identity, evidence paths, worktree identity, timeline and timestamps. Reconnect grace120s differs from wrapper reconnect grace. Cleanup checks registered canonical worktree path and matching branch; preserve user's branch/unrelated work.

Handoff facade should accept only `handoff::Record` allowlist. JSONL version1 is per record; unknown kinds remain readable/writable. Text is masked and capped320 runes; files are capped20 plus summary. Transcript, note and workdoc fields hold paths only. No dynamic metadata or content/env/diff/raw-input fields. `HandoffFrom` is a predecessor link, not a parent. Every handoff/board/relay/prompt temp path must use RuntimePaths.

Subagent reader interface should be keyed by provider adapter (`subagent:claude-v1`, `codex-v1`, `grok-v1`) and take resolved parent path, turn cutoff, reader-owned typed continuation and budget; return optional tree plus next continuation. Default head64KiB/tail128KiB/max50 nodes; poll3s, resolve retry30s. Nil tree means no change, not erase. Reattach preserves reader state/cutoff but resets broadcast dedupe and timer generation. Proto SubagentNode is an allowlist; transcript contents are not added.

C3 needs typed core events for first/last user message, session state, completion, transcript growth/path, workflow state, git-turn capture, handoff update and subagent update. Final ownership of workflow_journal/workflow_scan/workflow_task_detail/done_summary/cross_session_message and the rest of `session` fields remains a freeze item. Do not silently omit them because they are not individually named in 02-core.

## Baseline discrepancies and intentional strengthening decisions

1. **Shutdown drain:**02-core asks for shutdown drain. Baseline `Store.Close` signals quit, waits at most6s, then closes DB. `asyncWriter` can stop with queued events present. `TestStoreEventAsyncDrainsQueue` waits for ordinary background progress before deferred Close; it does not prove shutdown drain. Recommendation: expose explicit shutdown policy/report, preserve compatibility stop/drop behavior until parent chooses bounded drain as an intentional strengthening, and add close-under-backlog tests. Never report current Go behavior as guaranteed drain.
2. **Failed Windows Job attachment:**headless Run ignores attachment failure, uses direct-child kill fallback, and waits for pipe readers before `cmd.Wait`; a descendant retaining pipes can keep it blocked forever. Existing whole-tree test covers successful containment, not injected failure. Required new acceptance must inject attachment failure, inherited pipes, cancellation and timeout, prove bounded return, and separately verify no surviving owned descendants. Forced pipe close alone is bounded return, not process cleanup proof.
3. **Output queue comment drift:**reattach_replay.go describes an old64-chunk queue. Actual wrapper constant is1024. Use source constant and current queue fixtures.
4. **Repository helpers default home:**existing config/handoff/prompt/hook helpers derive user paths. RuntimePaths injection is a deliberate trial-mode extension; production defaults must stay compatible and trial missing paths must fail closed.
5. **Instructions versus broad completion claims:**a parser, DB open/search, successful fake spawn, static guard, or compile is insufficient for the complete call-chain contracts. Each JSON row remains implementation/validation pending.

## Implementation ordering after freeze

1. Parent freezes IDs, all baseline proto/config shapes, RuntimePaths, ProcessPlan/ManagedProcess/ExitOutcome/Cancellation, and session/event/storage/action API names with C3 callers.
2. Independently implement pure VT/replay and SQLite schema/repository/writer/history in disjoint files. Port focused synthetic fixtures before wiring.
3. Implement approval identity/record/detector/policy and bounded transcript readers. Wire record effects through storage and proto, including first resolved poll/ledger/restart/next-poll regression.
4. Implement wrapper/process adapters, sequence-preserving ACK/reconnect, UI priming and resize ownership, registration/end/reset/dismiss lifecycle. Fake adapters first; isolated native acceptance separately.
5. Implement atomic admission and persistent confirmations; headless runner/parser; prompt readiness/delivery; board authorization/event queues; relay state/effects/persistence/worktree; typed handoff and all subagent readers.
6. Parent/C3 wire all existing routes and observers. Run paired Go/Rust synthetic fixtures, complete call-chain/UI checks, synthetic old/new DB rollback checks and native OS acceptance. Parent serializes dependencies/builds and records receipts.

## Explicit pending acceptance

- Every Rust implementation and actual Rust test
- Full source/assertion semantic review beyond the focused interfaces, including provider parser fixtures
- FTS available/unavailable, additive old-schema migration, replacement connection pragmas, rollback readability with the Go oracle
- Close under queue backlog and reset under queued/inflight writes
- Linux/macOS process groups, detached descendants, early exit, double close/wait, inherited output handles
- Windows version-specific ConPTY teardown, failed Job attachment, descendants holding pipes, exit classification and native timing
- Synthetic canonical-path/rename/symlink races and registered worktree cleanup
- Existing Web UI record/replay/input/resize behavior on actual session caller
- Copied-data backup/restore, operator real provider/native/remote acceptance and release artifact identity

No unmet acceptance is converted into success by a skip, stub, empty response, source scan or absent toolchain. This lane stops at inventory until the parent releases implementation ownership.
