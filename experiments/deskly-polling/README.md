# Offline Deskly polling experiment

This is an isolated synthetic experiment, not a Hub feature or a Deskly client.
Run from the repository root with Rust 1.90.0:

```powershell
rustc +1.90.0 --edition 2021 --test experiments/deskly-polling/mock.rs -o $env:TEMP/deskly-mock-test.exe
& $env:TEMP/deskly-mock-test.exe
```

The clock is simulated by explicit ticks. The 300 second value is only a default
in the fixture; no scheduler or configurable production setting exists. Work,
identity, approval, claim and ACK are synthetic, not authenticated. The reader
is a counter, not a GitHub client. The in-memory set is lost on restart and must
never be used as a production deduplication guarantee.

## Proposed Hub boundary

`rust/src/application/main_program/serve.rs` composes `SessionEngine`,
`OrchestrationProgram`, and `OrdinarySpawnService`, starts owned producers in
`run`, then stops and joins them before effect admission closes. A future opt-in
polling service belongs at this composition/lifecycle boundary, with injected
Deskly transport, durable recovery store, clock, local approval and a narrowly
typed read-only launch interface. It must stop and join with the Hub; no
polling loop belongs in `SessionEngine` or `child_launch`.

`SessionEngine` remains the owner of session admission and registration.
`OrdinarySpawnService::spawn` currently requires a verified confirmation and
owns task admission; a new adapter must preserve those checks, not fabricate a
`VerifiedConfirmationRequest`. Whether its interface can serve a locally
approved read-only request requires implementation review. `child_launch`
and `OrchestrationProgram` manage child roles and conductor/board lifecycle;
they are candidates only when a real parent/conductor exists. Do not bypass
their existing ownership to manufacture one. No connection is implemented.

## Restart and uncertain delivery proposal

Deskly remains the authoritative request, policy, authority and delivery state.
Discovery is advisory. Before any launch, verify registered identity, local
consent, approved read-only capability, current repo/PR head, expiry, generation
and authorization epoch. An atomic server-side one-time authority consumption
must bind the exact request, generation, identity and session attempt. The
server contract is not yet available; a client-side set or lease alone cannot
ensure exactly-once execution across devices or crashes.

Persist a minimal local recovery journal before the claim is sent, keyed by
request and generation and bound to identity and a unique attempt. Record
claim-sent, confirmed consumption, launch-intent and proven local session
identity with crash-safe ordering. Persisted values are recovery hints, not a
second authority; protect them and avoid storing request contents or secrets.
After restart, reconcile every nonterminal attempt with Deskly and inspect
reliable local session evidence before accepting another launch for that tuple.
If either side is unavailable, the journal is missing/corrupt, a response was
lost, or evidence cannot exclude a surviving process, hold for reconciliation.
Absence of a local session record is not proof no process was started. A timeout,
expired lease, disconnect, duplicate poll, or lost ACK must never itself permit
retry. Only a server decision backed by evidence that no prior start occurred
can authorize a fresh attempt; otherwise require manual resolution. Recheck
revocation and target SHA at the launch boundary. ACK proves receipt only, not
execution or completion; report verified results through Deskly's canonical
event path once its contract exists.

Crash-injection tests around every journal/claim/launch boundary, simultaneous
device claims, permission changes, stale SHA and restart recovery remain to be
designed and run. There is no implementation of this recovery proposal here.


## 2026-10-30 Deskly PR #132 discovery endpoint reconciliation

Deskly Draft PR #132 now contains an **unmerged, not deployed, default-off**
authenticated MCP read API `development_runtime_discover_approved_relay`.
This is **not** a standalone HTTP `poll` endpoint, nor a device-registration
mechanism. It is offered only through the existing machine MCP endpoint
(`POST /mcp/development-runtime`) with a valid registered orchestrator key
and PC relay opt-in. The arguments must be exactly `{}`.

The non-authoritative MCP tool content is:

```json
{
  "profile": "pc_relay_discovery_v1",
  "work": [
    {
      "request_id": "<Deskly UUID>",
      "expected_revision": 3,
      "generation": 1,
      "authorization_epoch": 1,
      "session_id": "<observed worker session>"
    }
  ],
  "truncated": false
}
```

The numeric fields are examples. The tool runs the **existing Deskly
PC-relay claim eligibility checks** over current `wired` work, but returns
neither a claim, a delivery receipt nor execution permission.

**Critical mismatch with `mock.rs`:** the experiment's synthetic `Work`
includes `device`, `repo`, `sha`, `deadline`, `approved`, and
`read_only`, whereas the current Deskly discovery result **does not supply
any of those fields**. Do not deserialize a real response directly into this
synthetic `Work`, invent default values, or treat advisory candidate metadata
as trusted GitHub PR read authorization.

Before any connection or session start, a separately reviewed and authenticated
resolution must supply the permitted repository/PR and immutable current HEAD
SHA, approved read-only scope, deadline, owner-local confirmation, and the
existing authority/attempt tuple. A Rust `SessionEngine` session must also
match the independently observed `session_id` expected by Deskly
`development_runtime_relay_claim`; the current mock has not established that
mapping. If any part is unavailable, **hold**. After genuine current-state
claim/handoff/validate and crash-safe local journal reconciliation are proven,
a read-only PR summary can be attempted as a separate authorized milestone.

Deskly PR #132 synthetic service/MCP tests **passed 9/9** at tested
commit `ddda878609cfbc6429582e25efd1d2f70080b782`:
[bounded Deskly CI evidence](https://github.com/ishizakahiroshi/deskly-private/actions/runs/40992856578).
This is not real provider polling, ACK, PR HEAD verification or execution.

The independent Rust `experiments/deskly-polling/discovery_v1.rs` now tests
separate authenticated read-only proof from advisory discovery, matching
revision/generation/authorization_epoch/session, explicit local consent,
stale repo/SHA, expiry, truncated batches, unknown consumption, duplicate
and two-client one-time authority simulation. It is **not** a production
client/decoder/Hub adapter. At commit `37ec0491515485562066e3e9271068d3029857f3`,
the bounded [Rust Actions run 40993174070](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/40993174070)
completed **3/3 existing mock and 7/7 typed contract tests**. No
automated Rust process launch, live polling, deploy or merge is authorized.

## 2026-10-30 Hub-side intake module (default off, not composed)

`rust/src/application/deskly_intake.rs` and its submodules move the offline
contract into the Rust crate. Nothing calls it from `serve`, so the shipped
Hub behaves exactly as before. Pieces:

- `mcp.rs`: stateless JSON-RPC `tools/call` to Deskly's existing
  `POST /mcp/development-runtime` with `Authorization: Bearer <registered
  orchestrator key>`. Endpoint rule copied from Deskly `targetUrl` (HTTPS, or
  literal-loopback HTTP; exact path; no userinfo/query/fragment). Redirects are
  not followed, bodies are bounded, the key's `Debug` is redacted. Reply is
  plain JSON or one SSE `data:` frame; `isError` becomes a refusal. Failure
  classes: `Unauthorized` / `Rejected` / `NotDelivered` (connect failed) /
  `Unknown` (may have reached the server). For a mutating step, `Unknown` and
  an "ok" reply whose shape we refuse both mean *outcome unknown*.
- `wire.rs`: closed (`deny_unknown_fields`) decoders for discovery v1, claim /
  handoff previews (`preview_token`), claim apply, handoff apply
  (`issued` / `spent`) and `pc_relay_issued_validation_v1`, following the
  shapes and checks in Deskly PR #132 `agent-development-pc-relay-tools.ts`
  and `agent-development-pc-relay-model.ts`. The Slack wire `text` is dropped
  at decode time and never parsed into a command.
- `github.rs`: one unauthenticated `GET /repos/{o}/{r}/pulls/{n}` to read the
  live HEAD SHA, state and base repository. Read-only by construction; private
  repositories report `NotVisible`.
- `journal.rs`: SQLite (`WAL`, `synchronous=FULL`) row per
  `(request_id, generation)`. Holds only ids, revision, epoch, session id,
  repo/PR/SHA and state; no wire text, preview token or key. Opening it is
  recovery: any in-flight row (`approved` .. `launch_sent`) becomes `held`.
  It never authorizes or retries; Deskly stays canonical.
- `launch.rs`: launch goes through `OrdinarySpawnService::spawn`, so
  `SessionEngine` stays the only admission/registration owner. It requires the
  owner's `VerifiedConfirmationRequest` and checks its auth epoch against
  `SessionEngine::auth_epoch()`. Only `claude` (`permission_mode=plan`) and
  `codex` (`sandbox=read-only`, `ask_for_approval=untrusted`) are offered;
  `client_request_id = deskly-<attempt>` correlates the registration with the
  journal row. The prompt names only repo / PR / SHA.

Order in `IntakeCoordinator`:

1. `poll_once`: discovery (advisory). Truncated or failed reads do nothing.
   Each new candidate needs a bound `ApprovedTarget` from `TargetSource`
   whose request/revision/epoch match discovery, scope exactly `["read"]`,
   not expired, repository in the owner's local allowlist, and a live PR
   that is open in that repository at exactly the bound SHA. Only then is
   an `owner_pending` row written. Poll never calls claim.
2. `approve`: the owner's approval names the SHA they saw; it must match
   the row, and the Hub auth epoch must be current. Target and live HEAD
   are re-checked (TOCTOU). Then claim preview -> durable `claim_sent` ->
   claim apply -> handoff preview (confirmation session must match) ->
   durable `handoff_sent` -> handoff apply with `client_attempt_id` =
   journal attempt -> issued tuple check -> `validate_issued` ->
   durable `launch_sent` -> launch.
3. Any unknown reply, `spent`, refusal after the claim, or launch error ends
   in `held` with a reason. Nothing in the module ever leaves `held`.

### Owner decisions (2026-10-30)

- Q1 binding: Deskly fixes repository, PR number, HEAD SHA and read-only
  scope at approval time and returns them over MCP. Deskly defines the shape;
  until it exists `NoTargetContract` stays the production source.
- Q2 authority: Deskly adds a separate "read PR only" work kind instead of
  reusing the Slack worker handoff.
- Q3 caller: the Hub calls claim/handoff for that work kind; the Node PC relay
  receiver does not handle it.
- Q4 key: Windows Credential Manager generic credential
  `many-ai-cli/deskly-orchestrator-key` (`key_store.rs`).
- Q5 GitHub: no credential; public repositories only. Private PRs stop as
  `NotVisible`.

The list below is kept as the original record of what was open.

### Not decided / not implemented (do not guess)

- **Target binding**: Deskly has no authenticated contract binding a stage-4
  request to a repository, PR number, immutable HEAD SHA and read-only scope.
  The production `TargetSource` (`NoTargetContract`) therefore always refuses,
  and the coordinator records nothing. The handoff wire is a Slack worker
  message (`transport=slack_auto`, `kind=code`), not a PR-read instruction.
- **Which process owns claim/handoff**: the existing Node PC relay receiver
  also calls `relay_claim` / `relay_handoff` and keeps its own pending
  journal. Running both for the same orchestrator key would make two owners.
  Whether the Hub replaces the receiver for this work kind, or calls into it,
  is undecided.
- **Credentials**: where the orchestrator key is stored on the PC, and which
  GitHub credential (if any) may read private repositories, are undecided.
  Reading token scopes to prove a GitHub credential is read-only is not
  implemented; the reader sends no credential.
- **Owner approval UI / route and `serve` composition**: no HTTP route,
  scheduler (300 s default is only a constant) or UI exists yet.
- **Result evidence and reconciliation**: no result report to Deskly; `held`
  rows need a manual reconciliation procedure that does not exist yet.
- **Decline**: local only; no Deskly call for an owner decline is known.
