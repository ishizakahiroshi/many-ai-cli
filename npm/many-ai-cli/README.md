# many-ai-cli

Web dashboard to manage approvals and progress across multiple AI coding CLIs
(Claude Code / Codex) running in parallel — a single local Hub + browser UI.

## Install

```sh
pnpm add -g many-ai-cli
# or: bun install -g many-ai-cli
# or: npm install -g many-ai-cli
```

This package ships the native Go binary for your platform as an optional
dependency, so nothing is downloaded in a browser and there is no Windows
SmartScreen "Mark of the Web" prompt. It is **not** a substitute for code
signing — Smart App Control / WDAC / AppLocker / EDR policies are separate.

## Usage

```sh
many-ai-cli serve        # start the Hub (binds 127.0.0.1, opens the browser UI)
many-ai-cli claude       # run Claude Code wrapped by the Hub
many-ai-cli codex        # run Codex wrapped by the Hub
many-ai-cli --version
```

The Hub binds to `127.0.0.1` only and is never exposed externally.

## Supported platforms

| OS | Arch | Package |
|----|------|---------|
| Windows | x64 | `many-ai-cli-win32-x64` |
| Linux | x64 | `many-ai-cli-linux-x64` |
| macOS | Intel (x64) | `many-ai-cli-darwin-x64` |
| macOS | Apple Silicon (arm64) | `many-ai-cli-darwin-arm64` |

Other platforms: download from
[GitHub Releases](https://github.com/ishizakahiroshi/many-ai-cli/releases).

## License

MIT © Hiroshi Ishizaka (ishizakahiroshi)
