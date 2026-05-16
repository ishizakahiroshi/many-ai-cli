# any-ai-cli

![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey)
![License](https://img.shields.io/badge/license-MIT-green)
![Go](https://img.shields.io/badge/go-1.22+-blue)

![any-ai-cli dashboard](assets/readme-dashboard.jpg)

A local web dashboard to manage multiple AI coding CLIs (Claude Code / Codex CLI) from a single screen — approvals, monitoring, and terminal in one place.

[日本語版 README はこちら](README.ja.md)

---

## Quick Download

Get the latest release from [GitHub Releases](https://github.com/ishizakahiroshi/any-ai-cli/releases/latest).

| Platform | Download |
|----------|----------|
| Windows  | `any-ai-cli-windows-amd64.exe` |
| macOS    | `any-ai-cli-darwin-amd64` / `any-ai-cli-darwin-arm64` |
| Linux    | `any-ai-cli-linux-amd64` |

> Settings and logs are stored in `~/.any-ai-cli/` (created on first run).
> Session logs contain user input and AI output. Treat them as sensitive data.

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
  --certificate-identity-regexp "https://github.com/ishizakahiroshi/any-ai-cli/.github/workflows/release.yml@refs/tags/v.*" \
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
- **Local-first UI** — Hub HTTP/WebSocket server binds to `127.0.0.1` only; no telemetry from `any-ai-cli` itself

---

## Quick Start (Recommended)

The normal flow: launch the binary, then drive everything from the browser. You do not need to run any CLI command yourself.

1. Download `any-ai-cli.exe` (or the macOS / Linux binary) from the table above
2. **Double-click `any-ai-cli.exe`** (or run `any-ai-cli` with no arguments)
   - The Hub starts and your browser opens automatically at `http://127.0.0.1:47777/?token=<token>`
   - If a Hub is already running, your browser is reopened against the existing instance
3. In the Hub UI, click **"+ New Session"** to launch a Claude Code / Codex CLI session
4. When an approval prompt appears, an action bar shows up under the input — click a button or use the keyboard to respond

Sessions can be created, monitored, and approved entirely from the Hub UI; you do not need to keep a separate terminal open.

> **⚠ About the console window**
> Double-clicking the binary opens a console window alongside the browser. **That console *is* the Hub server process** — closing it with `×` terminates the Hub. If it gets in the way, **minimize** it instead of closing it.
> If the Hub does go down (whether by `×`, a crash, or a manual restart), running AI sessions wait up to **60 minutes** for the Hub to come back before terminating themselves (configurable in `config.yaml` up to 24 hours — extend it for long-running autonomous tasks). A Web UI bug or restart will not silently kill your work. See [Shutdown, zombie protection & Hub crash resilience](#shutdown-zombie-protection--hub-crash-resilience) for details.
> To stop the Hub intentionally, use the `⏻` button in the top-right of the Hub UI, or run `any-ai-cli stop` from another terminal.

---

## Launching from a terminal (advanced)

If you prefer driving things from a shell — for scripting, shell integration, or muscle memory — these options are equivalent to clicking "+ New Session" in the UI. Use whichever you like.

### Option A: provider as a subcommand

```bash
any-ai-cli claude      # auto-starts Hub in the background if not running, then launches Claude
any-ai-cli codex       # same
```

You do not need to run `any-ai-cli serve` first.

### Option B: `wrap` subcommand (for debugging)

```bash
any-ai-cli wrap claude
any-ai-cli wrap codex
```

Functionally identical to Option A; useful when you want to be explicit about the wrapper layer.

### Option C: transparent mode (`ANY_AI_CLI_AUTO`)

Initialize the shell once, then your normal `claude` / `codex` commands transparently go through the wrapper.

> `any-ai-cli shell-init` emits **POSIX shell (bash / zsh) only** function definitions. There is no PowerShell snippet — see below for a manual alternative.

```bash
# Run once per shell startup (bash / zsh)
eval "$(any-ai-cli shell-init)"

# Turn on per-session — only the shells where you opt in are wrapped
export ANY_AI_CLI_AUTO=1
claude    # → goes through the wrapper, auto-starts Hub if needed
codex     # → same
```

Without `ANY_AI_CLI_AUTO=1`, `claude` / `codex` behave exactly as the original commands. No global `.bashrc` modification.

#### OS-specific automation examples

**PowerShell (Windows)**

Add the following to your `$PROFILE` (since `shell-init` does not support PowerShell, the functions are defined directly):

```powershell
if ($env:ANY_AI_CLI_AUTO -eq '1') {
    function claude { any-ai-cli claude @args }
    function codex  { any-ai-cli codex  @args }
}
```

Set `ANY_AI_CLI_AUTO=1` on a specific Windows Terminal profile to enable transparent mode only in that tab:

```jsonc
{
  "name": "AI Watch",
  "commandline": "pwsh.exe -NoExit",
  "environment": { "ANY_AI_CLI_AUTO": "1" }
}
```

**iTerm2 (macOS)**

- Profiles → Environment → Variables: `ANY_AI_CLI_AUTO=1`
- Profiles → General → Send text at start: `eval "$(any-ai-cli shell-init)"`

**tmux (all OSes)**

```bash
# ~/.tmux.conf
set-option -g default-command "ANY_AI_CLI_AUTO=1 bash -c 'eval \"$(any-ai-cli shell-init)\"; exec bash'"
```

---

## Settings

Settings are stored in `~/.any-ai-cli/config.yaml` (auto-created on first run).

| Key | Description | Default |
|-----|-------------|---------|
| `port` | Hub port | `47777` |
| `language` | UI language (`en` or `ja`) | `en` |
| `auto_open_browser` | Auto-open browser on Hub start | `true` |

### Where settings are saved

Settings are split into three categories:

| Category | Examples | Storage |
|---|---|---|
| **D1: UI display state** (per-device is natural) | theme, font size, language, sidebar width | Browser **localStorage** |
| **D2: User feature settings** (shared across devices / ports) | voice, wake word, trigger, notification sound, approval auto-switch, quick commands, usage links, favorites, session order, spawn defaults | `~/.any-ai-cli/config.yaml` under `user_prefs:`, read/written via `GET/PUT /api/user-prefs` |
| **D3: Server operation settings** | hub port, log config, approval enable/disable, slash command sources, approval pattern sources, token | `~/.any-ai-cli/config.yaml` (direct edit or dedicated Settings UI) |

D2 settings survive port changes (e.g. the WSL launcher shifting from 47777 to 47877) because they are stored server-side rather than in per-origin localStorage.

On first load the browser mirrors D2 values from the server. Subsequent changes are written to both localStorage (as a cache) and the server simultaneously. Any existing localStorage values are pushed to the server automatically on first run.

Custom notification sounds are stored as a binary file at `~/.any-ai-cli/notify_sound_custom.bin`, with the MIME type recorded in `user_prefs.notify_sound.custom_mime`.

---

## Hub UI

Open `http://127.0.0.1:47777/?token=<token>` in your browser.

```
┌─ ANY-AI-CLI  [1][0][6] │ ● Claude:2  ● Codex:5         [⏻] [Settings] ─┐
├──────────────────────────┬──────────────────────────────────────────────┤
│ [+ New Session]          │ ● Codex  cwd: C:\dev\any-ai-cli  [↑ to top] │
│ 📁 any-ai-cli  [1][0][6] │ Terminal output — Windows PowerShell         │
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

Configuration knobs in `~/.any-ai-cli/config.yaml`:

- `hub.wrapper_reconnect_grace_sec` — `0` disables reconnect (legacy "kill immediately" behavior). Range `0`–`86400` seconds (up to 24 h). Default `3600` (60 min). Also editable in Settings (in minutes). **Applies to new sessions only** — running sessions keep the value they were spawned with.
- `hub.idle_timeout_min` — how long the Hub keeps wrappers alive when no UI is connected. `0` disables. Range `0`–`1440` minutes. Also editable in Settings.

For a clean shutdown, prefer the `⏻` button in the Hub UI top-right or `any-ai-cli stop`; closing the console window now leaves wrappers waiting for the Hub to come back rather than killing them right away.

---

## Architecture

```
AI CLI (claude / codex)
    └─ any-ai-cli wrap  <── PTY wrapper
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
| Hub log | `~/.any-ai-cli/logs/hub.log` | Hub server runtime logs (rotated by lumberjack; configured via the `log:` section) |
| Session raw log | `~/.any-ai-cli/logs/sessions/<provider>_<YYYY-MM-DD_HHMMSS>_<folder>_s<id>.log` | Raw PTY stream for each wrapped session (includes ANSI sequences) |
| Session history | `~/.any-ai-cli/logs/sessions/<provider>_<YYYY-MM-DD_HHMMSS>_<folder>_s<id>.jsonl` | Structured session events (`session_start`, `user_input`, `pty_output`, `attach`, `session_end`, `session_dismiss`) |
| Clean transcript | `~/.any-ai-cli/logs/sessions/<provider>_<YYYY-MM-DD_HHMMSS>_<folder>_s<id>.txt` | Human-readable text (ANSI / spinners / control bytes stripped). Generated automatically on session end; any sessions missed due to a Hub crash are reconstructed at the next `serve` startup |

The Hub UI log-path button copies the log directory path to your clipboard.

---

## Troubleshooting

### Session card shows `Disconnected` immediately after spawn (Windows + pnpm-installed CLI)

If you installed Claude Code or Codex CLI via `pnpm add -g`, the Hub may fail to spawn it with the session card flipping to `Disconnected` within a second and a 0-byte raw log. The card now also shows a short reason such as `reason: codex not found in PATH`.

The Hub inherits the `PATH` snapshot of the shell that launched it. If that shell did not have `PNPM_HOME` exported, the persistent `%PNPM_HOME%\bin` entry in your USER `Path` is not expanded by Windows at process start and the pnpm bin directory effectively drops out — so `exec.LookPath("codex")` inside the wrap subprocess fails.

**To recover:**

1. `any-ai-cli stop`
2. Open an interactive PowerShell where `$env:PNPM_HOME` resolves correctly (verify with `$env:PATH -split ';' | Select-String pnpm`).
3. From that shell, run `any-ai-cli claude` (or `codex`) — the Hub will be re-spawned with the fresh `PATH` snapshot.

Hub diagnostics for each spawn are written to `~/.any-ai-cli/logs/spawn/<provider>-<timestamp>.log` and include the resolved `PATH`, detected package managers, and an explicit hint when `executable file not found` is the underlying cause.

> **v0.1.4 and later:** The Hub re-expands `%VAR%`-style entries in the inherited USER `Path` just before spawning a wrap process (reading `HKCU\Environment` and falling back to `%LOCALAPPDATA%\pnpm` when the directory exists), so this manual restart is normally no longer needed. The recovery procedure above remains as a fallback when the fix cannot resolve the variable.

---

## Security / Privacy

- The Hub HTTP/WebSocket server binds to `127.0.0.1` only — external hosts cannot reach it directly
- Random token in URL prevents unauthorized local access
- `any-ai-cli` itself sends no telemetry or usage data to any service

### Outbound network traffic

`any-ai-cli` is local-first, but the following outbound HTTPS requests can occur and you should be aware of them:

- **Slash command list (Hub itself)** — When the slash command picker is opened, the Hub fetches a markdown file from `https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/{claude,codex}.md` and caches it for 24 hours. The source URL can be changed (or pointed to a local file path) in **Settings → Slash command sources**.
- **Wrapped CLI traffic (the CLIs themselves)** — The CLIs you wrap (Claude Code, Codex CLI) talk directly to their respective vendor APIs (Anthropic, OpenAI) over HTTPS. `any-ai-cli` only relays PTY I/O via local WebSocket; it does not intercept, log, or proxy these API requests. Whatever network behavior the underlying CLI has applies as-is.

### ⚠️ Data retention by wrapped CLIs

`any-ai-cli` does not collect or transmit your data, but **the CLIs it wraps do** — and each vendor's data-handling rules differ. Because the Hub only relays PTY I/O, the wrapped CLI's policy applies to you **as-is**.

The table below summarizes each vendor's stance as of 2026. Always verify the current terms before use.

| CLI / Backend | Used for model training by default? | Opt-out / controls | Retention |
|---|---|---|---|
| **Claude Code** (Anthropic Commercial Terms: API / Claude for Work / Enterprise / Education / Gov) | **No** — excluded by default under commercial terms | No opt-out needed; Zero Data Retention available via enterprise agreement | API logs up to 30 days, reduced to **7-day auto-delete** after 2025-09-14 |
| **Codex CLI** (OpenAI: via ChatGPT Plus / Pro / Business plans) | **Possibly** — content from ChatGPT plans can be used for training | "Do not train on my content" toggle in the privacy portal; separate "allow training on full environments" control in Codex Settings | Abuse-monitoring logs up to 30 days; ZDR / Modified Abuse Monitoring available |
| **GitHub Copilot CLI** (GitHub: Product Specific Terms, March 2026) | **Yes** — prompts are retained and used to fine-tune your private model | No explicit opt-out documented (verify current terms) | Not specified |

### ⚠️ Terms-of-service change risk

Wrapped-CLI vendors may change their terms — including restricting or prohibiting third-party wrapper / automation access — at any time. If that happens, using the CLI through `any-ai-cli` could become a terms violation.

- Recent precedent: Google began enforcing a ToS clause in 2026 that forbids accessing Gemini Code Assist through third-party wrappers, resulting in `403 ToS` account bans for tools like OpenClaw / OpenCode / Antigravity. For this reason, **Gemini CLI is intentionally out of scope** for `any-ai-cli`.
- The same risk applies to every CLI in the table above. **Support for any wrapped CLI may be discontinued without notice** if its vendor restricts third-party automation. It is your responsibility to review each CLI's current terms before use.

### ⚠️ Important: Localhost-only by design

`any-ai-cli` is designed to run on the **same machine** as your browser. Do **not**:

- Run `serve` on a remote server (VPS / cloud) and connect to it from another host
- Modify the bind address to anything other than `127.0.0.1` (e.g. `0.0.0.0`, LAN IP)
- Expose the Hub UI through a reverse proxy (nginx / Caddy / etc.)
- Share the Hub URL (with its token) with anyone

The Hub UI exposes APIs that perform host-level actions (e.g. `/api/open-dir` opens folders in the OS file manager). These are only safe under the localhost assumption — exposing them externally could lead to **arbitrary folder access or information disclosure**.

---

## Build from Source

Requires Go 1.22+.

```bash
git clone https://github.com/ishizakahiroshi/any-ai-cli.git
cd any-ai-cli
go build -o any-ai-cli ./cmd/any-ai-cli
```

#### Cross-compilation

```bash
GOOS=windows GOARCH=amd64 go build -o dist/any-ai-cli-windows-amd64.exe ./cmd/any-ai-cli
GOOS=darwin  GOARCH=amd64 go build -o dist/any-ai-cli-darwin-amd64      ./cmd/any-ai-cli
GOOS=darwin  GOARCH=arm64 go build -o dist/any-ai-cli-darwin-arm64      ./cmd/any-ai-cli
GOOS=linux   GOARCH=amd64 go build -o dist/any-ai-cli-linux-amd64       ./cmd/any-ai-cli
```

---

## License

MIT — see [LICENSE](LICENSE) for details.

Third-party dependency notices are provided in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), and the vendored/browser-side license texts are provided in [web/src/vendor/THIRD_PARTY_LICENSES.txt](web/src/vendor/THIRD_PARTY_LICENSES.txt).

---

## Disclaimer

This tool is provided as-is without warranty. Use at your own risk.
