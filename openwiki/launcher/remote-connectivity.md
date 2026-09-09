---
type: architecture-component
title: Unified Launcher and Remote Connectivity
description: many-ai-cli-launcher's WSL and SSH (serve/tunnel) connection profiles, its default browser-based profile picker, the active-connection registry that avoids duplicate tunnels, and the terminal-launched alternatives that sidestep SmartScreen.
tags: [launcher, ssh, wsl, tunnel, remote, smartscreen]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-526655761f28371205f45634
    resource: repo://cmd/many-ai-cli-launcher/main.go
  - id: openwiki-source-1be77af1816e790274253fe0
    resource: repo://internal/launcher/active.go
  - id: openwiki-source-97f7f07394f5d6962ce6da95
    resource: repo://internal/launcher/connector_ssh.go
  - id: openwiki-source-23775c3de52f3ab95a13cb8b
    resource: repo://README.md
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## Two profile types, and SSH's two modes

`many-ai-cli-launcher` reads `~/.many-ai-cli/launcher-profiles.yaml` and connects to a Hub through one of two profile types: `wsl` (Windows-only — starts `many-ai-cli serve` inside WSL and opens the Windows browser once the Linux side prints its Hub URL) or `ssh` (any OS). An `ssh` profile further has two modes: **`serve`** (SSH into a remote host and start `many-ai-cli serve` there) and **`tunnel`** (port-forward to a Hub that is already running and kept resident on the remote — via systemd, tmux, or Docker compose). In both SSH modes the remote Hub still binds `127.0.0.1` only; the launcher's own local forward (`-L 127.0.0.1:<port>:127.0.0.1:<port>`) is what makes it reachable from the Windows browser, never a change to the remote Hub's own bind address (see [Security, Auth, and Remote Access Protection](/openwiki/hub/security-remote-access.md) for why the Hub is built around that bind assumption everywhere). A `wsl` profile calls `wsl.exe` internally with `bash -ilc` (login + interactive), specifically so `~/.bashrc` entries — `nvm`, `pnpm`, `cargo`, and similar PATH-modifying setup — are fully loaded before the Linux-side `many-ai-cli` binary is invoked.

## Default behavior: a browser picker, not a silent auto-connect

`cmd/many-ai-cli-launcher/main.go`'s own doc comment states the default precisely: run with no flags at all — the ordinary case for a plain double-click or a direct shortcut invocation — and the launcher always opens a browser-based profile-selection page on a random loopback port, where already-connected profiles are marked as such; connecting directly without that picker UI requires an explicit `--profile <name>` or `--last` flag (typically baked into a dedicated shortcut a user built for one specific profile). `--ui` forces the picker even when a single profile would otherwise auto-connect.

## Avoiding duplicate tunnels: the active-connection registry

Each launcher process records its established connections in `~/.many-ai-cli/launcher-active.json` (`internal/launcher/active.go`) so a second launcher instance — most commonly the profile-selection UI itself — can see which profiles are already connected and reuse the existing Hub URL rather than starting a second tunnel or a second `serve` for the same profile. Staleness detection uses the same double-guard pattern the Hub's own PID file staleness check uses (`internal/hub`'s `killStalePid`, referenced directly in this package's comment): a recorded entry is treated as alive only when *both* the recorded launcher PID is still running *and* the recorded Hub URL still answers `/api/info` — an entry failing either check is pruned on read, since a PID number can be reused by an unrelated process once the original launcher has exited.

## Quiet mode: never echoing a remote token to a console

`SSHConnector`'s `out io.Writer` field controls where the scanned child-process (SSH) output is mirrored: left `nil`, it mirrors to the normal launcher-exe console the user is watching, but the Hub's own embedded connection flow (the in-Hub 🖥 Server modal — see below) sets it to `io.Discard` via a `setQuiet` path specifically so that hosting a tunnel through the Hub UI produces no console noise and — the comment is explicit about the actual motivation — never echoes the remote Hub's URL and token to any terminal at all.

## Sidestepping Windows SmartScreen without abandoning the launcher's connection logic

If Windows SmartScreen or an organization policy blocks double-clicking `many-ai-cli-launcher.exe`, the README documents that this reflects how the executable was *launched*, not a defect in the connection logic itself: launching a process via `CreateProcess` from a terminal does not go through Explorer's reputation-check UI, so SmartScreen's "Windows protected your PC" interstitial generally does not appear for a terminal-launched process even when it would for the same binary double-clicked from Explorer. Two terminal-launched routes reuse the exact same connection code without ever requiring a double-click on the launcher `.exe`: the Hub UI's **🖥 Server** button/modal (which runs `many-ai-cli serve`, opens the dashboard, and manages/connects profiles from inside the already-running Hub process — with the SSH/WSL child process held by the Hub itself, so no extra console window is left open, and quiet mode as described above), and `many-ai-cli connect --profile <name>` (or `--last`), which is the same connection flow exposed as a subcommand on the main `many-ai-cli` binary rather than the separate launcher executable (see [CLI Entrypoints and Subcommands](/openwiki/architecture/cli-entrypoints.md)). If a SmartScreen *dialog* (as opposed to an actual Defender quarantine of the binary) still appears, it stems from the Mark-of-the-Web an internet-zone download carries, which `unblock-windows.cmd` or a manual `Unblock-File` clears — a package-manager install avoids acquiring that mark in the first place, since the binary is materialized locally rather than downloaded as a zip/exe.

## Security invariants the launcher does not change

Per the README's dedicated launcher-security summary: no `0.0.0.0` bind, ever — only `127.0.0.1`-to-`127.0.0.1` local forwarding, never `-g`/`GatewayPorts`; SSH authentication is key-based only, with `-o BatchMode=yes` so a password prompt fails immediately rather than hanging or falling back to interactive entry; passwords and key passphrases are never saved by the launcher; and a token obtained through a tunnel-mode profile's `token_command` is used only for the current session's connection and is never written into `launcher-profiles.yaml` itself.
