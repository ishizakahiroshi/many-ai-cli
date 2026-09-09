---
type: architecture-component
title: Web Frontend Architecture
description: The unbundled, per-file esbuild pipeline behind the TypeScript Web UI, the classic-script global-scope module style it targets, and how it reaches the browser as a go:embed asset.
tags: [frontend, typescript, esbuild, i18n, state, terminal, xterm]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-7a573106b23a08d6d5fd36ce
    resource: repo://web/scripts/build.mjs
  - id: openwiki-source-15a6f7a97f9386472ca60847
    resource: repo://web/src/app-entry.ts
  - id: openwiki-source-c8412e49121db5a6b59211aa
    resource: repo://web/src/app.ts
  - id: openwiki-source-244f7dad4f356d1989362bda
    resource: repo://web/src/app/state.ts
  - id: openwiki-source-e243b08ba93638a6ef00055e
    resource: repo://web/src/i18n.ts
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## Not a bundle: one compiled file per source file

`web/scripts/build.mjs` drives the frontend build with esbuild's `bundle: false`: every `.ts`/`.js` file under `web/src` (excluding `vendor/` and, unless opted in, `web/src/debug/`) is transpiled individually into a matching file under `web/dist`, each still holding real `import`/`export` statements rather than being merged into one bundle. `web/src/app-entry.ts` is the generated ESM entry point that imports every top-level module in a fixed load order for side effects — `i18n.js`, `util.js`, `user-prefs.js`, `state.js`, `multi-pane.js`, `app.js`, then the rest, roughly forty modules deep, followed by explicit `init*()` calls for features that need to run after the DOM is ready (token status bar, detached-grid mode detection, the Server modal, workflow progress polling, live-status color, the mobile connect QR wizard, remote-access protection, and more).

Because the build does not bundle, load order is meaningful and encoded directly in `app-entry.ts`'s import list rather than left to a bundler's dependency graph — a module that expects another to have already run its side effects at import time (as opposed to only calling its exported functions) depends on being listed after it.

## `app.ts`: the orchestration layer

`app.ts` (roughly 2,800 lines) is the glue file: it imports named exports from around 25 other modules — `state.ts` for the shared session/terminal maps, `terminal.ts` for xterm sizing and scroll helpers, `approval.ts` for action-bar interaction, `chat-history.ts`, `attachments.ts`, `files-view.ts`, `host-expose.ts`, and so on — and wires them into the top-level UI behaviors (session switching, quick commands, settings load, theme/font/lang application, approval auto-switch queueing). It is the file most other feature modules ultimately get pulled into via `app-entry.ts`'s import chain, which is also why `web/src/debug/probe.ts` (the observability sink) is always emitted even in a non-debug build — product code imports it directly, and with `bundle: false` a missing import target would be a hard build error, not just dead code; the debug build flag instead makes the probe's body a no-op via an esbuild `define`.

## `state.ts`: shared types and session maps, not a store

`state.ts` is explicitly commented as "extracted from `app.js`... classic-script global scope; no module wrapper" — it holds the cross-cutting TypeScript interfaces (`TerminalEntry` — one entry per session's xterm instance, pending-chunk buffers, live-status line-assembly state, and per-terminal filter carry-over buffers for the marker/reverse-video/screen-clear filters; `ApprovalOptionLike`; `SequentialChoicePrompt`) and the shared `Map`-based session/terminal registries keyed by session ID, rather than being a single reactive store — modules read and mutate these maps directly.

## Terminal rendering (`terminal.ts`)

`terminal.ts` owns xterm.js instance lifecycle and layout: fitting the terminal to its container, keeping the view pinned to the bottom across resizes, and the plumbing that decides whether a PTY resize should be sent to the wrapper or suppressed (e.g., while the on-screen keyboard or input layout is transiently changing size). Incoming PTY bytes pass through this layer's filter chain — including [`hub-marker-filter.ts`](/openwiki/frontend/approval-ui.md) among others — before being written to the xterm buffer.

## i18n: runtime-fetched dictionaries, not build-time

Localization is not baked in at build time. `web/src/i18n.ts` is an IIFE that runs once at load: it resolves the active language from `localStorage`'s `ai_cli_hub_lang`, falling back to the browser's `navigator.language` (matching `vi`/`en`/`ja`, defaulting to `ja` for anything else), sets `document.documentElement.lang`, and then `fetch`es `/i18n/<lang>.json` — one of three static JSON dictionaries (`en.json`, `ja.json`, `vi.json`) served by the Hub alongside the rest of the static assets — to populate a global `window.t()` translation function. `i18n.ts`'s own exported `t()`/`setLang()` are late-bound wrappers around `window.t`/`window.setLang`, with a plain identity fallback (`t(key) => key`) installed at the top of `app.ts` for the window before `i18n.js` has finished loading its dictionary, so text never breaks visibly during the brief async fetch.

## Reaching the browser: `go:embed` and freshness detection

`web/dist` — the esbuild output plus copied static assets (`index.html`, `styles.css`, `styles/`, `vendor/`, `icons/`, `i18n/`, the PWA manifest) — is embedded directly into the Go binary via `go:embed` (see [Build, Packaging, and Release Pipeline](/openwiki/deployment/build-and-release.md) for how `make build-windows`/`build-linux` invoke the web build first). The build script also writes `web/dist/.src-hash`, a SHA-256 over every source file's relative path and contents (mixed with whether the debug build flag was set, since that changes the emitted output for the same source), which `internal/hub.NewServer` compares against a freshly recomputed hash of `web/src` at Hub startup to detect and warn ("run `make build`") when the running binary's embedded frontend is stale relative to the source tree next to it — a check that only applies in a source checkout and is treated as fresh-by-default in environments (VPS/Docker) where `web/src` is not present at all.
