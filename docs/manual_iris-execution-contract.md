---
type: reference
status: draft
tags: [many-iris, execution-contract, mcp]
owner: maintainers
review_status: draft
related: [../README.md, ../README.ja.md, ../CHANGELOG.md]
last_reviewed: 2026-11-03
---

# Many Iris execution contract

This opt-in boundary separates durable execution admission from HTTP delivery,
ordinary session spawning, and process termination. It uses the MCP 2026-07-28
profile at `POST /api/iris-execution/mcp` on the selected loopback Hub port.

The initial production composition is **inspection-only**. It reports
`live_provider_execution: false`, `execution_ready: false`,
`fenced_stop_proof: false` and `result_sha256: false`. Direct `executor_start`
calls are rejected. The existing spawn/input routes retain their behavior.

## Opt-in inspection

Under the selected runtime root (normally `~/.many-ai-cli`), the operator can
create `iris-execution/identity.json` with the following public identifiers:

```json
{"principal_id":"iris-operator","workspace_id":"example-workspace","execution_host_id":"example-host"}
```

`iris-execution/credential` must contain a separately generated random 256-bit
credential encoded as exactly 64 hexadecimal characters, without a newline.
Protect that file and the directory for the owning OS user. Do not reuse the
Hub token, Iris UI key, Iris MCP token, or provider credentials. Creating or
rotating these files is an operator action; apply changes by restarting the Hub.
Never put credentials in a URL, command-line argument, log, or checked-in file.

If identity is absent, this route returns 503 and creates no execution state.
Malformed opt-in configuration fails startup rather than silently resetting it.
The configured boundary requires one `Authorization: Bearer` header, an exact
loopback Host, a loopback peer, no query string, and a matching Origin if supplied.
Cookie and trusted-network authentication do not apply. The token is loaded at
startup, so deleting the file alone does not revoke a running instance.

Every request needs the current MCP metadata and mirrored protocol/method/name
headers. Discovery, tools/list and the capabilities tool send no provider work.
Requests have a 64 KiB body limit and reject duplicate JSON keys and trailing data.

## Durable records and permission

`journal.sqlite3` is separate from session history and its retention policy.
Admission uses a synchronous FULL SQLite transaction before any backend action.
Bindings include principal/workspace, plan/task and instruction revisions/hashes,
the independently pinned grant revision/scope, host/instance, attempt/fence, and
provider/profile. Grants are registered only by trusted local application code;
there is no MCP grant-creation or approval tool. A fixed local operator manifest
can register pinned grants. The production launch adapter remains disabled.

The tools are `capabilities`, `executor_start`, `executor_poll`, `executor_stop`
and `executor_request`. Attempt operations carry a complete `binding` object;
start/stop additionally carry a `request_id`. Request lookup takes `request_id`
under the authenticated principal/workspace. It returns an immutable original
request receipt separately from the current state. Same request and content
replay the original receipt; changed content conflicts. Missing data never
authorizes a replacement attempt. A new service lifetime marks unfinished work
unknown and does not replay an outbox. Explicit restore inspection uses a hold;
arbitrary filesystem rollback cannot be detected from an old DB alone.

`executor_evidence` is a separately authenticated, read-only tool with a complete
`binding`. It returns persisted owner observations even when execution admission
is disabled. It cannot select a database path, issue a grant, append evidence or
start a process. Iris can inspect these records without downgrading its start
capability checks.

## Local operator grants

The fixed runtime-root file `iris-execution-approved-grants.json` has
`schema_version: 1`, a positive `manifest_revision`, `approval_state: "approved"`
and a `grants` array. Each entry contains the complete `binding`, `expires_at`,
`revoked`, an absolute `approved_cwd`, relative `instruction_path` and
`result_path`, `max_artifact_bytes`, `effect_scope` and `allow_remote_jobs`.
Instruction and result paths resolve only beneath `iris-execution-artifacts`.
The instruction content must match the binding's SHA256. These are explicit
operator files; normal MCP callers cannot create or modify them.

`iris-execution-grant-trust.sqlite3` retains manifest and per-task permission
revisions separately from the manifest. Downgrades, same-version changes and
implicit removal of active permissions are rejected. Revoke the old entry
explicitly; deleting an entry does not silently authorize a different scope.
Unreadable or invalid authority disables admission. Restoring an old manifest
and trust database is not a method of re-enabling old permissions.

## Native owner records

For an explicitly core-issued Iris PTY launch, the wrapper verifies current
operator approval, a read-only snapshot of the Hub admission journal, the actual
prepared instruction content and the working directory. It commits a one-use
owner spawn claim before native creation. A duplicate claim, including the same
ticket after a restart, cannot launch a replacement. Internal owner bootstrap
metadata is removed before provider exec, including launch payloads. Custom
argv, headless and login launches are not enabled for this owner path.

The wrapper keeps its owner observer independently of the Hub socket and saves
records in `iris-execution/owner.sqlite3` before the final session frame. Windows
uses retained native process and Job handles: checked termination must succeed,
the root handle must be signaled and the owned Job's active process count must
reach zero within the deadline. Unsupported platforms and failed observations
produce no cessation proof. No process is rediscovered by PID or label.

This proof means `local_tree_ceased: true` for `windows-owned-job`. It does not
mean `owned_execution_ceased`, `remote_jobs_stopped`, `effects_verified` or
`task_outcome_verified`. Natural exit is `local_exit`; a requested stop is bound
to its persisted request, owner epoch, attempt and fence. A later stop request
cannot reuse an older natural-exit observation.

Result artifacts must be regular, single-link files within the approved root.
The verifier bounds their size, rejects aliases and traversal, hashes their
bytes independently and checks the full instruction/owner/attempt binding.
The JSON result document contains `binding`, `owner_id`, `owner_epoch`,
`instruction_hash`, `outcome`, `effects` and `synthetic`. A valid artifact is
saved as a report with `verification_scope: "artifact-integrity-only"`.
`verified_task_outcome` remains `unknown`: a provider's success statement and
the hash of that statement are not independent business acceptance.

## Trusted internal dispatch lane

The internal `OwnedDispatcher` composes the existing journal, fixed operator
grant loader, owner store and `CoreLaunchPort`. It is not installed as the HTTP
backend. The production endpoint remains inspection-only, and no live execution
or automatic handoff capability is enabled by these components.

New journals use internal SQLite schema 2 with a separate `dispatch_claims`
ledger and indexed audit lookups. A claim consumes one dispatch opportunity in
the same transaction that rechecks current approval, boot, attempt and the
original start receipt. Read queries cannot recreate the private, non-cloneable
queue token. A crash after acceptance or claim does not authorize a second
enqueue. Existing schema 1 journals keep their previous query/start/stop
semantics; the new claim API rejects them. The coordinator rejects schema 1
before creating a new start intent, and rejects restored/held journals as well.
Normal startup does not migrate an
existing journal. Older binaries reject schema 2, and the HTTP wire contract
remains `iris-execution/1`.

The bounded queue returns original receipts on replay even when full. Start
and stop receipt replays never call the launch/effect port again. The
worker rechecks the fixed grant and instruction bytes, then calls the existing
Core spawn path with no preissued proof, elevated grants or automatic risk
confirmation. A registration ACK does not mean the provider root has started.
The journal stays accepted until the exact owner ticket, session, root and
current Core binding are observed. A saved stop intent wins over publishing
running. Stops use the owner-bound SessionBridge and never PID/label lookup.

Worker failure, cancellation and shutdown preserve ambiguous queued or active
work as unknown when storage is available; claims are never released for retry.
Shutdown checks the current cancellation value as well as notifications and
uses a bounded join with an explicit failure fallback. Unknown-state cleanup
uses a bounded, separate blocking lane so mutex or SQLite contention cannot
block its Tokio cleanup deadline. Pending or failed cleanup returns an error;
the durable claim remains consumed and journal restart still reconciles unknown
work. This does not impose a real-time guarantee on the ordinary synchronous
submit/query/grant operations. Stop API receipts remain
durable intent when delivery fails or times out. They are not local cessation,
remote-job, task-outcome or effects verification.

## Independent fixed-payload records

The trusted Rust `VerificationStore` is a separate, opt-in local library. It is
not wired into HTTP, the scheduler or the operator grant manifest. It does not
grant execution permission. Its schema-1 SQLite database stores immutable
policies and original comparison receipts with FULL transactions. The owner
database writer lock spans each new policy commit, so another connection cannot
begin the attempt between the absence check and the durable pin.

An operator-owned policy uses schema_version 1, a policy_id and positive
policy_revision, verifier `strict-json-payload-equality/1`, the complete binding,
expires_at_ms and expected_payload. Policies are at most 64 KiB, reject duplicate
keys and unknown envelope fields, and use the shared depth limit. The raw policy
SHA256 identifies the exact specification. The policy is immutable per complete
binding; a different policy requires a separately authorized binding. There is
no command, URL or expression evaluator. Payload strings remain data.

Result artifacts may include an optional `payload` beside the existing envelope.
The owner receipt still verifies integrity only. The independent comparison
requires that owner's exact ticket, observed root and integrity receipt, then
reads the approved artifact with the permit and hashes the same bytes it parses.
Only equality to the pre-pinned expected_payload determines pass or fail.
Provider outcome/effects fields do not determine that verdict. Malformed,
missing, changed, expired or mismatched input returns a fixed error, representing
an unavailable comparison; it does not create a success receipt.

Receipts include the complete binding, owner ID/epoch, input SHA256, verifier and
policy identity, spec hash, original observation time and fixed-payload-only
scope. Neither expected nor actual payload is included. task_outcome_verified,
effects_verified and remote_jobs_stopped stay false for pass and fail alike.
The owner's verified_task_outcome remains unknown. In particular, fail does not
authorize retry/handoff, and pass does not satisfy global task acceptance.

Exact pin/verification replays return the original historical receipt, including
after expiry or artifact removal. They do not certify that the file or approval
is currently valid. Changed input or ticket conflicts. Explicit restored mode
persists a write hold: pin and verify, including replays, are refused while query
can inspect old receipts. Startup and queries validate hashes, binding and
receipt semantics, including the original observation's policy time window.
These are corruption checks, not authenticity against rewriting the entire
database or undetectable filesystem rollback. Independent effects acceptance
remains subsequent work; the following profile adds isolated HTTP composition.

## Explicit owned HTTP acceptance profile

Trusted local composition can inject `OwnedProfile` on the same HTTP endpoint.
It must share the exact journal and owner `Arc` objects with the HTTP owner;
another journal/owner instance is rejected. Production `from_local` remains
inspection-only and does not inject a provider launch port.

The separate profile is `iris-owned-dispatch/1`. Its tools are `owned_start`,
`owned_poll`, `owned_stop`, `owned_request` and `owned_evidence`; every call must
include the exact `profile`. Start/stop require full binding and request ID;
poll/evidence require full binding; request lookup requires the original request
ID and derives binding from the authenticated principal/workspace record. Caller
arguments cannot select cwd, prompt, verification policy or expected payload.
The legacy synthetic `execution_ready` and proof gates retain their meaning.

The fixed runtime-root source is `iris-execution/owned-policies.json`: strict
schema-1 JSON with `profile`, boolean `enabled` and `policies` containing existing
fixed-payload policy documents. It rejects aliases, duplicate/unknown fields,
oversize input and duplicate bindings. First acceptance supports only synthetic
provider/profile. Admission pins the exact policy before dispatcher submission;
queued launch rechecks enablement, expiry, binding, pinned spec hash and the
verification-store restore hold on a bounded blocking lane. Historical pin and
verification reads never renew launch authority.

The HTTP owner delegates start once to the dispatcher. Exact historical request
replay precedes fresh policy/instance/queue checks; uncertainty uses lookup.
Stop awaits after releasing journal/owner locks. Capabilities and synchronous
store inspection run off the async timer lane. `owned_request` returns separate
`profile`, `binding`, original `request`, native `execution` and historical
`verification` fields. A missing verification is null; other storage errors fail
closed. These observations never claim task success, global effects or remote
job cessation. Restored stores permit history inspection but refuse new work.

## Native stop and result limits

The exact-session bridge can request cancellation only for a previously proven
SessionBinding, fenced attempt and executor instance. Its receipts are local to
that process and remain `stop_unconfirmed`, with `owned_execution_ceased: false`.
It never uses kill-all. Socket closure, missing session, wrapper exit, Done text
and exit code zero do not become process-tree stop or successful-task evidence.

Isolated tests inject an explicitly synthetic backend and synthetic hash proofs.
Those proofs are accepted only for the synthetic provider/profile and cannot
complete a real provider attempt. The native owner path now persists Windows
local-process observations and artifact-integrity reports. Real provider
dispatch, remote-job cessation and independent task/effects acceptance still
need an adopted scope and verification policy before execution can be enabled.
