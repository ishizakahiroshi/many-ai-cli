---
type: architecture-component
title: Voice Input and Managed Whisper
description: Browser-recognition vs. local-Whisper voice dictation, the mobile AudioWorklet-to-Hub relay path, false-trigger suppression, and the self-contained managed Whisper binary/model lifecycle (Windows-only).
tags: [voice, whisper, audioworklet, whisperruntime, transcription]
sources:
  - id: openwiki-source-a8910515ddd14810ad43f5c1
    resource: repo://internal/config/config.go
  - id: openwiki-source-75b2ea4bf49ea74a0edb7e8c
    resource: repo://internal/hub/http_helpers.go
  - id: openwiki-source-f431f3f3162c4ecd701d3620
    resource: repo://internal/hub/voice_transcribe.go
  - id: openwiki-source-07428a59976f49cee2f354a1
    resource: repo://internal/hub/whisper_job_other.go
  - id: openwiki-source-0f675f7a277ee6f94d3ded75
    resource: repo://internal/hub/whisper_job_windows.go
  - id: openwiki-source-ef5a45d0fca360068fae811c
    resource: repo://internal/hub/whisper_manage.go
  - id: openwiki-source-7b5413c7efc86e548f890d29
    resource: repo://internal/whisperruntime/embed.go
  - id: openwiki-source-e2c49974cddb3e1b24dfb308
    resource: repo://internal/whisperruntime/fetch_windows_runtime.ps1
  - id: openwiki-source-99398c07e8cf6e83721a8731
    resource: repo://web/src/app/user-prefs.ts
  - id: openwiki-source-869c1a62c1047ed8af409725
    resource: repo://web/src/app/voice-engine.ts
  - id: openwiki-source-0109a6acc03207ba0cc9afb6
    resource: repo://web/src/app/voice-whisper.ts
  - id: openwiki-source-8cce8ea260d95e4192074470
    resource: repo://web/src/vendor/vtype-core/whisper.js
generated: { by: "claude-code", at: "2026-09-21T12:35:03.565Z" }
verified:
  - by: openwiki/0.5.0
    at: 2026-09-21T12:35:03.565Z
---

## Two engines, chosen per device

Voice input has three engine settings — `off` / `browser` / `whisper` — stored per-device in `localStorage` (`getVoiceEngine` in `user-prefs.ts`) and deliberately left out of the server-synced user preferences, so a PC can stay on `browser` (the built-in WebSpeech API) while an iPhone on the same account uses `whisper` at the same time. Both engines are created in one place, `voice-engine.ts`, through the vendored `vtype-core` library (`web/src/vendor/vtype-core/`, a copy kept in step by `scripts/sync-vtype-core.mjs`), which also arbitrates the microphone between SpeechRecognition and `getUserMedia`. `voice.ts` and `voice-whisper.ts` now only own the UI: the button, the voice bar and waveform, toasts, and shortcuts.

On the Whisper path the phone never runs inference: `vtype-core` records through `getUserMedia` + `AudioContext` (an `AudioWorklet` served as the same-origin static file `/whisper-recorder-worklet.js`, because a blob-URL worklet is blocked by the Hub's `script-src 'self'` CSP; `ScriptProcessor` only as a fallback), resamples to 16 kHz, encodes WAV client-side, and posts it to `/api/voice/transcribe`, which the Hub relays to a Whisper server. `MediaRecorder` is avoided on purpose: it produces AAC on iOS, which would require `ffmpeg` on the server.

## Suppressing false triggers

A recognized transcript is inserted into the input box for the user to review; it is sent automatically only when the Whisper auto-submit option has been explicitly turned on (it is off unless its `localStorage` key is `1`) **and** the text ends with the configured trigger phrase. Before any audio is uploaded, `vtype-core` discards a recording that is too short or never gets loud enough (peak RMS below `WHISPER_MIN_PEAK_RMS`, or less voiced time than `WHISPER_MIN_VOICED_MS`) as `no_speech`, and an empty transcript is discarded as `empty_result`. Recording also stops on its own after a configurable stretch of silence (2 s by default, clamped to 0–10 s).

The hallucination filter now runs on the Hub, not in the browser: `handleVoiceTranscribe` rejects a transcript that, after normalization (lower-casing, removing whitespace and trailing punctuation or brackets), is **exactly equal** to one of `voice.whisper.hallucination_phrases` — by default a fixed list of phrases Whisper tends to produce on silence, such as a Japanese "thanks for watching" sign-off or "please subscribe". A partial match is never discarded, so a real sentence that merely contains one of those phrases gets through; setting the list to `[]` turns the filter off. Every discard (`no_speech`, `empty_result`, `hallucination`) is shown as a toast explaining why, since a silent discard would look like the feature not working.

The Hub talks to the Whisper server only through `newLoopbackHTTPClient`, which enforces the loopback boundary on the initial request and on every redirect — so `server_url` has to point at a server on the same machine (managed, baked into the Docker image, or run by the user locally), and audio is never relayed off the host.

## Managed Whisper lifecycle

When the `whisper` engine is used with `voice.whisper.managed` set, the first transcription request calls `ensureManagedWhisper`, which starts `whisper-server` as a child process bound to `127.0.0.1` on the configured `server_port` if it is free, or otherwise on a fresh free port, with `-t` set to the host's CPU count (the upstream default of 4 threads left recognition noticeably slower on many-core hosts). Starting is serialized so two concurrent requests cannot launch two servers. The child is tied to the Hub's lifetime: on Windows it is placed in a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so even a Hub crash takes it down; on Linux/macOS it runs in its own process group (`Setpgid`) that is killed as a group; and in the Docker image `init: true` (tini as PID 1) is the backstop that reaps anything left.

## Self-contained binary and model distribution (Windows-first)

Downloadable managed install is defined only for `windows/amd64` in the `whisperBinaries` table (OS/arch → `{URL, SHA256, ServerNames, KeepFromArchive, Runtime}`), so adding a platform is a table entry; Linux/macOS builds are listed as a TODO pending self-built release assets. The Docker image instead bakes `whisper-server` in and points `MANY_AI_CLI_WHISPER_SERVER` at it, which makes it managed without any download. Everything a managed install writes lands under `~/.many-ai-cli/whisper/{bin,models,tmp}/`. Downloads are HTTPS-only, guarded by a stall watchdog, verified against the entry's SHA256 before being renamed into place, and a failed or mismatched download's temporary file is deleted so a retry starts clean; a model download is additionally preceded by a free-disk-space check. Uninstall stops the server and then does a plain `os.RemoveAll` of that directory tree, leaving nothing in `System32` or anywhere else outside the tool's own directory.

`internal/whisperruntime`'s own package doc states its purpose precisely: it embeds OS-local runtime payloads (the Windows VC++ runtime DLLs `whisper-server` links against — `vcomp140.dll`, `msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`) via `go:embed` and lays them down next to the managed binary specifically so the install needs no System32 write and no machine-wide runtime dependency — everything again lives under `~/.many-ai-cli/whisper/`, so an uninstall's `RemoveAll` leaves zero trace of these either. Because the real signed DLLs are gitignored (obtained only at release/CI time — see [Build, Packaging, and Release Pipeline](/openwiki/deployment/build-and-release.md) for the release workflow's `prepare-whisper-runtime` job), the `files/` embed directory ships placeholder README files so `go:embed` always finds at least one file and a from-source build succeeds even before the real DLLs are dropped in; until they are, `Ensure` (the function that copies the embedded payload out) simply copies nothing.

`fetch_windows_runtime.ps1` — the script that actually obtains those four DLLs for CI/release — deliberately never pulls from a third-party DLL distribution site: it searches, in priority order, a local Visual Studio Redist folder (located via `vswhere`) and then falls back to the Microsoft-signed servicing copies already present in `System32`, and validates every DLL it selects has a valid Authenticode signature naming "Microsoft Corporation" as the subject and an x64 (`0x8664`) PE machine type before accepting it.
