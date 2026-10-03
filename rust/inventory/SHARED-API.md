# C1 shared API handoff candidate

This is the testable handoff surface for the next implementation lanes. It remains
subject to independent review of the exact next code SHA; it is not application
acceptance. The Go behavior oracle is 21d0bc7; review diff base is 6b0fb8e.

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
  construction distinguishes an untrusted origin claim from an authenticated UI
  binding. Internal grants cannot be supplied by conductor JSON. Tri-state fields
  preserve absent/false/empty decisions.
- `AdmissionLedger` and provider launch/update leases are concurrency contracts.
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
