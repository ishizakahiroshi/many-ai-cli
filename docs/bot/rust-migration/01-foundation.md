# #3 many-ai-cli rust: C1 foundation

> 最終更新: 2026-10-03(土) 12:07:35

## Purpose and source oracle

Implement the shared Rust foundation before parallel lanes. Read cmd/many-ai-cli/main.go, cmd/many-ai-cli-launcher/main.go, internal/proto/messages.go, web/src/types/proto.ts, internal/config/config.go, internal/securefile, internal/execpath, internal/provider/schema.go, web/web.go and existing tests. Use README and the actual command dispatch as the command inventory, including hidden commands and provider aliases.

## Change ownership

Only C1 edits rust/Cargo.toml, Cargo.lock, build.rs, src/lib.rs, src/bin/many-ai-cli.rs, src/bin/many-ai-cli-launcher.rs, src/proto/, src/config/, src/process/ and corresponding foundation fixtures. Existing Go sources remain unchanged. Do not modify another lane's modules. Integration owner wires accepted modules into shared entries later.

## Work

1. Choose dependencies from official documentation after verifying current versions, supported targets, license/provenance, required features and security evidence. Pin the resolved lockfile. Record actual features for SQLite FTS, PTY/native and embed rather than guessing. No unapproved workstation software installation.
2. Inventory all CLI branches, aliases, flags, env precedence, stdio/exit behavior and wire structures. Save synthetic golden fixtures including omitted fields, false/zero values, null/empty arrays, byte base64, timestamps and unknown fields. Preserve tolerated malformed optional configuration behavior instead of replacing it with destructive reset.
3. Implement command parsing and typed configuration/private atomic IO. Keep secrets private and expose only permitted fields. Add an explicit trial-root contract covering every resource named in 00-contracts.
4. Define and test shared interfaces for session lifecycle/snapshots, input queue/ACK/epoch, storage writer, approval records, provider command plan, process spawning/cancellation, routes and events. Freeze method/data names for lanes. Types must cover callers, failure/timeout and cancellation, not just happy paths.
5. Embed the generated Web bundle and resources without requiring the launch cwd to be the repository. Missing assets must produce a clear build/validation failure rather than an empty working dashboard. Preserve all required native resource/version metadata.
6. Publish module/interface ownership and a machine-readable entry/route/type/fixture inventory. Then release C2/C3/C4 to parallel work. Do not claim unimplemented CLI branches work.

## Validation and completion

Use synthetic CLI helpers and temporary roots to compare argv/env/stdout/stderr/exit against Go. A child helper must observe the env actually received. Trial root tests must fail on fallback into the default home, sharing DB/lock/registry, updating the old binary, or stopping an unowned process.

After worktree-only Web build, run cargo fmt --manifest-path rust/Cargo.toml -- --check, cargo check --locked --manifest-path rust/Cargo.toml --all-targets and focused cargo tests. The parent owns dependency/build serialization; do not race Cargo.lock or generated output. Exact acceptance command receipts and exit codes go into PROGRESS.

C1 finishes only when every shared interface is tested and usable by callers, both command inventories are traced, and isolation reaches all resources. Remaining CLI behaviors owned by downstream modules stay listed pending. C5 adds the relevant CHANGELOG Unreleased line after functionality exists; do not announce implementation in public docs ahead of code.
