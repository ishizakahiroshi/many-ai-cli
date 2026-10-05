# Rust migration candidate

This directory is an incremental implementation of the fixed Go contracts, not a
replacement for the running Go application. Follow
[`docs/bot/rust-migration/README.md`](../docs/bot/rust-migration/README.md) and the
[progress ledger](../docs/bot/rust-migration/PROGRESS.md).

The recovery candidate connects the main Hub, wrapper and launcher owners to
concrete binary entry points. The current source audit maps 32 CLI entries and
155 fixed Go HTTP registrations to their Rust callers. The 49-row historical WS
inventory includes three DTO discriminators and one internal relay request;
those rows are preserved without claiming that they are outgoing WS messages.
See `inventory/recovery-current-20261005.json` for source references and limits.

Source binding and synthetic test success are separate from production acceptance.
The candidate does not delegate application execution to Go. Trial mode uses an
explicit separate root and port, refuses real GUI, external pattern downloads,
real push delivery and recovered external PID termination, and confines vendor
artifact reads to held file handles. Real provider, account, OS, device and
cutover acceptance remains pending.

## Validation

Use Rust 1.90.0, Go 1.26.8 and Bun 1.3.14. The original dependency snapshot and official input
checksums are in `inventory/dependencies.json`; the current `Cargo.lock` fixes
resolved inputs. Final dependency license/SBOM distribution acceptance is pending.

```sh
(cd ../web && bun install --frozen-lockfile && bun run check && bun run build)
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --bins
```

The build requires the real generated Web bundle and embeds it plus the existing
launcher UI; it does not substitute an empty dashboard. No working-directory
lookup is required at runtime. Windows resource/native runtime packaging, signed
distribution and clean native acceptance on every target remain pending.

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


## Recovery validation boundaries

The immutable recovered checkpoint and the newly implemented recovery candidate
are separate inputs. A lost later source tree cannot be reconstructed by reusing
its historical test count. The latest completed local all-target receipt has
1151 passing tests, zero failures and zero ignored tests, including signal-error
handling. Matching strict Clippy passed. Binary/browser checks, clean four-target
CI and production acceptance are recorded separately.
The progress ledger records that distinction.

The candidate CI script requires a clean checkout before using HEAD as its build
identity. A dirty local candidate instead needs a separate source manifest,
Cargo lock hash, both binary hashes and explicit pending gates. Neither a local
binary nor the existing draft PR proves clean four-target CI, installation,
browser/device acceptance, copied real-data recovery or live cutover.
