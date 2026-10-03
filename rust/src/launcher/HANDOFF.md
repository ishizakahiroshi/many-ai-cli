# Launcher implementation handoff (C4 / task #3)

Go oracle: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`.
Instruction start: `6b0fb8e198750245ef8b4b475fbac9070db0ac3e`, including current README/00–06 and PROGRESS supplements read at implementation.

Status: the reusable launcher slice is implemented and Linux synthetic tests pass; full binary/Hub wiring, two concrete implementation follow-ons below, strict global lint and native/distribution acceptance remain open. This is not whole-application completion.

## Public call graph

- Standalone launcher parses using the existing `cli::parse_launcher`, configures console and prints `startup_banner(version)`. Load `LauncherStore`, call `ProfilesFile::validate`; UI/default -> `UiServer::new(manager).serve(cancel)`, print/open its `url`, wait for SIGINT/SIGTERM, cancel and await `UiServerHandle::wait`. Explicit profile/last -> `ProfilesFile::select`, then `connect_cli`.
- Main `connect` uses the same select/connect flow, without standalone banner; retain the main CLI's existing missing-flag error. The integration owner alone changes binary dispatch.
- `connect_cli` owns startup lock, existing-instance reuse, active registration, browser callback, connector cancellation and terminal lifetime. Supply `ConnectorConfig::with_console()` for CLI/UI output; quiet is the default for Hub callers.
- Hub Servers: one `Arc<ConnectionManager>` supports `profiles`, `replace_profiles`, `connect`, `status`, `disconnect`, `close_all`. Trim connect names in Hub (the standalone UI intentionally doesn't). `LauncherError` carries HTTP status, existing Hub error code and detail. UI errors retain the different launcher envelope `{ok:false,error:detail}`.
- Hub `/api/profiles/fetch`: Go-compatible decode `FetchParams`, trim, require host, `ConnectorConfig::fetch_remote_profile`, then `complete_fetched_profile` with the current file. It returns form data only; saving stays explicit. This route does not exist in the standalone Go launcher UI.
- Main `profile-export`: use `build_export_profile(ExportOptions, ExportIdentity::local())`, pretty JSON plus newline. Tests inject identity instead of enumerating the executor's environment/network.
- The shared `RuntimePaths`, YAML bridge, Go JSON decoder/time helpers, process driver and `files::safe_fs::Dir` are reused. No Go delegation or alternate configuration/process owner.

## Trial and ownership

Every local file/lock is under a held capability for the explicit RuntimePaths root. Trial connections require an explicit `RemoteTrial::new(root, binary)`; no local root or home is silently interpreted as a remote path. Both remote fields must be absolute. Candidate remote argv carry the shared leading `--trial-root` and `--trial-port` pair. The integration owner must expose an explicit way to supply the remote candidate scope for trial connections; production defaults remain unchanged.

Active registry ownership remains `(profile, PID)` with PID plus Hub probe double guard. Cross-process file locks overlap the legacy Go lock. Startup locks keep the same first-eight-SHA256 naming. Capability-backed private writes protect symlink/rename boundaries. Read-modify-write profile saves now also take a cross-process lock. Shutdown waits for connector cleanup. Reconnect waits for the prior attempt's scoped cleanup before starting a replacement.

## Explicit source differences / source quirks retained

- Required supplement repair: SSH serve/import quote both embedded values and the complete script passed through OpenSSH join to bash. The Go oracle omitted the outer quote. Tests execute both outer-shell parsing and the actual synthetic child. OpenSSH's `ssh.c` parses `--` after the destination in its second option pass, then joins remaining remote argv with spaces: https://github.com/openssh/openssh-portable/blob/master/ssh.c . The fixture models this observed source boundary; no real host connection is used.
- `token_command` remains an entire user-supplied remote shell script, unchanged by an extra bash interpreter layer.
- Trial remote launch uses a unique per-connection argv[0] marker via bash exec -a. Production argv stays unchanged. Cleanup ERE is anchored, includes candidate root, marker and binary, and rejects port-prefix matches. Go used an unanchored binary/port pattern. No cleanup ever matches all processes by program name.
- Forwarded Hub URLs are restricted to explicit IPv4-loopback HTTP endpoints and redirects are rejected, avoiding opaque-token transmission to an arbitrary registry URL. Diagnostics omit token-bearing remote URLs/raw SSH stderr.
- Startup-lock stale replacement is serialized, and release checks timestamp plus PID. This closes the Go create-before-write and same-process stale-handle races. Concurrent prune preserves newer records rather than overwriting them after an asynchronous probe.
- Profile historical reads do not invoke the new-input validator. Unknown fields follow Go's ignore/drop-on-save behavior; supported values and nil/empty distinctions are preserved. Shared YAML defensive limits remain explicit.
- Go banner's literal `Runtime: Windows` is retained on every OS. Existing npm shim returns exit 1 for a terminating signal despite a nearby comment claiming 128+signal; source behavior remains the delivery contract.
- Native SSH receives `-t`, matching the Go argument exactly. Native TTY/SIGHUP behavior is not accepted by a fake SSH process.

## Source to tests

- profile.go/connect.go -> profile.rs/persistence.rs/manager.rs; historical YAML, selection, validation, private atomic failure, concurrency, actual Go corpus.
- active.go/filelock/PID -> active.rs/shared process::pid_alive; double guard, stale locks, concurrent writes, independent roots, capability-root replacement.
- SSH/WSL/import/export/scan/shell/port -> commands.rs/connector.rs/exchange.rs/network.rs; actual Go pure corpora, fake-SSH serve retries/exit/cancel/import/tunnel, opaque token URL/readiness, outer shell + actual child argv/env/cwd.
- ui_server.go/Hub servers.go -> ui.rs/manager.rs; real synthetic loopback server and embedded UI bytes, auth ordering, first-value JSON, owned disconnect after remote failure, lifetime timeout.
- banner/browser/console -> platform.rs; Go byte-exact banner, browser argv, native OS adapters (real-device pending).
- .goreleaser.yaml/npm staging/release inputs -> delivery.rs; four targets, channel file differences, eight-binary manifest, assets/runtime provenance, per-file readback hashes and artifacts.json adapter. No real candidate package is produced by synthetic metadata fixtures.

## Coordinator integration requests

1. Register `pub mod launcher`; retain exclusive Cargo/bin/router ownership. Windows features needed: Console, WindowsProgramming, IpHelper, WinSock and Ndis, in addition to the shared process driver features.
2. Wire both binaries and main profile-export, all Hub Servers/fetch callers, native signal cancellation and manager `close_all` into shutdown. Preserve the existing launcher UI asset.
3. Establish an explicit trial-remote-root/binary input, distinct from production profile fields; reject missing trial remote scope rather than touching a daily remote instance.
4. Clean native Rust fmt/clippy/tests/builds for Windows x64, Linux x64 and both macOS architectures. Rust source metadata checks do not prove native console/WSL/process behavior.
5. Candidate artifacts must record actual source revision, lockfile SHA, toolchain, build time/version, both binaries, every embedded asset and verified runtime input. `DeliveryManifest::goreleaser_artifacts` supplies existing npm staging metadata; npm intentionally stages only the main binary.
6. Existing release ZIP/deb/rpm ship both binaries; Homebrew/winget consume matching archives. Preserve Windows resources/version/icon/manifest, docs/licenses/notices, unblock helper, hashes/signature/certificate and SBOM expectations. Separate uncredentialed dependency/build/package jobs from hash-verified publish-only jobs. Do not rebuild under publish credentials.
7. Existing `scripts/smoke-npm.mjs` uses `pkg.includes('win32')` although package names use `windows-x64`; its Windows filename expectation needs a coordinator-owned correction. It also allows unstaged binaries/skip as success, so candidate acceptance must separately reject missing targets/zero staged bytes.

## Evidence and pending gates

- Actual Go corpus generation: exit 0, 129 cases from 37 source-hash-verified Go inputs, private temporary module and existing pinned Go 1.26.8/cache, GOPROXY=off. The generator does not read a daily home or connect SSH.
- Rustfmt on all owned source/test files: exit 0 (parse/format only).
- `cargo test --locked --offline --manifest-path rust/Cargo.toml --test launcher_contracts -- --test-threads=4`: exit 0, **24 passed, 0 failed, 0 ignored**, including all 129 Go observations. Compile 24.30 seconds; tests 0.85 seconds. Complete logs retained outside Git.
- `cargo clippy --locked --offline --manifest-path rust/Cargo.toml --test launcher_contracts -- -D warnings`: exit 101, blocked by 19 unowned terminal journal/session findings (derive/type complexity/collapsible-if/large enum). No launcher-source finding was emitted. The launcher test target's lint pass is not established because library lint failed first. Complete diagnostics retained outside Git.
- Both commands used the installed Rust 1.90.0, existing CARGO_HOME, the shared compact runtime-target, CARGO_INCREMENTAL=0, CARGO_PROFILE_DEV_DEBUG=0, CARGO_PROFILE_TEST_DEBUG=0 and CARGO_BUILD_JOBS=2. Each command had a 180-second outer bound. Cargo was explicitly released to the application/service-routes lane after the lint result.
- First shared source check found sha2 0.11's digest lacks LowerHex. Replaced digest formatting with explicit zero-padded byte hex before the successful test run. A manager nonminimal-bool lint found by the preceding lane was also corrected before that run. No test assertion or safety guard was weakened.
- No real SSH, browser/device UI, Windows WSL/console/Job/ACL, macOS native launcher, clean four-target artifacts, release runtime signature/ABI, npm real-binary smoke, copied-data rollback or cutover acceptance. All remain pending.

## Concrete follow-on implementation gaps (not merely unperformed acceptance)

- Windows hostname snapshot currently calls `GetComputerNameW`. The pinned Go 1.26.8 `os/sys_windows.go` uses `GetComputerNameExW(ComputerNamePhysicalDnsHostname)` with a growing buffer. The integration owner must make that narrow correction and source/native-check it; the current adapter may return a different clustered/long host name. `Win32_System_SystemInformation` exposes the correct API in windows-sys.
- Launcher HTTP transport has a 30-second header timeout and bounded 30-second body read, but it does not yet implement the exact Go per-request 60-second write deadline, nor combine header+body into Go's single absolute 30-second read deadline. Shared transport deadline integration needs a focused caller test before claiming complete UI transport parity.

## Executed test IDs

`launcher_actual_go_oracle_contracts` (129 observations), `launcher_profile_roundtrip_selection_and_failed_replace`, `launcher_load_does_not_validate_or_reset_historical_profile`, `launcher_active_concurrent_updates_and_root_isolation`, `launcher_active_double_guard_and_stale_lock`, `launcher_persistence_rejects_symlink_targets_and_pins_root`, `launcher_export_candidates_order_and_import_completion`, `launcher_remote_scope_wsl_and_cleanup_are_bounded`, `launcher_fake_ssh_serve_retry_ready_lifetime_and_cleanup`, `launcher_fake_ssh_early_exit_and_import`, `launcher_manager_timeout_only_bounds_connecting_and_closes_own_connection`, `launcher_ui_auth_shapes_and_real_loopback_asset`, `launcher_tokens_scanner_limits_and_browser_argv`, `launcher_ssh_join_and_remote_shell_deliver_exact_argv_env_cwd`, `launcher_fake_tunnel_opaque_token_readiness_redirect_and_quiet`, `launcher_ui_disconnect_failure_still_cleans_owned_only`, `launcher_delivery_target_channel_contract_and_hash_readback`, `launcher_trial_cleanup_distinguishes_profiles_even_on_same_port`, `launcher_registry_subprocess_helper` (helper used by six child processes), `launcher_registry_cross_process_updates_do_not_lose_records`, `launcher_manager_superseded_attempt_cannot_overwrite_or_remove_new_connection`, `launcher_registry_readiness_rejects_redirect_without_following_token`, `launcher_cli_caller_reuses_active_and_cleans_on_signal_cancellation`, `launcher_ui_asset_head_range_and_conditional_match_servefile`.

Integration receipt: the fixed registered slice passes430 tests and strict all-target clippy. Windows hostname source was corrected to the pinned Go PhysicalDnsHostname/growing-buffer API after the initial24-test receipt; native execution remains pending. The exact60-second launcher response write deadline and full binary/Hub wiring remain open.
