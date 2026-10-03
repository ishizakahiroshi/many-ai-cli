# #3 many-ai-cli rust: HTTP and application services

> 最終更新: 2026-10-03(土) 12:07:38

This is an implementation instruction for the GitHub-connected bot. Document creation does not mean implementation has started. Read the repository's `AGENTS.md`, `CLAUDE.md`, and `docs/bot/rust-migration/00-contracts.md` before implementing. Use repository source and tests as the behavior oracle. Do not require private files or local machine paths.

The project label is `#3 many-ai-cli rust`; it is separate from GitHub issue or PR number 3. Send progress to the integration owner for PROGRESS.md. The operator starts from Slack or ChatGPT Web and controls external messages. When directed continue only in that same #3 task conversation; do not initiate messages or report into #1/#2 tasks.

## Ownership and prerequisites

Implement the service modules in the single `rust/Cargo.toml` package. The foundation owner provides `rust/src/lib.rs`, both binary entry points, shared protocol/config/storage types, private atomic file IO, and process interfaces. Read those actual interfaces before using them. Ask the integration owner to change shared interfaces rather than creating competing types.

Own only these source directories, including their inline tests and synthetic fixtures:

- `rust/src/hub/`
- `rust/src/files/`
- `rust/src/profile/`
- `rust/src/update/`
- `rust/src/routine/`
- `rust/src/notify/`
- `rust/src/voice/`

The foundation owner edits the manifest, lockfile, shared types, library root, and binary roots. The core owner implements terminal/session/approval/orchestration state machines. The delivery owner handles launcher/distribution. Send integration requirements to those owners. Leave existing Go code, Web TypeScript, public manifests outside `rust/`, and existing release assets unchanged. CHANGELOG/README edits belong to the integration owner; return concrete release-note input.

Prerequisite interfaces include session snapshots/actions, session cwd and git root, spawn admission, per-provider update admission, approval/done events, cancellation and managed processes, transcript/history/path-mention lookup, configuration snapshots and private persistence. Module compilation with test stubs is useful evidence, but integrated application behavior requires the actual core implementations.

## Area 1: HTTP, WebSocket, authentication, and assets

The routing oracle is `internal/hub/server.go`, including all `Handle` and `HandleFunc` registrations. Start by building an exhaustive inventory of registered paths and dynamic suffix handlers. For each path, record methods, query/body fields, status and error codes, JSON shape, body limits, authentication/Host/Origin requirements, logical-remote policy, and associated Web caller. Cover the supplementary APIs as well as the primary session endpoints; do not silently omit unknown paths.

Read `internal/hub/http_helpers.go`, `pin_auth.go`, `auth_handlers.go`, `internal/proto/messages.go`, and Web callers under `web/src`. Preserve omitted fields versus null/empty arrays, byte/base64 representation, epoch fields, numeric identifiers, and timestamp formatting. Connect the WebSocket handler to core state and preserve its authentication and disconnect behavior.

Preserve the existing authenticated single-user trust model, direct-loopback versus logical-remote distinction, state-changing Origin checks, allowed Host normalization, authentication revocation, UI authentication epochs, and optional PIN configuration and lockout behavior. Do not impose a new capability-token or mandatory-PIN architecture as part of the migration. Static asset routes and authenticated API routes have different existing guard policies; portless Host behavior must follow the current implementation.

Use `web/web.go` and existing asset registrations as the asset oracle. Serve the existing built Web application from embedded assets so both binaries work independently of current working directory. Include app entry/modules, styles, vendor scripts, locale files, manifest/service worker, recorder worklet, icons, and approval-pattern assets. Keep current Markdown sanitation/fail-closed rendering, module boot guard, xterm behavior, and UI structure.

Suggested files, all new within owned directories: `rust/src/hub/{mod,http,auth,assets,api_contract_tests}.rs`. Coordinate embedded-byte generation with foundation/delivery owners.

Fixtures: `internal/hub/auth_handlers_test.go`, `audit_host_guard_test.go`, `audit_dns_rebinding_test.go`, `c4_audit_test.go`, and `web/tests/boot-guard.test.ts`. Add a route-coverage test that checks every inventory entry has an implementation and behavior fixture. Exercise valid and rejected tokens, Host/Origin variations, IPv6, allowed hosts, remote classification, optional PIN transitions, revocation, stale UI epochs, and unauthenticated static routes.

Keep outbound HTTPS/public-network transport separate from managed-service loopback-only HTTP transport. Preserve DNS validation followed by dialing the validated address, original TLS host verification, redirects, body limits, and timeouts. Use fake DNS/transport fixtures rather than live network services.

## Area 2: providers, profiles, models, and usage

Read `internal/provider/{registry,schema,merge,launch,history,usage_adapter}.go`, provider definitions and `schema.json`, `internal/subscription/{adapter,seed,resolve,codex,usage_source}.go`, and `internal/hub/subscriptions_handlers.go`, `subscription*.go`, `models*.go`, and `nvidia_nim_handlers.go`. Include all provider CRUD/distribution/icon/history/model/usage routes in the inventory. Maintain builtin/custom overlays and validation, enabled state, launch candidates, route/environment decisions, model selection, and history adapters.

Profiles isolate vendor configuration directories and never transport login credentials. Preserve additive seed behavior, existing-profile synchronization, owned/state keys, default-wins overrides, policy deletion synchronization, symlink/link fallback, and private permissions. Automatic subscription selection remains spawn-time round-robin rather than quota-based account switching.

Synthetic profile-seeding fixtures must cover both absent and existing destination settings, a user's own hooks, tool-owned temporary session hooks, nested TOML tables/arrays, malformed input, symlink defaults, and restricted link permissions. Expected behavior: user-maintained hooks and policy survive; tool-owned temporary session references are not propagated to a new profile; credentials remain outside the seed set. Do not remove all hooks or overwrite profile-owned authentication/state fields.

Keep subscription usage refresh, optional probe, cache, freshness/observed-at, and session token totals as separate concepts. Preserve manual refresh and periodic/probe opt-in. Fixtures use fake app-server/provider processes and synthetic cache data, never a paid AI request or automatic periodic collection.

NIM settings/results must not echo a key. Environment-supplied keys cannot be deleted or replaced through the settings API. Retain existing safe result/error classification and model-route behavior.

Suggested files: `rust/src/profile/{mod,registry,seed,resolve,models,usage,nim,tests}.rs`, `rust/src/hub/provider_api.rs`, and `subscription_api.rs`.

Fixtures: `internal/subscription/seed_test.go`, `subscription_test.go`, `codex_usage_appserver_test.go`, `internal/hub/subscription_*_test.go`, `models_*_test.go`, `nvidia_nim_handlers_test.go`, and provider tests. Preserve the meaningful existing cases, then add the first-seed and temporary-reference expectations above.

## Area 3: Files, Git, attachments, and CLI updates

Read `internal/hub/files_scope.go`, `files_content.go`, all `files_*` handlers, `git_*` handlers, and attachment handling in `misc_handlers.go`. Implement every corresponding route. Keep canonical-path handling, root restrictions, remote mention fallbacks, read-only responses, modification-time conflict detection, atomic creation, upload limits, attachment history/purge, and version-control metadata mutation restrictions.

Reads and writes have distinct scopes: direct-loopback reads represent the local user, logical-remote reads follow allowed roots and recorded mentions, and writes remain restricted to session cwd/git roots. Existing sensitive-file checks apply according to their original scope. Do not replace these policies with a blanket cwd restriction. Memo and conversation path mentions require the actual core/workspace lookup interface.

Use temporary synthetic repositories and file trees for empty/CJK/spaced paths, canonical symlinks, rename/symlink interleavings, readonly mentions, upload limits, stale mtime, and attachment deletion. Never read a user's credential/config directory as fixture input. Git commit/fetch/pull/push and generated commit-message APIs remain supported; test them with a temporary repo and fake executable/transport rather than public repository writes or real AI calls.

For updates read `internal/provider/update.go`, `internal/hub/cli_update.go`, `cli_version.go`, and `internal/execpath`. Resolve the configured update command independently when appropriate. Synthetic launch-command A/update-command B fixtures must show that eligibility, preview, displayed executable/arguments, logs, and the executed job identify the same resolved update command. Include absent B, secondary launch candidates, disabled/unconfigured/login-required updates, running-session/update mutual exclusion, registry changes during a job, timeout, bounded output, before/after versions, and exit classifications. Use local fake commands rather than real installers.

Suggested files: `rust/src/files/{mod,scope,content,mutation,git,attachments,tests}.rs`, `rust/src/update/{mod,plan,job,tests}.rs`, `rust/src/hub/files_api.rs`, and `update_api.rs`.

Fixtures: `internal/hub/files_*_test.go`, `git_*_test.go`, `internal/provider/update_test.go`, and `internal/hub/cli_update_test.go`. Include the expected command-consistency behavior above even where existing fixtures do not cover that combination.

## Area 4: routines, memos, preferences, notifications, and native services

Read `internal/hub/routine_store.go`, `routine_runner.go`, and `routine_handlers.go`. Preserve admission and persistence before process launch, request-ID idempotence, timezone/DST schedules, disabled state, restart behavior, session label/database/Hub-instance matching, immutable completion results, retry on failed writes, and prevention of replaying prompts for missing sessions. Use the core spawn/completion interface rather than a second session manager.

Read `memo_store.go`, `memo_handlers.go`, `memo_images.go`, and `user_prefs_handlers.go`. Preserve versioned private atomic JSON, the current memo text/count bounds, server-derived project git roots, corrupt-store rejection without overwrite, image-basename constraints, readonly mentioned paths, preferences defaults, update semantics, and reload persistence.

`internal/notify` supplies outbound backends; push support is `internal/hub/push.go`. Preserve configured opt-in, bounded/masked payloads, deadlines, pending/successful-send deduplication, failure reporting semantics, VAPID/subscription persistence, and notification URLs without Hub tokens. Test with local fake transports; do not send real notifications during fixture runs.

`internal/tray` implements an independent Windows resident process that survives Hub stop and can reopen/start the Hub. Other OSes return the existing unsupported result. Preserve single-instance behavior, lifecycle distinction, and setup/startup contracts; coordinate setup wiring with the foundation/delivery owners. Stub fixtures do not prove desktop/autostart behavior.

Voice lives in `internal/hub/voice_transcribe.go` and `whisper_manage.go`, `whisper_job_*.go`, and `whisper_disk_*.go`. Preserve audio limits, empty-input/errors, cancellation/deadlines, loopback-only transport/redirects, request-path fallback, transcript normalization/hallucination filtering, native managed process lifecycle, runtime/model integrity, install/uninstall/status/start/stop behavior, and local-model routes. Use synthetic audio, fake HTTP services/processes and manifests. Do not install a native model or invoke a real cloud/provider request as an automatic fixture side effect.

Suggested files: `rust/src/routine/{mod,store,runner,memo,preferences,tests}.rs`, `rust/src/notify/{mod,backend,push,tray,tests}.rs`, `rust/src/voice/{mod,transcribe,managed,tests}.rs`, `rust/src/hub/workspace_api.rs`, and `native_api.rs`.

Fixtures: `internal/hub/routine_test.go`, `memo_test.go`, `memo_images_test.go`, `user_prefs_handlers_test.go`, `internal/notify/*_test.go`, `internal/hub/audit_push_dedup_test.go`, `internal/tray/*_test.go`, `internal/hub/voice_transcribe_test.go`, `whisper_manage_test.go`, `audit_whisper_download_test.go`, and `whisper_manifest_test.go`. Use controllable clocks, failed-write injection, fake exit/cancellation, and manager reloads to prove failure and recovery paths.

## Area 5: integration and acceptance evidence

Run module fixtures only after foundation interfaces exist. The Rust package test command is `cargo test --manifest-path rust/Cargo.toml --lib`; record the actual cases exercised and full failures. Full package compilation/build/test settings belong to the integration owner. `web/package.json`'s `test` includes a build, so do not describe it as a static-only check. Web type checking runs as `bun run check` from `web/`; any Web build uses the repository/integration instructions.

After core integration, check the exhaustive route matrix against real handlers, JSON/errors/auth, persistence, process actions, WS messages, and embedded assets. Runtime paths must not end in test-only, memory-only, or unsupported placeholders for existing features. Source/test agreement alone does not prove a served or native application works.

Provide the integration owner with an isolated acceptance checklist for the existing Web UI:

- Profile create/select, model candidates, manual Usage refresh, and key-free settings/results.
- Files read/write/readonly/conflict/move, Git status/diff, attachments preview/delete, and update eligibility/job output.
- Routine schedule/manual run/history/reload, memo/image CRUD, and preferences persistence.
- Opt-in notification/push behavior and separate Windows tray lifecycle evidence.
- Voice errors/success/cancellation and managed native runtime status/install/lifecycle evidence.
- Browser/mobile/i18n/PWA behavior, sanitized Markdown, module boot failure UI, and served-asset identity.

Use isolated ports/config/data/logs/process scope. Never start/stop/reload an existing user Hub or mutate real profile/database data as part of this work package. Cross-OS/provider/device/native acceptance and backup/rollback are integration gates, not implied by stub success. Record unperformed cases explicitly.

## Completion and report

Finish only when each route inventory entry maps to the Rust implementation and a meaningful fixture, profile/update expectations above pass, persistence survives reload/failure, core interfaces are connected, and source behavior matches frozen Web callers. Send integration requirements and unresolved runtime acceptance gates rather than marking them passed.

Return owned changed files, commands/results, route-to-test coverage, intended behavior differences, isolated acceptance evidence, and remaining OS/device/provider/native gates. Supply release-note text for profile-seeding and update consistency plus user-visible service compatibility; the integration owner edits CHANGELOG/README. Preserve complete failures. Do not merge, tag, publish, deploy, switch the existing application, or declare release acceptance from this document alone.
