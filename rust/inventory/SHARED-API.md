# C1 reviewed shared API handoff

C1 at e7c1c18ffed4bcd96fdf09fbdb103edfb6595f4a passed independent review and
103 tests. The handoff is released to implementation lanes; it is not application
acceptance. Additive interfaces below carry their own later receipts. The Go
behavior oracle is 21d0bc7; review diff base is 6b0fb8e.

## Ownership

The integration owner alone edits Cargo/lock/build/lib/bin, `proto/`, `config/`,
`process/`, shared inventories and Git. C2 owns `terminal/`, `storage/`, `approval/`,
`orchestration/`; C3 owns `hub/`, `files/`, `profile/`, `update/`, `routine/`,
`notify/`, `voice/`; C4 owns `launcher/` and `delivery/`. Send proposed common API
changes to the integration owner. Do not work around an interface by making a
second config, storage, session manager or process abstraction.

## Concrete shared contracts

- `proto::*`: every 22-struct/302-field Go WebSocket DTO. Wire-facing readers use
  `proto::decode_wire::<Message>` for Go case-folded names, repeated-field merge,
  nullable values and base64 rules. Plain Serde DTO reads accept canonical internal
  values and are not an equivalent wire parser. Extend the schema/fixtures for
  other wire records before using the compatibility decoder with them.
- Additive HTTP boundary: `proto::decode_http_json<T: GoWire>` decodes one value
  without EOF, using the caller's explicit `GoWire::SCHEMAS`. Route-specific body
  limits stay with the service; do not use it for whole-frame WS messages.
- `proto::time::Timestamp` is the logical wall clock: normalized signed Unix
  seconds plus nanoseconds, independent of Windows FILETIME precision. Parsing,
  arithmetic, shared event fields and exact fixture clocks use Timestamp directly.
  Native OS/metadata reads enter through `from_system_time`; an OS-facing write
  uses `to_system_time_exact`, which rejects precision loss. `utc`/`from_utc` are
  explicit civil-time boundaries. `Instant`, Tokio deadlines and Duration remain
  the elapsed-time primitives. See `docs/bot/rust-migration/TIMESTAMP-DESIGN.md`.
  RFC3339 second/nanosecond selection, local timezone and Go-compatible parsing
  remain shared; do not hand-roll independent calendar conversions.
- `proto::provider::*`: complete provider data/registry persistence DTOs. Use
  `provider::to_go_json` for typed values whose bytes enter Go-compatible hashes;
  JSON value equality alone is insufficient for signatures/digests.
- `proto::core::*`: `SessionStorage`, `SessionCore`, input/processed-input,
  approval actions, spawn/update admission and confirmation interfaces, and typed
  snapshots/rows/events. All 34 existing Store methods are represented. These
  traits are not implementations of SQLite or the Hub.
- IDs for live sessions, persistent rows, session incarnation, wrapper connection,
  input sequence, replay, approval source, approval version, history generation,
  auth epoch and events are distinct. Do not merge them into one counter.
- Verified UI origin is never deserialized from JSON. `ResolvedChildSpawn`
  construction distinguishes an untrusted origin claim from the existing signed
  UI-origin cookie + Fetch-Metadata + allowed-Origin proof. It does not require
  active WebSocket membership. Confirmation decisions instead use a distinct
  ordinary authenticated-request proof; neither type is deserialized. Internal grants cannot be supplied by conductor JSON. Tri-state fields
  preserve absent/false/empty decisions.
- `AdmissionState` and provider launch/update leases are concurrency contracts.
  Use one state owner across spawn/update; retain the launch lease until actual
  registration or failure. Dropping/failing a reservation must release it once.
- `config::Config`, `ConfigStore`, `ConfigSnapshot`: load with `Config::from_yaml`
  or `ConfigStore::load_or_create`, not direct generic YAML deserialization. Public
  API output uses `public_json`; private persistence uses `to_private_yaml`.
  Store replacement requires the snapshot revision and publishes only after save.
- `config::legacy_provider_definitions` is the shared custom-provider adapter.
- `RuntimePaths`: explicit disjoint trial root and port. Production launcher WSL
  log-home selection accepts a resolved path from the platform adapter; no WSL
  process is launched by the path constructor. `with_log_dir` preserves the Go
  history DB placement and update-log location. Handle an invalid log root as the
  existing storage-initialization failure path, not a new unconditional Hub crash.
- Resource variants preserve actual filenames (`any-ai-cli.db`,
  `hub-runtime.json`, `push_store.json`, launcher registry/lock, providers,
  subscriptions, approval rules, whisper runtime/models and private assets).
  Database WAL/SHM/reset markers derive from the returned database path. Launcher
  connection-lock names and vendor-profile files must be derived within these
  roots by their owning lanes. Trial temporary hooks use its own `tmp` path.
- `checked_child` rejects lexical escape, existing/dangling outside symlinks and
  unresolved installed-root aliases. It does not itself close adversarial
  rename/symlink races; filesystem mutation needs a directory-handle policy and
  corresponding caller tests. Never fall back to HOME when a trial path is absent.
- `private_io::write_atomic` writes private same-directory temporary files and
  atomically replaces the destination. Windows has an ACL/resource replacement
  adapter; native ACL acceptance is still required.
- `process::{ProcessPlan,Cancellation,ExitOutcome,ProcessOutput}` and
  `run_capped`: resolved executable/argv/cwd, explicit set/unset environment,
  input, bounded output, deadline, bounded pipe drain and owned cleanup.
- `ManagedProcess::spawn_owned` returns a streaming receiver and owner handle;
  `close`/`wait` are idempotent. Dropping the owner aborts its IO/operation and
  containment. Receiver lag is explicit, never a successful URL/output scan.
  These helpers do not make live provider, notification or SSH tests authorized.

## Verification boundaries

The Go-generated corpora cover canonical protocol values, JSON decoder edges,
provider wire bytes/legacy conversion, historical YAML and actual CLI flag/dispatch
behavior. Pure interface tests establish type shapes and admission invariants;
C2/C3/C4 must add real caller, storage, process, UI and failure fixtures.

YAML has explicit defensive limits (8 MiB, 200,000 nodes, depth 128, one million
expansion steps). Limit failures and unsupported non-UTF8 binary scalars do not
reset the configuration or rotate tokens; source bytes remain available for
operator recovery. These are documented decoding boundaries, not proof that every
possible YAML document is accepted.

Native Windows Job/ConPTY/ACL behavior, all target linkers/artifacts, release
resources, real SSH/provider/model boundaries, browser/mobile acceptance and
copied-data rollback are still separate gates. Generated Web assets are validated
against the frozen build-input contract before embedding. Existing Go and Web
source must remain untouched.

## Wrapper/input caller additions (2026-10-03)

- `process::pty::{PtyFactory,PtySession,NativePtyFactory,PtySize,PtyExit}` owns PTY IO/resize/close/wait separately from pipe-based ManagedProcess. PtyExit.code is i64 to preserve Windows DWORD values. Unix process-group cleanup is not a universal detached-descendant container; native Windows ConPTY/Job acceptance remains separate.
- `process::execpath::Resolver` consumes explicit platform/environment/cwd/filesystem inputs. `wrapper::entry::run_cli` consumes an already-loaded config and explicit runtime context; it does not establish the still-unfinished Hub startup/spawn ownership handoff.
- `SessionDetails.last_output_at: Option<Timestamp>` exposes the same core clock for exact inactivity decisions. The display snapshot's second-resolution RFC3339 value must not become a runtime clock.
- `CoreEffect::SendWrapperBestEffort` is used for source-tolerated resize delivery. Input transport stays strict. The WebSocket owner starts core.flush only after reattach ACK, polls the reader concurrently and cancels/drains that owned work with its socket lifecycle.
- `terminal::journal::session_log_paths` is a pure shared naming function. It retains the validated timestamp's source offset/date and creates no file or second persistence owner.


## Owned HTTP and startup additions (author implementation, 2026-10-03)

- `HubTaskOwner` is the external lifecycle owner; task-captured services keep
  only `HubTaskHandle` or `HubApprovalEffectOwner` weak handles. Request/effect
  permits synchronously own work before response waiters can drop. Stop/drain
  requests while effect admission remains open, then stop/cancel/drain effects
  before journal, socket writers and Tokio. Completion counters distinguish
  returned application futures, panics, cancellation and abandoned permits.
- `SpawnConfirmations::register` returns pending metadata, unapplied effects and
  an already-owned completion waiter. `accept_decision` reserves a decision;
  `AcceptedSpawnDecision::run` commits synchronously. Acquire a Hub effect permit
  first, then transfer its returned future in the same poll without await or a
  fallible enqueue. Pending shutdown and task cancellation are separate domains.
- `VerifiedConfirmationRequest` carries an auth epoch captured before the
  ordinary HTTP guard. It has no UI-origin capability or wire constructor.
  `ConfirmationMissing`/`ConfirmationDecided` preserve404/409; typed
  `SessionError::ChildLaunch { status, code, detail }` survives confirmation
  completion so the original caller can retain source error statuses.
- `SpawnRegistrationMetadata` is server-owned, has no serde implementation and
  is stored alongside the one-use launch proof. Core applies it to persistence,
  snapshot and ACK before publishing registration. Editable labels do not resolve
  parent/board/role ownership. Wrapper-declared model/effort/execution mode remain
  source-authoritative. Initial-prompt payload/board caller work still requires
  the production orchestration driver; merely setting its input gate is not a
  completed injection.
- `Registration.startup_receipt` is present only for core-consumed internal launch
  proofs. `ProcessWrappedSpawner::prepare_registration` matches the owned
  attempt/PID and disarms startup termination before ACK. Known pre-ACK rejection
  aborts; uncertain delivery/drop detaches. A Registered result proves Hub
  acceptance, not observed ACK delivery or provider execution.
- `WrapperStartup` owns only an unregistered detached process/group/Job and has
  an independent native reaper. `ReapReceipt` has no termination authority.
  Windows child adoption retains a query-only inherited Job handle until process
  exit; provider plans remove bootstrap metadata and handle inheritance.
- `PolicyStore` is shared by persisted batch rules and the live automatic-action
  callback. Enabled preferences are read live, and reload failure removes stale
  authorization. Exact Go Unicode15 regex property/casefold compatibility remains
  unfinished; the current bounded slice warns/disables unsupported valid-Go
  forms rather than accepting a wider pattern. See its source fixture README.

These additions are not covered by the old C1 independent-pass receipt. Current
independent re-review, full CLI/Hub/orchestration/service composition, native
startup acceptance, real-data rollback and cutover remain separate open gates.
