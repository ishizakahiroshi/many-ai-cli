# Approval-action observations

Fixed Go behavior oracle: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`.
The existing Go and Web source stays unchanged.

`generate_oracle.py` extracts these exact fixed-source declarations into
`go_oracle.go`, with a standalone synthetic harness:

- `approval_action.go`: `writeOneTapApprovalError`, manager `consume`
- `approval_batch.go`: request DTO and `approvalBatchSignature`
- `http_helpers.go`: capped first-value decoder and JSON error writer

The signature helper substitutes a local `approvalSummary` projection for
`proto.ApprovalSummary`; its command/risk values and algorithm are unchanged.
The nonce fixture uses only a literal synthetic nonce, no signing key or token.
The generated program observes 6 errors, 11 decoding cases, 3 signatures and
3 nanosecond expiry boundaries. No Hub or public listener is started.

Regenerate from the repository root using the approved isolated toolchain:

```sh
python3 rust/tests/fixtures/services/approval-actions/generate_oracle.py
gofmt -w rust/tests/fixtures/services/approval-actions/go_oracle.go
GOPROXY=off GOSUMDB=off GOTOOLCHAIN=local go run rust/tests/fixtures/services/approval-actions/go_oracle.go > rust/tests/fixtures/services/approval-actions/go_oracle.json
```

Use the same isolated Go cache/module/GOPATH roots as the migration. The fixture
script does not change them, install software, or contact external services.

## Rust caller coverage

`hub/approval_actions.rs` uses `SessionCore` and its existing
`prepare_and_send → nonce consume → commit` lease. Only the core chooses current
keystrokes after acquiring its input FIFO. `approval_actions_tests.rs` composes
the real engine, private temporary SQLite/journal, synthetic transport and
explicitly drained effect owner.

The operation tests cover matching errors, actual input and ledger/UI closure,
failed-send retry, high-risk approval versus rejection, post-send expiry,
consumed-token ordering, replacement candidates, queued focus changes, queued
cancellation, dropped sends, release guards, accepted-effect ownership and
best-effort closure failure. Batch tests cover low/mid/high risk, disconnected
matched counts, stale-screen exclusion, malformed/oversized/first-value JSON,
required rule-adapter result propagation and the inherited deny-session OR
signature matching behavior.

Preserved Go quirks:

- Verification does not check the used-nonce table. An already consumed token
  with a still-pending candidate sends input before failing nonce consumption.
- Expiry is sampled again after input. At the exact expiration second it can
  return 401 after sending, retaining the pending candidate. No nonce is added.
- A replacement arriving during send survives failed commit, but the old nonce
  has already been consumed. Ordinary replay after success reports no pending
  approval rather than an already-used token.
- Batch deny-session selects the requested session OR matching signature.
  Disconnected matched candidates count even though delivery fails.
- Post-commit persistence/broadcast failure is reported to the required warning
  boundary; it does not roll back state, resend input or change successful HTTP
  status. A dropped HTTP waiter cannot discard transferred effects.

## Integration gates

The router owns token-path/method/Host/Origin/Hub-token/PIN guard order and body
transport limits. These fixture tests are not served HTTP/WS or frontend
acceptance. The constructor requires the same `OneTapManager` used by action
issuance, the actual effect sink, and a Hub-owned `ApprovalEffectOwner`.
After stopping admission the Hub must drain that owner before closing the
journal, sockets or runtime. The batch `auto_rule` path requires a real
`ApprovalBatchRules` add/persist/reload/publish implementation; the synthetic
rules adapter proves only this boundary and response behavior.

Approval-rule settings enable/disable/dismiss and native injection cleanup from
`approval_handler.go` are outside this narrow action adapter. They must remain
unresolved until their actual integration exists. Native/provider, real
notification, four-platform, rollback and independent-review acceptance remain
separate gates.
