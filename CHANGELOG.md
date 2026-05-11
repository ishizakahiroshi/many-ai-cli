# Changelog

All notable changes to **ai-cli-hub** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Release artifacts are published at
<https://github.com/ishizakahiroshi/ai-cli-hub/releases>.

## [Unreleased]

### Added
- Documented the current Windows distribution limitation: release binaries are
  not Authenticode-signed yet, so Windows 11 Smart App Control may block
  `ai-cli-hub.exe` on PCs where that protection is enabled.

## [0.1.3] - 2026-05-11

### Fixed
- Codex approval prompts that follow the repository-standard plain
  `(Y:1/N:0)` format are now detected even when they are not wrapped in
  `[AI-CLI-HUB]` markers, so the Hub action bar appears for those
  confirmations.
- The favicon approval badge redraws after the base icon finishes loading,
  preventing missed pending-count indicators during initial page load.
- Voice input no longer opens a second live microphone stream just to animate
  the waveform, avoiding conflicts with the browser Speech Recognition
  microphone capture.

### Changed
- Local `dev` builds now derive the displayed version from the nearest Git tag
  when run from the repository, while release builds still use the GoReleaser
  `main.version` ldflags value as the source of truth.

## [0.1.2] - 2026-05-11

### Added
- **Single source of truth for the version string.** `cmd/ai-cli-hub/main.go`
  declares `var version = "dev"`, populated at release-build time by GoReleaser
  via `-X main.version={{.Version}}`. The Hub server returns it from
  `/api/info`, and the Web UI fetches it on load to render in the settings
  panel and the About panel. See `docs/manual_release.md` for the full design
  and the items still bumped manually (winres metadata, README references).
- `.gitattributes` pinning `THIRD_PARTY_NOTICES.md` and
  `web/src/vendor/THIRD_PARTY_LICENSES.txt` to LF, so the third-party check
  is byte-stable across `core.autocrlf` settings on Windows runners.

### Fixed
- Hub marker filter emits `\x1b[J` (erase-display-below) after a
  `[/AI-CLI-HUB]` end marker, so prompt remnants below the action-bar marker
  are cleared instead of leaving stale glyphs behind.
- `TestBaseName` is OS-neutral via `filepath.Join` instead of a hard-coded
  Windows path. Linux CI runners no longer fail because `\` is treated as a
  literal character there.
- `scripts/local/gen-third-party-notices.ps1` normalizes embedded LICENSE
  line endings to LF and writes the output via `WriteAllText` with explicit
  LF, eliminating the OS-dependent drift that previously flagged the file
  as outdated only on CI.

### Changed
- Hardcoded version strings removed from `web/src/index.html` and
  `web/src/i18n/{ja,en}.json`. The About-panel translation uses the i18n
  placeholder `{version}` and gets resolved at runtime.
- `.gitignore` ignores the entire `.claude/` directory (was only filtering
  `settings.json`), since `worktrees/` and `scheduled_tasks.lock` are also
  per-developer state.
- `winres/winres.json` and the regenerated `cmd/ai-cli-hub/rsrc_windows_*.syso`
  reflect 0.1.2, so the Windows .exe Properties dialog matches the runtime
  version.

## [0.1.1] - 2026-05-11

Initial official public release. v0.1.0 was an experimental pre-release that
was never published; its commit history was rewritten away during v0.1.1
preparation, so v0.1.1 is the earliest version visible on GitHub.

### Added
- Hub server (`ai-cli-hub serve`) with xterm.js Web UI:
  - Live PTY output streaming.
  - Action-bar approval detection from xterm.js buffer scans.
  - Approval response routed back to PTY; Hub UI dismisses the action-bar
    when approval is resolved by direct terminal input.
  - Image attach (paste / drag-and-drop → local save → PTY inject).
  - Slash-command capture for Claude Code (Ctrl+O folded sections).
  - Session spawn from `/api/spawn`.
  - Approval pattern editor and approval-rules opt-in.
- Wrapper subcommands `ai-cli-hub claude` / `ai-cli-hub codex` that
  auto-launch the Hub when not already running and connect to it.
- GoReleaser distribution for Windows / Linux / macOS (amd64) and
  macOS (arm64). Single Go binary per platform.
- `SHA256SUMS.txt` is signed with cosign keyless signing.
- Idle timeout, log rotation, slash-command fetch, settings panel.

### Notes
- Real-environment verification: Windows. Linux/macOS builds are produced
  but not deeply validated.
- Gemini CLI is intentionally out of scope for wrapping; see
  `docs/v0.1.x-ai-cli-hub-design.md` for the rationale.

[Unreleased]: https://github.com/ishizakahiroshi/ai-cli-hub/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/ishizakahiroshi/ai-cli-hub/releases/tag/v0.1.3
[0.1.2]: https://github.com/ishizakahiroshi/ai-cli-hub/releases/tag/v0.1.2
[0.1.1]: https://github.com/ishizakahiroshi/ai-cli-hub/releases/tag/v0.1.1
