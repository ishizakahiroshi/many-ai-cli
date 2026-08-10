# GitHub Copilot Slash Commands

| Command | Purpose | When to use it |
|---|---|---|
| `/add-dir` | Add an allowed working directory. | Grant Copilot access to another local directory. |
| `/agent` | Manage the active agent environment. | Switch or inspect agent setup. |
| `/allow-all` | Allow all tool use for the session. | Temporarily run without per-tool prompts. |
| `/app` | Manage application configuration and connectors. | Configure or inspect app-level settings. |
| `/ask` | Ask a direct question. | Get an answer without changing task mode. |
| `/autopilot` | Toggle autopilot behavior. | Let Copilot proceed more independently. |
| `/changelog` | Show recent CLI changes. | Check what changed after an update. |
| `/chronicle` | Open chronicle/session history features. | Review or manage past activity. |
| `/clear` | Clear the terminal view. | Reset visible output without leaving the session. |
| `/compact` | Compact conversation context. | Reclaim context during long sessions. |
| `/context` | Inspect current context usage. | Understand what is loaded into the session. |
| `/copy` | Copy current output or context. | Move session text to the clipboard. |
| `/cwd` | Show or change the current working directory. | Confirm where tools will run. |
| `/delegate` | Delegate work to another agent. | Split work across agents or side tasks. |
| `/diagnose` | Run environment diagnostics. | Troubleshoot local CLI or runtime issues. |
| `/diff` | Show working tree changes. | Review edits before committing or continuing. |
| `/env` | Inspect environment information. | Debug local CLI/runtime configuration. |
| `/exit` | Exit the CLI. | End the current session. |
| `/experimental` | Manage experimental features. | Try or inspect preview capabilities. |
| `/feedback` | Send feedback. | Report issues to GitHub. |
| `/fleet` | Manage a group of agents. | Coordinate several agent workers. |
| `/footer` | Configure footer display. | Customize the TUI status footer. |
| `/help` | Show help. | Discover available commands. |
| `/ide` | Connect IDE context. | Include editor state in the session. |
| `/init` | Initialize project guidance. | Generate or refresh repo instructions. |
| `/instructions` | Inspect instruction sources. | See guidance currently applied to the session. |
| `/keep-alive` | Keep the session alive. | Prevent an idle session from stopping. |
| `/limits` | Show usage limits and quotas. | Check your Copilot rate limits. |
| `/list-dirs` | List allowed directories. | Audit what paths Copilot can access. |
| `/login` | Sign in. | Authenticate GitHub Copilot CLI. |
| `/logout` | Sign out. | Clear the active GitHub authentication. |
| `/lsp` | Manage language-server context. | Debug editor/language intelligence integration. |
| `/mcp` | Manage MCP servers. | Inspect external tools connected to Copilot. |
| `/memory` | Manage session memory settings. | Configure memory use and retention. |
| `/model` | Select the model. | Switch model before continuing work. |
| `/new` | Start a new session. | Reset conversation context. |
| `/orchestrate` | Split work into child AI sessions using the Hub board API. | Coordinate design, implementation, and test workers from one conductor session. |
| `/permissions` | Switch between permission modes. | Change how much Copilot asks before it runs tools. |
| `/plan` | Enter planning mode. | Ask for a plan before implementation. |
| `/plugin` | Manage plugins. | Inspect or configure plugin features. |
| `/pr` | Work with pull requests. | Prepare or inspect PR-related changes. |
| `/remote` | Manage remote session features. | Share or control a session remotely. |
| `/refine` | Rewrite a rough prompt into a clearer version for review. | When you want Copilot to polish a stream-of-consciousness prompt. |
| `/rename` | Rename the session. | Give a session a clear title. |
| `/research` | Research a topic. | Gather background before coding. |
| `/reset-allowed-tools` | Reset tool permission rules. | Clear remembered permission decisions. |
| `/restart` | Restart the CLI. | Recover from a bad local session state. |
| `/resume` | Resume a previous session. | Continue saved work. |
| `/review` | Review code changes. | Ask Copilot to inspect the current diff. |
| `/rewind` | Rewind session state. | Return to an earlier point in the conversation. |
| `/rubber-duck` | Get an independent critique of your current work. | When you want a rubber-duck style review from another agent. |
| `/search` | Search. | Find context or content from the CLI. |
| `/security-review` | Analyze pending changes for security issues. | Scan the current diff for vulnerabilities. |
| `/session` | Show session details. | Inspect current session metadata. |
| `/settings` | Open Copilot CLI settings. | Adjust CLI preferences. |
| `/share` | Share session output. | Create a shareable session artifact. |
| `/skills` | Manage skills. | Inspect or load reusable instructions. |
| `/statusline` | Configure status line fields. | Customize TUI status display. |
| `/subagents` | Manage subagents. | Configure delegated helper agents. |
| `/tasks` | Manage tasks. | Track delegated or queued work. |
| `/terminal-setup` | Configure terminal integration. | Fix shell or terminal behavior. |
| `/theme` | Select a theme. | Change visual appearance. |
| `/update` | Check for or apply updates. | Keep GitHub Copilot CLI current. |
| `/usage` | Show usage information. | Inspect quota or usage stats. |
| `/user` | Show user/account info. | Confirm which GitHub identity is active. |
| `/version` | Show version. | Confirm installed CLI version. |
| `/voice` | Toggle voice dictation. | Use voice input for prompts. |
