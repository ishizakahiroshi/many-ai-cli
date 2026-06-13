# Changelog

All notable changes to **many-ai-cli** (formerly `any-ai-cli`) are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Release artifacts are published at
<https://github.com/ishizakahiroshi/many-ai-cli/releases>.

## [Unreleased]

## [0.3.1] - 2026-06-14

### Fixed
- The approval action bar no longer renders a tall empty column to the left of
  the question panel. In the batch / single-question / multi-select layouts the
  `APPROVAL NEEDED` label kept `flex-basis: 100%` (intended for the wrapped row
  layout), which in the `flex-direction: column` variants behaves as a
  100%-height basis and pushed the question tabs/panel into a second wrapped
  column. The label now uses `flex-basis: auto` and the column layouts set
  `flex-wrap: nowrap`.

### Changed
- The Cursor Agent slash-command resource
  (`resources/slash-commands/cursor-agent.md`) is now populated from the
  official Cursor CLI documentation (about 22 commands such as `/plan`, `/ask`,
  `/model`, `/auto-run`, `/sandbox`, `/compress`) instead of the previous
  4-command placeholder.

## [0.3.0] - 2026-06-13

### Added
- **npm registry distribution.** `many-ai-cli` is now installable with
  `pnpm add -g many-ai-cli` (fallbacks: `bun install -g` / `npm install -g`).
  Each platform's native Go binary ships in an optional dependency package
  (`many-ai-cli-<os>-<arch>`), so no standalone exe is downloaded in a browser
  and no Mark-of-the-Web SmartScreen prompt is triggered. The release workflow
  stages the GoReleaser binaries into the npm packages and publishes them with
  provenance.
- Windows release archives now include `unblock-windows.cmd`, a local helper
  that runs PowerShell `Unblock-File` for `many-ai-cli*.exe` files after zip
  extraction.
- **Detached Session Grid.** AI and Shell sessions can be popped out into a
  separate browser window as a standalone grid (with presets such as
  "Claude + Shell 2x2" / "Shell 3x3"); the Hub keeps managing approvals and
  session state from the main window.
- **Shell sessions.** A plain interactive shell (PowerShell / bash / sh) can be
  spawned as a regular Hub session alongside AI sessions; AI-only features
  (approval injection, Chat, token bar) are disabled automatically for shell
  sessions.
- **Always-on status bar.** A single bottom line shows the active session's
  state, provider/model, work label, project/branch and changed-file count,
  context-window gauge, token counts, prompt-cache hit rate, per-session and
  daily cost, burn rate, elapsed time, connection state, and a fleet badge
  (toggleable in Settings). Token/cost segments are populated for Claude and
  Codex sessions.
- **Model picker with Ollama routing.** The spawn form can select Anthropic /
  OpenAI / Ollama Cloud / Ollama Local models; the Hub injects the matching
  `ANTHROPIC_*` / `OPENAI_*` environment variables per session, with no shell
  setup required.
- **Voice input.** Prompts can be dictated through Browser speech recognition or
  a local Whisper server, including a Windows x64 managed whisper.cpp installer
  and an iPhone-Safari → Hub → PC Whisper relay, with near-silence and
  hallucination-phrase filtering on the Hub side.
- **Mobile / smartphone access.** The Hub UI is responsive (single-column layout
  on narrow viewports, touch-sized controls, and a mobile key panel for
  Esc/Ctrl/arrows) and documents reaching it from a phone over an SSH local
  forward; no public exposure is required or supported.
- **Outbound approval notifications (ntfy / webhook).** In addition to Web Push,
  the Hub can POST approval notifications to an ntfy topic or a generic webhook,
  which still works when no browser tunnel is connected. The Hub token is never
  included in the payload.
- **Token-less loopback access.** Requests from a `127.0.0.1` origin can reach
  the Hub UI without the URL token, configurable via an allowed-hosts setting;
  remote and tunnelled access still require the token.
- **`uninstall` subcommand.** `many-ai-cli uninstall [--purge]` removes settings
  and logs, and with `--purge` also removes the binary itself.
- **Unified launcher with connection profiles (cross-platform).**
  `many-ai-cli-launcher` handles WSL, SSH `serve`, and SSH `tunnel` profiles
  from one profile file and can reconnect to resident remote Hubs without
  restarting the remote session. The launcher binary now ships for Windows,
  Linux, and macOS (in every release archive and in the deb/rpm/Homebrew
  packages): SSH `serve`/`tunnel` profiles work on all three, while `wsl`
  profiles remain Windows-only and report a clear error elsewhere.
- **Workbench tab and session history.** The Hub now keeps a SQLite-backed
  session store and exposes stored sessions, timeline events, summaries,
  redacted exports, prompt templates, task/policy notes, diagnostics, usage
  summaries, stale-session views, file context helpers, and worktree helpers.
- **PWA and opt-in Web Push notifications.** The web UI ships a manifest,
  icons, service worker, push subscription settings, local VAPID key storage,
  and approval notifications that omit the Hub URL token.
- **Remote server / Docker deployment assets.** GHCR image publication, loopback-only
  per-user compose samples, resource limits, health checks, and an idempotent
  `aac-update.sh` workflow support resident remote Hub operation.
- **Files and Git operations.** The Files tab can create folders, save text
  files with base-mtime conflict detection, and delete empty folders. The Git
  view can fetch refs, run `git pull --ff-only`, and run a guarded plain
  `git push` (force/tag/delete disabled, non-interactive, confirmation dialog
  showing branch name and ahead count) from guarded Hub endpoints.
- **Additional provider coverage.** GitHub Copilot CLI and Cursor Agent CLI
  approval patterns, slash-command resources, usage links, and instruction
  injection paths are documented and wired into the Hub UI.

### Changed
- **Renamed the project from `any-ai-cli` to `many-ai-cli`** (binary, config
  directory `~/.many-ai-cli/`, `MANY_AI_CLI*` environment variables, approval
  markers `[MANY-AI-CLI]`, Hub banner, and all public docs/UI strings). The npm
  package name `any-ai-cli` was too similar to an existing package, so the
  project is published as `many-ai-cli`.
- README and release-operation docs now distinguish Mark-of-the-Web,
  SmartScreen, Smart App Control, organization policy, and checksum/signature
  verification for unsigned Windows builds, prefer package-manager installation
  when available, and keep the GitHub Releases zip as the manual fallback.
- The public README files now describe v0.3.0 features, platform validation,
  Docker deployment, PWA/Web Push behavior, and Go 1.25 build requirements.
- The Release workflow now lets GoReleaser's before-hooks perform the frontend
  install/build once, avoiding a duplicate `npm ci` + `npm run build` step.
- Windows resource metadata is bumped to `0.3.0` so executable properties and
  manifests match the release tag.
- Browser-side third-party notices now match the xterm 6.0.0 package versions
  used by the current frontend type/dependency set.

### Fixed
- Switching between sessions stays responsive after long-running connections:
  per-session pending output is now capped so a backlog no longer makes the
  first paint after a tab switch sluggish.
- Terminal output is no longer duplicated when the PTY size and the xterm.js
  render size briefly disagree during a window resize.
- Numerous terminal, approval, and voice UI refinements across xterm rendering,
  action-bar positioning/clearing, multi-pane interactions, Codex/Claude prompt
  detection, and voice diagnostics.
- GoReleaser no longer creates a Windows arm64 archive containing only
  the launcher binary and no matching main binary; Windows release
  archives remain x64-only until both binaries support arm64 together.
- SSH tunnel profile URLs now URL-encode Hub tokens for `/api/info`,
  `/api/net-hint`, and the browser URL, so manually configured tokens cannot
  break query parsing.
- The Hub status/probe URLs now also URL-encode tokens before local HTTP
  requests.
- Terminal output filtering now handles synchronized update control sequences
  without batching repaint output until the next user input.
- Public Docker samples no longer contain a personal user/service name.

### Removed
- **`any-ai-cli-wsl.exe` (standalone WSL launcher).** The unified Windows
  launcher `any-ai-cli-launcher.exe` fully replaces it: a `wsl` profile
  provides the same behavior (`bash -ilc` startup, `ANY_AI_CLI_WSL_LAUNCHER=1`,
  automatic port selection, browser open, and WSL-side orphan cleanup). The
  Windows release zip now ships `any-ai-cli.exe` and `any-ai-cli-launcher.exe`.

## [0.2.2] - 2026-05-24

### Changed
- Release archive names are now easier to read for end users: `windows-x64`,
  `linux-x64`, `macos-intel`, and `macos-apple-silicon` replace the previous
  `windows-amd64` / `linux-amd64` / `darwin-amd64` / `darwin-arm64` names.
  README download tables and WSL install instructions were updated to match.
- Windows executable file properties (`FileVersion` / `ProductVersion`) now
  report the release version instead of the stale `0.2.0` value.

## [0.2.0] - 2026-05-24

### Added
- **Files tab.** Project groups now open a persistent Files tab with a
  2-pane file tree and Markdown/code preview. Text preview uses vendored
  `marked.js`, `DOMPurify`, and highlight.js, with context actions for opening,
  copying paths, moving files, and renaming files. Existing tabs are restored
  per project after Hub restart.
- **Read-only Git view.** Clicking a session card's branch badge opens a Git
  tab with commit history, commit details, changed files, truncated diffs, ref
  switching (local / remote / tag / `--all`), row context-copy actions, and
  tab restore.
- **Commit all from the Hub.** The Git tab can stage all working-tree changes
  with `git add -A` and create a local commit after a Review step. The modal
  supports subject/body editing and commit-message generation. Push is never
  run by this action.
- **Chat history, split view, and multi-pane monitoring.** The unified tab bar
  now includes Terminal / Chat / Split / Multi / Files / Git. Chat history is
  built from the live PTY stream, Split keeps it beside the terminal, and Multi
  can show several sessions in a grid.
- **Multi-question approval action bar.** A single `[ANY-AI-CLI]` approval block
  can contain multiple numbered questions. The Hub renders them as stacked
  choices with progress, clear, keyboard navigation, and "Submit all"; selected
  answers are sent back to the PTY as a space-separated digit string.
- **Approval pattern profiles with remote sync.** Approval trigger phrases are
  fetched from `resources/approval-patterns/{claude,codex,common}.md` on
  GitHub at Hub startup (24h TTL). Each provider now has an `official`
  read-only profile and a user-editable `custom` profile, with Settings UI to
  switch profiles and copy official patterns into custom.
- **Server-side user preferences.** Voice, notification sound, avatar,
  approval auto-switch, quick commands, usage links, favorites, session order,
  and spawn defaults are stored under `user_prefs:` in
  `~/.any-ai-cli/config.yaml` via `GET/PUT /api/user-prefs`, so they survive
  port changes and WSL launcher use.
- **User avatar customization.** The chat view can show a configured user icon
  or display-name initial, stored through server-side user preferences.
- **WSL launcher.** A new Windows-only `any-ai-cli-wsl.exe` starts
  `any-ai-cli serve` inside WSL, chooses a Windows-side-safe port when needed,
  opens the Windows browser, sets `ANY_AI_CLI_WSL_LAUNCHER=1`, and cleans up
  WSL-side orphan `serve` processes on launcher exit.
- **Clean transcript command.** `any-ai-cli log-clean <session.jsonl>
  [-o transcript.txt]` generates the same ANSI/control-code-stripped transcript
  format that the Hub writes automatically on session end.

### Changed
- **Release artifacts now match the documented install flow.** The Windows
  GoReleaser zip now includes both `any-ai-cli.exe` and the WSL launcher
  `any-ai-cli-wsl.exe`; README install instructions now refer to the actual
  zip artifacts rather than standalone executable names.
- **Internal docs browser names were renamed to Files.** Public API paths moved
  from `/api/docs-*` to `/api/files-*`, with one-shot browser storage migration
  from old docs keys to files keys.
- **Hub auto-spawn now opens a dedicated console window on Windows.** When a
  wrap command (`any-ai-cli claude` / `codex` / `wrap <provider>`) auto-starts
  the Hub via `ensureHub`, the spawned `any-ai-cli serve` process is now
  created with `CREATE_NEW_CONSOLE`, so the ANY-AI logo banner and the
  "WARNING: This window is connected to the Web UI. Do not close it." line
  appear in a visible terminal window titled `any-ai-cli [hub]`. Previously
  the child inherited the parent's console and the banner was overwritten
  by the wrapper's PTY output, leaving the Hub effectively invisible. Unix
  behavior is unchanged: stdout/stderr stay inherited from the parent
  terminal.
- **Startup banner warning is now English-only and uses reverse video.** The
  Japanese warning "注意: この画面は Web UI と連結しています。閉じないでください。"
  was replaced with the English equivalent. The blink ANSI code (`\x1b[5m`)
  was dropped because Windows Terminal / ConPTY / VS Code integrated
  terminals ignore it; the warning now uses Bold + Reverse Video + Bright
  Orange so it stands out as an orange highlight bar on any terminal.
- Terminal auto-follow and wheel scrolling were simplified around the
  xterm.js bottom state, reducing snap-back after submit, approval resolution,
  and session switches.
- UI terminology, i18n keys, localStorage keys, and CSS classes now consistently
  use "files" rather than "docs" for the project file browser.
- Documented the Windows distribution limitation: release binaries are not
  Authenticode-signed yet, so Windows 11 Smart App Control may block
  `any-ai-cli.exe` on PCs where that protection is enabled.

### Fixed
- Release metadata was aligned for v0.2.0: Windows resource JSON and regenerated
  `.syso` files now report `0.2.0` / `0.2.0.0` instead of the stale v0.1.3
  values in the Windows Properties dialog.
- Files tab Markdown preview links render again with the vendored marked v12
  renderer signature. Relative Markdown/text links now keep their visible text
  and are routed through the Files preview link handlers.
- Release-build console noise was removed from the Web UI by dropping the
  leftover app build marker and voice-input `[VOICE-DBG]` logs while keeping
  the in-app voice diagnostic event history.
- Browser-side third-party license notices now include the current highlight.js
  copyright line from the vendored header.
- Release guidance no longer points at the missing `docs/any-ai-cli-design-v0.1.0.md`
  file; agent and release docs now use `docs/v0.2.0-any-ai-cli-design.md` as the
  current design source of truth.
- Codex and Claude approval detection now catches additional native prompt
  shapes, including free-form numbered choices and Codex approval prompts that
  appear while the terminal is not already scrolled to the bottom.
- Approval bars clear more reliably after direct terminal input, session
  switching, `/clear`, and action submission.
- File preview scrolling, empty tree messages, search filtering of child nodes,
  parent-git-root selection, restored orphan tabs, and cross-filesystem
  relative-path copy behavior were corrected.
- Windows GUI-spawned Claude/Codex sessions no longer disconnect immediately
  when inherited PATH entries are stale, empty, or contain `%VAR%`-style user
  path segments such as `%PNPM_HOME%\bin`.
- WSL integration fixes include folder picking through Windows dialogs,
  opening WSL files/directories with Windows handlers, defaulting launcher logs
  to the Windows user profile when launched from `any-ai-cli-wsl.exe`, and
  correct banner/logo rendering with East Asian width and console mode handling.
- Voice input fixes include avoiding competing microphone captures, normalizing
  trigger phrases, adding a diagnostic panel, and surfacing save-error details
  for user preferences. Wake-word code remains hidden/disabled in v0.2.0.
- Reverse-video terminal output no longer becomes unreadable white blocks in
  the Hub terminal theme.

## [0.1.3] - 2026-05-11

### Fixed
- Codex approval prompts that follow the repository-standard plain
  `(Y:1/N:0)` format are now detected even when they are not wrapped in
  `[ANY-AI-CLI]` markers, so the Hub action bar appears for those
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
- **Single source of truth for the version string.** `cmd/any-ai-cli/main.go`
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
  `[/ANY-AI-CLI]` end marker, so prompt remnants below the action-bar marker
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
- `winres/winres.json` and the regenerated `cmd/any-ai-cli/rsrc_windows_*.syso`
  reflect 0.1.2, so the Windows .exe Properties dialog matches the runtime
  version.

## [0.1.1] - 2026-05-11

Initial official public release. v0.1.0 was an experimental pre-release that
was never published; its commit history was rewritten away during v0.1.1
preparation, so v0.1.1 is the earliest version visible on GitHub.

### Added
- Hub server (`any-ai-cli serve`) with xterm.js Web UI:
  - Live PTY output streaming.
  - Action-bar approval detection from xterm.js buffer scans.
  - Approval response routed back to PTY; Hub UI dismisses the action-bar
    when approval is resolved by direct terminal input.
  - Image attach (paste / drag-and-drop → local save → PTY inject).
  - Slash-command capture for Claude Code (Ctrl+O folded sections).
  - Session spawn from `/api/spawn`.
  - Approval pattern editor and approval-rules opt-in.
- Wrapper subcommands `any-ai-cli claude` / `any-ai-cli codex` that
  auto-launch the Hub when not already running and connect to it.
- GoReleaser distribution for Windows / Linux / macOS (amd64) and
  macOS (arm64). Single Go binary per platform.
- `SHA256SUMS.txt` is signed with cosign keyless signing.
- Idle timeout, log rotation, slash-command fetch, settings panel.

### Notes
- Real-environment verification: Windows. Linux/macOS builds are produced
  but not deeply validated.
- Gemini CLI is intentionally out of scope for wrapping; see
  `docs/v0.2.0-any-ai-cli-design.md` for the rationale.

[Unreleased]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.3.1...HEAD
[0.3.1]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.2.0...v0.2.2
[0.2.0]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/ishizakahiroshi/many-ai-cli/releases/tag/v0.1.3
[0.1.2]: https://github.com/ishizakahiroshi/many-ai-cli/releases/tag/v0.1.2
[0.1.1]: https://github.com/ishizakahiroshi/many-ai-cli/releases/tag/v0.1.1
