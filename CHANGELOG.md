# Changelog

All notable changes to **any-ai-cli** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Release artifacts are published at
<https://github.com/ishizakahiroshi/any-ai-cli/releases>.

## [Unreleased]

### Added
- セッションカードの branch バッジクリックで読み取り専用の Git ビューを開けるようにした（メインタブとして独立寿命、Hub 再起動で復元）
- 詳細パネル (INFORMATION / CHANGES / FILES) で commit の author/message/parents/refs/変更ファイル/diff を 1 画面表示
- 表示対象 ref の切替（local / remote / tag / `--all`）— checkout は行わない安全な読み取り専用切替
- 行右クリックで Copy メニュー（short/full hash, subject, message, hash + subject, GitHub link）
- カード右クリックメニュー（Open Git View / Open Files Tab / Activate Session / Copy session ID）
- キーボードショートカット: `Ctrl+Shift+G` で現セッションの Git タブ、`Ctrl+Shift+F` で Files タブを open
- カード右上の open 中マーカー（⎇ = Git タブ open 中、📁 = Files タブ open 中）

### Backend
- `GET /api/git-log?ref=<ref>&limit=<n>&skip=<n>` 新設
- `GET /api/git-show?hash=<full>` 新設（diff は 1 ファイル 256KB 上限で truncate）
- `GET /api/git-refs` 新設

### Added
- **Remote sync + profile system for approval detection patterns.** Approval
  trigger phrases are now fetched from
  `resources/approval-patterns/{claude,codex,common}.md` on GitHub at Hub
  startup (24h TTL — refreshed on next restart). Each provider has two
  profiles, **official** (read-only, kept in sync with the remote md) and
  **custom** (user-editable). The Settings panel adds a Profile dropdown
  and a "Copy from official" button; the previous "Reset to defaults"
  button has been removed (switch back to the official profile instead).
  When the official profile changes via remote sync, a toast notifies the
  UI (`approval_patterns_updated` WS event). New endpoints:
  `GET/POST /api/approval-patterns/profile`,
  `POST /api/approval-patterns/copy-official`. New config keys:
  `approval_pattern_sources` (per-provider source URL override) and
  `approval_profiles` (per-provider active profile). On-disk layout in
  `~/.any-ai-cli/approval-patterns/` becomes
  `<provider>.{official,custom}.json`; legacy `<provider>.json` is migrated
  to `<provider>.custom.json` on first startup and continues to be written
  as a mirror of the active profile for backwards compatibility.
- **Batch approval UI for multi-question prompts.** AI can now place multiple
  questions inside a single `[ANY-AI-CLI]` block (each with its own numbered
  options); the Hub action-bar renders them as a vertical stack with
  per-section selection buttons, a progress counter, a "Clear" button, and a
  "Submit all" button. Keyboard: digit keys select the option for the focused
  section and auto-advance; Tab / Shift+Tab / ←/→ move between sections; Space
  advances to the next section; Enter submits when every section is selected.
  On submit, the choices are concatenated into a space-separated digit string
  (e.g. `1 2 1 3`) followed by `\r` and sent directly to the PTY, so the AI
  can recover each answer with a simple `split()`. The single-question
  format and Y/N format are unchanged. approval-rules.md is bumped to
  version 4 to advertise the new block layout to AI sessions.

### Changed
- **Internal identifiers renamed from `docs` to `files`** (no user-visible
  behavior change). The `📁 files` button (formerly `📁 docs`) and surrounding
  UI labels were already updated in a prior session; this change aligns the
  internal codebase with the UI. Affected: HTTP API paths
  (`/api/docs-list` → `/api/files-list`, plus `-content` / `-roots` / `-move`),
  Go handler / type / constant names (`handleDocs*` → `handleFiles*`,
  `docsListItem` → `filesListItem`, etc.), JS class names
  (`DocsTabManager` → `FilesTabManager` etc.), HTML id / CSS class /
  `data-docs-*` attributes, i18n keys (`docs_*` → `files_*`), and
  `localStorage` keys (`any-ai-cli.docs.tabs` → `any-ai-cli.files.tabs`,
  `any_ai_cli_docs_tree_width` → `any_ai_cli_files_tree_width`; one-shot
  migration is performed on first load so existing tab state and tree pane
  width carry over).

### Added
- **Docs browser v2: tabbed 2-pane viewer.** The header "📄 Documents" button
  and flat dropdown list (v1) are replaced with a full 2-pane viewer:
  1. Entry point moved from the global header to each project group row in the
     left sidebar as a `📁 docs` link, so the target project is unambiguous.
  2. A small modal lets users pick the docs root per project (auto-detects
     `docs/` under the git root; falls back to a custom-path input).
  3. The main area gains a tab bar so terminal sessions and one or more Docs
     tabs co-exist and are switchable without losing context.
  4. Each Docs tab shows a left file/folder tree and a right Markdown preview
     panel (rendered via vendored `marked.js`, sanitized with `DOMPurify`,
     read-only).
  Previously opened tabs are persisted to `localStorage` per git-root and
  restored automatically when the Hub restarts.

### Changed
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
- **Project renamed from `ai-cli-hub` to `any-ai-cli`** to better reflect the
  goal of supporting arbitrary AI CLI tools (beyond Claude Code and Codex CLI).
  Affected names: binary (`any-ai-cli`), Go module path (`any-ai-cli`),
  environment variable (`ANY_AI_CLI_AUTO`), config directory
  (`~/.any-ai-cli/`), action-bar marker (`[ANY-AI-CLI]`). Existing v0.1.x
  users will need to migrate `~/.ai-cli-hub/` to `~/.any-ai-cli/` and update
  any shell init blocks (`eval "$(any-ai-cli shell-init)"`).

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
  `docs/v0.1.x-any-ai-cli-design.md` for the rationale.

[Unreleased]: https://github.com/ishizakahiroshi/any-ai-cli/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/ishizakahiroshi/any-ai-cli/releases/tag/v0.1.3
[0.1.2]: https://github.com/ishizakahiroshi/any-ai-cli/releases/tag/v0.1.2
[0.1.1]: https://github.com/ishizakahiroshi/any-ai-cli/releases/tag/v0.1.1
