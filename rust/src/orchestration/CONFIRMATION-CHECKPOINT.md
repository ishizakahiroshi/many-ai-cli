# Confirmation ownership checkpoint

Updated: 2026-10-03 13:01 UTC. Shared contract and bounded core implementation
checkpoint. Fixed Go source: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`.

The actual `SessionEngine` now implements confirmation registration, replacement,
waiters, accepted-task ownership, pending UI replay, and pending shutdown. The
optional `ConfirmationExecutor` must supply a fallible live presentation snapshot
and real preparation/board operations; its absence rejects registration before
reservation or publication. No production executor or successful child launch is
provided by this checkpoint. Scoped receipts and remaining caller gaps are in
`rust/tests/fixtures/core/orchestration/traceability.json`.

## Shared API

Keep `SpawnConfirmations` separate from `SessionCore`. The same `SessionEngine`
implements both; its existing state and admission ledger remain the only owners.
All new clocks use `proto::time::Timestamp`, never OS `SystemTime` arithmetic.
The following internal types have no serde implementation and carry no wire grants.

```rust
pub struct ConfirmationRequest {
    pub parent: LiveSessionId,
    pub requested_provider: String,
    pub body: ResolvedChildSpawn,
    pub requested_at: Timestamp,
}
pub struct ConfirmationRegistration {
    pub pending: PendingSpawnConfirmation,
    pub effects: CoreEffects,
    pub waiter: Box<dyn ConfirmationWaiter>,
}
pub trait ConfirmationWaiter: Send {
    fn id(&self) -> &SpawnConfirmationId;
    fn wait(
        self: Box<Self>,
        cancel: HttpWaitCancellation,
    ) -> CoreFuture<'static, ConfirmationWaitOutcome>;
}
pub trait AcceptedSpawnDecision: Send {
    fn id(&self) -> &SpawnConfirmationId;
    // Performs the synchronous handoff transaction before returning the future.
    fn run(
        self: Box<Self>,
        cancel: TaskCancellation,
    ) -> Result<CoreFuture<'static, ConfirmationOutcome>, SessionError>;
}
pub trait SpawnConfirmations: Send + Sync {
    fn pending(&self) -> Vec<PendingSpawnConfirmation>;
    fn register(
        &self,
        request: ConfirmationRequest,
    ) -> Result<ConfirmationRegistration, AdmissionError>;
    fn accept_decision(
        self: Arc<Self>,
        response: SpawnConfirmationResponse,
        origin: VerifiedConfirmationRequest,
        now: Timestamp,
    ) -> Result<Box<dyn AcceptedSpawnDecision>, SessionError>;
    fn parent_ended(&self, parent: LiveSessionId) -> CoreEffects;
}
```

These replace `register(PendingSpawnConfirmation)`, `wait(id)`, and asynchronous
`decide`. Callers must not invent the confirmation ID, admission ID,
waiter state, or a final launch outcome. `register` allocates the ID and reserves
one child atomically. It returns unapplied effects. The registration waiter already
owns the completion handle, so a fast decision cannot disappear before `wait` starts.

The crate-visible admission entry point
`AdmissionState::reserve_confirmation(parent, slots, replace)` returning
`Result<AdmissionReservation, AdmissionError>` uses the existing ledger and
the source confirmation capacity lane: 256 children per parent, no autonomous
aggregate cap. Do not fabricate `VerifiedUiOrigin` or a human decision to reserve
this pre-decision capacity. A failed replacement preserves the old pending entry
and its reservation. This lane does not need caller-supplied `AdmissionLimits`.

## Lifetime and handoff

One per-confirmation completion cell is retained by the pending entry, waiter,
and accepted task as necessary. It contains the result/signal and waiter-gone
state, not another session map. Remove finished entries from the pending map;
there is no permanent outcome lookup map. Use the core state lock for decision,
replacement, parent-lifetime, and admission transitions. No filesystem, process,
socket, or arbitrary callback runs under that lock.

1. Registration creates the pending entry and waiter together. Same-parent/role
   replacement completes the old waiter as `Superseded`, releases its reservation,
   and orders the old closed frame before the new requested frame.
2. `accept_decision` applies approved overrides and validates launch options before
   reserving the decision. Invalid approval leaves the pending request unchanged;
   refusal ignores invalid launch overrides. The accepted object owns a unique
   decision lease. Until handoff the entry remains pending, including the dismiss
   guard. A duplicate decision cannot acquire the same lease.
3. Dropping that object before `run` clears only its still-current decision lease.
   It never resurrects an entry already superseded or expired. The original
   reservation and request remain available for retry when still current.
4. The Hub task owner obtains an owned task permit before `run`. Calling `run`
   synchronously revalidates the lease and parent lifecycle, marks the decision,
   removes the pending entry, and transfers approved admission to the returned
   task. Refusal releases admission at this point. A replacement/expiry that won
   first causes an error before HTTP success and cannot launch a child.
5. The permit must accept the returned future without a subsequent fallible
   enqueue. Construction and transfer to the task owner happen in the same poll,
   with no intervening await. Only then may C3 return `{ok:true}`. The task owner
   stays accepting until admitted HTTP operations have drained. This boundary replaces
   Go's immediate response followed by `safeGo`; launching remains asynchronous.
   If the existing task owner cannot provide this guarantee, it needs an explicit
   rollback-capable handoff interface before the route can be advertised.
6. The owned task performs one refusal or one real child preparation/launch,
   completes the waiter once, then applies the source-ordered closed/board
   effects outside locks. No HTTP waiter performs the launch. Partial effect
   application is reported without replaying the whole batch.

Waiter cancellation or drop marks `waiter_gone` only while undecided and returns
`WaiterCancelled`; the pending request/admission remains. Cancellation after the
handoff does not retroactively set waiter-gone, matching Go's `!Decided` check.
The Hub task continues independently. An ordinary request disconnect must never
cancel that task or cause a second launch.

Task shutdown must signal `TaskCancellation` and drain the task. Dropping the
committed future, even before its first poll, must synchronously release its
owned admission and publish a failure to the
completion cell, but cannot claim asynchronous board/broadcast cleanup happened;
it must report that incomplete cleanup and never restore an accepted decision.
`SessionEngine::shutdown_confirmations` completes pending waiters as `HubStopped`
and releases pending admission without generating a new wire close reason. It
does not cancel or release committed tasks owned elsewhere. The real Hub must
call it in the drain sequence above; the owned task permit and HTTP handoff are
still pending composition.

## Permission and actual launch boundary

The request body is already server-bound `ResolvedChildSpawn`. Raw JSON origin,
labels, and grant-like fields confer no authority. C3 normalizes/binds origin
before constructing it: Go downgrades an unverified `ui` claim to autonomous and
drops permission-memory intent; unknown nonempty origins are invalid. Applying a
verified human confirmation permits explicit overrides/folder trust but retains
autonomous launch defaults. Only a direct verified UI request uses UI defaults.

Reuse `child_options::prepare` and the existing config validation after applying
the human choice. Do not replace the choice with a later unvalidated composition.
The real Hub child executor still must own provider/model validation, depth and
role checks, board/worktree preparation and recovery, actual launch plan, initial
prompt route/gate, and post-success memory updates. The launcher UI's
`ConnectionManager` is not this executor. No success stub may stand in for it.

At dispatch, place child metadata and admission correlation into the existing
one-use registration-proof/pending-attempt owner before launch. A label-only
lookup or a second C3 child-session map is insufficient. Registration must install
parent/role/board metadata and the actual prompt route before ACK. Windows shim
usability and headless/interactive routing belong to the resolved launch plan.
The executor's exact interface and injection remain integration-owner coordination.

## Source evidence and first bounded regressions

`internal/hub/orchestration.go` at the fixed commit:

- 476–512: confirmation admission uses the human capacity lane before decision.
- 634–704: checked parent, collision-resistant sequence, same-role replacement,
  reservation transaction, superseded/requested broadcast order.
- 706–722: waiter cancellation changes only undecided waiter-gone state.
- 724–767: oldest-first pending snapshot and pending-confirmation dismiss guard.
  Equal-timestamp order is unspecified; do not assert Go map tie ordering.
- 772–799: parent expiry releases admission and closes as `parent_gone`, never
  `user_refusal`.
- 803–868: approved overrides validated before deciding, immediate HTTP success,
  one background resolver.
- 878–935: sole refusal/launch owner, exactly one outcome, admission release,
  `refused`/`approved`/`spawn_failed` close reasons, waiter-gone board handling.
- 1180–1280 and 1348 onward: preparation/dispatch/registration and actual prompt
  routing are required callers, not covered by the confirmation state alone.
- 1440–1470: server-bound origin; 4428–4465: human override field semantics.

The bounded implementation tests cover registration/admission atomicity,
same-role replacement failure and ID exhaustion, fast completion before wait,
waiter cancellation before/after handoff, invalid approval retry, refusal ignoring
invalid options, decision-lease drop, duplicate decisions, replacement/parent-end
races, and zero launches before an accepted task handoff. Next exercise real
preparation failure, exactly one successful synthetic launch, admission transfer,
nonce-independent human provenance, and task shutdown/cleanup. Use injected
logical clocks and owned synthetic roots/helpers. The traceability receipt owns
exact test counts; these checks do not close the remaining production orchestration
or native acceptance gaps.

## HTTP proof boundary correction applied to shared types

Source inspection found that the earlier shared `VerifiedUiOrigin(UiBinding)`
required an active WebSocket member, while these Go HTTP callers do not. The
shared types and core admission checks now use the two epoch-bearing purposes:

- Direct `origin: ui` uses the existing signed ui_origin cookie,
  Sec-Fetch-Site=same-origin and allowed nonempty Origin
  (`orchestration.go:1076–1093`), after the ordinary request guard. A WebSocket
  connection is not a prerequisite. The unbound UI claim is downgraded to the
  autonomous/conductor origin and its permission-memory intent is discarded.
- The confirmation decision route calls the ordinary guard, validates the
  response and resolves the pending ID (`orchestration.go:803–868`). It does not
  call the direct-origin cookie-capability helper. That ordinary guard includes
  token, method, Host, Origin and applicable remote PIN
  (`http_helpers.go:435–454` and `pin_auth.go`). This is the baseline authenticated
  single-user boundary, not a cryptographic proof of a person's presence.
- Use distinct opaque server-created proof purposes for the browser-origin
  claim and the authenticated confirmation decision. Neither comes from JSON,
  and neither should require inventing a WebSocket connection ID. Do not treat
  the confirmation proof as a direct-UI-origin proof for a fresh request.
- Existing accepted-request/auth-revocation lifetime must be kept explicit at
  the HTTP owner; an already accepted asynchronous task must not be tied to a
  subsequently closed browser socket. Do not add a new capability/PIN scheme:
  migration instruction `03-services.md:33` requires the existing trust model.

`VerifiedUiOrigin` retains the ordinary request's authentication epoch after the
existing browser-origin checks. `VerifiedConfirmationRequest` retains the epoch
after the ordinary confirmation route guard. Core tests exercise acceptance with
no active socket and stale-epoch rejection. Actual HTTP proof creation, route
status mapping, task-owner handoff, and production executor wiring remain separate
caller work; no new authentication architecture is introduced here.
