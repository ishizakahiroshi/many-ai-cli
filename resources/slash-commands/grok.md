# Grok Build Slash Commands

> Sourced from Grok Build CLI's official user guide (`04-slash-commands.md`).
> Type `/` in the Grok prompt to autocomplete. Real `grok` `/help` output wins if it drifts.

| Command | Purpose | When to use it |
|---|---|---|
| `/help` | Show help for available commands. | When you need guidance on commands. |
| `/new` | Start a new session, clearing the current conversation. | Fresh thread without leaving the CLI. Aliases: `/clear` |
| `/clear` | Clear conversation history (alias of `/new`). | Start a clean thread without leaving the session. |
| `/resume` | Open the session picker to load a previous session from disk. | When returning to an earlier conversation. |
| `/history` | Open prompt-history search (fuzzy, newest first). | Recall a past prompt; `↑` on empty prompt is the quick path. |
| `/compact` | Summarize and compact the current conversation. | When reducing context usage during long sessions. |
| `/context` | Show context window usage and session stats. | Check how full the context window is. |
| `/session-info` | Show session details (model, turn count, context). | Inspect the active session. |
| `/fork` | Branch the current session into a new agent, preserving history. | Parallel exploration without losing the original. |
| `/rewind` | Rewind the conversation to an earlier turn. | Undo later turns after a wrong direction. |
| `/model` | Set or list the active model. | When switching which model you work with. Aliases: `/m` |
| `/effort` | Set reasoning effort on the current model (`low`/`medium`/`high`/`xhigh`). | Tune depth without re-picking the model. |
| `/always-approve` | Toggle skip-all permission prompts. | When you trust the session to run tools freely. |
| `/auto` | Toggle classifier-based auto permission mode. | Safer auto-run; dangerous tools may still prompt. |
| `/multiline` | Toggle multiline input mode. | Multi-line prompts; aliases: `/ml` |
| `/compact-mode` | Toggle compact display density. | Fit more output on screen. |
| `/vim-mode` | Toggle vim-style scrollback keybindings. | j/k navigation in scrollback. |
| `/minimal` | Switch this session to scrollback-native (minimal) mode. | Prefer normal scrollback over alt-screen TUI. |
| `/fullscreen` | Switch this session back to the standard alt-screen TUI. | Return from `/minimal`. Alias: `/full` |
| `/plan` | Enter plan mode. | Design before editing. |
| `/view-plan` | Open the current saved plan preview. | Review the active plan. Aliases: `/show-plan`, `/plan-view` |
| `/memory` | Manage cross-session memory. | When updating persistent context (needs experimental memory). |
| `/export` | Export the session transcript. | When saving the conversation. |
| `/copy` | Copy the most recent response (or write to a file path). | Grab the last answer without selecting text. |
| `/rename` | Rename the current session. | Label sessions. Alias: `/title` |
| `/home` | Exit the current session and return to the welcome screen. | Back to session picker. Aliases: `/welcome` |
| `/dashboard` | Open the Agent Dashboard view. | When monitoring multiple parallel sessions. |
| `/mcp` | Manage MCP server configurations. | When configuring external tool integrations. |
| `/login` | Sign in to Grok. | When authenticating the CLI. |
| `/logout` | Sign out and clear cached credentials. | When disconnecting your account on this machine. |
| `/quit` | Quit the application. | Exit Grok. Aliases: `/exit` |
