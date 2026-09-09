---
type: deployment-process
title: Build, Packaging, and Release Pipeline
description: How many-ai-cli is built locally with make, how git tags are the single source of version truth, and how the tag-driven GitHub Actions workflow publishes to GitHub Releases, winget, Homebrew, and npm.
tags: [release, build, makefile, npm, goreleaser, versioning, packaging, ci]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-4d1d392666be6dfdd7a91a2e
    resource: repo://.github/workflows/release.yml
  - id: openwiki-source-012f2c78e3b1446dfc35803f
    resource: repo://Makefile
  - id: openwiki-source-455a6f58075ee9f94f6f1b06
    resource: repo://npm/many-ai-cli/bin/many-ai-cli.mjs
  - id: openwiki-source-787105b636e9d06b34fe60ce
    resource: repo://scripts/check-version-sources.mjs
  - id: openwiki-source-8c1f3d4518966c8688bc06e1
    resource: repo://scripts/sync-npm-version.mjs
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## Local build: `make`

`Makefile` is the developer build entry point (`make build` is the standard "build everything" instruction — `bun run build` alone is not, since it only covers the web bundle). Targets:

- `build-web` — `cd web && bun install && bun run build -- --debug`, producing `web/dist` (embedded into the Go binary via `go:embed`).
- `build-windows` — depends on `build-web`; runs `go-winres make` to bake Windows executable resources, then `go build` with `GO_TAGS` (defaults to `maidebug`, an opt-in build tag for local observability/instrumentation code — see [Audit Tests and Static Quality Gates](/openwiki/testing/audit-and-quality-gates.md)) and `GO_LDFLAGS` injecting `main.version`/`main.gitCommit`/`main.buildTime` from `git describe --tags --always --dirty`, the short commit SHA, and the HEAD commit's ISO-8601 date.
- `build-launcher` — builds `many-ai-cli-launcher` similarly, with its own `winres-launcher.json` resource set.
- `build-linux` — cross-compiles the Linux binary with `CGO_ENABLED=0 GOOS=linux GOARCH=amd64`.
- `deploy-wsl` — runs `scripts/deploy-wsl.ps1` to push a locally built Linux binary into WSL for local testing.
- `run` — `build-windows` then `many-ai-cli.exe serve`.
- `fmt-check` / `fmt` — delegate to `scripts/check-gofmt.mjs` rather than a plain `gofmt` invocation, because that script needs to scope which files it checks and normalize CRLF before comparing, which a one-line Makefile rule cannot express.
- `debug-purge` / `debug-restore` — wrap `scripts/check-instrumentation.mjs --purge` and `scripts/debug-restore.mjs`, the tooling behind the `maidebug` observability-code ledger.

A local `make build` embeds `git describe --tags --always --dirty` (e.g. `v0.3.3-21-g3664cac`) rather than a bare version, specifically so a build that is behind the latest tag or built from uncommitted changes is visually distinguishable in the running Hub's version display — this was added after a bug where a `develop` branch that had not merged in `main`'s latest tag silently displayed a stale old version.

## Single source of version truth: the git tag

There is deliberately no place in the repository where a human edits a version number to cut a release. Three files nominally carry a version string — `npm/many-ai-cli/package.json`, `npm/many-ai-cli-<platform>/package.json` (×4), and `winres/winres.json` — and all three are expected to sit stale between releases, overwritten by CI from the pushed git tag each time. `scripts/sync-npm-version.mjs` performs the npm-side sync (`npm/many-ai-cli/package.json`'s `version` and its `optionalDependencies` versions, plus each platform package's own `version`), pinning the root package's optional dependencies to the *exact* same version so a published root package can never resolve a mismatched platform binary. `scripts/check-version-sources.mjs` is a CI gate that checks the *wiring* — that `.github/workflows/release.yml` still invokes `sync-npm-version.mjs` and the equivalent goreleaser/winres hooks — rather than checking that the in-repo files currently match the latest tag, since staleness between releases is the expected, correct state and re-synchronizing it by hand would reintroduce dual-sourcing.

## npm distribution model

The published `many-ai-cli` npm package is a thin shim, not the binary itself: `bin/many-ai-cli.mjs` resolves an OS/arch-specific `optionalDependencies` entry (`many-ai-cli-windows-x64`, `-linux-x64`, `-macos-intel`, `-macos-apple-silicon`) and `execFileSync`s that package's bundled native binary, forwarding argv/stdio/exit code. Distributing this way — rather than a browser-downloaded `.zip`/`.exe` — means the actual executable is materialized locally by the package manager at install time and carries no Mark-of-the-Web, avoiding the Windows SmartScreen prompt that a downloaded binary would trigger.

## Tag-driven release workflow

`.github/workflows/release.yml` triggers on pushing a `v*.*.*` tag (or via `workflow_dispatch` for an npm-only re-publish against an already-released tag). Its shape:

1. **`prepare-whisper-runtime`** (`windows-latest`) — fetches and verifies Microsoft-signed VC++ runtime DLLs (`vcomp140.dll`, `msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`) needed by managed Whisper, and uploads them as a build artifact; these are gitignored and obtained fresh per release rather than committed.
2. **`release`** (`ubuntu-latest`, depends on the above) — validates the tag format and, for a normal tag push, that the checked-out commit matches the tag; sets up Go/Node/Bun; installs `cosign` and `syft` (for SBOM generation); syncs the `winget-pkgs` fork with upstream `microsoft/winget-pkgs` (non-fatal on failure — a stale fork base only degrades the winget PR, it does not block the GitHub Release/Homebrew/npm publish); downloads and cosign-verifies a pinned `goreleaser` binary rather than trusting a GitHub Action's bundled copy; runs `goreleaser release --clean` (which drives the actual GitHub Release, winget-pkgs PR, and Homebrew tap publish per `.goreleaser.yaml`); then, if `NPM_TOKEN` is configured, stages npm binaries either from the just-built `dist/` (normal path) or by extracting and SHA256-verifying binaries out of the already-published, cosign-signed Release zips (the `npm_only` re-run path, so a failed npm publish can be retried without re-running goreleaser and risking a duplicate Release) via `scripts/stage-npm-from-release.mjs` / `scripts/stage-npm-binaries.mjs`, then publishes.

Release artifacts include `SHA256SUMS.txt` plus a `cosign` signature/certificate pair (`SHA256SUMS.txt.sig` / `.pem`), so a downloaded archive's integrity can be verified independent of (and in addition to) npm/winget/Homebrew's own transport trust.

## Packaging channels

Beyond npm, the same tagged build reaches users through: GitHub Releases (raw `.zip` per platform, plus `.deb`/`.rpm` for Linux), Windows `winget` (`ishizakahiroshi.many-ai-cli`, published via a `winget-pkgs` fork PR since Microsoft does not offer a direct API), and macOS Homebrew (`ishizakahiroshi/tap/many-ai-cli` cask). See [Docker and Remote Server Deployment](/openwiki/deployment/docker-remote-server.md) for the separate GHCR container image build, which is not part of this same tag-triggered `release.yml` run.
