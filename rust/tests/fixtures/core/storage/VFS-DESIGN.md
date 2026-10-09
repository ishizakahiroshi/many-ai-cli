# SQLite confinement architecture review proposal

Status: proposal only. No custom VFS, process-lifetime descriptor registry, root
cap, or new platform fail-closed behavior is implemented by the narrow repair.
The full DB/WAL/SHM confinement finding remains open.

## Fixed evidence and threat boundary

- Go oracle `21d0bc7935a2c4696fb89ccff2e324157a528c2d`,
  `internal/sessionstore/store.go`: `OpenForLogDir` derives a pathname, opens
  SQLite, and later marker/recovery operations reuse that pathname. The ordinary
  compatibility contract includes one physical connection, WAL, FTS5, additive
  migration and Go readback. Go's pathname implementation is not evidence of
  resistance to concurrent directory replacement.
- Reviewed Rust `8355b1060ca8753b8d469ec1244a33c355d307bd` reused a saved PathBuf
  through `private_io::write_atomic` for reset markers. Moving an opened trial
  directory and replacing its old name could redirect that later marker write.
- Threat: another actor able to rename entries in the caller-owned synthetic
  parent replaces an ancestor or final child between validation and use. The
  assistant must not write/delete outside its held directory capability. This
  proposal does not promise protection against an attacker already executing
  arbitrary code inside the assistant process or directly rewriting allowed
  database bytes.
- Locked dependency source inspected: `libsqlite3-sys 0.38.2`, bundled
  `sqlite3/sqlite3.c`. Relevant functions:
  - `unixFullPathname` (about line 47195) resolves pathname symlinks.
  - `unixOpen` (about 46725) uses final-component O_NOFOLLOW.
  - `unixOpenSharedMemory` (about 45220) uses O_NOFOLLOW for SHM opens.
  - `unixShmUnmap` (45690) may unlink a shared node's saved filename only when
    its last reference closes; `unixShmPurge` (44998) then closes/maps/frees it.
  - `winOpen` (54060; flags about 54205–54223) passes normal/random-access
    attributes to CreateFileW, without OPEN_REPARSE_POINT. Ancestor directory
    pins alone therefore do not establish final DB/WAL/SHM reparse safety.

## Narrow repair being evaluated separately

`storage::directory` opens and retains `files::safe_fs::Dir`.
Marker replacement, marker metadata reads, recovery deletion, empty-file creation
and Unix permission setup use that held capability. No fresh pathname check is
used as a substitute for those capability-relative operations. A write-error
observer is rejected before acquiring the foreground close mutex, preventing a
join/observer-close deadlock. These changes do not anchor SQLite's native IO.

## Why the small Linux pathname shim was rejected

A VFS shim can prevent expansion of `/proc/self/fd/<dirfd>/any-ai-cli.db`, leaving
native SQLite to open DB/WAL/SHM beneath a held directory. Passing that alias to
an unmodified VFS does not work: native full-path expansion can resolve it back
to a mutable pathname.

Even with expansion overridden, per-store descriptor lifetime is insufficient.
Native Unix SQLite may share SHM nodes between same-process connections by inode.
The first node's saved SHM pathname can outlive its originating connection. If
that store closes its dir fd and the number is reused for another directory,
later native unlink could follow the reused fd alias. Keeping a VFS context alive
only until its own Connection drops does not prove safety. The draft shim was
removed rather than presented as a fix.

## Alternatives requiring architecture decision

1. Established SQLite APIs: NOFOLLOW and connection pragmas do not provide a
   directory capability for all future sidecar operations. Backup operates on
   connections, not an existing directory handle. Serialize/deserialize plus
   capability-relative replacement could confine snapshots, but replacing live
   disk/WAL operation changes crash recovery, concurrent-reader visibility and
   persistence costs. It is not a compatible drop-in storage repair.
2. Held namespace or owned helper process: a stable mount/namespace containing
   the opened directory could allow native VFS semantics. Feasibility depends on
   platform facilities and permissions, explicit process ownership and cleanup,
   recovery after helper termination, and availability on every supported OS.
   No privilege, mount, namespace or process setup is authorized implicitly here.
3. Custom capability VFS: own all open/delete/access/full-path operations and
   every database/journal/WAL/SHM/temp file lifetime. This must preserve SQLite
   lock-byte semantics, mmap/SHM barriers, crash durability, Go/native readback,
   callbacks without unwinding, buffer/layout safety, failure cleanup and cross-
   connection SHM sharing. Merely overriding xOpen or xFullPathname is insufficient.
4. Bounded process-lifetime anchor registry (proposal only): retain immutable
   VFS contexts and directory descriptors until process exit; deduplicate by held
   directory identity and impose an explicit small root limit before filesystem
   mutation. This avoids fd-number reuse even for late unmanaged native SHM
   callbacks, but introduces a Go-compatibility resource limit. Review must cover
   synchronization, permanent callback data, aliases, root identity reuse,
   registered-name uniqueness, temporary-name lifetime and bounded memory as well
   as fd count. No unlimited retention is acceptable. The limit and error must
   be user-visible/documented if this design is chosen.

## Required acceptance for any full fix

- Replacement before first SQLite open, after open, before WAL/SHM reopen, and
  during reset/close; assert outside sentinel contents and metadata unchanged.
- Multiple managed and unmanaged same-process connections sharing the DB/SHM;
  close the first store, force descriptor reuse, then close the last connection.
- Connection/open failure, callback failure, duplicate close, writer observer
  reentry, temporary-file cleanup and process termination.
- Preserve WAL, busy timeout, FTS fallback, incremental vacuum, SQLite backup,
  and sequential Rust/Go rollback tests.
- Native Windows final reparse and ancestor-pin behavior; native macOS solution
  rather than assuming `/dev/fd` supports safe directory traversal.

Until those gates pass, the narrow marker/deadlock repair is partial. Existing
production and trial SQLite path semantics remain in place and are unaccepted
against this adversarial DB/WAL/SHM threat; no secure-success claim is made.
