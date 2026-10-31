# many-ai-cli

Web dashboard to manage approvals and progress across multiple AI coding CLIs
(Claude Code / Codex / Copilot / Cursor / OpenCode / Grok / Command Code) running in parallel — a single local Hub + browser UI.

## Install

Choose a published version from the
[release notes](https://github.com/ishizakahiroshi/many-ai-cli/releases).
Replace `<version>` below with that version, without the leading `v`.

```sh
pnpm add -g many-ai-cli@<version>
many-ai-cli --version
many-ai-cli setup
```

You can also install the same version from the npm registry with
`npm install -g many-ai-cli@<version>` or `bun install -g many-ai-cli@<version>`.
Keep optional dependencies enabled: the matching platform package contains the
native executable. Use a maintained Node.js release for the npm command shim.
If the command is not found after installation, open a terminal with your package
manager's global bin directory on `PATH` before running `setup`.

The npm wrapper launches that executable; it is independent of whether the
installed release was built from Go or Rust. From v1.0.0 the executable is the
Rust build, and the package names and paths are unchanged from v0.9.0 (Go).
Check the selected release's notes and version before switching from a
personal executable.

Installing through a package manager avoids manually downloading and locating
an executable. It does **not** guarantee the absence of Windows SmartScreen or
Mark-of-the-Web warnings. Smart App Control and organization policies such as
WDAC, AppLocker, or EDR are separate; package-manager installation is not code
signing or a way to disable those policies.

## Windows desktop use

For the Rust v1.0.0 startup flow, run `many-ai-cli setup` once after installation,
then double-click the desktop shortcut to start the Hub and open its browser UI.
If the Hub is already running, the shortcut opens that Hub. The shortcut points
directly to the native executable, so daily icon use does not need a terminal
window or a fresh npm lookup.

If you use a custom home or port, run `setup` with that same home environment.
Use `many-ai-cli setup --port <port>` to retain a port explicitly, especially
when that Hub is stopped. The shortcuts retain the resolved home and port.

Login startup is separate: when enabled, it starts the tray without opening a
browser. On Windows, `many-ai-cli tray --open` is the manual command to open the
Hub through the tray; plain `many-ai-cli tray` starts an idle tray. The tray menu
offers separate actions to open the Hub, stop it, or exit the tray while keeping
the Hub running. Check the menu label before stopping active work.

## Usage

```sh
many-ai-cli serve --open # start the foreground Hub and request the browser UI
many-ai-cli serve        # start the foreground Hub; browser opening is separate
many-ai-cli claude       # run Claude Code wrapped by the Hub
many-ai-cli codex        # run Codex wrapped by the Hub
many-ai-cli --version
```

The Hub binds to `127.0.0.1` only and is never exposed externally. A foreground
`serve` terminal must stay open while that Hub is in use.

## Update or return to a previous version

Choose the target version and repeat the install and setup steps:

```sh
pnpm add -g many-ai-cli@<version>
many-ai-cli --version
many-ai-cli setup
```

Run `setup` again because package managers may move the native executable to a
version-specific directory. Recreating the shortcuts connects them to the
currently selected executable. An already running Hub continues to use its old
executable until you deliberately stop and reopen it; check its version too.

Finish or save running AI work before switching the running Hub. These steps do
not tell you to kill or automatically restart it. If installation fails because
the executable is in use or a file is locked, leave the existing Hub running,
finish or save the work, then stop it deliberately and retry. Return to an earlier
published version with the same sequence and follow that release's data
compatibility guidance. Do not delete the existing configuration or data to make
an update succeed.

## Supported platforms

| OS | Arch | Package |
|----|------|---------|
| Windows | x64 | `many-ai-cli-windows-x64` |
| Linux | x64 | `many-ai-cli-linux-x64` |
| macOS | Intel (x64) | `many-ai-cli-macos-intel` |
| macOS | Apple Silicon (arm64) | `many-ai-cli-macos-apple-silicon` |

Other platforms: download from
[GitHub Releases](https://github.com/ishizakahiroshi/many-ai-cli/releases).

## License

MIT © Hiroshi Ishizaka (ishizakahiroshi)
