# Rust/Web spawn correlation contract

Candidate for #P-20261008-007 requirement 11. Final source review, published SHA and
product acceptance are separate gates; this document is not an acceptance receipt.

## Additive wire contract

Authenticated Rust ordinary and grid spawn requests accept optional
`client_request_id`. Omission or the empty string preserves legacy behavior.
Nonempty values must be at most 128 ASCII bytes and contain only letters, digits,
`_` or `-`. Explicit request `GoWire::SCHEMAS` include this field. Validation occurs
before planning/launch side effects. The value grants no authority and is not an
idempotency key; retrying a request may launch additional sessions.

The server carries this value in its own `SpawnRegistrationMetadata`, bound to a
trusted registration proof. Arbitrary wrapper registration input does not confer
correlation ownership. Labels, tab names, authentication, proof IDs, attempt IDs
and lease information are neither repurposed nor exposed.

After registration, the Rust core emits the normal initial `session_update`, then
a typed `session_spawn_correlated` event through the same ordered effects/UI queue:

- `type`: `session_spawn_correlated`
- `hub_instance`: the current Hub lifecycle identifier
- `session_id`: the actual registered session ID
- `started_at`: that actual lifecycle's start time
- `client_request_id`: the authenticated request's bounded value

The typed variant follows normal route, priming, queued-approval retention, drain
and live best-effort socket delivery. It does not bypass the queue with a separate
HTTP or WS broadcast. Reconnect snapshots include an optional omitted-when-empty
`client_request_id` field on the Rust-owned `SessionSnapshot`. Warm reattach keeps
the value and original start time; usage probes do not publish the event.
`proto/generated.rs`, Go code and Go oracle fixtures are unchanged.

Old clients can omit the request field and ignore the additive event/snapshot
field. Against an older Hub that does not publish correlation, the new Web keeps
sessions visible in the default tab and does not guess a launch or grid identity.

## Browser correlation and membership

The browser creates a cryptographically random request value and captures the
selected sidebar tab at the first launch click, before risk/model waits or saving
preferences. The derive risk-confirmation retry retains that same request/tab.
Nested grid launches retain the originating click's tab. Request IDs and tab IDs
are not saved in server spawn defaults.

Both the explicit live event and snapshot optional field enter one bounded
tracker. The full live session map is built before family resolution. Matching
requires this page session's pending request, the current Hub instance and an
exact `(id, started_at)` lifecycle. Another browser/external launch cannot consume
an unrelated pending request. A missing ancestor defers assignment; a child
inherits the actual root. A surviving explicit root membership always wins,
including a manual move made before the delayed correlation arrived.

A deleted captured destination falls back to the surviving default tab. Tab
switching while the request is pending does not retarget it. All accepted grid
siblings retain the same request association; the first match does not consume
the entire request. Detached grids open only confirmed matching live IDs, never
all IDs above a previous maximum.

## Cancellation, loss and bounds

HTTP failure, observer cancellation or a later grid start failing does not prove
that earlier starts were rolled back. Therefore pending correlation survives
those outcomes. Live events before HTTP responses are accepted; a reconnect
snapshot can recover a lost event or response. A grid waits up to 15 seconds for
its expected matching sessions, then can show confirmed partial results with a
warning. Later accepted starts can still receive their correct tab even after
the view-opening wait expires. Ordinary/derive HTTP failures retain membership
tracking but do not claim HTTP success or automatically retry the launch.

Pending records use origin-scoped `sessionStorage`, separate from the sidebar's
`localStorage` configuration and synchronized preferences. Only request/tab IDs,
Hub scope, creation time, expected count and matched lifecycle identities are
stored; no conversation, complete session snapshot or runtime status is copied.
There are at most 128 requests, 18 matched lifecycles per request, and a 30-minute
TTL. Completed records stay until expiry to deduplicate replay and preserve
manual moves. Capacity refuses a new tracked launch rather than evicting accepted
work. Expired/unrecoverable launches remain visible via normal default assignment.
Closing the page session can discard pending attribution. Storage failure keeps
current-page memory tracking and warns that reload recovery is unavailable.

A persisted Hub boundary is checked on the FIRST snapshot after page reload, not
only against a fresh in-memory WS variable. A different Hub instance discards
old scoped pending work. Requests created while disconnected bind to the next
actual snapshot; disconnected live-view matches are cleared. Tabs themselves
remain governed by the existing lifecycle-aware sidebar state.

## Focused verification

Rust fixtures cover absent/empty/invalid input and explicit schemas, ordinary
cancellation, orchestration metadata, accepted grid siblings after partial
failure, trusted registration versus forged wrapper input, ordered priming/live
JSON, snapshot/warm retention and queued approval filtering.

Web fixtures cover bounded normalization, simultaneous requests, external launches,
ID reuse, first-snapshot Hub changes, WS-before-map/response, reload/manual replay,
all grid siblings/partial HTTP failure, first-click/risk/nested-grid callers,
preference isolation, full-map family resolution, storage failure and existing
actual tab/card drag listeners. Pure fixtures are explicitly registered in
`web/package.json`; caller/listener tests live under `web/tests/`.

Rollback removes the additive Rust/Web feature together. Existing tab state may
remain; clearing `many-ai-cli-pending-spawns-v1` only removes pending attribution,
not sessions, processes, conversation history or tab organization.
