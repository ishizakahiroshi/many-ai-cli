# Cursor Agent Slash Commands

> 出典: 公式ドキュメント https://cursor.com/docs/cli/reference/slash-commands （2026-07-22 取得）。実機 `cursor-agent` の `/help` と差分があれば実機出力を優先する。

| Command | Purpose | When to use it |
|---|---|---|
| `/about` | Show CLI version, system, and account info. | When you need system diagnostics or version checks. |
| `/ask` | Toggle Ask mode for read-only questions. | When investigating code without triggering actions. |
| `/bedrock` | Configure Bedrock when the Bedrock feature is enabled. | When setting up AWS Bedrock integration. |
| `/clear` / `/new` / `/new-chat` / `/newchat` | Start a new chat session. | When beginning a fresh conversation. |
| `/config` | Configure CLI settings interactively. | When customizing tool behavior. |
| `/copy` | Copy a previous user message to the clipboard. | When reusing an earlier input. |
| `/copy-conversation-id` | Copy the current conversation ID to the clipboard. | When referencing the current session. |
| `/copy-request-id` | Copy the last request ID to the clipboard. | When sharing request details for support. |
| `/debug` | Toggle Debug mode or submit a prompt in Debug mode. | When diagnosing issues that need extra context. |
| `/feedback` | Share feedback with the Cursor team. | When reporting issues or suggestions. |
| `/fork` | Fork the current chat into a new session. | When exploring an alternate direction from this point. |
| `/help` | Show help, optionally for a specific command. | When you need guidance on available commands. |
| `/line-numbers` | Toggle line numbers in code blocks. | When you need reference points in code output. |
| `/logout` | Sign out from Cursor. | When disconnecting your account on this machine. |
| `/logs` | Show the debug log path and copy it to the clipboard. | When accessing debug logs for troubleshooting. |
| `/max-mode` | Toggle Max Mode on legacy request-based plans. | When working with older plan configurations. |
| `/mcp` | Manage MCP servers and list tools for a server. | When configuring external tool integrations. |
| `/model` | Select a model, with tab to filter. | When switching which model you work with. |
| `/open` / `/cursor` | Open the repository's Git root in Cursor. | When jumping into the project editor. |
| `/orchestrate` | Split work into child AI sessions using the Hub board API. | Coordinate design, implementation, and test workers from one conductor session. |
| `/plan` | Switch to Plan mode, show the current plan, or submit a Plan-mode prompt. | When outlining strategy before implementation. |
| `/plugin` | Manage plugins and marketplaces. | When installing or configuring extensions. |
| `/quit` / `/exit` | Exit the CLI. | When closing the session. |
| `/rename` | Rename the current chat session. | When you want a memorable label for the conversation. |
| `/resume` | Open recent chats and resume one. | When returning to an earlier conversation. |
| `/rewind` | Jump back to a previous message. | When revisiting an earlier point in the conversation. |
| `/run-everything` / `/auto-run` | Toggle automatic execution or check its current status. | When controlling whether commands run without confirmation. |
| `/sandbox` | Configure sandbox mode and network access settings. | When managing execution environment restrictions. |
| `/setup-terminal` | Configure terminal newline keybindings. | When setting up terminal integration. |
| `/shell` / `/sh` / `/run` | Enter Shell Mode. | When running terminal commands directly. |
| `/show-thinking` | Toggle thinking block display. | When you want to see the model's reasoning. |
| `/status-indicators` | Toggle terminal title status indicators. | When you want visual status feedback in the terminal title. |
| `/summarize` / `/compress` | Summarize the conversation to reduce context. | When token usage is becoming excessive. |
| `/update` | Update Cursor Agent to the latest version. | When you want the newest features and fixes. |
| `/vim` | Toggle Vim keybindings. | When enabling Vim-style editing. |
