# Synthetic SQLite compatibility evidence

Behavior oracle: Go commit `21d0bc7935a2c4696fb89ccff2e324157a528c2d`.
`traceability.json` maps all 34 public Store methods to Rust and executable tests.
These tests establish repository behavior, not Hub/UI, native OS, copied-data,
backup/restore, or production cutover acceptance.

## Running

Requirements: the locked Rust 1.90 toolchain/dependencies, Go 1.26.8 as specified
by the unchanged root `go.mod`, and its
locked module dependencies. Set Go on PATH or `MANY_AI_GO_BINARY` to its absolute
path. Set `CARGO_HOME`, `GOCACHE`, `GOMODCACHE`, `GOPATH`, and `XDG_CACHE_HOME` to
explicit isolated tool caches; tests use private temporary roots for all databases.
The Go oracle process sets `GOTOOLCHAIN=local` and `-mod=readonly` and never starts
a Hub or provider. No real configuration, transcript, credential, profile or
existing database is read.

- `cargo test --manifest-path rust/Cargo.toml --locked --test storage_contracts`
- `cargo test --manifest-path rust/Cargo.toml --locked --lib storage::`

`fixed_go_cross_read_and_rollback_continuation` runs in the normal integration
target. It writes through Rust, fully closes Rust, verifies relevant Go source
against `go-source-manifest.json`, copies only verified source files into a new
temporary module, invokes `rollback-oracle.go` by exact filename, asserts its JSON
receipt, and reopens the Go-written continuation in Rust. The manifest contains
SHA-256 hashes generated from the fixed commit's Git blobs, with CRLF normalized
to LF for text checkouts. Runtime verification needs neither Git nor historical
Git objects, so immutable archives and shallow CI checkouts work. Added local Go
files are not copied; `GOWORK=off` and cleared `GOFLAGS` prevent external workspace
or flag overrides from changing the oracle module. A regression test verifies
archive-style copying, unlisted-file exclusion, CRLF compatibility, and rejection
of a mutated pinned source. Go and
Rust are never active database writers simultaneously. No skipped test is used as
a compatibility receipt. The Go utility's build-ignore constraint excludes it
from normal Go package discovery. Its sole argument must name a canonical
`many-ai-storage-oracle-*` directory under the system temporary directory with a
bounded regular synthetic marker file.

`old-schema.sql` is an additive pre-metadata/pre-approval-identity schema with
synthetic old rows. Tests copy/create it inside their own temporary roots.

## Compatibility and deliberate differences

- All schema tables, indexes, nullable metadata and external-content FTS5 names
  remain compatible with Go, including the historical `any-ai-cli.db` filename.
- `close()` fully drains accepted events, an explicitly requested strengthening
  of Go's bounded best-effort stop. `shutdown(Drain { timeout })` exposes bounded
  waiting without losing ownership: after timeout/cancellation accepted work
  continues draining, and a later close still joins it. CompatibilityStop remains
  available as an explicit policy with dropped-work accounting.
- Reset clears external-content FTS exactly once, avoiding repeated deletion of
  the same index entries after the full-index clear. Base-table deletions remain
  bounded independent chunks and pre-reset counts retain their Go meaning.
- A pending file reset removal failure retains its marker and fails open rather
  than reopening partially removed data. No permission fallback accesses home.
- Trial database opens use SQLite NOFOLLOW and reject outside companion symlinks.
  Production symlink behavior remains compatible. Adversarial ancestor rename
  races still need the separately tracked directory-handle integration gate.
- Message masking matches `sessionlog.MaskSecrets`; event payload/derived-session
  field treatment is unchanged from Go. This is not a claim that all history
  fields are secret-free. FTS insert failure preserves the message and emits a
  sanitized warning; FTS query errors fall back to literal escaped LIKE.

Remaining gates: actual session/approval/storage caller integration; native
Windows/macOS locking, permissions and SQLite operation; all-target binaries;
full user-data backup/restore rehearsal; operator-led cutover.

## Review repair boundary

See `CHECKPOINT.md` for the capability-bound marker/recovery and close-order
repair, and `VFS-DESIGN.md` for the unimplemented DB/WAL/SHM confinement proposal.
The marker regression is not evidence that native SQLite path reopening is safe.
