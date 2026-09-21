---
type: architecture-component
title: Configuration and On-Disk Layout
description: How config.yaml is loaded, defaulted, migrated, and atomically saved by internal/config, and what lives under ~/.many-ai-cli.
tags: [config, configyaml, internal-config, atomic-write, migration, schema]
sources:
  - id: openwiki-source-a8910515ddd14810ad43f5c1
    resource: repo://internal/config/config.go
  - id: openwiki-source-2ca17b5cb95fde6517958229
    resource: repo://internal/config/subscription.go
  - id: openwiki-source-f683e0b10daf979d1b0b7c74
    resource: repo://internal/hub/orchestration.go
  - id: openwiki-source-3735fd619becc2d912d40b6c
    resource: repo://internal/hub/push.go
  - id: openwiki-source-9443ba695e07d37176b3ebad
    resource: repo://internal/hub/user_prefs_handlers.go
  - id: openwiki-source-622d0b47281c620a9a87f705
    resource: repo://internal/hubruntime/runtime.go
  - id: openwiki-source-cb383a97d856afdec98a8fbe
    resource: repo://internal/launcher/profile.go
generated: { by: "claude-code", at: "2026-09-21T12:35:03.565Z" }
verified:
  - by: openwiki/0.5.0
    at: 2026-09-21T12:35:03.565Z
---

## `~/.many-ai-cli/`: the on-disk root

Every piece of Hub-owned state lives under one per-user directory, `~/.many-ai-cli/` (`internal/config.Dir()`), created with restrictive permissions on first run. Besides `config.yaml`, notable files/subdirectories include the runtime ledger `hub-runtime.json` (see [CLI Entrypoints and Subcommands](/openwiki/architecture/cli-entrypoints.md)), `logs/`, `launcher-profiles.yaml`, `push_store.json` (VAPID keys and Web Push subscriptions — kept out of `config.yaml` deliberately), `subscriptions/<provider>/<id>/` (per-profile CLI config directories), `orchestration/<id>/` (board and relay files), and `attachments/`.

If a legacy `~/.any-ai-cli` directory exists and the new `~/.many-ai-cli` directory does not, `migrateLegacyDir` moves it in place with `os.Rename` (not a copy) the first time `LoadOrCreate` runs, as a one-time compatibility step for the `any-ai-cli` → `many-ai-cli` rename; if the new directory already exists, or the rename fails, this is a no-op and startup continues either way.

## Load/create/repair: `config.LoadOrCreate()`

`LoadOrCreate()` is the single entry point the CLI and Hub both call at startup. Its behavior branches on what it finds at `config.yaml`:

- **File exists and parses** — the file's values are unmarshaled over a `defaultConfig(home)` baseline (so any field the file omits keeps its default), then a chain of post-load normalizers runs (see below).
- **File exists but fails to parse** — the corrupt bytes are backed up to `config.yaml.bak` (atomically written), a fresh default config is generated, given a token, and written back, and startup continues on the fresh config rather than failing outright.
- **File does not exist** — a default config is generated, given a token, and written to disk.
- **Any other read error** (permissions, I/O) — returned as a hard error; an unreadable-but-present file is never silently treated as absent, so an existing configuration is never quietly discarded.

After loading, if `cfg.Token` is still empty (e.g., an old config predating the token field) `ensureToken` fills and persists it — token generation is state, so it must survive process restarts rather than being regenerated in memory every launch (which would invalidate the Hub URL and any already-open wrapper sessions). A one-time migration also moves `cfg.Spawn.LastModel` into `cfg.UserPrefs.Spawn.LastModel` for any provider not already present at the new location.

`LoadOrCreate` finishes by running normalizers/defaults for terminal color, handoff intent mode, slash-command sources, approval-pattern sources, and approval profiles, then `cfg.applyDefaults()`, then `cfg.Validate()` — an invalid config after all of that is a hard startup error.

## Atomic writes

Every config write — the initial default, the corrupt-config backup, the token backfill, and `Save(cfg)` — goes through the same `writeConfigAtomic(dir, path, out)` helper: write to a `config-*.yaml.tmp` file in the *same directory* as the target, `Sync()`, `chmod 0o600`, then `os.Rename` over the final path. A same-volume rename is effectively atomic, so a crash or power loss mid-write can never leave `config.yaml` partially written/corrupted — the reader either sees the old file or the fully-written new one. On Windows, `os.Chmod(0o600)` does not narrow the NTFS DACL the way POSIX permissions would, so `writeConfigAtomic` additionally calls `securefile.RestrictFile(path)` after the rename to explicitly restrict the ACL, since `config.yaml` holds the auth token, `AuthCookieSecret`, and `RemotePINHash`; a failure to restrict the ACL is logged-and-ignored rather than failing the write, because the write itself already succeeded.

`Save(cfg)` — used whenever the Hub or CLI persists a config change at runtime — reapplies `applyDefaults()` and `Validate()` before writing, so a save can never persist a config that would fail validation on the next load.

## Schema shape

`config.yaml`'s top-level sections (see `internal/config/config.go`'s `Config` struct) group by subsystem: `hub` (port, browser auto-open, auto-shutdown, log dir, idle timeout, wrapper reconnect grace, loopback/token exceptions, trusted networks/allowed hosts, `env_kind`), `log` (rotation size/backups/compression, plus opt-in PTY session logging with its own retention/size caps), `approval`, `models_source`, `ollama`/`lmstudio` base URLs, `voice.whisper`, `notify` (outbound ntfy/webhook backends, separate from Web Push), `slash_cmd_sources` / `approval_pattern_sources` / `approval_profiles` (per-provider remote-sync URLs and official-vs-custom profile selection), `local_models`, `token`, `user_prefs` (the large bag of UI-facing preferences: notification sound, quick commands, usage links, voice grace period, display theme/locale and user-defined custom themes, session/group ordering, favorites, spawn defaults, avatar), `orchestration` (board notify mode, spawn confirm mode, child full-bypass, worktree isolation, per-parent child limits), and `custom_providers`.

Session logs, the SQLite-era session-history store, launcher profiles, and Web Push subscription state are deliberately kept in their own files rather than folded into `config.yaml` — `config.yaml` is meant to stay small enough to hand-edit (as the [Custom Providers](/openwiki/extensibility/custom-providers.md) workflow expects) and to avoid mixing frequently-changing high-volume data with the low-churn settings document that also carries the auth token.

`user_prefs.display.custom_themes` holds up to 20 named themes (a Light/Dark base plus a hue of 0–359 and a contrast of 0–100, each with an id starting `u-`). The Hub does not trust the browser's copy: `PUT /api/user-prefs` runs `SanitizeCustomThemes` (drops invalid or duplicate ids and empty names, clamps hue/contrast, caps the count) and then `SanitizeDisplayTheme`, which falls the active theme back to `light` if it names a custom id that no longer exists.

## Custom providers live in the same file

`custom_providers:` — the mechanism a user hand-edits to register an arbitrary AI CLI as a spawn option — is a field on the same `Config` struct and goes through the same `LoadOrCreate`/`Save`/`Validate` path as every other setting; there is no separate file or Hub UI writer for it. See [Custom Providers](/openwiki/extensibility/custom-providers.md) for the id/command syntax rules `internal/config/custom_provider_command.go` enforces.

## Consumption pattern

`internal/config.Config` is a plain struct passed by pointer into `hub.NewServer`, `doctor.Run`, `wrapper.Run`, and the other subcommand entry points shown in [CLI Entrypoints and Subcommands](/openwiki/architecture/cli-entrypoints.md) — there is no global/singleton config accessor; each subcommand loads it once via `LoadOrCreate()` in `main.run()` and threads it down explicitly. `Config.Clone()` returns a deep copy (individually copying map/slice fields) specifically so a copy can be handed to `Save` without holding the Hub's config mutex, avoiding a concurrent-map-iteration panic during `yaml.Marshal` if another goroutine mutates the live config at the same time.
