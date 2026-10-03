# Rust migration candidate

This directory is an incremental implementation of the fixed Go contracts, not a
replacement for the running Go application. Follow
[`docs/bot/rust-migration/README.md`](../docs/bot/rust-migration/README.md) and the
[progress ledger](../docs/bot/rust-migration/PROGRESS.md).

Current binary entry points deliberately return an error for unintegrated commands.
No successful HTTP/CLI stubs and no fallback that delegates application work to Go
are provided. `version`/`help` are candidate diagnostics; command compatibility is
still pending.

## Validation

Use Rust 1.90.0, Go 1.26.8 and Bun 1.3.14. The package versions and official input
checksums are in `inventory/dependencies.json`; `Cargo.lock` fixes resolved inputs.

```sh
(cd ../web && bun install --frozen-lockfile && bun run check && bun run build)
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --bins
```

The build requires the real generated Web bundle and embeds it plus the existing
launcher UI; it does not substitute an empty dashboard. No working-directory
lookup is required at runtime. Windows resource/native runtime packaging and all
application route wiring remain pending.

Regenerate the protocol DTOs and synthetic Go serialization oracle from the repo
root, then format and run the Rust contracts:

```sh
python3 rust/scripts/generate-proto.py
go run ./rust/tests/oracle > rust/tests/fixtures/foundation/proto-golden.json
cargo fmt --manifest-path rust/Cargo.toml
cargo test --locked --manifest-path rust/Cargo.toml --test proto_golden
```

The oracle imports only the frozen protocol package and emits synthetic data. It
never loads real configuration, contacts providers, or starts a Hub. Tests use
temporary roots and fake processes. Cross-target `cargo check` proves source
compilation, not native PTY/process/ACL/mobile/device acceptance.

## Shared foundation

- `proto/generated.rs`: all 22 Go WebSocket DTOs, 302 tagged fields, byte/base64,
  optional zero values, explicit activity flags and nullable required arrays
- `proto/core.rs`: distinct session/database/epoch identities; session/storage,
  input, approvals, event, admission and spawn/update interfaces; real in-memory
  admission invariant helpers, with downstream service implementations pending
- `config/`: explicit root/resource paths, private atomic files and typed settings
- `process/`: bounded owned subprocess capture, input, cancellation and outcome
- `inventory/`: source-to-contract inventories; null implementation/receipt fields
  mean work is outstanding, regardless of the number of discovered test names

Do not install over an existing application or run old/new writers on one database.
A trial requires an explicit disjoint root and non-default loopback port. The
current app entry refuses unintegrated runtime commands. Real-data rollback,
operator acceptance, merge, release and cutover have not occurred.
