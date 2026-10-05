# Persisted auto-approval observations

Behavior oracle: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`.
The Go implementation and existing Web source remain unchanged.

## Source and isolated fixture production

`generate_oracle.py` extracts the complete `internal/autoapproval/policy.go`
from the pinned Git object. It changes only the package/imports needed by the
standalone harness and replaces `Path()` with a harness-selected synthetic
filename. All load, rule compilation, hard-block, evaluation, AddRule, hash,
duplicate and write algorithms are otherwise the original functions.
The imported `internal/approval/summary.go` and `internal/securefile/atomic.go`
are checked for byte equality with the pinned versions before generation.

The harness creates and removes its own temporary directory. It never resolves
or changes HOME, runs a provider, starts a listener, connects to a service, or
opens a real policy. Reproduction uses the migration's approved Go 1.26.8
binary and isolated module/cache roots, with network dependency access disabled:

```sh
python3 rust/tests/fixtures/services/autoapproval/generate_oracle.py
gofmt -w rust/tests/fixtures/services/autoapproval/go_oracle.go
GOPROXY=off GOSUMDB=off GOTOOLCHAIN=local \
  go run rust/tests/fixtures/services/autoapproval/go_oracle.go \
  > rust/tests/fixtures/services/autoapproval/go_oracle.json
```

The checked-in JSON records 14 load/evaluation cases, 11 AddRule cases and
53 regular-expression acceptance/matching cases. YAML covers missing/empty/null
files, first-document behavior, aliases/merges, scalar spelling, null slices,
ignored unknown extensions, old/unknown versions, malformed input, duplicate
fields, ordered warning branches and CJK literals. Regex cases cover ASCII
shorthands/word boundaries, class punctuation/negation/POSIX classes, quoted
literals, octal/hex escapes, duplicate/leading-digit capture names, flags,
counted-repeat bounds and rejected Rust-only syntax.

## Rust implementation and caller boundary

- `approval::policy::load`: always returns a policy; missing files are empty,
  malformed YAML returns a disabled warning policy without a fatal error, and
  read failures return a disabled warning policy plus their I/O error.
- `Policy::evaluate`: preserves empty-command, hard-block, low-risk, ordered
  command and optional cwd checks.
- `add_rule`: uses explicit `RuntimePaths`, the shared schema-directed YAML
  decoder, per-path mutation serialization and shared private atomic I/O.
- `PolicyStore`: owns the actual currently published policy and a callback for
  current enabled preferences. The same `Arc<PolicyStore>` must be passed to
  both the Hub batch-rule boundary and `EngineOptions.approval_policy` through
  `live_callback()`. It does not turn on enabled preferences when adding a rule.
- `ApprovalBatchRules::add_and_reload`: persists the rule, reloads, and publishes
  to that same live callback. It never substitutes an in-memory success result.

The 17 focused tests in `approval/policy/tests.rs` include the fixed-source
comparisons plus synthetic disk reload/restart, malformed/read failure removal
of old authorization, exact first-rule-ID revalidation, dynamic enable/disable,
callback lock ordering, ignored post-add reload errors, returned inactive
duplicates, private file mode, concurrent additions, unchanged files after
rejected updates and CJK/special-character literal command/cwd round trips.
These are module/callback receipts, not whole-Hub route wiring, UI, native OS,
provider or deployment acceptance.

## Intentionally inherited Go behavior

These are source behavior, not newly introduced policy decisions:

- A version other than 1 warns but does not disable otherwise valid rules.
- An ID becomes reserved before command/cwd/risk validation, so a later duplicate
  cannot replace an invalid earlier rule.
- Omitted or empty risk is allowed at load, but evaluation always requires low.
- Rules remain ordered; callback action revalidation requires the same first ID.
- Command is trimmed; cwd is not. Empty cwd means no cwd restriction.
- AddRule alone checks hard blocks and multiline input, not risk classification.
  The batch HTTP boundary requires an actual pending low-risk approval.
- Duplicate literal command/cwd returns the old rule even with invalid ID/risk,
  with no rewrite, version update, repair or new authorization.
- A real append preserves a nonzero version and known old fields, and drops
  unknown fields through typed YAML serialization like the Go implementation.
- Startup discards Load's warning policy on an I/O error; reload publishes it.
- Successful AddRule followed by failed reload still returns the added rule,
  while publishing the disabled error policy. Failed AddRule does not reload.

## Open compatibility gaps, not accepted behavior changes

Go 1.26.8 uses Unicode 15.0 tables. The adapter now uses generated pinned-Go
property and SimpleFold data instead of the Rust regex dependency's newer data.
The original 53-case corpus has no exclusions, and the Unicode corpus under
`../autoapproval-unicode` compares 349,028 exact matching observations over
7,028 patterns. Valid Han and non-ASCII case-insensitive stored rules now remain
active. The generated source includes the Go BSD license, which is also carried
in candidate artifacts and described in `rust/THIRD-PARTY-NOTICES.md`.

A narrow compatibility gap remains: 29 valid Go anchored-surrogate optimization
forms in that corpus return an explicit unsupported warning rather than a
possibly broader match. The simple anchored-literal replacement-rune case is
implemented. The corpus README names the unresolved family; users should not be
told to rewrite their stored rules. Passing this corpus is not a proof of every
Go regexp expression or a whole-application acceptance receipt.

The shared YAML/file foundations impose finite decoding/read limits and redact
malformed-field/parser details. AddRule retains its source error prefix and the
Hub's `auto_rule_not_added` error boundary, but full Go YAML parser error prose
is not reproduced. Regex backend size limits likewise fail closed. These limits
and diagnostic differences must remain visible in integration reporting.
