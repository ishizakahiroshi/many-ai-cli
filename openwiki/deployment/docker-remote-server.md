---
type: deployment-process
title: Docker and Remote Server Deployment
description: The per-user GHCR container model for running many-ai-cli on a remote server — Dockerfile stages, the loopback-only socat relay, entrypoint lifecycle, and opt-in cron-based image updates.
tags: [docker, deployment, remote-server, ghcr, compose, socat, multiuser]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-2f5eaf4fb32ea423253b9b78
    resource: repo://deploy/docker/aac-update.sh
  - id: openwiki-source-6d8dd4d7fce664c5358bed1b
    resource: repo://deploy/docker/compose.yaml
  - id: openwiki-source-1a0d8a98db0ff747dfbee636
    resource: repo://deploy/docker/Dockerfile
  - id: openwiki-source-7a4578cffddbeaf1b095b3fc
    resource: repo://deploy/docker/entrypoint.sh
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## One container per user

The Docker deployment model is explicitly **one container per user**, not one shared multi-tenant Hub. `deploy/docker/compose.yaml` is a thin root that `include`s one file per user under `users/<user>.yaml` (copied from `users/example.yaml`), each defining that user's assigned port and bind-mounted work volume; the port/volume assignment table lives in an `assign.md` alongside the compose file on the server, not in the repository. This mirrors the Hub's general assumption of a single owner per running instance — isolation between users is achieved by giving each their own container and Hub process rather than by any in-Hub multi-tenancy.

## Dockerfile build stages

`deploy/docker/Dockerfile` builds from the repository root as context (normally driven by `.github/workflows/docker-image.yml`, which is triggered separately from the tag-driven release pipeline — see [Build, Packaging, and Release Pipeline](/openwiki/deployment/build-and-release.md)):

- **`web-build`** — builds the TypeScript frontend into `web/dist` using `bun install --frozen-lockfile` for dependency resolution but `node`/`bun run build` for the actual build script execution, deliberately keeping `bun` out of the script-execution path to minimize attack surface.
- **`provider-cli-build`** — installs pinned exact versions of the Claude Code, Codex, and Copilot CLIs from a tracked `package-lock.json` via `npm ci --omit=dev`, so provider CLI versions are locked to specific releases rather than tracking "latest."
- **`build`** — compiles the `many-ai-cli` binary itself (`CGO_ENABLED=0`, trimmed/stripped) with `web/dist` copied in ahead of the Go build so `go:embed` bakes the frontend into the binary, exactly as a normal release build does.
- **`whisper-build`** — builds `whisper-server` from a pinned `whisper.cpp` commit (verified against a hardcoded commit SHA after checkout) with `GGML_OPENMP=OFF` (no `libgomp` dependency; parallelism via whisper.cpp's own thread pool), `BUILD_SHARED_LIBS=OFF`, and `GGML_NATIVE=OFF` (no build-host-specific CPU instruction tuning) — done from source because upstream whisper.cpp does not publish a Linux server binary release.
- **Final runtime stage** — Ubuntu 24.04 with the compiled binary, provider CLIs, and `whisper-server` copied in; `cursor-agent` is fetched separately in this stage by downloading the installer's pinned versioned package URL directly (rather than running the official "always latest" installer) and verifying its SHA256 against a hardcoded expected hash before extracting it, because the official installer otherwise always resolves to the newest version with no pin. The container runs as the non-root `ubuntu` user (uid 1000) with `sudo`/`adm` group membership explicitly stripped as defense in depth, even though the `sudo` binary itself is not installed.

The image sets `MANY_AI_CLI_WHISPER_SERVER` to the baked-in `whisper-server` path so the Hub treats managed Whisper as already installed and only needs to fetch the model, and `DISABLE_AUTOUPDATER=1` so provider CLIs are updated only by rebuilding the image (avoiding writes to `/usr/lib` under the non-root user).

## Network model: loopback Hub + socat relay

Inside the container, the Hub still binds to `127.0.0.1` only, exactly as it does outside Docker — the Host/Origin validation in `internal/hub` requires the client-visible port to match the Hub's own bound port exactly, which is why `HUB_PORT` must be passed through consistently to both the Hub's `--port` and the host's published port. External reachability is provided by `socat TCP-LISTEN:48000,fork,reuseaddr,bind=0.0.0.0 TCP:127.0.0.1:$HUB_PORT` running in the entrypoint's background loop: binding `0.0.0.0:48000` is safe here because it is only within the container's own network namespace — the host side only ever publishes the mapped port on `127.0.0.1`, per each `users/<user>.yaml`'s `ports:` definition, so the Hub is never actually reachable from outside the host. If `socat` itself dies, the loop restarts it after a 1-second sleep without touching the running Hub.

## Entrypoint lifecycle (`aac-entrypoint.sh`)

On first boot (no existing `config.yaml`), the entrypoint pre-seeds a minimal container-appropriate config before the Hub's own `LoadOrCreate` ever runs, because `LoadOrCreate` unmarshals the YAML file *over* code defaults — any key not written here falls back to the normal desktop default, which would be wrong for a headless remote container: `open_browser: false` (a headless container must never try to launch a browser), `auto_shutdown: false` (keep the Hub resident rather than exiting when idle), and `idle_timeout_min: 0` (disable the "kill all PTY sessions 60 minutes after the last UI disconnect" behavior, since remote operation — tunnel drops, laptop sleep — makes UI disconnects routine, and killing in-flight AI sessions on that basis would be real user-visible damage; this is overridable later from the Settings UI). It also appends a one-time, idempotent shell-init snippet to `~/.bashrc` so an interactive `docker exec` shell gets the transparent-wrap behavior (`MANY_AI_CLI_AUTO=1` + `many-ai-cli shell-init`).

Shutdown is layered and ordered: on `SIGTERM`/`SIGINT`, the entrypoint first sends `TERM` to any running wrapper processes and waits up to a fixed grace period for them to exit, then stops the `socat` relay loop (carefully killing the foreground `socat` process before `wait`ing on the loop's PID, since waiting first would deadlock — the loop cannot process its own `TERM` trap while blocked in the foreground `socat` call), and only then sends `TERM` to the Hub itself and waits for it. If the Hub exits with status 0 — the "stop Web UI only" path a user can trigger from the Hub itself — the entrypoint does **not** tear down the container; because the entrypoint is PID 1, exiting it would kill every wrapper process along with it, defeating the intent of "stop the Hub but keep sessions" (wrappers reconnect to the freshly restarted Hub within their reconnect grace period). Any non-zero Hub exit (a crash) does tear the wrappers and container down, deferring to Docker's `restart: unless-stopped` policy to bring it back up. The entrypoint also removes any stale Hub PID file at the start of every Hub (re)start loop iteration, because a container reboot can reuse the same PID number across boots and a stale leftover PID file could otherwise cause the Hub's own stale-PID-kill logic to target an unrelated process.

## Opt-in auto-update (`aac-update.sh`)

`aac-update.sh` is a separate, explicitly cron-scheduled script (not run automatically by anything in the container or compose file itself) intended for a daily root crontab entry: `docker compose pull` followed by `docker compose up -d --wait --wait-timeout 180`, which only recreates containers whose image actually changed and waits for them to report healthy (or just running, for services without a healthcheck) before proceeding; a failure here — via `set -eu` — stops the script before it reaches image pruning, and only dangling (untagged) images are pruned, never a named tag like `:dev`. Placing a `HOLD` file in the same directory as the script causes it to skip the update entirely, which is the documented way to freeze a server during active development or investigation.
