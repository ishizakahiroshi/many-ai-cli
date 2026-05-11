# ai-cli-hub

![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey)
![License](https://img.shields.io/badge/license-MIT-green)
![Go](https://img.shields.io/badge/go-1.22+-blue)

<!-- TODO: Add screenshot here -->

A local web dashboard to manage multiple AI coding CLIs (Claude Code / Codex CLI) from a single screen — approvals, monitoring, and terminal in one place.

[日本語版 README はこちら](README.ja.md)

---

## Quick Download

Get the latest release from [GitHub Releases](https://github.com/ishizakahiroshi/ai-cli-hub/releases/latest).

| Platform | Download |
|----------|----------|
| Windows  | `ai-cli-hub-windows-amd64.exe` |
| macOS    | `ai-cli-hub-darwin-amd64` / `ai-cli-hub-darwin-arm64` |
| Linux    | `ai-cli-hub-linux-amd64` |

> Settings and logs are stored in `~/.ai-cli-hub/` (created on first run).
> Session logs contain user input and AI output. Treat them as sensitive data.

### Windows Smart App Control Notice

The Windows release binary is not currently Authenticode-signed. On Windows 11
PCs where Smart App Control is enabled, Windows may block `ai-cli-hub.exe` as an
untrusted app. This is separate from checksum verification: `SHA256SUMS.txt` is
signed for release integrity, but the `.exe` itself is not code-signed.

### Platform Verification for v0.1.3

- Verified in real environment: Windows
- Not yet verified in real environment: Linux, macOS

Linux/macOS builds are expected to work, but they have not been fully validated in real environments for v0.1.3.
Please use at your own discretion and report any issues.

### Verify Release Artifacts (Checksum + Signature)

`v0.1.2` and later releases include:

- `SHA256SUMS.txt`
- `SHA256SUMS.txt.sig`
- `SHA256SUMS.txt.pem`

1. Verify the signature on `SHA256SUMS.txt`:

```bash
cosign verify-blob \
  --certificate SHA256SUMS.txt.pem \
  --signature SHA256SUMS.txt.sig \
  --certificate-identity-regexp "https://github.com/ishizakahiroshi/ai-cli-hub/.github/workflows/release.yml@refs/tags/v.*" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  SHA256SUMS.txt
```

2. Verify your downloaded binary against the checksums:

```bash
sha256sum -c SHA256SUMS.txt
```

---

## Features

- **Unified approval panel** — approve/reject Claude Code and Codex CLI prompts from the browser
- **Real-time PTY output** via xterm.js over WebSocket
- **Image attach** — paste or drag-and-drop images into the terminal session
- **Voice input** — speak your prompt using the browser's Speech Recognition API (Chrome / Edge)
- **Multi-session view** — switch between multiple AI CLI sessions in one tab
- **Spawn new sessions** from the UI (`/api/spawn`)
- **Language switching** (English / Japanese)
- **Local-first UI** — Hub HTTP/WebSocket server binds to `127.0.0.1` only; no telemetry from `ai-cli-hub` itself

---

## Quick Start (Recommended)

The normal flow: launch the binary, then drive everything from the browser. You do not need to run any CLI command yourself.

1. Download `ai-cli-hub.exe` (or the macOS / Linux binary) from the table above
2. **Double-click `ai-cli-hub.exe`** (or run `ai-cli-hub` with no arguments)
   - The Hub starts and your browser opens automatically at `http://127.0.0.1:47777/?token=<token>`
   - If a Hub is already running, your browser is reopened against the existing instance
3. In the Hub UI, click **"+ New Session"** to launch a Claude Code / Codex CLI session
4. When an approval prompt appears, an action bar shows up under the input — click a button or use the keyboard to respond

Sessions can be created, monitored, and approved entirely from the Hub UI; you do not need to keep a separate terminal open.

> **⚠ About the console window**
> Double-clicking the binary opens a console window alongside the browser. **That console *is* the Hub server process** — closing it with `×` terminates the Hub. If it gets in the way, **minimize** it instead of closing it.
> If the Hub does go down (whether by `×`, a crash, or a manual restart), running AI sessions wait up to **60 minutes** for the Hub to come back before terminating themselves (configurable in `config.yaml` up to 24 hours — extend it for long-running autonomous tasks). A Web UI bug or restart will not silently kill your work. See [Shutdown, zombie protection & Hub crash resilience](#shutdown-zombie-protection--hub-crash-resilience) for details.
> To stop the Hub intentionally, use the `⏻` button in the top-right of the Hub UI, or run `ai-cli-hub stop` from another terminal.

---

## Launching from a terminal (advanced)

If you prefer driving things from a shell — for scripting, shell integration, or muscle memory — these options are equivalent to clicking "+ New Session" in the UI. Use whichever you like.

### Option A: provider as a subcommand

```bash
ai-cli-hub claude      # auto-starts Hub in the background if not running, then launches Claude
ai-cli-hub codex       # same
```

You do not need to run `ai-cli-hub serve` first.

### Option B: `wrap` subcommand (for debugging)

```bash
ai-cli-hub wrap claude
ai-cli-hub wrap codex
```

Functionally identical to Option A; useful when you want to be explicit about the wrapper layer.

### Option C: transparent mode (`AI_CLI_HUB_AUTO`)

Initialize the shell once, then your normal `claude` / `codex` commands transparently go through the wrapper.

> `ai-cli-hub shell-init` emits **POSIX shell (bash / zsh) only** function definitions. There is no PowerShell snippet — see below for a manual alternative.

```bash
# Run once per shell startup (bash / zsh)
eval "$(ai-cli-hub shell-init)"

# Turn on per-session — only the shells where you opt in are wrapped
export AI_CLI_HUB_AUTO=1
claude    # → goes through the wrapper, auto-starts Hub if needed
codex     # → same
```

Without `AI_CLI_HUB_AUTO=1`, `claude` / `codex` behave exactly as the original commands. No global `.bashrc` modification.

#### OS-specific automation examples

**PowerShell (Windows)**

Add the following to your `$PROFILE` (since `shell-init` does not support PowerShell, the functions are defined directly):

```powershell
if ($env:AI_CLI_HUB_AUTO -eq '1') {
    function claude { ai-cli-hub claude @args }
    function codex  { ai-cli-hub codex  @args }
}
```

Set `AI_CLI_HUB_AUTO=1` on a specific Windows Terminal profile to enable transparent mode only in that tab:

```jsonc
{
  "name": "AI Watch",
  "commandline": "pwsh.exe -NoExit",
  "environment": { "AI_CLI_HUB_AUTO": "1" }
}
```

**iTerm2 (macOS)**

- Profiles → Environment → Variables: `AI_CLI_HUB_AUTO=1`
- Profiles → General → Send text at start: `eval "$(ai-cli-hub shell-init)"`

**tmux (all OSes)**

```bash
# ~/.tmux.conf
set-option -g default-command "AI_CLI_HUB_AUTO=1 bash -c 'eval \"$(ai-cli-hub shell-init)\"; exec bash'"
```

---

## Settings

Settings are stored in `~/.ai-cli-hub/config.yaml` (auto-created on first run).

| Key | Description | Default |
|-----|-------------|---------|
| `port` | Hub port | `47777` |
| `language` | UI language (`en` or `ja`) | `en` |
| `auto_open_browser` | Auto-open browser on Hub start | `true` |

---

## Hub UI

Open `http://127.0.0.1:47777/?token=<token>` in your browser.

```
┌─ AI-CLI-HUB  [1][0][6] │ ● Claude:2  ● Codex:5         [⏻] [Settings] ─┐
├──────────────────────────┬──────────────────────────────────────────────┤
│ [+ New Session]          │ ● Codex  cwd: C:\dev\ai-cli-hub  [↑ to top] │
│ 📁 ai-cli-hub  [1][0][6] │ Terminal output — Windows PowerShell         │
│ ─────────────────────── │                                              │
│ ★ #7 ● Codex  Running × │   (xterm.js terminal output)                │
│    Last: 00:11:57       │                                              │
│    docs/local/plan_…    │                                              │
│                         │                                              │
│ ☆ #6 ● Codex  Standby × │                                              │
│    Last: 00:05:48       │   ┌─ Approval (only when waiting) ──────┐    │
│    docs/local/plan_…    │   │ Command: npm install axios          │    │
│                         │   │ Risk: MEDIUM                        │    │
│ ☆ #4 ● Claude Standby × │   │ [YES (y)] [NO (n)]                  │    │
│    Last: 23:00:38       │   └─────────────────────────────────────┘    │
│    Mostly local exec…   │ ─────────────────────────────────────────── │
│                         │ [📎] Input  auto mode on (shift+tab)        │
│   …(rest omitted)…      │      [Send] [🪄] [/clear] [/model] [/]      │
└──────────────────────────┴──────────────────────────────────────────────┘
  Header chips [1][0][6] = "running / waiting / standby" session counts
```

### Layout

- **Header**
  - Status summary chips `[running][waiting][standby]` (the waiting chip blinks when > 0) and per-provider connection counts `Claude:N / Codex:N`.
  - Right edge: `⏻` (stop the Hub) and `Settings` (language, theme, timeouts, log dir, etc.).
- **Left sidebar (session list)**
  - Top: `+ New Session` button (opens the spawn dialog).
  - Sessions are grouped by **project folder** (the directory where the wrapper was launched). Each group shows its own session-count chips.
  - Each session card: `★` (favorite) / `×` (close) / provider-colored dot + ID + state badge (Running / Standby / Waiting / Completed / Error / Disconnected) / last response time / one-line preview of recent output.
  - Completed and errored sessions stay in the list until you click `×`.
- **Right pane (terminal + input)**
  - Top bar: active session's provider and cwd, plus `↑ to top` to scroll the PTY buffer back to the start.
  - Center: PTY output rendered live with xterm.js.
  - Bottom: multi-line input box, attach / send buttons, slash-command picker (`/clear`, `/model`, `/`), and the auto-mode toggle hint `shift+tab`.
- **Approval action bar**: appears above the input when an approval is pending. Click a button, or focus with `←` / `→` and press `Enter`.
- **Sync with terminal input**: if you resolve the prompt by typing `y` / `n` directly in the terminal, the action bar disappears automatically.
- **Image attach**: paste or drag-and-drop into the attach area; the file is materialized locally and a path reference is injected into the PTY on send.

### Usage notes

- **Approval**: When an AI CLI requires approval, an action bar appears above the input. Click the button or navigate with `←` / `→` and confirm with `Enter`.
- **Terminal input**: Type directly in the input field and press `Enter` to send. Use `Shift+Enter` for a newline.
- **Image attach**: Paste (`Ctrl+V`) or drag-and-drop an image onto the attach area to inject a local file path reference into the session.
- **Voice input**: Click the 🎤 button or press `Alt+V` to start voice input. Click again or press `Alt+V` to stop. Transcribed text is inserted into the input field.
  - Requires Chrome or Edge (uses the browser's built-in Speech Recognition API).
  - Microphone permission must be granted on first use.
- **Auto-submit trigger**: In Settings → Auto-submit trigger, enable the toggle and set a phrase (e.g. `send`). When voice recognition finishes with that phrase, the input is sent automatically. Also works with typed text.
- **Spawn**: Click **+ New Session** to start a new AI CLI session from the browser.

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Shift+Enter` | New line in input |
| `Tab` / `Shift+Tab` | Switch to next / previous session |
| `←` / `→` | Move focus between action bar buttons (when action bar is visible and input is empty) |
| `Enter` | Click focused action bar button |
| `Alt+V` | Toggle voice input on/off |
| `Ctrl+V` | Paste image into attach area |
| `Ctrl+C` | Send SIGINT to PTY (or copy selected text) |
| `Ctrl+D` | Send EOF to PTY |
| `Ctrl+O` | Expand Claude Code folded content |

---

## Shutdown, zombie protection & Hub crash resilience

Two goals are balanced here:

1. Don't let child AI sessions (Claude Code / Codex CLI) keep running — and billing — when the user has clearly walked away.
2. Don't lose in-flight AI work just because the Hub Web UI hit a bug, was restarted, or its console window was closed.

When the wrapper's WebSocket to the Hub drops, the wrapper **probes the Hub's HTTP endpoint** to tell *intentional disconnects* from *Hub crashes*:

| Scenario | Wrapper behavior |
|---|---|
| **Intentional disconnect** — UI `×` (dismiss), "stop everything", or idle-timeout fired<br>(Hub HTTP responds normally) | Kill the PTY child (`claude` / `codex`) **immediately**. No grace period. |
| **Hub crash / `.exe` console closed**<br>(Hub HTTP unreachable) | Retry dial+register every 2 s for up to `wrapper_reconnect_grace_sec` (default **3600 s = 60 min**).<br>　• If Hub comes back: re-register as a new session, replay the last 64 KB of PTY output to the UI, and resume.<br>　• If the grace expires with Hub still down: kill the PTY. |
| **Browser closed but Hub still running** (no UI connected) | After `idle_timeout_min` minutes (default 60), the Hub force-disconnects every wrapper, which is then handled as the "intentional disconnect" row above. |

> **Why**: this lets you recover from a Hub-side bug, panic, or manual restart without losing your AI session — as long as the Hub comes back within the grace window. For long-running autonomous tasks (multi-hour agent loops), bump `wrapper_reconnect_grace_sec` up to e.g. 12 h (`43200`). Cases where the user *meant* to stop (dismiss, "stop everything", browser closed and forgotten) still terminate sessions promptly.

Configuration knobs in `~/.ai-cli-hub/config.yaml`:

- `hub.wrapper_reconnect_grace_sec` — `0` disables reconnect (legacy "kill immediately" behavior). Range `0`–`86400` seconds (up to 24 h). Default `3600` (60 min). Also editable in Settings (in minutes). **Applies to new sessions only** — running sessions keep the value they were spawned with.
- `hub.idle_timeout_min` — how long the Hub keeps wrappers alive when no UI is connected. `0` disables. Range `0`–`1440` minutes. Also editable in Settings.

For a clean shutdown, prefer the `⏻` button in the Hub UI top-right or `ai-cli-hub stop`; closing the console window now leaves wrappers waiting for the Hub to come back rather than killing them right away.

---

## Architecture

```
AI CLI (claude / codex)
    └─ ai-cli-hub wrap  <── PTY wrapper
           │ WebSocket
    ┌──────▼──────┐
    │  Hub Server │  127.0.0.1:47777
    └──────┬──────┘
           │ WebSocket
    ┌──────▼──────┐
    │ Browser UI  │  xterm.js / Vanilla JS
    └─────────────┘
```

The Hub server acts as a relay between PTY sessions and the browser UI. Each AI CLI runs inside a PTY wrapper that forwards I/O over WebSocket to the Hub, which in turn serves the browser UI.

## Logs

| Type | Path | Content |
|---|---|---|
| Hub log | `~/.ai-cli-hub/logs/hub.log` | Hub server runtime logs (rotated by lumberjack; configured via the `log:` section) |
| Session raw log | `~/.ai-cli-hub/logs/sessions/<provider>_<YYYY-MM-DD_HHMMSS>_<folder>_s<id>.log` | Raw PTY stream for each wrapped session (includes ANSI sequences) |
| Session history | `~/.ai-cli-hub/logs/sessions/<provider>_<YYYY-MM-DD_HHMMSS>_<folder>_s<id>.jsonl` | Structured session events (`session_start`, `user_input`, `pty_output`, `attach`, `session_end`, `session_dismiss`) |

The Hub UI log-path button copies the log directory path to your clipboard.

---

## Security / Privacy

- The Hub HTTP/WebSocket server binds to `127.0.0.1` only — external hosts cannot reach it directly
- Random token in URL prevents unauthorized local access
- `ai-cli-hub` itself sends no telemetry or usage data to any service

### Outbound network traffic

`ai-cli-hub` is local-first, but the following outbound HTTPS requests can occur and you should be aware of them:

- **Slash command list (Hub itself)** — When the slash command picker is opened, the Hub fetches a markdown file from `https://raw.githubusercontent.com/ishizakahiroshi/ai-cli-hub/main/resources/slash-commands/{claude,codex}.md` and caches it for 24 hours. The source URL can be changed (or pointed to a local file path) in **Settings → Slash command sources**.
- **Wrapped CLI traffic (the CLIs themselves)** — The CLIs you wrap (Claude Code, Codex CLI) talk directly to their respective vendor APIs (Anthropic, OpenAI) over HTTPS. `ai-cli-hub` only relays PTY I/O via local WebSocket; it does not intercept, log, or proxy these API requests. Whatever network behavior the underlying CLI has applies as-is.

### ⚠️ Important: Localhost-only by design

`ai-cli-hub` is designed to run on the **same machine** as your browser. Do **not**:

- Run `serve` on a remote server (VPS / cloud) and connect to it from another host
- Modify the bind address to anything other than `127.0.0.1` (e.g. `0.0.0.0`, LAN IP)
- Expose the Hub UI through a reverse proxy (nginx / Caddy / etc.)
- Share the Hub URL (with its token) with anyone

The Hub UI exposes APIs that perform host-level actions (e.g. `/api/open-dir` opens folders in the OS file manager). These are only safe under the localhost assumption — exposing them externally could lead to **arbitrary folder access or information disclosure**.

---

## Build from Source

Requires Go 1.22+.

```bash
git clone https://github.com/ishizakahiroshi/ai-cli-hub.git
cd ai-cli-hub
go build -o ai-cli-hub ./cmd/ai-cli-hub
```

#### Cross-compilation

```bash
GOOS=windows GOARCH=amd64 go build -o dist/ai-cli-hub-windows-amd64.exe ./cmd/ai-cli-hub
GOOS=darwin  GOARCH=amd64 go build -o dist/ai-cli-hub-darwin-amd64      ./cmd/ai-cli-hub
GOOS=darwin  GOARCH=arm64 go build -o dist/ai-cli-hub-darwin-arm64      ./cmd/ai-cli-hub
GOOS=linux   GOARCH=amd64 go build -o dist/ai-cli-hub-linux-amd64       ./cmd/ai-cli-hub
```

---

## License

MIT — see [LICENSE](LICENSE) for details.

Third-party dependency notices are provided in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), and the vendored/browser-side license texts are provided in [web/src/vendor/THIRD_PARTY_LICENSES.txt](web/src/vendor/THIRD_PARTY_LICENSES.txt).

---

## Disclaimer

This tool is provided as-is without warranty. Use at your own risk.
