# Claude Slash Commands

| Command | Purpose | When to use it |
|---|---|---|
| `/add-dir` | Add a working directory for file access. | When Claude needs files from an external directory. |
| `/advisor` | Toggle secondary-model advisor guidance. | When you want a second-opinion advisor on the current turn. |
| `/agents` | Manage subagent configurations. | When delegating tasks to background agents. |
| `/autocompact` | Set how full context gets before auto-compacting. | When managing long conversations that risk overflow. |
| `/autofix-pr` | Watch the current branch's PR and push fixes when CI fails. | When you want automatic CI-driven fixes on a PR. |
| `/background` | Detach the current session to run as a background agent. | When you need to free the terminal while work continues. |
| `/batch` | Orchestrate large-scale changes across a codebase in parallel. | When making wide-ranging changes across many files. |
| `/branch` | Branch the current conversation at this point. | When exploring multiple paths from the same point. |
| `/btw` | Ask a quick side question without adding to the conversation. | When you need clarification without bloating history. |
| `/bug` | Report a bug and share the conversation history. | When something breaks and you want it recorded. |
| `/cd` | Change the session's working directory. | When you want to switch cwd without restarting the session. |
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
| `/dataviz` | Load data visualization design guidance. | When designing charts, dashboards, or other data visuals. |
| `/debug` | Enable debug logging and troubleshoot issues. | When diagnosing installation or runtime problems. |
| `/deep-research` | Fan out web searches and synthesize a cited report. | When researching complex topics that need verification. |
| `/design-login` | Authorize access to design-system assets. | When enabling design system export from a design tool. |
| `/design-sync` | Convert a design system into React components. | When importing a design system into a React codebase. |
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
| `/fork` | Spawn a forked subagent from the current conversation. | When you want an independent thread from the same context. |
| `/goal` | Set a completion condition and auto-continue turns until it is met. | Let Claude keep working toward a stated goal; /goal clear cancels. |
| `/heapdump` | Write a JavaScript heap snapshot for diagnosing memory usage. | When Claude Code is using too much memory. |
| `/help` | Show help. | Check available commands and usage. |
| `/hooks` | View and manage hook configurations for tool events. | When managing automation and tool permissions. |
| `/ide` | Manage IDE integrations and show status. | When configuring VS Code or JetBrains integration. |
| `/import` | Bring configuration over from Codex or Gemini CLI. | When migrating from another coding agent. |
| `/init` | Initialize CLAUDE.md guidance. | Bootstrap project instructions for Claude Code. |
| `/insights` | Generate a report analyzing your Claude Code sessions. | When reviewing usage patterns and friction points. |
| `/install-github-app` | Set up the Claude GitHub Actions app. | When enabling Claude in your GitHub workflow. |
| `/install-slack-app` | Install the Claude Slack app. | When integrating Claude into Slack. |
| `/keybindings` | Open or create your keybindings configuration file. | When customizing keyboard shortcuts. |
| `/list-agents` | List subagents and sessions Claude can message. | When you need an agent name for cross-session messaging. |
| `/login` | Sign in to Claude services. | Authenticate the CLI in a new environment. |
| `/logout` | Sign out from Claude services. | Remove current authentication from this environment. |
| `/loop` | Run a prompt repeatedly while the session stays open. | When monitoring tasks or running periodic checks. |
| `/mcp` | Manage MCP server connections and OAuth authentication. | When configuring external tools and data sources. |
| `/memory` | Edit CLAUDE.md memory files and manage auto-memory. | When updating project knowledge or context. |
| `/mobile` | Show a QR code to download the Claude mobile app. | When you want to access Claude on your phone. |
| `/model` | Change model selection. | Switch models for speed, quality, or cost tradeoffs. |
| `/orchestrate` | Split work into child AI sessions using the Hub board API. | Coordinate design, implementation, and test workers from one conductor session. |
| `/passes` | Share a free week of Claude Code with friends. | When inviting others to try Claude Code. |
| `/permissions` | Manage allow, ask, and deny rules for tool permissions. | When configuring security and access controls. |
| `/plan` | Enter plan mode directly from the prompt. | When you want to review planned changes before execution. |
| `/plugin` | Manage Claude Code plugins. | When installing or configuring plugins. |
| `/plugins` | Manage installed plugins and their settings. | When configuring or troubleshooting an extension. |
| `/powerup` | Discover Claude Code features through interactive lessons. | When learning new capabilities. |
| `/pr` | Create or manage pull requests from the session. | When you want the PR workflow driven from here. |
| `/privacy-settings` | View and update your privacy settings. | When managing data and telemetry preferences. |
| `/quiet` | Suppress notifications and reduce output verbosity. | When you want a minimal, focused interface. |
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
| `/save` | Save the current conversation or session state. | When preserving work before switching contexts. |
| `/schedule` | Create or manage routines that execute on a schedule. | When you want autonomous tasks on Anthropic infrastructure. |
| `/scroll-speed` | Adjust mouse wheel scroll speed interactively. | When customizing scroll behavior in fullscreen mode. |
| `/security-review` | Analyze pending changes for security vulnerabilities. | When checking code for injection, auth, or data exposure risks. |
| `/session` | Manage session metadata and properties. | When organizing or renaming an active session. |
| `/settings` | Open the settings interface to adjust preferences. | When changing theme, model, or output options. |
| `/setup-bedrock` | Configure Amazon Bedrock authentication and model pins. | When using Bedrock as your API provider. |
| `/setup-vertex` | Configure Google Vertex AI authentication and model pins. | When using Vertex AI as your API provider. |
| `/share` | Share the conversation or send product feedback. | When collaborating or reporting an issue upstream. |
| `/simplify` | Review code for cleanup and apply fixes. | When optimizing code without hunting for bugs. |
| `/skills` | List available skills. | When discovering what skills are available. |
| `/stats` | Show usage statistics on the Stats tab. | When viewing usage statistics. |
| `/status` | Show current status. | Inspect account, connectivity, and runtime state. |
| `/statusline` | Configure Claude Code's status line display. | When customizing the status bar. |
| `/stickers` | Order Claude Code stickers. | When you want branded merchandise. |
| `/subtask` | Hand a side task to a subagent that reports back. | When parallelizing work without leaving the main thread. |
| `/tasks` | List and manage background tasks. | When monitoring or controlling parallel work. |
| `/team-onboarding` | Generate a team onboarding guide from usage history. | When sharing best practices with teammates. |
| `/teleport` | Pull a Claude Code on the web session into this terminal. | When continuing a web session locally. |
| `/test` | Run automated tests on the current project. | When validating changes before committing. |
| `/thinking` | Toggle extended thinking. | When you want the model to think longer before responding. |
| `/uninstall` | Remove installed skills, plugins, or MCP servers. | When cleaning up unused extensions. |
| `/upgrade` | Open the upgrade page. | When switching to a higher plan tier. |
| `/usage` | Show usage statistics. | Review limits and ongoing usage trends. |
| `/verify` | Confirm a code change by building and running the app. | When validating changes work in the running application. |
| `/web` | Use web browsing and search capabilities. | When researching or fetching online content. |
| `/why` | Explain the reasoning behind recent actions. | When you want the decision-making made explicit. |
| `/worktrees` | Manage git worktrees for parallel branches. | When juggling multiple branches without switching cwd. |
