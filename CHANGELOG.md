# Changelog

All notable changes to **many-ai-cli** (formerly `any-ai-cli`) are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Release artifacts are published at
<https://github.com/ishizakahiroshi/many-ai-cli/releases>.

## [Unreleased]

### Added
- **A toggle decides what picking a prompt template does.** The template
  palette above the input bar now has a **Send on select** switch: off (the
  default, and the previous behavior) drops the template into the input box so
  you can edit it first; on sends it immediately through the normal send path.
  A one-line note under the switch states which mode is active. The setting is
  stored server-side as `user_prefs.template_send.immediate`, so it follows you
  across browsers and devices (`web/src/app/prompt-templates.ts`,
  `internal/config/config.go`).

## [0.6.0] - 2026-08-11

### Added
- **Turn-by-turn diff review in the Review tab.** A new **Scope** selector
  lists "Uncommitted changes (vs HEAD)", "Last turn", and every completed turn
  of the session, so you can see exactly which files the AI touched in one
  turn instead of everything accumulated since the last commit. A turn is
  bounded by your confirmed input and the session's completion summary, and
  the diff is computed between two throw-away Git trees, so untracked files
  are included. Snapshots live in memory only — at most 100 per session,
  discarded when the Hub restarts. When a turn finishes having edited files, a
  card above the action bar shows `Turn #n: N files edited` with `+added` /
  `-removed` counts and a **Review** button that opens the tab already scoped
  to that turn. Adds `GET /api/git-turns` and `GET /api/git-turn-diff`
  (`internal/hub/git_turns.go`, `web/src/app/review-view.ts`).
- **In-browser folder picker for the spawn panel's Browse button.** Where the
  OS-native folder dialog cannot help you, Browse now opens a folder browser
  inside the Hub UI, walking the Hub host's filesystem through
  `/api/list-subdirs`. It is used automatically when `/api/info` reports
  `env_kind: remote` (a native dialog would have opened on the server,
  invisible to you) and as the fallback when the native picker fails or is
  missing (Linux without zenity/kdialog) — that case previously produced only
  an error toast (`web/src/app/spawn-panel.ts`).
- **A dismissible banner now explains why an approval panel was withheld.**
  Structurally broken marker blocks were suppressed silently, leaving you
  waiting for an approval that never appeared with no way to tell why. The
  banner names the reason, offers a **↻ Re-detect** button that rescans the
  browser's own terminal buffer, and is throttled so repeated suppressions do
  not stack up. Adds the `approval_marker_suppressed` WebSocket message
  (`internal/hub/approval_marker.go`, `web/src/app/approval-ui.ts`, ja/en/vi
  strings).
- **Clear buttons for the working-directory history and favorites** in the
  spawn panel's `cwd` dropdown, each with a confirmation
  (`web/src/app/spawn-panel.ts`).
- **The Chat pane now reads the CLI's own structured transcript instead of
  scraping the terminal.** Chat text used to be assembled from PTY output by
  committing whatever had arrived after 1.5 seconds of silence — but Claude and
  Codex repaint their spinner several times a second, so that pause almost
  never happened and redraw frames landed in the database as independent
  messages. Roughly 60–65 % of stored AI messages were shorter than 25
  characters (`Reticulating…`, `✶9`, `759.7k ↓8`), and longer ones carried
  status bars and input-box borders inline. The Hub now parses Claude's
  `~/.claude/projects/<slug>/<uuid>.jsonl` and Codex's
  `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` directly, separating assistant
  text from thinking blocks and tool calls, and tails them for live updates
  over `GET /api/agent-chat` plus a WebSocket feed. Thinking and tool calls are
  kept rather than dropped: both are collapsed by default with a header toggle
  to expand them. The session is bound to its transcript deterministically by
  injecting `claude --session-id`, falling back to nearest-match for providers
  that do not accept one. PTY-derived AI messages are no longer accumulated in
  the session database for the providers that have a transcript
  (`internal/hub/agent_chat_parse.go`, `internal/hub/agent_chat_handler.go`,
  `internal/wrapper/claude_session.go`, `web/src/app/chat-history.ts`).
- **Live workflow progress, computed by the Hub instead of each browser.** The
  Hub parses every session's VT mirror in Go and broadcasts progress — agents
  done/total, elapsed time, and the agent tree — as a new `workflow_progress`
  message, sent on change plus a 7-second heartbeat. For Claude it also tails
  `journal.jsonl` for the authoritative completion count, reading only the
  `type` / `agentId` metadata: result bodies are never held, logged, forwarded
  or persisted. The browser prefers the Hub's numbers and falls back to its own
  local VT parser. The UI gains elapsed time, a background-waiting count, a
  completion ledger (capped at 200 entries) and a mini progress bar on each
  sidebar card, in all three languages. Completion can raise a Web Push
  notification. Two new settings: `workflow.journal_enabled` (default on) and
  the `workflow_completion_notify` user preference (default off)
  (`internal/hub/workflow_scan.go`, `internal/hub/workflow_journal.go`).
- **OpenCode can be launched with approvals turned off.** The spawn panel shows
  a "launch with everything allowed" option for OpenCode, which sends
  `permission_mode` and makes the wrapper add `--auto` alongside the
  `opencode.json` permission rules the Hub already writes. The option is
  reflected in the spawn risk summary so an unattended full-allow session is
  not started silently (`internal/hub/spawn_handler.go`,
  `internal/hub/spawn_risk.go`, `internal/wrapper/wrapper.go`,
  `web/src/app/spawn-panel.ts`).
- **A banner now warns when the Hub is running an outdated binary.** Replacing
  the executable does not affect a Hub that is already running, so after a
  rebuild the dashboard could keep showing behaviour from the old build — in
  one measured case for nearly five hours, making an already-fixed approval bug
  look unfixed. The Hub re-evaluates staleness on every `/api/info` response
  and pushes `binary_stale` to all connected UIs only when the state changes,
  so an open dashboard gets the banner as soon as the next session is started.
  Recovery is broadcast too, so the banner cannot stick. There is no polling
  loop: the added cost is one `os.Stat` per `/api/info`, with no extra
  goroutine or timer, since only developers ever swap the executable
  (`internal/hub/server.go`, `web/src/app.ts`).
- **A ✕ close button in the Review tab header,** next to ↻, so the tab can be
  closed from where the diff is being read instead of only from the tab bar
  (`web/src/app/review-view.ts`, `web/src/app/files-view.ts`).
- **The turn-completion card can be dismissed,** with a ✕ button and automatic
  removal when the next command starts, including cleanup on turn renumbering
  and session deletion (`web/src/app/review-view.ts`).
- **Individual entries can be deleted from the `cwd` history dropdown,**
  keeping the favorites list in sync and preserving the current filter text
  across the redraw (`web/src/app/spawn-panel.ts`).

### Changed
- **Breaking: sending a message in a Git working tree now takes a Git snapshot
  before the input reaches the CLI.** The Hub runs `git read-tree` +
  `git add -A` + `git write-tree` against a temporary index, so your real
  staging area is never modified — but loose blob and tree objects for every
  changed and untracked file are written into the repository's object database
  twice per turn and remain there until `git gc`. The pre-input snapshot is
  synchronous, so submitting is briefly blocked while it runs (5 s timeout,
  after which the turn is simply not recorded). This happens for every session
  whose directory resolves to a Git repository, whether or not the Review tab
  is open, and there is currently no config key to disable it
  (`internal/hub/git_turns.go`, `input_gate.go`, `done_summary.go`).
- **Breaking: a session's PTY size is now driven by exactly one browser view.**
  Resize authority is claimed by the view that registers with the session
  active, activates it in the session list, or types into it; `pty_resize`
  from any other connected UI is ignored. When ownership transfers, the new
  owner's last known size is applied once, and when the owning view
  disconnects the next resize from a remaining view takes over
  (`internal/hub/server.go`, `ui_broadcast.go`, `web/src/app/session-list.ts`).
- **Suggested commit messages are now built around the file with the largest
  change,** weighed by `git diff --numstat HEAD` (untracked files by line
  count; binaries and files over 1 MiB counted as zero), instead of the first
  entry in the status list. A small new file that happens to export a type no
  longer hijacks the subject. Analysis also covers TypeScript/JavaScript
  (`.ts` / `.tsx` / `.js` / `.jsx` / `.mts` / `.mjs` / `.cjs`) and Python in
  addition to Go — exported functions, consts, classes, types and interfaces,
  Express-style and decorator-based routes, and i18n keys under `locales`
  (`internal/hub/git_commit.go`).
- **Links inside AI output now ask before opening.** Clicking a URL in the
  Chat pane or the Grok conversation viewer shows a confirmation with the full
  destination, and only `http` / `https` targets are opened
  (`web/src/app/util.ts`, `chat-history.ts`, `grok-chat-viewer.ts`).
- **New-device security notifications no longer fire on every browser update.**
  The device fingerprint uses a coarse browser-brand/OS bucket instead of the
  raw User-Agent, and proxied clients are identified by their Tailscale login
  (`internal/hub/pin_auth.go`).
- **The bundled Grok slash-command reference now follows Grok Build's official
  guide,** adding `/new`, `/history`, `/context`, `/session-info`, `/fork`,
  `/rewind`, `/effort`, `/always-approve`, `/auto`, `/multiline`,
  `/compact-mode`, `/vim-mode`, `/minimal`, `/fullscreen`, `/plan`,
  `/view-plan`, `/copy`, `/rename`, `/home`, `/quit` and their aliases
  (`resources/slash-commands/grok.md`).
- `many-ai-cli doctor` now waits up to 3 seconds (was 1) for each provider's
  `--version`, so slow-starting CLIs are reported with their version instead
  of the bare name (`internal/doctor/doctor.go`).
- **A failure while syncing the winget fork no longer aborts the whole
  release.** The release job refreshed the `winget-pkgs` fork from upstream
  before GoReleaser ran, so that the generated manifest PR would branch from an
  up-to-date base. That step was fail-fast and sat ahead of GoReleaser, which
  meant a problem affecting one distribution channel could — and did — prevent
  the GitHub Release itself from ever being created. It now logs a warning and
  continues; the only degradation is that the winget PR may branch from a stale
  base, which does not affect the GitHub Release, Homebrew or npm. It stays
  ahead of GoReleaser, because moving it after would leave every release
  branching from a fork one version out of date
  (`.github/workflows/release.yml`).
- **Validate CI now also runs on pushes to `develop`.** The workflow only
  triggered on pull requests and pushes to `main`, so commits landing directly
  on `develop` reached tagging without ever passing the quality gate
  (`.github/workflows/validate.yml`).
- **Files preview no longer refuses to open documents that live outside the
  current repository.** Reading was previously confined to the session cwd, its
  git root, and the attachments/orchestration directories, with a narrow
  fallback for paths the user had typed into chat — so reference material kept
  next to a project (a `docs/` tree in a sibling folder, a recap note, a shared
  spec) came back as a bare HTTP 403. That boundary did not actually contain
  anything: the AI CLI itself runs as the same OS user and can read any file
  with its own tools, and a caller holding the Hub token can already spawn a
  shell in nearly any directory. Requests that arrive from a browser on the Hub
  host itself are now served without the allowed-roots check
  (`internal/hub/files_scope.go`). Files outside the project still come back
  flagged `readOnly`, so the editor stays disabled for them.
- **Logically remote callers keep the old, narrower scope.** Requests routed
  through tailscale serve, `trusted_networks`, or any reverse proxy — detected
  by `isLogicallyRemote`, the same split `/api/list-subdirs` already used — are
  still limited to the allowed roots plus paths the user referenced in their own
  chat input (`role='user'` only; AI output remains excluded).
- **Secret-like files are refused on every out-of-scope read.** Private keys
  (`*.pem`, `*.key`, `id_rsa*`), credential files (`*credentials*`), environment
  files (`.env`, `.env.*`), the Hub's own session-history database
  (`any-ai-cli.db` and its `-wal`/`-shm` siblings, which hold every chat
  transcript), and `~/.many-ai-cli/config.yaml*` (which stores the Hub token in
  plaintext — matched by prefix so that a hand-made copy of the file cannot leak
  the same token) are rejected even from the Hub host. The file tree no longer
  extracts a content summary for them either, so walking a directory cannot
  bulk-harvest the first bytes of each one. Paths inside the project keep their
  previous behaviour, so a repository's own `.env` still previews as before.
- **Writes and terminal launches are unchanged.** `files-save`, `files-create`,
  `files-mkdir`, `files-move`, `files-rename`, `files-delete-dir`, and
  `/api/open-terminal` still require the session cwd or git root, so a widened
  read scope cannot turn into an edit or a shell in an arbitrary directory.
- **Orchestration children accept Claude Code's cross-session messages instead
  of stalling on an approval dialog.** Claude Code v2.1.224 added
  [cross-session messaging](https://code.claude.com/docs/en/cross-session-messaging),
  where sessions reach each other over a per-session inbox socket. Its inbound
  default holds every message for approval when the *receiving* session bypasses
  permission prompts — and orchestration children always run with
  `bypassPermissions` (`applyChildApprovalDefaults`) while nobody is watching
  their terminal. A held message would have parked the child behind a dialog
  until `dialogExpiry` (five minutes by default) with nothing recorded on the
  board to explain the gap. Children now launch with `crossSessionInbound` set
  to `accept`. Conductors and hand-started sessions keep the default, since a
  human is watching those. The setting travels through the wrapper-owned temp
  settings file that already carried `statusLine`, so the shared
  `.claude/settings.local.json` is still never touched
  (`internal/wrapper/usage_hooks.go`, `internal/wrapper/wrapper.go`). It is
  omitted on native Windows, where the feature does not exist; sessions inside
  WSL 2 are Linux and do get it.
- **A bad `ollama` / `lm_studio` private-host setting no longer stops the Hub
  from starting.** Requiring `allow_private_hosts` (above) turned a
  misconfigured optional integration into a fatal startup error, taking down
  every session with it. The mismatch is now reported as a warning written to
  `hub.log` at startup and the Hub serves normally; only the model-list feature
  stays blocked (`internal/config/config.go`, `internal/hub/server.go`).
- **Six Cursor Agent commands were missing from the slash-command picker.**
  `cursor-agent.md` grouped aliases onto one row — `` | `/clear` / `/new` /
  `/new-chat` | ... `` — but the parser that reads these files cannot cross a
  backtick, so those rows matched nothing at all. `/clear`, `/quit`, `/open`,
  `/run-everything`, `/shell` and `/summarize` had never appeared in the picker.
  Every file is now one command per row, and `scripts/check-slash-commands.mjs`
  fails the build when a row does not parse, when the ordering breaks, or when a
  description carries markup the parser would strip
  (`resources/slash-commands/`, `.github/workflows/validate.yml`).
- **The bundled slash-command reference is rebuilt against the installed CLIs.**
  Earlier passes compared the reference against each vendor's documentation
  alone, which suggested deleting 27 Claude Code commands and 20 Codex commands
  — those pages simply do not list everything. Each command is now checked
  against the CLI's own executable as well, so presence is verified rather than
  inferred. Claude Code gains `/autocompact`, `/bug`, `/import`, `/list-agents`,
  `/plugins`, `/pr`, `/quiet`, `/save`, `/session`, `/settings`, `/share`,
  `/subtask`, `/test`, `/uninstall`, `/web` and `/why`, and loses
  `/work-with-pr`, which appears in neither source. Cursor Agent gains nine
  entries, Grok is rebuilt from xAI's command reference at 69 entries, GitHub
  Copilot gains `/permissions`, and OpenCode — which had shipped as an empty
  placeholder — now lists its 17 TUI commands. Codex needed no changes.
- **The spawn panel's model suggestions now cover the current model
  generation.** The list had stopped at Claude Opus 4.8 and GPT-5.5, so the
  whole Claude 5 generation and every GPT-5.6 variant were missing from the
  dropdown and had to be typed by hand. Anthropic entries gain Opus 5, Sonnet 5
  and Fable 5; the Codex list is rebuilt from the CLI's documented `--model`
  values including the GPT-5.6 Sol, Terra and Luna variants; the Cursor Agent
  list is regenerated from `cursor-agent --list-models` and now carries the
  Claude 5, GPT-5.6, Kimi K3, GLM 5.2 and Gemini 3.6 entries; the GitHub Copilot
  list is rebuilt from GitHub's published set of supported models, adding the
  Claude 5 family, the GPT-5.6 variants, Gemini 3.6 Flash, Grok 4.5 and the Kimi
  entries; and Grok drops two entries that its own binary does not know about —
  one of them was the product name rather than a model — in favour of the three
  it does. Like the slash-command reference,
  `resources/models/defaults.json` is fetched at run time from `main`, so this
  reaches existing installations without an upgrade.

### Fixed
- **An answered batch approval no longer reappears in the action bar.** After
  a terminal resize the Hub kept reading its VT mirror while it was still
  reflowing, and the TUI composer's frame rule overwrote the tail of an option
  label. The block kept both markers and an intact numbering, so it passed the
  corruption check, but its text — and therefore its signature — had changed,
  which slipped past both the consumed-signature and answered-marker guards.
  Marker extraction now honours the same resize debounce that native approval
  scanning already used, and a run of box-drawing characters inside a marker
  block (which `approval-rules.md` forbids, so it can only come from a redraw)
  is treated as corruption on both the Hub and the browser
  (`internal/hub/wrapper_loop.go`, `approval_marker_verdict.go`,
  `web/src/app/approval-parser.ts`).
- **Structurally broken approval marker blocks are no longer delivered.** When
  the Hub's VT mirror drifted from the real terminal, a block could keep both
  markers while losing option lines, producing a panel whose choices started
  at 3 or that had a single button — answering it sent a number the agent
  never offered. Such blocks are now suppressed without rewriting the dedupe
  signature, so the next intact block is still delivered
  (`internal/hub/approval_marker_verdict.go`).
- **An approval question you already answered no longer reappears.** When the
  fallback parser re-detected the same marker block, the cached options lost
  their marker identity, so answering never recorded the permanent suppression
  and an Ink/SIGWINCH redraw brought the panel back
  (`web/src/app/approval-parser.ts`, `approval-ui.ts`).
- **Input is no longer silently discarded while the Hub connection is down.**
  `sendText` now reports failure, and every caller — composer, quick commands,
  single and batch approval choices, multi-select answers, sequential
  questions, the free-text field, and the mobile approval sheet — checks it
  before clearing the input box, consuming attachments, hiding the action bar,
  or recording the approval as answered. A toast is shown and your text and
  selections stay untouched so the same input can be sent again after
  reconnecting.
- **Codex sessions no longer stop at an old frame in the terminal pane.**
  Codex and Grok use absolute cursor positioning for body text and full-frame
  redraws, and the cursor-hide filter was discarding those blocks as if they
  were status bars, so the pane stopped following the output and gaps appeared
  mid-line. Both providers now bypass the filter. As a trade-off, Codex status
  and `Working` lines are kept in the scrollback instead of being filtered out
  (`web/src/app/cursor-hide-filter.ts`, `terminal.ts`).
- **Switching sessions no longer leaves the previous session's output on
  screen.** A concurrent flush could invalidate the sequence number and
  swallow the callback that re-attaches the terminal container; completion
  callbacks are now queued and run once the in-flight write drains
  (`web/src/app/terminal.ts`).
- **Bringing a window back into focus no longer force-resizes the PTY.** A
  resize is sent only when the refit actually changed the column/row count,
  so two windows watching one session stop pushing their geometry at each
  other (`web/src/app/terminal.ts`).
- **Grok's full-screen views (`/resume`, `/history`, `/help`) no longer render
  as a blank screen** (`web/src/app/terminal.ts`).
- **The Grok conversation viewer no longer opens an empty transcript.** All
  candidate session directories are collected and the nearest one that
  actually contains messages wins, instead of the closest by timestamp — which
  could be an empty stub Grok had just created
  (`internal/hub/grok_history_handler.go`).
- Pressing Enter to confirm a Japanese IME conversion also submitted the
  free-text approval answer. The action-bar field and the mobile approval
  sheet now ignore Enter while a composition is active.
- **Arrow keys reach Claude's `/config` and `/model` menus again** even when
  the composer still contains text (`web/src/app.ts`).
- A `Q1`-style token inside a numbered option's label (for example
  "1. Remove the Q1 form") was treated as a new question heading, splitting
  one question into pseudo-sections and showing duplicate question tabs
  (`web/src/app/approval-parser.ts`).
- **Private paths were not redacted from bug reports when the development root
  was not on drive C:.** The redaction pattern hard-coded `c:`, so `kb`,
  `.ssh`, and `github\private` paths on any other drive were included
  verbatim. Drive letters are no longer part of the pattern, for both the
  private-path and home-directory rules (`internal/report/redact.go`).
- **On Windows, an exited process was reported as still running.** `pidAlive`
  treated "a handle could be opened" as alive, but Windows keeps the process
  object around after exit, so `many-ai-cli stop` waited out its timeout on an
  already-dead Hub and then force-killed a PID that may have been recycled to
  an unrelated process. Liveness is now decided by `GetExitCodeProcess`
  (`internal/hub/pid_windows.go`, `internal/launcher/pid_windows.go`).
- Only the first completion summary was published when a provider emitted
  several completion markers in one burst of output
  (`internal/hub/approval_native.go`).
- Clean-text transcript generation stopped at the first JSONL line larger than
  8 MiB and dropped every later event (`internal/sessionlog/transcript.go`).
- Spawning two worktree-backed sessions with the same label in the same minute
  could fail with a `git worktree add` error because both picked the same
  directory name; worktree creation is now serialized
  (`internal/hub/normal_worktree.go`).
- The LM Studio model list was cached without regard to the URL it came from,
  so changing `lm_studio.base_url` kept returning the previous server's models
  for up to a minute (`internal/hub/models_fetch.go`).
- Linux desktop shortcuts created by `many-ai-cli setup` now escape `%` in the
  executable path (`internal/setupcmd/setup_linux.go`).
- Leftover `launcher-active-*.json.tmp` files from a launcher killed mid-write
  are removed once they are older than an hour instead of accumulating
  (`internal/launcher/active.go`).
- **The web terminal no longer freezes on a stale frame after the wrapper
  reconnects to the Hub.** When a burst of PTY output overflows the wrapper's
  send queue, the wrapper deliberately drops the WebSocket and reattaches two
  seconds later — a designed self-recovery path. Its replay buffer was applied
  to the Hub's internal mirror only, so an already-open browser never received
  the bytes emitted during the gap. Terminals assume a lossless byte stream, so
  a TUI that repaints with absolute cursor addressing (Codex, Grok) drew every
  later frame at the wrong coordinates and could not catch up. The wrapper now
  reports how many PTY bytes it has read, the Hub tracks how many it has
  received, and on reattach exactly the missing tail is broadcast to connected
  UIs — no gap, and no duplicate of what was already displayed. Reattach also
  keeps the existing scrollback and VT mirror instead of truncating them to the
  64 KB replay window, so reloading the browser after a reconnect no longer
  loses history. Reattaches are now logged with `reattach replay gap`
  (`internal/hub/reattach_replay.go`, `internal/hub/wrapper_loop.go`,
  `internal/wrapper/wrapper.go`).
- **Text sent from the web UI is no longer swallowed when the wrapper
  reconnects.** In Codex sessions a message could stay sitting in the CLI's
  input box until Enter was pressed a second time. Two layers were at fault:
  the wrapper's 64-chunk PTY output queue overflowed on Codex's output bursts
  (measured at up to 582 chunks in a 100 ms window, 147 self-disconnects over
  13 hours, against a maximum of 30 chunks and zero disconnects for Claude),
  and the confirming CR that arrived during that window passed the Hub's
  "delivered" check and vanished, because the send path had no acknowledgement
  and a `nil` error did not mean arrival. The queue now holds 1024 chunks and
  coalesces pending output up to 256 KiB, and a new `pty_input_ack` message
  lets the Hub retain unacknowledged input and resend it on reattach. Resends
  keep their original sequence number so the wrapper's watermark can drop
  duplicates — an acknowledgement lost to the disconnect no longer results in
  the confirming CR being typed twice (`internal/hub/wrapper_loop.go`,
  `internal/wrapper/wrapper.go`, `internal/proto/messages.go`).
- **A wrapper that reconnects is no longer numbered as a new session.** When
  the wrapper dropped its own WebSocket after a queue overflow and reattached
  before the Hub had noticed the disconnect, the reattach loop read it as an ID
  collision with a different wrapper and allocated a new number — producing two
  cards for one process, an old one still showing a pending approval with
  nowhere to send the answer, and a new one starting from zero bytes with no
  scrollback (measured 2026-08-05: #15 and #17 both `pid=6224`). The PID the
  wrapper already sends on reattach is now matched together with provider and
  cwd; wrappers too old to send one still take the renumbering path, since the
  OS reuses PIDs and one alone is not proof of identity. Teardown of the
  superseded connection is also guarded, so a late-finishing old connection can
  no longer mark the live session disconnected, broadcast `session_end`, and
  discard its pending input and approval rules (`internal/hub/wrapper_loop.go`).
- **Answered Codex approvals no longer flicker back into the action bar.** In
  long, high-output sessions an already-answered approval block was re-detected
  as new on every reattach, TUI repaint and VT reflow, so the action bar
  rebuilt itself continuously and could not be clicked. Approval candidates now
  carry a logical identity and a replay boundary on the wire: the Hub batches
  detection while replaying and suppresses corrupted and reconnect-borne
  candidates, and the browser suppresses answered candidates by logical ID and
  generation. Action-bar DOM changes and PTY resizes are folded into a single
  settle so one candidate cannot drive a resize feedback loop. Approvals that
  are genuinely still waiting are shown as before
  (`internal/hub/approval_identity.go`, `internal/hub/approval_native.go`,
  `web/src/app/approval.ts`, `web/src/app/state.ts`).
- **The OpenCode approval bar no longer flickers and refuses clicks.** An
  approval's identity was derived from a snapshot of the whole terminal window,
  so a spinner frame or a ticking elapsed-seconds counter changed the signature
  and OpenCode re-broadcast `approval_detected` on every frame; the browser
  rebuilt the action bar each time. Identity is now scoped to the dialog itself
  (from `Permission required` down to the button row). Narrowing it exposed
  three follow-on problems that are fixed with it: hint-word detection and
  `/model` selector suppression still receive the full screen, the question line
  is extracted properly instead of always being empty (so two different requests
  are now distinguishable), and the button row is searched from the bottom up so
  a stale dialog left on screen is not picked up
  (`internal/hub/approval_native.go`).
- **OpenCode's second-stage approval now reaches the web UI.** Choosing "Allow
  always" makes OpenCode show a follow-up Confirm / Cancel dialog, which the
  Hub could not detect because its dialog detection keyed on a line containing
  "allow once" — wording the second stage does not have, along with "permission
  required". You had to switch to the terminal and press Enter there. The
  horizontal Confirm / Cancel button row is now recognised together with its
  "Always allow" heading, both stages are searched with the lower one on screen
  winning (the first stage often remains drawn above the second), and the
  approval bar's heading shows the permission pattern being granted so it is
  visible before the button is pressed (`internal/hub/approval_native.go`).
- **The "approval was withheld" banner no longer appears while the approval
  panel is on screen.** When only the Hub's VT mirror was corrupted, the browser
  could still rebuild the panel from its own xterm buffer, but that path never
  retracted the notice — so the panel and a message saying it had been withheld
  were shown at once. The banner's lifetime is now tied to whether the panel is
  actually displayed, which also makes it disappear when **↻ Re-detect**
  succeeds (`web/src/app/approval-ui.ts`).
- **Session cards can be reordered by drag and drop again, and new sessions
  appear in creation order.** Every render overwrote the drag-and-drop order
  with a provider-based sort, so a dropped card sprang back and the list was
  split up by AI agent; the drop handler itself had been working and was even
  saving successfully. Persistence was broken as well — the order is an array
  of numeric session IDs but was sanitised as an array of strings, so every
  element was discarded and nothing ever reached the server. The stored type is
  now `[]int`, with a lenient decoder that accepts both numbers and numeric
  strings so an older `config.yaml` does not fail to parse and trigger a
  `.bak` rescue plus a regenerated default config (`web/src/app/session-list.ts`,
  `internal/config/config.go`).
- **A terminal resize no longer produces a bogus `marker_leak` verdict.** The
  VT mirror's scrollback is discarded on resize, and the show/hide cycle of the
  suppression banner no longer amplifies into a `pty_resize` loop
  (`internal/hub/vt_buffer.go`, `internal/hub/input_gate.go`).
- **Tool results now appear in the Chat pane without a reload.** The parser's
  map of pending tool-call IDs was rebuilt empty on every parse call, so a
  result that arrived in a later incremental read had nothing to attach to and
  was dropped. Call/result association is kept across polls, messages carry a
  stable `message_id`, and reattach state is tracked separately with the
  generation verified, so a live tool result is rendered as it arrives
  (`internal/hub/agent_chat_parse.go`, `internal/hub/agent_chat_handler.go`).

### Removed
- **The temporary `/api/debug/batch-log` endpoint and its front-end
  instrumentation.** [0.5.1] introduced it to diagnose the batch-approval
  action bar and promised its removal once the cause was confirmed; the cause
  is fixed above, so the endpoint, the 25 `dlog` call sites, and the
  `~/.many-ai-cli/logs/debug/batch-log.jsonl` writes — which recorded approval
  answer bodies verbatim — are gone. Existing log files are left untouched
  (`internal/hub/debug_batch_log_handler.go`, `web/src/app/approval.ts`).
- **The temporary input tracing added while chasing the lost-keystroke bug
  above.** The wrapper wrote `wrapper-trace.jsonl` into the log directory —
  created with mode `0644`, appended to without rotation or a size cap, and
  active regardless of whether session logging was enabled — and the Hub logged
  a matching `input_trace` line carrying a hex tail of the same bytes. Because
  the traced bytes are whatever you type, that included passwords and API keys
  pasted into a session, unmasked, on every normal run. Both sides are removed:
  `internal/wrapper/trace.go`, `internal/hub/input_trace.go`, and their call
  sites. Existing trace files are left on disk — delete
  `~/.many-ai-cli/logs/wrapper-trace.jsonl` if one was produced by 0.5.x.
- **Two more pieces of temporary instrumentation that were still shipping.** The
  `/api/debug/ui-log` endpoint and its front end recorded every action-bar
  show/hide, every resize and its layout snapshot, and ran a 2-second timer that
  counted blank lines in the active terminal buffer — all enabled by default,
  writing a daily `logs/debug/ui-log-*.jsonl`. It was added to find what was
  driving the Codex resize oscillation, which is fixed above, and its own
  comment said to remove it once the cause was known. A second channel,
  `ui_input_trace`, reported every deferred-Enter decision to `hub.log` with no
  opt-in; it was added for the swallowed-keystroke bug, also fixed above. It
  survived the removal of the input tracing because it had no file of its own —
  it lived in three shared files. Both are gone
  (`internal/hub/debug_ui_log.go`, `web/src/app/debug-ui-log.ts`, and the
  `sendUITrace` / `traceEnter` paths).

### Security
- **Breaking: PIN sessions are now tracked on the server and bound to the
  device that logged in.** The PIN cookie carries a random nonce and a device
  hash — client IP, or the Tailscale identity when the Hub sits behind a
  loopback reverse proxy, combined with a coarse browser/OS bucket — next to
  the expiry, and the Hub only accepts a cookie whose nonce is still in its
  session registry. Cookies issued by earlier versions are rejected, the
  registry is in memory only so a Hub restart signs every remote client out,
  and moving to another network or browser requires entering the PIN again.
  At most 256 PIN sessions are kept and the oldest is evicted when a new login
  exceeds that (`internal/hub/pin_auth.go`).
- **Breaking: authentication cookies are now marked `Secure` when the request
  arrives over HTTPS.** This covers the Hub token cookie, the PIN cookie, and
  the deletion cookies sent on logout. TLS is detected either directly or, for
  loopback requests forwarded by a reverse proxy such as Tailscale Serve, from
  an `X-Forwarded-Proto: https` header. A proxy that reports `https` while the
  browser still talks plain HTTP will now have its cookies dropped by the
  browser (`internal/hub/http_helpers.go`, `pin_auth.go`, `auth_handlers.go`).
- **Breaking:** Explicit private-network `ollama.base_url` and
  `lm_studio.base_url` values now require `allow_private_hosts: true` in the
  corresponding config section. Only an omitted base URL keeps using the
  built-in loopback default without opt-in. Every explicitly configured
  private or loopback address—including `localhost`, `127.0.0.1`, `::1`, and
  loopback services on non-default ports—must opt in explicitly.
- **Breaking: model-list requests are now blocked at connect time** when they
  land on a private address without `allow_private_hosts: true`, which also
  catches host names that only resolve to a private IP at dial time. Redirects
  are capped at three hops and must stay on http/https without embedded
  credentials (`internal/hub/models_fetch.go`).
- **Breaking: WebSocket connections without an `Origin` header are rejected
  when the request is logically remote.** Origin-less connections are still
  accepted from local clients, which is what the wrapper uses
  (`internal/hub/server.go`).
- **Breaking: `MANY_AI_CLI_WHISPER_SERVER` is only honoured when it points at
  a whisper server binary name** (`whisper-server` / `server`, with or without
  the platform extension). Any other path is ignored and the Hub falls back to
  the managed binary (`internal/hub/whisper_manage.go`).
- **Logging out now invalidates the PIN session on the server.**
  `POST /api/auth/logout` revokes exactly the session presented by the
  request, so a copied cookie value can no longer be replayed until it
  expires, and revoke-all drops every PIN session in addition to rotating the
  token (`internal/hub/auth_handlers.go`).
- **PIN lockouts are no longer shared by every client behind a reverse
  proxy.** Requests that reach the Hub over loopback but are logically remote
  are bucketed per Tailscale identity instead of all landing in the proxy's
  `127.0.0.1` bucket (`internal/hub/pin_auth.go`).
- The Hub UI is now served with a Content-Security-Policy: scripts, styles and
  connections are restricted to same-origin (plus `ws:` / `wss:` for the
  WebSocket), plugins are blocked, `base-uri` is disabled and the page cannot
  be framed (`web/src/index.html`).
- Session-log redaction now masks xAI (`xai-…`), `gsk_…`, and Cohere / Mistral
  API keys, which in turn covers what bug reports and notification bodies can
  leak (`internal/sessionlog/sessionlog.go`).
- The custom notification sound stored in user preferences is now validated on
  save: only the Hub-managed `notify_sound_custom.bin` with an `audio/*` MIME
  type is kept (`internal/hub/user_prefs_handlers.go`).
- **The corrupt-approval-block dump now follows the session-log opt-in.** When a
  marker block arrives damaged, the Hub can save the rendered block together
  with the tail of the PTY bytes that produced it, so the mirror corruption can
  be reproduced later — the underlying cause is still unknown, so the diagnostic
  is worth keeping. It ran regardless of whether session logging was enabled,
  which is the same objection the pre-release audit raised against the input
  tracing: secrets are masked on the way in, but what is stored is still raw PTY
  output and masking can miss things. It is now off unless
  `log.session_enabled` is on (`internal/hub/approval_corrupt_dump.go`).
- **Temporary instrumentation is now tracked in a ledger and enforced by CI.**
  Three releases in a row shipped, or nearly shipped, debugging code that was
  meant to be temporary: the `/api/debug/batch-log` endpoint in 0.5.1, the input
  tracing in 0.5.x, and the two channels removed above. Writing "remove this
  once the cause is confirmed" in a comment did not work. `instrumentation.json`
  now records every piece of instrumentation with its purpose, its gate, the
  condition for removing it and a deadline, and `scripts/check-instrumentation.mjs`
  fails the build when instrumentation appears that is not in the ledger, when an
  active entry is always-on or past its deadline, or when an entry claims to be
  removed while the code is still there. It runs as its own Validate job and as
  part of the release preflight. It found the `ui_input_trace` leftover on its
  first run.
- **Transcript and workflow-journal reads are bounded before anything is
  allocated.** The new Chat pane and workflow progress read files the Hub does
  not control the size of. The message limit was applied only after the whole
  file had been parsed, and `ReadBytes` allocated an entire line before any cap
  was consulted, so one very long line was enough to exhaust CPU and memory;
  the workflow journal had no limit at all. Per-line length, cumulative bytes,
  record count and constructed message count are now capped up front, reads are
  paginated with a cursor, and a line over 4 MiB is carried across polls
  instead of being buffered whole. The tail page also commits its cursor at the
  same boundary as the records it actually returned, so hitting the 100 ms
  parse deadline part-way through a page can no longer skip the unparsed
  records permanently (`internal/hub/agent_chat_parse.go`,
  `internal/hub/workflow_journal.go`).

## [0.5.1] - 2026-07-22

### Added
- **Bug reporting from the Hub UI and the CLI.** A "🐞 Bug report" button in
  the Hub opens a dialog where you describe the symptom and the steps to
  reproduce; the Hub then opens a pre-filled GitHub Issue form. Everything
  that will leave your machine is shown in a preview first. The same flow is
  available as `many-ai-cli issue` (`--title` / `--provider` / `--dry-run`);
  under `MANY_AI_CLI_AUTO=1` only `--dry-run` is permitted. Environment
  details are gathered through an explicit allow-list (version, OS, arch, Go
  version, provider, model, User-Agent, hub port, per-provider last model) —
  the config file is never attached. Adds `POST /api/bug-report/preview` and
  `POST /api/bug-report/finalize`
  (`internal/hub/bug_report_handler.go`, `internal/report/`,
  `cmd/many-ai-cli/issue.go`, `web/src/app/bug-report-modal.ts`).
- **Optional session-log attachment for bug reports.** Off by default. When
  enabled, the last 200 lines are redacted and shown to you, and only after
  you confirm are they uploaded via `gh gist create --secret` and linked from
  the issue body. Without `gh`, or if the gist fails, or if the issue URL
  would exceed 8192 bytes, the redacted report is written to
  `~/.many-ai-cli/reports/` instead and GitHub opens with the title only.
- **Vietnamese language support.** Hub UI locale (`web/src/i18n/vi.json`),
  a `Tiếng Việt` entry in the language selector, automatic selection when
  `navigator.language` starts with `vi`, and `vi-VN` speech recognition.
  Vietnamese translations of the README, CLAUDE.md, and 11 manuals under
  `docs/` are included. The UI locale is at full parity with Japanese and
  English (1337 keys each).
  Contributed by [@ngav1491](https://github.com/ngav1491) in
  [#2](https://github.com/ishizakahiroshi/many-ai-cli/pull/2).
- **Copy button on each message in the Grok conversation viewer**
  (`web/src/app/grok-chat-viewer.ts`).
- **`include_body` toggle in Hub Settings.** The notification section now
  exposes the `notify.include_body` config key that was previously only
  editable by hand in `config.yaml`, with a note explaining that enabling it
  sends approval questions and completion summaries to an external service.
- **`hub.wrapper_send_write_timeout_sec` setting** (default 5) for the
  WebSocket write deadline used by the wrapper.

### Changed
- **Approval markers are now detected across scrollback.** The VT buffer keeps
  a 500-line scrollback ring and marker extraction scans 300 lines instead of
  the visible 120, so multi-question blocks taller than the terminal (as Grok
  emits) are picked up. Native approval detection still looks only at the
  current screen, to avoid re-detecting prompts that were already answered
  (`internal/hub/vt_buffer.go`, `approval_detector.go`, `wrapper_loop.go`).
- **The wrapper no longer blocks PTY reads on WebSocket writes.** Output now
  goes through a bounded 64-slot queue drained by a dedicated goroutine, so a
  stalled Hub connection can no longer freeze the wrapped CLI. If the queue
  overflows or a send fails, the connection is dropped as a transport fault
  rather than silently discarding output, and the existing 64 KB replay
  buffer restores a consistent screen on reconnect. The reconnect supervisor
  now distinguishes "Hub HTTP is alive but the WebSocket transport is broken"
  from a deliberate disconnect (`internal/wrapper/wrapper.go`).
- **AI-generated commit messages now follow Conventional Commits.** Subjects
  are `<type>(<scope>): …`, where the scope is the first non-generic segment
  of the deepest common directory (`src` / `internal` / `cmd` / `pkg` / `lib`
  are skipped, so `web/src/app` yields `app`). The verb is chosen from ten
  candidates including new `move`, `simplify`, and `handle` detections
  (`internal/hub/git_commit.go`).
- **The session list no longer reorders cards when approvals arrive.** Cards
  keep their position; pending approvals are indicated by the badge and the
  per-project pending count instead. Summary counts are localized and collapse
  to numbers only when space is tight (`web/src/app/session-list.ts`).
- **Working-directory favorites in the spawn panel are sorted automatically**
  by folder name (ties broken by full path) instead of being manually
  ordered, and every row now commits on mousedown
  (`web/src/app/spawn-panel.ts`).
- **Task-completion notifications no longer send the summary body by default.**
  `SendDone` now follows the same opt-in rule as approval notifications: only
  `session #<id>: <status label>` is sent unless `notify.include_body` is
  enabled, and when it is, the summary goes through `MaskSecrets` first.
  Previously the raw summary was sent to ntfy / webhook with truncation only.

### Fixed
- The ✕ on the action bar now suppresses the approval the same way the
  "✕ approve" control next to the input does. Previously it only hid the bar
  for 60 seconds, so the same question reappeared once the timer expired.
- The action bar could stay hidden after switching back to a session when the
  Hub had already delivered that marker. It is now redrawn whenever the bar is
  empty or not visible (`web/src/app/approval.ts`).
- A free-input option ("N. User specifies") was dropped when a single-section
  `Q1`-style block was flattened, so no free-input control appeared on the
  action bar (`web/src/app/approval-parser.ts`).
- PTY resize suppression while the approval bar is shown was cut from 60
  seconds to 350 ms, followed by one authoritative viewport-size update. This
  stops Codex from redrawing against a stale row count and fossilizing blank
  lines into the scrollback (`web/src/app/terminal.ts`).
- The preamble panel could not be expanded after being drag-resized, because
  the inline `max-height` survived; wheel events inside it no longer propagate
  to the terminal.
- Approval summary cards no longer stretch vertically and leave a large gap in
  column layouts (`web/src/styles/approval.css`).
- Dragging to select text in the Grok conversation and history viewers no
  longer clears the selection through focus recovery
  (`web/src/app.ts`, `web/src/app/attachments.ts`).
- **Session-dismiss no longer reorders other project groups.** Dismissing a
  session in one project group used to bump neighbouring groups up/down
  because their keys were not yet registered in `groupOrder`. Unknown keys
  are now auto-appended so the visible ordering is stable across dismiss /
  spawn cycles (`web/src/app/session-list.ts`).
- **Auto-dismiss after Claude Code exits is now 24 h, not 5 s.** The
  previous 5-second timer removed sessions from the list while the user
  was away from the desk, hiding recent exit context. Long-running desks
  now keep the finished session visible for up to a day.
- **Codex synchronized-update sequences pass through to xterm.js 6.0.0.**
  `ESC[?2026h` / `ESC[?2026l` were being stripped, causing Codex to
  redraw past output line-by-line on every turn. The sequence is now
  forwarded so xterm.js batches the repaint as intended.
- **`/api/debug/batch-log` diagnostic endpoint.** Temporary read-only
  endpoint for investigating the batch-approval action-bar redraw path;
  it will be removed once the underlying issue is confirmed fixed.

### Removed
- Drag-and-drop reordering of working-directory favorites in the spawn panel,
  superseded by automatic sorting (see Changed).

### Security
- **All bug-report output is redacted at every boundary** — preview, finalize,
  issue URL, and local fallback all pass through `internal/report/redact.go`.
  It masks provider and platform tokens (`sk-` / `sk-ant-api*`, `ghp_` and the
  other GitHub prefixes, `glpat-`, `xox[abprs]-`, `AIza`, `hf_`, `npm_`,
  `pypi-`, `xai-`, `gsk_`, AWS key ids, JWTs, PEM private key blocks),
  generic secrets (`?token=` / `Bearer` / `key: value` pairs / credentials in
  URLs), private paths, home directories (so the account name never leaks),
  non-loopback IP addresses, email addresses, and private hostnames.
- Attaching a session log requires a single-use preview token (32 random
  bytes, 15-minute TTL, constant-time comparison), so a gist is only created
  for content that was actually shown to you. Log reads are restricted to
  `.jsonl` files under `<log_dir>/sessions` after symlink resolution, capped
  at 512 KB, and returned gist URLs are strictly validated.
- **Second-pass audit fixes from `plan_bughunt_audit_2026-07-19.md`
  (F1 / F3 / F9 / F10 / F14 / F15 / F18 / F35).**
  - **F1**: `internal/hub/doctor_handler.go` now threads `r.Context()`
    through the diagnostics HTTP calls instead of `context.Background()`,
    so a cancelled client no longer leaks probe goroutines.
  - **F3**: `internal/hub/grok_history_handler.go` returns early when
    `os.UserHomeDir` fails instead of walking an empty root, which used
    to expand into the process CWD on some Windows setups.
  - **F9**: `internal/usagerelay` validates that the injected `hubURL`
    resolves to a loopback address before forwarding, closing an SSRF
    surface exposed to co-tenant processes.
  - **F10**: HTML escaping for approval / bug-report previews is now
    served by a single local `escapeHtml` helper that also escapes
    single quotes, replacing three drifting per-file copies.
  - **F14**: `privateNetworkBlockingDialContext` now resolves the
    hostname itself and dials the verified IP directly, eliminating a
    DNS-rebinding TOCTOU window between the allow-list check and the
    real dial.
  - **F15**: `handleNotifyConfig` now routes the incoming payload
    through `validateNotifyBackend`, so an invalid backend combination
    can no longer be persisted to `config.yaml` and crash on next load.
  - **F18**: `handleAuthLogin` performs a strict PIN pre-format check
    before hitting the crypto compare path, rejecting malformed input
    without touching timing-sensitive code.
  - **F35**: WSL "open dir" now spawns `explorer.exe <path>` directly
    instead of `cmd.exe /c start "" <path>`, closing a
    command-injection surface on paths containing `&` / `|` / `"`.
- Updated `golang.org/x/crypto` 0.52.0 → 0.54.0, `golang.org/x/net` 0.55.0 →
  0.57.0, `golang.org/x/sys` 0.45.0 → 0.47.0, `golang.org/x/term` 0.43.0 →
  0.45.0, and `modernc.org/sqlite` 1.51.0 → 1.54.0. Note that GO-2026-5932
  (x/crypto) has no fixed release upstream yet; `govulncheck` reports it at
  module level only and the affected symbols are not called by this project.

## [0.5.0] - 2026-07-13

### Added
- **`many-ai-cli setup` subcommand.** A one-shot post-install command for all
  three OSes (Windows / macOS / Linux). Creates a "Many AI Hub Start"
  shortcut on the desktop that launches `many-ai-cli serve` and opens the Hub
  in the browser. Introduced so non-technical users can go from `pnpm add -g`
  to a running Hub without touching a terminal.
- **`many-ai-cli doctor` subcommand.** Environment diagnostics that check
  provider CLIs (claude / codex / copilot / cursor-agent / grok / opencode),
  Hub port / token / ACL, Ollama and Whisper endpoints, Tailscale status,
  session log directory, and config.yaml validity. Available both as a CLI
  and via `/api/doctor` (`internal/doctor/doctor.go` +
  `internal/hub/doctor_handler.go`).
- **Auto-approval policy engine.** `~/.many-ai-cli/config.yaml` now supports
  an `autoapproval.*` section that lets you allow-list specific approval
  prompts by command / working-directory pattern, with hard-block guards
  against catastrophic patterns (`.*`, empty command). Implemented in
  `internal/autoapproval/policy.go` + `internal/hub/auto_approval.go`, with
  a simulation endpoint so you can preview what history would have been
  auto-approved.
- **`/api/git-diff` endpoint + Turn Diff viewer.** The Hub can now surface
  the working-tree git diff (tracked + untracked, with size cap and binary
  detection) for the session's cwd, so the browser Chat pane can show what
  a turn actually changed. Backed by `internal/hub/git_diff.go`.
- **`/api/grok-history` endpoint.** Reads Grok Build CLI's local session
  history (`~/.grok/`) and surfaces it in the Hub's Grok chat viewer,
  including UUIDv7 correlation and cwd matching
  (`internal/hub/grok_history_handler.go`).
- **`/api/input-config` + session activity / meta / store endpoints.** Fine-
  grained per-session metadata (labels, pin, color, note, tags, summary,
  auto-title) and activity heartbeats are now first-class API resources
  (`internal/hub/session_activity.go` / `session_meta.go` /
  `session_store_handlers.go` / `settings_handlers.go`).
- **Approval action + batch API rework.** `/api/approval/action` and
  `/api/approval/batch` are consolidated so the Hub UI can approve /
  reject / auto-rule single or multiple pending approvals in one round
  trip, with low-risk gating for `auto_rule` and deny-session support
  (`internal/hub/approval_action.go` + `approval_batch.go`).
- **Approval AI summary.** `[MANY-AI-CLI]` approval blocks now carry a
  short human-readable summary so the action bar / mobile sheet can show
  "what am I approving" at a glance (`internal/approval/summary.go`).
- **`[MANY-AI-CLI-DONE]` marker + done-summary panel.** Sessions now emit
  a completion marker with a 1-2 sentence recap that the Hub captures and
  persists per session (`internal/hub/done_summary.go`).
- **Normal-worktree mode.** Non-orchestration sessions can now opt in to
  running in a dedicated git worktree under
  `.many-ai-cli/worktrees/normal/<label>/` so parallel work on the same
  repo does not step on each other's branches
  (`internal/hub/normal_worktree.go`).
- **Commit-all rework: AI commit message + push.** The Hub's "Commit all"
  path can now generate an AI commit message from the staged diff and
  optionally push in the same round (`internal/hub/git_commit_ai.go` +
  `push.go`).
- **First-run tour.** New users are walked through the Hub's core panels
  (session list, spawn, terminals, approvals, mobile connect) on first
  load (`web/src/app/first-run-tour.ts` + `styles/first-run-tour.css`).
- **Review UI + prompt templates + session-search palette + zero-session
  empty state.** The desktop Hub gains a dedicated review view for turn
  history, a CRUD palette for reusable prompt templates, a global
  session-search palette, and a friendlier empty state when no sessions
  exist yet.
- **Launcher active-session file lock.** The unified launcher now uses a
  cross-process file lock on Windows and Unix so multiple Hub / UI
  instances no longer fight over PTY resize ownership
  (`internal/launcher/active_filelock_unix.go` +
  `active_filelock_windows.go`).
- **Regression test additions.** `dismiss_race_test.go`,
  `usage_hooks_test.go`, `approval_action_test.go`,
  `approval_batch_test.go`, `approval_detector_test.go`,
  `approval_handler_test.go`, `done_summary_test.go`,
  `git_diff_test.go`, `grok_history_handler_test.go`,
  `normal_worktree_test.go`, `orchestration_test.go`,
  `session_activity_test.go`, `session_meta_test.go`, notify's
  `approval_actions_test.go`, plus wrapper `approval_rules_test.go`.

### Changed
- **Notify subsystem.** `internal/notify/notify.go` is restructured so
  each approval action can trigger its own event, with cleaner separation
  between ntfy / webhook / Web Push backends.
- **Orchestration.** `handleSpawnChild` and companion helpers gain
  timeout retries, child-log isolation, waitForInputReady staleguard, and
  richer board.md progress markers (`internal/hub/orchestration.go` +
  `internal/orchestrate/orchestrate.go`).
- **Session log / transcript pipeline.** Repetitive-progress-line
  filtering and transcript-line size handling are now shared between the
  live PTY stream, the persisted JSONL, and the exported `.txt`
  (`internal/sessionlog/*` + `internal/sessionstore/store.go`).
- **Config.** New settings (autoapproval, input-config, setup-related
  paths, external notification body toggle) are all backward-compatible
  additions to `~/.many-ai-cli/config.yaml`
  (`internal/config/config.go`).
- **Idle state / input gate.** State transitions are re-worked to
  suppress spurious idle detections that used to cause false-positive
  "session stuck" notifications.
- **UX improvements batch #11-#20.** A grab-bag of desktop and mobile
  UI refinements (app.ts / app-entry.ts / i18n / CSS) that ship together
  under this batch label.
- **README.ja synced.** The Japanese README gains the Light orchestration
  section that had existed only in the English README.
- **Spawn panel fav / history sort.** Favorites and history results in the
  spawn cwd picker are now sorted by manual order (favorites) and most
  recent first (history), instead of a single raw score.

### Fixed
- **Pre-release audit findings F1-F17** (see
  `docs/local/report_audit-v0.5.0_2026-07-13.md`).
  - **F1 / F3**: Bumped Go toolchain to 1.25.12 to clear
    `GO-2026-5856` (crypto/tls ECH privacy leak) and `GO-2026-4970`
    (os symlink + trailing slash root escape). `govulncheck ./...`
    now reports zero reachable findings (`go.mod`).
  - **F2**: `applyOneTapApproval` no longer leaves the pending map
    inconsistent when the PTY send fails; the pending entry is now
    restored so a retry is possible
    (`internal/hub/approval_action.go`).
  - **F4**: Orchestration board `## DONE` markers are validated
    against the emitting child's source file, closing an IDOR that
    let a sibling child forge another child's completion
    (`internal/hub/orchestration.go`).
  - **F5**: `ClearSessionHistory` now nulls the derived columns
    (`summary`, `auto_title`, `last_event_at`) so the session card
    does not keep displaying stale metadata after a clear
    (`internal/sessionstore/store.go`).
  - **F6**: Approval-rules injection uses an 8 MiB `bufio.Scanner`
    buffer so long AGENTS.md / CLAUDE.md files no longer fail
    silently at the 64 KiB default limit
    (`internal/wrapper/approval_rules.go`).
  - **F7**: ntfy / webhook approval and done-summary bodies are now
    opt-in via `notify.include_body` and pass through `MaskSecrets`
    when enabled (`internal/config/config.go`,
    `internal/notify/notify.go`, `internal/hub/push.go`).
  - **F8**: `internal/doctor` now checks the `http.NewRequestWithContext`
    error before use, eliminating a nil-panic path on malformed URLs
    (`internal/doctor/doctor.go`).
  - **F9**: Autoapproval `working_dir` regexes are compiled once at
    `Load` time and validated up front, instead of being recompiled
    on every evaluation (`internal/autoapproval/policy.go`).
  - **F10**: `autoapproval.AddRule` writes through a path-scoped
    mutex and a temp-file + rename atomic write, so concurrent rule
    edits can no longer corrupt `config.yaml`
    (`internal/autoapproval/policy.go`, `internal/securefile/atomic.go`).
  - **F11**: Added dedicated tests for `approval_batch.go`
    (`TestApprovalBatchAutoRuleRequiresLow`,
    `TestApprovalBatchDenySession`,
    `TestApprovalBatchApproveSkipsMidHigh`).
  - **F12**: Added `internal/doctor` test coverage
    (`TestOllamaHTTPChecks`, `TestWhisperHTTPChecks`,
    `TestTokenAndACLPermissions`).
  - **F13**: Consolidated duplicated code between `wrapperLoop` and
    `reattachLoop` in `internal/hub/wrapper_loop.go`.
  - **F14**: Split `handleSpawnChild` (cyclomatic > 15) into smaller
    helpers (`internal/hub/orchestration.go`).
  - **F15**: `pruneSessionRow` no longer races with in-flight
    `StoreEvent` calls; events for ended sessions are now skipped
    instead of resurrecting the row
    (`internal/sessionstore/store.go`).
  - **F16**: `/api/git-diff` now streams untracked files through
    `io.LimitReader` at the size cap, so a large untracked binary
    can no longer OOM the Hub (`internal/hub/git_diff.go`).
  - **F17**: `StartSession` picks a unique `virtual-live-<ts>` value
    for the `jsonl_path` column when the session has no on-disk
    log, avoiding a UNIQUE-constraint collision when multiple such
    sessions coexist (`internal/sessionstore/store.go`).

- Numerous mobile / desktop bugfixes documented individually in
  `docs/local/bugfix_*_2026-07-*.md` (session card tap-miss, codex
  SIGWINCH full-repaint scroll, OAuth login Ctrl+U prefix, Grok stop
  key ESC vs Ctrl+C, Claude Code TUI image attach regression, statusline
  settings skip, new-session startup latency 3x, disconnected-session
  dismiss ghost, single-line send CR absorbed, approval-only header
  button, done-summary history-panel removal, card message low-contrast
  color, star/pin unification, session card label gray / branch push-
  out, Grok approval options glued, orchestration tab icon mismatch).

## [0.4.0] - 2026-07-05

### Removed
- **Workbench tab and Hub-internal chat proxy retired.** The `Workbench` tab
  (SQLite-backed session history browser, timeline/summaries/redacted exports,
  prompt templates, task/policy notes, diagnostics, usage summaries,
  stale-session and worktree helpers) and the associated `/api/workbench/*`
  routes have been removed. The Hub-internal chat proxy that intercepted
  `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` to capture request payloads
  (`internal/proxy/`, `internal/hub/chat_proxy*.go`) has also been removed;
  wrapped `claude` / `codex` sessions now talk directly to the upstream API.
- **Side effect: Sonnet 5+ default 1M context restored under the Hub.** Because
  Claude Code no longer sees a rewritten `ANTHROPIC_BASE_URL` pointing at an
  unknown gateway, it stops downgrading to the 200K cap and honours the
  Sonnet 5-era default 1M context window again. No more per-model
  `claude-sonnet-5[1m]` workaround. Ollama / LM Studio routing (which legitimately
  needs a rewritten base URL) is unaffected.
- Temporary `/api/debug/cursor-hide-log` diagnostic endpoint (introduced and
  removed within the v0.4.0 cycle).

### Added
- **Grok Build CLI provider support.** `many-ai-cli grok` (and `wrap grok`)
  now wraps xAI's official `grok` terminal coding agent in a PTY, joining
  Claude Code / Codex CLI / GitHub Copilot CLI / Cursor Agent CLI as the fifth
  supported provider. It is handled like Cursor Agent — `grok --model <id>`
  passthrough with no route or env injection, the shared approval-rules block
  injected into the project-root `AGENTS.md`, and official approval patterns
  from `resources/approval-patterns/grok.md`. `/api/spawn` validation,
  diagnostics, slash-command/approval-pattern sources, and the model picker
  were extended accordingly. Requires a SuperGrok or X Premium+ subscription;
  `many-ai-cli` never reads, stores, or proxies xAI session tokens.
- **opencode CLI wrapper.** `many-ai-cli opencode` (and `wrap opencode`) wraps
  the community `opencode` CLI in a PTY. Instead of pattern-scraping approval
  prompts, the Hub writes the provider's own `opencode.json` (`permission`
  field) into the session cwd: normal sessions get `ask` (surface the approval
  to the Hub UI), while orchestration child sessions get `allow` (bypass the
  provider's own approval so the conductor stays in charge). The original
  `opencode.json` is restored on session end.
- **Light orchestration API.** `POST /api/sessions/:id/spawn-child` lets a
  conductor session spawn child AI sessions with a role, provider, model,
  initial prompt, and optional cwd. The Hub creates
  `~/.many-ai-cli/orchestration/<orchestration_id>/board.md`, injects the
  board path into the child prompt, and watches the board for appended
  progress and `## DONE <role> session=<child_id>` markers. When the parent
  cwd is a git repository, children run in dedicated worktrees under
  `.many-ai-cli/worktrees/<orchestration_id>/<role>` by default. The Hub does
  not auto-merge child branches; the conductor / user decides what to merge.
  Also ships companion improvements: send-side duplicate guard, idle-false-
  positive suppression, board notification path isolation, and a
  `waitForInputReady` staleguard (`internal/wrapper/staleguard.go`).
- **VT-based approval marker detection.** In addition to the existing
  xterm.js buffer scan, the Hub now detects `[MANY-AI-CLI]` markers directly
  from the Hub-side VT stream (`internal/hub/approval_marker.go`). This
  catches approval prompts that the pure xterm scan used to lose to CUP
  positioning or the cursor-hide filter (a recurring class of v0.3.x bugs).
- **Configurable Ollama daemon base URL.** `ollama.base_url` in
  `~/.many-ai-cli/config.yaml` can now point Ollama routing at any HTTP(S)
  daemon reachable from the Hub process, not just `localhost:11434`. This
  supports host/guest setups such as Windows host + Hyper-V guest or Windows
  host + WSL guest: `/api/models` reads `<base_url>/api/tags`, Claude Code gets
  `ANTHROPIC_BASE_URL=<base_url>`, and Codex gets
  `OPENAI_BASE_URL=<base_url>/v1`.
- **Mobile UX overhaul.** A dedicated mobile home screen
  (`web/src/app/mobile-home.ts` + `web/src/styles/mobile-home.css`), a
  lightweight mobile terminal (`mobile-terminal-lite.ts`), and a mobile
  transcript view (`mobile-transcript.ts`) replace the previous
  responsive-only layout. The redesign covers the approval sheet, drawer,
  swipe-based navigation gestures, and PWA polish (manifest, icons, service
  worker updates).
- **Claude Code skill-derived slash-command auto-discovery.**
  `internal/hub/slash_cmd_fetch.go` (`discoverSkillSlashCmds`) walks
  `~/.claude/skills/*/SKILL.md` and surfaces registered skills in the Hub's
  slash-command listing.
- **Attachment filename sanitization.** `sanitizeFilename` in
  `internal/attach/store.go` keeps the original filename visible while
  stripping path-traversal and unsafe characters from the on-disk path.
- **Consolidated build info.** `internal/hub/buildinfo.go` centralizes the
  version / build metadata exposed by `/api/info` and the About panel, so the
  ldflags-injected values have a single source of truth on the server side.
- **Session log noise filter.** A repetitive-progress-line filter is applied
  when writing the clean transcript (`internal/sessionstore/store.go` +
  `noise_output_test.go`), so re-drawn progress bars no longer dominate the
  saved `.txt`.
- **Regression tests** for input splitting (`internal/wrapper/input_split_test.go`)
  and the cursor-hide filter (`web/src/app/cursor-hide-filter-fixtures.ts`).

### Changed
- README (English / Japanese) opening hook rewritten around "run multiple AI
  CLIs in parallel and approve from your phone" so the primary value
  proposition is clear from the first paragraph.
- Ollama routing env-var assembly now flows through
  `EnvPresetForWithOllamaBase`, deriving `ANTHROPIC_BASE_URL` /
  `OPENAI_BASE_URL` from `ollama.base_url` in one place.
- Windows resource metadata (`winres/winres.json` and the regenerated
  `rsrc_windows_*.syso`) is bumped to `0.4.0` so the executable properties
  dialog matches the release tag.
- The session-list / spawn-panel / settings / terminal / token-statusbar
  frontends were reorganized to share layout primitives with the new mobile
  UI without regressing the desktop Hub.

### Fixed
- Hotfix → develop merges no longer leave the develop-side version constant
  stale (`develop-version-stale-v0.3.4`).
- Cursor Agent no longer drops chunks of larger inputs that contain a Ctrl+U
  (`cursor-agent-ctrl-u-chunk-input-drop`).
- Grok history viewer no longer renders blank rows caused by leftover SGR
  state (`grok-history-viewer-sgr-blank`), and the Grok build display label
  no longer breaks alignment (`grok-build-display-label`).
- Grok responses and opencode pickers are no longer eaten by the cursor-hide
  filter (`grok-response-discarded-by-cursor-hide-filter`,
  `opencode-picker-discarded-by-cursor-hide-filter`).
- The orchestration conductor's initial prompt is now submitted with an
  Enter (`orchestration-conductor-initial-prompt-not-submitted`), and
  codex child sessions spawn reliably again
  (`orchestration-codex-child-spawn-failures`).
- Mobile drawer no longer shows a transparent background layer or lingers on
  PC-width viewports (`mobile-drawer-bg-layer-transparent`,
  `mobile-drawer-residue-on-pc-width`). The floating hamburger, header height,
  and tab bar dead space were also fixed
  (`mobile-header-band-floating-hamburger`, `mobile-header-height-reduction`,
  `mobile-header-tab-bar-dead-space`).
- Empty input bar no longer renders a stale placeholder height
  (`input-bar-empty-placeholder-height`), and clicks in the empty wrap area
  no longer lose focus (`input-wrap-empty-area-focus`).
- The Hub marker filter no longer swallows CUP positioning, so layouts
  driven by cursor positioning stay intact
  (`hub-marker-filter-strips-cup-positioning`).
- Batch approval Q-tabs no longer overflow the action bar
  (`batch-approval-tabs-overflow`), and marker blocks no longer collapse
  onto a single line (`marker-block-line-collapse`).
- The spawn cwd chip no longer collides with a full-path prefix
  (`spawn-cwd-chip-fullpath-prefix-collision`).
- Approval popups no longer leak across sessions
  (`approval-popup-cross-session-leak`).
- Local-LLM routing fixes for Qwen via LM Studio / Ollama
  (`local-llm-qwen-lmstudio-ollama-unusable`).
- **WebGL terminal renderer works again (vendored xterm.js generation mismatch).**
  The vendored `xterm-addon-webgl.min.js` had been updated to 0.19.0 (the
  xterm.js 6.0.0 generation, which requires the `mainDocument` API of the
  terminal core), while `xterm.min.js` and the fit / unicode11 / web-links
  addons were still the older 5.x-generation files — only the license ledger
  and the About dialog had been bumped to 6.0.0. Every `loadAddon` call then
  threw `Cannot read properties of undefined (reading 'createElement')` inside
  the addon, so the UI silently fell back to the DOM renderer on each session
  switch, re-introducing the fullwidth-glyph / selection-highlight drift the
  WebGL renderer exists to fix. The vendored core (`xterm.min.js`,
  `xterm.min.css`) and the three addons are now the actual 6.0.0-generation
  artifacts, matching the ledger.
- **Orchestration fallback ID no longer collides across Hub restarts.** When a
  conductor started a child via `spawn-child` without going through
  `/api/orchestration/create`, the orchestration ID was `s<parentSessionID>`
  (e.g. `s3`). After a Hub restart the session number was reassigned from #1,
  so the same conductor session ID could reuse the previous run's
  `~/.many-ai-cli/orchestration/s<N>/board.md` (header/purpose stayed from the
  old run) and its `child-<sessionID>.md` history. The fallback is now
  `s<parentSessionID>-<UnixNano>`, which is unique per Hub start. Existing
  `s<N>/` folders are harmless leftovers and can be moved aside or deleted
  manually.
- **No more scroll residue when switching between terminals.** `attachTerminal`
  used to append the terminal DOM node first and then flush the buffered
  chunks that had accumulated while the tab was hidden, which made the target
  tab briefly show the pre-switch viewport and then scroll down through the
  buffered output. `flushPending` now takes an `onDrained` callback, and
  `attachTerminal` flushes first — appending the container, releasing hidden
  WebGL renderers, scrolling to the bottom, and fitting the terminal only
  after the buffered chunks are fully written.

## [0.3.4] - 2026-06-22

### Fixed
- Replace real-data test fixtures and docs with synthetic equivalents
  (kb-derived names detected by the cross-repo secrets-scan sweep on
  2026-06-22). Parser validation coverage (long Japanese preamble, full-width
  punctuation, line-wrapped `(Y:1/N:0)` shape, 3-item `・`-separated list) is
  preserved.

## [0.3.3] - 2026-06-20

### Fixed
- **Spawn no longer overrides the CLI's default model.** When a session was
  spawned from the UI with the model field left on *auto*, the Hub re-injected
  the previous `last_model` as an explicit `--model` argument. This silently
  overrode the model the user had picked with the CLI's own `/model` command —
  in particular it clobbered the 1M-context Opus variant on Max plans and pinned
  the session back to a 200K window. The Hub now omits `--model` unless a model
  is explicitly selected, so the CLI's own default (including the auto-granted
  1M window) is respected. The spawn panel also stops pre-filling the model
  field from the saved value (`spawn-model-override`).

### Added
- 1M-context Opus variants (`claude-opus-4-8[1m]` / `claude-opus-4-7[1m]`) are
  now selectable in the spawn model list for users who want to pin them
  explicitly.

## [0.3.2] - 2026-06-16

### Added
- **Built-in API proxy + Payload chat view (β).** The Hub now embeds a
  transparent HTTP proxy for `https://api.anthropic.com/v1/messages`,
  `https://api.openai.com/v1/chat/completions`, and
  `https://api.openai.com/v1/responses`. Wrapped CLIs (Claude Code / Codex CLI)
  are pointed at the local proxy via `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL`
  env injection, so every request/response is captured per session without TLS
  MITM or CA distribution. A new "Payload 表示 (β)" toggle on the chat pane
  shows a clean, structured turn list sourced from the proxy (independent of
  PTY scraping), and each turn has a `[Raw]` toggle that opens the raw JSON.
  `Authorization` headers are stripped before the payload reaches the in-memory
  sink — credentials are never persisted.
- **Tailscale wizard: inline "Enable external exposure" button.** The mobile
  connect wizard step ④ now ships a one-click button that flips the Tailscale
  serve toggle from inside the wizard instead of bouncing the user back to the
  toolbar.
- README (English / Japanese) now flags the smartphone section as a
  beta / draft preview shipped in v0.3.x.

### Fixed
- **Approval popup garbage.** ANSI escape leftovers in the action-bar question
  panel (`approval-popup-stripansi-garbage`), the missing option 1 in
  wrapped-fallback layouts (`approval-fallback-wrapped-opt1-missing`), the free
  input misalignment in inline-right layouts (`approval-free-input-inline-right`),
  the Q-tab shrinkage when a batch preamble was long
  (`batch-preamble-shrinks-qtabs`), and the preamble's table/rule garbage are
  all addressed by moving the preamble inside the marker and stripping any
  rules/tables that slip through.
- **Token statusbar / cost popover.** The cost popover no longer ends up
  visually behind the statusbar (`token-statusbar-cost-popover-under-statusbar`).
- **Live status palette popover.** No longer clipped at the terminal edge
  (`live-status-palette-popover-clipped`).
- **Spawn cwd.** Typed paths are now existence-checked before the session is
  created (`spawn-cwd-existence-unchecked-for-typed-path`).
- **Session card order.** Eliminated the random-snapshot reorder that swapped
  card positions on refresh (`session-card-order-random-snapshot`).
- **Card branch badge.** Restored right padding on the branch badge
  (`card-branch-badge-right-padding-gap`).
- **AI commit message marker.** Stripped the stray bullet prefix that leaked
  into the generated commit message (`commit-ai-marker-bullet-prefix`).
- **Project group grid button style.** Aligned with the rest of the grid
  (`project-group-grid-btn-style`).
- **Quick-command settings.** Restored vertical breathing room
  (`quick-cmd-settings-vertical-compress`).
- **Settings confirm.** Confirmation now uses the toast pattern instead of
  closing the panel (`settings-confirm-closes-panel-toast-unify`).
- **Workflow modal.** Spinner now settles correctly
  (`workflow-modal-spinner-not-settling`); progress bar percentage is readable
  again (`workflow-progress-bar-pct-unreadable`).
- **Mobile statusbar overflow.** Long status text no longer pushes the
  statusbar off the viewport (`mobile-statusbar-overflow`).
- **Mobile WebSocket auth.** The mobile flow now falls back to the cookie when
  the URL token is unavailable (`mobile-ws-token-cookie-fallback`).

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

[Unreleased]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.3.4...v0.4.0
[0.3.4]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.2.0...v0.2.2
[0.2.0]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/ishizakahiroshi/many-ai-cli/releases/tag/v0.1.3
[0.1.2]: https://github.com/ishizakahiroshi/many-ai-cli/releases/tag/v0.1.2
[0.1.1]: https://github.com/ishizakahiroshi/many-ai-cli/releases/tag/v0.1.1
