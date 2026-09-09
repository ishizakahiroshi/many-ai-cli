---
type: architecture-component
title: Voice Input and Managed Whisper
description: Browser-recognition vs. local-Whisper voice dictation, the mobile AudioWorklet-to-Hub relay path, false-trigger suppression, and the self-contained managed Whisper binary/model lifecycle (Windows-only).
tags: [voice, whisper, audioworklet, whisperruntime, transcription]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-b5500eb12b9efc8794e83269
    resource: repo://docs/v0.3.x-many-ai-cli-design.md
  - id: openwiki-source-7b5413c7efc86e548f890d29
    resource: repo://internal/whisperruntime/embed.go
  - id: openwiki-source-e2c49974cddb3e1b24dfb308
    resource: repo://internal/whisperruntime/fetch_windows_runtime.ps1
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## Two engines, chosen per device

Voice input has three engine settings — `off` / `browser` / `whisper` — stored per-device in `localStorage` and deliberately excluded from server-side user-preference sync, specifically so a PC can stay on `browser` (the built-in WebSpeech API) while an iPhone on the same account uses `whisper` at the same time. This split exists because iOS Safari's WebKit-forced WebSpeech implementation is not usable quality, so the mobile path separates microphone capture from inference entirely: an iPhone captures 16kHz PCM through an `AudioWorklet`, encodes it to WAV client-side, and posts it to `/api/voice/transcribe`, which the Hub relays to a Whisper server running on the PC or a remote server — the desktop browser's own built-in recognition (`voice.ts`) is left untouched and not replaced by this path. Recording deliberately avoids `MediaRecorder` in favor of WebAudio/`AudioWorklet` across every browser uniformly, specifically to sidestep iOS's AAC-only recording output and the server-side `ffmpeg` dependency that format would otherwise require; the new mobile-relay logic was added as a separate `voice-whisper.ts` module rather than modifying the existing `voice.ts`, to keep the working browser-recognition path untouched.

## Suppressing false triggers

The Whisper path's automatic-send-on-trailing-phrase trigger defaults to **off** and only activates on explicit opt-in with an exact-match requirement — a transcription result otherwise always lands as text in the input box for the user to review before sending, treating human eyes as the final confirmation layer. Two client-side filters run before a recording is even sent for transcription: a pre-check on the recording itself (an `AnalyserNode`-measured volume that stays near-silent for the whole recording is never sent) and a fixed hallucination-phrase filter — known filler phrases Whisper models are prone to hallucinate on silence (e.g. a Japanese "thanks for watching" sign-off) are discarded only when they match the **entire** transcription result exactly, deliberately not on a partial match, so a genuine sentence that happens to contain part of one of those phrases is never silently dropped. Every discard is surfaced as a toast explaining why, since a silent discard would read as the feature simply not working. The server side's recommended (not forced) setting is to enable `silero` VAD, a whisper.cpp option. A `server_url` pointed at an OpenAI-compatible external API is allowed but treated as an explicit opt-in with a UI disclosure that audio leaves the machine; the default only ever permits a local (`127.0.0.1`/`localhost`) server URL.

## Managed Whisper lifecycle

On the first request using the `whisper` engine (or at Hub startup, if already configured), the Hub spawns `whisper-server` as a child process on an auto-selected free port and updates `server_url` to point at it — and guarantees it is killed on Hub exit or `many-ai-cli stop` using an OS-appropriate mechanism so it can never be orphaned: a Windows Job Object with `KILL_ON_JOB_CLOSE` (so even a Hub crash takes the child down with it), `Setpgid` plus an explicit kill on Linux/macOS, and `init: true` (tini as PID 1) inside the Docker image to reap zombies. On a Hub restart, any orphaned `whisper-server` left over from a prior run is detected and reclaimed — both to free the port it was holding and to avoid running two instances at once.

## Self-contained binary and model distribution (Windows-first)

Managed install ships for Windows x64 only in its initial version; macOS and Linux are steered toward manually pointing at an external Whisper server URL instead. The binary/model manifest is a table keyed by OS/arch → `{URL, SHA256, ServerNames, KeepFromZip, Runtime}`, so adding a new supported platform is a manifest entry rather than new code. Everything managed Whisper installs lands under `~/.many-ai-cli/whisper/{bin,models,tmp}/`; a download is preceded by a free-disk-space check, verified against a SHA256 checksum before extraction, and a failed/partial download is deleted so a retry starts clean rather than resuming a possibly-corrupt partial file. Uninstall is a plain `os.RemoveAll` of that directory tree — no trace is left in `System32` or anywhere else outside the tool's own directory.

`internal/whisperruntime`'s own package doc states its purpose precisely: it embeds OS-local runtime payloads (the Windows VC++ runtime DLLs `whisper-server` links against — `vcomp140.dll`, `msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`) via `go:embed` and lays them down next to the managed binary specifically so the install needs no System32 write and no machine-wide runtime dependency — everything again lives under `~/.many-ai-cli/whisper/`, so an uninstall's `RemoveAll` leaves zero trace of these either. Because the real signed DLLs are gitignored (obtained only at release/CI time — see [Build, Packaging, and Release Pipeline](/openwiki/deployment/build-and-release.md) for the release workflow's `prepare-whisper-runtime` job), the `files/` embed directory ships placeholder README files so `go:embed` always finds at least one file and a from-source build succeeds even before the real DLLs are dropped in; until they are, `Ensure` (the function that copies the embedded payload out) simply copies nothing.

`fetch_windows_runtime.ps1` — the script that actually obtains those four DLLs for CI/release — deliberately never pulls from a third-party DLL distribution site: it searches, in priority order, a local Visual Studio Redist folder (located via `vswhere`) and then falls back to the Microsoft-signed servicing copies already present in `System32`, and validates every DLL it selects has a valid Authenticode signature naming "Microsoft Corporation" as the subject and an x64 (`0x8664`) PE machine type before accepting it.
