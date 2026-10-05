# Persisted provider oracle

The generator imports the frozen `internal/provider` at Go behavioral baseline
`21d0bc7935a2c4696fb89ccff2e324157a528c2d`. It emits ordinary synthetic typed
Definition bytes, definition SHA-256, timestamp-pinned revision IDs, and an
embedded-plus-user registry revision. It never loads a home directory, invokes a
provider, changes a live Hub, or contacts a service.

Recorded with Go 1.26.8, offline, on 2026-10-03:

```
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off go run ./rust/fixtures/provider-store-oracle > rust/fixtures/provider-store-oracle/expected.json
```

`profile::store::tests::go_oracle_canonical_hash_revision_and_registry` checks the
fixed bytes and digests rather than substituting a JSON Value comparison.

This is storage/algorithm evidence, not live provider, accepted-signature,
remote download, native-platform, or migration/cutover acceptance.
