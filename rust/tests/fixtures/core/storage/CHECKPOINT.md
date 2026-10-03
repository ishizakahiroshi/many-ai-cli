# Partial storage repair checkpoint

Review base: `8355b1060ca8753b8d469ec1244a33c355d307bd`.
Status: narrow marker/recovery and close-order repair; full SQLite namespace
confinement remains blocked. No custom VFS, root admission cap, descriptor
quarantine or new per-platform fail-closed branch is included.

## Code

- `src/storage/directory.rs`: opens a private held `files::safe_fs::Dir` using
  C3's `Dir::open_or_create_private`. Directory creation/restriction and later
  file operations use capabilities, not a checked pathname reused for mutation.
- `src/storage/mod.rs`: retains the directory capability and immutable writer
  thread identity.
- `src/storage/repository.rs`: reset-marker replace/read and initial private
  file setup use the held directory. Unix file permissions are changed through
  an opened handle before SQLite takes its own locks.
- `src/storage/schema.rs`: pending-reset DB/WAL/SHM/marker removal is relative to
  the held directory. A moved directory's replacement path is not consulted.
- `src/storage/writer.rs`: writer-thread reentrant close is rejected before
  acquiring `close_lock` or the JoinHandle mutex. A foreground closer may still
  join the writer, but its observer cannot block on that same closer's lock.

`storage/text.rs` byte-masking additions and exports are separately owned by the
runtime integration worker; they are not part of this security repair.

## Regression evidence

All tests use owned synthetic temporary directories. The observer-close case
runs in a supervised child test process, with a six-second outer bound, so the
pre-fix deadlock cannot hang the full test runner.

- Red: `cargo test --manifest-path rust/Cargo.toml --locked --lib storage::writer::tests::observer_close_during_concurrent_close_is_bounded -- --exact`
  failed with `writer observer deadlocked against close: Timeout` against the
  previous lock order. Exit 101. The complete failure log was retained in the isolated validation workspace.
- Green unit target: `cargo test --manifest-path rust/Cargo.toml --locked --lib storage::`
  passed 13 tests, zero failed/ignored. This includes the same observer-close
  regression and `reset_recovery_uses_held_directory_after_path_replacement`,
  which moves/replaces the path before recovery and before capability-relative
  file creation, preserving outside sentinel contents.
- Green integration target: `cargo test --manifest-path rust/Cargo.toml --locked --test storage_contracts`
  passed 17 tests, zero failed/ignored, including fixed Go cross-read. Full local
  full unit/integration receipts were retained in the isolated validation workspace. No compiler warnings occurred.
  The target includes
  `pending_reset_marker_stays_with_opened_directory_after_root_replacement`:
  an outside marker is neither adopted nor overwritten; our marker stays with
  the originally opened directory. This does not assert native SQLite sidecar
  confinement.

Pinned test environment: Rust 1.90.0 and Go 1.26.8; explicit Cargo/Go caches;
`GOTOOLCHAIN=local`; `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`,
`CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_BUILD_JOBS=2`; shared isolated runtime target
instead of the immutable review target.

## Remaining P1 boundary

Native SQLite is still opened by its legacy pathname. Root replacement between
validation and native open, or a later DB/WAL/SHM reopen/delete, is not fixed by
these marker changes. Windows ancestor pins do not prove final-child reparse
safety; macOS `/dev/fd` directory traversal has not been established. Existing
production/trial path behavior remains unchanged and unaccepted for that threat.

`VFS-DESIGN.md` is the read-only architecture proposal with source evidence and
alternatives. In particular, the discarded per-store Linux fd-path shim was not
safe when native same-process SHM ownership outlived its originating connection.
A dedicated architecture/native review and adversarial lifecycle fixtures are
required before the full P1 finding or migration can be accepted.
