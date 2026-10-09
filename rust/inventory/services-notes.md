# Services contract inventory (C1 preparation)

Baseline: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`.
Prepared 2026-10-03. This is source inventory and an interface proposal, not implementation, an API freeze, test execution, or release acceptance.

Current Rust implementation status is recorded separately at 4dc961ce in `recovery-current-20261005.json` and `../src/hub/route_coverage.json` under `current_source_coverage`. All 155 registration rows and49 dynamic-operation rows have source owners/main bindings; the null implementation/fixture fields in the fixed-Go `services.json` below remain historical contract-index fields, not current missing-Rust totals. Synthetic/native CI and product acceptance stay separate.

## Coverage and how to use the JSON

`services.json` inventories all 155 `mux.Handle` / `mux.HandleFunc` registrations in `internal/hub/server.go:1477-1642`, including 14 static registrations and the WebSocket endpoint. It adds 49 dynamic suffix/operation entries, 298 handler/security/helper source references and contract expressions, 307 JSON-tagged Go struct declarations, 333 Web source endpoint lines, 720 named baseline test references, and the source census of 1,923 Hub function signatures. The exact numbers are also machine-readable in `counts`.

Every route has its registration, handler and dispatch references, method observations, dynamic path methods, guard policy, request decoding/query evidence, body-limit expressions, wire-type references, response/error expressions, Web callers, baseline tests (including test helper callers), proposed Rust fixture identifier, and null implementation/fixture/receipt fields. Baseline source references and retained contract expressions identify inline request structs and map responses that have no standalone DTO. `wire_structs` preserves exact declarations, tags, omitted fields, pointers and Go types; non-struct named types and source constants are indexed separately. `storage_interface_uses` gives all 25 actual Hub storage method calls and exact signatures. `source_hashes` permits detecting drift. Schema v2 stores each value as a `{sha256: digest}` object so digest values are clearly identified and not adjacent to authentication-related filenames; all hashes were rechecked against the fixed Go SHA.

The extraction is a lexical index, not a Go typechecker or proof that a linked test covers every branch. `methods_observed` is a union across a dispatcher and its handlers, not permission for every method on every suffix. Exact guard/branch source is authoritative. Query names computed dynamically need their source branch, e.g. agent-chat's cursor/offset alias. Helpers are linked through `hub_function_index`; unrelated identically named methods may appear as conservative lexical candidates. No executable fixture or runtime acceptance was run.

The required `.omitnix/index.json` was read before source-role searches. Its generated commit matches the fixed baseline but `dirty=true`; it reports unresolved dynamic SQL in sessionstore, so its apparent zero references were never treated as absence of behavior. No source-index regeneration was performed.

## Shared interfaces requiring foundation freeze

These names are proposed shared boundaries, not competing Rust implementations. C1 owns final names/types. The exact operations and source evidence are machine-readable under `proposed_shared_interfaces`.

- RuntimePaths: explicit root for config, data/DB, logs, attachments, orchestration, profiles, models, native runtimes, update logs, usage hooks, PID/active/locks/temp. Trial mode fails closed instead of falling back to the actual home. Propagate root and owned-process identity to both binaries and child launches.
- ConfigStore: cloned snapshot; private atomic persistence; preserve optional/unset values and original field naming. Runtime port is separate from configured port. Never hold config/session locks simultaneously. Revoke token/cookie-secret memory rollback and UI epoch invalidation must follow its original persisted boundary; other settings have differing historical failure handling.
- SessionCore: Hub/session snapshots, active IDs, wrapped spawn/registration wait, input FIFO+ACK+epochs, stop/dismiss, events, provider-update admission, and UI-auth epoch invalidation. Snapshot must carry immutable `launch_label` separately from editable `label`, provider revision/profile ID, actual execution/permission modes, CWD/project/git roots, pending approval/activity and last output.
- SessionStorage: exact existing 25-method receiver-call list plus OpenForLogDir/SetOnWriteError/CloseStaleSessions initialization dependencies in JSON (earlier candidate APIs such as UsageSummary/Timeline are not established HTTP dependencies). Queries must exist in addition to the writer: live-vs-durable session IDs, chat/search/approval history, user-message path mentions, session overviews/card metadata, reset preserve IDs, reset-pending and prune. Reset cannot resurrect queued pre-reset events.
- WorkspaceEvidence: user-only session mentions, recorded handoff memo paths, memo mentions, transcript snapshots and byte-cursor pages. Files must use real evidence instead of broad access or denied-everything stubs.
- ManagedProcess: executable resolution; capped output run with args/cwd/env/deadline/cancellation; start-error vs timeout vs exit; owned process tree cancel/wait for Whisper/SSH/wrappers. An update must execute the same resolved argv[0] that its plan displays.

A single session state owner is necessary. Routines observe launch labels and non-fallback DoneSummary events, then obtain durable DB IDs; HTTP must not create a second session manager. Provider updates count disconnected-but-undismissed cards too. Input deferred work must revalidate UI auth epochs before mutating core state.

## Authentication and transport contract

- Standard guard order: token, allowed methods, Host, state-changing Origin, then optional logical-remote PIN. `guardBase` excludes PIN for auth status/login/logout. Token precedence is nonempty query token, exact `Bearer ` authorization prefix, then token cookie; comparison is constant-time and empty tokens never match.
- `allow_loopback_without_token` also gates trusted-network bypass. Direct loopback bypass excludes logical-remote requests. A nonloopback peer, nondefault Host, or populated `Tailscale-User-Login`/`X-Forwarded-*` header makes a request logically remote. PIN/remote notification/file-scope decisions use that classification.
- Allowed Host lowercases, trims surrounding whitespace/brackets and one trailing dot; defaults are 127.0.0.1, localhost, ::1 plus configured exact hosts. Portless allowed Host is accepted even when Hub has a known port. An explicit port must match. HTTP Origin without a port is compared as port 80; HTTPS Origin requires a configured host and does not match default loopbacks merely because they are defaults.
- Origin-less mutations allow only absent/none/same-origin Sec-Fetch-Site; safe GET/HEAD/OPTIONS bypass this extra CSRF check, not Host. WebSocket handshake additionally rejects Origin-less logical-remote clients.
- `/` is GET only; authenticated document omits PIN to permit showing its dialog. Unauthorized HTML navigation returns a 401 reauth document; other unauthorized callers get JSON. The token cookie is emitted only when an actual valid token was presented, never from bypass alone. Token cookie: HttpOnly, Lax, 24 hours, Secure only for direct TLS or trusted-loopback forwarded HTTPS. The index also issues the UI-origin capability cookie used by child-spawn confirmation.
- All 14 static routes are intentionally unauthenticated FileServer paths with global security headers only. `/approval-patterns/` is a separate guarded user-configuration asset route. Unknown paths fall through `/`; retain ServeMux slash redirects/path behavior rather than replacing with universal JSON 404.
- `/api/approval-action/{action_token}` uses its own one-tap token, POST/Host/Origin gates, and one-tap claim verification. Do not add Hub-token/PIN requirements or remove pending-approval identity validation.
- PIN is optional, digits only (6–32), bcrypt; 12-hour device-bound signed cookie plus in-memory nonce registry, cap 256. Per-key threshold 5, global threshold 30, escalating 1/5/30-minute lockouts; reserve attempts before verification. Loopback Tailscale login is hashed for limiter identity. Remote set-PIN/revoke-all additionally require an existing valid PIN cookie even when PIN is not yet enabled. Revoke-all rotates persisted token/secret and clears sessions only after save succeeds; stale UI connections and queued input stop, wrapper connections survive.
- Preserve pre-guard malformed-path behavior of provider dispatch (some unknown nested paths return 404 and unsupported methods 405 before auth); session prefixes authenticate token before parsing. The migration should not silently claim a uniform gate that the baseline does not have.
- Keep outbound public HTTPS transport separate from managed-service loopback HTTP. Reject private/reserved DNS results before any dial, dial the validated IP, preserve TLS host verification, and retain timeout/redirect/body limits. Fake transports/DNS only during acceptance preparation.

Sources: `http_helpers.go`, `pin_auth.go`, `auth_handlers.go`, `server.go:1643-1701,1891-2290`, `approval_action.go`, `ui_origin_cookie.go` and `orchestration.go:803`. JSON contains exact source references, contract expressions and current test links, including Host, DNS rebinding, PIN and revoke-epoch cases.

## Wire and body-limit traps

Global JSON errors are `{ok:false,error,detail?}`; default success `writeJSON` is HTTP 200 with application/json and a final JSON newline. Some endpoints return 204, raw HTML/media/files, or route-specific error structures, so there is no universal success envelope.

`decodeJSON` uses a 1 MiB MaxBytesReader, returns 400 `bad_request` / `invalid json` on decode errors including oversize, allows unknown fields, and does not check EOF after the first value. Do not replace it globally with a strict decoder or 413 default. Exceptions include Jev's 8 KiB strict unknown-field/EOF-checked JSON and 1–4096-byte text; files-save's 2 MiB complete JSON body plus 1 MiB content and explicit 413; Whisper control's optional body with a 1 MiB LimitReader.

Other caps: attachment file 10 MiB with 11 MiB multipart envelope (oversize 400); avatar 5 MiB (read failure 400); custom sound 2 MiB; provider icon 512 KiB (explicit 413); memo image 10 MiB and 500 MiB aggregate; voice audio 25 MiB (413 audio_too_large); WebSocket receive 4 MiB; files preview 1 MiB with truncation; file walk depth 8 and 2,000 entries. See constants and exact caller body-limit expressions for boundary/status tests.

Byte slices are base64. Numeric live IDs and signed 64-bit DB IDs are distinct. `omitempty` bool/int/string/slice/pointer behavior matters; allocated empty arrays differ from nil/null. Crucially, Go `time.Time` is a struct: a zero value tagged omitempty is still serialized, e.g. `filesSaveResp.mtime` on many errors. Preserve camelCase names (`readOnly`, `baseMtime`, `rawText`, `normalizedText`) alongside snake_case fields. Preserve RFC3339Nano precision for file mtime and field-specific string timestamp formatting.

Agent-chat uses a newline-aligned byte cursor; omitted/default -1 primes a bounded tail, `offset` is a compatibility alias, and cursor wins when both are present. Parser-only cursor tests are insufficient: follow adoption of next_cursor into the next Web poll. Routines/routine runs and memo lists intentionally return allocated empty arrays. Check all `accepted`, `excluded`, `messages`, `items`, `options`, `relays`, pointer booleans and time fields with absent/zero/empty goldens.

## Files, Git, attachments and history

Read and mutation scopes differ. Content/media/download accepts CWD/git root/attachments/orchestration. Recorded handoff memo is a special read-only files-content path. Outside roots, deny secret-like original AND canonical paths before loopback or mention fallback; then direct loopback may read readonly and logical remote requires a user-chat or memo mention. Assistant-only path mentions do not grant access. Mention-derived download retains its type gate. Writes stay in CWD/git roots, reject VCS metadata paths and validate canonical containment. Read-only mention access must never authorize write.

Missing/invalid/unknown `?session=` in Files CWD lookup falls back to Hub startup CWD; Git APIs use their own required session parser and resolve Git root explicitly. Files-save compares complete nanosecond mtime, returns conflict with current mtime, preserves existing mode, and does not create a missing file. Creation uses its own atomic-exclusive path. Do not claim save itself is atomic: baseline uses os.WriteFile. Rename/move/delete require synthetic symlink/rename interleaving tests and canonical root protection.

Git supports log/show/refs/status/diff/turns/turn-diff/commit-all/commit-message/fetch/pull/push. Retain revision/path argument validation, unborn/detached HEAD, limit/skip/default behavior, capped/sanitized command diagnostics and cancellation. Git-log default 100/max 1000; ordinary Git command timeout 5 seconds, network/generation paths have their own timeouts. Test in isolated synthetic repos/fake processes; no actual push or paid commit-message generation.

Session history APIs gracefully return existing empty shapes when sessionStore is absent. Queries keep live ID vs DB ID precedence and limits/order. Reset preserves active IDs and reports deferred file reset; transcript-noise pruning does not touch provider transcript files or user/approval/attachment messages. Attachments and log purge must protect live scope and preserve original count/error response shapes.

## Provider/profile/models/usage and two specified corrections

Inventory includes provider CRUD/validation, history/diff/reset/restore, backup list/verify/restore, recovery GET and POST, distribution status/diff/check/accept/rollback, icons, approval profiles/patterns, subscriptions add/update/remove/test/login, models, usage/auth refresh/probe, NIM, slash-command sources and remote defaults.

Profiles isolate vendor configuration directories through child environment overlay. Missing/disabled profiles fail; no-profile env remains byte-for-byte unchanged; live credentials never swap. No `ReadUsage` belongs on the subscription Adapter. Spawn-time auto selection remains round-robin, not quota-based. Session token totals, subscription usage observations, manual refresh, periodic opt-in, optional paid probe and freshness/auth status are separate contracts. NIM settings/results/errors never echo keys; environment-owned key cannot be replaced/deleted by API.

The migration instructions intentionally require corrections beyond a blind Go port:

1. First Codex seed: `seed.go:536` copies the whole source when destination is absent, bypassing the existing-destination state-key merge. `codex.go` marks the entire hooks table state-owned, which does not solve first-copy temporary hooks. Required behavior is to filter only application-owned temporary hook/session references while preserving user hooks, settings, nested TOML and authentication/state ownership. Test absent AND existing destinations. Do not remove all hooks.
2. Update command identity: `cli_update.go:139` saves the resolved launch candidate path while `provider/update.go:61` can return another update executable. `cli_update.go:440` runs the launch-resolved path with update args. Required launch=A/update=B behavior resolves/executes B consistently across eligibility, preview, displayed argv, logs and process; missing B cannot execute A. Synthetic regression is mandatory.

Other update contracts: six exclusion codes, queued/running/final outcomes, bounded 10-job in-memory history, max concurrency 8, 1 MiB job output and 256 KiB log response; registry snapshot, cancellation on Hub shutdown, before/after version observations, disabled/unconfigured/login-required cases and provider admission exclusion. These fixes are pending, not already implemented or validated.

## Routines, memos, preferences, notification and native services

Routines persist admission/idempotence before launch, use timezone/DST schedules and request-ID dedupe, match immutable launch label/Hub instance/DB ID, avoid replay on missing sessions, preserve completed result, and retry failed result persistence. Manual request ID is required/max 128 bytes; saved routines cap 200; run list returns newest first/max 200 and optional routine_id filter. After missing live observation for >90 seconds an active run becomes interrupted; standby >30 seconds without completion becomes waiting. Non-fallback done result is secret-masked, summary truncated and full result capped 256 KiB. Corrupt/version-mismatched store is not silently overwritten.

Memos: version-1 private atomic JSON, 500 entries, 4000-byte text limit, server-derived project git root, optional sparse PATCH pointers, done_at transitions, image basename validation and orphan cleanup after metadata commit. Memo images are immutable random names; image retrieval is guarded and privately cached. Preferences need full config UserPrefs wire mirror, defaults/effective values, copy semantics, template-version header/conflict gate, custom theme sanitization and reload persistence.

Notifications/push: explicit opt-in and filters, masked/bounded payloads, token-free URLs, pending/success dedupe with retry/error semantics, VAPID/subscription persistence; fake local transports only. Windows tray is a separate resident process surviving Hub stop; other OS behavior stays unsupported as before. Desktop/autostart evidence remains operator/OS acceptance.

Voice: raw audio bounds, empty/error handling, normalized hallucination filtering, cancellation/deadlines, loopback-only transport, endpoint fallback, native runtime/model integrity and redirect guards. Whisper installation/start/stop/uninstall/status need actual owned process lifecycle and failure rollback, never successful placeholders. No native download/install or external model/provider call is authorized by this preparation.

Supplementary endpoints are included, not deferred out of the inventory: bug-report preview/finalize (local redacted artifacts and explicit external-gist boundary), Jev evaluate, doctor, mobile/Tailscale status and serve, net-hint, picker/open/terminal configuration, shutdown/kill-all, encoding check, remote server profile connect/status/disconnect and profiles fetch. These require C4 or core integration rather than dropping their routes.

## WebSocket and frozen Web integration

`/ws` has 4 MiB receive cap; handshake validates Host/Origin, first message accepts message token or HTTP token fallback, applies remote PIN, then branches UI vs register/reattach wrapper. UI receives snapshot, bounded replay and reattach completion frames; registration/replay is guarded by auth epoch. UI types: pty_resize, pty_input, ui_active_session, approval_consumed, approval_resync, session_history_reset, session_dismiss, attach_request. Wrapper types include pty_data, pty_input_ack, session_end. Unknown types follow baseline ignore/close behavior. Complete Message DTO, emitted type literals, loops and Web mirror are recorded.

`web/src/app/util.ts:63` apiFetch adds bearer token if absent and defaults credentials to same-origin. `web/src/app/ws-client.ts:230` opens `/ws` without token in URL. Preserve embedded app-entry/app modules, debug/probe module import, styles, icons, manifest/service worker, worklet, locale modules/JSON and vendor files. Asset availability/identity must be checked after C1 embeds the real built bundle; no empty dashboard placeholder and no dependency on launch working directory.

## Acceptance work still required

For each registration AND dynamic method/path branch, instantiate the proposed route fixture IDs with synthetic golden requests/responses, auth ordering, unsupported methods/malformed suffixes, limit boundary, missing/empty/null values, errors, and actual core integration. Baseline test links are evidence locations, not test pass receipts. The 155-registration coverage assertion is necessary but insufficient: 49 suffix operations, conditional methods and generated response branches need real tests.

Specific gate groups: A02 protocol/auth/epoch; A05 storage/reload/failure; A07 canonical synthetic files/Git; A08 profile/usage/NIM; A09 updates and fake SSH; A10 served Web/mobile/PWA/rendering; A11 routine/session lifecycle; A12 notification/device/native/tray/OS. Native OS/provider/device/remote tests and copied-data backup/restore remain explicitly pending. No build, test suite, real service, external communication, credentials, install or Git mutation was performed by this worker.

## Registered-route matrix

Methods below are union observations for prefix dispatchers. Follow `dynamic_routes` and exact guard source for a particular suffix. Source link is the handler, not an assertion that all its behavior is ported.

| Registered path | Methods | Go handler source | Linked Web lines | Baseline tests (direct/helper) |
|---|---|---|---:|---:|
| `/` | GET | `internal/hub/server.go:1891` | 0 | 11 |
| `/app-entry.js` | GET, HEAD | `net/http.FileServer` | 0 | 0 |
| `/app.js` | GET, HEAD | `net/http.FileServer` | 0 | 2 |
| `/app/` | GET, HEAD | `net/http.FileServer` | 2 | 3 |
| `/debug/` | GET, HEAD | `net/http.FileServer` | 1 | 2 |
| `/styles.css` | GET, HEAD | `net/http.FileServer` | 0 | 1 |
| `/styles/` | GET, HEAD | `net/http.FileServer` | 0 | 2 |
| `/icon.svg` | GET, HEAD | `net/http.FileServer` | 0 | 0 |
| `/icons/` | GET, HEAD | `net/http.FileServer` | 0 | 0 |
| `/manifest.webmanifest` | GET, HEAD | `net/http.FileServer` | 0 | 0 |
| `/sw.js` | GET, HEAD | `net/http.FileServer` | 0 | 0 |
| `/whisper-recorder-worklet.js` | GET, HEAD | `net/http.FileServer` | 0 | 0 |
| `/i18n.js` | GET, HEAD | `net/http.FileServer` | 0 | 0 |
| `/i18n/` | GET, HEAD | `net/http.FileServer` | 0 | 2 |
| `/vendor/` | GET, HEAD | `net/http.FileServer` | 0 | 0 |
| `/approval-patterns/` | GET | `internal/hub/approval_patterns.go:342` | 4 | 3 |
| `/ws` | GET | `internal/hub/server.go:2116` | 9 | 7 |
| `/api/info` | GET | `internal/hub/misc_handlers.go:31` | 42 | 9 |
| `/api/jev/evaluate` | POST | `internal/hub/jev_evaluate.go:47` | 1 | 0 |
| `/api/providers` | GET, POST | `internal/hub/provider_handlers.go:155` | 16 | 17 |
| `/api/providers/` | DELETE, GET, PATCH, POST | `internal/hub/provider_handlers.go:219` | 13 | 24 |
| `/api/provider-distributions/` | GET, POST | `internal/hub/provider_distribution_handlers.go:18` | 0 | 8 |
| `/api/provider-icons/` | DELETE, GET, PUT | `internal/hub/provider_icon_handlers.go:111` | 4 | 12 |
| `/api/bug-report/preview` | POST | `internal/hub/bug_report_handler.go:123` | 2 | 8 |
| `/api/bug-report/finalize` | POST | `internal/hub/bug_report_handler.go:199` | 1 | 6 |
| `/api/doctor` | GET | `internal/hub/doctor_handler.go:11` | 1 | 0 |
| `/api/mobile-connect` | GET | `internal/hub/mobile_connect.go:29` | 7 | 12 |
| `/api/mobile-connect/tailscale` | GET | `internal/hub/tailscale_handlers.go:25` | 6 | 8 |
| `/api/mobile-connect/tailscale/serve` | DELETE, POST | `internal/hub/tailscale_handlers.go:56` | 4 | 3 |
| `/api/auth/revoke-all` | POST | `internal/hub/auth_handlers.go:72` | 1 | 5 |
| `/api/auth/status` | GET | `internal/hub/pin_auth.go:396` | 2 | 3 |
| `/api/auth/login` | POST | `internal/hub/pin_auth.go:426` | 1 | 7 |
| `/api/auth/logout` | POST | `internal/hub/auth_handlers.go:29` | 0 | 1 |
| `/api/auth/set-pin` | POST | `internal/hub/pin_auth.go:511` | 2 | 1 |
| `/api/net-hint` | POST | `internal/hub/misc_handlers.go:199` | 0 | 4 |
| `/api/avatar` | GET | `internal/hub/misc_handlers.go:247` | 3 | 3 |
| `/api/spawn` | POST | `internal/hub/spawn_handler.go:718` | 14 | 28 |
| `/api/spawn-grid` | POST | `internal/hub/spawn_handler.go:1396` | 1 | 1 |
| `/api/session/` | PATCH | `internal/hub/session_meta.go:133` | 0 | 2 |
| `/api/sessions/` | GET, PATCH, POST | `internal/hub/orchestration.go:969` | 8 | 23 |
| `/api/pick-directory` | POST | `internal/hub/misc_handlers.go:356` | 1 | 0 |
| `/api/path-exists` | POST | `internal/hub/misc_handlers.go:400` | 1 | 1 |
| `/api/list-subdirs` | POST | `internal/hub/misc_handlers.go:430` | 4 | 0 |
| `/api/pick-file` | POST | `internal/hub/misc_handlers.go:372` | 1 | 0 |
| `/api/open-default-file` | POST | `internal/hub/open_handlers.go:103` | 3 | 1 |
| `/api/open-folder` | POST | `internal/hub/open_handlers.go:125` | 3 | 0 |
| `/api/open-terminal` | POST | `internal/hub/open_handlers.go:151` | 1 | 1 |
| `/api/terminal-app` | GET, POST | `internal/hub/open_handlers.go:174` | 2 | 0 |
| `/api/kill-all` | POST | `internal/hub/lifecycle.go:21` | 2 | 2 |
| `/api/shutdown` | POST | `internal/hub/lifecycle.go:78` | 2 | 4 |
| `/api/log-config` | GET, POST | `internal/hub/settings_handlers.go:45` | 2 | 0 |
| `/api/session-chat` | GET | `internal/hub/session_store_handlers.go:11` | 2 | 0 |
| `/api/session-log` | GET | `internal/hub/session_log_handler.go:32` | 2 | 7 |
| `/api/agent-log` | GET | `internal/hub/agent_log_handler.go:40` | 2 | 0 |
| `/api/agent-log/open` | POST | `internal/hub/agent_log_handler.go:56` | 1 | 0 |
| `/api/agent-chat` | GET | `internal/hub/agent_chat_handler.go:26` | 3 | 3 |
| `/api/grok-history` | GET | `internal/hub/grok_history_handler.go:49` | 2 | 2 |
| `/api/session-search` | GET | `internal/hub/session_store_handlers.go:46` | 3 | 0 |
| `/api/session-history` | GET | `internal/hub/session_store_handlers.go:71` | 1 | 0 |
| `/api/approval-history` | GET | `internal/hub/session_store_handlers.go:141` | 3 | 2 |
| `/api/session-store/reset` | POST | `internal/hub/session_store_handlers.go:89` | 1 | 0 |
| `/api/session-store/prune-transcript-noise` | POST | `internal/hub/session_store_handlers.go:115` | 0 | 0 |
| `/api/logs/purge` | POST | `internal/hub/purge_handlers.go:37` | 2 | 1 |
| `/api/logs/legacy-notice` | GET, POST | `internal/hub/purge_handlers.go:133` | 3 | 0 |
| `/api/attachments/purge` | POST | `internal/hub/purge_handlers.go:168` | 2 | 0 |
| `/api/open-dir` | POST | `internal/hub/misc_handlers.go:593` | 2 | 1 |
| `/api/idle-timeout` | GET, POST | `internal/hub/settings_handlers.go:182` | 2 | 0 |
| `/api/terminal-color` | GET, POST | `internal/hub/settings_handlers.go:116` | 2 | 0 |
| `/api/handoff-intent-mode` | GET, POST | `internal/hub/settings_handlers.go:152` | 2 | 0 |
| `/api/handoff` | GET | `internal/hub/handoff_handler.go:20` | 9 | 6 |
| `/api/handoff/` | GET, POST | `internal/hub/handoff_handler.go:47` | 5 | 4 |
| `/api/reconnect-grace` | GET, POST | `internal/hub/settings_handlers.go:317` | 2 | 0 |
| `/api/input-config` | GET, POST | `internal/hub/settings_handlers.go:352` | 2 | 0 |
| `/api/orchestration-config` | GET, POST | `internal/hub/settings_handlers.go:386` | 3 | 2 |
| `/api/subscriptions` | GET, POST | `internal/hub/subscriptions_handlers.go:25` | 7 | 7 |
| `/api/subscriptions/` | GET, POST | `internal/hub/subscriptions_handlers.go:38` | 5 | 3 |
| `/api/subscription-usage` | GET | `internal/hub/subscription_usage_handlers.go:39` | 4 | 0 |
| `/api/subscription-usage/refresh` | POST | `internal/hub/subscription_usage_handlers.go:53` | 1 | 0 |
| `/api/subscription-usage/probe` | DELETE, POST | `internal/hub/subscription_usage_handlers.go:131` | 2 | 0 |
| `/api/notify-config` | GET, POST | `internal/hub/settings_handlers.go:225` | 2 | 0 |
| `/api/notify-test` | POST | `internal/hub/settings_handlers.go:266` | 1 | 0 |
| `/api/notify-generate-topic` | POST | `internal/hub/settings_handlers.go:305` | 1 | 0 |
| `/api/nvidia-nim` | GET, PUT | `internal/hub/nvidia_nim_handlers.go:39` | 5 | 6 |
| `/api/nvidia-nim/key` | DELETE | `internal/hub/nvidia_nim_handlers.go:101` | 1 | 2 |
| `/api/nvidia-nim/test` | POST | `internal/hub/nvidia_nim_handlers.go:127` | 1 | 2 |
| `/api/encoding-check` | GET | `internal/hub/encoding_check.go:21` | 1 | 0 |
| `/api/approval/status` | GET | `internal/hub/approval_handler.go:417` | 2 | 0 |
| `/api/approval/enable` | POST | `internal/hub/approval_handler.go:431` | 1 | 0 |
| `/api/approval/disable` | POST | `internal/hub/approval_handler.go:446` | 1 | 0 |
| `/api/approval/dismiss` | POST | `internal/hub/approval_handler.go:461` | 1 | 0 |
| `/api/approval/batch` | POST | `internal/hub/approval_batch.go:56` | 1 | 6 |
| `/api/approval-action/` | POST | `internal/hub/approval_action.go:166` | 1 | 5 |
| `/api/attach` | POST | `internal/hub/misc_handlers.go:287` | 3 | 1 |
| `/api/slash-cmd-sources` | GET, POST | `internal/hub/slash_handlers.go:60` | 3 | 2 |
| `/api/slash-commands` | GET, POST | `internal/hub/slash_handlers.go:142` | 4 | 1 |
| `/api/usage-link-defaults` | GET | `internal/hub/slash_handlers.go:279` | 1 | 0 |
| `/api/install-link-defaults` | GET | `internal/hub/slash_handlers.go:290` | 1 | 0 |
| `/api/cli-versions` | GET, POST | `internal/hub/cli_version.go:379` | 1 | 3 |
| `/api/cli-updates` | POST | `internal/hub/cli_update.go:608` | 4 | 1 |
| `/api/cli-updates/` | GET | `internal/hub/cli_update.go:658` | 2 | 0 |
| `/api/cli-update-eligibility` | GET | `internal/hub/cli_update.go:725` | 2 | 0 |
| `/api/models` | GET, POST | `internal/hub/models_handlers.go:13` | 2 | 1 |
| `/api/approval-patterns` | GET | `internal/hub/approval_patterns.go:362` | 4 | 0 |
| `/api/approval-patterns/` | GET, POST, PUT | `internal/hub/approval_patterns.go:377` | 4 | 0 |
| `/api/files-list` | GET | `internal/hub/files_list.go:62` | 4 | 5 |
| `/api/files-content` | GET | `internal/hub/files_content.go:303` | 4 | 14 |
| `/api/files-asset` | GET | `internal/hub/files_content.go:458` | 1 | 1 |
| `/api/files-download` | GET | `internal/hub/files_content.go:398` | 1 | 8 |
| `/api/files-roots` | GET | `internal/hub/files_roots.go:24` | 0 | 0 |
| `/api/files-move` | POST | `internal/hub/files_move.go:235` | 1 | 12 |
| `/api/files-rename` | POST | `internal/hub/files_rename.go:36` | 1 | 10 |
| `/api/files-mkdir` | POST | `internal/hub/files_mkdir.go:38` | 1 | 8 |
| `/api/files-create` | POST | `internal/hub/files_create.go:28` | 1 | 6 |
| `/api/files-save` | POST | `internal/hub/files_save.go:44` | 2 | 13 |
| `/api/files-delete-dir` | POST | `internal/hub/files_delete.go:23` | 1 | 7 |
| `/api/git-log` | GET | `internal/hub/git_log.go:46` | 2 | 0 |
| `/api/git-show` | GET | `internal/hub/git_show.go:46` | 2 | 0 |
| `/api/git-refs` | GET | `internal/hub/git_refs.go:20` | 2 | 0 |
| `/api/git-status` | GET | `internal/hub/git_status.go:38` | 4 | 0 |
| `/api/git-diff` | GET | `internal/hub/git_diff.go:36` | 2 | 0 |
| `/api/git-turns` | GET | `internal/hub/git_turns.go:513` | 4 | 1 |
| `/api/git-turn-diff` | GET | `internal/hub/git_turns.go:551` | 2 | 1 |
| `/api/git-commit-all` | POST | `internal/hub/git_commit.go:52` | 1 | 0 |
| `/api/git-commit-message` | POST | `internal/hub/git_commit.go:135` | 2 | 0 |
| `/api/git-fetch` | POST | `internal/hub/git_fetch.go:25` | 1 | 0 |
| `/api/git-pull` | POST | `internal/hub/git_pull.go:27` | 1 | 0 |
| `/api/git-push` | POST | `internal/hub/git_push.go:34` | 1 | 0 |
| `/api/user-prefs/notify-sound-custom` | GET, PUT | `internal/hub/user_prefs_handlers.go:198` | 3 | 2 |
| `/api/user-prefs/avatar` | DELETE, PUT | `internal/hub/user_prefs_handlers.go:37` | 1 | 2 |
| `/api/user-prefs` | GET, PUT | `internal/hub/user_prefs_handlers.go:86` | 26 | 10 |
| `/api/routines` | DELETE, GET, POST, PUT | `internal/hub/routine_handlers.go:18` | 4 | 5 |
| `/api/routines/` | DELETE, GET, POST, PUT | `internal/hub/routine_handlers.go:18` | 2 | 5 |
| `/api/routine-runs` | GET | `internal/hub/routine_handlers.go:156` | 2 | 5 |
| `/api/routine-runs/` | GET | `internal/hub/routine_handlers.go:156` | 1 | 5 |
| `/api/memos` | DELETE, GET, PATCH, POST | `internal/hub/memo_handlers.go:20` | 6 | 11 |
| `/api/memos/` | DELETE, GET, PATCH, POST | `internal/hub/memo_handlers.go:20` | 3 | 11 |
| `/api/memo-images` | GET, POST | `internal/hub/memo_images.go:155` | 2 | 3 |
| `/api/memo-images/` | GET, POST | `internal/hub/memo_images.go:155` | 1 | 3 |
| `/api/auto-approval/status` | GET | `internal/hub/auto_approval.go:233` | 0 | 0 |
| `/api/auto-approval/simulate` | GET | `internal/hub/auto_approval.go:250` | 1 | 1 |
| `/api/push/status` | GET | `internal/hub/push.go:460` | 1 | 0 |
| `/api/push/vapid-public-key` | GET | `internal/hub/push.go:471` | 1 | 0 |
| `/api/push/subscriptions` | DELETE, POST | `internal/hub/push.go:482` | 2 | 0 |
| `/api/voice/transcribe` | POST | `internal/hub/voice_transcribe.go:36` | 1 | 3 |
| `/api/whisper/status` | GET | `internal/hub/whisper_manage.go:237` | 1 | 1 |
| `/api/whisper/install` | POST | `internal/hub/whisper_manage.go:244` | 0 | 0 |
| `/api/whisper/uninstall` | POST | `internal/hub/whisper_manage.go:286` | 0 | 0 |
| `/api/whisper/start` | POST | `internal/hub/whisper_manage.go:326` | 0 | 0 |
| `/api/whisper/stop` | POST | `internal/hub/whisper_manage.go:364` | 0 | 0 |
| `/api/session-usage` | POST | `internal/hub/usage_stat.go:325` | 0 | 0 |
| `/api/servers` | GET, POST | `internal/hub/servers.go:137` | 7 | 0 |
| `/api/servers/connect` | POST | `internal/hub/servers.go:194` | 3 | 0 |
| `/api/servers/connect/status` | GET | `internal/hub/servers.go:267` | 2 | 0 |
| `/api/servers/disconnect` | POST | `internal/hub/servers.go:295` | 1 | 0 |
| `/api/profiles/fetch` | POST | `internal/hub/profiles_fetch.go:34` | 1 | 0 |

## Dynamic suffix/operation matrix

These are logical operation patterns, not new ServeMux registrations. Whitespace/slash trimming, provider validation and extra-segment acceptance must match each dispatcher source exactly.

| Pattern | Methods | Handler |
|---|---|---|
| `/approval-patterns/{provider}[.official|.custom].json` | GET | `handleApprovalPatternAsset` |
| `/api/providers/validate` | POST | `handleProviderValidate` |
| `/api/providers/{id}` | GET | `handleProviderDetail` |
| `/api/providers/{id}` | PATCH | `handleProviderPatch` |
| `/api/providers/{id}` | DELETE | `handleProviderDelete` |
| `/api/providers/{id}/history` | GET | `handleProviderHistory` |
| `/api/providers/{id}/history/{revision}/diff` | GET | `handleProviderHistoryDiff` |
| `/api/providers/{id}/reset` | POST | `handleProviderReset` |
| `/api/providers/{id}/restore` | POST | `handleProviderRestore` |
| `/api/providers/{id}/backups` | GET | `handleProviderBackups` |
| `/api/providers/{id}/backups/{backup}/verify` | GET | `handleProviderBackupVerify` |
| `/api/providers/{id}/backups/{backup}/restore` | POST | `handleProviderBackupRestore` |
| `/api/providers/{id}/recovery` | GET, POST | `handleProviderRecovery` |
| `/api/provider-distributions/status` | GET | `handleProviderDistributionStatus` |
| `/api/provider-distributions/diff` | GET | `handleProviderDistributionDiff` |
| `/api/provider-distributions/check` | POST | `handleProviderDistributionCheck` |
| `/api/provider-distributions/accept` | POST | `handleProviderDistributionAccept` |
| `/api/provider-distributions/rollback` | POST | `handleProviderDistributionRollback` |
| `/api/provider-icons/{id}` | GET, PUT, DELETE | `handleProviderIcon` |
| `/api/session/{id}/meta` | PATCH | `handleSessionMeta` |
| `/api/sessions/{id}/meta` | PATCH | `handleSessionMeta` |
| `/api/sessions/{id}/spawn-child` | POST | `handleSpawnChild` |
| `/api/sessions/{id}/spawn-confirm` | POST | `handleSpawnConfirmation` |
| `/api/sessions/{id}/send-child` | POST | `handleSendChild` |
| `/api/sessions/{id}/inject` | POST | `handleSessionInject` |
| `/api/sessions/{id}/children` | GET | `handleSessionChildren` |
| `/api/sessions/{id}/relay` | GET | `handleRelayGet` |
| `/api/sessions/{id}/relay` | POST | `handleRelayStart` |
| `/api/sessions/{id}/relay-stop` | POST | `handleRelayStop` |
| `/api/sessions/{id}/relay-resume` | POST | `handleRelayResume` |
| `/api/sessions/{id}/relay-cleanup` | POST | `handleRelayCleanup` |
| `/api/handoff/{id}` | GET | `handleHandoffItem` |
| `/api/handoff/{id}/note` | POST | `handleHandoffNoteRequest` |
| `/api/handoff/{id}/manual-note` | POST | `handleHandoffManualNoteSave` |
| `/api/subscriptions/update` | POST | `handleSubscriptionUpdate` |
| `/api/subscriptions/remove` | POST | `handleSubscriptionRemove` |
| `/api/subscriptions/test` | POST | `handleSubscriptionTest` |
| `/api/subscriptions/login` | POST | `handleSubscriptionLogin` |
| `/api/approval-action/{action_token}` | POST | `handleOneTapApproval` |
| `/api/cli-updates/{job_id}` | GET | `handleCLIUpdateJobStatus` |
| `/api/cli-updates/{job_id}/{provider_id}/log` | GET | `handleCLIUpdateJobLog` |
| `/api/approval-patterns/profile` | GET, POST | `handleApprovalProfile` |
| `/api/approval-patterns/copy-official` | POST | `handleApprovalCopyOfficial` |
| `/api/approval-patterns/{provider}[/{ignored_suffix}]` | PUT | `handleApprovalPatternsItem` |
| `/api/routines/{id}` | GET, PUT, DELETE | `handleRoutines` |
| `/api/routines/{id}/runs` | POST | `handleRoutines` |
| `/api/routine-runs/{id}` | GET | `handleRoutineRuns` |
| `/api/memos/{id}` | PATCH, DELETE | `handleMemos` |
| `/api/memo-images/{basename}` | GET | `handleMemoImages` |

## Validation receipt and change ownership

Performed: read-only source/index/guide inspection; Python lexical extraction; JSON parse/schema-reference/count checks. All 155 registrations resolve; proposed fixture IDs are unique. No application build/test/runtime acceptance executed. Early source-lookup guesses for several filenames returned file-not-found; the actual locations were resolved from the source index/functions and are the ones in the artifact. No uncertainty from those failed lookups is presented as missing behavior.

Owned changes only: `rust/inventory/services.json`, `rust/inventory/services-notes.md`.
Next integration action: review/freeze the shared interfaces, then assign service implementation ownership and connect the real core/delivery implementations. The two correction expectations need explicit regression goldens before port acceptance.

The integration owner removed duplicated whole source bodies and the regenerable whole-Hub graph before publication; read the exact fixed-SHA sources for full implementations. Per-function source hashes and all 155 registrations remain.
