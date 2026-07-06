# Claude Slash Commands

| Command | Purpose | When to use it |
|---|---|---|
| `/add-dir` | Add a working directory for file access. | When Claude needs files from an external directory. |
| `/agents` | Manage subagent configurations. | When delegating tasks to background agents. |
| `/autofix-pr` | Watch the current branch's PR and push fixes when CI fails. | When you want automatic CI-driven fixes on a PR. |
| `/background` | Detach the current session to run as a background agent. | When you need to free the terminal while work continues. |
| `/batch` | Orchestrate large-scale changes across a codebase in parallel. | When making wide-ranging changes across many files. |
| `/branch` | Branch the current conversation at this point. | When exploring multiple paths from the same point. |
| `/btw` | Ask a quick side question without adding to the conversation. | When you need clarification without bloating history. |
| `/chrome` | Configure Claude in Chrome settings. | When managing the Chrome extension integration. |
| `/claude-api` | Load Claude API reference material or migrate API code. | When building with or upgrading Claude API code. |
| `/clear` | Clear conversation history. | Start a clean thread without leaving the session. |
| `/code-review` | Review the current diff for bugs and cleanups. | Before shipping code or when you want quality feedback. |
| `/color` | Set the prompt bar color for the current session. | When you want visual differentiation between sessions. |
| `/compact` | Summarize and compact the current conversation. | Reduce context usage while keeping important details. |
| `/config` | Open CLI configuration. | Review or change project/user config values. |
| `/context` | Visualize current context usage as a colored grid. | When you want to see where your context window is going. |
| `/copy` | Copy the last assistant response to the clipboard. | When you need to extract code or text from a response. |
| `/cost` | Show token and cost usage. | Check recent usage impact while iterating. |
| `/debug` | Enable debug logging and troubleshoot issues. | When diagnosing installation or runtime problems. |
| `/deep-research` | Fan out web searches and synthesize a cited report. | When researching complex topics that need verification. |
| `/desktop` | Continue the current session in the Desktop app. | When switching from CLI to desktop. |
| `/diff` | Open an interactive diff viewer of uncommitted changes. | When reviewing what code changed during a session. |
| `/doctor` | Run environment diagnostics. | Troubleshoot local setup and runtime issues. |
| `/effort` | Set the model effort level: low, medium, high, xhigh, max, or ultracode. | When you want to adjust reasoning depth and token usage. |
| `/exit` | Exit the CLI. | When ending a session. |
| `/export` | Export the current conversation as plain text. | When you want to save the conversation to a file. |
| `/fast` | Toggle fast mode. | When you want faster output on supported models. |
| `/feedback` | Submit feedback or report a bug to Anthropic. | When reporting issues or sharing feedback. |
| `/fewer-permission-prompts` | Add an allowlist for common read-only calls. | When you are tired of permission prompts for safe operations. |
| `/focus` | Toggle the focus view showing only the last prompt and response. | When you want a cleaner interface. |
| `/goal` | Set a completion condition and auto-continue turns until it is met. | Let Claude keep working toward a stated goal; /goal clear cancels. |
| `/heapdump` | Write a JavaScript heap snapshot for diagnosing memory usage. | When Claude Code is using too much memory. |
| `/help` | Show help. | Check available commands and usage. |
| `/hooks` | View and manage hook configurations for tool events. | When managing automation and tool permissions. |
| `/ide` | Manage IDE integrations and show status. | When configuring VS Code or JetBrains integration. |
| `/init` | Initialize `CLAUDE.md` guidance. | Bootstrap project instructions for Claude Code. |
| `/insights` | Generate a report analyzing your Claude Code sessions. | When reviewing usage patterns and friction points. |
| `/install-github-app` | Set up the Claude GitHub Actions app. | When enabling Claude in your GitHub workflow. |
| `/install-slack-app` | Install the Claude Slack app. | When integrating Claude into Slack. |
| `/keybindings` | Open or create your keybindings configuration file. | When customizing keyboard shortcuts. |
| `/login` | Sign in to Claude services. | Authenticate the CLI in a new environment. |
| `/logout` | Sign out from Claude services. | Remove current authentication from this environment. |
| `/loop` | Run a prompt repeatedly while the session stays open. | When monitoring tasks or running periodic checks. |
| `/mcp` | Manage MCP server connections and OAuth authentication. | When configuring external tools and data sources. |
| `/memory` | Edit `CLAUDE.md` memory files and manage auto-memory. | When updating project knowledge or context. |
| `/mobile` | Show a QR code to download the Claude mobile app. | When you want to access Claude on your phone. |
| `/model` | Change model selection. | Switch models for speed, quality, or cost tradeoffs. |
| `/passes` | Share a free week of Claude Code with friends. | When inviting others to try Claude Code. |
| `/permissions` | Manage allow, ask, and deny rules for tool permissions. | When configuring security and access controls. |
| `/plan` | Enter plan mode directly from the prompt. | When you want to review planned changes before execution. |
| `/plugin` | Manage Claude Code plugins. | When installing or configuring plugins. |
| `/powerup` | Discover Claude Code features through interactive lessons. | When learning new capabilities. |
| `/privacy-settings` | View and update your privacy settings. | When managing data and telemetry preferences. |
| `/radio` | Open Claude FM lo-fi radio in your browser. | When you want background music while coding. |
| `/recap` | Generate a one-line summary of the current session. | When you want a quick reminder of session context. |
| `/release-notes` | View the changelog in an interactive version picker. | When checking what's new in Claude Code. |
| `/reload-plugins` | Reload all active plugins to apply pending changes. | When you have modified plugin code during a session. |
| `/reload-skills` | Re-scan skill directories to discover new skills. | When you have added skills during a session. |
| `/remote-control` | Make this session available for remote control from claude.ai. | When controlling this session from another device. |
| `/remote-env` | Configure the default remote environment for web sessions. | When setting up default tools for cloud sessions. |
| `/rename` | Rename the current session. | When you want a memorable label in the session picker. |
| `/resume` | Resume a prior conversation. | Continue work from an earlier thread. |
| `/review` | Review a pull request locally. | When analyzing code changes in a PR. |
| `/rewind` | Rewind the conversation and code to a previous point. | When you want to undo changes and conversation. |
| `/run` | Launch and drive your project's app to see a change working. | When testing changes in a running application. |
| `/run-skill-generator` | Teach /run and /verify how to build and launch your project. | When setting up automated build and launch recipes. |
| `/sandbox` | Toggle sandbox mode. | When controlling execution environment isolation. |
| `/schedule` | Create or manage routines that execute on a schedule. | When you want autonomous tasks on Anthropic infrastructure. |
| `/scroll-speed` | Adjust mouse wheel scroll speed interactively. | When customizing scroll behavior in fullscreen mode. |
| `/security-review` | Analyze pending changes for security vulnerabilities. | When checking code for injection, auth, or data exposure risks. |
| `/setup-bedrock` | Configure Amazon Bedrock authentication and model pins. | When using Bedrock as your API provider. |
| `/setup-vertex` | Configure Google Vertex AI authentication and model pins. | When using Vertex AI as your API provider. |
| `/simplify` | Review code for cleanup and apply fixes. | When optimizing code without hunting for bugs. |
| `/skills` | List available skills. | When discovering what skills are available. |
| `/stats` | Show usage statistics on the Stats tab. | When viewing usage statistics. |
| `/status` | Show current status. | Inspect account, connectivity, and runtime state. |
| `/statusline` | Configure Claude Code's status line display. | When customizing the status bar. |
| `/stickers` | Order Claude Code stickers. | When you want branded merchandise. |
| `/stop` | Stop the current background session. | When terminating a running background agent. |
| `/tasks` | List and manage background tasks. | When monitoring or controlling parallel work. |
| `/team-onboarding` | Generate a team onboarding guide from usage history. | When sharing best practices with teammates. |
| `/teleport` | Pull a Claude Code on the web session into this terminal. | When continuing a web session locally. |
| `/terminal-setup` | Configure terminal keybindings for shortcuts. | When setting up Shift+Enter and other shortcuts. |
| `/theme` | Change the color theme. | When customizing the visual appearance. |
| `/tui` | Set the terminal UI renderer. | When switching between standard and fullscreen rendering. |
| `/ultraplan` | Draft a plan in a cloud session and review it. | When planning complex work with deep reasoning. |
| `/ultrareview` | Run a deep multi-agent code review in the cloud. | When getting thorough code analysis. |
| `/upgrade` | Open the upgrade page. | When switching to a higher plan tier. |
| `/usage` | Show usage statistics. | Review limits and ongoing usage trends. |
| `/usage-credits` | Configure usage credits for overages. | When setting up extra usage beyond your plan. |
| `/verify` | Confirm a code change by building and running the app. | When validating changes work in the running application. |
| `/voice` | Toggle voice dictation or set the mode. | When using voice input for prompts. |
| `/web-setup` | Connect your GitHub account for Claude Code on the web. | When setting up web session GitHub integration. |
| `/workflows` | Open the workflow progress view. | When monitoring and controlling dynamic workflows. |
