---
type: architecture-component
title: Session Logging, Transcripts, and Attach
description: The Hub's structured log file, the opt-in raw PTY session log with secret masking, clean transcript generation, the SQLite session-history store, and the file/image attachment store used by paste-and-drop.
tags: [logging, transcript, sessionstore, sqlite, attach, secrets, mask-secrets]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-8970ecf6bbb35d8f0bd12797
    resource: repo://internal/attach/store.go
  - id: openwiki-source-b3b67094b286e55952c7bbfa
    resource: repo://internal/log/log.go
  - id: openwiki-source-194bfe6410841e9f0a3235f9
    resource: repo://internal/sessionlog/sessionlog.go
  - id: openwiki-source-492c9654cfe2d087656a4a40
    resource: repo://internal/sessionlog/transcript.go
  - id: openwiki-source-a1d29c10dd77b25f0094e895
    resource: repo://internal/sessionstore/store.go
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## `internal/log`: the Hub's own structured log

`log.NewFileLogger` builds the `slog.Logger` the Hub and CLI subcommands use for their own operational log (`hub.log`, rotated via `lumberjack`). When `cfg.Log.Enabled` is false it falls back to stderr-only; passing `alsoStderr=true` produces a `MultiWriter` of file plus stderr (used for `serve`'s own console so a directly-launched Hub shows activity in its terminal); `debug=true` raises the level to `slog.LevelDebug`, otherwise `slog.LevelInfo`. This is a separate log from the per-session PTY logging described next — it is the Hub's own diagnostic trail, not a record of what an AI CLI said.

## `internal/sessionlog`: opt-in raw PTY logging with secret masking

Raw per-session PTY output logging is off by default (`config.yaml`'s `log.session_enabled: false` — see [Configuration and On-Disk Layout](/openwiki/architecture/configuration.md)) precisely because a raw terminal capture can contain secrets in plaintext. When enabled, every logged line is passed through `sessionlog`'s `secretPatterns` — a deliberately heuristic denylist of known secret-shaped substrings (`API_KEY=...`, `PASSWORD=`/`SECRET=`/`TOKEN=`-style assignments requiring a 6+ character value to avoid over-masking ordinary text, credential-bearing URLs) — before being written to disk. The package's own comment is explicit about the tradeoff: this is a heuristic, so over-masking is accepted and under-masking (a secret shape not on the list) is an accepted residual risk rather than a guarantee. Most patterns preserve their matched prefix (the key name) and replace only the value with `***`, so a masked log line still shows *which* variable held a secret without revealing its value.

## Clean transcript generation

`sessionlog`'s transcript generation (invoked automatically, or manually via `many-ai-cli log-clean <session.jsonl>` — see [CLI Entrypoints and Subcommands](/openwiki/architecture/cli-entrypoints.md)) turns the raw `.jsonl` session log into a readable `.txt` transcript by stripping ANSI control sequences (`controlRE`) and collapsing runs of 3+ blank lines, and — the same logic mirrored on the frontend in `chat-history.ts`'s `isThinkingNoiseLine` (see [Session Views](/openwiki/frontend/session-views.md)) — dropping an AI CLI's animated "thinking..." status-line redraw frames entirely, since ANSI-stripping alone leaves large numbers of near-duplicate spinner frames (e.g. repeated `"✳ Imploring… (12s · ↑3.2k tokens · esc to interrupt)"` redraws) cluttering what should read as a conversation transcript. Detection is deliberately conservative, keyed on strong signals (an `"esc to interrupt"` phrase, a token-count progress bar, "thinking" combined with a spinner glyph, or a dense run of spinner glyphs) using a narrow glyph range (star dingbats U+2722–U+273F, Braille patterns U+2800–U+28FF) specifically chosen because ordinary conversational punctuation and commonly-used dingbats like ✓/✗ never fall in that range, avoiding false-positive deletion of real conversation text that happens to contain a checkmark or similar symbol.

## `internal/sessionstore`: the SQLite session-history store

Session history (the searchable record behind `/api/session-search`, distinct from the raw per-session `.jsonl`/`.txt` logs) is kept in a SQLite database via `modernc.org/sqlite` (a pure-Go driver, avoiding a CGO dependency). Every new connection applies a fixed sequence of `PRAGMA`s, and the package's own comment is explicit about why order matters: `PRAGMA busy_timeout` must be the very first pragma issued, because it defaults to 0 (fail immediately on `SQLITE_BUSY` rather than waiting), and any pragma issued *before* `busy_timeout` is set would itself fail immediately under lock contention rather than waiting for the timeout — `journal_mode=WAL` (issued during `init()`, which creates a `-wal` file) is exactly the kind of pragma that can contend with another process that already has the database open, which is why the ordering guard matters in practice, not just in theory.

## `internal/attach`: pasted/dropped file and image storage

`internal/attach` is the storage layer behind the "paste or drag-and-drop images and files into the terminal session" feature — not related to reconnecting a wrapper to a running PTY (that reattach flow is `/ws`'s `Type: "reattach"` role, covered in [Hub HTTP/WebSocket Server and Session Registry](/openwiki/hub/server-and-sessions.md)). `attach.Save(baseDir, sessionID, provider, data, filename)` determines the file's extension from the supplied filename first, falling back to sniffing the actual file content's magic bytes (recognizing PNG/JPEG/GIF/WebP/PDF, defaulting to `.bin` otherwise) when the filename doesn't yield one — so a browser-side rename or a missing extension does not cause a misidentified file type. Each save writes to a per-session subdirectory (`baseDir/<sessionID>/`) using a timestamp-plus-nanosecond base name and `os.OpenFile` with `O_CREATE|O_EXCL`, retrying with an incrementing numeric suffix on any collision, so two attachments saved in the same process tick never overwrite one another. `Save` also returns an `inject` string — the provider-specific text form of the saved path meant to be typed into the session's input so the wrapped AI CLI is told about the new file. Retention is handled by `CleanOld(baseDir, retentionDays)`, which removes files older than the configured retention window and then prunes any session subdirectory left empty, and — symmetric with how `retentionDays <= 0` disables day-based deletion entirely rather than being treated as "expire everything" — a parallel total-size enforcement path exists with the same `<= 0` means-disabled convention, so a 0/negative config value is deliberately a safe no-op in both mechanisms rather than an edge case that would delete everything.
