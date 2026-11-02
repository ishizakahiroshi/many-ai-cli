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
there is no MCP grant-creation or approval tool. No production grant/bootstrap
or real provider launch adapter is installed by this initial composition.

The tools are `capabilities`, `executor_start`, `executor_poll`, `executor_stop`
and `executor_request`. Attempt operations carry a complete `binding` object;
start/stop additionally carry a `request_id`. Request lookup takes `request_id`
under the authenticated principal/workspace. It returns an immutable original
request receipt separately from the current state. Same request and content
replay the original receipt; changed content conflicts. Missing data never
authorizes a replacement attempt. A new service lifetime marks unfinished work
unknown and does not replay an outbox. Explicit restore inspection uses a hold;
arbitrary filesystem rollback cannot be detected from an old DB alone.

## Stop and result limits

The exact-session bridge can request cancellation only for a previously proven
SessionBinding, fenced attempt and executor instance. Its receipts are local to
that process and remain `stop_unconfirmed`, with `owned_execution_ceased: false`.
It never uses kill-all. Socket closure, missing session, wrapper exit, Done text
and exit code zero do not become process-tree stop or successful-task evidence.

Isolated tests inject an explicitly synthetic backend and synthetic hash proofs.
Those proofs are accepted only for the synthetic provider/profile and cannot
complete a real provider attempt. Production still needs durable receipts from
the actual PTY/process owner, bounded process-tree/remote-job stop verification,
and independently verified task artifacts before execution can be enabled.
