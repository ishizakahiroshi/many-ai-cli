# #3 many-ai-cli rust: compatibility and isolation contracts

> 最終更新: 2026-10-03(土) 12:07:34

## Fixed interfaces and module ownership

Use the fixed Go SHA from README as the behavioral oracle. Rust is one package under rust/Cargo.toml, with src/lib.rs and two bin entry points named many-ai-cli and many-ai-cli-launcher. The integration owner alone edits Cargo.toml, Cargo.lock, build.rs, lib.rs, bin entries, proto, config and process interfaces. Declare dependency versions/features and upstream evidence, commit the lockfile and verify the supported targets before use. Do not invent crate features or pick latest without verification.

Freeze the Go implementation and existing Web TypeScript. The migration must port complete call chains, not expose placeholder routes. Keep wire names, enum values, omission/null/empty distinctions, byte base64, time formats, error/status codes, body caps, pagination, ordering and defaults. The Go protocol is internal/proto/messages.go; Web mirror is web/src/types/proto.ts. List every used type and route in fixtures before implementation.

Trial mode must require an explicit runtime root and isolated loopback port. Propagate that root to both binaries and all config/data/DB/log/token/profile/PID/active/lock/temp/model/runtime/update/usage-hook paths. Do not fall back to the real home if a trial path is absent. Do not change system HOME, CODEX_HOME or shell profiles. Disable trial notification/probe/periodic refresh and remote provider calls unless separately selected for acceptance. Child stop, update, discovery and cleanup must identify only trial-owned processes/files.

Keep production default CLI/storage semantics. Trial isolation is an explicit mode, not a silent change of users' defaults. Do not permit old/new simultaneous writes to one DB. Start with synthetic files, stubs and databases, then copied data, then operator-led acceptance.

## Required traceability

For every row, record Go entry/caller, Rust implementation, test IDs and acceptance receipt in PROGRESS. These identifiers group requirements; passing one test does not accept a whole group.

| ID | Required behavior | Owner | Acceptance |
|---|---|---|---|
| K01 | Both binaries, every CLI subcommand/alias/hidden command, argv/env/stdio/exit/shim semantics | C1 + callers | A01 CLI/env |
| K02 | Every HTTP/WS route, token/Host/Origin/PIN/remote/local read/write distinction | C3 | A02 protocol/auth |
| K03 | PTY, VT bytes, CJK/UTF8, alt-screen/replay/input ACK/resize/reattach/close | C2 | A03 VT; A04 OS/input |
| K04 | SQLite schema/WAL/FTS fallback, writer queue/reset/prune, persistence formats | C2 | A05 storage/rollback |
| K05 | Immutable approval records, identity/epoch, single transcript-or-VT source and provider contracts | C2 | A06 parser and callers/UI |
| K06 | Files/Git/attachment/mention scopes, canonical paths, readonly and mtime | C3 | A07 synthetic files/repo |
| K07 | Provider/profile/model/usage/env/cache, quota distinct from session tokens | C3 | A08 synthetic profiles |
| K08 | CLI update eligibility/preview/executable/job/version/timeout/exclusion | C3 | A09 stub updates |
| K09 | Existing TS UI, all embedded assets, pane/history/mobile/PWA/render guards | C3 + C5 | A10 served UI |
| K10 | Orchestration/admission/relay/headless/handoff/subagents/cancel/recovery | C2 | A11 synthetic child lifecycle |
| K11 | Routine scheduling/restart/dedupe, memo/private images/preferences | C3 | A05/A11 persisted services |
| K12 | Notify/push/tray/autostart/stop and unsupported OS behavior | C3 | A12 stubs + operator devices |
| K13 | Voice/audio limits, native managed processes/models/hash/redirect/loopback | C3 | A12 stubs + native acceptance |
| K14 | Launcher profile/UI/import/export, SSH serve/tunnel, env/cwd/cleanup | C4 | A09 fake SSH; A12 remote |
| K15 | Four current targets, binary archives/npm differences, runtime/signature/hash/install/update/doctor | C4 + C5 | A12 clean artifacts |

Retain the supported target set: Windows x64, Linux x64, macOS Intel and Apple Silicon. Current release archives and deb/rpm carry both binaries; current npm packages stage the main binary only. Do not add new targets or strengthen a channel's bundle contract without stating the change.

## Focused behavioral acceptance

When creating an absent Codex profile, omit only application-owned temporary hooks/session references while preserving user settings and user hooks. Test absent destination and existing destination, not only repeat synchronization.

For provider update, resolve and run the update command's argv[0]. A synthetic launch=A/update=B case must have matching preview, log and actual executable; missing B must fail without running A.

For SSH serve/import, preserve the whole script as one argument through both the SSH join and remote shell parsing layers. Use synthetic paths with spaces, quotes, newlines and shell punctuation; no real host connection in unit fixtures.

## Validation gaps that must be closed

Prove Windows headless cancellation under failed Job attachment and descendant-held pipes; prove interactive ConPTY teardown across supported OS behavior. Test canonical-path/rename/symlink races with synthetic files. Validate dependency/vendor provenance and reachable advisories from official sources. Evaluate build/publish credential separation and clean artifact identity. Confirm real deployment/native boundaries and copied-data backup/restore before cutover. A safely redacted history secret scan is a separate evidence step; never print matching secret bytes or upload them to an external scanner.

Unavailable environment or uncertain results remain pending with a concrete next action. Do not reinterpret stub success as real-provider/remote/mobile/OS acceptance.
