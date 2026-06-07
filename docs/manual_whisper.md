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

The installer stores files under `~/.any-ai-cli/whisper/`:

| Path | Purpose |
|---|---|
| `bin/` | Extracted whisper.cpp Windows server binaries |
| `models/` | Downloaded ggml model files |
| `tmp/` | Temporary downloads |
| `whisper-server.log` | Managed server stdout/stderr |

The Hub starts the managed server on `127.0.0.1` and writes the selected local URL back to `voice.whisper.server_url`.

## Downloads And Verification

Managed install is opt-in. It downloads:

- whisper.cpp Windows x64 release archive from `https://github.com/ggml-org/whisper.cpp/releases`
- the selected ggml model from `https://huggingface.co/ggerganov/whisper.cpp`

The whisper.cpp release archive is SHA-256 verified before extraction. Model entries without a published hash are downloaded over HTTPS and shown in the UI as hash-unverified.

## Manual Server: macOS, Linux, WSL, Or Custom Builds

For non-Windows Hub environments, run a Whisper-compatible server yourself and point the Hub at it.

Example shape:

```bash
whisper-server -m /path/to/ggml-large-v3-turbo-q5_0.bin --host 127.0.0.1 --port 8080
```

Then edit `~/.any-ai-cli/config.yaml`:

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

## Config Reference

| Key | Meaning |
|---|---|
| `voice.whisper.managed` | `true` lets the Hub manage the local whisper.cpp server. Windows x64 only. |
| `voice.whisper.model` | Managed model ID. Default: `large-v3-turbo-q5_0`. |
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
| `Whisper server is not installed` | On Windows x64, run Settings -> Voice -> Install. On other platforms, configure `server_url`. |
| `Whisper server is not configured` | Set `voice.whisper.server_url` or enable managed install. |
| Connection error | Confirm the server is listening on `127.0.0.1` and that the configured port matches. |
| Slow transcription | Try a smaller model such as `small` or `tiny-q5_1`. |
| Empty or hallucinated result | Enable VAD/no-speech filtering and keep auto-submit disabled until validated. |
| Managed server fails to start | Check `~/.any-ai-cli/whisper/whisper-server.log`. |
