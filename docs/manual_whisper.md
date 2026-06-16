# Whisper Voice Input Setup

This guide covers local Whisper voice input for the Hub UI.

## Managed Install: Windows x64 Hub

Managed install is available only when the Hub process itself runs on Windows x64.

1. Start the Hub and open the UI.
2. Open Settings -> Voice.
3. Choose `Whisper (local)`.
4. In Local Whisper, choose a model.
5. Click Install and wait for the progress bar to finish.
6. Click Start, then use the microphone button or `Alt+V`.

The installer stores files under `~/.many-ai-cli/whisper/`:

| Path | Purpose |
|---|---|
| `bin/` | Extracted whisper.cpp server binaries plus bundled runtime DLLs |
| `models/` | Downloaded ggml model files |
| `tmp/` | Temporary downloads |
| `whisper-server.log` | Managed server stdout/stderr |

The Hub starts the managed server on `127.0.0.1` and writes the selected local URL back to `voice.whisper.server_url`.

### Self-contained VC++ runtime (no System32 dependency)

`whisper-server.exe` links against the Microsoft Visual C++ runtime — in
particular `vcomp140.dll` (OpenMP), which the official `whisper-bin-x64.zip` does
not ship. On machines without the VC++ redistributable this caused a startup
failure (`0xC0000135` / `STATUS_DLL_NOT_FOUND`).

To stay self-contained, many-ai-cli bundles four x64 runtime DLLs
(`vcomp140.dll`, `msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`) inside
the binary (`go:embed`) and lays them down next to `whisper-server.exe` in `bin/`
on install and before each start. Nothing is written to System32, so an uninstall
(`RemoveAll` of `~/.many-ai-cli/whisper/`) leaves no trace. The UCRT
(`ucrtbase.dll`) is a Windows 10/11 system component and is not bundled.

The DLLs are sourced from the Visual Studio `\VC\redist` folder by
`internal/whisperruntime/fetch_windows_runtime.ps1`, which checks the Authenticode
signature and PE machine type. They are gitignored (not committed), so they must
be placed into `internal/whisperruntime/files/windows-amd64/` **before the Go
build** for `go:embed` to pick them up — by running that script on the build
machine, or by committing the signed DLLs.

> Note: this placement is **not yet automated in the release pipeline**
> (`release.yml` builds on Linux via GoReleaser and does not run the Windows-only
> fetch script). Until that is wired up — e.g. a `windows-latest` build leg that
> runs the script, or committing the DLLs — a released Windows binary embeds the
> runtime only if the DLLs were present at build time. When they are absent,
> `whisperruntime.Ensure` is a safe no-op and managed Whisper still relies on a
> machine-wide VC++ runtime (the original `0xC0000135` exposure).

See `docs/local/plan_unified-local-whisper-all-os.md` (C2) for the licensing terms
(Microsoft "Distributable Code", redistributed app-local).

## Downloads And Verification

Managed install is opt-in. It downloads:

- whisper.cpp Windows x64 release archive from `https://github.com/ggml-org/whisper.cpp/releases`
- the selected ggml model from `https://huggingface.co/ggerganov/whisper.cpp`

The whisper.cpp release archive is SHA-256 verified before extraction. Model entries without a published hash are downloaded over HTTPS and shown in the UI as hash-unverified.

## Managed Install: Docker (Linux / リモートサーバー) With A Bundled Server

The primary remote use case is iPhone -> SSH tunnel -> リモートサーバー Hub -> localhost
Whisper. For that, the many-ai-cli Docker image bakes a `whisper-server` binary
(built from whisper.cpp with `GGML_OPENMP=OFF`, so no libgomp dependency) into
the image and points the Hub at it with the `MANY_AI_CLI_WHISPER_SERVER`
environment variable.

When that variable points at an existing executable, the Hub treats Whisper as
"managed and already installed": the binary download is skipped and only the
selected model is downloaded into `~/.many-ai-cli/whisper/models/`. With the
default compose setup that path lives on the user's home volume, so the model
survives container recreation and is not re-downloaded.

The compose service runs with `init: true` (tini as PID 1) so the spawned
`whisper-server` child is reaped and never left as a zombie/orphan if the Hub
crashes; the Hub additionally kills the process group on shutdown.

No port is published for Whisper — it listens on `127.0.0.1` inside the
container's network namespace and is reached only by the Hub in the same
container.

## Manual Server: macOS, Linux, WSL, Or Custom Builds

For non-Windows Hub environments without a bundled server, run a
Whisper-compatible server yourself and point the Hub at it.

Example shape:

```bash
whisper-server -m /path/to/ggml-large-v3-turbo-q5_0.bin --host 127.0.0.1 --port 8080
```

Then edit `~/.many-ai-cli/config.yaml`:

```yaml
voice:
  whisper:
    managed: false
    server_url: "http://127.0.0.1:8080"
    request_path: ""
    language: "ja"
    timeout_seconds: 60
```

The Hub first tries OpenAI-compatible `/v1/audio/transcriptions` and then falls back to `/inference`. Set `request_path` only when your server needs a fixed custom path.

## Model Choice

Managed Whisper runs on the CPU (the bundled whisper.cpp build has no GPU
support), so transcription latency is bound by CPU cores. Measured on a 10
logical-CPU machine with a 5-7 s Japanese utterance:

| Model | Latency | Accuracy |
|---|---|---|
| `small` (default) | 2-3 s | Good sentence structure; may misspell technical terms |
| `large-v3-turbo-q5_0` | ~7 s | Best accuracy; latency grows quickly on fewer cores |
| `tiny-q5_1` | <1 s | Smoke test only; not usable for real input |

Start with `small`. Switch to `large-v3-turbo-q5_0` only on a fast multi-core
CPU, or when pointing `server_url` at an external GPU-backed Whisper server.

## Config Reference

| Key | Meaning |
|---|---|
| `voice.whisper.managed` | `true` lets the Hub manage the local whisper.cpp server. Supported on Windows x64, and on images/hosts that provide a server via `MANY_AI_CLI_WHISPER_SERVER`. |
| `voice.whisper.model` | Managed model ID. Default: `small`. |
| `voice.whisper.server_url` | Local Whisper server URL. Managed mode writes this automatically. |
| `voice.whisper.server_port` | Preferred managed server port. `0` means auto-pick. |
| `voice.whisper.request_path` | Optional endpoint override. Empty means auto-probe. |
| `voice.whisper.language` | Language hint such as `ja`, `en`, or `auto`. |
| `voice.whisper.timeout_seconds` | Hub proxy timeout for one transcription request. |

## Recommended Server Options

Whisper can hallucinate fixed phrases on silence or background noise. Keep these layers enabled where possible:

- Use server-side VAD or no-speech filtering when your whisper.cpp build supports it.
- Use deterministic or low-temperature decoding.
- Keep the server bound to `127.0.0.1`.
- Review the inserted text before sending. Whisper mode inserts text into the input field by default.

The browser side also drops near-silent recordings and discards exact matches for known hallucination phrases.

## Troubleshooting

| Symptom | Check |
|---|---|
| `Whisper server is not installed` | On Windows x64 (or a Docker image with a bundled server), run Settings -> Voice -> Install. On unsupported platforms, configure `server_url`. |
| `Whisper server is not configured` | Set `voice.whisper.server_url` or enable managed install. |
| Connection error | Confirm the server is listening on `127.0.0.1` and that the configured port matches. |
| Slow transcription | Try a smaller model such as `small` or `tiny-q5_1`. |
| Empty or hallucinated result | Enable VAD/no-speech filtering and keep auto-submit disabled until validated. |
| Managed server fails to start | Check `~/.many-ai-cli/whisper/whisper-server.log`. |
