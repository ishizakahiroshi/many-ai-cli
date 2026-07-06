# Grok Build Slash Commands

> Grok Build は Claude Code / Cursor 互換 harness。スラッシュコマンドは `grok --help` のサブコマンド群（dashboard / export / login / logout / mcp / memory / models / sessions 等）と Claude Code 系の共通コマンド（help / model / clear / compact / resume）から導いた高確度分のみを掲載。実機 `grok` の `/help` と差分があれば実機出力を優先する（要確定）。

| Command | Purpose | When to use it |
|---|---|---|
| `/help` | Show help for available commands. | When you need guidance on commands. |
| `/model` | Set or list the active model. | When switching which model you work with. |
| `/clear` | Clear conversation history. | Start a clean thread without leaving the session. |
| `/compact` | Summarize and compact the current conversation. | When reducing context usage during long sessions. |
| `/resume` | Resume a previous session. | When returning to an earlier conversation. |
| `/dashboard` | Open the Agent Dashboard view. | When monitoring multiple parallel sessions. |
| `/mcp` | Manage MCP server configurations. | When configuring external tool integrations. |
| `/memory` | Manage cross-session memory. | When updating persistent context. |
| `/export` | Export the session transcript. | When saving the conversation. |
| `/login` | Sign in to Grok. | When authenticating the CLI. |
| `/logout` | Sign out and clear cached credentials. | When disconnecting your account on this machine. |
