# Rust development mainline

Since 2026-10-19 this Rust version is the main line on `develop`. The frozen Go
sources moved to [`../go/`](../go/) and are neither built nor checked. Source
comments such as `Source: internal/hub/...` cite the fixed Go commit named next
to them, where those paths are at the repository root. Assets that the Rust
build embeds (launcher UI, provider manifests, Windows runtime payload) live in
[`assets/`](assets/); nothing in the product reads `go/`.

This directory began as an incremental implementation of the fixed Go contracts.
The Rust Hub, wrapper and launcher now provide the runtime entry points on
`develop`. Historical migration checkpoints are recorded in
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

Use Rust 1.90.0 and Bun 1.3.14. The original dependency snapshot and official input
checksums are in `inventory/dependencies.json`; the current `Cargo.lock` fixes
resolved inputs. Final dependency license/SBOM distribution acceptance is pending.

```sh
(cd ../web && bun install --frozen-lockfile && bun run check && bun run build)
cargo +1.90.0 fmt -- --check
cargo +1.90.0 clippy --locked --all-targets -- -D warnings
cargo +1.90.0 test --locked
```

The build requires the real generated Web bundle and embeds it plus the existing
launcher UI; it does not substitute an empty dashboard. No working-directory
lookup is required at runtime. Formal Windows builds require prepared icon/version
resources and verified runtime inputs; see [packaging/README.md](packaging/README.md)
for the current contract. Building and starting the product are separate operator
actions under the repository's agent guidance. Final distribution and acceptance
of the same candidate on every target remain separate gates.

Protocol DTOs and compatibility fixtures are checked in. Rust contract tests read
those fixed inputs; routine validation does not regenerate them or rebuild the
frozen Go application. From the repository root, run:

```sh
cargo +1.90.0 test --locked --manifest-path rust/Cargo.toml --test proto_golden
```

Some historical compatibility tests explicitly require a pinned Go oracle. Their
results belong to the environment that actually runs them; an unavailable oracle
must not be reported as a successful compatibility check or worked around by
weakening assertions. Tests use temporary roots and fake processes. Cross-target
`cargo check` proves source compilation, not native PTY/process/ACL/mobile/device
or update/recovery acceptance.

## Shared foundation

- `proto/generated.rs`: all 22 Go WebSocket DTOs, 302 tagged fields, byte/base64,
  optional zero values, explicit activity flags and nullable required arrays
- `proto/core.rs`: distinct session/database/epoch identities; session/storage,
  input, approvals, event, admission and spawn/update interfaces; real in-memory
  admission invariant helpers
- `config/`: explicit root/resource paths, private atomic files and typed settings
- `process/`: bounded owned subprocess capture, input, cancellation and outcome
- `inventory/`: source-to-contract inventories; null implementation/receipt fields
  mean work is outstanding, regardless of the number of discovered test names

Do not install over an existing application or run old/new writers on one database.
A trial requires an explicit disjoint root and non-default loopback port. The
Rust Hub, wrapper and launcher are wired into the binary entry points on
`develop`. Source integration does not establish formal release, copied-data
rollback or operator acceptance. Official Docker distribution is outside the
v1.0.0 support scope.


## Recovery validation boundaries

The immutable recovered checkpoint and the newly implemented recovery candidate
are separate inputs. A lost later source tree cannot be reconstructed by reusing
its historical test count. Earlier recovery receipts and their test counts are
historical checkpoints. Binary/browser checks, clean four-target CI and
production acceptance are recorded separately and must be matched to the source
and candidate that actually ran.
The progress ledger records that distinction.

The candidate CI script requires a clean checkout before using HEAD as its build
identity. A dirty local candidate instead needs a separate source manifest,
Cargo lock hash, both binary hashes and explicit pending gates. Neither a local
binary nor the existing draft PR proves clean four-target CI, installation,
browser/device acceptance, copied real-data recovery or live cutover.
